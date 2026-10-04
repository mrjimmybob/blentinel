//! Installing the hub and the probe as services (M2-08, M2-09, D-52, D-53).
//!
//! What both need, written once: what a service manager says about a
//! service, and on Windows the Service Control Manager and folder
//! permissions through the Win32 API. No external tools (the probe may use
//! none, FP-2).

/// What the service manager says about a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    NotInstalled,
    Running,
    Stopped(String),
    Other(String),
}

impl std::fmt::Display for ServiceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceState::NotInstalled => write!(f, "not installed"),
            ServiceState::Running => write!(f, "running"),
            ServiceState::Stopped(why) | ServiceState::Other(why) => write!(f, "{why}"),
        }
    }
}

#[cfg(windows)]
pub mod windows {
    //! The Service Control Manager and folder permissions.

    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use windows_service::service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
        ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
        ServiceState as WinState, ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    };

    use super::ServiceState;

    /// A Windows error with what was being done.
    #[derive(Debug, thiserror::Error)]
    #[error("{what}: {source}")]
    pub struct Error {
        what: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    }

    fn err(
        what: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Error {
        Error {
            what: what.into(),
            source: Box::new(source),
        }
    }

    pub type Result<T> = std::result::Result<T, Error>;

    /// Full control for SYSTEM and Administrators only, inherited by
    /// everything inside, nothing inherited from above (`P`): other local
    /// users can't read keys, databases or configs.
    pub const ADMINS_ONLY_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Can this process create services (an elevated Administrator)?
    pub fn is_admin() -> bool {
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE).is_ok()
    }

    /// Create `dir` if needed and give it (and everything in it) [`ADMINS_ONLY_SDDL`].
    pub fn restrict_to_admins(dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)
            .map_err(|e| err(format!("can't create {}", dir.display()), e))?;
        apply_sddl(dir, ADMINS_ONLY_SDDL)
    }

    /// Replace the DACL of `path` with the one in `sddl`; inheritable entries
    /// reach everything inside.
    pub fn apply_sddl(path: &Path, sddl: &str) -> Result<()> {
        let protected = if sddl.starts_with("D:P") {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let text = wide(OsStr::new(sddl));
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: valid NUL-terminated string and out-pointer; freed below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(err(
                "can't build the permissions",
                std::io::Error::last_os_error(),
            ));
        }
        let result = (|| {
            let (mut present, mut defaulted) = (0, 0);
            let mut dacl: *mut ACL = std::ptr::null_mut();
            // SAFETY: `descriptor` is the valid descriptor made above.
            if unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
            } == 0
            {
                return Err(err(
                    "can't read the permissions",
                    std::io::Error::last_os_error(),
                ));
            }
            let target = wide(path.as_os_str());
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
                return Err(err(
                    format!("can't set the permissions of {}", path.display()),
                    std::io::Error::from_raw_os_error(rc as i32),
                ));
            }
            Ok(())
        })();
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(descriptor) };
        result
    }

    /// The permissions of `path` as SDDL (`D:PAI(A;OICI;FA;;;SY)…`).
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
            return Err(err(
                format!("can't read the permissions of {}", path.display()),
                std::io::Error::from_raw_os_error(rc as i32),
            ));
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
            Err(err(
                "can't read the permissions",
                std::io::Error::last_os_error(),
            ))
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

    /// A DACL in SDDL: its flags (`PAI`) and its entries (`A;OICI;FA;;;SY`, …).
    fn parse_dacl(sddl: &str) -> Option<(&str, Vec<&str>)> {
        let rest = sddl.strip_prefix("D:")?;
        let flags_end = rest.find('(').unwrap_or(rest.len());
        let aces = rest[flags_end..]
            .split(['(', ')'])
            .filter(|ace| !ace.is_empty())
            .collect();
        Some((&rest[..flags_end], aces))
    }

    /// Every entry names SYSTEM or Administrators (the SID is the last field).
    pub fn only_system_and_admins(sddl: &str) -> bool {
        parse_dacl(sddl).is_some_and(|(_, aces)| {
            aces.iter()
                .all(|ace| matches!(ace.rsplit(';').next(), Some("SY" | "BA")))
        })
    }

    /// Only SYSTEM and Administrators, and nothing inherited from above (`P`).
    pub fn admins_only(sddl: &str) -> bool {
        parse_dacl(sddl).is_some_and(|(flags, _)| flags.contains('P'))
            && only_system_and_admins(sddl)
    }

    /// A service to register: `exe` started with `args`, as LocalSystem,
    /// automatically, restarted 5 s after a failure. A failure is a crash or
    /// a stop with an error code: the probe stops with one to have the
    /// service start the version it just switched to (D-55).
    pub struct ServiceSpec<'a> {
        pub name: &'a str,
        pub display_name: &'a str,
        pub description: &'a str,
        pub exe: PathBuf,
        pub args: Vec<OsString>,
    }

    /// Create or update the service, and start it if `start`.
    pub fn install(spec: &ServiceSpec, start: bool) -> Result<()> {
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .map_err(|e| {
            err(
                "can't open the Service Control Manager (run as Administrator)",
                e,
            )
        })?;
        let access = ServiceAccess::QUERY_STATUS
            | ServiceAccess::START
            | ServiceAccess::STOP
            | ServiceAccess::CHANGE_CONFIG;
        let info = ServiceInfo {
            name: OsString::from(spec.name),
            display_name: OsString::from(spec.display_name),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: spec.exe.clone(),
            launch_arguments: spec.args.clone(),
            dependencies: vec![],
            account_name: None, // LocalSystem
            account_password: None,
        };
        let service = match manager.open_service(spec.name, access) {
            Ok(service) => {
                service
                    .change_config(&info)
                    .map_err(|e| err("can't update the service", e))?;
                service
            }
            Err(_) => manager
                .create_service(&info, access)
                .map_err(|e| err("can't create the service", e))?,
        };
        service
            .set_description(spec.description)
            .map_err(|e| err("can't describe the service", e))?;
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
            .map_err(|e| err("can't set the service's restart policy", e))?;
        service
            .set_failure_actions_on_non_crash_failures(true)
            .map_err(|e| err("can't set the service's restart policy", e))?;
        let running = service
            .query_status()
            .map_err(|e| err("can't query the service", e))?
            .current_state
            == WinState::Running;
        if start && !running {
            service
                .start(&[] as &[&OsStr])
                .map_err(|e| err("can't start the service", e))?;
        }
        Ok(())
    }

    fn open(name: &str, access: ServiceAccess) -> Option<windows_service::service::Service> {
        let manager =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
        manager.open_service(name, access).ok()
    }

    /// Stop the service and wait (up to 30 s) until it has.
    pub fn stop(name: &str) -> Result<()> {
        let Some(service) = open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) else {
            return Ok(());
        };
        let query = |s: &windows_service::service::Service| {
            s.query_status()
                .map(|status| status.current_state)
                .map_err(|e| err("can't query the service", e))
        };
        if query(&service)? != WinState::Stopped {
            service
                .stop()
                .map_err(|e| err("can't stop the service", e))?;
        }
        for _ in 0..60 {
            if query(&service)? == WinState::Stopped {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(err(
            "the service didn't stop within 30 s",
            std::io::Error::from(std::io::ErrorKind::TimedOut),
        ))
    }

    /// Stop and delete the service.
    pub fn remove(name: &str) -> Result<()> {
        stop(name)?;
        if let Some(service) = open(name, ServiceAccess::DELETE) {
            service
                .delete()
                .map_err(|e| err("can't delete the service", e))?;
        }
        Ok(())
    }

    pub fn state(name: &str) -> ServiceState {
        let Some(service) = open(name, ServiceAccess::QUERY_STATUS) else {
            return ServiceState::NotInstalled;
        };
        match service.query_status().map(|s| s.current_state) {
            Ok(WinState::Running) => ServiceState::Running,
            Ok(WinState::Stopped) => ServiceState::Stopped("stopped".into()),
            Ok(other) => ServiceState::Other(format!("{other:?}")),
            Err(e) => ServiceState::Other(e.to_string()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_folder_is_restricted_to_system_and_administrators() {
            let dir = tempfile::tempdir().unwrap();
            let folder = dir.path().join("state");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("identity.key"), [7u8; 32]).unwrap();
            assert!(
                !admins_only(&dacl_sddl(&folder).unwrap()),
                "a temp folder is inherited"
            );

            restrict_to_admins(&folder).unwrap();

            let sddl = dacl_sddl(&folder).unwrap();
            assert!(admins_only(&sddl), "{sddl}");
            let inside = dacl_sddl(&folder.join("identity.key")).unwrap();
            assert!(
                only_system_and_admins(&inside),
                "files inside inherit it: {inside}"
            );

            // Open it up again so this (unelevated) test can delete it.
            apply_sddl(&folder, "D:(A;OICI;FA;;;WD)").unwrap();
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
}
