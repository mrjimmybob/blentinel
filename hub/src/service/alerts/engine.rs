//! When to alert: Up/Down transitions, usage thresholds and probe expiry.
//! A check's alert state and silence are keyed by its ID (D-49).
//! Covered by `hub/tests/alerts.rs`.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use common::checks::{CheckResult, CheckSpec, Health, metric};
use common::models::StatusReport;
use tokio::sync::RwLock;

use super::{Notifier, email, silence, state};
use crate::config::{HubConfig, ThresholdConfig};
use crate::model::short_probe_id;
use crate::storage::{AlertStore, CheckRecord, Storage};

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

/// Name of a probe (its record's), else its short ID.
async fn probe_name(store: &dyn Storage, probe_id: &str) -> String {
    match store.probe_record(probe_id).await {
        Ok(Some(record)) => record.name,
        _ => short_probe_id(probe_id).to_string(),
    }
}

/// Evaluate alerts for a status report received at `now`, whose results
/// are for `checks` (the probe's checks, as ingest found them).
pub async fn evaluate_alerts_for_report(
    store: &dyn Storage,
    notifier: &dyn Notifier,
    config: &Arc<RwLock<HubConfig>>,
    report: &StatusReport,
    checks: &[CheckRecord],
    now: DateTime<Utc>,
) -> Result<()> {
    let name = probe_name(store, &report.probe_id).await;
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
        (name, thresholds, recipients_for(&cfg, &report.company_id))
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

    for result in &report.results {
        // Ingest keeps only results of the probe's checks.
        let Some(check) = checks.iter().find(|c| c.id == result.check_id) else {
            continue;
        };
        let key = check.id.as_str();

        if silence::is_check_silenced(store, &check.id, now).await? {
            // Silenced: track the state, but don't notify. A recovery still ends
            // the silence (it was for this outage); no recovery alert is sent for
            // an outage nobody was alerted about.
            let status = result.status.as_str();
            let recovered =
                result.status == Health::Up && state::should_send_alert(store, key, status).await?;
            if recovered {
                silence::clear_silence_on_recovery(store, &check.id).await?;
            }
            state::update_alert_state(store, key, status, false, now).await?;
            continue;
        }

        evaluate_check_alert(store, notifier, &ctx, check, result, now).await?;
    }

    Ok(())
}

async fn evaluate_check_alert(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    check: &CheckRecord,
    result: &CheckResult,
    now: DateTime<Utc>,
) -> Result<()> {
    let key = check.id.as_str();
    let current_status = result.status.as_str();
    let status_changed = state::should_send_alert(store, key, current_status).await?;

    let alert_sent = if status_changed {
        send_status_alert(store, notifier, ctx, check, result).await
    } else {
        false
    };
    state::update_alert_state(store, key, current_status, alert_sent, now).await?;

    // Thresholds are their own state, independent of Up/Down (D-33): local
    // metrics are always "Up", so tying them to status changes meant never.
    if let Some((name, threshold)) = threshold_for(&check.spec, ctx.thresholds)
        && let Some(usage) = result.metric(metric::USAGE)
    {
        evaluate_threshold(
            store,
            notifier,
            ctx,
            check,
            name,
            usage.value,
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
    check: &CheckRecord,
    result: &CheckResult,
) -> bool {
    let (kind, target) = (check.spec.kind(), check.spec.target());
    match result.status {
        Health::Down => {
            let email = email::format_resource_down_email(
                &ctx.source,
                &check.name,
                kind,
                &target,
                &result.message,
            );
            ctx.send(notifier, email, "down").await
        }
        Health::Up => {
            let email =
                email::format_resource_recovery_email(&ctx.source, &check.name, kind, &target);
            let sent = ctx.send(notifier, email, "recovery").await;
            if let Err(e) = silence::clear_silence_on_recovery(store, &check.id).await {
                tracing::error!("Failed to clear silence: {}", e);
            }
            sent
        }
    }
}

/// Threshold state per check: alert when its usage crosses the limit, stay
/// quiet while it stays over, re-arm when it drops back below (D-33).
/// Nothing is written for checks that never exceed their limit.
#[allow(clippy::too_many_arguments)] // engine context (store, notifier, ctx, now) + one metric
async fn evaluate_threshold(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    check: &CheckRecord,
    metric: &str,
    value: f64,
    threshold: u32,
    now: DateTime<Utc>,
) -> Result<()> {
    let threshold_key = state::threshold_key(&check.id);

    if value < f64::from(threshold) {
        // Back under the limit: re-arm (no-op if it never fired).
        store
            .set_alert_status_if_present(&threshold_key, state::THRESHOLD_OK)
            .await?;
        return Ok(());
    }

    if state::should_send_alert(store, &threshold_key, state::THRESHOLD_EXCEEDED).await? {
        let email =
            email::format_threshold_email(&ctx.source, &check.name, metric, value, threshold);
        // Recorded only once sent, so a failed send is retried on the next report.
        if ctx.send(notifier, email, "threshold").await {
            state::update_alert_state(store, &threshold_key, state::THRESHOLD_EXCEEDED, true, now)
                .await?;
        }
    }
    Ok(())
}

/// The name and configured threshold for a check's usage metric, if its kind has one.
fn threshold_for(spec: &CheckSpec, thresholds: &ThresholdConfig) -> Option<(&'static str, u32)> {
    match spec {
        CheckSpec::Disk { .. } => Some(("disk", thresholds.disk_percent)),
        CheckSpec::Cpu => Some(("cpu", thresholds.cpu_percent)),
        CheckSpec::Memory => Some(("memory", thresholds.mem_percent)),
        _ => None,
    }
}

/// Alert on probes that just stopped reporting (`(probe_id, company_id)` pairs).
pub async fn evaluate_probe_expiry(
    store: &dyn Storage,
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

        let name = probe_name(store, probe_id).await;
        let recipients = recipients_for(&*config.read().await, company_id);
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
