//! Incidents and their timelines (M3-05, D-67).

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::scope;
use crate::model::{CompanyScope, IncidentEventKind, Status};
use crate::storage::{StoredIncident, StoredIncidentEvent};

const COLUMNS: &str = "id, company_id, company_name, site_id, site_name, probe_id, probe_name, \
     check_id, check_name, severity, status, opened_at, closed_at, acknowledged_by, acknowledged_at";

fn status(row: &SqliteRow, column: &str) -> anyhow::Result<Status> {
    let text: String = row.get(column);
    Status::parse(&text).ok_or_else(|| anyhow::anyhow!("unknown incident {column} {text:?}"))
}

fn to_incident(row: &SqliteRow) -> anyhow::Result<StoredIncident> {
    Ok(StoredIncident {
        id: row.get("id"),
        company_id: row.get("company_id"),
        company_name: row.get("company_name"),
        site_id: row.get("site_id"),
        site_name: row.get("site_name"),
        probe_id: row.get("probe_id"),
        probe_name: row.get("probe_name"),
        check_id: row.get("check_id"),
        check_name: row.get("check_name"),
        severity: status(row, "severity")?,
        status: status(row, "status")?,
        opened_at: row.get("opened_at"),
        closed_at: row.get("closed_at"),
        acknowledged_by: row.get("acknowledged_by"),
        acknowledged_at: row.get("acknowledged_at"),
    })
}

async fn insert_event(
    conn: &mut SqliteConnection,
    id: i64,
    event: &StoredIncidentEvent,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO incident_events (incident_id, at, kind, username, text) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(event.at)
    .bind(event.kind.as_str())
    .bind(&event.username)
    .bind(&event.text)
    .execute(conn)
    .await?;
    Ok(())
}

pub(super) async fn open(
    pool: &SqlitePool,
    incident: &StoredIncident,
    opened: &StoredIncidentEvent,
) -> anyhow::Result<i64> {
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO incidents (company_id, company_name, site_id, site_name, probe_id, probe_name,
             check_id, check_name, severity, status, opened_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&incident.company_id)
    .bind(&incident.company_name)
    .bind(&incident.site_id)
    .bind(&incident.site_name)
    .bind(&incident.probe_id)
    .bind(&incident.probe_name)
    .bind(&incident.check_id)
    .bind(&incident.check_name)
    .bind(incident.severity.as_str())
    .bind(incident.status.as_str())
    .bind(incident.opened_at)
    .fetch_one(&mut *tx)
    .await?;
    insert_event(&mut tx, id, opened).await?;
    tx.commit().await?;
    Ok(id)
}

pub(super) async fn open_of(
    pool: &SqlitePool,
    probe_id: &str,
    check_id: Option<&str>,
) -> anyhow::Result<Option<StoredIncident>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM incidents
         WHERE probe_id = ? AND IFNULL(check_id, '') = ? AND closed_at IS NULL"
    );
    let row = sqlx::query(&sql)
        .bind(probe_id)
        .bind(check_id.unwrap_or(""))
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(to_incident).transpose()
}

pub(super) async fn get(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<StoredIncident>> {
    let sql = format!("SELECT {COLUMNS} FROM incidents WHERE id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(to_incident).transpose()
}

pub(super) async fn list(
    pool: &SqlitePool,
    scope: &CompanyScope,
    open: Option<bool>,
    limit: u32,
) -> anyhow::Result<Vec<StoredIncident>> {
    let (in_scope, binds) = scope::condition("company_id", scope);
    let state = match open {
        Some(true) => "closed_at IS NULL",
        Some(false) => "closed_at IS NOT NULL",
        None => "1 = 1",
    };
    let sql = format!(
        "SELECT {COLUMNS} FROM incidents WHERE {in_scope} AND {state}
         ORDER BY opened_at DESC, id DESC LIMIT ?"
    );
    let mut query = sqlx::query(&sql);
    for bind in binds {
        query = query.bind(bind);
    }
    let rows = query.bind(limit).fetch_all(pool).await?;
    rows.iter().map(to_incident).collect()
}

pub(super) async fn events(pool: &SqlitePool, id: i64) -> anyhow::Result<Vec<StoredIncidentEvent>> {
    let rows = sqlx::query(
        "SELECT at, kind, username, text FROM incident_events WHERE incident_id = ? ORDER BY id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let kind: String = row.get("kind");
            let at: DateTime<Utc> = row.get("at");
            Ok(StoredIncidentEvent {
                at,
                kind: IncidentEventKind::parse(&kind)
                    .ok_or_else(|| anyhow::anyhow!("unknown incident event {kind:?}"))?,
                username: row.get("username"),
                text: row.get("text"),
            })
        })
        .collect()
}

pub(super) async fn add_event(
    pool: &SqlitePool,
    id: i64,
    event: &StoredIncidentEvent,
) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    insert_event(&mut conn, id, event).await
}

pub(super) async fn change(
    pool: &SqlitePool,
    id: i64,
    status: Status,
    event: &StoredIncidentEvent,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    // Down is worse than Warning: the severity only goes up.
    sqlx::query(
        "UPDATE incidents SET status = ?1,
             severity = CASE WHEN ?1 = 'Down' THEN 'Down' ELSE severity END
         WHERE id = ?2",
    )
    .bind(status.as_str())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    insert_event(&mut tx, id, event).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn close(
    pool: &SqlitePool,
    id: i64,
    event: &StoredIncidentEvent,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query("UPDATE incidents SET closed_at = ? WHERE id = ? AND closed_at IS NULL")
        .bind(event.at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    insert_event(&mut tx, id, event).await?;
    tx.commit().await?;
    Ok(true)
}

pub(super) async fn acknowledge(
    pool: &SqlitePool,
    id: i64,
    event: &StoredIncidentEvent,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query(
        "UPDATE incidents SET acknowledged_by = ?, acknowledged_at = ?
         WHERE id = ? AND closed_at IS NULL AND acknowledged_at IS NULL",
    )
    .bind(&event.username)
    .bind(event.at)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    insert_event(&mut tx, id, event).await?;
    tx.commit().await?;
    Ok(true)
}
