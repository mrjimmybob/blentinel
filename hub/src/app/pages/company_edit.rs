//! One company's record: name, contacts, notes, its sites, archiving (D-44).
//! Editable with `ManageCompanies`; read-only otherwise.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for};
use crate::app::ui::Me;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
#[cfg(not(feature = "ssr"))]
use crate::model::{ArchiveCompanyRequest, UpdateCompanyRequest, UpdateSiteRequest};
use crate::model::{CompanyDetail, Permission, SiteInfo};

#[component]
pub(crate) fn CompanyEditPage() -> impl IntoView {
    let params = use_params_map();
    let id = Signal::derive(move || params.get().get("company_id").unwrap_or_default());
    let me = Me::current();
    let can_edit = Signal::derive(move || me.can(Permission::ManageCompanies));
    // Archiving hides the company from everyone: Admins only (D-45).
    let can_archive = Signal::derive(move || me.can(Permission::ManageData));

    let detail = RwSignal::new(None::<CompanyDetail>);
    let error = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());

    Effect::new(move |_| {
        let _id = id.get();
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match fetch_json::<CompanyDetail>(&format!("/api/companies/{_id}")).await {
                Ok(d) => detail.set(Some(d)),
                Err(e) => error.set(e),
            }
        });
    });

    // Every edit answers with the whole record, which replaces the page's copy.
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let send = move |url: String, body: String, done: &'static str| {
        error.set(String::new());
        message.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_for::<CompanyDetail>(&url, &body).await {
                Ok(d) => {
                    detail.set(Some(d));
                    message.set(done.to_string());
                }
                Err(e) => error.set(e),
            }
        });
    };

    view! {
        <div class="breadcrumb">
            <a href="/companies">"Companies"</a>
            <span class="sep">"›"</span>
            {move || detail.get().map(|d| d.company.name).unwrap_or_else(|| id.get())}
        </div>
        <div class="login-error">{error}</div>
        <div class="form-message">{message}</div>
        {move || detail.get().map(|d| {
            let company_id = d.company.id.clone();
            let archived = d.company.archived;
            view! {
                <h1 class="page-title">
                    {d.company.name.clone()}
                    {d.company.is_new.then(|| view! { <span class="badge badge-new">"New"</span> })}
                    {archived.then(|| view! { <span class="badge">"Archived"</span> })}
                </h1>
                <p class="info-text">
                    "ID "<code>{company_id.clone()}</code>" (the company_id its probes send; it can't change). "
                    <a href=format!("/company/{company_id}")>"Open its dashboard"</a>
                </p>
                <CompanyForm detail=d.clone() can_edit=can_edit send=send/>
                <div class="admin-section">
                    <h2>"Sites"</h2>
                    <p class="info-text">"Physical locations, as the probes report them. A new one appears with its first report."</p>
                    <table class="admin-table sites-table">
                        <thead>
                            <tr><th>"Name"</th><th>"ID"</th><th>"Address"</th><th>"Notes"</th><th></th></tr>
                        </thead>
                        <tbody>
                            {d.sites.iter().cloned().map(|site| view! {
                                <SiteRow company_id=company_id.clone() site=site can_edit=can_edit send=send/>
                            }).collect_view()}
                        </tbody>
                    </table>
                </div>
                <Show when=move || can_archive.get()>
                    <div class="admin-section">
                        <h2>{if archived { "Unarchive" } else { "Archive" }}</h2>
                        <p class="info-text">
                            {if archived {
                                "Show this company on the dashboard again."
                            } else {
                                "Hide this company from the dashboard, e.g. a typo in a probe's config or a former client. Its probes can still report and alert; nothing is deleted."
                            }}
                        </p>
                        <button class="btn btn-secondary" on:click={
                            let url = format!("/api/companies/{}/archive", d.company.id);
                            move |_| {
                                #[cfg(not(feature = "ssr"))]
                                {
                                    let body = serde_json::to_string(&ArchiveCompanyRequest { archived: !archived }).unwrap_or_default();
                                    send(url.clone(), body, if archived { "Unarchived." } else { "Archived." });
                                }
                                #[cfg(feature = "ssr")]
                                let _ = &url;
                            }
                        }>
                            {if archived { "Unarchive" } else { "Archive" }}
                        </button>
                    </div>
                </Show>
            }
        })}
    }
}

/// Name, contacts and notes.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // `send` and `url` are browser-only
fn CompanyForm(
    detail: CompanyDetail,
    can_edit: Signal<bool>,
    send: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let c = detail.company;
    let url = format!("/api/companies/{}", c.id);
    let name = RwSignal::new(c.name);
    let contact_name = RwSignal::new(c.contact_name.unwrap_or_default());
    let contact_email = RwSignal::new(c.contact_email.unwrap_or_default());
    let contact_phone = RwSignal::new(c.contact_phone.unwrap_or_default());
    let notes = RwSignal::new(c.notes.unwrap_or_default());

    let field = move |signal: RwSignal<String>, field_name: &'static str, label: &'static str| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| signal.set(input_value(&ev));
        #[cfg(feature = "ssr")]
        let on_input = |_| {};
        view! {
            <div class="form-group">
                <label for=field_name>{label}</label>
                <input id=field_name type="text" name=field_name
                    prop:value=move || signal.get() on:input=on_input
                    prop:readOnly=move || !can_edit.get()/>
            </div>
        }
    };
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let save = move || {
        #[cfg(not(feature = "ssr"))]
        {
            let some = |s: RwSignal<String>| Some(s.get_untracked());
            let body = UpdateCompanyRequest {
                name: name.get_untracked(),
                notes: some(notes),
                contact_name: some(contact_name),
                contact_email: some(contact_email),
                contact_phone: some(contact_phone),
            };
            send(
                url.clone(),
                serde_json::to_string(&body).unwrap_or_default(),
                "Saved.",
            );
        }
    };

    view! {
        <div class="admin-section">
            <h2>"Details"</h2>
            <form class="form-card" on:submit=move |ev| {
                ev.prevent_default();
                save();
            }>
                {field(name, "company_name", "Name")}
                {field(contact_name, "contact_name", "Contact")}
                {field(contact_email, "contact_email", "Contact e-mail")}
                {field(contact_phone, "contact_phone", "Contact phone")}
                {field(notes, "company_notes", "Notes")}
                <Show when=move || can_edit.get()>
                    <button type="submit" class="btn btn-primary">"Save"</button>
                </Show>
            </form>
        </div>
    }
}

/// One site, editable in place.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // `send` and `url` are browser-only
fn SiteRow(
    company_id: String,
    site: SiteInfo,
    can_edit: Signal<bool>,
    send: impl Fn(String, String, &'static str) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    // In a StoredValue so `save` is Copy and can sit inside the reactive row.
    let url = StoredValue::new(format!("/api/companies/{company_id}/sites/{}", site.id));
    let editing = RwSignal::new(false);
    let name = RwSignal::new(site.name.clone());
    let address = RwSignal::new(site.address.clone().unwrap_or_default());
    let notes = RwSignal::new(site.notes.clone().unwrap_or_default());
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let save = move || {
        #[cfg(not(feature = "ssr"))]
        {
            let body = UpdateSiteRequest {
                name: name.get_untracked(),
                address: Some(address.get_untracked()),
                notes: Some(notes.get_untracked()),
            };
            send(
                url.get_value(),
                serde_json::to_string(&body).unwrap_or_default(),
                "Site saved.",
            );
        }
    };
    let input = move |signal: RwSignal<String>, field_name: &'static str| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| signal.set(input_value(&ev));
        #[cfg(feature = "ssr")]
        let on_input = |_| {};
        view! {
            <input type="text" name=field_name prop:value=move || signal.get() on:input=on_input/>
        }
    };

    view! {
        <tr>
            {move || if editing.get() {
                view! {
                    <td>{input(name, "site_name")}</td>
                    <td><code>{site.id.clone()}</code></td>
                    <td>{input(address, "site_address")}</td>
                    <td>{input(notes, "site_notes")}</td>
                    <td class="row-actions">
                        <button class="btn btn-primary btn-sm" on:click=move |_| {
                            save();
                            editing.set(false);
                        }>"Save"</button>
                        <button class="btn btn-secondary btn-sm" on:click=move |_| editing.set(false)>"Cancel"</button>
                    </td>
                }.into_any()
            } else {
                view! {
                    <td>
                        {move || name.get()}
                        {site.is_new.then(|| view! { " " <span class="badge badge-new">"New"</span> })}
                    </td>
                    <td><code>{site.id.clone()}</code></td>
                    <td>{move || address.get()}</td>
                    <td>{move || notes.get()}</td>
                    <td class="row-actions">
                        <Show when=move || can_edit.get()>
                            <button class="btn btn-secondary btn-sm" on:click=move |_| editing.set(true)>"Edit"</button>
                        </Show>
                    </td>
                }.into_any()
            }}
        </tr>
    }
}
