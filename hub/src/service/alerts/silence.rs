#![cfg(feature = "ssr")]

use crate::model::SILENCE_CHECK;
use crate::storage::AlertStore;
use anyhow::Result;
use chrono::{DateTime, Utc};
use common::checks::CheckId;

/// Is the check silenced (as of `now`)?
pub async fn is_check_silenced(
    store: &dyn AlertStore,
    check_id: &CheckId,
    now: DateTime<Utc>,
) -> Result<bool> {
    store
        .is_silenced(SILENCE_CHECK, check_id.as_str(), now)
        .await
}

/// A recovered check's silence ends: it was for that outage.
pub async fn clear_silence_on_recovery(store: &dyn AlertStore, check_id: &CheckId) -> Result<()> {
    store.clear_check_silence(check_id).await
}
