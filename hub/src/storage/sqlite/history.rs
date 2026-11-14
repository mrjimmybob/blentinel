//! History rollups and retention (proposal 0007 §2, D-111), for SQLite.
//!
//! A report's results are added to every rollup width in the transaction that
//! stores it ([`add_report`]), so rollups are always exact sums over every
//! result ever stored, and pruning raw rows never changes them.

use chrono::{DateTime, TimeZone, Utc};
use sqlx::{SqliteConnection, SqlitePool};

use crate::storage::{
    CheckBucket, CheckTotals, HistoryCutoffs, HistoryRetention, MetricBucket, PrunedHistory,
    RollupWidth, StatusCounts, StoredResult,
};

/// Every rollup width, as a table to join.
const WIDTHS: &str = "(SELECT 300 AS width UNION ALL SELECT 3600 UNION ALL SELECT 86400)";

/// The start of `r.timestamp`'s bucket of `w.width` seconds, in Unix seconds.
const BUCKET: &str = "(CAST(strftime('%s', r.timestamp) AS INTEGER) / w.width) * w.width";

/// Add the results of the reports matching `filter` (over `reports r`) to the
/// rollups. Adding twice counts twice: callers add each report once.
fn add_checks_sql(filter: &str) -> String {
    format!(
        "INSERT INTO check_rollups
             (width, bucket_start, company_id, probe_id, check_id, up, warning, down, unknown,
              maintenance, latency_min, latency_max, latency_sum, latency_count)
         SELECT w.width, {BUCKET}, r.company_id, r.probe_id, cr.check_id,
                SUM(cr.maintenance = 0 AND cr.status = 'Up'),
                SUM(cr.maintenance = 0 AND cr.status = 'Warning'),
                SUM(cr.maintenance = 0 AND cr.status = 'Down'),
                SUM(cr.maintenance = 0 AND cr.status = 'Unknown'),
                SUM(cr.maintenance <> 0),
                MIN(cr.latency_ms), MAX(cr.latency_ms),
                COALESCE(SUM(cr.latency_ms), 0), COUNT(cr.latency_ms)
         FROM reports r
         JOIN check_results cr ON cr.report_id = r.id
         CROSS JOIN {WIDTHS} w
         WHERE {filter}
         GROUP BY w.width, {BUCKET}, r.company_id, r.probe_id, cr.check_id
         ON CONFLICT (width, check_id, bucket_start, company_id) DO UPDATE SET
             up            = up + excluded.up,
             warning       = warning + excluded.warning,
             down          = down + excluded.down,
             unknown       = unknown + excluded.unknown,
             maintenance   = maintenance + excluded.maintenance,
             latency_min   = COALESCE(MIN(latency_min, excluded.latency_min),
                                      latency_min, excluded.latency_min),
             latency_max   = COALESCE(MAX(latency_max, excluded.latency_max),
                                      latency_max, excluded.latency_max),
             latency_sum   = latency_sum + excluded.latency_sum,
             latency_count = latency_count + excluded.latency_count"
    )
}

/// As [`add_checks_sql`], for the results' metrics.
fn add_metrics_sql(filter: &str) -> String {
    format!(
        "INSERT INTO metric_rollups
             (width, bucket_start, company_id, probe_id, check_id, metric_key, unit,
              value_min, value_max, value_sum, value_count)
         SELECT w.width, {BUCKET}, r.company_id, r.probe_id, cr.check_id, m.key, MAX(m.unit),
                MIN(m.value), MAX(m.value), SUM(m.value), COUNT(*)
         FROM reports r
         JOIN check_results cr ON cr.report_id = r.id
         JOIN check_metrics m ON m.result_id = cr.id
         CROSS JOIN {WIDTHS} w
         WHERE {filter}
         GROUP BY w.width, {BUCKET}, r.company_id, r.probe_id, cr.check_id, m.key
         ON CONFLICT (width, check_id, metric_key, bucket_start, company_id) DO UPDATE SET
             unit        = excluded.unit,
             value_min   = MIN(value_min, excluded.value_min),
             value_max   = MAX(value_max, excluded.value_max),
             value_sum   = value_sum + excluded.value_sum,
             value_count = value_count + excluded.value_count"
    )
}

/// Add report `report_id`'s results to the rollups (inside `save_report`'s
/// transaction, so a report is added exactly once).
pub(super) async fn add_report(conn: &mut SqliteConnection, report_id: i64) -> sqlx::Result<()> {
    sqlx::query(&add_checks_sql("r.id = ?1"))
        .bind(report_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(&add_metrics_sql("r.id = ?1"))
        .bind(report_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Recompute every rollup from the reports present (an archive's copy, which
/// keeps only its older reports).
pub(super) async fn rebuild(conn: &mut SqliteConnection) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM check_rollups")
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM metric_rollups")
        .execute(&mut *conn)
        .await?;
    // `WHERE 1`: an upsert's SELECT needs a WHERE clause (SQLite's grammar).
    sqlx::query(&add_checks_sql("1"))
        .execute(&mut *conn)
        .await?;
    sqlx::query(&add_metrics_sql("1"))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Delete the rollups of a company or probe (`column` is `company_id` or
/// `probe_id`), with its reports.
pub(super) async fn delete_of(
    conn: &mut SqliteConnection,
    column: &'static str,
    id: &str,
) -> sqlx::Result<()> {
    for table in ["check_rollups", "metric_rollups"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE {column} = ?1"))
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

fn at(unix: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(unix, 0).single().unwrap_or_default()
}

/// Buckets overlapping `[from, to)`: `bucket_start + width > from` and
/// `bucket_start < to`. Returns the two bounds to bind.
fn bounds(width: RollupWidth, from: DateTime<Utc>, to: DateTime<Utc>) -> (i64, i64) {
    (from.timestamp() - width.seconds(), to.timestamp())
}

type CountsRow = (i64, i64, i64, i64, i64);

/// `bucket_start`, the five counts, latency min, max, sum and count.
type CheckBucketRow = (
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
);

fn counts((up, warning, down, unknown, maintenance): CountsRow) -> StatusCounts {
    StatusCounts {
        up,
        warning,
        down,
        unknown,
        maintenance,
    }
}

pub(super) async fn check_history(
    pool: &SqlitePool,
    company_id: &str,
    check_id: &str,
    width: RollupWidth,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<CheckBucket>> {
    let (after, before) = bounds(width, from, to);
    let rows: Vec<CheckBucketRow> = sqlx::query_as(
        "SELECT bucket_start, up, warning, down, unknown, maintenance,
                    latency_min, latency_max, latency_sum, latency_count
             FROM check_rollups
             WHERE width = ?1 AND check_id = ?2 AND company_id = ?3
               AND bucket_start > ?4 AND bucket_start < ?5
             ORDER BY bucket_start",
    )
    .bind(width.seconds())
    .bind(check_id)
    .bind(company_id)
    .bind(after)
    .bind(before)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(start, up, warning, down, unknown, maintenance, min, max, sum, n)| CheckBucket {
                start: at(start),
                counts: counts((up, warning, down, unknown, maintenance)),
                latency_min_ms: min,
                latency_max_ms: max,
                latency_avg_ms: (n > 0).then(|| sum as f64 / n as f64),
            },
        )
        .collect())
}

pub(super) async fn metric_history(
    pool: &SqlitePool,
    company_id: &str,
    check_id: &str,
    width: RollupWidth,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<MetricBucket>> {
    let (after, before) = bounds(width, from, to);
    let rows: Vec<(i64, String, String, f64, f64, f64, i64)> = sqlx::query_as(
        "SELECT bucket_start, metric_key, unit, value_min, value_max, value_sum, value_count
         FROM metric_rollups
         WHERE width = ?1 AND check_id = ?2 AND company_id = ?3
           AND bucket_start > ?4 AND bucket_start < ?5
         ORDER BY metric_key, bucket_start",
    )
    .bind(width.seconds())
    .bind(check_id)
    .bind(company_id)
    .bind(after)
    .bind(before)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(start, key, unit, min, max, sum, n)| MetricBucket {
            start: at(start),
            key,
            unit,
            min,
            max,
            avg: sum / n.max(1) as f64,
        })
        .collect())
}

pub(super) async fn company_check_totals(
    pool: &SqlitePool,
    company_id: &str,
    width: RollupWidth,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<CheckTotals>> {
    let (after, before) = bounds(width, from, to);
    let rows: Vec<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT check_id, probe_id, SUM(up), SUM(warning), SUM(down), SUM(unknown),
                SUM(maintenance)
         FROM check_rollups
         WHERE width = ?1 AND company_id = ?2 AND bucket_start > ?3 AND bucket_start < ?4
         GROUP BY check_id, probe_id
         ORDER BY check_id, probe_id",
    )
    .bind(width.seconds())
    .bind(company_id)
    .bind(after)
    .bind(before)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(check_id, probe_id, up, warning, down, unknown, maintenance)| CheckTotals {
                check_id,
                probe_id,
                counts: counts((up, warning, down, unknown, maintenance)),
            },
        )
        .collect())
}

/// The company's uptime buckets of `group` seconds from its rollups of
/// `width` overlapping `[from, ∞)` (`None`: all), as `("YYYY-MM-DD HH:MM",
/// up + warning, down, maintenance)`.
pub(super) async fn company_uptime(
    pool: &SqlitePool,
    company_id: &str,
    width: RollupWidth,
    group: i64,
    from: Option<DateTime<Utc>>,
) -> anyhow::Result<Vec<(String, i64, i64, i64)>> {
    let after = from.map_or(i64::MIN, |from| from.timestamp() - width.seconds());
    Ok(sqlx::query_as(
        "SELECT strftime('%Y-%m-%d %H:%M', (bucket_start / ?4) * ?4, 'unixepoch') AS bucket,
                SUM(up + warning), SUM(down), SUM(maintenance)
         FROM check_rollups
         WHERE width = ?1 AND company_id = ?2 AND bucket_start > ?3
         GROUP BY (bucket_start / ?4)
         ORDER BY (bucket_start / ?4)",
    )
    .bind(width.seconds())
    .bind(company_id)
    .bind(after)
    .bind(group)
    .fetch_all(pool)
    .await?)
}

pub(super) async fn retention(pool: &SqlitePool) -> anyhow::Result<HistoryRetention> {
    let row: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT raw_days, five_minute_days, hourly_days FROM history_retention WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some((raw, five, hourly)) => HistoryRetention {
            raw_days: u32::try_from(raw)?,
            five_minute_days: u32::try_from(five)?,
            hourly_days: u32::try_from(hourly)?,
        },
        None => HistoryRetention::default(),
    })
}

pub(super) async fn set_retention(
    pool: &SqlitePool,
    retention: &HistoryRetention,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO history_retention
             (id, raw_days, five_minute_days, hourly_days, updated_by, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
             raw_days = excluded.raw_days,
             five_minute_days = excluded.five_minute_days,
             hourly_days = excluded.hourly_days,
             updated_by = excluded.updated_by,
             updated_at = excluded.updated_at",
    )
    .bind(i64::from(retention.raw_days))
    .bind(i64::from(retention.five_minute_days))
    .bind(i64::from(retention.hourly_days))
    .bind(updated_by)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn prune(
    pool: &SqlitePool,
    cutoffs: &HistoryCutoffs,
) -> anyhow::Result<PrunedHistory> {
    let mut tx = super::write_transaction(pool).await?;
    // Results and metrics go with their report (ON DELETE CASCADE). Each
    // probe's newest report stays: it is the probe's current state.
    let reports = sqlx::query(
        "DELETE FROM reports WHERE timestamp < ?1
           AND id NOT IN (SELECT MAX(id) FROM reports GROUP BY probe_id)",
    )
    .bind(cutoffs.raw)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let mut buckets = [0u64; 2];
    for (n, (width, cutoff)) in [
        (RollupWidth::FiveMinutes, cutoffs.five_minutes),
        (RollupWidth::Hourly, cutoffs.hourly),
    ]
    .into_iter()
    .enumerate()
    {
        buckets[n] =
            sqlx::query("DELETE FROM check_rollups WHERE width = ?1 AND bucket_start < ?2")
                .bind(width.seconds())
                .bind(cutoff.timestamp())
                .execute(&mut *tx)
                .await?
                .rows_affected();
        sqlx::query("DELETE FROM metric_rollups WHERE width = ?1 AND bucket_start < ?2")
            .bind(width.seconds())
            .bind(cutoff.timestamp())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(PrunedHistory {
        reports,
        five_minute_buckets: buckets[0],
        hourly_buckets: buckets[1],
    })
}

/// A check belongs to one probe, whose reports are stored in time order
/// (replay protection, D-28), so the newest results have the highest IDs.
pub(super) async fn check_results(
    pool: &SqlitePool,
    check_id: &str,
    limit: u32,
) -> anyhow::Result<Vec<StoredResult>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT cr.id, r.timestamp, cr.status, cr.reason, cr.message, cr.latency_ms, cr.maintenance
         FROM check_results cr
         JOIN reports r ON r.id = cr.report_id
         WHERE cr.check_id = ?
         ORDER BY cr.id DESC
         LIMIT ?",
    )
    .bind(check_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Their metrics in one query, in the order reported.
    let ids: Vec<i64> = rows.iter().map(|row| row.get("id")).collect();
    let marks = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "SELECT result_id, key, value, unit FROM check_metrics
         WHERE result_id IN ({marks}) ORDER BY result_id, position"
    );
    let mut query = sqlx::query(&sql);
    for id in &ids {
        query = query.bind(id);
    }
    let mut metrics: std::collections::HashMap<i64, Vec<common::checks::Metric>> =
        std::collections::HashMap::new();
    for row in query.fetch_all(pool).await? {
        metrics
            .entry(row.get("result_id"))
            .or_default()
            .push(common::checks::Metric {
                key: row.get("key"),
                value: row.get("value"),
                unit: row.get("unit"),
            });
    }

    rows.iter()
        .map(|row| {
            let id: i64 = row.get("id");
            Ok(StoredResult {
                at: row.get("timestamp"),
                status: super::checks::stored_status(row.get("status"))?,
                reason: row.get("reason"),
                message: row.get("message"),
                latency_ms: row.get("latency_ms"),
                in_maintenance: row.get("maintenance"),
                metrics: metrics.remove(&id).unwrap_or_default(),
            })
        })
        .collect()
}
