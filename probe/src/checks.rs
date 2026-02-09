use async_trait::async_trait;
use common::models::{Health, ResourceStatus, ResourceType};
use surge_ping::{PingIdentifier, PingSequence};
use std::net::IpAddr;
use std::time::Duration;
use sysinfo::System;
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
        metric_value: None,
        metric_unit: None,
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
                metric_value: None,
                metric_unit: None,
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
                metric_value: None,
                metric_unit: None,
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
                metric_value: None,
                metric_unit: None,
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
// LOCAL SYSTEM MONITORING
// ─────────────────────────────────────────────────────────────────────────────

/// CPU usage percentage
pub struct LocalCpuCheck {
    pub name: String,
}

#[async_trait]
impl Check for LocalCpuCheck {
    async fn run(&self, monitor: &Monitor) -> ResourceStatus {
        // Use shared system info (already refreshed by Monitor::refresh)
        let sys = monitor.sys_info.read().await;
        let cpu_usage = sys.global_cpu_usage();

        ResourceStatus {
            name: self.name.clone(),
            resource_type: ResourceType::LocalCpu,
            target: "localhost".to_string(),
            status: Health::Up,
            message: format!("CPU usage: {:.1}%", cpu_usage),
            latency_ms: None,
            metric_value: Some(cpu_usage as f64),
            metric_unit: Some("%".to_string()),
        }
    }
}

/// Memory usage percentage
pub struct LocalMemCheck {
    pub name: String,
}

#[async_trait]
impl Check for LocalMemCheck {
    async fn run(&self, monitor: &Monitor) -> ResourceStatus {
        // Use shared system info (already refreshed by Monitor::refresh)
        let sys = monitor.sys_info.read().await;

        let total = sys.total_memory();
        let used = sys.used_memory();

        if total == 0 {
            return error_status(&self.name, "localhost", "Failed to read memory info");
        }

        let usage_pct = (used as f64 / total as f64) * 100.0;
        let used_gb = used as f64 / 1024.0 / 1024.0 / 1024.0;
        let total_gb = total as f64 / 1024.0 / 1024.0 / 1024.0;

        ResourceStatus {
            name: self.name.clone(),
            resource_type: ResourceType::LocalMem,
            target: "localhost".to_string(),
            status: Health::Up,
            message: format!("Memory: {:.1} GB / {:.1} GB ({:.1}%)", used_gb, total_gb, usage_pct),
            latency_ms: None,
            metric_value: Some(usage_pct),
            metric_unit: Some("%".to_string()),
        }
    }
}

/// Disk usage (single disk or all disks based on target)
pub struct LocalDiskCheck {
    pub name: String,
    pub target: String,
}

#[async_trait]
impl Check for LocalDiskCheck {
    async fn run(&self, monitor: &Monitor) -> ResourceStatus {
        // Use shared disk info (already refreshed by Monitor::refresh)
        let disks = monitor.disk_info.read().await;

        if disks.is_empty() {
            return error_status(&self.name, &self.target, "No disks found");
        }

        // Find the matching disk
        let disk = if self.target == "auto" || self.target.is_empty() {
            // Default disk: root (/) on Linux, system drive on Windows
            #[cfg(target_os = "windows")]
            let default_path = "C:\\";
            #[cfg(not(target_os = "windows"))]
            let default_path = "/";

            disks.iter().find(|d| d.mount_point().to_string_lossy().starts_with(default_path))
        } else {
            disks.iter().find(|d| d.mount_point().to_string_lossy() == self.target.as_str())
        };

        match disk {
            Some(disk) => {
                let total = disk.total_space();
                let available = disk.available_space();
                let used = total - available;

                if total == 0 {
                    return error_status(&self.name, &self.target, "Disk has zero capacity");
                }

                let usage_pct = (used as f64 / total as f64) * 100.0;
                let used_gb = used as f64 / 1024.0 / 1024.0 / 1024.0;
                let total_gb = total as f64 / 1024.0 / 1024.0 / 1024.0;

                ResourceStatus {
                    name: self.name.clone(),
                    resource_type: ResourceType::LocalDisk,
                    target: disk.mount_point().to_string_lossy().to_string(),
                    status: Health::Up,
                    message: format!("{}: {:.1} GB / {:.1} GB ({:.1}%)",
                        disk.mount_point().display(), used_gb, total_gb, usage_pct),
                    latency_ms: None,
                    metric_value: Some(usage_pct),
                    metric_unit: Some("%".to_string()),
                }
            }
            None => error_status(&self.name, &self.target, "Disk not found"),
        }
    }
}

/// System load average (1 minute) — Linux only
pub struct LocalLoadCheck {
    pub name: String,
}

#[async_trait]
impl Check for LocalLoadCheck {
    async fn run(&self, _monitor: &Monitor) -> ResourceStatus {
        #[cfg(target_os = "linux")]
        {
            let load = System::load_average();
            ResourceStatus {
                name: self.name.clone(),
                resource_type: ResourceType::LocalLoad,
                target: "localhost".to_string(),
                status: Health::Up,
                message: format!("Load: {:.2} (1m), {:.2} (5m), {:.2} (15m)",
                    load.one, load.five, load.fifteen),
                latency_ms: None,
                metric_value: Some(load.one),
                metric_unit: Some("load".to_string()),
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            ResourceStatus {
                name: self.name.clone(),
                resource_type: ResourceType::LocalLoad,
                target: "localhost".to_string(),
                status: Health::Up,
                message: "Not supported on this platform".to_string(),
                latency_ms: None,
                metric_value: None,
                metric_unit: None,
            }
        }
    }
}

/// System uptime in seconds
pub struct LocalUptimeCheck {
    pub name: String,
}

#[async_trait]
impl Check for LocalUptimeCheck {
    async fn run(&self, _monitor: &Monitor) -> ResourceStatus {
        // System::uptime() is a static method, doesn't need instance
        let uptime = System::uptime();

        let days = uptime / 86400;
        let hours = (uptime % 86400) / 3600;
        let minutes = (uptime % 3600) / 60;

        ResourceStatus {
            name: self.name.clone(),
            resource_type: ResourceType::LocalUptime,
            target: "localhost".to_string(),
            status: Health::Up,
            message: format!("Uptime: {}d {}h {}m", days, hours, minutes),
            latency_ms: None,
            metric_value: Some(uptime as f64),
            metric_unit: Some("seconds".to_string()),
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

/// Construct [`Check`] instances from a [`ResourceConfig`].
///
/// Most resource types produce a single check. `LocalData` expands into multiple
/// checks (CPU, memory, disk, load, uptime) at creation time. This keeps the
/// execution model unchanged — each check still produces exactly one ResourceStatus.
pub fn from_config(res: &ResourceConfig) -> Vec<Box<dyn Check>> {
    match res.r#type {
        ResourceType::Ping => vec![Box::new(PingCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        })],
        ResourceType::Http => vec![Box::new(HttpCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        })],
        ResourceType::Tcp => vec![Box::new(TcpCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        })],
        ResourceType::LocalCpu => vec![Box::new(LocalCpuCheck {
            name: res.name.clone(),
        })],
        ResourceType::LocalMem => vec![Box::new(LocalMemCheck {
            name: res.name.clone(),
        })],
        ResourceType::LocalDisk => vec![Box::new(LocalDiskCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        })],
        ResourceType::LocalLoad => vec![Box::new(LocalLoadCheck {
            name: res.name.clone(),
        })],
        ResourceType::LocalUptime => vec![Box::new(LocalUptimeCheck {
            name: res.name.clone(),
        })],
        // LocalData is a meta-type that expands into all local checks
        ResourceType::LocalData => {
            let base_name = &res.name;
            // Determine default disk target
            #[cfg(target_os = "windows")]
            let default_disk = "C:\\";
            #[cfg(not(target_os = "windows"))]
            let default_disk = "/";

            vec![
                Box::new(LocalCpuCheck {
                    name: format!("{} (CPU)", base_name),
                }) as Box<dyn Check>,
                Box::new(LocalMemCheck {
                    name: format!("{} (Memory)", base_name),
                }),
                Box::new(LocalDiskCheck {
                    name: format!("{} (Disk)", base_name),
                    target: default_disk.to_string(),
                }),
                Box::new(LocalLoadCheck {
                    name: format!("{} (Load)", base_name),
                }),
                Box::new(LocalUptimeCheck {
                    name: format!("{} (Uptime)", base_name),
                }),
            ]
        }
        ResourceType::Unknown => vec![Box::new(ErrorCheck {
            name: res.name.clone(),
            target: res.target.clone(),
        })],
    }
}
