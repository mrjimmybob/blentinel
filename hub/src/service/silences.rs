//! Silencing alerts for a check (D-49: silences are keyed by `CheckId`).
//! Wider silences (a probe, a site, a company) are `service::maintenance`'s.

use chrono::{DateTime, Duration, Utc};
use common::checks::CheckId;

use crate::model::Scope;
use crate::storage::{Storage, StorageResult};

/// Silence alerts for a check: for `duration_hours` from `now`, or until it
/// recovers (or is unmuted) when `None`. Silencing an already-silenced check
/// replaces its silence. Returns the silence ID.
pub async fn silence_check(
    store: &dyn Storage,
    check_id: &CheckId,
    reason: &str,
    duration_hours: Option<u32>,
    now: DateTime<Utc>,
) -> StorageResult<i64> {
    let company = store
        .check(check_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such check"))?
        .company_id;
    let expires_at = duration_hours.map(|hours| now + Duration::hours(i64::from(hours)));
    let scope = Scope::Check {
        check_id: check_id.to_string(),
    };
    store
        .create_silence(&company, &scope, reason, expires_at, now)
        .await
}
