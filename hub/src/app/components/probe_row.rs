//! One probe on the company page, expandable to its devices (with silencing,
//! except in an archive, which is read-only).

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_json, refresh_silences};
use crate::app::ui::{Me, any, fmt_ts, nothing, severity_label, status_class, status_label};
use crate::model::{AlertSilence, CompanyProbe, Permission, ProbeDevice, Status};

#[component]
pub(crate) fn ProbeRow(
    probe: CompanyProbe,
    probe_count: usize,
    /// `/api`, or `/api/archive/{id}` for an archive.
    api_base: String,
    /// An archive: no silencing.
    read_only: bool,
    silences: RwSignal<Vec<AlertSilence>>,
    silence_modal_open: RwSignal<bool>,
    silence_device: RwSignal<Option<ProbeDevice>>,
) -> impl IntoView {
    let me = Me::current();
    let is_single = probe_count == 1;
    let expanded = RwSignal::new(is_single);
    let cached_devices = RwSignal::new(Vec::<ProbeDevice>::new());
    let fetched = RwSignal::new(false);

    let probe_id_for_fetch = probe.probe_id.clone();

    Effect::new(move |_| {
        if expanded.get() && !fetched.get() {
            let _pid = probe_id_for_fetch.clone();
            let _api_base = api_base.clone();
            leptos::task::spawn_local(async move {
                #[cfg(not(feature = "ssr"))]
                {
                    let url = format!("{}/probe/{}/devices", _api_base, _pid);
                    if let Ok(devs) = fetch_json::<Vec<ProbeDevice>>(&url).await {
                        cached_devices.set(devs);
                    }
                }
                fetched.set(true);
            });
        }
    });

    let toggle = move |_: _| {
        expanded.update(|e| *e = !*e);
    };

    // Severity is computed by the server (crate::model::Severity::for_probe).
    let is_expired = !probe.is_active();
    let severity_text = severity_label(probe.severity);
    let severity_class = format!("status-badge severity-{}", probe.severity.as_str());
    let dot_class = format!("status-dot {}", probe.severity.as_str());
    let display_name = probe.display_name();
    let hostname_display = probe.hostname.clone().unwrap_or_else(|| "—".to_string());
    let site_display = probe.site.clone().unwrap_or_else(|| "—".to_string());
    let last_seen = probe
        .last_seen_at
        .as_deref()
        .map(fmt_ts)
        .unwrap_or_else(|| "Unknown".to_string());
    let probe_up = probe.devices_up;
    let probe_down = probe.devices_down;

    view! {
        <tr class="probe-summary-row">
            <td><a href=format!("/probes/{}", probe.probe_id) title="Checks">{display_name}</a></td>
            <td>{hostname_display}</td>
            <td>{site_display}</td>
            <td><span class=severity_class>{severity_text}</span></td>
            <td class={if is_expired { "stale-count" } else { "" }}>{probe_up}</td>
            <td class={if is_expired { "stale-count" } else { "" }}>{probe_down}</td>
            <td><span class=dot_class></span>{last_seen}</td>
            <td>
                {if !is_single {
                    any(view! {
                        <button class="expand-btn" on:click=toggle>
                            {move || if expanded.get() { "▲ Hide" } else { "▼ Show" }}
                        </button>
                    })
                } else {
                    nothing()
                }}
            </td>
        </tr>

        {move || -> AnyView {
            if expanded.get() {
                // Archives are read-only; elsewhere, only roles that may silence
                // see the buttons. Read here, inside the closure, so the table
                // is drawn again once the user has loaded: the devices can
                // arrive first, and the user is nobody until then.
                let read_only = read_only || !me.can(Permission::Silence);
                let mut devs = cached_devices.get();
                // Worst first: Down, Warning, Unknown, Up. Stable: the order within a status stays.
                devs.sort_by_key(|d| d.status.rank());
                any(view! {
                    <tr class="device-subtable-row">
                        <td colspan="8">
                            {if is_expired {
                                any(view! {
                                    <div class="probe-expired-banner">
                                        "\u{26A0} Probe expired \u{2014} device states may be outdated."
                                    </div>
                                })
                            } else {
                                nothing()
                            }}
                            <table class="device-table">
                                <thead>
                                    <tr>
                                        <th>"Name"</th>
                                        <th>"Target"</th>
                                        <th>"Status"</th>
                                        <th>"Latency"</th>
                                        <th>"Metric"</th>
                                        <th>"Message"</th>
                                        {(!read_only).then(|| view! {
                                            <th>"Alerts"</th>
                                            <th>"Actions"</th>
                                        })}
                                    </tr>
                                </thead>
                                <tbody>
                                    <For
                                        each=move || devs.clone()
                                        key=|d: &ProbeDevice| d.check_id.clone()
                                        children=move |dev: ProbeDevice| {
                                            let status_class = status_class(dev.status);
                                            let row_class = match dev.status {
                                                Status::Down => "device-row-down",
                                                Status::Warning => "device-row-warning",
                                                _ => "",
                                            };
                                            let status_label = status_label(dev.status);
                                            let latency_str = dev.latency_ms
                                                .map(|l| format!("{}ms", l))
                                                .unwrap_or_else(|| "—".to_string());
                                            let metric_str = dev.headline().unwrap_or_else(|| "—".to_string());
                                            // The hub's reason first (a threshold), else the probe's message.
                                            let msg = dev.reason.clone().or_else(|| dev.message.clone()).unwrap_or_else(|| "—".to_string());

                                            // Reactive is_silenced — re-evaluates whenever `silences` signal changes.
                                            // Signal::derive returns a Signal<bool> which is Copy,
                                            // so it can be used in multiple reactive closures.
                                            let check_id = dev.check_id.clone();
                                            let is_silenced = Signal::derive(move || {
                                                silence_of(&silences.get(), &check_id).is_some()
                                            });

                                            // Clone data for click handler
                                            let dev_for_click = dev.clone();

                                            // Toggle handler: unmute if silenced, open modal if not
                                            let silence_click = move |_| {
                                                match silence_of(&silences.get(), &dev_for_click.check_id) {
                                                    Some(sid) => {
                                                        // Optimistic: remove from local signal immediately
                                                        silences.update(|list| list.retain(|s| s.id != sid));
                                                        // Then confirm with the hub's list, which also
                                                        // restores the silence if the delete failed.
                                                        leptos::task::spawn_local(async move {
                                                            #[cfg(not(feature = "ssr"))]
                                                            {
                                                                let body = serde_json::json!({ "silence_id": sid });
                                                                if let Err(e) = post_json("/api/silence/delete", &body.to_string()).await {
                                                                    leptos::logging::error!("Unmute failed: {}", e);
                                                                }
                                                                refresh_silences(silences).await;
                                                            }
                                                        });
                                                    }
                                                    None => {
                                                        // Not silenced — open modal
                                                        silence_device.set(Some(dev_for_click.clone()));
                                                        silence_modal_open.set(true);
                                                    }
                                                }
                                            };

                                            view! {
                                                <tr class=row_class>
                                                    <td>{dev.name.clone()}</td>
                                                    <td>{dev.target.clone()}</td>
                                                    <td>
                                                        <span class=format!("dev-status {}", status_class)></span>
                                                        <strong>{status_label}</strong>
                                                        {dev.in_maintenance.then(|| view! {
                                                            " " <span class="badge badge-maintenance" title="Its latest result came during a maintenance window: no alerts">"maintenance"</span>
                                                        })}
                                                    </td>
                                                    <td>{latency_str}</td>
                                                    <td>{metric_str}</td>
                                                    <td>{msg}</td>
                                                    {(!read_only).then(|| view! {
                                                        <td class="silence-indicator">
                                                            {move || if is_silenced.get() { "🔕" } else { "" }}
                                                        </td>
                                                        <td>
                                                            <button
                                                                class=move || if is_silenced.get() { "btn-small btn-muted" } else { "btn-small" }
                                                                on:click=silence_click
                                                            >
                                                                {move || if is_silenced.get() { "\u{1F515} Muted" } else { "Silence" }}
                                                            </button>
                                                        </td>
                                                    })}
                                                </tr>
                                            }
                                        }
                                    />
                                </tbody>
                            </table>
                        </td>
                    </tr>
                })
            } else {
                nothing()
            }
        }}
    }
}

/// The ID of the silence on a check, if it is silenced.
fn silence_of(silences: &[AlertSilence], check_id: &str) -> Option<i64> {
    silences
        .iter()
        .find(|s| s.is_of_check(check_id))
        .map(|s| s.id)
}
