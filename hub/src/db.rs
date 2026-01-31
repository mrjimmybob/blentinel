#![cfg(feature = "ssr")]
use sqlx::{SqlitePool, Row};

#[cfg(feature = "ssr")]
pub async fn setup_tables(pool: &SqlitePool) -> anyhow::Result<()> {
    // We use a transaction to ensure the schema is applied atomically
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

    Ok(())
}

#[cfg(feature = "ssr")]
pub async fn save_report(pool: &SqlitePool, report: &common::models::StatusReport) -> anyhow::Result<()> {
    // Start a transaction so we don't get partial saves
    let mut tx = pool.begin().await?;

    // Insert the main report
    let row = sqlx::query("INSERT INTO reports (probe_id, company_id, timestamp, interval_seconds) 
                        VALUES (?, ?, ?, ?) RETURNING id")
        .bind(&report.probe_id)
        .bind(&report.company_id)
        .bind(report.timestamp)
        .bind(report.interval_seconds)
        .fetch_one(&mut *tx)
        .await?;

    let report_id: i64 = row.get("id");

    // Inside your loop in save_report:
    for res in &report.resources {
        sqlx::query("INSERT INTO resource_statuses (report_id, name, resource_type, target, status, message, latency_ms) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(report_id)
            .bind(&res.name)
            .bind(&res.resource_type) // Use the correct field name from your struct
            .bind(&res.target)
            .bind(format!("{:?}", res.status)) // Convert Enum to String for DB
            .bind(&res.message)
            .bind(res.latency_ms.map(|l| l as i64)) // Convert Option<u64> to Option<i64>
            .execute(tx.as_mut()) // Correct way to use transaction
            .await?;
    }

    tx.commit().await?;
    Ok(())
}