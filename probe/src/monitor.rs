use reqwest::Client as HttpClient;
use std::time::Duration;
use surge_ping::{Client, Config};
use sysinfo::{Disks, System};
use tokio::sync::RwLock;
use std::sync::Arc;

/// Shared resources used by all check implementations.
///
/// Network resources (ICMP, HTTP) are expensive to create (syscalls, OS handles).
/// System info (CPU, memory, disk) is expensive to poll repeatedly.
///
/// `Monitor` is constructed once at startup and shared via `Arc<Monitor>` across
/// every spawned check task. System info is refreshed once per monitoring cycle
/// before checks run, then read-locked by individual checks.
pub struct Monitor {
    pub(crate) icmp_client: Client,
    pub(crate) http_client: HttpClient,
    pub(crate) sys_info: Arc<RwLock<System>>,
    pub(crate) disk_info: Arc<RwLock<Disks>>,
}

impl Monitor {
    pub fn new() -> Self {
        let icmp_config = Config::default();
        Self {
            icmp_client: Client::new(&icmp_config).expect("Failed to create ICMP client"),
            http_client: HttpClient::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Failed to create HTTP client"),
            sys_info: Arc::new(RwLock::new(System::new_all())),
            disk_info: Arc::new(RwLock::new(Disks::new_with_refreshed_list())),
        }
    }

    /// Refresh system and disk information before running checks.
    /// Called once per monitoring cycle to get fresh data.
    pub async fn refresh(&self) {
        // Refresh system info (CPU, memory, uptime)
        let mut sys = self.sys_info.write().await;

        // For accurate CPU measurements, we need a small delay between refreshes
        tokio::time::sleep(Duration::from_millis(200)).await;
        sys.refresh_all();
        drop(sys);

        // Refresh disk info
        let mut disks = self.disk_info.write().await;
        disks.refresh(true);  // true = refresh all disks
        drop(disks);
    }
}
