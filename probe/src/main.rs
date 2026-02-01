mod args;
mod config;
mod crypto;
mod identity;
mod monitor;
mod storage;
mod transport;

use anyhow::{Context, Result};
use common::models::StatusReport;
use monitor::Monitor;
use transport::HubTransport;
use chrono::Utc;
use std::sync::Arc;
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
    let cfg = config::load().context("Failed to load probe configuration")?;
    let signing_key = identity::load_or_create_key();
    let probe_id = hex::encode(signing_key.verifying_key().as_bytes());

    // Wrap monitor in Arc so multiple tasks can share it safely
    let monitor = Arc::new(Monitor::new());
    
    // Initialize transport once
    let transport = HubTransport::new(cfg.agent.hub_url.clone());

    // Initialize black box storage
    let black_box = storage::BlackBox::new().await.context("Failed to initialize storage")?;


    // Retrieve Hub Public Key (if not in config, perform handshake)
    let hub_pk_bytes: Vec<u8> = if let Some(key_hex) = &cfg.agent.hub_public_key {
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

    if args.verbose {
        println!("--- BLENTINEL PROBE ONLINE ---");
        println!("ID: {} | Company: {}", &probe_id[..8], cfg.agent.company_id);
        println!("Monitoring {} resources every {}s", cfg.resources.len(), cfg.agent.interval);
    }

    // Main monitoring loop
    loop {
        let start_time = Utc::now();
        let mut tasks = Vec::new();

        // Spawn checks in parallel
        for res in cfg.resources.clone() {
            let m = Arc::clone(&monitor);
            let task = tokio::spawn(async move {
                match res.r#type.as_str() {
                    "ping" => m.check_ping(res.name, res.target).await,
                    "http" => m.check_http(res.name, res.target).await,
                    "tcp"  => m.check_tcp(res.name, res.target).await,
                    _ => m.error_status(res.name, res.target, "Unknown resource type"),
                }
            });
            tasks.push(task);
        }

        // Collect results
        let mut results = Vec::new();
        for task in tasks {
            if let Ok(res) = task.await {
                results.push(res);
            }
        }

        // Create the Report (The "Sentinel's Eyes")
        let mut report = StatusReport {
            probe_id: probe_id.clone(),
            company_id: cfg.agent.company_id.clone(),
            timestamp: start_time,
            interval_seconds: cfg.agent.interval as u32,
            resources: results,
            signature: None, // Logic for Ed25519 signing goes here next
            ephemeral_public_key: None,
        };

        // Sign the resources (already correct)
        let json_to_sign = serde_json::to_vec(&report.resources)?;
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

        // Sleep for the configured interval
        sleep(Duration::from_secs(cfg.agent.interval)).await;
    }
}