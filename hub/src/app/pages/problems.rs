//! What is wrong now (M6-03, proposal 0007 §3.3, D-114): every check that
//! is Down, Warning or Unknown and every probe that stopped reporting, in
//! the user's companies, grouped by company › site, with who is on it.
//! Searched and filtered on the hub; refreshed every 15 seconds.

use std::collections::BTreeSet;

use leptos::prelude::*;

use crate::app::components::kind_label;
use crate::app::ui::{any, encode, status_class, status_label};
#[cfg(not(feature = "ssr"))]
use crate::app::{browser, http::fetch_json, ui::input_value};
use crate::model::{PROBLEM_STATUSES, Problem, ProblemCompany, ProblemsView, Status};

/// `q=…&status=down,warning&kind=ping` from the filters (empty ones left out).
pub(crate) fn query_string(q: &str, statuses: &BTreeSet<&'static str>, kind: &str) -> String {
    let mut parts = Vec::new();
    if !q.trim().is_empty() {
        parts.push(format!("q={}", encode(q.trim())));
    }
    if !statuses.is_empty() {
        let list: Vec<&str> = statuses.iter().copied().collect();
        parts.push(format!("status={}", list.join(",")));
    }
    if !kind.is_empty() {
        parts.push(format!("kind={}", encode(kind)));
    }
    parts.join("&")
}

fn status_key(status: Status) -> &'static str {
    match status {
        Status::Down => "down",
        Status::Warning => "warning",
        Status::Unknown => "unknown",
        Status::Up => "up",
    }
}

#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn ProblemsPage() -> impl IntoView {
    let search = RwSignal::new(String::new());
    let statuses = RwSignal::new(BTreeSet::<&'static str>::new());
    let kind = RwSignal::new(String::new());
    let query = Memo::new(move |_| query_string(&search.get(), &statuses.get(), &kind.get()));
    let data = RwSignal::new(None::<Result<ProblemsView, String>>);

    #[cfg(not(feature = "ssr"))]
    {
        // Load for the filters as they are; a late answer for older filters
        // is dropped.
        let load = move || {
            let asked = query.get_untracked();
            leptos::task::spawn_local(async move {
                let result = fetch_json::<ProblemsView>(&format!("/api/problems?{asked}")).await;
                if query.get_untracked() == asked {
                    data.set(Some(result));
                }
            });
        };
        Effect::new(move |_| {
            query.track();
            load();
        });
        Effect::new(move |_| browser::poll_every(15_000, load));
    }

    #[cfg(not(feature = "ssr"))]
    let (on_search, on_kind) = (
        move |ev| search.set(input_value(&ev)),
        move |ev| kind.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_search, on_kind) = (|_| {}, |_| {});

    view! {
        <div class="page problems-page">
            <h1 class="page-title">"Problems"</h1>
            <p class="info-text">"What is wrong now: every check that is Down, Warning or Unknown, and every probe that stopped reporting. Refreshed every 15 seconds."</p>
            <div class="problems-filters">
                <input type="search" name="problems_search" placeholder="Search companies, sites, probes, checks, targets"
                    prop:value=move || search.get() on:input=on_search/>
                {PROBLEM_STATUSES.into_iter().map(|status| {
                    let key = status_key(status);
                    let count = move || data.get().and_then(Result::ok).map(|v| match status {
                        Status::Down => v.down,
                        Status::Warning => v.warning,
                        _ => v.unknown,
                    });
                    view! {
                        <label class=format!("status-filter {}", status_class(status))>
                            <input type="checkbox" name=format!("problems_{key}")
                                prop:checked=move || statuses.get().contains(key)
                                on:change=move |_| statuses.update(|s| {
                                    if !s.remove(key) {
                                        s.insert(key);
                                    }
                                })/>
                            {format!(" {}", status_label(status))}
                            {move || count().map(|n| format!(" ({n})"))}
                        </label>
                    }
                }).collect_view()}
                <select name="problems_kind" prop:value=move || kind.get() on:change=on_kind>
                    <option value="">"Every kind"</option>
                    {move || data.get().and_then(Result::ok).map(|v| v.kinds).unwrap_or_default().into_iter().map(|k| {
                        let label = kind_label(&k);
                        view! { <option value=k.clone()>{label}</option> }
                    }).collect_view()}
                </select>
            </div>
            {move || -> AnyView {
                match data.get() {
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    Some(Ok(v)) if v.total == 0 => any(view! {
                        <p class="empty-state problems-none">"Nothing is wrong: every check is Up and every probe reports."</p>
                    }),
                    Some(Ok(v)) if v.companies.is_empty() => any(view! {
                        <p class="empty-state">"No problem matches the search and filters."</p>
                    }),
                    Some(Ok(v)) => any(v.companies.into_iter().map(|c| view! { <Company c=c/> }).collect_view()),
                }
            }}
        </div>
    }
}

/// One company: its sites, each with its problems.
#[component]
fn Company(c: ProblemCompany) -> impl IntoView {
    view! {
        <div class="admin-section problems-company">
            <h2>
                <span class=format!("dev-status {}", status_class(c.worst))></span>
                " " <a href=format!("/company/{}", c.company_id)>{c.name.clone()}</a>
            </h2>
            {c.sites.into_iter().map(|site| view! {
                <h3 class="problems-site">{if site.name.is_empty() { "No site".to_string() } else { site.name.clone() }}</h3>
                <table class="admin-table checks-table problems-table">
                    <thead><tr>
                        <th>"Status"</th><th>"Probe"</th><th>"Check"</th><th>"Target"</th>
                        <th>"What"</th><th>"Incident"</th><th>"Acknowledged"</th>
                    </tr></thead>
                    <tbody>{site.problems.into_iter().map(|p| view! { <Row p=p/> }).collect_view()}</tbody>
                </table>
            }).collect_view()}
        </div>
    }
}

#[component]
fn Row(p: Problem) -> impl IntoView {
    let check = match (&p.check_id, &p.check_name) {
        (Some(id), Some(name)) => any(view! {
            <a href=format!("/checks/{id}")>{name.clone()}</a>
            {p.kind.as_deref().map(|k| view! { <span class="check-kind">{format!(" · {}", kind_label(k))}</span> })}
        }),
        _ => any(view! { <em>"Stopped reporting"</em> }),
    };
    let what = if p.check_id.is_none() {
        format!(
            "Last report {} UTC",
            p.last_seen_at.clone().unwrap_or_else(|| "never".into())
        )
    } else {
        [p.reason.clone(), p.message.clone()]
            .into_iter()
            .flatten()
            .filter(|m| !m.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let (incident, acknowledged) = match &p.incident {
        Some(i) => (
            any(view! {
                <a href=format!("/incidents/{}", i.id)>{format!("#{}", i.id)}</a>
                {format!(" since {} UTC", i.opened_at)}
            }),
            match &i.acknowledged_by {
                Some(by) => any(view! {
                    <span class="acknowledged">{format!("{by}, {}", i.acknowledged_at.clone().unwrap_or_default())}</span>
                }),
                None => any(view! { <span class="not-acknowledged">"Not yet"</span> }),
            },
        ),
        None => (any(view! { "—" }), any(view! { "—" })),
    };
    view! {
        <tr class=format!("problem {}", status_class(p.status))>
            <td class=format!("check-result {}", status_class(p.status))>
                {status_label(p.status)}
                {p.in_maintenance.then(|| view! {
                    " " <span class="badge badge-maintenance" title="It came during a maintenance window: no alerts">"maintenance"</span>
                })}
            </td>
            <td><a href=format!("/probes/{}", p.probe_id)>{p.probe_name.clone()}</a></td>
            <td>{check}</td>
            <td>{p.target.clone().unwrap_or_default()}</td>
            <td>{what}</td>
            <td>{incident}</td>
            <td>{acknowledged}</td>
        </tr>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_become_a_query_string() {
        let none = BTreeSet::new();
        assert_eq!(query_string("", &none, ""), "");
        let two = BTreeSet::from(["warning", "down"]);
        assert_eq!(
            query_string(" core switch ", &two, "ping"),
            "q=core%20switch&status=down,warning&kind=ping"
        );
        assert_eq!(query_string("", &none, "disk"), "kind=disk");
    }
}
