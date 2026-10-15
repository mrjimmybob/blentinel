//! SQLite queries for hub upgrades (M4-04, D-75), behind
//! [`super::SqliteStorage`]'s `UpgradeStore` implementation.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::storage::UpgradeCheck;

pub(super) async fn get(pool: &SqlitePool) -> anyhow::Result<UpgradeCheck> {
    let row = sqlx::query(
        "SELECT enabled, checked_at, latest_version, error FROM hub_upgrade_check WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map_or_else(UpgradeCheck::default, |row| UpgradeCheck {
        enabled: row.get("enabled"),
        checked_at: row.get("checked_at"),
        latest_version: row.get("latest_version"),
        error: row.get("error"),
    }))
}

pub(super) async fn set_enabled(
    pool: &SqlitePool,
    enabled: bool,
    by: Option<&str>,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO hub_upgrade_check (id, enabled, updated_by, updated_at) VALUES (1, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled,
             updated_by = excluded.updated_by, updated_at = excluded.updated_at",
    )
    .bind(enabled)
    .bind(by)
    .bind(at)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn record(
    pool: &SqlitePool,
    at: DateTime<Utc>,
    latest: Option<&str>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    // A failed check keeps the release found before: it is still out there.
    sqlx::query(
        "INSERT INTO hub_upgrade_check (id, enabled, updated_at, checked_at, latest_version, error)
         VALUES (1, 0, ?1, ?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE SET checked_at = excluded.checked_at,
             latest_version = COALESCE(excluded.latest_version, latest_version),
             error = excluded.error",
    )
    .bind(at)
    .bind(latest)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}
