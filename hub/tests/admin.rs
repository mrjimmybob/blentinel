//! Admin endpoints: listings, data deletion, probe removal, archives.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Timelike, Utc};
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

/// D-26: a probe that is still reporting today appears in the archive, with
/// its old reports, as last seen at its last archived report.
#[tokio::test]
async fn archive_shows_probes_that_are_still_reporting() {
    let (hub, probe, id) = hub_with_archive().await;

    let companies: Vec<Value> = hub.get_json(&format!("/api/archive/{id}/companies")).await;
    assert_eq!(companies.len(), 1);
    assert_eq!(companies[0]["company_id"], "acme");
    assert_eq!(
        companies[0]["devices_down"], 1,
        "the archived report's state"
    );

    let probes: Vec<Value> = hub
        .get_json(&format!("/api/archive/{id}/company/acme/probes"))
        .await;
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0]["probe_id"], probe.id);
    let last_seen: DateTime<Utc> = probes[0]["last_seen_at"].as_str().unwrap().parse().unwrap();
    assert!(
        last_seen < hub.now() - Duration::days(90),
        "last seen is capped at the archive's data, not today: {last_seen}"
    );
}

/// D-26: archive uptime ends at the archive's newest report, not at "now",
/// so every range shows the archived data.
#[tokio::test]
async fn archive_uptime_contains_archived_data_for_every_range() {
    let (hub, _, id) = hub_with_archive().await;

    for range in ["24h", "7d", "30d", "all"] {
        let buckets: Vec<Value> = hub
            .get_json(&format!(
                "/api/archive/{id}/company/acme/uptime?range={range}"
            ))
            .await;
        assert_eq!(buckets.len(), 1, "range {range}");
        assert_eq!(buckets[0]["down_count"], 1, "range {range}");
    }
}

/// D-26: the range parameter is honoured: it picks the bucket size.
#[tokio::test]
async fn archive_uptime_honours_the_range() {
    let (hub, _, id) = hub_with_archive().await;
    let archived_at = hub.now() - Duration::days(100); // see hub_with_archive
    let quarter_hour = archived_at
        .with_minute(archived_at.minute() / 15 * 15)
        .unwrap();

    let bucket = async |range: &str| -> String {
        let buckets: Vec<Value> = hub
            .get_json(&format!(
                "/api/archive/{id}/company/acme/uptime?range={range}"
            ))
            .await;
        buckets[0]["bucket"].as_str().unwrap().to_string()
    };
    assert_eq!(
        bucket("24h").await,
        quarter_hour.format("%Y-%m-%d %H:%M").to_string(),
        "15-minute buckets"
    );
    assert_eq!(
        bucket("all").await,
        archived_at.format("%Y-%m-%d 00:00").to_string(),
        "1-day buckets"
    );
}
