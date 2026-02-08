use async_trait::async_trait;
use common::models::{Health, ResourceStatus, ResourceType};
use surge_ping::{PingIdentifier, PingSequence};
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use crate::monitor::Monitor;
use crate::config::ResourceConfig;

/// A single monitoring check that produces exactly one [`ResourceStatus`].
///
/// Each implementor holds the per-resource parameters (name, target) captured
/// from config. Shared network resources (ICMP/HTTP clients) are borrowed from
/// [`Monitor`] at run time, which is the same pattern as the original methods.
#[async_trait]
pub trait Check: Send + Sync {
    async fn run(&self, monitor: &Monitor) -> ResourceStatus;
}

/// Mirrors `Monitor::error_status`: always sets `resource_type` to `"error"`
/// so downstream consumers (hub, storage) see the same shape as before.
fn error_status(name: &str, target: &str, msg: &str) -> ResourceStatus {
    ResourceStatus {
        name: name.to_string(),
        resource_type: ResourceType::Unknown,
        target: target.to_string(),
        status: Health::Down,
        message: msg.to_string(),
        latency_ms: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PING
// ─────────────────────────────────────────────────────────────────────────────

pub struct PingCheck {
    pub name: String,
    pub target: String,
}

#[async_trait]
impl Check for PingCheck {
    async fn run(&self, monitor: &Monitor) -> ResourceStatus {
        let ip: IpAddr = match self.target.parse() {
            Ok(addr) => addr,
            Err(_) => return error_status(&self.name, &self.target, "Invalid IP address"),
        };

        let payload = [0u8; 8];
        let mut pinger = monitor.icmp_client.pinger(ip, PingIdentifier(0)).await;

        match pinger.ping(PingSequence(0), &payload).await {
            Ok((_, duration)) => ResourceStatus {
                name: self.name.clone(),
                resource_type: ResourceType::Ping,
                target: self.target.clone(),
                status: Health::Up,
                message: format!("Responded in {:?}", duration),
                latency_ms: Some(duration.as_millis() as u64),
            },
            Err(e) => error_status(&self.name, &self.target, &format!("Ping failed: {}", e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP
// ─────────────────────────────────────────────────────────────────────────────

pub struct HttpCheck {
    pub name: String,
    pub target: String,
}

#[async_trait]
impl Check for HttpCheck {
    async fn run(&self, monitor: &Monitor) -> ResourceStatus {
        match monitor.http_client.get(&self.target).send().await {
            Ok(resp) if resp.status().is_success() => ResourceStatus {
                name: self.name.clone(),
                resource_type: ResourceType::Http,
                target: self.target.clone(),
                status: Health::Up,
                message: format!("HTTP {}", resp.status()),
                latency_ms: None,
            },
            Ok(resp) => error_status(
                &self.name,
                &self.target,
                &format!("HTTP Error: {}", resp.status()),
            ),
            Err(e) => error_status(
                &self.name,
                &self.target,
                &format!("Connection failed: {}", e),
            ),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TCP
// ─────────────────────────────────────────────────────────────────────────────

pub struct TcpCheck {
    pub name: String,
    pub target: String,
}

#[async_trait]
impl Check for TcpCheck {
    // TCP connections go directly through the OS; no shared client is needed.
    async fn run(&self, _monitor: &Monitor) -> ResourceStatus {
        let start = std::time::Instant::now();
        let timeout_duration = Duration::from_secs(3);

        let connection = tokio::time::timeout(
            timeout_duration,
            TcpStream::connect(&self.target),
        )
        .await;

        match connection {
            Ok(Ok(_)) => ResourceStatus {
                name: self.name.clone(),
                resource_type: ResourceType::Tcp,
                target: self.target.clone(),
                status: Health::Up,
                message: "Port Open".to_string(),
                latency_ms: Some(start.elapsed().as_millis() as u64),
            },
            Ok(Err(e)) => error_status(
                &self.name,
                &self.target,
                &format!("Connection Refused: {}", e),
            ),
            Err(_) => error_status(&self.name, &self.target, "Connection Timeout"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UNKNOWN / FALLBACK
// ─────────────────────────────────────────────────────────────────────────────

/// Fallback for resource types not recognised in config.
/// Produces the same output as the original `_ => m.error_status(...)` branch.
struct ErrorCheck {
    name: String,
    target: String,
}

#[async_trait]
impl Check for ErrorCheck {
    async fn run(&self, _monitor: &Monitor) -> ResourceStatus {
        // No network call needed; the type was already rejected by config validation.
        // This branch mirrors the original `_ => m.error_status(...)` fallback.
        error_status(&self.name, &self.target, "Unknown resource type")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FACTORY
// ─────────────────────────────────────────────────────────────────────────────

/// Construct a [`Check`] from a [`ResourceConfig`].
pub fn from_config(res: &ResourceConfig) -> Box<dyn Check> {
    match res.r#type {
        ResourceType::Ping => Box::new(PingCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        }),
        ResourceType::Http => Box::new(HttpCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        }),
        ResourceType::Tcp => Box::new(TcpCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        }),
        ResourceType::Unknown => Box::new(ErrorCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        }),
    }
}
