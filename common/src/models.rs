use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Health {
    Up,
    Down,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceStatus {
    pub name: String,
    pub resource_type: String,
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