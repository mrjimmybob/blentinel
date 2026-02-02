use crate::config::{self, Config};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;

/// Watch the config file for changes and reload when detected
pub async fn watch_config(config: Arc<RwLock<Config>>, verbose: bool) {
    let config_path = config::get_config_path();

    if verbose {
        println!("[Hot Reload] Watching config file: {}", config_path.display());
    }

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
                eprintln!("[Hot Reload] Failed to create file watcher: {}", e);
                eprintln!("[Hot Reload] Waiting 30s before retry...");
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        // Start watching the config file
        if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
            eprintln!("[Hot Reload] Failed to watch config file: {}", e);
            eprintln!("[Hot Reload] Waiting 30s before retry...");
            sleep(Duration::from_secs(30)).await;
            continue;
        }

        // Process file change events
        while let Some(_event) = rx.recv().await {
            // Debounce: wait 500ms for additional events (handles editors that write in chunks)
            sleep(Duration::from_millis(500)).await;

            // Drain any additional events that came in during debounce
            while rx.try_recv().is_ok() {}

            if verbose {
                println!("[Hot Reload] Config file changed, attempting reload...");
            }

            // Try to load the new config
            match config::load() {
                Ok(new_config) => {
                    // Acquire write lock and update config
                    let mut cfg = config.write().await;
                    *cfg = new_config;

                    println!("✓ Configuration reloaded successfully");

                    if verbose {
                        println!("  - Hub URL: {}", cfg.agent.hub_url);
                        println!("  - Interval: {}s", cfg.agent.interval);
                        println!("  - Resources: {}", cfg.resources.len());
                    }
                }
                Err(e) => {
                    eprintln!("✗ Failed to reload config: {}", e);
                    eprintln!("  Keeping previous configuration");
                }
            }
        }

        // If we exit the loop, the watcher died - try to recreate
        eprintln!("[Hot Reload] Watcher stopped unexpectedly, recreating...");
        sleep(Duration::from_secs(5)).await;
    }
}
