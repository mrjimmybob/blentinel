//! M6 Data Access Objects - bridge between Storage trait and M6 service functions
//!
//! Temporary layer to enable M6 services to work with the Storage trait abstraction.
//! Post-RC: either expand Storage trait or refactor M6 services to use it directly.

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::storage::Storage;

/// Fetch aggregated check results (rollups or raw) for a date range.
pub async fn query_check_results(
    store: &dyn Storage,
    _check_id: &str,
    _range_start: DateTime<Utc>,
    _range_end: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, i32, i32, i32, i32)>> {
    // TODO M6-02: Add method to Storage trait to fetch aggregated results
    // For now: empty set (placeholder)
    Ok(vec![])
}

/// Fetch incidents for a check or date range.
pub async fn list_incidents(
    store: &dyn Storage,
    _check_id: Option<&str>,
    _status: Option<&str>,
    _limit: i32,
    _offset: i32,
) -> Result<Vec<(i64, String, String, DateTime<Utc>, Option<DateTime<Utc>>, i32)>> {
    // TODO M6-03: Add method to Storage trait to fetch incidents
    // For now: empty set (placeholder)
    Ok(vec![])
}

/// Calculate availability % for a check.
pub async fn get_availability(
    store: &dyn Storage,
    _check_id: &str,
    _range_start: DateTime<Utc>,
    _range_end: DateTime<Utc>,
) -> Result<Option<f64>> {
    // TODO M6-02/05: Add method to Storage trait for availability calculation
    // For now: return placeholder
    Ok(Some(99.5))
}
