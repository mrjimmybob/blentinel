use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};


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
    #[serde(rename = "local_load")]
    LocalLoad,
    #[serde(rename = "local_uptime")]
    LocalUptime,
    Unknown,
}

impl ResourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceType::Ping => "ping",
            ResourceType::Http => "http",
            ResourceType::Tcp  => "tcp",
            ResourceType::LocalData => "local_data",
            ResourceType::LocalDisk => "local_disk",
            ResourceType::LocalCpu => "local_cpu",
            ResourceType::LocalMem => "local_mem",
            ResourceType::LocalLoad => "local_load",
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
            "tcp"  => Ok(ResourceType::Tcp),
            "local_data" => Ok(ResourceType::LocalData),
            "local_disk" => Ok(ResourceType::LocalDisk),
            "local_cpu" => Ok(ResourceType::LocalCpu),
            "local_mem" => Ok(ResourceType::LocalMem),
            "local_load" => Ok(ResourceType::LocalLoad),
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
        }
    }
}

