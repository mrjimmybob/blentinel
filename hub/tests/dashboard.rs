//! Read APIs behind the dashboard: companies, company probes, probe devices.
//! Assertions are on raw JSON because that wire format is the contract the
//! web UI (and later the mobile app) depends on.
#![cfg(feature = "ssr")]

mod support;

use chrono::Duration;
use common::models::ResourceType;
use serde_json::{Value, json};
use support::*;

#[tokio::test]
async fn empty_hub_has_no_companies() {
    let hub = TestHub::new(&[]).await;

    let companies: Value = hub.get_json("/api/dashboard/companies").await;

    assert_eq!(companies, json!([]));
}

#[tokio::test]
async fn dashboard_counts_devices_from_each_probes_latest_report_only() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    hub.report_now(
        &probe,
        "acme",
        vec![ping("a", "10.0.0.1", true), ping("b", "10.0.0.2", false)],
    )
    .await;
    hub.clock.advance(Duration::seconds(60));
    hub.report_now(
        &probe,
        "acme",
        vec![
            ping("a", "10.0.0.1", true),
            ping("b", "10.0.0.2", true),
            ping("c", "10.0.0.3", true),
        ],
    )
    .await;

    let companies: Value = hub.get_json("/api/dashboard/companies").await;

    assert_eq!(
        companies,
        json!([{
            "company_id": "acme",
            "name": "acme",
            "total_probes": 1,
            "active_probes": 1,
            "expired_probes": 0,
            "devices_up": 3,
            "devices_down": 0,
            "severity": "healthy",
            "last_report": "2026-03-10T12:01:00Z"
        }])
    );
}

#[tokio::test]
async fn dashboard_aggregates_several_probes_per_company_and_sorts_by_company_id() {
    let hq = TestProbe::new("hq");
    let branch = TestProbe::new("branch");
    let other = TestProbe::new("other");
    let hub = TestHub::new(&[&hq, &branch, &other]).await;

    hub.report_now(&other, "zeta", vec![ping("x", "10.9.0.1", true)])
        .await;
    hub.report_now(
        &hq,
        "acme",
        vec![ping("a", "10.0.0.1", true), ping("b", "10.0.0.2", false)],
    )
    .await;
    hub.report_now(&branch, "acme", vec![ping("c", "10.1.0.1", false)])
        .await;

    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;

    let ids: Vec<&str> = companies
        .iter()
        .map(|c| c["company_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["acme", "zeta"],
        "API sorts alphabetically; the UI re-sorts by severity"
    );
    assert_eq!(companies[0]["total_probes"], 2);
    assert_eq!(companies[0]["devices_up"], 1);
    assert_eq!(companies[0]["devices_down"], 2);
}

#[tokio::test]
async fn last_report_keeps_sub_second_precision() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::starting_at(at("2026-03-10T12:00:00.250Z"), &[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "10.0.0.1", true)])
        .await;

    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;

    assert_eq!(companies[0]["last_report"], "2026-03-10T12:00:00.250Z");
}

#[tokio::test]
async fn company_probes_show_whitelist_name_latest_host_and_status() {
    let hq = TestProbe::new("hq");
    let branch = TestProbe::new("branch");
    let hub = TestHub::new(&[&hq, &branch]).await;

    hub.report_now(
        &hq,
        "acme",
        vec![ping("a", "10.0.0.1", true), ping("b", "10.0.0.2", false)],
    )
    .await;
    hub.clock.advance(Duration::seconds(30));
    hub.report_now(&branch, "acme", vec![ping("c", "10.1.0.1", true)])
        .await;

    let probes: Vec<Value> = hub.get_json("/api/company/acme/probes").await;

    assert_eq!(probes.len(), 2);
    // Most recently seen first.
    assert_eq!(
        probes[0],
        json!({
            "probe_id": branch.id,
            "probe_name": "branch",
            "hostname": "branch-host",
            "site": "branch-site",
            "status": "active",
            "severity": "healthy",
            "last_seen_at": "2026-03-10T12:00:30Z",
            "devices_up": 1,
            "devices_down": 0
        })
    );
    assert_eq!(probes[1]["probe_name"], "hq");
    assert_eq!(probes[1]["devices_down"], 1);
}

#[tokio::test]
async fn unknown_company_has_no_probes() {
    let hub = TestHub::new(&[]).await;

    let probes: Value = hub.get_json("/api/company/nobody/probes").await;

    assert_eq!(probes, json!([]));
}

#[tokio::test]
async fn probe_devices_come_from_latest_report_sorted_by_name() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    hub.report_now(&probe, "acme", vec![ping("old-only", "10.0.0.9", true)])
        .await;
    hub.clock.advance(Duration::seconds(60));
    hub.report_now(
        &probe,
        "acme",
        vec![
            ping("switch", "10.0.0.1", false),
            metric("cpu", ResourceType::LocalCpu, 23.4, "%"),
            ping("gateway", "10.0.0.254", true),
        ],
    )
    .await;

    let devices: Vec<Value> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;

    let names: Vec<&str> = devices
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["cpu", "gateway", "switch"]);
    assert_eq!(
        devices[0],
        json!({
            "name": "cpu",
            "resource_type": "local_cpu",
            "target": "localhost",
            "status": "Up",
            "message": "cpu: 23.4%",
            "latency_ms": null,
            "metric_value": 23.4,
            "metric_unit": "%"
        })
    );
    assert_eq!(devices[1]["latency_ms"], 12);
    assert_eq!(devices[2]["status"], "Down");
}

// ---------------------------------------------------------------------------
// Severity (one rule, computed by the server: crate::model::Severity)
// ---------------------------------------------------------------------------

/// Two probes in one company: `live` stays active, `stale` expires.
async fn company_with_live_and_stale_probe(live_down: bool, stale_down: bool) -> TestHub {
    let live = TestProbe::new("live");
    let stale = TestProbe::new("stale");
    let hub = TestHub::new(&[&live, &stale]).await;
    hub.report_now(&stale, "acme", vec![ping("s", "10.1.0.1", !stale_down)])
        .await;
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 - 10));
    hub.report_now(&live, "acme", vec![ping("l", "10.0.0.1", !live_down)])
        .await;
    hub.clock.advance(Duration::seconds(20)); // `stale` passes the timeout, `live` doesn't
    assert_eq!(hub.sweep_expired().await.len(), 1);
    hub
}

async fn company_severity(hub: &TestHub) -> Value {
    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;
    companies[0]["severity"].clone()
}

#[tokio::test]
async fn live_outage_makes_the_company_critical_even_with_an_expired_probe() {
    // Previously the card showed amber here, hiding a real outage.
    let hub = company_with_live_and_stale_probe(true, false).await;

    assert_eq!(company_severity(&hub).await, "critical");
}

#[tokio::test]
async fn stale_down_devices_of_an_expired_probe_only_degrade_the_company() {
    // Previously the dashboard *sorted* this as critical from stale data.
    let hub = company_with_live_and_stale_probe(false, true).await;

    assert_eq!(company_severity(&hub).await, "degraded");
    let probes: Vec<Value> = hub.get_json("/api/company/acme/probes").await;
    let severities: Vec<(&str, &str)> = probes
        .iter()
        .map(|p| {
            (
                p["probe_name"].as_str().unwrap(),
                p["severity"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(severities.contains(&("stale", "degraded")));
    assert!(severities.contains(&("live", "healthy")));
}

#[tokio::test]
async fn company_with_only_live_probes_and_nothing_down_is_healthy() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "10.0.0.1", true)])
        .await;

    assert_eq!(company_severity(&hub).await, "healthy");
}
