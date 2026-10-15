//! One company on the dashboard (or in an archive).

use leptos::prelude::*;

use crate::app::ui::{fmt_ts, severity_border_class};
use crate::model::DashboardCompany;

#[component]
pub(crate) fn CompanyCard(
    company: Memo<DashboardCompany>,
    /// Where the card leads: the live or the archived company page.
    href: String,
) -> impl IntoView {
    view! {
        // Reactive class: updates when the server-computed severity changes.
        <a href=href class=move || format!("company-card {}", severity_border_class(company.get().severity))>
            <div class="card-title">{move || company.get().name}</div>
            <div class="card-stats">
                <span class="cs-label">"Active"</span>
                <span class="cs-value cs-up">{move || company.get().active_probes}</span>
                <span class="cs-label">"Expired"</span>
                <span class=move || format!("cs-value {}", if company.get().expired_probes > 0 { "cs-down" } else { "" })>
                    {move || company.get().expired_probes}
                </span>
                <span class="cs-label">"Devices Up"</span>
                <span class="cs-value cs-up">{move || company.get().devices_up}</span>
                <span class="cs-label">"Devices Down"</span>
                <span class="cs-value cs-down">{move || company.get().devices_down}</span>
                <span class="cs-label">"Warnings"</span>
                <span class=move || format!("cs-value {}", if company.get().devices_warning > 0 { "cs-warning" } else { "" })>
                    {move || company.get().devices_warning}
                </span>
            </div>
            <div class="card-footer">
                <span class=move || format!("status-dot {}", company.get().severity.as_str())></span>
                {"Last report: "}{move || company.get().last_report.as_deref().map(fmt_ts).unwrap_or_else(|| "Never".to_string())}
            </div>
        </a>
    }
}
