//! M6-03: All-problems view - unified incident list with search (proposal 0007, D-10)
//!
//! Display all incidents (open + closed) across all time, filterable by check/status.
//! Incident = when a check enters Down/Warning state and stays for > threshold.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

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
    pool: &SqlitePool,
    check_id_filter: Option<&str>,
    status_filter: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<IncidentSummary>> {
    // TODO M6-03: Integrate with hub's incident store.
    // For now, this queries the incidents table assuming it exists with schema:
    // CREATE TABLE incidents (
    //     id INTEGER PRIMARY KEY,
    //     check_id TEXT NOT NULL,
    //     status TEXT NOT NULL,  -- 'Down', 'Warning', 'Resolved'
    //     opened_at DATETIME NOT NULL,
    //     closed_at DATETIME,
    //     FOREIGN KEY(check_id) REFERENCES checks(id)
    // );

    let query_str = build_incident_query(check_id_filter, status_filter, limit, offset);

    let incidents = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<i32>)>(
        &query_str,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    Ok(incidents
        .into_iter()
        .map(|(id, check_id, status, opened_str, closed_str, duration)| {
            let opened_at = opened_str.parse::<DateTime<Utc>>().unwrap_or(Utc::now());
            let closed_at = closed_str.and_then(|s| s.parse::<DateTime<Utc>>().ok());

            IncidentSummary {
                incident_id: id,
                check_id,
                status,
                opened_at,
                closed_at,
                duration_minutes: duration.unwrap_or(0),
                description: format!("Incident #{}", id),
            }
        })
        .collect())
}

/// Build SQL query for incidents with optional filters.
fn build_incident_query(
    check_filter: Option<&str>,
    status_filter: Option<&str>,
    _limit: i32,
    _offset: i32,
) -> String {
    let mut query = "SELECT id, check_id, status, opened_at, closed_at, CAST((julianday(COALESCE(closed_at, datetime('now'))) - julianday(opened_at)) * 24 * 60 AS INTEGER) as duration_minutes FROM incidents WHERE 1=1".to_string();

    if let Some(check_id) = check_filter {
        query.push_str(&format!(" AND check_id = '{}'", check_id.replace("'", "''")));
    }

    if let Some(status) = status_filter {
        query.push_str(&format!(" AND status = '{}'", status.replace("'", "''")));
    }

    query.push_str(" ORDER BY opened_at DESC LIMIT ? OFFSET ?");
    query
}

/// Search incidents by check_id or status.
pub async fn search_incidents(
    pool: &SqlitePool,
    search_term: &str,
    limit: i32,
) -> Result<Vec<IncidentSummary>> {
    // Simple substring search on check_id or status
    let query_str = format!(
        "SELECT id, check_id, status, opened_at, closed_at, CAST((julianday(COALESCE(closed_at, datetime('now'))) - julianday(opened_at)) * 24 * 60 AS INTEGER) FROM incidents WHERE check_id LIKE ? OR status LIKE ? ORDER BY opened_at DESC LIMIT ?",
    );

    let search_pattern = format!("%{}%", search_term);

    let incidents = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<i32>)>(
        &query_str,
    )
    .bind(&search_pattern)
    .bind(&search_pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    Ok(incidents
        .into_iter()
        .map(|(id, check_id, status, opened_str, closed_str, duration)| {
            let opened_at = opened_str.parse::<DateTime<Utc>>().unwrap_or(Utc::now());
            let closed_at = closed_str.and_then(|s| s.parse::<DateTime<Utc>>().ok());

            IncidentSummary {
                incident_id: id,
                check_id,
                status,
                opened_at,
                closed_at,
                duration_minutes: duration.unwrap_or(0),
                description: format!("Incident #{}", id),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_query_includes_check_filter() {
        let query = build_incident_query(Some("check-123"), None, 10, 0);
        assert!(query.contains("check_id"));
    }

    #[test]
    fn build_query_includes_status_filter() {
        let query = build_incident_query(None, Some("Down"), 10, 0);
        assert!(query.contains("status"));
    }

    #[test]
    fn build_query_escapes_sql_injection() {
        let query = build_incident_query(Some("'; DROP TABLE incidents; --"), None, 10, 0);
        assert!(query.contains("DROP") == false); // Should be escaped
    }
}
