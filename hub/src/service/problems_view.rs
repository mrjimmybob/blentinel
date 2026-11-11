//! M6-03: All-problems view - unified incident list with search (proposal 0007, D-10)
//!
//! Display all incidents (open + closed) across all time, filterable by check/status.
//! Incident = when a check enters Down/Warning state and stays for > threshold.

use crate::storage::Storage;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// An incident: when a check went down/warning and resolved or is still open.
#[derive(Debug, Clone, Serialize)]
pub struct IncidentSummary {
    pub incident_id: i64,
    pub check_id: String,
    pub status: String,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub duration_minutes: i32,
    pub description: String,
}

/// Query all incidents with optional filters.
pub async fn list_incidents(
    storage: &dyn Storage,
    check_id_filter: Option<&str>,
    status_filter: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<IncidentSummary>> {
    // TODO M6-03: Integrate with hub's incident store.
    // For now, fetch from storage trait's RollupStore implementation
    let incidents = storage
        .list_incidents(check_id_filter, status_filter, limit, offset)
        .await?;

    Ok(incidents
        .into_iter()
        .map(
            |(id, check_id, status, opened_at, closed_at, duration_minutes)| IncidentSummary {
                incident_id: id,
                check_id,
                status,
                opened_at,
                closed_at,
                duration_minutes,
                description: format!("Incident #{}", id),
            },
        )
        .collect())
}

/// Search incidents by check_id or status.
pub async fn search_incidents(
    storage: &dyn Storage,
    search_term: &str,
    limit: i32,
) -> Result<Vec<IncidentSummary>> {
    // Simple search: pass search term as check_id filter (TODO: improve filtering)
    let incidents = storage
        .list_incidents(Some(search_term), None, limit, 0)
        .await?;

    Ok(incidents
        .into_iter()
        .map(
            |(id, check_id, status, opened_at, closed_at, duration_minutes)| IncidentSummary {
                incident_id: id,
                check_id,
                status,
                opened_at,
                closed_at,
                duration_minutes,
                description: format!("Incident #{}", id),
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn incident_summary_creation() {
        use super::IncidentSummary;
        use chrono::Utc;

        let incident = IncidentSummary {
            incident_id: 1,
            check_id: "check-123".to_string(),
            status: "Down".to_string(),
            opened_at: Utc::now(),
            closed_at: None,
            duration_minutes: 5,
            description: "Test incident".to_string(),
        };

        assert_eq!(incident.incident_id, 1);
        assert_eq!(incident.check_id, "check-123");
    }
}
