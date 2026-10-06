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
use common::checks::{CheckId, CheckSpec, ConfigRejection};
use common::commands::CommandSpec;
use common::models::{StatusReport, UpdateFailure};

use crate::model::{
    AcmeSettings, AdminProbe, AlertAfter, AlertSilence, ArchiveMetadata, AuditEntry,
    BackupSchedule, CommandState, CompanyDetail, CompanyInfo, CompanyProbe, CompanyScope,
    DashboardCompany, EnrollmentCodeInfo, ProbeDevice, ProbeInfo, Role, RotationState,
    UpdateCompanyRequest, UpdateSiteRequest, UptimeBucket, UserInfo,
};

pub use sqlite::SqliteStorage;

/// Open (creating and migrating if needed) the configured database. The one
/// place a backend is chosen: at startup and when a reload changes `db_path`.
pub async fn open(db_path: &std::path::Path) -> StorageResult<std::sync::Arc<dyn Storage>> {
    Ok(std::sync::Arc::new(SqliteStorage::open(db_path).await?))
}

/// Open an existing database read-only, without migrating it (`hub status`,
/// `hub doctor`: they must not change what they inspect).
pub async fn open_read_only(
    db_path: &std::path::Path,
) -> StorageResult<std::sync::Arc<dyn Storage>> {
    Ok(std::sync::Arc::new(
        SqliteStorage::open_read_only(db_path).await?,
    ))
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

/// Last alert decision recorded for an alert key (a check ID, its threshold key, a probe expiry key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertState {
    pub key: String,
    /// The confirmed status: what people were last told (or would have been).
    pub last_status: String,
    pub last_alert_sent_at: Option<DateTime<Utc>>,
    /// Results in a row that disagree with `last_status` (M3-02, D-63).
    pub streak: u32,
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
    pub scope: CompanyScope,
    pub must_change_password: bool,
}

/// The editable fields of a user.
#[derive(Debug, Clone, PartialEq)]
pub struct UserUpdate {
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    pub scope: CompanyScope,
    pub is_active: bool,
}

/// A probe record (M2-01, D-47): who the probe is and where it belongs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeRecord {
    /// Hex of its Ed25519 public key.
    pub id: String,
    pub name: String,
    pub company_id: String,
    pub site_id: String,
    /// `active` or `revoked`.
    pub state: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub probe_version: Option<String>,
    pub enrolled_at: DateTime<Utc>,
    pub token_id: Option<String>,
    /// Seconds between two reports, sent to the probe with its checks (D-49).
    /// Kept when the probe enrolls again.
    pub interval_secs: u32,
    /// The config revision the probe last said it runs (`None`: none yet).
    /// Kept when the probe enrolls again, like its checks.
    pub config_revision: Option<String>,
    /// The last config the probe refused, and why.
    pub config_rejected: Option<ConfigRejection>,
    /// The last update the probe refused or rolled back (D-55).
    pub update_failed: Option<UpdateFailure>,
}

/// What a report says about what the probe runs ([`ProbeRecordStore::record_probe_state`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReportedState<'a> {
    pub config_revision: Option<&'a str>,
    pub config_rejected: Option<&'a ConfigRejection>,
    /// `None` (a v4 probe) keeps the version on record.
    pub probe_version: Option<&'a str>,
    pub update_failed: Option<&'a UpdateFailure>,
    /// The hub pins it holds (v7, D-61); `None` (a v6 probe) keeps the record.
    pub pins: Option<&'a common::models::PinDigests>,
}

/// The release probes are offered, and who chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedVersion {
    pub version: String,
    pub approved_by: Option<String>,
    pub approved_at: DateTime<Utc>,
}

impl ProbeRecord {
    pub fn is_active(&self) -> bool {
        self.state == PROBE_ACTIVE
    }
}

pub const PROBE_ACTIVE: &str = "active";
pub const PROBE_REVOKED: &str = "revoked";

/// A check (M2-04, D-49): one thing one probe looks at.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRecord {
    pub id: CheckId,
    pub probe_id: String,
    /// The probe's company (read from the probe record; ignored when saving).
    pub company_id: String,
    pub name: String,
    pub spec: CheckSpec,
    /// Disabled checks stay on the hub but aren't sent to the probe.
    pub enabled: bool,
    /// When it alerts (hub-only; not in the probe's config).
    pub alert_after: AlertAfter,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Username of whoever changed it last.
    pub updated_by: Option<String>,
}

/// A command for a probe (M2-06, D-56).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRecord {
    pub id: String,
    pub probe_id: String,
    /// The probe's company (read from the probe record; ignored when saving).
    pub company_id: String,
    pub spec: CommandSpec,
    /// Username of whoever queued it.
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Not sent after this.
    pub expires_at: DateTime<Utc>,
    /// As stored: never [`CommandState::Expired`], which is worked out on
    /// reading (`service::commands`).
    pub state: CommandState,
    pub delivered_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// The probe's message, or the hub's reason.
    pub result: Option<String>,
}

/// An enrollment code to store. The secret itself is never stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEnrollmentToken {
    pub id: String,
    pub secret_hash: String,
    pub company_id: String,
    pub site_id: String,
    pub probe_name: Option<String>,
    pub created_by: Option<String>,
    pub expires_at: DateTime<Utc>,
    /// How many probes may enroll with it; `None`: any number until it
    /// expires or is revoked.
    pub max_uses: Option<u32>,
}

/// Where a redeemed code enrolls its probe.
#[derive(Debug, Clone, PartialEq)]
pub struct RedeemedToken {
    pub company_id: String,
    pub site_id: String,
    pub probe_name: Option<String>,
}

/// An audit entry to record (M1-06). `user_id`/`username` are `None` for
/// the hub itself or the server console.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewAuditEntry {
    pub at: DateTime<Utc>,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub actor_ip: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub company_id: Option<String>,
    pub details: Option<String>,
}

/// Parsed filters for [`AuditStore::audit_entries`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuditFilter {
    pub username: Option<String>,
    pub action_prefix: Option<String>,
    pub target: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub before_id: Option<i64>,
    pub limit: u32,
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

    /// One row per company in `scope` that has probes, with probe/device
    /// counts, severity and display name, by company ID. Archived companies
    /// are left out.
    async fn dashboard_companies(
        &self,
        scope: &CompanyScope,
    ) -> StorageResult<Vec<DashboardCompany>>;

    /// The company a probe reports for (its liveness record's, else its
    /// latest report's); `None` for an unknown probe.
    async fn probe_company(&self, probe_id: &str) -> StorageResult<Option<String>>;

    /// The company's probes, most recently seen first, named from their records
    /// (empty for a probe without one).
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
    async fn alert_state(&self, key: &str) -> StorageResult<Option<AlertState>>;

    /// Record the confirmed status and the streak of results against it;
    /// `alert_sent` also stamps `last_alert_sent_at = now`.
    async fn upsert_alert_state(
        &self,
        key: &str,
        status: &str,
        streak: u32,
        alert_sent: bool,
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// Change the status of an *existing* alert state; no-op if absent or unchanged.
    async fn set_alert_status_if_present(&self, key: &str, status: &str) -> StorageResult<()>;

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

    /// Remove the silence of a check, if any.
    async fn clear_check_silence(&self, check_id: &CheckId) -> StorageResult<()>;
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

/// Companies and their sites as records (proposal 0002 §4.1, D-44). Keyed by
/// the text probes send (`company_id`, `site`), which never changes.
#[async_trait]
pub trait CompanyStore: Send + Sync {
    /// Create a company (marked reviewed). `false` if the ID is taken.
    async fn create_company(&self, id: &str, name: &str, now: DateTime<Utc>)
    -> StorageResult<bool>;

    /// Create a site of an existing company. `false` if the ID is taken
    /// there (or the company doesn't exist).
    async fn create_site(
        &self,
        company_id: &str,
        id: &str,
        name: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool>;

    /// Every company in `scope`, archived ones included, by display name.
    async fn companies(&self, scope: &CompanyScope) -> StorageResult<Vec<CompanyInfo>>;

    /// A company and its sites (by display name).
    async fn company(&self, id: &str) -> StorageResult<Option<CompanyDetail>>;

    /// Replace the editable fields and mark the company reviewed. `false` if no such company.
    async fn update_company(
        &self,
        id: &str,
        fields: &UpdateCompanyRequest,
        now: DateTime<Utc>,
    ) -> StorageResult<bool>;

    /// Archive or unarchive (also marks it reviewed). `false` if no such company.
    async fn set_company_archived(
        &self,
        id: &str,
        archived: bool,
        now: DateTime<Utc>,
    ) -> StorageResult<bool>;

    /// Replace a site's editable fields and mark it reviewed. `false` if no such site.
    async fn update_site(
        &self,
        company_id: &str,
        site_id: &str,
        fields: &UpdateSiteRequest,
        now: DateTime<Utc>,
    ) -> StorageResult<bool>;
}

/// Probe records and enrollment codes (M2-01, D-47).
#[async_trait]
pub trait ProbeRecordStore: Send + Sync {
    async fn probe_record(&self, id: &str) -> StorageResult<Option<ProbeRecord>>;

    /// Insert the probe, or replace its record (re-enrollment of the same
    /// key). A replaced record keeps its interval and config state, like its checks.
    async fn save_probe_record(&self, record: &ProbeRecord) -> StorageResult<()>;

    /// Probes in `scope` (revoked ones too), with company and site names and
    /// their last report, by company then name.
    async fn probe_records(&self, scope: &CompanyScope) -> StorageResult<Vec<ProbeInfo>>;

    /// Rename, move within its company, set its report interval. `false` if
    /// no such probe or site.
    async fn update_probe_record(
        &self,
        id: &str,
        name: &str,
        site_id: &str,
        interval_secs: u32,
    ) -> StorageResult<bool>;

    /// What the probe said in its latest report about what it runs: config
    /// (D-49), version and failed update (D-55).
    async fn record_probe_state(&self, id: &str, state: &ReportedState<'_>) -> StorageResult<()>;

    /// The probe release the admin approved, if any (D-55).
    async fn approved_probe_version(&self) -> StorageResult<Option<ApprovedVersion>>;

    /// Approve a release for every probe, or none (`None`: no updates).
    async fn set_approved_probe_version(
        &self,
        approval: Option<&ApprovedVersion>,
    ) -> StorageResult<()>;

    /// Refuse its reports from now on. `false` if no such probe.
    async fn revoke_probe_record(&self, id: &str, now: DateTime<Utc>) -> StorageResult<bool>;

    /// Forget the probe record (its reports are removed separately).
    async fn delete_probe_record(&self, id: &str) -> StorageResult<()>;

    async fn create_enrollment_token(
        &self,
        token: &NewEnrollmentToken,
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// Codes in `scope` that can still be used at `now`, newest first.
    async fn enrollment_tokens(
        &self,
        scope: &CompanyScope,
        now: DateTime<Utc>,
    ) -> StorageResult<Vec<EnrollmentCodeInfo>>;

    /// A code's company, to check who may revoke it.
    async fn enrollment_token_company(&self, id: &str) -> StorageResult<Option<String>>;

    /// Revoke a code. `false` if no such code.
    async fn revoke_enrollment_token(&self, id: &str, now: DateTime<Utc>) -> StorageResult<bool>;

    /// Use the code once, atomically: only if the secret matches and it isn't
    /// expired, used up or revoked. `None` otherwise (deliberately one answer).
    async fn redeem_enrollment_token(
        &self,
        id: &str,
        secret_hash: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<Option<RedeemedToken>>;
}

/// Checks as hub records (M2-04, D-49).
#[async_trait]
pub trait CheckStore: Send + Sync {
    /// The probe's checks, disabled ones too, by name.
    async fn probe_checks(&self, probe_id: &str) -> StorageResult<Vec<CheckRecord>>;

    async fn check(&self, id: &CheckId) -> StorageResult<Option<CheckRecord>>;

    async fn create_check(&self, check: &CheckRecord) -> StorageResult<()>;

    /// Replace name, spec, enabled, alert_after and the `updated_*` fields. `false` if no such check.
    async fn update_check(&self, check: &CheckRecord) -> StorageResult<bool>;

    /// `false` if no such check. Its results stay in the history.
    async fn delete_check(&self, id: &CheckId) -> StorageResult<bool>;
}

/// Commands for probes (M2-06, D-56). A command is `Queued`, then
/// `Delivered` once sent, then `Succeeded` or `Failed` when its result
/// arrives; only a queued one can be `Cancelled`.
#[async_trait]
pub trait CommandStore: Send + Sync {
    async fn create_command(&self, command: &CommandRecord) -> StorageResult<()>;

    async fn command(&self, id: &str) -> StorageResult<Option<CommandRecord>>;

    /// The probe's commands, newest first, at most `limit`.
    async fn probe_commands(&self, probe_id: &str, limit: u32)
    -> StorageResult<Vec<CommandRecord>>;

    /// The probe's queued or delivered commands that haven't expired at
    /// `now`, oldest first, at most `limit`.
    async fn pending_commands(
        &self,
        probe_id: &str,
        now: DateTime<Utc>,
        limit: u32,
    ) -> StorageResult<Vec<CommandRecord>>;

    /// Mark queued commands delivered at `now`; others are left alone.
    async fn mark_commands_delivered(
        &self,
        ids: &[String],
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// Record the outcome (`Succeeded` or `Failed`) of a queued or delivered
    /// command of `probe_id`. `false` if it isn't one.
    async fn complete_command(
        &self,
        probe_id: &str,
        id: &str,
        state: CommandState,
        result: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool>;

    /// Cancel a queued command. `false` if it isn't one.
    async fn cancel_command(&self, id: &str, now: DateTime<Utc>) -> StorageResult<bool>;
}

/// Who did what (M1-06, D-46). Append-only: entries are never changed.
#[async_trait]
pub trait AuditStore: Send + Sync {
    async fn append_audit(&self, entry: &NewAuditEntry) -> StorageResult<()>;

    /// Matching entries, newest first, at most `filter.limit`.
    async fn audit_entries(&self, filter: &AuditFilter) -> StorageResult<Vec<AuditEntry>>;
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

    /// Write a consistent copy of the whole database to `path` (a new file),
    /// while the hub keeps running (`hub backup`, M2-08).
    async fn backup_to(&self, path: &std::path::Path) -> StorageResult<()>;

    /// The newest schema migration applied to this database.
    async fn schema_version(&self) -> StorageResult<i64>;

    /// The database's own consistency check: `Err` with what's wrong.
    async fn check_integrity(&self) -> StorageResult<Result<(), String>>;

    /// Close every connection, so the file can be moved or replaced (a
    /// failed upgrade puts the database back, D-60). Unusable afterwards.
    async fn close(&self);
}

/// A rotation of the hub's certificate and/or key (M2-13, D-61).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationRecord {
    pub id: i64,
    pub rotates_tls: bool,
    pub rotates_key: bool,
    pub window_days: u32,
    pub started_by: Option<String>,
    pub started_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub state: RotationState,
    /// `None` with `ended_at`: the hub switched by itself.
    pub ended_by: Option<String>,
    pub ended_at: Option<DateTime<Utc>>,
    /// The deadline passed and the hub said so (once).
    pub deadline_warned: bool,
}

/// A rotation to record ([`RotationStore::start_rotation`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRotation {
    pub rotates_tls: bool,
    pub rotates_key: bool,
    pub window_days: u32,
    pub started_by: Option<String>,
    pub started_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

/// What an active probe last said it pins (v7): digests, `None` before its
/// first v7 report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbePins {
    pub probe_id: String,
    pub current: Option<String>,
    pub next: Option<String>,
}

/// Rotations of the hub's certificate and key (D-61). The keys are files.
#[async_trait]
pub trait RotationStore: Send + Sync {
    /// The latest `limit` rotations, newest first.
    async fn rotations(&self, limit: u32) -> StorageResult<Vec<RotationRecord>>;

    /// Record a new rotation, `announcing`. Returns its ID.
    async fn start_rotation(&self, rotation: &NewRotation) -> StorageResult<i64>;

    /// End an `announcing` rotation as `state`; `false` if it wasn't announcing.
    async fn end_rotation(
        &self,
        id: i64,
        state: RotationState,
        ended_by: Option<&str>,
        at: DateTime<Utc>,
    ) -> StorageResult<bool>;

    async fn mark_rotation_warned(&self, id: i64) -> StorageResult<()>;

    /// The pins of every active (not revoked) probe.
    async fn active_probe_pins(&self) -> StorageResult<Vec<ProbePins>>;
}

/// The last scheduled backup (D-60).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRun {
    pub ran_at: DateTime<Utc>,
    /// `None`: it worked.
    pub error: Option<String>,
}

/// When the hub backs itself up (M2-12, D-60). The files are on disk.
#[async_trait]
pub trait BackupStore: Send + Sync {
    /// The schedule set on the admin page; `None`: never set (the default applies).
    async fn backup_schedule(&self) -> StorageResult<Option<BackupSchedule>>;

    async fn set_backup_schedule(
        &self,
        schedule: &BackupSchedule,
        updated_by: Option<&str>,
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    async fn last_backup_run(&self) -> StorageResult<Option<BackupRun>>;

    async fn record_backup_run(&self, run: &BackupRun) -> StorageResult<()>;
}

/// How the hub's last request for a certificate went (M2-14, D-62).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcmeStatus {
    pub last_attempt_at: DateTime<Utc>,
    /// `None`: it worked.
    pub last_error: Option<String>,
    /// Failed attempts in a row (0 after one that worked): retries back off.
    pub failures: u32,
    /// The last time a certificate was issued.
    pub last_issued_at: Option<DateTime<Utc>>,
}

/// The hub's certificate from an ACME authority (M2-14, D-62). The
/// certificate and the account key are files.
#[async_trait]
pub trait AcmeStore: Send + Sync {
    /// The settings set on the admin page; `None`: never set (off).
    async fn acme_settings(&self) -> StorageResult<Option<AcmeSettings>>;

    async fn set_acme_settings(
        &self,
        settings: &AcmeSettings,
        updated_by: Option<&str>,
        now: DateTime<Utc>,
    ) -> StorageResult<()>;

    /// `None`: the hub never asked.
    async fn acme_status(&self) -> StorageResult<Option<AcmeStatus>>;

    /// Record an attempt at `at`: `error` `None` means a certificate was
    /// issued (failures go back to 0), otherwise failures count up.
    async fn record_acme_attempt(
        &self,
        at: DateTime<Utc>,
        error: Option<&str>,
    ) -> StorageResult<()>;
}

/// The newest schema version this hub knows (its last migration). A database
/// with a newer one was made by a newer hub.
pub fn latest_schema_version() -> i64 {
    sqlite::latest_schema_version()
}

/// Everything the hub needs from a storage backend.
pub trait Storage:
    ReportStore
    + ProbeStore
    + AlertStore
    + UserStore
    + CompanyStore
    + ProbeRecordStore
    + CheckStore
    + CommandStore
    + AuditStore
    + ArchiveStore
    + BackupStore
    + RotationStore
    + AcmeStore
{
}

impl<
    T: ReportStore
        + ProbeStore
        + AlertStore
        + UserStore
        + CompanyStore
        + ProbeRecordStore
        + CheckStore
        + CommandStore
        + AuditStore
        + ArchiveStore
        + BackupStore
        + RotationStore
        + AcmeStore,
> Storage for T
{
}

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
