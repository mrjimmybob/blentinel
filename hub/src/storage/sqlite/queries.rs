//! SQLite queries behind [`super::SqliteStorage`]. Moved from `db.rs` in M0-09/M0-14
//! with the query bodies unchanged; only `super` (the backend) calls these.

use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, CompanyScope, DashboardCompany, Scope,
    Severity, UptimeBucket,
};
use crate::storage::{AlertState, SaveOutcome, UptimeRange};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{ConnectOptions, Connection, Row, SqliteConnection, SqlitePool};
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Opening the database and applying the schema
// ---------------------------------------------------------------------------

/// Versioned schema migrations, embedded at compile time (ENGINEERING.md §2.6).
/// One folder per storage backend (proposal 0011); SQLite is the default.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

/// The newest migration version compiled into this hub.
pub(crate) fn latest_schema_version() -> i64 {
    MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

/// A consistent copy of the database in a new file (works while others write).
pub(super) async fn vacuum_into(pool: &SqlitePool, path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::bail!("{} already exists", path.display());
    }
    sqlx::query("VACUUM INTO ?")
        .bind(path.to_string_lossy().into_owned())
        .execute(pool)
        .await?;
    Ok(())
}

/// The newest applied migration (0 for a database without any).
pub(super) async fn schema_version(pool: &SqlitePool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success")
            .fetch_one(pool)
            .await?,
    )
}

/// SQLite's `PRAGMA quick_check`: `Ok` if it says "ok".
pub(super) async fn quick_check(pool: &SqlitePool) -> anyhow::Result<Result<(), String>> {
    let rows: Vec<String> = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_all(pool)
        .await?;
    Ok(if rows == ["ok"] {
        Ok(())
    } else {
        Err(rows.join("; "))
    })
}

/// Open (creating if needed) the hub database and bring its schema up to date.
pub(super) async fn open(db_path: &Path) -> anyhow::Result<SqlitePool> {
    // Migrated over a connection of its own, closed whatever happens: a pool
    // gives back a connection from a failed migration later, in the
    // background, so the file would stay open, and Windows can't move an open
    // file when a failed upgrade puts the database back (D-60).
    let mut conn = options(db_path).connect().await?;
    let migrated = migrate(&mut conn).await;
    let closed = conn.close().await;
    migrated?;
    closed?;
    connect(db_path).await
}

/// Connect with the settings every connection in the pool needs. SQLite applies
/// these per connection, so they belong here and not in a migration.
pub(super) async fn connect(db_path: &Path) -> anyhow::Result<SqlitePool> {
    Ok(SqlitePool::connect_with(options(db_path)).await?)
}

fn options(db_path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        // Readers never block writers: the web UI reads while probes write.
        .journal_mode(SqliteJournalMode::Wal)
        // Wait for a lock instead of failing immediately.
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
}

/// Apply pending migrations. Refuses databases created before migrations
/// existed: in pre-alpha they are deleted, not adopted (DECISIONS.md D-30).
pub(super) async fn migrate(conn: &mut SqliteConnection) -> anyhow::Result<()> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name IN ('reports', '_sqlx_migrations')",
    )
    .fetch_all(&mut *conn)
    .await?;
    let has = |name: &str| tables.iter().any(|t| t == name);
    if has("reports") && !has("_sqlx_migrations") {
        anyhow::bail!(
            "this database was created before schema migrations existed and cannot be upgraded.              Blentinel is pre-alpha: stop the hub, delete the database file (and its -wal/-shm files),              and start again. Probes keep their identities and only need to be allowed in blentinel_hub.toml              as before (DECISIONS.md D-30)"
        );
    }
    MIGRATOR.run_direct(conn).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Report persistence
// ---------------------------------------------------------------------------

/// Store a report, but only if it is strictly newer than every report already
/// stored for that probe (replay protection, D-25).
///
/// The "is it newer?" check and the insert are one SQL statement, so two
/// copies of the same report arriving at the same moment cannot both be stored.
pub(super) async fn save_report(
    pool: &SqlitePool,
    report: &common::models::StatusReport,
    evaluations: &[crate::model::Evaluation],
) -> anyhow::Result<SaveOutcome> {
    anyhow::ensure!(
        evaluations.len() == report.results.len(),
        "one evaluation per result"
    );
    let mut tx = super::write_transaction(pool).await?;

    let row = sqlx::query(
        "INSERT INTO reports (probe_id, company_id, hostname, site, timestamp, interval_seconds)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6
         WHERE NOT EXISTS (SELECT 1 FROM reports WHERE probe_id = ?1 AND timestamp >= ?5)
         RETURNING id",
    )
    .bind(&report.probe_id)
    .bind(&report.company_id)
    .bind(&report.hostname)
    .bind(&report.site)
    .bind(report.timestamp)
    .bind(report.interval_seconds)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Ok(SaveOutcome::NotNewer); // tx rolls back on drop; nothing was written
    };
    let report_id: i64 = row.get("id");
    super::checks::insert_results(&mut tx, report_id, &report.results, evaluations).await?;

    tx.commit().await?;
    Ok(SaveOutcome::Stored)
}

// ---------------------------------------------------------------------------
// Probe heartbeat / expiry
// ---------------------------------------------------------------------------

/// Record (or refresh) a probe's heartbeat.  Called after every successful
/// `save_report`.  New probes are inserted; existing ones have their
/// `last_seen_at` and `status` updated — this automatically resurrects a
/// probe that was previously marked expired.
pub(super) async fn upsert_heartbeat(
    pool: &SqlitePool,
    probe_id: &str,
    company_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO probe_heartbeats (probe_id, company_id, status, last_seen_at, first_seen_at)
         VALUES (?1, ?2, 'active', ?3, ?3)
         ON CONFLICT(probe_id) DO UPDATE SET
             status      = 'active',
             last_seen_at = excluded.last_seen_at",
    )
    .bind(probe_id)
    .bind(company_id)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Scan for probes that have not reported within `timeout_secs` and flip them
/// to `expired`.  Returns the list of newly-expired `(probe_id, company_id)`
/// pairs so the caller can log or act on them.
pub(super) async fn check_expired_probes(
    pool: &SqlitePool,
    timeout_secs: u64,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<(String, String)>> {
    let cutoff = now - chrono::Duration::seconds(timeout_secs as i64);

    let mut tx = super::write_transaction(pool).await?;

    // Snapshot which probes are about to expire (need the list for logging).
    let expired: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT probe_id, company_id FROM probe_heartbeats
         WHERE status = 'active' AND last_seen_at < ?1",
    )
    .bind(cutoff)
    .fetch_all(&mut *tx)
    .await?;

    if !expired.is_empty() {
        sqlx::query(
            "UPDATE probe_heartbeats SET status = 'expired'
             WHERE status = 'active' AND last_seen_at < ?1",
        )
        .bind(cutoff)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(expired)
}

// ---------------------------------------------------------------------------
// Dashboard queries
// ---------------------------------------------------------------------------

pub(super) async fn get_dashboard_companies(
    pool: &SqlitePool,
    scope: &CompanyScope,
) -> anyhow::Result<Vec<DashboardCompany>> {
    let (in_scope, binds) = super::scope::condition("hb.company_id", scope);
    let sql = format!(
        "WITH latest_reports AS (
            SELECT probe_id, MAX(id) AS report_id FROM reports GROUP BY probe_id
        )
        SELECT
            hb.company_id,
            COALESCE(c.name, hb.company_id)                                      AS name,
            COUNT(DISTINCT hb.probe_id)                                          AS total_probes,
            COUNT(DISTINCT CASE WHEN hb.status='active'  THEN hb.probe_id END)  AS active_probes,
            COUNT(DISTINCT CASE WHEN hb.status='expired' THEN hb.probe_id END)  AS expired_probes,
            COUNT(CASE WHEN rs.status='Up'   THEN 1 END)                        AS devices_up,
            COUNT(CASE WHEN rs.status='Down' THEN 1 END)                        AS devices_down,
            COUNT(CASE WHEN rs.status='Warning' THEN 1 END)                     AS devices_warning,
            COUNT(CASE WHEN rs.status='Down' AND hb.status='active' THEN 1 END) AS active_devices_down,
            COUNT(CASE WHEN rs.status='Warning' AND hb.status='active' THEN 1 END)
                                                                                 AS active_devices_warning,
            MAX(r.timestamp)                                                     AS last_report
        FROM probe_heartbeats hb
        LEFT JOIN companies c          ON c.id = hb.company_id
        LEFT JOIN latest_reports lr    ON lr.probe_id = hb.probe_id
        LEFT JOIN reports r            ON r.id = lr.report_id
        LEFT JOIN check_results rs ON rs.report_id = lr.report_id
        WHERE c.archived_at IS NULL AND {in_scope}
        GROUP BY hb.company_id
        ORDER BY hb.company_id ASC"
    );
    let mut query = sqlx::query(&sql);
    for id in &binds {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let last_report: Option<DateTime<Utc>> = row.get("last_report");
        let expired_probes: i64 = row.get("expired_probes");
        results.push(DashboardCompany {
            company_id: row.get("company_id"),
            name: row.get("name"),
            total_probes: row.get("total_probes"),
            active_probes: row.get("active_probes"),
            expired_probes,
            devices_up: row.get("devices_up"),
            devices_down: row.get("devices_down"),
            devices_warning: row.get("devices_warning"),
            severity: Severity::for_company(
                row.get("active_devices_down"),
                row.get("active_devices_warning"),
                expired_probes,
            ),
            last_report: last_report.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
        });
    }
    Ok(results)
}

pub(super) async fn probe_company(
    pool: &SqlitePool,
    probe_id: &str,
) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(
            (SELECT company_id FROM probe_heartbeats WHERE probe_id = ?1),
            (SELECT company_id FROM reports WHERE probe_id = ?1 ORDER BY id DESC LIMIT 1))",
    )
    .bind(probe_id)
    .fetch_one(pool)
    .await?)
}

// ---------------------------------------------------------------------------
// Company detail queries
// ---------------------------------------------------------------------------

pub(super) async fn get_company_probes(
    pool: &SqlitePool,
    company_id: &str,
) -> anyhow::Result<Vec<CompanyProbe>> {
    let rows = sqlx::query(
        "WITH latest_reports AS (
            SELECT probe_id, MAX(id) AS report_id FROM reports GROUP BY probe_id
        )
        SELECT
            hb.probe_id, hb.status, hb.last_seen_at, COALESCE(p.name, '') AS probe_name,
            r.hostname, COALESCE(s.name, r.site) AS site,
            COUNT(CASE WHEN rs.status='Up'   THEN 1 END) AS devices_up,
            COUNT(CASE WHEN rs.status='Down' THEN 1 END) AS devices_down,
            COUNT(CASE WHEN rs.status='Warning' THEN 1 END) AS devices_warning
        FROM probe_heartbeats hb
        LEFT JOIN latest_reports lr    ON lr.probe_id = hb.probe_id
        LEFT JOIN reports r            ON r.id = lr.report_id
        LEFT JOIN sites s              ON s.company_id = r.company_id AND s.id = r.site
        LEFT JOIN probes p             ON p.id = hb.probe_id
        LEFT JOIN check_results rs ON rs.report_id = lr.report_id
        WHERE hb.company_id = ?1
        GROUP BY hb.probe_id
        ORDER BY hb.last_seen_at DESC",
    )
    .bind(company_id)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let last_seen: Option<DateTime<Utc>> = row.get("last_seen_at");
        let status: String = row.get("status");
        let devices_down: i64 = row.get("devices_down");
        let devices_warning: i64 = row.get("devices_warning");
        results.push(CompanyProbe {
            probe_id: row.get("probe_id"),
            probe_name: row.get("probe_name"),
            hostname: row.get("hostname"),
            site: row.get("site"),
            severity: Severity::for_probe(status == "active", devices_down, devices_warning),
            status,
            last_seen_at: last_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
            devices_up: row.get("devices_up"),
            devices_down,
            devices_warning,
        });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Uptime history — adaptive time bucketing
// ---------------------------------------------------------------------------
//
// Supported ranges and their bucket strategies:
//
//   Range   | Time window             | Bucket size  | ~Buckets
//   --------|-------------------------|--------------|----------
//   "24h"   | now() - 24 hours        | 15 minutes   | ~96
//   "7d"    | now() - 7 days          | 1 hour       | ~168
//   "30d"   | now() - 30 days         | 4 hours      | ~180
//   "all"   | earliest retained record| 1 day        | <=retention
//
// Why different bucket sizes?
// Smaller windows need fine-grained resolution for real-time monitoring.
// Larger windows aggregate more aggressively to keep the data point count
// bounded (~100–180 points), which avoids both performance issues and an
// overcrowded chart. The SVG frontend renders any number of buckets as-is,
// so no frontend changes are required.

pub(super) async fn get_company_uptime_history(
    pool: &SqlitePool,
    company_id: &str,
    range: UptimeRange,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<UptimeBucket>> {
    // -----------------------------------------------------------------------
    // Range mapping: translate the range into a cutoff time and a SQL
    // bucket expression. (Parsing, including the 24h fallback, is UptimeRange::parse.)
    // -----------------------------------------------------------------------
    //
    // Bucket expressions use SQLite strftime + integer truncation:
    //   - 15 min: truncate minutes to nearest 15  →  "%Y-%m-%d %H:" || printf(…)
    //   - 1 hour: zero out minutes                →  strftime("%Y-%m-%d %H:00", …)
    //   - 4 hour: truncate hour to nearest 4      →  "%Y-%m-%d " || printf(…) || ":00"
    //   - 1 day:  date only + " 00:00"            →  strftime("%Y-%m-%d", …) || " 00:00"
    //
    // All bucket expressions produce the canonical "YYYY-MM-DD HH:MM" format
    // so the response JSON structure stays identical across all ranges.

    let (cutoff_sql, bucket_expr) = match range {
        UptimeRange::Days7 => {
            // 7-day window, 1-hour buckets (~168 points)
            let cutoff = now - chrono::Duration::days(7);
            (
                Some(cutoff),
                "strftime('%Y-%m-%d %H:00', r.timestamp)".to_string(),
            )
        }
        UptimeRange::Days30 => {
            // 30-day window, 4-hour buckets (~180 points)
            let cutoff = now - chrono::Duration::days(30);
            (
                Some(cutoff),
                concat!(
                    "strftime('%Y-%m-%d ', r.timestamp) || ",
                    "printf('%02d', (CAST(strftime('%H', r.timestamp) AS INTEGER) / 4) * 4) || ",
                    "':00'"
                )
                .to_string(),
            )
        }
        UptimeRange::All => {
            // No cutoff — all retained data, 1-day buckets
            (
                None,
                "strftime('%Y-%m-%d', r.timestamp) || ' 00:00'".to_string(),
            )
        }
        // 24-hour window, 15-min buckets (~96 points)
        UptimeRange::Hours24 => {
            let cutoff = now - chrono::Duration::seconds(86400);
            (
                Some(cutoff),
                concat!(
                    "strftime('%Y-%m-%d %H:', r.timestamp) || ",
                    "printf('%02d', (CAST(strftime('%M', r.timestamp) AS INTEGER) / 15) * 15)"
                )
                .to_string(),
            )
        }
    };

    // Build the query dynamically based on whether we have a cutoff or not.
    // The "all" range omits the timestamp filter entirely so SQLite can use
    // whatever index coverage it has from the earliest record onward.
    let rows = if let Some(cutoff) = cutoff_sql {
        let sql = format!(
            "SELECT {expr} AS bucket,
                    COUNT(CASE WHEN rs.maintenance = 0 AND rs.status IN ('Up', 'Warning') THEN 1 END)
                                                                                   AS up_count,
                    COUNT(CASE WHEN rs.maintenance = 0 AND rs.status='Down' THEN 1 END) AS down_count,
                    COUNT(CASE WHEN rs.maintenance = 1 THEN 1 END) AS maintenance_count
             FROM reports r
             JOIN check_results rs ON rs.report_id = r.id
             WHERE r.company_id = ?1 AND r.timestamp >= ?2
             GROUP BY bucket
             ORDER BY bucket ASC",
            expr = bucket_expr,
        );
        sqlx::query(&sql)
            .bind(company_id)
            .bind(cutoff)
            .fetch_all(pool)
            .await?
    } else {
        let sql = format!(
            "SELECT {expr} AS bucket,
                    COUNT(CASE WHEN rs.maintenance = 0 AND rs.status IN ('Up', 'Warning') THEN 1 END)
                                                                                   AS up_count,
                    COUNT(CASE WHEN rs.maintenance = 0 AND rs.status='Down' THEN 1 END) AS down_count,
                    COUNT(CASE WHEN rs.maintenance = 1 THEN 1 END) AS maintenance_count
             FROM reports r
             JOIN check_results rs ON rs.report_id = r.id
             WHERE r.company_id = ?1
             GROUP BY bucket
             ORDER BY bucket ASC",
            expr = bucket_expr,
        );
        sqlx::query(&sql).bind(company_id).fetch_all(pool).await?
    };

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let raw_bucket: String = row.get("bucket");
        // Re-parse and re-format to guarantee zero-padded minutes
        let bucket = match chrono::NaiveDateTime::parse_from_str(&raw_bucket, "%Y-%m-%d %H:%M") {
            Ok(ndt) => ndt.format("%Y-%m-%d %H:%M").to_string(),
            Err(_) => raw_bucket, // fallback: use as-is
        };
        results.push(UptimeBucket {
            bucket,
            up_count: row.get("up_count"),
            maintenance_count: row.get("maintenance_count"),
            down_count: row.get("down_count"),
        });
    }
    Ok(results)
}

pub(super) async fn latest_report_at(pool: &SqlitePool) -> anyhow::Result<Option<DateTime<Utc>>> {
    let row = sqlx::query("SELECT MAX(timestamp) AS latest FROM reports")
        .fetch_one(pool)
        .await?;
    Ok(row.get("latest"))
}

// ---------------------------------------------------------------------------
// Admin queries
// ---------------------------------------------------------------------------

pub(super) async fn get_all_companies(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT company_id FROM probe_heartbeats ORDER BY company_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub(super) async fn get_all_probes(pool: &SqlitePool) -> anyhow::Result<Vec<AdminProbe>> {
    let rows = sqlx::query(
        "SELECT probe_id, company_id, status, last_seen_at, first_seen_at
         FROM probe_heartbeats ORDER BY company_id, probe_id",
    )
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let last_seen: Option<DateTime<Utc>> = row.get("last_seen_at");
        let first_seen: Option<DateTime<Utc>> = row.get("first_seen_at");
        results.push(AdminProbe {
            probe_id: row.get("probe_id"),
            company_id: row.get("company_id"),
            status: row.get("status"),
            last_seen_at: last_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
            first_seen_at: first_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
        });
    }
    Ok(results)
}

/// Delete all reports and their results for a company (keeps heartbeats).
pub(super) async fn delete_company_data(pool: &SqlitePool, company_id: &str) -> anyhow::Result<()> {
    let mut tx = super::write_transaction(pool).await?;

    sqlx::query(
        "DELETE FROM check_results WHERE report_id IN (SELECT id FROM reports WHERE company_id = ?1)"
    )
        .bind(company_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM reports WHERE company_id = ?1")
        .bind(company_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM incidents WHERE company_id = ?1")
        .bind(company_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Delete all reports and their results for a single probe (keeps heartbeat).
pub(super) async fn delete_probe_data(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<()> {
    let mut tx = super::write_transaction(pool).await?;

    sqlx::query(
        "DELETE FROM check_results WHERE report_id IN (SELECT id FROM reports WHERE probe_id = ?1)",
    )
    .bind(probe_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM reports WHERE probe_id = ?1")
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM incidents WHERE probe_id = ?1")
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Remove a probe entirely: delete its data AND its heartbeat row.
pub(super) async fn remove_probe(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<()> {
    let mut tx = super::write_transaction(pool).await?;

    sqlx::query(
        "DELETE FROM check_results WHERE report_id IN (SELECT id FROM reports WHERE probe_id = ?1)",
    )
    .bind(probe_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM reports WHERE probe_id = ?1")
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM incidents WHERE probe_id = ?1")
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM probe_heartbeats WHERE probe_id = ?1")
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Archive metadata queries
// ---------------------------------------------------------------------------

pub(super) async fn get_archives(pool: &SqlitePool) -> anyhow::Result<Vec<ArchiveMetadata>> {
    let rows = sqlx::query(
        "SELECT id, filename, created_at, cutoff_date, size_mb
         FROM archives ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let created: DateTime<Utc> = row.get("created_at");
        let cutoff: DateTime<Utc> = row.get("cutoff_date");
        results.push(ArchiveMetadata {
            id: row.get("id"),
            filename: row.get("filename"),
            created_at: created.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            cutoff_date: cutoff.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            size_mb: row.get("size_mb"),
        });
    }
    Ok(results)
}

pub(super) async fn insert_archive_record(
    pool: &SqlitePool,
    filename: &str,
    cutoff_date: DateTime<Utc>,
    size_mb: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO archives (filename, created_at, cutoff_date, size_mb)
         VALUES (?, ?, ?, ?)",
    )
    .bind(filename)
    .bind(now)
    .bind(cutoff_date)
    .bind(size_mb)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn get_db_size_mb(db_path: &str) -> anyhow::Result<u64> {
    let metadata = std::fs::metadata(db_path)?;
    Ok(metadata.len() / (1024 * 1024))
}

// ---------------------------------------------------------------------------
// Alert state management
// ---------------------------------------------------------------------------

pub(super) async fn get_alert_state(
    pool: &SqlitePool,
    key: &str,
) -> anyhow::Result<Option<AlertState>> {
    let row = sqlx::query(
        "SELECT resource_key, last_status, last_alert_sent_at, streak FROM alert_states
         WHERE resource_key = ?",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        Ok(AlertState {
            key: r.get("resource_key"),
            last_status: r.get("last_status"),
            last_alert_sent_at: r.get("last_alert_sent_at"),
            streak: u32::try_from(r.get::<i64, _>("streak"))?,
        })
    })
    .transpose()
}

pub(super) async fn upsert_alert_state(
    pool: &SqlitePool,
    key: &str,
    status: &str,
    streak: u32,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let alert_time = if alert_sent { Some(now) } else { None };

    sqlx::query(
        "INSERT INTO alert_states (resource_key, last_status, last_alert_sent_at, streak)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(resource_key) DO UPDATE SET
             last_status = excluded.last_status,
             last_alert_sent_at = COALESCE(excluded.last_alert_sent_at, last_alert_sent_at),
             streak = excluded.streak",
    )
    .bind(key)
    .bind(status)
    .bind(alert_time)
    .bind(i64::from(streak))
    .execute(pool)
    .await?;

    Ok(())
}

/// Change `last_status` of an existing alert state. Does nothing if the key has
/// no state yet or already has that status, so calling it on every report is cheap.
pub(super) async fn set_alert_status_if_present(
    pool: &SqlitePool,
    key: &str,
    status: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE alert_states SET last_status = ?1
         WHERE resource_key = ?2 AND last_status <> ?1",
    )
    .bind(status)
    .bind(key)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Alert silence management
// ---------------------------------------------------------------------------

pub(super) async fn create_silence(
    pool: &SqlitePool,
    company_id: &str,
    scope: &Scope,
    reason: &str,
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO alert_silences (company_id, scope_type, scope_id, reason, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(company_id, scope_type, scope_id) DO UPDATE SET
             reason = excluded.reason,
             created_at = excluded.created_at,
             expires_at = excluded.expires_at
         RETURNING id",
    )
    .bind(company_id)
    .bind(scope.kind())
    .bind(scope.id().unwrap_or(""))
    .bind(reason)
    .bind(now)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;

    Ok(result.get("id"))
}

pub(super) async fn get_active_silences(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<AlertSilence>> {
    let rows = sqlx::query(
        "SELECT id, company_id, scope_type, scope_id, reason, created_at, expires_at
         FROM alert_silences
         WHERE expires_at IS NULL OR expires_at > ?1
         ORDER BY id",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let created: DateTime<Utc> = row.get("created_at");
        let expires: Option<DateTime<Utc>> = row.get("expires_at");
        let kind: String = row.get("scope_type");
        let id: String = row.get("scope_id");
        let scope = Scope::from_parts(&kind, (!id.is_empty()).then_some(id))
            .ok_or_else(|| anyhow::anyhow!("unknown silence scope {kind}"))?;
        results.push(AlertSilence {
            id: row.get("id"),
            company_id: row.get("company_id"),
            scope,
            reason: row.get("reason"),
            created_at: created.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            expires_at: expires.map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        });
    }
    Ok(results)
}

pub(super) async fn is_silenced(
    pool: &SqlitePool,
    company_id: &str,
    site_id: &str,
    probe_id: &str,
    check_id: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM alert_silences
         WHERE company_id = ?1
           AND (expires_at IS NULL OR expires_at > ?5)
           AND (scope_type = 'company'
                OR (scope_type = 'site' AND scope_id = ?2)
                OR (scope_type = 'probe' AND scope_id = ?3)
                OR (scope_type = 'check' AND scope_id = ?4))",
    )
    .bind(company_id)
    .bind(site_id)
    .bind(probe_id)
    .bind(check_id)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(count > 0)
}

pub(super) async fn delete_silence(pool: &SqlitePool, silence_id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM alert_silences WHERE id = ?")
        .bind(silence_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn clear_check_silence(pool: &SqlitePool, check_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM alert_silences WHERE scope_type = 'check' AND scope_id = ?")
        .bind(check_id)
        .execute(pool)
        .await?;
    Ok(())
}
