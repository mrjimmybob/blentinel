//! M6-05: SLA reports — generate monthly availability reports

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "ssr"))]
use crate::app::http::fetch_json;

use crate::app::ui::any;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SLAReportResult {
    pub format: String,
    pub html: Option<String>,
    pub report: Option<SLAReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SLAReport {
    pub check_id: String,
    pub check_name: String,
    pub month: String,
    pub year: i32,
    pub availability_pct: f64,
    pub sla_target_pct: f64,
    pub meets_sla: bool,
}

#[component]
pub fn M6ReportsPage() -> impl IntoView {
    let check_id = RwSignal::new("".to_string());
    let month = RwSignal::new(11u32); // November
    let year = RwSignal::new(2026u32);
    let sla_target = RwSignal::new(99.0);

    let report: LocalResource<Result<SLAReportResult, String>> = LocalResource::new(
        move || {
            let id = check_id.get();
            let m = month.get();
            let y = year.get();
            let target = sla_target.get();
            async move {
                if id.is_empty() {
                    return Err("Enter a check ID".to_string());
                }
                #[cfg(not(feature = "ssr"))]
                {
                    fetch_json::<SLAReportResult>(&format!(
                        "/api/generate-sla-report?check_id={}&month={}&year={}&sla_target_pct={}&format=html",
                        id, m, y, target
                    ))
                    .await
                }
                #[cfg(feature = "ssr")]
                {
                    let _ = (id, m, y, target);
                    Err(String::new())
                }
            }
        },
    );

    view! {
        <div class="page m6-reports">
            <h1 class="page-title">"Availability Reports"</h1>
            <p class="info-text">"Generate monthly SLA reports for any check."</p>

            <div class="report-form">
                <label for="check-id">"Check ID: "</label>
                <input
                    id="check-id"
                    type="text"
                    placeholder="check-123"
                    on:input=move |ev| {
                        check_id.set(crate::app::ui::input_value(&ev.into()));
                    }
                />

                <label for="month-input">"Month: "</label>
                <input
                    id="month-input"
                    type="number"
                    min="1"
                    max="12"
                    value=month.get().to_string()
                    on:input=move |ev| {
                        if let Ok(m) = crate::app::ui::input_value(&ev.into()).parse::<u32>() {
                            if m >= 1 && m <= 12 {
                                month.set(m);
                            }
                        }
                    }
                />

                <label for="year-input">"Year: "</label>
                <input
                    id="year-input"
                    type="number"
                    value=year.get().to_string()
                    on:input=move |ev| {
                        if let Ok(y) = crate::app::ui::input_value(&ev.into()).parse::<u32>() {
                            year.set(y);
                        }
                    }
                />

                <label for="sla-target">"SLA Target (%): "</label>
                <input
                    id="sla-target"
                    type="number"
                    step="0.1"
                    value=sla_target.get().to_string()
                    on:input=move |ev| {
                        if let Ok(t) = crate::app::ui::input_value(&ev.into()).parse::<f64>() {
                            sla_target.set(t);
                        }
                    }
                />
            </div>

            <Suspense fallback=|| view! { <div>"Generating report..."</div> }>
                {move || -> AnyView {
                    match report.get() {
                        Some(Ok(result)) => {
                            if let Some(html) = result.html {
                                any(view! {
                                    <div class="report-html" inner_html=html />
                                })
                            } else if let Some(rep) = result.report {
                                any(view! {
                                    <div class="report-summary">
                                        <h2>{rep.check_name} " — " {rep.month} "/" {rep.year}</h2>
                                        <table>
                                            <tr>
                                                <td>"Availability:"</td>
                                                <td><strong>{format!("{:.1}%", rep.availability_pct)}</strong></td>
                                            </tr>
                                            <tr>
                                                <td>"Target:"</td>
                                                <td>{format!("{:.1}%", rep.sla_target_pct)}</td>
                                            </tr>
                                            <tr>
                                                <td>"Compliance:"</td>
                                                <td>
                                                    {if rep.meets_sla {
                                                        view! { <span class="pass">"✓ PASS"</span> }
                                                    } else {
                                                        view! { <span class="fail">"✗ FAIL"</span> }
                                                    }}
                                                </td>
                                            </tr>
                                        </table>
                                    </div>
                                })
                            } else {
                                any(view! { <div>"No report data returned"</div> })
                            }
                        }
                        Some(Err(e)) => any(view! { <div class="error">"Error: " {e}</div> }),
                        None => any(view! { <div class="empty-state">"Enter check details and submit"</div> }),
                    }
                }}
            </Suspense>
        </div>
    }
}
