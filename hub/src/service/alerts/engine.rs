//! When to alert: Up/Down transitions, usage thresholds and probe expiry.
//! A check's alert state and silence are keyed by its ID (D-49). Both a
//! check's Up/Down state and its threshold change only after enough results
//! in a row ([`flap`], D-63). Covered by `hub/tests/alerts.rs`.

use super::flap::{self, FlapState, Transition};

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

/// The evaluation of each probe's latest report, by probe ID: the next one
/// waits for it (D-63). Entries stay (one per probe); a finished one is
/// waited for at once.
static IN_ORDER: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Receiver<()>>>,
> = std::sync::LazyLock::new(Default::default);

/// Evaluate a report's alerts in the background, so the probe gets its
/// answer at once, **after** the evaluation of that probe's previous report.
/// A check's state counts results in a row (D-63): two reports evaluated at
/// the same time (a probe delivering its buffer after an outage) would both
/// read the same count and lose one. The order is taken here, when the
/// report arrives, not when the task happens to start.
pub fn spawn_in_order(
    storage: Arc<dyn Storage>,
    notifier: Arc<dyn Notifier>,
    config: Arc<RwLock<HubConfig>>,
    report: StatusReport,
    checks: Vec<CheckRecord>,
    now: DateTime<Utc>,
) -> tokio::task::JoinHandle<()> {
    let (done, finished) = tokio::sync::oneshot::channel();
    let previous = IN_ORDER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(report.probe_id.clone(), finished);
    tokio::spawn(async move {
        if let Some(previous) = previous {
            let _ = previous.await; // done, or its task ended: either way, our turn
        }
        if let Err(e) = evaluate_alerts_for_report(
            storage.as_ref(),
            notifier.as_ref(),
            &config,
            &report,
            &checks,
            now,
        )
        .await
        {
            tracing::error!("Failed to evaluate alerts: {}", e);
        }
        let _ = done.send(());
    })
}

/// Evaluate alerts for a status report received at `now`, whose results
/// are for `checks` (the probe's checks, as ingest found them). One report
/// of a probe at a time: [`spawn_in_order`].
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
        let silenced = silence::is_check_silenced(store, &check.id, now).await?;
        evaluate_check_alert(store, notifier, &ctx, check, result, silenced, now).await?;
    }

    Ok(())
}

/// Store `after` if it says more than `before`: a check that was never
/// anything but fine leaves no state.
async fn keep(
    store: &dyn AlertStore,
    key: &str,
    statuses: state::Statuses,
    before: Option<FlapState>,
    after: FlapState,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    let unchanged = before.unwrap_or_default() == after;
    if unchanged && !alert_sent {
        return Ok(());
    }
    state::save_flap_state(store, key, statuses, after, alert_sent, now).await
}

async fn evaluate_check_alert(
    store: &dyn AlertStore,
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    check: &CheckRecord,
    result: &CheckResult,
    silenced: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    let key = check.id.as_str();
    let before = state::flap_state(store, key, state::UP_DOWN).await?;
    let (after, transition) = flap::next(before, result.status == Health::Down, check.alert_after);
    let alert_sent = match transition {
        Transition::None => false,
        // Silenced: the state still follows, but nobody is told.
        Transition::Failed if silenced => false,
        Transition::Failed => send_down_alert(notifier, ctx, check, result).await,
        Transition::Recovered => {
            // A recovery ends the silence (it was for this outage). No recovery
            // alert for an outage that was silenced.
            if let Err(e) = silence::clear_silence_on_recovery(store, &check.id).await {
                tracing::error!("Failed to clear silence: {}", e);
            }
            !silenced && send_recovery_alert(notifier, ctx, check).await
        }
    };
    keep(store, key, state::UP_DOWN, before, after, alert_sent, now).await?;
    if silenced {
        return Ok(());
    }

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

/// The check is confirmed down; `true` if the alert went out.
async fn send_down_alert(
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    check: &CheckRecord,
    result: &CheckResult,
) -> bool {
    let email = email::format_resource_down_email(
        &ctx.source,
        &check.name,
        check.spec.kind(),
        &check.spec.target(),
        &result.message,
    );
    ctx.send(notifier, email, "down").await
}

/// The check is confirmed up again; `true` if the alert went out.
async fn send_recovery_alert(
    notifier: &dyn Notifier,
    ctx: &ReportContext<'_>,
    check: &CheckRecord,
) -> bool {
    let email = email::format_resource_recovery_email(
        &ctx.source,
        &check.name,
        check.spec.kind(),
        &check.spec.target(),
    );
    ctx.send(notifier, email, "recovery").await
}

/// Threshold state per check: alert once its usage is over the limit for
/// enough reports in a row (D-63), stay quiet while it stays over, re-arm
/// once it is back under for enough reports (D-33). Nothing is written for
/// checks that never exceed their limit.
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
    let key = state::threshold_key(&check.id);
    let before = state::flap_state(store, &key, state::THRESHOLD).await?;
    let over = value >= f64::from(threshold);
    let (after, transition) = flap::next(before, over, check.alert_after);
    if transition == Transition::Failed {
        let email =
            email::format_threshold_email(&ctx.source, &check.name, metric, value, threshold);
        // Recorded only once sent, so a failed send is retried on the next report.
        if !ctx.send(notifier, email, "threshold").await {
            return Ok(());
        }
        return keep(store, &key, state::THRESHOLD, before, after, true, now).await;
    }
    // Back under it re-arms without an e-mail.
    keep(store, &key, state::THRESHOLD, before, after, false, now).await
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
