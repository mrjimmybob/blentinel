//! Watching a config file for changes (hot reload), shared by the probe and
//! the hub. Each side decides what a reload means; this module only says when.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

/// How long to wait for more events after the first one. Editors often write
/// a file in several steps; this turns them into one reload.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Wait before retrying when the watcher can't be created or fails.
const RETRY: Duration = Duration::from_secs(30);

/// Call `on_change` every time `path` is modified (debounced). Runs forever:
/// watcher failures are logged and retried, so run it in its own task.
pub async fn watch_file<F, Fut>(path: PathBuf, mut on_change: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    tracing::info!("Watching {} for changes", path.display());
    loop {
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res
                    && matches!(event.kind, EventKind::Modify(_))
                {
                    let _ = tx.blocking_send(());
                }
            },
            notify::Config::default(),
        );
        let mut watcher = match watcher {
            Ok(watcher) => watcher,
            Err(e) => {
                tracing::error!("Can't create a file watcher: {e}; retrying in 30 s");
                sleep(RETRY).await;
                continue;
            }
        };
        if let Err(e) = watcher.watch(&path, RecursiveMode::NonRecursive) {
            tracing::error!("Can't watch {}: {e}; retrying in 30 s", path.display());
            sleep(RETRY).await;
            continue;
        }

        while rx.recv().await.is_some() {
            sleep(DEBOUNCE).await;
            while rx.try_recv().is_ok() {}
            on_change().await;
        }

        // The watcher's channel closed: recreate it.
        tracing::warn!("Watcher for {} stopped; recreating it", path.display());
        sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn a_burst_of_writes_is_one_change() {
        let dir = std::env::temp_dir().join(format!("blentinel-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "a = 1").unwrap();

        let changes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&changes);
        let task = tokio::spawn(watch_file(path.clone(), move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async {}
        }));
        sleep(Duration::from_millis(300)).await; // let the watcher start

        for i in 0..3 {
            std::fs::write(&path, format!("a = {i}")).unwrap();
        }
        // Wait for the debounce window, with slack for slow file systems.
        for _ in 0..50 {
            if changes.load(Ordering::SeqCst) > 0 {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        sleep(DEBOUNCE * 2).await;
        task.abort();

        assert_eq!(changes.load(Ordering::SeqCst), 1);
    }
}
