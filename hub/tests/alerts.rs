//! Alerting behaviour, observed through a recording notifier (nothing is sent).
//! Most tests call the alert service directly, so they're deterministic; two go
//! through the real HTTP / liveness paths. A check alerts after 2 results in
//! a row by default (flap suppression, M3-02, D-63).
#![cfg(feature = "ssr")]

mod support;

use chrono::Duration;
use common::checks::{CheckId, CheckSpec};
use hub_lib::model::AlertAfter;
use hub_lib::service::{alerts, liveness, silences};
use support::*;

/// Evaluate alerts for a report from `probe` one minute after the previous one.
async fn evaluate(hub: &TestHub, probe: &TestProbe, resources: Vec<Res>) {
    hub.clock.advance(Duration::minutes(1));
    let report = probe.signed_report("acme", hub.now(), resources);
    hub.ensure_checks(&report).await;
    let checks = hub.store().probe_checks(&probe.id).await.unwrap();
    alerts::evaluate_alerts_for_report(
        hub.store(),
        hub.notifier.as_ref(),
        &hub.state.config,
        &report,
        &checks,
        hub.now(),
    )
    .await
    .expect("evaluate alerts");
}

fn nas(up: bool) -> Vec<Res> {
    vec![ping("NAS", "10.0.0.20", up)]
}

/// Set when the NAS check alerts, as an admin would on its page.
async fn nas_alerts_after(hub: &TestHub, probe: &TestProbe, alert_after: AlertAfter) {
    let id = CheckId(
        hub.add_check(
            probe,
            "NAS",
            CheckSpec::Ping {
                host: "10.0.0.20".into(),
            },
        )
        .await,
    );
    let mut check = hub.store().check(&id).await.unwrap().unwrap();
    check.alert_after = alert_after;
    assert!(hub.store().update_check(&check).await.unwrap());
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
async fn a_resource_that_is_down_on_first_sight_alerts_on_the_second_result() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    evaluate(&hub, &probe, nas(false)).await;
    assert!(
        hub.notifier.subjects().is_empty(),
        "one result could be a blip"
    );
    evaluate(&hub, &probe, nas(false)).await;

    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN"]);
}

/// The point of M3-02: a lost ping, or a link that comes and goes, alerts
/// nobody; it took two e-mails per blip before.
#[tokio::test]
async fn single_failed_results_alert_nobody() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for up in [true, false, true, false, true, true, false, true] {
        evaluate(&hub, &probe, nas(up)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn a_short_recovery_doesnt_end_the_outage() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for up in [false, false, true, false, false, true, true] {
        evaluate(&hub, &probe, nas(up)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN", "RECOVERED"]);
}

#[tokio::test]
async fn each_check_says_how_many_results_in_a_row_alert() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    // At once, as before M3-02: alerting on a single result.
    nas_alerts_after(
        &hub,
        &probe,
        AlertAfter {
            fail_after: Some(1),
            recover_after: Some(3),
        },
    )
    .await;

    evaluate(&hub, &probe, nas(false)).await;
    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN"]);
    for _ in 0..2 {
        evaluate(&hub, &probe, nas(true)).await;
    }
    assert_eq!(
        kinds(&hub.notifier.subjects()),
        ["DOWN"],
        "two aren't three"
    );
    evaluate(&hub, &probe, nas(true)).await;
    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN", "RECOVERED"]);
}

#[tokio::test]
async fn a_silenced_resource_does_not_alert() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = CheckId(check_id(&probe.id, "NAS"));
    silences::silence_check(hub.store(), &key, "maintenance", Some(24), hub.now())
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
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(&probe, "acme", nas(false)).await;

    assert_eq!(kinds(&hub.wait_for_alerts(1).await), ["DOWN"]);
}

/// A notifier that takes its time, as a slow SMTP server does.
#[derive(Default)]
struct SlowNotifier(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl alerts::Notifier for SlowNotifier {
    async fn send(&self, email: alerts::AlertEmail) -> anyhow::Result<()> {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.0.lock().unwrap().push(email.subject);
        Ok(())
    }
}

/// Regression test (D-63): reports arriving together, as a probe delivers
/// its buffer after an outage, were evaluated at the same time, so both read
/// the same count of failed results and the outage never alerted. They are
/// evaluated one at a time, in the order they arrived.
#[tokio::test]
async fn reports_arriving_together_are_evaluated_in_order() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let notifier = std::sync::Arc::new(SlowNotifier::default());
    let mut tasks = Vec::new();
    // Down, down (alert, slowly), up, up (recovery), each as soon as the last.
    for up in [false, false, true, true] {
        hub.clock.advance(Duration::minutes(1));
        let report = probe.signed_report("acme", hub.now(), nas(up));
        hub.ensure_checks(&report).await;
        let checks = hub.store().probe_checks(&probe.id).await.unwrap();
        tasks.push(alerts::spawn_in_order(
            hub.state.storage.get(),
            notifier.clone(),
            hub.state.config.clone(),
            report,
            checks,
            hub.now(),
        ));
    }
    for task in tasks {
        task.await.unwrap();
    }

    let sent = notifier.0.lock().unwrap().clone();
    assert_eq!(kinds(&sent), ["DOWN", "RECOVERED"]);
}

#[tokio::test]
async fn an_expired_probe_alerts_once_per_outage() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let (storage, notifier) = (hub.state.storage.get(), hub.state.notifier.clone());
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

fn disk(percent: f64) -> Vec<Res> {
    vec![metric("C:", CheckSpec::Disk { path: None }, percent, "%")]
}

/// Regression test for D-33: threshold checks used to run only when the
/// resource's Up/Down status changed, and local metrics are always Up.
#[tokio::test]
async fn a_disk_over_its_threshold_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await; // default disk threshold: 90 %

    evaluate(&hub, &probe, disk(50.0)).await;
    evaluate(&hub, &probe, disk(95.0)).await;
    evaluate(&hub, &probe, disk(95.0)).await;

    assert_eq!(kinds(&hub.notifier.subjects()), ["95%"]);
}

#[tokio::test]
async fn a_threshold_alerts_once_while_exceeded() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for percent in [95.0, 96.0, 97.0, 98.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(
        kinds(&hub.notifier.subjects()),
        ["96%"],
        "the second one over"
    );
}

#[tokio::test]
async fn a_threshold_re_arms_after_dropping_below_it() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for percent in [95.0, 96.0, 50.0, 40.0, 92.0, 93.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["96%", "93%"]);
}

/// A CPU spike (or a disk briefly full) is the classic flap: one report over
/// the limit alerts nobody, by default.
#[tokio::test]
async fn a_spike_over_a_threshold_alerts_nobody() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;

    for percent in [50.0, 99.0, 50.0, 97.0, 60.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
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

async fn silenced(hub: &TestHub, key: &CheckId) -> bool {
    hub_lib::service::alerts::silence::is_check_silenced(hub.store(), key, hub.now())
        .await
        .unwrap()
}

/// Regression test for D-33: "clear the silence on recovery" was unreachable
/// for silenced resources, so a silence outlived the outage it was made for.
#[tokio::test]
async fn recovery_clears_the_silence() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = CheckId(check_id(&probe.id, "NAS"));
    evaluate(&hub, &probe, nas(true)).await;
    evaluate(&hub, &probe, nas(false)).await;
    evaluate(&hub, &probe, nas(false)).await;
    silences::silence_check(hub.store(), &key, "fixing it", None, hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(true)).await;
    assert!(silenced(&hub, &key).await, "one good result isn't recovery");
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
    let key = CheckId(check_id(&probe.id, "NAS"));
    evaluate(&hub, &probe, nas(true)).await;
    silences::silence_check(hub.store(), &key, "planned work", Some(24), hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(true)).await;
    evaluate(&hub, &probe, nas(true)).await;

    assert!(
        silenced(&hub, &key).await,
        "nothing recovered, so the silence stays"
    );
}

// ---------------------------------------------------------------------------
// Race: probe comes back while its "offline" alert is still being sent
// ---------------------------------------------------------------------------

/// A notifier whose `send` blocks until the test releases it (a slow SMTP server).
struct GatedNotifier {
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl alerts::Notifier for GatedNotifier {
    async fn send(&self, _email: alerts::AlertEmail) -> anyhow::Result<()> {
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        Ok(())
    }
}

/// Regression test: the expiry alert used to record "expired" *after* sending.
/// A probe that reported again during the send was re-armed, then overwritten
/// back to "expired", so its next outage never alerted (D-27 via a race).
#[tokio::test]
async fn a_probe_returning_during_its_offline_alert_still_alerts_next_time() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::with_alerts(&[&probe]).await;
    let key = alerts::state::probe_expiry_key("acme", &probe.id);
    let gated = std::sync::Arc::new(GatedNotifier {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });

    hub.report_now(&probe, "acme", nas(true)).await;
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    let expired = hub.sweep_expired().await;

    // The offline alert is being sent (slowly)...
    let (storage, notifier, config, now) = (
        hub.state.storage.get(),
        gated.clone(),
        hub.state.config.clone(),
        hub.now(),
    );
    let sending = tokio::spawn(async move {
        alerts::evaluate_probe_expiry(
            storage.as_ref(),
            notifier.as_ref(),
            &config,
            &expired,
            PROBE_TIMEOUT_SECS,
            now,
        )
        .await
    });
    gated.entered.notified().await;

    // ...when the probe comes back, and only then does the send finish.
    hub.report_now(&probe, "acme", nas(true)).await;
    gated.release.add_permits(1);
    sending.await.unwrap().unwrap();

    assert!(
        alerts::state::should_send_alert(hub.store(), &key, alerts::state::PROBE_EXPIRED)
            .await
            .unwrap(),
        "the probe is back, so its next outage must alert"
    );
}
