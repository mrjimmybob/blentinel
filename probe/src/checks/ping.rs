//! The `ping` check (M5-04, D-80): several ICMP echoes to an address, and
//! what came back: how many were lost, how fast the answers were, and how
//! much that varied (jitter).
//!
//! The host is an IP address, or a name the probe's machine resolves each
//! time (so a device whose address changes is followed).
//!
//! It is up when at least one echo is answered. How much loss is too much is
//! the hub's threshold (proposal 0006 §1.5). With a TCP port to fall back on,
//! a host that answers no echo (many firewalls drop them), or a probe that
//! may not send any, is up when that port accepts a connection.

use std::net::IpAddr;
use std::time::Duration;

use common::checks::{Health, Metric, metric};
use surge_ping::{PingIdentifier, PingSequence, SurgeError};

use super::Outcome;
use crate::monitor::Monitor;

/// How long one echo may take to be answered.
const ECHO_TIMEOUT: Duration = Duration::from_secs(1);
/// The pause between two echoes: a burst is what devices that limit ICMP
/// drop first, and that would read as loss.
const ECHO_GAP: Duration = Duration::from_millis(200);
const PAYLOAD: [u8; 8] = [0; 8];
/// How long resolving a host name may take.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// Send `count` echoes to `host` and say what came back; without any answer,
/// try `tcp_port` if the check has one.
pub async fn check(host: &str, count: u8, tcp_port: Option<u16>, monitor: &Monitor) -> Outcome {
    let ip = match host.parse::<IpAddr>() {
        Ok(ip) => ip,
        Err(_) => match resolve(host, monitor).await {
            Some(ip) => ip,
            None => return Outcome::down(format!("{host} doesn't resolve to an address")),
        },
    };
    let Some(client) = monitor.icmp_client(ip) else {
        let why = "This probe may not send pings (no permission for ICMP)";
        return match tcp_port {
            Some(port) => via_tcp(host, port, why).await,
            None => Outcome::down(format!("{why}: give the check a TCP port to try instead")),
        };
    };
    // An identifier of its own: two checks of one host don't take each
    // other's answers.
    let mut pinger = client.pinger(ip, PingIdentifier(rand::random())).await;
    pinger.timeout(ECHO_TIMEOUT);
    let mut answers = Vec::with_capacity(usize::from(count));
    let mut failure = None;
    for sequence in 0..u16::from(count) {
        if sequence > 0 {
            tokio::time::sleep(ECHO_GAP).await;
        }
        match pinger.ping(PingSequence(sequence), &PAYLOAD).await {
            Ok((_, elapsed)) => answers.push(Some(elapsed)),
            Err(e) => {
                answers.push(None);
                // A timeout is a lost echo; anything else is worth saying.
                if !matches!(e, SurgeError::Timeout { .. }) {
                    failure = Some(e.to_string());
                }
            }
        }
    }
    let mut outcome = summarize(&answers);
    if outcome.status == Health::Down
        && let Some(failure) = failure
    {
        outcome.message = format!("Ping failed: {failure}");
    }
    match (outcome.status, tcp_port) {
        (Health::Down, Some(port)) => via_tcp(host, port, &outcome.message).await,
        _ => outcome,
    }
}

/// The address of a host name as this machine resolves it (its DNS servers,
/// its hosts file): one the probe can send echoes to, if it has several.
async fn resolve(host: &str, monitor: &Monitor) -> Option<IpAddr> {
    let found = tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, 0)))
        .await
        .ok()?
        .ok()?;
    let addresses: Vec<IpAddr> = found.map(|address| address.ip()).collect();
    addresses
        .iter()
        .copied()
        .find(|ip| monitor.icmp_client(*ip).is_some())
        .or_else(|| addresses.first().copied())
}

/// Instead of an echo: is the host there on `port`? `why` says why it came
/// to that.
async fn via_tcp(host: &str, port: u16, why: &str) -> Outcome {
    let tcp = super::tcp(host, port).await;
    match tcp.status {
        Health::Up => Outcome {
            message: format!("{why}; TCP port {port} answers"),
            ..tcp
        },
        Health::Down => Outcome::down(format!("{why}; TCP port {port}: {}", tcp.message)),
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// `0.4 ms`, `12 ms`.
fn ms(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.1} ms")
    } else {
        format!("{value:.0} ms")
    }
}

/// What a run of echoes is worth: each `Some` an answer and how long it
/// took, each `None` an echo that was lost.
fn summarize(answers: &[Option<Duration>]) -> Outcome {
    let sent = answers.len();
    let times: Vec<f64> = answers.iter().flatten().copied().map(millis).collect();
    let lost = sent - times.len();
    let loss = if sent == 0 {
        100.0
    } else {
        lost as f64 / sent as f64 * 100.0
    };
    let mut metrics = vec![Metric::new(metric::LOSS, loss, metric::PERCENT)];
    if times.is_empty() {
        let message = match sent {
            0 | 1 => "No answer".to_string(),
            n => format!("No answer to {n} pings"),
        };
        return Outcome::down(message).with_metrics(metrics);
    }
    let average = times.iter().sum::<f64>() / times.len() as f64;
    let slowest = times.iter().copied().fold(0.0, f64::max);
    metrics.push(Metric::new(
        metric::LATENCY_MAX,
        slowest,
        metric::MILLISECONDS,
    ));
    // Jitter: how much one answer's time differs from the one before, on average.
    let jitter = (times.len() > 1).then(|| {
        times.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (times.len() - 1) as f64
    });
    if let Some(jitter) = jitter {
        metrics.push(Metric::new(metric::JITTER, jitter, metric::MILLISECONDS));
    }
    let message = if sent == 1 {
        format!("Answered in {}", ms(average))
    } else {
        let answered = match lost {
            0 => format!("{sent} of {sent} answered"),
            _ => format!("{} of {sent} answered ({loss:.0}% lost)", times.len()),
        };
        let spread = match jitter {
            Some(jitter) => format!(" (max {}, jitter {})", ms(slowest), ms(jitter)),
            None => String::new(),
        };
        format!("{answered}, {} on average{spread}", ms(average))
    };
    Outcome::up(message)
        .with_latency(Duration::from_secs_f64(average / 1000.0))
        .with_metrics(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(times_ms: &[Option<u64>]) -> Outcome {
        let answers: Vec<_> = times_ms
            .iter()
            .map(|t| t.map(Duration::from_millis))
            .collect();
        summarize(&answers)
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value)
    }

    #[test]
    fn every_echo_answered_is_up_with_average_slowest_and_jitter() {
        let outcome = run(&[Some(10), Some(14), Some(12), Some(20)]);
        assert_eq!(outcome.status, Health::Up);
        assert_eq!(
            outcome.message,
            "4 of 4 answered, 14 ms on average (max 20 ms, jitter 4.7 ms)"
        );
        assert_eq!(outcome.latency_ms, Some(14));
        assert_eq!(outcome.metrics[0].key, metric::LOSS, "loss is the headline");
        assert_eq!(value(&outcome, metric::LOSS), Some(0.0));
        assert_eq!(value(&outcome, metric::LATENCY_MAX), Some(20.0));
        // |14-10| + |12-14| + |20-12| = 14, over 3 steps.
        assert!((value(&outcome, metric::JITTER).unwrap() - 14.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn lost_echoes_are_counted_and_one_answer_is_enough_to_be_up() {
        let outcome = run(&[Some(2), None, Some(4), Some(3)]);
        assert_eq!(outcome.status, Health::Up);
        assert_eq!(
            outcome.message,
            "3 of 4 answered (25% lost), 3.0 ms on average (max 4.0 ms, jitter 1.5 ms)"
        );
        assert_eq!(value(&outcome, metric::LOSS), Some(25.0));

        let one = run(&[None, None, Some(7), None]);
        assert_eq!(one.status, Health::Up);
        assert_eq!(one.message, "1 of 4 answered (75% lost), 7.0 ms on average");
        assert_eq!(value(&one, metric::JITTER), None, "one answer has none");
    }

    #[test]
    fn no_answer_is_down_with_everything_lost() {
        let outcome = run(&[None, None, None, None]);
        assert_eq!(outcome.status, Health::Down);
        assert_eq!(outcome.message, "No answer to 4 pings");
        assert_eq!(value(&outcome, metric::LOSS), Some(100.0));
        assert_eq!(outcome.latency_ms, None);
        assert_eq!(run(&[None]).message, "No answer");
        assert_eq!(run(&[Some(3)]).message, "Answered in 3.0 ms");
    }

    #[tokio::test]
    async fn a_tcp_port_answers_for_a_host_that_answers_no_echo() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let open = via_tcp("127.0.0.1", port, "No answer to 4 pings").await;
        assert_eq!(open.status, Health::Up);
        assert_eq!(
            open.message,
            format!("No answer to 4 pings; TCP port {port} answers")
        );
        assert!(open.latency_ms.is_some());
        assert!(open.metrics.is_empty(), "no loss to speak of");

        drop(listener);
        let closed = via_tcp("127.0.0.1", port, "No answer to 4 pings").await;
        assert_eq!(closed.status, Health::Down);
        assert!(
            closed
                .message
                .starts_with(&format!("No answer to 4 pings; TCP port {port}: ")),
            "{}",
            closed.message
        );
    }

    /// Against this machine: echoes where the system lets the tests send
    /// them, the TCP port where it doesn't. Either way the check is up.
    #[tokio::test]
    async fn this_machine_answers_by_echo_or_by_its_tcp_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let monitor = Monitor::new();

        let outcome = check("127.0.0.1", 2, Some(port), &monitor).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        if monitor.icmp_client("127.0.0.1".parse().unwrap()).is_none() {
            assert!(outcome.message.contains("may not send pings"));
            let alone = check("127.0.0.1", 2, None, &monitor).await;
            assert_eq!(alone.status, Health::Down);
            assert!(alone.message.contains("TCP port to try instead"));
        }
        // A name is resolved first (`.invalid` never resolves), and
        // `localhost` is this machine.
        assert_eq!(
            check("no-such-host.invalid", 1, None, &monitor)
                .await
                .message,
            "no-such-host.invalid doesn't resolve to an address"
        );
        let named = check("localhost", 1, Some(port), &monitor).await;
        assert_eq!(named.status, Health::Up, "{}", named.message);
    }
}
