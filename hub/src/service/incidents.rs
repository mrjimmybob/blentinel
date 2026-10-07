//! Incidents (M3-05, proposal 0005 §2.3, D-67): one outage of a check, or of
//! a probe that stopped reporting, from its first alert to its recovery, with
//! a timeline, an acknowledgement ("I'm on it") and notes.
//!
//! The alert engine drives them: the news that alerts (after flap
//! suppression and maintenance, D-63, D-65) opens, changes and closes an
//! incident, also when a silence keeps everyone from being told. People
//! acknowledge them and add notes through the API.

use chrono::{DateTime, Utc};

use crate::model::{
    CompanyScope, IncidentDetail, IncidentEvent, IncidentEventKind, IncidentInfo, IncidentNote,
    IncidentsQuery, Status, duration_text,
};
use crate::service::notify::IncidentRef;
use crate::storage::{IncidentStore, StoredIncident, StoredIncidentEvent};

/// The most incidents a list returns.
pub const LIST_LIMIT: u32 = 500;

#[derive(Debug, thiserror::Error)]
pub enum IncidentError {
    #[error("no such incident")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    /// Closed, or acknowledged already.
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// What an incident is about, with the names people know.
#[derive(Debug, Clone)]
pub struct Subject {
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
    pub probe_id: String,
    pub probe_name: String,
    /// `(id, name)`; `None`: the probe itself.
    pub check: Option<(String, String)>,
}

impl Subject {
    fn incident(&self, status: Status, now: DateTime<Utc>) -> StoredIncident {
        StoredIncident {
            id: 0,
            company_id: self.company_id.clone(),
            company_name: self.company_name.clone(),
            site_id: self.site_id.clone(),
            site_name: self.site_name.clone(),
            probe_id: self.probe_id.clone(),
            probe_name: self.probe_name.clone(),
            check_id: self.check.as_ref().map(|(id, _)| id.clone()),
            check_name: self.check.as_ref().map(|(_, name)| name.clone()),
            severity: status,
            status,
            opened_at: now,
            closed_at: None,
            acknowledged_by: None,
            acknowledged_at: None,
        }
    }

    fn check_id(&self) -> Option<&str> {
        self.check.as_ref().map(|(id, _)| id.as_str())
    }
}

fn by_hub(kind: IncidentEventKind, text: String, at: DateTime<Utc>) -> StoredIncidentEvent {
    StoredIncidentEvent {
        at,
        kind,
        username: None,
        text,
    }
}

fn reference(incident: &StoredIncident) -> IncidentRef {
    IncidentRef {
        id: incident.id,
        opened_at: incident.opened_at,
    }
}

/// `status: why`, or just the status.
fn saying(status: Status, why: &str) -> String {
    match why.trim() {
        "" => status.as_str().to_string(),
        why => format!("{}: {why}", status.as_str()),
    }
}

/// A check's news: Down or Warning opens its incident (or changes the open
/// one), Up closes it. `silenced`: nobody is told; the timeline says so.
/// The incident, for the alert to link to.
pub async fn on_check_news(
    store: &dyn IncidentStore,
    subject: &Subject,
    status: Status,
    why: &str,
    silenced: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<IncidentRef>> {
    let open = store
        .open_incident_of(&subject.probe_id, subject.check_id())
        .await?;
    match (status, open) {
        (Status::Up, Some(open)) => {
            let text = match why.trim() {
                "" => "Recovered".to_string(),
                why => format!("Recovered: {why}"),
            };
            store
                .close_incident(open.id, &by_hub(IncidentEventKind::Closed, text, now))
                .await?;
            Ok(Some(reference(&open)))
        }
        (Status::Down | Status::Warning, Some(open)) => {
            if open.status != status {
                let event = by_hub(IncidentEventKind::Changed, saying(status, why), now);
                store.change_incident(open.id, status, &event).await?;
            }
            Ok(Some(reference(&open)))
        }
        (Status::Down | Status::Warning, None) => {
            let mut text = saying(status, why);
            if silenced {
                text.push_str(" (silenced: nobody was told)");
            }
            let incident = subject.incident(status, now);
            let opened = by_hub(IncidentEventKind::Opened, text, now);
            let id = store.open_incident(&incident, &opened).await?;
            Ok(Some(IncidentRef { id, opened_at: now }))
        }
        // Up with nothing open (it was silenced before incidents, or held), or Unknown.
        _ => Ok(None),
    }
}

/// A probe stopped reporting: its incident (opened now, unless one is open).
pub async fn on_probe_expired(
    store: &dyn IncidentStore,
    subject: &Subject,
    why: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<IncidentRef> {
    if let Some(open) = store.open_incident_of(&subject.probe_id, None).await? {
        return Ok(reference(&open));
    }
    let incident = subject.incident(Status::Down, now);
    let opened = by_hub(
        IncidentEventKind::Opened,
        format!("Stopped reporting: {why}"),
        now,
    );
    let id = store.open_incident(&incident, &opened).await?;
    Ok(IncidentRef { id, opened_at: now })
}

/// A probe reports again: its "stopped reporting" incident closes.
pub async fn on_probe_reporting(
    store: &dyn IncidentStore,
    probe_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    if let Some(open) = store.open_incident_of(probe_id, None).await? {
        let event = by_hub(
            IncidentEventKind::Closed,
            "The probe reports again".into(),
            now,
        );
        store.close_incident(open.id, &event).await?;
    }
    Ok(())
}

/// A check was deleted: its open incident closes (its history stays).
pub async fn on_check_deleted(
    store: &dyn IncidentStore,
    probe_id: &str,
    check_id: &str,
    by: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    if let Some(open) = store.open_incident_of(probe_id, Some(check_id)).await? {
        let event = StoredIncidentEvent {
            at: now,
            kind: IncidentEventKind::Closed,
            username: Some(by.to_string()),
            text: "The check was deleted".into(),
        };
        store.close_incident(open.id, &event).await?;
    }
    Ok(())
}

fn format_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// An incident for the API; its duration up to `now` while open.
pub fn info(incident: &StoredIncident, now: DateTime<Utc>) -> IncidentInfo {
    let end = incident.closed_at.unwrap_or(now);
    IncidentInfo {
        id: incident.id,
        company_id: incident.company_id.clone(),
        company_name: incident.company_name.clone(),
        site_id: incident.site_id.clone(),
        site_name: incident.site_name.clone(),
        probe_id: incident.probe_id.clone(),
        probe_name: incident.probe_name.clone(),
        check_id: incident.check_id.clone(),
        check_name: incident.check_name.clone(),
        severity: incident.severity,
        status: incident.status,
        opened_at: format_time(incident.opened_at),
        closed_at: incident.closed_at.map(format_time),
        duration: duration_text((end - incident.opened_at).num_seconds()),
        acknowledged_by: incident.acknowledged_by.clone(),
        acknowledged_at: incident.acknowledged_at.map(format_time),
    }
}

/// Incidents in `scope`: open ones by default.
pub async fn list(
    store: &dyn IncidentStore,
    scope: &CompanyScope,
    query: &IncidentsQuery,
    now: DateTime<Utc>,
) -> Result<Vec<IncidentInfo>, IncidentError> {
    let open = match query.state.as_deref().unwrap_or("open") {
        "open" => Some(true),
        "closed" => Some(false),
        "all" => None,
        other => {
            return Err(IncidentError::Invalid(format!(
                "state is open, closed or all, not {other:?}"
            )));
        }
    };
    let incidents = store.incidents(scope, open, LIST_LIMIT).await?;
    Ok(incidents.iter().map(|i| info(i, now)).collect())
}

/// The incident, if it is in `scope` (else "not found", D-45).
async fn scoped(
    store: &dyn IncidentStore,
    scope: &CompanyScope,
    id: i64,
) -> Result<StoredIncident, IncidentError> {
    match store.incident(id).await? {
        Some(incident) if scope.allows(&incident.company_id) => Ok(incident),
        _ => Err(IncidentError::NotFound),
    }
}

/// An incident and its timeline. `internal`: with what people wrote (not
/// for Clients, D-67).
pub async fn detail(
    store: &dyn IncidentStore,
    scope: &CompanyScope,
    id: i64,
    internal: bool,
    now: DateTime<Utc>,
) -> Result<IncidentDetail, IncidentError> {
    let incident = scoped(store, scope, id).await?;
    let events = store
        .incident_events(id)
        .await?
        .into_iter()
        .filter(|e| internal || e.kind != IncidentEventKind::Note)
        .map(|e| IncidentEvent {
            at: format_time(e.at),
            kind: e.kind,
            username: e.username,
            text: if internal || !e.kind.is_internal() {
                e.text
            } else {
                String::new()
            },
        })
        .collect();
    Ok(IncidentDetail {
        incident: info(&incident, now),
        events,
    })
}

/// "I'm on it": `username` acknowledges the open incident, with a note (may
/// be empty). Once per incident.
pub async fn acknowledge(
    store: &dyn IncidentStore,
    scope: &CompanyScope,
    id: i64,
    username: &str,
    note: &IncidentNote,
    now: DateTime<Utc>,
) -> Result<IncidentInfo, IncidentError> {
    note.validate(false).map_err(IncidentError::Invalid)?;
    let incident = scoped(store, scope, id).await?;
    let event = StoredIncidentEvent {
        at: now,
        kind: IncidentEventKind::Acknowledged,
        username: Some(username.to_string()),
        text: note.text.trim().to_string(),
    };
    if !store.acknowledge_incident(id, &event).await? {
        return Err(IncidentError::Conflict(match incident.acknowledged_by {
            Some(by) => format!("{by} acknowledged it already"),
            None => "it is closed".into(),
        }));
    }
    let incident = scoped(store, scope, id).await?;
    Ok(info(&incident, now))
}

/// A note on the incident's timeline, open or closed.
pub async fn add_note(
    store: &dyn IncidentStore,
    scope: &CompanyScope,
    id: i64,
    username: &str,
    note: &IncidentNote,
    now: DateTime<Utc>,
) -> Result<IncidentInfo, IncidentError> {
    note.validate(true).map_err(IncidentError::Invalid)?;
    let incident = scoped(store, scope, id).await?;
    let event = StoredIncidentEvent {
        at: now,
        kind: IncidentEventKind::Note,
        username: Some(username.to_string()),
        text: note.text.trim().to_string(),
    };
    store.add_incident_event(id, &event).await?;
    Ok(info(&incident, now))
}
