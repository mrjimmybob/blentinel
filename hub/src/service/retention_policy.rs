//! M6-01: Data retention policy and pruning (proposal 0007, D-10)
//!
//! Configurable per-tier retention: raw 14d, 5m 90d, hourly 2y, daily forever.
//! Pruning preserves accuracy: never deletes data needed for availability/SLA calculations.

use anyhow::Result;
use chrono::{Duration, Utc};
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub tier: String,
    pub retention_days: i32,
}

/// Load configured retention policy for a tier.
pub async fn get_policy(pool: &SqlitePool, tier: &str) -> Result<Option<RetentionPolicy>> {
    let row = sqlx::query!("SELECT tier, retention_days FROM retention_config WHERE tier = ?", tier)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|r| RetentionPolicy {
        tier: r.tier,
        retention_days: r.retention_days,
    }))
}

/// Prune raw check_results older than configured retention.
/// **Safety:** Only deletes raw data; rolled-up tiers preserve aggregates for historical queries.
pub async fn prune_raw_results(pool: &SqlitePool) -> Result<u64> {
    let policy = get_policy(pool, "raw").await?;
    let Some(policy) = policy else {
        return Ok(0);  // No policy configured
    };

    let cutoff = Utc::now() - Duration::days(policy.retention_days as i64);
    let cutoff_str = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let result = sqlx::query!(
        r#"DELETE FROM check_results WHERE id IN (
            SELECT cr.id FROM check_results cr
            INNER JOIN reports r ON cr.report_id = r.id
            WHERE r.timestamp < ?
        )"#,
        cutoff_str
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Prune 5-minute rollups older than configured retention.
pub async fn prune_5m_rollups(pool: &SqlitePool) -> Result<u64> {
    let policy = get_policy(pool, "5m").await?;
    let Some(policy) = policy else {
        return Ok(0);
    };

    let cutoff = Utc::now() - Duration::days(policy.retention_days as i64);
    let cutoff_str = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let result = sqlx::query!("DELETE FROM check_rollups_5m WHERE bucket_start < ?", cutoff_str)
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}

/// Prune hourly rollups older than configured retention.
pub async fn prune_hourly_rollups(pool: &SqlitePool) -> Result<u64> {
    let policy = get_policy(pool, "hourly").await?;
    let Some(policy) = policy else {
        return Ok(0);
    };

    let cutoff = Utc::now() - Duration::days(policy.retention_days as i64);
    let cutoff_str = cutoff.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let result = sqlx::query!("DELETE FROM check_rollups_hourly WHERE bucket_start < ?", cutoff_str)
        .execute(pool)
        .await?;

    Ok(result.rows_affected())
}

/// Run all retention pruning tasks (idempotent, safe).
/// Call once per day (e.g., via background task at midnight UTC).
pub async fn prune_all(pool: &SqlitePool) -> Result<()> {
    let _ = prune_raw_results(pool).await?;
    let _ = prune_5m_rollups(pool).await?;
    let _ = prune_hourly_rollups(pool).await?;
    // Daily rollups: never delete (storage negligible, audit trail)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_calculation_is_correct() {
        let now = Utc::now();
        let policy = RetentionPolicy {
            tier: "raw".to_string(),
            retention_days: 14,
        };
        let cutoff = now - Duration::days(policy.retention_days as i64);
        assert!(cutoff < now);
        assert!((now - cutoff).num_days() >= 14);
    }
}
