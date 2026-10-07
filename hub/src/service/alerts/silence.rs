#![cfg(feature = "ssr")]

use crate::storage::AlertStore;
use anyhow::Result;
use common::checks::CheckId;

/// A recovered check's silence ends: it was for that outage.
pub async fn clear_silence_on_recovery(store: &dyn AlertStore, check_id: &CheckId) -> Result<()> {
    store.clear_check_silence(check_id).await
}
