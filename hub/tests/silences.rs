//! Alert silences via the API, including expiry against the hub clock.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::{CheckId, CheckSpec};
use hub_lib::service::alerts::silence;
use serde_json::{Value, json};
use support::*;

/// A hub whose probe has a NAS check; its ID.
async fn hub_with_check() -> (TestHub, String) {
    let probe = TestProbe::new("probe1");
    let hub = TestHub::new(&[&probe]).await;
    let spec = CheckSpec::Ping {
        host: "10.0.0.20".into(),
    };
    let check = hub.add_check(&probe, "NAS", spec).await;
    (hub, check)
}

async fn create(hub: &TestHub, cookie: &str, check: &str, reason: &str, hours: Option<u32>) -> i64 {
    let response = hub
        .post_json(
            "/api/silence",
            Some(cookie),
            json!({ "check_id": check, "reason": reason, "duration_hours": hours }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body: Value = response.json();
    assert_eq!(body["success"], true);
    body["silence_id"].as_i64().expect("silence_id")
}

#[tokio::test]
async fn create_list_and_delete_a_silence() {
    let (hub, check) = hub_with_check().await;
    let cookie = hub.login().await;

    let id = create(&hub, &cookie, &check, "maintenance", Some(24)).await;

    let silences: Value = hub.get_json("/api/silences").await;
    assert_eq!(
        silences,
        json!([{
            "id": id,
            "scope_type": "check",
            "scope_id": check,
            "company_id": "acme",
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
    let (hub, check) = hub_with_check().await;
    let cookie = hub.login().await;
    create(&hub, &cookie, &check, "reboot", Some(1)).await;
    let id = CheckId(check);

    hub.clock.advance(Duration::minutes(59));
    assert!(
        silence::is_check_silenced(hub.store(), &id, hub.now())
            .await
            .unwrap()
    );

    hub.clock.advance(Duration::minutes(2));
    assert!(
        !silence::is_check_silenced(hub.store(), &id, hub.now())
            .await
            .unwrap()
    );
    assert_eq!(hub.get_json::<Value>("/api/silences").await, json!([]));
}

#[tokio::test]
async fn forever_silence_never_expires() {
    let (hub, check) = hub_with_check().await;
    let cookie = hub.login().await;
    create(&hub, &cookie, &check, "decommissioned", None).await;

    hub.clock.advance(Duration::days(3650));

    let silences: Vec<Value> = hub.get_json("/api/silences").await;
    assert_eq!(silences.len(), 1);
    assert_eq!(silences[0]["expires_at"], Value::Null);
}

#[tokio::test]
async fn silencing_the_same_check_again_updates_the_existing_silence() {
    let (hub, check) = hub_with_check().await;
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
async fn only_an_existing_check_can_be_silenced_and_a_deleted_ones_silence_is_hidden() {
    let (hub, check) = hub_with_check().await;
    let cookie = hub.login().await;

    let unknown = json!({ "check_id": "no-such-check", "reason": "x", "duration_hours": 1 });
    let response = hub.post_json("/api/silence", Some(&cookie), unknown).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

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
