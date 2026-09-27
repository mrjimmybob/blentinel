//! When to alert: Up/Down transitions, metric thresholds and probe expiry.
//! Covered by `hub/tests/alerts.rs`.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use common::models::{Health, ResourceStatus, ResourceType, StatusReport};
use tokio::sync::RwLock;

use super::{Notifier, email, silence, state};
use crate::config::{HubConfig, ThresholdConfig};
use crate::model::short_probe_id;
use crate::storage::AlertStore;

/// Who an alert about one report concerns and who receives it.
struct ReportContext<'a> {
    source: email::AlertSource<'a>,
    thresholds: &'a ThresholdConfig,
    recipients: &'a [String],
}

impl ReportContext<'_> {
    /// Send `email` to this context's recipients; `true` if it went out.
    async fn send(
        &self,
        notifier: &dyn Notifier,
        mut email: email::AlertEmail,
        what: &str,
    ) -> bool {
        email.to = self.recipients.to_vec();
        match notifier.send(email).await {
            Ok(()) => true,
            Err(e) => {
                tracing::error!("Failed to send {} alert: {}", what, e);
                false
            }
        }
    }
}

/// Recipients for a company: its override list, else the global default.
fn recipients_for(cfg: &HubConfig, company_id: &str) -> Vec<String> {
    cfg.alerts
        .company_overrides
        .get(company_id)
        .map(|o| o.alert_emails.clone())
        .unwrap_or_else(|| cfg.alerts.default_recipients.clone())
}

/// Name of a probe in the whitelist, else its short ID.
fn probe_name(cfg: &HubConfig, probe_id: &str) -> String {
    cfg.probe_whitelist()
        .get(probe_id)
        .cloned()
        .unwrap_or_else(|| short_probe_id(probe_id).to_string())
}

/// Evaluate alerts for a status report received at `now`.
pub async fn evaluate_alerts_for_report(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    config: &Arc<RwLock<HubConfig>>,
    report: &StatusReport,
    now: DateTime<Utc>,
) -> Result<()> {
    let (probe_name, thresholds, recipients) = {
        let cfg = config.read().await;
        if !cfg.alerts.enabled {
            return Ok(());
        }
        let thresholds = cfg
            .alerts
            .company_overrides
            .get(&report.company_id)
            .and_then(|o| o.thresholds.clone())
            .unwrap_or_else(|| cfg.alerts.thresholds.clone());
        (
            probe_name(&cfg, &report.probe_id),
            thresholds,
            recipients_for(&cfg, &report.company_id),
        )
    }; // config lock released before any I/O

    if recipients.is_empty() {
        return Ok(());
    }

    let ctx = ReportContext {
        source: email::AlertSource {
            company_id: &report.company_id,
            probe_name: &probe_name,
            hostname: &report.hostname,
            site: &report.site,
        },
        thresholds: &thresholds,
        recipients: &recipients,
    };

    for resource in &report.resources {
        let resource_key = state::resource_key(
            &report.company_id,
            &report.probe_id,
            &resource.name,
            &resource.target,
        );

        if silence::is_resource_silenced(store, &resource_key, now).await? {
            // Silenced: track the state, but don't notify. A recovery still ends
            // the silence (it was for this outage); no recovery alert is sent for
            // an outage nobody was alerted about.
            let status = format!("{:?}", resource.status);
            let recovered = matches!(resource.status, Health::Up)
                && state::should_send_alert(store, &resource_key, &status).await?;
            if recovered {
                silence::clear_silence_on_recovery(store, &resource_key).await?;
            }
            state::update_alert_state(store, &resource_key, &status, false, now).await?;
            continue;
        }

        evaluate_resource_alert(store, notifier, &ctx, &resource_key, resource, now).await?;
    }

    Ok(())
}

async fn evaluate_resource_alert(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    resource_key: &str,
    resource: &ResourceStatus,
    now: DateTime<Utc>,
) -> Result<()> {
    let current_status = format!("{:?}", resource.status);
    let status_changed = state::should_send_alert(store, resource_key, &current_status).await?;

    let alert_sent = if status_changed {
        send_status_alert(store, notifier, ctx, resource_key, resource).await
    } else {
        false
    };
    state::update_alert_state(store, resource_key, &current_status, alert_sent, now).await?;

    // Thresholds are their own state, independent of Up/Down (D-33): local
    // metrics are always "Up", so tying them to status changes meant never.
    if let Some(value) = resource.metric_value
        && let Some((metric, threshold)) = threshold_for(resource.resource_type, ctx.thresholds)
    {
        evaluate_threshold(
            store,
            notifier,
            ctx,
            resource_key,
            &resource.name,
            metric,
            value,
            threshold,
            now,
        )
        .await?;
    }
    Ok(())
}

/// Alert on a Down or Up (recovery) transition; `true` if the alert went out.
async fn send_status_alert(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    resource_key: &str,
    resource: &ResourceStatus,
) -> bool {
    let resource_type = format!("{:?}", resource.resource_type);
    match resource.status {
        Health::Down => {
            let email = email::format_resource_down_email(
                &ctx.source,
                &resource.name,
                &resource_type,
                &resource.target,
                &resource.message,
            );
            ctx.send(notifier, email, "down").await
        }
        Health::Up => {
            let email = email::format_resource_recovery_email(
                &ctx.source,
                &resource.name,
                &resource_type,
                &resource.target,
            );
            let sent = ctx.send(notifier, email, "recovery").await;
            if let Err(e) = silence::clear_silence_on_recovery(store, resource_key).await {
                tracing::error!("Failed to clear silence: {}", e);
            }
            sent
        }
    }
}

/// Threshold state per resource: alert when the metric crosses the limit, stay
/// quiet while it stays over, re-arm when it drops back below (D-33).
/// Nothing is written for metrics that never exceed their limit.
#[allow(clippy::too_many_arguments)] // engine context (store, notifier, ctx, now) + one metric
async fn evaluate_threshold(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    resource_key: &str,
    resource_name: &str,
    metric: &str,
    value: f64,
    threshold: u32,
    now: DateTime<Utc>,
) -> Result<()> {
    let threshold_key = state::threshold_key(resource_key);

    if value < f64::from(threshold) {
        // Back under the limit: re-arm (no-op if it never fired).
        store
            .set_alert_status_if_present(&threshold_key, state::THRESHOLD_OK)
            .await?;
        return Ok(());
    }

    if state::should_send_alert(store, &threshold_key, state::THRESHOLD_EXCEEDED).await? {
        let email =
            email::format_threshold_email(&ctx.source, resource_name, metric, value, threshold);
        // Recorded only once sent, so a failed send is retried on the next report.
        if ctx.send(notifier, email, "threshold").await {
            state::update_alert_state(store, &threshold_key, state::THRESHOLD_EXCEEDED, true, now)
                .await?;
        }
    }
    Ok(())
}

/// The metric name and configured threshold for a resource type, if it has one.
fn threshold_for(
    resource_type: ResourceType,
    thresholds: &ThresholdConfig,
) -> Option<(&'static str, u32)> {
    match resource_type {
        ResourceType::LocalDisk => Some(("disk", thresholds.disk_percent)),
        ResourceType::LocalCpu => Some(("cpu", thresholds.cpu_percent)),
        ResourceType::LocalMem => Some(("memory", thresholds.mem_percent)),
        _ => None,
    }
}

/// Alert on probes that just stopped reporting (`(probe_id, company_id)` pairs).
pub async fn evaluate_probe_expiry(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    config: &Arc<RwLock<HubConfig>>,
    expired_probes: &[(String, String)],
    timeout_secs: u64,
    now: DateTime<Utc>,
) -> Result<()> {
    if !config.read().await.alerts.enabled {
        return Ok(());
    }

    for (probe_id, company_id) in expired_probes {
        let expiry_key = state::probe_expiry_key(company_id, probe_id);
        if !state::should_send_alert(store, &expiry_key, state::PROBE_EXPIRED).await? {
            continue;
        }

        let (name, recipients) = {
            let cfg = config.read().await;
            (probe_name(&cfg, probe_id), recipients_for(&cfg, company_id))
        };
        if recipients.is_empty() {
            continue;
        }

        let last_seen = "Unknown"; // TODO: from the probe's heartbeat
        let mut email =
            email::format_probe_expiry_email(company_id, probe_id, &name, last_seen, timeout_secs);
        email.to = recipients;

        // Record the alert *before* sending. Sending can be slow (SMTP), and if
        // the probe reports again meanwhile, its report re-arms this state; a
        // write after the send would overwrite that and suppress the probe's
        // next outage. Nothing is lost: an expired probe isn't re-reported by
        // later sweeps, so a failed send was never retried anyway.
        state::update_alert_state(store, &expiry_key, state::PROBE_EXPIRED, true, now).await?;
        if let Err(e) = notifier.send(email).await {
            tracing::error!("Failed to send probe expiry alert: {}", e);
        }
    }

    Ok(())
}
