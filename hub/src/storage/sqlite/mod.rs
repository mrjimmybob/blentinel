//! Embedded SQLite backend, the default (PP-3).
//!
//! `SqliteStorage` implements the [`crate::storage`] traits by delegating to
//! the SQL in `queries.rs`. Nothing outside this module sees a pool or SQL.

mod archive;
mod audit;
mod companies;
mod probes;
mod queries;
mod scope;
mod types;
mod users;

use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::models::StatusReport;
use sqlx::SqlitePool;

use super::{
    AlertState, AlertStore, ArchiveCreated, ArchiveStore, AuditFilter, AuditStore, CompanyStore,
    NewAuditEntry, NewEnrollmentToken, NewUser, ProbeRecord, ProbeRecordStore, ProbeStore,
    RedeemedToken, ReportStore, SaveOutcome, Storage, StorageResult, StoredUser, UptimeRange,
    UserStore, UserUpdate,
};
use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice,
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
    async fn save_report(&self, report: &StatusReport) -> StorageResult<SaveOutcome> {
        queries::save_report(&self.pool, report).await
    }

    async fn probe_devices(&self, probe_id: &str) -> StorageResult<Vec<ProbeDevice>> {
        queries::get_probe_devices(&self.pool, probe_id).await
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
    async fn alert_state(&self, resource_key: &str) -> StorageResult<Option<AlertState>> {
        queries::get_alert_state(&self.pool, resource_key).await
    }

    async fn upsert_alert_state(
        &self,
        resource_key: &str,
        status: &str,
        alert_sent: bool,
        now: DateTime<Utc>,
    ) -> StorageResult<()> {
        queries::upsert_alert_state(&self.pool, resource_key, status, alert_sent, now).await
    }

    async fn set_alert_status_if_present(
        &self,
        resource_key: &str,
        status: &str,
    ) -> StorageResult<()> {
        queries::set_alert_status_if_present(&self.pool, resource_key, status).await
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

    async fn clear_resource_silence(&self, resource_key: &str) -> StorageResult<()> {
        queries::clear_silence_by_resource(&self.pool, resource_key).await
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
    ) -> StorageResult<bool> {
        probes::update(&self.pool, id, name, site_id).await
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
}
