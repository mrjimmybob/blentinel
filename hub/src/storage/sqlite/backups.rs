//! The backup schedule and its last run (M2-12, D-60).

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::model::BackupSchedule;
use crate::storage::BackupRun;

pub(super) async fn schedule(pool: &SqlitePool) -> anyhow::Result<Option<BackupSchedule>> {
    let row = sqlx::query("SELECT enabled, hour_utc, keep FROM backup_schedule WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    row.map(|row| {
        Ok(BackupSchedule {
            enabled: row.get("enabled"),
            hour_utc: u8::try_from(row.get::<i64, _>("hour_utc"))?,
            keep: u32::try_from(row.get::<i64, _>("keep"))?,
        })
    })
    .transpose()
}

pub(super) async fn set_schedule(
    pool: &SqlitePool,
    schedule: &BackupSchedule,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO backup_schedule (id, enabled, hour_utc, keep, updated_by, updated_at)
         VALUES (1, ?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled,
             hour_utc = excluded.hour_utc, keep = excluded.keep,
             updated_by = excluded.updated_by, updated_at = excluded.updated_at",
    )
    .bind(schedule.enabled)
    .bind(i64::from(schedule.hour_utc))
    .bind(i64::from(schedule.keep))
    .bind(updated_by)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn last_run(pool: &SqlitePool) -> anyhow::Result<Option<BackupRun>> {
    let row = sqlx::query("SELECT ran_at, error FROM backup_last_run WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| BackupRun {
        ran_at: row.get("ran_at"),
        error: row.get("error"),
    }))
}

pub(super) async fn record_run(pool: &SqlitePool, run: &BackupRun) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO backup_last_run (id, ran_at, error) VALUES (1, ?, ?)
         ON CONFLICT (id) DO UPDATE SET ran_at = excluded.ran_at, error = excluded.error",
    )
    .bind(run.ran_at)
    .bind(&run.error)
    .execute(pool)
    .await?;
    Ok(())
}
