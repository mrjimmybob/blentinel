//! SQLite queries for the managed probe rollout (M4-01, D-74), behind
//! [`super::SqliteStorage`]'s `RolloutStore` implementation.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{CompanyUpdatePolicy, RolloutStage, UpdateMode, UpdateWindow};
use crate::storage::{Rollout, RolloutChange, RolloutPaused};

fn to_rollout(row: &SqliteRow) -> Rollout {
    let stage: String = row.get("stage");
    let soak: i64 = row.get("soak_minutes");
    let paused_at: Option<DateTime<Utc>> = row.get("paused_at");
    Rollout {
        version: row.get("version"),
        approved_by: row.get("approved_by"),
        approved_at: row.get("approved_at"),
        stage: RolloutStage::parse(&stage).unwrap_or(RolloutStage::All),
        soak_minutes: u32::try_from(soak).unwrap_or(0),
        canaries_ready_at: row.get("canaries_ready_at"),
        promoted_at: row.get("promoted_at"),
        promoted_by: row.get("promoted_by"),
        paused: paused_at.map(|at| RolloutPaused {
            at,
            by: row.get("paused_by"),
            reason: row
                .get::<Option<String>, _>("pause_reason")
                .unwrap_or_default(),
        }),
    }
}

pub(super) async fn get(pool: &SqlitePool) -> anyhow::Result<Option<Rollout>> {
    let row = sqlx::query(
        "SELECT version, approved_by, approved_at, stage, soak_minutes, canaries_ready_at,
                promoted_at, promoted_by, paused_at, paused_by, pause_reason
         FROM probe_rollout WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(to_rollout))
}

pub(super) async fn set(pool: &SqlitePool, rollout: Option<&Rollout>) -> anyhow::Result<()> {
    let mut tx = super::write_transaction(pool).await?;
    sqlx::query("DELETE FROM probe_rollout")
        .execute(&mut *tx)
        .await?;
    if let Some(r) = rollout {
        sqlx::query(
            "INSERT INTO probe_rollout (id, version, approved_by, approved_at, stage,
                 soak_minutes, canaries_ready_at, promoted_at, promoted_by,
                 paused_at, paused_by, pause_reason)
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.version)
        .bind(&r.approved_by)
        .bind(r.approved_at)
        .bind(r.stage.as_str())
        .bind(i64::from(r.soak_minutes))
        .bind(r.canaries_ready_at)
        .bind(r.promoted_at)
        .bind(&r.promoted_by)
        .bind(r.paused.as_ref().map(|p| p.at))
        .bind(r.paused.as_ref().and_then(|p| p.by.as_deref()))
        .bind(r.paused.as_ref().map(|p| p.reason.as_str()))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn change(
    pool: &SqlitePool,
    version: &str,
    change: &RolloutChange<'_>,
    at: DateTime<Utc>,
) -> anyhow::Result<bool> {
    // Each change names the state it applies to, so it can't undo another.
    let query = match change {
        RolloutChange::Pause { by, reason } => sqlx::query(
            "UPDATE probe_rollout SET paused_at = ?, paused_by = ?, pause_reason = ?
             WHERE version = ? AND paused_at IS NULL",
        )
        .bind(at)
        .bind(*by)
        .bind(*reason),
        RolloutChange::Resume => sqlx::query(
            "UPDATE probe_rollout SET paused_at = NULL, paused_by = NULL, pause_reason = NULL
             WHERE version = ? AND paused_at IS NOT NULL",
        ),
        RolloutChange::CanariesReady(since) => sqlx::query(
            "UPDATE probe_rollout SET canaries_ready_at = ?
             WHERE version = ? AND stage = 'canary'",
        )
        .bind(*since),
        // The hub promotes only a rollout that isn't paused; an admin may
        // promote a paused one (and so resume it).
        RolloutChange::Promote { by } => sqlx::query(
            "UPDATE probe_rollout SET stage = 'all', promoted_at = ?, promoted_by = ?,
                    paused_at = NULL, paused_by = NULL, pause_reason = NULL
             WHERE version = ? AND stage = 'canary' AND (? OR paused_at IS NULL)",
        )
        .bind(at)
        .bind(*by),
    };
    let query = query.bind(version);
    let query = match change {
        RolloutChange::Promote { by } => query.bind(by.is_some()),
        _ => query,
    };
    Ok(query.execute(pool).await?.rows_affected() > 0)
}

pub(super) async fn set_canary(
    pool: &SqlitePool,
    probe_id: &str,
    canary: bool,
) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE probes SET update_canary = ? WHERE id = ?")
        .bind(canary)
        .bind(probe_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

fn to_policy(row: &SqliteRow) -> CompanyUpdatePolicy {
    let mode: String = row.get("mode");
    let start: Option<String> = row.get("window_start");
    let end: Option<String> = row.get("window_end");
    CompanyUpdatePolicy {
        mode: UpdateMode::parse(&mode).unwrap_or_default(),
        version: row.get("version"),
        window: start
            .zip(end)
            .map(|(start, end)| UpdateWindow { start, end }),
    }
}

pub(super) async fn policy(
    pool: &SqlitePool,
    company_id: &str,
) -> anyhow::Result<CompanyUpdatePolicy> {
    let row = sqlx::query(
        "SELECT mode, version, window_start, window_end
         FROM company_update_policy WHERE company_id = ?",
    )
    .bind(company_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(to_policy).unwrap_or_default())
}

pub(super) async fn policies(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<(String, CompanyUpdatePolicy)>> {
    let rows = sqlx::query(
        "SELECT company_id, mode, version, window_start, window_end
         FROM company_update_policy ORDER BY company_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("company_id"), to_policy(row)))
        .collect())
}

pub(super) async fn set_policy(
    pool: &SqlitePool,
    company_id: &str,
    policy: &CompanyUpdatePolicy,
    by: Option<&str>,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    if policy.is_default() {
        sqlx::query("DELETE FROM company_update_policy WHERE company_id = ?")
            .bind(company_id)
            .execute(pool)
            .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO company_update_policy
             (company_id, mode, version, window_start, window_end, updated_by, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (company_id) DO UPDATE SET mode = excluded.mode,
             version = excluded.version, window_start = excluded.window_start,
             window_end = excluded.window_end, updated_by = excluded.updated_by,
             updated_at = excluded.updated_at",
    )
    .bind(company_id)
    .bind(policy.mode.as_str())
    .bind(policy.version.as_deref().map(str::trim))
    .bind(policy.window.as_ref().map(|w| w.start.trim()))
    .bind(policy.window.as_ref().map(|w| w.end.trim()))
    .bind(by)
    .bind(at)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn site_timezone(
    pool: &SqlitePool,
    company_id: &str,
    site_id: &str,
) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT timezone FROM sites WHERE company_id = ? AND id = ?")
            .bind(company_id)
            .bind(site_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub(super) async fn site_timezones(
    pool: &SqlitePool,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let rows = sqlx::query("SELECT company_id, id, timezone FROM sites")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("company_id"), row.get("id"), row.get("timezone")))
        .collect())
}
