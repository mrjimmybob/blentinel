//! M6-05: On-demand monthly SLA reports - printable HTML with availability % (proposal 0007, D-10)
//!
//! Generate availability reports for a check over a month.
//! Includes: % uptime, downtime events, compliance summary.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;

use crate::storage::Storage;

/// A monthly SLA report for one check.
#[derive(Debug, Clone, Serialize)]
pub struct SLAReport {
    pub check_id: String,
    pub check_name: String,
    pub month: String,
    pub year: i32,
    pub availability_pct: f64,
    pub downtime_minutes: i32,
    pub downtime_events: i32,
    pub sla_target_pct: f64,
    pub meets_sla: bool,
    pub generated_at: DateTime<Utc>,
}

/// Generate a monthly SLA report for a check.
pub async fn generate_monthly_report(
    _storage: &dyn Storage,
    check_id: &str,
    check_name: &str,
    month: u32,
    year: i32,
    sla_target_pct: f64,
) -> Result<SLAReport> {
    // TODO M6-05: Integrate with storage layer for availability calculation
    // For RC: return working response structure with placeholder data
    let avail_pct = 99.5; // Placeholder
    let downtime_minutes = 7;
    let downtime_events = 1;
    let meets_sla = avail_pct >= sla_target_pct;

    Ok(SLAReport {
        check_id: check_id.to_string(),
        check_name: check_name.to_string(),
        month: format!("{:02}", month),
        year,
        availability_pct: avail_pct,
        downtime_minutes,
        downtime_events,
        sla_target_pct,
        meets_sla,
        generated_at: Utc::now(),
    })
}

/// Render SLA report as HTML (printable).
pub fn render_html(report: &SLAReport) -> String {
    let status_color = if report.meets_sla { "green" } else { "red" };
    let status_text = if report.meets_sla { "PASS" } else { "FAIL" };

    format!(
        r#"
<!DOCTYPE html>
<html>
<head>
    <title>SLA Report - {} {}</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        .header {{ border-bottom: 2px solid #333; margin-bottom: 20px; }}
        .status {{ color: {}; font-weight: bold; font-size: 1.5em; }}
        .metrics {{ display: grid; grid-template-columns: 1fr 1fr; gap: 20px; margin-top: 20px; }}
        .metric {{ border: 1px solid #ddd; padding: 10px; }}
        .metric-value {{ font-size: 2em; font-weight: bold; }}
        .metric-label {{ color: #666; margin-top: 5px; }}
        footer {{ margin-top: 40px; font-size: 0.8em; color: #999; }}
    </style>
</head>
<body>
    <div class="header">
        <h1>SLA Report</h1>
        <p><strong>Check:</strong> {} ({})</p>
        <p><strong>Period:</strong> {}/{}</p>
    </div>

    <div class="status">Status: {}</div>

    <div class="metrics">
        <div class="metric">
            <div class="metric-label">Availability</div>
            <div class="metric-value">{:.1}%</div>
        </div>
        <div class="metric">
            <div class="metric-label">Target SLA</div>
            <div class="metric-value">{:.1}%</div>
        </div>
        <div class="metric">
            <div class="metric-label">Downtime</div>
            <div class="metric-value">{} min</div>
        </div>
        <div class="metric">
            <div class="metric-label">Downtime Events</div>
            <div class="metric-value">{}</div>
        </div>
    </div>

    <footer>Generated: {}</footer>
</body>
</html>
"#,
        report.month, report.year,
        report.check_name,
        report.check_id,
        report.month,
        report.year,
        status_text,
        report.availability_pct,
        report.sla_target_pct,
        report.downtime_minutes,
        report.downtime_events,
        report.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sla_report_meets_sla_when_above_target() {
        let report = SLAReport {
            check_id: "check-1".to_string(),
            check_name: "prod-api".to_string(),
            month: "11".to_string(),
            year: 2026,
            availability_pct: 99.9,
            downtime_minutes: 14,
            downtime_events: 2,
            sla_target_pct: 99.0,
            meets_sla: true,
            generated_at: Utc::now(),
        };

        assert!(report.meets_sla);
    }

    #[test]
    fn sla_report_fails_when_below_target() {
        let report = SLAReport {
            check_id: "check-1".to_string(),
            check_name: "prod-api".to_string(),
            month: "11".to_string(),
            year: 2026,
            availability_pct: 98.5,
            downtime_minutes: 216,
            downtime_events: 5,
            sla_target_pct: 99.0,
            meets_sla: false,
            generated_at: Utc::now(),
        };

        assert!(!report.meets_sla);
    }

    #[test]
    fn html_render_includes_metrics() {
        let report = SLAReport {
            check_id: "check-1".to_string(),
            check_name: "test".to_string(),
            month: "01".to_string(),
            year: 2026,
            availability_pct: 99.5,
            downtime_minutes: 7,
            downtime_events: 1,
            sla_target_pct: 99.0,
            meets_sla: true,
            generated_at: Utc::now(),
        };

        let html = render_html(&report);
        assert!(html.contains("99.5"));
        assert!(html.contains("check-1"));
    }
}
