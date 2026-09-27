use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Ping,
    Http,
    Tcp,
    // Local system monitoring
    #[serde(rename = "local_data")]
    LocalData,
    #[serde(rename = "local_disk")]
    LocalDisk,
    #[serde(rename = "local_cpu")]
    LocalCpu,
    #[serde(rename = "local_mem")]
    LocalMem,
    #[serde(rename = "local_uptime")]
    LocalUptime,
    Unknown,
}

impl ResourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceType::Ping => "ping",
            ResourceType::Http => "http",
            ResourceType::Tcp => "tcp",
            ResourceType::LocalData => "local_data",
            ResourceType::LocalDisk => "local_disk",
            ResourceType::LocalCpu => "local_cpu",
            ResourceType::LocalMem => "local_mem",
            ResourceType::LocalUptime => "local_uptime",
            ResourceType::Unknown => "error",
        }
    }
}

impl TryFrom<&str> for ResourceType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "ping" => Ok(ResourceType::Ping),
            "http" => Ok(ResourceType::Http),
            "tcp" => Ok(ResourceType::Tcp),
            "local_data" => Ok(ResourceType::LocalData),
            "local_disk" => Ok(ResourceType::LocalDisk),
            "local_cpu" => Ok(ResourceType::LocalCpu),
            "local_mem" => Ok(ResourceType::LocalMem),
            "local_uptime" => Ok(ResourceType::LocalUptime),
            "error" => Ok(ResourceType::Unknown),
            other => Err(format!("Unknown resource type '{}'", other)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Health {
    Up,
    Down,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceStatus {
    pub name: String,
    pub resource_type: ResourceType,
    pub target: String,
    pub status: Health,
    pub message: String,
    pub latency_ms: Option<u64>,
    // Metrics for local system monitoring
    pub metric_value: Option<f64>,
    pub metric_unit: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatusReport {
    pub probe_id: String,
    pub company_id: String,
    pub hostname: String,
    pub site: String,
    pub timestamp: DateTime<Utc>,
    pub interval_seconds: u32,
    pub resources: Vec<ResourceStatus>,
    pub signature: Option<Vec<u8>>,
    pub ephemeral_public_key: Option<Vec<u8>>,
    /// Protocol version of the sender. Absent (`None`) means version 1: legacy
    /// probes never sent it. See [`crate::protocol::PROTOCOL_VERSION`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u16>,
}

/// Canonical signable representation of a StatusReport.
/// Excludes signature and ephemeral_public_key to avoid self-reference.
/// This struct defines the exact data that is cryptographically signed.
#[derive(Debug, Serialize, Clone)]
pub struct SignableReport {
    // WARNING: Field order is part of the signed canonical representation.
    // Do not reorder fields without bumping protocol version.
    pub probe_id: String,
    pub company_id: String,
    pub hostname: String,
    pub site: String,
    pub timestamp: DateTime<Utc>,
    pub interval_seconds: u32,
    pub resources: Vec<ResourceStatus>,
    /// Appended in protocol v2 and omitted when absent, so a v1 report
    /// serialises to exactly the bytes legacy probes signed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u16>,
}

impl StatusReport {
    /// Returns the canonical signable representation of this report.
    /// Both probe (signing) and hub (verification) must use this method
    /// to ensure they sign/verify the exact same serialized bytes.
    pub fn to_signable(&self) -> SignableReport {
        SignableReport {
            probe_id: self.probe_id.clone(),
            company_id: self.company_id.clone(),
            hostname: self.hostname.clone(),
            site: self.site.clone(),
            timestamp: self.timestamp,
            interval_seconds: self.interval_seconds,
            resources: self.resources.clone(),
            protocol_version: self.protocol_version,
        }
    }

    /// The protocol version this report was produced with (1 when absent).
    pub fn effective_protocol_version(&self) -> u16 {
        self.protocol_version.unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(version: Option<u16>) -> StatusReport {
        StatusReport {
            probe_id: "p".into(),
            company_id: "c".into(),
            hostname: "h".into(),
            site: "s".into(),
            timestamp: DateTime::parse_from_rfc3339("2026-03-10T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            interval_seconds: 60,
            resources: vec![],
            signature: None,
            ephemeral_public_key: None,
            protocol_version: version,
        }
    }

    #[test]
    fn v1_signable_bytes_are_unchanged_by_the_new_field() {
        // Exactly what protocol-v1 probes signed. If this changes, every
        // deployed v1 probe's signature stops verifying.
        let expected = r#"{"probe_id":"p","company_id":"c","hostname":"h","site":"s","timestamp":"2026-03-10T12:00:00Z","interval_seconds":60,"resources":[]}"#;
        assert_eq!(
            serde_json::to_string(&report(None).to_signable()).unwrap(),
            expected
        );
    }

    #[test]
    fn v2_signable_bytes_include_the_version_last() {
        let json = serde_json::to_string(&report(Some(2)).to_signable()).unwrap();
        assert!(
            json.ends_with(r#""resources":[],"protocol_version":2}"#),
            "{json}"
        );
    }

    #[test]
    fn missing_version_deserialises_as_v1() {
        let json = r#"{"probe_id":"p","company_id":"c","hostname":"h","site":"s","timestamp":"2026-03-10T12:00:00Z","interval_seconds":60,"resources":[],"signature":null,"ephemeral_public_key":null}"#;
        let parsed: StatusReport = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.protocol_version, None);
        assert_eq!(parsed.effective_protocol_version(), 1);
    }
}
