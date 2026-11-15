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
    // Closed now, not whenever the pool is dropped: the file is complete.
    archive_pool.close().await;

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

/// Refuse to open an archive with a `-wal` file beside it (D-118). The hub
/// writes archives without one, so it is someone else's: say so, and look
/// at nothing. (The archive is opened immutable, so SQLite wouldn't read
/// that file anyway; this is what tells the admin that someone wrote into
/// `retention.archive_path`.)
pub(super) fn check_openable(path: &Path) -> Result<()> {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    if wal.exists() {
        bail!(
            "the archive {} has a write-ahead log beside it ({}), which the hub never writes: \
             not opened. Remove that file if you trust the folder.",
            path.display(),
            wal.display()
        );
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

/// The archive is a copy of the main database, which is in WAL mode. It is
/// switched to a rollback journal here (the mode is kept in the file), so an
/// archive never has a `-wal` file of its own and SQLite never reads one
/// beside it (D-118).
async fn open_archive_db_for_write(path: &Path) -> Result<SqlitePool> {
    let path_str = path.to_str().context("Invalid archive path")?;

    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path_str)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete)
        .create_if_missing(false);

    SqlitePool::connect_with(opts)
        .await
        .context("Failed to connect to archive database")
}

async fn prune_newer_data(pool: &SqlitePool, cutoff: &DateTime<Utc>) -> Result<()> {
    let mut tx = super::write_transaction(pool).await?;

    // The archive keeps reports strictly older than the cutoff; the main
    // database keeps the rest (`delete_old_data`), so no report is in both.
    let deleted_resources = sqlx::query(
        "DELETE FROM check_results WHERE report_id IN
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

    // An archive holds monitoring history only: never accounts, password
    // hashes, sessions or the audit log (the copy starts as the whole
    // database). The VACUUM that follows also clears them from free pages.
    for table in [
        "sessions",
        "user_companies",
        "users",
        "audit_log",
        "enrollment_tokens",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
    }

    // The copied rollups include newer results: recompute them from the
    // reports the archive keeps (D-111). The main database keeps its rollups.
    super::history::rebuild(&mut tx).await?;

    tx.commit().await?;
    tracing::debug!(
        "Archive: Deleted {} reports and {} check results from archive",
        deleted_reports,
        deleted_resources
    );
    Ok(())
}

async fn delete_old_data(pool: &SqlitePool, cutoff: &DateTime<Utc>) -> Result<()> {
    let mut tx = super::write_transaction(pool).await?;

    // Delete check_results for reports OLDER than cutoff
    let deleted_resources = sqlx::query(
        "DELETE FROM check_results WHERE report_id IN
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ArchiveStore;
    use crate::storage::sqlite::SqliteStorage;
    use chrono::Duration;

    /// A hub database and an archive made from it, in temporary folders.
    async fn archived() -> (tempfile::TempDir, SqliteStorage, String, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let main = SqliteStorage::open(&dir.path().join("hub.db"))
            .await
            .unwrap();
        let archives = dir.path().join("archives");
        let archives = archives.to_str().unwrap().to_string();
        let now = Utc::now();
        let created = main
            .create_archive(&archives, now - Duration::days(90), now)
            .await
            .unwrap();
        let path = Path::new(&archives).join(&created.filename);
        (dir, main, archives, path)
    }

    /// `deny.toml` ignores RUSTSEC-2026-0459 only while the bundled SQLite
    /// is older than the fix (3.54, libsqlite3-sys 0.39), like a
    /// `known_issue_*` test (D-118). Once sqlx brings the fix, this fails
    /// until the ignore goes: it can't outlive its reason.
    #[tokio::test]
    async fn the_sqlite_advisory_is_ignored_only_while_sqlite_needs_it() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let (version,): (String,) = sqlx::query_as("SELECT sqlite_version()")
            .fetch_one(&pool)
            .await
            .unwrap();
        let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
        let fixed = (parts[0], parts[1]) >= (3, 54);

        let deny = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deny.toml");
        let deny = std::fs::read_to_string(deny).unwrap();
        let ignored = deny.contains("RUSTSEC-2026-0459");

        assert!(
            !(fixed && ignored),
            "SQLite {version} has the fix for RUSTSEC-2026-0459: remove its ignore \
             from deny.toml and close D-118's roadmap row"
        );
        assert!(
            fixed || ignored,
            "SQLite {version} is affected by RUSTSEC-2026-0459 but deny.toml doesn't \
             say why it is accepted (D-118)"
        );
    }

    /// The race D-118 closes: whoever can write into the archive folder can
    /// put a WAL-mode file and its `-wal` there after the hub's check. Opened
    /// immutable, SQLite reads the database file alone, whatever its header
    /// says; read the usual way, it would read the `-wal` too.
    #[tokio::test]
    async fn an_archive_is_read_without_any_write_ahead_log_even_in_wal_mode() {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
        let dir = tempfile::tempdir().unwrap();
        let writer = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(dir.path().join("w.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
        let mut c = writer.acquire().await.unwrap();
        for sql in [
            "CREATE TABLE t (x)",
            "INSERT INTO t VALUES (1)",
            "PRAGMA wal_checkpoint(TRUNCATE)",
            "PRAGMA wal_autocheckpoint = 0",
            "INSERT INTO t VALUES (2)", // in the -wal only
        ] {
            sqlx::query(sql).execute(&mut *c).await.unwrap();
        }
        // The planted pair: a WAL-mode database and a -wal with more in it.
        let archive = dir.path().join("archive.db");
        std::fs::copy(dir.path().join("w.db"), &archive).unwrap();
        std::fs::copy(
            dir.path().join("w.db-wal"),
            dir.path().join("archive.db-wal"),
        )
        .unwrap();
        std::fs::copy(
            dir.path().join("w.db-shm"),
            dir.path().join("archive.db-shm"),
        )
        .unwrap();
        assert_eq!(
            &std::fs::read(&archive).unwrap()[18..20],
            &[2, 2],
            "WAL mode"
        );

        let rows = |storage: SqliteStorage| async move {
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM t")
                .fetch_one(storage.pool())
                .await
                .unwrap();
            n
        };
        let usual = SqliteStorage::open_read_only(&archive).await.unwrap();
        assert_eq!(rows(usual).await, 2, "read the usual way, the -wal is read");
        let immutable = SqliteStorage::open_archive_file(&archive).await.unwrap();
        assert_eq!(rows(immutable).await, 1, "as an archive, it never is");
        drop(c);
    }

    #[tokio::test]
    async fn an_archive_is_written_without_a_write_ahead_log() {
        let (_dir, _main, _archives, path) = archived().await;
        // The file header's read and write versions: 1 is a rollback
        // journal, 2 is WAL. SQLite reads a `-wal` only for 2.
        let header = std::fs::read(&path).unwrap();
        assert_eq!(&header[18..20], &[1, 1], "rollback journal, not WAL");
        assert!(!PathBuf::from(format!("{}-wal", path.display())).exists());
    }

    #[tokio::test]
    async fn an_archive_with_a_write_ahead_log_beside_it_is_not_opened() {
        let (_dir, main, archives, path) = archived().await;
        let id = main.archives().await.unwrap()[0].id;
        assert!(main.open_archive(&archives, id).await.unwrap().is_some());

        std::fs::write(format!("{}-wal", path.display()), b"planted").unwrap();
        let refused = main
            .open_archive(&archives, id)
            .await
            .err()
            .expect("refused");
        assert!(
            refused.to_string().contains("write-ahead log beside it"),
            "{refused:#}"
        );
    }
}
