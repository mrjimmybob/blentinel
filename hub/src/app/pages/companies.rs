//! Companies: every company the hub knows, including ones a report just
//! registered ("New") and archived ones (D-44).

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::fetch_json;
use crate::model::CompanyInfo;

#[component]
pub(crate) fn CompaniesPage() -> impl IntoView {
    let show_archived = RwSignal::new(false);
    let companies: LocalResource<Result<Vec<CompanyInfo>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<CompanyInfo>>("/api/companies").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    view! {
        <h1 class="page-title">"Companies"</h1>
        <p class="info-text">
            "A company appears here when one of its probes first reports. "
            "Give it a name and contacts, name its sites, or archive it (e.g. a typo in a probe's config)."
        </p>
        <label class="filter-toggle">
            <input type="checkbox" name="show_archived"
                prop:checked=move || show_archived.get()
                on:change=move |_| show_archived.update(|v| *v = !*v)/>
            " Show archived"
        </label>
        {move || -> AnyView {
            match companies.get() {
                Some(Ok(list)) => {
                    let visible: Vec<CompanyInfo> = list
                        .into_iter()
                        .filter(|c| show_archived.get() || !c.archived)
                        .collect();
                    if visible.is_empty() {
                        return view! { <div class="empty-state">"No companies yet."</div> }.into_any();
                    }
                    view! {
                        <table class="admin-table companies-table">
                            <thead>
                                <tr>
                                    <th>"Name"</th><th>"ID"</th><th>"Sites"</th>
                                    <th>"Probes"</th><th>"Status"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {visible.into_iter().map(|c| {
                                    let href = format!("/companies/{}", c.id);
                                    view! {
                                        <tr class:inactive=c.archived>
                                            <td><a href=href>{c.name}</a></td>
                                            <td><code>{c.id}</code></td>
                                            <td>{c.site_count}</td>
                                            <td>{c.probe_count}</td>
                                            <td>
                                                {c.is_new.then(|| view! { <span class="badge badge-new">"New"</span> })}
                                                {c.archived.then(|| view! { <span class="badge">"Archived"</span> })}
                                            </td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                }
                Some(Err(e)) => view! { <div class="error">{e}</div> }.into_any(),
                None => view! { <div class="loading">"Loading…"</div> }.into_any(),
            }
        }}
    }
}
