//! Embedded SQLite backend, the default (PP-3).
//!
//! `SqliteStorage` implements the [`crate::storage`] traits by delegating to
//! the SQL in `queries.rs`. Nothing outside this module sees a pool or SQL.

mod acme;
mod archive;
mod audit;
mod backups;
mod checks;
mod commands;
mod companies;
mod probes;
mod queries;
mod rotation;
mod scope;
mod thresholds;
mod users;

pub(super) use queries::latest_schema_version;

use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::checks::CheckId;
use common::models::StatusReport;
use sqlx::SqlitePool;

use super::{
    AcmeStatus, AcmeStore, AlertState, AlertStore, ApprovedVersion, ArchiveCreated, ArchiveStore,
    AuditFilter, AuditStore, BackupRun, BackupStore, CheckRecord, CheckStore, CommandRecord,
    CommandStore, CompanyStore, NewAuditEntry, NewEnrollmentToken, NewRotation, NewUser, ProbePins,
    ProbeRecord, ProbeRecordStore, ProbeStore, RedeemedToken, ReportStore, ReportedState,
    RotationRecord, RotationStore, SaveOutcome, Storage, StorageResult, StoredUser, ThresholdStore,
    UptimeRange, UserStore, UserUpdate,
};
use crate::model::{
    AcmeSettings, AdminProbe, AlertSilence, ArchiveMetadata, BackupSchedule, CommandState,
    CompanyProbe, DashboardCompany, Evaluation, ProbeDevice, RotationState, Thresholds,
    UptimeBucket, UserInfo,
};
use crate::model::{
    AuditEntry, CompanyDetail, CompanyInfo, CompanyScope, EnrollmentCodeInfo, ProbeInfo,
    UpdateCompanyRequest, UpdateSiteRequest,
};

/// The hub's database in a single SQLite file.
#[derive(Debug, Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    path: PathBuf,
}

impl SqliteStorage {
    /// Open (creating if needed) the database file and apply pending migrations.
    /// Refuses databases that predate migrations (DECISIONS.md D-30).
    pub async fn open(path: &Path) -> StorageResult<Self> {
        let pool = queries::open(path).await?;
        Ok(Self {
            pool,
            path: path.to_path_buf(),
        })
    }

    /// Open an existing file read-only, without migrating (used for archives).
    pub async fn open_read_only(path: &Path) -> StorageResult<Self> {
        if !path.exists() {
            anyhow::bail!("database file does not exist: {}", path.display());
        }
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false);
        let pool = SqlitePool::connect_with(options)
            .await
            .with_context(|| format!("failed to open {} read-only", path.display()))?;
        Ok(Self {
            pool,
            path: path.to_path_buf(),
        })
    }

    /// The underlying pool. SQLite-specific: only for tests and diagnostics
    /// that deliberately inspect the file. Business code must use the traits.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl ReportStore for SqliteStorage {
    async fn save_report(
        &self,
        report: &StatusReport,
        evaluations: &[Evaluation],
    ) -> StorageResult<SaveOutcome> {
        queries::save_report(&self.pool, report, evaluations).await
    }

    async fn probe_devices(&self, probe_id: &str) -> StorageResult<Vec<ProbeDevice>> {
        checks::latest_devices(&self.pool, probe_id).await
    }

    async fn company_uptime(
        &self,
        company_id: &str,
        range: UptimeRange,
        now: DateTime<Utc>,
    ) -> StorageResult<Vec<UptimeBucket>> {
        queries::get_company_uptime_history(&self.pool, company_id, range, now).await
    }

    async fn latest_report_at(&self) -> StorageResult<Option<DateTime<Utc>>> {
        queries::latest_report_at(&self.pool).await
    }

    async fn delete_company_reports(&self, company_id: &str) -> StorageResult<()> {
        queries::delete_company_data(&self.pool, company_id).await
    }

    async fn delete_probe_reports(&self, probe_id: &str) -> StorageResult<()> {
        queries::delete_probe_data(&self.pool, probe_id).await
    }
}

#[async_trait]
impl ProbeStore for SqliteStorage {
    async fn record_heartbeat(
        &self,
        probe_id: &str,
        company_id: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        queries::upsert_heartbeat(&self.pool, probe_id, company_id, now).await
    }

    async fn expire_silent_probes(
        &self,
        timeout_secs: u64,
        now: DateTime<Utc>,
    ) -> StorageResult<Vec<(String, String)>> {
        queries::check_expired_probes(&self.pool, timeout_secs, now).await
    }

    async fn dashboard_companies(
        &self,
        scope: &CompanyScope,
    ) -> StorageResult<Vec<DashboardCompany>> {
        queries::get_dashboard_companies(&self.pool, scope).await
    }

    async fn probe_company(&self, probe_id: &str) -> StorageResult<Option<String>> {
        queries::probe_company(&self.pool, probe_id).await
    }

    async fn company_probes(&self, company_id: &str) -> StorageResult<Vec<CompanyProbe>> {
        queries::get_company_probes(&self.pool, company_id).await
    }

    async fn company_ids(&self) -> StorageResult<Vec<String>> {
        queries::get_all_companies(&self.pool).await
    }

    async fn all_probes(&self) -> StorageResult<Vec<AdminProbe>> {
        queries::get_all_probes(&self.pool).await
    }

    async fn remove_probe(&self, probe_id: &str) -> StorageResult<()> {
        queries::remove_probe(&self.pool, probe_id).await
    }
}

#[async_trait]
impl AlertStore for SqliteStorage {
    async fn alert_state(&self, key: &str) -> StorageResult<Option<AlertState>> {
        queries::get_alert_state(&self.pool, key).await
    }

    async fn upsert_alert_state(
        &self,
        key: &str,
        status: &str,
        streak: u32,
        alert_sent: bool,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        queries::upsert_alert_state(&self.pool, key, status, streak, alert_sent, now).await
    }

    async fn set_alert_status_if_present(&self, key: &str, status: &str) -> StorageResult<()> {
        queries::set_alert_status_if_present(&self.pool, key, status).await
    }

    async fn create_silence(
        &self,
        scope_type: &str,
        scope_id: &str,
        reason: &str,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> StorageResult<i64> {
        queries::create_silence(&self.pool, scope_type, scope_id, reason, expires_at, now).await
    }

    async fn active_silences(&self, now: DateTime<Utc>) -> StorageResult<Vec<AlertSilence>> {
        queries::get_active_silences(&self.pool, now).await
    }

    async fn is_silenced(
        &self,
        scope_type: &str,
        scope_id: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        queries::is_silenced(&self.pool, scope_type, scope_id, now).await
    }

    async fn delete_silence(&self, silence_id: i64) -> StorageResult<()> {
        queries::delete_silence(&self.pool, silence_id).await
    }

    async fn clear_check_silence(&self, check_id: &CheckId) -> StorageResult<()> {
        queries::clear_check_silence(&self.pool, check_id.as_str()).await
    }
}

#[async_trait]
impl UserStore for SqliteStorage {
    async fn create_user(&self, user: &NewUser, now: DateTime<Utc>) -> StorageResult<()> {
        users::create_user(&self.pool, user, now).await
    }

    async fn user(&self, id: &str) -> StorageResult<Option<StoredUser>> {
        users::user(&self.pool, id).await
    }

    async fn user_by_username(&self, username: &str) -> StorageResult<Option<StoredUser>> {
        users::user_by_username(&self.pool, username).await
    }

    async fn users(&self) -> StorageResult<Vec<UserInfo>> {
        users::users(&self.pool).await
    }

    async fn user_count(&self) -> StorageResult<u64> {
        users::user_count(&self.pool).await
    }

    async fn active_owner_count(&self) -> StorageResult<u64> {
        users::active_owner_count(&self.pool).await
    }

    async fn update_user(&self, id: &str, update: &UserUpdate) -> StorageResult<()> {
        users::update_user(&self.pool, id, update).await
    }

    async fn set_password(
        &self,
        id: &str,
        password_hash: &str,
        must_change_password: bool,
    ) -> StorageResult<()> {
        users::set_password(&self.pool, id, password_hash, must_change_password).await
    }

    async fn record_login(&self, id: &str, now: DateTime<Utc>) -> StorageResult<()> {
        users::record_login(&self.pool, id, now).await
    }

    async fn create_session(
        &self,
        id_hash: &str,
        user_id: &str,
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> StorageResult<()> {
        users::create_session(&self.pool, id_hash, user_id, now, expires_at).await
    }

    async fn resume_session(
        &self,
        id_hash: &str,
        now: DateTime<Utc>,
        idle_timeout: chrono::Duration,
    ) -> StorageResult<Option<StoredUser>> {
        users::resume_session(&self.pool, id_hash, now, idle_timeout).await
    }

    async fn delete_session(&self, id_hash: &str) -> StorageResult<()> {
        users::delete_session(&self.pool, id_hash).await
    }

    async fn delete_user_sessions(&self, user_id: &str, keep: Option<&str>) -> StorageResult<()> {
        users::delete_user_sessions(&self.pool, user_id, keep).await
    }

    async fn delete_dead_sessions(
        &self,
        now: DateTime<Utc>,
        idle_timeout: chrono::Duration,
    ) -> StorageResult<u64> {
        users::delete_dead_sessions(&self.pool, now, idle_timeout).await
    }
}

#[async_trait]
impl CompanyStore for SqliteStorage {
    async fn create_company(
        &self,
        id: &str,
        name: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        companies::create_company(&self.pool, id, name, now).await
    }

    async fn create_site(
        &self,
        company_id: &str,
        id: &str,
        name: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        companies::create_site(&self.pool, company_id, id, name, now).await
    }

    async fn companies(&self, scope: &CompanyScope) -> StorageResult<Vec<CompanyInfo>> {
        companies::companies(&self.pool, scope).await
    }

    async fn company(&self, id: &str) -> StorageResult<Option<CompanyDetail>> {
        companies::company(&self.pool, id).await
    }

    async fn update_company(
        &self,
        id: &str,
        fields: &UpdateCompanyRequest,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        companies::update_company(&self.pool, id, fields, now).await
    }

    async fn set_company_archived(
        &self,
        id: &str,
        archived: bool,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        companies::set_company_archived(&self.pool, id, archived, now).await
    }

    async fn update_site(
        &self,
        company_id: &str,
        site_id: &str,
        fields: &UpdateSiteRequest,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        companies::update_site(&self.pool, company_id, site_id, fields, now).await
    }
}

#[async_trait]
impl ProbeRecordStore for SqliteStorage {
    async fn probe_record(&self, id: &str) -> StorageResult<Option<ProbeRecord>> {
        probes::record(&self.pool, id).await
    }

    async fn save_probe_record(&self, record: &ProbeRecord) -> StorageResult<()> {
        probes::save(&self.pool, record).await
    }

    async fn probe_records(&self, scope: &CompanyScope) -> StorageResult<Vec<ProbeInfo>> {
        probes::list(&self.pool, scope).await
    }

    async fn update_probe_record(
        &self,
        id: &str,
        name: &str,
        site_id: &str,
        interval_secs: u32,
    ) -> StorageResult<bool> {
        probes::update(&self.pool, id, name, site_id, interval_secs).await
    }

    async fn record_probe_state(&self, id: &str, state: &ReportedState<'_>) -> StorageResult<()> {
        probes::record_state(&self.pool, id, state).await
    }

    async fn approved_probe_version(&self) -> StorageResult<Option<ApprovedVersion>> {
        probes::approved_version(&self.pool).await
    }

    async fn set_approved_probe_version(
        &self,
        approval: Option<&ApprovedVersion>,
    ) -> StorageResult<()> {
        probes::set_approved_version(&self.pool, approval).await
    }

    async fn revoke_probe_record(&self, id: &str, now: DateTime<Utc>) -> StorageResult<bool> {
        probes::revoke(&self.pool, id, now).await
    }

    async fn delete_probe_record(&self, id: &str) -> StorageResult<()> {
        probes::delete(&self.pool, id).await
    }

    async fn create_enrollment_token(
        &self,
        token: &NewEnrollmentToken,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        probes::create_token(&self.pool, token, now).await
    }

    async fn enrollment_tokens(
        &self,
        scope: &CompanyScope,
        now: DateTime<Utc>,
    ) -> StorageResult<Vec<EnrollmentCodeInfo>> {
        probes::tokens(&self.pool, scope, now).await
    }

    async fn enrollment_token_company(&self, id: &str) -> StorageResult<Option<String>> {
        probes::token_company(&self.pool, id).await
    }

    async fn revoke_enrollment_token(&self, id: &str, now: DateTime<Utc>) -> StorageResult<bool> {
        probes::revoke_token(&self.pool, id, now).await
    }

    async fn redeem_enrollment_token(
        &self,
        id: &str,
        secret_hash: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<Option<RedeemedToken>> {
        probes::redeem(&self.pool, id, secret_hash, now).await
    }
}

#[async_trait]
impl CheckStore for SqliteStorage {
    async fn probe_checks(&self, probe_id: &str) -> StorageResult<Vec<CheckRecord>> {
        checks::probe_checks(&self.pool, probe_id).await
    }

    async fn check(&self, id: &CheckId) -> StorageResult<Option<CheckRecord>> {
        checks::check(&self.pool, id).await
    }

    async fn create_check(&self, check: &CheckRecord) -> StorageResult<()> {
        checks::create(&self.pool, check).await
    }

    async fn update_check(&self, check: &CheckRecord) -> StorageResult<bool> {
        checks::update(&self.pool, check).await
    }

    async fn delete_check(&self, id: &CheckId) -> StorageResult<bool> {
        checks::delete(&self.pool, id).await
    }
}

#[async_trait]
impl CommandStore for SqliteStorage {
    async fn create_command(&self, command: &CommandRecord) -> StorageResult<()> {
        commands::create(&self.pool, command).await
    }

    async fn command(&self, id: &str) -> StorageResult<Option<CommandRecord>> {
        commands::get(&self.pool, id).await
    }

    async fn probe_commands(
        &self,
        probe_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<CommandRecord>> {
        commands::of_probe(&self.pool, probe_id, limit).await
    }

    async fn pending_commands(
        &self,
        probe_id: &str,
        now: DateTime<Utc>,
        limit: u32,
    ) -> StorageResult<Vec<CommandRecord>> {
        commands::pending(&self.pool, probe_id, now, limit).await
    }

    async fn mark_commands_delivered(
        &self,
        ids: &[String],
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        commands::mark_delivered(&self.pool, ids, now).await
    }

    async fn complete_command(
        &self,
        probe_id: &str,
        id: &str,
        state: CommandState,
        result: &str,
        now: DateTime<Utc>,
    ) -> StorageResult<bool> {
        commands::complete(&self.pool, probe_id, id, state, result, now).await
    }

    async fn cancel_command(&self, id: &str, now: DateTime<Utc>) -> StorageResult<bool> {
        commands::cancel(&self.pool, id, now).await
    }
}

#[async_trait]
impl AuditStore for SqliteStorage {
    async fn append_audit(&self, entry: &NewAuditEntry) -> StorageResult<()> {
        audit::append(&self.pool, entry).await
    }

    async fn audit_entries(&self, filter: &AuditFilter) -> StorageResult<Vec<AuditEntry>> {
        audit::entries(&self.pool, filter).await
    }
}

#[async_trait]
impl ArchiveStore for SqliteStorage {
    async fn size_mb(&self) -> StorageResult<u64> {
        queries::get_db_size_mb(&self.path.to_string_lossy()).await
    }

    async fn archives(&self) -> StorageResult<Vec<ArchiveMetadata>> {
        queries::get_archives(&self.pool).await
    }

    async fn create_archive(
        &self,
        archive_dir: &str,
        cutoff: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> StorageResult<ArchiveCreated> {
        let request = archive::ArchiveRequest {
            main_db_path: self.path.to_string_lossy().into_owned(),
            archive_path_dir: archive_dir.to_string(),
            cutoff_date: cutoff,
            now,
        };
        archive::create_archive(&self.pool, request).await
    }

    async fn open_archive(
        &self,
        archive_dir: &str,
        archive_id: i64,
    ) -> StorageResult<Option<Box<dyn Storage>>> {
        let archives = self.archives().await?;
        let Some(archive) = archives.iter().find(|a| a.id == archive_id) else {
            return Ok(None);
        };
        let path = Path::new(archive_dir).join(&archive.filename);
        let storage = SqliteStorage::open_read_only(&path).await?;
        Ok(Some(Box::new(storage)))
    }

    async fn backup_to(&self, path: &Path) -> StorageResult<()> {
        queries::vacuum_into(&self.pool, path).await
    }

    async fn schema_version(&self) -> StorageResult<i64> {
        queries::schema_version(&self.pool).await
    }

    async fn check_integrity(&self) -> StorageResult<Result<(), String>> {
        queries::quick_check(&self.pool).await
    }

    async fn close(&self) {
        self.pool.close().await;
    }
}

#[async_trait]
impl BackupStore for SqliteStorage {
    async fn backup_schedule(&self) -> StorageResult<Option<BackupSchedule>> {
        backups::schedule(&self.pool).await
    }

    async fn set_backup_schedule(
        &self,
        schedule: &BackupSchedule,
        updated_by: Option<&str>,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        backups::set_schedule(&self.pool, schedule, updated_by, now).await
    }

    async fn last_backup_run(&self) -> StorageResult<Option<BackupRun>> {
        backups::last_run(&self.pool).await
    }

    async fn record_backup_run(&self, run: &BackupRun) -> StorageResult<()> {
        backups::record_run(&self.pool, run).await
    }
}

#[async_trait]
impl RotationStore for SqliteStorage {
    async fn rotations(&self, limit: u32) -> StorageResult<Vec<RotationRecord>> {
        rotation::rotations(&self.pool, limit).await
    }

    async fn start_rotation(&self, new: &NewRotation) -> StorageResult<i64> {
        rotation::start(&self.pool, new).await
    }

    async fn end_rotation(
        &self,
        id: i64,
        state: RotationState,
        ended_by: Option<&str>,
        at: DateTime<Utc>,
    ) -> StorageResult<bool> {
        rotation::end(&self.pool, id, state, ended_by, at).await
    }

    async fn mark_rotation_warned(&self, id: i64) -> StorageResult<()> {
        rotation::mark_warned(&self.pool, id).await
    }

    async fn active_probe_pins(&self) -> StorageResult<Vec<ProbePins>> {
        rotation::active_probe_pins(&self.pool).await
    }
}

#[async_trait]
impl AcmeStore for SqliteStorage {
    async fn acme_settings(&self) -> StorageResult<Option<AcmeSettings>> {
        acme::settings(&self.pool).await
    }

    async fn set_acme_settings(
        &self,
        settings: &AcmeSettings,
        updated_by: Option<&str>,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        acme::set_settings(&self.pool, settings, updated_by, now).await
    }

    async fn acme_status(&self) -> StorageResult<Option<AcmeStatus>> {
        acme::status(&self.pool).await
    }

    async fn record_acme_attempt(
        &self,
        at: DateTime<Utc>,
        error: Option<&str>,
    ) -> StorageResult<()> {
        acme::record_attempt(&self.pool, at, error).await
    }
}

#[async_trait]
impl ThresholdStore for SqliteStorage {
    async fn threshold_defaults(&self) -> StorageResult<Option<Thresholds>> {
        thresholds::defaults(&self.pool).await
    }

    async fn set_threshold_defaults(
        &self,
        values: &Thresholds,
        updated_by: Option<&str>,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        thresholds::set_defaults(&self.pool, values, updated_by, now).await
    }

    async fn company_thresholds(&self, company_id: &str) -> StorageResult<Thresholds> {
        thresholds::company(&self.pool, company_id).await
    }

    async fn set_company_thresholds(
        &self,
        company_id: &str,
        values: &Thresholds,
    ) -> StorageResult<bool> {
        thresholds::set_company(&self.pool, company_id, values).await
    }

    async fn site_thresholds(&self, company_id: &str, site_id: &str) -> StorageResult<Thresholds> {
        thresholds::site(&self.pool, company_id, site_id).await
    }

    async fn set_site_thresholds(
        &self,
        company_id: &str,
        site_id: &str,
        values: &Thresholds,
    ) -> StorageResult<bool> {
        thresholds::set_site(&self.pool, company_id, site_id, values).await
    }
}
