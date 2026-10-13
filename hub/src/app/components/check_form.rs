//! The add/edit dialog for a check (M2-05). The form shows the settings of
//! the chosen kind, and checks them with the same code as the hub and the
//! probe (`common::checks`) before anything is sent.

use common::checks::{
    CHECK_KINDS, CheckSpec, DEFAULT_PING_COUNT, DEFAULT_PROCESS_MIN, DEFAULT_SNMP_PORT,
    DEFAULT_STALE_DAYS, DEFAULT_TLS_PORT, DEFAULT_UPDATE_DAYS, DbEngine, DnsRecord, HttpHeader,
    HttpMethod, HttpOptions, MAX_PROCESS_COUNT, MAX_UPDATE_DAYS, Secrets, SnmpAuth, SnmpPrivacy,
    SnmpVersion, secret,
};
use common::checks::{
    DEFAULT_EVENT_COUNT, DEFAULT_EVENT_MINUTES, EventLevel, MAX_EVENT_COUNT, MAX_EVENT_MINUTES,
};
use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::post_for;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::model::{
    AlertAfter, CheckInfo, CheckRequest, Limit, MetricLimits, ResolvedLimits, ThresholdLevel,
    ThresholdMetric,
};

/// A kind as the UI names it.
pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "ping" => "Ping",
        "http" => "Web page (HTTP)",
        "tcp" => "TCP port",
        "tls" => "TLS certificate",
        "dns" => "DNS name",
        "database" => "Database server (PostgreSQL, MySQL, SQL Server, MongoDB)",
        "snmp" => "SNMP (a switch, a printer, a NAS, a UPS)",
        "cpu" => "CPU usage",
        "memory" => "Memory usage",
        "disk" => "Disk usage (one disk)",
        "disks" => "Disk usage (every disk, found by the probe)",
        "smart" => "Disk health (SMART: what the drives say of themselves)",
        "temperature" => "Temperature (the machine's own sensors: CPU, board, drives)",
        "file" => "File or folder (a backup that must be recent)",
        "service" => "Service (a Windows service, a systemd unit)",
        "process" => "Process (a program that must run: how many, their CPU and memory)",
        "interface" => "Network interface (its link, and what goes through it)",
        "eventlog" => "Windows event log (errors that appear, or an event that must)",
        "updates" => "Pending updates and restart (Windows, Linux; only reported)",
        "uptime" => "Uptime",
        _ => "Unknown",
    }
}

/// The form's text fields. Only those of the chosen kind are shown and used.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CheckFields {
    pub kind: String,
    pub host: String,
    pub port: String,
    pub url: String,
    pub path: String,
    /// `tls`, `http`: accept a certificate a client wouldn't.
    pub unverified: bool,
    /// `http`: `get` or `head` (empty: `get`).
    pub method: String,
    /// `http`: the status codes expected (empty: 200-299).
    pub status: String,
    /// `http`: text the page must, and must not, contain.
    pub contains: String,
    pub absent: String,
    /// `http`: a redirect is the answer.
    pub no_redirects: bool,
    /// `http`: the headers to send, one `Name: value` a line.
    pub headers: String,
    /// `ping`: how many echoes (empty: 4).
    pub count: String,
    /// `ping`: the TCP port to try without an answer (empty: none).
    pub tcp_port: String,
    /// `service`: the service's name.
    pub service: String,
    /// `interface`: the interface's name.
    pub interface: String,
    /// `database`: the engine, as in the JSON (empty: the first one).
    pub engine: String,
    /// `dns`: the record type, as in the JSON (empty: `a`).
    pub record: String,
    /// `dns`: the server to ask (empty: the machine's).
    pub server: String,
    /// `dns`: the answers that are fine, separated by commas (empty: any).
    pub expect: String,
    /// `disks`: the mount points to leave out, separated by commas.
    pub ignore: String,
    /// `file`: the names that count in a folder (empty: every file).
    pub pattern: String,
    /// `file`: the subfolders too.
    pub recursive: bool,
    /// `file`: the smallest size that is fine, in MB (empty: any).
    pub min_mb: String,
    /// `eventlog`: the log, the source, the event IDs (separated by commas;
    /// empty: any), the level as in the JSON (empty: `error`), how far back
    /// in minutes (empty: 60), how many events (empty: 1), and the two
    /// choices of what they mean.
    pub log: String,
    pub source: String,
    pub event_ids: String,
    pub level: String,
    pub minutes: String,
    pub event_count: String,
    pub wanted: bool,
    pub as_warning: bool,
    /// `process`: the program's name, text of its command line, and the
    /// fewest and the most that may run (empty: 1, and any number).
    pub process: String,
    pub command: String,
    pub min_count: String,
    pub max_count: String,
    /// `updates`: its limits in days ([`parse_update_days`]).
    pub security_days: String,
    pub reboot_days: String,
    pub stale_days: String,
    /// `snmp`: `v2c` or `v3` (empty: `v2c`).
    pub version: String,
    /// `snmp` v3: the user, how it is authenticated and the exchange
    /// encrypted (empty: not).
    pub user: String,
    pub auth: String,
    pub privacy: String,
    /// `snmp`: the values to ask for, one a line
    /// ([`common::snmp::parse_values`]).
    pub oids: String,
    /// `snmp`: its secrets as typed. Empty: keep the one the check has.
    pub community: String,
    pub auth_password: String,
    pub priv_password: String,
}

const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// A size typed in megabytes (`500`, `0.5`), in bytes; empty: none.
fn parse_min_mb(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<f64>()
        .ok()
        .filter(|mb| mb.is_finite() && *mb > 0.0 && *mb <= 1e9)
        .map(|mb| Some((mb * BYTES_PER_MB).round() as u64))
        .ok_or_else(|| format!("{text}: the smallest size is a number of MB, e.g. 500 or 0.5"))
}

fn min_mb_text(bytes: Option<u64>) -> String {
    bytes
        // Not rounded: dividing by a power of two is exact, so the text gives
        // back the same bytes (rounding it made 512 bytes 524 on the next save).
        .map(|bytes| format!("{}", bytes as f64 / BYTES_PER_MB))
        .unwrap_or_default()
}

/// Event IDs as typed: numbers separated by commas or spaces.
fn parse_event_ids(text: &str) -> Result<Vec<u32>, String> {
    text.split([',', ' ', ';'])
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| {
            id.parse::<u32>()
                .map_err(|_| format!("{id}: an event ID is a number, e.g. 7, 51, 153"))
        })
        .collect()
}

/// A whole number of an `eventlog` check as typed (`1..=most`); empty:
/// `usual`.
fn parse_event_number(text: &str, what: &str, usual: u32, most: u32) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(usual);
    }
    text.parse::<u32>()
        .ok()
        .filter(|n| (1..=most).contains(n))
        .ok_or_else(|| format!("{text}: {what} is a number, 1-{most}"))
}

/// A number of processes as typed; empty: `usual`.
fn parse_process_count<T>(text: &str, what: &str, usual: T) -> Result<T, String>
where
    T: From<u32>,
{
    let text = text.trim();
    if text.is_empty() {
        return Ok(usual);
    }
    text.parse::<u32>()
        .ok()
        .filter(|count| *count <= MAX_PROCESS_COUNT)
        .map(T::from)
        .ok_or_else(|| format!("{text}: {what} is a number, 0-{MAX_PROCESS_COUNT}"))
}

/// What turns a limit of an `updates` check off, typed in its field.
const NEVER: &str = "never";

/// A limit of an `updates` check as typed, in days: empty is the `usual`
/// one, `never` none.
fn parse_update_days(text: &str, usual: u32) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Some(usual));
    }
    if text.eq_ignore_ascii_case(NEVER) {
        return Ok(None);
    }
    text.parse::<u32>()
        .ok()
        .filter(|days| *days <= MAX_UPDATE_DAYS)
        .map(Some)
        .ok_or_else(|| {
            format!("{text}: a limit is a number of days (0-{MAX_UPDATE_DAYS}), or \"{NEVER}\"")
        })
}

fn update_days_text(days: Option<u32>, usual: u32) -> String {
    match days {
        None => NEVER.to_string(),
        Some(days) if days == usual => String::new(),
        Some(days) => days.to_string(),
    }
}

/// Headers as typed: one `Name: value` a line; empty lines don't count.
fn parse_headers(text: &str) -> Result<Vec<HttpHeader>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("\"{line}\": a header is written Name: value"))?;
            Ok(HttpHeader {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
            })
        })
        .collect()
}

fn headers_text(headers: &[HttpHeader]) -> String {
    headers
        .iter()
        .map(|h| format!("{}: {}", h.name, h.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Which inputs a kind needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Inputs {
    pub host: bool,
    pub port: bool,
    pub url: bool,
    pub path: bool,
    pub unverified: bool,
    /// The `http` settings beyond the URL.
    pub http: bool,
    /// The `file` settings.
    pub file: bool,
    /// The `updates` limits.
    pub updates: bool,
    /// The `process` settings.
    pub process: bool,
    /// The `eventlog` settings.
    pub eventlog: bool,
    /// The `ping` settings beyond the host.
    pub ping: bool,
    /// The `disks` setting.
    pub disks: bool,
    /// The `smart` setting.
    pub smart: bool,
    /// The `temperature` setting.
    pub temperature: bool,
    /// The `dns` settings.
    pub dns: bool,
    /// The `service` setting.
    pub service: bool,
    /// The `interface` setting.
    pub interface: bool,
    /// The `snmp` settings beyond the host.
    pub snmp: bool,
    /// The `database` check's engine.
    pub database: bool,
}

/// A list typed with commas between its items; empty items don't count.
fn comma_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

impl CheckFields {
    pub fn from_spec(spec: &CheckSpec) -> Self {
        let mut fields = Self {
            kind: spec.kind().to_string(),
            ..Self::default()
        };
        match spec {
            CheckSpec::Ping {
                host,
                count,
                tcp_port,
            } => {
                fields.host = host.clone();
                if *count != DEFAULT_PING_COUNT {
                    fields.count = count.to_string();
                }
                fields.tcp_port = tcp_port.map(|p| p.to_string()).unwrap_or_default();
            }
            CheckSpec::Http { url, options } => {
                fields.url = url.clone();
                fields.method = options.method.as_str().to_string();
                fields.status = options.status.clone();
                fields.contains = options.contains.clone();
                fields.absent = options.absent.clone();
                fields.no_redirects = !options.follow_redirects;
                fields.headers = headers_text(&options.headers);
                fields.unverified = !options.verify;
            }
            CheckSpec::Tcp { host, port } => {
                fields.host = host.clone();
                fields.port = port.to_string();
            }
            CheckSpec::Database { engine, host, port } => {
                fields.engine = engine.as_str().to_string();
                fields.host = host.clone();
                fields.port = port.map(|port| port.to_string()).unwrap_or_default();
            }
            CheckSpec::Tls { host, port, verify } => {
                fields.host = host.clone();
                fields.port = port.to_string();
                fields.unverified = !verify;
            }
            CheckSpec::Disk { path } => fields.path = path.clone().unwrap_or_default(),
            CheckSpec::Disks { ignore } | CheckSpec::Smart { ignore } => {
                fields.ignore = ignore.join(", ")
            }
            CheckSpec::Temperature { sensors } => fields.pattern = sensors.join(", "),
            CheckSpec::Dns {
                name,
                record,
                server,
                expect,
            } => {
                fields.host = name.clone();
                fields.record = record.as_str().to_string();
                fields.server = server.clone().unwrap_or_default();
                fields.expect = expect.join(", ");
            }
            CheckSpec::File {
                path,
                pattern,
                recursive,
                min_bytes,
            } => {
                fields.path = path.clone();
                fields.pattern = pattern.clone();
                fields.recursive = *recursive;
                fields.min_mb = min_mb_text(*min_bytes);
            }
            CheckSpec::Service { name } => fields.service = name.clone(),
            CheckSpec::Interface { name } => fields.interface = name.clone(),
            CheckSpec::Snmp {
                host,
                port,
                version,
                user,
                auth,
                privacy,
                oids,
            } => {
                fields.host = host.clone();
                if *port != DEFAULT_SNMP_PORT {
                    fields.port = port.to_string();
                }
                fields.version = version.as_str().to_string();
                fields.user = user.clone();
                fields.auth = auth.map(|a| a.as_str().to_string()).unwrap_or_default();
                fields.privacy = privacy.map(|p| p.as_str().to_string()).unwrap_or_default();
                fields.oids = common::snmp::values_text(oids);
            }
            CheckSpec::Eventlog {
                log,
                source,
                ids,
                level,
                minutes,
                count,
                wanted,
                warning,
            } => {
                fields.log = log.clone();
                fields.source = source.clone();
                fields.event_ids = ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                fields.level = level.as_str().to_string();
                if *minutes != DEFAULT_EVENT_MINUTES {
                    fields.minutes = minutes.to_string();
                }
                if *count != DEFAULT_EVENT_COUNT {
                    fields.event_count = count.to_string();
                }
                fields.wanted = *wanted;
                fields.as_warning = *warning;
            }
            CheckSpec::Process {
                name,
                command,
                min,
                max,
            } => {
                fields.process = name.clone();
                fields.command = command.clone();
                if *min != DEFAULT_PROCESS_MIN {
                    fields.min_count = min.to_string();
                }
                fields.max_count = max.map(|max| max.to_string()).unwrap_or_default();
            }
            CheckSpec::Updates {
                security_days,
                reboot_days,
                stale_days,
            } => {
                fields.security_days = update_days_text(*security_days, DEFAULT_UPDATE_DAYS);
                fields.reboot_days = update_days_text(*reboot_days, DEFAULT_UPDATE_DAYS);
                fields.stale_days = update_days_text(*stale_days, DEFAULT_STALE_DAYS);
            }
            CheckSpec::Cpu | CheckSpec::Memory | CheckSpec::Uptime => {}
        }
        fields
    }

    /// Is this an SNMP v3 check, as the form stands?
    pub fn is_v3(&self) -> bool {
        self.version == SnmpVersion::V3.as_str()
    }

    /// The secrets typed for this check (D-86): only those its kind and
    /// settings use, and only the ones typed (an empty one keeps what the
    /// check has). `None`: none typed.
    pub fn secrets(&self) -> Option<Secrets> {
        if self.kind != "snmp" {
            return None;
        }
        let typed: &[(&str, &String, bool)] = &[
            (secret::COMMUNITY, &self.community, !self.is_v3()),
            (
                secret::AUTH_PASSWORD,
                &self.auth_password,
                self.is_v3() && !self.auth.is_empty(),
            ),
            (
                secret::PRIV_PASSWORD,
                &self.priv_password,
                self.is_v3() && !self.auth.is_empty() && !self.privacy.is_empty(),
            ),
        ];
        let given: std::collections::BTreeMap<String, String> = typed
            .iter()
            .filter(|(_, value, used)| *used && !value.is_empty())
            .map(|(name, value, _)| (name.to_string(), value.to_string()))
            .collect();
        (!given.is_empty()).then_some(Secrets(given))
    }

    pub fn inputs(&self) -> Inputs {
        match self.kind.as_str() {
            "ping" => Inputs {
                host: true,
                ping: true,
                ..Inputs::default()
            },
            "tcp" => Inputs {
                host: true,
                port: true,
                ..Inputs::default()
            },
            "database" => Inputs {
                database: true,
                host: true,
                port: true,
                ..Inputs::default()
            },
            "tls" => Inputs {
                host: true,
                port: true,
                unverified: true,
                ..Inputs::default()
            },
            "http" => Inputs {
                url: true,
                http: true,
                unverified: true,
                ..Inputs::default()
            },
            "disk" => Inputs {
                path: true,
                ..Inputs::default()
            },
            "disks" => Inputs {
                disks: true,
                ..Inputs::default()
            },
            "smart" => Inputs {
                smart: true,
                ..Inputs::default()
            },
            "temperature" => Inputs {
                temperature: true,
                ..Inputs::default()
            },
            "dns" => Inputs {
                dns: true,
                ..Inputs::default()
            },
            "service" => Inputs {
                service: true,
                ..Inputs::default()
            },
            "interface" => Inputs {
                interface: true,
                ..Inputs::default()
            },
            "snmp" => Inputs {
                host: true,
                snmp: true,
                ..Inputs::default()
            },
            "file" => Inputs {
                file: true,
                ..Inputs::default()
            },
            "eventlog" => Inputs {
                eventlog: true,
                ..Inputs::default()
            },
            "process" => Inputs {
                process: true,
                ..Inputs::default()
            },
            "updates" => Inputs {
                updates: true,
                ..Inputs::default()
            },
            _ => Inputs::default(),
        }
    }

    /// The spec these fields describe, if it's valid (`CheckSpec::validate`).
    pub fn to_spec(&self) -> Result<CheckSpec, String> {
        let text = |value: &str| value.trim().to_string();
        let port = |default: Option<u16>| match (self.port.trim(), default) {
            ("", Some(default)) => Ok(default),
            (port, _) => port
                .parse()
                .map_err(|_| "the port must be 1-65535".to_string()),
        };
        let spec = match self.kind.as_str() {
            "ping" => CheckSpec::Ping {
                host: text(&self.host),
                count: match self.count.trim() {
                    "" => DEFAULT_PING_COUNT,
                    count => count
                        .parse()
                        .map_err(|_| format!("{count}: how many echoes is a number"))?,
                },
                tcp_port: match self.tcp_port.trim() {
                    "" => None,
                    port => Some(
                        port.parse()
                            .map_err(|_| "the TCP port must be 1-65535".to_string())?,
                    ),
                },
            },
            "http" => CheckSpec::Http {
                url: text(&self.url),
                options: HttpOptions {
                    method: match self.method.as_str() {
                        "" => HttpMethod::Get,
                        method => HttpMethod::parse(method)
                            .ok_or_else(|| "the method is GET or HEAD".to_string())?,
                    },
                    status: text(&self.status),
                    // As typed: spaces can be part of what to look for.
                    contains: self.contains.clone(),
                    absent: self.absent.clone(),
                    follow_redirects: !self.no_redirects,
                    headers: parse_headers(&self.headers)?,
                    verify: !self.unverified,
                },
            },
            "tcp" => CheckSpec::Tcp {
                host: text(&self.host),
                port: port(None)?,
            },
            "database" => CheckSpec::Database {
                engine: match self.engine.as_str() {
                    "" => DbEngine::ALL[0],
                    engine => DbEngine::parse(engine)
                        .ok_or_else(|| "choose the kind of database".to_string())?,
                },
                host: text(&self.host),
                // Empty: the engine's usual port.
                port: match self.port.trim() {
                    "" => None,
                    _ => Some(port(None)?),
                },
            },
            "tls" => CheckSpec::Tls {
                host: text(&self.host),
                port: port(Some(DEFAULT_TLS_PORT))?,
                verify: !self.unverified,
            },
            "cpu" => CheckSpec::Cpu,
            "memory" => CheckSpec::Memory,
            "disk" => CheckSpec::Disk {
                path: Some(text(&self.path)).filter(|p| !p.is_empty()),
            },
            "disks" => CheckSpec::Disks {
                ignore: comma_list(&self.ignore),
            },
            "smart" => CheckSpec::Smart {
                ignore: comma_list(&self.ignore),
            },
            "temperature" => CheckSpec::Temperature {
                sensors: comma_list(&self.pattern),
            },
            "dns" => CheckSpec::Dns {
                name: text(&self.host),
                record: match self.record.as_str() {
                    "" => DnsRecord::default(),
                    record => DnsRecord::parse(record)
                        .ok_or_else(|| "choose the record type".to_string())?,
                },
                server: Some(text(&self.server)).filter(|s| !s.is_empty()),
                expect: comma_list(&self.expect),
            },
            "file" => CheckSpec::File {
                path: text(&self.path),
                pattern: text(&self.pattern),
                recursive: self.recursive,
                min_bytes: parse_min_mb(&self.min_mb)?,
            },
            "service" => CheckSpec::Service {
                name: text(&self.service),
            },
            "interface" => CheckSpec::Interface {
                name: text(&self.interface),
            },
            "snmp" => {
                let v3 = self.is_v3();
                CheckSpec::Snmp {
                    host: text(&self.host),
                    port: port(Some(DEFAULT_SNMP_PORT))?,
                    version: if v3 {
                        SnmpVersion::V3
                    } else {
                        SnmpVersion::V2c
                    },
                    // What v2c doesn't have is left out, whatever was typed before.
                    user: if v3 { text(&self.user) } else { String::new() },
                    auth: match (v3, self.auth.as_str()) {
                        (false, _) | (_, "") => None,
                        (_, auth) => {
                            Some(SnmpAuth::parse(auth).ok_or_else(|| {
                                "choose how the user is authenticated".to_string()
                            })?)
                        }
                    },
                    privacy: match (v3 && !self.auth.is_empty(), self.privacy.as_str()) {
                        (false, _) | (_, "") => None,
                        (_, privacy) => {
                            Some(SnmpPrivacy::parse(privacy).ok_or_else(|| {
                                "choose how the exchange is encrypted".to_string()
                            })?)
                        }
                    },
                    oids: common::snmp::parse_values(&self.oids)?,
                }
            }
            "uptime" => CheckSpec::Uptime,
            "eventlog" => CheckSpec::Eventlog {
                log: text(&self.log),
                source: text(&self.source),
                ids: parse_event_ids(&self.event_ids)?,
                level: match self.level.as_str() {
                    "" => EventLevel::default(),
                    level => EventLevel::parse(level)
                        .ok_or_else(|| "choose which events count".to_string())?,
                },
                minutes: parse_event_number(
                    &self.minutes,
                    "how far back to look, in minutes,",
                    DEFAULT_EVENT_MINUTES,
                    MAX_EVENT_MINUTES,
                )?,
                count: parse_event_number(
                    &self.event_count,
                    "the number of events",
                    DEFAULT_EVENT_COUNT,
                    MAX_EVENT_COUNT,
                )?,
                wanted: self.wanted,
                warning: self.as_warning,
            },
            "process" => CheckSpec::Process {
                name: text(&self.process),
                command: text(&self.command),
                min: parse_process_count(
                    &self.min_count,
                    "the fewest that must run",
                    DEFAULT_PROCESS_MIN,
                )?,
                max: parse_process_count(&self.max_count, "the most that may run", None::<u32>)?,
            },
            "updates" => CheckSpec::Updates {
                security_days: parse_update_days(&self.security_days, DEFAULT_UPDATE_DAYS)?,
                reboot_days: parse_update_days(&self.reboot_days, DEFAULT_UPDATE_DAYS)?,
                stale_days: parse_update_days(&self.stale_days, DEFAULT_STALE_DAYS)?,
            },
            _ => return Err("choose what to check".into()),
        };
        spec.validate().map_err(|e| e.to_string())?;
        Ok(spec)
    }
}

/// A count typed in the form: empty is the default (`None`).
fn optional_count(text: &str) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| format!("{text} isn't a whole number"))
}

/// When a check alerts, as typed: failed and good results in a row.
pub(crate) fn alert_after(fail_after: &str, recover_after: &str) -> Result<AlertAfter, String> {
    let alert_after = AlertAfter {
        fail_after: optional_count(fail_after)?,
        recover_after: optional_count(recover_after)?,
    };
    alert_after.validate()?;
    Ok(alert_after)
}

/// A check's own limits as typed, for its kind's metric (none for a kind
/// without thresholds, whatever was typed).
pub(crate) fn check_limits(kind: &str, warning: &str, down: &str) -> Result<MetricLimits, String> {
    match ThresholdMetric::of_kind(kind) {
        Some(metric) => MetricLimits::from_text(metric, warning, down),
        None => Ok(MetricLimits::default()),
    }
}

/// The request for a check called `name` with these fields, checked like
/// the hub checks it. `depends_on`: the ID of the check it is behind ("": none).
pub(crate) fn check_request(
    name: &str,
    fields: &CheckFields,
    enabled: bool,
    alert_after: AlertAfter,
    limits: MetricLimits,
    depends_on: &str,
) -> Result<CheckRequest, String> {
    common::checks::validate_name(name.trim()).map_err(|e| e.to_string())?;
    Ok(CheckRequest {
        name: name.trim().to_string(),
        spec: fields.to_spec()?,
        enabled,
        alert_after,
        limits,
        depends_on: Some(depends_on.trim().to_string()).filter(|d| !d.is_empty()),
        secrets: fields.secrets(),
    })
}

/// The request that enables a disabled `check`, or disables it: the whole
/// check with only `enabled` flipped. What a request leaves out is its
/// default, so sending less reset when it alerts, its limits and what it is
/// behind.
pub(crate) fn switched(check: &CheckInfo) -> CheckRequest {
    CheckRequest {
        name: check.name.clone(),
        spec: check.spec.clone(),
        enabled: !check.enabled,
        alert_after: check.alert_after,
        limits: check.limits,
        depends_on: check.depends_on.clone(),
        // None given: it keeps the ones it has.
        secrets: None,
    }
}

/// A count as the form shows it (empty: the default).
fn count_text(count: Option<u32>) -> String {
    count.map(|n| n.to_string()).unwrap_or_default()
}

/// Add a check to `probe_id` (`editing` = `None`) or change one. `checks`:
/// the probe's checks, to choose the one it is behind (D-68).
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn CheckDialog(
    probe_id: String,
    editing: Option<CheckInfo>,
    checks: Vec<CheckInfo>,
    open: RwSignal<bool>,
    saved: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let title = if editing.is_some() {
        "Edit check"
    } else {
        "Add check"
    };
    let name = RwSignal::new(editing.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    let enabled = RwSignal::new(editing.as_ref().is_none_or(|c| c.enabled));
    let current = editing.as_ref().map(|c| c.alert_after).unwrap_or_default();
    let fail_after = RwSignal::new(count_text(current.fail_after));
    let recover_after = RwSignal::new(count_text(current.recover_after));
    let (own_warning, own_down) = editing
        .as_ref()
        .map(|c| c.limits.texts())
        .unwrap_or_default();
    let warning_at = RwSignal::new(own_warning);
    let down_at = RwSignal::new(own_down);
    let depends_on = RwSignal::new(
        editing
            .as_ref()
            .and_then(|c| c.depends_on.clone())
            .unwrap_or_default(),
    );
    // What it can be behind: the probe's other checks (the hub refuses loops).
    let parents: Vec<(String, String)> = checks
        .into_iter()
        .filter(|c| editing.as_ref().is_none_or(|e| e.id != c.id))
        .map(|c| (c.id, c.name))
        .collect();
    // What applies when left empty: the levels above (D-64), if the hub said.
    let in_force = editing.as_ref().and_then(|c| c.limits_in_force);
    let inherited = move |pick: fn(&ResolvedLimits) -> Limit, metric: ThresholdMetric| {
        in_force
            .map(|l| pick(&l))
            .filter(|l| l.from != ThresholdLevel::Check)
            .map_or_else(|| "inherited".to_string(), |l| l.text(metric))
    };
    let fields = RwSignal::new(editing.as_ref().map_or_else(
        || CheckFields {
            kind: "ping".into(),
            ..CheckFields::default()
        },
        |c| CheckFields::from_spec(&c.spec),
    ));
    // The secrets the check has (their names): a field left empty keeps its.
    let kept = StoredValue::new(
        editing
            .as_ref()
            .map(|c| c.secrets_set.clone())
            .unwrap_or_default(),
    );
    let error = RwSignal::new(String::new());
    let url = match &editing {
        Some(check) => format!("/api/checks/{}", check.id),
        None => format!("/api/probes/{probe_id}/checks"),
    };
    let url = StoredValue::new(url);

    let save = move || {
        let kind = fields.with_untracked(|f| f.kind.clone());
        let request = match alert_after(&fail_after.get_untracked(), &recover_after.get_untracked())
            .and_then(|alert_after| {
                let limits =
                    check_limits(&kind, &warning_at.get_untracked(), &down_at.get_untracked())?;
                check_request(
                    &name.get_untracked(),
                    &fields.get_untracked(),
                    enabled.get_untracked(),
                    alert_after,
                    limits,
                    &depends_on.get_untracked(),
                )
            }) {
            Ok(request) => request,
            Err(e) => {
                error.set(e);
                return;
            }
        };
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = serde_json::to_string(&request).unwrap_or_default();
                match post_for::<CheckInfo>(&url.get_value(), &body).await {
                    Ok(_) => {
                        open.set(false);
                        saved();
                    }
                    Err(e) => error.set(e),
                }
            }
            #[cfg(feature = "ssr")]
            let _ = request;
        });
    };

    // One text input bound to a field of `fields`.
    let input = move |id: &'static str,
                      label: &'static str,
                      placeholder: &'static str,
                      get: fn(&CheckFields) -> String,
                      set: fn(&mut CheckFields, String)| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| fields.update(|f| set(f, input_value(&ev)));
        #[cfg(feature = "ssr")]
        let on_input = move |_| {
            let _ = set;
        };
        view! {
            <div class="form-group">
                <label for=id>{label}</label>
                <input id=id name=id type="text" placeholder=placeholder
                    prop:value=move || fields.with(get) on:input=on_input/>
            </div>
        }
    };
    // One secret: typed blind, never shown again (D-86).
    let secret_input = move |id: &'static str,
                             label: &'static str,
                             name: &'static str,
                             get: fn(&CheckFields) -> String,
                             set: fn(&mut CheckFields, String)| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| fields.update(|f| set(f, input_value(&ev)));
        #[cfg(feature = "ssr")]
        let on_input = move |_| {
            let _ = set;
        };
        let placeholder = if kept.with_value(|k| k.iter().any(|n| n == name)) {
            "saved: leave empty to keep it"
        } else {
            ""
        };
        view! {
            <div class="form-group">
                <label for=id>{label}</label>
                <input id=id name=id type="password" autocomplete="new-password"
                    placeholder=placeholder
                    prop:value=move || fields.with(get) on:input=on_input/>
            </div>
        }
    };
    #[cfg(not(feature = "ssr"))]
    let (on_version, on_auth, on_privacy, on_oids) = (
        move |ev| fields.update(|f| f.version = input_value(&ev)),
        move |ev| fields.update(|f| f.auth = input_value(&ev)),
        move |ev| fields.update(|f| f.privacy = input_value(&ev)),
        move |ev| fields.update(|f| f.oids = event_target_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_version, on_auth, on_privacy, on_oids) = (|_| {}, |_| {}, |_| {}, |_| {});
    #[cfg(not(feature = "ssr"))]
    let on_depends = move |ev| depends_on.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_depends = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_record = move |ev| fields.update(|f| f.record = input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_record = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_engine = move |ev| fields.update(|f| f.engine = input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_engine = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_level = move |ev| fields.update(|f| f.level = input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_level = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_wanted = move |ev| fields.update(|f| f.wanted = event_target_checked(&ev));
    #[cfg(feature = "ssr")]
    let on_wanted = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_as_warning = move |ev| fields.update(|f| f.as_warning = event_target_checked(&ev));
    #[cfg(feature = "ssr")]
    let on_as_warning = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_recursive = move |ev| fields.update(|f| f.recursive = event_target_checked(&ev));
    #[cfg(feature = "ssr")]
    let on_recursive = |_| {};
    #[cfg(not(feature = "ssr"))]
    let (on_unverified, on_no_redirects, on_method, on_headers) = (
        move |ev| fields.update(|f| f.unverified = event_target_checked(&ev)),
        move |ev| fields.update(|f| f.no_redirects = event_target_checked(&ev)),
        move |ev| fields.update(|f| f.method = input_value(&ev)),
        move |ev| fields.update(|f| f.headers = event_target_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_unverified, on_no_redirects, on_method, on_headers) = (|_| {}, |_| {}, |_| {}, |_| {});
    // What not verifying means depends on the kind.
    let unverified_label = move || {
        if fields.with(|f| f.kind == "tls") {
            " Only watch the expiry date: accept a certificate that is self-signed, from a private authority, or for another name"
        } else {
            " Accept any certificate over HTTPS (self-signed, a private authority, expired, another name)"
        }
    };
    #[cfg(not(feature = "ssr"))]
    let (on_name, on_kind, on_enabled, on_fail, on_recover, on_warning, on_down) = (
        move |ev| name.set(input_value(&ev)),
        move |ev| fields.update(|f| f.kind = input_value(&ev)),
        move |ev| enabled.set(event_target_checked(&ev)),
        move |ev| fail_after.set(input_value(&ev)),
        move |ev| recover_after.set(input_value(&ev)),
        move |ev| warning_at.set(input_value(&ev)),
        move |ev| down_at.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_name, on_kind, on_enabled, on_fail, on_recover, on_warning, on_down) =
        (|_| {}, |_| {}, |_| {}, |_| {}, |_| {}, |_| {}, |_| {});
    let metric = move || fields.with(|f| ThresholdMetric::of_kind(&f.kind));
    let default = AlertAfter::DEFAULT.to_string();
    let max = AlertAfter::MAX.to_string();
    let shown = move || fields.with(CheckFields::inputs);

    view! {
        <div class="modal-overlay" on:click=move |_| open.set(false)>
            <div class="modal check-dialog" on:click=|e| e.stop_propagation()>
                <h3>{title}</h3>
                <div class="form-group">
                    <label for="check_name">"Name"</label>
                    <input id="check_name" name="check_name" type="text" placeholder="e.g. Core switch"
                        prop:value=move || name.get() on:input=on_name/>
                </div>
                <div class="form-group">
                    <label for="check_kind">"What to check"</label>
                    <select id="check_kind" name="check_kind" prop:value=move || fields.with(|f| f.kind.clone()) on:change=on_kind>
                        {CHECK_KINDS.iter().map(|kind| view! {
                            <option value=*kind>{kind_label(kind)}</option>
                        }).collect_view()}
                    </select>
                </div>
                <Show when=move || shown().database>
                    <div class="form-group">
                        <label for="check_engine">"Database (the probe greets it in its own protocol; it logs in nowhere)"</label>
                        <select id="check_engine" name="check_engine"
                            prop:value=move || fields.with(|f| if f.engine.is_empty() { DbEngine::ALL[0].as_str().to_string() } else { f.engine.clone() })
                            on:change=on_engine>
                            {DbEngine::ALL.iter().map(|engine| view! {
                                <option value=engine.as_str()>{format!("{} (port {})", engine.label(), engine.default_port())}</option>
                            }).collect_view()}
                        </select>
                    </div>
                </Show>
                <Show when=move || shown().host>
                    {input("check_host", "Host (an IP address, or a name the probe's machine resolves)", "192.168.1.1, printer.lan", |f| f.host.clone(), |f, v| f.host = v)}
                </Show>
                <Show when=move || shown().ping>
                    {input("check_count", "Echoes each time (empty: 4)", "1-20", |f| f.count.clone(), |f, v| f.count = v)}
                    {input("check_tcp_port", "Without an answer, try this TCP port (a firewall that drops pings; empty: don't)", "e.g. 443", |f| f.tcp_port.clone(), |f, v| f.tcp_port = v)}
                </Show>
                <Show when=move || shown().port>
                    {input("check_port", "Port (empty: 443 for a TLS certificate, its usual one for a database)", "e.g. 443", |f| f.port.clone(), |f, v| f.port = v)}
                </Show>
                <Show when=move || shown().url>
                    {input("check_url", "URL", "https://example.com", |f| f.url.clone(), |f, v| f.url = v)}
                </Show>
                <Show when=move || shown().http>
                    <div class="form-group">
                        <label for="check_method">"Method"</label>
                        <select id="check_method" name="check_method"
                            prop:value=move || fields.with(|f| if f.method.is_empty() { "get".to_string() } else { f.method.clone() })
                            on:change=on_method>
                            <option value="get">"GET (the whole page)"</option>
                            <option value="head">"HEAD (headers only)"</option>
                        </select>
                    </div>
                    {input("check_status", "Status codes that are fine (empty: 200-299)", "e.g. 200, 301-308, 401", |f| f.status.clone(), |f, v| f.status = v)}
                    {input("check_contains", "The page must contain", "e.g. Sign in", |f| f.contains.clone(), |f, v| f.contains = v)}
                    {input("check_absent", "The page must not contain", "e.g. Maintenance", |f| f.absent.clone(), |f, v| f.absent = v)}
                    <div class="form-group">
                        <label for="check_headers">"Headers to send (one a line; not for passwords or tokens: everyone who sees the check sees them)"</label>
                        <textarea id="check_headers" name="check_headers" rows="2" placeholder="Accept-Language: en"
                            prop:value=move || fields.with(|f| f.headers.clone()) on:input=on_headers></textarea>
                    </div>
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_no_redirects"
                                prop:checked=move || fields.with(|f| f.no_redirects) on:change=on_no_redirects/>
                            " Don't follow redirects (the redirect is the answer: expect its 3xx status)"
                        </label>
                    </div>
                </Show>
                <Show when=move || shown().service>
                    {input("check_service", "Service on the probe's machine. Windows: its service name, not its display name (Spooler, not Print Spooler). Linux: the systemd unit", "Spooler, MSSQLSERVER, nginx, backup.timer", |f| f.service.clone(), |f, v| f.service = v)}
                </Show>
                <Show when=move || shown().interface>
                    {input("check_interface", "Network interface of the probe's machine, by its name there (the probe's page lists them under Machine). Down when its link is; what goes in and out, and errors, are shown", "Ethernet, Wi-Fi, eth0, enp3s0", |f| f.interface.clone(), |f, v| f.interface = v)}
                </Show>
                <Show when=move || shown().snmp>
                    {input("check_snmp_port", "UDP port (empty: 161)", "161", |f| f.port.clone(), |f, v| f.port = v)}
                    <div class="form-group">
                        <label for="check_snmp_version">"SNMP version"</label>
                        <select id="check_snmp_version" name="check_snmp_version"
                            prop:value=move || fields.with(|f| if f.is_v3() { "v3" } else { "v2c" })
                            on:change=on_version>
                            {SnmpVersion::ALL.iter().map(|version| view! {
                                <option value=version.as_str()>{version.label()}</option>
                            }).collect_view()}
                        </select>
                    </div>
                    <Show when=move || !fields.with(CheckFields::is_v3)>
                        {secret_input("check_community", "Community (sent in the clear on the probe's network; nobody can read it back here)", secret::COMMUNITY, |f| f.community.clone(), |f, v| f.community = v)}
                    </Show>
                    <Show when=move || fields.with(CheckFields::is_v3)>
                        {input("check_snmp_user", "User", "e.g. monitor", |f| f.user.clone(), |f, v| f.user = v)}
                        <div class="form-group">
                            <label for="check_snmp_auth">"Authentication"</label>
                            <select id="check_snmp_auth" name="check_snmp_auth"
                                prop:value=move || fields.with(|f| f.auth.clone()) on:change=on_auth>
                                <option value="">"None"</option>
                                {SnmpAuth::ALL.iter().map(|auth| view! {
                                    <option value=auth.as_str()>{auth.label()}</option>
                                }).collect_view()}
                            </select>
                        </div>
                        <Show when=move || fields.with(|f| !f.auth.is_empty())>
                            {secret_input("check_auth_password", "Authentication password (8 characters or more)", secret::AUTH_PASSWORD, |f| f.auth_password.clone(), |f, v| f.auth_password = v)}
                            <div class="form-group">
                                <label for="check_snmp_privacy">"Privacy (encryption)"</label>
                                <select id="check_snmp_privacy" name="check_snmp_privacy"
                                    prop:value=move || fields.with(|f| f.privacy.clone()) on:change=on_privacy>
                                    <option value="">"None"</option>
                                    {SnmpPrivacy::ALL.iter().map(|privacy| view! {
                                        <option value=privacy.as_str()>{privacy.label()}</option>
                                    }).collect_view()}
                                </select>
                            </div>
                            <Show when=move || fields.with(|f| !f.privacy.is_empty())>
                                {secret_input("check_priv_password", "Privacy password (8 characters or more)", secret::PRIV_PASSWORD, |f| f.priv_password.clone(), |f, v| f.priv_password = v)}
                            </Show>
                        </Show>
                    </Show>
                    <div class="form-group">
                        <label for="check_oids">"Values to ask for, one a line: an OID in numbers; a name before it (Name = OID); what it must be after it (OID = value). Then options after a semicolon: a table (walk, rows OID), a share (of OID), a rate (per second), a factor (x 0.1), a unit, meanings (means 1=up, 2=down), what it must not be (not down), and limits (warn < 20 ; down < 5)"</label>
                        <textarea id="check_oids" name="check_oids" rows="6"
                            placeholder="1.3.6.1.2.1.1.3.0\nUplink = 1.3.6.1.2.1.2.2.1.8.1 = 1"
                            prop:value=move || fields.with(|f| f.oids.clone()) on:input=on_oids></textarea>
                    </div>
                </Show>
                <Show when=move || shown().dns>
                    {input("check_dns_name", "Name to look up", "intranet.example.com", |f| f.host.clone(), |f, v| f.host = v)}
                    <div class="form-group">
                        <label for="check_record">"Record"</label>
                        <select id="check_record" name="check_record"
                            prop:value=move || fields.with(|f| if f.record.is_empty() { "a".to_string() } else { f.record.clone() })
                            on:change=on_record>
                            {DnsRecord::ALL.into_iter().map(|record| view! {
                                <option value=record.as_str()>{record.label()}</option>
                            }).collect_view()}
                        </select>
                    </div>
                    {input("check_server", "DNS server to ask (an IP address; empty: the ones the probe's machine uses)", "e.g. 192.168.1.1", |f| f.server.clone(), |f, v| f.server = v)}
                    {input("check_expect", "Answers that are fine (separated by commas; empty: any answer)", "192.168.1.20, 192.168.1.21", |f| f.expect.clone(), |f, v| f.expect = v)}
                </Show>
                <Show when=move || shown().disks>
                    {input("check_ignore", "Disks to leave out (separated by commas; empty: none). Removable, read-only and network disks never count", "E:, /mnt/scratch, /mnt/*", |f| f.ignore.clone(), |f, v| f.ignore = v)}
                </Show>
                <Show when=move || shown().smart>
                    {input("check_smart_ignore", "Drives to leave out, by model or serial number (separated by commas; * for any characters; empty: none). Every drive of the probe's machine that reports its health counts: Down when one says it is failing, a Warning for bad sectors and other signs. The disks of a virtual machine report none", "WDC WD40*, S3Z9NB0K", |f| f.ignore.clone(), |f, v| f.ignore = v)}
                </Show>
                <Show when=move || shown().temperature>
                    {input("check_sensors", "Sensors that count, by name (separated by commas; * for any characters; empty: every one). The hottest one's temperature is judged; a sensor at the critical limit of its own hardware is Down. A virtual machine has no sensors, and Windows shows only the firmware's own, where there is one", "coretemp*, acpitz*", |f| f.pattern.clone(), |f, v| f.pattern = v)}
                </Show>
                <Show when=move || shown().eventlog>
                    {input("check_log", "Event log of the probe's Windows machine", "System, Application, Microsoft-Windows-Backup", |f| f.log.clone(), |f, v| f.log = v)}
                    {input("check_source", "Only events from this source (empty: any)", "disk, Ntfs, Service Control Manager", |f| f.source.clone(), |f, v| f.source = v)}
                    {input("check_event_ids", "Only these event IDs (separated by commas; empty: any)", "7, 51, 153", |f| f.event_ids.clone(), |f, v| f.event_ids = v)}
                    <div class="form-group">
                        <label for="check_level">"Which events count"</label>
                        <select id="check_level" name="check_level"
                            prop:value=move || fields.with(|f| if f.level.is_empty() { EventLevel::default().as_str().to_string() } else { f.level.clone() })
                            on:change=on_level>
                            {EventLevel::ALL.iter().map(|level| view! {
                                <option value=level.as_str()>{level.label()}</option>
                            }).collect_view()}
                        </select>
                    </div>
                    {input("check_minutes", "Look back this many minutes (empty: 60; a day is 1440)", "60", |f| f.minutes.clone(), |f, v| f.minutes = v)}
                    {input("check_event_count", "How many events it takes (empty: 1)", "1", |f| f.event_count.clone(), |f, v| f.event_count = v)}
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_wanted"
                                prop:checked=move || fields.with(|f| f.wanted) on:change=on_wanted/>
                            " These events must be there (a backup that succeeded): too few is the trouble"
                        </label>
                    </div>
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_as_warning"
                                prop:checked=move || fields.with(|f| f.as_warning) on:change=on_as_warning/>
                            " Trouble is a Warning, not Down"
                        </label>
                    </div>
                </Show>
                <Show when=move || shown().process>
                    {input("check_process", "Program name (as the task list shows it; .exe or not; * for any characters)", "sqlservr, nginx, backup-*", |f| f.process.clone(), |f, v| f.process = v)}
                    {input("check_command", "Only those whose command line contains (empty: all of that name)", "app.jar, --config /etc/agent.toml", |f| f.command.clone(), |f, v| f.command = v)}
                    {input("check_min_count", "The fewest that must run (empty: 1; 0: none need to)", "1", |f| f.min_count.clone(), |f, v| f.min_count = v)}
                    {input("check_max_count", "The most that may run (empty: any number; 0 with 0 above: it must not run)", "e.g. 1", |f| f.max_count.clone(), |f, v| f.max_count = v)}
                </Show>
                <Show when=move || shown().updates>
                    {input("check_security_days", "Warn once security updates have waited this many days (empty: as soon as one waits; \"never\": don't warn). The probe only reports what its system already knows: it installs nothing", "0", |f| f.security_days.clone(), |f, v| f.security_days = v)}
                    {input("check_reboot_days", "Warn once a restart has been needed for this many days (empty: as soon as one is; \"never\": don't warn)", "0", |f| f.reboot_days.clone(), |f, v| f.reboot_days = v)}
                    {input("check_stale_days", "Warn once the system hasn't looked for updates for this many days (empty: 14; \"never\": don't warn)", "14", |f| f.stale_days.clone(), |f, v| f.stale_days = v)}
                </Show>
                <Show when=move || shown().file>
                    {input("check_file_path", "File or folder on the probe's machine (a folder: its newest file counts)", "/var/backups, D:\\Backups, \\\\nas\\backups", |f| f.path.clone(), |f, v| f.path = v)}
                    {input("check_pattern", "In a folder, only files named (empty: every file)", "*.bak, backup-*.zip", |f| f.pattern.clone(), |f, v| f.pattern = v)}
                    {input("check_min_mb", "The newest file must be at least (MB; empty: any size)", "e.g. 500", |f| f.min_mb.clone(), |f, v| f.min_mb = v)}
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_recursive"
                                prop:checked=move || fields.with(|f| f.recursive) on:change=on_recursive/>
                            " Look in its subfolders too"
                        </label>
                    </div>
                </Show>
                <Show when=move || shown().path>
                    {input("check_path", "Disk (empty: the system disk)", "/, /data, C:", |f| f.path.clone(), |f, v| f.path = v)}
                </Show>
                <Show when=move || shown().unverified>
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_unverified"
                                prop:checked=move || fields.with(|f| f.unverified) on:change=on_unverified/>
                            {unverified_label}
                        </label>
                    </div>
                </Show>
                {move || metric().map(|metric| view! {
                    <div class="form-group check-limits">
                        <label>
                            {format!("{} warning at ", metric.label())}
                            <input type="text" name="check_warning_at" size="6"
                                placeholder=inherited(|l| l.warning, metric)
                                prop:value=move || warning_at.get() on:input=on_warning/>
                            ", down at "
                            <input type="text" name="check_down_at" size="6"
                                placeholder=inherited(|l| l.down, metric)
                                prop:value=move || down_at.get() on:input=on_down/>
                            {format!(
                                " ({}; empty: the site's, company's or hub's; \"off\": none)",
                                match metric {
                                    ThresholdMetric::Tls => "days left or fewer",
                                    ThresholdMetric::FileAge => "hours old or more",
                                    other => other.unit().trim(),
                                },
                            )}
                        </label>
                    </div>
                })}
                <div class="form-group alert-after">
                    <label>
                        "Alert after "
                        <input type="number" name="check_fail_after" min="1" max=max.clone()
                            placeholder=default.clone() prop:value=move || fail_after.get() on:input=on_fail/>
                        " failed results in a row; recovered after "
                        <input type="number" name="check_recover_after" min="1" max=max
                            placeholder=default prop:value=move || recover_after.get() on:input=on_recover/>
                        " good ones"
                    </label>
                </div>
                <div class="form-group">
                    <label for="check_depends_on">"Behind (while it is down, this check is Unknown and doesn't alert)"</label>
                    <select id="check_depends_on" name="check_depends_on"
                        prop:value=move || depends_on.get() on:change=on_depends>
                        <option value="">"Nothing"</option>
                        {parents.into_iter().map(|(id, name)| view! { <option value=id>{name}</option> }).collect_view()}
                    </select>
                </div>
                <div class="form-group form-check">
                    <label>
                        <input type="checkbox" name="check_enabled" prop:checked=move || enabled.get() on:change=on_enabled/>
                        " Enabled (sent to the probe)"
                    </label>
                </div>
                <div class="login-error">{error}</div>
                <div class="modal-actions">
                    <button class="btn btn-secondary" on:click=move |_| open.set(false)>"Cancel"</button>
                    <button class="btn btn-primary" on:click=move |_| save()>"Save"</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(kind: &str) -> CheckFields {
        CheckFields {
            kind: kind.into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn every_kind_has_a_label_and_round_trips_through_the_form() {
        let specs = [
            CheckSpec::ping("10.0.0.1"),
            CheckSpec::Ping {
                host: "10.0.0.2".into(),
                count: 8,
                tcp_port: Some(3389),
            },
            CheckSpec::http("https://example.com"),
            CheckSpec::Http {
                url: "https://example.com/login".into(),
                options: HttpOptions {
                    method: HttpMethod::Head,
                    status: "200, 401".into(),
                    contains: String::new(),
                    absent: String::new(),
                    follow_redirects: false,
                    headers: vec![HttpHeader {
                        name: "Accept".into(),
                        value: "text/html".into(),
                    }],
                    verify: false,
                },
            },
            CheckSpec::Tcp {
                host: "db".into(),
                port: 5432,
            },
            CheckSpec::Tls {
                host: "example.com".into(),
                port: 8443,
                verify: false,
            },
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk {
                path: Some("/data".into()),
            },
            CheckSpec::Disk { path: None },
            CheckSpec::Disks { ignore: vec![] },
            CheckSpec::Disks {
                ignore: vec!["E:".into(), "/mnt/*".into()],
            },
            CheckSpec::Service {
                name: "MSSQL$SQLEXPRESS".into(),
            },
            CheckSpec::Dns {
                name: "example.com".into(),
                record: DnsRecord::A,
                server: None,
                expect: vec![],
            },
            CheckSpec::Dns {
                name: "example.com".into(),
                record: DnsRecord::Mx,
                server: Some("192.168.1.1".into()),
                expect: vec!["10 mail.example.com".into(), "20 backup.example.com".into()],
            },
            CheckSpec::File {
                path: "D:\\Backups".into(),
                pattern: "*.bak".into(),
                recursive: true,
                min_bytes: Some(524_288_000),
            },
            CheckSpec::File {
                path: "/srv/dump.sql".into(),
                pattern: String::new(),
                recursive: false,
                min_bytes: Some(512),
            },
            CheckSpec::Uptime,
        ];
        for spec in specs {
            assert_ne!(kind_label(spec.kind()), "Unknown");
            assert_eq!(CheckFields::from_spec(&spec).to_spec(), Ok(spec));
        }
    }

    #[test]
    fn a_kind_shows_only_its_inputs() {
        assert_eq!(
            fields("tcp").inputs(),
            Inputs {
                host: true,
                port: true,
                ..Inputs::default()
            }
        );
        assert_eq!(fields("cpu").inputs(), Inputs::default());
    }

    #[test]
    fn values_are_trimmed_and_checked_like_the_hub_checks_them() {
        let mut tcp = fields("tcp");
        tcp.host = " db ".into();
        tcp.port = "http".into();
        assert!(tcp.to_spec().unwrap_err().contains("port"));
        tcp.port = " 5432 ".into();
        assert_eq!(
            tcp.to_spec(),
            Ok(CheckSpec::Tcp {
                host: "db".into(),
                port: 5432
            })
        );

        // A DNS name: an A record from the machine's servers unless typed.
        let mut dns = fields("dns");
        dns.host = " intranet.example.com ".into();
        assert_eq!(
            dns.to_spec(),
            Ok(CheckSpec::Dns {
                name: "intranet.example.com".into(),
                record: DnsRecord::A,
                server: None,
                expect: vec![],
            })
        );
        dns.server = "dns.example.com".into();
        assert!(dns.to_spec().unwrap_err().contains("DNS server"));
        dns.server = "192.168.1.1".into();
        dns.record = "srv".into();
        assert!(dns.to_spec().unwrap_err().contains("record type"));

        // Every disk: the ones to leave out are a list.
        let mut disks = fields("disks");
        assert_eq!(disks.to_spec(), Ok(CheckSpec::Disks { ignore: vec![] }));
        disks.ignore = " E: ,, /mnt/* ".into();
        assert_eq!(
            disks.to_spec(),
            Ok(CheckSpec::Disks {
                ignore: vec!["E:".into(), "/mnt/*".into()]
            })
        );

        // A ping: four echoes and no TCP port unless typed.
        let mut ping = fields("ping");
        ping.host = "10.0.0.1".into();
        assert_eq!(ping.to_spec(), Ok(CheckSpec::ping("10.0.0.1")));
        ping.count = " 10 ".into();
        ping.tcp_port = "443".into();
        assert_eq!(
            ping.to_spec(),
            Ok(CheckSpec::Ping {
                host: "10.0.0.1".into(),
                count: 10,
                tcp_port: Some(443),
            })
        );
        ping.count = "50".into();
        assert!(ping.to_spec().unwrap_err().contains("1-20"));
        ping.count = "many".into();
        assert!(ping.to_spec().unwrap_err().contains("a number"));

        // A file: sizes are typed in MB.
        let mut file = fields("file");
        file.path = " /var/backups ".into();
        file.min_mb = "0.5".into();
        assert_eq!(
            file.to_spec(),
            Ok(CheckSpec::File {
                path: "/var/backups".into(),
                pattern: String::new(),
                recursive: false,
                min_bytes: Some(524_288),
            })
        );
        file.min_mb = "big".into();
        assert!(file.to_spec().unwrap_err().contains("number of MB"));
        file.min_mb.clear();
        file.path = "backups".into();
        assert!(file.to_spec().unwrap_err().contains("full one"));

        // A web page: a plain GET unless the form says more.
        let mut http = fields("http");
        http.url = " https://example.com ".into();
        assert_eq!(http.to_spec(), Ok(CheckSpec::http("https://example.com")));
        http.contains = "Sign in ".into();
        http.headers = "Accept-Language: en\n\n X-Site : branch 7 ".into();
        let Ok(CheckSpec::Http { options, .. }) = http.to_spec() else {
            panic!("{:?}", http.to_spec());
        };
        assert_eq!(options.contains, "Sign in ", "as typed");
        let sent: Vec<_> = options
            .headers
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
            .collect();
        assert_eq!(sent, [("Accept-Language", "en"), ("X-Site", "branch 7")]);
        http.headers = "no colon".into();
        assert!(http.to_spec().unwrap_err().contains("Name: value"));
        http.headers.clear();
        http.status = "2xx".into();
        assert!(http.to_spec().unwrap_err().contains("status code"));
        http.status = common::checks::DEFAULT_HTTP_STATUS.into();
        http.method = "head".into();
        assert!(
            http.to_spec().unwrap_err().contains("HEAD"),
            "text needs a body"
        );

        // A certificate is on port 443 unless the form says otherwise, and verified.
        let mut tls = fields("tls");
        tls.host = "example.com".into();
        assert_eq!(
            tls.to_spec(),
            Ok(CheckSpec::Tls {
                host: "example.com".into(),
                port: 443,
                verify: true
            })
        );
        tls.port = "https".into();
        assert!(tls.to_spec().unwrap_err().contains("port"));
        let mut tcp = fields("tcp");
        tcp.host = "db".into();
        assert!(tcp.to_spec().is_err(), "a TCP check needs its port");

        let mut ping = fields("ping");
        ping.host = "router lan".into();
        assert!(ping.to_spec().unwrap_err().contains("IP address"));
        ping.host = "router.lan".into();
        assert_eq!(ping.to_spec(), Ok(CheckSpec::ping("router.lan")), "a name");
        assert!(fields("snmp").to_spec().is_err());
    }

    #[test]
    fn a_request_needs_a_name() {
        let cpu = fields("cpu");
        let none = MetricLimits::default();
        assert!(check_request("  ", &cpu, true, AlertAfter::default(), none, "").is_err());
        let request = check_request(" CPU ", &cpu, false, AlertAfter::default(), none, "").unwrap();
        assert_eq!((request.name.as_str(), request.enabled), ("CPU", false));
        assert_eq!(request.depends_on, None, "empty: behind nothing");
        let behind = check_request("CPU", &cpu, true, AlertAfter::default(), none, " gw ");
        assert_eq!(behind.unwrap().depends_on.as_deref(), Some("gw"));
    }

    /// Regression test: Disable and Enable sent only the name, spec and flag,
    /// so a check lost when it alerts and its own limits.
    #[test]
    fn switching_a_check_on_or_off_keeps_the_rest_of_it() {
        use crate::model::Setting;
        let check = CheckInfo {
            id: "c".into(),
            probe_id: "p".into(),
            name: "Disk".into(),
            spec: CheckSpec::Disk { path: None },
            target: String::new(),
            enabled: true,
            alert_after: AlertAfter {
                fail_after: Some(3),
                recover_after: None,
            },
            limits: MetricLimits {
                warning: Some(Setting::At(80.0)),
                down: None,
            },
            limits_in_force: None,
            depends_on: Some("gw".into()),
            secrets_set: vec![],
            created_at: String::new(),
            updated_at: String::new(),
            updated_by: None,
        };
        let request = switched(&check);
        assert!(!request.enabled);
        assert_eq!(request.alert_after, check.alert_after);
        assert_eq!(request.limits, check.limits);
        assert_eq!(request.depends_on.as_deref(), Some("gw"));
    }

    #[test]
    fn when_it_alerts_is_optional_and_checked() {
        assert_eq!(
            alert_after("", " "),
            Ok(AlertAfter::default()),
            "empty: the default"
        );
        assert_eq!(
            alert_after("3", ""),
            Ok(AlertAfter {
                fail_after: Some(3),
                recover_after: None,
            })
        );
        assert!(alert_after("0", "").is_err());
        assert!(alert_after("", "11").is_err());
        assert!(alert_after("two", "").unwrap_err().contains("whole number"));
    }

    #[test]
    fn a_checks_limits_are_typed_for_its_kind() {
        use crate::model::Setting;
        assert_eq!(
            check_limits("disk", "85", " off "),
            Ok(MetricLimits {
                warning: Some(Setting::At(85.0)),
                down: Some(Setting::Off),
            })
        );
        assert_eq!(check_limits("ping", "", ""), Ok(MetricLimits::default()));
        assert!(check_limits("cpu", "120", "").is_err(), "over 100 %");
        assert!(
            check_limits("disk", "95", "90")
                .unwrap_err()
                .contains("below")
        );
        assert!(check_limits("ping", "fast", "").is_err());
        assert!(check_limits("tls", "30", "14").is_ok(), "days left fall");
        assert!(check_limits("file", "26", "50").is_ok(), "hours");
        assert!(check_limits("file", "50", "26").is_err());
        assert_eq!(
            check_limits("service", "1", "2"),
            Ok(MetricLimits::default()),
            "running or not: nothing to limit"
        );
        assert!(
            check_limits("tls", "14", "30")
                .unwrap_err()
                .contains("above")
        );
        assert_eq!(
            check_limits("uptime", "5", "9"),
            Ok(MetricLimits::default()),
            "no thresholds for its kind"
        );
    }
}

#[cfg(test)]
mod snmp_tests {
    use super::*;
    use common::checks::SnmpOid;

    fn snmp() -> CheckFields {
        CheckFields {
            kind: "snmp".into(),
            host: "192.168.1.5".into(),
            oids: "1.3.6.1.2.1.1.3.0".into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn the_values_of_a_check_are_shown_as_the_lines_that_were_typed() {
        let text = "Uplink = 1.3.6.1.2.1.2.2.1.8.1 = 1\nToner = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 20 ; down < 5";
        let typed = CheckFields {
            oids: format!("\n  {text}\n"),
            ..snmp()
        };
        let spec = typed.to_spec().unwrap();
        let CheckSpec::Snmp { oids, .. } = &spec else {
            panic!("an snmp check");
        };
        assert_eq!(oids.len(), 2);
        assert!(oids[1].walk);
        // And back, as the form shows a check being edited.
        let again = CheckFields::from_spec(&spec);
        assert_eq!(again.oids, text);
        assert_eq!(again.to_spec().unwrap(), spec);

        // A line that can't be read, and one that makes no sense.
        let unreadable = CheckFields {
            oids: "Fan = 1.3.6.1.4.1.9.1 ; sometimes".into(),
            ..snmp()
        };
        assert!(
            unreadable
                .to_spec()
                .unwrap_err()
                .starts_with("Fan: \"sometimes\": not an option")
        );
        let senseless = CheckFields {
            oids: "Fan = 1.3.6.1.4.1.9.1 ; warn > 60 ; down < 5".into(),
            ..snmp()
        };
        assert!(
            senseless
                .to_spec()
                .unwrap_err()
                .contains("both over or both under")
        );
    }

    #[test]
    fn an_snmp_check_goes_through_the_form_and_back() {
        let plain = snmp().to_spec().unwrap();
        assert_eq!(
            plain,
            CheckSpec::snmp("192.168.1.5", vec![SnmpOid::of("1.3.6.1.2.1.1.3.0")])
        );
        assert_eq!(CheckFields::from_spec(&plain).to_spec().unwrap(), plain);

        let v3 = CheckFields {
            port: "1161".into(),
            version: "v3".into(),
            user: "monitor".into(),
            auth: "sha256".into(),
            privacy: "aes128".into(),
            ..snmp()
        };
        let spec = v3.to_spec().unwrap();
        assert!(matches!(
            &spec,
            CheckSpec::Snmp { port: 1161, version: SnmpVersion::V3, user, auth: Some(SnmpAuth::Sha256), privacy: Some(SnmpPrivacy::Aes128), .. } if user == "monitor"
        ));
        let again = CheckFields::from_spec(&spec);
        assert_eq!(
            (again.port.as_str(), again.auth.as_str()),
            ("1161", "sha256")
        );
        assert_eq!(again.to_spec().unwrap(), spec);

        // Back to v2c: what was typed for v3 is left out, not refused.
        let back = CheckFields {
            version: "v2c".into(),
            ..v3
        };
        assert_eq!(
            back.to_spec().unwrap(),
            CheckSpec::Snmp {
                host: "192.168.1.5".into(),
                port: 1161,
                version: SnmpVersion::V2c,
                user: String::new(),
                auth: None,
                privacy: None,
                oids: vec![SnmpOid::of("1.3.6.1.2.1.1.3.0")],
            }
        );
        assert!(
            CheckFields {
                oids: "sysUpTime.0".into(),
                ..snmp()
            }
            .to_spec()
            .unwrap_err()
            .contains("isn't an OID in numbers")
        );
    }

    #[test]
    fn only_the_secrets_typed_and_used_are_sent() {
        assert_eq!(snmp().secrets(), None, "nothing typed: keep what there is");
        let typed = CheckFields {
            community: "public".into(),
            auth_password: "left over from v3".into(),
            ..snmp()
        };
        assert_eq!(typed.secrets().unwrap().names(), ["community"]);

        let v3 = CheckFields {
            version: "v3".into(),
            auth: "sha1".into(),
            auth_password: "long enough".into(),
            priv_password: "not used: no privacy".into(),
            community: "not used: v3".into(),
            ..snmp()
        };
        assert_eq!(v3.secrets().unwrap().names(), ["auth_password"]);
        let private = CheckFields {
            privacy: "aes128".into(),
            ..v3
        };
        assert_eq!(
            private.secrets().unwrap().names(),
            ["auth_password", "priv_password"]
        );
        // Another kind has none, whatever was typed.
        let ping = CheckFields {
            kind: "ping".into(),
            community: "public".into(),
            ..CheckFields::default()
        };
        assert_eq!(ping.secrets(), None);
        // And they are in the request, not in the spec.
        let request = check_request(
            "Switch",
            &typed,
            true,
            AlertAfter::default(),
            MetricLimits::default(),
            "",
        )
        .unwrap();
        assert_eq!(request.secrets.unwrap().get("community"), Some("public"));
        assert!(
            !serde_json::to_string(&request.spec)
                .unwrap()
                .contains("public")
        );
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;

    fn database() -> CheckFields {
        CheckFields {
            kind: "database".into(),
            host: "db.lan".into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn a_database_check_goes_through_the_form_and_back() {
        // Nothing chosen: the first engine, on its usual port.
        let plain = database().to_spec().unwrap();
        assert_eq!(
            plain,
            CheckSpec::Database {
                engine: DbEngine::Postgres,
                host: "db.lan".into(),
                port: None,
            }
        );
        let again = CheckFields::from_spec(&plain);
        assert_eq!(
            (again.engine.as_str(), again.port.as_str()),
            ("postgres", "")
        );
        assert_eq!(again.to_spec().unwrap(), plain);

        for engine in DbEngine::ALL {
            let typed = CheckFields {
                engine: engine.as_str().into(),
                port: " 14330 ".into(),
                ..database()
            };
            let spec = typed.to_spec().unwrap();
            assert_eq!(
                spec,
                CheckSpec::Database {
                    engine: *engine,
                    host: "db.lan".into(),
                    port: Some(14330),
                }
            );
            assert_eq!(CheckFields::from_spec(&spec).to_spec().unwrap(), spec);
        }

        let wrong = |fields: CheckFields| fields.to_spec().unwrap_err();
        assert_eq!(
            wrong(CheckFields {
                engine: "oracle".into(),
                ..database()
            }),
            "choose the kind of database"
        );
        assert_eq!(
            wrong(CheckFields {
                port: "0".into(),
                ..database()
            }),
            "the port must be 1-65535"
        );
        assert!(
            wrong(CheckFields {
                port: "many".into(),
                ..database()
            })
            .contains("port")
        );
        // Its latency has limits, like a TCP port's.
        assert_eq!(
            ThresholdMetric::of_kind("database"),
            Some(ThresholdMetric::Latency)
        );
        assert!(database().inputs().database);
        assert_ne!(kind_label("database"), "Unknown");
    }
}

#[cfg(test)]
mod updates_tests {
    use super::*;

    fn updates() -> CheckFields {
        CheckFields {
            kind: "updates".into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn an_updates_checks_limits_go_through_the_form_and_back() {
        // Nothing typed: the usual limits, shown as empty fields again.
        let usual = updates().to_spec().unwrap();
        assert_eq!(usual, CheckSpec::updates());
        let again = CheckFields::from_spec(&usual);
        assert_eq!(
            (
                again.security_days.as_str(),
                again.reboot_days.as_str(),
                again.stale_days.as_str()
            ),
            ("", "", "")
        );

        let typed = CheckFields {
            security_days: " 7 ".into(),
            reboot_days: "Never".into(),
            stale_days: "30".into(),
            ..updates()
        };
        let spec = typed.to_spec().unwrap();
        assert_eq!(
            spec,
            CheckSpec::Updates {
                security_days: Some(7),
                reboot_days: None,
                stale_days: Some(30),
            }
        );
        let shown = CheckFields::from_spec(&spec);
        assert_eq!(
            (
                shown.security_days.as_str(),
                shown.reboot_days.as_str(),
                shown.stale_days.as_str()
            ),
            ("7", "never", "30")
        );
        assert_eq!(shown.to_spec().unwrap(), spec);

        for wrong in ["soon", "-1", "366", "1.5"] {
            let error = CheckFields {
                stale_days: wrong.into(),
                ..updates()
            }
            .to_spec()
            .unwrap_err();
            assert!(error.contains("number of days (0-365)"), "{error}");
        }
        assert!(updates().inputs().updates);
        assert_ne!(kind_label("updates"), "Unknown");
        // Its limits are its own: none of the site's or the hub's kind.
        assert_eq!(ThresholdMetric::of_kind("updates"), None);
    }
}

#[cfg(test)]
mod temperature_tests {
    use super::*;

    #[test]
    fn a_temperature_check_goes_through_the_form_and_back() {
        let plain = CheckFields {
            kind: "temperature".into(),
            ..CheckFields::default()
        };
        assert_eq!(
            plain.to_spec(),
            Ok(CheckSpec::Temperature { sensors: vec![] })
        );
        let typed = CheckFields {
            pattern: " coretemp* ,, acpitz* ".into(),
            ..plain.clone()
        };
        let spec = typed.to_spec().unwrap();
        assert_eq!(
            spec,
            CheckSpec::Temperature {
                sensors: vec!["coretemp*".into(), "acpitz*".into()]
            }
        );
        let again = CheckFields::from_spec(&spec);
        assert_eq!(again.pattern, "coretemp*, acpitz*");
        assert_eq!(again.to_spec().unwrap(), spec);

        // Its own field and limits in degrees; no host, no file's path.
        let inputs = plain.inputs();
        assert!(inputs.temperature && !inputs.smart && !inputs.host && !inputs.path);
        assert_ne!(kind_label("temperature"), "Unknown");
        assert_eq!(
            ThresholdMetric::of_kind("temperature"),
            Some(ThresholdMetric::Temperature)
        );
    }
}

#[cfg(test)]
mod smart_tests {
    use super::*;

    #[test]
    fn a_smart_check_goes_through_the_form_and_back() {
        let plain = CheckFields {
            kind: "smart".into(),
            ..CheckFields::default()
        };
        assert_eq!(plain.to_spec(), Ok(CheckSpec::Smart { ignore: vec![] }));
        let typed = CheckFields {
            ignore: " WDC WD40* ,, S3Z9NB0K ".into(),
            ..plain.clone()
        };
        let spec = typed.to_spec().unwrap();
        assert_eq!(
            spec,
            CheckSpec::Smart {
                ignore: vec!["WDC WD40*".into(), "S3Z9NB0K".into()]
            }
        );
        let again = CheckFields::from_spec(&spec);
        assert_eq!(again.ignore, "WDC WD40*, S3Z9NB0K");
        assert_eq!(again.to_spec().unwrap(), spec);

        // The drives say themselves how they are: no thresholds, no host,
        // and not the disk usage check's field.
        let inputs = plain.inputs();
        assert!(inputs.smart && !inputs.disks && !inputs.host);
        assert_ne!(kind_label("smart"), "Unknown");
        assert_eq!(ThresholdMetric::of_kind("smart"), None);
    }
}

#[cfg(test)]
mod interface_tests {
    use super::*;

    #[test]
    fn an_interface_check_goes_through_the_form_and_back() {
        let typed = CheckFields {
            kind: "interface".into(),
            interface: " Ethernet 2 ".into(),
            ..CheckFields::default()
        };
        let spec = typed.to_spec().unwrap();
        assert_eq!(
            spec,
            CheckSpec::Interface {
                name: "Ethernet 2".into()
            }
        );
        let again = CheckFields::from_spec(&spec);
        assert_eq!(again.interface, "Ethernet 2");
        assert_eq!(again.to_spec().unwrap(), spec);

        let unnamed = CheckFields {
            interface: String::new(),
            ..typed.clone()
        };
        assert!(unnamed.to_spec().unwrap_err().contains("interface name"));
        // Its link is up or it isn't: one field, no thresholds.
        assert!(typed.inputs().interface);
        assert!(!typed.inputs().host);
        assert_ne!(kind_label("interface"), "Unknown");
        assert_eq!(ThresholdMetric::of_kind("interface"), None);
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;

    fn process() -> CheckFields {
        CheckFields {
            kind: "process".into(),
            process: " sqlservr ".into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn a_process_check_goes_through_the_form_and_back() {
        // Only a name: at least one must run.
        let plain = process().to_spec().unwrap();
        assert_eq!(plain, CheckSpec::process("sqlservr"));
        let again = CheckFields::from_spec(&plain);
        assert_eq!(
            (
                again.process.as_str(),
                again.min_count.as_str(),
                again.max_count.as_str()
            ),
            ("sqlservr", "", "")
        );
        assert_eq!(again.to_spec().unwrap(), plain);

        let typed = CheckFields {
            process: "java".into(),
            command: " app.jar ".into(),
            min_count: "2".into(),
            max_count: " 4 ".into(),
            ..process()
        };
        let spec = typed.to_spec().unwrap();
        assert_eq!(
            spec,
            CheckSpec::Process {
                name: "java".into(),
                command: "app.jar".into(),
                min: 2,
                max: Some(4),
            }
        );
        assert_eq!(CheckFields::from_spec(&spec).to_spec().unwrap(), spec);
        // A program that must not run.
        let unwanted = CheckFields {
            min_count: "0".into(),
            max_count: "0".into(),
            ..process()
        }
        .to_spec()
        .unwrap();
        assert_eq!(CheckFields::from_spec(&unwanted).min_count, "0");
        assert_eq!(
            CheckFields::from_spec(&unwanted).to_spec().unwrap(),
            unwanted
        );

        let wrong = |fields: CheckFields| fields.to_spec().unwrap_err();
        assert!(
            wrong(CheckFields {
                process: String::new(),
                ..process()
            })
            .contains("process name")
        );
        assert!(
            wrong(CheckFields {
                min_count: "some".into(),
                ..process()
            })
            .contains("the fewest that must run is a number")
        );
        assert!(
            wrong(CheckFields {
                max_count: "10001".into(),
                ..process()
            })
            .contains("the most that may run is a number, 0-10000")
        );
        assert!(
            wrong(CheckFields {
                min_count: "3".into(),
                max_count: "2".into(),
                ..process()
            })
            .contains("fewest not above")
        );
        assert!(process().inputs().process);
        assert_ne!(kind_label("process"), "Unknown");
        assert_eq!(ThresholdMetric::of_kind("process"), None);
    }
}

#[cfg(test)]
mod eventlog_tests {
    use super::*;

    fn eventlog() -> CheckFields {
        CheckFields {
            kind: "eventlog".into(),
            log: " System ".into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn an_eventlog_check_goes_through_the_form_and_back() {
        // Only a log: an error in the last hour.
        let plain = eventlog().to_spec().unwrap();
        assert_eq!(plain, CheckSpec::eventlog("System"));
        let again = CheckFields::from_spec(&plain);
        assert_eq!(
            (
                again.log.as_str(),
                again.level.as_str(),
                again.minutes.as_str(),
                again.event_count.as_str()
            ),
            ("System", "error", "", "")
        );
        assert_eq!(again.to_spec().unwrap(), plain);

        let typed = CheckFields {
            source: "disk".into(),
            event_ids: "7, 51;153  11".into(),
            level: "warning".into(),
            minutes: " 1440 ".into(),
            event_count: "3".into(),
            wanted: true,
            as_warning: true,
            ..eventlog()
        };
        let spec = typed.to_spec().unwrap();
        assert_eq!(
            spec,
            CheckSpec::Eventlog {
                log: "System".into(),
                source: "disk".into(),
                ids: vec![7, 51, 153, 11],
                level: EventLevel::Warning,
                minutes: 1440,
                count: 3,
                wanted: true,
                warning: true,
            }
        );
        let shown = CheckFields::from_spec(&spec);
        assert_eq!(shown.event_ids, "7, 51, 153, 11");
        assert_eq!(shown.to_spec().unwrap(), spec);

        let wrong = |fields: CheckFields| fields.to_spec().unwrap_err();
        for (fields, reason) in [
            (
                CheckFields {
                    log: String::new(),
                    ..eventlog()
                },
                "log",
            ),
            (
                CheckFields {
                    event_ids: "7, disk".into(),
                    ..eventlog()
                },
                "disk: an event ID is a number",
            ),
            (
                CheckFields {
                    level: "loud".into(),
                    ..eventlog()
                },
                "choose which events count",
            ),
            (
                CheckFields {
                    minutes: "0".into(),
                    ..eventlog()
                },
                "in minutes, is a number, 1-44640",
            ),
            (
                CheckFields {
                    event_count: "many".into(),
                    ..eventlog()
                },
                "the number of events is a number, 1-1000",
            ),
            (
                CheckFields {
                    source: "disk']".into(),
                    ..eventlog()
                },
                "names as Windows shows them",
            ),
        ] {
            let error = wrong(fields);
            assert!(error.contains(reason), "{error}");
        }
        assert!(eventlog().inputs().eventlog);
        assert_ne!(kind_label("eventlog"), "Unknown");
        assert_eq!(ThresholdMetric::of_kind("eventlog"), None);
    }
}
