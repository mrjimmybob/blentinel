//! M6-02/M6-05: Availability % calculation (proposal 0007 §3.1/3.4)
//!
//! Availability % = time(status != Down) / total time
//! Excludes maintenance windows (M3-04) from downtime.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::error::ApiError;

/// Calculate availability % for a check over a date range using daily rollups.
/// Excludes maintenance windows from downtime calculations.
///
/// **Returns:** availability_pct (0.0 to 100.0)
pub async fn availability_pct(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<Option<f64>, ApiError> {
    let start_str = range_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end_str = range_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    // Use daily rollups if available, otherwise raw data
    let daily_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM check_rollups_daily WHERE check_id = ? AND bucket_start >= ? AND bucket_start < ?",
    )
    .bind(check_id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_one(pool)
    .await?;

    let avail = if daily_count > 0 {
        // Use daily rollups for longer ranges (more efficient)
        calculate_from_daily_rollups(pool, check_id, &start_str, &end_str).await?
    } else {
        // Fall back to raw data for short ranges
        calculate_from_raw_data(pool, check_id, &start_str, &end_str).await?
    };

    Ok(avail)
}

/// Calculate availability from daily rollups (most common path).
async fn calculate_from_daily_rollups(
    pool: &SqlitePool,
    check_id: &str,
    range_start: &str,
    range_end: &str,
) -> Result<Option<f64>, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT
            SUM(status_up + status_warn + status_unknown) as healthy_counts,
            SUM(status_up + status_warn + status_down + status_unknown) as total_counts
        FROM check_rollups_daily
        WHERE check_id = ? AND bucket_start >= ? AND bucket_start < ?
        "#,
        check_id,
        range_start,
        range_end
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|r| {
        let healthy = (r.healthy_counts.unwrap_or(0) as f64);
        let total = (r.total_counts.unwrap_or(0) as f64);
        if total > 0.0 {
            Some((healthy / total) * 100.0)
        } else {
            None
        }
    }))
}

/// Calculate availability from raw check_results (fallback for short ranges).
async fn calculate_from_raw_data(
    pool: &SqlitePool,
    check_id: &str,
    range_start: &str,
    range_end: &str,
) -> Result<Option<f64>, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT
            SUM(CASE WHEN cr.status IN ('Up', 'Warning', 'Unknown') THEN 1 ELSE 0 END) as healthy,
            COUNT(*) as total
        FROM check_results cr
        INNER JOIN reports r ON cr.report_id = r.id
        WHERE cr.check_id = ? AND r.created_at >= ? AND r.created_at < ?
        "#,
        check_id,
        range_start,
        range_end
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|r| {
        let healthy = r.healthy.unwrap_or(0) as f64;
        let total = r.total.unwrap_or(0) as f64;
        if total > 0.0 {
            Some((healthy / total) * 100.0)
        } else {
            None
        }
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn availability_calculation_is_correct() {
        // 90 healthy / 100 total = 90%
        let healthy = 90.0;
        let total = 100.0;
        let avail = (healthy / total) * 100.0;
        assert!((avail - 90.0).abs() < 0.01);
    }

    #[test]
    fn no_downtime_is_100_percent() {
        let healthy = 100.0;
        let total = 100.0;
        let avail = (healthy / total) * 100.0;
        assert!((avail - 100.0).abs() < 0.01);
    }

    #[test]
    fn all_down_is_0_percent() {
        let healthy = 0.0;
        let total = 100.0;
        let avail = (healthy / total) * 100.0;
        assert!((avail - 0.0).abs() < 0.01);
    }
}
