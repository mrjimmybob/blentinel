//! SQLite queries for checks and their results, behind
//! [`super::SqliteStorage`]'s `CheckStore` and `ReportStore` (M2-04, D-49).

use common::checks::{CheckId, CheckResult, CheckSpec, Metric};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::model::{AlertAfter, Evaluation, MetricLimits, ProbeDevice, Status};
use crate::storage::CheckRecord;

const CHECK_COLUMNS: &str = "k.id, k.probe_id, p.company_id, k.name, k.spec, k.enabled,
                             k.fail_after, k.recover_after, k.limits, k.created_at,
                             k.updated_at, k.updated_by";

/// A result's status as stored (`Up`, `Warning`, `Down`).
fn stored_status(text: String) -> anyhow::Result<Status> {
    Status::parse(&text).ok_or_else(|| anyhow::anyhow!("unknown stored status {text:?}"))
}

/// A check's own limits as stored (`NULL`: none).
fn stored_limits(json: Option<String>) -> anyhow::Result<MetricLimits> {
    json.map_or(Ok(MetricLimits::default()), |json| {
        serde_json::from_str(&json).map_err(|e| anyhow::anyhow!("stored limits {json:?}: {e}"))
    })
}

/// `NULL` for no limits.
fn limits_json(limits: &MetricLimits) -> anyhow::Result<Option<String>> {
    Ok((!limits.is_empty())
        .then(|| serde_json::to_string(limits))
        .transpose()?)
}

/// An optional count as stored (`NULL`: the default).
fn count(row: &SqliteRow, column: &str) -> anyhow::Result<Option<u32>> {
    row.get::<Option<i64>, _>(column)
        .map(u32::try_from)
        .transpose()
        .map_err(Into::into)
}

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
        alert_after: AlertAfter {
            fail_after: count(row, "fail_after")?,
            recover_after: count(row, "recover_after")?,
        },
        limits: stored_limits(row.get("limits"))?,
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
        "INSERT INTO checks (id, probe_id, name, kind, spec, enabled, fail_after, recover_after,
             limits, created_at, updated_at, updated_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(c.id.as_str())
    .bind(&c.probe_id)
    .bind(&c.name)
    .bind(c.spec.kind())
    .bind(serde_json::to_string(&c.spec)?)
    .bind(c.enabled)
    .bind(c.alert_after.fail_after.map(i64::from))
    .bind(c.alert_after.recover_after.map(i64::from))
    .bind(limits_json(&c.limits)?)
    .bind(c.created_at)
    .bind(c.updated_at)
    .bind(&c.updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn update(pool: &SqlitePool, c: &CheckRecord) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE checks SET name = ?, kind = ?, spec = ?, enabled = ?, fail_after = ?,
             recover_after = ?, limits = ?, updated_at = ?, updated_by = ?
         WHERE id = ?",
    )
    .bind(&c.name)
    .bind(c.spec.kind())
    .bind(serde_json::to_string(&c.spec)?)
    .bind(c.enabled)
    .bind(c.alert_after.fail_after.map(i64::from))
    .bind(c.alert_after.recover_after.map(i64::from))
    .bind(limits_json(&c.limits)?)
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
    evaluations: &[Evaluation],
) -> anyhow::Result<()> {
    for (result, evaluation) in results.iter().zip(evaluations) {
        let result_id: i64 = sqlx::query_scalar(
            "INSERT INTO check_results
                 (report_id, check_id, status, reason, maintenance, message, latency_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(report_id)
        .bind(result.check_id.as_str())
        .bind(evaluation.status.as_str())
        .bind(&evaluation.reason)
        .bind(evaluation.in_maintenance)
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
        "SELECT r.id AS result_id, r.check_id, k.name, k.spec, r.status, r.reason, r.message,
                r.latency_ms, r.maintenance, hb.status AS probe_state
         FROM check_results r
         JOIN checks k ON k.id = r.check_id
         LEFT JOIN probe_heartbeats hb ON hb.probe_id = ?1
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
            // A stopped probe's last results are history: nobody knows now (D-64).
            status: match row.get::<Option<String>, _>("probe_state").as_deref() {
                Some("expired") => Status::Unknown,
                _ => stored_status(row.get("status"))?,
            },
            reason: row.get("reason"),
            in_maintenance: row.get("maintenance"),
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
