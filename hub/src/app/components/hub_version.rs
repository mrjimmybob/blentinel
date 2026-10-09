//! "Hub version" on the admin page (M4-04, D-75): the version running, the
//! opt-in daily check for a newer release, and, when one is out, the command
//! that upgrades the hub on its server. The page never upgrades the hub.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for};
use crate::app::ui::{Me, any, nothing};
use crate::model::{HubUpgradeInfo, Permission};

#[component]
pub(crate) fn HubVersionSection() -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageData));
    let info: RwSignal<Option<Result<HubUpgradeInfo, String>>> = RwSignal::new(None);
    let busy = RwSignal::new(false);

    #[cfg(not(feature = "ssr"))]
    leptos::task::spawn_local(async move {
        info.set(Some(
            fetch_json::<HubUpgradeInfo>("/api/admin/hub-upgrade").await,
        ));
    });

    // POST, and show what the hub answers (the section, or its reason).
    let post = move |url: &'static str, body: String| {
        busy.set(true);
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            info.set(Some(post_for::<HubUpgradeInfo>(url, &body).await));
            #[cfg(feature = "ssr")]
            let _ = (url, body);
            busy.set(false);
        });
    };

    view! {
        <div class="admin-section hub-version">
            <h2>"Hub Version"</h2>
            {move || -> AnyView {
                match info.get() {
                    Some(Ok(data)) => any(view! { <HubVersion data=data can_manage=can_manage busy=busy post=post/> }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
        </div>
    }
}

#[component]
fn HubVersion(
    data: HubUpgradeInfo,
    can_manage: Signal<bool>,
    busy: RwSignal<bool>,
    post: impl Fn(&'static str, String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let enabled = data.check_enabled;
    let toggle = move |_| {
        let body = serde_json::json!({ "enabled": !enabled }).to_string();
        post("/api/admin/hub-upgrade", body);
    };
    let check_now = move |_| post("/api/admin/hub-upgrade/check", "{}".into());
    let available = data.available.clone().map(|version| {
        view! {
            <div class="upgrade-available storage-warning-banner">
                <strong>{format!("Blentinel {version} is available.")}</strong>
                " Upgrade the hub on its server with "
                <code>{data.command.clone()}</code>
                ": it checks the release's signature, tries the new hub on a copy of the database, \
                 backs up, and goes back by itself if the new hub doesn't run. Then approve its \
                 probes under Probe updates."
            </div>
        }
    });
    let last = match (&data.checked_at, &data.latest, &data.error) {
        (None, _, _) => "Not checked yet.".to_string(),
        (Some(at), _, Some(error)) => format!("Last check {at} failed: {error}"),
        (Some(at), Some(latest), None) if data.available.is_none() => {
            format!("Checked {at}: the newest release is {latest}; this hub is up to date.")
        }
        (Some(at), _, None) => format!("Checked {at}."),
    };
    let status = if enabled {
        "The hub checks for a newer release once a day."
    } else {
        "The hub doesn't check for newer releases (it contacts no outside server)."
    };
    view! {
        <div class="storage-info-card">
            <div class="storage-stats">
                <div class="storage-stat">
                    <span class="stat-label">"Version:"</span>
                    <span class="stat-value">{data.running.clone()}</span>
                </div>
            </div>
        </div>
        {available}
        <p class="upgrade-check">
            {status} " " <span class="upgrade-last">{last}</span>
        </p>
        <p class="info-text">"Releases from " <code>{data.release_url.clone()}</code>
            " ([upgrades] release_url); only ones signed with a release key this hub trusts."</p>
        {move || if can_manage.get() {
            any(view! {
                <div class="upgrade-actions">
                    <button class="btn btn-secondary btn-sm" name="upgrade_check_toggle" disabled=busy on:click=toggle>
                        {if enabled { "Stop checking" } else { "Check daily" }}
                    </button>
                    " "
                    <button class="btn btn-secondary btn-sm" name="upgrade_check_now" disabled=busy on:click=check_now>
                        "Check now"
                    </button>
                </div>
            })
        } else {
            nothing()
        }}
    }
}
