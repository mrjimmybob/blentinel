use reqwest::Client as HttpClient;
use reqwest::redirect::Policy;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use surge_ping::{Client, Config, ICMP};
use sysinfo::{Disks, System};
use tokio::sync::RwLock;

/// Shared resources used by all check implementations.
///
/// Network resources (ICMP, HTTP) are expensive to create (syscalls, OS handles),
/// and so is the list of authorities `tls` checks trust.
/// System info (CPU, memory, disk) is expensive to poll repeatedly.
///
/// `Monitor` is constructed once at startup and shared via `Arc<Monitor>` across
/// every spawned check task. System info is refreshed once per monitoring cycle
/// before checks run, then read-locked by individual checks.
/// An ICMP socket for one IP version. Without the permission for it (an
/// unprivileged account, a container without `CAP_NET_RAW`) the probe still
/// runs: its `ping` checks say so, or use their TCP port (D-80).
fn icmp_client(kind: ICMP) -> Option<Client> {
    Client::new(&Config::builder().kind(kind).build())
        .inspect_err(|e| tracing::warn!(?kind, error = %e, "No ICMP socket: ping checks of this IP version can't send echoes"))
        .ok()
}

/// How long an `http` check waits for the whole answer.
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// How many redirects an `http` check follows.
const MAX_REDIRECTS: usize = 10;

/// An HTTP client for checks. Its TLS is the probe's own configuration
/// (`crate::tls`): the authorities compiled in, or any certificate; never
/// the machine's certificate store (FP-2).
fn http_client(follow_redirects: bool, verify: bool) -> HttpClient {
    let tls = if verify {
        crate::tls::public_client_config()
    } else {
        crate::tls::unverified_client_config()
    };
    let redirects = if follow_redirects {
        Policy::limited(MAX_REDIRECTS)
    } else {
        Policy::none()
    };
    // Neither fails with the provider and the settings compiled in.
    HttpClient::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(redirects)
        .tls_backend_preconfigured(tls.expect("Failed to configure TLS"))
        .build()
        .expect("Failed to create HTTP client")
}

pub struct Monitor {
    /// For `ping` checks, one per IP version. `None`: the system doesn't
    /// let this probe open that ICMP socket ([`Monitor::icmp_client`]).
    icmp_v4: Option<Client>,
    icmp_v6: Option<Client>,
    /// One per combination of following redirects and verifying
    /// certificates (both belong to a client, not to a request):
    /// [`Monitor::http_client`].
    http_clients: [HttpClient; 4],
    pub(crate) tls_trust: crate::checks::Trust,
    pub(crate) sys_info: Arc<RwLock<System>>,
    pub(crate) disk_info: Arc<RwLock<Disks>>,
    /// The network interfaces' counters, for `interface` checks.
    pub(crate) net_info: Arc<RwLock<crate::checks::interface::Traffic>>,
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            icmp_v4: icmp_client(ICMP::V4),
            icmp_v6: icmp_client(ICMP::V6),
            http_clients: [
                http_client(false, false),
                http_client(false, true),
                http_client(true, false),
                http_client(true, true),
            ],
            tls_trust: crate::checks::public_trust(),
            sys_info: Arc::new(RwLock::new(System::new_all())),
            disk_info: Arc::new(RwLock::new(Disks::new_with_refreshed_list())),
            net_info: Arc::new(RwLock::new(crate::checks::interface::Traffic::new())),
        }
    }

    /// The client that sends echoes to `ip`, if the probe may send any.
    pub(crate) fn icmp_client(&self, ip: IpAddr) -> Option<&Client> {
        match ip {
            IpAddr::V4(_) => self.icmp_v4.as_ref(),
            IpAddr::V6(_) => self.icmp_v6.as_ref(),
        }
    }

    /// The client for an `http` check that follows redirects or not, and
    /// verifies certificates or not.
    pub(crate) fn http_client(&self, follow_redirects: bool, verify: bool) -> &HttpClient {
        &self.http_clients[usize::from(follow_redirects) * 2 + usize::from(verify)]
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
        disks.refresh(true); // true = refresh all disks
        drop(disks);

        // The interfaces' counters: what changed since the last cycle
        self.net_info.write().await.refresh();
    }
}
