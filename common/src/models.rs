use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Ping,
    Http,
    Tcp,
    Unknown,
}

impl ResourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceType::Ping => "ping",
            ResourceType::Http => "http",
            ResourceType::Tcp  => "tcp",
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatusReport {
    pub probe_id: String,
    pub company_id: String,
    pub timestamp: DateTime<Utc>,
    pub interval_seconds: u32,
    pub resources: Vec<ResourceStatus>,
    pub signature: Option<Vec<u8>>,
    pub ephemeral_public_key: Option<Vec<u8>>,
}

