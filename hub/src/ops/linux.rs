//! Linux: systemd, the service user and file ownership, through the system's
//! own tools (`systemctl`, `useradd`, `chown`). Only the probe must avoid
//! external tools (FP-2); the hub may use them.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::ServiceState;
use super::layout::{Layout, SERVICE_NAME, SERVICE_USER, systemd_unit};

/// Where the unit file goes.
fn unit_path() -> String {
    format!("/etc/systemd/system/{SERVICE_NAME}.service")
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

/// Run a system tool; its stderr becomes the error.
fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
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
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Create the service user if it doesn't exist.
pub fn ensure_service_user() -> Result<()> {
    if run("id", &["-u", SERVICE_USER]).is_err() {
        run(
            "useradd",
            &[
                "--system",
                "--no-create-home",
                "--shell",
                "/usr/sbin/nologin",
                SERVICE_USER,
            ],
        )?;
    }
    Ok(())
}

/// The state folder: owned by the service user, nobody else may enter.
pub fn secure_state_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let owner = format!("{SERVICE_USER}:{SERVICE_USER}");
    run("chown", &["-R", &owner, &dir.to_string_lossy()])?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// The config: root writes it, the service reads it, others can't (it may
/// hold the SMTP password).
pub fn secure_config(path: &Path) -> Result<()> {
    run(
        "chown",
        &[&format!("root:{SERVICE_USER}"), &path.to_string_lossy()],
    )?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))?;
    Ok(())
}

/// Write the unit, enable it, and (re)start it if `start`.
pub fn install_service(layout: &Layout, start: bool) -> Result<()> {
    std::fs::write(unit_path(), systemd_unit(layout))
        .with_context(|| format!("can't write {}", unit_path()))?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", SERVICE_NAME])?;
    if start {
        run("systemctl", &["restart", SERVICE_NAME])?;
    }
    Ok(())
}

pub fn stop_service() -> Result<()> {
    if service_state() != ServiceState::NotInstalled {
        run("systemctl", &["stop", SERVICE_NAME])?;
    }
    Ok(())
}

/// Stop, disable and delete the unit.
pub fn remove_service() -> Result<()> {
    if service_state() == ServiceState::NotInstalled {
        return Ok(());
    }
    let _ = run("systemctl", &["disable", "--now", SERVICE_NAME]);
    std::fs::remove_file(unit_path()).with_context(|| format!("can't delete {}", unit_path()))?;
    run("systemctl", &["daemon-reload"])?;
    Ok(())
}

pub fn service_state() -> ServiceState {
    if !Path::new(&unit_path()).exists() {
        return ServiceState::NotInstalled;
    }
    // `is-active` exits non-zero when not active; its output still says why.
    let output = Command::new("systemctl")
        .args(["is-active", SERVICE_NAME])
        .output();
    match output.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()) {
        Ok(state) if state == "active" => ServiceState::Running,
        Ok(state) if state == "inactive" || state == "failed" => ServiceState::Stopped(state),
        Ok(state) => ServiceState::Other(state),
        Err(e) => ServiceState::Other(e.to_string()),
    }
}

/// Folder permissions `hub doctor` accepts: nobody but the owner.
pub fn private_mode(path: &Path) -> Result<bool> {
    let mode = std::fs::metadata(path)?.permissions().mode();
    Ok(mode & 0o077 == 0)
}
