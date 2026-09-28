//! SQLite queries behind [`super::SqliteStorage`]. Moved from `db.rs` in M0-09/M0-14
//! with the query bodies unchanged; only `super` (the backend) calls these.

use super::types::DbResourceType;
use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice,
    Severity, UptimeBucket,
};
use crate::storage::{AlertState, SaveOutcome, UptimeRange};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Opening the database and applying the schema
// ---------------------------------------------------------------------------

/// Versioned schema migrations, embedded at compile time (ENGINEERING.md §2.6).
/// One folder per storage backend (proposal 0011); SQLite is the default.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

/// Open (creating if needed) the hub database and bring its schema up to date.
pub(super) async fn open(db_path: &Path) -> anyhow::Result<SqlitePool> {
    let pool = connect(db_path).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Connect with the settings every connection in the pool needs. SQLite applies
/// these per connection, so they belong here and not in a migration.
pub(super) async fn connect(db_path: &Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        // Readers never block writers: the web UI reads while probes write.
        .journal_mode(SqliteJournalMode::Wal)
        // Wait for a lock instead of failing immediately.
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    Ok(SqlitePool::connect_with(options).await?)
}

/// Apply pending migrations. Refuses databases created before migrations
/// existed: in pre-alpha they are deleted, not adopted (DECISIONS.md D-30).
pub(super) async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let table_exists = |name: &'static str| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .map(|n| n > 0)
    };
    if table_exists("reports").await? && !table_exists("_sqlx_migrations").await? {
        anyhow::bail!(
            "this database was created before schema migrations existed and cannot be upgraded. \
             Blentinel is pre-alpha: stop the hub, delete the database file (and its -wal/-shm files), \
             and start again. Probes keep their identities and only need to be allowed in blentinel_hub.toml \
             as before (DECISIONS.md D-30)"
        );
    }
    MIGRATOR.run(pool).await?;
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
) -> anyhow::Result<SaveOutcome> {
    let mut tx = pool.begin().await?;

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

    for res in &report.resources {
        sqlx::query(
            "INSERT INTO resource_statuses
             (report_id, name, resource_type, target, status, message, latency_ms, metric_value, metric_unit)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
            .bind(report_id)
            .bind(&res.name)
            .bind(DbResourceType(res.resource_type))
            .bind(&res.target)
            .bind(format!("{:?}", res.status))
            .bind(&res.message)
            .bind(res.latency_ms.map(|l| l as i64))
            .bind(res.metric_value)
            .bind(&res.metric_unit)
            .execute(tx.as_mut())
            .await?;
    }

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

    let mut tx = pool.begin().await?;

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
) -> anyhow::Result<Vec<DashboardCompany>> {
    let rows = sqlx::query(
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
            COUNT(CASE WHEN rs.status='Down' AND hb.status='active' THEN 1 END) AS active_devices_down,
            MAX(r.timestamp)                                                     AS last_report
        FROM probe_heartbeats hb
        LEFT JOIN companies c          ON c.id = hb.company_id
        LEFT JOIN latest_reports lr    ON lr.probe_id = hb.probe_id
        LEFT JOIN reports r            ON r.id = lr.report_id
        LEFT JOIN resource_statuses rs ON rs.report_id = lr.report_id
        WHERE c.archived_at IS NULL
        GROUP BY hb.company_id
        ORDER BY hb.company_id ASC"
    )
        .fetch_all(pool)
        .await?;

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
            severity: Severity::for_company(row.get("active_devices_down"), expired_probes),
            last_report: last_report.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
        });
    }
    Ok(results)
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
            hb.probe_id, hb.status, hb.last_seen_at,
            r.hostname, COALESCE(s.name, r.site) AS site,
            COUNT(CASE WHEN rs.status='Up'   THEN 1 END) AS devices_up,
            COUNT(CASE WHEN rs.status='Down' THEN 1 END) AS devices_down
        FROM probe_heartbeats hb
        LEFT JOIN latest_reports lr    ON lr.probe_id = hb.probe_id
        LEFT JOIN reports r            ON r.id = lr.report_id
        LEFT JOIN sites s              ON s.company_id = r.company_id AND s.id = r.site
        LEFT JOIN resource_statuses rs ON rs.report_id = lr.report_id
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
        results.push(CompanyProbe {
            probe_id: row.get("probe_id"),
            probe_name: String::new(), // filled by the API handler from the whitelist
            hostname: row.get("hostname"),
            site: row.get("site"),
            severity: Severity::for_probe(status == "active", devices_down),
            status,
            last_seen_at: last_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
            devices_up: row.get("devices_up"),
            devices_down,
        });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Probe device query
// ---------------------------------------------------------------------------

pub(super) async fn get_probe_devices(
    pool: &SqlitePool,
    probe_id: &str,
) -> anyhow::Result<Vec<ProbeDevice>> {
    let rows = sqlx::query(
        "SELECT rs.name, rs.resource_type, rs.target, rs.status, rs.message, rs.latency_ms,
                rs.metric_value, rs.metric_unit
         FROM resource_statuses rs
         JOIN reports r ON r.id = rs.report_id
         WHERE r.probe_id = ?1
           AND r.id = (SELECT MAX(id) FROM reports WHERE probe_id = ?1)
         ORDER BY rs.name ASC",
    )
    .bind(probe_id)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        results.push(ProbeDevice {
            name: row.get("name"),
            resource_type: row.get("resource_type"),
            target: row.get("target"),
            status: row.get("status"),
            message: row.get("message"),
            latency_ms: row.get("latency_ms"),
            metric_value: row.get("metric_value"),
            metric_unit: row.get("metric_unit"),
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
                    COUNT(CASE WHEN rs.status='Up'   THEN 1 END) AS up_count,
                    COUNT(CASE WHEN rs.status='Down' THEN 1 END) AS down_count
             FROM reports r
             JOIN resource_statuses rs ON rs.report_id = r.id
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
                    COUNT(CASE WHEN rs.status='Up'   THEN 1 END) AS up_count,
                    COUNT(CASE WHEN rs.status='Down' THEN 1 END) AS down_count
             FROM reports r
             JOIN resource_statuses rs ON rs.report_id = r.id
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

/// Delete all reports and resource_statuses for a company (keeps heartbeats).
pub(super) async fn delete_company_data(pool: &SqlitePool, company_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "DELETE FROM resource_statuses WHERE report_id IN (SELECT id FROM reports WHERE company_id = ?1)"
    )
        .bind(company_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM reports WHERE company_id = ?1")
        .bind(company_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Delete all reports and resource_statuses for a single probe (keeps heartbeat).
pub(super) async fn delete_probe_data(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "DELETE FROM resource_statuses WHERE report_id IN (SELECT id FROM reports WHERE probe_id = ?1)"
    )
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM reports WHERE probe_id = ?1")
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Remove a probe entirely: delete its data AND its heartbeat row.
pub(super) async fn remove_probe(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "DELETE FROM resource_statuses WHERE report_id IN (SELECT id FROM reports WHERE probe_id = ?1)"
    )
        .bind(probe_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM reports WHERE probe_id = ?1")
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
    resource_key: &str,
) -> anyhow::Result<Option<AlertState>> {
    let row = sqlx::query(
        "SELECT resource_key, last_status, last_alert_sent_at FROM alert_states WHERE resource_key = ?"
    )
    .bind(resource_key)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AlertState {
        resource_key: r.get("resource_key"),
        last_status: r.get("last_status"),
        last_alert_sent_at: r.get("last_alert_sent_at"),
    }))
}

pub(super) async fn upsert_alert_state(
    pool: &SqlitePool,
    resource_key: &str,
    status: &str,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let alert_time = if alert_sent { Some(now) } else { None };

    sqlx::query(
        "INSERT INTO alert_states (resource_key, last_status, last_alert_sent_at)
         VALUES (?, ?, ?)
         ON CONFLICT(resource_key) DO UPDATE SET
             last_status = excluded.last_status,
             last_alert_sent_at = COALESCE(excluded.last_alert_sent_at, last_alert_sent_at)",
    )
    .bind(resource_key)
    .bind(status)
    .bind(alert_time)
    .execute(pool)
    .await?;

    Ok(())
}

/// Change `last_status` of an existing alert state. Does nothing if the key has
/// no state yet or already has that status, so calling it on every report is cheap.
pub(super) async fn set_alert_status_if_present(
    pool: &SqlitePool,
    resource_key: &str,
    status: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE alert_states SET last_status = ?1
         WHERE resource_key = ?2 AND last_status <> ?1",
    )
    .bind(status)
    .bind(resource_key)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Alert silence management
// ---------------------------------------------------------------------------

pub(super) async fn create_silence(
    pool: &SqlitePool,
    scope_type: &str,
    scope_id: &str,
    reason: &str,
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO alert_silences (scope_type, scope_id, reason, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(scope_type, scope_id) DO UPDATE SET
             reason = excluded.reason,
             created_at = excluded.created_at,
             expires_at = excluded.expires_at
         RETURNING id",
    )
    .bind(scope_type)
    .bind(scope_id)
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
        "SELECT id, scope_type, scope_id, reason, created_at, expires_at
         FROM alert_silences
         WHERE expires_at IS NULL OR expires_at > ?",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let created: DateTime<Utc> = row.get("created_at");
        let expires: Option<DateTime<Utc>> = row.get("expires_at");
        results.push(AlertSilence {
            id: row.get("id"),
            scope_type: row.get("scope_type"),
            scope_id: row.get("scope_id"),
            reason: row.get("reason"),
            created_at: created.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            expires_at: expires.map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        });
    }
    Ok(results)
}

pub(super) async fn is_silenced(
    pool: &SqlitePool,
    scope_type: &str,
    scope_id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM alert_silences
         WHERE scope_type = ? AND scope_id = ?
         AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(scope_type)
    .bind(scope_id)
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

pub(super) async fn clear_silence_by_resource(
    pool: &SqlitePool,
    resource_key: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM alert_silences WHERE scope_type = 'resource' AND scope_id = ?")
        .bind(resource_key)
        .execute(pool)
        .await?;
    Ok(())
}
