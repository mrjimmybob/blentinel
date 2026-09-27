//! Read-only view of an archive (same shapes as the live dashboard).

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::fetch_json;
use crate::app::ui::any;
use crate::model::DashboardCompany;

#[component]
pub(crate) fn ArchiveViewerPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let archive_id = move || {
        params
            .read()
            .get("archive_id")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
    };

    let companies: LocalResource<Result<Vec<DashboardCompany>, String>> =
        LocalResource::new(move || async move {
            let id = archive_id();
            if id == 0 {
                return Err("Invalid archive ID".to_string());
            }
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<DashboardCompany>>(&format!("/api/archive/{}/companies", id)).await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    view! {
        <div class="archive-banner">
            "📂 HISTORICAL VIEW — READ ONLY ARCHIVE"
        </div>

        <Suspense fallback=|| view! { <div class="loading">"Loading historical data…"</div> }>
            {move || -> AnyView {
                match companies.get() {
                    Some(Ok(list)) if !list.is_empty() => {
                        any(view! {
                            <div class="company-grid">
                                <For
                                    each=move || list.clone()
                                    key=|c: &DashboardCompany| c.company_id.clone()
                                    children=move |company: DashboardCompany| {
                                        let last_report_display = company.last_report
                                            .as_deref()
                                            .unwrap_or("No reports yet")
                                            .to_string();

                                        view! {
                                            <div class="company-card">
                                                <div class="company-header">
                                                    <h3 class="company-name">{company.company_id.clone()}</h3>
                                                    <span class="last-report">{last_report_display}</span>
                                                </div>
                                                <div class="company-stats">
                                                    <div class="stat">
                                                        <span class="stat-label">"Probes"</span>
                                                        <span class="stat-value">{company.total_probes}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Active"</span>
                                                        <span class="stat-value stat-up">{company.active_probes}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Expired"</span>
                                                        <span class="stat-value stat-down">{company.expired_probes}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Devices Up"</span>
                                                        <span class="stat-value stat-up">{company.devices_up}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Devices Down"</span>
                                                        <span class="stat-value stat-down">{company.devices_down}</span>
                                                    </div>
                                                </div>
                                                <div class="company-actions">
                                                    <span class="archive-note">"Historical data only - no actions available"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                />
                            </div>
                        })
                    }
                    Some(Ok(_)) => any(view! {
                        <div class="empty-state">"No companies in this archive."</div>
                    }),
                    Some(Err(e)) => any(view! {
                        <div class="error">
                            <p>"Failed to load archive data"</p>
                            <p class="error-detail">{e}</p>
                        </div>
                    }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
        </Suspense>
    }
}
