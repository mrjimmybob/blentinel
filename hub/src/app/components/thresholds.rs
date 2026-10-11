//! Thresholds above the check (M3-01, D-64): the hub's default on the admin
//! page, a company's and its sites' on the company page. Each limit left
//! empty is inherited; the placeholder says what applies then and from where.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{any, nothing};
use crate::model::{
    MetricLimits, Permission, SiteInfo, ThresholdMetric, Thresholds, ThresholdsView,
};

/// One metric's row as typed.
#[derive(Clone, Copy)]
struct Row {
    metric: ThresholdMetric,
    warning: RwSignal<String>,
    down: RwSignal<String>,
}

/// The thresholds the rows say, checked like the hub checks them.
fn thresholds_of(rows: &[Row]) -> Result<Thresholds, String> {
    let mut thresholds = Thresholds::default();
    for row in rows {
        let limits = MetricLimits::from_text(
            row.metric,
            &row.warning.get_untracked(),
            &row.down.get_untracked(),
        )?;
        thresholds.set(row.metric, limits);
    }
    Ok(thresholds)
}

/// Edit the thresholds at `url` (GET a [`ThresholdsView`], POST [`Thresholds`]).
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
fn ThresholdsEditor(url: Signal<String>, can_edit: Signal<bool>) -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    let view_data: LocalResource<Result<ThresholdsView, String>> = LocalResource::new(move || {
        refresh.track();
        let url = url.get();
        async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<ThresholdsView>(&url).await
            }
            #[cfg(feature = "ssr")]
            {
                let _ = url;
                Err(String::new())
            }
        }
    });

    view! {
        {move || -> AnyView {
            match view_data.get() {
                Some(Ok(data)) => {
                    let rows: Vec<Row> = ThresholdMetric::ALL
                        .into_iter()
                        .map(|metric| {
                            let (warning, down) = data.own.get(metric).texts();
                            Row {
                                metric,
                                warning: RwSignal::new(warning),
                                down: RwSignal::new(down),
                            }
                        })
                        .collect();
                    let stored = StoredValue::new(rows.clone());
                    let save = move |_| {
                        error.set(String::new());
                        message.set(String::new());
                        let thresholds = match thresholds_of(&stored.get_value()) {
                            Ok(t) => t,
                            Err(e) => {
                                error.set(format!("Not saved: {e}."));
                                return;
                            }
                        };
                        let body = serde_json::to_string(&thresholds).unwrap_or_default();
                        leptos::task::spawn_local(async move {
                            #[cfg(not(feature = "ssr"))]
                            match post_json(&url.get_untracked(), &body).await {
                                Ok(()) => {
                                    message.set("Saved. It applies from each probe's next report.".into());
                                    refresh.update(|n| *n += 1);
                                }
                                Err(e) => error.set(e),
                            }
                            #[cfg(feature = "ssr")]
                            let _ = body;
                        });
                    };
                    let body = rows
                        .into_iter()
                        .map(|row| {
                            let inherited = data
                                .inherited
                                .iter()
                                .find(|(m, _)| *m == row.metric)
                                .map(|(_, l)| *l);
                            let hint = move |pick: fn(&crate::model::ResolvedLimits) -> crate::model::Limit| {
                                inherited.map(|l| {
                                    let limit = pick(&l);
                                    (limit.text(row.metric), format!("from {}", limit.from.label()))
                                })
                            };
                            let (warning_placeholder, warning_title) = hint(|l| l.warning).unwrap_or_default();
                            let (down_placeholder, down_title) = hint(|l| l.down).unwrap_or_default();
                            #[cfg(not(feature = "ssr"))]
                            let (on_warning, on_down) = (
                                move |ev| row.warning.set(input_value(&ev)),
                                move |ev| row.down.set(input_value(&ev)),
                            );
                            #[cfg(feature = "ssr")]
                            let (on_warning, on_down) = (|_| {}, |_| {});
                            let name = row.metric.label().to_lowercase().replace(' ', "_");
                            view! {
                                <tr>
                                    <td>{row.metric.label()}</td>
                                    <td>
                                        <input type="text" size="6" name=format!("{name}_warning")
                                            placeholder=warning_placeholder title=warning_title
                                            disabled=move || !can_edit.get()
                                            prop:value=move || row.warning.get() on:input=on_warning/>
                                    </td>
                                    <td>
                                        <input type="text" size="6" name=format!("{name}_down")
                                            placeholder=down_placeholder title=down_title
                                            disabled=move || !can_edit.get()
                                            prop:value=move || row.down.get() on:input=on_down/>
                                    </td>
                                    <td>{row.metric.unit().trim()}</td>
                                </tr>
                            }
                        })
                        .collect_view();
                    any(view! {
                        <table class="admin-table thresholds-table">
                            <thead>
                                <tr><th></th><th>"Warning at"</th><th>"Down at"</th><th></th></tr>
                            </thead>
                            <tbody>{body}</tbody>
                        </table>
                        <Show when=move || can_edit.get()>
                            <button class="btn btn-primary btn-sm" name="thresholds_save" on:click=save>"Save"</button>
                        </Show>
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
        {move || {
            let m = message.get();
            if m.is_empty() { nothing() } else { any(view! { <div class="success-message">{m}</div> }) }
        }}
    }
}

const EXPLAINED: &str = "At or over a limit, a check is Warning or Down: its usage for CPU, \
     memory and disk checks, its response time for ping, HTTP and TCP checks. A file check: the age of its newest file, in hours. A TLS certificate \
     check is the other way round: at or under a number of days left. An empty limit \
     is inherited (shown greyed: what applies, hover for where from); \"off\" switches it off.";

/// The hub's default thresholds, on the admin page.
#[component]
pub(crate) fn GlobalThresholdsSection() -> impl IntoView {
    let me = crate::app::ui::Me::current();
    let can_edit = Signal::derive(move || me.can(Permission::ManageData));
    let url = Signal::derive(|| "/api/admin/thresholds".to_string());
    view! {
        <div class="admin-section thresholds">
            <h2>"Thresholds"</h2>
            <p class="info-text">
                {EXPLAINED}
                " These are the hub's defaults; each company, site and check can set its own."
            </p>
            <ThresholdsEditor url can_edit/>
        </div>
    }
}

/// A company's thresholds, or one of its sites', on the company page.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))]
pub(crate) fn CompanyThresholdsSection(company_id: String, sites: Vec<SiteInfo>) -> impl IntoView {
    let me = crate::app::ui::Me::current();
    let can_edit = Signal::derive(move || me.can(Permission::ManageCompanies));
    // Empty: the whole company; else a site's ID.
    let site = RwSignal::new(String::new());
    let base = format!("/api/companies/{company_id}");
    let url = Signal::derive(move || match site.get() {
        s if s.is_empty() => format!("{base}/thresholds"),
        s => format!("{base}/sites/{s}/thresholds"),
    });
    #[cfg(not(feature = "ssr"))]
    let on_site = move |ev| site.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_site = |_| {};
    let options = sites
        .into_iter()
        .map(|s| view! { <option value=s.id.clone()>{format!("Site: {}", s.name)}</option> })
        .collect_view();
    view! {
        <div class="admin-section thresholds">
            <h2>"Thresholds"</h2>
            <p class="info-text">
                {EXPLAINED}
                " A site's limits apply to its probes' checks, before the company's."
            </p>
            <label>
                "For "
                <select name="thresholds_level" on:change=on_site>
                    <option value="">"The whole company"</option>
                    {options}
                </select>
            </label>
            <ThresholdsEditor url can_edit/>
        </div>
    }
}
