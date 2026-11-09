//! M6-03: Problems — current incidents affecting checks

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::fetch_json;

use crate::app::ui::{any, status_class};
use crate::model::IncidentInfo;

#[component]
pub fn M6ProblemsPage() -> impl IntoView {
    let incidents: LocalResource<Result<Vec<IncidentInfo>, String>> = LocalResource::new(|| async {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<Vec<IncidentInfo>>("/api/incidents?state=open").await
        }
        #[cfg(feature = "ssr")]
        {
            Err(String::new())
        }
    });

    view! {
        <div class="page m6-problems">
            <h1 class="page-title">"Problems"</h1>
            <p class="info-text">"Current incidents: active problems affecting checks and probes."</p>

            {move || -> AnyView {
                match incidents.get() {
                    Some(Ok(list)) if list.is_empty() => any(view! {
                        <div class="empty-state">"No active problems — all checks are healthy."</div>
                    }),
                    Some(Ok(list)) => any(view! {
                        <table class="admin-table incidents-table">
                            <thead>
                                <tr>
                                    <th>"Severity"</th>
                                    <th>"Check"</th>
                                    <th>"Company"</th>
                                    <th>"Opened"</th>
                                    <th>"Duration"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {list.into_iter().map(|i| {
                                    let status_str = i.status.as_str().to_uppercase();
                                    view! {
                                        <tr class=format!("incident-{}", i.status.as_str())>
                                            <td><span class=format!("incident-badge {}", status_class(i.status))>{status_str}</span></td>
                                            <td>{i.check_name.clone().unwrap_or_else(|| "Probe offline".to_string())}</td>
                                            <td>{i.company_name}</td>
                                            <td>{i.opened_at.clone()}</td>
                                            <td>{i.duration.clone()}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }),
                    Some(Err(e)) => any(view! { <div class="error">"Error: " {e}</div> }),
                    None => any(view! { <div class="loading">"Loading incidents…"</div> }),
                }
            }}
        </div>
    }
}
