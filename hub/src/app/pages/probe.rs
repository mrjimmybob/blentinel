//! One probe: its checks, their latest results, its report interval and
//! whether it runs what the hub has for it (M2-05, proposal 0003 §4.5, D-49),
//! "Run checks now" and its latest commands (M2-06, D-56), and how the
//! probe itself is doing (M4-02, D-73), what its machine is made of, the
//! checks that suggests, and templates (M5-12, D-85).
//! Everyone who sees the probe sees this page; `ManageCompanies` can change
//! it. Changes reach the probe with the answer to its next report.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::app::components::{
    CheckDialog, MachineSection, ProbeCommands, SuggestedChecks, TemplatesDialog, kind_label,
    switched,
};
use crate::app::pages::fleet::clock_text;
use crate::app::ui::{Me, fmt_ts, fmt_utc, status_class, status_label};
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{fetch_json, post_json},
    ui::input_value,
};
use crate::model::{
    CheckInfo, CommandInfo, ConfigSync, MachineInfo, Permission, ProbeChecks, ProbeDevice,
    ProbeInfo,
};

/// How often the page reloads (the probe's state changes as it reports).
#[cfg(not(feature = "ssr"))]
const REFRESH_MS: i32 = 15_000;

/// The sync badge: class and text.
fn sync_badge(state: &ProbeChecks) -> (&'static str, String) {
    match state.sync {
        ConfigSync::InSync => ("sync-badge in-sync", "In sync".to_string()),
        ConfigSync::Pending => (
            "sync-badge pending",
            "Pending: the probe gets the changes with its next report".to_string(),
        ),
        ConfigSync::Rejected => (
            "sync-badge rejected",
            format!(
                "Rejected by the probe: {}",
                state.config_error.clone().unwrap_or_default()
            ),
        ),
    }
}

#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn ProbePage() -> impl IntoView {
    let params = use_params_map();
    let probe_id = Signal::derive(move || params.get().get("probe_id").unwrap_or_default());
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageCompanies));

    let probes = RwSignal::new(Vec::<ProbeInfo>::new());
    let state = RwSignal::new(None::<ProbeChecks>);
    let devices = RwSignal::new(Vec::<ProbeDevice>::new());
    let commands = RwSignal::new(Vec::<CommandInfo>::new());
    let machine = RwSignal::new(None::<MachineInfo>);
    let error = RwSignal::new(String::new());
    let loaded = RwSignal::new(false);

    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let load = move || {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let id = probe_id.get_untracked();
                match fetch_json::<ProbeChecks>(&format!("/api/probes/{id}/checks")).await {
                    Ok(s) => state.set(Some(s)),
                    Err(e) => error.set(e),
                }
                if let Ok(list) = fetch_json::<Vec<ProbeInfo>>("/api/probes").await {
                    probes.set(list);
                }
                if let Ok(list) =
                    fetch_json::<Vec<ProbeDevice>>(&format!("/api/probe/{id}/devices")).await
                {
                    devices.set(list);
                }
                if let Ok(list) =
                    fetch_json::<Vec<CommandInfo>>(&format!("/api/probes/{id}/commands")).await
                {
                    commands.set(list);
                }
                if let Ok(info) =
                    fetch_json::<MachineInfo>(&format!("/api/probes/{id}/machine")).await
                {
                    machine.set(Some(info));
                }
            }
            loaded.set(true);
        });
    };
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            load();
            browser::poll_every(REFRESH_MS, load);
        }
    });

    // A POST, then a reload.
    #[cfg_attr(feature = "ssr", allow(unused_variables))]
    let act = move |url: String, body: String| {
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json(&url, &body).await {
                Ok(()) => load(),
                Err(e) => error.set(e),
            }
        });
    };

    let probe = Memo::new(move |_| {
        let id = probe_id.get();
        probes.get().into_iter().find(|p| p.id == id)
    });
    let dialog_open = RwSignal::new(false);
    let editing = RwSignal::new(None::<CheckInfo>);
    let templates_open = RwSignal::new(false);

    view! {
        <div class="probe-page">
            {move || match probe.get() {
                Some(p) => view! { <ProbeHeader probe=p/> }.into_any(),
                None => view! { <h1>"Probe"</h1> }.into_any(),
            }}
            <div class="login-error">{error}</div>
            <Show when=move || loaded.get() && state.get().is_none() && error.get().is_empty()>
                <p class="info-text">"No such probe."</p>
            </Show>
            {move || probe.get().map(|p| view! { <ProbeHealthSection probe=p/> })}
            {move || machine.get().map(|m| view! { <MachineSection machine=m/> })}
            {move || state.get().map(|s| {
                let (class, text) = sync_badge(&s);
                view! {
                    <div class="admin-section">
                        <h2>"Reporting"</h2>
                        <p><span class=class>{text}</span></p>
                        {move || probe.get().map(|p| view! {
                            <IntervalEditor probe=p can_manage=can_manage act=act/>
                        })}
                    </div>
                }
            })}
            <div class="admin-section">
                <h2>"Checks"</h2>
                <Show when=move || can_manage.get()>
                    <div class="section-actions">
                        <button class="btn btn-primary" on:click=move |_| {
                            editing.set(None);
                            dialog_open.set(true);
                        }>"Add check"</button>
                        <button class="btn btn-secondary"
                            title="Add a set of checks in one go, or save this probe's checks as a template"
                            on:click=move |_| templates_open.set(true)>"Templates…"</button>
                        {move || probe.get().map(|p| {
                            let run_url = format!("/api/probes/{}/commands", p.id);
                            let active = p.state == "active";
                            view! {
                                <button class="btn btn-secondary" disabled=!active
                                    title="The probe runs its checks and reports as soon as it gets this, with the answer to its next report"
                                    on:click=move |_| act(run_url.clone(), r#"{"kind":"run_checks"}"#.into())>
                                    "Run checks now"
                                </button>
                                <CopyChecks probe=p probes=probes act=act/>
                            }
                        })}
                    </div>
                </Show>
                {move || -> AnyView {
                    let Some(s) = state.get() else { return ().into_any() };
                    if s.checks.is_empty() {
                        return view! {
                            <p class="info-text">"No checks yet: the probe reports, but checks nothing."</p>
                        }.into_any();
                    }
                    let results = devices.get();
                    view! {
                        <table class="admin-table checks-table">
                            <thead>
                                <tr>
                                    <th>"Name"</th><th>"Kind"</th><th>"Target"</th>
                                    <th>"Last result"</th><th></th>
                                </tr>
                            </thead>
                            <tbody>
                                {s.checks.clone().into_iter().map(|check| {
                                    let result = results.iter().find(|d| d.check_id == check.id).cloned();
                                    // The check it is behind, by name (D-68).
                                    let behind = check.depends_on.as_ref().and_then(|d| {
                                        s.checks.iter().find(|c| &c.id == d).map(|c| c.name.clone())
                                    });
                                    view! {
                                        <CheckRow check=check result=result behind=behind can_manage=can_manage
                                            editing=editing dialog_open=dialog_open act=act/>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                }}
                <p class="info-text">
                    "Changes reach the probe with the answer to its next report. A probe checks each target from its own network."
                </p>
                {move || {
                    let suggestions = machine.get().map(|m| m.suggestions).unwrap_or_default();
                    (can_manage.get() && !suggestions.is_empty()).then(|| view! {
                        <SuggestedChecks probe_id=probe_id.get_untracked() suggestions=suggestions act=act/>
                    })
                }}
            </div>
            <ProbeCommands commands=commands can_manage=can_manage act=act/>
            {move || dialog_open.get().then(|| view! {
                <CheckDialog probe_id=probe_id.get_untracked() editing=editing.get_untracked()
                    checks=state.get_untracked().map(|s| s.checks).unwrap_or_default()
                    open=dialog_open saved=load/>
            })}
            {move || templates_open.get().then(|| view! {
                <TemplatesDialog probe_id=probe_id.get_untracked()
                    has_checks=state.get_untracked().is_some_and(|s| !s.checks.is_empty())
                    open=templates_open changed=load/>
            })}
        </div>
    }
}

/// Name, company, site, host and last report.
#[component]
fn ProbeHeader(probe: ProbeInfo) -> impl IntoView {
    let host = [probe.hostname.clone(), probe.os.clone(), probe.arch.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    view! {
        <h1>{probe.name.clone()}</h1>
        <p class="probe-meta">
            <a href=format!("/companies/{}", probe.company_id)>{probe.company_name.clone()}</a>
            " · " {probe.site_name.clone()}
            {(!host.is_empty()).then(|| view! { " · " {host.clone()} })}
            " · last report: "
            {probe.last_seen_at.as_deref().map(fmt_ts).unwrap_or_else(|| "never".into())}
            {(probe.state != "active").then(|| view! { " · " <strong>"Revoked"</strong> })}
        </p>
    }
}

/// What the probe last said about itself (protocol v8, D-73).
#[component]
fn ProbeHealthSection(probe: ProbeInfo) -> impl IntoView {
    let concerns = probe.concerns();
    let body = match probe.health.clone() {
        None => view! {
            <p class="info-text">"Not known yet: a probe says how it is doing with each report; an older probe does once it updates."</p>
        }
        .into_any(),
        Some(h) => {
            let problem = h
                .last_error
                .map(|p| format!("{}: {}", fmt_utc(&p.at), p.message))
                .unwrap_or_else(|| "None in the past day".into());
            view! {
                <dl class="probe-health">
                    <dt>"Running since"</dt><dd>{fmt_utc(&h.started_at)}</dd>
                    <dt>"Reports held back"</dt><dd>{h.queued_reports}</dd>
                    <dt>"Clock against the hub's"</dt><dd>{clock_text(h.clock_offset_secs)}</dd>
                    <dt>"Last problem"</dt><dd class="probe-health-problem">{problem}</dd>
                </dl>
            }
            .into_any()
        }
    };
    view! {
        <div class="admin-section">
            <h2>"Health"</h2>
            {(!concerns.is_empty()).then(|| view! {
                <p class="probe-concerns">"Needs attention: " {concerns.join(" · ")}</p>
            })}
            {body}
        </div>
    }
}

/// The report interval, editable with `ManageCompanies`.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn IntervalEditor(
    probe: ProbeInfo,
    can_manage: Signal<bool>,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let seconds = RwSignal::new(probe.interval_secs.to_string());
    let probe = StoredValue::new(probe);
    #[cfg(not(feature = "ssr"))]
    let on_input = move |ev| seconds.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_input = |_| {};
    let save = move || {
        #[cfg(not(feature = "ssr"))]
        {
            let p = probe.get_value();
            let body = serde_json::json!({
                "name": p.name,
                "site_id": p.site_id,
                // Not a number: 0, which the hub refuses with the allowed range.
                "interval_secs": seconds.get_untracked().trim().parse::<u32>().unwrap_or(0),
            });
            act(format!("/api/probes/{}", p.id), body.to_string());
        }
    };
    view! {
        <div class="interval-editor">
            <label for="interval_secs">"Report every (seconds)"</label>
            <input id="interval_secs" name="interval_secs" type="number" min="5" max="3600"
                prop:value=move || seconds.get() on:input=on_input
                disabled=move || !can_manage.get()/>
            <Show when=move || can_manage.get()>
                <button class="btn btn-secondary btn-sm" on:click=move |_| save()>"Save"</button>
            </Show>
        </div>
    }
}

/// One check with its latest result and actions.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn CheckRow(
    check: CheckInfo,
    result: Option<ProbeDevice>,
    behind: Option<String>,
    can_manage: Signal<bool>,
    editing: RwSignal<Option<CheckInfo>>,
    dialog_open: RwSignal<bool>,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let (status_class, last) = match (&result, check.enabled) {
        (_, false) => ("", "Disabled".to_string()),
        (None, true) => ("", "Not reported yet".to_string()),
        (Some(r), true) => {
            // The hub's reason first (a threshold, D-64), else the probe's message.
            let why = r
                .reason
                .clone()
                .or_else(|| r.message.clone())
                .unwrap_or_default();
            (
                status_class(r.status),
                match r.headline() {
                    Some(value) => format!("{} · {value} · {why}", status_label(r.status)),
                    None => format!("{} · {why}", status_label(r.status)),
                },
            )
        }
    };
    let stored = StoredValue::new(check.clone());
    let toggle = move || {
        let c = stored.get_value();
        let body = serde_json::to_string(&switched(&c)).unwrap_or_default();
        act(format!("/api/checks/{}", c.id), body);
    };
    let delete = move || {
        act(
            format!("/api/checks/{}/delete", stored.get_value().id),
            "{}".into(),
        )
    };

    view! {
        <tr class:inactive=!check.enabled>
            <td>{check.name.clone()}</td>
            <td class="check-kind">{kind_label(check.spec.kind())}</td>
            <td>
                {check.target.clone()}
                {behind.map(|b| view! { <span class="check-behind">{format!(" · behind {b}")}</span> })}
            </td>
            <td class=format!("check-result {status_class}")>{last}</td>
            <td class="row-actions">
                <Show when=move || can_manage.get()>
                    <button class="btn btn-secondary btn-sm" on:click=move |_| {
                        editing.set(Some(stored.get_value()));
                        dialog_open.set(true);
                    }>"Edit"</button>
                    <button class="btn btn-secondary btn-sm" on:click=move |_| toggle()>
                        {if stored.get_value().enabled { "Disable" } else { "Enable" }}
                    </button>
                    <button class="btn btn-danger btn-sm" on:click=move |_| delete()>"Delete"</button>
                </Show>
            </td>
        </tr>
    }
}

/// "Copy checks from": add copies of another probe's checks.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn CopyChecks(
    probe: ProbeInfo,
    probes: RwSignal<Vec<ProbeInfo>>,
    act: impl Fn(String, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let from = RwSignal::new(String::new());
    let target = StoredValue::new(probe.id.clone());
    let others = move || {
        probes
            .get()
            .into_iter()
            .filter(|p| p.id != target.get_value())
            .collect::<Vec<_>>()
    };
    #[cfg(not(feature = "ssr"))]
    let on_change = move |ev| from.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_change = |_| {};
    let copy = move || {
        let source = from.get_untracked();
        if source.is_empty() {
            return;
        }
        let body = serde_json::json!({ "from_probe_id": source });
        act(
            format!("/api/probes/{}/checks/copy", target.get_value()),
            body.to_string(),
        );
    };
    view! {
        <Show when=move || !others().is_empty()>
            <span class="copy-checks">
                <select name="copy_from" prop:value=move || from.get() on:change=on_change>
                    <option value="">"Copy checks from…"</option>
                    {move || others().into_iter().map(|p| view! {
                        <option value=p.id.clone()>{format!("{} ({})", p.name, p.company_name)}</option>
                    }).collect_view()}
                </select>
                <button class="btn btn-secondary btn-sm" on:click=move |_| copy()>"Copy"</button>
            </span>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(sync: ConfigSync) -> ProbeChecks {
        ProbeChecks {
            probe_id: "p".into(),
            interval_secs: 60,
            config_revision: "r2".into(),
            applied_revision: Some("r1".into()),
            sync,
            config_error: Some("unknown kind".into()),
            checks: vec![],
        }
    }

    #[test]
    fn the_sync_badge_says_where_the_probe_is() {
        assert_eq!(sync_badge(&state(ConfigSync::InSync)).1, "In sync");
        assert!(
            sync_badge(&state(ConfigSync::Pending))
                .1
                .starts_with("Pending")
        );
        let (class, text) = sync_badge(&state(ConfigSync::Rejected));
        assert_eq!(class, "sync-badge rejected");
        assert!(text.ends_with("unknown kind"));
    }
}
