//! One check over a period (M6-02, proposal 0007 §3.1): availability, MTTR
//! and incidents, the status band, a chart per metric, and the newest raw
//! results. Everyone who sees the check's company sees this page.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::app::components::{MetricChart, StatusBand, number};
use crate::app::ui::{any, status_class, status_label};
#[cfg(not(feature = "ssr"))]
use crate::app::{http::fetch_json, ui::input_value};
use crate::model::{CheckHistory, CheckResultEntry, HistoryRange, duration_text};

/// What the period picker asks for, as the query string.
fn range_query(range: HistoryRange) -> String {
    format!("range={}", range.param())
}

#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn CheckPage() -> impl IntoView {
    let params = use_params_map();
    let check_id = Signal::derive(move || params.get().get("check_id").unwrap_or_default());
    let query = RwSignal::new(range_query(HistoryRange::Day));
    let from_day = RwSignal::new(String::new());
    let to_day = RwSignal::new(String::new());
    let picker_error = RwSignal::new(String::new());

    let history: LocalResource<Result<CheckHistory, String>> = LocalResource::new(move || {
        let path = format!("/api/checks/{}/history?{}", check_id.get(), query.get());
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<CheckHistory>(&path).await
            }
            #[cfg(feature = "ssr")]
            {
                let _ = path;
                Err(String::new())
            }
        }
    });

    #[cfg(not(feature = "ssr"))]
    let (on_from, on_to) = (
        move |ev| from_day.set(input_value(&ev)),
        move |ev| to_day.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_from, on_to) = (|_| {}, |_| {});
    let show_days = move |_| {
        let (from, to) = (from_day.get_untracked(), to_day.get_untracked());
        if from.is_empty() || to.is_empty() {
            picker_error.set("Choose the first and the last day.".into());
            return;
        }
        // `YYYY-MM-DD` sorts as dates do. The hub checks the rest.
        if from > to {
            picker_error.set("The first day is after the last.".into());
            return;
        }
        picker_error.set(String::new());
        query.set(format!("from={from}&to={to}"));
    };

    let buttons = HistoryRange::ALL
        .into_iter()
        .map(|range| {
            let mine = range_query(range);
            let active = {
                let mine = mine.clone();
                move || query.get() == mine
            };
            view! {
                <button class="btn btn-sm" class:btn-primary=active.clone() class:btn-secondary=move || !active()
                    name=format!("range_{}", range.param())
                    on:click=move |_| {
                        picker_error.set(String::new());
                        query.set(mine.clone());
                    }>
                    {range.label()}
                </button>
            }
        })
        .collect_view();

    view! {
        <div class="page check-page">
            {move || -> AnyView {
                match history.get() {
                    Some(Ok(h)) => any(view! { <Heading h=h/> }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            <div class="range-picker">
                {buttons}
                <span class="range-days">
                    <input type="date" name="history_from" prop:value=move || from_day.get() on:input=on_from/>
                    " to "
                    <input type="date" name="history_to" prop:value=move || to_day.get() on:input=on_to/>
                    <button class="btn btn-sm btn-secondary" name="history_days" on:click=show_days>"Show"</button>
                </span>
            </div>
            {move || {
                let e = picker_error.get();
                (!e.is_empty()).then(|| view! { <div class="error-message">{e}</div> })
            }}
            {move || -> AnyView {
                match history.get() {
                    Some(Ok(h)) => any(view! { <Body h=h/> }),
                    _ => ().into_any(),
                }
            }}
        </div>
    }
}

/// Name, where it is, what it checks.
#[component]
fn Heading(h: CheckHistory) -> impl IntoView {
    view! {
        <p class="breadcrumb">
            <a href=format!("/company/{}", h.company_id)>{h.company_name.clone()}</a>
            {(!h.site_name.is_empty()).then(|| format!(" › {}", h.site_name))}
            " › "
            <a href=format!("/probes/{}", h.probe_id)>{h.probe_name.clone()}</a>
        </p>
        <h1 class="page-title">{h.name.clone()}</h1>
        <p class="probe-meta">
            {h.kind.clone()} " · " {h.target.clone()}
            {(!h.enabled).then(|| view! { " · " <strong>"Disabled"</strong> })}
        </p>
    }
}

/// Everything for the period.
#[component]
fn Body(h: CheckHistory) -> impl IntoView {
    let availability = h
        .availability_pct
        .map_or_else(|| "—".to_string(), |pct| format!("{}%", number(pct)));
    let mttr = h.mttr_secs.map_or_else(|| "—".to_string(), duration_text);
    let t = h.totals;
    let breakdown = format!(
        "{} results: Up {}, Warning {}, Down {}, Unknown {}, in maintenance {}",
        t.total(),
        t.up,
        t.warning,
        t.down,
        t.unknown,
        t.maintenance
    );
    let step = match h.bucket_secs {
        300 => "5 minutes",
        3600 => "hour",
        _ => "day",
    };
    let charts = h
        .series
        .iter()
        .cloned()
        .map(|series| {
            let title = if series.unit.is_empty() {
                series.key.clone()
            } else {
                format!("{} ({})", series.key, series.unit)
            };
            let h = h.clone();
            view! {
                <div class="admin-section metric-section">
                    <h2>{title}</h2>
                    <MetricChart series=series history=h/>
                </div>
            }
        })
        .collect_view();

    view! {
        <div class="history-summary">
            <div class="summary-tile" title="Up and Warning results of those that were Up, Warning or Down. Unknown results and those during maintenance don't count.">
                <span class="tile-label">"Availability"</span>
                <span class="tile-value availability">{availability}</span>
            </div>
            <div class="summary-tile" title="Incidents that opened in the period">
                <span class="tile-label">"Incidents"</span>
                <span class="tile-value incident-count">{h.incident_count}</span>
            </div>
            <div class="summary-tile" title="Mean time to recover: how long the incidents that closed in the period lasted, on average">
                <span class="tile-label">"MTTR"</span>
                <span class="tile-value mttr">{mttr}</span>
            </div>
        </div>
        <div class="admin-section">
            <h2>"Status"</h2>
            <p class="info-text">
                {format!("{} UTC to {} UTC, a block for each {step}; each shows its worst result. ", h.from, h.to)}
                {breakdown}
            </p>
            <StatusBand history=h.clone()/>
        </div>
        {charts}
        <Incidents h=h.clone()/>
        <RecentResults results=h.recent.clone()/>
    }
}

/// The incidents open at some time in the period.
#[component]
fn Incidents(h: CheckHistory) -> impl IntoView {
    view! {
        <div class="admin-section">
            <h2>"Incidents"</h2>
            {if h.incidents.is_empty() {
                any(view! { <p class="empty-state">"No incidents in this period."</p> })
            } else {
                any(view! {
                    <table class="admin-table incidents-in-period">
                        <thead><tr><th>"#"</th><th>"Worst"</th><th>"Opened (UTC)"</th><th>"Closed (UTC)"</th><th>"Lasted"</th></tr></thead>
                        <tbody>
                            {h.incidents.iter().rev().map(|i| {
                                let lasted = i.closed_unix.map_or_else(
                                    || "still open".to_string(),
                                    |closed| duration_text(closed - i.opened_unix),
                                );
                                view! {
                                    <tr>
                                        <td><a href=format!("/incidents/{}", i.id)>{format!("#{}", i.id)}</a></td>
                                        <td class=format!("check-result {}", status_class(i.severity))>{status_label(i.severity)}</td>
                                        <td>{i.opened_at.clone()}</td>
                                        <td>{i.closed_at.clone().unwrap_or_else(|| "—".into())}</td>
                                        <td>{lasted}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                })
            }}
        </div>
    }
}

/// The newest raw results, whatever the period.
#[component]
fn RecentResults(results: Vec<CheckResultEntry>) -> impl IntoView {
    view! {
        <div class="admin-section">
            <h2>"Recent results"</h2>
            <p class="info-text">"The newest results as the probe sent them, newest first. Raw results are kept for a shorter time than the history above (Admin › History)."</p>
            {if results.is_empty() {
                any(view! { <p class="empty-state">"No results kept."</p> })
            } else {
                any(view! {
                    <table class="admin-table checks-table recent-results">
                        <thead><tr><th>"When (UTC)"</th><th>"Status"</th><th>"Response"</th><th>"Values"</th><th>"Message"</th></tr></thead>
                        <tbody>
                            {results.into_iter().map(|r| {
                                let values = r.metrics
                                    .iter()
                                    .map(|m| format!("{} {}{}", m.key, number(m.value), m.unit))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let message = [r.reason.clone(), r.message.clone()]
                                    .into_iter()
                                    .flatten()
                                    .filter(|m| !m.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" · ");
                                view! {
                                    <tr>
                                        <td>{r.at.clone()}</td>
                                        <td class=format!("check-result {}", status_class(r.status))>
                                            {status_label(r.status)}
                                            {r.in_maintenance.then(|| view! {
                                                " " <span class="badge badge-maintenance" title="It came during a maintenance window">"maintenance"</span>
                                            })}
                                        </td>
                                        <td>{r.latency_ms.map(|ms| format!("{ms} ms")).unwrap_or_default()}</td>
                                        <td>{values}</td>
                                        <td>{message}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                })
            }}
        </div>
    }
}
