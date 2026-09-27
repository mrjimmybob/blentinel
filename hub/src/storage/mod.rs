#![cfg(feature = "ssr")]
//! Storage layer: the only way the rest of the hub reaches its database
//! (ENGINEERING.md §2.2, proposal 0011).
//!
//! Business code depends on the [`Storage`] traits, never on SQL or a driver.
//! Each backend implements them in its own dialect and must pass the shared
//! conformance suite (`hub/tests/storage_conformance.rs`). Today there is one
//! backend, embedded SQLite ([`SqliteStorage`]). Optional backends
//! (PostgreSQL, …) are future work (ROADMAP M7).
//!
//! The traits are split by area so each file of business code only needs the
//! part it uses; [`Storage`] bundles them for `AppState`.

pub mod sqlite;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::models::StatusReport;

use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice,
    UptimeBucket,
};

pub use sqlite::SqliteStorage;

/// Open (creating and migrating if needed) the configured database. The one
/// place a backend is chosen: at startup and when a reload changes `db_path`.
pub async fn open(db_path: &std::path::Path) -> StorageResult<std::sync::Arc<dyn Storage>> {
    Ok(std::sync::Arc::new(SqliteStorage::open(db_path).await?))
}

/// Result type for storage operations. A single alias so typed errors
/// (ENGINEERING.md §2.5, ROADMAP M0-11) can be introduced in one place.
pub type StorageResult<T> = anyhow::Result<T>;

// ===========================================================================
// Backend-neutral types
// ===========================================================================

/// Result of [`ReportStore::save_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Stored,
    /// The probe already has a report at or after this timestamp: a replay, a
    /// duplicate delivery, or an out-of-order report. Nothing was written.
    NotNewer,
}

/// Time window and bucket size for uptime history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UptimeRange {
    /// Last 24 hours, 15-minute buckets.
    #[default]
    Hours24,
    /// Last 7 days, 1-hour buckets.
    Days7,
    /// Last 30 days, 4-hour buckets.
    Days30,
    /// Everything retained, 1-day buckets.
    All,
}

impl UptimeRange {
    /// Parse the API's `?range=` value (`24h`, `7d`, `30d`, `all`). Anything
    /// else, including a missing value, means the default 24 hours.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("7d") => UptimeRange::Days7,
            Some("30d") => UptimeRange::Days30,
            Some("all") => UptimeRange::All,
            _ => UptimeRange::Hours24,
        }
    }
}

/// Last alert decision recorded for a resource key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertState {
    pub resource_key: String,
    pub last_status: String,
    pub last_alert_sent_at: Option<DateTime<Utc>>,
}

/// A created archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveCreated {
    pub filename: String,
    pub size_mb: i64,
}

// ===========================================================================
// Traits
// ===========================================================================

/// Probe reports and the per-device results inside them.
#[async_trait]
pub trait ReportStore: Send + Sync {
    /// Store a report only if it is strictly newer than every report already
    /// stored for that probe (replay protection, D-28). Must be atomic.
    async fn save_report(&self, report: &StatusReport) -> StorageResult<SaveOutcome>;

    /// Devices from the probe's latest report, sorted by name.
    async fn probe_devices(&self, probe_id: &str) -> StorageResult<Vec<ProbeDevice>>;

    /// Up/down counts per time bucket for a company, relative to `now`.
    async fn company_uptime(
        &self,
        company_id: &str,
        range: UptimeRange,
        now: DateTime<Utc>,
    ) -> StorageResult<Vec<UptimeBucket>>;

    /// Delete all reports of a company (its probes remain known).
    async fn delete_company_reports(&self, company_id: &str) -> StorageResult<()>;

    /// Delete all reports of a probe (the probe remains known).
    async fn delete_probe_reports(&self, probe_id: &str) -> StorageResult<()>;
}

/// Known probes, their liveness, and the views built from them.
#[async_trait]
pub trait ProbeStore: Send + Sync {
    /// Mark the probe active and seen at `now` (inserting it if new).
    async fn record_heartbeat(
        &self,
        probe_id: &str,
        company_id: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// Mark probes not seen for `timeout_secs` as expired. Returns only the
    /// probes that expired in this call, as `(probe_id, company_id)`.
    async fn expire_silent_probes(
        &self,
        timeout_secs: u64,
        now: DateTime<Utc>,
    ) -> StorageResult<Vec<(String, String)>>;

    /// One row per company with probe/device counts and severity, by company ID.
    async fn dashboard_companies(&self) -> StorageResult<Vec<DashboardCompany>>;

    /// The company's probes, most recently seen first. `probe_name` is left
    /// empty (names come from the hub config, not storage).
    async fn company_probes(&self, company_id: &str) -> StorageResult<Vec<CompanyProbe>>;

    /// Every known company ID, sorted.
    async fn company_ids(&self) -> StorageResult<Vec<String>>;

    /// Every known probe, by company then probe ID.
    async fn all_probes(&self) -> StorageResult<Vec<AdminProbe>>;

    /// Forget a probe completely: its reports and its liveness record.
    async fn remove_probe(&self, probe_id: &str) -> StorageResult<()>;
}

/// Alert decisions and silences.
#[async_trait]
pub trait AlertStore: Send + Sync {
    async fn alert_state(&self, resource_key: &str) -> StorageResult<Option<AlertState>>;

    /// Record the latest status; `alert_sent` also stamps `last_alert_sent_at = now`.
    async fn upsert_alert_state(
        &self,
        resource_key: &str,
        status: &str,
        alert_sent: bool,
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// Change the status of an *existing* alert state; no-op if absent or unchanged.
    async fn set_alert_status_if_present(
        &self,
        resource_key: &str,
        status: &str,
    ) -> StorageResult<()>;

    /// Create or replace the silence for `(scope_type, scope_id)`; returns its ID.
    async fn create_silence(
        &self,
        scope_type: &str,
        scope_id: &str,
        reason: &str,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> StorageResult<i64>;

    /// Silences that have not expired at `now`.
    async fn active_silences(&self, now: DateTime<Utc>) -> StorageResult<Vec<AlertSilence>>;

    async fn is_silenced(
        &self,
        scope_type: &str,
        scope_id: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool>;

    async fn delete_silence(&self, silence_id: i64) -> StorageResult<()>;

    /// Remove the resource-scoped silence for `resource_key`, if any.
    async fn clear_resource_silence(&self, resource_key: &str) -> StorageResult<()>;
}

/// Size management and archives. Archiving is backend-specific by nature
/// (SQLite copies and prunes its file); D-20 may retire it once rollups exist.
#[async_trait]
pub trait ArchiveStore: Send + Sync {
    /// Current size of the database, in whole MB.
    async fn size_mb(&self) -> StorageResult<u64>;

    /// Archives created so far, newest first.
    async fn archives(&self) -> StorageResult<Vec<ArchiveMetadata>>;

    /// Move reports older than `cutoff` into a new archive under `archive_dir`.
    async fn create_archive(
        &self,
        archive_dir: &str,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> StorageResult<ArchiveCreated>;

    /// Open an archive read-only as a storage of its own (`None` if no such archive).
    async fn open_archive(
        &self,
        archive_dir: &str,
        archive_id: i64,
    ) -> StorageResult<Option<Box<dyn Storage>>>;
}

/// Everything the hub needs from a storage backend.
pub trait Storage: ReportStore + ProbeStore + AlertStore + ArchiveStore {}

impl<T: ReportStore + ProbeStore + AlertStore + ArchiveStore> Storage for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_range_parses_known_values_and_defaults_to_24h() {
        assert_eq!(UptimeRange::parse(Some("7d")), UptimeRange::Days7);
        assert_eq!(UptimeRange::parse(Some("30d")), UptimeRange::Days30);
        assert_eq!(UptimeRange::parse(Some("all")), UptimeRange::All);
        assert_eq!(UptimeRange::parse(Some("24h")), UptimeRange::Hours24);
        assert_eq!(UptimeRange::parse(Some("fortnight")), UptimeRange::Hours24);
        assert_eq!(UptimeRange::parse(None), UptimeRange::Hours24);
    }
}
