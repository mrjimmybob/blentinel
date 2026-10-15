//! "Probe updates" on the admin page (M2-15, D-55; the rollout M4-01, D-74):
//! the signed releases on the hub, the one being rolled out (canaries first,
//! pause, promote; none by default), adding and removing releases, which
//! probes are canaries, what each probe runs and will get, and each
//! company's update policy and window.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_file, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::model::ProbeRelease;
use crate::model::{
    CompanyUpdatePolicy, CompanyUpdates, DEFAULT_SOAK_MINUTES, Permission, ProbeInfo, ProbeUpdates,
    RolloutInfo, RolloutStage, UpdateMode, UpdateWindow,
};

/// POST `body` to `url`; then reload the section, or show the hub's reason.
#[derive(Clone, Copy)]
struct Act {
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    refresh: RwSignal<u32>,
}

impl Act {
    fn post(self, url: String, body: String) {
        self.busy.set(true);
        self.error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            if let Err(e) = post_json(&url, &body).await {
                self.error.set(e);
            }
            #[cfg(feature = "ssr")]
            let _ = (url, body);
            self.busy.set(false);
            self.refresh.update(|n| *n += 1);
        });
    }
}

#[component]
pub(crate) fn ProbeUpdatesSection() -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let act = Act {
        busy,
        error,
        refresh,
    };
    let soak = RwSignal::new(DEFAULT_SOAK_MINUTES.to_string());
    let file_input = NodeRef::<leptos::html::Input>::new();

    let updates: LocalResource<Result<ProbeUpdates, String>> = LocalResource::new(move || {
        refresh.track();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<ProbeUpdates>("/api/admin/probe-updates").await
            }
            #[cfg(feature = "ssr")]
            {
                Err(String::new())
            }
        }
    });

    let approve = move |version: Option<String>| {
        let soak_minutes = match &version {
            Some(_) => match soak.get().trim().parse::<u32>() {
                Ok(minutes) => Some(minutes),
                Err(_) => {
                    error.set("Give the wait after the canaries in minutes.".into());
                    return;
                }
            },
            None => None,
        };
        let body = serde_json::json!({ "version": version, "soak_minutes": soak_minutes });
        act.post("/api/admin/probe-updates/approve".into(), body.to_string());
    };
    let remove = move |version: String| {
        act.post(
            format!("/api/admin/probe-releases/{version}/delete"),
            "{}".into(),
        );
    };
    let upload = move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            let Some(file) = file_input
                .get()
                .and_then(|input| input.files())
                .and_then(|files| files.get(0))
            else {
                error.set("Choose the release's .tar.gz first.".into());
                return;
            };
            busy.set(true);
            error.set(String::new());
            leptos::task::spawn_local(async move {
                match post_file::<ProbeRelease>("/api/admin/probe-releases", file).await {
                    Ok(_) => {
                        if let Some(input) = file_input.get() {
                            input.set_value("");
                        }
                    }
                    Err(e) => error.set(e),
                }
                busy.set(false);
                refresh.update(|n| *n += 1);
            });
        }
    };

    view! {
        <div class="admin-section probe-updates">
            <h2>"Probe updates"</h2>
            {move || -> AnyView {
                match updates.get() {
                    Some(Ok(data)) => {
                        let versions: Vec<String> =
                            data.releases.iter().map(|r| r.version.clone()).collect();
                        any(view! {
                            <Rollout data=data.clone() can_manage=can_manage act=act on_stop=move || approve(None)/>
                            <Releases
                                data=data.clone()
                                can_manage=can_manage
                                busy=busy
                                soak=soak
                                on_approve=move |v| approve(Some(v))
                                on_remove=remove
                            />
                            <ProbeVersions probes=data.probes.clone() can_manage=can_manage act=act/>
                            <CompanyPolicies companies=data.companies.clone() versions=versions can_manage=can_manage act=act/>
                        })
                    }
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            <Show when=move || can_manage.get()>
                <div class="release-upload">
                    <label>
                        "Add a release or a rollback (the .tar.gz from "
                        <code>"blentinelmake release build"</code>
                        " or "
                        <code>"release rollback"</code>
                        "): "
                        <input type="file" accept=".gz,.tgz" node_ref=file_input/>
                    </label>
                    <button class="btn btn-primary" disabled=busy on:click=upload>
                        "Add release"
                    </button>
                </div>
            </Show>
            {move || {
                let e = error.get();
                if e.is_empty() { nothing() } else { any(view! { <div class="error-message">{e}</div> }) }
            }}
        </div>
    }
}

/// The release being rolled out, where it is, and its buttons.
#[component]
fn Rollout(
    data: ProbeUpdates,
    can_manage: Signal<bool>,
    act: Act,
    on_stop: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let key_warning = (!data.has_release_key).then(|| {
        view! {
            <div class="storage-warning-banner">
                "This hub was built without a release key, so it can't take a release. \
                 Run `blentinelmake release keygen`, then build the hub and probes again."
            </div>
        }
    });
    let Some(rollout) = data.rollout else {
        return any(view! {
            {key_warning}
            <p class="update-status">
                "Updates are off: no release is approved (companies pinned to a release still get it). \
                 Approve one below: canaries get it first, then every probe."
            </p>
        });
    };
    let busy = act.busy;
    let post = move |what: &str| act.post(format!("/api/admin/probe-updates/{what}"), "{}".into());
    let RolloutInfo {
        version,
        approved_by,
        approved_at,
        stage,
        soak_minutes,
        canaries,
        canaries_ready,
        promote_at,
        promoted_at,
        promoted_by,
        paused,
    } = rollout;
    let is_paused = paused.is_some();
    let stage_text = match (&paused, stage) {
        (Some(p), _) => format!(
            "Paused by {} on {}: {}. No more probes are offered it until it is resumed.",
            p.by.clone().unwrap_or_else(|| "the hub".into()),
            p.at,
            p.reason
        ),
        (None, RolloutStage::Canary) => {
            let when = promote_at.map_or_else(
                || format!("{soak_minutes} minutes after the last canary runs it"),
                |at| format!("at {at}"),
            );
            format!(
                "Canaries first: {canaries_ready} of {canaries} canaries run it. Every probe gets it {when}."
            )
        }
        (None, RolloutStage::All) => match (promoted_at, promoted_by) {
            (Some(at), Some(by)) => format!("Rolling out to every probe (sent by {by} on {at})."),
            (Some(at), None) => format!("Rolling out to every probe (the canaries ran it; {at})."),
            _ => "Rolling out to every probe.".into(),
        },
    };
    let is_canary_stage = stage == RolloutStage::Canary;
    any(view! {
        {key_warning}
        <p class="update-status">
            "Probes are offered "<strong>{version}</strong>
            {format!(
                " (approved by {} on {}). ",
                approved_by.unwrap_or_else(|| "?".into()),
                approved_at
            )}
            <span class="rollout-stage">{stage_text}</span>
            " Each probe updates itself after its next report, then must report within 15 minutes or it goes back."
        </p>
        <Show when=move || can_manage.get()>
            <div class="rollout-actions">
                {is_paused.then(|| view! {
                    <button class="btn btn-primary btn-sm" disabled=busy on:click=move |_| post("resume")>"Resume"</button>
                    " "
                })}
                {(!is_paused).then(|| view! {
                    <button class="btn btn-secondary btn-sm" disabled=busy on:click=move |_| post("pause")>"Pause"</button>
                    " "
                })}
                {is_canary_stage.then(|| view! {
                    <button class="btn btn-secondary btn-sm" disabled=busy on:click=move |_| post("promote")>"Roll out to all now"</button>
                    " "
                })}
                <button class="btn btn-secondary btn-sm" disabled=busy on:click=move |_| on_stop()>"Stop updates"</button>
            </div>
        </Show>
    })
}

/// The releases on the hub, newest first.
#[component]
fn Releases(
    data: ProbeUpdates,
    can_manage: Signal<bool>,
    busy: RwSignal<bool>,
    soak: RwSignal<String>,
    on_approve: impl Fn(String) + Copy + Send + Sync + 'static,
    on_remove: impl Fn(String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    if data.releases.is_empty() {
        return any(view! {
            <div class="empty-state">
                "No probe releases yet. They come with the hub (blentinelmake hub publish --with-probes) or are added here."
            </div>
        });
    }
    #[cfg(not(feature = "ssr"))]
    let on_soak = move |ev| soak.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_soak = |_| {};
    let approved = data.approved().map(str::to_string);
    let rows = data
        .releases
        .iter()
        .map(|release| {
            let is_approved = approved.as_deref() == Some(release.version.as_str());
            let running = data
                .probes
                .iter()
                .filter(|p| p.probe_version.as_deref() == Some(release.version.as_str()))
                .count();
            let (to_approve, to_remove) = (release.version.clone(), release.version.clone());
            let actions = (!is_approved).then(|| {
                view! {
                    <Show when=move || can_manage.get()>
                        <button class="btn btn-primary btn-sm" disabled=busy on:click={
                            let v = to_approve.clone();
                            move |_| on_approve(v.clone())
                        }>"Approve"</button>
                        " "
                        <button class="btn btn-danger btn-sm" disabled=busy on:click={
                            let v = to_remove.clone();
                            move |_| on_remove(v.clone())
                        }>"Remove"</button>
                    </Show>
                }
            });
            let rollback = (!release.rollback_from.is_empty()).then(|| {
                view! {
                    " " <span class="badge rollback">{format!("rollback from {}", release.rollback_from.join(", "))}</span>
                }
            });
            view! {
                <tr class:approved=is_approved>
                    <td>
                        {release.version.clone()}
                        {is_approved.then(|| view! { " " <span class="badge">"approved"</span> })}
                        {rollback}
                    </td>
                    <td>{release.released.clone()}</td>
                    <td>{release.platforms.join(", ")}</td>
                    <td>{running}</td>
                    <td>{actions}</td>
                </tr>
            }
        })
        .collect_view();
    any(view! {
        <Show when=move || can_manage.get()>
            <p class="rollout-soak">
                <label>
                    "When approving, wait "
                    <input type="number" name="soak_minutes" min="0" max=crate::model::MAX_SOAK_MINUTES.to_string()
                        prop:value=move || soak.get() on:input=on_soak/>
                    " minutes after every canary runs it before every probe gets it."
                </label>
            </p>
        </Show>
        <table class="admin-table releases">
            <thead>
                <tr>
                    <th>"Release"</th>
                    <th>"Built"</th>
                    <th>"Platforms"</th>
                    <th>"Probes on it"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    })
}

/// What each probe runs, whether it is a canary, what it gets next, and the
/// last update it didn't install.
#[component]
fn ProbeVersions(probes: Vec<ProbeInfo>, can_manage: Signal<bool>, act: Act) -> impl IntoView {
    if probes.is_empty() {
        return nothing();
    }
    let busy = act.busy;
    let rows = probes
        .into_iter()
        .map(|probe| {
            let version = probe.probe_version.clone().unwrap_or_else(|| "?".into());
            let status = probe
                .update_status
                .as_ref()
                .map(|s| s.describe())
                .unwrap_or_default();
            let outdated = matches!(
                probe.update_status,
                Some(crate::model::UpdateStatus::Updating { .. })
            );
            let failed = probe
                .update_failed
                .as_ref()
                .map(|f| format!("{} not installed: {}", f.version, f.error));
            let href = format!("/probes/{}", probe.id);
            let has_failed = failed.is_some();
            let (id, canary) = (probe.id.clone(), probe.canary);
            let toggle = move |_| {
                let body = serde_json::json!({ "canary": !canary }).to_string();
                act.post(format!("/api/admin/probe-updates/probes/{id}/canary"), body);
            };
            view! {
                <tr class:update-failed=has_failed>
                    <td><a href=href>{probe.name.clone()}</a></td>
                    <td>{probe.company_name.clone()}</td>
                    <td class:outdated=outdated>{version}</td>
                    <td>
                        <input type="checkbox" name="canary" prop:checked=canary
                            disabled=move || busy.get() || !can_manage.get() on:change=toggle/>
                    </td>
                    <td class="update-status-cell">{status}</td>
                    <td>{failed.unwrap_or_default()}</td>
                </tr>
            }
        })
        .collect_view();
    any(view! {
        <table class="admin-table probe-versions">
            <thead>
                <tr>
                    <th>"Probe"</th>
                    <th>"Company"</th>
                    <th>"Version"</th>
                    <th>"Canary"</th>
                    <th>"Update"</th>
                    <th>"Last failed update"</th>
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    })
}

/// Each company's policy (follow, off, pinned) and update window.
#[component]
fn CompanyPolicies(
    companies: Vec<CompanyUpdates>,
    versions: Vec<String>,
    can_manage: Signal<bool>,
    act: Act,
) -> impl IntoView {
    if companies.is_empty() {
        return nothing();
    }
    let rows = companies
        .into_iter()
        .map(|company| {
            view! { <CompanyPolicyRow company=company versions=versions.clone() can_manage=can_manage act=act/> }
        })
        .collect_view();
    any(view! {
        <h3>"Updates by company"</h3>
        <p class="info-text">
            "A company follows the rollout, takes no updates, or stays on one release (pinned). \
             The window is local time at each probe's site; outside it, its probes wait."
        </p>
        <table class="admin-table company-updates">
            <thead>
                <tr>
                    <th>"Company"</th>
                    <th>"Updates"</th>
                    <th>"Window"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    })
}

#[component]
fn CompanyPolicyRow(
    company: CompanyUpdates,
    versions: Vec<String>,
    can_manage: Signal<bool>,
    act: Act,
) -> impl IntoView {
    let policy = company.policy;
    let mode = RwSignal::new(policy.mode.as_str().to_string());
    let pinned = RwSignal::new(
        policy
            .version
            .clone()
            .or_else(|| versions.first().cloned())
            .unwrap_or_default(),
    );
    let (start, end) = policy
        .window
        .map_or((String::new(), String::new()), |w| (w.start, w.end));
    let (start, end) = (RwSignal::new(start), RwSignal::new(end));
    #[cfg(not(feature = "ssr"))]
    let (on_mode, on_pinned, on_start, on_end) = (
        move |ev| mode.set(input_value(&ev)),
        move |ev| pinned.set(input_value(&ev)),
        move |ev| start.set(input_value(&ev)),
        move |ev| end.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_mode, on_pinned, on_start, on_end) = (|_| {}, |_| {}, |_| {}, |_| {});

    let id = company.company_id.clone();
    let save = move |_| {
        let mode = UpdateMode::parse(&mode.get()).unwrap_or_default();
        let window = match (start.get().trim().to_string(), end.get().trim().to_string()) {
            (s, e) if s.is_empty() && e.is_empty() => None,
            (start, end) => Some(UpdateWindow { start, end }),
        };
        let policy = CompanyUpdatePolicy {
            mode,
            version: (mode == UpdateMode::Pin).then(|| pinned.get()),
            window,
        };
        if let Err(e) = policy.validate() {
            act.error.set(format!("Not saved: {e}."));
            return;
        }
        let body = serde_json::to_string(&policy).unwrap_or_default();
        act.post(format!("/api/admin/probe-updates/companies/{id}"), body);
    };
    let options = move || {
        versions
            .iter()
            .map(|v| view! { <option value=v.clone()>{v.clone()}</option> })
            .collect_view()
    };
    let disabled = move || act.busy.get() || !can_manage.get();
    view! {
        <tr data-company=company.company_id.clone()>
            <td>{company.company_name}</td>
            <td>
                <select name="update_mode" prop:value=move || mode.get() on:change=on_mode disabled=disabled>
                    <option value="follow">"Follow the rollout"</option>
                    <option value="off">"Off"</option>
                    <option value="pin">"Pinned to"</option>
                </select>
                <Show when=move || mode.get() == "pin">
                    " "
                    <select name="update_version" prop:value=move || pinned.get() on:change=on_pinned disabled=disabled>
                        {options.clone()}
                    </select>
                </Show>
            </td>
            <td>
                <input type="time" name="window_start" prop:value=move || start.get() on:input=on_start disabled=disabled/>
                " to "
                <input type="time" name="window_end" prop:value=move || end.get() on:input=on_end disabled=disabled/>
            </td>
            <td>
                <Show when=move || can_manage.get()>
                    <button class="btn btn-secondary btn-sm" name="update_policy_save" disabled=move || act.busy.get() on:click=save.clone()>"Save"</button>
                </Show>
            </td>
        </tr>
    }
}
