//! The four statuses and the threshold hierarchy (M3-01, D-16, D-64): each
//! result judged on the hub when it arrives, against the check's, site's,
//! company's or hub's limits, and shown the same everywhere.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::CheckSpec;
use hub_lib::model::{
    CheckInfo, DashboardCompany, ProbeDevice, Role, Severity, Status, ThresholdLevel,
    ThresholdMetric, ThresholdsView,
};
use serde_json::{Value, json};
use support::*;

fn disk(percent: f64) -> Vec<Res> {
    vec![metric("Data", CheckSpec::Disk { path: None }, percent, "%")]
}

async fn devices(hub: &TestHub, probe: &TestProbe) -> Vec<ProbeDevice> {
    hub.get_json(&format!("/api/probe/{}/devices", probe.id))
        .await
}

/// The limits in force for `metric` in a view's inherited list.
fn inherited(
    view: &ThresholdsView,
    metric: ThresholdMetric,
) -> (Option<f64>, Option<f64>, ThresholdLevel) {
    let (_, l) = view.inherited.iter().find(|(m, _)| *m == metric).unwrap();
    (l.warning.value, l.down.value, l.warning.from)
}

#[tokio::test]
async fn a_report_is_judged_against_its_thresholds_and_shown_so() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    hub.report_now(&probe, "acme", disk(93.0)).await;

    let device = &devices(&hub, &probe).await[0];
    assert_eq!(device.status, Status::Warning);
    assert_eq!(
        device.reason.as_deref(),
        Some("disk usage 93% is over the warning limit (90%, from the built-in default)")
    );
    let dashboard: Vec<DashboardCompany> = hub.get_json("/api/dashboard/companies").await;
    assert_eq!(
        (dashboard[0].devices_warning, dashboard[0].severity),
        (1, Severity::Warning)
    );

    hub.clock.advance(Duration::minutes(1));
    hub.report_now(&probe, "acme", disk(98.0)).await;
    assert_eq!(devices(&hub, &probe).await[0].status, Status::Down);

    // The probe stops reporting: nobody knows any more.
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    hub.sweep_expired().await;
    assert_eq!(devices(&hub, &probe).await[0].status, Status::Unknown);
}

#[tokio::test]
async fn each_level_overrides_the_ones_above_it() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let site = format!("{}-site", probe.name);
    let post = |path: String, body: Value| {
        let owner = owner.clone();
        let hub = &hub;
        async move { hub.post_json(&path, Some(&owner), body).await }
    };

    // The hub's default, then the company's, then the site's.
    let saved = post(
        "/api/admin/thresholds".into(),
        json!({ "disk": { "warning": 80 } }),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    hub.report_now(&probe, "acme", disk(85.0)).await;
    assert_eq!(devices(&hub, &probe).await[0].status, Status::Warning);

    post(
        "/api/companies/acme/thresholds".into(),
        json!({ "disk": { "warning": 88 } }),
    )
    .await;
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(&probe, "acme", disk(85.0)).await;
    assert_eq!(devices(&hub, &probe).await[0].status, Status::Up);

    post(
        format!("/api/companies/acme/sites/{site}/thresholds"),
        json!({ "disk": { "warning": 60, "down": "off" } }),
    )
    .await;
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(&probe, "acme", disk(99.0)).await;
    let device = &devices(&hub, &probe).await[0];
    assert_eq!(
        device.status,
        Status::Warning,
        "down switched off at the site"
    );
    assert!(device.reason.as_deref().unwrap().contains("from the site"));

    // The site's view shows what it inherits: the company's warning, the hub's down.
    let view: ThresholdsView = hub
        .get_json(&format!("/api/companies/acme/sites/{site}/thresholds"))
        .await;
    assert_eq!(
        inherited(&view, ThresholdMetric::Disk),
        (Some(88.0), Some(97.0), ThresholdLevel::Company)
    );

    // The check's own limits come first, and its page says which apply.
    let check_id = check_id(&probe.id, "Data");
    let body = json!({
        "name": "Data", "spec": { "kind": "disk" },
        "limits": { "warning": 99.5 },
    });
    let changed = post(format!("/api/checks/{check_id}"), body).await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text());
    let info: CheckInfo = changed.json();
    let in_force = info.limits_in_force.unwrap();
    assert_eq!(
        (in_force.warning.value, in_force.warning.from),
        (Some(99.5), ThresholdLevel::Check)
    );
    assert_eq!(
        (in_force.down.value, in_force.down.from),
        (None, ThresholdLevel::Site)
    );
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(&probe, "acme", disk(99.0)).await;
    assert_eq!(devices(&hub, &probe).await[0].status, Status::Up);

    let audited: Vec<Value> = hub.get_json("/api/audit?action=thresholds").await;
    assert_eq!(audited.len(), 3, "hub, company, site");
}

#[tokio::test]
async fn latency_has_no_limits_until_someone_sets_them() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let mut slow = ping("Router", "10.0.0.1", true);
    slow.latency_ms = Some(900);

    hub.report_now(&probe, "acme", vec![slow.clone()]).await;
    assert_eq!(devices(&hub, &probe).await[0].status, Status::Up);

    hub.post_json(
        "/api/admin/thresholds",
        Some(&owner),
        json!({ "latency": { "warning": 300, "down": 2000 } }),
    )
    .await;
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(&probe, "acme", vec![slow]).await;
    let device = &devices(&hub, &probe).await[0];
    assert_eq!(device.status, Status::Warning);
    assert!(
        device
            .reason
            .as_deref()
            .unwrap()
            .starts_with("latency 900 ms")
    );
}

/// A `tls` check's result: the certificate has `days_left` days.
fn certificate(days_left: f64) -> Res {
    let spec = CheckSpec::Tls {
        host: "example.com".into(),
        port: 443,
        verify: true,
    };
    let mut result = metric("Website certificate", spec, days_left, "d");
    result.metrics[0].key = common::checks::metric::DAYS_LEFT.into();
    result
}

// M5-01, D-77: the one metric where less is worse.
#[tokio::test]
async fn a_certificate_warns_and_goes_down_as_its_days_run_out() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let status_at = |days_left: f64| {
        let (hub, probe) = (&hub, &probe);
        async move {
            hub.clock.advance(Duration::minutes(1));
            hub.report_now(probe, "acme", vec![certificate(days_left)])
                .await;
            devices(hub, probe).await.remove(0)
        }
    };

    // The built-in limits: a warning with 21 days left, down with 7.
    assert_eq!(status_at(60.0).await.status, Status::Up);
    let warning = status_at(20.5).await;
    assert_eq!(warning.status, Status::Warning);
    assert_eq!(
        warning.reason.as_deref(),
        Some(
            "certificate expiry in 20.5 days is under the warning limit \
             (21 days, from the built-in default)"
        )
    );
    assert_eq!(status_at(6.0).await.status, Status::Down);

    // The hub's own, and what a view inherits.
    let saved = hub
        .post_json(
            "/api/admin/thresholds",
            Some(&owner),
            json!({ "tls": { "warning": 45, "down": "off" } }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert_eq!(status_at(40.0).await.status, Status::Warning);
    assert_eq!(status_at(1.0).await.status, Status::Warning, "down is off");
    let view: ThresholdsView = hub.get_json("/api/companies/acme/thresholds").await;
    assert_eq!(
        inherited(&view, ThresholdMetric::Tls),
        (Some(45.0), None, ThresholdLevel::Global)
    );

    // The warning limit is the higher one here.
    let refused = hub
        .post_json(
            "/api/admin/thresholds",
            Some(&owner),
            json!({ "tls": { "warning": 7, "down": 30 } }),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.text().contains("above the down limit"));
}

// M5-04, D-80: a ping is judged by its latency and, beside it, by its loss.
#[tokio::test]
async fn a_ping_that_loses_echoes_is_a_warning() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let status_at = |loss: f64| {
        let (hub, probe) = (&hub, &probe);
        async move {
            let mut result = ping("Router", "10.0.0.1", true);
            result.metrics = vec![common::checks::Metric::new(
                common::checks::metric::LOSS,
                loss,
                "%",
            )];
            hub.clock.advance(Duration::minutes(1));
            hub.report_now(probe, "acme", vec![result]).await;
            devices(hub, probe).await.remove(0)
        }
    };

    assert_eq!(status_at(25.0).await.status, Status::Up, "one echo of four");
    let lossy = status_at(50.0).await;
    assert_eq!(lossy.status, Status::Warning);
    assert_eq!(
        lossy.reason.as_deref(),
        Some("packet loss 50% is over the warning limit (50%, from the built-in default)")
    );

    // The hub's own limits; the check's own stay about latency.
    let saved = hub
        .post_json(
            "/api/admin/thresholds",
            Some(&owner),
            json!({ "loss": { "warning": 10, "down": 60 } }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert_eq!(status_at(25.0).await.status, Status::Warning);
    assert_eq!(status_at(75.0).await.status, Status::Down);
    let view: ThresholdsView = hub.get_json("/api/companies/acme/thresholds").await;
    assert_eq!(
        inherited(&view, ThresholdMetric::Loss),
        (Some(10.0), Some(60.0), ThresholdLevel::Global)
    );
}

// M5-03, D-79: a backup that is late, then missed.
#[tokio::test]
async fn a_file_warns_and_goes_down_as_it_gets_old() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let status_at = |hours: f64| {
        let (hub, probe) = (&hub, &probe);
        async move {
            let spec = CheckSpec::File {
                path: "/var/backups".into(),
                pattern: "*.bak".into(),
                recursive: false,
                min_bytes: None,
            };
            let mut result = metric("Nightly backup", spec, hours, "h");
            result.metrics[0].key = common::checks::metric::AGE.into();
            hub.clock.advance(Duration::minutes(1));
            hub.report_now(probe, "acme", vec![result]).await;
            devices(hub, probe).await.remove(0)
        }
    };

    assert_eq!(status_at(9.0).await.status, Status::Up);
    let late = status_at(27.5).await;
    assert_eq!(late.status, Status::Warning);
    assert_eq!(
        late.reason.as_deref(),
        Some("file age 27.5 h is over the warning limit (26 h, from the built-in default)")
    );
    assert_eq!(status_at(51.0).await.status, Status::Down);

    // A weekly backup: the company says so.
    let saved = hub
        .post_json(
            "/api/companies/acme/thresholds",
            Some(&owner),
            json!({ "file_age": { "warning": 170, "down": 340 } }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert_eq!(status_at(51.0).await.status, Status::Up);
    assert_eq!(status_at(200.0).await.status, Status::Warning);
}

#[tokio::test]
async fn bad_thresholds_are_refused_with_the_reason() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    for (body, reason) in [
        (
            json!({ "disk": { "warning": 95, "down": 90 } }),
            "below the down limit",
        ),
        (json!({ "cpu": { "warning": 150 } }), "0 to 100"),
        (json!({ "latency": { "down": -5 } }), "0 to 600000"),
    ] {
        let refused = hub
            .post_json("/api/admin/thresholds", Some(&owner), body.clone())
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(refused.error_code(), "invalid_thresholds");
        assert!(refused.text().contains(reason), "{}", refused.text());
    }
    let word = hub
        .post_json(
            "/api/admin/thresholds",
            Some(&owner),
            json!({ "disk": { "warning": "high" } }),
        )
        .await;
    assert_eq!(word.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A check's limits are for its own metric: none for uptime.
    let uptime = json!({ "name": "Up", "spec": { "kind": "uptime" }, "limits": { "warning": 5 } });
    let refused = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(&owner),
            uptime,
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.text().contains("no thresholds"));

    let viewer = hub.login_as(Role::Viewer).await;
    let forbidden = hub
        .post_json("/api/admin/thresholds", Some(&viewer), json!({}))
        .await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_hubs_default_starts_as_the_built_in_one() {
    let hub = TestHub::new(&[]).await;
    let view: ThresholdsView = hub.get_json("/api/admin/thresholds").await;
    assert_eq!(view.own, Default::default(), "nothing set");
    assert_eq!(
        inherited(&view, ThresholdMetric::Disk),
        (Some(90.0), Some(97.0), ThresholdLevel::BuiltIn)
    );
    assert_eq!(
        inherited(&view, ThresholdMetric::Latency),
        (None, None, ThresholdLevel::BuiltIn)
    );
}
