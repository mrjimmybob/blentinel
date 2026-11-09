//! M6-01: Rollups timeline - historical data aggregations by time bucket

use leptos::prelude::*;
use crate::app::ui::Me;

#[component]
pub fn M6RollupsPage() -> impl IntoView {
    let me = Me::current();

    view! {
        <div class="page m6-rollups">
            <h1>"Rollups Timeline"</h1>
            <p class="subtitle">"M6-01: Historical check result aggregations"</p>

            <div class="info-box">
                <h2>"5-Minute Rollups"</h2>
                <p>"Aggregated check results in 5-minute windows"</p>
                <pre>"- Counts by status (Up, Warning, Down, Unknown)"
"- Min/avg/max latency"
"- Idempotent computation"</pre>
            </div>

            <div class="info-box">
                <h2>"Hourly Rollups"</h2>
                <p>"Aggregated from 5-minute buckets"</p>
                <pre>"- 12 buckets per hour"
"- Status rollup summation"
"- Latency extremes tracked"</pre>
            </div>

            <div class="info-box">
                <h2>"Daily Rollups"</h2>
                <p>"Aggregated from hourly buckets"</p>
                <pre>"- 24 buckets per day"
"- Used for availability calculations"
"- Retention: preserved indefinitely"</pre>
            </div>

            <div class="info-box">
                <h2>"Technical Details"</h2>
                <pre>"Status layout (4-tuple per bucket):"
"  status_up, status_warn, status_down, status_unknown"
"  "
"Latency layout (3-value per bucket):"
"  latency_min, latency_avg, latency_max"
"  "
"Computation: Hourly background task"
"Retention: Configured per tier"
"Safety: Only closed buckets (idempotent re-run safe)"</pre>
            </div>
        </div>
    }
}
