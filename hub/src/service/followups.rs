//! Reminders and escalation (M3-06, proposal 0005 §2.3, D-69): while an
//! incident is open and nobody acknowledged it, the routing rules its alert
//! matches remind their channels every so often, and escalate it once to
//! more channels. Acknowledging stops both; so does recovery.
//!
//! [`sweep`] runs every few seconds and works it all out from the incidents
//! in the database (when each was opened, last reminded, escalated), so a
//! restart neither loses nor repeats anything. Nothing is sent about an
//! incident that opened silenced (nobody was told in the first place), nor
//! while a silence or a maintenance window covers it.

use anyhow::Result;
use chrono::{DateTime, Utc};
use common::checks::CheckId;

use crate::model::{IncidentEventKind, follow_up};
use crate::service::alerts::Notifier;
use crate::service::incidents::alert_kind;
use crate::service::maintenance;
use crate::service::notify::{CheckRef, IncidentRef, Notification, Nudge};
use crate::storage::{Storage, StoredIncident, StoredIncidentEvent};

/// Is `incident` covered by a silence or an open maintenance window now?
async fn quiet(store: &dyn Storage, incident: &StoredIncident, now: DateTime<Utc>) -> Result<bool> {
    let Some(check_id) = incident.check_id.as_deref() else {
        return maintenance::probe_is_quiet(store, &incident.probe_id, now).await;
    };
    if store
        .is_silenced(
            &incident.company_id,
            &incident.site_id,
            &incident.probe_id,
            Some(check_id),
            now,
        )
        .await?
    {
        return Ok(true);
    }
    let windows = maintenance::open_scopes(store, &incident.company_id, now).await?;
    Ok(windows
        .iter()
        .any(|w| w.covers(&incident.site_id, &incident.probe_id, Some(check_id))))
}

/// `channels` by name, for the timeline.
async fn names(store: &dyn Storage, channels: &[i64]) -> Result<String> {
    let all = store.channels().await?;
    Ok(channels
        .iter()
        .filter_map(|id| all.iter().find(|c| c.id == *id))
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join(", "))
}

/// The alert about `incident`, as it stands, with `nudge`.
async fn notification(
    store: &dyn Storage,
    incident: &StoredIncident,
    check: Option<CheckRef>,
    nudge: Nudge,
    now: DateTime<Utc>,
) -> Result<Notification> {
    // Why: what its timeline last said about it.
    let detail = store
        .incident_events(incident.id)
        .await?
        .into_iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind,
                IncidentEventKind::Opened | IncidentEventKind::Changed
            )
        })
        .map(|e| e.text)
        .unwrap_or_default();
    Ok(Notification {
        kind: alert_kind(incident),
        at: now,
        company_id: incident.company_id.clone(),
        company_name: incident.company_name.clone(),
        site_id: incident.site_id.clone(),
        site_name: incident.site_name.clone(),
        probe_id: incident.probe_id.clone(),
        probe_name: incident.probe_name.clone(),
        hostname: String::new(),
        check,
        detail,
        test: false,
        incident: Some(IncidentRef {
            id: incident.id,
            opened_at: incident.opened_at,
            escalated: incident.escalated_at.map(|_| alert_kind(incident)),
        }),
        nudge: Some(nudge),
    })
}

/// Send the reminders and escalations that are due at `now`.
pub async fn sweep(store: &dyn Storage, notifier: &dyn Notifier, now: DateTime<Utc>) -> Result<()> {
    let open = store.unacknowledged_incidents().await?;
    if open.is_empty() {
        return Ok(());
    }
    let rules = store.route_rules().await?;
    for incident in open {
        if quiet(store, &incident, now).await? {
            continue;
        }
        let check = match &incident.check_id {
            Some(id) => store.check(&CheckId(id.clone())).await?.map(|c| CheckRef {
                id: c.id.to_string(),
                name: c.name.clone(),
                kind: c.spec.kind().to_string(),
                target: c.spec.target(),
            }),
            None => None,
        };
        let follow = follow_up(
            &rules,
            alert_kind(&incident),
            &incident.company_id,
            &incident.site_id,
            check.as_ref().map(|c| c.kind.as_str()),
        );
        let minutes_since = |at: DateTime<Utc>| (now - at).num_minutes();

        if let Some((after, channels)) = follow.escalate
            && incident.escalated_at.is_none()
            && minutes_since(incident.opened_at) >= i64::from(after)
        {
            let event = StoredIncidentEvent {
                at: now,
                kind: IncidentEventKind::Escalated,
                username: None,
                text: format!(
                    "Not acknowledged for {}: escalated to {}",
                    crate::model::duration_text(i64::from(after) * 60),
                    names(store, &channels).await?
                ),
            };
            // Recorded before it is sent: a slow channel can't make it twice.
            if store.record_escalation(incident.id, &event).await? {
                let escalated = StoredIncident {
                    escalated_at: Some(now),
                    ..incident.clone()
                };
                let nudge = Nudge::Escalation {
                    after_minutes: after,
                    channels,
                };
                let n = notification(store, &escalated, check.clone(), nudge, now).await?;
                send(notifier, n).await;
            }
        }

        if let Some((every, channels)) = follow.remind {
            let since = incident.last_reminded_at.unwrap_or(incident.opened_at);
            if minutes_since(since) >= i64::from(every) {
                let count = incident.reminders + 1;
                let event = StoredIncidentEvent {
                    at: now,
                    kind: IncidentEventKind::Reminded,
                    username: None,
                    text: format!("Reminder {count} to {}", names(store, &channels).await?),
                };
                store.record_reminder(incident.id, &event).await?;
                let nudge = Nudge::Reminder { count, channels };
                let n = notification(store, &incident, check, nudge, now).await?;
                send(notifier, n).await;
            }
        }
    }
    Ok(())
}

async fn send(notifier: &dyn Notifier, n: Notification) {
    let subject = n.subject();
    if let Err(e) = notifier.notify(n).await {
        tracing::error!("Can't send {subject}: {e:#}");
    }
}
