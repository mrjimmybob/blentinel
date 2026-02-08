use reqwest::Client as HttpClient;
use std::time::Duration;
use surge_ping::{Client, Config};

/// Shared network resources used by all check implementations.
///
/// Creating an ICMP socket and an HTTP connection pool is expensive (syscalls,
/// OS handles).  `Monitor` is constructed once at startup and shared — via
/// `Arc<Monitor>` — across every spawned check task, just as before.
///
/// The individual check types live in [`crate::checks`] and borrow these
/// fields at run time.
pub struct Monitor {
    pub(crate) icmp_client: Client,
    pub(crate) http_client: HttpClient,
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
        }
    }
}
