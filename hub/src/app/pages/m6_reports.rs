//! M6-05: SLA reports - monthly availability reports with SLA compliance

use leptos::prelude::*;
use crate::app::ui::Me;

#[component]
pub fn M6ReportsPage() -> impl IntoView {
    let me = Me::current();

    view! {
        <div class="page m6-reports">
            <h1>"SLA Reports"</h1>
            <p class="subtitle">"M6-05: Monthly availability and compliance tracking"</p>

            <div class="info-box">
                <h2>"Availability Calculation"</h2>
                <p>"Availability % = time(status != Down) / total time"</p>
                <pre>"- Excludes maintenance windows (M3-04)"
"- Uses daily rollups for efficiency"
"- Falls back to raw data for short ranges"
"  "
"Formula:"
"  healthy = sum(status_up + status_warn + status_unknown)"
"  total = sum(all statuses)"
"  availability = (healthy / total) * 100.0"</pre>
            </div>

            <div class="info-box">
                <h2>"SLA Compliance"</h2>
                <pre>"Each check has configurable SLA target (e.g., 99.0%)"
"  "
"Report shows:"
"  ✓ PASS: availability >= target"
"  ✗ FAIL: availability < target"
"  "
"Metrics:"
"  - availability_pct: Current month's %"
"  - sla_target_pct: Expected % (e.g., 99.0)"
"  - downtime_minutes: Total outage time"
"  - downtime_events: Number of outages"</pre>
            </div>

            <div class="info-box">
                <h2>"Report Generation"</h2>
                <pre>"Monthly reports generated:"
"  - Check name and ID"
"  - Time period (month/year)"
"  - Availability calculation"
"  - Compliance summary"
"  - Generated timestamp"
"  "
"Output format:"
"  - HTML (printable, for email)"
"  - JSON (for API consumers)"</pre>
            </div>

            <div class="info-box">
                <h2>"Use Cases"</h2>
                <pre>"- Compliance reporting (SOC 2, ISO 27001)"
"- Stakeholder communication"
"- SLA tracking over time"
"- Cost/credit calculation if SLA breached"
"- Trend analysis (improving/degrading availability)"</pre>
            </div>

            <div class="info-box">
                <h2>"Example Report"</h2>
                <pre>"Check: prod-api (check-123)"
"Period: November 2026"
"  "
"Availability: 99.5%"
"Target: 99.0%"
"Status: ✓ PASS (0.5% better than target)"
"  "
"Downtime Summary:"
"  - 7 minutes of Down status"
"  - 1 outage event"
"  - Generated: 2026-11-09 12:00:00 UTC"</pre>
            </div>
        </div>
    }
}
