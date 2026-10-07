//! Incidents (M3-05, D-67): the list (open or closed) and one incident with
//! its timeline, acknowledging it and adding notes. Everyone sees the
//! incidents of their companies; Technicians and up act on them.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, fmt_ts, status_class, status_label};
use crate::model::{
    IncidentDetail, IncidentEvent, IncidentEventKind, IncidentInfo, IncidentNote, Permission,
};

/// The badge of an incident: its status while open, else CLOSED.
fn badge(incident: &IncidentInfo) -> impl IntoView + use<> {
    let (class, text) = match incident.is_open() {
        true => (
            format!("incident-badge {}", status_class(incident.status)),
            status_label(incident.status),
        ),
        false => ("incident-badge closed".to_string(), "CLOSED".to_string()),
    };
    view! { <span class=class>{text}</span> }
}

#[component]
pub(crate) fn IncidentsPage() -> impl IntoView {
    // `open`, `closed` or `all`.
    let state = RwSignal::new("open".to_string());
    let incidents: LocalResource<Result<Vec<IncidentInfo>, String>> =
        LocalResource::new(move || {
            let state = state.get();
            async move {
                #[cfg(not(feature = "ssr"))]
                {
                    fetch_json::<Vec<IncidentInfo>>(&format!("/api/incidents?state={state}")).await
                }
                #[cfg(feature = "ssr")]
                {
                    let _ = state;
                    Err(String::new())
                }
            }
        });
    let tab = move |value: &'static str, label: &'static str| {
        view! {
            <button
                class="btn btn-secondary btn-sm"
                class:active=move || state.get() == value
                name=format!("incidents_{value}")
                on:click=move |_| state.set(value.to_string())
            >
                {label}
            </button>
        }
    };

    view! {
        <h1 class="page-title">"Incidents"</h1>
        <p class="info-text">
            "Each outage of a check, or of a probe that stopped reporting, from its first alert to its recovery."
        </p>
        <div class="incident-tabs">{tab("open", "Open")}{tab("closed", "Closed")}{tab("all", "All")}</div>
        {move || -> AnyView {
            match incidents.get() {
                Some(Ok(list)) if list.is_empty() => any(view! {
                    <div class="empty-state">
                        {if state.get() == "open" { "No open incidents: all is well." } else { "No incidents." }}
                    </div>
                }),
                Some(Ok(list)) => any(view! {
                    <table class="admin-table incidents-table">
                        <thead>
                            <tr><th></th><th>"What"</th><th>"Where"</th><th>"Since"</th><th>"For"</th><th>"Acknowledged"</th></tr>
                        </thead>
                        <tbody>
                            {list.into_iter().map(|i| view! {
                                <tr>
                                    <td>{badge(&i)}</td>
                                    <td><a href=format!("/incidents/{}", i.id)>{i.title()}</a></td>
                                    <td>{i.place()}</td>
                                    <td>{fmt_ts(&i.opened_at)}</td>
                                    <td>{i.duration.clone()}</td>
                                    <td>{i.acknowledged_by.clone().unwrap_or_else(|| "—".into())}</td>
                                </tr>
                            }).collect_view()}
                        </tbody>
                    </table>
                }),
                Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                None => any(view! { <div class="loading">"Loading…"</div> }),
            }
        }}
    }
}

fn event_line(event: IncidentEvent) -> impl IntoView {
    let who = event.username.clone().unwrap_or_else(|| "Hub".into());
    let what = match event.kind {
        IncidentEventKind::Opened => "Opened",
        IncidentEventKind::Changed => "Changed",
        IncidentEventKind::Acknowledged => "Acknowledged",
        IncidentEventKind::Note => "Note",
        IncidentEventKind::Closed => "Closed",
    };
    view! {
        <li class=format!("incident-event {}", event.kind.as_str())>
            <span class="incident-event-at">{fmt_ts(&event.at)}</span>
            <span class="incident-event-kind">{what}</span>
            <span class="incident-event-who">{who}</span>
            <span class="incident-event-text">{event.text}</span>
        </li>
    }
}

#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
pub(crate) fn IncidentPage() -> impl IntoView {
    let params = use_params_map();
    let id = Signal::derive(move || params.get().get("incident_id").unwrap_or_default());
    let me = Me::current();
    let can_act = Signal::derive(move || me.can(Permission::Silence));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let note = RwSignal::new(String::new());

    let detail: LocalResource<Result<IncidentDetail, String>> = LocalResource::new(move || {
        refresh.track();
        let id = id.get();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<IncidentDetail>(&format!("/api/incidents/{id}")).await
            }
            #[cfg(feature = "ssr")]
            {
                let _ = id;
                Err(String::new())
            }
        }
    });

    // POST the note to `what` (`acknowledge` or `notes`), then reload.
    let send = move |what: &'static str| {
        error.set(String::new());
        let body = IncidentNote {
            text: note.get_untracked(),
        };
        if let Err(e) = body.validate(what == "notes") {
            error.set(format!("Not saved: {e}."));
            return;
        }
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let url = format!("/api/incidents/{}/{what}", id.get_untracked());
                let body = serde_json::to_string(&body).unwrap_or_default();
                match post_for::<IncidentInfo>(&url, &body).await {
                    Ok(_) => {
                        note.set(String::new());
                        refresh.update(|n| *n += 1);
                    }
                    Err(e) => error.set(e),
                }
            }
            #[cfg(feature = "ssr")]
            let _ = (what, body);
        });
    };
    #[cfg(not(feature = "ssr"))]
    let on_note = move |ev| note.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_note = |_| {};

    view! {
        <p><a href="/incidents">"← Incidents"</a></p>
        {move || -> AnyView {
            match detail.get() {
                Some(Ok(IncidentDetail { incident, events })) => {
                    let open = incident.is_open();
                    let acknowledged = incident.acknowledged_by.is_some();
                    any(view! {
                        <h1 class="page-title">{badge(&incident)}" "{incident.title()}</h1>
                        <div class="incident-facts">
                            <div>
                                <a href=format!("/company/{}", incident.company_id)>{incident.company_name.clone()}</a>
                                " › "{incident.site_name.clone()}" › "
                                <a href=format!("/probes/{}", incident.probe_id)>{incident.probe_name.clone()}</a>
                            </div>
                            <div>"Opened "{fmt_ts(&incident.opened_at)}
                                {incident.closed_at.as_deref().map(|c| format!(", closed {}", fmt_ts(c)))}
                                " · "{incident.duration.clone()}
                                " · worst: "{status_label(incident.severity)}
                            </div>
                            <div class="incident-ack">
                                {match &incident.acknowledged_by {
                                    Some(by) => format!(
                                        "Acknowledged by {by} at {}",
                                        fmt_ts(incident.acknowledged_at.as_deref().unwrap_or_default())
                                    ),
                                    None if open => "Not acknowledged".to_string(),
                                    None => String::new(),
                                }}
                            </div>
                        </div>
                        <Show when=move || can_act.get()>
                            <div class="incident-actions">
                                <textarea name="incident_note" rows="2"
                                    placeholder=if open && !acknowledged { "A note (optional for acknowledging)" } else { "A note" }
                                    prop:value=move || note.get() on:input=on_note></textarea>
                                <div class="form-actions">
                                    {(open && !acknowledged).then(|| view! {
                                        <button class="btn btn-primary btn-sm" name="incident_acknowledge"
                                            on:click=move |_| send("acknowledge")>"Acknowledge"</button>
                                    })}
                                    <button class="btn btn-secondary btn-sm" name="incident_add_note"
                                        on:click=move |_| send("notes")>"Add note"</button>
                                </div>
                            </div>
                        </Show>
                        <div class="login-error">{error}</div>
                        <h2>"Timeline"</h2>
                        <ul class="incident-timeline">
                            {events.into_iter().map(event_line).collect_view()}
                        </ul>
                    })
                }
                Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                None => any(view! { <div class="loading">"Loading…"</div> }),
            }
        }}
    }
}
