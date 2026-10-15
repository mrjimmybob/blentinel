//! Companies: every company the user can see, archived ones on request, and
//! "Add company" for Admins (D-44, D-47).

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for};
use crate::app::ui::Me;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
#[cfg(not(feature = "ssr"))]
use crate::model::{CompanyDetail, CreateCompanyRequest, slugify};
use crate::model::{CompanyInfo, Permission};

#[component]
pub(crate) fn CompaniesPage() -> impl IntoView {
    let me = Me::current();
    let show_archived = RwSignal::new(false);
    let refresh = RwSignal::new(0u32);
    let companies: LocalResource<Result<Vec<CompanyInfo>, String>> =
        LocalResource::new(move || {
            refresh.track();
            async move {
                #[cfg(not(feature = "ssr"))]
                {
                    fetch_json::<Vec<CompanyInfo>>("/api/companies").await
                }
                #[cfg(feature = "ssr")]
                {
                    Ok(vec![])
                }
            }
        });

    // Add company: a name, and an ID suggested from it.
    let new_name = RwSignal::new(String::new());
    let new_id = RwSignal::new(String::new());
    let add_error = RwSignal::new(String::new());
    #[cfg(not(feature = "ssr"))]
    let (on_name, on_id) = (
        move |ev| {
            let value = input_value(&ev);
            new_id.set(slugify(&value));
            new_name.set(value);
        },
        move |ev| new_id.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_name, on_id) = (|_| {}, |_| {});
    let add = move || {
        add_error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = CreateCompanyRequest {
                    id: new_id.get_untracked(),
                    name: new_name.get_untracked(),
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<CompanyDetail>("/api/companies", &body).await {
                    Ok(created) => {
                        crate::app::browser::navigate(&format!("/companies/{}", created.company.id))
                    }
                    Err(e) => add_error.set(e),
                }
            }
            #[cfg(feature = "ssr")]
            refresh.update(|n| *n += 1);
        });
    };

    view! {
        <h1 class="page-title">"Companies"</h1>
        <p class="info-text">
            "A company is a customer you monitor (or your own company). Open one to add its sites and probes."
        </p>
        <Show when=move || me.can(Permission::ManageData)>
            <form class="form-card form-inline add-company" on:submit=move |ev| {
                ev.prevent_default();
                add();
            }>
                <input type="text" name="new_company_name" placeholder="New company name"
                    prop:value=move || new_name.get() on:input=on_name/>
                <input type="text" name="new_company_id" placeholder="ID"
                    prop:value=move || new_id.get() on:input=on_id/>
                <button type="submit" class="btn btn-primary">"Add company"</button>
            </form>
            <div class="login-error">{add_error}</div>
        </Show>
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
