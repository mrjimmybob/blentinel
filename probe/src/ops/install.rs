//! `probe install` and `probe uninstall` (D-53).
//!
//! Install is idempotent: the first run sets the probe up (and enrolls it
//! with `--code`), later runs upgrade it; a new `--code` enrolls it again in
//! place (another hub, company or site).

use std::path::Path;

use anyhow::{Context, Result};

use super::{Layout, SERVICE_NAME, needs_admin, platform};
use crate::{enrollment, identity};

pub async fn install(code: Option<&str>) -> Result<()> {
    if !platform::is_admin() {
        return Err(needs_admin("install"));
    }
    let layout = Layout::system();

    platform::stop_service().await?;
    platform::secure_folder(&layout.dir)?;
    copy_self(&layout.exe)?;

    let enrolled = match code {
        Some(code) => {
            let (key, _) = identity::load_or_create_key_in(&layout.dir)?;
            Some(enrollment::enroll(code, &key, &layout.dir).await?)
        }
        None => enrollment::load(&layout.dir)?,
    };
    // Files written just now (as root) must be the service's.
    platform::secure_folder(&layout.dir)?;
    platform::install_service(&layout, true).await?;

    println!(
        "Blentinel probe {} installed in {}",
        env!("CARGO_PKG_VERSION"),
        layout.dir.display()
    );
    println!("  Service: {SERVICE_NAME} (starts with the machine, restarts on failure)");
    match enrolled {
        Some(e) => {
            println!(
                "  Reports to {} as \"{}\" ({}, {})",
                e.hub_url, e.probe_name, e.company_name, e.site_name
            );
            println!();
            println!(
                "Give it its checks on the hub: open the probe's page (its name on the company's page)."
            );
        }
        None => {
            println!();
            println!("Not enrolled yet: it waits. Enroll it with a code from the hub's Add probe:");
            println!("  probe install --code <code>");
        }
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
    // Copy beside it, then rename: a running old binary is never half-written.
    let staged = target.with_extension("new");
    std::fs::copy(&current, &staged)
        .with_context(|| format!("can't copy the probe to {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&staged, target)
        .with_context(|| format!("can't replace {}", target.display()))?;
    Ok(())
}

/// `probe uninstall`: the service and the program go; the identity,
/// enrollment, checks and buffer stay unless `delete_data`, which removes
/// the whole folder (FP-2: nothing left behind).
pub async fn uninstall(delete_data: bool) -> Result<()> {
    if !platform::is_admin() {
        return Err(needs_admin("uninstall"));
    }
    let layout = Layout::system();
    platform::remove_service().await?;
    println!("Service {SERVICE_NAME} removed.");
    let result = if delete_data {
        platform::delete_folder(&layout.dir).map(|()| {
            println!(
                "Deleted {} (identity, enrollment, checks, buffer).",
                layout.dir.display()
            );
        })
    } else if layout.exe.exists() {
        std::fs::remove_file(&layout.exe).map_err(Into::into).map(|()| {
            println!(
                "Deleted the program; kept {} (its identity: installing again keeps the same probe). \
                 `probe uninstall --delete-data` removes it.",
                layout.dir.display()
            );
        })
    } else {
        Ok(())
    };
    match result {
        // Windows can't delete the running program.
        Err(e) if cfg!(windows) => println!(
            "Couldn't delete everything in {} ({e}); delete it by hand once this command has ended.",
            layout.dir.display()
        ),
        other => other?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_itself_replaces_the_target_whole() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("folder").join("probe-copy");
        copy_self(&target).unwrap();
        assert_eq!(
            std::fs::metadata(&target).unwrap().len(),
            std::fs::metadata(std::env::current_exe().unwrap())
                .unwrap()
                .len()
        );
        copy_self(&target).unwrap();
        assert!(!target.with_extension("new").exists());
    }
}
