//! API types and domain rules shared by the server (`ssr`) and the browser
//! (`hydrate`). This is the **single definition** of every JSON shape the hub
//! API returns, and of the rules the UI and server must agree on.
//!
//! Compiled for both targets, so it may only depend on `serde`: no database,
//! no async runtime, no browser APIs.

use std::collections::BTreeSet;

use common::checks::{CheckResult, CheckSpec, Health, Metric};
use serde::{Deserialize, Serialize};

// ===========================================================================
// Severity
// ===========================================================================

/// How bad things are for a probe or a company.
///
/// Variants are declared **worst first**, so the derived `Ord` sorts
/// `Critical < Degraded < Healthy` and a plain ascending sort lists problems first.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// An active probe reports at least one device down: a known outage.
    Critical,
    /// A probe stopped reporting: its data is stale, so the state is unknown.
    Degraded,
    /// Something is over a warning threshold (D-64): working, but look at it.
    Warning,
    #[default]
    Healthy,
}

impl Severity {
    /// A probe's severity. An expired probe is `Degraded` whatever its (stale)
    /// device counts say; only a live probe can prove an outage.
    pub fn for_probe(probe_active: bool, devices_down: i64, devices_warning: i64) -> Self {
        if !probe_active {
            Severity::Degraded
        } else if devices_down > 0 {
            Severity::Critical
        } else if devices_warning > 0 {
            Severity::Warning
        } else {
            Severity::Healthy
        }
    }

    /// A company is as bad as its worst probe. The device counts are of
    /// *active* probes only: an expired probe's are stale.
    pub fn for_company(
        active_devices_down: i64,
        active_devices_warning: i64,
        expired_probes: i64,
    ) -> Self {
        if active_devices_down > 0 {
            Severity::Critical
        } else if expired_probes > 0 {
            Severity::Degraded
        } else if active_devices_warning > 0 {
            Severity::Warning
        } else {
            Severity::Healthy
        }
    }

    /// Lowercase name, as serialised (`"critical"`, `"degraded"`, `"healthy"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::Degraded => "degraded",
            Severity::Warning => "warning",
            Severity::Healthy => "healthy",
        }
    }
}

// ===========================================================================
// Status and thresholds (M3-01, proposal 0005 §2.1, D-16, D-64)
// ===========================================================================

/// A check's status, worked out **on the hub** from the probe's result and
/// the thresholds that apply (D-16): the one value the dashboard, the probe
/// page, uptime and alerts all use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Status {
    Up,
    /// Over a warning threshold: working, but look at it.
    Warning,
    /// The check failed, or is over a down threshold.
    Down,
    /// Nobody knows: its probe stopped reporting (shown, not stored), or a
    /// check it depends on is down (stored, D-68).
    Unknown,
}

impl Status {
    /// `"Up"`, `"Warning"`, `"Down"`, `"Unknown"`, as serialised and stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Up => "Up",
            Status::Warning => "Warning",
            Status::Down => "Down",
            Status::Unknown => "Unknown",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Status::Up, Status::Warning, Status::Down, Status::Unknown]
            .into_iter()
            .find(|s| s.as_str() == text)
    }

    /// The probe's own verdict, before thresholds.
    pub fn of_health(health: Health) -> Self {
        match health {
            Health::Up => Status::Up,
            Health::Down => Status::Down,
        }
    }

    /// Worst first, for sorting a list: Down, Warning, Unknown, Up.
    pub fn rank(self) -> u8 {
        match self {
            Status::Down => 0,
            Status::Warning => 1,
            Status::Unknown => 2,
            Status::Up => 3,
        }
    }
}

/// One limit at one level: a number, or explicitly off (so a lower level
/// can switch off what a higher one sets). In JSON: a number or `"off"`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SettingJson", into = "SettingJson")]
pub enum Setting {
    Off,
    At(f64),
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SettingJson {
    At(f64),
    Word(String),
}

impl TryFrom<SettingJson> for Setting {
    type Error = String;
    fn try_from(json: SettingJson) -> Result<Self, String> {
        match json {
            SettingJson::At(value) => Ok(Setting::At(value)),
            SettingJson::Word(word) if word == "off" => Ok(Setting::Off),
            SettingJson::Word(word) => Err(format!("{word:?}: a number or \"off\"")),
        }
    }
}

impl From<Setting> for SettingJson {
    fn from(setting: Setting) -> Self {
        match setting {
            Setting::Off => SettingJson::Word("off".into()),
            Setting::At(value) => SettingJson::At(value),
        }
    }
}

impl Setting {
    /// As the forms show it: the number, or `off`.
    pub fn text(self) -> String {
        match self {
            Setting::Off => "off".into(),
            Setting::At(value) => format_number(value),
        }
    }

    /// As typed in a form: empty is `None` (the level above decides).
    pub fn parse_text(text: &str) -> Result<Option<Self>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        if text.eq_ignore_ascii_case("off") {
            return Ok(Some(Setting::Off));
        }
        text.parse::<f64>()
            .map(|value| Some(Setting::At(value)))
            .map_err(|_| format!("{text}: a number, \"off\", or empty"))
    }

    fn value(self) -> Option<f64> {
        match self {
            Setting::Off => None,
            Setting::At(value) => Some(value),
        }
    }
}

/// `90`, `92.5`: no needless decimals.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// What thresholds apply to: a usage percentage, a latency, the days a
/// certificate has left, or a file's age.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdMetric {
    /// `cpu` checks: percent used.
    Cpu,
    /// `memory` checks: percent used.
    Memory,
    /// `disk` and `disks` checks: percent used (of the fullest disk).
    Disk,
    /// `ping`, `http`, `tcp`, `dns` checks: milliseconds.
    Latency,
    /// `tls` checks: days until the certificate expires. The one metric
    /// where less is worse ([`ThresholdMetric::falls`]).
    Tls,
    /// `file` checks: hours since the newest file was written.
    FileAge,
    /// `ping` checks: percent of the echoes lost. Judged beside the
    /// latency ([`ThresholdMetric::also_of`]).
    Loss,
}

impl ThresholdMetric {
    pub const ALL: [ThresholdMetric; 7] = [
        ThresholdMetric::Cpu,
        ThresholdMetric::Memory,
        ThresholdMetric::Disk,
        ThresholdMetric::Latency,
        ThresholdMetric::Loss,
        ThresholdMetric::Tls,
        ThresholdMetric::FileAge,
    ];

    /// The metric of a check kind (`ping`, `disk`, …), as the forms know it.
    pub fn of_kind(kind: &str) -> Option<Self> {
        match kind {
            "cpu" => Some(ThresholdMetric::Cpu),
            "memory" => Some(ThresholdMetric::Memory),
            "disk" | "disks" => Some(ThresholdMetric::Disk),
            "ping" | "http" | "tcp" | "dns" | "snmp" | "database" => Some(ThresholdMetric::Latency),
            "tls" => Some(ThresholdMetric::Tls),
            "file" => Some(ThresholdMetric::FileAge),
            _ => None,
        }
    }

    /// The metric a check's thresholds apply to (`None`: `uptime`,
    /// `service`, `process`; `updates`, whose limits are its own, D-89).
    pub fn of(spec: &CheckSpec) -> Option<Self> {
        match spec {
            CheckSpec::Cpu => Some(ThresholdMetric::Cpu),
            CheckSpec::Memory => Some(ThresholdMetric::Memory),
            CheckSpec::Disk { .. } | CheckSpec::Disks { .. } => Some(ThresholdMetric::Disk),
            CheckSpec::Ping { .. }
            | CheckSpec::Http { .. }
            | CheckSpec::Tcp { .. }
            | CheckSpec::Dns { .. }
            | CheckSpec::Database { .. }
            | CheckSpec::Snmp { .. } => Some(ThresholdMetric::Latency),
            CheckSpec::Tls { .. } => Some(ThresholdMetric::Tls),
            CheckSpec::File { .. } => Some(ThresholdMetric::FileAge),
            CheckSpec::Service { .. }
            | CheckSpec::Process { .. }
            | CheckSpec::Eventlog { .. }
            | CheckSpec::Uptime
            | CheckSpec::Updates { .. } => None,
        }
    }

    /// What a check is judged by beside its own metric. These have no
    /// limits on the check itself: its site's, company's or the hub's apply.
    pub fn also_of(spec: &CheckSpec) -> &'static [ThresholdMetric] {
        match spec {
            CheckSpec::Ping { .. } => &[ThresholdMetric::Loss],
            _ => &[],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThresholdMetric::Cpu => "CPU",
            ThresholdMetric::Memory => "Memory",
            ThresholdMetric::Disk => "Disk",
            ThresholdMetric::Latency => "Latency",
            ThresholdMetric::Tls => "Certificate",
            ThresholdMetric::FileAge => "File age",
            ThresholdMetric::Loss => "Packet loss",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            ThresholdMetric::Latency => " ms",
            ThresholdMetric::Tls => " days",
            ThresholdMetric::FileAge => " h",
            _ => "%",
        }
    }

    /// The largest limit that makes sense.
    fn max(self) -> f64 {
        match self {
            ThresholdMetric::Latency => 600_000.0,
            ThresholdMetric::Tls => 3650.0,
            // Ten years, in hours.
            ThresholdMetric::FileAge => 87_600.0,
            _ => 100.0,
        }
    }

    /// Less is worse: a limit is reached **at or under** it, and the warning
    /// limit is the higher one. The others rise: at or over.
    pub fn falls(self) -> bool {
        self == ThresholdMetric::Tls
    }

    /// Does `value` reach the limit `at`?
    pub fn reaches(self, value: f64, at: f64) -> bool {
        if self.falls() {
            value <= at
        } else {
            value >= at
        }
    }

    /// What the value is, in a reason: `disk usage`, `latency`.
    fn what(self) -> String {
        match self {
            ThresholdMetric::Latency => "latency".to_string(),
            ThresholdMetric::Tls => "certificate expiry in".to_string(),
            ThresholdMetric::FileAge => "file age".to_string(),
            ThresholdMetric::Loss => "packet loss".to_string(),
            other => format!("{} usage", other.label().to_lowercase()),
        }
    }

    /// The measured value thresholds compare with.
    pub fn value(self, result: &CheckResult) -> Option<f64> {
        use common::checks::metric;
        match self {
            ThresholdMetric::Latency => result.latency_ms.map(|ms| ms as f64),
            ThresholdMetric::Tls => result.metric(metric::DAYS_LEFT).map(|m| m.value),
            ThresholdMetric::FileAge => result.metric(metric::AGE).map(|m| m.value),
            ThresholdMetric::Loss => result.metric(metric::LOSS).map(|m| m.value),
            _ => result.metric(metric::USAGE).map(|m| m.value),
        }
    }
}

/// One metric's limits at one level; `None`: the level above decides.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricLimits {
    /// At or over it (under, for a metric that falls): Warning.
    #[serde(default)]
    pub warning: Option<Setting>,
    /// At or over it (under, for a metric that falls): Down.
    #[serde(default)]
    pub down: Option<Setting>,
}

impl MetricLimits {
    /// As typed in a form (each: a number, `off`, or empty to inherit),
    /// checked for `metric`.
    pub fn from_text(metric: ThresholdMetric, warning: &str, down: &str) -> Result<Self, String> {
        let label = metric.label();
        let limits = Self {
            warning: Setting::parse_text(warning).map_err(|e| format!("{label} warning: {e}"))?,
            down: Setting::parse_text(down).map_err(|e| format!("{label} down: {e}"))?,
        };
        limits.validate(metric)?;
        Ok(limits)
    }

    /// Each limit as a form shows it (empty: inherited).
    pub fn texts(&self) -> (String, String) {
        let text = |s: Option<Setting>| s.map(Setting::text).unwrap_or_default();
        (text(self.warning), text(self.down))
    }

    pub fn is_empty(&self) -> bool {
        self.warning.is_none() && self.down.is_none()
    }

    /// `Err` with what's wrong, for the forms and the API alike.
    pub fn validate(&self, metric: ThresholdMetric) -> Result<(), String> {
        let label = metric.label();
        for value in [self.warning, self.down]
            .into_iter()
            .flatten()
            .filter_map(Setting::value)
        {
            if !(0.0..=metric.max()).contains(&value) {
                return Err(format!(
                    "{label}: a limit is 0 to {}{}",
                    format_number(metric.max()),
                    metric.unit()
                ));
            }
        }
        // The warning limit is the one reached first.
        if let (Some(Setting::At(warning)), Some(Setting::At(down))) = (self.warning, self.down)
            && metric.reaches(warning, down)
        {
            let side = if metric.falls() { "above" } else { "below" };
            return Err(format!(
                "{label}: the warning limit is {side} the down limit"
            ));
        }
        Ok(())
    }
}

/// The thresholds a level sets (global, company, site), metric by metric.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    #[serde(default)]
    pub cpu: MetricLimits,
    #[serde(default)]
    pub memory: MetricLimits,
    #[serde(default)]
    pub disk: MetricLimits,
    #[serde(default)]
    pub latency: MetricLimits,
    #[serde(default)]
    pub tls: MetricLimits,
    #[serde(default)]
    pub file_age: MetricLimits,
    #[serde(default)]
    pub loss: MetricLimits,
}

impl Thresholds {
    pub fn get(&self, metric: ThresholdMetric) -> MetricLimits {
        match metric {
            ThresholdMetric::Cpu => self.cpu,
            ThresholdMetric::Memory => self.memory,
            ThresholdMetric::Disk => self.disk,
            ThresholdMetric::Latency => self.latency,
            ThresholdMetric::Tls => self.tls,
            ThresholdMetric::FileAge => self.file_age,
            ThresholdMetric::Loss => self.loss,
        }
    }

    pub fn set(&mut self, metric: ThresholdMetric, limits: MetricLimits) {
        match metric {
            ThresholdMetric::Cpu => self.cpu = limits,
            ThresholdMetric::Memory => self.memory = limits,
            ThresholdMetric::Disk => self.disk = limits,
            ThresholdMetric::Latency => self.latency = limits,
            ThresholdMetric::Tls => self.tls = limits,
            ThresholdMetric::FileAge => self.file_age = limits,
            ThresholdMetric::Loss => self.loss = limits,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        ThresholdMetric::ALL
            .into_iter()
            .try_for_each(|metric| self.get(metric).validate(metric))
    }

    /// What applies where nobody set anything: the alert thresholds the hub
    /// had before (CPU 95 %, memory 90 %, disk 90 %) as warnings, a nearly
    /// full disk as down, no latency limits (they depend on the link), and
    /// a certificate with 21 days left as a warning, 7 as down (D-77), and
    /// a file older than 26 hours as a warning, 50 as down: a nightly backup
    /// that is late, then one that was missed twice (D-79). Half of a
    /// ping's echoes lost is a warning (D-80); all of them is Down anyway.
    pub fn built_in() -> Self {
        let limits = |warning: f64, down: Option<f64>| MetricLimits {
            warning: Some(Setting::At(warning)),
            down: Some(down.map_or(Setting::Off, Setting::At)),
        };
        Self {
            cpu: limits(95.0, None),
            memory: limits(90.0, None),
            disk: limits(90.0, Some(97.0)),
            latency: MetricLimits {
                warning: Some(Setting::Off),
                down: Some(Setting::Off),
            },
            tls: limits(21.0, Some(7.0)),
            file_age: limits(26.0, Some(50.0)),
            loss: limits(50.0, None),
        }
    }
}

/// Where a limit comes from, lowest level first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdLevel {
    Check,
    Site,
    Company,
    Global,
    BuiltIn,
}

impl ThresholdLevel {
    pub fn label(self) -> &'static str {
        match self {
            ThresholdLevel::Check => "the check",
            ThresholdLevel::Site => "the site",
            ThresholdLevel::Company => "the company",
            ThresholdLevel::Global => "the hub's default",
            ThresholdLevel::BuiltIn => "the built-in default",
        }
    }
}

/// A limit in force and the level it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    /// `None`: off.
    pub value: Option<f64>,
    pub from: ThresholdLevel,
}

/// One metric's limits in force for a check (or for a level's children).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResolvedLimits {
    pub warning: Limit,
    pub down: Limit,
}

impl Limit {
    /// As a placeholder shows what applies: `90%`, `200ms`, `21 days`, `off`.
    pub fn text(&self, metric: ThresholdMetric) -> String {
        let unit = match metric {
            ThresholdMetric::Tls | ThresholdMetric::FileAge => metric.unit(),
            _ => metric.unit().trim(),
        };
        self.value
            .map_or_else(|| "off".into(), |v| format!("{}{unit}", format_number(v)))
    }
}

impl ResolvedLimits {
    /// Each limit from the lowest level that sets it (`levels` lowest
    /// first), else the built-in default (proposal 0005 §2.1).
    pub fn resolve(metric: ThresholdMetric, levels: &[(ThresholdLevel, MetricLimits)]) -> Self {
        let built_in = Thresholds::built_in().get(metric);
        let pick = |field: fn(&MetricLimits) -> Option<Setting>| {
            levels
                .iter()
                .find_map(|(level, limits)| field(limits).map(|s| (s, *level)))
                .or_else(|| field(&built_in).map(|s| (s, ThresholdLevel::BuiltIn)))
                .map_or(
                    Limit {
                        value: None,
                        from: ThresholdLevel::BuiltIn,
                    },
                    |(setting, from)| Limit {
                        value: setting.value(),
                        from,
                    },
                )
        };
        Self {
            warning: pick(|l| l.warning),
            down: pick(|l| l.down),
        }
    }
}

/// A result as the hub judged it, with why when a threshold decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub status: Status,
    /// E.g. `disk usage 92.5% is over the warning limit (90%)`,
    /// `certificate expiry in 12.5 days is under the warning limit (21 days)`.
    pub reason: Option<String>,
    /// It came during a maintenance window (D-65): no alerts, no uptime.
    pub in_maintenance: bool,
    /// Unknown because this check (its ID) is down: the root cause (D-68).
    pub behind: Option<String>,
}

impl Evaluation {
    /// The probe's verdict as it is (no thresholds).
    pub fn of_health(health: Health) -> Self {
        Self {
            status: Status::of_health(health),
            reason: None,
            in_maintenance: false,
            behind: None,
        }
    }

    /// Judge `result`: Down if the probe says so; else Down or Warning if
    /// its `metric` reaches a limit ([`ThresholdMetric::reaches`]); else Up.
    pub fn of(
        result: &CheckResult,
        metric: Option<ThresholdMetric>,
        limits: Option<&ResolvedLimits>,
    ) -> Self {
        if result.status == Health::Down {
            return Self::of_health(Health::Down);
        }
        let (Some(metric), Some(limits)) = (metric, limits) else {
            return Self::of_health(Health::Up);
        };
        let Some(value) = metric.value(result) else {
            return Self::of_health(Health::Up);
        };
        let what = metric.what();
        let side = if metric.falls() { "under" } else { "over" };
        for (status, limit, name) in [
            (Status::Down, limits.down, "down"),
            (Status::Warning, limits.warning, "warning"),
        ] {
            if let Some(at) = limit.value
                && metric.reaches(value, at)
            {
                return Self {
                    in_maintenance: false,
                    behind: None,
                    status,
                    reason: Some(format!(
                        "{what} {}{unit} is {side} the {name} limit ({}{unit}, from {})",
                        format_number((value * 10.0).round() / 10.0),
                        format_number(at),
                        limit.from.label(),
                        unit = metric.unit(),
                    )),
                };
            }
        }
        Self::of_health(Health::Up)
    }
}

/// A level's thresholds as the forms edit them
/// (`GET /api/admin/thresholds`, `…/companies/{id}/thresholds`, `…/sites/{site}/thresholds`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdsView {
    /// What this level sets.
    pub own: Thresholds,
    /// What applies where it sets nothing: the levels above, metric by metric
    /// (in the order of [`ThresholdMetric::ALL`]).
    pub inherited: Vec<(ThresholdMetric, ResolvedLimits)>,
}

// ===========================================================================
// Maintenance windows and silences (M3-04, proposal 0005 §2.4, D-65)
// ===========================================================================

/// What a silence or a maintenance window covers, within one company.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every probe and check of the company.
    Company,
    Site {
        site_id: String,
    },
    Probe {
        probe_id: String,
    },
    Check {
        check_id: String,
    },
}

impl Scope {
    /// `company`, `site`, `probe`, `check`, as stored.
    pub fn kind(&self) -> &'static str {
        match self {
            Scope::Company => "company",
            Scope::Site { .. } => "site",
            Scope::Probe { .. } => "probe",
            Scope::Check { .. } => "check",
        }
    }

    /// The ID it names (`None`: the whole company).
    pub fn id(&self) -> Option<&str> {
        match self {
            Scope::Company => None,
            Scope::Site { site_id } => Some(site_id),
            Scope::Probe { probe_id } => Some(probe_id),
            Scope::Check { check_id } => Some(check_id),
        }
    }

    /// From its stored kind and ID.
    pub fn from_parts(kind: &str, id: Option<String>) -> Option<Self> {
        Some(match (kind, id) {
            ("company", _) => Scope::Company,
            ("site", Some(site_id)) => Scope::Site { site_id },
            ("probe", Some(probe_id)) => Scope::Probe { probe_id },
            ("check", Some(check_id)) => Scope::Check { check_id },
            _ => return None,
        })
    }

    /// Does it cover a check (`check_id`) of a probe (`probe_id`) at a site
    /// (`site_id`) of its company? `check_id` `None`: the probe itself (its
    /// "stopped reporting" alert).
    pub fn covers(&self, site_id: &str, probe_id: &str, check_id: Option<&str>) -> bool {
        match self {
            Scope::Company => true,
            Scope::Site { site_id: s } => s == site_id,
            Scope::Probe { probe_id: p } => p == probe_id,
            Scope::Check { check_id: c } => Some(c.as_str()) == check_id,
        }
    }
}

/// `HH:MM` (24 h) as hours and minutes.
pub fn parse_hhmm(text: &str) -> Option<(u32, u32)> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && text.trim().len() == 5).then_some((h, m))
}

/// `YYYY-MM-DDTHH:MM` (or with a space) as its parts, checked.
pub fn parse_local_datetime(text: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let text = text.trim();
    let (date, time) = text.split_once(['T', ' '])?;
    let mut parts = date.split('-');
    let (y, mo, d) = (
        parts.next()?.parse::<i32>().ok()?,
        parts.next()?.parse::<u32>().ok()?,
        parts.next()?.parse::<u32>().ok()?,
    );
    let (h, mi) = parse_hhmm(time)?;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    ((2000..=2999).contains(&y) && (1..=days).contains(&d)).then_some((y, mo, d, h, mi))
}

/// Which day of a month a monthly window falls on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum MonthDay {
    /// The day of the month (1–31; a shorter month: its last day).
    Date { day: u32 },
    /// The `week`-th `weekday` of the month (1–4, 5 = the last), e.g.
    /// the 2nd Tuesday (Patch Tuesday). `weekday`: 1 = Monday … 7 = Sunday.
    Weekday { week: u32, weekday: u32 },
}

/// When a maintenance window is open, in its own time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "every", rename_all = "snake_case")]
pub enum Schedule {
    /// Once, from `start` to `end` (`YYYY-MM-DDTHH:MM`, local time).
    Once { start: String, end: String },
    /// On `days` (1 = Monday … 7 = Sunday) at `start` (`HH:MM`) for `minutes`.
    Weekly {
        days: Vec<u32>,
        start: String,
        minutes: u32,
    },
    /// On one day each month at `start` for `minutes`.
    Monthly {
        on: MonthDay,
        start: String,
        minutes: u32,
    },
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

impl Schedule {
    /// The longest a window lasts: a week (a longer one is a silence).
    pub const MAX_MINUTES: u32 = 7 * 24 * 60;

    /// `Err` with what's wrong, for the form and the API alike.
    pub fn validate(&self) -> Result<(), String> {
        let minutes = |m: u32| match (1..=Self::MAX_MINUTES).contains(&m) {
            true => Ok(()),
            false => Err("a window lasts 1 minute to 7 days".to_string()),
        };
        let time = |t: &str| {
            parse_hhmm(t)
                .map(|_| ())
                .ok_or(format!("{t}: a time is HH:MM"))
        };
        match self {
            Schedule::Once { start, end } => {
                let s = parse_local_datetime(start)
                    .ok_or(format!("{start}: a start is a date and time"))?;
                let e =
                    parse_local_datetime(end).ok_or(format!("{end}: an end is a date and time"))?;
                if e <= s {
                    return Err("the end is after the start".into());
                }
                Ok(())
            }
            Schedule::Weekly {
                days,
                start,
                minutes: m,
            } => {
                if days.is_empty() || days.iter().any(|d| !(1..=7).contains(d)) {
                    return Err("choose the days of the week".into());
                }
                time(start)?;
                minutes(*m)
            }
            Schedule::Monthly {
                on,
                start,
                minutes: m,
            } => {
                match on {
                    MonthDay::Date { day } if !(1..=31).contains(day) => {
                        return Err("a day of the month is 1 to 31".into());
                    }
                    MonthDay::Weekday { week, weekday }
                        if !(1..=5).contains(week) || !(1..=7).contains(weekday) =>
                    {
                        return Err("choose which weekday of the month".into());
                    }
                    _ => {}
                }
                time(start)?;
                minutes(*m)
            }
        }
    }

    /// In words, e.g. `every Tue, Thu at 22:00 for 4 h`.
    pub fn describe(&self) -> String {
        let length = |m: u32| match (m / 60, m % 60) {
            (0, m) => format!("{m} min"),
            (h, 0) => format!("{h} h"),
            (h, m) => format!("{h} h {m} min"),
        };
        match self {
            Schedule::Once { start, end } => {
                format!(
                    "from {} to {}",
                    start.replace('T', " "),
                    end.replace('T', " ")
                )
            }
            Schedule::Weekly {
                days,
                start,
                minutes,
            } => {
                let mut days = days.clone();
                days.sort_unstable();
                days.dedup();
                let names: Vec<&str> = days
                    .iter()
                    .filter_map(|d| WEEKDAYS.get((*d as usize).wrapping_sub(1)).copied())
                    .collect();
                format!(
                    "every {} at {start} for {}",
                    names.join(", "),
                    length(*minutes)
                )
            }
            Schedule::Monthly { on, start, minutes } => {
                let day = match on {
                    MonthDay::Date { day } => format!("on day {day}"),
                    MonthDay::Weekday { week, weekday } => {
                        let nth = ["1st", "2nd", "3rd", "4th", "last"]
                            .get((*week as usize).wrapping_sub(1))
                            .copied()
                            .unwrap_or("?");
                        let name = WEEKDAYS
                            .get((*weekday as usize).wrapping_sub(1))
                            .copied()
                            .unwrap_or("?");
                        format!("on the {nth} {name}")
                    }
                };
                format!("every month {day} at {start} for {}", length(*minutes))
            }
        }
    }
}

/// A maintenance window: planned work during which a scope sends no alerts
/// and its results don't count for uptime (D-65).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    pub id: i64,
    pub company_id: String,
    pub scope: Scope,
    /// Why, e.g. `Patch Tuesday`.
    pub reason: String,
    /// The IANA zone its times are in, e.g. `Europe/Madrid`.
    pub timezone: String,
    pub schedule: Schedule,
    pub created_by: Option<String>,
    pub created_at: String,
}

/// `POST /api/companies/{id}/maintenance`: a new window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewWindowRequest {
    pub scope: Scope,
    pub reason: String,
    pub timezone: String,
    pub schedule: Schedule,
}

impl NewWindowRequest {
    /// The checks a form can make (the time zone is checked on the hub).
    pub fn validate(&self) -> Result<(), String> {
        if self.reason.trim().is_empty() {
            return Err("say what the maintenance is for".into());
        }
        if self.timezone.trim().is_empty() {
            return Err("choose a time zone".into());
        }
        self.schedule.validate()
    }
}

/// A window as the maintenance page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub window: MaintenanceWindow,
    /// What it covers, in words (`the whole company`, `site HQ`, …).
    pub scope_label: String,
    pub schedule_text: String,
    /// Open now.
    pub active: bool,
    /// The current or next opening, in the window's zone (`None`: over).
    pub next: Option<String>,
}

/// `POST /api/silence`: silence alerts now, for a while or until removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateSilenceRequest {
    pub company_id: String,
    pub scope: Scope,
    pub reason: String,
    /// `None`: until removed (or, for a check, until it recovers).
    #[serde(default)]
    pub duration_hours: Option<u32>,
}

/// A check a window or a silence can name, for the maintenance page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceTarget {
    pub probe_id: String,
    pub probe_name: String,
    pub site_id: String,
    /// `(check ID, name)`, by name.
    pub checks: Vec<(String, String)>,
}

/// `GET /api/companies/{id}/maintenance`: a company's windows and silences,
/// and what they can cover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaintenanceOverview {
    pub windows: Vec<WindowInfo>,
    pub silences: Vec<AlertSilence>,
    pub sites: Vec<SiteInfo>,
    pub probes: Vec<MaintenanceTarget>,
}

// ===========================================================================
// Alert channels and routing (M3-03, proposal 0005 §2.6, D-04, D-66)
// ===========================================================================

/// What an alert says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Down,
    Warning,
    Recovered,
    /// A probe stopped reporting.
    ProbeExpired,
}

impl AlertKind {
    pub const ALL: [AlertKind; 4] = [
        AlertKind::Down,
        AlertKind::Warning,
        AlertKind::Recovered,
        AlertKind::ProbeExpired,
    ];

    /// As alerts' subjects end: `DOWN`, `WARNING`, `RECOVERED`, `EXPIRED`.
    pub fn word(self) -> &'static str {
        match self {
            AlertKind::Down => "DOWN",
            AlertKind::Warning => "WARNING",
            AlertKind::Recovered => "RECOVERED",
            AlertKind::ProbeExpired => "EXPIRED",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AlertKind::Down => "Down",
            AlertKind::Warning => "Warning",
            AlertKind::Recovered => "Recovered",
            AlertKind::ProbeExpired => "Probe stopped reporting",
        }
    }
}

/// Where alerts can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// To addresses, through the hub's mail server.
    Email,
    /// A signed JSON POST to any URL.
    Webhook,
    /// A Microsoft Teams "Workflows" webhook.
    Teams,
    Slack,
    Discord,
    /// A Telegram bot, to a chat.
    Telegram,
    /// An ntfy topic, on ntfy.sh or the installer's own server (D-70).
    Ntfy,
}

impl ChannelKind {
    pub const ALL: [ChannelKind; 7] = [
        ChannelKind::Email,
        ChannelKind::Webhook,
        ChannelKind::Teams,
        ChannelKind::Slack,
        ChannelKind::Discord,
        ChannelKind::Telegram,
        ChannelKind::Ntfy,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ChannelKind::Email => "email",
            ChannelKind::Webhook => "webhook",
            ChannelKind::Teams => "teams",
            ChannelKind::Slack => "slack",
            ChannelKind::Discord => "discord",
            ChannelKind::Telegram => "telegram",
            ChannelKind::Ntfy => "ntfy",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }

    pub fn label(self) -> &'static str {
        match self {
            ChannelKind::Email => "E-mail",
            ChannelKind::Webhook => "Webhook",
            ChannelKind::Teams => "Microsoft Teams",
            ChannelKind::Slack => "Slack",
            ChannelKind::Discord => "Discord",
            ChannelKind::Telegram => "Telegram",
            ChannelKind::Ntfy => "ntfy",
        }
    }

    /// What its (non-secret) target is, for the form; `None`: it has none.
    pub fn target_label(self) -> Option<&'static str> {
        match self {
            ChannelKind::Email => Some("Addresses (separated by commas)"),
            ChannelKind::Webhook => Some("URL (https://…)"),
            ChannelKind::Telegram => Some("Chat ID"),
            ChannelKind::Ntfy => Some("Topic URL (https://ntfy.example.com/your-topic)"),
            ChannelKind::Teams | ChannelKind::Slack | ChannelKind::Discord => None,
        }
    }

    /// What its secret is, for the form; `None`: it has none.
    pub fn secret_label(self) -> Option<&'static str> {
        match self {
            ChannelKind::Email => None,
            ChannelKind::Webhook => Some("Signing key (the receiver checks the signature with it)"),
            ChannelKind::Teams => Some("Workflow webhook URL"),
            ChannelKind::Slack => Some("Incoming webhook URL"),
            ChannelKind::Discord => Some("Webhook URL"),
            ChannelKind::Telegram => Some("Bot token (from @BotFather)"),
            ChannelKind::Ntfy => Some("Access token (optional: if the server needs one)"),
        }
    }

    /// Must it have its secret? ntfy's token is optional (a public topic).
    pub fn secret_required(self) -> bool {
        !matches!(self, ChannelKind::Email | ChannelKind::Ntfy)
    }
}

/// An ntfy topic URL's server and topic: `https://ntfy.example.com/alerts`
/// is `("https://ntfy.example.com", "alerts")` (D-70). `None` if it isn't
/// `http(s)://host[/path]/topic` with a topic of letters, digits, `-`, `_`.
pub fn ntfy_parts(url: &str) -> Option<(&str, &str)> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, _) = rest.split_once('/')?;
    let (server, topic) = url.rsplit_once('/')?;
    let topic_ok = (1..=64).contains(&topic.len())
        && topic
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (!host.is_empty() && topic_ok).then_some((server, topic))
}

/// A topic shorter than this on a server anyone can read is easy to guess.
pub const NTFY_SAFE_TOPIC_LEN: usize = 16;

/// The e-mail addresses in a channel's target, trimmed.
pub fn addresses(target: &str) -> Vec<String> {
    target
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect()
}

fn looks_like_address(address: &str) -> bool {
    let mut parts = address.split('@');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(user), Some(domain), None)
            if !user.is_empty() && domain.contains('.') && !address.contains(char::is_whitespace)
    )
}

/// A channel as an Admin sets it (`POST /api/admin/alerts/channels[/{id}]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRequest {
    pub name: String,
    pub kind: ChannelKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Addresses, a webhook's URL, a Telegram chat ID (see
    /// [`ChannelKind::target_label`]); empty for the others.
    #[serde(default)]
    pub target: String,
    /// The secret ([`ChannelKind::secret_label`]). Write-only: `None` on a
    /// change keeps the stored one.
    #[serde(default)]
    pub secret: Option<String>,
}

impl ChannelRequest {
    /// `Err` with what's wrong, for the form and the API alike. `has_secret`:
    /// a secret is stored already (a change may leave it out).
    pub fn validate(&self, has_secret: bool) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("give the channel a name".into());
        }
        let target = self.target.trim();
        let secret = self
            .secret
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let needs_secret = self.kind.secret_required();
        if needs_secret && secret.is_none() && !has_secret {
            return Err(format!(
                "the {} is needed",
                self.kind
                    .secret_label()
                    .unwrap_or("secret")
                    .split(" (")
                    .next()
                    .unwrap_or("secret")
                    .to_lowercase()
            ));
        }
        let https = |url: &str| url.starts_with("https://") && url.len() > "https://".len();
        match self.kind {
            ChannelKind::Email => {
                let list = addresses(target);
                if list.is_empty() {
                    return Err("give at least one address".into());
                }
                if let Some(bad) = list.iter().find(|a| !looks_like_address(a)) {
                    return Err(format!("{bad} isn't an e-mail address"));
                }
            }
            ChannelKind::Webhook if !(https(target) || target.starts_with("http://")) => {
                return Err("the webhook URL starts with https:// (or http://)".into());
            }
            ChannelKind::Teams | ChannelKind::Slack | ChannelKind::Discord => {
                if let Some(url) = secret
                    && !https(url)
                {
                    return Err("the webhook URL starts with https://".into());
                }
            }
            ChannelKind::Telegram if target.is_empty() => {
                return Err("give the chat ID (e.g. -1001234567890)".into());
            }
            ChannelKind::Ntfy if ntfy_parts(target).is_none() => {
                return Err(
                    "the topic URL is https://<your ntfy server>/<topic> (letters, digits, - and _)"
                        .into(),
                );
            }
            _ => {}
        }
        Ok(())
    }
}

/// A channel as the alerts page shows it (never its secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: i64,
    pub name: String,
    pub kind: ChannelKind,
    pub enabled: bool,
    pub target: String,
    /// A secret is stored (it is never shown).
    pub secret_set: bool,
    pub last_delivery_at: Option<String>,
    /// Why the last delivery failed (`None`: it worked, or none yet).
    pub last_error: Option<String>,
}

/// A routing rule: which alerts go to which channels. Rules are tried in
/// order; an empty list matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRule {
    pub name: String,
    /// Company IDs; empty: all.
    #[serde(default)]
    pub companies: Vec<String>,
    /// `company/site` IDs; empty: all.
    #[serde(default)]
    pub sites: Vec<String>,
    /// Empty: all kinds.
    #[serde(default)]
    pub kinds: Vec<AlertKind>,
    /// Check kinds (`ping`, `disk`, …); empty: all (and probe alerts).
    #[serde(default)]
    pub check_kinds: Vec<String>,
    /// Channel IDs it sends to.
    pub channels: Vec<i64>,
    /// No later rule is tried once this one matched.
    #[serde(default)]
    pub stop: bool,
    /// Remind its channels every this many minutes while the incident is
    /// open and nobody acknowledged it (D-69); `None`: never.
    #[serde(default)]
    pub remind_every_minutes: Option<u32>,
    /// Escalate after this many minutes unacknowledged, to `escalate_to`.
    #[serde(default)]
    pub escalate_after_minutes: Option<u32>,
    /// Channel IDs an escalation (and then the recovery) goes to.
    #[serde(default)]
    pub escalate_to: Vec<i64>,
}

impl RouteRule {
    /// Does it match an alert of `kind` about `company`, `site`, and a check
    /// of `check_kind` (`None`: about the probe itself)?
    pub fn matches(
        &self,
        kind: AlertKind,
        company: &str,
        site: &str,
        check_kind: Option<&str>,
    ) -> bool {
        let site_key = format!("{company}/{site}");
        (self.companies.is_empty() || self.companies.iter().any(|c| c == company))
            && (self.sites.is_empty() || self.sites.contains(&site_key))
            && (self.kinds.is_empty() || self.kinds.contains(&kind))
            && (self.check_kinds.is_empty()
                || check_kind.is_some_and(|k| self.check_kinds.iter().any(|c| c == k)))
    }
}

/// The rules an alert matches, in order, up to the first matching one
/// that says stop.
pub fn matching<'a>(
    rules: &'a [RouteRule],
    kind: AlertKind,
    company: &str,
    site: &str,
    check_kind: Option<&str>,
) -> Vec<&'a RouteRule> {
    let mut matched = Vec::new();
    for rule in rules
        .iter()
        .filter(|r| r.matches(kind, company, site, check_kind))
    {
        matched.push(rule);
        if rule.stop {
            break;
        }
    }
    matched
}

/// `ids` and then `more`, each once.
fn union(mut ids: Vec<i64>, more: &[i64]) -> Vec<i64> {
    for id in more {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    ids
}

/// The channels an alert goes to: every matching rule's, in order, each
/// once, until a matching rule says stop.
pub fn route(
    rules: &[RouteRule],
    kind: AlertKind,
    company: &str,
    site: &str,
    check_kind: Option<&str>,
) -> Vec<i64> {
    matching(rules, kind, company, site, check_kind)
        .into_iter()
        .fold(Vec::new(), |ids, rule| union(ids, &rule.channels))
}

/// What the rules an alert matches say about its incident while nobody
/// acknowledges it (D-69): reminders (the shortest interval, to every
/// reminding rule's channels) and one escalation (at the earliest time, to
/// every escalating rule's channels).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FollowUp {
    /// `(every minutes, channels)`.
    pub remind: Option<(u32, Vec<i64>)>,
    /// `(after minutes, channels)`.
    pub escalate: Option<(u32, Vec<i64>)>,
}

pub fn follow_up(
    rules: &[RouteRule],
    kind: AlertKind,
    company: &str,
    site: &str,
    check_kind: Option<&str>,
) -> FollowUp {
    let mut follow = FollowUp::default();
    for rule in matching(rules, kind, company, site, check_kind) {
        if let Some(every) = rule.remind_every_minutes {
            follow.remind = Some(match follow.remind.take() {
                None => (every, rule.channels.clone()),
                Some((was, ids)) => (was.min(every), union(ids, &rule.channels)),
            });
        }
        if let Some(after) = rule.escalate_after_minutes {
            follow.escalate = Some(match follow.escalate.take() {
                None => (after, rule.escalate_to.clone()),
                Some((was, ids)) => (was.min(after), union(ids, &rule.escalate_to)),
            });
        }
    }
    follow
}

/// How often a rule may remind, in minutes: every 15 minutes to once a week.
pub const REMIND_MINUTES: std::ops::RangeInclusive<u32> = 15..=10_080;
/// When a rule may escalate, in minutes unacknowledged: 5 minutes to a day.
pub const ESCALATE_MINUTES: std::ops::RangeInclusive<u32> = 5..=1440;

/// The routing rules, as saved (`POST /api/admin/alerts/routes`): in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutesRequest {
    pub rules: Vec<RouteRule>,
}

impl RoutesRequest {
    pub fn validate(&self, channels: &[i64]) -> Result<(), String> {
        for (n, rule) in self.rules.iter().enumerate() {
            let which = match rule.name.trim() {
                "" => format!("rule {}", n + 1),
                name => format!("rule {name}"),
            };
            if rule.channels.is_empty() {
                return Err(format!("{which}: choose where it sends"));
            }
            if rule
                .channels
                .iter()
                .chain(&rule.escalate_to)
                .any(|c| !channels.contains(c))
            {
                return Err(format!("{which}: a channel it names doesn't exist"));
            }
            if let Some(every) = rule.remind_every_minutes
                && !REMIND_MINUTES.contains(&every)
            {
                return Err(format!(
                    "{which}: remind every {}-{} minutes",
                    REMIND_MINUTES.start(),
                    REMIND_MINUTES.end()
                ));
            }
            match (rule.escalate_after_minutes, rule.escalate_to.is_empty()) {
                (Some(after), false) if !ESCALATE_MINUTES.contains(&after) => {
                    return Err(format!(
                        "{which}: escalate after {}-{} minutes",
                        ESCALATE_MINUTES.start(),
                        ESCALATE_MINUTES.end()
                    ));
                }
                (Some(_), true) => return Err(format!("{which}: choose where it escalates to")),
                (None, false) => return Err(format!("{which}: say after how long it escalates")),
                _ => {}
            }
        }
        Ok(())
    }
}

/// How the hub talks to its mail server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailSecurity {
    /// STARTTLS, usually port 587.
    #[default]
    Starttls,
    /// TLS from the start, usually port 465.
    Tls,
    /// None, e.g. a relay on the local network (port 25).
    None,
}

impl MailSecurity {
    pub fn as_str(self) -> &'static str {
        match self {
            MailSecurity::Starttls => "starttls",
            MailSecurity::Tls => "tls",
            MailSecurity::None => "none",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [
            MailSecurity::Starttls,
            MailSecurity::Tls,
            MailSecurity::None,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
    }
}

/// The hub's alert settings: its mail server, and its address for links.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertSettings {
    /// E.g. `https://hub.example.com`: alerts link to the check's page.
    /// Empty: no links.
    #[serde(default)]
    pub hub_url: String,
    #[serde(default)]
    pub smtp_server: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_security: MailSecurity,
    #[serde(default)]
    pub smtp_username: String,
    /// The sender address.
    #[serde(default)]
    pub smtp_from: String,
    /// Write-only: `None` keeps the stored password; `Some("")` clears it.
    #[serde(default)]
    pub smtp_password: Option<String>,
    /// A password is stored (answers only; never the password).
    #[serde(default)]
    pub smtp_password_set: bool,
}

fn default_smtp_port() -> u16 {
    587
}

impl AlertSettings {
    pub fn validate(&self) -> Result<(), String> {
        let url = self.hub_url.trim();
        if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("the hub address starts with https:// (or http://)".into());
        }
        if !self.smtp_server.trim().is_empty() {
            if self.smtp_port == 0 {
                return Err("give the mail server's port".into());
            }
            if !looks_like_address(self.smtp_from.trim()) {
                return Err("give the sender address (From)".into());
            }
        }
        Ok(())
    }
}

/// The daily digest (M3-09, D-71): every day at `hour` (local to
/// `timezone`), a summary e-mail to `channels`. No channels: off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSettings {
    #[serde(default = "default_digest_hour")]
    pub hour: u8,
    /// IANA, e.g. `Europe/Madrid` (checked on the hub).
    #[serde(default = "default_digest_timezone")]
    pub timezone: String,
    /// E-mail channel IDs.
    #[serde(default)]
    pub channels: Vec<i64>,
}

fn default_digest_hour() -> u8 {
    7
}

fn default_digest_timezone() -> String {
    "UTC".into()
}

impl Default for DigestSettings {
    fn default() -> Self {
        Self {
            hour: default_digest_hour(),
            timezone: default_digest_timezone(),
            channels: Vec::new(),
        }
    }
}

impl DigestSettings {
    /// `Err` with what's wrong; `email_channels`: the IDs of the e-mail
    /// channels (the time zone itself is checked on the hub).
    pub fn validate(&self, email_channels: &[i64]) -> Result<(), String> {
        if self.hour > 23 {
            return Err("the digest goes at an hour from 0 to 23".into());
        }
        if self.timezone.trim().is_empty() {
            return Err("give the digest's time zone (e.g. Europe/Madrid, UTC)".into());
        }
        if self.channels.iter().any(|c| !email_channels.contains(c)) {
            return Err("the digest goes to e-mail channels".into());
        }
        Ok(())
    }
}

/// A company and its sites, for choosing what a rule matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTarget {
    pub company_id: String,
    pub company_name: String,
    /// `(site ID, name)`.
    pub sites: Vec<(String, String)>,
}

/// `GET /api/admin/alerts`: the alerts page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertsOverview {
    pub settings: AlertSettings,
    pub channels: Vec<ChannelInfo>,
    pub rules: Vec<RouteRule>,
    pub companies: Vec<RouteTarget>,
    /// The daily digest (D-71), and the local date it last went out.
    #[serde(default)]
    pub digest: DigestSettings,
    #[serde(default)]
    pub digest_last_sent: Option<String>,
}

/// The answer to "Send test": how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    pub ok: bool,
    pub error: Option<String>,
}

// ===========================================================================
// Incidents (M3-05, proposal 0005 §2.3, D-67)
// ===========================================================================

/// What happened to an incident, in its timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentEventKind {
    /// It started: the first alert (or what would have been one, silenced).
    Opened,
    /// Down became Warning, or the other way.
    Changed,
    /// Someone is on it.
    Acknowledged,
    /// Nobody acknowledged it: a reminder went out (D-69).
    Reminded,
    /// Nobody acknowledged it in time: it was escalated (D-69).
    Escalated,
    /// A note someone added.
    Note,
    /// It ended: recovered, the probe reports again, or the check was deleted.
    Closed,
}

impl IncidentEventKind {
    pub const ALL: [IncidentEventKind; 7] = [
        IncidentEventKind::Opened,
        IncidentEventKind::Changed,
        IncidentEventKind::Acknowledged,
        IncidentEventKind::Reminded,
        IncidentEventKind::Escalated,
        IncidentEventKind::Note,
        IncidentEventKind::Closed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IncidentEventKind::Opened => "opened",
            IncidentEventKind::Changed => "changed",
            IncidentEventKind::Acknowledged => "acknowledged",
            IncidentEventKind::Reminded => "reminded",
            IncidentEventKind::Escalated => "escalated",
            IncidentEventKind::Note => "note",
            IncidentEventKind::Closed => "closed",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }

    /// Written by a person, so internal: Clients don't see it (D-67).
    pub fn is_internal(self) -> bool {
        matches!(
            self,
            IncidentEventKind::Acknowledged | IncidentEventKind::Note
        )
    }
}

/// An incident, as the API shows it. Times are RFC 3339 (UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentInfo {
    pub id: i64,
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
    pub probe_id: String,
    pub probe_name: String,
    /// `None`: the probe stopped reporting.
    pub check_id: Option<String>,
    pub check_name: Option<String>,
    /// The worst it got: Down or Warning.
    pub severity: Status,
    /// Now (while open), or when it closed.
    pub status: Status,
    pub opened_at: String,
    /// `None`: still open.
    pub closed_at: Option<String>,
    /// How long it lasted, or has lasted so far, e.g. `1 h 5 min`.
    pub duration: String,
    pub acknowledged_by: Option<String>,
    pub acknowledged_at: Option<String>,
    /// Reminders sent while nobody acknowledged it (D-69).
    #[serde(default)]
    pub reminders: u32,
    /// When it was escalated, if it was.
    #[serde(default)]
    pub escalated_at: Option<String>,
}

impl IncidentInfo {
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    /// What it is about: the check's name, or the probe's.
    pub fn title(&self) -> String {
        match &self.check_name {
            Some(check) => check.clone(),
            None => format!("Probe {} stopped reporting", self.probe_name),
        }
    }

    /// `company › site › probe`.
    pub fn place(&self) -> String {
        format!(
            "{} › {} › {}",
            self.company_name, self.site_name, self.probe_name
        )
    }
}

/// One line of an incident's timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentEvent {
    pub at: String,
    pub kind: IncidentEventKind,
    /// `None`: the hub.
    pub username: Option<String>,
    pub text: String,
}

/// `GET /api/incidents/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentDetail {
    pub incident: IncidentInfo,
    /// Oldest first; without notes and acknowledgement notes for Clients.
    pub events: Vec<IncidentEvent>,
}

/// `GET /api/incidents?state=…`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentsQuery {
    /// `open` (default), `closed` or `all`.
    #[serde(default)]
    pub state: Option<String>,
}

/// The body of `acknowledge` (the note may be empty) and of `notes`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentNote {
    #[serde(default)]
    pub text: String,
}

/// The longest note.
pub const MAX_NOTE_CHARS: usize = 2000;

impl IncidentNote {
    /// `Err` with what's wrong; `required`: a note can't be empty.
    pub fn validate(&self, required: bool) -> Result<(), String> {
        let text = self.text.trim();
        if required && text.is_empty() {
            return Err("write the note".into());
        }
        if text.chars().count() > MAX_NOTE_CHARS {
            return Err(format!("a note is at most {MAX_NOTE_CHARS} characters"));
        }
        Ok(())
    }
}

/// A length of time for people: `45 s`, `3 min`, `1 h 5 min`, `2 d 4 h`.
pub fn duration_text(seconds: i64) -> String {
    let s = seconds.max(0);
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s} s"),
        (0, 0, m) => format!("{m} min"),
        (0, h, 0) => format!("{h} h"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, 0, _) => format!("{d} d"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

// ===========================================================================
// Identity helpers
// ===========================================================================

/// First 8 characters of a probe ID (the hex of its public key), for logs and
/// compact display.
pub fn short_probe_id(probe_id: &str) -> &str {
    probe_id.get(..8).unwrap_or(probe_id)
}

// ===========================================================================
// API types (JSON shapes)
// ===========================================================================

/// `GET /api/dashboard/companies` (also the archive viewer).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DashboardCompany {
    pub company_id: String,
    /// Display name (the company record's; the ID until someone sets one).
    pub name: String,
    pub total_probes: i64,
    pub active_probes: i64,
    pub expired_probes: i64,
    pub devices_up: i64,
    /// All devices down in each probe's latest report, including stale ones
    /// from expired probes. Use `severity` to judge the company.
    pub devices_down: i64,
    /// Devices over a warning threshold, counted like `devices_down`.
    pub devices_warning: i64,
    pub severity: Severity,
    /// ISO 8601; `None` if no reports yet.
    pub last_report: Option<String>,
}

/// `GET /api/company/{company_id}/probes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanyProbe {
    pub probe_id: String,
    /// Name from the hub's probe whitelist; empty if unnamed.
    pub probe_name: String,
    pub hostname: Option<String>,
    /// The site's display name (its record's), from the probe's latest report.
    pub site: Option<String>,
    /// `"active"` or `"expired"`.
    pub status: String,
    pub severity: Severity,
    pub last_seen_at: Option<String>,
    pub devices_up: i64,
    pub devices_down: i64,
    pub devices_warning: i64,
}

impl CompanyProbe {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// The whitelist name, or the short probe ID followed by `…` if unnamed.
    pub fn display_name(&self) -> String {
        if self.probe_name.is_empty() {
            format!("{}…", short_probe_id(&self.probe_id))
        } else {
            self.probe_name.clone()
        }
    }
}

/// `GET /api/probe/{probe_id}/devices`: one check's result from the probe's
/// latest report. Checks deleted since then are left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeDevice {
    pub check_id: String,
    pub name: String,
    /// The check's kind (`ping`, `http`, …).
    pub kind: String,
    pub target: String,
    /// As the hub judged it (D-64); `Unknown` while its probe is expired.
    pub status: Status,
    /// Why a threshold made it Warning or Down.
    pub reason: Option<String>,
    /// Its latest result came during a maintenance window (D-65).
    #[serde(default)]
    pub in_maintenance: bool,
    pub message: Option<String>,
    pub latency_ms: Option<i64>,
    /// The measured values, headline first.
    pub metrics: Vec<Metric>,
}

impl ProbeDevice {
    /// The headline metric, as `42.5%` (`None` without metrics).
    pub fn headline(&self) -> Option<String> {
        self.metrics
            .first()
            .map(|m| format!("{:.1}{}", m.value, m.unit))
    }
}

/// `GET /api/company/{company_id}/uptime`: one time bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UptimeBucket {
    /// `"YYYY-MM-DD HH:MM"`, UTC.
    pub bucket: String,
    /// Results that worked: Up, and Warning (it works; look at it, D-64).
    /// Results during maintenance windows aren't counted (D-65).
    pub up_count: i64,
    pub down_count: i64,
    /// Results during maintenance windows: the chart shades the bucket.
    #[serde(default)]
    pub maintenance_count: i64,
}

/// `GET /api/admin/probes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminProbe {
    pub probe_id: String,
    pub company_id: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub first_seen_at: Option<String>,
}

/// `GET /api/admin/archives`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveMetadata {
    pub id: i64,
    pub filename: String,
    pub created_at: String,
    pub cutoff_date: String,
    pub size_mb: i64,
}

/// `GET /api/admin/storage-info`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageInfo {
    pub current_db_size_mb: u64,
    pub warn_threshold_mb: u64,
    pub retention_days: u32,
    pub last_archive_date: Option<String>,
    pub last_archive_size_mb: Option<i64>,
    pub warning: bool,
}

/// `GET /api/silences`: alerts silenced now, for a check, a probe, a site
/// or a whole company (D-65).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertSilence {
    pub id: i64,
    pub company_id: String,
    pub scope: Scope,
    pub reason: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

impl AlertSilence {
    /// Is this the silence of check `check_id` (not a wider one)?
    pub fn is_of_check(&self, check_id: &str) -> bool {
        matches!(&self.scope, Scope::Check { check_id: c } if c == check_id)
    }
}

// ===========================================================================
// Users, roles and permissions (proposal 0001)
// ===========================================================================

/// What a user is. Global roles see every company; the others will be limited
/// to their assigned companies once scoping is enforced (M1-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Everything, including managing Owners. At least one active Owner exists.
    Owner,
    /// Everything except managing Owners.
    Admin,
    /// Views data and silences alerts.
    Technician,
    /// Read-only (internal staff).
    Viewer,
    /// Read-only view of their own companies (needs M1-04).
    Client,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Owner,
        Role::Admin,
        Role::Technician,
        Role::Viewer,
        Role::Client,
    ];

    /// Lowercase name, as serialised and stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Technician => "technician",
            Role::Viewer => "viewer",
            Role::Client => "client",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Role::ALL.into_iter().find(|r| r.as_str() == value)
    }

    /// Display name ("Owner", …).
    pub fn label(self) -> &'static str {
        match self {
            Role::Owner => "Owner",
            Role::Admin => "Admin",
            Role::Technician => "Technician",
            Role::Viewer => "Viewer",
            Role::Client => "Client",
        }
    }

    /// Owner and Admin always see every company.
    pub fn is_global(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

/// Something a request needs to be allowed to do. Every API route declares
/// one (`crate::routes`); the UI uses the same check to hide what it can't do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Dashboards, company and probe views, uptime, archives, silence list.
    ViewData,
    /// Create and remove alert silences.
    Silence,
    /// The admin page: storage size, archive list, company and probe listings.
    ViewAdmin,
    /// Delete data, remove probes, create archives.
    ManageData,
    /// Create and edit users (only Owners edit Owners, see `service::users`).
    ManageUsers,
    /// Edit companies and sites: names, contacts, notes, addresses (within
    /// the user's company scope). Archiving is `ManageData`.
    ManageCompanies,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Permission::ViewData,
        Permission::Silence,
        Permission::ViewAdmin,
        Permission::ManageData,
        Permission::ManageUsers,
        Permission::ManageCompanies,
    ];
}

/// The single authorization rule: may `role` do `permission`?
pub fn authorize(role: Role, permission: Permission) -> bool {
    use Permission::*;
    match permission {
        ViewData => true,
        Silence | ManageCompanies => {
            matches!(role, Role::Owner | Role::Admin | Role::Technician)
        }
        ViewAdmin | ManageData | ManageUsers => role.is_global(),
    }
}

/// A user as the API shows it (`/api/auth/me`, `/api/users`). Never includes
/// the password hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    /// Which companies the user sees (ignored for Owner and Admin, who see
    /// all: use [`UserInfo::effective_scope`]).
    pub scope: CompanyScope,
    pub is_active: bool,
    /// Must set a new password before doing anything else.
    pub must_change_password: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

impl UserInfo {
    pub fn can(&self, permission: Permission) -> bool {
        authorize(self.role, permission)
    }

    /// The companies this user may see and act on. Owner and Admin: all.
    pub fn effective_scope(&self) -> CompanyScope {
        if self.role.is_global() {
            CompanyScope::All
        } else {
            self.scope.clone()
        }
    }
}

/// Which companies a user sees (D-45). One person monitoring their own
/// company never needs more than `All`, the default; an IT provider can give
/// each technician, viewer or customer login exactly their companies.
/// JSON: `{"mode": "all"}`, `{"mode": "only", "companies": [...]}`,
/// `{"mode": "except", "companies": [...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "companies", rename_all = "lowercase")]
pub enum CompanyScope {
    /// Every company, including ones registered later.
    #[default]
    All,
    /// Only these companies.
    Only(BTreeSet<String>),
    /// Every company except these, including ones registered later.
    #[serde(rename = "except")]
    AllExcept(BTreeSet<String>),
}

impl CompanyScope {
    pub fn allows(&self, company_id: &str) -> bool {
        match self {
            CompanyScope::All => true,
            CompanyScope::Only(ids) => ids.contains(company_id),
            CompanyScope::AllExcept(ids) => !ids.contains(company_id),
        }
    }

    /// Stored form: (`all` | `only` | `except`, listed companies).
    pub fn mode(&self) -> &'static str {
        match self {
            CompanyScope::All => "all",
            CompanyScope::Only(_) => "only",
            CompanyScope::AllExcept(_) => "except",
        }
    }

    pub fn companies(&self) -> Vec<String> {
        match self {
            CompanyScope::All => Vec::new(),
            CompanyScope::Only(ids) | CompanyScope::AllExcept(ids) => ids.iter().cloned().collect(),
        }
    }

    /// Build from the stored form; an unknown mode is `None`.
    pub fn from_parts(mode: &str, companies: impl IntoIterator<Item = String>) -> Option<Self> {
        let ids = companies.into_iter().collect();
        match mode {
            "all" => Some(CompanyScope::All),
            "only" => Some(CompanyScope::Only(ids)),
            "except" => Some(CompanyScope::AllExcept(ids)),
            _ => None,
        }
    }
}

/// `POST /api/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// `GET /api/setup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupStatus {
    /// No user exists yet: the setup wizard must create the first Owner.
    pub needed: bool,
}

/// `POST /api/setup`: create the first Owner with the setup code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    pub code: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
}

/// `POST /api/me/password`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// `POST /api/users`. Without a password, the hub generates a temporary one
/// (returned once) that must be changed at first login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub role: Role,
    /// Defaults to all companies.
    #[serde(default)]
    pub scope: CompanyScope,
}

/// `POST /api/users/{id}`: the editable fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub role: Role,
    #[serde(default)]
    pub scope: CompanyScope,
    pub is_active: bool,
}

// ===========================================================================
// Companies and sites (proposal 0002 §4.1, D-44)
// ===========================================================================

/// A company: `GET /api/companies`, and the head of `CompanyDetail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanyInfo {
    /// The `company_id` probes send. Immutable: reports are stored under it.
    pub id: String,
    pub name: String,
    pub notes: Option<String>,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    /// Registered by a report and never edited yet.
    pub is_new: bool,
    /// Hidden from the dashboard (its probes can still report).
    pub archived: bool,
    pub created_at: String,
    pub site_count: i64,
    pub probe_count: i64,
}

/// A physical location of a company.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteInfo {
    /// The `site` probes send. Immutable.
    pub id: String,
    pub name: String,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub is_new: bool,
    /// Its IANA time zone, e.g. `Europe/Madrid`: maintenance windows there
    /// follow its local time (D-65).
    #[serde(default = "utc")]
    pub timezone: String,
}

fn utc() -> String {
    "UTC".into()
}

/// `GET /api/companies/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanyDetail {
    pub company: CompanyInfo,
    pub sites: Vec<SiteInfo>,
}

impl CompanyInfo {
    /// Without the provider's internal notes and contacts (what a Client sees).
    pub fn without_internal_fields(self) -> Self {
        Self {
            notes: None,
            contact_name: None,
            contact_email: None,
            contact_phone: None,
            ..self
        }
    }
}

impl CompanyDetail {
    /// Without internal notes and contacts, on the company and its sites.
    pub fn without_internal_fields(self) -> Self {
        Self {
            company: self.company.without_internal_fields(),
            sites: self
                .sites
                .into_iter()
                .map(|site| SiteInfo {
                    notes: None,
                    ..site
                })
                .collect(),
        }
    }
}

/// `POST /api/companies/{id}`: the editable fields.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCompanyRequest {
    pub name: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub contact_name: Option<String>,
    #[serde(default)]
    pub contact_email: Option<String>,
    #[serde(default)]
    pub contact_phone: Option<String>,
}

/// `POST /api/companies/{id}/sites/{site}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateSiteRequest {
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// IANA zone; default `UTC`.
    #[serde(default = "utc")]
    pub timezone: String,
}

/// `POST /api/companies`: a new company. `id` is its permanent key (a slug
/// such as `acme-ltd`); the UI derives it from the name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateCompanyRequest {
    pub id: String,
    pub name: String,
}

/// `POST /api/companies/{id}/sites`: a new site of a company.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateSiteRequest {
    pub id: String,
    pub name: String,
}

/// A slug for `name`: lowercase ASCII letters and digits joined by `-`
/// ("ACME Ltd." → `acme-ltd`). The UI suggests it; the hub validates IDs with
/// [`is_valid_slug`].
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').chars().take(64).collect()
}

/// A company or site ID: 1–64 of `a-z 0-9 - _ .`, starting with a letter or digit.
pub fn is_valid_slug(id: &str) -> bool {
    let valid =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.');
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(valid)
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
}

/// `POST /api/companies/{id}/archive`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveCompanyRequest {
    pub archived: bool,
}

// ===========================================================================
// Probes and enrollment codes (M2-01, D-47)
// ===========================================================================

/// A probe the hub knows (`GET /api/probes`): added by an enrollment code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeInfo {
    /// Hex of its Ed25519 public key.
    pub id: String,
    pub name: String,
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
    /// `active`, or `revoked` (its reports are refused).
    pub state: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub probe_version: Option<String>,
    pub enrolled_at: String,
    /// Its last report, if any.
    pub last_seen_at: Option<String>,
    /// Seconds between two reports.
    pub interval_secs: u32,
    /// The last update it refused or rolled back, and why (D-55).
    #[serde(default)]
    pub update_failed: Option<common::models::UpdateFailure>,
    /// It stopped reporting: nothing for the hub's probe timeout.
    #[serde(default)]
    pub silent: bool,
    /// The config revision it last said it runs.
    #[serde(default)]
    pub config_revision: Option<String>,
    /// The last config it refused, and why.
    #[serde(default)]
    pub config_rejected: Option<common::checks::ConfigRejection>,
    /// Whether it runs its current checks; `None` where not worked out
    /// (`GET /api/probes` works it out).
    #[serde(default)]
    pub config_sync: Option<ConfigSync>,
    /// What it last said about itself (protocol v8, D-73).
    #[serde(default)]
    pub health: Option<common::models::ProbeHealth>,
    /// It gets a release before the others (M4-01, D-74).
    #[serde(default)]
    pub canary: bool,
    /// Its next update, or why it waits; `None` where not worked out (the
    /// admin page's probe updates work it out).
    #[serde(default)]
    pub update_status: Option<UpdateStatus>,
}

impl ProbeInfo {
    /// What about this probe needs someone's attention (M4-02, D-73), for
    /// the fleet view; empty when all is well. A revoked probe needs none.
    pub fn concerns(&self) -> Vec<String> {
        if self.state != "active" {
            return Vec::new();
        }
        let mut concerns = Vec::new();
        if self.last_seen_at.is_none() {
            concerns.push("Never reported".to_string());
        } else if self.silent {
            concerns.push("Not reporting".to_string());
        }
        if self.config_sync == Some(ConfigSync::Rejected) {
            concerns.push("Refused its checks".to_string());
        }
        if let Some(failed) = &self.update_failed {
            concerns.push(format!("Didn't update to {}", failed.version));
        }
        if let Some(health) = &self.health {
            if health.queued_reports > 0 {
                concerns.push(format!("{} reports held back", health.queued_reports));
            }
            if let Some(offset) = health.clock_offset_secs
                && offset.abs() > common::protocol::CLOCK_WARNING_SECS
            {
                concerns.push(format!("Clock off by {offset} s"));
            }
            if health.last_error.is_some() {
                concerns.push("Had a problem".to_string());
            }
        }
        concerns
    }
}

/// An enrollment code that can still be used (`GET /api/enrollment-codes`).
/// The code itself is shown only once, when created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrollmentCodeInfo {
    pub id: String,
    pub company_id: String,
    pub site_id: String,
    pub probe_name: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    /// `None`: any number of probes until it expires or is revoked.
    pub max_uses: Option<i64>,
    pub uses: i64,
}

/// `POST /api/enrollment-codes`: "Add probe".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCodeRequest {
    pub company_id: String,
    pub site_id: String,
    /// The probe's name; empty: its hostname.
    #[serde(default)]
    pub probe_name: Option<String>,
    /// How many probes may enroll with it; omitted or `null`: any number
    /// until it expires or is revoked.
    #[serde(default)]
    pub max_uses: Option<u32>,
    /// How long it stays valid (default 24 hours).
    #[serde(default = "default_day")]
    pub valid_hours: u32,
    /// Where probes reach the hub, e.g. `https://hub.example.com:3443`.
    pub hub_url: String,
}

fn default_day() -> u32 {
    24
}

/// The answer to "Add probe": the code, shown only now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedCode {
    pub code: String,
    pub info: EnrollmentCodeInfo,
    /// Where probes reach the hub (in the code too): the wizard's download links.
    pub hub_url: String,
}

/// A probe binary the hub offers (`GET /api/probe-downloads`, M2-09, D-53).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeDownload {
    /// Rust target triple, e.g. `x86_64-unknown-linux-musl`.
    pub target: String,
    /// `linux` or `windows`.
    pub os: String,
    /// `x86_64`, `arm64`, `armv7`.
    pub arch: String,
    pub size: u64,
    /// Hex SHA-256 of the file: the one-line commands check the download with it.
    pub sha256: String,
    /// The release it comes from (D-55).
    pub version: String,
}

/// A signed probe release on the hub (D-55).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeRelease {
    pub version: String,
    pub released: String,
    /// The protocol version its probes speak.
    pub protocol: u16,
    /// e.g. `linux x86_64`, in the hub's platform order.
    pub platforms: Vec<String>,
    /// The versions its signed rollback lets go back to it (D-74).
    #[serde(default)]
    pub rollback_from: Vec<String>,
}

/// Where a rollout is (M4-01, D-74).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStage {
    /// Only the canary probes are offered the release.
    Canary,
    /// Every probe is.
    All,
}

impl RolloutStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Canary => "canary",
            Self::All => "all",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "canary" => Some(Self::Canary),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// How long after every canary runs the release it goes to every probe,
/// unless someone says otherwise when approving it.
pub const DEFAULT_SOAK_MINUTES: u32 = 60;
/// The longest wait after the canaries: a week.
pub const MAX_SOAK_MINUTES: u32 = 7 * 24 * 60;

/// The release probes are being offered, as the admin page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutInfo {
    pub version: String,
    pub approved_by: Option<String>,
    pub approved_at: String,
    pub stage: RolloutStage,
    /// Minutes to wait after every canary runs it.
    pub soak_minutes: u32,
    /// Canaries that follow the rollout, and how many of them run it.
    pub canaries: u32,
    pub canaries_ready: u32,
    /// When it goes to every probe on its own (every canary runs it).
    pub promote_at: Option<String>,
    pub promoted_at: Option<String>,
    /// Who sent it to every probe; `None` with `promoted_at`: the hub.
    pub promoted_by: Option<String>,
    pub paused: Option<RolloutPause>,
}

/// Why a rollout stopped offering the release, and who stopped it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutPause {
    pub at: String,
    /// `None`: the hub (a canary didn't install it).
    pub by: Option<String>,
    pub reason: String,
}

/// Whether a company's probes update (M4-01, D-74).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// To the release being rolled out (the default).
    #[default]
    Follow,
    /// Never: they stay on what they run.
    Off,
    /// To one release, whatever is rolled out.
    Pin,
}

impl UpdateMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Off => "off",
            Self::Pin => "pin",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "follow" => Some(Self::Follow),
            "off" => Some(Self::Off),
            "pin" => Some(Self::Pin),
            _ => None,
        }
    }
}

/// When a company's probes may update: local time at each probe's site,
/// `HH:MM` to `HH:MM`, past midnight if the end is earlier than the start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWindow {
    pub start: String,
    pub end: String,
}

impl UpdateWindow {
    pub fn validate(&self) -> Result<(), String> {
        let (Some(start), Some(end)) = (parse_hhmm(&self.start), parse_hhmm(&self.end)) else {
            return Err("give the update window as HH:MM to HH:MM, e.g. 02:00 to 05:00".into());
        };
        if start == end {
            return Err("the update window must end at another time than it starts".into());
        }
        Ok(())
    }

    /// Is local time `hour:minute` in the window? The start is in, the end out.
    pub fn contains(&self, hour: u32, minute: u32) -> bool {
        let (Some((sh, sm)), Some((eh, em))) = (parse_hhmm(&self.start), parse_hhmm(&self.end))
        else {
            return true; // never stored: validated first
        };
        let (start, end, now) = (sh * 60 + sm, eh * 60 + em, hour * 60 + minute);
        if start < end {
            (start..end).contains(&now)
        } else {
            now >= start || now < end
        }
    }

    pub fn label(&self) -> String {
        format!("{}–{}", self.start.trim(), self.end.trim())
    }
}

/// A company's update policy (`POST /api/admin/probe-updates/companies/{id}`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyUpdatePolicy {
    pub mode: UpdateMode,
    /// The release, with [`UpdateMode::Pin`].
    #[serde(default)]
    pub version: Option<String>,
    /// `None`: any time.
    #[serde(default)]
    pub window: Option<UpdateWindow>,
}

impl CompanyUpdatePolicy {
    /// The checks a form can make (that the release is on the hub is
    /// checked there). A version only goes with a pin.
    pub fn validate(&self) -> Result<(), String> {
        match (self.mode, self.version.as_deref().map(str::trim)) {
            (UpdateMode::Pin, None | Some("")) => {
                return Err("choose the release to pin the company to".into());
            }
            (UpdateMode::Follow | UpdateMode::Off, Some(_)) => {
                return Err("only a pinned company names a release".into());
            }
            _ => {}
        }
        self.window.as_ref().map_or(Ok(()), UpdateWindow::validate)
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A company and its update policy, on the admin page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyUpdates {
    pub company_id: String,
    pub company_name: String,
    pub policy: CompanyUpdatePolicy,
}

/// What a probe's next update is, or why it waits (M4-01, D-74).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// It runs the release meant for it.
    UpToDate,
    /// It is offered `version` with its next report.
    Updating { version: String },
    /// No release is approved, and its company has no pin.
    NoneApproved,
    /// Its company takes no updates.
    CompanyOff,
    /// It reports no version: too old to update itself.
    NoVersion,
    /// Revoked: it takes nothing.
    Revoked,
    /// The rollout is paused.
    Paused { version: String },
    /// The canaries go first.
    AwaitingCanaries { version: String },
    /// Its company's update window is closed at its site.
    OutsideWindow {
        version: String,
        window: UpdateWindow,
    },
    /// It refused or rolled back `version` (its last failed update).
    Refused { version: String },
    /// The release isn't on the hub (any more).
    Missing { version: String },
    /// Older than what it runs, and no signed rollback lets it go back.
    NoRollback { version: String },
}

impl UpdateStatus {
    /// For the admin page.
    pub fn describe(&self) -> String {
        match self {
            Self::UpToDate => "Up to date".into(),
            Self::Updating { version } => format!("Updating to {version}"),
            Self::NoneApproved => "No release approved".into(),
            Self::CompanyOff => "Off for its company".into(),
            Self::NoVersion => "Too old to update itself: install it again".into(),
            Self::Revoked => "Revoked".into(),
            Self::Paused { version } => format!("{version} paused"),
            Self::AwaitingCanaries { version } => format!("{version} after the canaries"),
            Self::OutsideWindow { version, window } => {
                format!("{version} in its window ({})", window.label())
            }
            Self::Refused { version } => format!("{version} not installed"),
            Self::Missing { version } => format!("{version} isn't on the hub"),
            Self::NoRollback { version } => {
                format!("{version} is older and has no signed rollback for it")
            }
        }
    }
}

/// The admin page's "Probe updates" (`GET /api/admin/probe-updates`, D-55).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeUpdates {
    /// The release being rolled out; `None`: none approved (companies
    /// pinned to a release still get it).
    pub rollout: Option<RolloutInfo>,
    /// Newest first.
    pub releases: Vec<ProbeRelease>,
    /// The hub trusts a release key (it was built with one): without one it
    /// can't take a release.
    pub has_release_key: bool,
    /// Every probe, with its version, last failed update, whether it is a
    /// canary, and its [`UpdateStatus`].
    pub probes: Vec<ProbeInfo>,
    /// Every company that isn't archived, with its update policy.
    pub companies: Vec<CompanyUpdates>,
}

impl ProbeUpdates {
    /// The version being rolled out, if any.
    pub fn approved(&self) -> Option<&str> {
        self.rollout.as_ref().map(|r| r.version.as_str())
    }
}

/// Where a rotation of the hub's certificate or key is (D-61).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationState {
    /// The next pins ride in every answer; probes take them.
    Announcing,
    /// The hub uses the new certificate or key.
    Switched,
    Cancelled,
}

impl RotationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Announcing => "announcing",
            Self::Switched => "switched",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Announcing, Self::Switched, Self::Cancelled]
            .into_iter()
            .find(|s| s.as_str() == text)
    }
}

/// Start rotating the hub's certificate and/or key
/// (`POST /api/admin/hub-keys/rotation`, D-61).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRotationRequest {
    /// The TLS certificate (a new key pair, self-signed).
    pub tls: bool,
    /// The hub's key (X25519, and the signing key derived from it).
    pub key: bool,
    /// After this many days a rotation still waiting for probes is flagged.
    pub window_days: u32,
}

impl StartRotationRequest {
    pub const DEFAULT_WINDOW_DAYS: u32 = 14;
    pub const MAX_WINDOW_DAYS: u32 = 90;

    /// `Err` with what's wrong, for the form and the API alike.
    pub fn validate(&self) -> Result<(), String> {
        if !self.tls && !self.key {
            return Err("choose the certificate, the key, or both".into());
        }
        if !(1..=Self::MAX_WINDOW_DAYS).contains(&self.window_days) {
            return Err(format!("the window is 1 to {} days", Self::MAX_WINDOW_DAYS));
        }
        Ok(())
    }
}

/// A probe a rotation still waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingProbe {
    pub id: String,
    pub name: String,
    pub company_name: String,
    pub probe_version: Option<String>,
    pub last_seen_at: Option<String>,
    /// Why it isn't ready, in words.
    pub reason: String,
}

/// A rotation of the hub's certificate and/or key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotationInfo {
    pub id: i64,
    pub rotates_tls: bool,
    pub rotates_key: bool,
    pub window_days: u32,
    pub state: RotationState,
    pub started_by: Option<String>,
    /// `YYYY-MM-DD HH:MM UTC`, like the times below.
    pub started_at: String,
    pub deadline_at: String,
    /// `None` with `ended_at`: the hub switched by itself (every probe ready).
    pub ended_by: Option<String>,
    pub ended_at: Option<String>,
    /// Announcing past its window.
    pub overdue: bool,
    /// Active probes that hold the next pins (announcing only).
    pub ready: u32,
    /// Active probes that don't yet (announcing only).
    pub waiting: Vec<WaitingProbe>,
}

/// The admin page's "Hub keys" (`GET /api/admin/hub-keys`, D-61).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubKeysOverview {
    /// Hex of the hub's X25519 public key.
    pub hub_key: String,
    /// The hub serves HTTPS itself (it has a certificate to rotate).
    pub tls_enabled: bool,
    /// Hex of the SHA-256 of its certificate's public key: what probes pin.
    pub tls_spki_sha256: Option<String>,
    /// The certificate's expiry date.
    pub tls_expires: Option<String>,
    /// The rotation in progress.
    pub rotation: Option<RotationInfo>,
    /// Earlier rotations, newest first.
    pub history: Vec<RotationInfo>,
}

/// Who issues the hub's certificate over ACME (M2-14, D-62).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcmeAuthority {
    #[default]
    LetsEncrypt,
    /// Let's Encrypt's test service: certificates browsers don't trust, and
    /// generous limits, for trying the settings out.
    LetsEncryptStaging,
    /// Another ACME service, by its directory URL (a company's own CA, …).
    Custom,
}

impl AcmeAuthority {
    pub const LETS_ENCRYPT_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";
    pub const LETS_ENCRYPT_STAGING_URL: &str =
        "https://acme-staging-v02.api.letsencrypt.org/directory";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LetsEncrypt => "lets_encrypt",
            Self::LetsEncryptStaging => "lets_encrypt_staging",
            Self::Custom => "custom",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::LetsEncrypt, Self::LetsEncryptStaging, Self::Custom]
            .into_iter()
            .find(|a| a.as_str() == text)
    }
}

/// How the certificate authority checks that the hub answers for its names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcmeChallenge {
    /// TLS-ALPN-01, on the HTTPS listener: the authority connects to port 443.
    #[default]
    TlsAlpn,
    /// HTTP-01, on the plain-HTTP listener: the authority connects to port 80.
    Http,
}

impl AcmeChallenge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TlsAlpn => "tls_alpn",
            Self::Http => "http",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::TlsAlpn, Self::Http]
            .into_iter()
            .find(|c| c.as_str() == text)
    }

    /// The port the authority connects to, whatever the hub listens on.
    pub fn public_port(self) -> u16 {
        match self {
            Self::TlsAlpn => 443,
            Self::Http => 80,
        }
    }
}

/// The hub's certificate from an ACME authority such as Let's Encrypt
/// (M2-14, D-62), set on the admin page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcmeSettings {
    /// Off: the hub serves a self-signed certificate.
    pub enabled: bool,
    /// The names the certificate is for; the first is its subject.
    pub domains: Vec<String>,
    /// A contact for the authority; may be empty.
    pub email: String,
    pub authority: AcmeAuthority,
    /// [`AcmeAuthority::Custom`]'s directory URL.
    pub directory_url: String,
    /// [`AcmeAuthority::Custom`]: the root certificate (PEM) its own HTTPS
    /// uses, when that isn't a public one. May be empty.
    pub ca_pem: String,
    pub challenge: AcmeChallenge,
    /// The authority's terms of service were accepted.
    pub terms_agreed: bool,
}

impl AcmeSettings {
    pub const MAX_DOMAINS: usize = 10;

    /// The authority's directory URL.
    pub fn directory(&self) -> &str {
        match self.authority {
            AcmeAuthority::LetsEncrypt => AcmeAuthority::LETS_ENCRYPT_URL,
            AcmeAuthority::LetsEncryptStaging => AcmeAuthority::LETS_ENCRYPT_STAGING_URL,
            AcmeAuthority::Custom => self.directory_url.trim(),
        }
    }

    /// The same settings with the names trimmed, in lower case, once each,
    /// and what doesn't apply to the authority emptied.
    pub fn normalized(mut self) -> Self {
        let mut seen = BTreeSet::new();
        self.domains = self
            .domains
            .iter()
            .map(|d| d.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|d| !d.is_empty() && seen.insert(d.clone()))
            .collect();
        self.email = self.email.trim().to_string();
        if self.authority == AcmeAuthority::Custom {
            self.directory_url = self.directory_url.trim().to_string();
            self.ca_pem = self.ca_pem.trim().to_string();
        } else {
            self.directory_url.clear();
            self.ca_pem.clear();
        }
        self
    }

    /// `Err` with what's wrong, for the form and the API alike. Settings
    /// that are off are kept as typed, so only the switched-on ones are checked.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.domains.is_empty() {
            return Err("give the name (or names) the hub is reached by".into());
        }
        if self.domains.len() > Self::MAX_DOMAINS {
            return Err(format!("at most {} names", Self::MAX_DOMAINS));
        }
        for domain in &self.domains {
            validate_domain(domain)?;
        }
        if !self.email.is_empty()
            && (self.email.contains(char::is_whitespace)
                || self.email.split('@').filter(|p| !p.is_empty()).count() != 2)
        {
            return Err(format!("{} isn't an e-mail address", self.email));
        }
        if self.authority == AcmeAuthority::Custom {
            if !self.directory_url.starts_with("https://") {
                return Err("the ACME directory URL starts with https://".into());
            }
            if !self.ca_pem.is_empty() && !self.ca_pem.contains("-----BEGIN CERTIFICATE-----") {
                return Err("the authority's root certificate is a PEM certificate".into());
            }
        }
        if !self.terms_agreed {
            return Err("accept the authority's terms of service".into());
        }
        Ok(())
    }
}

/// A name a public certificate can be issued for: letters, digits and
/// hyphens, at least two labels, not an IP address. No wildcards (they
/// need a DNS challenge).
fn validate_domain(domain: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("{domain}: {why}"));
    if domain.starts_with("*.") {
        return bad("wildcard names aren't supported (they need DNS validation)");
    }
    if domain.len() > 253 {
        return bad("too long");
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return bad("give the full name, such as hub.example.com");
    }
    for label in &labels {
        let valid = (1..=63).contains(&label.len())
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !valid {
            return bad("not a valid DNS name");
        }
    }
    if labels
        .last()
        .is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit()))
    {
        return bad("an IP address can't have a public certificate: use a DNS name");
    }
    Ok(())
}

/// A certificate as the admin page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateInfo {
    /// Who signed it (its issuer's common name, or the whole name).
    pub issuer: String,
    /// The DNS names (and addresses) it is for.
    pub names: Vec<String>,
    pub self_signed: bool,
    pub not_before: String,
    pub not_after: String,
}

/// The admin page's "HTTPS certificate" (`GET /api/admin/certificate`, D-62).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateOverview {
    /// The hub serves HTTPS itself (not off, not behind a proxy only).
    pub tls_enabled: bool,
    pub settings: AcmeSettings,
    /// The certificate it serves now.
    pub certificate: Option<CertificateInfo>,
    /// A rotation's next certificate, while one announces (D-61).
    pub next_certificate: Option<CertificateInfo>,
    /// When the hub next asks the authority (`None`: nothing to do).
    pub next_attempt_at: Option<String>,
    pub last_attempt_at: Option<String>,
    /// Why the last attempt failed (`None`: it worked, or none yet).
    pub last_error: Option<String>,
    pub last_issued_at: Option<String>,
    /// Where the hub listens for HTTPS, and for plain HTTP (split mode), so
    /// the page can say which port the authority must reach.
    pub https_port: u16,
    pub http_port: Option<u16>,
}

/// The answer to saving the certificate settings (`POST /api/admin/certificate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateSaved {
    pub success: bool,
    /// The settings asked for a certificate and the authority didn't issue
    /// it: why. They are saved anyway; the hub tries again later.
    pub error: Option<String>,
}

/// When the hub backs itself up (M2-12, D-60): set on the admin page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSchedule {
    /// Off: only "Back up now", `hub backup`, and the backup before an upgrade.
    pub enabled: bool,
    /// The hour of the day (UTC, 0–23) the daily backup runs.
    pub hour_utc: u8,
    /// How many scheduled backups to keep; older ones are deleted.
    pub keep: u32,
}

impl BackupSchedule {
    /// The most scheduled backups a hub keeps (about a year of daily ones).
    pub const MAX_KEEP: u32 = 365;

    /// `Err` with what's wrong, for the form and the API alike.
    pub fn validate(&self) -> Result<(), String> {
        if self.hour_utc > 23 {
            return Err("the hour must be 0 to 23".into());
        }
        if !(1..=Self::MAX_KEEP).contains(&self.keep) {
            return Err(format!("keep 1 to {} backups", Self::MAX_KEEP));
        }
        Ok(())
    }
}

impl Default for BackupSchedule {
    /// Daily at 03:00 UTC, the last 7 kept: a new hub is backed up without
    /// anyone asking.
    fn default() -> Self {
        Self {
            enabled: true,
            hour_utc: 3,
            keep: 7,
        }
    }
}

/// Why a backup was made; it decides how long the hub keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    /// "Back up now" or `hub backup`: kept until someone deletes it.
    Manual,
    /// The daily backup: the newest [`BackupSchedule::keep`] are kept.
    Scheduled,
    /// Made before a new hub upgraded the database's schema.
    PreUpgrade,
}

impl BackupKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::PreUpgrade => "before upgrade",
        }
    }
}

/// A backup file in the backups folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    /// The file name; also its ID in the API.
    pub name: String,
    pub kind: BackupKind,
    /// `YYYY-MM-DD HH:MM UTC`.
    pub created_at: String,
    pub size_bytes: u64,
}

/// The admin page's "Backups" (`GET /api/admin/backups`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupOverview {
    pub schedule: BackupSchedule,
    /// Where backups are written, on the hub's server.
    pub folder: String,
    /// When the next scheduled backup runs (`None`: schedule off).
    pub next_at: Option<String>,
    /// When the last scheduled backup ran, and why it failed if it did.
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    /// Newest first.
    pub backups: Vec<BackupFile>,
}

/// Ask to download a backup (`POST /api/admin/backups/{name}/download`): the
/// user's own password again, since a backup holds the hub's private key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDownloadRequest {
    pub password: String,
}

/// Where to download it from: a one-time link, valid for a minute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDownloadLink {
    pub url: String,
}

/// Roll out a release, canaries first, or stop (`POST /api/admin/probe-updates/approve`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveRequest {
    /// `None`: stop offering updates.
    pub version: Option<String>,
    /// Minutes to wait after every canary runs it; default
    /// [`DEFAULT_SOAK_MINUTES`].
    #[serde(default)]
    pub soak_minutes: Option<u32>,
}

/// The admin page's "Hub version" (`GET /api/admin/hub-upgrade`, M4-04, D-75).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubUpgradeInfo {
    /// The version running.
    pub running: String,
    /// The daily check for a newer release (off by default).
    pub check_enabled: bool,
    pub checked_at: Option<String>,
    /// The release the last check found.
    pub latest: Option<String>,
    /// `latest`, if it is newer than the version running.
    pub available: Option<String>,
    /// Why the last check failed.
    pub error: Option<String>,
    /// Where releases come from (`[upgrades] release_url`).
    pub release_url: String,
    /// What to run on the server to upgrade.
    pub command: String,
}

/// Turn the check for a newer hub release on or off (`POST /api/admin/hub-upgrade`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeCheckRequest {
    pub enabled: bool,
}

/// Make a probe a canary, or not (`POST /api/admin/probe-updates/probes/{id}/canary`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanaryRequest {
    pub canary: bool,
}

impl ProbeDownload {
    /// Where to download it from a hub reached at `hub_url`.
    pub fn url(&self, hub_url: &str) -> String {
        format!(
            "{}/download/probe/{}",
            hub_url.trim_end_matches('/'),
            self.target
        )
    }

    /// The one command that downloads it, checks its SHA-256 and installs it
    /// with `code`: a shell line for Linux (as a user who may `sudo`), a
    /// PowerShell line for Windows (an elevated window).
    ///
    /// The certificate isn't checked for the download (`-k`, the PowerShell
    /// equivalent): the SHA-256, shown on the hub's own HTTPS page, is what
    /// proves the file is the hub's. The probe then pins the hub itself.
    ///
    /// Both fail closed: the shell line is one `&&` chain; the PowerShell
    /// line stops at any error (`$ErrorActionPreference='Stop'`: otherwise a
    /// failing statement is skipped and the next one runs) and runs the probe
    /// only inside `if (hash matches)`. The hash uses .NET directly, so it
    /// needs no module (Windows PowerShell 5.1 and PowerShell 7 alike).
    pub fn install_command(&self, hub_url: &str, code: &str) -> String {
        let url = self.url(hub_url);
        if self.os == "windows" {
            format!(
                "$ErrorActionPreference='Stop'; $f=\"$env:TEMP\\blentinel-probe.exe\"; \
                 $o=@{{UseBasicParsing=$true}}; \
                 if ($PSVersionTable.PSVersion.Major -ge 6) {{ $o.SkipCertificateCheck=$true }} \
                 else {{ [Net.ServicePointManager]::SecurityProtocol='Tls12'; \
                 [Net.ServicePointManager]::ServerCertificateValidationCallback={{$true}} }}; \
                 Invoke-WebRequest '{url}' -OutFile $f @o; \
                 $h=[BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes($f))).Replace('-',''); \
                 if ($h -eq '{sha}') {{ & $f install --code {code} }} else {{ throw 'checksum mismatch' }}",
                sha = self.sha256.to_uppercase(),
            )
        } else {
            format!(
                "curl -fsSLk -o probe '{url}' && echo '{sha}  probe' | sha256sum -c - \
                 && chmod +x probe && sudo ./probe install --code {code}",
                sha = self.sha256,
            )
        }
    }

    /// "Linux x86_64", "Windows x86_64".
    pub fn label(&self) -> String {
        let os = if self.os == "windows" {
            "Windows"
        } else {
            "Linux"
        };
        format!("{os} {}", self.arch)
    }
}

/// `POST /api/probes/{id}`: rename, move to another site of its company,
/// set how often it reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateProbeRequest {
    pub name: String,
    pub site_id: String,
    /// Seconds between two reports (`common::checks::MIN_INTERVAL_SECS`..=`MAX_INTERVAL_SECS`).
    pub interval_secs: u32,
}

// ===========================================================================
// Checks (M2-04, D-49)
// ===========================================================================

/// A check as the API shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckInfo {
    pub id: String,
    pub probe_id: String,
    pub name: String,
    /// What it checks; `kind` is inside (`{"kind": "tcp", "host": …}`).
    pub spec: CheckSpec,
    /// For people: `10.0.0.1`, `db:5432`, a URL.
    pub target: String,
    pub enabled: bool,
    /// When it alerts (`null`: the default, 2 results in a row).
    #[serde(flatten)]
    pub alert_after: AlertAfter,
    /// Its own thresholds (D-64); empty: the site's, company's or hub's.
    #[serde(default)]
    pub limits: MetricLimits,
    /// The thresholds that apply to it, and from where (`None`: its kind has none).
    #[serde(default)]
    pub limits_in_force: Option<ResolvedLimits>,
    /// The check of the same probe it is behind (D-68): while that one is
    /// down, this one is Unknown and doesn't alert.
    #[serde(default)]
    pub depends_on: Option<String>,
    /// The names of the secrets it has (D-86): `community`, … Their values
    /// are never shown.
    #[serde(default)]
    pub secrets_set: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub updated_by: Option<String>,
}

/// How many results in a row change a check's alert state (M3-02, D-63):
/// one lost ping or one slow page alerts nobody. Hub-only: probes never see
/// it, so changing it doesn't change the config they run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertAfter {
    /// Failed results in a row (Down, or over its usage threshold) before
    /// it alerts; `None`: [`AlertAfter::DEFAULT`].
    #[serde(default)]
    pub fail_after: Option<u32>,
    /// Good results in a row before it counts as recovered; `None`: the default.
    #[serde(default)]
    pub recover_after: Option<u32>,
}

impl AlertAfter {
    /// Two in a row (proposal 0005 §2.2): a blip is ignored, a real outage
    /// alerts one check interval later.
    pub const DEFAULT: u32 = 2;
    pub const MAX: u32 = 10;

    /// Failed results in a row that alert.
    pub fn failures(self) -> u32 {
        self.fail_after.unwrap_or(Self::DEFAULT)
    }

    /// Good results in a row that recover.
    pub fn recoveries(self) -> u32 {
        self.recover_after.unwrap_or(Self::DEFAULT)
    }

    /// `Err` with what's wrong, for the form and the API alike.
    pub fn validate(self) -> Result<(), String> {
        for (value, what) in [(self.fail_after, "failed"), (self.recover_after, "good")] {
            if value.is_some_and(|n| !(1..=Self::MAX).contains(&n)) {
                return Err(format!(
                    "alert after 1 to {} {what} results in a row",
                    Self::MAX
                ));
            }
        }
        Ok(())
    }
}

/// `POST /api/probes/{id}/checks` and `POST /api/checks/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckRequest {
    pub name: String,
    pub spec: CheckSpec,
    /// Default `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Default: [`AlertAfter::DEFAULT`] both ways.
    #[serde(flatten, default)]
    pub alert_after: AlertAfter,
    /// Its own thresholds (D-64); default: none (the levels above decide).
    #[serde(default)]
    pub limits: MetricLimits,
    /// The ID of the check of the same probe it is behind (D-68); default: none.
    #[serde(default)]
    pub depends_on: Option<String>,
    /// Its secrets, by name (D-86), write-only: one given replaces the one
    /// kept; one left out (or empty) keeps it. Default: none given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<common::checks::Secrets>,
}

fn default_true() -> bool {
    true
}

impl CheckRequest {
    /// A check with a name and what it checks, the rest as a new check has it.
    pub fn new(name: impl Into<String>, spec: CheckSpec) -> Self {
        Self {
            name: name.into(),
            spec,
            enabled: true,
            alert_after: AlertAfter::default(),
            limits: MetricLimits::default(),
            depends_on: None,
            secrets: None,
        }
    }
}

/// `POST /api/probes/{id}/checks/copy`: copy another probe's checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyChecksRequest {
    pub from_probe_id: String,
}

// ===========================================================================
// The probe's machine, suggested checks and templates (M5-12, D-85)
// ===========================================================================

/// A check the hub proposes for a probe, from what its machine is made of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub name: String,
    pub spec: CheckSpec,
    /// Why it is proposed, for people.
    pub why: String,
}

/// `GET /api/probes/{id}/machine`: what the probe last said its machine is
/// made of, and the checks that suggests. All empty before its first
/// inventory (an older probe sends none).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MachineInfo {
    pub inventory: Option<common::inventory::Inventory>,
    pub reported_at: Option<String>,
    pub suggestions: Vec<Suggestion>,
}

/// `POST /api/probes/{id}/checks/batch`: several checks at once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddChecksRequest {
    pub checks: Vec<CheckRequest>,
}

/// What adding several checks did: the new checks, and how many the probe
/// already had (the same settings) and were left alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddedChecks {
    pub added: Vec<CheckInfo>,
    pub skipped: usize,
}

/// A check of a template: a name and a spec as JSON. In a built-in
/// template, `{key}` in any text stands for one of its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateCheck {
    pub name: String,
    pub spec: serde_json::Value,
}

/// A value a template asks for when it is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateParameter {
    /// What stands for it in the checks: `host` for `{host}`.
    pub key: String,
    pub label: String,
    pub example: String,
    /// A secret of the checks (a community), by its name: typed like a
    /// password, kept with the checks that need it, and never shown.
    #[serde(default)]
    pub secret: bool,
}

/// `GET /api/check-templates`: a named set of checks to add to a probe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateInfo {
    /// `builtin:…` for the ones that come with the hub.
    pub id: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub parameters: Vec<TemplateParameter>,
    pub checks: Vec<TemplateCheck>,
    pub created_by: Option<String>,
}

/// `POST /api/check-templates`: save a probe's checks as a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewTemplateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub from_probe_id: String,
}

/// `POST /api/probes/{id}/checks/template`: add a template's checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyTemplateRequest {
    pub template_id: String,
    /// A value for each of the template's parameters, by key.
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, String>,
}

/// Does the probe run what the hub wants it to?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSync {
    /// It runs the current checks.
    InSync,
    /// It gets them with the answer to its next report.
    Pending,
    /// It refused them; see `ProbeChecks::config_error`.
    Rejected,
}

/// `GET /api/probes/{id}/checks`: a probe's checks and whether it runs them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeChecks {
    pub probe_id: String,
    pub interval_secs: u32,
    /// The revision the probe should run.
    pub config_revision: String,
    /// The revision the probe last said it runs (`None` before its first report).
    pub applied_revision: Option<String>,
    pub sync: ConfigSync,
    /// Why the probe refused the current config.
    pub config_error: Option<String>,
    /// Every check of the probe, disabled ones too, by name.
    pub checks: Vec<CheckInfo>,
}

// ===========================================================================
// Commands (M2-06, D-56)
// ===========================================================================

/// Where a command is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    /// Waiting for the probe's next report.
    Queued,
    /// Sent to the probe; its result comes with a later report.
    Delivered,
    Succeeded,
    Failed,
    /// Cancelled before it was sent.
    Cancelled,
    /// Not done before it expired (never stored: worked out on reading).
    Expired,
}

impl CommandState {
    pub fn as_str(self) -> &'static str {
        match self {
            CommandState::Queued => "queued",
            CommandState::Delivered => "delivered",
            CommandState::Succeeded => "succeeded",
            CommandState::Failed => "failed",
            CommandState::Cancelled => "cancelled",
            CommandState::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => CommandState::Queued,
            "delivered" => CommandState::Delivered,
            "succeeded" => CommandState::Succeeded,
            "failed" => CommandState::Failed,
            "cancelled" => CommandState::Cancelled,
            "expired" => CommandState::Expired,
            _ => return None,
        })
    }

    /// Waiting to be sent or for its result.
    pub fn is_pending(self) -> bool {
        matches!(self, CommandState::Queued | CommandState::Delivered)
    }
}

/// A command as the API shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandInfo {
    pub id: String,
    pub probe_id: String,
    /// `run_checks`.
    pub kind: String,
    pub created_by: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub state: CommandState,
    pub delivered_at: Option<String>,
    pub completed_at: Option<String>,
    /// The probe's message, or the hub's reason.
    pub result: Option<String>,
}

/// `POST /api/probes/{id}/commands`: `{"kind": "run_checks"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    #[serde(flatten)]
    pub spec: common::commands::CommandSpec,
}

// ===========================================================================
// Audit log (M1-06, D-46)
// ===========================================================================

/// One entry of `GET /api/audit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    /// RFC 3339.
    pub at: String,
    /// Who did it; `None` for the hub itself or the server console (`hub user`).
    pub username: Option<String>,
    pub actor_ip: Option<String>,
    /// What happened, e.g. `user.create`, `silence.delete`, `login.failed`.
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub company_id: Option<String>,
    /// Extra facts, as JSON text (e.g. the fields that changed).
    pub details: Option<String>,
}

/// Filters of `GET /api/audit` (all optional), newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditQuery {
    /// Exact username (case-insensitive).
    pub user: Option<String>,
    /// Action or action prefix: `user` matches `user.create`, `user.update`, …
    pub action: Option<String>,
    /// Target ID or company ID.
    pub target: Option<String>,
    /// Entries at or after this RFC 3339 time.
    pub from: Option<String>,
    /// Entries before this RFC 3339 time.
    pub to: Option<String>,
    /// Page: entries with a smaller ID than this.
    pub before_id: Option<i64>,
    /// At most this many (default 200, at most 1000).
    pub limit: Option<u32>,
}

/// A generated temporary password, shown once (create, reset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporaryPassword {
    pub user: UserInfo,
    pub temporary_password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_info() -> ProbeInfo {
        serde_json::from_value(serde_json::json!({
            "id": "p1", "name": "p1", "company_id": "acme", "company_name": "Acme",
            "site_id": "hq", "site_name": "HQ", "state": "active", "hostname": null,
            "os": null, "arch": null, "probe_version": "0.3.0",
            "enrolled_at": "2026-10-01T00:00:00Z", "last_seen_at": "2026-10-08T10:00:00Z",
            "interval_secs": 60, "config_sync": "in_sync"
        }))
        .unwrap()
    }

    #[test]
    fn a_probe_that_reports_and_runs_its_checks_needs_no_attention() {
        let mut probe = probe_info();
        assert!(probe.concerns().is_empty());
        probe.health = Some(common::models::ProbeHealth {
            started_at: "2026-10-08T09:00:00Z".parse().unwrap(),
            queued_reports: 0,
            clock_offset_secs: Some(common::protocol::CLOCK_WARNING_SECS),
            last_error: None,
        });
        assert!(probe.concerns().is_empty(), "a minute off is fine");
    }

    #[test]
    fn each_concern_is_named() {
        let mut probe = probe_info();
        probe.last_seen_at = None;
        assert_eq!(probe.concerns(), ["Never reported"]);

        let mut probe = probe_info();
        probe.silent = true;
        probe.config_sync = Some(ConfigSync::Rejected);
        probe.update_failed = Some(common::models::UpdateFailure {
            version: "0.4.0".into(),
            error: "self-test failed".into(),
        });
        probe.health = Some(common::models::ProbeHealth {
            started_at: "2026-10-08T09:00:00Z".parse().unwrap(),
            queued_reports: 2,
            clock_offset_secs: Some(-61),
            last_error: Some(common::models::ProbeProblem {
                at: "2026-10-08T09:30:00Z".parse().unwrap(),
                message: "Can't reach the hub".into(),
            }),
        });
        assert_eq!(
            probe.concerns(),
            [
                "Not reporting",
                "Refused its checks",
                "Didn't update to 0.4.0",
                "2 reports held back",
                "Clock off by -61 s",
                "Had a problem"
            ]
        );
        probe.state = "revoked".into();
        assert!(probe.concerns().is_empty(), "a revoked probe is done with");
    }

    #[test]
    fn a_pending_config_is_no_concern() {
        let mut probe = probe_info();
        probe.config_sync = Some(ConfigSync::Pending);
        assert!(
            probe.concerns().is_empty(),
            "it gets it with its next answer"
        );
    }

    #[test]
    fn the_digest_is_checked_like_the_hub_checks_it() {
        let digest = |hour: u8, channels: Vec<i64>| DigestSettings {
            hour,
            channels,
            ..DigestSettings::default()
        };
        assert!(
            DigestSettings::default().validate(&[]).is_ok(),
            "no channels: off"
        );
        assert!(digest(23, vec![1]).validate(&[1]).is_ok());
        assert!(digest(24, vec![]).validate(&[]).is_err());
        assert!(
            digest(7, vec![2])
                .validate(&[1])
                .unwrap_err()
                .contains("e-mail")
        );
        let no_zone = DigestSettings {
            timezone: " ".into(),
            ..DigestSettings::default()
        };
        assert!(no_zone.validate(&[]).is_err());
    }

    #[test]
    fn an_ntfy_topic_url_is_a_server_and_a_topic() {
        assert_eq!(
            ntfy_parts("https://ntfy.sh/blentinel-0123"),
            Some(("https://ntfy.sh", "blentinel-0123"))
        );
        assert_eq!(
            ntfy_parts(" http://10.0.0.5:8080/push/alerts_1/ "),
            Some(("http://10.0.0.5:8080/push", "alerts_1"))
        );
        for bad in [
            "ntfy.sh/alerts",
            "https://ntfy.sh",
            "https://ntfy.sh/",
            "https:///alerts",
            "https://ntfy.sh/bad topic",
            "https://ntfy.sh/caf\u{e9}",
        ] {
            assert_eq!(ntfy_parts(bad), None, "{bad}");
        }
        let ntfy = |target: &str, secret: Option<&str>| ChannelRequest {
            name: "Phones".into(),
            kind: ChannelKind::Ntfy,
            enabled: true,
            target: target.into(),
            secret: secret.map(str::to_string),
        };
        assert!(
            ntfy("https://ntfy.sh/blentinel-0123", None)
                .validate(false)
                .is_ok(),
            "the token is optional"
        );
        assert!(
            ntfy("https://ntfy.example.com/alerts", Some("tk_1"))
                .validate(false)
                .is_ok()
        );
        assert!(
            ntfy("ntfy.sh/alerts", None)
                .validate(false)
                .unwrap_err()
                .contains("topic URL")
        );
        assert!(ChannelKind::Webhook.secret_required() && !ChannelKind::Ntfy.secret_required());
    }

    #[test]
    fn follow_ups_take_the_shortest_reminder_and_earliest_escalation() {
        let rule = |channels: Vec<i64>, remind: Option<u32>, escalate: Option<(u32, Vec<i64>)>| {
            RouteRule {
                channels,
                remind_every_minutes: remind,
                escalate_after_minutes: escalate.as_ref().map(|e| e.0),
                escalate_to: escalate.map(|e| e.1).unwrap_or_default(),
                ..RouteRule::default()
            }
        };
        let rules = [
            rule(vec![1], Some(60), Some((30, vec![9]))),
            rule(vec![2], Some(15), None),
            rule(vec![3], None, Some((10, vec![8, 9]))),
        ];
        let follow = follow_up(&rules, AlertKind::Down, "acme", "hq", Some("ping"));
        assert_eq!(follow.remind, Some((15, vec![1, 2])));
        assert_eq!(follow.escalate, Some((10, vec![9, 8])));

        // A rule that stops hides the ones after it.
        let mut stopping = rules.clone();
        stopping[0].stop = true;
        let follow = follow_up(&stopping, AlertKind::Down, "acme", "hq", Some("ping"));
        assert_eq!(follow.remind, Some((60, vec![1])));
        assert_eq!(follow.escalate, Some((30, vec![9])));
        assert_eq!(
            follow_up(&[], AlertKind::Down, "acme", "hq", None),
            FollowUp::default()
        );
    }

    #[test]
    fn durations_read_like_people_say_them() {
        let cases = [
            (-5, "0 s"),
            (45, "45 s"),
            (180, "3 min"),
            (3600, "1 h"),
            (3900, "1 h 5 min"),
            (86_400, "1 d"),
            (187_200, "2 d 4 h"),
        ];
        for (seconds, text) in cases {
            assert_eq!(duration_text(seconds), text, "{seconds} s");
        }
    }

    #[test]
    fn notes_are_checked_and_internal_events_known() {
        assert!(IncidentNote::default().validate(false).is_ok());
        assert!(IncidentNote::default().validate(true).is_err());
        let long = IncidentNote {
            text: "x".repeat(MAX_NOTE_CHARS + 1),
        };
        assert!(long.validate(false).is_err());
        for kind in IncidentEventKind::ALL {
            assert_eq!(IncidentEventKind::parse(kind.as_str()), Some(kind));
        }
        assert!(IncidentEventKind::Note.is_internal());
        assert!(!IncidentEventKind::Closed.is_internal());
    }

    #[test]
    fn authorization_matrix() {
        // (role, [ViewData, Silence, ViewAdmin, ManageData, ManageUsers, ManageCompanies])
        let expected = [
            (Role::Owner, [true, true, true, true, true, true]),
            (Role::Admin, [true, true, true, true, true, true]),
            (Role::Technician, [true, true, false, false, false, true]),
            (Role::Viewer, [true, false, false, false, false, false]),
            (Role::Client, [true, false, false, false, false, false]),
        ];
        for (role, allowed) in expected {
            for (perm, want) in Permission::ALL.into_iter().zip(allowed) {
                assert_eq!(authorize(role, perm), want, "{role:?} {perm:?}");
            }
        }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scopes_allow_all_only_or_all_except() {
        assert!(CompanyScope::All.allows("anything"));
        let only = CompanyScope::Only(set(&["c1", "c3"]));
        assert!(only.allows("c1") && !only.allows("c2"));
        let except = CompanyScope::AllExcept(set(&["c8"]));
        assert!(
            except.allows("c9") && !except.allows("c8"),
            "later companies included"
        );
    }

    #[test]
    fn scopes_have_a_stable_json_and_stored_form() {
        let except = CompanyScope::AllExcept(set(&["c8"]));
        let json = serde_json::to_value(&except).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "mode": "except", "companies": ["c8"] })
        );
        assert_eq!(
            serde_json::from_value::<CompanyScope>(json).unwrap(),
            except
        );
        assert_eq!(
            serde_json::from_value::<CompanyScope>(serde_json::json!({ "mode": "all" })).unwrap(),
            CompanyScope::All
        );
        for scope in [CompanyScope::All, CompanyScope::Only(set(&["a"])), except] {
            assert_eq!(
                CompanyScope::from_parts(scope.mode(), scope.companies()),
                Some(scope)
            );
        }
        assert_eq!(CompanyScope::from_parts("some", Vec::new()), None);
    }

    #[test]
    fn owners_and_admins_always_see_every_company() {
        let user = |role| UserInfo {
            id: "u".into(),
            username: "u".into(),
            display_name: "u".into(),
            email: None,
            role,
            scope: CompanyScope::Only(set(&["c1"])),
            is_active: true,
            must_change_password: false,
            created_at: String::new(),
            last_login_at: None,
        };
        assert_eq!(user(Role::Admin).effective_scope(), CompanyScope::All);
        assert_eq!(user(Role::Owner).effective_scope(), CompanyScope::All);
        assert_eq!(
            user(Role::Viewer).effective_scope(),
            CompanyScope::Only(set(&["c1"]))
        );
    }

    #[test]
    fn slugs_are_made_from_names_and_validated() {
        assert_eq!(slugify("ACME Ltd."), "acme-ltd");
        assert_eq!(slugify("  Warehouse #2 (north) "), "warehouse-2-north");
        assert_eq!(slugify("Ñandú"), "and");
        for ok in ["acme", "acme-ltd", "hq_1", "site.2"] {
            assert!(is_valid_slug(ok), "{ok}");
        }
        for bad in ["", "Acme", "-acme", "a b", "a/b", &"a".repeat(65)] {
            assert!(!is_valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn roles_round_trip_through_their_names() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
            assert_eq!(
                serde_json::to_string(&role).unwrap(),
                format!("\"{}\"", role.as_str())
            );
        }
        assert_eq!(Role::parse("root"), None);
    }

    #[test]
    fn expired_probe_is_degraded_even_with_devices_down() {
        assert_eq!(Severity::for_probe(false, 3, 1), Severity::Degraded);
        assert_eq!(Severity::for_probe(false, 0, 0), Severity::Degraded);
    }

    #[test]
    fn an_active_probe_is_as_bad_as_its_worst_device() {
        assert_eq!(Severity::for_probe(true, 1, 1), Severity::Critical);
        assert_eq!(Severity::for_probe(true, 0, 2), Severity::Warning);
        assert_eq!(Severity::for_probe(true, 0, 0), Severity::Healthy);
    }

    #[test]
    fn company_with_a_live_outage_is_critical_even_if_another_probe_expired() {
        // Previously the card showed amber here and hid the outage.
        assert_eq!(Severity::for_company(1, 0, 1), Severity::Critical);
    }

    #[test]
    fn company_with_only_stale_problems_is_degraded() {
        assert_eq!(Severity::for_company(0, 0, 1), Severity::Degraded);
        assert_eq!(
            Severity::for_company(0, 3, 1),
            Severity::Degraded,
            "not knowing beats a warning"
        );
        assert_eq!(Severity::for_company(0, 1, 0), Severity::Warning);
        assert_eq!(Severity::for_company(0, 0, 0), Severity::Healthy);
    }

    #[test]
    fn severity_sorts_worst_first_and_serialises_lowercase() {
        let mut all = vec![
            Severity::Healthy,
            Severity::Warning,
            Severity::Critical,
            Severity::Degraded,
        ];
        all.sort();
        assert_eq!(
            all,
            [
                Severity::Critical,
                Severity::Degraded,
                Severity::Warning,
                Severity::Healthy
            ]
        );
        assert_eq!(
            serde_json::to_string(&Severity::Degraded).unwrap(),
            r#""degraded""#
        );
        assert_eq!(Severity::Critical.as_str(), "critical");
    }

    #[test]
    fn short_probe_id_takes_eight_chars_and_tolerates_short_ids() {
        assert_eq!(short_probe_id("0123456789abcdef"), "01234567");
        assert_eq!(short_probe_id("abc"), "abc");
    }

    #[test]
    fn a_device_headlines_its_first_metric() {
        let mut device = ProbeDevice {
            check_id: "c1".into(),
            name: "Disk".into(),
            kind: "disk".into(),
            target: "/".into(),
            status: Status::Up,
            reason: None,
            in_maintenance: false,
            message: None,
            latency_ms: None,
            metrics: vec![
                Metric::new("usage", 42.26, "%"),
                Metric::new("free", 9.0, "GB"),
            ],
        };
        assert_eq!(device.headline().as_deref(), Some("42.3%"));
        device.metrics.clear();
        assert_eq!(device.headline(), None);
    }

    fn download(os: &str, target: &str) -> ProbeDownload {
        ProbeDownload {
            target: target.into(),
            os: os.into(),
            arch: "x86_64".into(),
            size: 7,
            sha256: "ab12".repeat(16),
            version: "0.1.0".into(),
        }
    }

    #[test]
    fn the_linux_command_downloads_checks_and_installs() {
        let linux = download("linux", "x86_64-unknown-linux-musl");
        let command = linux.install_command("https://hub.example.com:3443/", "BLT1-ABC");
        assert_eq!(
            command,
            format!(
                "curl -fsSLk -o probe 'https://hub.example.com:3443/download/probe/x86_64-unknown-linux-musl' \
                 && echo '{}  probe' | sha256sum -c - && chmod +x probe && sudo ./probe install --code BLT1-ABC",
                "ab12".repeat(16)
            )
        );
        assert_eq!(linux.label(), "Linux x86_64");
    }

    #[test]
    fn the_windows_command_runs_the_probe_only_when_the_hash_matches() {
        let windows = download("windows", "x86_64-pc-windows-msvc");
        let command = windows.install_command("http://hub:3000", "BLT1-ABC");
        // Any error stops the line (a failed download or hash can't be skipped)…
        assert!(
            command.starts_with("$ErrorActionPreference='Stop';"),
            "{command}"
        );
        // …and the probe runs only inside the branch where the hash matched.
        assert!(
            command.contains(&format!(
                "if ($h -eq '{}') {{ & $f install --code BLT1-ABC }} else {{ throw 'checksum mismatch' }}",
                "AB12".repeat(16)
            )),
            "{command}"
        );
        assert_eq!(command.matches("& $f").count(), 1, "run in one place only");
        assert!(command.contains("'http://hub:3000/download/probe/x86_64-pc-windows-msvc'"));
        assert!(!command.contains('\n'), "one line");
    }

    #[test]
    fn config_sync_serialises_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConfigSync::InSync).unwrap(),
            r#""in_sync""#
        );
    }

    #[test]
    fn display_name_falls_back_to_the_short_id() {
        let mut probe = CompanyProbe {
            probe_id: "0123456789abcdef".into(),
            probe_name: String::new(),
            hostname: None,
            site: None,
            status: "active".into(),
            severity: Severity::Healthy,
            last_seen_at: None,
            devices_up: 0,
            devices_down: 0,
            devices_warning: 0,
        };
        assert_eq!(probe.display_name(), "01234567…");
        probe.probe_name = "hq".into();
        assert_eq!(probe.display_name(), "hq");
    }

    fn acme(domains: &[&str]) -> AcmeSettings {
        AcmeSettings {
            enabled: true,
            domains: domains.iter().map(|d| d.to_string()).collect(),
            terms_agreed: true,
            ..AcmeSettings::default()
        }
    }

    #[test]
    fn acme_names_are_trimmed_lowercased_and_kept_once() {
        let settings =
            acme(&[" Hub.Example.com. ", "hub.example.com", "", "b.example.org"]).normalized();
        assert_eq!(settings.domains, ["hub.example.com", "b.example.org"]);
        assert_eq!(settings.validate(), Ok(()));
    }

    #[test]
    fn acme_settings_need_public_dns_names_and_the_terms() {
        assert!(acme(&[]).validate().is_err());
        for bad in [
            "localhost",
            "*.example.com",
            "192.168.1.10",
            "hub_1.example.com",
            "-hub.example.com",
            "hub..example.com",
        ] {
            assert!(acme(&[bad]).validate().is_err(), "{bad}");
        }
        let names: Vec<String> = (0..=AcmeSettings::MAX_DOMAINS)
            .map(|i| format!("h{i}.example.com"))
            .collect();
        let many: Vec<&str> = names.iter().map(String::as_str).collect();
        assert!(acme(&many).validate().is_err(), "too many");

        let mut settings = acme(&["hub.example.com"]);
        settings.terms_agreed = false;
        assert!(settings.validate().is_err());
        settings.enabled = false;
        assert_eq!(settings.validate(), Ok(()), "off: kept as typed");
    }

    #[test]
    fn acme_contacts_and_custom_authorities_are_checked() {
        let mut settings = acme(&["hub.example.com"]);
        settings.email = "ops@example.com".into();
        assert_eq!(settings.validate(), Ok(()));
        for bad in ["ops", "ops@", "a b@example.com", "a@b@c"] {
            settings.email = bad.into();
            assert!(settings.validate().is_err(), "{bad}");
        }
        settings.email.clear();

        settings.authority = AcmeAuthority::Custom;
        settings.directory_url = "http://ca.internal/directory".into();
        assert!(settings.validate().is_err(), "plain HTTP");
        settings.directory_url = "https://ca.internal/acme/directory".into();
        assert_eq!(settings.validate(), Ok(()));
        assert_eq!(settings.directory(), "https://ca.internal/acme/directory");
        settings.ca_pem = "not a certificate".into();
        assert!(settings.validate().is_err());

        settings.authority = AcmeAuthority::LetsEncryptStaging;
        let settings = settings.normalized();
        assert!(settings.directory_url.is_empty() && settings.ca_pem.is_empty());
        assert_eq!(
            settings.directory(),
            AcmeAuthority::LETS_ENCRYPT_STAGING_URL
        );
    }

    #[test]
    fn acme_choices_round_trip_as_text() {
        for authority in [
            AcmeAuthority::LetsEncrypt,
            AcmeAuthority::LetsEncryptStaging,
            AcmeAuthority::Custom,
        ] {
            assert_eq!(AcmeAuthority::parse(authority.as_str()), Some(authority));
        }
        for challenge in [AcmeChallenge::TlsAlpn, AcmeChallenge::Http] {
            assert_eq!(AcmeChallenge::parse(challenge.as_str()), Some(challenge));
        }
    }

    #[test]
    fn statuses_round_trip_and_rank_worst_first() {
        for status in [Status::Up, Status::Warning, Status::Down, Status::Unknown] {
            assert_eq!(Status::parse(status.as_str()), Some(status));
            assert_eq!(
                serde_json::to_string(&status).unwrap(),
                format!("\"{}\"", status.as_str())
            );
        }
        let mut all = [Status::Up, Status::Unknown, Status::Warning, Status::Down];
        all.sort_by_key(|s| s.rank());
        assert_eq!(
            all,
            [Status::Down, Status::Warning, Status::Unknown, Status::Up]
        );
    }

    #[test]
    fn a_limit_is_a_number_or_off_in_json_and_in_forms() {
        let limits = MetricLimits {
            warning: Some(Setting::At(85.5)),
            down: Some(Setting::Off),
        };
        let json = serde_json::to_string(&limits).unwrap();
        assert_eq!(json, r#"{"warning":85.5,"down":"off"}"#);
        assert_eq!(serde_json::from_str::<MetricLimits>(&json).unwrap(), limits);
        assert_eq!(
            serde_json::from_str::<MetricLimits>("{}").unwrap(),
            MetricLimits::default(),
            "missing: inherited"
        );
        assert!(serde_json::from_str::<MetricLimits>(r#"{"warning":"high"}"#).is_err());

        assert_eq!(limits.texts(), ("85.5".to_string(), "off".to_string()));
        assert_eq!(
            MetricLimits::from_text(ThresholdMetric::Disk, " 85.5 ", "OFF"),
            Ok(limits)
        );
        assert_eq!(
            MetricLimits::from_text(ThresholdMetric::Disk, "", ""),
            Ok(MetricLimits::default())
        );
    }

    #[test]
    fn limits_are_checked_for_their_metric() {
        let at = |w: f64, d: f64| MetricLimits {
            warning: Some(Setting::At(w)),
            down: Some(Setting::At(d)),
        };
        assert_eq!(at(80.0, 95.0).validate(ThresholdMetric::Disk), Ok(()));
        assert!(
            at(95.0, 80.0).validate(ThresholdMetric::Disk).is_err(),
            "inverted"
        );
        assert!(
            at(80.0, 101.0).validate(ThresholdMetric::Cpu).is_err(),
            "over 100 %"
        );
        assert_eq!(at(200.0, 1000.0).validate(ThresholdMetric::Latency), Ok(()));
        assert!(at(-1.0, 10.0).validate(ThresholdMetric::Latency).is_err());
        // Days left fall: the warning comes first, so it is the higher limit.
        assert_eq!(at(30.0, 7.0).validate(ThresholdMetric::Tls), Ok(()));
        assert_eq!(
            at(7.0, 30.0).validate(ThresholdMetric::Tls),
            Err("Certificate: the warning limit is above the down limit".into())
        );
        assert!(at(7.0, 7.0).validate(ThresholdMetric::Tls).is_err());
        assert!(at(4000.0, 7.0).validate(ThresholdMetric::Tls).is_err());
        assert_eq!(Thresholds::built_in().validate(), Ok(()));
    }

    #[test]
    fn every_check_kind_that_measures_has_a_threshold_metric() {
        for kind in common::checks::CHECK_KINDS {
            // A service runs or doesn't, and so do processes, within the
            // numbers their check gives; uptime is only shown; an updates
            // check has limits of its own, in days, and an event log check
            // its own count.
            let expected = !["uptime", "service", "process", "eventlog", "updates"].contains(&kind);
            assert_eq!(ThresholdMetric::of_kind(kind).is_some(), expected, "{kind}");
        }
    }
}
