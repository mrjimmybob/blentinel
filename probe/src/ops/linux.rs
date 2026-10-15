//! Linux: the probe's systemd service, managed over D-Bus with `zbus` (pure
//! Rust, D-53).
//!
//! The unit runs the probe as its own system user, [`SERVICE_USER`], with its
//! folder as `StateDirectory=`: systemd keeps the folder owned by that user
//! and closed to others (`0700`). `install` makes the user and `uninstall`
//! removes it, with the system's `useradd` and `userdel` (D-84): the only
//! tools the probe runs, and every systemd machine has them.

use std::path::{Path, PathBuf};
use std::process::Command;
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
    /// Every unit systemd has loaded: name, description, load state, active
    /// state, sub-state, and what the inventory doesn't use.
    #[allow(clippy::type_complexity)]
    fn list_units(
        &self,
    ) -> zbus::Result<
        Vec<(
            String,
            String,
            String,
            String,
            String,
            String,
            OwnedObjectPath,
            u32,
            String,
            OwnedObjectPath,
        )>,
    >;
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
    #[zbus(property)]
    fn sub_state(&self) -> zbus::Result<String>;
}

/// What systemd says of a unit: `LoadState` (`loaded`, `not-found`, …),
/// `ActiveState` (`active`, `failed`, …) and `SubState` (`running`, `dead`).
pub struct UnitStates {
    pub load: String,
    pub active: String,
    pub sub: String,
}

/// Ask systemd about any unit, by its full name (`nginx.service`): only
/// reads (the `service` check, D-83). `None`: this process isn't let onto
/// the system bus. That is a probe installed before D-84 on Debian and
/// Ubuntu: their bus daemon (dbus-daemon) turns away the throwaway users of
/// `DynamicUser` units, which the probe's was.
pub async fn unit_states(unit: &str) -> Result<Option<UnitStates>> {
    let Ok(connection) = zbus::Connection::system().await else {
        return Ok(None);
    };
    let path = manager(&connection).await?.load_unit(unit).await?;
    let unit = UnitProxy::builder(&connection).path(path)?.build().await?;
    Ok(Some(UnitStates {
        load: unit.load_state().await?,
        active: unit.active_state().await?,
        sub: unit.sub_state().await?,
    }))
}

/// The services running now, each with systemd's description of it (the
/// probe's inventory, D-85). `None`: this process isn't let onto the system
/// bus, as for [`unit_states`].
pub async fn running_services() -> Result<Option<Vec<(String, String)>>> {
    let Ok(connection) = zbus::Connection::system().await else {
        return Ok(None);
    };
    let units = manager(&connection).await?.list_units().await?;
    Ok(Some(
        units
            .into_iter()
            .filter(|unit| {
                unit.0.ends_with(".service") && unit.3 == "active" && unit.4 == "running"
            })
            .map(|unit| (unit.0, unit.1))
            .collect(),
    ))
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

/// The account the service runs as (D-84). A user of the system's own list,
/// not a `DynamicUser`: Debian's and Ubuntu's bus daemon turns those away,
/// and the `service` check asks systemd over the bus.
pub const SERVICE_USER: &str = "blentinel-probe";

/// Where the system's own tools are. Looked up here, not on `PATH`: after a
/// plain `su`, Debian's root has no `/usr/sbin` on it.
const TOOL_FOLDERS: [&str; 4] = ["/usr/sbin", "/sbin", "/usr/bin", "/bin"];

fn tool(name: &str) -> PathBuf {
    TOOL_FOLDERS
        .iter()
        .map(|folder| Path::new(folder).join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Run a system tool; its stderr becomes the error.
fn run(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(tool(program))
        .args(args)
        .output()
        .with_context(|| format!("can't run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// The user and group numbers of `name` in this text of `/etc/passwd`.
fn ids(passwd: &str, name: &str) -> Option<(u32, u32)> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next() != Some(name) {
            return None;
        }
        let uid = fields.nth(1)?.parse().ok()?;
        Some((uid, fields.next()?.parse().ok()?))
    })
}

fn service_user() -> Option<(u32, u32)> {
    ids(&std::fs::read_to_string("/etc/passwd").ok()?, SERVICE_USER)
}

/// Make the service's user if it isn't there: no password, no login, no
/// home of its own. Its user and group numbers.
fn ensure_service_user(dir: &Path) -> Result<(u32, u32)> {
    if let Some(ids) = service_user() {
        return Ok(ids);
    }
    let shell = ["/usr/sbin/nologin", "/sbin/nologin"]
        .into_iter()
        .find(|shell| Path::new(shell).exists())
        .unwrap_or("/bin/false");
    run(
        "useradd",
        &[
            "--system",
            "--user-group",
            "--no-create-home",
            "--home-dir",
            &dir.to_string_lossy(),
            "--shell",
            shell,
            SERVICE_USER,
        ],
    )
    .with_context(|| format!("can't make the user {SERVICE_USER} the service runs as"))?;
    service_user().with_context(|| format!("useradd made no user {SERVICE_USER} in /etc/passwd"))
}

/// Give a folder and all in it to a user. Links are not followed.
fn give(path: &Path, (uid, gid): (u32, u32)) -> std::io::Result<()> {
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
    if path.symlink_metadata()?.is_dir() {
        for entry in std::fs::read_dir(path)? {
            give(&entry?.path(), (uid, gid))?;
        }
    }
    Ok(())
}

/// A probe installed before D-84 ran as a `DynamicUser`: systemd kept its
/// folder in `/var/lib/private/` behind a link. Put it where the link is.
fn out_of_private(dir: &Path) -> Result<()> {
    if !dir.is_symlink() {
        return Ok(());
    }
    let real = std::fs::canonicalize(dir)
        .with_context(|| format!("{} is a link to nowhere: delete it", dir.display()))?;
    std::fs::remove_file(dir)?;
    std::fs::rename(&real, dir)
        .with_context(|| format!("can't move {} to {}", real.display(), dir.display()))
}

/// Remove the service's user. What it owned in the probe's folder becomes
/// root's first: a kept folder (`uninstall` without `--delete-data`) must
/// not belong to a number the next new user of the machine could get.
fn remove_service_user(dir: &Path) -> Result<()> {
    if service_user().is_none() {
        return Ok(());
    }
    if let Ok(real) = std::fs::canonicalize(dir) {
        give(&real, (0, 0)).with_context(|| format!("can't give {} to root", real.display()))?;
    }
    run("userdel", &[SERVICE_USER])
}

/// The unit: the probe as its own user, in its own folder, allowed to ping.
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
         # Its own user (made by `probe install`); systemd keeps the folder\n\
         # that user's and private.\n\
         User={SERVICE_USER}\n\
         StateDirectory={folder}\n\
         StateDirectoryMode=0700\n\
         # It writes nowhere but its folder, and gains no rights.\n\
         ProtectSystem=strict\n\
         ProtectHome=read-only\n\
         PrivateTmp=yes\n\
         NoNewPrivileges=yes\n\
         RemoveIPC=yes\n\
         # Ping without root.\n\
         AmbientCapabilities=CAP_NET_RAW\n\
         CapabilityBoundingSet=CAP_NET_RAW\n\
         # Also starts the new version after a self-update (exit 75, D-55).\n\
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

/// The folder and all in it: the service user's (made here if it isn't
/// there yet), closed to everyone else. Also for what root wrote into it
/// since (the program, a new enrollment), and for a folder kept by an
/// earlier uninstall or left by an install from before D-84.
pub fn secure_folder(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let user = ensure_service_user(dir)?;
    out_of_private(dir)?;
    std::fs::create_dir_all(dir)?;
    give(dir, user).with_context(|| format!("can't give {} to {SERVICE_USER}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Write the unit, enable it, and (re)start it if `start`.
pub async fn install_service(layout: &Layout, start: bool) -> Result<()> {
    ensure_service_user(&layout.dir)?;
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

/// Stop, disable and delete the unit, and remove its user.
pub async fn remove_service() -> Result<()> {
    if Path::new(&unit_path()).exists() {
        stop_service().await?;
        let connection = connect().await?;
        let manager = manager(&connection).await?;
        manager
            .disable_unit_files(&[&unit_name()], false)
            .await
            .context("can't disable the service")?;
        std::fs::remove_file(unit_path())
            .with_context(|| format!("can't delete {}", unit_path()))?;
        manager.reload().await?;
    }
    // Not worth stopping for: the rest of the uninstall still goes on.
    if let Err(e) = remove_service_user(&Layout::system().dir) {
        eprintln!(
            "[WARN] The user {SERVICE_USER} is still there ({e:#}). Remove it with: userdel {SERVICE_USER}"
        );
    }
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

/// Delete the folder. A probe installed before D-84 (a `DynamicUser`) has
/// it in `/var/lib/private/` behind a symlink: both go.
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
    fn the_unit_runs_the_probe_as_its_own_user_in_its_folder() {
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
            "User=blentinel-probe",
            "StateDirectory=blentinel-probe",
            "StateDirectoryMode=0700",
            "AmbientCapabilities=CAP_NET_RAW",
            // What `DynamicUser` gave without being asked (D-84).
            "ProtectSystem=strict",
            "NoNewPrivileges=yes",
        ] {
            assert!(settings.contains(&expected), "{expected}:\n{unit}");
        }
        // The bus of Debian and Ubuntu turns a throwaway user away (D-84).
        assert!(!unit.contains("DynamicUser"), "{unit}");
        // systemd refuses a comment after a value (D-34).
        assert!(settings.iter().all(|l| !l.contains(" #")), "{unit}");
    }

    #[test]
    fn a_user_is_found_by_its_whole_name() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\n\
                      blentinel-probe-old:x:997:996::/nonexistent:/usr/sbin/nologin\n";
        assert_eq!(ids(passwd, "root"), Some((0, 0)));
        assert_eq!(ids(passwd, "blentinel-probe"), None);
        assert_eq!(
            ids(
                &format!(
                    "{passwd}blentinel-probe:x:999:983::/var/lib/blentinel-probe:/bin/false\n"
                ),
                "blentinel-probe"
            ),
            Some((999, 983))
        );
    }

    /// A folder given to its own owner: every file and folder is walked, and
    /// a link in it isn't followed (its target isn't there).
    #[test]
    fn a_folder_is_given_whole_and_links_are_left_alone() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("a").join("b");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("file"), "x").unwrap();
        std::os::unix::fs::symlink("/blentinel/no/such/place", inner.join("link")).unwrap();
        let meta = std::fs::metadata(dir.path()).unwrap();
        give(dir.path(), (meta.uid(), meta.gid())).unwrap();
    }

    /// The folder of an install from before D-84 comes out from behind its link.
    #[test]
    fn a_folder_behind_a_link_is_moved_to_where_the_link_was() {
        let dir = tempfile::tempdir().unwrap();
        let (real, link) = (dir.path().join("private"), dir.path().join("probe"));
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("identity.key"), "k").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        out_of_private(&link).unwrap();
        assert!(link.is_dir() && !link.is_symlink());
        assert!(link.join("identity.key").exists());
        assert!(!real.exists());
        // And a folder that is already in its place stays.
        out_of_private(&link).unwrap();
        assert!(link.join("identity.key").exists());
    }

    #[test]
    fn a_tool_is_looked_up_in_the_systems_folders() {
        // `sh` is on every Linux; a name nobody has is left to `PATH`.
        assert!(tool("sh").is_absolute());
        assert_eq!(
            tool("blentinel-no-such-tool"),
            PathBuf::from("blentinel-no-such-tool")
        );
    }
}
