//! Probe liveness: find probes that stopped reporting and alert on them.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use super::alerts::{self, Notifier};
use crate::config::HubConfig;
use crate::storage::{Storage, StorageResult};

/// One expiry sweep at `now`: mark probes silent for longer than the configured
/// timeout as expired, then (in the background) alert on them. Returns the
/// newly expired `(probe_id, company_id)` pairs.
pub async fn sweep_expired_probes(
    storage: &Arc<dyn Storage>,
    notifier: &Arc<dyn Notifier>,
    config: &Arc<RwLock<HubConfig>>,
    now: DateTime<Utc>,
) -> StorageResult<Vec<(String, String)>> {
    // Read the timeout on every sweep: it is hot-reloadable.
    let timeout = config.read().await.server.probe_timeout_secs;
    let expired = storage.expire_silent_probes(timeout, now).await?;

    if !expired.is_empty() {
        let (storage, notifier, config, alert_on) = (
            Arc::clone(storage),
            Arc::clone(notifier),
            Arc::clone(config),
            expired.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) = alerts::evaluate_probe_expiry(
                storage.as_ref(),
                notifier.as_ref(),
                &config,
                &alert_on,
                timeout,
                now,
            )
            .await
            {
                tracing::error!("Failed to evaluate probe expiry alerts: {}", e);
            }
        });
    }
    Ok(expired)
}
