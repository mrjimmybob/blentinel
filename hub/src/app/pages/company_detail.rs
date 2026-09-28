//! Company detail: its probes, their devices, and the uptime chart. Also
//! serves `/archive/{archive_id}/company/{company_id}`, read-only (D-26).

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::app::components::{ProbeRow, UptimeChart};
use crate::app::ui::input_value;
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{UNAUTHORIZED, fetch_json, post_json, refresh_silences},
};
use crate::model::{AlertSilence, CompanyProbe, ProbeDevice, UptimeBucket};

#[component]
pub(crate) fn CompanyDetailPage() -> impl IntoView {
    let params = use_params_map();
    let company_id = Signal::derive(move || params.get().get("company_id").unwrap_or_default());
    // Set when viewing an archive: read-only, and nothing to poll.
    let archive_id = Signal::derive(move || params.get().get("archive_id"));
    let api_base = Signal::derive(move || match archive_id.get() {
        Some(id) => format!("/api/archive/{id}"),
        None => "/api".to_string(),
    });
    let read_only = Signal::derive(move || archive_id.get().is_some());

    // Range selector state — defaults to "24h"
    let selected_range = RwSignal::new("24h".to_string());

    // ---------------------------------------------------------------------------
    // Silence modal state — lifted here so the modal renders at page root,
    // outside all <table> elements.  Signals are passed down to ProbeRow.
    // ---------------------------------------------------------------------------
    let silence_modal_open = RwSignal::new(false);
    let silence_device = RwSignal::new(None::<ProbeDevice>);
    let silence_company_id = RwSignal::new(String::new());
    let silence_probe_id = RwSignal::new(String::new());
    let silence_reason = RwSignal::new(String::new());
    let silence_duration = RwSignal::new(String::from("24"));

    // Shared silences signal — all probe rows read from this
    let silences = RwSignal::new(Vec::<AlertSilence>::new());

    // Fetch active silences once on mount (archives have none)
    Effect::new(move |_| {
        if read_only.get_untracked() {
            return;
        }
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            refresh_silences(silences).await;
        });
    });

    let probes_data = RwSignal::new(Vec::<CompanyProbe>::new());
    let uptime_data = RwSignal::new(Vec::<UptimeBucket>::new());
    // Always start loading — SSR and client must agree on initial DOM to avoid
    // hydration mismatch (which causes Leptos to tear down and rebuild the component).
    let is_initial_loading = RwSignal::new(true);
    let probes_fetch_error: RwSignal<Option<String>> = RwSignal::new(None);

    // Initial fetch + polling interval. RwSignal never has a None/pending state, so
    // Memos reading it never cause the owning scope to suspend — no page reload on refetch.
    #[cfg(not(feature = "ssr"))]
    let urls = move || {
        let base = format!(
            "{}/company/{}",
            api_base.get_untracked(),
            company_id.get_untracked()
        );
        let range = selected_range.get_untracked();
        (
            format!("{base}/probes"),
            format!("{base}/uptime?range={range}"),
        )
    };
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            if company_id.get_untracked().is_empty() {
                is_initial_loading.set(false);
                return;
            }
            let (probe_url, uptime_url) = urls();
            leptos::task::spawn_local(async move {
                match fetch_json::<Vec<CompanyProbe>>(&probe_url).await {
                    Ok(list) => {
                        probes_data.set(list);
                        is_initial_loading.set(false);
                    }
                    Err(e) if e == UNAUTHORIZED => {
                        browser::navigate("/login");
                    }
                    Err(e) => {
                        probes_fetch_error.set(Some(e));
                        is_initial_loading.set(false);
                    }
                }
            });
            leptos::task::spawn_local(async move {
                if let Ok(buckets) = fetch_json::<Vec<UptimeBucket>>(&uptime_url).await {
                    uptime_data.set(buckets);
                }
            });
            if read_only.get_untracked() {
                return; // an archive never changes
            }
            browser::poll_every(15_000, move || {
                let (probe_url2, uptime_url2) = urls();
                leptos::task::spawn_local(async move {
                    if let Ok(list) = fetch_json::<Vec<CompanyProbe>>(&probe_url2).await {
                        probes_data.set(list);
                    }
                });
                leptos::task::spawn_local(async move {
                    if let Ok(buckets) = fetch_json::<Vec<UptimeBucket>>(&uptime_url2).await {
                        uptime_data.set(buckets);
                    }
                });
            });
        }
    });

    // Re-fetch uptime immediately when the user changes the range selector.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            selected_range.track(); // re-runs on every range change
            if company_id.get_untracked().is_empty() {
                return;
            }
            let (_, url) = urls();
            leptos::task::spawn_local(async move {
                if let Ok(buckets) = fetch_json::<Vec<UptimeBucket>>(&url).await {
                    uptime_data.set(buckets);
                }
            });
        }
    });

    // Dynamic title based on selected range
    // In an archive the range ends at the archive's last report, not now.
    let chart_title = Signal::derive(move || {
        let span = match selected_range.get().as_str() {
            "7d" => "Last 7 Days",
            "30d" => "Last 30 Days",
            "all" => "All Data",
            _ => "Last 24 Hours",
        };
        let of_archive = if read_only.get() {
            " of the Archive"
        } else {
            ""
        };
        format!("Uptime \u{2014} {span}{of_archive}")
    });

    let uptime_memo = Memo::new(move |_| uptime_data.get());
    let probes_memo = Memo::new(move |_| probes_data.get());
    let is_loading: Memo<bool> = Memo::new(move |_| is_initial_loading.get());
    let probes_error: Memo<Option<String>> = Memo::new(move |_| probes_fetch_error.get());

    view! {
        {move || archive_id.get().map(|_| view! {
            <div class="archive-banner">"📂 HISTORICAL VIEW — READ ONLY ARCHIVE"</div>
        })}
        <div class="breadcrumb">
            <a href="/">"Dashboard"</a>
            <span class="sep">"›"</span>
            {move || archive_id.get().map(|id| view! {
                <a href=format!("/archive/{id}")>{format!("Archive {id}")}</a>
                <span class="sep">"›"</span>
            })}
            {move || company_id.get()}
            {move || archive_id.get().is_none().then(|| view! {
                <a class="breadcrumb-action" href=format!("/companies/{}", company_id.get())>"Company details"</a>
            })}
        </div>

        // Uptime chart — re-renders only when uptime_memo fires (data actually changed).
        <Show when=move || !uptime_memo.get().is_empty()>
            {move || view! {
                <UptimeChart buckets=uptime_memo.get() title=chart_title selected_range=selected_range/>
            }}
        </Show>

        <div class="section-header">
            <h2>"Probes"</h2>
        </div>

        <Show when=move || is_loading.get()>
            <div class="loading">"Loading…"</div>
        </Show>
        <Show when=move || probes_error.get().is_some()>
            <div class="error">{move || probes_error.get().unwrap_or_default()}</div>
        </Show>
        <Show when=move || !is_loading.get() && probes_error.get().is_none() && probes_memo.get().is_empty()>
            <div class="empty-state">"No probes found for this company."</div>
        </Show>
        <Show when=move || !probes_memo.get().is_empty()>
            <div class="probe-table-wrap">
                <table class="probe-table">
                    <thead>
                        <tr>
                            <th>"Probe"</th>
                            <th>"Hostname"</th>
                            <th>"Site"</th>
                            <th>"Status"</th>
                            <th>"Up"</th>
                            <th>"Down"</th>
                            <th>"Last Seen"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=move || probes_memo.get()
                            key=|p: &CompanyProbe| p.probe_id.clone()
                            children=move |probe: CompanyProbe| {
                                let cid = company_id.get();
                                let probe_count = probes_memo.get().len();
                                view! {
                                    <ProbeRow
                                        probe=probe
                                        company_id=cid
                                        probe_count=probe_count
                                        api_base=api_base.get_untracked()
                                        read_only=read_only.get_untracked()
                                        silences=silences
                                        silence_modal_open=silence_modal_open
                                        silence_device=silence_device
                                        silence_company_id=silence_company_id
                                        silence_probe_id=silence_probe_id
                                    />
                                }
                            }
                        />
                    </tbody>
                </table>
            </div>
        </Show>

        // -------------------------------------------------------------------
        // Silence modal — rendered at page root, outside all tables.
        // Uses the existing .modal-overlay / .modal CSS classes.
        // -------------------------------------------------------------------
        <Show when=move || silence_modal_open.get()>
            {move || {
                let dev = silence_device.get();
                let dev_name = dev.as_ref().map(|d| d.name.clone()).unwrap_or_default();

                view! {
                    <div class="modal-overlay" on:click=move |_| silence_modal_open.set(false)>
                        <div class="modal" on:click=|e| e.stop_propagation()>
                            <h3>"Silence Alerts"</h3>
                            <p>"Device: " <strong>{dev_name}</strong></p>

                            <div class="form-group">
                                <label>"Reason:"</label>
                                <input
                                    type="text"
                                    placeholder="e.g., Planned maintenance"
                                    prop:value=silence_reason
                                    on:input=move |ev| {
                                        silence_reason.set(input_value(&ev));
                                    }
                                />
                            </div>

                            <div class="form-group">
                                <label>"Duration:"</label>
                                <select
                                    prop:value=silence_duration
                                    on:change=move |ev| {
                                        silence_duration.set(input_value(&ev));
                                    }
                                >
                                    <option value="1">"1 hour"</option>
                                    <option value="24" selected>"24 hours"</option>
                                    <option value="168">"7 days"</option>
                                    <option value="forever">"Forever"</option>
                                </select>
                            </div>

                            <div class="modal-actions">
                                <button class="btn btn-secondary" on:click=move |_| silence_modal_open.set(false)>
                                    "Cancel"
                                </button>
                                <button class="btn btn-primary" on:click=move |_| {
                                    // Validate reason before submitting
                                    if silence_reason.get().trim().is_empty() {
                                        return;
                                    }

                                    leptos::task::spawn_local(async move {
                                        #[cfg(not(feature = "ssr"))]
                                        {
                                            let Some(dev) = silence_device.get() else { return; };
                                            let cid = silence_company_id.get();
                                            let pid = silence_probe_id.get();
                                            let reason = silence_reason.get();

                                            let resource_key = crate::model::resource_key(&cid, &pid, &dev.name, &dev.target);

                                            let duration_str = silence_duration.get();
                                            let duration_hours: Option<u32> = if duration_str == "forever" {
                                                None
                                            } else {
                                                duration_str.parse().ok()
                                            };

                                            let body = serde_json::json!({
                                                "resource_key": resource_key,
                                                "reason": reason,
                                                "duration_hours": duration_hours
                                            });

                                            match post_json("/api/silence", &body.to_string()).await {
                                                Ok(()) => {
                                                    silence_modal_open.set(false);
                                                    // Reload from the hub: unmuting needs the silence's real ID.
                                                    refresh_silences(silences).await;
                                                }
                                                Err(e) => {
                                                    // Keep modal open on error
                                                    leptos::logging::error!("Silence failed: {}", e);
                                                }
                                            }
                                        }
                                    });
                                }>
                                    "Silence"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            }}
        </Show>
    }
}
