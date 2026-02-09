mod args;
mod checks;
mod config;
mod crypto;
mod hot_reload;
mod identity;
mod monitor;
mod storage;
mod tls;
mod transport;

use anyhow::{Context, Result};
use common::models::StatusReport;
use monitor::Monitor;
use transport::HubTransport;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();

    if let Err(e) = run(args).await {
        eprintln!("\n[ERROR] {:#}", e);
        std::process::exit(1);
    }
}

async fn run(args: args::Args) -> Result<()> {
    // Load config and wrap in Arc<RwLock> for hot reloading
    let cfg = Arc::new(RwLock::new(
        config::load().context("Failed to load probe configuration")?
    ));

    let (signing_key, is_new) = identity::load_or_create_key();
    let probe_id = hex::encode(signing_key.verifying_key().as_bytes());

    // Wrap monitor in Arc so multiple tasks can share it safely
    let monitor = Arc::new(Monitor::new());

    // Initialize transport once (we'll read hub_url from config when needed)
    let hub_url = cfg.read().await.agent.hub_url.clone();
    let transport = HubTransport::new(hub_url);

    // Initialize black box storage
    let black_box = storage::BlackBox::new().await.context("Failed to initialize storage")?;

    // Spawn the config file watcher
    tokio::spawn(hot_reload::watch_config(Arc::clone(&cfg), args.verbose));

    // Retrieve Hub Public Key (if not in config, perform handshake)
    let hub_pk_bytes: Vec<u8> = if let Some(key_hex) = &cfg.read().await.agent.hub_public_key {
        hex::decode(key_hex).context("Invalid hub_public_key in config (expected hex string)")?
    } else {
        println!("No Hub key in config. Performing handshake with Hub...");
        loop {
            match transport.fetch_hub_pk().await {
                Ok(pk) => {
                    if args.verbose { println!("Handshake successful."); }
                    break pk.to_vec();
                }
                Err(e) => {
                    println!("[WARN] Handshake failed ({}). Hub may not be running yet. Retrying in 120s...", e);
                    sleep(Duration::from_secs(120)).await;
                }
            }
        }
    };

    let hub_pk: [u8; 32] = hub_pk_bytes.try_into()
        .map_err(|_| anyhow::anyhow!("Hub public key is the wrong length (expected 32 bytes)"))?;

    if is_new {
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

    if args.verbose {
        let cfg_read = cfg.read().await;
        println!("--- BLENTINEL PROBE ONLINE ---");
        println!("ID: {} | Company: {}", &probe_id[..8], cfg_read.agent.company_id);
        println!("Monitoring {} resources every {}s", cfg_read.resources.len(), cfg_read.agent.interval);
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
            println!("[DEBUG] Cleartext report:\n{}", serde_json::to_string_pretty(&report)?);
        }

        // Encrypt that buffer
        let encrypted_payload = crypto::SecureSeal::encrypt_for_hub(
            &report_bytes,
            &hub_pk
        )?;

        if args.verbose {
            println!("[{}] Signed and Encrypted. Payload size: {} bytes",
                    report.timestamp.format("%H:%M:%S"),
                    encrypted_payload.len());
        }

        // Send to Hub
        match transport.ship_report(encrypted_payload.clone()).await {
            Ok(_) => {
                if args.verbose {
                    println!("[{}] Report successfully delivered.", Utc::now().format("%H:%M:%S"));
                }
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
            Err(_) => {
                if args.verbose {
                    println!("[{}] Hub unreachable. Storing report in Black Box...", Utc::now().format("%H:%M:%S"));
                }
                black_box.queue_report(&encrypted_payload).await?;
            }
        }

        if args.verbose {
            println!("[{}] Sent report for {} resources.",
                     report.timestamp.format("%H:%M:%S"),
                     report.resources.len());
        }

        // Sleep for the configured interval (read from config each time)
        let sleep_duration = cfg.read().await.agent.interval;
        sleep(Duration::from_secs(sleep_duration)).await;
    }
}