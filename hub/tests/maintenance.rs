//! Maintenance windows (M3-04, D-65): planned work during which a scope sends
//! no alerts and its results don't count for uptime; what is still wrong
//! when the window closes alerts then.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use hub_lib::model::{MaintenanceOverview, ProbeDevice, Role, UptimeBucket};
use serde_json::{Value, json};
use support::*;

const WINDOWS: &str = "/api/companies/acme/maintenance";

fn nas(up: bool) -> Vec<Res> {
    vec![ping("NAS", "10.0.0.20", up)]
}

/// A one-off window over the whole company from 11:00 to 13:00 UTC on the
/// test clock's day (it starts at 12:00).
fn noon_window() -> Value {
    json!({
        "scope": { "kind": "company" },
        "reason": "router swap",
        "timezone": "UTC",
        "schedule": { "every": "once", "start": "2026-03-10T11:00", "end": "2026-03-10T13:00" },
    })
}

fn kinds(subjects: &[String]) -> Vec<&str> {
    subjects
        .iter()
        .map(|s| s.rsplit(' ').next().unwrap_or(""))
        .collect()
}

/// Report `up` a minute later, and let the alert evaluation run.
async fn report(hub: &TestHub, probe: &TestProbe, up: bool) {
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(probe, "acme", nas(up)).await;
    // Alerts are evaluated in the background, one report at a time.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
async fn windows_are_added_listed_and_removed() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let site = format!("{}-site", probe.name);

    let weekly = json!({
        "scope": { "kind": "site", "site_id": site },
        "reason": "Patch Tuesday",
        "timezone": "Europe/Madrid",
        "schedule": {
            "every": "monthly", "on": { "by": "weekday", "week": 2, "weekday": 2 },
            "start": "22:00", "minutes": 180,
        },
    });
    let added = hub.post_json(WINDOWS, Some(&owner), weekly).await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text());
    let id = added.json::<Value>()["id"].as_i64().unwrap();

    let page: MaintenanceOverview = hub.get_json(WINDOWS).await;
    let window = &page.windows[0];
    assert_eq!(window.scope_label, format!("site {site}"));
    assert_eq!(
        window.schedule_text,
        "every month on the 2nd Tue at 22:00 for 3 h"
    );
    assert!(!window.active);
    // 2nd Tuesday of March 2026: the 10th, 22:00 in Madrid (UTC+1).
    assert_eq!(
        window.next.as_deref(),
        Some("2026-03-10 22:00 – 2026-03-11 01:00 (Europe/Madrid)")
    );
    assert_eq!(page.sites[0].timezone, "UTC");
    assert_eq!(page.probes[0].checks.len(), 0);

    let removed = hub
        .post_json(
            &format!("/api/maintenance/{id}/delete"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(removed.status, StatusCode::OK);
    assert!(
        hub.get_json::<MaintenanceOverview>(WINDOWS)
            .await
            .windows
            .is_empty()
    );

    let audited: Vec<Value> = hub.get_json("/api/audit?action=maintenance").await;
    assert_eq!(audited.len(), 2, "added, removed");
}

#[tokio::test]
async fn bad_windows_are_refused_with_the_reason() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let mut bad_zone = noon_window();
    bad_zone["timezone"] = json!("Mars/Olympus");
    let mut backwards = noon_window();
    backwards["schedule"]["end"] = json!("2026-03-10T10:00");
    let mut no_reason = noon_window();
    no_reason["reason"] = json!(" ");
    let mut nowhere = noon_window();
    nowhere["scope"] = json!({ "kind": "probe", "probe_id": "not-ours" });
    for (body, why) in [
        (bad_zone, "not a time zone"),
        (backwards, "after the start"),
        (no_reason, "what the maintenance is for"),
        (nowhere, "no such probe"),
    ] {
        let refused = hub.post_json(WINDOWS, Some(&owner), body).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{why}");
        assert!(refused.text().contains(why), "{}", refused.text());
    }

    // Viewers see the page but don't change it.
    let viewer = hub.login_as(Role::Viewer).await;
    assert_eq!(hub.get(WINDOWS, Some(&viewer)).await.status, StatusCode::OK);
    let forbidden = hub.post_json(WINDOWS, Some(&viewer), noon_window()).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_window_holds_alerts_back_until_it_closes() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, true).await;
    hub.post_json(WINDOWS, Some(&owner), noon_window()).await;

    // Down during the window: nobody is told, the page says why.
    report(&hub, &probe, false).await;
    report(&hub, &probe, false).await;
    report(&hub, &probe, false).await;
    assert!(hub.notifier.subjects().is_empty());
    let devices: Vec<ProbeDevice> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;
    assert!(devices[0].in_maintenance);

    // Uptime leaves the window out (and shades it).
    let uptime: Vec<UptimeBucket> = hub.get_json("/api/company/acme/uptime?range=24h").await;
    let total: (i64, i64, i64) = uptime.iter().fold((0, 0, 0), |(u, d, m), b| {
        (u + b.up_count, d + b.down_count, m + b.maintenance_count)
    });
    assert_eq!(
        total,
        (1, 0, 3),
        "the up before it counts; the downs in it don't"
    );

    // The window closes at 13:00; still down: the first report after it alerts.
    hub.clock.advance(Duration::minutes(60));
    report(&hub, &probe, false).await;
    assert_eq!(kinds(&hub.wait_for_alerts(1).await), ["DOWN"]);
}

#[tokio::test]
async fn what_recovered_during_the_window_alerts_nobody() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, true).await;
    hub.post_json(WINDOWS, Some(&owner), noon_window()).await;

    report(&hub, &probe, false).await;
    report(&hub, &probe, false).await;
    report(&hub, &probe, true).await;
    hub.clock.advance(Duration::minutes(60));
    report(&hub, &probe, true).await;
    report(&hub, &probe, true).await;

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn a_probe_that_stops_during_a_window_alerts_once_it_closes() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let owner = hub.login().await;
    hub.report_now(&probe, "acme", nas(true)).await;
    hub.post_json(WINDOWS, Some(&owner), noon_window()).await;

    let sweep = |hub: &TestHub| {
        let (storage, notifier, config, now) = (
            hub.state.storage.get(),
            hub.state.notifier.clone(),
            hub.state.config.clone(),
            hub.now(),
        );
        async move {
            hub_lib::service::liveness::sweep_expired_probes(&storage, &notifier, &config, now)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };

    // Silent past its timeout, inside the window: no alert.
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    sweep(&hub).await;
    assert!(hub.notifier.subjects().is_empty());

    // After 13:00, still silent: the next sweep alerts, once.
    hub.clock.advance(Duration::minutes(60));
    sweep(&hub).await;
    sweep(&hub).await;
    assert_eq!(kinds(&hub.wait_for_alerts(1).await), ["EXPIRED"]);
}

#[tokio::test]
async fn a_sites_time_zone_is_set_on_the_site() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let path = format!("/api/companies/acme/sites/{}-site", probe.name);

    let set = hub
        .post_json(
            &path,
            Some(&owner),
            json!({ "name": "HQ", "timezone": "America/New_York" }),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text());
    let page: MaintenanceOverview = hub.get_json(WINDOWS).await;
    assert_eq!(page.sites[0].timezone, "America/New_York");

    let bad = hub
        .post_json(
            &path,
            Some(&owner),
            json!({ "name": "HQ", "timezone": "Nowhere/Land" }),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.error_code(), "invalid_timezone");
}
