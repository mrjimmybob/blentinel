//! Alert silences via the API, including expiry against the hub clock, and
//! silences wider than a check: a probe, a site, a whole company (D-65).
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::CheckSpec;
use serde_json::{Value, json};
use support::*;

/// A hub whose probe has a NAS check; its ID.
async fn hub_with_check() -> (TestHub, TestProbe, String) {
    let probe = TestProbe::new("probe1");
    let hub = TestHub::new(&[&probe]).await;
    let spec = CheckSpec::ping("10.0.0.20");
    let check = hub.add_check(&probe, "NAS", spec).await;
    (hub, probe, check)
}

async fn silence(
    hub: &TestHub,
    cookie: &str,
    scope: Value,
    reason: &str,
    hours: Option<u32>,
) -> TestResponse {
    hub.post_json(
        "/api/silence",
        Some(cookie),
        json!({ "company_id": "acme", "scope": scope, "reason": reason, "duration_hours": hours }),
    )
    .await
}

async fn create(hub: &TestHub, cookie: &str, check: &str, reason: &str, hours: Option<u32>) -> i64 {
    let response = silence(
        hub,
        cookie,
        json!({ "kind": "check", "check_id": check }),
        reason,
        hours,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body: Value = response.json();
    assert_eq!(body["success"], true);
    body["silence_id"].as_i64().expect("silence_id")
}

async fn silenced(hub: &TestHub, probe: &TestProbe, check: Option<&str>) -> bool {
    let site = format!("{}-site", probe.name);
    hub.store()
        .is_silenced("acme", &site, &probe.id, check, hub.now())
        .await
        .unwrap()
}

#[tokio::test]
async fn create_list_and_delete_a_silence() {
    let (hub, _, check) = hub_with_check().await;
    let cookie = hub.login().await;

    let id = create(&hub, &cookie, &check, "maintenance", Some(24)).await;

    let silences: Value = hub.get_json("/api/silences").await;
    assert_eq!(
        silences,
        json!([{
            "id": id,
            "company_id": "acme",
            "scope": { "kind": "check", "check_id": check },
            "reason": "maintenance",
            "created_at": "2026-03-10T12:00:00Z",
            "expires_at": "2026-03-11T12:00:00Z"
        }])
    );

    let deleted = hub
        .post_json(
            "/api/silence/delete",
            Some(&cookie),
            json!({ "silence_id": id }),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.json::<Value>(), json!({ "success": true }));
    assert_eq!(hub.get_json::<Value>("/api/silences").await, json!([]));
}

#[tokio::test]
async fn timed_silence_expires_on_the_hub_clock() {
    let (hub, probe, check) = hub_with_check().await;
    let cookie = hub.login().await;
    create(&hub, &cookie, &check, "reboot", Some(1)).await;

    hub.clock.advance(Duration::minutes(59));
    assert!(silenced(&hub, &probe, Some(&check)).await);

    hub.clock.advance(Duration::minutes(2));
    assert!(!silenced(&hub, &probe, Some(&check)).await);
    assert_eq!(hub.get_json::<Value>("/api/silences").await, json!([]));
}

#[tokio::test]
async fn forever_silence_never_expires() {
    let (hub, _, check) = hub_with_check().await;
    let cookie = hub.login().await;
    create(&hub, &cookie, &check, "decommissioned", None).await;

    hub.clock.advance(Duration::days(3650));

    let silences: Vec<Value> = hub.get_json("/api/silences").await;
    assert_eq!(silences.len(), 1);
    assert_eq!(silences[0]["expires_at"], Value::Null);
}

#[tokio::test]
async fn silencing_the_same_check_again_updates_the_existing_silence() {
    let (hub, _, check) = hub_with_check().await;
    let cookie = hub.login().await;

    let first = create(&hub, &cookie, &check, "first", Some(1)).await;
    let second = create(&hub, &cookie, &check, "second", None).await;

    assert_eq!(first, second);
    let silences: Vec<Value> = hub.get_json("/api/silences").await;
    assert_eq!(silences.len(), 1);
    assert_eq!(silences[0]["reason"], "second");
    assert_eq!(silences[0]["expires_at"], Value::Null);
}

#[tokio::test]
async fn only_an_existing_check_can_be_silenced_and_a_deleted_ones_silence_goes() {
    let (hub, _, check) = hub_with_check().await;
    let cookie = hub.login().await;

    let unknown = silence(
        &hub,
        &cookie,
        json!({ "kind": "check", "check_id": "no-such-check" }),
        "x",
        Some(1),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert!(unknown.text().contains("no such check"));

    create(&hub, &cookie, &check, "maintenance", None).await;
    let deleted = hub
        .post_json(
            &format!("/api/checks/{check}/delete"),
            Some(&cookie),
            json!({}),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    assert_eq!(hub.get_json::<Value>("/api/silences").await, json!([]));
}

#[tokio::test]
async fn a_probe_site_or_company_silence_covers_everything_under_it() {
    let (hub, probe, check) = hub_with_check().await;
    let cookie = hub.login().await;
    let site = format!("{}-site", probe.name);

    for scope in [
        json!({ "kind": "probe", "probe_id": probe.id }),
        json!({ "kind": "site", "site_id": site }),
        json!({ "kind": "company" }),
    ] {
        let response = silence(&hub, &cookie, scope.clone(), "planned move", Some(2)).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{scope}: {}",
            response.text()
        );
        let id = response.json::<Value>()["silence_id"].as_i64().unwrap();
        assert!(
            silenced(&hub, &probe, Some(&check)).await,
            "{scope}: its checks"
        );
        assert!(
            silenced(&hub, &probe, None).await,
            "{scope}: the probe itself"
        );
        hub.post_json(
            "/api/silence/delete",
            Some(&cookie),
            json!({ "silence_id": id }),
        )
        .await;
        assert!(!silenced(&hub, &probe, Some(&check)).await);
    }

    // A check's silence covers that check only, not the probe's own alert.
    create(&hub, &cookie, &check, "flaky", None).await;
    assert!(!silenced(&hub, &probe, None).await);

    let elsewhere = silence(
        &hub,
        &cookie,
        json!({ "kind": "site", "site_id": "no-such-site" }),
        "x",
        None,
    )
    .await;
    assert_eq!(elsewhere.status, StatusCode::BAD_REQUEST);
}
