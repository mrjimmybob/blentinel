//! M6-05: Availability reports — monthly SLA compliance per check

use leptos::prelude::*;

#[component]
pub fn M6ReportsPage() -> impl IntoView {
    view! {
        <div class="page m6-reports">
            <h1 class="page-title">"Availability Reports"</h1>
            <p class="info-text">"Monthly SLA compliance tracking for checks."</p>

            <section>
                <h2>"What is Availability?"</h2>
                <p>
                    "Availability % = (time NOT down) / total time × 100"
                </p>
                <ul>
                    <li>"Status Up: counted as available"</li>
                    <li>"Status Warning: counted as available (partially functional)"</li>
                    <li>"Status Down: counted as unavailable (critical outage)"</li>
                    <li>"Status Unknown: probe didn't report (counted as unavailable)"</li>
                    <li>"Excludes maintenance windows from downtime"</li>
                </ul>
            </section>

            <section>
                <h2>"SLA Compliance"</h2>
                <p>"Each check has a target availability (e.g., 99.0%, 99.5%, 99.9%)"</p>
                <p><strong>"Report shows:"</strong></p>
                <ul>
                    <li>"Actual availability for the month"</li>
                    <li>"Target SLA %"</li>
                    <li>"Pass/Fail: meets or misses target"</li>
                    <li>"Total downtime: minutes and event count"</li>
                    <li>"Generated timestamp (UTC)"</li>
                </ul>
            </section>

            <section>
                <h2>"Data Source"</h2>
                <p>
                    "Reports are calculated from daily rollups (aggregations of hourly/5-minute data)."
                </p>
                <ul>
                    <li>"One daily rollup per 24-hour UTC boundary"</li>
                    <li>"Retained indefinitely for historical reporting"</li>
                    <li>"Accurate for any month in retained history"</li>
                </ul>
            </section>

            <section>
                <h2>"Report Examples"</h2>
                <p>"SLA Report - November 2026"</p>
                <pre>
"Check: prod-api
Availability: 99.5%
Target: 99.0%
Status: ✓ PASS (exceeded by 0.5%)

Downtime: 7 minutes (1 event)
Generated: 2026-11-09 12:00:00 UTC"
                </pre>

                <p>"SLA Report - September 2026"</p>
                <pre>
"Check: payment-service
Availability: 98.8%
Target: 99.5%
Status: ✗ FAIL (missed by 0.7%)

Downtime: 172 minutes (3 events)
Generated: 2026-11-09 12:00:00 UTC"
                </pre>
            </section>

            <section>
                <h2>"API Endpoint"</h2>
                <p><code>"GET /api/generate-sla-report?check_id=...&month=11&year=2026&sla_target_pct=99.0&format=html"</code></p>
                <p>"Returns HTML or JSON report. Month: 1-12, SLA target as decimal (e.g., 99.0 for 99%)."</p>
            </section>
        </div>
    }
}
