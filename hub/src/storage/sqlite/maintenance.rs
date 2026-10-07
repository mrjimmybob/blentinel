//! Maintenance windows (M3-04, D-65). The schedule is stored as JSON.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{MaintenanceWindow, NewWindowRequest, Scope};

const COLUMNS: &str =
    "id, company_id, scope_kind, scope_id, reason, timezone, schedule, created_by, created_at";

fn to_window(row: &SqliteRow) -> anyhow::Result<MaintenanceWindow> {
    let kind: String = row.get("scope_kind");
    let schedule: String = row.get("schedule");
    let created_at: DateTime<Utc> = row.get("created_at");
    Ok(MaintenanceWindow {
        id: row.get("id"),
        company_id: row.get("company_id"),
        scope: Scope::from_parts(&kind, row.get("scope_id"))
            .ok_or_else(|| anyhow::anyhow!("unknown window scope {kind}"))?,
        reason: row.get("reason"),
        timezone: row.get("timezone"),
        schedule: serde_json::from_str(&schedule)
            .map_err(|e| anyhow::anyhow!("stored schedule {schedule:?}: {e}"))?,
        created_by: row.get("created_by"),
        created_at: created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    })
}

pub(super) async fn windows(
    pool: &SqlitePool,
    company_id: &str,
) -> anyhow::Result<Vec<MaintenanceWindow>> {
    let sql = format!("SELECT {COLUMNS} FROM maintenance_windows WHERE company_id = ? ORDER BY id");
    let rows = sqlx::query(&sql).bind(company_id).fetch_all(pool).await?;
    rows.iter().map(to_window).collect()
}

pub(super) async fn window(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<MaintenanceWindow>> {
    let sql = format!("SELECT {COLUMNS} FROM maintenance_windows WHERE id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(to_window).transpose()
}

pub(super) async fn create(
    pool: &SqlitePool,
    company_id: &str,
    window: &NewWindowRequest,
    created_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<i64> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO maintenance_windows
             (company_id, scope_kind, scope_id, reason, timezone, schedule, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(company_id)
    .bind(window.scope.kind())
    .bind(window.scope.id())
    .bind(window.reason.trim())
    .bind(window.timezone.trim())
    .bind(serde_json::to_string(&window.schedule)?)
    .bind(created_by)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub(super) async fn delete(pool: &SqlitePool, id: i64) -> anyhow::Result<bool> {
    let done = sqlx::query("DELETE FROM maintenance_windows WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
