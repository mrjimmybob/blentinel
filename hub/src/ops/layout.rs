//! Where an installed hub keeps its files, per operating system (D-52).
//!
//! | | Linux | Windows |
//! |---|---|---|
//! | program | `/opt/blentinel/hub/hub` | `C:\Program Files\Blentinel\Hub\hub.exe` |
//! | config | `/etc/blentinel/blentinel_hub.toml` | `C:\ProgramData\Blentinel\Hub\blentinel_hub.toml` |
//! | state | `/var/lib/blentinel` | `C:\ProgramData\Blentinel\Hub` |
//! | service | systemd unit `blentinel-hub`, user `blentinel` | Windows service `BlentinelHub` |

use std::path::{Path, PathBuf};

/// The files of an installed hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub program_dir: PathBuf,
    pub exe: PathBuf,
    pub config: PathBuf,
    pub state_dir: PathBuf,
}

/// The name the service is registered under.
#[cfg(windows)]
pub const SERVICE_NAME: &str = "BlentinelHub";
#[cfg(not(windows))]
pub const SERVICE_NAME: &str = "blentinel-hub";

/// The system user the Linux service runs as (no login, no home).
pub const SERVICE_USER: &str = "blentinel";

/// The config file's name, wherever it is.
pub const CONFIG_FILE: &str = "blentinel_hub.toml";

impl Layout {
    /// This operating system's layout.
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            let program_dir = PathBuf::from(r"C:\Program Files\Blentinel\Hub");
            let state_dir = PathBuf::from(r"C:\ProgramData\Blentinel\Hub");
            Self {
                exe: program_dir.join("hub.exe"),
                config: state_dir.join(CONFIG_FILE),
                program_dir,
                state_dir,
            }
        }
        #[cfg(not(windows))]
        {
            let program_dir = PathBuf::from("/opt/blentinel/hub");
            Self {
                exe: program_dir.join("hub"),
                config: PathBuf::from("/etc/blentinel").join(CONFIG_FILE),
                program_dir,
                state_dir: PathBuf::from("/var/lib/blentinel"),
            }
        }
    }
}

/// The config a command uses without `--config`: `blentinel_hub.toml` in the
/// working directory if there is one, else the installed hub's, else the
/// working directory's (for the "not found" message).
pub fn default_config_path(working_dir: &Path, installed: &Path) -> PathBuf {
    let local = working_dir.join(CONFIG_FILE);
    if !local.exists() && installed.exists() {
        installed.to_path_buf()
    } else {
        PathBuf::from(CONFIG_FILE)
    }
}

/// The systemd unit of the Linux service.
pub fn systemd_unit(layout: &Layout) -> String {
    format!(
        "# Written by `hub install` (Blentinel, D-52). Re-run it to update.\n\
         [Unit]\n\
         Description=Blentinel Hub\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} --config {config}\n\
         WorkingDirectory={state}\n\
         User={SERVICE_USER}\n\
         Group={SERVICE_USER}\n\
         Restart=always\n\
         RestartSec=5\n\
         # Listen on ports below 1024 (e.g. 443) without running as root.\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        exe = layout.exe.display(),
        config = layout.config.display(),
        state = layout.state_dir.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_layout_keeps_state_and_program_apart() {
        let layout = Layout::system();
        assert!(layout.exe.starts_with(&layout.program_dir));
        assert!(!layout.state_dir.starts_with(&layout.program_dir));
        assert!(layout.config.ends_with(CONFIG_FILE));
    }

    #[test]
    fn the_working_directory_config_wins_over_the_installed_one() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("etc").join(CONFIG_FILE);
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();

        // Nothing anywhere: the working directory's, to say it's missing.
        assert_eq!(
            default_config_path(&work, &installed),
            PathBuf::from(CONFIG_FILE)
        );
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "").unwrap();
        assert_eq!(default_config_path(&work, &installed), installed);
        std::fs::write(work.join(CONFIG_FILE), "").unwrap();
        assert_eq!(
            default_config_path(&work, &installed),
            PathBuf::from(CONFIG_FILE)
        );
    }

    #[test]
    fn the_unit_runs_the_installed_binary_with_its_config_as_the_service_user() {
        let layout = Layout {
            program_dir: "/opt/blentinel/hub".into(),
            exe: "/opt/blentinel/hub/hub".into(),
            config: "/etc/blentinel/blentinel_hub.toml".into(),
            state_dir: "/var/lib/blentinel".into(),
        };
        let unit = systemd_unit(&layout);
        let settings: Vec<&str> = unit
            .lines()
            .filter(|l| !l.starts_with('#') && l.contains('='))
            .collect();
        assert!(settings.contains(
            &"ExecStart=/opt/blentinel/hub/hub --config /etc/blentinel/blentinel_hub.toml"
        ));
        assert!(settings.contains(&"User=blentinel"));
        assert!(settings.contains(&"WorkingDirectory=/var/lib/blentinel"));
        // systemd refuses a comment after a value (D-34).
        assert!(settings.iter().all(|l| !l.contains(" #")), "{unit}");
    }
}
