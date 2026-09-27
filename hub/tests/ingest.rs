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
// Protocol version (M0-06)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn responses_carry_the_hub_protocol_version() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let expected = common::protocol::PROTOCOL_VERSION.to_string();

    let handshake = hub.get("/api/handshake", None).await;
    let rejected = hub.post_bytes("/api/report", vec![0u8; 20]).await;

    for response in [handshake, rejected] {
        assert_eq!(
            response.headers[common::protocol::PROTOCOL_HEADER],
            expected.as_str()
        );
    }
}

#[tokio::test]
async fn legacy_v1_report_without_version_field_is_still_accepted() {
    let probe = TestProbe::new("legacy");
    let hub = TestHub::new(&[&probe]).await;
    let mut report = probe.unsigned_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    report.protocol_version = None; // exactly what pre-v2 probes send
    probe.sign(&mut report);

    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

#[tokio::test]
async fn unsupported_protocol_version_is_a_bad_request() {
    let probe = TestProbe::new("future");
    let hub = TestHub::new(&[&probe]).await;
    let mut report = probe.unsigned_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    report.protocol_version = Some(common::protocol::PROTOCOL_VERSION + 1);
    probe.sign(&mut report);

    let response = hub
        .post_bytes("/api/report", seal(&report, &hub.public_key()))
        .await;

    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(response.text().contains("unsupported protocol version"));
}

#[tokio::test]
async fn protocol_version_is_covered_by_the_signature() {
    // Downgrading a signed v2 report to "v1" (to dodge future v2-only checks)
    // must break the signature.
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let mut report = probe.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    report.protocol_version = None;

    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// Replay protection (M0-17, D-25)
// ---------------------------------------------------------------------------

/// Regression test for D-25 part 2: a captured sealed report used to be stored again.
#[tokio::test]
async fn replayed_report_is_rejected_as_conflict() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let report = probe.signed_report("acme", hub.now(), vec![ping("gw", "10.0.0.1", true)]);
    let sealed = seal(&report, &hub.public_key());

    assert_eq!(
        hub.post_bytes("/api/report", sealed.clone()).await.status,
        StatusCode::OK
    );
    let replay = hub.post_bytes("/api/report", sealed).await;

    assert_eq!(replay.status, StatusCode::CONFLICT);
    assert_eq!(count(&hub, "reports").await, 1);
}

#[tokio::test]
async fn report_older_than_the_probes_latest_is_rejected() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let earlier = probe.signed_report(
        "acme",
        hub.now() - chrono::Duration::minutes(1),
        vec![ping("gw", "1", true)],
    );
    let later = probe.signed_report("acme", hub.now(), vec![ping("gw", "1", true)]);

    assert_eq!(hub.send_report(&later).await, StatusCode::OK);

    assert_eq!(hub.send_report(&earlier).await, StatusCode::CONFLICT);
    assert_eq!(count(&hub, "reports").await, 1);
}

#[tokio::test]
async fn ordering_is_exact_to_the_millisecond() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let t = hub.now();
    let first = probe.signed_report("acme", t, vec![ping("gw", "1", true)]);
    let ms_later = probe.signed_report(
        "acme",
        t + chrono::Duration::milliseconds(1),
        vec![ping("gw", "1", true)],
    );
    let ms_earlier = probe.signed_report(
        "acme",
        t - chrono::Duration::milliseconds(1),
        vec![ping("gw", "1", true)],
    );

    assert_eq!(hub.send_report(&first).await, StatusCode::OK);
    assert_eq!(hub.send_report(&ms_later).await, StatusCode::OK);
    assert_eq!(hub.send_report(&ms_earlier).await, StatusCode::CONFLICT);
}

#[tokio::test]
async fn replay_check_is_per_probe() {
    let a = TestProbe::new("a");
    let b = TestProbe::new("b");
    let hub = TestHub::new(&[&a, &b]).await;

    // Both probes reporting the same instant is normal, not a replay.
    assert_eq!(
        hub.send_report(&a.signed_report("acme", hub.now(), vec![]))
            .await,
        StatusCode::OK
    );
    assert_eq!(
        hub.send_report(&b.signed_report("acme", hub.now(), vec![]))
            .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn buffered_reports_delivered_in_order_after_an_outage_are_accepted() {
    // FP-4: a probe that was offline for days flushes its backlog oldest-first.
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let now = hub.now();

    for days_ago in [6, 3, 1, 0] {
        let report = probe.signed_report(
            "acme",
            now - chrono::Duration::days(days_ago),
            vec![ping("gw", "1", true)],
        );
        assert_eq!(
            hub.send_report(&report).await,
            StatusCode::OK,
            "{days_ago} days old"
        );
    }
    assert_eq!(count(&hub, "reports").await, 4);
}

#[tokio::test]
async fn report_older_than_any_probe_buffers_is_unprocessable() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let max_age = chrono::Duration::seconds(
        common::protocol::MAX_BUFFERED_REPORT_AGE_SECS + common::protocol::MAX_CLOCK_SKEW_SECS,
    );
    let ancient = probe.signed_report(
        "acme",
        hub.now() - max_age - chrono::Duration::seconds(1),
        vec![],
    );

    let response = hub
        .post_bytes("/api/report", seal(&ancient, &hub.public_key()))
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count(&hub, "reports").await, 0);
}

#[tokio::test]
async fn report_from_a_probe_clock_far_in_the_future_is_unprocessable() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let ahead = probe.signed_report("acme", hub.now() + chrono::Duration::minutes(6), vec![]);

    let response = hub
        .post_bytes("/api/report", seal(&ahead, &hub.public_key()))
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.text().contains("future"));
}

#[tokio::test]
async fn window_is_checked_only_after_the_signature() {
    // An attacker must not learn anything about the window from forged reports.
    let victim = TestProbe::new("victim");
    let attacker = TestProbe::new("attacker");
    let hub = TestHub::new(&[&victim]).await;
    let mut forged = attacker.signed_report("acme", hub.now() + chrono::Duration::days(1), vec![]);
    forged.probe_id = victim.id.clone();

    assert_eq!(hub.send_report(&forged).await, StatusCode::FORBIDDEN);
}
