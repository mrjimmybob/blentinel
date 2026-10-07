//! Probe liveness: find probes that stopped reporting and alert on them.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use super::alerts::{self, Notifier};
use crate::config::HubConfig;
use crate::storage::{Storage, StorageResult};

/// One expiry sweep at `now`: mark probes silent for longer than the configured
/// timeout as expired, then (in the background) alert on every expired probe
/// not alerted yet. Returns the newly expired `(probe_id, company_id)` pairs.
pub async fn sweep_expired_probes(
    storage: &Arc<dyn Storage>,
    notifier: &Arc<dyn Notifier>,
    config: &Arc<RwLock<HubConfig>>,
    now: DateTime<Utc>,
) -> StorageResult<Vec<(String, String)>> {
    // Read the timeout on every sweep: it is hot-reloadable.
    let timeout = config.read().await.server.probe_timeout_secs;
    let expired = storage.expire_silent_probes(timeout, now).await?;
    // Every expired probe, not only the new ones: one whose alert waited for
    // a maintenance window or a silence alerts when it ends (D-65). Each
    // probe alerts once per outage (its alert state).
    let alert_on: Vec<(String, String)> = storage
        .all_probes()
        .await?
        .into_iter()
        .filter(|p| p.status == "expired")
        .map(|p| (p.probe_id, p.company_id))
        .collect();

    if !alert_on.is_empty() {
        let (storage, notifier, config) = (
            Arc::clone(storage),
            Arc::clone(notifier),
            Arc::clone(config),
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
