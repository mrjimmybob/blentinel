//! Running the probe on its machine (M2-09, proposal 0004 §5.3, D-53):
//! `probe install`, `uninstall`, `status` and `doctor`.
//!
//! The probe keeps everything in one folder (FP-2), the binary included:
//!
//! | | Linux | Windows |
//! |---|---|---|
//! | folder | `/var/lib/blentinel-probe` | `C:\Program Files\Blentinel\Probe` |
//! | service | systemd `blentinel-probe`, as its own user `blentinel-probe` | Windows service `BlentinelProbe` (LocalSystem) |
//! | managed with | D-Bus (`zbus`) | the Service Control Manager |
//!
//! Natively, but for the Linux user: `useradd` at install and `userdel` at
//! uninstall, which every systemd machine has (FP-2 as of D-84: nothing to
//! install; what the system always has may be used).

pub mod doctor;
pub mod install;
pub mod status;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::{running_services, unit_states};

use std::path::PathBuf;

/// Where an installed probe lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

/// The name the service is registered under.
#[cfg(windows)]
pub const SERVICE_NAME: &str = crate::service::SERVICE_NAME;
#[cfg(not(windows))]
pub const SERVICE_NAME: &str = "blentinel-probe";

impl Layout {
    /// This operating system's layout.
    pub fn system() -> Self {
        #[cfg(windows)]
        let dir = PathBuf::from(r"C:\Program Files\Blentinel\Probe");
        #[cfg(not(windows))]
        let dir = PathBuf::from("/var/lib/blentinel-probe");
        let exe = dir.join(if cfg!(windows) { "probe.exe" } else { "probe" });
        Self { dir, exe }
    }
}

/// The folder `status` and `doctor` look at: the installed probe's if there
/// is one, else this binary's own.
pub fn probe_dir() -> PathBuf {
    let installed = Layout::system();
    if installed.exe.exists() || installed.dir.join(crate::identity::KEY_FILE).exists() {
        installed.dir
    } else {
        crate::config::get_base_dir()
    }
}

/// The service manager and folder permissions.
pub(crate) mod platform {
    #[cfg(target_os = "linux")]
    pub use super::linux::{
        delete_folder, install_service, is_admin, private, remove_service, secure_folder,
        service_state, stop_service,
    };

    #[cfg(windows)]
    mod windows {
        use super::super::Layout;
        use anyhow::Result;
        use common::service::ServiceState;
        use common::service::windows as win;
        use std::ffi::OsString;
        use std::path::Path;

        pub use win::is_admin;

        const NAME: &str = super::super::SERVICE_NAME;

        pub fn secure_folder(dir: &Path) -> Result<()> {
            Ok(win::restrict_to_admins(dir)?)
        }
        pub async fn install_service(layout: &Layout, start: bool) -> Result<()> {
            let spec = win::ServiceSpec {
                name: NAME,
                display_name: "Blentinel Probe",
                description: "Blentinel network probe",
                exe: layout.exe.clone(),
                args: Vec::<OsString>::new(),
            };
            Ok(win::install(&spec, start)?)
        }
        pub async fn stop_service() -> Result<()> {
            Ok(win::stop(NAME)?)
        }
        pub async fn remove_service() -> Result<()> {
            Ok(win::remove(NAME)?)
        }
        pub async fn service_state() -> ServiceState {
            win::state(NAME)
        }
        pub fn delete_folder(dir: &Path) -> Result<()> {
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
            Ok(())
        }
        pub fn private(dir: &Path) -> Result<bool> {
            Ok(win::admins_only(&win::dacl_sddl(dir)?))
        }
    }
    #[cfg(windows)]
    pub use windows::*;

    #[cfg(not(any(target_os = "linux", windows)))]
    mod unsupported {
        use super::super::Layout;
        use anyhow::{Result, bail};
        use common::service::ServiceState;
        use std::path::Path;
        const WHY: &str = "`probe install` supports Linux (systemd) and Windows";
        pub fn is_admin() -> bool {
            false
        }
        pub fn secure_folder(_: &Path) -> Result<()> {
            bail!(WHY)
        }
        pub async fn install_service(_: &Layout, _: bool) -> Result<()> {
            bail!(WHY)
        }
        pub async fn stop_service() -> Result<()> {
            Ok(())
        }
        pub async fn remove_service() -> Result<()> {
            Ok(())
        }
        pub async fn service_state() -> ServiceState {
            ServiceState::NotInstalled
        }
        pub fn delete_folder(dir: &Path) -> Result<()> {
            Ok(std::fs::remove_dir_all(dir)?)
        }
        pub fn private(dir: &Path) -> Result<bool> {
            use std::os::unix::fs::PermissionsExt;
            Ok(std::fs::metadata(dir)?.permissions().mode() & 0o077 == 0)
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    pub use unsupported::*;
}

/// The message for a command that needs root or Administrator.
pub(crate) fn needs_admin(command: &str) -> anyhow::Error {
    if cfg!(windows) {
        anyhow::anyhow!("`probe {command}` must run in an elevated (Administrator) terminal")
    } else {
        anyhow::anyhow!("`probe {command}` must run as root: sudo ./probe {command} …")
    }
}

/// Server administration commands (see `args`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Install {
        code: Option<String>,
    },
    Uninstall {
        delete_data: bool,
    },
    Status,
    Doctor,
    /// `probe updates on|off`: the local emergency brake (D-55).
    Updates {
        on: bool,
    },
}

/// `probe updates on|off`, for the installed probe (or this one).
fn set_updates(on: bool) -> anyhow::Result<()> {
    let dir = probe_dir();
    crate::update::state::Files::new(&dir)
        .set_brake(!on)
        .map_err(|e| e.context("run it as root / Administrator for an installed probe"))?;
    if on {
        println!("Updates are on: the probe installs the version the hub approves.");
    } else {
        println!(
            "Updates are off in {}: the probe refuses what the hub offers (the hub shows why). \
             `probe updates on` undoes it.",
            dir.display()
        );
    }
    Ok(())
}

/// Run a command; the process's exit code.
pub async fn run(command: &Command) -> i32 {
    let result: anyhow::Result<i32> = match command {
        Command::Install { code } => install::install(code.as_deref()).await.map(|()| 0),
        Command::Uninstall { delete_data } => install::uninstall(*delete_data).await.map(|()| 0),
        Command::Status => {
            print!("{}", status::status(&probe_dir()).await);
            Ok(0)
        }
        Command::Doctor => {
            let checks = doctor::run(&probe_dir()).await;
            let (report, worst) = doctor::report(&checks);
            print!("{report}");
            Ok(i32::from(worst == doctor::Level::Fail))
        }
        Command::Updates { on } => set_updates(*on).map(|()| 0),
    };
    result.unwrap_or_else(|e| {
        eprintln!("[ERROR] {e:#}");
        1
    })
}
