#![cfg(feature = "ssr")]
//! M6-01 integration tests: workflow verification and API contracts

mod support;

use hub_lib::model::IncidentInfo;
use support::*;

#[tokio::test]
async fn m6_03_problems_lists_open_incidents() {
    // Verify M6-03 Problems page can fetch open incidents
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    // Trigger an incident by creating a down status
    hub.evaluate_report(
        &probe,
        vec![ping("api", "1", false), ping("api", "1", false)],
    )
    .await;

    // Fetch open incidents
    let incidents: Vec<IncidentInfo> = hub.get_json("/api/incidents?state=open").await;
    assert!(
        !incidents.is_empty(),
        "Should list open incidents when checks are down"
    );
    // Status is "Down" (title case) in the API
    assert_eq!(incidents[0].status.as_str(), "Down");
}

#[tokio::test]
async fn m6_05_reports_generates_sla() {
    // Verify M6-05 Reports page can generate HTML SLA reports
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    // Seed check results in chronological order for February 2026
    hub.seed_report(&probe.signed_report(
        "acme",
        at("2026-02-10T12:00:00Z"),
        vec![ping("check1", "1", true)],
    ))
    .await;
    hub.seed_report(&probe.signed_report(
        "acme",
        at("2026-02-15T12:00:00Z"),
        vec![ping("check1", "1", true)],
    ))
    .await;

    // Generate SLA report using the check_id format: {probe_id}.{check_name}
    let check_id = check_id(&probe.id, "check1");
    let report: serde_json::Value = hub
        .get_json(&format!(
            "/api/generate-sla-report?check_id={}&month=2&year=2026&sla_target_pct=99.0&format=html",
            check_id
        ))
        .await;

    assert_eq!(report.get("format").and_then(|v| v.as_str()), Some("html"));
    assert!(
        report.get("html").is_some(),
        "HTML report should be generated"
    );
}
