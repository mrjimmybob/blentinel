//! The `interface` check (M5-15, D-92): is a network interface of the
//! probe's machine up, and what goes through it?
//!
//! Two things are read. The counters every system keeps for an interface
//! (bytes in and out, packets with an error) are read once a cycle with the
//! rest of the machine (`Monitor::refresh`); what changed since the cycle
//! before, over the time between, is the rate. Whether the link is up, and
//! at what speed, the system says itself: Linux in the files of
//! `/sys/class/net`, Windows in its list of adapters. Nothing is run (FP-2).

use std::time::{Duration, Instant};

use sysinfo::Networks;

use common::checks::{Metric, metric};

use super::Outcome;

/// The shortest time between two readings a rate is worked out over.
const MIN_PERIOD: Duration = Duration::from_secs(1);
const BITS_PER_MEGABIT: f64 = 1_000_000.0;
/// A speed above this isn't one: what a system says for "unknown".
const MAX_SPEED_MBIT: u64 = 1_000_000;
/// How many interfaces are named when the one asked for isn't there.
const MAX_NAMES_SHOWN: usize = 8;

/// The machine's interfaces with their counters, read once a cycle.
pub struct Traffic {
    networks: Networks,
    read_at: Instant,
    /// The time between the last two readings: what the counters changed
    /// over. `None`: read once only.
    period: Option<Duration>,
}

impl Traffic {
    pub fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
            read_at: Instant::now(),
            period: None,
        }
    }

    /// Read the counters again; interfaces that came or went with them.
    pub fn refresh(&mut self) {
        let now = Instant::now();
        self.networks.refresh(true);
        self.period = Some(now.duration_since(self.read_at));
        self.read_at = now;
    }

    /// The names of the machine's interfaces, in order: the system's own
    /// list where it has one (Windows counts layers of drivers beside its
    /// adapters), else what has counters.
    fn names(&self) -> Vec<String> {
        let mut names =
            platform::names().unwrap_or_else(|| self.networks.list().keys().cloned().collect());
        names.sort();
        names
    }

    /// What went through the interface `name` since the reading before.
    fn counted(&self, name: &str) -> Option<Counted> {
        let list = self.networks.list();
        let data = list.get(name).or_else(|| {
            list.iter()
                .find(|(known, _)| same(known, name))
                .map(|(_, data)| data)
        })?;
        Some(Counted {
            received: data.received(),
            sent: data.transmitted(),
            errors: data
                .errors_on_received()
                .saturating_add(data.errors_on_transmitted()),
        })
    }
}

/// Two names of one interface: Windows' names are whatever the case.
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// What went through an interface between two readings.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Counted {
    /// Bytes.
    received: u64,
    sent: u64,
    /// Packets with an error, in or out.
    errors: u64,
}

/// The link of an interface, as the system says it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(any(target_os = "linux", windows, test)), allow(dead_code))]
enum Link {
    /// Up, at this many megabits a second where the system knows.
    Up { speed: Option<u64> },
    /// Not up, with the system's word for it.
    Down(String),
    /// The machine has no such interface.
    Missing,
}

/// A speed the system gave, if it is one.
fn speed(megabits: u64) -> Option<u64> {
    (1..=MAX_SPEED_MBIT).contains(&megabits).then_some(megabits)
}

/// A rate in megabits a second, as people read it: `340 kbit/s`,
/// `2.4 Mbit/s`, `1 Gbit/s`.
fn rate(megabits: f64) -> String {
    let (value, unit) = match megabits {
        m if m >= 1000.0 => (m / 1000.0, "Gbit/s"),
        m if m >= 1.0 => (m, "Mbit/s"),
        m => (m * 1000.0, "kbit/s"),
    };
    let rounded = (value * 10.0).round() / 10.0;
    if unit == "kbit/s" || rounded.fract() == 0.0 {
        format!("{rounded:.0} {unit}")
    } else {
        format!("{rounded:.1} {unit}")
    }
}

/// Megabits a second of `bytes` over `period`, to a kilobit.
fn megabits(bytes: u64, period: Duration) -> f64 {
    let rate = bytes as f64 * 8.0 / BITS_PER_MEGABIT / period.as_secs_f64();
    (rate * 1000.0).round() / 1000.0
}

/// What the check says of the interface `name`. `link`: `None` where the
/// system doesn't say; an interface with counters is taken to be up there.
fn describe(
    name: &str,
    link: Option<Link>,
    counted: Option<Counted>,
    period: Option<Duration>,
    names: &[String],
) -> Outcome {
    let link = link.unwrap_or(match counted {
        Some(_) => Link::Up { speed: None },
        None => Link::Missing,
    });
    let speed = match link {
        Link::Up { speed } => speed,
        Link::Down(state) if state == "down" => {
            return Outcome::down(format!("{name}: the link is down"));
        }
        Link::Down(state) => return Outcome::down(format!("{name}: the link is down ({state})")),
        Link::Missing => {
            let mut known = names
                .iter()
                .take(MAX_NAMES_SHOWN)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if names.len() > MAX_NAMES_SHOWN {
                known.push_str(", …");
            }
            return Outcome::down(match names.is_empty() {
                true => format!("No network interface named {name}"),
                false => format!("No network interface named {name} (this machine has: {known})"),
            });
        }
    };

    let mut message = match speed {
        Some(speed) => format!("{name}: up at {}", rate(speed as f64)),
        None => format!("{name}: up"),
    };
    let mut metrics = Vec::new();
    // The first reading has nothing before it to measure against.
    let measured = counted.zip(period.filter(|period| *period >= MIN_PERIOD));
    if let Some((counted, period)) = measured {
        let (received, sent) = (
            megabits(counted.received, period),
            megabits(counted.sent, period),
        );
        message.push_str(&format!(", in {}, out {}", rate(received), rate(sent)));
        match counted.errors {
            0 => {}
            1 => message.push_str(", 1 error"),
            errors => message.push_str(&format!(", {errors} errors")),
        }
        metrics.extend([
            Metric::new(metric::RECEIVED, received, metric::MEGABITS_PER_SECOND),
            Metric::new(metric::SENT, sent, metric::MEGABITS_PER_SECOND),
            Metric::new(metric::ERRORS, counted.errors as f64, ""),
        ]);
    }
    if let Some(speed) = speed {
        metrics.push(Metric::new(
            metric::SPEED,
            speed as f64,
            metric::MEGABITS_PER_SECOND,
        ));
    }
    Outcome::up(message).with_metrics(metrics)
}

pub fn check(name: &str, traffic: &Traffic) -> Outcome {
    describe(
        name,
        platform::link(name),
        traffic.counted(name),
        traffic.period,
        &traffic.names(),
    )
}

/// The link as Linux' files of an interface say it: `operstate` (`up`,
/// `down`, `lowerlayerdown`, …; `unknown` for an interface that doesn't
/// say, as the loopback and tunnels), `carrier` (`1`: a signal) and `speed`
/// in megabits a second (`-1`, or unreadable: not known).
#[cfg(any(target_os = "linux", test))]
fn from_files(state: &str, carrier: Option<&str>, megabits: Option<&str>) -> Link {
    let up = match state {
        "up" => true,
        "unknown" => carrier != Some("0"),
        _ => false,
    };
    if up {
        Link::Up {
            speed: megabits.and_then(|text| text.parse().ok()).and_then(speed),
        }
    } else {
        Link::Down(state.to_string())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Link, from_files};

    pub fn link(name: &str) -> Option<Link> {
        let dir = std::path::Path::new("/sys/class/net").join(name);
        let read = |file: &str| {
            std::fs::read_to_string(dir.join(file))
                .ok()
                .map(|text| text.trim().to_string())
        };
        Some(match read("operstate") {
            Some(state) => from_files(&state, read("carrier").as_deref(), read("speed").as_deref()),
            None => Link::Missing,
        })
    }

    pub fn names() -> Option<Vec<String>> {
        None
    }
}

#[cfg(windows)]
mod platform {
    use ipconfig::OperStatus;

    use super::{BITS_PER_MEGABIT, Link, same, speed};

    pub fn link(name: &str) -> Option<Link> {
        let adapters = ipconfig::get_adapters()
            .inspect_err(|e| tracing::debug!("Can't list the network adapters: {e}"))
            .ok()?;
        let Some(adapter) = adapters.iter().find(|a| same(a.friendly_name(), name)) else {
            return Some(Link::Missing);
        };
        let state = match adapter.oper_status() {
            OperStatus::IfOperStatusUp => {
                let megabits = adapter.transmit_link_speed() / BITS_PER_MEGABIT as u64;
                return Some(Link::Up {
                    speed: speed(megabits),
                });
            }
            OperStatus::IfOperStatusDown => "down",
            OperStatus::IfOperStatusDormant => "dormant",
            OperStatus::IfOperStatusNotPresent => "not present",
            OperStatus::IfOperStatusLowerLayerDown => "lower layer down",
            OperStatus::IfOperStatusTesting => "testing",
            _ => "unknown",
        };
        Some(Link::Down(state.to_string()))
    }

    pub fn names() -> Option<Vec<String>> {
        let adapters = ipconfig::get_adapters().ok()?;
        Some(
            adapters
                .iter()
                .map(|adapter| adapter.friendly_name().to_string())
                .collect(),
        )
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::Link;

    pub fn link(_name: &str) -> Option<Link> {
        None
    }

    pub fn names() -> Option<Vec<String>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    const MINUTE: Option<Duration> = Some(Duration::from_secs(60));

    fn up(speed: Option<u64>) -> Option<Link> {
        Some(Link::Up { speed })
    }

    fn counted(received: u64, sent: u64, errors: u64) -> Option<Counted> {
        Some(Counted {
            received,
            sent,
            errors,
        })
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value)
    }

    #[test]
    fn what_went_through_is_said_as_a_rate_over_the_time_between_two_readings() {
        // 18 MB in and 1.5 MB out in a minute: 2.4 Mbit/s and 200 kbit/s.
        let busy = describe(
            "eth0",
            up(Some(1000)),
            counted(18_000_000, 1_500_000, 0),
            MINUTE,
            &[],
        );
        assert_eq!(busy.status, Health::Up);
        assert_eq!(
            busy.message,
            "eth0: up at 1 Gbit/s, in 2.4 Mbit/s, out 200 kbit/s"
        );
        let keys: Vec<&str> = busy.metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                metric::RECEIVED,
                metric::SENT,
                metric::ERRORS,
                metric::SPEED
            ]
        );
        assert_eq!(value(&busy, metric::RECEIVED), Some(2.4));
        assert_eq!(value(&busy, metric::SENT), Some(0.2));
        assert_eq!(value(&busy, metric::ERRORS), Some(0.0));
        assert_eq!(value(&busy, metric::SPEED), Some(1000.0));
        assert_eq!(busy.metrics[0].unit, "Mbit/s");

        // Errors are said, and a link whose speed isn't known says none.
        let faulty = describe("wlan0", up(None), counted(0, 0, 3), MINUTE, &[]);
        assert_eq!(
            faulty.message,
            "wlan0: up, in 0 kbit/s, out 0 kbit/s, 3 errors"
        );
        assert_eq!(value(&faulty, metric::ERRORS), Some(3.0));
        assert_eq!(value(&faulty, metric::SPEED), None);
        let one = describe("wlan0", up(None), counted(0, 0, 1), MINUTE, &[]);
        assert!(one.message.ends_with(", 1 error"), "{}", one.message);

        assert_eq!(rate(0.0004), "0 kbit/s");
        assert_eq!(rate(0.34), "340 kbit/s");
        assert_eq!(rate(54.0), "54 Mbit/s");
        assert_eq!(rate(999.96), "1000 Mbit/s");
        assert_eq!(rate(2500.0), "2.5 Gbit/s");
        assert_eq!(rate(10_000.0), "10 Gbit/s");
    }

    #[test]
    fn a_first_reading_says_the_link_only() {
        for period in [None, Some(Duration::from_millis(300))] {
            let first = describe("eth0", up(Some(100)), counted(9_000, 9_000, 0), period, &[]);
            assert_eq!(first.status, Health::Up);
            assert_eq!(first.message, "eth0: up at 100 Mbit/s");
            assert_eq!(value(&first, metric::RECEIVED), None);
            assert_eq!(value(&first, metric::SPEED), Some(100.0));
        }
        // Up, by the system's word, though nothing counts it.
        let uncounted = describe("tun0", up(None), None, MINUTE, &[]);
        assert_eq!(
            (uncounted.status, uncounted.message.as_str()),
            (Health::Up, "tun0: up")
        );
        assert!(uncounted.metrics.is_empty());
    }

    #[test]
    fn a_link_that_is_down_or_an_interface_that_isnt_there_is_down() {
        let down = |state: &str| {
            describe(
                "eth0",
                Some(Link::Down(state.into())),
                counted(1, 1, 0),
                MINUTE,
                &[],
            )
        };
        assert_eq!(down("down").status, Health::Down);
        assert_eq!(down("down").message, "eth0: the link is down");
        assert_eq!(
            down("lowerlayerdown").message,
            "eth0: the link is down (lowerlayerdown)"
        );
        assert!(down("down").metrics.is_empty());

        let names: Vec<String> = ["eth0", "lo"].map(String::from).to_vec();
        let missing = describe("eth1", Some(Link::Missing), None, MINUTE, &names);
        assert_eq!(missing.status, Health::Down);
        assert_eq!(
            missing.message,
            "No network interface named eth1 (this machine has: eth0, lo)"
        );
        let many: Vec<String> = (0..10).map(|n| format!("veth{n}")).collect();
        let missing = describe("eth1", Some(Link::Missing), None, MINUTE, &many);
        assert!(
            missing.message.ends_with("veth7, …)"),
            "{}",
            missing.message
        );
        let alone = describe("eth1", Some(Link::Missing), None, MINUTE, &[]);
        assert_eq!(alone.message, "No network interface named eth1");

        // Where the system doesn't say: an interface with counters is up,
        // one without isn't there.
        assert_eq!(
            describe("em0", None, counted(0, 0, 0), None, &[]).status,
            Health::Up
        );
        assert_eq!(
            describe("em1", None, None, None, &[]).message,
            "No network interface named em1"
        );
    }

    #[test]
    fn linux_files_say_whether_the_link_is_up_and_how_fast() {
        assert_eq!(
            from_files("up", Some("1"), Some("1000")),
            Link::Up { speed: Some(1000) }
        );
        // Wi-Fi and virtual interfaces: no speed, or one that isn't.
        for unknown in [None, Some("-1"), Some("0"), Some("4294967295"), Some("")] {
            assert_eq!(
                from_files("up", Some("1"), unknown),
                Link::Up { speed: None },
                "{unknown:?}"
            );
        }
        // The loopback doesn't say: its carrier does.
        assert_eq!(
            from_files("unknown", Some("1"), None),
            Link::Up { speed: None }
        );
        assert_eq!(from_files("unknown", None, None), Link::Up { speed: None });
        assert_eq!(
            from_files("unknown", Some("0"), None),
            Link::Down("unknown".into())
        );
        for state in ["down", "lowerlayerdown", "dormant", "notpresent"] {
            assert_eq!(
                from_files(state, None, Some("1000")),
                Link::Down(state.into())
            );
        }
    }

    #[test]
    fn this_machines_interfaces_are_found_by_the_names_their_counters_have() {
        let mut traffic = Traffic::new();
        let names = traffic.names();
        assert!(!names.is_empty(), "a machine has a loopback at least");
        std::thread::sleep(Duration::from_millis(1100));
        traffic.refresh();
        // Not every interface that is up is counted (Windows' loopback
        // isn't), but one is.
        let mut measured = 0;
        for name in &names {
            let outcome = check(name, &traffic);
            println!("{:?}: {}", outcome.status, outcome.message);
            assert!(
                !outcome.message.starts_with("No network interface"),
                "{}",
                outcome.message
            );
            measured += usize::from(value(&outcome, metric::RECEIVED).is_some());
        }
        assert!(measured > 0, "no interface of {names:?} is up and counted");
        let nobody = check("blentinel-no-such-interface", &traffic);
        assert_eq!(nobody.status, Health::Down);
        assert!(
            nobody
                .message
                .starts_with("No network interface named blentinel-no-such-interface (this"),
            "{}",
            nobody.message
        );
    }
}
