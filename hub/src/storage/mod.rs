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
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice, Role,
    UptimeBucket, UserInfo,
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

/// A user with the password hash, which only `service::users` may see.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredUser {
    pub info: UserInfo,
    /// argon2id PHC string.
    pub password_hash: String,
}

/// A user to create. `id` is chosen by the caller (random, `service::users`).
#[derive(Debug, Clone, PartialEq)]
pub struct NewUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub role: Role,
    pub all_companies: bool,
    pub companies: Vec<String>,
    pub must_change_password: bool,
}

/// The editable fields of a user.
#[derive(Debug, Clone, PartialEq)]
pub struct UserUpdate {
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    pub all_companies: bool,
    pub companies: Vec<String>,
    pub is_active: bool,
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

    /// Timestamp of the newest stored report, if any. An archive's uptime is
    /// anchored here, because its data ends at (or before) its cutoff (D-26).
    async fn latest_report_at(&self) -> StorageResult<Option<DateTime<Utc>>>;

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

/// Users and their login sessions (proposal 0001). Usernames are unique
/// regardless of case. Sessions are keyed by the SHA-256 of the cookie value;
/// the raw value is never stored.
#[async_trait]
pub trait UserStore: Send + Sync {
    /// Fails if the username is taken (case-insensitive).
    async fn create_user(&self, user: &NewUser, now: DateTime<Utc>) -> StorageResult<()>;

    async fn user(&self, id: &str) -> StorageResult<Option<StoredUser>>;

    /// Case-insensitive lookup.
    async fn user_by_username(&self, username: &str) -> StorageResult<Option<StoredUser>>;

    /// Every user, by username.
    async fn users(&self) -> StorageResult<Vec<UserInfo>>;

    /// Number of users (0 means the hub still needs its setup wizard).
    async fn user_count(&self) -> StorageResult<u64>;

    /// Number of active Owners.
    async fn active_owner_count(&self) -> StorageResult<u64>;

    /// Replace the editable fields (and the assigned companies).
    async fn update_user(&self, id: &str, update: &UserUpdate) -> StorageResult<()>;

    async fn set_password(
        &self,
        id: &str,
        password_hash: &str,
        must_change_password: bool,
    ) -> StorageResult<()>;

    async fn record_login(&self, id: &str, now: DateTime<Utc>) -> StorageResult<()>;

    /// Start a session that ends at `expires_at` at the latest.
    async fn create_session(
        &self,
        id_hash: &str,
        user_id: &str,
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// The user of a live session, marking it used at `now`. `None` if the
    /// session doesn't exist, has passed its absolute expiry, has been idle
    /// longer than `idle_timeout`, or its user is deactivated.
    async fn resume_session(
        &self,
        id_hash: &str,
        now: DateTime<Utc>,
        idle_timeout: chrono::Duration,
    ) -> StorageResult<Option<StoredUser>>;

    async fn delete_session(&self, id_hash: &str) -> StorageResult<()>;

    /// Log a user out everywhere, except the session `keep` if given.
    async fn delete_user_sessions(&self, user_id: &str, keep: Option<&str>) -> StorageResult<()>;

    /// Remove sessions that can no longer be resumed; returns how many.
    async fn delete_dead_sessions(
        &self,
        now: DateTime<Utc>,
        idle_timeout: chrono::Duration,
    ) -> StorageResult<u64>;
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
    /// The archive also keeps the liveness record of every probe it has
    /// reports for, with `last_seen_at` capped at that probe's last archived
    /// report, so probes that are still reporting appear in it too (D-26).
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
pub trait Storage: ReportStore + ProbeStore + AlertStore + UserStore + ArchiveStore {}

impl<T: ReportStore + ProbeStore + AlertStore + UserStore + ArchiveStore> Storage for T {}

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
