//! Admin endpoints: listings, data deletion, probe removal, archives.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use support::*;

async fn hub_with_two_companies() -> (TestHub, TestProbe, TestProbe) {
    let hq = TestProbe::new("hq");
    let other = TestProbe::new("other");
    let hub = TestHub::new(&[&hq, &other]).await;
    hub.report_now(&hq, "acme", vec![ping("a", "1", true)])
        .await;
    hub.report_now(&other, "zeta", vec![ping("z", "9", false)])
        .await;
    (hub, hq, other)
}

async fn reports_for(hub: &TestHub, probe: &TestProbe) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE probe_id = ?")
        .bind(&probe.id)
        .fetch_one(hub.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn admin_lists_companies_and_probes() {
    let (hub, hq, other) = hub_with_two_companies().await;

    let companies: Value = hub.get_json("/api/admin/companies").await;
    let probes: Vec<Value> = hub.get_json("/api/admin/probes").await;

    assert_eq!(companies, json!(["acme", "zeta"]));
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0]["probe_id"], hq.id);
    assert_eq!(probes[0]["company_id"], "acme");
    assert_eq!(probes[0]["first_seen_at"], "2026-03-10T12:00:00Z");
    assert_eq!(probes[1]["probe_id"], other.id);
}

#[tokio::test]
async fn deleting_company_data_keeps_its_probes_listed() {
    let (hub, hq, other) = hub_with_two_companies().await;
    let cookie = hub.login().await;

    let response = hub
        .post_json(
            "/api/admin/delete-company-data",
            Some(&cookie),
            json!({ "company_id": "acme" }),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(reports_for(&hub, &hq).await, 0);
    assert_eq!(
        reports_for(&hub, &other).await,
        1,
        "other companies are untouched"
    );
    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(companies[0]["company_id"], "acme");
    assert_eq!(companies[0]["total_probes"], 1);
    assert_eq!(companies[0]["devices_up"], 0);
    assert_eq!(companies[0]["last_report"], Value::Null);
}

#[tokio::test]
async fn deleting_probe_data_keeps_the_probe() {
    let (hub, hq, _) = hub_with_two_companies().await;
    let cookie = hub.login().await;

    let response = hub
        .post_json(
            "/api/admin/delete-probe-data",
            Some(&cookie),
            json!({ "probe_id": hq.id }),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(reports_for(&hub, &hq).await, 0);
    let probes: Vec<Value> = hub.get_json("/api/company/acme/probes").await;
    assert_eq!(probes.len(), 1);
}

#[tokio::test]
async fn removing_a_probe_deletes_it_and_its_now_empty_company() {
    let (hub, hq, _) = hub_with_two_companies().await;
    let cookie = hub.login().await;

    let response = hub
        .post_json(
            "/api/admin/remove-probe",
            Some(&cookie),
            json!({ "probe_id": hq.id }),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK);
    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;
    let ids: Vec<&str> = companies
        .iter()
        .map(|c| c["company_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["zeta"]);
}

#[tokio::test]
async fn removed_probe_reappears_if_it_is_still_whitelisted_and_reports() {
    // Removal deletes data, not trust: the TOML whitelist still accepts the probe.
    let (hub, hq, _) = hub_with_two_companies().await;
    let cookie = hub.login().await;
    hub.post_json(
        "/api/admin/remove-probe",
        Some(&cookie),
        json!({ "probe_id": hq.id }),
    )
    .await;

    hub.report_now(&hq, "acme", vec![ping("a", "1", true)])
        .await;

    let probes: Vec<Value> = hub.get_json("/api/company/acme/probes").await;
    assert_eq!(probes.len(), 1);
}

#[tokio::test]
async fn storage_info_reports_retention_settings() {
    let (hub, _, _) = hub_with_two_companies().await;

    let info: Value = hub.get_json("/api/admin/storage-info").await;

    assert_eq!(info["warn_threshold_mb"], 1000);
    assert_eq!(info["retention_days"], 90);
    assert_eq!(info["last_archive_date"], Value::Null);
    assert_eq!(info["warning"], false);
}

// ---------------------------------------------------------------------------
// Archives. The archive engine reads the real system clock (not yet injected),
// so these hubs start at the real "now" and use data older than the cutoff.
// ---------------------------------------------------------------------------

async fn hub_with_archive() -> (TestHub, TestProbe, i64) {
    let probe = TestProbe::new("hq");
    let hub = TestHub::starting_at(Utc::now(), &[&probe]).await;
    let old = probe.signed_report(
        "acme",
        hub.now() - Duration::days(100),
        vec![ping("a", "1", false)],
    );
    hub.seed_report(&old).await; // too old to ingest; seed it as history
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await; // probe is still alive today

    let cookie = hub.login().await;
    let response = hub
        .post_json(
            "/api/admin/archive",
            Some(&cookie),
            json!({ "cutoff_days": 90 }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json::<Value>()["success"], true);

    let archives: Vec<Value> = hub.get_json("/api/admin/archives").await;
    assert_eq!(archives.len(), 1);
    let id = archives[0]["id"].as_i64().unwrap();
    (hub, probe, id)
}

#[tokio::test]
async fn archiving_moves_old_reports_out_of_the_main_database() {
    let (hub, probe, _) = hub_with_archive().await;

    assert_eq!(
        reports_for(&hub, &probe).await,
        1,
        "only the recent report remains"
    );
    let info: Value = hub.get_json("/api/admin/storage-info").await;
    assert!(info["last_archive_date"].is_string());
}

/// D-26: the archive keeps only heartbeats of probes that went silent before the
/// cutoff, and the archive dashboard groups by heartbeat, so a probe that is
/// still reporting today vanishes from the archive view despite its old reports.
#[tokio::test]
#[ignore = "known issue D-26: archive view hides probes that are still alive"]
async fn known_issue_archive_shows_companies_whose_probes_are_still_reporting() {
    let (hub, _, id) = hub_with_archive().await;

    let companies: Vec<Value> = hub.get_json(&format!("/api/archive/{id}/companies")).await;

    assert_eq!(companies.len(), 1);
    assert_eq!(companies[0]["company_id"], "acme");
}

/// D-26: archive uptime is computed for "the last 24 h relative to now", but an
/// archive only contains data older than its cutoff, so it is always empty.
#[tokio::test]
#[ignore = "known issue D-26: archive uptime is always empty"]
async fn known_issue_archive_uptime_contains_archived_data() {
    let (hub, _, id) = hub_with_archive().await;

    let buckets: Vec<Value> = hub
        .get_json(&format!("/api/archive/{id}/company/acme/uptime"))
        .await;

    assert!(!buckets.is_empty());
}
