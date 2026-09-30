//! Windows: the `BlentinelHub` service and the data folder's permissions,
//! through `common::service::windows` (shared with the probe, D-53).

use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use common::service::windows as win;

use super::ServiceState;
use super::layout::{Layout, SERVICE_NAME};

pub use win::{admins_only, dacl_sddl, is_admin};

/// The data folder: SYSTEM and Administrators only.
pub fn secure_state_dir(dir: &Path) -> Result<()> {
    Ok(win::restrict_to_admins(dir)?)
}

/// Create or update the service, and start it if `start`.
pub fn install_service(layout: &Layout, start: bool) -> Result<()> {
    let spec = win::ServiceSpec {
        name: SERVICE_NAME,
        display_name: "Blentinel Hub",
        description: "Blentinel central monitoring hub",
        exe: layout.exe.clone(),
        args: vec![OsString::from("--config"), layout.config.clone().into()],
    };
    Ok(win::install(&spec, start)?)
}

pub fn stop_service() -> Result<()> {
    Ok(win::stop(SERVICE_NAME)?)
}

pub fn remove_service() -> Result<()> {
    Ok(win::remove(SERVICE_NAME)?)
}

pub fn service_state() -> ServiceState {
    win::state(SERVICE_NAME)
}
