//! Probe → hub ingestion: handshake, sealing, whitelist, signatures, persistence.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use support::*;

async fn count(hub: &TestHub, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(hub.pool())
        .await
        .expect("count rows")
}

#[tokio::test]
async fn handshake_returns_hub_public_key_as_hex() {
    let hub = TestHub::new(&[]).await;

    let response = hub.get("/api/handshake", None).await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.text(), hex::encode(hub.public_key()));
}

#[tokio::test]
async fn signed_report_from_whitelisted_probe_is_stored() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let report = probe.signed_report(
        "acme",
        hub.now(),
        vec![ping("gw", "10.0.0.1", true), ping("nas", "10.0.0.2", false)],
    );

    assert_eq!(hub.send_report(&report).await, StatusCode::OK);

    assert_eq!(count(&hub, "reports").await, 1);
    assert_eq!(count(&hub, "resource_statuses").await, 2);
    let (status, last_seen): (String, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT status, last_seen_at FROM probe_heartbeats WHERE probe_id = ?")
            .bind(&probe.id)
            .fetch_one(hub.pool())
            .await
            .expect("heartbeat row");
    assert_eq!(status, "active");
    assert_eq!(
        last_seen,
        hub.now(),
        "heartbeat uses the hub's clock, not the report timestamp"
    );
}

#[tokio::test]
async fn report_sealed_for_another_hub_is_rejected_as_unauthorized() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let other_hub = TestHub::new(&[&probe]).await;
    let report = probe.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);

    let sealed_for_other = seal(&report, &other_hub.public_key());
    let response = hub.post_bytes("/api/report", sealed_for_other).await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(count(&hub, "reports").await, 0);
}

#[tokio::test]
async fn truncated_payload_is_rejected_as_unauthorized() {
    let hub = TestHub::new(&[]).await;

    let response = hub.post_bytes("/api/report", vec![0u8; 20]).await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sealed_payload_that_is_not_a_report_is_a_bad_request() {
    let hub = TestHub::new(&[]).await;

    let sealed = seal(&serde_json::json!({ "hello": "world" }), &hub.public_key());
    let response = hub.post_bytes("/api/report", sealed).await;

    assert_eq!(response.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn report_from_unregistered_probe_is_forbidden_and_not_stored() {
    let registered = TestProbe::new("registered");
    let stranger = TestProbe::new("stranger");
    let hub = TestHub::new(&[&registered]).await;
    let report = stranger.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);

    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);
    assert_eq!(count(&hub, "reports").await, 0);
    assert_eq!(count(&hub, "probe_heartbeats").await, 0);
}

#[tokio::test]
async fn report_modified_after_signing_is_forbidden() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let mut report = probe.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    report.company_id = "someone-else".to_string();

    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);
    assert_eq!(count(&hub, "reports").await, 0);
}

#[tokio::test]
async fn report_impersonating_a_registered_probe_is_forbidden() {
    let victim = TestProbe::new("victim");
    let attacker = TestProbe::new("attacker");
    let hub = TestHub::new(&[&victim]).await;
    // The attacker claims the victim's (public) probe ID but can only sign with its own key.
    let mut report = attacker.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    report.probe_id = victim.id.clone();

    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);
    assert_eq!(count(&hub, "reports").await, 0);
}

/// Regression test for D-25 (fixed in M0-16): a probe's ID and the hub's key
/// are both public, so an unsigned report could be forged by anyone.
#[tokio::test]
async fn unsigned_report_is_forbidden_and_not_stored() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let report = probe.unsigned_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);

    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);
    assert_eq!(count(&hub, "reports").await, 0);
    assert_eq!(count(&hub, "probe_heartbeats").await, 0);
}

// ---------------------------------------------------------------------------
// Known issues: these assert the *intended* behaviour and are ignored until
// fixed. Run them with `cargo test-all -- --ignored`.
// ---------------------------------------------------------------------------

/// D-25: there is no replay protection. A captured sealed report can be resent
/// and is stored again.
#[tokio::test]
#[ignore = "known issue D-25: replayed reports are accepted"]
async fn known_issue_replayed_report_is_rejected() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let report = probe.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    let sealed = seal(&report, &hub.public_key());

    assert_eq!(
        hub.post_bytes("/api/report", sealed.clone()).await.status,
        StatusCode::OK
    );
    let replay = hub.post_bytes("/api/report", sealed).await;

    assert_ne!(replay.status, StatusCode::OK);
    assert_eq!(count(&hub, "reports").await, 1);
}
