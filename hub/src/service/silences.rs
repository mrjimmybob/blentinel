//! Silencing alerts for a check (D-49: silences are keyed by `CheckId`).

use chrono::{DateTime, Duration, Utc};
use common::checks::CheckId;

use crate::model::SILENCE_CHECK;
use crate::storage::{AlertStore, StorageResult};

/// Silence alerts for a check: for `duration_hours` from `now`, or forever
/// when `None`. Silencing an already-silenced check replaces its silence.
/// Returns the silence ID.
pub async fn silence_check(
    store: &dyn AlertStore,
    check_id: &CheckId,
    reason: &str,
    duration_hours: Option<u32>,
    now: DateTime<Utc>,
) -> StorageResult<i64> {
    let expires_at = duration_hours.map(|hours| now + Duration::hours(i64::from(hours)));
    store
        .create_silence(SILENCE_CHECK, check_id.as_str(), reason, expires_at, now)
        .await
}
