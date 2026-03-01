use crate::config::{self, Config};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Watch the config file for changes and reload it when a modification is detected.
///
/// Runs indefinitely as a spawned task.  Errors from the underlying file watcher
/// are logged and retried; the task never exits on its own.
pub async fn watch_config(config: Arc<RwLock<Config>>) {
    let config_path = config::get_config_path();
    info!("Hot reload watching: {}", config_path.display());

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
                error!("Failed to create file watcher: {}. Retrying in 30s", e);
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        // Start watching the config file
        if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
            error!("Failed to watch config file: {}. Retrying in 30s", e);
            sleep(Duration::from_secs(30)).await;
            continue;
        }

        // Process file change events
        while let Some(_event) = rx.recv().await {
            // Debounce: wait 500ms for additional events (handles editors that write in chunks)
            sleep(Duration::from_millis(500)).await;

            // Drain any additional events that came in during debounce
            while rx.try_recv().is_ok() {}

            info!("Config file changed, reloading");

            // Try to load the new config
            match config::load() {
                Ok(new_config) => {
                    let mut cfg = config.write().await;
                    *cfg = new_config;
                    info!(
                        hub_url = %cfg.agent.hub_url,
                        interval_secs = cfg.agent.interval,
                        resources = cfg.resources.len(),
                        "Configuration reloaded successfully"
                    );
                }
                Err(e) => {
                    error!("Failed to reload config: {}. Keeping previous configuration", e);
                }
            }
        }

        // If we exit the inner loop, the watcher died — recreate it
        warn!("File watcher stopped unexpectedly, recreating in 5s");
        sleep(Duration::from_secs(5)).await;
    }
}
