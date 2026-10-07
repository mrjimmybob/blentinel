//! Alerting behaviour, observed through a recording notifier: which alerts
//! the engine raises (where they go: `tests/channels.rs`).
//! Most tests call the alert service directly, so they're deterministic; two go
//! through the real HTTP / liveness paths. A check alerts after 2 results in
//! a row by default (flap suppression, M3-02, D-63).
#![cfg(feature = "ssr")]

mod support;

use chrono::Duration;
use common::checks::{CheckId, CheckSpec};
use hub_lib::model::AlertAfter;
use hub_lib::service::{alerts, liveness, silences, thresholds};
use support::*;

/// Evaluate alerts for a report from `probe` one minute after the previous one.
async fn evaluate(hub: &TestHub, probe: &TestProbe, resources: Vec<Res>) {
    hub.clock.advance(Duration::minutes(1));
    let report = probe.signed_report("acme", hub.now(), resources);
    hub.ensure_checks(&report).await;
    let checks = hub.store().probe_checks(&probe.id).await.unwrap();
    // Judged as ingest judges it, with the thresholds that apply (D-64).
    let evaluations = thresholds::evaluate_report(hub.store(), &report, &checks)
        .await
        .unwrap();
    alerts::evaluate_alerts_for_report(
        hub.store(),
        hub.notifier.as_ref(),
        &report,
        &evaluations,
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
    let hub = TestHub::new(&[&probe]).await;

    for _ in 0..3 {
        evaluate(&hub, &probe, nas(true)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn going_down_alerts_once_and_recovering_alerts_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    for up in [true, false, false, false, true, true] {
        evaluate(&hub, &probe, nas(up)).await;
    }

    let sent = hub.notifier.sent();
    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN", "RECOVERED"]);
    assert!(
        sent[0].subject().contains("[acme]"),
        "{}",
        sent[0].subject()
    );
    let check = sent[0].check.as_ref().expect("a check's alert");
    assert_eq!((check.name.as_str(), check.kind.as_str()), ("NAS", "ping"));
    assert_eq!(sent[0].site_id, "hq-site");
}

#[tokio::test]
async fn a_resource_that_is_down_on_first_sight_alerts_on_the_second_result() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

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
    let hub = TestHub::new(&[&probe]).await;

    for up in [true, false, true, false, true, true, false, true] {
        evaluate(&hub, &probe, nas(up)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn a_short_recovery_doesnt_end_the_outage() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    for up in [false, false, true, false, false, true, true] {
        evaluate(&hub, &probe, nas(up)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["DOWN", "RECOVERED"]);
}

#[tokio::test]
async fn each_check_says_how_many_results_in_a_row_alert() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
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
    let hub = TestHub::new(&[&probe]).await;
    // A silence is of an existing check.
    let key = CheckId(
        hub.add_check(
            &probe,
            "NAS",
            CheckSpec::Ping {
                host: "10.0.0.20".into(),
            },
        )
        .await,
    );
    silences::silence_check(hub.store(), &key, "maintenance", Some(24), hub.now())
        .await
        .unwrap();

    evaluate(&hub, &probe, nas(false)).await;
    evaluate(&hub, &probe, nas(false)).await;
    evaluate(&hub, &probe, nas(true)).await;

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn a_report_received_over_http_triggers_its_alerts() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

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
    async fn notify(&self, n: hub_lib::service::notify::Notification) -> anyhow::Result<()> {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.0.lock().unwrap().push(n.subject());
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
    let hub = TestHub::new(&[&probe]).await;
    let notifier = std::sync::Arc::new(SlowNotifier::default());
    let mut tasks = Vec::new();
    // Down, down (alert, slowly), up, up (recovery), each as soon as the last.
    for up in [false, false, true, true] {
        hub.clock.advance(Duration::minutes(1));
        let report = probe.signed_report("acme", hub.now(), nas(up));
        hub.ensure_checks(&report).await;
        let checks = hub.store().probe_checks(&probe.id).await.unwrap();
        let evaluations = thresholds::evaluate_report(hub.store(), &report, &checks)
            .await
            .unwrap();
        tasks.push(alerts::spawn_in_order(
            hub.state.storage.get(),
            notifier.clone(),
            report,
            evaluations,
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
    let hub = TestHub::new(&[&probe]).await;
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
// Thresholds: Warning and Down (D-33, D-64)
// ---------------------------------------------------------------------------

fn disk(percent: f64) -> Vec<Res> {
    vec![metric("C:", CheckSpec::Disk { path: None }, percent, "%")]
}

/// Regression test for D-33: thresholds used to be checked only when the
/// check's Up/Down status changed, and local metrics are always Up.
#[tokio::test]
async fn a_disk_over_its_warning_limit_warns() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await; // built in: warning 90 %, down 97 %

    for percent in [50.0, 95.0, 95.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["WARNING"]);
    let detail = &hub.notifier.sent()[0].detail;
    assert!(
        detail
            .contains("disk usage 95% is over the warning limit (90%, from the built-in default)"),
        "{detail}"
    );
}

#[tokio::test]
async fn a_warning_alerts_once_while_it_lasts_and_recovers() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    for percent in [95.0, 96.0, 95.0, 96.0, 50.0, 40.0, 92.0, 93.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(
        kinds(&hub.notifier.subjects()),
        ["WARNING", "RECOVERED", "WARNING"]
    );
}

#[tokio::test]
async fn a_full_disk_goes_down_then_back_to_a_warning_then_up() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    for percent in [98.0, 99.0, 92.0, 92.0, 50.0, 50.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(
        kinds(&hub.notifier.subjects()),
        ["DOWN", "WARNING", "RECOVERED"]
    );
    assert!(
        hub.notifier.sent()[0]
            .detail
            .contains("over the down limit (97%")
    );
}

/// A CPU spike (or a disk briefly full) is the classic flap: one report over
/// the limit alerts nobody, by default.
#[tokio::test]
async fn a_spike_over_a_threshold_alerts_nobody() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    for percent in [50.0, 99.0, 50.0, 97.0, 60.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
}

#[tokio::test]
async fn a_metric_below_its_threshold_never_alerts_or_leaves_state() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    for percent in [10.0, 89.9, 50.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert!(hub.notifier.subjects().is_empty());
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_states")
        .fetch_one(hub.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "no state for a check that was never anything but Up"
    );
}

#[tokio::test]
async fn a_companys_limits_apply_before_the_hubs() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let mut strict = hub_lib::model::Thresholds::default();
    strict.disk.warning = Some(hub_lib::model::Setting::At(50.0));
    assert!(
        hub.store()
            .set_company_thresholds("acme", &strict)
            .await
            .unwrap()
    );

    for percent in [60.0, 60.0] {
        evaluate(&hub, &probe, disk(percent)).await;
    }

    assert_eq!(kinds(&hub.notifier.subjects()), ["WARNING"]);
    assert!(
        hub.notifier.sent()[0]
            .detail
            .contains("(50%, from the company)")
    );
}

// ---------------------------------------------------------------------------
// Silences and recovery (D-33, fixed in M0-21)
// ---------------------------------------------------------------------------

async fn silenced(hub: &TestHub, key: &CheckId) -> bool {
    hub.store()
        .active_silences(hub.now())
        .await
        .unwrap()
        .iter()
        .any(|s| s.is_of_check(key.as_str()))
}

/// Regression test for D-33: "clear the silence on recovery" was unreachable
/// for silenced resources, so a silence outlived the outage it was made for.
#[tokio::test]
async fn recovery_clears_the_silence() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
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
    let hub = TestHub::new(&[&probe]).await;
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

/// A notifier whose `notify` blocks until the test releases it (a slow SMTP server).
struct GatedNotifier {
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl alerts::Notifier for GatedNotifier {
    async fn notify(&self, _n: hub_lib::service::notify::Notification) -> anyhow::Result<()> {
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
    let hub = TestHub::new(&[&probe]).await;
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
    let (storage, notifier, now) = (hub.state.storage.get(), gated.clone(), hub.now());
    let sending = tokio::spawn(async move {
        alerts::evaluate_probe_expiry(
            storage.as_ref(),
            notifier.as_ref(),
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
