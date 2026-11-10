#![cfg(feature = "ssr")]
//! M6-01 integration tests: workflow verification and API contracts

mod support;

use hub_lib::model::{CompanyInfo, IncidentInfo, UptimeBucket};
use support::*;

#[tokio::test]
async fn m6_02_timeline_returns_company_uptime() {
    // Verify M6-02 Timeline page can list companies and fetch their uptime
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    // Seed uptime data for a company spanning 24h
    hub.seed_report(&probe.signed_report(
        "acme",
        at("2026-03-09T12:00:00Z"),
        vec![ping("api", "1", true)],
    ))
    .await;
    hub.seed_report(&probe.signed_report(
        "acme",
        at("2026-03-10T12:00:00Z"),
        vec![ping("api", "1", true), ping("web", "2", false)],
    ))
    .await;

    // Fetch uptime data - should be aggregated at company level
    let uptime: Vec<UptimeBucket> = hub.get_json("/api/company/acme/uptime?range=24h").await;
    assert!(!uptime.is_empty(), "Uptime buckets should be returned for company");
    // Each bucket should have up_count and down_count aggregated across all checks in company
    assert!(uptime[0].up_count > 0 || uptime[0].down_count > 0);
}

#[tokio::test]
async fn m6_03_problems_lists_open_incidents() {
    // Verify M6-03 Problems page can fetch open incidents
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    // Trigger an incident by creating a down status
    hub.evaluate_report(&probe, vec![ping("api", "1", false), ping("api", "1", false)])
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
    assert!(report.get("html").is_some(), "HTML report should be generated");
}

#[tokio::test]
async fn timeline_company_selector_lists_companies() {
    // Timeline page dropdown needs to list all companies
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    // Create "zeta" company explicitly (TestHub::new only creates "acme")
    hub.place_probe(&probe.id, &probe.name, "zeta", "zeta-site")
        .await;

    // Create checks for multiple companies in chronological order
    hub.seed_report(&probe.signed_report(
        "acme",
        at("2026-03-09T12:00:00Z"),
        vec![ping("api", "1", true)],
    ))
    .await;
    hub.seed_report(&probe.signed_report(
        "zeta",
        at("2026-03-10T12:00:00Z"),
        vec![ping("dns", "2", true)],
    ))
    .await;

    // List companies
    let companies: Vec<CompanyInfo> = hub.get_json("/api/companies").await;
    let company_ids: Vec<String> = companies.into_iter().map(|c| c.id).collect();
    assert!(
        company_ids.iter().any(|id| id == "acme"),
        "Should list acme company"
    );
    assert!(
        company_ids.iter().any(|id| id == "zeta"),
        "Should list zeta company"
    );
}
