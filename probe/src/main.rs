mod args;
mod checks;
mod commands;
mod config;
mod enrollment;
mod health;
mod hub_link;
mod identity;
mod inventory;
mod logging;
mod monitor;
mod ops;
mod outbox;
mod poller;
#[cfg(windows)]
mod service;
mod storage;
mod tls;
mod transport;
mod update;

use anyhow::{Context, Result};
use chrono::Utc;
use common::models::StatusReport;
use hub_link::HubLink;
use monitor::Monitor;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

/// How the monitoring loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Told to stop.
    Stopped,
    /// It switched to another version of itself: start that one (D-55).
    Restart,
}

fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();
    // Taken now: after an update switches files, the running program's own
    // path may name the previous version.
    let exe = std::env::current_exe().ok();

    // --self-test: a downloaded version proves it can run here (D-55).
    if args.self_test {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build Tokio runtime");
        std::process::exit(runtime.block_on(update::self_test::run()));
    }

    // probe install, status, …: then exit.
    if let Some(command) = &args.ops {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build Tokio runtime");
        std::process::exit(runtime.block_on(ops::run(command)));
    }

    // enroll <CODE>: join a hub, then exit (a CLI operation).
    if let Some(code) = args.enroll.as_deref() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build Tokio runtime");
        match runtime.block_on(enroll_command(code)) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("[ERROR] {e:#}");
                std::process::exit(1);
            }
        }
    }

    // Initialise file + stdout logging.  Levels are derived from CLI flags:
    //   normal  → file INFO+, console OFF
    //   verbose → file INFO+, console INFO+
    //   debug   → file DEBUG+, console DEBUG+
    // On failure: warn to stderr and continue without log file.
    let _log_guard = match logging::init_from_args(args.debug, args.verbose) {
        Ok(guard) => Some(guard),
        Err(e) => {
            eprintln!(
                "[WARN] File logging unavailable: {}. Continuing without log file.",
                e
            );
            None
        }
    };

    info!(
        "=== Blentinel Probe {} starting ===",
        env!("CARGO_PKG_VERSION")
    );

    // On Windows, attempt to register with the Service Control Manager.
    // `try_run_as_service` blocks and runs the full probe lifecycle when
    // launched by SCM, then returns Ok(()) when the service stops.
    // When launched from a terminal it returns Err immediately (< 1 ms),
    // so we fall through to CLI mode — no flag required for either path.
    #[cfg(windows)]
    if let Ok(()) = service::try_run_as_service() {
        return;
    }

    // CLI mode: build the async runtime here.
    // We avoid #[tokio::main] so that service mode can create its own runtime
    // in service.rs without ever nesting two Tokio runtimes.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");

    // The CLI shutdown receiver is never signalled; Ctrl-C terminates naturally.
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);

    match runtime.block_on(run(args, shutdown_rx)) {
        Ok(Outcome::Stopped) => {}
        Ok(Outcome::Restart) => {
            drop(runtime);
            drop(_log_guard); // flush the log before this process ends
            match exe {
                Some(exe) => update::relaunch(&exe),
                None => std::process::exit(update::EXIT_RESTART),
            }
        }
        Err(e) => {
            eprintln!("\n[ERROR] {:#}", e);
            std::process::exit(1);
        }
    }
}

/// `probe enroll <CODE>`: redeem the code and show where the probe now belongs.
async fn enroll_command(code: &str) -> Result<()> {
    let (signing_key, _) = identity::load_or_create_key()?;
    let dir = config::get_base_dir();
    let enrolled = enrollment::enroll(code, &signing_key, &dir).await?;
    println!("Enrolled with {}", enrolled.hub_url);
    println!("  Probe   : {}", enrolled.probe_name);
    println!("  Company : {}", enrolled.company_name);
    println!("  Site    : {}", enrolled.site_name);
    println!("  Saved   : {}", enrollment::path_in(&dir).display());
    println!(
        "Start the probe (or its service) to begin reporting; a running probe picks this up by itself."
    );
    Ok(())
}

/// How often an unenrolled probe looks for `enrollment.json`.
const ENROLLMENT_POLL: Duration = Duration::from_secs(10);

/// The probe's enrollment, waiting for it if the probe hasn't been enrolled
/// yet (e.g. a service installed before `probe enroll`). `None` if told to
/// shut down while waiting.
async fn wait_for_enrollment(
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<enrollment::Enrollment>> {
    let dir = config::get_base_dir();
    let mut warned = false;
    loop {
        if let Some(enrolled) = enrollment::load(&dir)? {
            return Ok(Some(enrolled));
        }
        if !warned {
            warn!(
                "Not enrolled: get a code from the hub's \"Add probe\" and run `probe enroll <code>`. Waiting for {}",
                enrollment::path_in(&dir).display()
            );
            eprintln!(
                "Not enrolled yet. Get a code from the hub (Companies > company > Add probe) and run:\n  probe enroll <code>"
            );
            warned = true;
        }
        tokio::select! {
            _ = sleep(ENROLLMENT_POLL) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(None);
                }
            }
        }
    }
}

pub(crate) async fn run(args: args::Args, mut shutdown: watch::Receiver<bool>) -> Result<Outcome> {
    // Log the resolved base directory first — before any file I/O — so that
    // path bugs (e.g. CWD vs exe-dir) are immediately visible in the log.
    let dir = config::get_base_dir();
    info!(base_dir = %dir.display(), "Probe state directory");

    // The update watchdog first: a new version that keeps crashing must be
    // rolled back before whatever crashes it (D-55).
    let (mut updater, restart) = update::Updater::start(&dir)?;
    if restart.is_some() {
        return Ok(Outcome::Restart);
    }

    // The checks the hub gave it last time, if any (FP-4).
    let mut active = config::ActiveConfig::load(&dir);

    let (signing_key, _) = identity::load_or_create_key()?;
    let probe_id = hex::encode(signing_key.verifying_key().as_bytes());

    // Wrap monitor in Arc so multiple tasks can share it safely
    let monitor = Arc::new(Monitor::new());
    // Commands from the hub, and their results for the next report (D-56).
    let mut commands = commands::Commands::default();
    // What the probe says about itself in each report (D-73).
    let mut health = health::Health::new(Utc::now());
    // What the machine is made of, sent now and then (D-85).
    let mut inventory = inventory::Collector::default();

    // The hub, its keys and certificate, and where this probe belongs.
    let Some(mut enrolled) = wait_for_enrollment(&mut shutdown).await? else {
        return Ok(Outcome::Stopped); // told to stop while waiting
    };

    // The black box remembers which identities its queued reports were made
    // with; if either changed since the last run, stale payloads are purged.
    let outbox = outbox::Outbox::new(
        storage::BlackBox::new(&probe_id, &enrolled.hub_public_key)
            .await
            .context("Failed to initialize storage")?,
    );

    // Asking for commands between reports, on its own loop (D-76): it sends
    // what it gets here; the loop below runs it and reports at once.
    let (poll_target, poll_target_rx) = watch::channel(None::<poller::Target>);
    let (polled_tx, mut polled) = mpsc::channel(8);
    let poll_task = poller::spawn(signing_key.clone(), poll_target_rx, polled_tx);

    // One pass per enrollment: a revoked probe waits to be enrolled again.
    let outcome = 'enrollment: loop {
        let mut link = HubLink::new(&enrolled)?;
        info!(
            probe_id = %&probe_id[..8],
            name = %enrolled.probe_name,
            company = %enrolled.company_id,
            site = %enrolled.site_id,
            hub = %enrolled.hub_url,
            checks = active.config.as_ref().map_or(0, |c| c.checks.len()),
            interval_secs = active.interval().as_secs(),
            "Probe online"
        );
        let mut clock_skew_warned = false;

        // Main monitoring loop
        loop {
            let start_time = Utc::now();
            // Enrolling again (new code) applies from this cycle on.
            match enrollment::load(&dir) {
                Ok(Some(current)) => link.follow(&current),
                Ok(None) => warn!("enrollment.json is gone; still reporting to the last hub"),
                Err(e) => {
                    warn!("Can't read the enrollment, keeping the current hub: {e:#}");
                    health.problem(format!("Can't read the enrollment: {e:#}"), Utc::now());
                }
            }
            // The poller asks the hub this loop reports to.
            poll_target.send_if_modified(|target| {
                if target.as_ref().map(|t| &t.enrollment) == Some(link.enrollment()) {
                    return false;
                }
                *target = Some(poller::Target {
                    enrollment: link.enrollment().clone(),
                    transport: link.transport.clone(),
                    hub_key: link.hub_key,
                    hub_signing_keys: link.hub_signing_keys.clone(),
                });
                true
            });

            let results = run_checks(&active, &monitor, &mut health).await;

            // Company and site are informational: the hub files reports by
            // the probe's record (D-47).
            let (company_id, site) = {
                let enrolled = link.enrollment();
                (enrolled.company_id.clone(), enrolled.site_id.clone())
            };

            // Get hostname (fallback to "unknown" if unavailable)
            let hostname = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string());

            let mut report = StatusReport {
                protocol_version: common::protocol::PROTOCOL_VERSION,
                probe_id: probe_id.clone(),
                company_id,
                hostname,
                site,
                timestamp: start_time,
                interval_seconds: active.interval().as_secs() as u32,
                results,
                config_revision: active.revision(),
                config_rejected: active.rejected.clone(),
                probe_version: Some(update::VERSION.to_string()),
                update_failed: updater.failure(),
                command_results: commands.take_results(),
                pins: link.enrollment().pin_digests().ok(),
                health: Some(health.snapshot(outbox.queued().await?, start_time)),
                inventory: inventory.due(&monitor, start_time).await,
                signature: None,
            };

            // Sign the full report (every field but the signature).
            let json_to_sign = serde_json::to_vec(&report.to_signable())?;
            report.signature = Some(common::crypto::sign(&json_to_sign, &signing_key));

            // Serialize the WHOLE report once
            // This is the "Cleartext" that we want to hide
            let report_bytes = serde_json::to_vec(&report)?;

            if args.debug {
                debug!(
                    "Cleartext report:\n{}",
                    serde_json::to_string_pretty(&report)?
                );
            }

            // Encrypt that buffer
            let encrypted_payload = common::crypto::seal_for_hub(&report_bytes, &link.hub_key)?;

            debug!(
                payload_bytes = encrypted_payload.len(),
                "Report signed and encrypted"
            );

            // Deliver in order, buffering only while the hub is unreachable (see outbox.rs).
            let summary = outbox
                .submit(&link.sender(&signing_key), encrypted_payload)
                .await?;
            if summary.revoked {
                error!(
                    "The hub revoked this probe. It stops reporting until it is enrolled again: probe enroll <code>"
                );
                outbox.clear().await?;
                poll_target.send_replace(None);
                match enrollment::set_aside_revoked(&dir) {
                    Ok(()) => {
                        enrolled = match wait_for_enrollment(&mut shutdown).await? {
                            Some(next) => next,
                            None => break 'enrollment Outcome::Stopped,
                        };
                        continue 'enrollment;
                    }
                    // Keep the normal pace rather than spin against the hub.
                    Err(e) => error!("{e:#}"),
                }
            }
            // A new config runs at once, not an interval later.
            let mut run_again_now = false;
            if let Some(problem) = summary.problem {
                health.problem(problem, Utc::now());
            }
            if let Some(ack) = summary.last_ack {
                health.heard_from_hub(ack.hub_time, Utc::now());
                match transport::clock_skew(ack.hub_time, Utc::now()) {
                    Some(skew) if !clock_skew_warned => {
                        warn!(
                            skew_secs = skew,
                            "This machine's clock is off the hub's; the hub refuses reports beyond {} s. Sync the clock (NTP).",
                            common::protocol::MAX_CLOCK_SKEW_SECS
                        );
                        clock_skew_warned = true;
                    }
                    None if clock_skew_warned => {
                        info!("The clock agrees with the hub's again");
                        clock_skew_warned = false;
                    }
                    _ => {}
                }
                if let Some(offer) = &ack.update {
                    updater.offer(offer, &link.transport);
                }
                if let Some(offered) = ack.config {
                    run_again_now = active.apply(&dir, offered);
                }
                if let Some(rotation) = &ack.rotation
                    && let Err(problem) =
                        follow_rotation(&dir, &outbox, link.enrollment(), rotation).await
                {
                    health.problem(problem, Utc::now());
                }
                // After the config: "run now" runs the checks it brought.
                let checks = active.config.as_ref().map_or(0, |c| c.checks.len());
                run_again_now |= commands.receive(&ack.commands, checks).run_now;
            }
            let restart = updater
                .after_report(summary.delivered > 0, active.interval(), Utc::now())
                .await;
            if restart.is_some() {
                break 'enrollment Outcome::Restart;
            }
            if summary.queued > 0 {
                warn!(
                    buffered = summary.queued,
                    "Reports buffered until the hub is reachable"
                );
            } else if summary.delivered > 0 {
                info!(
                    results = report.results.len(),
                    delivered = summary.delivered,
                    "Report delivered to hub"
                );
            } // permanent rejections are logged as errors by the outbox

            if run_again_now {
                continue;
            }
            // Sleep for the interval, but wake immediately when a shutdown
            // signal arrives (service stop, runtime drop, etc.), or commands
            // the poller got need running and reporting (D-76).
            let next_cycle = tokio::time::Instant::now() + active.interval();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(next_cycle) => break,
                    result = shutdown.changed() => {
                        // `result` is Err when the sender is dropped (CLI exit path);
                        // it is Ok when the sender explicitly sent `true` (service stop).
                        if result.is_err() || *shutdown.borrow() {
                            info!("Shutdown signal received, exiting monitoring loop");
                            break 'enrollment Outcome::Stopped;
                        }
                    }
                    Some(received) = polled.recv() => {
                        let checks = active.config.as_ref().map_or(0, |c| c.checks.len());
                        let actions = commands.receive(&received, checks);
                        // Run now, or at least send the results now.
                        if actions.run_now || commands.has_results() {
                            break;
                        }
                    }
                }
            }
        }
    };

    poll_task.abort();
    Ok(outcome)
}

/// The hub announced its next pins, or switched to them (D-61). The ack was
/// signed with a key the probe pins; the link follows the saved enrollment
/// from the next cycle on. `Err`: what went wrong, for the probe's health.
async fn follow_rotation(
    dir: &std::path::Path,
    outbox: &outbox::Outbox,
    current: &enrollment::Enrollment,
    rotation: &common::pins::Rotation,
) -> Result<(), String> {
    use enrollment::RotationStep;
    match enrollment::follow_rotation(dir, current, rotation) {
        Ok(None) => {}
        Ok(Some((_, RotationStep::Announced))) => {
            info!("The hub announced new keys; this probe is ready for its switch")
        }
        Ok(Some((rotated, RotationStep::Switched { hub_key_changed }))) => {
            info!("The hub switched to its new keys; the old ones are dropped");
            if hub_key_changed && let Err(e) = outbox.set_hub_key(&rotated.hub_public_key).await {
                warn!("Can't record the hub's new key in the black box: {e:#}");
                return Err(format!("Can't record the hub's new key: {e:#}"));
            }
        }
        Err(e) => {
            error!("Can't take the hub's new keys: {e:#}");
            return Err(format!("Can't take the hub's new keys: {e:#}"));
        }
    }
    Ok(())
}

/// Run every check of the active config in parallel, in the config's order.
/// A check whose task failed is left out (logged, and noted in `health`).
async fn run_checks(
    active: &config::ActiveConfig,
    monitor: &Arc<Monitor>,
    health: &mut health::Health,
) -> Vec<common::checks::CheckResult> {
    let Some(config) = &active.config else {
        return Vec::new();
    };
    // System info once per cycle, for the checks of this machine.
    monitor.refresh().await;
    let tasks: Vec<_> = config
        .checks
        .iter()
        .cloned()
        .map(|check| {
            let monitor = Arc::clone(monitor);
            tokio::spawn(async move { checks::run(&check, &monitor).await })
        })
        .collect();
    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        match task.await {
            Ok(result) => results.push(result),
            Err(e) => {
                error!("A check failed to run: {e}");
                health.problem(format!("A check failed to run: {e}"), Utc::now());
            }
        }
    }
    results
}
