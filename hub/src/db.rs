#![cfg(feature = "ssr")]
use chrono::{DateTime, Utc};
use serde::Serialize;
// use sqlx::{SqlitePool, Row, Encode, Decode, Type, Sqlite, encode::IsNull};
// use sqlx::sqlite::{SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef};
// use std::borrow::Cow;
pub mod types;
use crate::db::types::DbResourceType;
use sqlx::{SqlitePool, Row};


// ---------------------------------------------------------------------------
// Table setup
// ---------------------------------------------------------------------------

pub async fn setup_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    // WAL mode: readers never block writers and vice versa.
    // Critical once the web UI is reading while probes are writing.
    sqlx::query("PRAGMA journal_mode=WAL;").execute(pool).await?;
    // Instead of failing immediately on a locked DB, retry for up to 5 s.
    sqlx::query("PRAGMA busy_timeout = 5000;").execute(pool).await?;

    // ---------------------------------------------------------------------------
    // Core tables
    // ---------------------------------------------------------------------------
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS reports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            probe_id TEXT NOT NULL,
            company_id TEXT NOT NULL,
            hostname TEXT NOT NULL,
            site TEXT NOT NULL,
            timestamp DATETIME NOT NULL,
            interval_seconds INTEGER NOT NULL,
            received_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );"
    ).execute(pool).await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS resource_statuses (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            report_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            target TEXT NOT NULL,
            status TEXT NOT NULL, -- 'Up' or 'Down'
            message TEXT,
            latency_ms INTEGER,
            metric_value REAL,
            metric_unit TEXT,
            FOREIGN KEY(report_id) REFERENCES reports(id)
        );"
    ).execute(pool).await?;

    // ---------------------------------------------------------------------------
    // Probe liveness tracking
    // ---------------------------------------------------------------------------
    // One row per known probe.  Updated on every successful report; a background
    // task flips status → 'expired' when last_seen_at falls behind the timeout.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS probe_heartbeats (
            probe_id TEXT PRIMARY KEY,
            company_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            last_seen_at DATETIME NOT NULL,
            first_seen_at DATETIME NOT NULL
        );"
    ).execute(pool).await?;

    // ---------------------------------------------------------------------------
    // Indexes — columns the web UI will filter/sort on
    // ---------------------------------------------------------------------------
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_reports_probe_id     ON reports(probe_id);").execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_reports_company_id   ON reports(company_id);").execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_reports_timestamp    ON reports(timestamp);").execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_resource_statuses_report_id ON resource_statuses(report_id);").execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_heartbeats_company   ON probe_heartbeats(company_id);").execute(pool).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Report persistence
// ---------------------------------------------------------------------------

pub async fn save_report(pool: &SqlitePool, report: &common::models::StatusReport) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query(
        "INSERT INTO reports (probe_id, company_id, hostname, site, timestamp, interval_seconds)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id"
    )
        .bind(&report.probe_id)
        .bind(&report.company_id)
        .bind(&report.hostname)
        .bind(&report.site)
        .bind(report.timestamp)
        .bind(report.interval_seconds)
        .fetch_one(&mut *tx)
        .await?;

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
    Ok(())
}

// ---------------------------------------------------------------------------
// Probe heartbeat / expiry
// ---------------------------------------------------------------------------

/// Record (or refresh) a probe's heartbeat.  Called after every successful
/// `save_report`.  New probes are inserted; existing ones have their
/// `last_seen_at` and `status` updated — this automatically resurrects a
/// probe that was previously marked expired.
pub async fn upsert_heartbeat(pool: &SqlitePool, probe_id: &str, company_id: &str) -> anyhow::Result<()> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO probe_heartbeats (probe_id, company_id, status, last_seen_at, first_seen_at)
         VALUES (?1, ?2, 'active', ?3, ?3)
         ON CONFLICT(probe_id) DO UPDATE SET
             status      = 'active',
             last_seen_at = excluded.last_seen_at"
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
pub async fn check_expired_probes(pool: &SqlitePool, timeout_secs: u64) -> anyhow::Result<Vec<(String, String)>> {
    let cutoff = Utc::now() - chrono::Duration::seconds(timeout_secs as i64);

    let mut tx = pool.begin().await?;

    // Snapshot which probes are about to expire (need the list for logging).
    let expired: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT probe_id, company_id FROM probe_heartbeats
         WHERE status = 'active' AND last_seen_at < ?1"
    )
        .bind(cutoff)
        .fetch_all(&mut *tx)
        .await?;

    if !expired.is_empty() {
        sqlx::query(
            "UPDATE probe_heartbeats SET status = 'expired'
             WHERE status = 'active' AND last_seen_at < ?1"
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

#[derive(Debug, Serialize)]
pub struct DashboardCompany {
    pub company_id:     String,
    pub total_probes:   i64,
    pub active_probes:  i64,
    pub expired_probes: i64,
    pub devices_up:     i64,
    pub devices_down:   i64,
    pub last_report:    Option<String>,  // ISO 8601 string; None if no reports yet
}

pub async fn get_dashboard_companies(pool: &SqlitePool) -> anyhow::Result<Vec<DashboardCompany>> {
    let rows = sqlx::query(
        "WITH latest_reports AS (
            SELECT probe_id, MAX(id) AS report_id FROM reports GROUP BY probe_id
        )
        SELECT
            hb.company_id,
            COUNT(DISTINCT hb.probe_id)                                          AS total_probes,
            COUNT(DISTINCT CASE WHEN hb.status='active'  THEN hb.probe_id END)  AS active_probes,
            COUNT(DISTINCT CASE WHEN hb.status='expired' THEN hb.probe_id END)  AS expired_probes,
            COUNT(CASE WHEN rs.status='Up'   THEN 1 END)                        AS devices_up,
            COUNT(CASE WHEN rs.status='Down' THEN 1 END)                        AS devices_down,
            MAX(r.timestamp)                                                     AS last_report
        FROM probe_heartbeats hb
        LEFT JOIN latest_reports lr    ON lr.probe_id = hb.probe_id
        LEFT JOIN reports r            ON r.id = lr.report_id
        LEFT JOIN resource_statuses rs ON rs.report_id = lr.report_id
        GROUP BY hb.company_id
        ORDER BY hb.company_id ASC"
    )
        .fetch_all(pool)
        .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let last_report: Option<DateTime<Utc>> = row.get("last_report");
        results.push(DashboardCompany {
            company_id:     row.get("company_id"),
            total_probes:   row.get("total_probes"),
            active_probes:  row.get("active_probes"),
            expired_probes: row.get("expired_probes"),
            devices_up:     row.get("devices_up"),
            devices_down:   row.get("devices_down"),
            last_report:    last_report.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
        });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Company detail queries
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct CompanyProbe {
    pub probe_id:     String,
    pub probe_name:   String,
    pub hostname:     Option<String>,
    pub site:         Option<String>,
    pub status:       String,
    pub last_seen_at: Option<String>,
    pub devices_up:   i64,
    pub devices_down: i64,
}

pub async fn get_company_probes(pool: &SqlitePool, company_id: &str) -> anyhow::Result<Vec<CompanyProbe>> {
    let rows = sqlx::query(
        "WITH latest_reports AS (
            SELECT probe_id, MAX(id) AS report_id FROM reports GROUP BY probe_id
        )
        SELECT
            hb.probe_id, hb.status, hb.last_seen_at,
            r.hostname, r.site,
            COUNT(CASE WHEN rs.status='Up'   THEN 1 END) AS devices_up,
            COUNT(CASE WHEN rs.status='Down' THEN 1 END) AS devices_down
        FROM probe_heartbeats hb
        LEFT JOIN latest_reports lr    ON lr.probe_id = hb.probe_id
        LEFT JOIN reports r            ON r.id = lr.report_id
        LEFT JOIN resource_statuses rs ON rs.report_id = lr.report_id
        WHERE hb.company_id = ?1
        GROUP BY hb.probe_id
        ORDER BY hb.last_seen_at DESC"
    )
        .bind(company_id)
        .fetch_all(pool)
        .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let last_seen: Option<DateTime<Utc>> = row.get("last_seen_at");
        results.push(CompanyProbe {
            probe_id:     row.get("probe_id"),
            probe_name:   String::new(),  // filled by API handler from whitelist
            hostname:     row.get("hostname"),
            site:         row.get("site"),
            status:       row.get("status"),
            last_seen_at: last_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
            devices_up:   row.get("devices_up"),
            devices_down: row.get("devices_down"),
        });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Probe device query
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ProbeDevice {
    pub name:          String,
    pub resource_type: String,
    pub target:        String,
    pub status:        String,
    pub message:       Option<String>,
    pub latency_ms:    Option<i64>,
    pub metric_value:  Option<f64>,
    pub metric_unit:   Option<String>,
}

pub async fn get_probe_devices(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<Vec<ProbeDevice>> {
    let rows = sqlx::query(
        "SELECT rs.name, rs.resource_type, rs.target, rs.status, rs.message, rs.latency_ms,
                rs.metric_value, rs.metric_unit
         FROM resource_statuses rs
         JOIN reports r ON r.id = rs.report_id
         WHERE r.probe_id = ?1
           AND r.id = (SELECT MAX(id) FROM reports WHERE probe_id = ?1)
         ORDER BY rs.name ASC"
    )
        .bind(probe_id)
        .fetch_all(pool)
        .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        results.push(ProbeDevice {
            name:          row.get("name"),
            resource_type: row.get("resource_type"),
            target:        row.get("target"),
            status:        row.get("status"),
            message:       row.get("message"),
            latency_ms:    row.get("latency_ms"),
            metric_value:  row.get("metric_value"),
            metric_unit:   row.get("metric_unit"),
        });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Uptime history (15-min buckets, last 24 h)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct UptimeBucket {
    pub bucket:    String,   // "YYYY-MM-DD HH:MM"
    pub up_count:  i64,
    pub down_count: i64,
}

pub async fn get_company_uptime_history(pool: &SqlitePool, company_id: &str) -> anyhow::Result<Vec<UptimeBucket>> {
    let cutoff = Utc::now() - chrono::Duration::seconds(86400);

    let rows = sqlx::query(
        "SELECT
            strftime('%Y-%m-%d %H:', r.timestamp) ||
            printf('%02d', (CAST(strftime('%M', r.timestamp) AS INTEGER) / 15) * 15)
                AS bucket,
            COUNT(CASE WHEN rs.status='Up'   THEN 1 END) AS up_count,
            COUNT(CASE WHEN rs.status='Down' THEN 1 END) AS down_count
         FROM reports r
         JOIN resource_statuses rs ON rs.report_id = r.id
         WHERE r.company_id = ?1 AND r.timestamp >= ?2
         GROUP BY bucket
         ORDER BY bucket ASC"
    )
        .bind(company_id)
        .bind(cutoff)
        .fetch_all(pool)
        .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let raw_bucket: String = row.get("bucket");
        // Re-parse and re-format to guarantee zero-padded minutes
        let bucket = match chrono::NaiveDateTime::parse_from_str(&raw_bucket, "%Y-%m-%d %H:%M") {
            Ok(ndt) => ndt.format("%Y-%m-%d %H:%M").to_string(),
            Err(_)  => raw_bucket, // fallback: use as-is
        };
        results.push(UptimeBucket {
            bucket,
            up_count:   row.get("up_count"),
            down_count: row.get("down_count"),
        });
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Admin queries
// ---------------------------------------------------------------------------

pub async fn get_all_companies(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT company_id FROM probe_heartbeats ORDER BY company_id"
    )
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

#[derive(Debug, Serialize)]
pub struct AdminProbe {
    pub probe_id:      String,
    pub company_id:    String,
    pub status:        String,
    pub last_seen_at:  Option<String>,
    pub first_seen_at: Option<String>,
}

pub async fn get_all_probes(pool: &SqlitePool) -> anyhow::Result<Vec<AdminProbe>> {
    let rows = sqlx::query(
        "SELECT probe_id, company_id, status, last_seen_at, first_seen_at
         FROM probe_heartbeats ORDER BY company_id, probe_id"
    )
        .fetch_all(pool)
        .await?;

    let mut results = Vec::with_capacity(rows.len());
    for row in &rows {
        let last_seen:  Option<DateTime<Utc>> = row.get("last_seen_at");
        let first_seen: Option<DateTime<Utc>> = row.get("first_seen_at");
        results.push(AdminProbe {
            probe_id:      row.get("probe_id"),
            company_id:    row.get("company_id"),
            status:        row.get("status"),
            last_seen_at:  last_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
            first_seen_at: first_seen.map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()),
        });
    }
    Ok(results)
}

/// Delete all reports and resource_statuses for a company (keeps heartbeats).
pub async fn delete_company_data(pool: &SqlitePool, company_id: &str) -> anyhow::Result<()> {
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
pub async fn delete_probe_data(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<()> {
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
pub async fn remove_probe(pool: &SqlitePool, probe_id: &str) -> anyhow::Result<()> {
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

 
 
