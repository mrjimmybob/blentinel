//! The `nut` check (M5-16e, D-97): what a UPS says of itself, asked of the
//! server that watches it (Network UPS Tools' `upsd`, TCP 3493).
//!
//! A UPS hangs on a machine by USB or a serial line, and that machine's NUT
//! server tells the network about it: a NAS (Synology, QNAP), a Linux
//! server, a router. The probe asks it for the UPS's values, in the
//! server's own plain protocol, and says goodbye. It reads only: it logs in
//! nowhere, and never tells a UPS to do anything.
//!
//! The UPS's own word is the verdict. Battery low, a shutdown under way or
//! its output off is Down. Running on battery, a battery to replace, an
//! overload and the like are counted for the hub, which makes a Warning of
//! them (the probe says Up or Down only).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use common::checks::{DEFAULT_NUT_PORT, Metric, metric};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use super::database::said;
use super::lines::Wire;
use super::{Outcome, TCP_TIMEOUT};

/// How long the server has for the whole exchange once the port is open.
const TALK_TIMEOUT: Duration = Duration::from_secs(8);
/// How long it has to answer the goodbye: a server that doesn't is still up.
const BYE_TIMEOUT: Duration = Duration::from_secs(1);
/// The most lines of one list the probe reads (a UPS has some 30 to 80
/// values).
const MAX_LINES: usize = 1024;
/// How many UPS names a message lists.
const NAMED: usize = 8;

/// Why a server didn't tell of a UPS.
#[derive(Debug, PartialEq)]
enum Trouble {
    /// No answer of that protocol (something else listens there, or it hung
    /// up).
    Silent(String),
    /// It is a NUT server, and can't tell of this UPS: why, for people.
    Says(String),
}

impl From<String> for Trouble {
    fn from(why: String) -> Self {
        Trouble::Silent(why)
    }
}

/// The words of a line of the protocol: separated by spaces, with text in
/// double quotes as one word (`VAR ups ups.model "Back-UPS ES 700G"`), in
/// which `\"` and `\\` stand for the character.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' if quoted => word.extend(characters.next()),
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            other => {
                word.push(other);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// What the server's refusal (`ERR CODE`) means for the UPS called `ups`.
fn refusal(code: &str, ups: &str) -> String {
    match code {
        "UNKNOWN-UPS" => format!("the server has no UPS called {ups}"),
        "DATA-STALE" => {
            "the server has lost contact with the UPS (its values are stale): a cable, or its \
             driver"
                .to_string()
        }
        "DRIVER-NOT-CONNECTED" => "the UPS's driver isn't running on the server".to_string(),
        "ACCESS-DENIED" => {
            "the server refuses this machine (ACCESS-DENIED): allow the probe's address in its \
             settings"
                .to_string()
        }
        "" => "the server refused".to_string(),
        code => format!("the server refused ({})", said(code)),
    }
}

/// The lines of the list asked for with `LIST {what}`, each as its words.
async fn list<S: AsyncRead + AsyncWrite + Unpin>(
    wire: &mut Wire<S>,
    what: &str,
    ups: &str,
) -> Result<Vec<Vec<String>>, Trouble> {
    let not_nut = || Trouble::Silent("its answer isn't a UPS server's (NUT)".to_string());
    wire.send(&format!("LIST {what}")).await?;
    let first = words(&wire.line().await?);
    match first.first().map(String::as_str) {
        Some("BEGIN") => {}
        Some("ERR") => {
            let code = first.get(1).map(String::as_str).unwrap_or_default();
            return Err(Trouble::Says(refusal(code, ups)));
        }
        _ => return Err(not_nut()),
    }
    let mut lines = Vec::new();
    for _ in 0..MAX_LINES {
        let line = words(&wire.line().await?);
        match line.first().map(String::as_str) {
            Some("END") => return Ok(lines),
            Some("ERR") => {
                let code = line.get(1).map(String::as_str).unwrap_or_default();
                return Err(Trouble::Says(refusal(code, ups)));
            }
            _ => lines.push(line),
        }
    }
    Err(not_nut())
}

/// The whole exchange: the values of the UPS called `ups`, or of the only
/// one the server has if `ups` is empty; with its name.
async fn talk<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    ups: &str,
) -> Result<(String, BTreeMap<String, String>), Trouble> {
    let mut wire = Wire::new(stream);
    let name = if ups.is_empty() {
        let mut names: Vec<String> = list(&mut wire, "UPS", ups)
            .await?
            .into_iter()
            .filter(|line| line.first().is_some_and(|word| word == "UPS"))
            .filter_map(|line| line.into_iter().nth(1))
            .collect();
        match names.len() {
            0 => return Err(Trouble::Says("the server knows no UPS".to_string())),
            1 => names.remove(0),
            count => {
                names.truncate(NAMED);
                return Err(Trouble::Says(format!(
                    "the server has {count} UPSes ({}): name one in the check",
                    names.join(", ")
                )));
            }
        }
    } else {
        ups.to_string()
    };
    let values = list(&mut wire, &format!("VAR {name}"), &name)
        .await?
        .into_iter()
        .filter(|line| line.len() >= 4 && line[0] == "VAR")
        .map(|mut line| {
            let value = line.swap_remove(3);
            (line.swap_remove(2), value)
        })
        .collect();
    // So the server's log has no connection that just ended.
    let _ = tokio::time::timeout(BYE_TIMEOUT, async {
        wire.send("LOGOUT").await?;
        wire.line().await
    })
    .await;
    Ok((name, values))
}

/// A number among a UPS's values.
fn number(values: &BTreeMap<String, String>, key: &str) -> Option<f64> {
    values
        .get(key)?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

/// What the check says of the UPS called `name` with these `values`.
fn describe(name: &str, values: &BTreeMap<String, String>) -> Outcome {
    let label = ["ups.model", "device.model"]
        .into_iter()
        .filter_map(|key| values.get(key))
        .map(|model| said(model))
        .find(|model| !model.is_empty())
        .unwrap_or_else(|| name.to_string());
    let Some(status) = values.get("ups.status").filter(|s| !s.trim().is_empty()) else {
        return Outcome::down(format!("{label}: the UPS reports no status"));
    };
    let has = |flag: &str| status.split_whitespace().any(|word| word == flag);

    // What the UPS says is wrong: its own alarm for "act now", then the
    // signs that come before.
    let failing: Vec<&str> = [
        ("LB", "battery low"),
        ("FSD", "shutting down what it powers"),
        ("OFF", "its output is off"),
    ]
    .into_iter()
    .filter_map(|(flag, words)| has(flag).then_some(words))
    .collect();
    let signs: Vec<&str> = [
        ("OB", "on battery"),
        ("RB", "its battery must be replaced"),
        ("OVER", "overloaded"),
        ("BYPASS", "on bypass (what it powers isn't protected)"),
        ("ALARM", "it reports an alarm"),
    ]
    .into_iter()
    .filter_map(|(flag, words)| has(flag).then_some(words))
    .collect();

    let charge = number(values, "battery.charge");
    // Seconds, by the protocol.
    let runtime = number(values, "battery.runtime").map(|seconds| seconds / 60.0);
    let load = number(values, "ups.load");

    let mut said_of_it: Vec<String> = signs
        .iter()
        .chain(&failing)
        .map(|words| words.to_string())
        .collect();
    if said_of_it.is_empty() {
        said_of_it.push(
            match (has("OL"), has("CHRG")) {
                (true, true) => "on mains power, charging",
                (true, false) => "on mains power",
                (false, _) => "no trouble reported",
            }
            .to_string(),
        );
    }
    if let Some(charge) = charge {
        said_of_it.push(format!("battery {charge:.0} %"));
    }
    if let Some(minutes) = runtime {
        said_of_it.push(format!("{minutes:.0} min of runtime"));
    }
    if let Some(load) = load {
        said_of_it.push(format!("load {load:.0} %"));
    }
    let message = format!("{label}: {}", said_of_it.join(", "));

    let mut metrics = Vec::new();
    if let Some(charge) = charge {
        metrics.push(Metric::new(metric::CHARGE, charge, metric::PERCENT));
    }
    if let Some(minutes) = runtime {
        metrics.push(Metric::new(
            metric::RUNTIME,
            (minutes * 10.0).round() / 10.0,
            metric::MINUTES,
        ));
    }
    if let Some(load) = load {
        metrics.push(Metric::new(metric::LOAD, load, metric::PERCENT));
    }
    metrics.push(Metric::new(metric::WARNINGS, signs.len() as f64, ""));

    if failing.is_empty() {
        Outcome::up(message)
    } else {
        Outcome::down(message)
    }
    .with_metrics(metrics)
}

/// Ask the UPS server at `host` about `ups` (empty: its only one).
pub async fn check(host: &str, port: Option<u16>, ups: &str) -> Outcome {
    ask(host, port.unwrap_or(DEFAULT_NUT_PORT), ups, TALK_TIMEOUT).await
}

async fn ask(host: &str, port: u16, ups: &str, timeout: Duration) -> Outcome {
    let start = Instant::now();
    let stream = match tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Outcome::down(format!("Connection failed: {e}")),
        Err(_) => return Outcome::down("Connection timeout"),
    };
    match tokio::time::timeout(timeout, talk(stream, ups)).await {
        Ok(Ok((name, values))) => describe(&name, &values).with_latency(start.elapsed()),
        Ok(Err(Trouble::Says(why))) => Outcome::down(format!("No word of the UPS: {why}")),
        Ok(Err(Trouble::Silent(why))) => Outcome::down(format!(
            "The port is open, but a UPS server (NUT) didn't answer: {why}"
        )),
        Err(_) => Outcome::down(format!(
            "The port is open, but a UPS server (NUT) didn't answer within {} s",
            timeout.as_secs_f32()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// An APC Back-UPS as its USB driver shows it, with this status.
    fn back_ups(status: &str) -> BTreeMap<String, String> {
        values(&[
            ("battery.charge", "100"),
            ("battery.runtime", "2520"),
            ("battery.voltage", "13.6"),
            ("device.mfr", "American Power Conversion"),
            ("device.model", "Back-UPS ES 700G"),
            ("input.voltage", "232.0"),
            ("ups.load", "23"),
            ("ups.model", "Back-UPS ES 700G"),
            ("ups.status", status),
        ])
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|metric| metric.key == key)
            .map(|metric| metric.value)
    }

    #[test]
    fn a_line_is_split_into_words_and_quoted_text() {
        assert_eq!(
            words(r#"VAR ups ups.model "Back-UPS ES 700G""#),
            ["VAR", "ups", "ups.model", "Back-UPS ES 700G"]
        );
        assert_eq!(
            words(r#"UPS rack "The \"big\" one, C:\\ups" "#),
            ["UPS", "rack", r#"The "big" one, C:\ups"#]
        );
        // An empty value is a word; spaces between words don't make any.
        assert_eq!(
            words(r#"VAR ups  ups.test.result """#),
            ["VAR", "ups", "ups.test.result", ""]
        );
        assert_eq!(words("END LIST VAR ups"), ["END", "LIST", "VAR", "ups"]);
        assert_eq!(words(""), Vec::<String>::new());
        assert_eq!(
            words(r#"VAR ups x "never closed"#),
            ["VAR", "ups", "x", "never closed"]
        );
    }

    #[test]
    fn a_ups_on_mains_power_is_up_with_its_numbers() {
        let online = describe("ups", &back_ups("OL"));
        assert_eq!(online.status, Health::Up);
        assert_eq!(
            online.message,
            "Back-UPS ES 700G: on mains power, battery 100 %, 42 min of runtime, load 23 %"
        );
        assert_eq!(value(&online, metric::CHARGE), Some(100.0));
        assert_eq!(value(&online, metric::RUNTIME), Some(42.0));
        assert_eq!(value(&online, metric::LOAD), Some(23.0));
        assert_eq!(value(&online, metric::WARNINGS), Some(0.0));

        let charging = describe("ups", &back_ups("OL CHRG"));
        assert!(
            charging
                .message
                .contains(": on mains power, charging, battery")
        );

        // A UPS that says little: its name, and what it does say.
        let bare = describe("rack", &values(&[("ups.status", "OL")]));
        assert_eq!(bare.message, "rack: on mains power");
        assert_eq!(bare.metrics, [Metric::new(metric::WARNINGS, 0.0, "")]);
    }

    #[test]
    fn warning_signs_are_counted_for_the_hub_and_the_upss_own_alarm_is_down() {
        let mut on_battery = back_ups("OB DISCHRG");
        on_battery.insert("battery.charge".into(), "85".into());
        on_battery.insert("battery.runtime".into(), "1870".into());
        let outage = describe("ups", &on_battery);
        assert_eq!(outage.status, Health::Up, "a Warning is the hub's");
        assert_eq!(
            outage.message,
            "Back-UPS ES 700G: on battery, battery 85 %, 31 min of runtime, load 23 %"
        );
        assert_eq!(value(&outage, metric::WARNINGS), Some(1.0));
        assert_eq!(value(&outage, metric::RUNTIME), Some(31.2));

        let worn = describe("ups", &back_ups("OL RB"));
        assert_eq!(worn.status, Health::Up);
        assert!(
            worn.message
                .contains(": its battery must be replaced, battery")
        );
        let several = describe("ups", &back_ups("OL OVER BYPASS ALARM"));
        assert_eq!(value(&several, metric::WARNINGS), Some(3.0));
        assert!(several.message.contains(
            ": overloaded, on bypass (what it powers isn't protected), it reports an alarm,"
        ));

        // The battery is running out: the UPS's own alarm.
        let low = describe("ups", &back_ups("OB LB"));
        assert_eq!(low.status, Health::Down);
        assert!(
            low.message.contains(": on battery, battery low, battery"),
            "{}",
            low.message
        );
        assert_eq!(value(&low, metric::WARNINGS), Some(1.0));
        assert_eq!(describe("ups", &back_ups("OB LB FSD")).status, Health::Down);
        let off = describe("ups", &back_ups("OFF"));
        assert_eq!(off.status, Health::Down);
        assert!(off.message.contains(": its output is off,"));

        // `LB` inside another word isn't the flag.
        assert_eq!(describe("ups", &back_ups("OL NOLB")).status, Health::Up);
    }

    #[test]
    fn a_ups_without_status_is_down() {
        let mut silent = back_ups("");
        assert_eq!(
            describe("ups", &silent).message,
            "Back-UPS ES 700G: the UPS reports no status"
        );
        silent.remove("ups.status");
        assert_eq!(describe("ups", &silent).status, Health::Down);
        // Numbers that aren't: left out.
        let odd = describe(
            "ups",
            &values(&[
                ("ups.status", "OL"),
                ("battery.charge", "n/a"),
                ("ups.load", "NaN"),
            ]),
        );
        assert_eq!(odd.message, "ups: on mains power");
    }

    /// A UPS a test server has, by name: its values, or the refusal the
    /// server gives instead.
    type Known = (&'static str, Result<BTreeMap<String, String>, &'static str>);

    /// A NUT server on a local port with these UPSes. Returns its port.
    async fn server(upses: Vec<Known>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let upses = upses.clone();
                tokio::spawn(async move {
                    let mut wire = Wire::new(socket);
                    while let Ok(line) = wire.line().await {
                        let asked = words(&line);
                        let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
                        let mut answer = Vec::new();
                        match asked[..] {
                            ["LIST", "UPS"] => {
                                answer.push("BEGIN LIST UPS".to_string());
                                for (name, _) in &upses {
                                    answer.push(format!("UPS {name} \"A UPS\""));
                                }
                                answer.push("END LIST UPS".to_string());
                            }
                            ["LIST", "VAR", name] => {
                                match upses.iter().find(|(known, _)| *known == name) {
                                    None => answer.push("ERR UNKNOWN-UPS".to_string()),
                                    Some((_, Err(code))) => answer.push(format!("ERR {code}")),
                                    Some((_, Ok(values))) => {
                                        answer.push(format!("BEGIN LIST VAR {name}"));
                                        for (key, value) in values {
                                            answer.push(format!("VAR {name} {key} \"{value}\""));
                                        }
                                        answer.push(format!("END LIST VAR {name}"));
                                    }
                                }
                            }
                            ["LOGOUT"] => answer.push("OK Goodbye".to_string()),
                            _ => answer.push("ERR UNKNOWN-COMMAND".to_string()),
                        }
                        for line in answer {
                            if wire.send(&line).await.is_err() {
                                return;
                            }
                        }
                    }
                });
            }
        });
        port
    }

    async fn asked(port: u16, ups: &str) -> Outcome {
        ask("127.0.0.1", port, ups, Duration::from_millis(1500)).await
    }

    #[tokio::test]
    async fn the_servers_only_ups_is_read_or_the_one_named() {
        let one = server(vec![("ups", Ok(back_ups("OL")))]).await;
        let outcome = asked(one, "").await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            "Back-UPS ES 700G: on mains power, battery 100 %, 42 min of runtime, load 23 %"
        );
        assert!(outcome.latency_ms.is_some());
        assert_eq!(asked(one, "ups").await.message, outcome.message);

        let two = server(vec![
            ("rack", Ok(back_ups("OL"))),
            ("desk", Ok(back_ups("OB LB"))),
        ])
        .await;
        let which = asked(two, "").await;
        assert_eq!(which.status, Health::Down);
        assert_eq!(
            which.message,
            "No word of the UPS: the server has 2 UPSes (rack, desk): name one in the check"
        );
        assert_eq!(asked(two, "rack").await.status, Health::Up);
        let desk = asked(two, "desk").await;
        assert_eq!(desk.status, Health::Down);
        assert!(desk.message.contains("battery low"));
    }

    #[tokio::test]
    async fn a_server_that_cant_tell_of_the_ups_is_down_with_the_reason() {
        let port = server(vec![
            ("stale", Err("DATA-STALE")),
            ("dead", Err("DRIVER-NOT-CONNECTED")),
            ("shut", Err("ACCESS-DENIED")),
            ("odd", Err("SOMETHING-NEW")),
        ])
        .await;
        for (ups, why) in [
            (
                "stale",
                "the server has lost contact with the UPS (its values are stale): a cable, or its driver",
            ),
            ("dead", "the UPS's driver isn't running on the server"),
            (
                "shut",
                "the server refuses this machine (ACCESS-DENIED): allow the probe's address in its settings",
            ),
            ("odd", "the server refused (SOMETHING-NEW)"),
            ("other", "the server has no UPS called other"),
        ] {
            let outcome = asked(port, ups).await;
            assert_eq!(outcome.status, Health::Down);
            assert_eq!(outcome.message, format!("No word of the UPS: {why}"));
        }
        let none = asked(server(vec![]).await, "").await;
        assert_eq!(none.message, "No word of the UPS: the server knows no UPS");
    }

    #[tokio::test]
    async fn something_else_a_silent_server_and_nobody_are_down() {
        // Something that isn't a UPS server answers.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 256];
                    let _ = socket.read(&mut buffer).await;
                    let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
                    while matches!(socket.read(&mut buffer).await, Ok(n) if n > 0) {}
                });
            }
        });
        let other = asked(port, "ups").await;
        assert_eq!(other.status, Health::Down);
        assert_eq!(
            other.message,
            "The port is open, but a UPS server (NUT) didn't answer: its answer isn't a UPS \
             server's (NUT)"
        );

        // It takes the question and says nothing.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let silent = asked(port, "ups").await;
        assert_eq!(
            silent.message,
            "The port is open, but a UPS server (NUT) didn't answer within 1.5 s"
        );

        let free = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let nobody = asked(free, "ups").await;
        assert!(
            nobody.message.starts_with("Connection"),
            "{}",
            nobody.message
        );
    }

    /// By hand (`--ignored --nocapture`), with a NUT server to ask in
    /// `NUT_HOST` (and a UPS's name in `NUT_UPS`, if it has several).
    #[tokio::test]
    #[ignore = "asks a NUT server named in NUT_HOST: run by hand"]
    async fn a_real_nut_server() {
        let host = std::env::var("NUT_HOST").expect("NUT_HOST");
        let ups = std::env::var("NUT_UPS").unwrap_or_default();
        let outcome = check(&host, None, &ups).await;
        println!(
            "{:?} in {:?} ms: {}",
            outcome.status, outcome.latency_ms, outcome.message
        );
        println!("{:?}", outcome.metrics);
    }
}
