use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::checks::{CheckResult, ConfigRejection};

/// What a probe sends every interval (protocol v4, D-49).
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
    pub signature: Option<Vec<u8>>,
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
