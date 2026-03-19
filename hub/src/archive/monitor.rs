#![cfg(feature = "ssr")]

use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::config::HubConfig;
use chrono::{Utc, Duration};

pub async fn spawn_db_size_monitor(
    pool: SqlitePool,
    config: Arc<RwLock<HubConfig>>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(600)).await; // 10 minutes

            let (enabled, auto, warn_threshold, archive_days, db_path, archive_path) = {
                let cfg = config.read().await;
                (
                    cfg.retention.enabled,
                    cfg.retention.auto,
                    cfg.retention.warn_db_size_mb,
                    cfg.retention.archive_older_than_days,
                    cfg.server.resolved_db_path().display().to_string(),
                    cfg.retention.archive_path.clone(),
                )
            };

            if !enabled {
                continue;
            }

            // Check current DB size
            let current_size = match crate::db::get_db_size_mb(&db_path).await {
                Ok(size) => size,
                Err(e) => {
                    eprintln!("[Archive Monitor] Failed to get DB size: {}", e);
                    continue;
                }
            };

            if current_size >= warn_threshold {
                eprintln!("[Archive Monitor] WARNING: Database size ({} MB) exceeds threshold ({} MB)",
                    current_size, warn_threshold);

                if auto {
                    eprintln!("[Archive Monitor] Auto-archive enabled, initiating archive process...");

                    let cutoff = Utc::now() - Duration::days(archive_days as i64);
                    let request = crate::archive::engine::ArchiveRequest {
                        main_db_path: db_path.clone(),
                        archive_path_dir: archive_path.clone(),
                        cutoff_date: cutoff,
                    };

                    match crate::archive::engine::create_archive(&pool, request).await {
                        Ok(result) => {
                            eprintln!("[Archive Monitor] Auto-archive completed: {} ({} MB)",
                                result.filename, result.size_mb);
                        }
                        Err(e) => {
                            eprintln!("[Archive Monitor] Auto-archive failed: {:#}", e);
                        }
                    }
                } else {
                    eprintln!("[Archive Monitor] Auto-archive disabled. Manual archiving recommended.");
                }
            }
        }
    });
}
