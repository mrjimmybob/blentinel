use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::checks::{CheckResult, ConfigRejection};
use crate::commands::CommandResult;

/// What a probe sends every interval (protocol v4, D-49; v5 adds the
/// probe's version and its last failed update, D-55; v6 the results of
/// commands, D-56; v7 the hub pins it holds, D-61; v8 its own health, D-73).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatusReport {
    /// Protocol version of the sender ([`crate::protocol::PROTOCOL_VERSION`]).
    pub protocol_version: u16,
    pub probe_id: String,
    /// Informational: the hub files reports by the probe's record (D-47).
    pub company_id: String,
    pub hostname: String,
    /// Informational, like `company_id`.
    pub site: String,
    pub timestamp: DateTime<Utc>,
    pub interval_seconds: u32,
    /// One per check of the config the probe runs.
    pub results: Vec<CheckResult>,
    /// The [`crate::checks::ProbeConfig::revision`] the probe runs; `None`
    /// before its first config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_revision: Option<String>,
    /// The last config the hub sent that the probe couldn't use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_rejected: Option<ConfigRejection>,
    /// The probe's own version (v5). `None` from a v4 probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_version: Option<String>,
    /// The last update the probe refused or rolled back (v5), until it
    /// updates to another version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_failed: Option<UpdateFailure>,
    /// What came of the commands the probe ran since its last report (v6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_results: Vec<CommandResult>,
    /// Which hub pins the probe holds (v7, D-61), as digests
    /// (`common::pins::HubPins::digest`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pins: Option<PinDigests>,
    /// How the probe itself is doing (v8, D-73).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<ProbeHealth>,
    pub signature: Option<Vec<u8>>,
}

/// How a probe itself is doing when it makes a report (v8, D-73): what the
/// hub can't see from the results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeHealth {
    /// When this run of the probe started: its uptime.
    pub started_at: DateTime<Utc>,
    /// Reports waiting for the hub when this one was made.
    pub queued_reports: u32,
    /// This machine's clock minus the hub's, in seconds, from the hub's last
    /// answer; `None` before the first answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_offset_secs: Option<i64>,
    /// The probe's last problem of the past day, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<ProbeProblem>,
}

/// A problem the probe ran into: a report it couldn't deliver, a check that
/// couldn't run, keys it couldn't save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeProblem {
    pub at: DateTime<Utc>,
    pub message: String,
}

/// Which hub pins a probe holds (v7, D-61): digests of its current pins and
/// of the next ones the hub announced. The same shape as
/// `common::pins::PinState`, here so the WASM front end needs no crypto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinDigests {
    pub current: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// An update a probe didn't install, or rolled back, and why (D-55).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateFailure {
    pub version: String,
    pub error: String,
}

/// Canonical signable representation of a StatusReport: every field except
/// the signature itself. Both probe (signing) and hub (verification) use
/// [`StatusReport::to_signable`], so they sign and verify the same bytes.
#[derive(Debug, Serialize, Clone)]
pub struct SignableReport<'a> {
    // Field order is part of the signed bytes: changing it is a protocol change.
    pub protocol_version: u16,
    pub probe_id: &'a str,
    pub company_id: &'a str,
    pub hostname: &'a str,
    pub site: &'a str,
    pub timestamp: DateTime<Utc>,
    pub interval_seconds: u32,
    pub results: &'a [CheckResult],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_revision: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_rejected: Option<&'a ConfigRejection>,
    // v5: appended and omitted when absent, so a v4 report signs the same bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_failed: Option<&'a UpdateFailure>,
    // v6: appended and omitted when empty, so a v5 report signs the same bytes.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub command_results: &'a [CommandResult],
    // v7: appended and omitted when absent, so a v6 report signs the same bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pins: Option<&'a PinDigests>,
    // v8: appended and omitted when absent, so a v7 report signs the same bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<&'a ProbeHealth>,
}

impl StatusReport {
    /// The canonical signable representation of this report.
    pub fn to_signable(&self) -> SignableReport<'_> {
        SignableReport {
            protocol_version: self.protocol_version,
            probe_id: &self.probe_id,
            company_id: &self.company_id,
            hostname: &self.hostname,
            site: &self.site,
            timestamp: self.timestamp,
            interval_seconds: self.interval_seconds,
            results: &self.results,
            config_revision: self.config_revision.as_deref(),
            config_rejected: self.config_rejected.as_ref(),
            probe_version: self.probe_version.as_deref(),
            update_failed: self.update_failed.as_ref(),
            command_results: &self.command_results,
            pins: self.pins.as_ref(),
            health: self.health.as_ref(),
        }
    }
}

/// Just the version of a report, read before the rest so a report from an
/// older or newer probe is refused for its version, not as malformed.
#[derive(Debug, Deserialize)]
pub struct ReportVersion {
    /// Absent before protocol v2.
    #[serde(default)]
    pub protocol_version: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckId, Health, Metric};

    fn report() -> StatusReport {
        StatusReport {
            protocol_version: 4,
            probe_id: "p".into(),
            company_id: "c".into(),
            hostname: "h".into(),
            site: "s".into(),
            timestamp: DateTime::parse_from_rfc3339("2026-03-10T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            interval_seconds: 60,
            results: vec![CheckResult {
                check_id: CheckId("c1".into()),
                status: Health::Up,
                message: "ok".into(),
                latency_ms: Some(3),
                metrics: vec![Metric::new("usage", 12.5, "%")],
            }],
            config_revision: Some("r1".into()),
            config_rejected: None,
            probe_version: None,
            update_failed: None,
            command_results: vec![],
            pins: None,
            health: None,
            signature: None,
        }
    }

    #[test]
    fn signable_bytes_are_pinned() {
        // What v4 probes sign. If this changes, deployed probes' signatures stop verifying.
        let expected = r#"{"protocol_version":4,"probe_id":"p","company_id":"c","hostname":"h","site":"s","timestamp":"2026-03-10T12:00:00Z","interval_seconds":60,"results":[{"check_id":"c1","status":"Up","message":"ok","latency_ms":3,"metrics":[{"key":"usage","value":12.5,"unit":"%"}]}],"config_revision":"r1"}"#;
        assert_eq!(
            serde_json::to_string(&report().to_signable()).unwrap(),
            expected
        );
    }

    #[test]
    fn v5_fields_are_signed_after_the_v4_ones() {
        let mut report = report();
        report.protocol_version = 5;
        report.probe_version = Some("0.2.0".into());
        report.update_failed = Some(UpdateFailure {
            version: "0.3.0".into(),
            error: "rolled back".into(),
        });
        let json = serde_json::to_string(&report.to_signable()).unwrap();
        assert!(
            json.ends_with(r#""config_revision":"r1","probe_version":"0.2.0","update_failed":{"version":"0.3.0","error":"rolled back"}}"#),
            "{json}"
        );
    }

    #[test]
    fn v6_command_results_are_signed_last() {
        let mut report = report();
        report.protocol_version = 6;
        report.probe_version = Some("0.2.0".into());
        report.command_results = vec![CommandResult::new("k1", true, "ran 1 check")];
        let json = serde_json::to_string(&report.to_signable()).unwrap();
        assert!(
            json.ends_with(r#""probe_version":"0.2.0","command_results":[{"id":"k1","ok":true,"message":"ran 1 check"}]}"#),
            "{json}"
        );
    }

    #[test]
    fn v7_pins_are_signed_last() {
        let mut report = report();
        report.protocol_version = 7;
        report.command_results = vec![CommandResult::new("k1", true, "ok")];
        report.pins = Some(PinDigests {
            current: "aa".into(),
            next: Some("bb".into()),
        });
        let json = serde_json::to_string(&report.to_signable()).unwrap();
        assert!(
            json.ends_with(r#""command_results":[{"id":"k1","ok":true,"message":"ok"}],"pins":{"current":"aa","next":"bb"}}"#),
            "{json}"
        );
    }

    #[test]
    fn v8_health_is_signed_last() {
        let mut report = report();
        report.protocol_version = 8;
        report.pins = Some(PinDigests {
            current: "aa".into(),
            next: None,
        });
        report.health = Some(ProbeHealth {
            started_at: "2026-03-10T11:00:00Z".parse().unwrap(),
            queued_reports: 3,
            clock_offset_secs: Some(-2),
            last_error: Some(ProbeProblem {
                at: "2026-03-10T11:30:00Z".parse().unwrap(),
                message: "hub unreachable".into(),
            }),
        });
        let json = serde_json::to_string(&report.to_signable()).unwrap();
        assert!(
            json.ends_with(r#""pins":{"current":"aa"},"health":{"started_at":"2026-03-10T11:00:00Z","queued_reports":3,"clock_offset_secs":-2,"last_error":{"at":"2026-03-10T11:30:00Z","message":"hub unreachable"}}}"#),
            "{json}"
        );
    }

    #[test]
    fn a_report_round_trips() {
        let json = serde_json::to_string(&report()).unwrap();
        let parsed: StatusReport = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&parsed.to_signable()).unwrap(),
            serde_json::to_string(&report().to_signable()).unwrap()
        );
    }

    #[test]
    fn the_version_is_read_from_any_report() {
        let v3 = r#"{"probe_id":"p","resources":[],"protocol_version":3}"#;
        let version: ReportVersion = serde_json::from_str(v3).unwrap();
        assert_eq!(version.protocol_version, Some(3));
        let v1: ReportVersion = serde_json::from_str(r#"{"probe_id":"p"}"#).unwrap();
        assert_eq!(v1.protocol_version, None);
    }
}
