//! Alerting behaviour, observed through a recording notifier (nothing is sent).
//! Most tests call the alert service directly, so they're deterministic; two go
//! through the real HTTP / liveness paths.
#![cfg(feature = "ssr")]

mod support;

use chrono::Duration;
use common::models::{ResourceStatus, ResourceType};
use hub_lib::service::{alerts, liveness, silences};
use support::*;

/// Evaluate alerts for a report from `probe` one minute after the previous one.
async fn evaluate(hub: &TestHub, probe: &TestProbe, resources: Vec<ResourceStatus>) {
    hub.clock.advance(Duration::minutes(1));
    let report = probe.signed_report("acme", hub.now(), resources);
    alerts::evaluate_alerts_for_report(
        hub.store(),
        hub.notifier.as_ref(),
        &hub.state.config,
        &report,
        hub.now(),
    )
    .await
    .expect("evaluate alerts");
}

fn nas(up: bool) -> Vec<ResourceStatus> {
    vec![ping("NAS", "10.0.0.20", up)]
}

fn kinds(subjects: &[String]) -> Vec<&str> {
    subjects
        .iter()
        .map(|s| s.rsplit(' ').next().unwrap_or(""))
        .collect()
}

#[tokio::test]
async fn a_healthy_resource_never_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for _ in 0..3 {
        evaluate(&hub, &probe, nas(true)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn going_down_alerts_once_and_recovering_alerts_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for up in [true, false, false, false, true, true] {
        evaluate(&hub, &probe, nas(up)).await;
    }

    let sent = hub.notifier.sent();
    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN", "RECOVERED"]);
    assert!(sent[0].subject.contains("[acme]"), "{}", sent[0].subject);
    assert!(sent.iter().all(|e| e.to == [ALERT_RECIPIENT]));
}

#[tokio::test]
async fn a_resource_that_is_down_on_first_sight_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    evaluate(&hub, &probe, nas(false)).await;

    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN"]);
}

#[tokio::test]
async fn a_silenced_resource_does_not_alert() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = hub_lib::model::resource_key("acme", &probe.id, "NAS", "10.0.0.20");
    silences::silence_resource(hub.store(), &key, "maintenance", Some(24), hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(false)).await;
    evaluate(&hub, &probe, nas(true)).await;

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn nothing_is_sent_when_alerting_is_disabled() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await; // alerts disabled by default

    evaluate(&hub, &probe, nas(false)).await;

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn a_report_received_over_http_triggers_its_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    hub.report_now(&probe, "acme", nas(false)).await;

    assert_eq!(kinds(&hub.wait_for_alerts(1).await), ["DOWN"]);
}

#[tokio::test]
async fn an_expired_probe_alerts_once_per_outage() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let (storage, notifier) = (hub.state.storage.clone(), hub.state.notifier.clone());
    let sweep = |now| {
        let (storage, notifier, config) =
            (storage.clone(), notifier.clone(), hub.state.config.clone());
        async move {
            liveness::sweep_expired_probes(&storage, &notifier, &config, now)
                .await
                .unwrap()
        }
    };

    // First outage.
    hub.report_now(&probe, "acme", nas(true)).await;
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    assert_eq!(sweep(hub.now()).await.len(), 1);
    assert_eq!(kinds(&hub.wait_for_alerts(1).await), ["EXPIRED"]);

    // Back online, then a second outage alerts again (D-27).
    hub.report_now(&probe, "acme", nas(true)).await;
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    assert_eq!(sweep(hub.now()).await.len(), 1);
    assert_eq!(kinds(&hub.wait_for_alerts(2).await), ["EXPIRED", "EXPIRED"]);
}

// ---------------------------------------------------------------------------
// Metric thresholds (D-33, fixed in M0-21)
// ---------------------------------------------------------------------------

fn disk(percent: f64) -> Vec<ResourceStatus> {
    vec![metric("C:", ResourceType::LocalDisk, percent, "%")]
}

/// Regression test for D-33: threshold checks used to run only when the
/// resource's Up/Down status changed, and local metrics are always Up.
#[tokio::test]
async fn a_disk_over_its_threshold_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await; // default disk threshold: 90 %

    evaluate(&hub, &probe, disk(50.0)).await;
    evaluate(&hub, &probe, disk(95.0)).await;

    assert_eq!(kinds(&hub.notifier.subjects()), ["95%"]);
}

#[tokio::test]
async fn a_threshold_alerts_once_while_exceeded() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for percent in [95.0, 96.0, 97.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["95%"]);
}

#[tokio::test]
async fn a_threshold_re_arms_after_dropping_below_it() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for percent in [95.0, 50.0, 92.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["95%", "92%"]);
}

#[tokio::test]
async fn a_metric_below_its_threshold_never_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for percent in [10.0, 89.9, 50.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
    let threshold_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM alert_states WHERE resource_key LIKE '%-threshold'",
    )
    .fetch_one(hub.pool())
    .await
    .unwrap();
    assert_eq!(
        threshold_rows, 0,
        "no state is written for metrics that never exceed"
    );
}

// ---------------------------------------------------------------------------
// Silences and recovery (D-33, fixed in M0-21)
// ---------------------------------------------------------------------------

async fn silenced(hub: &TestHub, key: &str) -> bool {
    hub.store()
        .is_silenced(silences::RESOURCE_SCOPE, key, hub.now())
        .await
        .unwrap()
}

/// Regression test for D-33: "clear the silence on recovery" was unreachable
/// for silenced resources, so a silence outlived the outage it was made for.
#[tokio::test]
async fn recovery_clears_the_silence() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = hub_lib::model::resource_key("acme", &probe.id, "NAS", "10.0.0.20");
    evaluate(&hub, &probe, nas(true)).await;
    evaluate(&hub, &probe, nas(false)).await;
    silences::silence_resource(hub.store(), &key, "fixing it", None, hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(true)).await;

    assert!(
        !silenced(&hub, &key).await,
        "recovery should clear the silence"
    );
    assert_eq!(
        kinds(&hub.notifier.subjects()),
        ["DOWN"],
        "no recovery alert for an outage that was silenced"
    );
}

#[tokio::test]
async fn a_silence_on_a_healthy_resource_is_kept() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = hub_lib::model::resource_key("acme", &probe.id, "NAS", "10.0.0.20");
    evaluate(&hub, &probe, nas(true)).await;
    silences::silence_resource(hub.store(), &key, "planned work", Some(24), hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(true)).await;
    evaluate(&hub, &probe, nas(true)).await;

    assert!(
        silenced(&hub, &key).await,
        "nothing recovered, so the silence stays"
    );
}
