//! Read-only view of an archive (same shapes as the live dashboard).

use leptos::prelude::*;

use crate::app::components::CompanyCard;
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
                        let id = archive_id();
                        any(view! {
                            <div class="company-grid">
                                <For
                                    each=move || list.clone()
                                    key=|c: &DashboardCompany| c.company_id.clone()
                                    children=move |company: DashboardCompany| {
                                        let href = format!("/archive/{id}/company/{}", company.company_id);
                                        // An archive never changes: a constant memo.
                                        let company = Memo::new(move |_| company.clone());
                                        view! { <CompanyCard company=company href=href/> }
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
