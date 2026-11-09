//! M6-01: Check timeline — historical status aggregations by time bucket

use leptos::prelude::*;
use crate::app::ui::any;

#[component]
pub fn M6RollupsPage() -> impl IntoView {
    view! {
        <div class="page m6-rollups">
            <h1 class="page-title">"Check Timeline"</h1>
            <p class="info-text">
                "Historical aggregations of check results in 5-minute, hourly, and daily buckets for efficient availability calculations."
            </p>

            <section>
                <h2>"Rollup Data"</h2>
                <p>"For each check and time bucket, aggregated statistics:"</p>
                <ul>
                    <li>"Status counts: how many results were Up, Warning, Down, Unknown"</li>
                    <li>"Latency extremes: minimum, average, and maximum milliseconds"</li>
                    <li>"Report count: number of check executions in the bucket"</li>
                </ul>
            </section>

            <section>
                <h2>"Bucket Hierarchy"</h2>
                <table>
                    <tr>
                        <td><strong>"5-minute"</strong></td>
                        <td>"Finest granularity; raw-level detail"</td>
                        <td>"Retained ~30 days by default"</td>
                    </tr>
                    <tr>
                        <td><strong>"Hourly"</strong></td>
                        <td>"Aggregated from 5-minute buckets (12 per hour)"</td>
                        <td>"Retained ~6 months by default"</td>
                    </tr>
                    <tr>
                        <td><strong>"Daily"</strong></td>
                        <td>"Aggregated from hourly buckets (24 per day)"</td>
                        <td>"Retained indefinitely; used for SLA reports"</td>
                    </tr>
                </table>
            </section>

            <section>
                <h2>"Background Computation"</h2>
                <p>"Rollups are computed hourly by an idempotent background task:"</p>
                <ul>
                    <li>"5-minute: computed for each closed bucket (after 5 min elapsed)"</li>
                    <li>"Hourly: aggregates completed 5-minute buckets"</li>
                    <li>"Daily: aggregates completed hourly buckets"</li>
                    <li>"Re-running is safe and produces identical results"</li>
                </ul>
            </section>

            <section>
                <h2>"Use Cases"</h2>
                <ul>
                    <li>"Availability % calculation: efficient range queries"</li>
                    <li>"SLA report generation: monthly compliance"</li>
                    <li>"Status trends: visualize check health over time"</li>
                    <li>"Capacity planning: latency trends"</li>
                </ul>
            </section>
        </div>
    }
}
