//! `hub install` and `hub uninstall` (D-52).
//!
//! Install is idempotent: the first run sets the hub up, later runs upgrade
//! it (a new binary; the config and data are kept).

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use super::layout::{Layout, SERVICE_NAME};
use super::{answers, needs_admin, platform, setup_code, web_url};
use crate::config::{self, InstallSettings};

/// `hub install` options: where a first install listens (ignored when the
/// config exists), and whether to register a service.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstallOptions {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub https_port: Option<u16>,
    /// Files only, no service (e.g. a container's entry point runs the hub).
    pub no_service: bool,
}

/// How long `hub install` waits for the hub to answer after starting it.
const START_TIMEOUT: Duration = Duration::from_secs(20);

/// The settings of a first install: all interfaces (probes on other machines
/// must reach it), the usual ports, the layout's state folder.
pub fn first_install_settings(options: &InstallOptions, layout: &Layout) -> InstallSettings {
    InstallSettings {
        host: options.host.clone().unwrap_or_else(|| "0.0.0.0".into()),
        port: options.port.unwrap_or(3000),
        https_port: options.https_port.unwrap_or(3443),
        state_dir: layout.state_dir.clone(),
    }
}

pub fn install(options: &InstallOptions) -> Result<()> {
    if !platform::is_admin() {
        return Err(needs_admin("install"));
    }
    let layout = Layout::system();

    // The binary: stop the running hub so its file can be replaced.
    platform::stop_service()?;
    copy_self(&layout.exe)?;
    platform::ensure_service_user()?;

    // The config, on a first install only.
    let first_install = !layout.config.exists();
    if first_install {
        let parent = layout.config.parent().context("the config has a folder")?;
        std::fs::create_dir_all(parent)?;
        let settings = first_install_settings(options, &layout);
        std::fs::write(&layout.config, config::install_config(&settings))
            .with_context(|| format!("can't write {}", layout.config.display()))?;
    } else if options.host.is_some() || options.port.is_some() || options.https_port.is_some() {
        println!(
            "Keeping the existing {}: edit it to change where the hub listens.",
            layout.config.display()
        );
    }
    let hub_config = config::load_from(&layout.config)
        .with_context(|| format!("{} is invalid", layout.config.display()))?;
    platform::secure_state_dir(&hub_config.server.state_dir)?;
    platform::secure_config(&layout.config)?;

    println!(
        "Blentinel hub {} installed in {}",
        env!("CARGO_PKG_VERSION"),
        layout.program_dir.display()
    );
    println!("  Config: {}", layout.config.display());
    println!("  Data:   {}", hub_config.server.state_dir.display());
    if options.no_service {
        println!("  No service registered (--no-service). Start it with:");
        println!(
            "    {} --config {}",
            layout.exe.display(),
            layout.config.display()
        );
        return Ok(());
    }
    platform::install_service(&layout, true)?;
    println!("  Service: {SERVICE_NAME} (starts with the machine, restarts on failure)");

    wait_until_listening(&hub_config);
    println!();
    println!("Open {}", web_url(&hub_config));
    match setup_code(&hub_config.server.state_dir) {
        Some(code) => println!("and create the first Owner with the setup code: {code}"),
        None if first_install => {
            println!(
                "The setup code isn't there yet; `hub status` shows it once the hub has started."
            )
        }
        None => {}
    }
    if hub_config.server.tls.enabled {
        println!(
            "(The certificate is self-signed: the browser warns once. Its fingerprint is in the hub's log.)"
        );
    }
    Ok(())
}

/// Put this executable at `target` (unless it is already running from there).
fn copy_self(target: &Path) -> Result<()> {
    let current = std::env::current_exe()?;
    let same = match (current.canonicalize(), target.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok(());
    }
    let dir = target.parent().context("the program has a folder")?;
    std::fs::create_dir_all(dir)?;
    // Copy next to it, then rename: a running old binary is never half-written.
    let staged = target.with_extension("new");
    std::fs::copy(&current, &staged)
        .with_context(|| format!("can't copy the hub to {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&staged, target)
        .with_context(|| format!("can't replace {}", target.display()))?;
    Ok(())
}

/// Wait until the hub answers (or give up quietly: `hub status` says more).
fn wait_until_listening(hub_config: &crate::config::HubConfig) {
    let deadline = std::time::Instant::now() + START_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if answers(&hub_config.server.host, hub_config.server.port) {
            // Give it a moment to write the setup code after binding.
            std::thread::sleep(Duration::from_millis(500));
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    println!(
        "The hub didn't answer within {} s: run `hub doctor`.",
        START_TIMEOUT.as_secs()
    );
}

/// `hub uninstall`: the service and the program go; the config and data
/// stay unless `delete_data` (D-52).
pub fn uninstall(delete_data: bool) -> Result<()> {
    if !platform::is_admin() {
        return Err(needs_admin("uninstall"));
    }
    let layout = Layout::system();
    let state_dir = config::load_from(&layout.config)
        .map(|c| c.server.state_dir)
        .unwrap_or_else(|_| layout.state_dir.clone());

    platform::remove_service()?;
    println!("Service {SERVICE_NAME} removed.");
    remove_program(&layout)?;

    if delete_data {
        for path in [&state_dir, &layout.config] {
            if path.is_dir() {
                std::fs::remove_dir_all(path)
                    .with_context(|| format!("can't delete {}", path.display()))?;
            } else if path.exists() {
                std::fs::remove_file(path)
                    .with_context(|| format!("can't delete {}", path.display()))?;
            }
        }
        println!("Deleted the data ({}) and the config.", state_dir.display());
    } else {
        println!(
            "Kept the data ({}) and the config ({}); `hub uninstall --delete-data` removes them.",
            state_dir.display(),
            layout.config.display()
        );
    }
    Ok(())
}

/// Delete the program folder. On Windows a running program can't delete
/// its own file: then it says so.
fn remove_program(layout: &Layout) -> Result<()> {
    if !layout.program_dir.exists() {
        return Ok(());
    }
    match std::fs::remove_dir_all(&layout.program_dir) {
        Ok(()) => println!("Deleted {}.", layout.program_dir.display()),
        Err(e) if cfg!(windows) => println!(
            "Couldn't delete {} ({e}); delete it by hand once this command has ended.",
            layout.program_dir.display()
        ),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("can't delete {}", layout.program_dir.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_install_listens_everywhere_on_the_usual_ports() {
        let layout = Layout::system();
        let settings = first_install_settings(&InstallOptions::default(), &layout);
        assert_eq!(
            (settings.host.as_str(), settings.port, settings.https_port),
            ("0.0.0.0", 3000, 3443)
        );
        assert_eq!(settings.state_dir, layout.state_dir);

        let chosen = InstallOptions {
            host: Some("10.0.0.5".into()),
            port: Some(80),
            https_port: Some(443),
            no_service: false,
        };
        let settings = first_install_settings(&chosen, &layout);
        assert_eq!(
            (settings.host.as_str(), settings.port, settings.https_port),
            ("10.0.0.5", 80, 443)
        );
    }

    #[test]
    fn copying_itself_replaces_the_target_whole() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("bin").join("hub-copy");
        copy_self(&target).unwrap();
        let size = std::fs::metadata(&target).unwrap().len();
        assert_eq!(
            size,
            std::fs::metadata(std::env::current_exe().unwrap())
                .unwrap()
                .len()
        );
        copy_self(&target).unwrap(); // again: replaced, no leftovers
        assert!(!target.with_extension("new").exists());
    }
}
