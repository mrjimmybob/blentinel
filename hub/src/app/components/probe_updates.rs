//! "Probe updates" on the admin page (M2-15, D-55): the signed releases on
//! the hub, which one probes are offered (none by default), adding and
//! removing releases, and which version each probe runs or refused.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_file, post_json};
use crate::app::ui::{Me, any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::model::ProbeRelease;
use crate::model::{Permission, ProbeInfo, ProbeUpdates};

#[component]
pub(crate) fn ProbeUpdatesSection() -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
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

    // POST `body` to `url`, then reload the section (or show the hub's reason).
    let act = move |url: String, body: String| {
        busy.set(true);
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            if let Err(e) = post_json(&url, &body).await {
                error.set(e);
            }
            #[cfg(feature = "ssr")]
            let _ = (url, body);
            busy.set(false);
            refresh.update(|n| *n += 1);
        });
    };
    let approve = move |version: Option<String>| {
        let body = serde_json::json!({ "version": version }).to_string();
        act("/api/admin/probe-updates/approve".into(), body);
    };
    let remove = move |version: String| {
        act(
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
                    Some(Ok(data)) => any(view! {
                        <Approval data=data.clone() can_manage=can_manage busy=busy on_stop=move || approve(None)/>
                        <Releases
                            data=data.clone()
                            can_manage=can_manage
                            busy=busy
                            on_approve=move |v| approve(Some(v))
                            on_remove=remove
                        />
                        <ProbeVersions probes=data.probes.clone() approved=data.approved.clone()/>
                    }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            <Show when=move || can_manage.get()>
                <div class="release-upload">
                    <label>
                        "Add a release (the .tar.gz from "
                        <code>"blentinelmake release build"</code>
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

/// Which release probes are offered, and the switch to stop.
#[component]
fn Approval(
    data: ProbeUpdates,
    can_manage: Signal<bool>,
    busy: RwSignal<bool>,
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
    let status = match &data.approved {
        Some(version) => any(view! {
            <p class="update-status">
                "Probes are offered "<strong>{version.clone()}</strong>
                {format!(
                    " (approved by {} on {}). Each probe running another version updates itself after its next report, then must report within 15 minutes or it goes back.",
                    data.approved_by.clone().unwrap_or_else(|| "?".into()),
                    data.approved_at.clone().unwrap_or_default()
                )}
                <Show when=move || can_manage.get()>
                    " "<button class="btn btn-secondary btn-sm" disabled=busy on:click=move |_| on_stop()>"Stop updates"</button>
                </Show>
            </p>
        }),
        None => any(view! {
            <p class="update-status">
                "Updates are off: no probe is offered a new version. Approve a release below to update every probe."
            </p>
        }),
    };
    view! { {key_warning} {status} }
}

/// The releases on the hub, newest first.
#[component]
fn Releases(
    data: ProbeUpdates,
    can_manage: Signal<bool>,
    busy: RwSignal<bool>,
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
    let approved = data.approved.clone();
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
            view! {
                <tr class:approved=is_approved>
                    <td>
                        {release.version.clone()}
                        {is_approved.then(|| view! { " " <span class="badge">"approved"</span> })}
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

/// What each probe runs, and the last update it didn't install.
#[component]
fn ProbeVersions(probes: Vec<ProbeInfo>, approved: Option<String>) -> impl IntoView {
    if probes.is_empty() {
        return nothing();
    }
    let rows = probes
        .into_iter()
        .map(|probe| {
            let version = probe.probe_version.clone().unwrap_or_else(|| "?".into());
            let current =
                approved.is_none() || approved.as_deref() == probe.probe_version.as_deref();
            let failed = probe
                .update_failed
                .as_ref()
                .map(|f| format!("{} not installed: {}", f.version, f.error));
            let href = format!("/probes/{}", probe.id);
            let has_failed = failed.is_some();
            view! {
                <tr class:update-failed=has_failed>
                    <td><a href=href>{probe.name.clone()}</a></td>
                    <td>{probe.company_name.clone()}</td>
                    <td class:outdated=!current>{version}</td>
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
                    <th>"Last failed update"</th>
                </tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    })
}
