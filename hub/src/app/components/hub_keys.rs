//! "Hub keys" on the admin page (M2-13, D-61): what probes pin of the hub,
//! and rotating its certificate and key: start, progress (which probes are
//! ready), switch now, cancel, and earlier rotations.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, nothing};
use crate::model::{
    HubKeysOverview, Permission, RotationInfo, RotationState, StartRotationRequest,
};

#[component]
pub(crate) fn HubKeysSection() -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let keys: LocalResource<Result<HubKeysOverview, String>> = LocalResource::new(move || {
        refresh.track();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<HubKeysOverview>("/api/admin/hub-keys").await
            }
            #[cfg(feature = "ssr")]
            {
                Err(String::new())
            }
        }
    });

    // POST `body` to `url`, then reload the section (or show the hub's reason).
    let act = move |url: &'static str, body: String| {
        busy.set(true);
        error.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            if let Err(e) = post_json(url, &body).await {
                error.set(e);
            }
            #[cfg(feature = "ssr")]
            let _ = (url, body);
            busy.set(false);
            refresh.update(|n| *n += 1);
        });
    };
    let start = move |request: StartRotationRequest| {
        let body = serde_json::to_string(&request).unwrap_or_default();
        act("/api/admin/hub-keys/rotation", body);
    };
    let switch_now = move || act("/api/admin/hub-keys/rotation/switch", "{}".into());
    let cancel = move || act("/api/admin/hub-keys/rotation/cancel", "{}".into());

    view! {
        <div class="admin-section hub-keys">
            <h2>"Hub keys"</h2>
            {move || -> AnyView {
                match keys.get() {
                    Some(Ok(data)) => {
                        let tls_enabled = data.tls_enabled;
                        let rotation = data.rotation.clone();
                        any(view! {
                            <Pins data=data.clone()/>
                            {match rotation {
                                Some(rotation) => any(view! {
                                    <Progress rotation can_manage busy on_switch=switch_now on_cancel=cancel/>
                                }),
                                None => any(view! {
                                    <Show when=move || can_manage.get()>
                                        <StartForm tls_enabled busy error on_start=start/>
                                    </Show>
                                }),
                            }}
                            <History rotations=data.history.clone()/>
                        })
                    }
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            {move || {
                let e = error.get();
                if e.is_empty() { nothing() } else { any(view! { <div class="error-message">{e}</div> }) }
            }}
        </div>
    }
}

/// Short form of a hex fingerprint, as people compare them.
fn short(hex: &str) -> String {
    hex.get(..16)
        .map_or_else(|| hex.to_string(), |s| format!("{s}…"))
}

/// What probes pin now.
#[component]
fn Pins(data: HubKeysOverview) -> impl IntoView {
    let tls = match (&data.tls_spki_sha256, &data.tls_expires) {
        (Some(pin), expires) => format!(
            "Certificate key {}{}",
            short(pin),
            expires
                .as_ref()
                .map(|e| format!(" (certificate expires {e})"))
                .unwrap_or_default()
        ),
        (None, _) => "No certificate of its own (plain HTTP, or behind a proxy).".into(),
    };
    view! {
        <p class="hub-pins">
            "Probes pin the hub's key " <code>{short(&data.hub_key)}</code> " and its "
            {tls}
        </p>
    }
}

/// The rotation in progress.
#[component]
fn Progress(
    rotation: RotationInfo,
    can_manage: Signal<bool>,
    busy: RwSignal<bool>,
    on_switch: impl Fn() + Copy + Send + Sync + 'static,
    on_cancel: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let total = rotation.ready as usize + rotation.waiting.len();
    let not_ready = rotation.waiting.len();
    let confirming = RwSignal::new(false);
    let overdue = rotation.overdue.then(|| {
        view! {
            <div class="storage-warning-banner rotation-overdue">
                {format!(
                    "The {}-day window has passed and {not_ready} probe(s) still don't hold the new keys. \
                     Wait for them, or switch now: they will need enrolling again.",
                    rotation.window_days
                )}
            </div>
        }
    });
    let waiting = (!rotation.waiting.is_empty()).then(|| {
        let rows = rotation
            .waiting
            .iter()
            .map(|probe| {
                let href = format!("/probes/{}", probe.id);
                view! {
                    <tr>
                        <td><a href=href>{probe.name.clone()}</a></td>
                        <td>{probe.company_name.clone()}</td>
                        <td>{probe.last_seen_at.clone().unwrap_or_else(|| "never".into())}</td>
                        <td>{probe.reason.clone()}</td>
                    </tr>
                }
            })
            .collect_view();
        view! {
            <table class="admin-table rotation-waiting">
                <thead>
                    <tr><th>"Not ready"</th><th>"Company"</th><th>"Last report"</th><th>"Why"</th></tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        }
    });
    let switch_label = StoredValue::new(if not_ready == 0 {
        "Switch now".to_string()
    } else {
        format!("Switch now ({not_ready} not ready)")
    });
    view! {
        <div class="rotation-progress">
            <p class="rotation-status">
                {format!(
                    "Rotating the {}: {} of {total} probes hold the new keys. Started {} by {}; window until {}. \
                     The hub switches by itself once every probe holds them.",
                    what(&rotation),
                    rotation.ready,
                    rotation.started_at,
                    rotation.started_by.clone().unwrap_or_else(|| "?".into()),
                    rotation.deadline_at,
                )}
            </p>
            {overdue}
            {waiting}
            <Show when=move || can_manage.get()>
                <div class="rotation-actions">
                    <Show
                        when=move || confirming.get()
                        fallback=move || view! {
                            <button class="btn btn-primary" name="rotation_switch" disabled=busy
                                on:click=move |_| if not_ready == 0 { on_switch() } else { confirming.set(true) }>
                                {switch_label.get_value()}
                            </button>
                            " "
                            <button class="btn btn-secondary" name="rotation_cancel" disabled=busy
                                on:click=move |_| on_cancel()>"Cancel rotation"</button>
                        }
                    >
                        {format!("{not_ready} probe(s) will no longer reach the hub until enrolled again. Switch? ")}
                        <button class="btn btn-danger btn-sm" name="rotation_confirm" disabled=busy on:click=move |_| {
                            confirming.set(false);
                            on_switch();
                        }>"Switch"</button>
                        " "
                        <button class="btn btn-secondary btn-sm" on:click=move |_| confirming.set(false)>"No"</button>
                    </Show>
                </div>
            </Show>
        </div>
    }
}

/// "certificate", "key", or "certificate and key".
fn what(rotation: &RotationInfo) -> &'static str {
    match (rotation.rotates_tls, rotation.rotates_key) {
        (true, true) => "certificate and key",
        (true, false) => "certificate",
        _ => "key",
    }
}

/// Choose what to rotate and the window.
#[component]
fn StartForm(
    tls_enabled: bool,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    on_start: impl Fn(StartRotationRequest) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let tls = RwSignal::new(false);
    let key = RwSignal::new(false);
    let days = RwSignal::new(StartRotationRequest::DEFAULT_WINDOW_DAYS.to_string());
    #[cfg(not(feature = "ssr"))]
    let (on_tls, on_key, on_days) = (
        move |ev| tls.set(event_target_checked(&ev)),
        move |ev| key.set(event_target_checked(&ev)),
        move |ev| days.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_tls, on_key, on_days) = (|_| {}, |_| {}, |_| {});
    let submit = move |_| {
        let request = StartRotationRequest {
            tls: tls.get(),
            key: key.get(),
            window_days: days.get().trim().parse().unwrap_or(0),
        };
        match request.validate() {
            Ok(()) => on_start(request),
            Err(e) => error.set(format!("Not started: {e}.")),
        }
    };
    view! {
        <div class="rotation-start">
            <label class:disabled=!tls_enabled>
                <input type="checkbox" name="rotate_tls" disabled=!tls_enabled
                    prop:checked=move || tls.get() on:change=on_tls/>
                " Certificate"
            </label>
            <label>
                <input type="checkbox" name="rotate_key" prop:checked=move || key.get() on:change=on_key/>
                " Hub key"
            </label>
            <label>
                "window "
                <input type="number" name="rotate_window" min="1" max=StartRotationRequest::MAX_WINDOW_DAYS.to_string()
                    prop:value=move || days.get() on:input=on_days/>
                " days"
            </label>
            <button class="btn btn-primary btn-sm" name="rotation_start" disabled=busy on:click=submit>"Start rotation"</button>
        </div>
        <p class="info-text">
            "Probes get the new keys with their next report and keep the old ones until the switch. \
             Enrollment codes made before the switch stop working."
        </p>
    }
}

/// Earlier rotations.
#[component]
fn History(rotations: Vec<RotationInfo>) -> impl IntoView {
    if rotations.is_empty() {
        return nothing();
    }
    let rows = rotations
        .iter()
        .map(|r| {
            let outcome = match r.state {
                RotationState::Switched => "switched",
                RotationState::Cancelled => "cancelled",
                RotationState::Announcing => "in progress",
            };
            let by = r.ended_by.clone().unwrap_or_else(|| match r.state {
                RotationState::Switched => "the hub (all ready)".into(),
                _ => "?".into(),
            });
            view! {
                <tr>
                    <td>{what(r)}</td>
                    <td>{r.started_at.clone()}</td>
                    <td>{outcome}</td>
                    <td>{r.ended_at.clone().unwrap_or_default()}</td>
                    <td>{by}</td>
                </tr>
            }
        })
        .collect_view();
    any(view! {
        <table class="admin-table rotation-history">
            <thead>
                <tr><th>"Rotated"</th><th>"Started"</th><th>"Outcome"</th><th>"Ended"</th><th>"By"</th></tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    })
}
