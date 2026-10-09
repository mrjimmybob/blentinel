//! Asking for commands between reports (M4-03, D-76): the hub holds a
//! probe's poll until a command is queued, answers with what is waiting and
//! when to ask again, and refuses polls it can't trust. The tests' hub holds
//! polls for 2 s (production: `common::poll::HOLD_SECS`).
#![cfg(feature = "ssr")]

mod support;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::commands::CommandSpec;
use serde_json::json;
use support::*;

async fn queue_run_checks(hub: &TestHub, cookie: &str, probe: &TestProbe) -> TestResponse {
    hub.post_json(
        &format!("/api/probes/{}/commands", probe.id),
        Some(cookie),
        json!({ "kind": "run_checks" }),
    )
    .await
}

#[tokio::test]
async fn a_held_poll_is_answered_as_soon_as_a_command_is_queued() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let started = Instant::now();
    let ((status, answer), queued) =
        tokio::join!(hub.poll(&probe, chrono::Duration::zero()), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            queue_run_checks(&hub, &owner, &probe).await
        });
    assert_eq!(queued.status, StatusCode::OK, "{}", queued.text());
    assert_eq!(status, StatusCode::OK);
    let answer = answer.expect("a trusted answer");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "answered when the command was queued, not at the end of the hold: {:?}",
        started.elapsed()
    );
    assert_eq!(answer.commands.len(), 1);
    assert_eq!(answer.commands[0].spec, CommandSpec::RunChecks);
    assert_eq!(answer.next_poll_secs, Some(0), "ask again at once");

    // Waiting already: answered at once, again until its result comes back.
    let started = Instant::now();
    let (_, again) = hub.poll(&probe, chrono::Duration::zero()).await;
    assert_eq!(again.unwrap().commands.len(), 1);
    assert!(started.elapsed() < Duration::from_millis(1000));
}

#[tokio::test]
async fn with_nothing_waiting_the_hub_holds_then_says_ask_again() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let started = Instant::now();
    let (status, answer) = hub.poll(&probe, chrono::Duration::zero()).await;
    assert_eq!(status, StatusCode::OK);
    let answer = answer.expect("a trusted answer");
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "held for the whole hold"
    );
    assert!(answer.commands.is_empty());
    assert_eq!(answer.next_poll_secs, Some(0));
    assert_eq!(hub.state.command_waiters.held(), 0, "nothing left held");
}

#[tokio::test]
async fn polls_that_cant_be_trusted_are_refused() {
    let probe = TestProbe::new("hq");
    let stranger = TestProbe::new("nobody");
    let hub = TestHub::new(&[&probe]).await;

    let (status, _) = hub.poll(&stranger, chrono::Duration::zero()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "not enrolled");

    let (status, _) = hub.poll(&probe, chrono::Duration::minutes(10)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "too far from the hub's clock"
    );

    let garbage = hub
        .post_bytes(common::poll::POLL_PATH, b"not sealed".to_vec())
        .await;
    assert_eq!(garbage.status, StatusCode::UNAUTHORIZED);

    let owner = hub.login().await;
    let revoked = hub
        .post_json(
            &format!("/api/probes/{}/revoke", probe.id),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    let (status, _) = hub.poll(&probe, chrono::Duration::zero()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "revoked");
}
