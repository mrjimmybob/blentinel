//! Windows Service Control Manager (SCM) integration.
//!
//! Compiled only on Windows.  Registers the probe with the SCM under the name
//! [`SERVICE_NAME`], manages service state transitions, and bridges the SCM
//! stop/shutdown signal to the async shutdown channel consumed by
//! [`crate::run`].
//!
//! # Entry point
//!
//! [`try_run_as_service`] is called from `main` before CLI mode is entered.
//! It blocks (and runs the full probe lifecycle) when the process was launched
//! by SCM, and returns `Err` immediately when running from a terminal — the
//! caller then falls through to normal CLI execution.
//!
//! # First-run note
//!
//! The initial identity-generation step (`--init` / first run key printing)
//! must be performed from the CLI *before* installing the service.  If the
//! service is started without a prior CLI setup, it will exit immediately
//! (the probe's own `std::process::exit(0)` call on first run), and SCM will
//! report the service as stopped.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tracing::{error, info};
use tokio::sync::watch;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

/// Service name as registered with the Windows SCM.
///
/// Must match the name used in `install_probe_service.ps1` (`sc.exe create`).
pub const SERVICE_NAME: &str = "BlentinelProbe";

/// Attempt to dispatch this process as a Windows service.
///
/// Blocks until the service lifecycle completes when running under SCM.
/// Returns `Err` immediately (< 1 ms) when not in an SCM context, which the
/// caller treats as "not a service — run in CLI mode instead".
pub fn try_run_as_service() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

// Generates the `extern "system" fn ffi_service_main` entry point that the
// SCM calls after `service_dispatcher::start`.
define_windows_service!(ffi_service_main, service_main);

/// SCM-dispatched entry point.  Errors are written to stderr; Windows Event
/// Log integration is out of scope for now.
fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        error!("Service fatal error: {:#}", e);
    }
}

/// Core service lifecycle: registers with SCM, runs the async probe loop,
/// and reports the final `Stopped` status when the loop exits.
fn run_service() -> anyhow::Result<()> {
    // Shutdown channel — the control handler sends `true` from its Windows
    // thread; the async loop wakes on `shutdown.changed()` and breaks.
    // `watch::Sender::send` is sync-safe: no runtime is needed on the sender side.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // MUST stay alive while runtime runs
    let _shutdown_guard = shutdown_tx.clone();
    let stop_signal = Arc::new(shutdown_tx);
    let stop_signal_cb = Arc::clone(&stop_signal);

    // Register the service control handler with SCM.
    // This callback is invoked from a dedicated Windows thread (not on the
    // Tokio runtime), so it must be a plain sync closure.
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                info!("SCM stop/shutdown received");
                let _ = stop_signal_cb.send(true);
                ServiceControlHandlerResult::NoError
            }
            // SCM polls the current service state via Interrogate.
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("Failed to register service control handler")?;

    info!("Service control handler registered");

    // Inform SCM that the service is running and accepting stop/shutdown.
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .context("Failed to report Running status to SCM")?;
    info!("Service reported RUNNING to SCM");

    // Build a dedicated async runtime for the probe.
    // Service mode creates its own runtime here; CLI mode creates one in main().
    // This avoids nesting two runtimes.
    //
    // If the build fails we must report STOPPED before returning so SCM does
    // not get stuck believing the service is still RUNNING after the process
    // exits.  The `?` operator would skip the STOPPED report below.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::ServiceSpecific(2),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            });
            return Err(anyhow::anyhow!("Failed to build Tokio runtime: {}", e));
        }
    };

    let exit_code = match runtime.block_on(crate::run(crate::args::Args::service_defaults(), shutdown_rx)) {
        Ok(()) => {
            info!("Probe loop exited cleanly");
            ServiceExitCode::Win32(0)
        }
        Err(e) => {
            error!("Probe loop exited with error: {:#}", e);
            ServiceExitCode::ServiceSpecific(1)
        }
    };

    // Notify SCM that the service has fully stopped.
    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code,
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .context("Failed to report Stopped status to SCM")?;
    info!("Service reported STOPPED to SCM");

    Ok(())
}
