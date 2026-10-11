//! The checks the probe runs, one per [`CheckSpec`] kind (D-49).
//!
//! A check only measures: it says whether it succeeded, with a message and
//! its metrics. Thresholds are applied on the hub (proposal 0006 §1.5).
//! [`run`] adds the check's ID and a time limit, so one slow check can't
//! hold up the report.

mod certificate;

pub use certificate::{Trust, public_trust};

use crate::monitor::Monitor;
use common::checks::{CheckDefinition, CheckResult, CheckSpec, Health, Metric, metric};
use std::net::IpAddr;
use std::time::{Duration, Instant};
use surge_ping::{PingIdentifier, PingSequence};
use sysinfo::{Disks, System};
use tokio::net::TcpStream;

/// The longest any check may take (its own timeouts are shorter).
pub const CHECK_TIME_LIMIT: Duration = Duration::from_secs(30);

/// How long a TCP connection may take to open.
const TCP_TIMEOUT: Duration = Duration::from_secs(3);

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
        CheckSpec::Http { url } => http(url, monitor).await,
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

async fn http(url: &str, monitor: &Monitor) -> Outcome {
    let start = Instant::now();
    match monitor.http_client.get(url).send().await {
        Ok(response) if response.status().is_success() => {
            Outcome::up(format!("HTTP {}", response.status())).with_latency(start.elapsed())
        }
        Ok(response) => Outcome::down(format!("HTTP Error: {}", response.status())),
        Err(e) => Outcome::down(format!("Connection failed: {e}")),
    }
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
