//! The error contract (M0-11): every error response, on the web API and the
//! probe endpoints, is `{"error": {"code", "message"}}` with a stable code.
//! Missing sessions are covered route by route in `auth.rs`.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use serde_json::json;
use support::*;

// ---------------------------------------------------------------------------
// Web API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_malformed_request_body_is_a_json_error() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    let response = hub
        .post_json("/api/silence", Some(&cookie), json!({ "wrong": "fields" }))
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.error_code(), "invalid_body");
}

#[tokio::test]
async fn an_unknown_archive_is_not_found() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    let response = hub.get("/api/archive/42/companies", Some(&cookie)).await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.error_code(), "not_found");
}

#[tokio::test]
async fn a_storage_failure_is_internal_and_does_not_leak_details() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;
    sqlx::query("DROP TABLE reports")
        .execute(hub.pool())
        .await
        .unwrap();

    let response = hub.get("/api/dashboard/companies", Some(&cookie)).await;

    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.error_code(), "internal");
    let text = response.text();
    assert!(!text.contains("reports"), "SQL details leaked: {text}");
}

// ---------------------------------------------------------------------------
// Probe endpoint: one code per rejection (the probe logs them)
// ---------------------------------------------------------------------------

async fn report_error(hub: &TestHub, sealed: Vec<u8>) -> (StatusCode, String) {
    let response = hub.post_bytes("/api/report", sealed).await;
    (response.status, response.error_code())
}

#[tokio::test]
async fn every_report_rejection_has_its_own_code() {
    let probe = TestProbe::new("hq");
    let stranger = TestProbe::new("stranger");
    let hub = TestHub::new(&[&probe]).await;
    let key = hub.public_key();
    let now = hub.now();
    let resources = || vec![ping("gw", "10.0.0.1", true)];

    let accepted = probe.signed_report("acme", now, resources());
    assert_eq!(hub.send_report(&accepted).await, StatusCode::OK);

    let mut tampered = probe.signed_report("acme", now + Duration::seconds(1), resources());
    tampered.company_id = "someone-else".to_string();
    let mut future_version = probe.unsigned_report("acme", now + Duration::seconds(2), resources());
    future_version.protocol_version = common::protocol::PROTOCOL_VERSION + 1;
    probe.sign(&mut future_version);

    let cases = [
        (
            b"garbage".to_vec(),
            StatusCode::UNAUTHORIZED,
            "undecryptable",
        ),
        (
            seal(
                &json!({ "protocol_version": common::protocol::MIN_SUPPORTED_PROTOCOL_VERSION }),
                &key,
            ),
            StatusCode::BAD_REQUEST,
            "not_a_report",
        ),
        (
            seal(&future_version, &key),
            StatusCode::BAD_REQUEST,
            "unsupported_protocol_version",
        ),
        (
            seal(&stranger.signed_report("acme", now, resources()), &key),
            StatusCode::FORBIDDEN,
            "unknown_probe",
        ),
        (
            seal(&probe.unsigned_report("acme", now, resources()), &key),
            StatusCode::FORBIDDEN,
            "missing_signature",
        ),
        (
            seal(&tampered, &key),
            StatusCode::FORBIDDEN,
            "bad_signature",
        ),
        (
            seal(
                &probe.signed_report("acme", now + Duration::hours(1), resources()),
                &key,
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
            "timestamp_rejected",
        ),
        (seal(&accepted, &key), StatusCode::CONFLICT, "not_newer"),
    ];
    for (sealed, status, code) in cases {
        assert_eq!(report_error(&hub, sealed).await, (status, code.to_string()));
    }
}
