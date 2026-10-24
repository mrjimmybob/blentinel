//! When to alert: a check's status changing (Up, Warning, Down, as the hub
//! judged each result with its thresholds, D-64) and probe expiry. A check's
//! alert state and silence are keyed by its ID (D-49); its status changes
//! only after enough results in a row ([`flap`], D-63). What it decides goes
//! to the [`Notifier`] as a [`Notification`]; the routing rules decide where
//! (D-66). Covered by `hub/tests/alerts.rs`.

use super::flap::{self, FlapState};

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use common::checks::CheckResult;
use common::models::StatusReport;

use super::{Notifier, silence, state};
use crate::model::{AlertKind, Evaluation, Status, short_probe_id};
use crate::service::incidents;
use crate::service::notify::{CheckRef, Notification};
use crate::storage::{CheckRecord, Storage};

/// Where an alert happens, with the names people know: the start of every
/// notification about a probe.
#[derive(Debug, Clone)]
struct Place {
    company_id: String,
    company_name: String,
    site_id: String,
    site_name: String,
    probe_id: String,
    probe_name: String,
    hostname: String,
}

impl Place {
    /// The names of a probe's company, site and itself.
    async fn of(
        store: &dyn Storage,
        company_id: &str,
        site_id: &str,
        probe_id: &str,
        hostname: &str,
    ) -> Result<Self> {
        let company = store.company(company_id).await?;
        let site_name = company
            .as_ref()
            .and_then(|c| c.sites.iter().find(|s| s.id == site_id))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| site_id.to_string());
        let probe_name = match store.probe_record(probe_id).await? {
            Some(record) if !record.name.is_empty() => record.name,
            _ => short_probe_id(probe_id).to_string(),
        };
        Ok(Self {
            company_id: company_id.to_string(),
            company_name: company.map_or_else(|| company_id.to_string(), |c| c.company.name),
            site_id: site_id.to_string(),
            site_name,
            probe_id: probe_id.to_string(),
            probe_name,
            hostname: hostname.to_string(),
        })
    }

    fn notification(
        &self,
        kind: AlertKind,
        check: Option<CheckRef>,
        detail: String,
        at: DateTime<Utc>,
    ) -> Notification {
        Notification {
            kind,
            at,
            company_id: self.company_id.clone(),
            company_name: self.company_name.clone(),
            site_id: self.site_id.clone(),
            site_name: self.site_name.clone(),
            probe_id: self.probe_id.clone(),
            probe_name: self.probe_name.clone(),
            hostname: self.hostname.clone(),
            check,
            detail,
            test: false,
            incident: None,
            nudge: None,
        }
    }

    /// The incident's subject: this place, and `check` (`None`: the probe).
    fn subject(&self, check: Option<(String, String)>) -> incidents::Subject {
        incidents::Subject {
            company_id: self.company_id.clone(),
            company_name: self.company_name.clone(),
            site_id: self.site_id.clone(),
            site_name: self.site_name.clone(),
            probe_id: self.probe_id.clone(),
            probe_name: self.probe_name.clone(),
            check,
        }
    }
}

/// Hand `notification` to the notifier; `true` if it took it.
async fn notify(notifier: &dyn Notifier, notification: Notification) -> bool {
    let subject = notification.subject();
    match notifier.notify(notification).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("Can't send the alert {subject}: {e:#}");
            false
        }
    }
}

/// Each probe's latest evaluation, by probe ID: the next one waits for it
/// (D-63). Its reports' alerts and its expiry take turns here. Entries stay
/// (one per probe); a finished one is waited for at once.
static IN_ORDER: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Receiver<()>>>,
> = std::sync::LazyLock::new(Default::default);

/// A probe's turn to evaluate its alerts (D-63). Taken in order ([`Turn::take`]),
/// waited for ([`Turn::wait`]), and given up when dropped, which lets the
/// next one go, also if the evaluation fails or panics.
pub struct Turn {
    previous: Option<tokio::sync::oneshot::Receiver<()>>,
    _done: tokio::sync::oneshot::Sender<()>,
}

impl Turn {
    /// The turn after the latest one taken for `probe_id`. The order is set
    /// here, when it is taken, not when its task happens to start.
    pub fn take(probe_id: &str) -> Self {
        let (done, finished) = tokio::sync::oneshot::channel();
        let previous = IN_ORDER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(probe_id.to_string(), finished);
        Self {
            previous,
            _done: done,
        }
    }

    /// Wait until the turn before this one is done.
    pub async fn wait(&mut self) {
        if let Some(previous) = self.previous.take() {
            let _ = previous.await; // done, or its task ended: either way, our turn
        }
    }
}

/// Evaluate a report's alerts in the background, so the probe gets its
/// answer at once, **after** the evaluation of that probe's previous report.
/// A check's state counts results in a row (D-63): two reports evaluated at
/// the same time (a probe delivering its buffer after an outage) would both
/// read the same count and lose one. The order is taken here, when the
/// report arrives, not when the task happens to start.
pub fn spawn_in_order(
    storage: Arc<dyn Storage>,
    notifier: Arc<dyn Notifier>,
    report: StatusReport,
    evaluations: Vec<Evaluation>,
    checks: Vec<CheckRecord>,
    now: DateTime<Utc>,
) -> tokio::task::JoinHandle<()> {
    let mut turn = Turn::take(&report.probe_id);
    tokio::spawn(async move {
        turn.wait().await;
        if let Err(e) = evaluate_alerts_for_report(
            storage.as_ref(),
            notifier.as_ref(),
            &report,
            &evaluations,
            &checks,
            now,
        )
        .await
        {
            tracing::error!("Failed to evaluate alerts: {}", e);
        }
        drop(turn); // this probe's next evaluation may go
    })
}

/// Evaluate alerts for a status report received at `now`, whose results
/// are for `checks` (the probe's checks, as ingest found them) and were
/// judged as `evaluations` (in the same order, D-64). One report of a probe
/// at a time: [`spawn_in_order`].
pub async fn evaluate_alerts_for_report(
    store: &dyn Storage,
    notifier: &dyn Notifier,
    report: &StatusReport,
    evaluations: &[Evaluation],
    checks: &[CheckRecord],
    now: DateTime<Utc>,
) -> Result<()> {
    let place = Place::of(
        store,
        &report.company_id,
        &report.site,
        &report.probe_id,
        &report.hostname,
    )
    .await?;
    // What is unreachable behind each check that is down (D-68), by its ID.
    let mut behind: std::collections::HashMap<&str, Vec<String>> = Default::default();
    for (result, evaluation) in report.results.iter().zip(evaluations) {
        if let (Some(root), Some(check)) = (
            evaluation.behind.as_deref(),
            checks.iter().find(|c| c.id == result.check_id),
        ) {
            behind.entry(root).or_default().push(check.name.clone());
        }
    }
    for (result, evaluation) in report.results.iter().zip(evaluations) {
        // Ingest keeps only results of the probe's checks.
        let Some(check) = checks.iter().find(|c| c.id == result.check_id) else {
            continue;
        };
        let silenced = store
            .is_silenced(
                &report.company_id,
                &report.site,
                &report.probe_id,
                Some(check.id.as_str()),
                now,
            )
            .await?;
        let judged = Judged {
            result,
            evaluation,
            behind_it: behind.get(check.id.as_str()).map_or(&[], Vec::as_slice),
        };
        evaluate_check_alert(store, notifier, &place, check, judged, silenced, now).await?;
    }
    Ok(())
}

/// Store `after` if it says more than `before`: a check that was never
/// anything but fine leaves no state.
async fn keep(
    store: &dyn Storage,
    key: &str,
    before: Option<FlapState>,
    after: FlapState,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    let unchanged = before.unwrap_or_default() == after;
    if unchanged && !alert_sent {
        return Ok(());
    }
    state::save_flap_state(store, key, after, alert_sent, now).await
}

/// A result and the hub's verdict on it.
#[derive(Clone, Copy)]
struct Judged<'a> {
    result: &'a CheckResult,
    evaluation: &'a Evaluation,
    /// The checks unreachable behind this one in the same report (D-68).
    behind_it: &'a [String],
}

impl Judged<'_> {
    /// What went wrong, in words: the threshold's reason, else the probe's
    /// message; and what is unreachable behind it.
    fn why(&self) -> String {
        let why = self
            .evaluation
            .reason
            .as_deref()
            .unwrap_or(&self.result.message);
        match self.behind_it {
            [] => why.to_string(),
            names => format!("{why} · unreachable behind it: {}", names.join(", ")),
        }
    }
}

async fn evaluate_check_alert(
    store: &dyn Storage,
    notifier: &dyn Notifier,
    place: &Place,
    check: &CheckRecord,
    judged: Judged<'_>,
    silenced: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    let status = judged.evaluation.status;
    // Unreachable behind a check that is down (D-68): the result says
    // nothing about this check, so its state stays as it was.
    if status == Status::Unknown {
        return Ok(());
    }
    let key = check.id.as_str();
    let before = state::flap_state(store, key).await?;
    let (after, news) = match flap::next(before, status, check.alert_after) {
        // A maintenance window holds the news back until it closes (D-65).
        (_, Some(_)) if judged.evaluation.in_maintenance => {
            (flap::held(before, status, check.alert_after), None)
        }
        next => next,
    };
    let kind = match news {
        None => None,
        Some(Status::Up) => {
            // A recovery ends the silence (it was for this outage). No recovery
            // alert for an outage that was silenced.
            if let Err(e) = silence::clear_silence_on_recovery(store, &check.id).await {
                tracing::error!("Failed to clear silence: {}", e);
            }
            (!silenced).then_some(AlertKind::Recovered)
        }
        // Silenced: the state still follows, but nobody is told.
        Some(_) if silenced => None,
        Some(Status::Warning) => Some(AlertKind::Warning),
        Some(_) => Some(AlertKind::Down),
    };
    // The incident follows the news, told or not (D-67).
    let incident = match news {
        None => None,
        Some(status) => {
            let why = match status {
                Status::Up => judged.result.message.clone(),
                _ => judged.why(),
            };
            let subject = place.subject(Some((check.id.to_string(), check.name.clone())));
            incidents::on_check_news(store, &subject, status, &why, silenced, now)
                .await
                .unwrap_or_else(|e| {
                    tracing::error!(check = %check.name, "Can't record the incident: {e:#}");
                    None
                })
        }
    };
    let alert_sent = match kind {
        None => false,
        Some(kind) => {
            let detail = match kind {
                AlertKind::Recovered => judged.result.message.clone(),
                _ => judged.why(),
            };
            let check = CheckRef {
                id: check.id.to_string(),
                name: check.name.clone(),
                kind: check.spec.kind().to_string(),
                target: check.spec.target(),
            };
            let notification = Notification {
                incident,
                ..place.notification(kind, Some(check), detail, now)
            };
            notify(notifier, notification).await
        }
    };
    keep(store, key, before, after, alert_sent, now).await
}

/// Alert on expired probes (`(probe_id, company_id)` pairs) not alerted yet.
///
/// Every sweep evaluates every expired probe, in the background. Each probe's
/// check ("not alerted yet?") and record ("alerted") happen in that probe's
/// turn (D-63), so two sweeps' evaluations that overlap can't both find it
/// not alerted and send twice (CI run 40082566575). Turns are per probe:
/// other probes' evaluations don't wait.
pub async fn evaluate_probe_expiry(
    store: &dyn Storage,
    notifier: &dyn Notifier,
    expired_probes: &[(String, String)],
    timeout_secs: u64,
    now: DateTime<Utc>,
) -> Result<()> {
    for (probe_id, company_id) in expired_probes {
        // Held to the end of this iteration, past the record and the send.
        let mut turn = Turn::take(probe_id);
        turn.wait().await;
        let expiry_key = state::probe_expiry_key(company_id, probe_id);
        if !state::should_send_alert(store, &expiry_key, state::PROBE_EXPIRED).await? {
            continue;
        }
        // In a window or silenced: not yet. Every sweep looks again, so it
        // alerts once the window closes if the probe is still silent (D-65).
        if crate::service::maintenance::probe_is_quiet(store, probe_id, now).await? {
            continue;
        }
        let site = store
            .probe_record(probe_id)
            .await?
            .map(|r| r.site_id)
            .unwrap_or_default();
        let place = Place::of(store, company_id, &site, probe_id, "").await?;
        let why = format!("no report for more than {timeout_secs} s");
        let incident = incidents::on_probe_expired(store, &place.subject(None), &why, now)
            .await
            .map_err(
                |e| tracing::error!(probe = %place.probe_name, "Can't record the incident: {e:#}"),
            )
            .ok();
        let notification = Notification {
            incident,
            ..place.notification(AlertKind::ProbeExpired, None, why, now)
        };

        // Record the alert *before* sending. Sending can be slow, and if the
        // probe reports again meanwhile, its report re-arms this state; a
        // write after the send would overwrite that and suppress the probe's
        // next outage.
        state::update_alert_state(store, &expiry_key, state::PROBE_EXPIRED, true, now).await?;
        notify(notifier, notification).await;
    }
    Ok(())
}
