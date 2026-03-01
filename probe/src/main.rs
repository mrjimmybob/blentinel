mod args;
mod checks;
mod config;
mod crypto;
mod hot_reload;
mod identity;
mod logging;
mod monitor;
#[cfg(windows)]
mod service;
mod storage;
mod tls;
mod transport;

use anyhow::{Context, Result};
use common::models::StatusReport;
use monitor::Monitor;
use transport::HubTransport;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};
use tokio::time::{sleep, Duration};
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

    // Initialise file + stdout logging.  Levels are derived from CLI flags:
    //   normal  → file INFO+, console OFF
    //   verbose → file INFO+, console INFO+
    //   debug   → file DEBUG+, console DEBUG+
    // On failure: warn to stderr and continue without log file.
    let _log_guard = match logging::init_from_args(args.debug, args.verbose) {
        Ok(guard) => Some(guard),
        Err(e) => {
            eprintln!("[WARN] File logging unavailable: {}. Continuing without log file.", e);
            None
        }
    };

    info!("=== Blentinel Probe {} starting ===", env!("CARGO_PKG_VERSION"));

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

pub(crate) async fn run(args: args::Args, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    // Log the resolved base directory first — before any file I/O — so that
    // path bugs (e.g. CWD vs exe-dir) are immediately visible in the log.
    info!(base_dir = %config::get_base_dir().display(), "Probe state directory");

    // Load config and wrap in Arc<RwLock> for hot reloading
    let cfg = Arc::new(RwLock::new(
        config::load().context("Failed to load probe configuration")?
    ));
    info!("Configuration loaded from {}", config::get_config_path().display());

    let (signing_key, is_new) = identity::load_or_create_key();
    let probe_id = hex::encode(signing_key.verifying_key().as_bytes());

    // Wrap monitor in Arc so multiple tasks can share it safely
    let monitor = Arc::new(Monitor::new());

    // Initialize transport once (we'll read hub_url from config when needed)
    let hub_url = cfg.read().await.agent.hub_url.clone();
    let transport = HubTransport::new(hub_url);

    // Initialize black box storage with identity fingerprints.
    // If keys changed since last run, stale queued payloads are purged automatically.
    let hub_public_key_cfg = cfg.read().await.agent.hub_public_key.clone().unwrap_or_default();
    let black_box = storage::BlackBox::new(&probe_id, &hub_public_key_cfg)
        .await
        .context("Failed to initialize storage")?;

    // Spawn the config file watcher
    tokio::spawn(hot_reload::watch_config(Arc::clone(&cfg)));

    // Retrieve Hub Public Key (if not in config, perform handshake)
    let hub_pk_bytes: Vec<u8> = if let Some(key_hex) = &cfg.read().await.agent.hub_public_key {
        hex::decode(key_hex).context("Invalid hub_public_key in config (expected hex string)")?
    } else {
        info!("Hub public key not in config, initiating handshake with hub");
        loop {
            match transport.fetch_hub_pk().await {
                Ok(pk) => {
                    info!("Hub handshake successful, public key received");
                    break pk.to_vec();
                }
                Err(e) => {
                    warn!("Hub handshake failed: {}. Hub may not be running, retrying in 120s", e);
                    // Interruptible wait: a service stop signal breaks out cleanly
                    // rather than blocking SCM for the full retry interval.
                    tokio::select! {
                        _ = sleep(Duration::from_secs(120)) => {}
                        result = shutdown.changed() => {
                            if result.is_err() || *shutdown.borrow() {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    };

    let hub_pk: [u8; 32] = hub_pk_bytes.try_into()
        .map_err(|_| anyhow::anyhow!("Hub public key is the wrong length (expected 32 bytes)"))?;

    if is_new {
        // Log to file before the terminal display — process::exit(0) below
        // bypasses Rust's drop machinery, so this may not flush in service mode,
        // but is best-effort.  The operator must run this from CLI first anyway.
        info!(probe_id = %probe_id, "First run: new probe identity generated");
        info!("Register this public key in blentinel_hub.toml, then restart the probe");

        let cfg_read = cfg.read().await;

        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string());

        // let verifying_key: VerifyingKey = (&signing_key).into();
        // println!("PUBLIC KEY: {}", hex::encode(verifying_key.as_bytes()));
        
        println!("--------------------------------------------------");
        println!("FIRST RUN: New Probe Identity Generated.");
        println!();

        println!("Hostname : {}", hostname);
        println!("Company  : {}", cfg_read.agent.company_id);
        println!("Site     : {}", cfg_read.agent.site);

        println!();
        println!("Probe ID (short): {}", &probe_id[..8]);

        println!();
        println!("PUBLIC KEY (paste into blentinel_hub.toml):");
        println!("{}", probe_id);

        println!();
        println!("Example:");
        println!("[[probes]]");
        println!("name = \"{}\"", hostname);
        println!("public_key = \"{}\"", probe_id);

        println!("--------------------------------------------------");
        println!("After adding the probe to the hub, restart the probe.");
        std::process::exit(0);

    }

    {
        let cfg_read = cfg.read().await;
        info!(
            probe_id = %&probe_id[..8],
            company_id = %cfg_read.agent.company_id,
            resources = cfg_read.resources.len(),
            interval_secs = cfg_read.agent.interval,
            "Probe online"
        );
    }

    // Main monitoring loop
    loop {
        let start_time = Utc::now();
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

        // Create the Report (The "Sentinel's Eyes")
        let (company_id, interval, site) = {
            let cfg_read = cfg.read().await;
            (cfg_read.agent.company_id.clone(), cfg_read.agent.interval, cfg_read.agent.site.clone())
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
        };

        // Sign the full report (excluding signature and ephemeral_public_key)
        // This protects all metadata: company_id, hostname, site, timestamp, etc.
        let signable = report.to_signable();
        let json_to_sign = serde_json::to_vec(&signable)?;
        report.signature = Some(crypto::SecureSeal::sign(&json_to_sign, &signing_key));

        // Serialize the WHOLE report once
        // This is the "Cleartext" that we want to hide
        let report_bytes = serde_json::to_vec(&report)?;

        if args.debug {
            debug!("Cleartext report:\n{}", serde_json::to_string_pretty(&report)?);
        }

        // Encrypt that buffer
        let encrypted_payload = crypto::SecureSeal::encrypt_for_hub(
            &report_bytes,
            &hub_pk
        )?;

        debug!(payload_bytes = encrypted_payload.len(), "Report signed and encrypted");

        // Send to Hub
        match transport.ship_report(encrypted_payload.clone()).await {
            Ok(_) => {
                info!(resources = report.resources.len(), "Report delivered to hub");
                // Since we are online, try to flush the "Black Box" if any reports are stored there.
                let queued = black_box.get_queued_reports().await?;
                for (id, old_payload) in queued {
                    if transport.ship_report(old_payload).await.is_ok() {
                        black_box.delete_report(id).await?;
                    } else {
                        break; // Hub went down again, stop flushing
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Hub unreachable, queuing report in black box");
                black_box.queue_report(&encrypted_payload).await?;
            }
        }

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