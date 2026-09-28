//! Embedded SQLite backend, the default (PP-3).
//!
//! `SqliteStorage` implements the [`crate::storage`] traits by delegating to
//! the SQL in `queries.rs`. Nothing outside this module sees a pool or SQL.

mod archive;
mod queries;
mod types;

use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::models::StatusReport;
use sqlx::SqlitePool;

use super::{
    AlertState, AlertStore, ArchiveCreated, ArchiveStore, ProbeStore, ReportStore, SaveOutcome,
    Storage, StorageResult, UptimeRange,
};
use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice,
    UptimeBucket,
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

    async fn dashboard_companies(&self) -> StorageResult<Vec<DashboardCompany>> {
        queries::get_dashboard_companies(&self.pool).await
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
