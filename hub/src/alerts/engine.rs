#![cfg(feature = "ssr")]

use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;

use crate::config::{HubConfig, ThresholdConfig};
use crate::alerts::{email, state, silence};
use common::models::StatusReport;

/// Evaluate alerts for a status report
pub async fn evaluate_alerts_for_report(
    pool: &SqlitePool,
    config: Arc<RwLock<HubConfig>>,
    report: &StatusReport,
) -> Result<()> {
    let cfg = config.read().await;

    // Skip if alerts not enabled
    if !cfg.alerts.enabled {
        return Ok(());
    }

    // Get probe name from whitelist
    let probe_whitelist = cfg.probe_whitelist();
    let probe_name = probe_whitelist
        .get(&report.probe_id)
        .cloned()
        .unwrap_or_else(|| report.probe_id[..8.min(report.probe_id.len())].to_string());

    // Get thresholds (company override or global) and clone them
    let thresholds = cfg
        .alerts
        .company_overrides
        .get(&report.company_id)
        .and_then(|o| o.thresholds.as_ref())
        .cloned()
        .unwrap_or_else(|| cfg.alerts.thresholds.clone());

    // Get alert recipients (company override or global)
    let recipients = cfg
        .alerts
        .company_overrides
        .get(&report.company_id)
        .map(|o| o.alert_emails.clone())
        .unwrap_or_else(|| cfg.alerts.default_recipients.clone());

    if recipients.is_empty() {
        // No recipients configured
        return Ok(());
    }

    let smtp_config = cfg.alerts.smtp.clone();
    drop(cfg); // Release lock before async operations

    // Evaluate each resource
    for resource in &report.resources {
        let resource_key = state::generate_resource_key(
            &report.company_id,
            &report.probe_id,
            &resource.name,
            &resource.target,
        );

        // Check if silenced
        if silence::is_resource_silenced(pool, &resource_key).await? {
            // Silenced - update state but don't send alert
            let status = format!("{:?}", resource.status);
            state::update_alert_state(pool, &resource_key, &status, false).await?;
            continue;
        }

        // Evaluate resource status
        evaluate_resource_alert(
            pool,
            &smtp_config,
            &resource_key,
            resource,
            &report.company_id,
            &probe_name,
            &report.hostname,
            &report.site,
            thresholds.clone(),
            &recipients,
        )
        .await?;
    }

    Ok(())
}

async fn evaluate_resource_alert(
    pool: &SqlitePool,
    smtp_config: &crate::config::SmtpConfig,
    resource_key: &str,
    resource: &common::models::ResourceStatus,
    company_id: &str,
    probe_name: &str,
    hostname: &str,
    site: &str,
    thresholds: ThresholdConfig,
    recipients: &[String],
) -> Result<()> {
    let current_status = format!("{:?}", resource.status);

    // Check if we should send an alert
    if !state::should_send_alert(pool, resource_key, &current_status).await? {
        // No state change - update state without sending alert
        state::update_alert_state(pool, resource_key, &current_status, false).await?;
        return Ok(());
    }

    // State changed - determine alert type
    let mut alert_sent = false;

    match resource.status {
        common::models::Health::Down => {
            // Resource went down
            let mut email_data = email::format_resource_down_email(
                company_id,
                probe_name,
                hostname,
                site,
                &resource.name,
                &format!("{:?}", resource.resource_type),
                &resource.target,
                &resource.message,
            );
            email_data.to = recipients.to_vec();

            if let Err(e) = email::send_alert_email(smtp_config, email_data).await {
                eprintln!("[ALERT ERROR] Failed to send down alert: {}", e);
            } else {
                alert_sent = true;
            }
        }
        common::models::Health::Up => {
            // Resource recovered
            let mut email_data = email::format_resource_recovery_email(
                company_id,
                probe_name,
                hostname,
                site,
                &resource.name,
                &format!("{:?}", resource.resource_type),
                &resource.target,
            );
            email_data.to = recipients.to_vec();

            if let Err(e) = email::send_alert_email(smtp_config, email_data).await {
                eprintln!("[ALERT ERROR] Failed to send recovery alert: {}", e);
            } else {
                alert_sent = true;
            }

            // Clear silence on recovery
            if let Err(e) = silence::clear_silence_on_recovery(pool, resource_key).await {
                eprintln!("[ALERT ERROR] Failed to clear silence: {}", e);
            }
        }
    }

    // Check thresholds for metrics
    if let Some(metric_value) = resource.metric_value {
        let threshold_exceeded = match resource.resource_type {
            common::models::ResourceType::LocalDisk => {
                metric_value >= thresholds.disk_percent as f64
            }
            common::models::ResourceType::LocalCpu => {
                metric_value >= thresholds.cpu_percent as f64
            }
            common::models::ResourceType::LocalMem => {
                metric_value >= thresholds.mem_percent as f64
            }
            _ => false,
        };

        if threshold_exceeded {
            let threshold = match resource.resource_type {
                common::models::ResourceType::LocalDisk => thresholds.disk_percent,
                common::models::ResourceType::LocalCpu => thresholds.cpu_percent,
                common::models::ResourceType::LocalMem => thresholds.mem_percent,
                _ => 0,
            };

            let metric_type = match resource.resource_type {
                common::models::ResourceType::LocalDisk => "disk",
                common::models::ResourceType::LocalCpu => "cpu",
                common::models::ResourceType::LocalMem => "memory",
                _ => "metric",
            };

            // Check if we already sent a threshold alert
            let threshold_key = format!("{}-threshold", resource_key);
            if state::should_send_alert(pool, &threshold_key, "threshold_exceeded").await? {
                let mut email_data = email::format_threshold_email(
                    company_id,
                    probe_name,
                    hostname,
                    site,
                    &resource.name,
                    metric_type,
                    metric_value,
                    threshold,
                );
                email_data.to = recipients.to_vec();

                if let Err(e) = email::send_alert_email(smtp_config, email_data).await {
                    eprintln!("[ALERT ERROR] Failed to send threshold alert: {}", e);
                } else {
                    state::update_alert_state(pool, &threshold_key, "threshold_exceeded", true).await?;
                }
            }
        }
    }

    // Update state
    state::update_alert_state(pool, resource_key, &current_status, alert_sent).await?;

    Ok(())
}

/// Evaluate probe expiry alerts
pub async fn evaluate_probe_expiry(
    pool: &SqlitePool,
    config: Arc<RwLock<HubConfig>>,
    expired_probes: &[(String, String)], // (probe_id, company_id)
    timeout_secs: u64,
) -> Result<()> {
    let cfg = config.read().await;

    // Skip if alerts not enabled
    if !cfg.alerts.enabled {
        return Ok(());
    }

    let probe_whitelist = cfg.probe_whitelist();
    let smtp_config = cfg.alerts.smtp.clone();
    drop(cfg); // Release lock

    for (probe_id, company_id) in expired_probes {
        // Get probe name
        let probe_name = probe_whitelist
            .get(probe_id)
            .cloned()
            .unwrap_or_else(|| probe_id[..8.min(probe_id.len())].to_string());

        // Check if already alerted for this probe expiry
        let expiry_key = format!("probe_expiry:{}:{}", company_id, probe_id);

        if !state::should_send_alert(pool, &expiry_key, "expired").await? {
            continue;
        }

        // Get recipients
        let cfg = config.read().await;
        let recipients = cfg
            .alerts
            .company_overrides
            .get(company_id)
            .map(|o| o.alert_emails.clone())
            .unwrap_or_else(|| cfg.alerts.default_recipients.clone());
        drop(cfg);

        if recipients.is_empty() {
            continue;
        }

        // Get last seen time
        let last_seen = "Unknown"; // TODO: Get from probe_heartbeats table

        let mut email_data = email::format_probe_expiry_email(
            company_id,
            probe_id,
            &probe_name,
            last_seen,
            timeout_secs,
        );
        email_data.to = recipients;

        if let Err(e) = email::send_alert_email(&smtp_config, email_data).await {
            eprintln!("[ALERT ERROR] Failed to send probe expiry alert: {}", e);
        } else {
            state::update_alert_state(pool, &expiry_key, "expired", true).await?;
        }
    }

    Ok(())
}
