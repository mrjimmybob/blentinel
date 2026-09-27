#![cfg(feature = "ssr")]

use crate::storage::AlertStore;
use anyhow::Result;
use chrono::{DateTime, Utc};

/// Check if a resource is currently silenced (as of `now`)
pub async fn is_resource_silenced(
    store: &dyn AlertStore,
    resource_key: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    store.is_silenced("resource", resource_key, now).await
}

/// Clear silence when resource recovers
pub async fn clear_silence_on_recovery(store: &dyn AlertStore, resource_key: &str) -> Result<()> {
    store.clear_resource_silence(resource_key).await
}
