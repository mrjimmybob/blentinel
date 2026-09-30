//! Linux: the probe's systemd service, managed over D-Bus with `zbus` (pure
//! Rust): the probe runs no external tools, not even `systemctl` (FP-2, D-53).
//!
//! The unit runs the probe as a throwaway user (`DynamicUser=yes`) with its
//! folder as `StateDirectory=`: no user to create, and systemd keeps the
//! folder owned by that user and closed to others (`0700`).

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use common::service::ServiceState;
use zbus::zvariant::OwnedObjectPath;

use super::{Layout, SERVICE_NAME};

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Manager {
    fn reload(&self) -> zbus::Result<()>;
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Vec<(String, String, String)>)>;
    fn disable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
    ) -> zbus::Result<Vec<(String, String, String)>>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn load_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
trait Unit {
    #[zbus(property)]
    fn active_state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn load_state(&self) -> zbus::Result<String>;
}

/// A connection to systemd over the system bus. (Not systemd's private
/// socket: zbus 5 panics on the sender names systemd uses there, D-53.)
async fn connect() -> Result<zbus::Connection> {
    zbus::Connection::system().await.context(
        "can't reach systemd over D-Bus: is D-Bus running (systemctl status dbus)? On a machine without systemd, run the probe yourself instead",
    )
}

async fn manager(connection: &zbus::Connection) -> Result<ManagerProxy<'static>> {
    Ok(ManagerProxy::new(connection).await?)
}

fn unit_name() -> String {
    format!("{SERVICE_NAME}.service")
}

fn unit_path() -> String {
    format!("/etc/systemd/system/{}", unit_name())
}

/// The unit: the probe as a throwaway user, in its own folder, allowed to ping.
pub fn systemd_unit(layout: &Layout) -> String {
    let folder = layout
        .dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "# Written by `probe install` (Blentinel, D-53). Re-run it to update.\n\
         [Unit]\n\
         Description=Blentinel Probe\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe}\n\
         WorkingDirectory={dir}\n\
         # A throwaway user; systemd keeps the folder its own and private.\n\
         DynamicUser=yes\n\
         StateDirectory={folder}\n\
         StateDirectoryMode=0700\n\
         # Ping without root.\n\
         AmbientCapabilities=CAP_NET_RAW\n\
         CapabilityBoundingSet=CAP_NET_RAW\n\
         Restart=always\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        exe = layout.exe.display(),
        dir = layout.dir.display(),
    )
}

/// Is this process root? (From `/proc`, no library needed.)
pub fn is_admin() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .and_then(|uids| uids.split_whitespace().nth(1).map(|e| e == "0"))
        })
        .unwrap_or(false)
}

/// Before the first start the folder is root's; afterwards systemd's user
/// owns it. Files written by root (re-enrolling) get that user as owner.
pub fn secure_folder(dir: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::create_dir_all(dir)?;
    let meta = std::fs::metadata(dir)?;
    if meta.uid() == 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        std::os::unix::fs::chown(entry.path(), Some(meta.uid()), Some(meta.gid()))?;
    }
    Ok(())
}

/// Write the unit, enable it, and (re)start it if `start`.
pub async fn install_service(layout: &Layout, start: bool) -> Result<()> {
    std::fs::write(unit_path(), systemd_unit(layout))
        .with_context(|| format!("can't write {}", unit_path()))?;
    let connection = connect().await?;
    let manager = manager(&connection).await?;
    manager.reload().await.context("systemd reload")?;
    manager
        .enable_unit_files(&[&unit_name()], false, true)
        .await
        .context("can't enable the service")?;
    if start {
        manager
            .restart_unit(&unit_name(), "replace")
            .await
            .context("can't start the service")?;
        wait_for(&connection, |state| state == "active" || state == "failed").await?;
        if state_of(&connection).await? == "failed" {
            bail!("the service failed to start: journalctl -u {SERVICE_NAME}");
        }
    }
    Ok(())
}

/// The unit's `ActiveState` (`active`, `inactive`, `failed`, …).
async fn state_of(connection: &zbus::Connection) -> Result<String> {
    let path = manager(connection).await?.load_unit(&unit_name()).await?;
    let unit = UnitProxy::builder(connection).path(path)?.build().await?;
    Ok(unit.active_state().await?)
}

/// Wait (up to 30 s) for the unit's state to satisfy `done`.
async fn wait_for(connection: &zbus::Connection, done: impl Fn(&str) -> bool) -> Result<()> {
    for _ in 0..60 {
        if done(&state_of(connection).await?) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    bail!("the service didn't settle within 30 s")
}

pub async fn stop_service() -> Result<()> {
    if !Path::new(&unit_path()).exists() {
        return Ok(());
    }
    let connection = connect().await?;
    manager(&connection)
        .await?
        .stop_unit(&unit_name(), "replace")
        .await
        .context("can't stop the service")?;
    wait_for(&connection, |state| {
        state != "active" && state != "deactivating"
    })
    .await
}

/// Stop, disable and delete the unit.
pub async fn remove_service() -> Result<()> {
    if !Path::new(&unit_path()).exists() {
        return Ok(());
    }
    stop_service().await?;
    let connection = connect().await?;
    let manager = manager(&connection).await?;
    manager
        .disable_unit_files(&[&unit_name()], false)
        .await
        .context("can't disable the service")?;
    std::fs::remove_file(unit_path()).with_context(|| format!("can't delete {}", unit_path()))?;
    manager.reload().await?;
    Ok(())
}

pub async fn service_state() -> ServiceState {
    if !Path::new(&unit_path()).exists() {
        return ServiceState::NotInstalled;
    }
    let state = match connect().await {
        Ok(connection) => state_of(&connection).await,
        Err(e) => Err(e),
    };
    match state {
        Ok(state) if state == "active" => ServiceState::Running,
        Ok(state) if state == "inactive" || state == "failed" => ServiceState::Stopped(state),
        Ok(state) => ServiceState::Other(state),
        Err(e) => ServiceState::Other(format!("{e:#}")),
    }
}

/// Delete the folder. With `DynamicUser`, systemd keeps it in
/// `/var/lib/private/` behind a symlink: both go.
pub fn delete_folder(dir: &Path) -> Result<()> {
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if real != dir {
        std::fs::remove_dir_all(&real)
            .with_context(|| format!("can't delete {}", real.display()))?;
        std::fs::remove_file(dir).with_context(|| format!("can't delete {}", dir.display()))?;
    } else if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("can't delete {}", dir.display()))?;
    }
    Ok(())
}

/// Folder permissions `probe doctor` accepts: nobody but the owner.
pub fn private(dir: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    Ok(std::fs::metadata(dir)?.permissions().mode() & 0o077 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_runs_the_probe_as_a_throwaway_user_in_its_folder() {
        let layout = Layout {
            dir: "/var/lib/blentinel-probe".into(),
            exe: "/var/lib/blentinel-probe/probe".into(),
        };
        let unit = systemd_unit(&layout);
        let settings: Vec<&str> = unit
            .lines()
            .filter(|l| !l.starts_with('#') && l.contains('='))
            .collect();
        for expected in [
            "ExecStart=/var/lib/blentinel-probe/probe",
            "DynamicUser=yes",
            "StateDirectory=blentinel-probe",
            "StateDirectoryMode=0700",
            "AmbientCapabilities=CAP_NET_RAW",
        ] {
            assert!(settings.contains(&expected), "{expected}:\n{unit}");
        }
        // systemd refuses a comment after a value (D-34).
        assert!(settings.iter().all(|l| !l.contains(" #")), "{unit}");
    }
}
