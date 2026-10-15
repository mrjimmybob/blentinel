//! The fleet view (M4-02, D-73): every probe the user sees, how each one is
//! doing (its version, since when it runs, the reports it holds back, its
//! clock, whether it runs its checks, its last problem), and which need
//! attention. Everyone sees the probes of their companies.

use leptos::prelude::*;

use crate::app::ui::{any, fmt_ts, fmt_utc};
#[cfg(not(feature = "ssr"))]
use crate::app::{browser, http::fetch_json, ui::input_value};
use crate::model::{ConfigSync, ProbeInfo};

/// How often the page reloads (probes report every minute or so).
#[cfg(not(feature = "ssr"))]
const REFRESH_MS: i32 = 30_000;

/// The checks column: class and text.
pub(crate) fn sync_cell(sync: Option<ConfigSync>) -> (&'static str, &'static str) {
    match sync {
        Some(ConfigSync::InSync) => ("sync-badge in-sync", "In sync"),
        Some(ConfigSync::Pending) => ("sync-badge pending", "Pending"),
        Some(ConfigSync::Rejected) => ("sync-badge rejected", "Rejected"),
        None => ("sync-badge neutral", "?"),
    }
}

/// The clock column: `+2 s` (ahead of the hub), `-90 s` (behind), `—`.
pub(crate) fn clock_text(offset: Option<i64>) -> String {
    match offset {
        Some(s) if s > 0 => format!("+{s} s"),
        Some(s) => format!("{s} s"),
        None => "—".into(),
    }
}

/// The badge of a probe: revoked, needs attention, or fine.
fn badge(probe: &ProbeInfo, concerns: &[String]) -> (&'static str, &'static str) {
    if probe.state != "active" {
        ("fleet-badge revoked", "REVOKED")
    } else if concerns.is_empty() {
        ("fleet-badge ok", "OK")
    } else {
        ("fleet-badge attention", "ATTENTION")
    }
}

/// Does `probe` match the text typed in the filter?
pub(crate) fn matches(probe: &ProbeInfo, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || [
            Some(&probe.name),
            Some(&probe.company_name),
            Some(&probe.site_name),
            probe.hostname.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(&filter))
}

fn row(probe: ProbeInfo) -> impl IntoView {
    let concerns = probe.concerns();
    let (badge_class, badge_text) = badge(&probe, &concerns);
    let (sync_class, sync_text) = sync_cell(probe.config_sync);
    let health = probe.health.clone();
    let since = health
        .as_ref()
        .map(|h| fmt_utc(&h.started_at))
        .unwrap_or_else(|| "—".into());
    let queued = health
        .as_ref()
        .map(|h| h.queued_reports.to_string())
        .unwrap_or_else(|| "—".into());
    let clock = clock_text(health.as_ref().and_then(|h| h.clock_offset_secs));
    let problem = health.as_ref().and_then(|h| h.last_error.clone()).map(|p| {
        view! {
            <span class="fleet-problem-at">{fmt_utc(&p.at)}</span>
            " "
            {p.message}
        }
    });
    view! {
        <tr class:needs-attention=!concerns.is_empty()>
            <td><span class=badge_class>{badge_text}</span></td>
            <td><a href=format!("/probes/{}", probe.id)>{probe.name.clone()}</a></td>
            <td>{format!("{} · {}", probe.company_name, probe.site_name)}</td>
            <td>{probe.last_seen_at.as_deref().map(fmt_ts).unwrap_or_else(|| "never".into())}</td>
            <td>{probe.probe_version.clone().unwrap_or_else(|| "?".into())}</td>
            <td>{since}</td>
            <td class="num">{queued}</td>
            <td class="num">{clock}</td>
            <td><span class=sync_class>{sync_text}</span></td>
            <td class="fleet-concerns">{concerns.join(" · ")}</td>
            <td class="fleet-problem">{problem}</td>
        </tr>
    }
}

#[component]
pub(crate) fn FleetPage() -> impl IntoView {
    let probes = RwSignal::new(None::<Result<Vec<ProbeInfo>, String>>);
    let only_attention = RwSignal::new(false);
    let filter = RwSignal::new(String::new());

    let load = move || {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            probes.set(Some(fetch_json::<Vec<ProbeInfo>>("/api/probes").await));
        });
    };
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            load();
            browser::poll_every(REFRESH_MS, load);
        }
        #[cfg(feature = "ssr")]
        let _ = load;
    });
    #[cfg(not(feature = "ssr"))]
    let on_filter = move |ev| filter.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_filter = |_| {};

    view! {
        <h1 class="page-title">"Probes"</h1>
        <p class="info-text">
            "Every probe, and how it is doing: what it last said about itself with its report."
        </p>
        <div class="fleet-filters">
            <input type="search" name="fleet_filter" placeholder="Filter by name, company, site or host"
                prop:value=move || filter.get() on:input=on_filter/>
            <label>
                <input type="checkbox" name="fleet_attention"
                    prop:checked=move || only_attention.get()
                    on:change=move |_| only_attention.update(|on| *on = !*on)/>
                " Only those that need attention"
            </label>
        </div>
        {move || -> AnyView {
            match probes.get() {
                Some(Ok(list)) if list.is_empty() => any(view! {
                    <div class="empty-state">"No probes yet. Add one from a company's page."</div>
                }),
                Some(Ok(list)) => {
                    let attention = list.iter().filter(|p| !p.concerns().is_empty()).count();
                    let summary = match attention {
                        0 => format!("{} probes; all is well.", list.len()),
                        1 => format!("{} probes; 1 needs attention.", list.len()),
                        n => format!("{} probes; {n} need attention.", list.len()),
                    };
                    let text = filter.get();
                    let shown: Vec<ProbeInfo> = list
                        .into_iter()
                        .filter(|p| matches(p, &text))
                        .filter(|p| !only_attention.get() || !p.concerns().is_empty())
                        .collect();
                    any(view! {
                        <p class="fleet-summary">{summary}</p>
                        <table class="admin-table fleet-table">
                            <thead>
                                <tr>
                                    <th></th>
                                    <th>"Probe"</th>
                                    <th>"Where"</th>
                                    <th>"Last report"</th>
                                    <th>"Version"</th>
                                    <th>"Running since"</th>
                                    <th title="Reports waiting for the hub when the last one was made">"Held back"</th>
                                    <th title="The probe's clock minus the hub's">"Clock"</th>
                                    <th>"Checks"</th>
                                    <th>"Needs attention"</th>
                                    <th>"Last problem (past day)"</th>
                                </tr>
                            </thead>
                            <tbody>{shown.into_iter().map(row).collect_view()}</tbody>
                        </table>
                    })
                }
                Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                None => any(view! { <div class="loading">"Loading…"</div> }),
            }
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> ProbeInfo {
        serde_json::from_value(serde_json::json!({
            "id": "p1", "name": "Gateway probe", "company_id": "acme",
            "company_name": "Acme", "site_id": "hq", "site_name": "HQ",
            "state": "active", "hostname": "fw-01", "os": null, "arch": null,
            "probe_version": "0.3.0", "enrolled_at": "2026-10-01T00:00:00Z",
            "last_seen_at": "2026-10-08T10:00:00Z", "interval_secs": 60
        }))
        .unwrap()
    }

    #[test]
    fn the_clock_shows_ahead_or_behind() {
        assert_eq!(clock_text(Some(2)), "+2 s");
        assert_eq!(clock_text(Some(0)), "0 s");
        assert_eq!(clock_text(Some(-90)), "-90 s");
        assert_eq!(clock_text(None), "—");
    }

    #[test]
    fn each_sync_state_has_its_badge() {
        assert_eq!(sync_cell(Some(ConfigSync::InSync)).1, "In sync");
        assert_eq!(
            sync_cell(Some(ConfigSync::Rejected)).0,
            "sync-badge rejected"
        );
        assert_eq!(sync_cell(None).1, "?");
    }

    #[test]
    fn the_filter_looks_at_names_places_and_host() {
        let p = probe();
        assert!(matches(&p, ""));
        assert!(matches(&p, "gateway"));
        assert!(matches(&p, " ACME "));
        assert!(matches(&p, "hq"));
        assert!(matches(&p, "fw-0"));
        assert!(!matches(&p, "branch"));
    }

    #[test]
    fn a_revoked_probe_is_revoked_not_attention() {
        let mut p = probe();
        p.state = "revoked".into();
        p.silent = true;
        assert_eq!(badge(&p, &p.concerns()).1, "REVOKED");
        let p = probe();
        assert_eq!(badge(&p, &p.concerns()).1, "OK");
    }
}
