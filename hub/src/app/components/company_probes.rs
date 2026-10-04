//! A company's probes and enrollment codes (M2-01, D-47): the probe list
//! (rename, move to another site, revoke), "Add probe" (a code shown once),
//! and the codes still waiting to be used.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, fmt_ts};
#[cfg(not(feature = "ssr"))]
use crate::model::{CreateCodeRequest, UpdateProbeRequest};
use crate::model::{
    CreatedCode, EnrollmentCodeInfo, Permission, ProbeDownload, ProbeInfo, SiteInfo,
};

#[component]
pub(crate) fn CompanyProbes(company_id: String, sites: Vec<SiteInfo>) -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageCompanies));
    let can_revoke = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let adding = RwSignal::new(false);
    let issued = RwSignal::new(None::<CreatedCode>);
    // Every probe belongs to a site: Add probe waits for the first one.
    let no_sites = sites.is_empty();
    let sites = StoredValue::new(sites);
    let company = StoredValue::new(company_id);

    let probes: LocalResource<Vec<ProbeInfo>> = LocalResource::new(move || {
        refresh.track();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                let company = company.get_value();
                fetch_json::<Vec<ProbeInfo>>("/api/probes")
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| p.company_id == company)
                    .collect()
            }
            #[cfg(feature = "ssr")]
            {
                Vec::new()
            }
        }
    });
    let codes: LocalResource<Vec<EnrollmentCodeInfo>> = LocalResource::new(move || {
        refresh.track();
        let allowed = can_manage.get();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                if !allowed {
                    return Vec::new();
                }
                let url = format!("/api/enrollment-codes?company={}", company.get_value());
                fetch_json::<Vec<EnrollmentCodeInfo>>(&url)
                    .await
                    .unwrap_or_default()
            }
            #[cfg(feature = "ssr")]
            {
                let _ = allowed;
                Vec::new()
            }
        }
    });
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let reload = move || refresh.update(|n| *n += 1);

    // A POST that returns {"success": true}, then a reload of the lists.
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let act = move |url: String, body: String| {
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json(&url, &body).await {
                Ok(()) => reload(),
                Err(e) => error.set(e),
            }
        });
    };

    view! {
        <div class="admin-section">
            <h2>"Probes"</h2>
            <div class="login-error">{error}</div>
            <Show when=move || can_manage.get()>
                <button class="btn btn-primary" disabled=no_sites on:click=move |_| {
                    issued.set(None);
                    adding.set(true);
                }>"Add probe"</button>
                {no_sites.then(|| view! {
                    <p class="info-text add-probe-hint">"Add a site first (under Sites): every probe belongs to one."</p>
                })}
            </Show>
            {move || issued.get().map(|created| view! {
                <div class="notice notice-info enrollment-code-notice">
                    <strong>"Enrollment code (shown only now):"</strong>
                    <code class="enrollment-code">{created.code.clone()}</code>
                    <InstallCommands code=created.code.clone() hub_url=created.hub_url.clone()/>
                    <p>{format!(
                        "Valid until {} UTC for {}. The probe then reports for this company and site, over the hub's pinned certificate.",
                        fmt_ts(&created.info.expires_at),
                        match created.info.max_uses {
                            Some(1) => "one probe".to_string(),
                            Some(n) => format!("{n} probes"),
                            None => "any number of probes (revoke it below when you're done)".to_string(),
                        },
                    )}</p>
                    <button class="btn btn-secondary btn-sm" on:click=move |_| issued.set(None)>"Done"</button>
                </div>
            })}
            {move || -> AnyView {
                let list = probes.get().unwrap_or_default();
                if list.is_empty() {
                    return view! { <p class="info-text">"No probes yet. Use Add probe to enroll one."</p> }.into_any();
                }
                view! {
                    <table class="admin-table probes-table">
                        <thead>
                            <tr>
                                <th>"Name"</th><th>"Site"</th><th>"State"</th><th>"Host"</th>
                                <th>"Version"</th><th>"Last report"</th><th></th>
                            </tr>
                        </thead>
                        <tbody>
                            {list.into_iter().map(|probe| view! {
                                <ProbeRecordRow probe=probe sites=sites.get_value() can_manage=can_manage can_revoke=can_revoke act=act/>
                            }).collect_view()}
                        </tbody>
                    </table>
                }.into_any()
            }}
            {move || {
                let pending = codes.get().unwrap_or_default();
                (!pending.is_empty()).then(|| view! {
                    <h3 class="subsection">"Codes still valid"</h3>
                    <table class="admin-table codes-table">
                        <thead>
                            <tr><th>"Site"</th><th>"Probe name"</th><th>"Used"</th><th>"Expires (UTC)"</th><th>"By"</th><th></th></tr>
                        </thead>
                        <tbody>
                            {pending.into_iter().map(|c| {
                                let url = format!("/api/enrollment-codes/{}/revoke", c.id);
                                view! {
                                    <tr>
                                        <td>{c.site_id}</td>
                                        <td>{c.probe_name.unwrap_or_else(|| "(hostname)".into())}</td>
                                        <td>{match c.max_uses {
                                            Some(max) => format!("{} / {max}", c.uses),
                                            None => format!("{} (no limit)", c.uses),
                                        }}</td>
                                        <td>{fmt_ts(&c.expires_at)}</td>
                                        <td>{c.created_by.unwrap_or_default()}</td>
                                        <td class="row-actions">
                                            <button class="btn btn-secondary btn-sm"
                                                on:click=move |_| act(url.clone(), "{}".into())>"Revoke"</button>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                })
            }}
        </div>
        {move || adding.get().then(|| view! {
            <AddProbeDialog company_id=company.get_value() sites=sites.get_value() adding=adding issued=issued reload=reload error=error/>
        })}
    }
}

/// How to install a probe with `code`: a one-line command per platform the
/// hub has a probe binary for (D-53), or the manual way.
#[component]
fn InstallCommands(code: String, hub_url: String) -> impl IntoView {
    let downloads: LocalResource<Vec<ProbeDownload>> = LocalResource::new(move || async move {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<Vec<ProbeDownload>>("/api/probe-downloads")
                .await
                .unwrap_or_default()
        }
        #[cfg(feature = "ssr")]
        {
            Vec::new()
        }
    });
    let code = StoredValue::new(code);
    let hub_url = StoredValue::new(hub_url);
    view! {
        {move || {
            let list = downloads.get().unwrap_or_default();
            let code = code.get_value();
            if list.is_empty() {
                return view! {
                    <p>"This hub has no probe binaries to offer (see the hub install guide). Copy the probe to the machine, then run, as root or Administrator:"</p>
                    <code class="enrollment-command">{format!("probe install --code {code}")}</code>
                }.into_any();
            }
            view! {
                <p>"On the machine that will be the probe, run the line for its system. It downloads the probe from this hub, checks it, installs it as a service and enrolls it:"</p>
                {list.into_iter().map(|download| {
                    let how = if download.os == "windows" {
                        "PowerShell, as Administrator"
                    } else {
                        "a shell (asks for sudo)"
                    };
                    let command = download.install_command(&hub_url.get_value(), &code);
                    view! {
                        <div class="install-command">
                            <strong>{format!("{} ({how})", download.label())}</strong>
                            <code class="enrollment-command">{command}</code>
                        </div>
                    }
                }).collect_view()}
                <p class="info-text">{format!(
                    "Or copy the probe to the machine yourself and run: probe install --code {code}"
                )}</p>
            }.into_any()
        }}
    }
}

/// One probe: rename or move (in place), revoke.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
fn ProbeRecordRow(
    probe: ProbeInfo,
    sites: Vec<SiteInfo>,
    can_manage: Signal<bool>,
    can_revoke: Signal<bool>,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let editing = RwSignal::new(false);
    let name = RwSignal::new(probe.name.clone());
    let site = RwSignal::new(probe.site_id.clone());
    let id = StoredValue::new(probe.id.clone());
    // Not edited here: kept as it is (the probe's page sets it).
    let interval_secs = probe.interval_secs;
    let revoked = probe.state != "active";
    #[cfg(not(feature = "ssr"))]
    let (on_name, on_site) = (
        move |ev| name.set(input_value(&ev)),
        move |ev| site.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_name, on_site) = (|_| {}, |_| {});
    let save = move || {
        #[cfg(not(feature = "ssr"))]
        {
            let body = UpdateProbeRequest {
                name: name.get_untracked(),
                site_id: site.get_untracked(),
                interval_secs,
            };
            act(
                format!("/api/probes/{}", id.get_value()),
                serde_json::to_string(&body).unwrap_or_default(),
            );
        }
        editing.set(false);
    };
    let host = [probe.hostname.clone(), probe.os.clone(), probe.arch.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    view! {
        <tr class:inactive=revoked>
            {move || if editing.get() {
                view! {
                    <td><input type="text" name="probe_name" prop:value=move || name.get() on:input=on_name/></td>
                    <td>
                        <select name="probe_site" prop:value=move || site.get() on:change=on_site>
                            {sites.iter().map(|s| view! { <option value=s.id.clone()>{s.name.clone()}</option> }).collect_view()}
                        </select>
                    </td>
                }.into_any()
            } else {
                view! {
                    <td><a href=format!("/probes/{}", id.get_value())>{move || name.get()}</a></td>
                    <td>{probe.site_name.clone()}</td>
                }.into_any()
            }}
            <td>{if revoked { "Revoked" } else { "Active" }}</td>
            <td class="probe-host">{host.clone()}</td>
            <td title=probe.update_failed.as_ref().map(|f| format!("{} not installed: {}", f.version, f.error))>
                {probe.probe_version.clone().unwrap_or_default()}
                {probe.update_failed.is_some().then(|| view! { " " <span class="badge badge-warning">"update failed"</span> })}
            </td>
            <td>{probe.last_seen_at.as_deref().map(fmt_ts).unwrap_or_else(|| "never".into())}</td>
            <td class="row-actions">
                <Show when=move || can_manage.get() && !revoked>
                    {move || if editing.get() {
                        view! {
                            <button class="btn btn-primary btn-sm" on:click=move |_| save()>"Save"</button>
                            <button class="btn btn-secondary btn-sm" on:click=move |_| editing.set(false)>"Cancel"</button>
                        }.into_any()
                    } else {
                        view! {
                            <button class="btn btn-secondary btn-sm" on:click=move |_| editing.set(true)>"Edit"</button>
                        }.into_any()
                    }}
                </Show>
                <Show when=move || can_revoke.get() && !revoked>
                    <button class="btn btn-danger btn-sm" on:click=move |_| {
                        act(format!("/api/probes/{}/revoke", id.get_value()), "{}".into())
                    }>"Revoke"</button>
                </Show>
            </td>
        </tr>
    }
}

/// "Add probe": choose site, name, an optional limit and validity; get a code.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
fn AddProbeDialog(
    company_id: String,
    sites: Vec<SiteInfo>,
    adding: RwSignal<bool>,
    issued: RwSignal<Option<CreatedCode>>,
    reload: impl Fn() + Copy + Send + Sync + 'static,
    error: RwSignal<String>,
) -> impl IntoView {
    let site = RwSignal::new(sites.first().map(|s| s.id.clone()).unwrap_or_default());
    let name = RwSignal::new(String::new());
    // Empty: no limit, so one code can serve as many machines as needed.
    let uses = RwSignal::new(String::new());
    let hours = RwSignal::new("24".to_string());
    // Where probes reach the hub: this page's own address is a good first guess.
    let hub_url = RwSignal::new({
        #[cfg(not(feature = "ssr"))]
        {
            crate::app::browser::origin()
        }
        #[cfg(feature = "ssr")]
        {
            String::new()
        }
    });
    let company = StoredValue::new(company_id);
    let no_sites = sites.is_empty();

    let create = move || {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = CreateCodeRequest {
                    company_id: company.get_value(),
                    site_id: site.get_untracked(),
                    probe_name: Some(name.get_untracked()),
                    // Not a number: 0, which the hub refuses with the valid range.
                    max_uses: Some(uses.get_untracked().trim().to_string())
                        .filter(|u| !u.is_empty())
                        .map(|u| u.parse().unwrap_or(0)),
                    valid_hours: hours.get_untracked().trim().parse().unwrap_or(24),
                    hub_url: hub_url.get_untracked(),
                };
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<CreatedCode>("/api/enrollment-codes", &body).await {
                    Ok(created) => {
                        issued.set(Some(created));
                        adding.set(false);
                        reload();
                    }
                    Err(e) => error.set(e),
                }
            }
        });
    };
    let field = move |signal: RwSignal<String>,
                      field_name: &'static str,
                      label: &'static str,
                      kind: &'static str| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| signal.set(input_value(&ev));
        #[cfg(feature = "ssr")]
        let on_input = |_| {};
        view! {
            <div class="form-group">
                <label for=field_name>{label}</label>
                <input id=field_name type=kind name=field_name prop:value=move || signal.get() on:input=on_input/>
            </div>
        }
    };
    #[cfg(not(feature = "ssr"))]
    let on_site = move |ev| site.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_site = |_| {};

    view! {
        <div class="modal-overlay" on:click=move |_| adding.set(false)>
            <div class="modal" on:click=|e| e.stop_propagation()>
                <h3>"Add probe"</h3>
                {no_sites.then(|| view! {
                    <p class="login-error">"Add a site first: every probe belongs to a site."</p>
                })}
                <div class="form-group">
                    <label for="code_site">"Site"</label>
                    <select id="code_site" name="code_site" prop:value=move || site.get() on:change=on_site>
                        {sites.iter().map(|s| view! { <option value=s.id.clone()>{s.name.clone()}</option> }).collect_view()}
                    </select>
                </div>
                {field(name, "code_probe_name", "Probe name (empty: its hostname)", "text")}
                {field(uses, "code_uses", "Limit: how many probes may use it (empty: no limit)", "number")}
                {field(hours, "code_hours", "Valid for (hours)", "number")}
                {field(hub_url, "code_hub_url", "Hub address, as probes reach it", "text")}
                <div class="modal-actions">
                    <button class="btn btn-secondary" on:click=move |_| adding.set(false)>"Cancel"</button>
                    <button class="btn btn-primary" disabled=no_sites on:click=move |_| create()>"Create code"</button>
                </div>
            </div>
        </div>
    }
}
