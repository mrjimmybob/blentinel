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
use std::ops::RangeInclusive;

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
    /// Request a URL; up when the answer is what [`HttpOptions`] expects
    /// (by default: a `GET` answered 2xx).
    Http {
        url: String,
        #[serde(flatten)]
        options: HttpOptions,
    },
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
    /// The newest file at a path on the probe's machine (or a share it can
    /// read): how old it is and how big. For backups that must have run.
    File {
        /// A file, or a folder whose newest file counts.
        path: String,
        /// In a folder, the names that count: `*.bak`, `backup-*.zip`
        /// (`*` any characters, `?` any one). Empty: every file.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pattern: String,
        /// Look in the folder's subfolders too.
        #[serde(default, skip_serializing_if = "is_default")]
        recursive: bool,
        /// The newest file must be at least this big, in bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_bytes: Option<u64>,
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

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// How an `http` check asks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    #[default]
    Get,
    /// Headers only: lighter, but there is no body to search.
    Head,
}

impl HttpMethod {
    /// `get`, `head`: as in the JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Head => "head",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "get" => Some(HttpMethod::Get),
            "head" => Some(HttpMethod::Head),
            _ => None,
        }
    }
}

/// A header an `http` check sends. Not for secrets: everyone who sees the
/// check sees it (D-78).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// What an `http` check sends and expects, beyond its URL. Each is left out
/// of the JSON at its default, so `{"kind": "http", "url": …}` is a plain
/// `GET` that must answer 2xx.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpOptions {
    #[serde(default, skip_serializing_if = "is_default")]
    pub method: HttpMethod,
    /// The status codes that count as up, e.g. `200-299, 401`
    /// ([`status_ranges`]). Empty: [`DEFAULT_HTTP_STATUS`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// Text the answer's body must contain (as written: case matters).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub contains: String,
    /// Text the answer's body must not contain.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub absent: String,
    /// `false`: a redirect is the answer (expect its 3xx status).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub follow_redirects: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HttpHeader>,
    /// `false`: any certificate will do over HTTPS.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub verify: bool,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            method: HttpMethod::Get,
            status: String::new(),
            contains: String::new(),
            absent: String::new(),
            follow_redirects: true,
            headers: Vec::new(),
            verify: true,
        }
    }
}

/// The status codes an `http` check expects when it names none.
pub const DEFAULT_HTTP_STATUS: &str = "200-299";
/// The most status codes or ranges an `http` check lists.
pub const MAX_STATUS_RANGES: usize = 20;
/// The longest text an `http` check looks for, in characters.
pub const MAX_KEYWORD_CHARS: usize = 200;
/// The most headers an `http` check sends.
pub const MAX_HTTP_HEADERS: usize = 10;

/// The status codes `text` names: codes and ranges separated by commas
/// (`200`, `200-299, 401`). Empty: [`DEFAULT_HTTP_STATUS`].
pub fn status_ranges(text: &str) -> Result<Vec<RangeInclusive<u16>>, CheckError> {
    let text = match text.trim() {
        "" => DEFAULT_HTTP_STATUS,
        text => text,
    };
    let code = |part: &str| {
        part.trim()
            .parse::<u16>()
            .ok()
            .filter(|code| (100..=599).contains(code))
            .ok_or_else(|| CheckError(format!("\"{}\" isn't a status code (100-599)", part.trim())))
    };
    let ranges = text
        .split(',')
        .map(|part| {
            let (from, to) = match part.split_once('-') {
                Some((from, to)) => (code(from)?, code(to)?),
                None => (code(part)?, code(part)?),
            };
            if from > to {
                return Err(CheckError(format!(
                    "the status range {from}-{to} is backwards"
                )));
            }
            Ok(from..=to)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ranges.len() > MAX_STATUS_RANGES {
        return Err(CheckError(format!(
            "more than {MAX_STATUS_RANGES} status codes or ranges"
        )));
    }
    Ok(ranges)
}

impl HttpOptions {
    /// Is `status` one this check expects? (Nothing is, if the list is invalid.)
    pub fn expects(&self, status: u16) -> bool {
        status_ranges(&self.status).is_ok_and(|ranges| ranges.iter().any(|r| r.contains(&status)))
    }

    /// The expected status codes as written, or the default.
    pub fn expected_status(&self) -> &str {
        match self.status.trim() {
            "" => DEFAULT_HTTP_STATUS,
            status => status,
        }
    }

    /// Does the check read the answer's body?
    pub fn searches_body(&self) -> bool {
        !self.contains.is_empty() || !self.absent.is_empty()
    }

    fn validate(&self) -> Result<(), CheckError> {
        status_ranges(&self.status)?;
        for keyword in [&self.contains, &self.absent] {
            if keyword.chars().count() > MAX_KEYWORD_CHARS {
                return Err(CheckError(format!(
                    "the text to look for is longer than {MAX_KEYWORD_CHARS} characters"
                )));
            }
        }
        if self.method == HttpMethod::Head && self.searches_body() {
            return Err(CheckError::new(
                "a HEAD request has no body to look for text in: use GET",
            ));
        }
        if self.headers.len() > MAX_HTTP_HEADERS {
            return Err(CheckError(format!("more than {MAX_HTTP_HEADERS} headers")));
        }
        for header in &self.headers {
            let name_ok = !header.name.is_empty()
                && header
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !name_ok {
                return Err(CheckError(format!(
                    "\"{}\" isn't a header name (letters, digits, - and _)",
                    header.name
                )));
            }
            let value_ok = header.value.chars().count() <= MAX_TARGET_CHARS
                && header.value.trim() == header.value
                && !header.value.chars().any(char::is_control);
            if !value_ok {
                return Err(CheckError(format!(
                    "the value of the header {} has spaces at its ends, control characters, or is too long",
                    header.name
                )));
            }
        }
        Ok(())
    }
}

/// Every kind, in the order the UI offers them.
pub const CHECK_KINDS: [&str; 9] = [
    "ping", "http", "tcp", "tls", "cpu", "memory", "disk", "file", "uptime",
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
    /// A plain `http` check: a `GET` of `url` that must answer 2xx.
    pub fn http(url: &str) -> Self {
        CheckSpec::Http {
            url: url.to_string(),
            options: HttpOptions::default(),
        }
    }

    /// The kind, as in the JSON `kind` field (one of [`CHECK_KINDS`]).
    pub fn kind(&self) -> &'static str {
        match self {
            CheckSpec::Ping { .. } => "ping",
            CheckSpec::Http { .. } => "http",
            CheckSpec::Tcp { .. } => "tcp",
            CheckSpec::Cpu => "cpu",
            CheckSpec::Memory => "memory",
            CheckSpec::Disk { .. } => "disk",
            CheckSpec::File { .. } => "file",
            CheckSpec::Uptime => "uptime",
            CheckSpec::Tls { .. } => "tls",
        }
    }

    /// What it looks at, for people: `10.0.0.1`, `db:5432`, a URL, a disk.
    /// Checks of the probe's own machine say `localhost`.
    pub fn target(&self) -> String {
        match self {
            CheckSpec::Ping { host } => host.clone(),
            CheckSpec::Http { url, .. } => url.clone(),
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
            CheckSpec::File { path, pattern, .. } if pattern.is_empty() => path.clone(),
            CheckSpec::File { path, pattern, .. } => format!("{path} ({pattern})"),
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
            CheckSpec::Http { url, options } => {
                options.validate()?;
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
            CheckSpec::File { path, pattern, .. } => {
                let path = required("path", path)?;
                // Whichever system the probe runs on: /…, C:\…, \\server\share.
                let drive = path.as_bytes().get(1) == Some(&b':')
                    && path.starts_with(|c: char| c.is_ascii_alphabetic())
                    && path[2..].starts_with(['\\', '/']);
                if !(path.starts_with('/') || path.starts_with("\\\\") || drive) {
                    return Err(CheckError::new(
                        "the path must be a full one, e.g. /var/backups, D:\\Backups or \\\\nas\\backups",
                    ));
                }
                if pattern.chars().count() > MAX_KEYWORD_CHARS
                    || pattern.contains(['/', '\\'])
                    || pattern.trim() != pattern
                {
                    return Err(CheckError::new(
                        "the pattern is for file names, e.g. *.bak: no folders, no spaces at its ends",
                    ));
                }
                Ok(())
            }
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
    /// `file`: hours since the newest file was written (hub thresholds
    /// apply to it).
    pub const AGE: &str = "age";
    /// `file`: megabytes of the newest file.
    pub const SIZE: &str = "size";
    /// `file`: how many files match.
    pub const FILES: &str = "files";
    /// `tls`: days until the certificate expires (hub thresholds apply to
    /// it: at or under a limit).
    pub const DAYS_LEFT: &str = "days_left";

    pub const PERCENT: &str = "%";
    pub const GIGABYTES: &str = "GB";
    pub const SECONDS: &str = "s";
    pub const DAYS: &str = "d";
    pub const HOURS: &str = "h";
    pub const MEGABYTES: &str = "MB";
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

    fn file(path: &str, pattern: &str) -> CheckSpec {
        CheckSpec::File {
            path: path.into(),
            pattern: pattern.into(),
            recursive: false,
            min_bytes: None,
        }
    }

    #[test]
    fn a_file_check_takes_a_full_path_of_any_system_and_a_name_pattern() {
        let json = r#"{"kind":"file","path":"D:\\Backups","pattern":"*.bak"}"#;
        let spec = file("D:\\Backups", "*.bak");
        assert_eq!(serde_json::to_string(&spec).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), spec);
        assert_eq!(spec.target(), "D:\\Backups (*.bak)");
        assert_eq!(file("/srv/dump.sql", "").target(), "/srv/dump.sql");
        let full = CheckSpec::File {
            path: "/var/backups".into(),
            pattern: String::new(),
            recursive: true,
            min_bytes: Some(1_048_576),
        };
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::json!({ "kind": "file", "path": "/var/backups", "recursive": true, "min_bytes": 1048576 })
        );

        for path in [
            "/var/backups",
            "D:\\Backups",
            "c:/backups",
            "\\\\nas\\backups\\sql",
        ] {
            assert_eq!(file(path, "").validate(), Ok(()), "{path}");
        }
        for (path, pattern, why) in [
            ("", "", "can't be empty"),
            ("backups", "", "full one"),
            ("D:Backups", "", "full one"),
            ("./backups", "*.bak", "full one"),
            ("/var/backups", "sql/*.bak", "file names"),
            ("/var/backups", " *.bak", "file names"),
        ] {
            let error = file(path, pattern).validate().expect_err(path);
            assert!(error.0.contains(why), "{path} {pattern}: {error}");
        }
    }

    fn http_with(change: impl FnOnce(&mut HttpOptions)) -> CheckSpec {
        let mut options = HttpOptions::default();
        change(&mut options);
        CheckSpec::Http {
            url: "https://example.com/health".into(),
            options,
        }
    }

    #[test]
    fn a_plain_http_check_is_only_its_url_and_options_sit_beside_it() {
        let plain = r#"{"kind":"http","url":"https://example.com/health"}"#;
        assert_eq!(serde_json::to_string(&http_with(|_| {})).unwrap(), plain);
        assert_eq!(
            serde_json::from_str::<CheckSpec>(plain).unwrap(),
            http_with(|_| {})
        );

        let full = http_with(|o| {
            o.method = HttpMethod::Head;
            o.status = "200, 301-308".into();
            o.follow_redirects = false;
            o.verify = false;
            o.headers = vec![HttpHeader {
                name: "Accept".into(),
                value: "text/html".into(),
            }];
        });
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "http", "url": "https://example.com/health", "method": "head",
                "status": "200, 301-308", "follow_redirects": false, "verify": false,
                "headers": [{ "name": "Accept", "value": "text/html" }],
            })
        );
        assert_eq!(serde_json::from_value::<CheckSpec>(json).unwrap(), full);
        assert_eq!(full.validate(), Ok(()));
    }

    #[test]
    fn expected_status_codes_are_codes_and_ranges() {
        assert_eq!(status_ranges("").unwrap(), vec![200..=299]);
        assert_eq!(
            status_ranges(" 200 , 301-308,401").unwrap(),
            vec![200..=200, 301..=308, 401..=401]
        );
        for (text, why) in [
            ("ok", "isn't a status code"),
            ("99", "isn't a status code"),
            ("200-", "isn't a status code"),
            ("200,,204", "isn't a status code"),
            ("299-200", "backwards"),
        ] {
            let error = status_ranges(text).expect_err(text);
            assert!(error.0.contains(why), "{text}: {error}");
        }
        let any = HttpOptions::default();
        assert!(any.expects(204) && !any.expects(301) && !any.expects(500));
        let login = HttpOptions {
            status: "401, 403".into(),
            ..HttpOptions::default()
        };
        assert!(login.expects(401) && !login.expects(200));
        assert_eq!(login.expected_status(), "401, 403");
        assert_eq!(any.expected_status(), "200-299");
    }

    #[test]
    fn http_options_are_checked() {
        let header = |name: &str, value: &str| HttpHeader {
            name: name.into(),
            value: value.into(),
        };
        let cases: [(CheckSpec, &str); 6] = [
            (http_with(|o| o.status = "2xx".into()), "status code"),
            (
                http_with(|o| o.contains = "x".repeat(MAX_KEYWORD_CHARS + 1)),
                "longer than",
            ),
            (
                http_with(|o| {
                    o.method = HttpMethod::Head;
                    o.absent = "error".into();
                }),
                "HEAD",
            ),
            (
                http_with(|o| o.headers = vec![header("X Y", "1")]),
                "header name",
            ),
            (
                http_with(|o| o.headers = vec![header("X-Y", "a\r\nb")]),
                "control characters",
            ),
            (
                http_with(|o| o.headers = vec![header("A", "1"); MAX_HTTP_HEADERS + 1]),
                "headers",
            ),
        ];
        for (spec, expected) in cases {
            let error = spec.validate().expect_err(&format!("{spec:?}"));
            assert!(error.0.contains(expected), "{spec:?}: {error}");
        }
        let fine = http_with(|o| {
            o.contains = "Welcome".into();
            o.absent = "Maintenance".into();
            o.headers = vec![header("Accept-Language", "en")];
        });
        assert_eq!(fine.validate(), Ok(()));
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
            CheckSpec::http("https://example.com"),
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
            file("/var/backups", "*.bak"),
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
            (CheckSpec::http("example.com"), "http://"),
            (CheckSpec::http("https://"), "needs a host"),
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
