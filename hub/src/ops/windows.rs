//! Windows: the `BlentinelHub` service (Service Control Manager) and the data
//! folder's permissions (Win32 security API), without external tools.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions,
    ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState as WinState,
    ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, UNPROTECTED_DACL_SECURITY_INFORMATION,
};

use super::ServiceState;
use super::layout::{Layout, SERVICE_NAME};

/// Full control for SYSTEM and Administrators only, inherited by everything
/// inside, nothing inherited from above (`P`): other local users can't read
/// the key, the database or the config.
pub const ADMINS_ONLY_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Can this process create services (an elevated Administrator)?
pub fn is_admin() -> bool {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE).is_ok()
}

/// Give `dir` (and everything in it) [`ADMINS_ONLY_SDDL`].
pub fn secure_state_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    apply_sddl(dir, ADMINS_ONLY_SDDL)
}

/// Replace the DACL of `dir` with the one in `sddl`; inheritable entries
/// reach everything inside.
fn apply_sddl(dir: &Path, sddl: &str) -> Result<()> {
    let protected = if sddl.starts_with("D:P") {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    let sddl = wide(OsStr::new(sddl));
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated string and out-pointer; freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error())
            .context("can't build the folder's permissions");
    }
    let result = (|| {
        let (mut present, mut defaulted) = (0, 0);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: `descriptor` is the valid descriptor made above.
        if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            == 0
        {
            return Err(std::io::Error::last_os_error()).context("can't read the permissions");
        }
        let target = wide(dir.as_os_str());
        // SAFETY: valid path and DACL; owner, group and SACL are left alone.
        let rc = unsafe {
            SetNamedSecurityInfoW(
                target.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | protected,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null(),
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(rc as i32))
                .with_context(|| format!("can't restrict {}", dir.display()));
        }
        Ok(())
    })();
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(descriptor) };
    result
}

/// The folder's permissions as SDDL (`D:P(A;OICI;FA;;;SY)…`), for `hub doctor`.
pub fn dacl_sddl(path: &Path) -> Result<String> {
    let target = wide(path.as_os_str());
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: valid path and out-pointer; the descriptor is freed below.
    let rc = unsafe {
        GetNamedSecurityInfoW(
            target.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(rc as i32))
            .with_context(|| format!("can't read the permissions of {}", path.display()));
    }
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `descriptor` is valid; `text` is allocated by the call.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            std::ptr::null_mut(),
        )
    };
    let result = if ok == 0 {
        Err(std::io::Error::last_os_error()).context("can't read the permissions")
    } else {
        // SAFETY: a NUL-terminated wide string from the call above.
        let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
        let chars = unsafe { std::slice::from_raw_parts(text, len) };
        Ok(String::from_utf16_lossy(chars))
    };
    // SAFETY: both were allocated by the calls above (LocalFree accepts null).
    unsafe {
        LocalFree(text.cast());
        LocalFree(descriptor);
    }
    result
}

fn service_info(layout: &Layout) -> ServiceInfo {
    ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("Blentinel Hub"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: layout.exe.clone(),
        launch_arguments: vec![OsString::from("--config"), layout.config.clone().into()],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    }
}

/// Create or update the service (automatic start, restart on failure), and
/// start it if `start`.
pub fn install_service(layout: &Layout, start: bool) -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("can't open the Service Control Manager (run as Administrator)")?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::CHANGE_CONFIG;
    let info = service_info(layout);
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => {
            service
                .change_config(&info)
                .context("can't update the service")?;
            service
        }
        Err(_) => manager
            .create_service(&info, access)
            .context("can't create the service")?,
    };
    service
        .set_description("Blentinel central monitoring hub")
        .context("can't describe the service")?;
    let restart = ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(5),
    };
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart.clone(), restart.clone(), restart]),
        })
        .context("can't set the service's restart policy")?;
    if start && service.query_status()?.current_state != WinState::Running {
        service
            .start(&[] as &[&OsStr])
            .context("can't start the service")?;
    }
    Ok(())
}

fn open(access: ServiceAccess) -> Option<windows_service::service::Service> {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    manager.open_service(SERVICE_NAME, access).ok()
}

/// Stop the service and wait (up to 30 s) until it has.
pub fn stop_service() -> Result<()> {
    let Some(service) = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) else {
        return Ok(());
    };
    if service.query_status()?.current_state != WinState::Stopped {
        service.stop().context("can't stop the service")?;
    }
    for _ in 0..60 {
        if service.query_status()?.current_state == WinState::Stopped {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("the service didn't stop within 30 s")
}

/// Stop and delete the service.
pub fn remove_service() -> Result<()> {
    stop_service()?;
    if let Some(service) = open(ServiceAccess::DELETE) {
        service.delete().context("can't delete the service")?;
    }
    Ok(())
}

pub fn service_state() -> ServiceState {
    let Some(service) = open(ServiceAccess::QUERY_STATUS) else {
        return ServiceState::NotInstalled;
    };
    match service.query_status().map(|s| s.current_state) {
        Ok(WinState::Running) => ServiceState::Running,
        Ok(WinState::Stopped) => ServiceState::Stopped("stopped".into()),
        Ok(other) => ServiceState::Other(format!("{other:?}")),
        Err(e) => ServiceState::Other(e.to_string()),
    }
}

/// A DACL in SDDL (`D:PAI(A;OICI;FA;;;SY)…`): its flags (`PAI`) and its
/// entries (`A;OICI;FA;;;SY`, …).
fn parse_dacl(sddl: &str) -> Option<(&str, Vec<&str>)> {
    let rest = sddl.strip_prefix("D:")?;
    let flags_end = rest.find('(').unwrap_or(rest.len());
    let aces = rest[flags_end..]
        .split(['(', ')'])
        .filter(|ace| !ace.is_empty())
        .collect();
    Some((&rest[..flags_end], aces))
}

/// The entries all name SYSTEM or Administrators (the SID is the last field).
fn only_system_and_admins(aces: &[&str]) -> bool {
    aces.iter()
        .all(|ace| matches!(ace.rsplit(';').next(), Some("SY" | "BA")))
}

/// Whether an SDDL DACL allows only SYSTEM and Administrators and inherits
/// nothing from above (`P`).
pub fn admins_only(sddl: &str) -> bool {
    parse_dacl(sddl)
        .is_some_and(|(flags, aces)| flags.contains('P') && only_system_and_admins(&aces))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_folder_is_restricted_to_system_and_administrators() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("hub_identity.key"), [7u8; 32]).unwrap();
        assert!(
            !admins_only(&dacl_sddl(&state).unwrap()),
            "a temp folder is inherited"
        );

        secure_state_dir(&state).unwrap();

        let sddl = dacl_sddl(&state).unwrap();
        assert!(admins_only(&sddl), "{sddl}");
        let inside = dacl_sddl(&state.join("hub_identity.key")).unwrap();
        let (_, aces) = parse_dacl(&inside).unwrap();
        assert!(
            only_system_and_admins(&aces),
            "files inside inherit it: {inside}"
        );

        // Open it up again so this (unelevated) test can delete it.
        apply_sddl(&state, "D:(A;OICI;FA;;;WD)").unwrap();
    }

    #[test]
    fn only_a_protected_system_and_admins_dacl_passes() {
        assert!(admins_only(ADMINS_ONLY_SDDL));
        assert!(
            admins_only("D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
            "as Windows reports it"
        );
        assert!(
            !admins_only("D:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
            "not protected"
        );
        assert!(
            !admins_only("D:P(A;OICI;FA;;;SY)(A;OICI;FR;;;BU)"),
            "users can read"
        );
    }
}
