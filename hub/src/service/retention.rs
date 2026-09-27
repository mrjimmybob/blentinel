//! Database size and archiving: the rules shared by the admin API and the
//! background monitor (previously written twice, once in each).

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;

use crate::clock::Clock;
use crate::config::{HubConfig, RetentionConfig};
use crate::model::StorageInfo;
use crate::state::Reloadable;
use crate::storage::{ArchiveCreated, ArchiveStore, Storage, StorageResult};

/// Size, limits and last archive, for the admin page and the header badge.
/// The size reads as 0 if it cannot be determined.
pub async fn storage_info(store: &dyn ArchiveStore, retention: &RetentionConfig) -> StorageInfo {
    let current_db_size_mb = store.size_mb().await.unwrap_or(0);
    let (last_archive_date, last_archive_size_mb) = match store.archives().await {
        Ok(archives) => archives
            .first() // newest first
            .map(|a| (Some(a.created_at.clone()), Some(a.size_mb)))
            .unwrap_or((None, None)),
        Err(_) => (None, None),
    };
    StorageInfo {
        current_db_size_mb,
        warn_threshold_mb: retention.warn_db_size_mb,
        retention_days: retention.archive_older_than_days,
        last_archive_date,
        last_archive_size_mb,
        warning: current_db_size_mb >= retention.warn_db_size_mb,
    }
}

/// Archive everything older than `older_than_days` (default: the retention
/// window) as of `now`.
pub async fn archive_old_data(
    store: &dyn ArchiveStore,
    retention: &RetentionConfig,
    older_than_days: Option<u32>,
    now: DateTime<Utc>,
) -> StorageResult<ArchiveCreated> {
    let days = older_than_days.unwrap_or(retention.archive_older_than_days);
    let cutoff = now - Duration::days(i64::from(days));
    store
        .create_archive(&retention.archive_path, cutoff, now)
        .await
}

/// One monitor check: warn when the database is over its size limit and, if
/// `retention.auto` is set, archive old data.
pub async fn check_size(store: &dyn ArchiveStore, retention: &RetentionConfig, now: DateTime<Utc>) {
    if !retention.enabled {
        return;
    }
    let size = match store.size_mb().await {
        Ok(size) => size,
        Err(e) => {
            tracing::error!("Archive monitor: Failed to get DB size: {}", e);
            return;
        }
    };
    if size < retention.warn_db_size_mb {
        return;
    }

    tracing::warn!(
        "Archive monitor: Database size ({} MB) exceeds threshold ({} MB)",
        size,
        retention.warn_db_size_mb
    );
    if !retention.auto {
        tracing::warn!("Archive monitor: Auto-archive disabled. Manual archiving recommended.");
        return;
    }

    tracing::info!("Archive monitor: Auto-archive enabled, initiating archive process...");
    match archive_old_data(store, retention, None, now).await {
        Ok(result) => tracing::info!(
            "Archive monitor: Auto-archive completed: {} ({} MB)",
            result.filename,
            result.size_mb
        ),
        Err(e) => tracing::error!("Archive monitor: Auto-archive failed: {:#}", e),
    }
}

/// Run [`check_size`] every 10 minutes in the background.
pub fn spawn_size_monitor(
    storage: Reloadable<Arc<dyn Storage>>,
    config: Arc<RwLock<HubConfig>>,
    clock: Arc<dyn Clock>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            let retention = config.read().await.retention.clone(); // hot-reloadable
            // The current database: a reload may have switched it (D-38).
            let storage = storage.get();
            check_size(storage.as_ref(), &retention, clock.now()).await;
        }
    });
}
