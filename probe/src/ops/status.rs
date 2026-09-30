//! `probe status`: where the probe reports, what it runs, what it waits to send.

use std::fmt::Write as _;
use std::path::Path;

use super::platform;
use crate::{config, enrollment, identity, storage};

/// The status report, ready to print.
pub async fn status(dir: &Path) -> String {
    let mut out = String::new();
    let line = |out: &mut String, label: &str, value: String| {
        let _ = writeln!(out, "  {label:<10} {value}");
    };
    let _ = writeln!(out, "Blentinel probe {}", env!("CARGO_PKG_VERSION"));
    line(
        &mut out,
        "Service",
        platform::service_state().await.to_string(),
    );
    line(&mut out, "Folder", dir.display().to_string());
    let id = match identity::probe_id_in(dir) {
        Ok(Some(id)) => id,
        Ok(None) => "none yet (created at enrollment)".into(),
        Err(e) => format!("can't be read: {e:#} (run as root/Administrator)"),
    };
    line(&mut out, "Probe ID", id);
    match enrollment::load(dir) {
        Ok(Some(e)) => line(
            &mut out,
            "Hub",
            format!(
                "{} as \"{}\" ({}, {})",
                e.hub_url, e.probe_name, e.company_name, e.site_name
            ),
        ),
        Ok(None) => line(
            &mut out,
            "Hub",
            "not enrolled: probe install --code <code>".into(),
        ),
        Err(e) => line(&mut out, "Hub", format!("{e:#}")),
    }
    let active = config::ActiveConfig::load(dir);
    let checks = match &active.config {
        Some(c) => format!(
            "{} checks, every {} s (revision {})",
            c.checks.len(),
            c.interval_secs,
            c.revision
        ),
        None => "none yet: the hub sends them with the answer to a report".into(),
    };
    line(&mut out, "Checks", checks);
    let buffered = match storage::BlackBox::pending_at(&dir.join(storage::FILE_NAME)).await {
        Ok(Some(0)) | Ok(None) => "nothing waiting".into(),
        Ok(Some(n)) => format!("{n} reports waiting for the hub"),
        Err(e) => format!("can't be read: {e:#}"),
    };
    line(&mut out, "Buffer", buffered);
    out
}
