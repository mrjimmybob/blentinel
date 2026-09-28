//! One probe on the company page, expandable to its devices (with silencing,
//! except in an archive, which is read-only).

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_json, refresh_silences};
use crate::app::ui::{any, fmt_ts, nothing, severity_label};
use crate::model::{AlertSilence, CompanyProbe, ProbeDevice};

#[component]
pub(crate) fn ProbeRow(
    probe: CompanyProbe,
    company_id: String,
    probe_count: usize,
    /// `/api`, or `/api/archive/{id}` for an archive.
    api_base: String,
    /// An archive: no silencing.
    read_only: bool,
    silences: RwSignal<Vec<AlertSilence>>,
    silence_modal_open: RwSignal<bool>,
    silence_device: RwSignal<Option<ProbeDevice>>,
    silence_company_id: RwSignal<String>,
    silence_probe_id: RwSignal<String>,
) -> impl IntoView {
    let is_single = probe_count == 1;
    let expanded = RwSignal::new(is_single);
    let cached_devices = RwSignal::new(Vec::<ProbeDevice>::new());
    let fetched = RwSignal::new(false);

    let probe_id_for_fetch = probe.probe_id.clone();
    let company_id_for_view = company_id.clone();
    let probe_id_for_view = probe.probe_id.clone();

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
            <td>{display_name}</td>
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
                let mut devs = cached_devices.get();
                // Sort by severity: DOWN (0) first, then UP (2). Stable sort preserves original order within ranks.
                devs.sort_by(|a, b| {
                    let rank_a: u8 = if a.status == "Down" { 0 } else { 2 };
                    let rank_b: u8 = if b.status == "Down" { 0 } else { 2 };
                    rank_a.cmp(&rank_b)
                });
                let cid = company_id_for_view.clone();
                let pid = probe_id_for_view.clone();
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
                                        key=|d: &ProbeDevice| format!("{}-{}", d.name, d.target)
                                        children=move |dev: ProbeDevice| {
                                            let status_class = if dev.status == "Up" { "up" } else { "down" };
                                            let row_class = if dev.status == "Down" { "device-row-down" } else { "" };
                                            let status_label = dev.status.to_uppercase();
                                            let latency_str = dev.latency_ms
                                                .map(|l| format!("{}ms", l))
                                                .unwrap_or_else(|| "—".to_string());
                                            let metric_str = if let (Some(val), Some(unit)) = (dev.metric_value, &dev.metric_unit) {
                                                format!("{:.1}{}", val, unit)
                                            } else {
                                                "—".to_string()
                                            };
                                            let msg = dev.message.clone().unwrap_or_else(|| "—".to_string());

                                            // Reactive is_silenced — re-evaluates whenever `silences` signal changes.
                                            // Signal::derive returns a Signal<bool> which is Copy,
                                            // so it can be used in multiple reactive closures.
                                            let resource_key = crate::model::resource_key(&cid, &pid, &dev.name, &dev.target);
                                            let rk = resource_key.clone();
                                            let is_silenced = Signal::derive(move || {
                                                silences.get().iter().any(|s| s.scope_type == "resource" && s.scope_id == rk)
                                            });

                                            // Clone data for click handler
                                            let dev_for_click = dev.clone();
                                            let cid_for_click = cid.clone();
                                            let pid_for_click = pid.clone();
                                            let rk_for_unmute = resource_key.clone();

                                            // Toggle handler: unmute if silenced, open modal if not
                                            let silence_click = move |_| {
                                                if silences.get().iter().any(|s| s.scope_type == "resource" && s.scope_id == rk_for_unmute) {
                                                    // Find the silence ID and unmute
                                                    let silence_id = silences.get().iter()
                                                        .find(|s| s.scope_type == "resource" && s.scope_id == rk_for_unmute)
                                                        .map(|s| s.id);
                                                    if let Some(sid) = silence_id {
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
                                                } else {
                                                    // Not silenced — open modal
                                                    silence_company_id.set(cid_for_click.clone());
                                                    silence_probe_id.set(pid_for_click.clone());
                                                    silence_device.set(Some(dev_for_click.clone()));
                                                    silence_modal_open.set(true);
                                                }
                                            };

                                            view! {
                                                <tr class=row_class>
                                                    <td>{dev.name.clone()}</td>
                                                    <td>{dev.target.clone()}</td>
                                                    <td>
                                                        <span class=format!("dev-status {}", status_class)></span>
                                                        <strong>{status_label}</strong>
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
