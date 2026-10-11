//! The checks the probe runs, one per [`CheckSpec`] kind (D-49).
//!
//! A check only measures: it says whether it succeeded, with a message and
//! its metrics. Thresholds are applied on the hub (proposal 0006 §1.5).
//! [`run`] adds the check's ID and a time limit, so one slow check can't
//! hold up the report.

mod certificate;

pub use certificate::{Trust, public_trust};

use crate::monitor::Monitor;
use common::checks::{
    CheckDefinition, CheckResult, CheckSpec, Health, HttpMethod, HttpOptions, Metric, metric,
};
use std::net::IpAddr;
use std::time::{Duration, Instant};
use surge_ping::{PingIdentifier, PingSequence};
use sysinfo::{Disks, System};
use tokio::net::TcpStream;

/// The longest any check may take (its own timeouts are shorter).
pub const CHECK_TIME_LIMIT: Duration = Duration::from_secs(30);

/// How long a TCP connection may take to open.
const TCP_TIMEOUT: Duration = Duration::from_secs(3);

/// How much of an answer's body an `http` check reads to look for text.
const MAX_BODY_BYTES: usize = 1024 * 1024;

const BYTES_PER_GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// What one run of a check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub status: Health,
    pub message: String,
    pub latency_ms: Option<u64>,
    pub metrics: Vec<Metric>,
}

impl Outcome {
    fn up(message: String) -> Self {
        Self {
            status: Health::Up,
            message,
            latency_ms: None,
            metrics: Vec::new(),
        }
    }

    fn down(message: impl Into<String>) -> Self {
        Self {
            status: Health::Down,
            ..Self::up(message.into())
        }
    }

    fn with_latency(mut self, elapsed: Duration) -> Self {
        self.latency_ms = Some(elapsed.as_millis() as u64);
        self
    }

    fn with_metrics(mut self, metrics: Vec<Metric>) -> Self {
        self.metrics = metrics;
        self
    }
}

/// Run one check within [`CHECK_TIME_LIMIT`] and label its result.
pub async fn run(check: &CheckDefinition, monitor: &Monitor) -> CheckResult {
    let outcome = tokio::time::timeout(CHECK_TIME_LIMIT, measure(&check.spec, monitor))
        .await
        .unwrap_or_else(|_| {
            Outcome::down(format!("No answer within {} s", CHECK_TIME_LIMIT.as_secs()))
        });
    CheckResult {
        check_id: check.id.clone(),
        status: outcome.status,
        message: outcome.message,
        latency_ms: outcome.latency_ms,
        metrics: outcome.metrics,
    }
}

async fn measure(spec: &CheckSpec, monitor: &Monitor) -> Outcome {
    match spec {
        CheckSpec::Ping { host } => ping(host, monitor).await,
        CheckSpec::Http { url, options } => http(url, options, monitor).await,
        CheckSpec::Tcp { host, port } => tcp(host, *port).await,
        CheckSpec::Tls { host, port, verify } => {
            certificate::check(host, *port, *verify, &monitor.tls_trust).await
        }
        CheckSpec::Cpu => cpu(&*monitor.sys_info.read().await),
        CheckSpec::Memory => memory(&*monitor.sys_info.read().await),
        CheckSpec::Disk { path } => disk(path.as_deref(), &*monitor.disk_info.read().await),
        CheckSpec::Uptime => uptime(),
    }
}

async fn ping(host: &str, monitor: &Monitor) -> Outcome {
    let Ok(ip) = host.parse::<IpAddr>() else {
        return Outcome::down("Invalid IP address");
    };
    let mut pinger = monitor.icmp_client.pinger(ip, PingIdentifier(0)).await;
    match pinger.ping(PingSequence(0), &[0u8; 8]).await {
        Ok((_, elapsed)) => Outcome::up(format!("Responded in {elapsed:?}")).with_latency(elapsed),
        Err(e) => Outcome::down(format!("Ping failed: {e}")),
    }
}

async fn http(url: &str, options: &HttpOptions, monitor: &Monitor) -> Outcome {
    let client = monitor.http_client(options.follow_redirects, options.verify);
    let mut request = match options.method {
        HttpMethod::Get => client.get(url),
        HttpMethod::Head => client.head(url),
    };
    for header in &options.headers {
        request = request.header(&header.name, &header.value);
    }
    let start = Instant::now();
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(e) => return Outcome::down(format!("Connection failed: {}", causes(&e))),
    };
    let status = response.status();
    if !options.expects(status.as_u16()) {
        return Outcome::down(format!(
            "HTTP {status} (expected {})",
            options.expected_status()
        ));
    }
    if options.searches_body() {
        let body = match read_body(&mut response).await {
            Ok(body) => body,
            Err(e) => {
                return Outcome::down(format!("HTTP {status}, but reading the page failed: {e}"));
            }
        };
        if let Some(problem) = text_problem(&body, options) {
            return Outcome::down(format!("HTTP {status}, but {problem}"));
        }
    }
    Outcome::up(format!("HTTP {status}")).with_latency(start.elapsed())
}

/// An error with what caused it: reqwest's own text stops at "error sending
/// request", and the reason (refused, a certificate, a timeout) is below.
fn causes(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

/// The start of the answer's body, up to [`MAX_BODY_BYTES`], as text.
async fn read_body(response: &mut reqwest::Response) -> reqwest::Result<String> {
    let mut body = Vec::new();
    while body.len() < MAX_BODY_BYTES {
        match response.chunk().await? {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }
    body.truncate(MAX_BODY_BYTES);
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// What is wrong with the text of a page, if anything.
fn text_problem(body: &str, options: &HttpOptions) -> Option<String> {
    if !options.contains.is_empty() && !body.contains(&options.contains) {
        return Some(format!("the page doesn't contain \"{}\"", options.contains));
    }
    if !options.absent.is_empty() && body.contains(&options.absent) {
        return Some(format!("the page contains \"{}\"", options.absent));
    }
    None
}

async fn tcp(host: &str, port: u16) -> Outcome {
    let start = Instant::now();
    match tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(_)) => Outcome::up("Port Open".to_string()).with_latency(start.elapsed()),
        Ok(Err(e)) => Outcome::down(format!("Connection Refused: {e}")),
        Err(_) => Outcome::down("Connection Timeout"),
    }
}

// Checks of the probe's own machine read the system info `Monitor::refresh`
// gathered once for this cycle.

fn cpu(sys: &System) -> Outcome {
    let usage = f64::from(sys.global_cpu_usage());
    Outcome::up(format!("CPU usage: {usage:.1}%")).with_metrics(vec![Metric::new(
        metric::USAGE,
        usage,
        metric::PERCENT,
    )])
}

fn memory(sys: &System) -> Outcome {
    let (total, used) = (sys.total_memory(), sys.used_memory());
    if total == 0 {
        return Outcome::down("Failed to read memory info");
    }
    usage_outcome("Memory", used, total)
}

fn disk(path: Option<&str>, disks: &Disks) -> Outcome {
    if disks.is_empty() {
        return Outcome::down("No disks found");
    }
    let wanted = path.map_or_else(|| SYSTEM_DISK.to_string(), normalize_disk_target);
    let Some(disk) = disks
        .iter()
        .find(|d| d.mount_point().to_string_lossy() == wanted.as_str())
    else {
        return Outcome::down(format!("Disk {wanted} not found"));
    };
    let (total, free) = (disk.total_space(), disk.available_space());
    if total == 0 {
        return Outcome::down("Disk has zero capacity");
    }
    let mut outcome = usage_outcome(
        &disk.mount_point().display().to_string(),
        total - free,
        total,
    );
    outcome.metrics.insert(
        2,
        Metric::new(metric::FREE, free as f64 / BYTES_PER_GB, metric::GIGABYTES),
    );
    outcome
}

/// "<what>: 3.2 GB / 8.0 GB (40.0%)" with usage %, used GB and total GB.
fn usage_outcome(what: &str, used: u64, total: u64) -> Outcome {
    let usage = used as f64 / total as f64 * 100.0;
    let (used_gb, total_gb) = (used as f64 / BYTES_PER_GB, total as f64 / BYTES_PER_GB);
    Outcome::up(format!(
        "{what}: {used_gb:.1} GB / {total_gb:.1} GB ({usage:.1}%)"
    ))
    .with_metrics(vec![
        Metric::new(metric::USAGE, usage, metric::PERCENT),
        Metric::new(metric::USED, used_gb, metric::GIGABYTES),
        Metric::new(metric::TOTAL, total_gb, metric::GIGABYTES),
    ])
}

fn uptime() -> Outcome {
    let secs = System::uptime();
    let (days, hours, minutes) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60);
    Outcome::up(format!("Uptime: {days}d {hours}h {minutes}m")).with_metrics(vec![Metric::new(
        metric::UPTIME,
        secs as f64,
        metric::SECONDS,
    )])
}

/// The disk a `disk` check without a path looks at.
#[cfg(target_os = "windows")]
const SYSTEM_DISK: &str = "C:\\";
#[cfg(not(target_os = "windows"))]
const SYSTEM_DISK: &str = "/";

/// `sysinfo` reports Windows mount points with a trailing backslash (`C:\`);
/// people often write `C:`. Other paths are unchanged.
fn normalize_disk_target(target: &str) -> String {
    #[cfg(target_os = "windows")]
    if target.len() == 2 && target.ends_with(':') {
        return format!("{target}\\");
    }
    target.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_tcp_check_finds_an_open_port_and_a_closed_one() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let open = tcp("127.0.0.1", port).await;
        assert_eq!(open.status, Health::Up);
        assert!(open.latency_ms.is_some());

        drop(listener);
        assert_eq!(tcp("127.0.0.1", port).await.status, Health::Down);
    }

    /// A web server on 127.0.0.1: `/status/<code>` answers that code,
    /// `/moved` redirects to `/page`, anything else answers 200 with the
    /// request it got as the page. Returns its address.
    async fn web_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    let (status, extra, body) = match path.strip_prefix("/status/") {
                        Some(code) => (code.to_string(), String::new(), String::new()),
                        None if path == "/moved" => (
                            "302".to_string(),
                            "location: /page\r\n".to_string(),
                            String::new(),
                        ),
                        None => ("200".to_string(), String::new(), request.clone()),
                    };
                    let body = if request.starts_with("HEAD") {
                        String::new()
                    } else {
                        body
                    };
                    let response = format!(
                        "HTTP/1.1 {status} X\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        address
    }

    fn options(change: impl FnOnce(&mut HttpOptions)) -> HttpOptions {
        let mut options = HttpOptions::default();
        change(&mut options);
        options
    }

    #[tokio::test]
    async fn an_http_check_is_up_on_the_status_it_expects() {
        let (server, monitor) = (web_server().await, Monitor::new());
        let plain = HttpOptions::default();

        let ok = http(&format!("{server}/page"), &plain, &monitor).await;
        assert_eq!(
            (ok.status, ok.message.as_str()),
            (Health::Up, "HTTP 200 OK")
        );
        assert!(ok.latency_ms.is_some());

        let login = format!("{server}/status/401");
        let refused = http(&login, &plain, &monitor).await;
        assert_eq!(refused.status, Health::Down);
        assert_eq!(refused.message, "HTTP 401 Unauthorized (expected 200-299)");
        let expected = options(|o| o.status = "200, 401".into());
        assert_eq!(http(&login, &expected, &monitor).await.status, Health::Up);
        let only_ok = options(|o| o.status = "401".into());
        let wrong = http(&format!("{server}/page"), &only_ok, &monitor).await;
        assert_eq!(wrong.message, "HTTP 200 OK (expected 401)");

        let head = options(|o| o.method = HttpMethod::Head);
        let headed = http(&format!("{server}/page"), &head, &monitor).await;
        assert_eq!(headed.status, Health::Up, "{}", headed.message);
    }

    #[tokio::test]
    async fn an_http_check_looks_for_text_and_sends_its_headers() {
        let (server, monitor) = (web_server().await, Monitor::new());
        let url = format!("{server}/welcome");

        // The page is the request the server got: its path, and the headers sent.
        let found = options(|o| o.contains = "GET /welcome".into());
        assert_eq!(http(&url, &found, &monitor).await.status, Health::Up);

        let missing = http(&url, &options(|o| o.contains = "Sign in".into()), &monitor).await;
        assert_eq!(missing.status, Health::Down);
        assert_eq!(
            missing.message,
            "HTTP 200 OK, but the page doesn't contain \"Sign in\""
        );
        let unwanted = http(&url, &options(|o| o.absent = "welcome".into()), &monitor).await;
        assert_eq!(
            unwanted.message,
            "HTTP 200 OK, but the page contains \"welcome\""
        );

        let with_header = options(|o| {
            o.contains = "x-site: branch-7".into();
            o.headers = vec![common::checks::HttpHeader {
                name: "X-Site".into(),
                value: "branch-7".into(),
            }];
        });
        let sent = http(&url, &with_header, &monitor).await;
        assert_eq!(sent.status, Health::Up, "{}", sent.message);
    }

    #[tokio::test]
    async fn an_http_check_follows_redirects_unless_told_not_to() {
        let (server, monitor) = (web_server().await, Monitor::new());
        let url = format!("{server}/moved");

        let followed = http(
            &url,
            &options(|o| o.contains = "GET /page".into()),
            &monitor,
        )
        .await;
        assert_eq!(followed.status, Health::Up, "{}", followed.message);

        let stopped = http(&url, &options(|o| o.follow_redirects = false), &monitor).await;
        assert_eq!(stopped.message, "HTTP 302 Found (expected 200-299)");
        let expected = options(|o| {
            o.follow_redirects = false;
            o.status = "301-308".into();
        });
        assert_eq!(http(&url, &expected, &monitor).await.status, Health::Up);
    }

    #[tokio::test]
    async fn an_http_check_trusts_public_authorities_only_unless_told_not_to() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // An HTTPS server with a self-signed certificate.
        let certified = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
        let key =
            rustls::pki_types::PrivateKeyDer::try_from(certified.key_pair.serialize_der()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(crate::tls::provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(socket).await else {
                        return;
                    };
                    let mut request = [0u8; 1024];
                    let _ = tls.read(&mut request).await;
                    let _ = tls
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        let monitor = Monitor::new();

        let refused = http(&url, &HttpOptions::default(), &monitor).await;
        assert_eq!(refused.status, Health::Down);
        assert!(
            refused.message.contains("UnknownIssuer"),
            "the reason is in the message: {}",
            refused.message
        );
        let accepted = http(&url, &options(|o| o.verify = false), &monitor).await;
        assert_eq!(accepted.status, Health::Up, "{}", accepted.message);
    }

    #[test]
    fn machine_checks_report_their_metrics_headline_first() {
        let keys = |o: &Outcome| o.metrics.iter().map(|m| m.key.clone()).collect::<Vec<_>>();
        let memory = memory(&System::new_all());
        assert_eq!(keys(&memory), [metric::USAGE, metric::USED, metric::TOTAL]);

        let disks = Disks::new_with_refreshed_list();
        let system_disk = disk(None, &disks);
        if system_disk.status == Health::Up {
            assert_eq!(
                keys(&system_disk),
                [metric::USAGE, metric::USED, metric::FREE, metric::TOTAL]
            );
        }
        let missing = disk(Some("/no/such/mount"), &disks);
        assert_eq!(missing.status, Health::Down);

        assert_eq!(keys(&uptime()), [metric::UPTIME]);
    }

    #[test]
    fn usage_is_worked_out_from_bytes() {
        let outcome = usage_outcome("Memory", 2 << 30, 8 << 30);
        assert_eq!(outcome.message, "Memory: 2.0 GB / 8.0 GB (25.0%)");
        assert_eq!(outcome.metrics[0].value, 25.0);
    }
}
