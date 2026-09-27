#![cfg(feature = "ssr")]

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use anyhow::Result;

/// Check if a resource is currently silenced (as of `now`)
pub async fn is_resource_silenced(
    pool: &SqlitePool,
    resource_key: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    crate::db::is_silenced(pool, "resource", resource_key, now).await
}

/// Clear silence when resource recovers
pub async fn clear_silence_on_recovery(
    pool: &SqlitePool,
    resource_key: &str,
) -> Result<()> {
    crate::db::clear_silence_by_resource(pool, resource_key).await
}
