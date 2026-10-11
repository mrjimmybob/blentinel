//! The `dns` check (M5-07, D-82): does a name resolve, from this network,
//! and to what it should?
//!
//! The probe asks a DNS server itself (hickory-resolver, pure Rust: FP-2):
//! the one the check names, or the ones this machine uses. Every run is a
//! real question: nothing is cached, and the machine's hosts file isn't
//! consulted. Up when there is an answer of the record type asked for and,
//! if the check lists what to expect, every answer is on that list.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use common::checks::{DnsRecord, Metric, metric};
use hickory_resolver::config::{NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts};
use hickory_resolver::net::NetError;
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::{Resolver, TokioResolver};

use super::Outcome;

/// How long one server has to answer, and how often it is asked.
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const QUERY_ATTEMPTS: usize = 2;
/// How many answers the message names.
const ANSWERS_NAMED: usize = 5;

/// Ask for `name`'s `record`s (`server`: an IP address, else this machine's
/// DNS servers) and judge the answer against `expect`.
pub async fn check(
    name: &str,
    record: DnsRecord,
    server: Option<&str>,
    expect: &[String],
) -> Outcome {
    let server = match server.map(str::parse::<IpAddr>) {
        Some(Ok(ip)) => Some(NameServerConfig::udp_and_tcp(ip)),
        Some(Err(_)) => return Outcome::down("The DNS server isn't an IP address"),
        None => None,
    };
    ask(name, record, server, expect).await
}

async fn ask(
    name: &str,
    record: DnsRecord,
    server: Option<NameServerConfig>,
    expect: &[String],
) -> Outcome {
    let resolver = match resolver(server) {
        Ok(resolver) => resolver,
        Err(e) => return Outcome::down(format!("DNS isn't set up on this machine: {e}")),
    };
    // A full name: this machine's search domains aren't tried after it.
    let full = format!("{}.", name.trim_end_matches('.'));
    let start = Instant::now();
    let asked = resolver.lookup(full.as_str(), record_type(record)).await;
    let elapsed = start.elapsed();
    match asked {
        Ok(lookup) => {
            let answers = lookup
                .answers()
                .iter()
                .filter(|r| r.record_type() == record_type(record))
                .map(|r| r.data.to_string())
                .collect();
            judge(answers, record, expect).with_latency(elapsed)
        }
        Err(e) if e.is_nx_domain() => Outcome::down(format!("{name} doesn't exist (NXDOMAIN)")),
        Err(e) if e.is_no_records_found() => {
            Outcome::down(format!("{name} has no {} record", record.label()))
        }
        Err(e) => Outcome::down(format!("No answer: {e}")),
    }
}

/// A resolver that asks `server`, or this machine's servers; never from a
/// cache or the hosts file.
fn resolver(server: Option<NameServerConfig>) -> Result<TokioResolver, NetError> {
    let mut builder = match server {
        Some(server) => Resolver::builder_with_config(
            ResolverConfig::from_name_servers(vec![server]),
            TokioRuntimeProvider::default(),
        ),
        None => TokioResolver::builder_tokio()?,
    };
    let options: &mut ResolverOpts = builder.options_mut();
    options.timeout = QUERY_TIMEOUT;
    options.attempts = QUERY_ATTEMPTS;
    options.cache_size = 0;
    options.use_hosts_file = ResolveHosts::Never;
    builder.build()
}

fn record_type(record: DnsRecord) -> RecordType {
    match record {
        DnsRecord::A => RecordType::A,
        DnsRecord::Aaaa => RecordType::AAAA,
        DnsRecord::Cname => RecordType::CNAME,
        DnsRecord::Mx => RecordType::MX,
        DnsRecord::Txt => RecordType::TXT,
        DnsRecord::Ns => RecordType::NS,
    }
}

/// An answer or an expected value, for comparing: names end with or without
/// a dot and in any letter case, and text records may be quoted.
fn normal(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_end_matches('.')
        .to_lowercase()
}

/// What the answers are worth: Down without any, or when one isn't among
/// `expect` (if the check expects anything); else Up.
fn judge(mut answers: Vec<String>, record: DnsRecord, expect: &[String]) -> Outcome {
    answers.sort();
    answers.dedup();
    if answers.is_empty() {
        return Outcome::down(format!("No {} record in the answer", record.label()));
    }
    let metrics = vec![Metric::new(metric::ANSWERS, answers.len() as f64, "")];
    let mut named: Vec<&str> = answers
        .iter()
        .take(ANSWERS_NAMED)
        .map(String::as_str)
        .collect();
    let more = format!("and {} more", answers.len().saturating_sub(ANSWERS_NAMED));
    if answers.len() > ANSWERS_NAMED {
        named.push(&more);
    }
    let listed = format!("{}: {}", record.label(), named.join(", "));

    let expected: Vec<String> = expect.iter().map(|e| normal(e)).collect();
    let unexpected: Vec<&str> = answers
        .iter()
        .filter(|a| !expected.is_empty() && !expected.contains(&normal(a)))
        .map(String::as_str)
        .collect();
    if unexpected.is_empty() {
        Outcome::up(listed).with_metrics(metrics)
    } else {
        Outcome::down(format!(
            "{listed}; not expected: {} (expected {})",
            unexpected.join(", "),
            expect.join(", ")
        ))
        .with_metrics(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;
    use hickory_resolver::config::ConnectionConfig;
    use tokio::net::UdpSocket;

    fn answers(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn answers_are_up_unless_one_is_not_expected() {
        let any = judge(answers(&["192.0.2.20", "192.0.2.10"]), DnsRecord::A, &[]);
        assert_eq!(any.status, Health::Up);
        assert_eq!(any.message, "A: 192.0.2.10, 192.0.2.20", "sorted");
        assert_eq!(any.metrics[0].value, 2.0);

        // Any of the expected addresses will do (a name served in turns).
        let expect = answers(&["192.0.2.10", "192.0.2.20", "192.0.2.30"]);
        let fine = judge(answers(&["192.0.2.20"]), DnsRecord::A, &expect);
        assert_eq!(fine.status, Health::Up);

        // One that isn't on the list: someone changed, or hijacked, the name.
        let hijacked = judge(
            answers(&["192.0.2.10", "203.0.113.66"]),
            DnsRecord::A,
            &expect,
        );
        assert_eq!(hijacked.status, Health::Down);
        assert_eq!(
            hijacked.message,
            "A: 192.0.2.10, 203.0.113.66; not expected: 203.0.113.66 \
             (expected 192.0.2.10, 192.0.2.20, 192.0.2.30)"
        );

        let none = judge(vec![], DnsRecord::Mx, &[]);
        assert_eq!(
            (none.status, none.message.as_str()),
            (Health::Down, "No MX record in the answer")
        );
        let many = judge(
            (0..8).map(|i| format!("192.0.2.{i}")).collect(),
            DnsRecord::A,
            &[],
        );
        assert!(many.message.ends_with(", and 3 more"), "{}", many.message);
    }

    #[test]
    fn names_compare_whatever_their_dot_case_or_quotes() {
        let mx = answers(&["10 Mail.Example.COM."]);
        assert_eq!(
            judge(mx, DnsRecord::Mx, &answers(&["10 mail.example.com"])).status,
            Health::Up
        );
        let txt = answers(&["\"v=spf1 -all\""]);
        assert_eq!(
            judge(txt, DnsRecord::Txt, &answers(&["v=spf1 -all"])).status,
            Health::Up
        );
    }

    /// A DNS server on 127.0.0.1 that answers every A question with
    /// 192.0.2.44, except names starting with `gone`, which don't exist.
    /// Returns its port.
    async fn dns_server() -> u16 {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buffer = [0u8; 512];
            while let Ok((length, from)) = socket.recv_from(&mut buffer).await {
                let question = &buffer[..length];
                // The header and the question as they came: the name (up to
                // its empty label), its type and class. What follows (the
                // client's EDNS record) isn't repeated.
                let name_end = 12 + question[12..].iter().position(|b| *b == 0).unwrap_or(0);
                let mut answer = question[..(name_end + 5).min(length)].to_vec();
                let gone = question[12..].starts_with(b"\x04gone");
                answer[2] = 0x81; // an answer, recursion desired
                answer[3] = if gone { 0x83 } else { 0x80 }; // NXDOMAIN, or fine
                answer[6..12].copy_from_slice(&[0, u8::from(!gone), 0, 0, 0, 0]);
                if !gone {
                    // The name (a pointer to the question's), A, IN, 60 s, 4 bytes.
                    answer.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
                    answer.extend_from_slice(&[192, 0, 2, 44]);
                }
                let _ = socket.send_to(&answer, from).await;
            }
        });
        port
    }

    fn server(port: u16) -> Option<NameServerConfig> {
        let mut udp = ConnectionConfig::udp();
        udp.port = port;
        Some(NameServerConfig::new(
            "127.0.0.1".parse().unwrap(),
            true,
            vec![udp],
        ))
    }

    #[tokio::test]
    async fn a_name_is_asked_of_the_server_and_judged() {
        let port = dns_server().await;

        let found = ask("intranet.example.lan", DnsRecord::A, server(port), &[]).await;
        assert_eq!(found.status, Health::Up, "{}", found.message);
        assert_eq!(found.message, "A: 192.0.2.44");
        assert!(found.latency_ms.is_some());

        let moved = ask(
            "intranet.example.lan",
            DnsRecord::A,
            server(port),
            &answers(&["192.0.2.1"]),
        )
        .await;
        assert_eq!(moved.status, Health::Down);
        assert!(moved.message.contains("not expected: 192.0.2.44"));

        let gone = ask("gone.example.lan", DnsRecord::A, server(port), &[]).await;
        assert_eq!(gone.status, Health::Down);
        assert_eq!(gone.message, "gone.example.lan doesn't exist (NXDOMAIN)");
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_down() {
        // A port nobody listens on.
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let outcome = ask("example.lan", DnsRecord::A, server(port), &[]).await;
        assert_eq!(outcome.status, Health::Down);
        assert!(
            outcome.message.starts_with("No answer"),
            "{}",
            outcome.message
        );

        let bad = check("example.lan", DnsRecord::A, Some("dns.lan"), &[]).await;
        assert_eq!(bad.message, "The DNS server isn't an IP address");
    }
}
