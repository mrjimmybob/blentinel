//! M6-01: Historical data rollups (proposal 0007, D-10)
//!
//! Idempotent computation of check result aggregations: 5-minute, hourly, daily.
//! Runs incrementally; re-runs safely overwrite closed buckets with identical results.
//! Handles late-arriving buffered reports (FP-4) correctly.

use anyhow::Result;
use chrono::{DateTime, Duration, Timelike, Utc};
use sqlx::{FromRow, SqlitePool};

/// A closed 5-minute bucket ready for rollup computation.
#[derive(Debug, Clone)]
pub struct RollupBucket5m {
    pub bucket_start: DateTime<Utc>,
}

impl RollupBucket5m {
    pub fn new(dt: DateTime<Utc>) -> Self {
        let minutes = dt.minute();
        let bucket_minute = (minutes / 5) * 5;
        let bucket = dt
            .with_minute(bucket_minute)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        Self {
            bucket_start: bucket,
        }
    }

    /// A 5-minute bucket is closed 5 minutes after it started.
    pub fn is_closed(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.bucket_start) >= Duration::minutes(5)
    }
}

/// Aggregated check result for a 5-minute window.
#[derive(Debug, Clone, FromRow)]
pub struct CheckRollup {
    pub check_id: String,
    pub bucket_start: String,  // DATETIME as TEXT from DB
    pub status_up: i32,
    pub status_warn: i32,
    pub status_down: i32,
    pub status_unknown: i32,
    pub latency_ms_min: Option<i32>,
    pub latency_ms_avg: Option<f64>,
    pub latency_ms_max: Option<i32>,
    pub count_reports: i32,
    pub computed_at: String,  // DATETIME as TEXT from DB
}

/// Compute 5-minute rollups for all closed buckets (idempotent).
///
/// Re-running this function for an already-computed bucket overwrites with identical results.
/// Late-arriving reports (received after a bucket closed) are included on re-run.
///
/// **Safety:** Only aggregates closed buckets; never touches incomplete windows.
pub async fn compute_5m_rollups(pool: &SqlitePool, now: DateTime<Utc>) -> Result<usize> {
    // Find the oldest report with results
    let oldest_report = sqlx::query_scalar::<_, String>(
        "SELECT MIN(created_at) FROM reports WHERE created_at < ?",
    )
    .bind(now - Duration::minutes(5))  // Only closed buckets
    .fetch_optional(pool)
    .await?;

    let Some(oldest_str) = oldest_report else {
        return Ok(0);  // No data yet
    };

    let oldest = oldest_str.parse::<DateTime<Utc>>().unwrap_or(now);
    let mut bucket_start = RollupBucket5m::new(oldest).bucket_start;
    let cutoff = RollupBucket5m::new(now - Duration::minutes(5)).bucket_start;

    let mut count = 0;

    while bucket_start <= cutoff {
        let bucket_end = bucket_start + Duration::minutes(5);
        count += compute_single_5m_bucket(pool, bucket_start, bucket_end).await?;
        bucket_start = bucket_end;
    }

    Ok(count)
}

/// Aggregate a single 5-minute bucket (UPSERT, idempotent).
async fn compute_single_5m_bucket(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    bucket_end: DateTime<Utc>,
) -> Result<usize> {
    // Aggregate check results in this bucket
    let rows = sqlx::query!(
        r#"
        SELECT
            cr.check_id,
            CAST(COUNT(*) as INTEGER) as count_reports,
            CAST(COALESCE(SUM(CASE WHEN cr.status = 'Up' THEN 1 ELSE 0 END), 0) as INTEGER) as status_up,
            CAST(COALESCE(SUM(CASE WHEN cr.status = 'Warning' THEN 1 ELSE 0 END), 0) as INTEGER) as status_warn,
            CAST(COALESCE(SUM(CASE WHEN cr.status = 'Down' THEN 1 ELSE 0 END), 0) as INTEGER) as status_down,
            CAST(COALESCE(SUM(CASE WHEN cr.status = 'Unknown' THEN 1 ELSE 0 END), 0) as INTEGER) as status_unknown,
            CAST(COALESCE(MIN(cr.latency_ms), 0) as INTEGER) as latency_min,
            COALESCE(AVG(CAST(cr.latency_ms as REAL)), 0.0) as latency_avg,
            CAST(COALESCE(MAX(cr.latency_ms), 0) as INTEGER) as latency_max
        FROM check_results cr
        INNER JOIN reports r ON cr.report_id = r.id
        WHERE r.timestamp >= ? AND r.timestamp < ?
        GROUP BY cr.check_id
        "#,
        bucket_start,
        bucket_end
    )
    .fetch_all(pool)
    .await?;

    let now_str = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let bucket_start_str = bucket_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    for row in rows {
        sqlx::query!(
            r#"
            INSERT INTO check_rollups_5m
            (check_id, bucket_start, status_up, status_warn, status_down, status_unknown,
             latency_ms_min, latency_ms_avg, latency_ms_max, count_reports, computed_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(check_id, bucket_start) DO UPDATE SET
                status_up = excluded.status_up,
                status_warn = excluded.status_warn,
                status_down = excluded.status_down,
                status_unknown = excluded.status_unknown,
                latency_ms_min = excluded.latency_ms_min,
                latency_ms_avg = excluded.latency_ms_avg,
                latency_ms_max = excluded.latency_ms_max,
                count_reports = excluded.count_reports,
                computed_at = excluded.computed_at
            "#,
            row.check_id,
            bucket_start_str,
            row.status_up,
            row.status_warn,
            row.status_down,
            row.status_unknown,
            row.latency_min,
            row.latency_avg,
            row.latency_max,
            row.count_reports,
            now_str
        )
        .execute(pool)
        .await?;
    }

    Ok(rows.len())
}

/// Compute hourly rollups from closed 5-minute buckets (idempotent).
pub async fn compute_hourly_rollups(pool: &SqlitePool, now: DateTime<Utc>) -> Result<usize> {
    // Find oldest complete hour
    let oldest = sqlx::query_scalar::<_, Option<String>>(
        "SELECT MIN(bucket_start) FROM check_rollups_5m WHERE bucket_start < ?",
    )
    .bind((now - Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    .fetch_one(pool)
    .await?;

    let Some(oldest_str) = oldest else {
        return Ok(0);
    };

    let oldest_dt = oldest_str.parse::<DateTime<Utc>>().unwrap_or(now);
    let mut hour_start = oldest_dt
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();

    let cutoff = (now - Duration::hours(1))
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();

    let mut count = 0;

    while hour_start <= cutoff {
        let hour_end = hour_start + Duration::hours(1);
        count += compute_single_hourly_bucket(pool, hour_start, hour_end).await?;
        hour_start = hour_end;
    }

    Ok(count)
}

/// Aggregate a single hourly bucket from 5-minute rollups (UPSERT, idempotent).
async fn compute_single_hourly_bucket(
    pool: &SqlitePool,
    hour_start: DateTime<Utc>,
    hour_end: DateTime<Utc>,
) -> Result<usize> {
    let rows = sqlx::query!(
        r#"
        SELECT
            check_id,
            CAST(COALESCE(SUM(status_up), 0) as INTEGER) as status_up,
            CAST(COALESCE(SUM(status_warn), 0) as INTEGER) as status_warn,
            CAST(COALESCE(SUM(status_down), 0) as INTEGER) as status_down,
            CAST(COALESCE(SUM(status_unknown), 0) as INTEGER) as status_unknown,
            CAST(COALESCE(MIN(latency_ms_min), 0) as INTEGER) as latency_min,
            COALESCE(AVG(CAST(latency_ms_avg as REAL)), 0.0) as latency_avg,
            CAST(COALESCE(MAX(latency_ms_max), 0) as INTEGER) as latency_max,
            CAST(COALESCE(SUM(count_reports), 0) as INTEGER) as count_reports
        FROM check_rollups_5m
        WHERE bucket_start >= ? AND bucket_start < ?
        GROUP BY check_id
        "#,
        hour_start,
        hour_end
    )
    .fetch_all(pool)
    .await?;

    let now_str = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let hour_start_str = hour_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    for row in rows {
        sqlx::query!(
            r#"
            INSERT INTO check_rollups_hourly
            (check_id, bucket_start, status_up, status_warn, status_down, status_unknown,
             latency_ms_min, latency_ms_avg, latency_ms_max, count_reports, computed_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(check_id, bucket_start) DO UPDATE SET
                status_up = excluded.status_up,
                status_warn = excluded.status_warn,
                status_down = excluded.status_down,
                status_unknown = excluded.status_unknown,
                latency_ms_min = excluded.latency_ms_min,
                latency_ms_avg = excluded.latency_ms_avg,
                latency_ms_max = excluded.latency_ms_max,
                count_reports = excluded.count_reports,
                computed_at = excluded.computed_at
            "#,
            row.check_id,
            hour_start_str,
            row.status_up,
            row.status_warn,
            row.status_down,
            row.status_unknown,
            row.latency_min,
            row.latency_avg,
            row.latency_max,
            row.count_reports,
            now_str
        )
        .execute(pool)
        .await?;
    }

    Ok(rows.len())
}

/// Compute daily rollups from closed hourly buckets (idempotent).
pub async fn compute_daily_rollups(pool: &SqlitePool, now: DateTime<Utc>) -> Result<usize> {
    let oldest = sqlx::query_scalar::<_, Option<String>>(
        "SELECT MIN(bucket_start) FROM check_rollups_hourly WHERE bucket_start < ?",
    )
    .bind((now - Duration::days(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    .fetch_one(pool)
    .await?;

    let Some(oldest_str) = oldest else {
        return Ok(0);
    };

    let oldest_dt = oldest_str.parse::<DateTime<Utc>>().unwrap_or(now);
    let mut day_start = oldest_dt
        .with_hour(0)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();

    let cutoff = (now - Duration::days(1))
        .with_hour(0)
        .unwrap()
        .with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();

    let mut count = 0;

    while day_start <= cutoff {
        let day_end = day_start + Duration::days(1);
        count += compute_single_daily_bucket(pool, day_start, day_end).await?;
        day_start = day_end;
    }

    Ok(count)
}

/// Aggregate a single daily bucket from hourly rollups (UPSERT, idempotent).
async fn compute_single_daily_bucket(
    pool: &SqlitePool,
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
) -> Result<usize> {
    let rows = sqlx::query!(
        r#"
        SELECT
            check_id,
            CAST(COALESCE(SUM(status_up), 0) as INTEGER) as status_up,
            CAST(COALESCE(SUM(status_warn), 0) as INTEGER) as status_warn,
            CAST(COALESCE(SUM(status_down), 0) as INTEGER) as status_down,
            CAST(COALESCE(SUM(status_unknown), 0) as INTEGER) as status_unknown,
            CAST(COALESCE(MIN(latency_ms_min), 0) as INTEGER) as latency_min,
            COALESCE(AVG(CAST(latency_ms_avg as REAL)), 0.0) as latency_avg,
            CAST(COALESCE(MAX(latency_ms_max), 0) as INTEGER) as latency_max,
            CAST(COALESCE(SUM(count_reports), 0) as INTEGER) as count_reports
        FROM check_rollups_hourly
        WHERE bucket_start >= ? AND bucket_start < ?
        GROUP BY check_id
        "#,
        day_start,
        day_end
    )
    .fetch_all(pool)
    .await?;

    let now_str = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let day_start_str = day_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    for row in rows {
        sqlx::query!(
            r#"
            INSERT INTO check_rollups_daily
            (check_id, bucket_start, status_up, status_warn, status_down, status_unknown,
             latency_ms_min, latency_ms_avg, latency_ms_max, count_reports, computed_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(check_id, bucket_start) DO UPDATE SET
                status_up = excluded.status_up,
                status_warn = excluded.status_warn,
                status_down = excluded.status_down,
                status_unknown = excluded.status_unknown,
                latency_ms_min = excluded.latency_ms_min,
                latency_ms_avg = excluded.latency_ms_avg,
                latency_ms_max = excluded.latency_ms_max,
                count_reports = excluded.count_reports,
                computed_at = excluded.computed_at
            "#,
            row.check_id,
            day_start_str,
            row.status_up,
            row.status_warn,
            row.status_down,
            row.status_unknown,
            row.latency_min,
            row.latency_avg,
            row.latency_max,
            row.count_reports,
            now_str
        )
        .execute(pool)
        .await?;
    }

    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn bucket_5m_starts_at_correct_minute() {
        // 14:37:30 should round down to 14:35:00 (within 35-40 bucket)
        let dt = Utc.with_ymd_and_hms(2026, 11, 8, 14, 37, 30).unwrap();
        let bucket = RollupBucket5m::new(dt);
        assert_eq!(bucket.bucket_start.minute(), 35);
        assert_eq!(bucket.bucket_start.second(), 0);
    }

    #[test]
    fn bucket_5m_is_closed_after_5_minutes() {
        let bucket_start = Utc.with_ymd_and_hms(2026, 11, 8, 14, 35, 0).unwrap();
        let bucket = RollupBucket5m { bucket_start };

        let before = bucket_start + Duration::minutes(4);
        let after = bucket_start + Duration::minutes(5);

        assert!(!bucket.is_closed(before));
        assert!(bucket.is_closed(after));
    }
}
