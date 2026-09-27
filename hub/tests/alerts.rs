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
// Known issues (D-33): intended behaviour, ignored until fixed.
// ---------------------------------------------------------------------------

fn disk(percent: f64) -> Vec<ResourceStatus> {
    vec![metric("C:", ResourceType::LocalDisk, percent, "%")]
}

/// D-33: threshold checks run only when a resource's Up/Down status changes.
/// Local metrics are always Up, so a disk sitting above the threshold never alerts.
#[tokio::test]
#[ignore = "known issue D-33: metric thresholds never alert"]
async fn known_issue_a_disk_over_its_threshold_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await; // default disk threshold: 90 %

    evaluate(&hub, &probe, disk(50.0)).await;
    evaluate(&hub, &probe, disk(95.0)).await;

    assert_eq!(kinds(&hub.notifier.subjects()), ["95%"]);
}

/// D-33: "clear the silence when the resource recovers" is unreachable for a
/// silenced resource (silenced resources skip evaluation entirely), so the
/// silence outlives the outage it was created for.
#[tokio::test]
#[ignore = "known issue D-33: silence is not cleared on recovery"]
async fn known_issue_recovery_clears_the_silence() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = hub_lib::model::resource_key("acme", &probe.id, "NAS", "10.0.0.20");
    evaluate(&hub, &probe, nas(true)).await;
    evaluate(&hub, &probe, nas(false)).await;
    silences::silence_resource(hub.store(), &key, "fixing it", None, hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(true)).await;

    let still_silenced = hub
        .store()
        .is_silenced(silences::RESOURCE_SCOPE, &key, hub.now())
        .await
        .unwrap();
    assert!(!still_silenced, "recovery should clear the silence");
}
