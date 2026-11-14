//! The monthly availability report (M6-05, proposal 0007 §3.4, D-113): a
//! company's month per site and check, its incidents and maintenance, to
//! print (or save as PDF from the print dialog) or download as CSV. Every
//! role that sees the company sees it, a Client too.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::app::components::number;
use crate::app::ui::{any, status_class, status_label};
#[cfg(not(feature = "ssr"))]
use crate::app::{browser, http::fetch_json, ui::input_value};
use crate::model::{CompanyInfo, MonthlyReport, SiteReport, duration_text};

/// `99.5%`, or a dash without results.
fn pct(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |p| format!("{}%", number(p)))
}

/// The change from the month before, in percentage points.
pub(crate) fn trend(now: Option<f64>, before: Option<f64>) -> String {
    match (now, before) {
        (Some(now), Some(before)) => {
            let change = now - before;
            if change.abs() < 0.005 {
                "=".to_string()
            } else {
                format!(
                    "{}{} pt",
                    if change > 0.0 { "+" } else { "−" },
                    number(change.abs())
                )
            }
        }
        _ => "—".to_string(),
    }
}

/// Below the report's target?
fn below(target: Option<f64>, availability: Option<f64>) -> bool {
    matches!((target, availability), (Some(t), Some(a)) if a < t)
}

/// `/reports`: the companies the user sees, each linking to its report.
#[component]
pub(crate) fn ReportsPage() -> impl IntoView {
    let companies: LocalResource<Result<Vec<CompanyInfo>, String>> = LocalResource::new(|| async {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<Vec<CompanyInfo>>("/api/companies").await
        }
        #[cfg(feature = "ssr")]
        {
            Err(String::new())
        }
    });
    view! {
        <div class="page reports-index">
            <h1 class="page-title">"Reports"</h1>
            <p class="info-text">"Each company's monthly availability: per site and per check, the month before, incidents and maintenance. Print it, save it as PDF, or download it as CSV."</p>
            {move || -> AnyView {
                match companies.get() {
                    Some(Ok(list)) if list.is_empty() => any(view! { <p class="empty-state">"No companies."</p> }),
                    Some(Ok(list)) => any(view! {
                        <ul class="report-companies">
                            {list.into_iter().filter(|c| !c.archived).map(|c| view! {
                                <li><a href=format!("/company/{}/report", c.id)>{c.name}</a></li>
                            }).collect_view()}
                        </ul>
                    }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
        </div>
    }
}

/// `/company/{id}/report`.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn ReportPage() -> impl IntoView {
    let params = use_params_map();
    let company_id = Signal::derive(move || params.get().get("company_id").unwrap_or_default());
    // Empty: the hub's default, last month.
    let month = RwSignal::new(String::new());
    let target = RwSignal::new(String::new());
    let query = RwSignal::new(String::new());

    let report: LocalResource<Result<MonthlyReport, String>> = LocalResource::new(move || {
        let path = format!("/api/companies/{}/report?{}", company_id.get(), query.get());
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<MonthlyReport>(&path).await
            }
            #[cfg(feature = "ssr")]
            {
                let _ = path;
                Err(String::new())
            }
        }
    });
    // Show the month the hub chose.
    Effect::new(move |_| {
        if let Some(Ok(r)) = report.get()
            && month.get_untracked().is_empty()
        {
            month.set(r.month.param());
        }
    });

    #[cfg(not(feature = "ssr"))]
    let (on_month, on_target, print) = (
        move |ev| month.set(input_value(&ev)),
        move |ev| target.set(input_value(&ev)),
        move |_| browser::print(),
    );
    #[cfg(feature = "ssr")]
    let (on_month, on_target, print) = (|_| {}, |_| {}, |_| {});
    let show = move |_| {
        let mut parts = Vec::new();
        let m = month.get_untracked();
        if !m.is_empty() {
            parts.push(format!("month={m}"));
        }
        let t = target.get_untracked();
        if !t.trim().is_empty() {
            parts.push(format!("target={}", t.trim()));
        }
        query.set(parts.join("&"));
    };
    let csv_href = move || {
        let m = month.get();
        format!(
            "/api/companies/{}/report.csv{}",
            company_id.get(),
            if m.is_empty() {
                String::new()
            } else {
                format!("?month={m}")
            }
        )
    };

    view! {
        <div class="page report-page">
            <div class="report-controls">
                <label>"Month " <input type="month" name="report_month" prop:value=move || month.get() on:input=on_month/></label>
                <label>"Target " <input type="number" name="report_target" min="0" max="100" step="0.01" placeholder="e.g. 99.9"
                    prop:value=move || target.get() on:input=on_target/> " %"</label>
                <button class="btn btn-primary btn-sm" name="report_show" on:click=show>"Show"</button>
                <button class="btn btn-secondary btn-sm" name="report_print" on:click=print title="Print it, or save it as PDF">"Print"</button>
                <a class="btn btn-secondary btn-sm" href=csv_href download="">"Download CSV"</a>
            </div>
            {move || -> AnyView {
                match report.get() {
                    Some(Ok(r)) => any(view! { <Report r=r/> }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
        </div>
    }
}

#[component]
fn Report(r: MonthlyReport) -> impl IntoView {
    let previous = r.month.previous().label();
    let notes = [
        r.partial.then(|| "The month isn't over: these are the figures so far.".to_string()),
        (!r.exact).then(|| {
            "This month is older than the hourly history is kept, so it was read from daily (UTC) summaries: a site's month is only roughly its local month.".to_string()
        }),
        r.target_pct.map(|t| format!("Rows below the target of {}% are marked.", number(t))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let t = r.totals;
    view! {
        <h1 class="page-title">{format!("{} — {}", r.company_name, r.month.label())}</h1>
        <p class="probe-meta">{format!("Availability report · generated {} UTC", r.generated_at)}</p>
        {notes.into_iter().map(|n| view! { <p class="info-text report-note">{n}</p> }).collect_view()}
        <div class="history-summary">
            <div class=move || if below(r.target_pct, r.availability_pct) { "summary-tile below-target" } else { "summary-tile" }>
                <span class="tile-label">"Availability"</span>
                <span class="tile-value availability">{pct(r.availability_pct)}</span>
                <span class="tile-label">{format!("{} vs {previous}", trend(r.availability_pct, r.previous_pct))}</span>
            </div>
            <div class="summary-tile">
                <span class="tile-label">"Incidents"</span>
                <span class="tile-value incident-count">{r.incident_count}</span>
            </div>
            <div class="summary-tile" title="How long checks were in an incident during the month, added up">
                <span class="tile-label">"Downtime"</span>
                <span class="tile-value downtime">{duration_text(r.downtime_secs)}</span>
            </div>
            <div class="summary-tile">
                <span class="tile-label">"Results"</span>
                <span class="tile-value">{t.total()}</span>
                <span class="tile-label">{format!("Down {}, Warning {}, in maintenance {}", t.down, t.warning, t.maintenance)}</span>
            </div>
        </div>
        <p class="info-text">"Availability is the Up and Warning results of those that were Up, Warning or Down; Unknown results and those during maintenance don't count. Each site's month is in its own time zone."</p>
        {r.sites.iter().cloned().map(|site| view! { <Site site=site target=r.target_pct/> }).collect_view()}
        <div class="admin-section">
            <h2>"Incidents"</h2>
            {if r.incidents.is_empty() {
                any(view! { <p class="empty-state">"No incidents this month."</p> })
            } else {
                any(view! {
                    <table class="admin-table report-incidents">
                        <thead><tr><th>"#"</th><th>"Where"</th><th>"Worst"</th><th>"Opened (UTC)"</th><th>"Closed (UTC)"</th><th>"Lasted"</th><th>"Cause"</th></tr></thead>
                        <tbody>
                            {r.incidents.iter().map(|i| {
                                let what = i.check_name.clone().unwrap_or_else(|| "the probe stopped reporting".into());
                                view! {
                                    <tr>
                                        <td><a href=format!("/incidents/{}", i.id)>{format!("#{}", i.id)}</a></td>
                                        <td>{format!("{} › {} › {what}", i.site_name, i.probe_name)}</td>
                                        <td class=format!("check-result {}", status_class(i.severity))>{status_label(i.severity)}</td>
                                        <td>{i.opened_at.clone()}</td>
                                        <td>{i.closed_at.clone().unwrap_or_else(|| "still open".into())}</td>
                                        <td>{duration_text(i.duration_secs)}</td>
                                        <td>{i.cause.clone()}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                })
            }}
        </div>
        <div class="admin-section">
            <h2>"Maintenance"</h2>
            {if r.maintenance.is_empty() {
                any(view! { <p class="empty-state">"No maintenance windows opened this month."</p> })
            } else {
                any(view! {
                    <table class="admin-table report-maintenance">
                        <thead><tr><th>"From"</th><th>"To"</th><th>"Where"</th><th>"Why"</th></tr></thead>
                        <tbody>
                            {r.maintenance.iter().map(|m| view! {
                                <tr>
                                    <td>{format!("{} ({})", m.start, m.timezone)}</td>
                                    <td>{m.end.clone()}</td>
                                    <td>{m.scope.clone()}</td>
                                    <td>{m.reason.clone()}</td>
                                </tr>
                            }).collect_view()}
                        </tbody>
                    </table>
                })
            }}
            <p class="info-text">"The windows as they are set now: one deleted since isn't listed. Results during maintenance are counted in each row."</p>
        </div>
    }
}

/// One site: its month and its checks.
#[component]
fn Site(site: SiteReport, target: Option<f64>) -> impl IntoView {
    let heading = if site.site_id.is_empty() {
        site.name.clone()
    } else {
        format!("{} ({})", site.name, site.timezone)
    };
    view! {
        <div class="admin-section report-site">
            <h2>{heading}</h2>
            <p class="info-text">
                {format!("{} to {}: availability {} ({} vs the month before), {} incidents, downtime {}.",
                    site.from, site.to, pct(site.availability_pct),
                    trend(site.availability_pct, site.previous_pct),
                    site.incident_count, duration_text(site.downtime_secs))}
            </p>
            {if site.checks.is_empty() {
                any(view! { <p class="empty-state">"No checks."</p> })
            } else {
                any(view! {
                    <table class="admin-table report-checks">
                        <thead><tr>
                            <th>"Probe"</th><th>"Check"</th><th>"Availability"</th><th>"Month before"</th>
                            <th>"Change"</th><th>"Incidents"</th><th>"Downtime"</th><th>"Results (Down)"</th>
                        </tr></thead>
                        <tbody>
                            {site.checks.into_iter().map(|c| {
                                let class = if below(target, c.availability_pct) { "below-target" } else { "" };
                                let name = if c.deleted {
                                    any(view! { {c.name.clone()} " " <span class="badge">"deleted"</span> })
                                } else {
                                    any(view! { <a href=format!("/checks/{}", c.check_id)>{c.name.clone()}</a> })
                                };
                                view! {
                                    <tr class=class>
                                        <td>{c.probe_name.clone()}</td>
                                        <td>{name}</td>
                                        <td class="availability">{pct(c.availability_pct)}</td>
                                        <td>{pct(c.previous_pct)}</td>
                                        <td>{trend(c.availability_pct, c.previous_pct)}</td>
                                        <td>{c.incident_count}</td>
                                        <td>{if c.downtime_secs > 0 { duration_text(c.downtime_secs) } else { "—".into() }}</td>
                                        <td>{format!("{} ({})", c.totals.total(), c.totals.down)}</td>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_change_is_in_points_with_its_sign() {
        assert_eq!(trend(Some(99.5), Some(99.0)), "+0.5 pt");
        assert_eq!(trend(Some(98.0), Some(99.25)), "−1.25 pt");
        assert_eq!(trend(Some(99.0), Some(99.001)), "=");
        assert_eq!(trend(Some(99.0), None), "—");
        assert_eq!(trend(None, Some(99.0)), "—");
    }

    #[test]
    fn only_rows_with_results_can_be_below_the_target() {
        assert!(below(Some(99.9), Some(99.0)));
        assert!(!below(Some(99.9), Some(99.9)));
        assert!(!below(Some(99.9), None));
        assert!(!below(None, Some(1.0)));
    }
}
