//! M6-01: Tier-selected historical queries (proposal 0007, D-10)
//!
//! Choose the right aggregation tier based on query date range:
//! - Recent (<1 day): raw check_results (accurate, no aggregation loss)
//! - Short-term (1-90 days): 5-minute rollups (balance accuracy/performance)
//! - Long-term (90+ days): hourly or daily rollups (efficient for trends/SLA)
//!
//! Falls back gracefully if a tier is empty.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

/// A rolled-up check result at any tier.
#[derive(Debug, Clone)]
pub struct RollupRow {
    pub check_id: String,
    pub bucket_start: DateTime<Utc>,
    pub status_up: i32,
    pub status_warn: i32,
    pub status_down: i32,
    pub status_unknown: i32,
    pub latency_ms_min: Option<i32>,
    pub latency_ms_avg: Option<f64>,
    pub latency_ms_max: Option<i32>,
    pub count_reports: i32,
}

/// Which tier to query for a date range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryTier {
    Raw,
    Rollups5m,
    RollupsHourly,
    RollupsDaily,
}

impl QueryTier {
    /// Choose tier based on range duration. Prefers efficiency over accuracy for long ranges.
    pub fn select(range_start: DateTime<Utc>, range_end: DateTime<Utc>) -> Self {
        let duration = range_end.signed_duration_since(range_start);

        if duration <= Duration::days(1) {
            QueryTier::Raw
        } else if duration <= Duration::days(90) {
            QueryTier::Rollups5m
        } else if duration <= Duration::days(730) {
            QueryTier::RollupsHourly
        } else {
            QueryTier::RollupsDaily
        }
    }
}

/// Query raw check results for a date range.
pub async fn query_raw_results(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<RollupRow>> {
    let start_str = range_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end_str = range_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            i32,
            i32,
            i32,
            i32,
            Option<i32>,
            Option<f64>,
            Option<i32>,
            i32,
        ),
    >(
        r#"
        SELECT
            cr.check_id,
            r.created_at,
            SUM(CASE WHEN cr.status = 'Up' THEN 1 ELSE 0 END),
            SUM(CASE WHEN cr.status = 'Warning' THEN 1 ELSE 0 END),
            SUM(CASE WHEN cr.status = 'Down' THEN 1 ELSE 0 END),
            SUM(CASE WHEN cr.status = 'Unknown' THEN 1 ELSE 0 END),
            MIN(cr.latency_ms),
            AVG(cr.latency_ms),
            MAX(cr.latency_ms),
            COUNT(*) as count_reports
        FROM check_results cr
        INNER JOIN reports r ON cr.report_id = r.id
        WHERE cr.check_id = ? AND r.created_at >= ? AND r.created_at < ?
        GROUP BY cr.check_id, r.created_at
        "#,
    )
    .bind(check_id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                check_id,
                bucket_start_str,
                up,
                warn,
                down,
                unknown,
                lat_min,
                lat_avg,
                lat_max,
                count,
            )| {
                let bucket_start = bucket_start_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or(Utc::now());
                RollupRow {
                    check_id,
                    bucket_start,
                    status_up: up,
                    status_warn: warn,
                    status_down: down,
                    status_unknown: unknown,
                    latency_ms_min: lat_min,
                    latency_ms_avg: lat_avg,
                    latency_ms_max: lat_max,
                    count_reports: count,
                }
            },
        )
        .collect())
}

/// Query 5-minute rollups for a date range.
pub async fn query_5m_rollups(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<RollupRow>> {
    let start_str = range_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end_str = range_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            i32,
            i32,
            i32,
            i32,
            Option<i32>,
            Option<f64>,
            Option<i32>,
            i32,
        ),
    >(
        r#"
        SELECT
            check_id,
            bucket_start,
            status_up,
            status_warn,
            status_down,
            status_unknown,
            latency_ms_min,
            latency_ms_avg,
            latency_ms_max,
            count_reports
        FROM check_rollups_5m
        WHERE check_id = ? AND bucket_start >= ? AND bucket_start < ?
        ORDER BY bucket_start ASC
        "#,
    )
    .bind(check_id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                check_id,
                bucket_start_str,
                up,
                warn,
                down,
                unknown,
                lat_min,
                lat_avg,
                lat_max,
                count,
            )| {
                let bucket_start = bucket_start_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or(Utc::now());
                RollupRow {
                    check_id,
                    bucket_start,
                    status_up: up,
                    status_warn: warn,
                    status_down: down,
                    status_unknown: unknown,
                    latency_ms_min: lat_min,
                    latency_ms_avg: lat_avg,
                    latency_ms_max: lat_max,
                    count_reports: count,
                }
            },
        )
        .collect())
}

/// Query hourly rollups for a date range.
pub async fn query_hourly_rollups(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<RollupRow>> {
    let start_str = range_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end_str = range_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            i32,
            i32,
            i32,
            i32,
            Option<i32>,
            Option<f64>,
            Option<i32>,
            i32,
        ),
    >(
        r#"
        SELECT
            check_id,
            bucket_start,
            status_up,
            status_warn,
            status_down,
            status_unknown,
            latency_ms_min,
            latency_ms_avg,
            latency_ms_max,
            count_reports
        FROM check_rollups_hourly
        WHERE check_id = ? AND bucket_start >= ? AND bucket_start < ?
        ORDER BY bucket_start ASC
        "#,
    )
    .bind(check_id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                check_id,
                bucket_start_str,
                up,
                warn,
                down,
                unknown,
                lat_min,
                lat_avg,
                lat_max,
                count,
            )| {
                let bucket_start = bucket_start_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or(Utc::now());
                RollupRow {
                    check_id,
                    bucket_start,
                    status_up: up,
                    status_warn: warn,
                    status_down: down,
                    status_unknown: unknown,
                    latency_ms_min: lat_min,
                    latency_ms_avg: lat_avg,
                    latency_ms_max: lat_max,
                    count_reports: count,
                }
            },
        )
        .collect())
}

/// Query daily rollups for a date range.
pub async fn query_daily_rollups(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<RollupRow>> {
    let start_str = range_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end_str = range_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            i32,
            i32,
            i32,
            i32,
            Option<i32>,
            Option<f64>,
            Option<i32>,
            i32,
        ),
    >(
        r#"
        SELECT
            check_id,
            bucket_start,
            status_up,
            status_warn,
            status_down,
            status_unknown,
            latency_ms_min,
            latency_ms_avg,
            latency_ms_max,
            count_reports
        FROM check_rollups_daily
        WHERE check_id = ? AND bucket_start >= ? AND bucket_start < ?
        ORDER BY bucket_start ASC
        "#,
    )
    .bind(check_id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                check_id,
                bucket_start_str,
                up,
                warn,
                down,
                unknown,
                lat_min,
                lat_avg,
                lat_max,
                count,
            )| {
                let bucket_start = bucket_start_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or(Utc::now());
                RollupRow {
                    check_id,
                    bucket_start,
                    status_up: up,
                    status_warn: warn,
                    status_down: down,
                    status_unknown: unknown,
                    latency_ms_min: lat_min,
                    latency_ms_avg: lat_avg,
                    latency_ms_max: lat_max,
                    count_reports: count,
                }
            },
        )
        .collect())
}

/// Execute a tier-selected query, attempting each tier in order until results are found.
/// Falls back gracefully if a tier is unavailable or empty.
pub async fn query_with_fallback(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Vec<RollupRow>> {
    let preferred = QueryTier::select(range_start, range_end);

    // Try preferred tier first
    let results = match preferred {
        QueryTier::Raw => query_raw_results(pool, check_id, range_start, range_end).await?,
        QueryTier::Rollups5m => query_5m_rollups(pool, check_id, range_start, range_end).await?,
        QueryTier::RollupsHourly => {
            query_hourly_rollups(pool, check_id, range_start, range_end).await?
        }
        QueryTier::RollupsDaily => {
            query_daily_rollups(pool, check_id, range_start, range_end).await?
        }
    };

    // If preferred tier returned results, use them
    if !results.is_empty() {
        return Ok(results);
    }

    // Fallback order: try next-finer tier if preferred was empty
    let fallback_results = match preferred {
        QueryTier::Rollups5m => query_raw_results(pool, check_id, range_start, range_end).await?,
        QueryTier::RollupsHourly => {
            let five_m = query_5m_rollups(pool, check_id, range_start, range_end).await?;
            if !five_m.is_empty() {
                five_m
            } else {
                query_raw_results(pool, check_id, range_start, range_end).await?
            }
        }
        QueryTier::RollupsDaily => {
            let hourly = query_hourly_rollups(pool, check_id, range_start, range_end).await?;
            if !hourly.is_empty() {
                hourly
            } else {
                let five_m = query_5m_rollups(pool, check_id, range_start, range_end).await?;
                if !five_m.is_empty() {
                    five_m
                } else {
                    query_raw_results(pool, check_id, range_start, range_end).await?
                }
            }
        }
        QueryTier::Raw => vec![],
    };

    Ok(fallback_results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_raw_for_short_ranges() {
        let start = Utc::now();
        let end = start + Duration::hours(6);
        assert_eq!(QueryTier::select(start, end), QueryTier::Raw);
    }

    #[test]
    fn select_5m_for_short_term() {
        let start = Utc::now() - Duration::days(30);
        let end = Utc::now();
        assert_eq!(QueryTier::select(start, end), QueryTier::Rollups5m);
    }

    #[test]
    fn select_hourly_for_medium_term() {
        let start = Utc::now() - Duration::days(180);
        let end = Utc::now();
        assert_eq!(QueryTier::select(start, end), QueryTier::RollupsHourly);
    }

    #[test]
    fn select_daily_for_long_term() {
        let start = Utc::now() - Duration::days(2 * 365);
        let end = Utc::now();
        assert_eq!(QueryTier::select(start, end), QueryTier::RollupsDaily);
    }
}
