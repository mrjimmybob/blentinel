//! M6-02: Check timelines - historical metrics and availability (proposal 0007, D-10)
//!
//! Render a check's status history over a date range using tier-selected queries.
//! Includes availability %, downtime events, incident summary.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::service::{availability, tier_select};

/// A point in a check's timeline (aggregated or raw result).
#[derive(Debug, Clone, Serialize)]
pub struct TimelinePoint {
    pub timestamp: DateTime<Utc>,
    pub status_up: i32,
    pub status_warn: i32,
    pub status_down: i32,
    pub status_unknown: i32,
    pub latency_ms_min: Option<i32>,
    pub latency_ms_avg: Option<f64>,
    pub latency_ms_max: Option<i32>,
}

/// Summary of a check's timeline over a date range.
#[derive(Debug, Clone, Serialize)]
pub struct CheckTimeline {
    pub check_id: String,
    pub range_start: DateTime<Utc>,
    pub range_end: DateTime<Utc>,
    pub availability_pct: Option<f64>,
    pub downtime_minutes: i32,
    pub points: Vec<TimelinePoint>,
}

/// Query timeline data for a check over a date range.
/// Tier-selected: uses raw/5m/hourly/daily based on range duration.
pub async fn check_timeline(
    pool: &SqlitePool,
    check_id: &str,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> Result<CheckTimeline> {
    // Get availability % for the range
    let avail = availability::availability_pct(pool, check_id, range_start, range_end).await?;

    // Fetch tier-selected data
    let rows = tier_select::query_with_fallback(pool, check_id, range_start, range_end).await?;

    // Convert to timeline points
    let points: Vec<TimelinePoint> = rows
        .iter()
        .map(|r| TimelinePoint {
            timestamp: r.bucket_start,
            status_up: r.status_up,
            status_warn: r.status_warn,
            status_down: r.status_down,
            status_unknown: r.status_unknown,
            latency_ms_min: r.latency_ms_min,
            latency_ms_avg: r.latency_ms_avg,
            latency_ms_max: r.latency_ms_max,
        })
        .collect();

    // Calculate downtime minutes (simplified: count Down status buckets)
    let downtime_minutes: i32 = points
        .iter()
        .map(|p| {
            if p.status_down > 0 {
                let bucket_duration = if points.len() == 1 { 1 } else { 5 }; // Rough estimate
                p.status_down * bucket_duration
            } else {
                0
            }
        })
        .sum();

    Ok(CheckTimeline {
        check_id: check_id.to_string(),
        range_start,
        range_end,
        availability_pct: avail,
        downtime_minutes,
        points,
    })
}

/// Summarize check status: ratio of Up / total.
pub fn check_status_summary(timeline: &CheckTimeline) -> String {
    if timeline.points.is_empty() {
        return "No data".to_string();
    }

    let total_up: i32 = timeline.points.iter().map(|p| p.status_up).sum();
    let total: i32 = timeline.points.iter().map(|p| {
        p.status_up + p.status_warn + p.status_down + p.status_unknown
    }).sum();

    if total == 0 {
        "Unknown".to_string()
    } else {
        let pct = (total_up as f64 / total as f64) * 100.0;
        format!("{:.1}% up", pct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_summary_formats_correctly() {
        let timeline = CheckTimeline {
            check_id: "test".to_string(),
            range_start: Utc::now(),
            range_end: Utc::now(),
            availability_pct: Some(95.0),
            downtime_minutes: 30,
            points: vec![
                TimelinePoint {
                    timestamp: Utc::now(),
                    status_up: 90,
                    status_warn: 5,
                    status_down: 5,
                    status_unknown: 0,
                    latency_ms_min: Some(10),
                    latency_ms_avg: Some(50.0),
                    latency_ms_max: Some(200),
                },
            ],
        };

        let summary = check_status_summary(&timeline);
        assert!(summary.contains("up"));
    }

    #[test]
    fn status_summary_handles_empty_timeline() {
        let timeline = CheckTimeline {
            check_id: "test".to_string(),
            range_start: Utc::now(),
            range_end: Utc::now(),
            availability_pct: None,
            downtime_minutes: 0,
            points: vec![],
        };

        let summary = check_status_summary(&timeline);
        assert_eq!(summary, "No data");
    }
}
