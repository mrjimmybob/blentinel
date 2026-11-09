//! M6-01: Background task for automatic rollup computation and retention pruning
//!
//! Run via background scheduler (e.g., once per hour for rollups, once per day for pruning).
//! Idempotent: safe to run multiple times.

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

use crate::service::{rollups, retention_policy};

/// Compute all pending rollups (5m, hourly, daily). Called periodically by background scheduler.
/// Idempotent: re-runs safely overwrite identical buckets.
pub async fn run_rollup_compute(pool: &SqlitePool) -> Result<RollupStats> {
    let now = Utc::now();

    let count_5m = rollups::compute_5m_rollups(pool, now).await?;
    let count_hourly = rollups::compute_hourly_rollups(pool, now).await?;
    let count_daily = rollups::compute_daily_rollups(pool, now).await?;

    Ok(RollupStats {
        computed_5m_buckets: count_5m,
        computed_hourly_buckets: count_hourly,
        computed_daily_buckets: count_daily,
    })
}

/// Prune old data according to retention policy (raw, 5m, hourly; daily never pruned).
/// Called once per day (e.g., midnight UTC). Idempotent.
pub async fn run_retention_prune(pool: &SqlitePool) -> Result<RetentionStats> {
    let deleted_raw = retention_policy::prune_raw_results(pool).await?;
    let deleted_5m = retention_policy::prune_5m_rollups(pool).await?;
    let deleted_hourly = retention_policy::prune_hourly_rollups(pool).await?;

    Ok(RetentionStats {
        deleted_raw_results: deleted_raw,
        deleted_5m_rollups: deleted_5m,
        deleted_hourly_rollups: deleted_hourly,
    })
}

/// Combined task: compute rollups then prune old data (idempotent).
/// Ensures rollups are completed before raw data is pruned.
pub async fn run_combined_task(pool: &SqlitePool) -> Result<(RollupStats, RetentionStats)> {
    let rollups = run_rollup_compute(pool).await?;
    let retention = run_retention_prune(pool).await?;
    Ok((rollups, retention))
}


/// Statistics from a rollup computation run.
#[derive(Debug, Clone)]
pub struct RollupStats {
    pub computed_5m_buckets: usize,
    pub computed_hourly_buckets: usize,
    pub computed_daily_buckets: usize,
}

impl RollupStats {
    pub fn total_buckets(&self) -> usize {
        self.computed_5m_buckets + self.computed_hourly_buckets + self.computed_daily_buckets
    }
}

/// Statistics from a retention pruning run.
#[derive(Debug, Clone)]
pub struct RetentionStats {
    pub deleted_raw_results: u64,
    pub deleted_5m_rollups: u64,
    pub deleted_hourly_rollups: u64,
}

impl RetentionStats {
    pub fn total_deleted(&self) -> u64 {
        self.deleted_raw_results + self.deleted_5m_rollups + self.deleted_hourly_rollups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_stats_totals_correctly() {
        let stats = RollupStats {
            computed_5m_buckets: 12,
            computed_hourly_buckets: 1,
            computed_daily_buckets: 1,
        };
        assert_eq!(stats.total_buckets(), 14);
    }

    #[test]
    fn retention_stats_totals_correctly() {
        let stats = RetentionStats {
            deleted_raw_results: 1000,
            deleted_5m_rollups: 100,
            deleted_hourly_rollups: 10,
        };
        assert_eq!(stats.total_deleted(), 1110);
    }
}
