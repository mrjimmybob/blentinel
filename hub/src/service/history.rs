//! History: how long raw results and rollups are kept, pruning, and which
//! rollup to read for a period (proposal 0007 §2, D-111).
//!
//! Storage writes the rollups itself, with each report, so there is nothing
//! here to compute them; the background task only prunes.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};

use crate::clock::Clock;
use crate::state::Reloadable;
use crate::storage::{
    HistoryCutoffs, HistoryRetention, HistoryStore, PrunedHistory, RollupWidth, Storage,
    StorageResult,
};

/// The most buckets [`width_for`] picks for one period: enough for a chart.
pub const MAX_POINTS: i64 = 750;

/// How often the pruner runs.
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// What's wrong, from [`HistoryRetention::validate`].
    #[error("{0}")]
    Invalid(String),
    /// A period that can't be shown (the check page, M6-02).
    #[error("{0}")]
    InvalidRange(String),
    /// Unknown, or outside the user's companies (deliberately the same).
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

pub async fn retention(store: &dyn HistoryStore) -> StorageResult<HistoryRetention> {
    store.history_retention().await
}

/// Save `retention`, if it's valid. It applies at the next prune.
pub async fn set_retention(
    store: &dyn HistoryStore,
    retention: &HistoryRetention,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), HistoryError> {
    retention.validate().map_err(HistoryError::Invalid)?;
    store
        .set_history_retention(retention, updated_by, now)
        .await?;
    Ok(())
}

/// What is older than `retention` allows, as of `now`.
pub fn cutoffs(retention: &HistoryRetention, now: DateTime<Utc>) -> HistoryCutoffs {
    let days = |n: u32| now - Duration::days(i64::from(n));
    HistoryCutoffs {
        raw: days(retention.raw_days),
        five_minutes: days(retention.five_minute_days),
        hourly: days(retention.hourly_days),
    }
}

/// Delete what the saved retention no longer keeps.
pub async fn prune(store: &dyn HistoryStore, now: DateTime<Utc>) -> StorageResult<PrunedHistory> {
    let retention = store.history_retention().await?;
    store.prune_history(&cutoffs(&retention, now)).await
}

/// The finest rollup width that still holds `from` (as `retention` keeps
/// it) and splits `[from, to)` into at most [`MAX_POINTS`] buckets. Daily
/// rollups are kept forever, so they are the fallback.
pub fn width_for(
    retention: &HistoryRetention,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> RollupWidth {
    let span = (to - from).num_seconds().max(0);
    let kept_since = |width| match width {
        RollupWidth::FiveMinutes => Some(cutoffs(retention, now).five_minutes),
        RollupWidth::Hourly => Some(cutoffs(retention, now).hourly),
        RollupWidth::Daily => None,
    };
    RollupWidth::ALL
        .into_iter()
        .find(|&width| {
            span / width.seconds() <= MAX_POINTS
                && kept_since(width).is_none_or(|since| from >= since)
        })
        .unwrap_or(RollupWidth::Daily)
}

/// Prune every hour, the first time a minute after the start. Reads the
/// current database each time: a reload may switch it (D-38).
pub fn spawn_pruner(storage: Reloadable<Arc<dyn Storage>>, clock: Arc<dyn Clock>) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            let store = storage.get();
            match prune(store.as_ref(), clock.now()).await {
                Ok(pruned) if pruned != PrunedHistory::default() => tracing::info!(
                    reports = pruned.reports,
                    five_minute_buckets = pruned.five_minute_buckets,
                    hourly_buckets = pruned.hourly_buckets,
                    "Pruned history past its retention"
                ),
                Ok(_) => {}
                Err(e) => tracing::error!("Pruning history failed: {e:#}"),
            }
            tokio::time::sleep(PRUNE_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn cutoffs_count_back_from_now() {
        let now = t("2026-03-31T12:00:00Z");
        let c = cutoffs(&HistoryRetention::default(), now);
        assert_eq!(c.raw, t("2026-03-17T12:00:00Z"));
        assert_eq!(c.five_minutes, t("2025-12-31T12:00:00Z"));
        assert_eq!(c.hourly, t("2024-03-31T12:00:00Z"));
    }

    #[test]
    fn the_width_is_the_finest_that_is_kept_and_fits() {
        let now = t("2026-03-31T12:00:00Z");
        let r = HistoryRetention::default();
        let back = |days| now - Duration::days(days);
        // 24 hours: 288 five-minute buckets.
        assert_eq!(width_for(&r, back(1), now, now), RollupWidth::FiveMinutes);
        // 7 days: 2016 five-minute buckets is too many; 168 hourly.
        assert_eq!(width_for(&r, back(7), now, now), RollupWidth::Hourly);
        // A day, 100 days ago: the 5-minute ones are gone (90 days).
        assert_eq!(width_for(&r, back(100), back(99), now), RollupWidth::Hourly);
        // A year: 8760 hourly buckets is too many.
        assert_eq!(width_for(&r, back(365), now, now), RollupWidth::Daily);
        // Three years back: only daily rollups are kept that long.
        assert_eq!(
            width_for(&r, back(1100), back(1099), now),
            RollupWidth::Daily
        );
    }
}
