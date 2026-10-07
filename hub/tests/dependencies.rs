//! Check dependencies (M3-07, D-68): behind a check that is down (a site's
//! gateway), a check is Unknown (unreachable): no alert, no incident, no
//! downtime; the parent's alert names it. Links stay on one probe and never
//! loop.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use common::checks::{CheckId, CheckSpec};
use hub_lib::model::{CheckInfo, IncidentInfo, ProbeChecks, ProbeDevice, Status, UptimeBucket};
use serde_json::{Value, json};
use support::*;

fn ping_spec(host: &str) -> CheckSpec {
    CheckSpec::Ping { host: host.into() }
}

/// `probe`'s checks GW and NAS, NAS behind GW (and, with `switch`, behind a
/// Switch behind GW).
async fn site(hub: &TestHub, probe: &TestProbe, switch: bool) {
    let gw = hub.add_check(probe, "GW", ping_spec("10.0.0.1")).await;
    let nas = hub.add_check(probe, "NAS", ping_spec("10.0.0.20")).await;
    let mut parent = gw;
    if switch {
        let id = hub.add_check(probe, "Switch", ping_spec("10.0.0.2")).await;
        link(hub, &id, &parent).await;
        parent = id;
    }
    link(hub, &nas, &parent).await;
}

async fn link(hub: &TestHub, child: &str, parent: &str) {
    let mut check = hub
        .store()
        .check(&CheckId(child.into()))
        .await
        .unwrap()
        .unwrap();
    check.depends_on = Some(CheckId(parent.into()));
    assert!(hub.store().update_check(&check).await.unwrap());
}

fn results(gw: bool, nas: bool) -> Vec<Res> {
    vec![ping("GW", "10.0.0.1", gw), ping("NAS", "10.0.0.20", nas)]
}

async fn device(hub: &TestHub, probe: &TestProbe, name: &str) -> ProbeDevice {
    let devices: Vec<ProbeDevice> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;
    devices.into_iter().find(|d| d.name == name).unwrap()
}

#[tokio::test]
async fn behind_a_dead_gateway_only_the_gateway_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    site(&hub, &probe, false).await;

    for _ in 0..3 {
        hub.report_now(&probe, "acme", results(false, false)).await;
        hub.clock.advance(chrono::Duration::minutes(1));
    }

    let subjects = hub.wait_for_alerts(1).await;
    assert_eq!(subjects.len(), 1, "{subjects:?}");
    assert!(subjects[0].ends_with("GW DOWN"), "{subjects:?}");
    let sent = hub.notifier.sent();
    assert!(
        sent[0].detail.ends_with("· unreachable behind it: NAS"),
        "{}",
        sent[0].detail
    );
    let nas = device(&hub, &probe, "NAS").await;
    assert_eq!(nas.status, Status::Unknown);
    assert_eq!(nas.reason.as_deref(), Some("unreachable: GW is down"));
    let incidents: Vec<IncidentInfo> = hub.get_json("/api/incidents").await;
    assert_eq!(incidents.len(), 1, "one incident, the root cause");
    assert_eq!(incidents[0].title(), "GW");
}

#[tokio::test]
async fn once_the_gateway_is_back_a_check_still_down_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    site(&hub, &probe, false).await;

    for (gw, nas) in [
        (false, false),
        (false, false),
        (true, false),
        (true, false),
        (true, false),
    ] {
        hub.evaluate_report(&probe, results(gw, nas)).await;
    }

    let subjects = hub.notifier.subjects();
    let words: Vec<&str> = subjects
        .iter()
        .map(|s| s.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(words, ["DOWN", "RECOVERED", "DOWN"], "{subjects:?}");
    assert!(subjects[2].ends_with("NAS DOWN"));
}

#[tokio::test]
async fn a_short_gateway_blip_doesnt_count_against_the_checks_behind_it() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    site(&hub, &probe, false).await;

    // One lost gateway ping (and the NAS with it), then a NAS failure: one
    // failure in a row for the NAS, not two.
    for (gw, nas) in [(true, true), (false, false), (true, false)] {
        hub.evaluate_report(&probe, results(gw, nas)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn the_farthest_check_down_is_the_root_cause() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    site(&hub, &probe, true).await;
    let all = |up: bool| {
        vec![
            ping("GW", "10.0.0.1", up),
            ping("Switch", "10.0.0.2", up),
            ping("NAS", "10.0.0.20", up),
        ]
    };

    for _ in 0..2 {
        hub.evaluate_report(&probe, all(false)).await;
    }

    let sent = hub.notifier.sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]
            .detail
            .ends_with("unreachable behind it: Switch, NAS")
            || sent[0]
                .detail
                .ends_with("unreachable behind it: NAS, Switch"),
        "{}",
        sent[0].detail
    );
}

#[tokio::test]
async fn unreachable_time_doesnt_count_as_downtime() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    site(&hub, &probe, false).await;

    hub.report_now(&probe, "acme", results(false, false)).await;

    let statuses: Vec<String> = sqlx::query_scalar(
        "SELECT r.status FROM check_results r JOIN checks k ON k.id = r.check_id
         WHERE k.name = 'NAS'",
    )
    .fetch_all(hub.pool())
    .await
    .unwrap();
    assert_eq!(statuses, ["Unknown"]);
    let uptime: Vec<UptimeBucket> = hub.get_json("/api/company/acme/uptime?range=24h").await;
    let counted: (i64, i64) = uptime.iter().fold((0, 0), |(up, down), b| {
        (up + b.up_count, down + b.down_count)
    });
    assert_eq!(counted, (0, 1), "the gateway's result only, not the NAS's");
}

async fn create(
    hub: &TestHub,
    cookie: &str,
    probe: &TestProbe,
    body: Value,
) -> (StatusCode, Value) {
    let response = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(cookie),
            body,
        )
        .await;
    (response.status, response.json())
}

fn ping_check(name: &str, depends_on: Option<&str>) -> Value {
    json!({ "name": name, "spec": { "kind": "ping", "host": "10.0.0.1" }, "depends_on": depends_on })
}

#[tokio::test]
async fn a_dependency_is_set_over_the_api_on_the_same_probe_without_loops() {
    let probe = TestProbe::new("hq");
    let other = TestProbe::new("branch");
    let hub = TestHub::new(&[&probe, &other]).await;
    let owner = hub.login().await;
    let (_, gw) = create(&hub, &owner, &probe, ping_check("GW", None)).await;
    let gw = gw["id"].as_str().unwrap().to_string();

    let (status, nas) = create(&hub, &owner, &probe, ping_check("NAS", Some(&gw))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(nas["depends_on"], gw.as_str());
    let nas_id = nas["id"].as_str().unwrap().to_string();

    // Another probe's check, itself, a loop: refused.
    let (status, body) = create(&hub, &owner, &other, ping_check("X", Some(&gw))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    for (id, parent, reason) in [
        (&nas_id, &nas_id, "itself"),
        (&gw, &nas_id, "NAS already depends"),
    ] {
        let response = hub
            .post_json(
                &format!("/api/checks/{id}"),
                Some(&owner),
                ping_check("C", Some(parent)),
            )
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert!(response.text().contains(reason), "{}", response.text());
    }

    // Deleting the parent clears the link.
    let response = hub
        .post_json(&format!("/api/checks/{gw}/delete"), Some(&owner), json!({}))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let checks: ProbeChecks = hub
        .get_json(&format!("/api/probes/{}/checks", probe.id))
        .await;
    assert_eq!(checks.checks[0].depends_on, None);
}

#[tokio::test]
async fn copied_checks_keep_their_links_between_the_copies() {
    let old = TestProbe::new("old");
    let new = TestProbe::new("new");
    let hub = TestHub::new(&[&old, &new]).await;
    let owner = hub.login().await;
    let (_, gw) = create(&hub, &owner, &old, ping_check("GW", None)).await;
    create(&hub, &owner, &old, ping_check("NAS", gw["id"].as_str())).await;

    let response = hub
        .post_json(
            &format!("/api/probes/{}/checks/copy", new.id),
            Some(&owner),
            json!({ "from_probe_id": old.id }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    let checks: Vec<CheckInfo> = hub
        .get_json::<ProbeChecks>(&format!("/api/probes/{}/checks", new.id))
        .await
        .checks;
    let gw_copy = checks.iter().find(|c| c.name == "GW").unwrap();
    let nas_copy = checks.iter().find(|c| c.name == "NAS").unwrap();
    assert_ne!(gw_copy.id, gw["id"].as_str().unwrap());
    assert_eq!(nas_copy.depends_on.as_deref(), Some(gw_copy.id.as_str()));
}
