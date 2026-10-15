//! The `ntp` check (M5-16d, D-96): does a time server answer, and how far
//! is the clock of the probe's machine from its time?
//!
//! One question and one answer of the time protocol (SNTP, RFC 4330) over
//! UDP: the probe notes its own clock when it asks and when the answer
//! comes, the server says when it got the question and when it answered,
//! and from the four times follow the difference of the clocks and how long
//! the answer was on its way. The probe only measures: it never sets a
//! clock.
//!
//! The difference is the check's value, which the hub judges by its limits.
//! A server that doesn't answer, refuses, or says itself that it doesn't
//! know the time is Down.

use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::checks::{DEFAULT_NTP_PORT, Metric, metric};
use tokio::net::UdpSocket;

use super::Outcome;

/// How long the name may take to resolve, and each answer to come. A
/// question that gets none is asked once more: UDP loses packets. The
/// second one goes to the name's next address, if it has several: a pool's
/// name stands for many servers, and one of them may be gone.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
const TRIES: usize = 2;

const PACKET_BYTES: usize = 48;
/// Version 4, mode 3: a client's question.
const QUESTION: u8 = 0x23;
const MODE_SERVER: u8 = 4;
/// "Clock not synchronised", in the two bits for leap seconds.
const LEAP_UNKNOWN: u8 = 3;
/// A stratum from here on is a server that follows no clock.
const STRATUM_UNSYNCHRONISED: u8 = 16;
/// Seconds from 1900, where the protocol's time starts, to 1970.
const TO_UNIX: f64 = 2_208_988_800.0;
/// The protocol's seconds are 32 bits and start again in 2036.
const ERA: f64 = 4_294_967_296.0;

/// What a server's answer says.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reading {
    /// Seconds the machine's clock is ahead of the server's (negative:
    /// behind).
    offset: f64,
    /// Seconds the question and the answer were on their way.
    delay: f64,
    /// How far the server is from a reference clock: 1 is next to one.
    stratum: u8,
}

/// Seconds since 1970 of a time as the protocol writes it (seconds from
/// 1900 and a fraction, 32 bits each). `None`: no time (all zero).
fn seconds(field: &[u8]) -> Option<f64> {
    let whole = u32::from_be_bytes(field.get(..4)?.try_into().ok()?);
    let fraction = u32::from_be_bytes(field.get(4..8)?.try_into().ok()?);
    if whole == 0 && fraction == 0 {
        return None;
    }
    // A first bit of 0 is a time after the 2036 turn: before 1968 there was
    // no such protocol.
    let era = if whole & 0x8000_0000 == 0 { ERA } else { 0.0 };
    Some(f64::from(whole) + era - TO_UNIX + f64::from(fraction) / ERA)
}

/// Read `answer` to the question that carried `asked` as its transmit time,
/// sent at `sent` and answered at `received` by the machine's clock (seconds
/// since 1970). `Err`: why it isn't a time, in words.
fn read(answer: &[u8], asked: &[u8; 8], sent: f64, received: f64) -> Result<Reading, String> {
    if answer.len() < PACKET_BYTES {
        return Err("its answer isn't a time server's".to_string());
    }
    let (leap, mode, stratum) = (answer[0] >> 6, answer[0] & 0b111, answer[1]);
    if mode != MODE_SERVER {
        return Err("its answer isn't a time server's".to_string());
    }
    // The server repeats the time of the question: an answer to another
    // question (a late one, or one made up by someone else) doesn't count.
    if answer[24..32] != asked[..] {
        return Err("its answer isn't to this question".to_string());
    }
    if stratum == 0 {
        // A refusal, with a word of four letters for why.
        let code: String = answer[12..16]
            .iter()
            .filter(|byte| byte.is_ascii_alphanumeric())
            .map(|byte| char::from(*byte))
            .collect();
        return Err(match code.as_str() {
            "RATE" => "it refuses: asked too often (RATE)".to_string(),
            "DENY" | "RSTR" => format!("it refuses this machine ({code})"),
            "" => "it refuses".to_string(),
            code => format!("it refuses ({code})"),
        });
    }
    if leap == LEAP_UNKNOWN || stratum >= STRATUM_UNSYNCHRONISED {
        return Err("it says its own clock isn't set (not synchronised)".to_string());
    }
    let (Some(got), Some(answered)) = (seconds(&answer[32..40]), seconds(&answer[40..48])) else {
        return Err("its answer has no time".to_string());
    };
    Ok(Reading {
        offset: ((sent - got) + (received - answered)) / 2.0,
        delay: ((received - sent) - (answered - got)).max(0.0),
        stratum,
    })
}

/// A span of time for people: `12 ms`, `4.2 s`, `3 min`, `2 h 5 min`,
/// `3 days`.
fn span(seconds: f64) -> String {
    let seconds = seconds.abs();
    if seconds < 0.9995 {
        return format!("{:.0} ms", seconds * 1000.0);
    }
    if seconds < 120.0 {
        return format!("{seconds:.1} s");
    }
    let minutes = (seconds / 60.0).round() as u64;
    match minutes {
        0..120 => format!("{minutes} min"),
        120..2880 if minutes.is_multiple_of(60) => format!("{} h", minutes / 60),
        120..2880 => format!("{} h {} min", minutes / 60, minutes % 60),
        _ => format!("{} days", (seconds / 86_400.0).round() as u64),
    }
}

/// What the check says of a server's answer.
fn describe(reading: &Reading, host: &str) -> Outcome {
    let side = if reading.offset >= 0.0 {
        "ahead of"
    } else {
        "behind"
    };
    Outcome::up(format!(
        "This machine's clock is {} {side} {host} (stratum {})",
        span(reading.offset),
        reading.stratum,
    ))
    .with_latency(Duration::from_secs_f64(reading.delay))
    .with_metrics(vec![
        Metric::new(
            metric::OFFSET,
            (reading.offset.abs() * 1000.0).round() / 1000.0,
            metric::SECONDS,
        ),
        Metric::new(metric::STRATUM, f64::from(reading.stratum), ""),
    ])
}

/// The machine's clock, in seconds since 1970.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or_default()
}

/// What asking one address came to, when it wasn't a time.
#[derive(Debug, PartialEq)]
enum Failure {
    /// Nothing came back in time.
    Silent,
    /// The address can't be reached, or nothing listens there: why.
    Unreachable(String),
    /// A server answered, and its answer is no time: why.
    Answer(String),
}

/// Ask the server at `address` once.
async fn ask_once(address: SocketAddr, timeout: Duration) -> Result<Reading, Failure> {
    let local: SocketAddr = match address {
        SocketAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
        SocketAddr::V6(_) => ([0u16; 8], 0).into(),
    };
    let failed =
        |e: std::io::Error| Failure::Unreachable(format!("the question couldn't be sent ({e})"));
    let socket = UdpSocket::bind(local).await.map_err(failed)?;
    // Only this server's packets are taken.
    socket.connect(address).await.map_err(failed)?;

    // The time in the question is a random number: the server only repeats
    // it, and a guessed answer is told apart by it.
    let asked: [u8; 8] = rand::random();
    let mut question = [0u8; PACKET_BYTES];
    question[0] = QUESTION;
    question[40..48].copy_from_slice(&asked);
    let sent = now();
    socket.send(&question).await.map_err(failed)?;

    let mut answer = [0u8; 512];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let length = match tokio::time::timeout_at(deadline, socket.recv(&mut answer)).await {
            Err(_) => return Err(Failure::Silent),
            // "Port unreachable" comes back as an error of the socket.
            Ok(Err(e)) => {
                return Err(Failure::Unreachable(format!("nothing listens there ({e})")));
            }
            Ok(Ok(length)) => length,
        };
        let received = now();
        match read(&answer[..length], &asked, sent, received) {
            // Another question's answer: this one's may still come.
            Err(why) if why.ends_with("this question") => continue,
            read => return read.map_err(Failure::Answer),
        }
    }
}

/// Ask the time server at `host`.
pub async fn check(host: &str, port: Option<u16>) -> Outcome {
    ask(host, port.unwrap_or(DEFAULT_NTP_PORT), ANSWER_TIMEOUT).await
}

async fn ask(host: &str, port: u16, timeout: Duration) -> Outcome {
    let found = tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, port))).await;
    let addresses: Vec<SocketAddr> = match found {
        Ok(Ok(addresses)) => addresses.collect(),
        Ok(Err(e)) => return Outcome::down(format!("{host} couldn't be resolved: {e}")),
        Err(_) => return Outcome::down(format!("{host} couldn't be resolved in time")),
    };
    if addresses.is_empty() {
        return Outcome::down(format!("{host} has no address"));
    }
    ask_each(&addresses, host, timeout).await
}

/// Ask `addresses` in turn, [`TRIES`] questions in all, until one tells
/// the time.
async fn ask_each(addresses: &[SocketAddr], host: &str, timeout: Duration) -> Outcome {
    let mut unreachable = None;
    let mut silent = false;
    for address in addresses.iter().cycle().take(TRIES) {
        match ask_once(*address, timeout).await {
            Ok(reading) => return describe(&reading, host),
            // A server spoke: what it said stands.
            Err(Failure::Answer(why)) => {
                return Outcome::down(format!("No time from {host}: {why}"));
            }
            Err(Failure::Silent) => silent = true,
            Err(Failure::Unreachable(why)) => unreachable = Some(why),
        }
    }
    match unreachable {
        Some(why) if !silent => Outcome::down(format!("No time from {host}: {why}")),
        _ => Outcome::down(format!(
            "No answer from {host} within {} s, asked {TRIES} times (UDP port {})",
            timeout.as_secs_f32(),
            addresses[0].port(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    /// A time as the protocol writes it.
    fn stamp(unix: f64) -> [u8; 8] {
        let ntp = unix + TO_UNIX;
        let whole = (ntp.floor() % ERA) as u32;
        let fraction = ((ntp - ntp.floor()) * ERA) as u32;
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&whole.to_be_bytes());
        bytes[4..].copy_from_slice(&fraction.to_be_bytes());
        bytes
    }

    const ASKED: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

    /// A server's answer to `asked`: it got the question at `got` and
    /// answered at `answered`.
    fn answer(asked: &[u8; 8], stratum: u8, got: f64, answered: f64) -> [u8; PACKET_BYTES] {
        let mut packet = [0u8; PACKET_BYTES];
        packet[0] = 0x24;
        packet[1] = stratum;
        packet[24..32].copy_from_slice(asked);
        packet[32..40].copy_from_slice(&stamp(got));
        packet[40..48].copy_from_slice(&stamp(answered));
        packet
    }

    /// 2026-10-13 12:00:00 UTC.
    const NOON: f64 = 1_791_892_800.0;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_four_times_give_the_difference_and_the_way() {
        // The machine is 2.5 s ahead; 20 ms each way, 1 ms at the server.
        let (sent, received) = (NOON + 2.5, NOON + 2.5 + 0.041);
        let packet = answer(&ASKED, 2, NOON + 0.020, NOON + 0.021);
        let reading = read(&packet, &ASKED, sent, received).unwrap();
        assert!(close(reading.offset, 2.5), "{reading:?}");
        assert!(close(reading.delay, 0.040), "{reading:?}");
        assert_eq!(reading.stratum, 2);

        // Behind: negative.
        let packet = answer(&ASKED, 1, NOON + 300.0, NOON + 300.0);
        let reading = read(&packet, &ASKED, NOON, NOON).unwrap();
        assert!(close(reading.offset, -300.0), "{reading:?}");
        assert!(close(reading.delay, 0.0));
    }

    #[test]
    fn times_are_read_across_the_turn_of_2036() {
        assert!(close(seconds(&stamp(NOON)).unwrap(), NOON));
        assert!(close(seconds(&stamp(NOON + 0.25)).unwrap(), NOON + 0.25));
        // 2040-01-01: the protocol's seconds have started again.
        let later = 2_208_988_800.0;
        assert!(stamp(later)[0] & 0x80 == 0);
        assert!(close(seconds(&stamp(later)).unwrap(), later));
        assert_eq!(seconds(&[0; 8]), None);
        assert_eq!(seconds(&[0; 4]), None);
    }

    #[test]
    fn an_answer_that_is_no_time_says_why() {
        let good = answer(&ASKED, 2, NOON, NOON);
        let why = |packet: &[u8]| read(packet, &ASKED, NOON, NOON).unwrap_err();

        assert_eq!(why(&good[..40]), "its answer isn't a time server's");
        // A client's packet, not a server's.
        let mut question = good;
        question[0] = QUESTION;
        assert_eq!(why(&question), "its answer isn't a time server's");
        // To another question.
        let other = answer(&[9; 8], 2, NOON, NOON);
        assert_eq!(why(&other), "its answer isn't to this question");

        // Refusals: stratum 0 and a word.
        let refusal = |code: &[u8; 4]| {
            let mut packet = answer(&ASKED, 0, NOON, NOON);
            packet[12..16].copy_from_slice(code);
            why(&packet)
        };
        assert_eq!(refusal(b"RATE"), "it refuses: asked too often (RATE)");
        assert_eq!(refusal(b"DENY"), "it refuses this machine (DENY)");
        assert_eq!(refusal(b"INIT"), "it refuses (INIT)");
        assert_eq!(refusal(&[0; 4]), "it refuses");

        // A server that follows no clock.
        let lost = answer(&ASKED, 16, NOON, NOON);
        assert!(why(&lost).contains("not synchronised"));
        let mut unknown = good;
        unknown[0] |= 0xC0;
        assert!(why(&unknown).contains("not synchronised"));

        let mut timeless = good;
        timeless[40..48].fill(0);
        assert_eq!(why(&timeless), "its answer has no time");
    }

    #[test]
    fn a_difference_is_said_in_the_unit_that_fits() {
        for (seconds, text) in [
            (0.0004, "0 ms"),
            (0.012, "12 ms"),
            (-0.75, "750 ms"),
            (0.9996, "1.0 s"),
            (4.24, "4.2 s"),
            (119.0, "119.0 s"),
            (180.0, "3 min"),
            (-7200.0, "2 h"),
            (7500.0, "2 h 5 min"),
            (86_400.0 * 3.2, "3 days"),
        ] {
            assert_eq!(span(seconds), text, "{seconds}");
        }
    }

    #[test]
    fn the_message_says_which_way_and_the_value_is_the_distance() {
        let ahead = describe(
            &Reading {
                offset: 0.0123,
                delay: 0.018,
                stratum: 3,
            },
            "time.example.com",
        );
        assert_eq!(ahead.status, Health::Up);
        assert_eq!(
            ahead.message,
            "This machine's clock is 12 ms ahead of time.example.com (stratum 3)"
        );
        assert_eq!(ahead.latency_ms, Some(18));
        assert_eq!(
            ahead.metrics,
            [
                Metric::new(metric::OFFSET, 0.012, metric::SECONDS),
                Metric::new(metric::STRATUM, 3.0, ""),
            ]
        );

        let behind = describe(
            &Reading {
                offset: -7500.0,
                delay: 0.0,
                stratum: 1,
            },
            "dc01",
        );
        assert_eq!(
            behind.message,
            "This machine's clock is 2 h 5 min behind dc01 (stratum 1)"
        );
        assert_eq!(
            behind.metrics[0].value, 7500.0,
            "the distance, not the side"
        );
    }

    /// A time server on a local port that answers every question with
    /// `reply(question)`; `None`: it says nothing.
    async fn server(reply: impl Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static) -> u16 {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut question = [0u8; 512];
            while let Ok((length, from)) = socket.recv_from(&mut question).await {
                if let Some(answer) = reply(&question[..length]) {
                    let _ = socket.send_to(&answer, from).await;
                }
            }
        });
        port
    }

    fn asked_in(question: &[u8]) -> [u8; 8] {
        question[40..48].try_into().unwrap()
    }

    async fn asked(port: u16) -> Outcome {
        ask("127.0.0.1", port, Duration::from_millis(400)).await
    }

    #[tokio::test]
    async fn a_server_that_answers_is_up_with_the_clocks_difference() {
        // A server whose clock is a minute ahead of this machine's.
        let port = server(|question| {
            assert_eq!(question.len(), PACKET_BYTES);
            assert_eq!(question[0], QUESTION);
            let time = now() + 60.0;
            Some(answer(&asked_in(question), 2, time, time).to_vec())
        })
        .await;
        let outcome = asked(port).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            "This machine's clock is 60.0 s behind 127.0.0.1 (stratum 2)"
        );
        assert!((outcome.metrics[0].value - 60.0).abs() < 0.5);
        assert!(outcome.latency_ms.is_some());
    }

    #[tokio::test]
    async fn a_silent_refusing_or_lost_server_is_down_with_the_reason() {
        let silent = asked(server(|_| None).await).await;
        assert_eq!(silent.status, Health::Down);
        assert!(
            silent
                .message
                .starts_with("No answer from 127.0.0.1 within 0.4 s, asked 2 times (UDP port "),
            "{}",
            silent.message
        );

        let refusing = server(|question| {
            let mut packet = answer(&asked_in(question), 0, NOON, NOON);
            packet[12..16].copy_from_slice(b"RATE");
            Some(packet.to_vec())
        })
        .await;
        assert_eq!(
            asked(refusing).await.message,
            "No time from 127.0.0.1: it refuses: asked too often (RATE)"
        );

        let lost = server(|question| Some(answer(&asked_in(question), 16, NOON, NOON).to_vec()));
        let lost = asked(lost.await).await;
        assert_eq!(lost.status, Health::Down);
        assert!(
            lost.message.contains("not synchronised"),
            "{}",
            lost.message
        );

        // Something else answers there.
        let other = asked(server(|_| Some(b"hello".to_vec())).await).await;
        assert_eq!(
            other.message,
            "No time from 127.0.0.1: its answer isn't a time server's"
        );

        let nowhere = ask("no-such-host.invalid", 123, Duration::from_millis(400)).await;
        assert_eq!(nowhere.status, Health::Down);
        assert!(
            nowhere.message.starts_with("no-such-host.invalid "),
            "{}",
            nowhere.message
        );
    }

    #[tokio::test]
    async fn an_answer_to_another_question_is_passed_over_and_a_lost_one_asked_again() {
        // Someone's made-up answer first, then the real one.
        let port = server(|_| Some(answer(&[9; 8], 2, NOON, NOON).to_vec())).await;
        let forged = asked(port).await;
        assert_eq!(forged.status, Health::Down, "{}", forged.message);
        assert!(forged.message.starts_with("No answer from"));

        // The first question is lost; the second is answered.
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = std::sync::Arc::clone(&seen);
        let port = server(move |question| {
            let first = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            let time = now();
            (!first).then(|| answer(&asked_in(question), 3, time, time).to_vec())
        })
        .await;
        let outcome = asked(port).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_second_question_goes_to_the_names_next_address() {
        // A pool's name: the first server is gone, the second tells the time.
        let dead = server(|_| None).await;
        let alive = server(|question| {
            let time = now();
            Some(answer(&asked_in(question), 2, time, time).to_vec())
        })
        .await;
        let at = |port: u16| SocketAddr::from(([127, 0, 0, 1], port));
        let timeout = Duration::from_millis(300);
        let outcome = ask_each(&[at(dead), at(alive)], "pool.example.org", timeout).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert!(outcome.message.ends_with("pool.example.org (stratum 2)"));

        // Only so many questions, however many addresses.
        let none = ask_each(
            &[at(dead), at(dead), at(alive)],
            "pool.example.org",
            timeout,
        )
        .await;
        assert_eq!(none.status, Health::Down);
        assert!(
            none.message
                .starts_with("No answer from pool.example.org within 0.3 s")
        );

        // A server that speaks is believed: the next one isn't asked.
        let refusing = server(|question| {
            let mut packet = answer(&asked_in(question), 0, NOON, NOON);
            packet[12..16].copy_from_slice(b"DENY");
            Some(packet.to_vec())
        })
        .await;
        let refused = ask_each(&[at(refusing), at(alive)], "pool.example.org", timeout).await;
        assert_eq!(
            refused.message,
            "No time from pool.example.org: it refuses this machine (DENY)"
        );
    }

    /// By hand (`--ignored --nocapture`): public time servers, asked.
    #[tokio::test]
    #[ignore = "asks time servers on the internet: run by hand"]
    async fn real_time_servers() {
        for host in [
            "time.cloudflare.com",
            "pool.ntp.org",
            "time.windows.com",
            "time.google.com",
            "ptbtime1.ptb.de",
        ] {
            let outcome = check(host, None).await;
            println!(
                "{host}: {:?} in {:?} ms: {} {:?}",
                outcome.status, outcome.latency_ms, outcome.message, outcome.metrics
            );
        }
    }
}
