//! M6-03: Problems - incident list with detection and closure timeline

use leptos::prelude::*;
use crate::app::ui::Me;

#[component]
pub fn M6ProblemsPage() -> impl IntoView {
    let me = Me::current();

    view! {
        <div class="page m6-problems">
            <h1>"Problems"</h1>
            <p class="subtitle">"M6-03: Incident detection and tracking"</p>

            <div class="info-box">
                <h2>"Incident Detection"</h2>
                <p>"Automatic detection of status changes from check results"</p>
                <pre>"Incident created when:"
"  - Check status transitions to Down"
"  - Check status transitions to Warning"
"  "
"Status persists in incident until:"
"  - Manually closed by operator"
"  - Auto-resolved based on threshold"
"  - All-clear time has passed"</pre>
            </div>

            <div class="info-box">
                <h2>"Incident Timeline"</h2>
                <pre>"opened_at: When status changed to unhealthy"
"closed_at: When operator closed or auto-resolved"
"duration_minutes: (closed_at - opened_at) / 60"
"  "
"Incident statuses:"
"  - Down: critical outage"
"  - Warning: degraded but operational"
"  - Unknown: check communication failed"</pre>
            </div>

            <div class="info-box">
                <h2>"Blocking Checks"</h2>
                <p>"Check blocking: an incident on this check prevents the following checks from running"</p>
                <pre>"Used for:"
"  - Avoid redundant checks when parent is down"
"  - API servers before application servers"
"  - Network before all host checks"
"  "
"Blocking relationship:"
"  - Prevents false cascade of incidents"
"  - Reduces noise in alert routing"</pre>
            </div>

            <div class="info-box">
                <h2>"Data Structures"</h2>
                <pre>"Incident:"
"  id: i64"
"  check_id: String"
"  status: String (Down|Warning|Unknown)"
"  opened_at: DateTime<Utc>"
"  closed_at: Option<DateTime<Utc>>"
"  duration_minutes: Option<i32>"</pre>
            </div>
        </div>
    }
}
