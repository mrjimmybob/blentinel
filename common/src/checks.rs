//! Checks as the hub defines them and the probe runs them (M2-04, proposal
//! 0003 §4.1, D-05, D-15, D-49).
//!
//! - [`CheckSpec`] is what to check, one variant per kind with typed
//!   parameters, and [`CheckSpec::validate`] is the single validation: the hub
//!   API, the hub UI and the probe all call it.
//! - [`ProbeConfig`] is everything a probe runs: its report interval and its
//!   checks. The hub sends it in its signed answer to a report
//!   ([`crate::ack::ReportAck::config`]) and the probe caches it on disk.
//! - [`CheckResult`] is one check's outcome in a report, keyed by its stable
//!   [`CheckId`], with any number of [`Metric`]s (proposal 0006 §1.4).
//!
//! Depends only on `serde` and `std`, so the hub's web UI can use it too.

use std::collections::HashSet;
use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// The stable identity of a check: a random ID the hub gives it when it is
/// created, kept when it is renamed or retargeted. Results, alert state and
/// silences are keyed by it (D-15).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckId(pub String);

impl CheckId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a check looks at. JSON: `{"kind": "tcp", "host": "10.0.0.5", "port": 443}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckSpec {
    /// One ICMP echo to an IP address.
    Ping { host: String },
    /// `GET` a URL; up on a 2xx answer.
    Http { url: String },
    /// Open a TCP connection.
    Tcp { host: String, port: u16 },
    /// CPU usage of the probe's machine.
    Cpu,
    /// Memory usage of the probe's machine.
    Memory,
    /// Usage of one of the probe machine's disks, by mount point (`/`,
    /// `C:\`). Without a path: the system disk.
    Disk {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// How long the probe's machine has been running.
    Uptime,
    /// The certificate a TLS server presents (HTTPS, LDAPS, SMTPS, …): how
    /// long until it expires, and whether a client would accept it.
    Tls {
        host: String,
        port: u16,
        /// `false`: only the dates count, not who issued it or for which
        /// name (a self-signed certificate, a private authority).
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        verify: bool,
    },
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// Every kind, in the order the UI offers them.
pub const CHECK_KINDS: [&str; 8] = [
    "ping", "http", "tcp", "tls", "cpu", "memory", "disk", "uptime",
];

/// The port a `tls` check looks at when none is typed (HTTPS).
pub const DEFAULT_TLS_PORT: u16 = 443;

/// The longest check name, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// The longest host, URL or path, in characters.
pub const MAX_TARGET_CHARS: usize = 2000;
/// The most checks one probe runs (keeps the hub's answer small).
pub const MAX_CHECKS_PER_PROBE: usize = 500;

/// Report intervals the hub accepts, in seconds.
pub const MIN_INTERVAL_SECS: u32 = 5;
pub const MAX_INTERVAL_SECS: u32 = 3600;
/// A probe's report interval until the hub says otherwise.
pub const DEFAULT_INTERVAL_SECS: u32 = 60;

impl CheckSpec {
    /// The kind, as in the JSON `kind` field (one of [`CHECK_KINDS`]).
    pub fn kind(&self) -> &'static str {
        match self {
            CheckSpec::Ping { .. } => "ping",
            CheckSpec::Http { .. } => "http",
            CheckSpec::Tcp { .. } => "tcp",
            CheckSpec::Cpu => "cpu",
            CheckSpec::Memory => "memory",
            CheckSpec::Disk { .. } => "disk",
            CheckSpec::Uptime => "uptime",
            CheckSpec::Tls { .. } => "tls",
        }
    }

    /// What it looks at, for people: `10.0.0.1`, `db:5432`, a URL, a disk.
    /// Checks of the probe's own machine say `localhost`.
    pub fn target(&self) -> String {
        match self {
            CheckSpec::Ping { host } => host.clone(),
            CheckSpec::Http { url } => url.clone(),
            CheckSpec::Tcp { host, port } | CheckSpec::Tls { host, port, .. }
                if host.contains(':') =>
            {
                format!("[{host}]:{port}")
            }
            CheckSpec::Tcp { host, port } | CheckSpec::Tls { host, port, .. } => {
                format!("{host}:{port}")
            }
            CheckSpec::Disk { path: Some(path) } => path.clone(),
            CheckSpec::Disk { path: None } => "system disk".to_string(),
            CheckSpec::Cpu | CheckSpec::Memory | CheckSpec::Uptime => "localhost".to_string(),
        }
    }

    /// Whether the probe can run it. The one rule set for the hub and the probe.
    pub fn validate(&self) -> Result<(), CheckError> {
        match self {
            CheckSpec::Ping { host } => {
                let host = required("host", host)?;
                host.parse::<IpAddr>()
                    .map(|_| ())
                    .map_err(|_| CheckError::new("ping needs an IP address, e.g. 192.168.1.1"))
            }
            CheckSpec::Http { url } => {
                let url = required("url", url)?;
                let rest = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"))
                    .ok_or_else(|| {
                        CheckError::new("the URL must start with http:// or https://")
                    })?;
                if rest.is_empty() || rest.starts_with('/') || url.contains(char::is_whitespace) {
                    return Err(CheckError::new(
                        "the URL needs a host, e.g. https://example.com",
                    ));
                }
                Ok(())
            }
            CheckSpec::Tcp { host, port } | CheckSpec::Tls { host, port, .. } => {
                let host = required("host", host)?;
                if host.contains(char::is_whitespace) || host.contains('/') {
                    return Err(CheckError::new("the host is a name or an IP address"));
                }
                if *port == 0 {
                    return Err(CheckError::new("the port must be 1-65535"));
                }
                Ok(())
            }
            CheckSpec::Disk { path: Some(path) } => required("path", path).map(|_| ()),
            CheckSpec::Disk { path: None } | CheckSpec::Cpu | CheckSpec::Memory => Ok(()),
            CheckSpec::Uptime => Ok(()),
        }
    }
}

/// `value`, which must be neither blank nor too long.
fn required<'a>(field: &str, value: &'a str) -> Result<&'a str, CheckError> {
    if value.trim().is_empty() {
        return Err(CheckError(format!("the {field} can't be empty")));
    }
    if value.trim() != value {
        return Err(CheckError(format!(
            "the {field} has spaces at the start or end"
        )));
    }
    if value.chars().count() > MAX_TARGET_CHARS {
        return Err(CheckError(format!(
            "the {field} is longer than {MAX_TARGET_CHARS} characters"
        )));
    }
    Ok(value)
}

/// Why a check or config can't be used. The text is for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError(pub String);

impl CheckError {
    fn new(message: &str) -> Self {
        Self(message.to_string())
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CheckError {}

/// A check's name: not blank, at most [`MAX_NAME_CHARS`].
pub fn validate_name(name: &str) -> Result<(), CheckError> {
    if name.trim().is_empty() {
        return Err(CheckError::new("the check name can't be empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(CheckError(format!(
            "the check name is longer than {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(())
}

/// A report interval within [`MIN_INTERVAL_SECS`]..=[`MAX_INTERVAL_SECS`].
pub fn validate_interval(secs: u32) -> Result<(), CheckError> {
    if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&secs) {
        return Err(CheckError(format!(
            "the interval must be {MIN_INTERVAL_SECS}-{MAX_INTERVAL_SECS} seconds"
        )));
    }
    Ok(())
}

/// One check as the probe runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckDefinition {
    pub id: CheckId,
    pub name: String,
    pub spec: CheckSpec,
}

/// Everything a probe runs, as the hub sends it (only enabled checks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeConfig {
    /// Identifies this exact content (the hub derives it from the rest). The
    /// probe echoes it in its reports, so the hub knows what it runs.
    pub revision: String,
    /// Seconds between two reports.
    pub interval_secs: u32,
    pub checks: Vec<CheckDefinition>,
}

impl ProbeConfig {
    /// Whether the probe can run all of it. A config is applied whole or not
    /// at all: the probe keeps the previous one if this fails.
    pub fn validate(&self) -> Result<(), CheckError> {
        if self.revision.is_empty() {
            return Err(CheckError::new("the config has no revision"));
        }
        validate_interval(self.interval_secs)?;
        if self.checks.len() > MAX_CHECKS_PER_PROBE {
            return Err(CheckError(format!(
                "more than {MAX_CHECKS_PER_PROBE} checks"
            )));
        }
        let mut ids = HashSet::new();
        for check in &self.checks {
            if !ids.insert(&check.id) {
                return Err(CheckError(format!("check {} appears twice", check.id)));
            }
            validate_name(&check.name)
                .and_then(|()| check.spec.validate())
                .map_err(|e| CheckError(format!("check \"{}\": {e}", check.name)))?;
        }
        Ok(())
    }
}

/// A config the probe refused, reported until the hub sends another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRejection {
    pub revision: String,
    pub error: String,
}

/// Did the check succeed? The probe reports only this; warning and down
/// thresholds on the metrics are the hub's business (proposal 0006 §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Health {
    Up,
    Down,
}

impl Health {
    /// `"Up"` or `"Down"`, as serialised and stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Up => "Up",
            Health::Down => "Down",
        }
    }
}

/// One measured value, e.g. `{"key": "usage", "value": 42.5, "unit": "%"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    /// Stable within a kind; see [`metric`] for the keys each kind reports.
    pub key: String,
    pub value: f64,
    pub unit: String,
}

impl Metric {
    pub fn new(key: &str, value: f64, unit: &str) -> Self {
        Self {
            key: key.to_string(),
            value,
            unit: unit.to_string(),
        }
    }
}

/// Metric keys and units. The first metric of a result is its headline value.
pub mod metric {
    /// `cpu`, `memory`, `disk`: percent used (hub thresholds apply to it).
    pub const USAGE: &str = "usage";
    /// `memory`, `disk`: gigabytes used.
    pub const USED: &str = "used";
    /// `disk`: gigabytes free.
    pub const FREE: &str = "free";
    /// `memory`, `disk`: gigabytes in total.
    pub const TOTAL: &str = "total";
    /// `uptime`: seconds since the machine started.
    pub const UPTIME: &str = "uptime";
    /// `tls`: days until the certificate expires (hub thresholds apply to
    /// it: at or under a limit).
    pub const DAYS_LEFT: &str = "days_left";

    pub const PERCENT: &str = "%";
    pub const GIGABYTES: &str = "GB";
    pub const SECONDS: &str = "s";
    pub const DAYS: &str = "d";
}

/// One check's outcome in a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_id: CheckId,
    pub status: Health,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
}

impl CheckResult {
    /// The metric with this key, if the result has it.
    pub fn metric(&self, key: &str) -> Option<&Metric> {
        self.metrics.iter().find(|m| m.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str, spec: CheckSpec) -> CheckDefinition {
        CheckDefinition {
            id: CheckId(id.into()),
            name: format!("check {id}"),
            spec,
        }
    }

    fn config(checks: Vec<CheckDefinition>) -> ProbeConfig {
        ProbeConfig {
            revision: "r1".into(),
            interval_secs: 60,
            checks,
        }
    }

    #[test]
    fn specs_serialise_with_their_kind() {
        let tcp = CheckSpec::Tcp {
            host: "db".into(),
            port: 5432,
        };
        assert_eq!(
            serde_json::to_value(&tcp).unwrap(),
            serde_json::json!({"kind": "tcp", "host": "db", "port": 5432})
        );
        let disk: CheckSpec = serde_json::from_str(r#"{"kind":"disk"}"#).unwrap();
        assert_eq!(disk, CheckSpec::Disk { path: None });
        assert_eq!(
            serde_json::to_string(&CheckSpec::Cpu).unwrap(),
            r#"{"kind":"cpu"}"#
        );
    }

    #[test]
    fn a_tls_check_verifies_the_certificate_unless_told_not_to() {
        let tls = |verify| CheckSpec::Tls {
            host: "example.com".into(),
            port: 443,
            verify,
        };
        let json = r#"{"kind":"tls","host":"example.com","port":443}"#;
        assert_eq!(serde_json::to_string(&tls(true)).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), tls(true));
        let unverified = serde_json::to_value(tls(false)).unwrap();
        assert_eq!(unverified["verify"], false);
        assert_eq!(
            serde_json::from_value::<CheckSpec>(unverified).unwrap(),
            tls(false)
        );
        assert_eq!(tls(true).target(), "example.com:443");
    }

    #[test]
    fn every_kind_is_listed_and_named_as_serialised() {
        let all = [
            CheckSpec::Ping {
                host: "10.0.0.1".into(),
            },
            CheckSpec::Http {
                url: "https://example.com".into(),
            },
            CheckSpec::Tcp {
                host: "db".into(),
                port: 1,
            },
            CheckSpec::Tls {
                host: "example.com".into(),
                port: DEFAULT_TLS_PORT,
                verify: true,
            },
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk { path: None },
            CheckSpec::Uptime,
        ];
        assert_eq!(all.len(), CHECK_KINDS.len());
        for (spec, kind) in all.iter().zip(CHECK_KINDS) {
            assert_eq!(spec.kind(), kind);
            let json = serde_json::to_value(spec).unwrap();
            assert_eq!(json["kind"], kind);
            assert_eq!(spec.validate(), Ok(()), "{kind}");
        }
    }

    #[test]
    fn targets_read_well() {
        let tcp = |host: &str| CheckSpec::Tcp {
            host: host.into(),
            port: 443,
        };
        assert_eq!(tcp("db").target(), "db:443");
        assert_eq!(tcp("::1").target(), "[::1]:443");
        assert_eq!(CheckSpec::Memory.target(), "localhost");
        assert_eq!(CheckSpec::Disk { path: None }.target(), "system disk");
    }

    #[test]
    fn invalid_specs_say_why() {
        let cases = [
            (CheckSpec::Ping { host: "".into() }, "can't be empty"),
            (
                CheckSpec::Ping {
                    host: "router.lan".into(),
                },
                "IP address",
            ),
            (
                CheckSpec::Http {
                    url: "example.com".into(),
                },
                "http://",
            ),
            (
                CheckSpec::Http {
                    url: "https://".into(),
                },
                "needs a host",
            ),
            (
                CheckSpec::Tcp {
                    host: "db".into(),
                    port: 0,
                },
                "port",
            ),
            (
                CheckSpec::Tcp {
                    host: " db".into(),
                    port: 5,
                },
                "spaces",
            ),
            (
                CheckSpec::Tls {
                    host: "https://example.com".into(),
                    port: 443,
                    verify: true,
                },
                "a name or an IP address",
            ),
            (
                CheckSpec::Disk {
                    path: Some("".into()),
                },
                "can't be empty",
            ),
        ];
        for (spec, expected) in cases {
            let error = spec.validate().expect_err(&format!("{spec:?}"));
            assert!(error.0.contains(expected), "{spec:?}: {error}");
        }
    }

    #[test]
    fn a_config_is_valid_only_as_a_whole() {
        assert_eq!(config(vec![check("a", CheckSpec::Cpu)]).validate(), Ok(()));

        let twice = config(vec![
            check("a", CheckSpec::Cpu),
            check("a", CheckSpec::Memory),
        ]);
        assert!(twice.validate().unwrap_err().0.contains("twice"));

        let bad = config(vec![
            check("a", CheckSpec::Cpu),
            check("b", CheckSpec::Ping { host: "x".into() }),
        ]);
        assert!(bad.validate().unwrap_err().0.contains("check \"check b\""));

        let mut slow = config(vec![]);
        slow.interval_secs = MAX_INTERVAL_SECS + 1;
        assert!(slow.validate().is_err());

        let mut unnamed = config(vec![check("a", CheckSpec::Cpu)]);
        unnamed.checks[0].name = " ".into();
        assert!(unnamed.validate().is_err());
    }

    #[test]
    fn a_config_of_an_unknown_kind_does_not_parse() {
        // A newer hub's new kind: the probe refuses the whole config and keeps its own.
        let json = r#"{"revision":"r","interval_secs":60,"checks":[{"id":"a","name":"x","spec":{"kind":"snmp"}}]}"#;
        assert!(serde_json::from_str::<ProbeConfig>(json).is_err());
    }

    #[test]
    fn results_leave_out_what_they_dont_have() {
        let result = CheckResult {
            check_id: CheckId("c1".into()),
            status: Health::Up,
            message: "ok".into(),
            latency_ms: None,
            metrics: vec![],
        };
        assert_eq!(
            serde_json::to_string(&result).unwrap(),
            r#"{"check_id":"c1","status":"Up","message":"ok"}"#
        );
        let with_metric = CheckResult {
            metrics: vec![Metric::new(metric::USAGE, 12.5, metric::PERCENT)],
            ..result
        };
        assert_eq!(with_metric.metric(metric::USAGE).unwrap().value, 12.5);
        assert!(with_metric.metric(metric::FREE).is_none());
    }
}
