#![cfg(feature = "ssr")]

use crate::config::{self, HubConfig};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;

/// Watch `config_path` for modifications and hot-reload the config on change.
///
/// The explicit path parameter lets this work with both the default config
/// location and a custom path supplied via `--config`.
pub async fn watch_config(config: Arc<RwLock<HubConfig>>, config_path: PathBuf) {
    tracing::info!("Watching config file {} for changes", config_path.display());

    loop {
        // Create a channel for file system events
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        // Create the watcher
        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only process modify events
                    if matches!(event.kind, EventKind::Modify(_)) {
                        let _ = tx.blocking_send(event);
                    }
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Config hot reload: failed to create file watcher: {e}; retrying in 30 s");
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        // Start watching the config file
        if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
            tracing::error!("Config hot reload: failed to watch the config file: {e}; retrying in 30 s");
            sleep(Duration::from_secs(30)).await;
            continue;
        }

        // Process file change events
        while let Some(_event) = rx.recv().await {
            // Debounce: wait 500ms for additional events (handles editors that write in chunks)
            sleep(Duration::from_millis(500)).await;

            // Drain any additional events that came in during debounce
            while rx.try_recv().is_ok() {}

            tracing::info!("Config file changed, reloading");

            // Get old config values for comparison
            let old_config = {
                let cfg = config.read().await;
                (
                    cfg.server.host.clone(),
                    cfg.server.port,
                    cfg.server.db_path.clone(),
                    cfg.server.identity_key_path.clone(),
                    cfg.server.auth_token_path.clone(),
                    cfg.probes.len(),
                )
            };

            // Try to load the new config
            match config::load_from(&config_path) {
                Ok(new_config) => {
                    // Check if any restart-required fields changed
                    let mut warnings = Vec::new();

                    if new_config.server.host != old_config.0 {
                        warnings.push(format!(
                            "server.host changed from '{}' to '{}'; restart the hub to apply it",
                            old_config.0, new_config.server.host
                        ));
                    }
                    if new_config.server.port != old_config.1 {
                        warnings.push(format!(
                            "server.port changed from {} to {}; restart the hub to apply it",
                            old_config.1, new_config.server.port
                        ));
                    }
                    if new_config.server.db_path != old_config.2 {
                        warnings.push(format!(
                            "server.db_path changed from '{}' to '{}'; restart the hub to apply it",
                            old_config.2, new_config.server.db_path
                        ));
                    }
                    if new_config.server.identity_key_path != old_config.3 {
                        warnings.push(format!(
                            "server.identity_key_path changed from '{}' to '{}'; restart the hub to apply it",
                            old_config.3, new_config.server.identity_key_path
                        ));
                    }
                    if new_config.server.auth_token_path != old_config.4 {
                        warnings.push(format!(
                            "server.auth_token_path changed from '{}' to '{}'; restart the hub to apply it",
                            old_config.4, new_config.server.auth_token_path
                        ));
                    }

                    // Report probe whitelist changes
                    let probe_change = (new_config.probes.len() as i64) - (old_config.5 as i64);
                    let whitelist_info = if probe_change > 0 {
                        format!("{} probe(s) added to the whitelist", probe_change)
                    } else if probe_change < 0 {
                        format!("{} probe(s) removed from the whitelist", -probe_change)
                    } else {
                        "probe whitelist unchanged".to_string()
                    };

                    // Acquire write lock and update config
                    let mut cfg = config.write().await;
                    *cfg = new_config;

                    tracing::info!("Configuration reloaded: {whitelist_info}");
                    for warning in warnings {
                        tracing::warn!("{warning}");
                    }
                }
                Err(e) => {
                    tracing::error!("Config reload failed, keeping the previous configuration: {e}");
                }
            }
        }

        // If we exit the loop, the watcher died - try to recreate
        tracing::warn!("Config file watcher stopped unexpectedly; recreating it");
        sleep(Duration::from_secs(5)).await;
    }
}
