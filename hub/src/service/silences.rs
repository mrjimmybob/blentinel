//! Silencing alerts for a resource.

use chrono::{DateTime, Duration, Utc};

use crate::storage::{AlertStore, StorageResult};

/// Scope type of a silence that covers one resource (company:probe:name:target).
pub const RESOURCE_SCOPE: &str = "resource";

/// Silence alerts for `resource_key`: for `duration_hours` from `now`, or
/// forever when `None`. Silencing an already-silenced resource replaces its
/// silence. Returns the silence ID.
pub async fn silence_resource(
    store: &dyn AlertStore,
    resource_key: &str,
    reason: &str,
    duration_hours: Option<u32>,
    now: DateTime<Utc>,
) -> StorageResult<i64> {
    let expires_at = duration_hours.map(|hours| now + Duration::hours(i64::from(hours)));
    store
        .create_silence(RESOURCE_SCOPE, resource_key, reason, expires_at, now)
        .await
}
