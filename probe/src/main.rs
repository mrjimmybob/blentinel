mod args;
mod checks;
mod config;
mod enrollment;
mod hot_reload;
mod hub_link;
mod identity;
mod logging;
mod monitor;
mod outbox;
#[cfg(windows)]
mod service;
mod storage;
mod tls;
mod transport;

use anyhow::{Context, Result};
use chrono::Utc;
use common::models::StatusReport;
use hub_link::HubLink;
use monitor::Monitor;
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();

    // --init: create config template and exit before any probe startup.
    // Always a CLI operation — a running service will never call --init.
    if args.init {
        match config::create_default_config_file() {
            Ok(true) => {
                println!("Created default configuration at: blentinel_probe.toml");
                println!("Review and edit before starting the probe.");
            }
            Ok(false) => {
                eprintln!("Configuration file already exists at: blentinel_probe.toml");
                eprintln!("Refusing to overwrite existing configuration.");
                eprintln!("Remove or rename the file manually if you want to regenerate it.");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to create configuration file: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // enroll <CODE>: join a hub, then exit (a CLI operation, like --init).
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

    if let Err(e) = runtime.block_on(run(args, shutdown_rx)) {
        eprintln!("\n[ERROR] {:#}", e);
        std::process::exit(1);
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

pub(crate) async fn run(args: args::Args, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    // Log the resolved base directory first — before any file I/O — so that
    // path bugs (e.g. CWD vs exe-dir) are immediately visible in the log.
    info!(base_dir = %config::get_base_dir().display(), "Probe state directory");

    // Load config and wrap in Arc<RwLock> for hot reloading
    let cfg = Arc::new(RwLock::new(
        config::load().context("Failed to load probe configuration")?,
    ));
    info!(
        "Configuration loaded from {}",
        config::get_config_path().display()
    );

    let (signing_key, _) = identity::load_or_create_key()?;
    let probe_id = hex::encode(signing_key.verifying_key().as_bytes());

    // Wrap monitor in Arc so multiple tasks can share it safely
    let monitor = Arc::new(Monitor::new());

    // Spawn the config file watcher
    tokio::spawn(hot_reload::watch_config(Arc::clone(&cfg)));

    // The hub, its key and certificate, and where this probe belongs.
    let Some(enrolled) = wait_for_enrollment(&mut shutdown).await? else {
        return Ok(()); // told to stop while waiting
    };
    let mut link = HubLink::new(&enrolled)?;

    // The black box remembers which identities its queued reports were made
    // with; if either changed since the last run, stale payloads are purged.
    let outbox = outbox::Outbox::new(
        storage::BlackBox::new(&probe_id, &enrolled.hub_public_key)
            .await
            .context("Failed to initialize storage")?,
    );

    {
        let cfg_read = cfg.read().await;
        info!(
            probe_id = %&probe_id[..8],
            name = %enrolled.probe_name,
            company = %enrolled.company_id,
            site = %enrolled.site_id,
            hub = %enrolled.hub_url,
            resources = cfg_read.resources.len(),
            interval_secs = cfg_read.agent.interval,
            "Probe online"
        );
    }

    // Main monitoring loop
    loop {
        let start_time = Utc::now();
        // Enrolling again (new code) applies from this cycle on.
        match enrollment::load(&config::get_base_dir()) {
            Ok(Some(current)) => link.follow(&current),
            Ok(None) => warn!("enrollment.json is gone; still reporting to the last hub"),
            Err(e) => warn!("Can't read the enrollment, keeping the current hub: {e:#}"),
        }
        let mut tasks = Vec::new();

        // Refresh system info once per cycle (for CPU, memory, disk checks)
        monitor.refresh().await;

        // Clone resources for this iteration (read lock released immediately)
        let resources = cfg.read().await.resources.clone();

        // Spawn checks in parallel
        // Note: LocalData expands into multiple checks, so we flatten here
        for res in &resources {
            let check_list = checks::from_config(res);
            for check in check_list {
                let m = Arc::clone(&monitor);
                let task = tokio::spawn(async move { check.run(&m).await });
                tasks.push(task);
            }
        }

        // Collect results
        let mut results = Vec::new();
        for task in tasks {
            if let Ok(res) = task.await {
                results.push(res);
            }
        }

        // Create the Report (The "Sentinel's Eyes"). Company and site are
        // informational: the hub files reports by the probe's record (D-47).
        let interval = cfg.read().await.agent.interval;
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
            probe_id: probe_id.clone(),
            company_id,
            hostname,
            site,
            timestamp: start_time,
            interval_seconds: interval as u32,
            resources: results,
            signature: None,
            ephemeral_public_key: None,
            protocol_version: Some(common::protocol::PROTOCOL_VERSION),
        };

        // Sign the full report (excluding signature and ephemeral_public_key)
        // This protects all metadata: company_id, hostname, site, timestamp, etc.
        let signable = report.to_signable();
        let json_to_sign = serde_json::to_vec(&signable)?;
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
        let summary = outbox.submit(&link.transport, encrypted_payload).await?;
        if summary.queued > 0 {
            warn!(
                buffered = summary.queued,
                "Reports buffered until the hub is reachable"
            );
        } else if summary.delivered > 0 {
            info!(
                resources = report.resources.len(),
                delivered = summary.delivered,
                "Report delivered to hub"
            );
        } // permanent rejections are logged as errors by the outbox

        // Sleep for the configured interval, but wake immediately when a
        // shutdown signal arrives (service stop, runtime drop, etc.).
        let sleep_duration = cfg.read().await.agent.interval;
        tokio::select! {
            _ = sleep(Duration::from_secs(sleep_duration)) => {}
            result = shutdown.changed() => {
                // `result` is Err when the sender is dropped (CLI exit path);
                // it is Ok when the sender explicitly sent `true` (service stop).
                if result.is_err() || *shutdown.borrow() {
                    info!("Shutdown signal received, exiting monitoring loop");
                    break;
                }
            }
        }
    }

    Ok(())
}
