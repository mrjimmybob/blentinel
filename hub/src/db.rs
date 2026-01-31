#![cfg(feature = "ssr")]
use chrono::Utc;
use sqlx::{SqlitePool, Row};

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
        "INSERT INTO reports (probe_id, company_id, timestamp, interval_seconds)
         VALUES (?, ?, ?, ?) RETURNING id"
    )
        .bind(&report.probe_id)
        .bind(&report.company_id)
        .bind(report.timestamp)
        .bind(report.interval_seconds)
        .fetch_one(&mut *tx)
        .await?;

    let report_id: i64 = row.get("id");

    for res in &report.resources {
        sqlx::query(
            "INSERT INTO resource_statuses
             (report_id, name, resource_type, target, status, message, latency_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
            .bind(report_id)
            .bind(&res.name)
            .bind(&res.resource_type)
            .bind(&res.target)
            .bind(format!("{:?}", res.status))
            .bind(&res.message)
            .bind(res.latency_ms.map(|l| l as i64))
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
