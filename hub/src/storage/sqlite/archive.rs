//! SQLite archiving: copy the database file, keep only old data in the copy,
//! then remove that old data from the main database. Backend-specific by
//! nature; reached only through `ArchiveStore` (see `super::SqliteStorage`).

use crate::storage::ArchiveCreated;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

pub(super) struct ArchiveRequest {
    pub main_db_path: String,
    pub archive_path_dir: String,
    pub cutoff_date: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

/// Main archive orchestration function - implements the 12-step archive process
pub(super) async fn create_archive(
    main_pool: &SqlitePool,
    request: ArchiveRequest,
) -> Result<ArchiveCreated> {
    // Step 1: Validate inputs
    validate_request(&request)?;

    // Step 2: Ensure archive directory exists
    ensure_archive_directory(&request.archive_path_dir)?;

    // Step 3: Generate archive filename
    let archive_filename = generate_archive_filename(&request.cutoff_date);
    let archive_full_path = PathBuf::from(&request.archive_path_dir).join(&archive_filename);

    tracing::info!(
        "Archive: Starting archive process for data older than {}",
        request.cutoff_date.format("%Y-%m-%d")
    );

    // Step 4: Flush WAL to ensure consistent copy
    tracing::debug!("Archive: Flushing WAL...");
    flush_wal(main_pool).await?;

    // Step 5: Copy main DB file
    tracing::debug!(
        "Archive: Copying database to {}...",
        archive_full_path.display()
    );
    copy_database_file(&request.main_db_path, &archive_full_path)?;

    // Step 6: Connect to archive DB
    tracing::debug!("Archive: Opening archive database...");
    let archive_pool = open_archive_db_for_write(&archive_full_path).await?;

    // Step 7: Prune NEW data from archive (keep only OLD data)
    tracing::debug!("Archive: Pruning recent data from archive...");
    prune_newer_data(&archive_pool, &request.cutoff_date)
        .await
        .context("Failed to prune archive database")?;

    // Step 8: Vacuum archive DB
    tracing::debug!("Archive: Vacuuming archive database...");
    vacuum_database(&archive_pool).await?;

    // Step 9: Get archive size
    let size_mb = get_file_size_mb(&archive_full_path)?;
    tracing::info!("Archive: Archive size: {} MB", size_mb);

    // Step 10: Insert record into main DB archives table
    tracing::debug!("Archive: Recording archive in main database...");
    super::queries::insert_archive_record(
        main_pool,
        &archive_filename,
        request.cutoff_date,
        size_mb,
        request.now,
    )
    .await?;

    // Step 11: Delete OLD data from main DB
    tracing::debug!("Archive: Removing old data from main database...");
    delete_old_data(main_pool, &request.cutoff_date)
        .await
        .context("Failed to delete old data from main database")?;

    // Step 12: Vacuum main DB
    tracing::debug!("Archive: Vacuuming main database...");
    vacuum_database(main_pool).await?;

    tracing::info!(
        "Archive: Archive completed successfully: {}",
        archive_filename
    );

    Ok(ArchiveCreated {
        filename: archive_filename,
        size_mb,
    })
}

fn validate_request(req: &ArchiveRequest) -> Result<()> {
    if !Path::new(&req.main_db_path).exists() {
        bail!("Main database does not exist: {}", req.main_db_path);
    }
    if req.cutoff_date >= req.now {
        bail!("Cutoff date must be in the past");
    }
    Ok(())
}

fn ensure_archive_directory(path: &str) -> Result<()> {
    std::fs::create_dir_all(path).context(format!("Failed to create archive directory: {}", path))
}

fn generate_archive_filename(cutoff: &DateTime<Utc>) -> String {
    format!("blentinel_archive_{}.db", cutoff.format("%Y%m%d"))
}

async fn flush_wal(pool: &SqlitePool) -> Result<()> {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE);")
        .execute(pool)
        .await?;
    Ok(())
}

fn copy_database_file(src: &str, dest: &Path) -> Result<()> {
    std::fs::copy(src, dest).context(format!("Failed to copy {} to {}", src, dest.display()))?;
    Ok(())
}

async fn open_archive_db_for_write(path: &Path) -> Result<SqlitePool> {
    let path_str = path.to_str().context("Invalid archive path")?;

    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path_str)
        .create_if_missing(false);

    SqlitePool::connect_with(opts)
        .await
        .context("Failed to connect to archive database")
}

async fn prune_newer_data(pool: &SqlitePool, cutoff: &DateTime<Utc>) -> Result<()> {
    let mut tx = pool.begin().await?;

    // The archive keeps reports strictly older than the cutoff; the main
    // database keeps the rest (`delete_old_data`), so no report is in both.
    let deleted_resources = sqlx::query(
        "DELETE FROM resource_statuses WHERE report_id IN
         (SELECT id FROM reports WHERE timestamp >= ?)",
    )
    .bind(cutoff)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let deleted_reports = sqlx::query("DELETE FROM reports WHERE timestamp >= ?")
        .bind(cutoff)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    // Heartbeats (D-26): the archive views group by probe heartbeat, so a probe
    // seen after the cutoff must stay if the archive holds reports from it.
    // Forget only probes the archive knows nothing about.
    sqlx::query(
        "DELETE FROM probe_heartbeats
         WHERE last_seen_at >= ? AND probe_id NOT IN (SELECT probe_id FROM reports)",
    )
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;

    // ...and show it as last seen at its last archived report, not today.
    sqlx::query(
        "UPDATE probe_heartbeats
         SET last_seen_at = (SELECT MAX(r.timestamp) FROM reports r
                             WHERE r.probe_id = probe_heartbeats.probe_id)
         WHERE last_seen_at >= ?",
    )
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE probe_heartbeats SET first_seen_at = MIN(first_seen_at, last_seen_at)")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    tracing::debug!(
        "Archive: Deleted {} reports and {} resource statuses from archive",
        deleted_reports,
        deleted_resources
    );
    Ok(())
}

async fn delete_old_data(pool: &SqlitePool, cutoff: &DateTime<Utc>) -> Result<()> {
    let mut tx = pool.begin().await?;

    // Delete resource_statuses for reports OLDER than cutoff
    let deleted_resources = sqlx::query(
        "DELETE FROM resource_statuses WHERE report_id IN
         (SELECT id FROM reports WHERE timestamp < ?)",
    )
    .bind(cutoff)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    // Delete reports OLDER than cutoff
    let deleted_reports = sqlx::query("DELETE FROM reports WHERE timestamp < ?")
        .bind(cutoff)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    tx.commit().await?;
    tracing::debug!(
        "Archive: Deleted {} reports and {} resource statuses from main database",
        deleted_reports,
        deleted_resources
    );
    Ok(())
}

async fn vacuum_database(pool: &SqlitePool) -> Result<()> {
    sqlx::query("VACUUM;").execute(pool).await?;
    Ok(())
}

fn get_file_size_mb(path: &Path) -> Result<i64> {
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.len() / (1024 * 1024)) as i64)
}
