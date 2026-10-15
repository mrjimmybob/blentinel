//! Thresholds above the check: the hub's default, a company's, a site's
//! (M3-01, D-64). Stored as JSON (`model::Thresholds`).

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::model::Thresholds;

fn parse(json: Option<String>) -> anyhow::Result<Thresholds> {
    json.map_or(Ok(Thresholds::default()), |json| {
        serde_json::from_str(&json).map_err(|e| anyhow::anyhow!("stored thresholds {json:?}: {e}"))
    })
}

/// `NULL` for nothing set.
fn to_json(thresholds: &Thresholds) -> anyhow::Result<Option<String>> {
    Ok((*thresholds != Thresholds::default())
        .then(|| serde_json::to_string(thresholds))
        .transpose()?)
}

pub(super) async fn defaults(pool: &SqlitePool) -> anyhow::Result<Option<Thresholds>> {
    let row = sqlx::query("SELECT thresholds FROM threshold_defaults WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    row.map(|row| parse(Some(row.get("thresholds"))))
        .transpose()
}

pub(super) async fn set_defaults(
    pool: &SqlitePool,
    thresholds: &Thresholds,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO threshold_defaults (id, thresholds, updated_by, updated_at) VALUES (1, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET thresholds = excluded.thresholds,
             updated_by = excluded.updated_by, updated_at = excluded.updated_at",
    )
    .bind(serde_json::to_string(thresholds)?)
    .bind(updated_by)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn company(pool: &SqlitePool, company_id: &str) -> anyhow::Result<Thresholds> {
    let json: Option<Option<String>> =
        sqlx::query_scalar("SELECT thresholds FROM companies WHERE id = ?")
            .bind(company_id)
            .fetch_optional(pool)
            .await?;
    parse(json.flatten())
}

pub(super) async fn set_company(
    pool: &SqlitePool,
    company_id: &str,
    thresholds: &Thresholds,
) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE companies SET thresholds = ? WHERE id = ?")
        .bind(to_json(thresholds)?)
        .bind(company_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn site(
    pool: &SqlitePool,
    company_id: &str,
    site_id: &str,
) -> anyhow::Result<Thresholds> {
    let json: Option<Option<String>> =
        sqlx::query_scalar("SELECT thresholds FROM sites WHERE company_id = ? AND id = ?")
            .bind(company_id)
            .bind(site_id)
            .fetch_optional(pool)
            .await?;
    parse(json.flatten())
}

pub(super) async fn set_site(
    pool: &SqlitePool,
    company_id: &str,
    site_id: &str,
    thresholds: &Thresholds,
) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE sites SET thresholds = ? WHERE company_id = ? AND id = ?")
        .bind(to_json(thresholds)?)
        .bind(company_id)
        .bind(site_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
