#![cfg(feature = "ssr")]
//! Running the hub on its server (M2-08, proposal 0004 §4.2, D-52):
//! `hub install`, `uninstall`, `status`, `doctor`, `backup` and `restore`.
//!
//! They work on the installed hub's [`Layout`](layout::Layout), or on the
//! config given with `--config`. The OS specifics (systemd and the service
//! user on Linux, the Service Control Manager and folder permissions on
//! Windows) are in `linux` and `windows`.

pub mod backup;
pub mod doctor;
pub mod install;
pub mod layout;
pub mod status;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
pub mod windows;

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use crate::config::HubConfig;

/// Run a server administration command; the process's exit code.
pub async fn run(command: &crate::args::OpsCommand, config_path: &Path) -> i32 {
    use crate::args::OpsCommand;
    let load = || {
        crate::config::load_from(config_path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", config_path.display()))
    };
    let result: anyhow::Result<i32> = async {
        match command {
            OpsCommand::Install(options) => install::install(options).map(|()| 0),
            OpsCommand::Uninstall { delete_data } => install::uninstall(*delete_data).map(|()| 0),
            OpsCommand::Status => {
                print!("{}", status::status(&load()?, config_path).await);
                Ok(0)
            }
            OpsCommand::Doctor => {
                let (report, worst) = doctor::report(&doctor::run(config_path).await);
                print!("{report}");
                Ok(i32::from(worst == doctor::Level::Fail))
            }
            OpsCommand::Backup { out } => {
                let config = load()?;
                let now = chrono::Utc::now();
                let out = out
                    .clone()
                    .unwrap_or_else(|| backup::default_path(&config, now));
                let manifest = backup::backup(&config, config_path, &out, now).await?;
                println!(
                    "Backup written: {} (schema {}). It holds the hub's private key: keep it safe.",
                    out.display(),
                    manifest.schema_version
                );
                Ok(0)
            }
            OpsCommand::ReleaseAdd { path } => release_add(&load()?, path).map(|()| 0),
            OpsCommand::ReleaseList => {
                release_list(&load()?);
                Ok(0)
            }
            OpsCommand::Restore { file } => {
                let config = load()?;
                let manifest = backup::manifest_of(file)?;
                let aside = backup::restore(
                    file,
                    &config,
                    config_path,
                    hub_running(&config),
                    chrono::Utc::now(),
                )?;
                if platform::is_admin() {
                    platform::secure_state_dir(&config.server.state_dir)?;
                }
                println!(
                    "Restored the backup of {} (hub {}) into {}.",
                    manifest.created_at.format("%Y-%m-%d %H:%M UTC"),
                    manifest.hub_version,
                    config.server.state_dir.display()
                );
                if let Some(aside) = aside {
                    println!("The files it replaced are in {}.", aside.display());
                }
                println!("Start the hub again; `hub doctor` checks the result.");
                Ok(0)
            }
        }
    }
    .await;
    result.unwrap_or_else(|e| {
        eprintln!("[ERROR] {e:#}");
        1
    })
}

/// What the service manager says about the hub's service.
pub use common::service::ServiceState;

/// The operating system's service manager and permissions. Linux and
/// Windows are supported (D-36); elsewhere there's no service to install.
pub(crate) mod platform {
    #[cfg(target_os = "linux")]
    pub use super::linux::{
        ensure_service_user, install_service, is_admin, remove_service, secure_config,
        secure_state_dir, service_state, stop_service,
    };
    #[cfg(windows)]
    pub use super::windows::{
        install_service, is_admin, remove_service, secure_state_dir, service_state, stop_service,
    };

    /// Windows: the config lives in the (already restricted) state folder.
    #[cfg(windows)]
    pub fn secure_config(_: &std::path::Path) -> anyhow::Result<()> {
        Ok(())
    }
    /// Windows services run as LocalSystem: no user to create.
    #[cfg(windows)]
    pub fn ensure_service_user() -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    mod unsupported {
        use super::super::{ServiceState, layout::Layout};
        use anyhow::{Result, bail};
        use std::path::Path;

        const WHY: &str = "`hub install` supports Linux (systemd) and Windows";
        pub fn is_admin() -> bool {
            false
        }
        pub fn ensure_service_user() -> Result<()> {
            bail!(WHY)
        }
        pub fn secure_state_dir(_: &Path) -> Result<()> {
            bail!(WHY)
        }
        pub fn secure_config(_: &Path) -> Result<()> {
            bail!(WHY)
        }
        pub fn install_service(_: &Layout, _: bool) -> Result<()> {
            bail!(WHY)
        }
        pub fn stop_service() -> Result<()> {
            Ok(())
        }
        pub fn remove_service() -> Result<()> {
            Ok(())
        }
        pub fn service_state() -> ServiceState {
            ServiceState::NotInstalled
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    pub use unsupported::*;
}

/// The message for a command that needs root or Administrator.
pub(crate) fn needs_admin(command: &str) -> anyhow::Error {
    if cfg!(windows) {
        anyhow::anyhow!("`hub {command}` must run in an elevated (Administrator) terminal")
    } else {
        anyhow::anyhow!("`hub {command}` must run as root: sudo ./hub {command}")
    }
}

/// Where to reach a port the hub listens on from this machine: its `host`,
/// or the loopback address when it listens on every interface.
pub(crate) fn local_address(host: &str, port: u16) -> Option<SocketAddr> {
    let host = match host {
        "0.0.0.0" | "" => "127.0.0.1",
        "::" | "[::]" => "::1",
        other => other,
    };
    (host, port).to_socket_addrs().ok()?.next()
}

/// Does something accept connections on `host:port`?
pub(crate) fn answers(host: &str, port: u16) -> bool {
    local_address(host, port)
        .is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok())
}

/// The ports the hub listens on: plain HTTP, and HTTPS if it's separate.
pub(crate) fn ports(config: &HubConfig) -> Vec<(&'static str, u16)> {
    let tls = &config.server.tls;
    if !tls.enabled {
        vec![("web UI and probes, HTTP", config.server.port)]
    } else if tls.https_port == config.server.port {
        vec![("web UI and probes, HTTPS", config.server.port)]
    } else {
        vec![
            ("probes, HTTP", config.server.port),
            ("web UI, HTTPS", tls.https_port),
        ]
    }
}

/// Is the hub running: its service, or anything answering on its port?
pub(crate) fn hub_running(config: &HubConfig) -> bool {
    platform::service_state() == ServiceState::Running
        || answers(&config.server.host, config.server.port)
}

/// This machine's name, for the URLs `hub install` prints.
pub(crate) fn machine_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this-server".to_string())
}

/// The web UI's address on this machine.
pub(crate) fn web_url(config: &HubConfig) -> String {
    let (scheme, port) = if config.server.tls.enabled {
        ("https", config.server.tls.https_port)
    } else {
        ("http", config.server.port)
    };
    format!("{scheme}://{}:{port}/", machine_name())
}

/// The pending setup code, if the hub still waits for its first Owner.
pub(crate) fn setup_code(state_dir: &Path) -> Option<String> {
    std::fs::read_to_string(state_dir.join(crate::service::setup::SETUP_CODE_FILE))
        .ok()
        .map(|code| code.trim().to_string())
        .filter(|code| !code.is_empty())
}

/// `hub release add <path>`: a release folder or its `.tar.gz` (D-55).
fn release_add(config: &crate::config::HubConfig, path: &Path) -> anyhow::Result<()> {
    use crate::service::releases;
    let root = config.server.state_dir.join(releases::RELEASES_DIR);
    let keys = common::release::trusted_keys();
    let manifest = if path.is_dir() {
        releases::add_from_dir(&root, path, &keys)?
    } else {
        let archive = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("can't read {}: {e}", path.display()))?;
        releases::add_from_archive(&root, &archive, &keys)?
    };
    // An installed hub's service must own what root just wrote; a hub run by
    // hand has no service user, and nothing to fix.
    if platform::is_admin()
        && let Err(e) = platform::secure_state_dir(&config.server.state_dir)
    {
        eprintln!("[WARN] {e:#}");
    }
    println!(
        "Added probe release {} ({} binaries). Approve it on the admin page (Probe updates) to update running probes.",
        manifest.version,
        manifest.artifacts.len()
    );
    Ok(())
}

/// `hub release list`.
fn release_list(config: &crate::config::HubConfig) {
    use crate::service::releases;
    let root = config.server.state_dir.join(releases::RELEASES_DIR);
    let listed = releases::list(&root, &common::release::trusted_keys());
    if listed.is_empty() {
        println!("No probe releases in {}.", root.display());
    }
    for stored in &listed {
        let release = releases::describe(stored);
        println!(
            "{:<12} {}  {}",
            release.version,
            release.released,
            release.platforms.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interface_is_reached_through_loopback() {
        assert_eq!(
            local_address("0.0.0.0", 3000),
            Some("127.0.0.1:3000".parse().unwrap())
        );
        assert_eq!(
            local_address("::", 3000),
            Some("[::1]:3000".parse().unwrap())
        );
        assert_eq!(
            local_address("127.0.0.1", 8),
            Some("127.0.0.1:8".parse().unwrap())
        );
    }

    #[test]
    fn a_split_hub_has_two_ports_and_an_https_only_one_one() {
        let mut config: HubConfig = toml::from_str(
            "[server]\nhost = '0.0.0.0'\nport = 3000\nstate_dir = '.'\ndb_path = 'hub.db'\n",
        )
        .unwrap();
        assert_eq!(ports(&config).len(), 2);
        config.server.tls.https_port = 3000;
        assert_eq!(ports(&config).len(), 1);
        config.server.tls.enabled = false;
        assert_eq!(ports(&config)[0].1, 3000);
        assert!(web_url(&config).starts_with("http://"));
    }
}
