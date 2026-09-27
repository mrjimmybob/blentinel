//! Alert silences via the API, including expiry against the hub clock.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use hub_lib::service::alerts::silence;
use serde_json::{json, Value};
use support::*;

const KEY: &str = "acme:probe1:NAS:10.0.0.20";

async fn create(hub: &TestHub, cookie: &str, key: &str, reason: &str, hours: Option<u32>) -> i64 {
    let response = hub
        .post_json(
            "/api/silence",
            Some(cookie),
            json!({ "resource_key": key, "reason": reason, "duration_hours": hours }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body: Value = response.json();
    assert_eq!(body["success"], true);
    body["silence_id"].as_i64().expect("silence_id")
}

#[tokio::test]
async fn create_list_and_delete_a_silence() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    let id = create(&hub, &cookie, KEY, "maintenance", Some(24)).await;

    let silences: Value = hub.get_json("/api/silences").await;
    assert_eq!(
        silences,
        json!([{
            "id": id,
            "scope_type": "resource",
            "scope_id": KEY,
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
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;
    create(&hub, &cookie, KEY, "reboot", Some(1)).await;

    hub.clock.advance(Duration::minutes(59));
    assert!(silence::is_resource_silenced(hub.store(), KEY, hub.now())
        .await
        .unwrap());

    hub.clock.advance(Duration::minutes(2));
    assert!(!silence::is_resource_silenced(hub.store(), KEY, hub.now())
        .await
        .unwrap());
    assert_eq!(hub.get_json::<Value>("/api/silences").await, json!([]));
}

#[tokio::test]
async fn forever_silence_never_expires() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;
    create(&hub, &cookie, KEY, "decommissioned", None).await;

    hub.clock.advance(Duration::days(3650));

    let silences: Vec<Value> = hub.get_json("/api/silences").await;
    assert_eq!(silences.len(), 1);
    assert_eq!(silences[0]["expires_at"], Value::Null);
}

#[tokio::test]
async fn silencing_the_same_resource_again_updates_the_existing_silence() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    let first = create(&hub, &cookie, KEY, "first", Some(1)).await;
    let second = create(&hub, &cookie, KEY, "second", None).await;

    assert_eq!(first, second);
    let silences: Vec<Value> = hub.get_json("/api/silences").await;
    assert_eq!(silences.len(), 1);
    assert_eq!(silences[0]["reason"], "second");
    assert_eq!(silences[0]["expires_at"], Value::Null);
}
