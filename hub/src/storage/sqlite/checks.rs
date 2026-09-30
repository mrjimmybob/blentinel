//! SQLite queries for checks and their results, behind
//! [`super::SqliteStorage`]'s `CheckStore` and `ReportStore` (M2-04, D-49).

use common::checks::{CheckId, CheckResult, CheckSpec, Metric};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::model::ProbeDevice;
use crate::storage::CheckRecord;

const CHECK_COLUMNS: &str = "k.id, k.probe_id, p.company_id, k.name, k.spec, k.enabled,
                             k.created_at, k.updated_at, k.updated_by";

fn parse_spec(json: &str) -> anyhow::Result<CheckSpec> {
    serde_json::from_str(json).map_err(|e| anyhow::anyhow!("stored check spec {json:?}: {e}"))
}

fn to_record(row: &SqliteRow) -> anyhow::Result<CheckRecord> {
    Ok(CheckRecord {
        id: CheckId(row.get("id")),
        probe_id: row.get("probe_id"),
        company_id: row.get("company_id"),
        name: row.get("name"),
        spec: parse_spec(row.get("spec"))?,
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        updated_by: row.get("updated_by"),
    })
}

pub(super) async fn probe_checks(
    pool: &SqlitePool,
    probe_id: &str,
) -> anyhow::Result<Vec<CheckRecord>> {
    let sql = format!(
        "SELECT {CHECK_COLUMNS} FROM checks k JOIN probes p ON p.id = k.probe_id
         WHERE k.probe_id = ? ORDER BY k.name COLLATE NOCASE, k.id"
    );
    let rows = sqlx::query(&sql).bind(probe_id).fetch_all(pool).await?;
    rows.iter().map(to_record).collect()
}

pub(super) async fn check(pool: &SqlitePool, id: &CheckId) -> anyhow::Result<Option<CheckRecord>> {
    let sql = format!(
        "SELECT {CHECK_COLUMNS} FROM checks k JOIN probes p ON p.id = k.probe_id WHERE k.id = ?"
    );
    let row = sqlx::query(&sql)
        .bind(id.as_str())
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(to_record).transpose()
}

pub(super) async fn create(pool: &SqlitePool, c: &CheckRecord) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO checks (id, probe_id, name, kind, spec, enabled, created_at, updated_at, updated_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(c.id.as_str())
    .bind(&c.probe_id)
    .bind(&c.name)
    .bind(c.spec.kind())
    .bind(serde_json::to_string(&c.spec)?)
    .bind(c.enabled)
    .bind(c.created_at)
    .bind(c.updated_at)
    .bind(&c.updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn update(pool: &SqlitePool, c: &CheckRecord) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE checks SET name = ?, kind = ?, spec = ?, enabled = ?, updated_at = ?, updated_by = ?
         WHERE id = ?",
    )
    .bind(&c.name)
    .bind(c.spec.kind())
    .bind(serde_json::to_string(&c.spec)?)
    .bind(c.enabled)
    .bind(c.updated_at)
    .bind(&c.updated_by)
    .bind(c.id.as_str())
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn delete(pool: &SqlitePool, id: &CheckId) -> anyhow::Result<bool> {
    let done = sqlx::query("DELETE FROM checks WHERE id = ?")
        .bind(id.as_str())
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Store a report's results and their metrics (inside `save_report`'s transaction).
pub(super) async fn insert_results(
    tx: &mut Transaction<'_, Sqlite>,
    report_id: i64,
    results: &[CheckResult],
) -> anyhow::Result<()> {
    for result in results {
        let result_id: i64 = sqlx::query_scalar(
            "INSERT INTO check_results (report_id, check_id, status, message, latency_ms)
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(report_id)
        .bind(result.check_id.as_str())
        .bind(result.status.as_str())
        .bind(&result.message)
        .bind(result.latency_ms.map(|l| l as i64))
        .fetch_one(tx.as_mut())
        .await?;
        for (position, metric) in result.metrics.iter().enumerate() {
            sqlx::query(
                "INSERT INTO check_metrics (result_id, position, key, value, unit)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(result_id)
            .bind(position as i64)
            .bind(&metric.key)
            .bind(metric.value)
            .bind(&metric.unit)
            .execute(tx.as_mut())
            .await?;
        }
    }
    Ok(())
}

/// The results of the probe's latest report, for checks that still exist, by name.
pub(super) async fn latest_devices(
    pool: &SqlitePool,
    probe_id: &str,
) -> anyhow::Result<Vec<ProbeDevice>> {
    let rows = sqlx::query(
        "SELECT r.id AS result_id, r.check_id, k.name, k.spec, r.status, r.message, r.latency_ms
         FROM check_results r
         JOIN checks k ON k.id = r.check_id
         WHERE r.report_id = (SELECT MAX(id) FROM reports WHERE probe_id = ?1)
         ORDER BY k.name COLLATE NOCASE, k.id",
    )
    .bind(probe_id)
    .fetch_all(pool)
    .await?;

    let mut devices = Vec::with_capacity(rows.len());
    for row in &rows {
        let spec = parse_spec(row.get("spec"))?;
        devices.push(ProbeDevice {
            check_id: row.get("check_id"),
            name: row.get("name"),
            kind: spec.kind().to_string(),
            target: spec.target(),
            status: row.get("status"),
            message: row.get("message"),
            latency_ms: row.get("latency_ms"),
            metrics: metrics(pool, row.get("result_id")).await?,
        });
    }
    Ok(devices)
}

async fn metrics(pool: &SqlitePool, result_id: i64) -> anyhow::Result<Vec<Metric>> {
    let rows = sqlx::query(
        "SELECT key, value, unit FROM check_metrics WHERE result_id = ? ORDER BY position",
    )
    .bind(result_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Metric {
            key: row.get("key"),
            value: row.get("value"),
            unit: row.get("unit"),
        })
        .collect())
}
