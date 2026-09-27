#![cfg(feature = "ssr")]

use std::sync::Arc;

use chrono::Duration;
use tokio::sync::RwLock;

use crate::clock::Clock;
use crate::config::HubConfig;
use crate::storage::Storage;

/// Every 10 minutes: warn when the database exceeds the configured size and,
/// if `retention.auto` is on, archive data older than the retention window.
pub async fn spawn_db_size_monitor(
    storage: Arc<dyn Storage>,
    config: Arc<RwLock<HubConfig>>,
    clock: Arc<dyn Clock>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(600)).await; // 10 minutes

            let (enabled, auto, warn_threshold, archive_days, archive_path) = {
                let cfg = config.read().await;
                (
                    cfg.retention.enabled,
                    cfg.retention.auto,
                    cfg.retention.warn_db_size_mb,
                    cfg.retention.archive_older_than_days,
                    cfg.retention.archive_path.clone(),
                )
            };

            if !enabled {
                continue;
            }

            let current_size = match storage.size_mb().await {
                Ok(size) => size,
                Err(e) => {
                    eprintln!("[Archive Monitor] Failed to get DB size: {}", e);
                    continue;
                }
            };

            if current_size >= warn_threshold {
                eprintln!(
                    "[Archive Monitor] WARNING: Database size ({} MB) exceeds threshold ({} MB)",
                    current_size, warn_threshold
                );

                if auto {
                    eprintln!(
                        "[Archive Monitor] Auto-archive enabled, initiating archive process..."
                    );

                    let now = clock.now();
                    let cutoff = now - Duration::days(archive_days as i64);
                    match storage.create_archive(&archive_path, cutoff, now).await {
                        Ok(result) => {
                            eprintln!(
                                "[Archive Monitor] Auto-archive completed: {} ({} MB)",
                                result.filename, result.size_mb
                            );
                        }
                        Err(e) => {
                            eprintln!("[Archive Monitor] Auto-archive failed: {:#}", e);
                        }
                    }
                } else {
                    eprintln!(
                        "[Archive Monitor] Auto-archive disabled. Manual archiving recommended."
                    );
                }
            }
        }
    });
}
