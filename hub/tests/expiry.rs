//! Probe liveness: heartbeats, expiry after `probe_timeout_secs`, and recovery.
#![cfg(feature = "ssr")]

mod support;

use chrono::Duration;
use hub_lib::service::alerts::state;
use serde_json::Value;
use support::*;

const TIMEOUT: i64 = PROBE_TIMEOUT_SECS as i64;

#[tokio::test]
async fn probe_is_not_expired_before_the_timeout() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;

    hub.clock.advance(Duration::seconds(TIMEOUT - 1));

    assert!(hub.sweep_expired().await.is_empty());
}

#[tokio::test]
async fn probe_expires_after_the_timeout_and_is_reported_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;

    hub.clock.advance(Duration::seconds(TIMEOUT + 1));

    assert_eq!(
        hub.sweep_expired().await,
        vec![(probe.id.clone(), "acme".to_string())]
    );
    assert!(
        hub.sweep_expired().await.is_empty(),
        "an already-expired probe is not reported again"
    );

    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(companies[0]["active_probes"], 0);
    assert_eq!(companies[0]["expired_probes"], 1);
    let probes: Vec<Value> = hub.get_json("/api/company/acme/probes").await;
    assert_eq!(probes[0]["status"], "expired");
}

#[tokio::test]
async fn expired_probe_keeps_showing_its_last_known_devices() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(
        &probe,
        "acme",
        vec![ping("a", "1", true), ping("b", "2", false)],
    )
    .await;
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));
    hub.sweep_expired().await;

    let companies: Vec<Value> = hub.get_json("/api/dashboard/companies").await;

    // Stale counts are still reported; the UI decides how to present them (D-16).
    assert_eq!(companies[0]["devices_up"], 1);
    assert_eq!(companies[0]["devices_down"], 1);
}

/// Regression test: the sweep writes while another connection may be
/// writing, such as the alert evaluation ingest leaves running in the
/// background (D-63). It must wait its turn (the busy timeout), not fail with
/// "database is locked": its transaction read first, and SQLite refuses at
/// once a reader that then wants to write while another connection writes.
#[tokio::test]
async fn the_expiry_sweep_waits_for_another_writer() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));

    let mut writer = hub.pool().acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let (expired, ()) = tokio::join!(hub.sweep_expired(), async {
        // Give the sweep time to reach the lock. Shorter can't make this test
        // fail, only miss the bug; the sweep waits up to the busy timeout.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        sqlx::query("COMMIT").execute(&mut *writer).await.unwrap();
    });

    assert_eq!(expired, vec![(probe.id.clone(), "acme".to_string())]);
}

#[tokio::test]
async fn a_new_report_reactivates_an_expired_probe() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));
    hub.sweep_expired().await;

    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;

    let probes: Vec<Value> = hub.get_json("/api/company/acme/probes").await;
    assert_eq!(probes[0]["status"], "active");
}

/// Regression test for D-27 (fixed in M0-18): the "probe offline" alert state
/// used to stay "expired" forever after the first e-mail, so a probe that went
/// offline a *second* time never alerted again.
#[tokio::test]
async fn probe_that_expires_twice_alerts_twice() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let key = state::probe_expiry_key("acme", &probe.id);
    let alert_due = || state::should_send_alert(hub.store(), &key, state::PROBE_EXPIRED);

    // First outage: the engine sends the e-mail and records it.
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));
    hub.sweep_expired().await;
    assert!(alert_due().await.unwrap(), "first outage alerts");
    state::update_alert_state(hub.store(), &key, state::PROBE_EXPIRED, true, hub.now())
        .await
        .unwrap();
    assert!(
        !alert_due().await.unwrap(),
        "the same outage alerts only once"
    );

    // Probe recovers, then goes offline again.
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));
    assert_eq!(hub.sweep_expired().await.len(), 1);

    assert!(alert_due().await.unwrap(), "second outage must alert again");
}

#[tokio::test]
async fn reporting_does_not_create_alert_state_for_probes_that_never_alerted() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_states")
        .fetch_one(hub.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// Start an expiry evaluation of `probes`, as each sweep does, in the background.
fn evaluate_in_background(hub: &TestHub, probes: &[&TestProbe]) -> tokio::task::JoinHandle<()> {
    let (storage, notifier, now) = (
        hub.state.storage.get(),
        hub.state.notifier.clone(),
        hub.now(),
    );
    let probes: Vec<(String, String)> = probes
        .iter()
        .map(|p| (p.id.clone(), "acme".to_string()))
        .collect();
    tokio::spawn(async move {
        hub_lib::service::alerts::evaluate_probe_expiry(
            storage.as_ref(),
            notifier.as_ref(),
            &probes,
            PROBE_TIMEOUT_SECS,
            now,
        )
        .await
        .unwrap()
    })
}

/// Two sweeps' evaluations of the same expired probe, overlapping, alert once
/// (CI run 40082566575 sent EXPIRED twice). The write lock forces the overlap
/// a slow machine allows: reads go on (WAL), writes wait, so the second
/// evaluation reaches the probe while the first holds "not alerted" in hand.
#[tokio::test]
async fn overlapping_expiry_evaluations_of_a_probe_alert_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("a", "1", true)])
        .await;
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));
    assert_eq!(hub.sweep_expired().await.len(), 1);

    let mut writer = hub.pool().acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let first = evaluate_in_background(&hub, &[&probe]);
    // Long enough for each to make its reads and reach a write; shorter can
    // only miss the bug, not fail the fix. Writes wait up to the busy timeout.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let second = evaluate_in_background(&hub, &[&probe]);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    sqlx::query("COMMIT").execute(&mut *writer).await.unwrap();
    first.await.unwrap();
    second.await.unwrap();

    let sent = hub.notifier.subjects();
    assert_eq!(sent.len(), 1, "one alert for one outage: {sent:?}");
    assert!(sent[0].ends_with("EXPIRED"), "{sent:?}");
}

/// Turns are per probe: while one probe's evaluation waits (its turn held, as
/// by a slow evaluation), another probe's goes ahead and alerts.
#[tokio::test]
async fn another_probes_expiry_does_not_wait_for_a_busy_one() {
    use hub_lib::service::alerts::engine::Turn;
    let (busy, other) = (TestProbe::new("hq"), TestProbe::new("branch"));
    let hub = TestHub::new(&[&busy, &other]).await;
    for probe in [&busy, &other] {
        hub.report_now(probe, "acme", vec![ping("a", "1", true)])
            .await;
    }
    hub.clock.advance(Duration::seconds(TIMEOUT + 1));
    assert_eq!(hub.sweep_expired().await.len(), 2);

    let mut held = Turn::take(&busy.id);
    held.wait().await; // after its report's evaluation
    let busy_one = evaluate_in_background(&hub, &[&busy]);
    let other_one = evaluate_in_background(&hub, &[&other]);
    other_one.await.unwrap();
    let sent = hub.wait_for_alerts(1).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("PROBE branch EXPIRED"), "{sent:?}");

    drop(held);
    busy_one.await.unwrap();
    let sent = hub.notifier.subjects();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[1].contains("PROBE hq EXPIRED"), "{sent:?}");
}
