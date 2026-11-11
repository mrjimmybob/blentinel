//! M6-01: Check timeline — view uptime and availability for a company

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "ssr"))]
use crate::app::http::fetch_json;

use crate::app::ui::any;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UptimeBucket {
    pub timestamp: String,
    pub availability_pct: f64,
    pub check_count: i32,
}

#[component]
pub fn M6RollupsPage() -> impl IntoView {
    let selected_company = RwSignal::new(None::<String>);
    let range = RwSignal::new("24h".to_string());

    let companies: LocalResource<Result<Vec<CompanyInfo>, String>> = LocalResource::new(|| async {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<Vec<CompanyInfo>>("/api/companies").await
        }
        #[cfg(feature = "ssr")]
        {
            Err(String::new())
        }
    });

    let uptime: LocalResource<Result<Vec<UptimeBucket>, String>> = LocalResource::new(move || {
        let company_id = selected_company.get();
        let _range = range.get();
        async move {
            match company_id {
                None => Err("Select a company".to_string()),
                Some(id) => {
                    #[cfg(not(feature = "ssr"))]
                    {
                        fetch_json::<Vec<UptimeBucket>>(&format!("/api/company/{}/uptime", id))
                            .await
                    }
                    #[cfg(feature = "ssr")]
                    {
                        let _ = id;
                        Err(String::new())
                    }
                }
            }
        }
    });

    view! {
        <div class="page m6-rollups">
            <h1 class="page-title">"Check Timeline"</h1>
            <p class="info-text">"Historical uptime and availability by company."</p>

            <Suspense fallback=|| view! { <div>"Loading companies..."</div> }>
                {move || -> AnyView {
                    match companies.get() {
                        Some(Ok(company_list)) => any(view! {
                            <div class="timeline-controls">
                                <label for="company-select">"Company: "</label>
                                <select
                                    id="company-select"
                                    on:change=move |ev| {
                                        let val = crate::app::ui::input_value(&ev);
                                        selected_company.set(if val.is_empty() { None } else { Some(val) });
                                    }
                                >
                                    <option value="">"-- Select a company --"</option>
                                    {company_list.into_iter().map(|c| {
                                        view! { <option value=c.id.clone()>{c.name}</option> }
                                    }).collect_view()}
                                </select>

                                <label for="range-select">"Time range: "</label>
                                <select
                                    id="range-select"
                                    on:change=move |ev| {
                                        range.set(crate::app::ui::input_value(&ev));
                                    }
                                >
                                    <option value="24h">"24 hours"</option>
                                    <option value="7d">"7 days"</option>
                                    <option value="30d">"30 days"</option>
                                    <option value="all">"All time"</option>
                                </select>
                            </div>

                            <Suspense fallback=|| view! { <div>"Loading timeline..."</div> }>
                                {move || -> AnyView {
                                    match uptime.get() {
                                        Some(Ok(buckets)) if !buckets.is_empty() => any(view! {
                                            <div class="uptime-table">
                                                <table>
                                                    <thead>
                                                        <tr>
                                                            <th>"Time"</th>
                                                            <th>"Availability"</th>
                                                            <th>"Checks"</th>
                                                        </tr>
                                                    </thead>
                                                    <tbody>
                                                        {buckets.into_iter().map(|bucket| {
                                                            view! {
                                                                <tr>
                                                                    <td>{bucket.timestamp}</td>
                                                                    <td><strong>{format!("{:.1}%", bucket.availability_pct)}</strong></td>
                                                                    <td>{bucket.check_count}</td>
                                                                </tr>
                                                            }
                                                        }).collect_view()}
                                                    </tbody>
                                                </table>
                                            </div>
                                        }),
                                        Some(Ok(_)) => any(view! {
                                            <div class="empty-state">"No data available for this time range"</div>
                                        }),
                                        Some(Err(e)) => any(view! {
                                            <div class="error">"Error: " {e}</div>
                                        }),
                                        None => any(view! { <div class="loading">"Loading…"</div> }),
                                    }
                                }}
                            </Suspense>
                        }),
                        Some(Err(e)) => any(view! { <div class="error">"Error: " {e}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </Suspense>
        </div>
    }
}
