//! The hub as a Windows service (`BlentinelHub`, registered by `hub install`,
//! D-52). Mirrors the probe's service mode (`probe/src/service.rs`).
//!
//! The Service Control Manager starts `hub.exe --config <path>`; `main`
//! offers the process to the dispatcher first. From a terminal the dispatcher
//! refuses at once and the hub runs as a normal program.
//!
//! A service has no console, so the log goes to `hub.log` beside the config.
//! Stopping the service ends the process: the database is SQLite in WAL mode,
//! which an ended process leaves consistent.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use hub_lib::args::Args;
use hub_lib::logging;
use hub_lib::ops::layout::SERVICE_NAME;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

/// What `main` parsed, for the SCM's callback (which can't take arguments).
static STARTUP: OnceLock<(Args, PathBuf)> = OnceLock::new();

/// Run as the service if the SCM started this process; `false` otherwise.
pub fn run_if_service(args: &Args, config_path: &Path) -> bool {
    let _ = STARTUP.set((args.clone(), config_path.to_path_buf()));
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).is_ok()
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    let Some((args, config_path)) = STARTUP.get().cloned() else {
        return;
    };
    let log = config_path
        .parent()
        .map_or_else(|| PathBuf::from("hub.log"), |dir| dir.join("hub.log"));
    logging::init_to_file(&args, &log);
    if let Err(e) = run_service(args, config_path) {
        tracing::error!("Service failed: {e:#}");
    }
}

fn status(state: ServiceState, exit_code: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(exit_code),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

fn run_service(args: Args, config_path: PathBuf) -> anyhow::Result<()> {
    let handle =
        std::sync::Arc::new(OnceLock::<service_control_handler::ServiceStatusHandle>::new());
    let for_handler = std::sync::Arc::clone(&handle);
    let registered =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                tracing::info!("Service stopping");
                if let Some(handle) = for_handler.get() {
                    let _ = handle.set_service_status(status(ServiceState::Stopped, 0));
                }
                std::process::exit(0);
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;
    let _ = handle.set(registered);
    registered.set_service_status(status(ServiceState::Running, 0))?;
    tracing::info!("Running as the Windows service {SERVICE_NAME}");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(crate::run(args, config_path));
    let code = if result.is_ok() { 0 } else { 1 };
    registered.set_service_status(status(ServiceState::Stopped, code))?;
    result
}
