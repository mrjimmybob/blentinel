//! A check's history (M6-02): `GET /api/checks/{id}/history` reads the
//! rollups for its period, its incidents, and its newest raw results.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use hub_lib::model::{CheckHistory, Role, Status, StatusCounts};
use support::*;

/// The hub's clock is 2026-03-10T12:00:00Z. Reports are seeded (not
/// ingested), oldest first, so history can go back weeks.
async fn hub_with_history() -> (TestHub, TestProbe) {
    let probe = TestProbe::new("hq");
    let other = TestProbe::new("other");
    let hub = TestHub::new(&[&probe, &other]).await;
    let send = |timestamp: &str, resources| {
        let report = probe.signed_report("acme", at(timestamp), resources);
        let hub = &hub;
        async move { hub.seed_report(&report).await }
    };
    send("2026-02-18T09:00:00Z", vec![ping("a", "1", false)]).await; // 20 days ago
    send(
        "2026-03-10T10:03:00Z",
        vec![ping("a", "1", true), ping("b", "2", false)],
    )
    .await;
    send("2026-03-10T10:07:00Z", vec![ping("a", "1", true)]).await;
    send("2026-03-10T10:20:00Z", vec![ping("a", "1", false)]).await;
    let noise = other.signed_report(
        "zeta",
        at("2026-03-10T10:05:00Z"),
        vec![ping("z", "9", false)],
    );
    hub.seed_report(&noise).await;
    (hub, probe)
}

fn counts(up: i64, down: i64) -> StatusCounts {
    StatusCounts {
        up,
        down,
        ..StatusCounts::default()
    }
}

#[tokio::test]
async fn a_day_of_a_check_comes_from_its_five_minute_rollups() {
    let (hub, probe) = hub_with_history().await;
    let a = check_id(&probe.id, "a");
    let history: CheckHistory = hub
        .get_json(&format!("/api/checks/{a}/history?range=24h"))
        .await;

    assert_eq!(history.check_id, a);
    assert_eq!(history.name, "a");
    assert_eq!(history.kind, "ping");
    assert_eq!(history.target, "1");
    assert_eq!(history.company_id, "acme");
    assert_eq!(history.probe_id, probe.id);
    assert_eq!(
        (history.from.as_str(), history.to.as_str()),
        ("2026-03-09 12:00", "2026-03-10 12:00")
    );
    assert_eq!(history.bucket_secs, 300);
    let buckets: Vec<(&str, StatusCounts)> = history
        .buckets
        .iter()
        .map(|b| (b.start.as_str(), b.counts))
        .collect();
    assert_eq!(
        buckets,
        [
            ("2026-03-10 10:00", counts(1, 0)),
            ("2026-03-10 10:05", counts(1, 0)),
            ("2026-03-10 10:20", counts(0, 1)),
        ],
        "only its own results, and not from 20 days ago"
    );
    assert_eq!(history.totals, counts(2, 1));
    let availability = history.availability_pct.unwrap();
    assert!((availability - 200.0 / 3.0).abs() < 1e-9, "{availability}");

    // A ping measures its response time: a point where it answered.
    assert_eq!(history.series.len(), 1);
    assert_eq!(history.series[0].key, "response time");
    assert_eq!(history.series[0].unit, "ms");
    let points: Vec<(&str, f64)> = history.series[0]
        .points
        .iter()
        .map(|p| (p.start.as_str(), p.avg))
        .collect();
    assert_eq!(
        points,
        [("2026-03-10 10:00", 12.0), ("2026-03-10 10:05", 12.0)]
    );

    // Raw results are newest first, whatever the period.
    let recent: Vec<(&str, Status)> = history
        .recent
        .iter()
        .map(|r| (r.at.as_str(), r.status))
        .collect();
    assert_eq!(
        recent,
        [
            ("2026-03-10 10:20:00", Status::Down),
            ("2026-03-10 10:07:00", Status::Up),
            ("2026-03-10 10:03:00", Status::Up),
            ("2026-02-18 09:00:00", Status::Down),
        ]
    );
    assert_eq!(
        history.recent[0].message.as_deref(),
        Some("Request timed out")
    );
    assert_eq!(history.incident_count, 0);
    assert_eq!(history.mttr_secs, None);
}

#[tokio::test]
async fn longer_periods_read_coarser_rollups_and_days_can_be_chosen() {
    let (hub, probe) = hub_with_history().await;
    let a = check_id(&probe.id, "a");
    let get = |query: String| {
        let hub = &hub;
        let a = a.clone();
        async move {
            hub.get_json::<CheckHistory>(&format!("/api/checks/{a}/history?{query}"))
                .await
        }
    };

    let month = get("range=30d".into()).await;
    assert_eq!(month.bucket_secs, 3600, "720 hourly buckets");
    assert_eq!(month.totals, counts(2, 2), "20 days ago is in");

    let year = get("range=1y".into()).await;
    assert_eq!(year.bucket_secs, 86_400);
    assert_eq!(year.totals, counts(2, 2));

    let day = get("from=2026-02-18&to=2026-02-18".into()).await;
    assert_eq!(
        (day.from.as_str(), day.to.as_str()),
        ("2026-02-18 00:00", "2026-02-19 00:00")
    );
    assert_eq!(day.totals, counts(0, 1));
    assert_eq!(day.availability_pct, Some(0.0));

    let owner = hub.login().await;
    for bad in [
        "range=2y",
        "from=2026-03-05&to=2026-03-01",
        "from=2026-03-01",
        "from=yesterday&to=2026-03-01",
    ] {
        let refused = hub
            .get(&format!("/api/checks/{a}/history?{bad}"), Some(&owner))
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(refused.error_code(), "invalid_range", "{bad}");
    }
}

#[tokio::test]
async fn incidents_and_mttr_come_from_the_checks_incidents() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    // Two failures in a row alert (the default); recovering closes it. Each
    // report is a minute later; the last moves the clock past the closing,
    // which a period ending now would otherwise leave out.
    for up in [false, false, false, true, true, true] {
        hub.evaluate_report(&probe, vec![ping("a", "1", up)]).await;
    }
    let a = check_id(&probe.id, "a");
    let history: CheckHistory = hub
        .get_json(&format!("/api/checks/{a}/history?range=24h"))
        .await;

    assert_eq!(history.incident_count, 1);
    let incident = &history.incidents[0];
    assert_eq!(incident.severity, Status::Down);
    let closed = incident.closed_unix.expect("closed by the recovery");
    assert_eq!(history.mttr_secs, Some(closed - incident.opened_unix));
    assert!(history.mttr_secs.unwrap() > 0);
}

#[tokio::test]
async fn only_checks_of_the_users_companies_are_shown() {
    let (hub, probe) = hub_with_history().await;
    let a = check_id(&probe.id, "a");
    let path = format!("/api/checks/{a}/history");

    let viewer = hub.login_as(Role::Viewer).await;
    assert_eq!(hub.get(&path, Some(&viewer)).await.status, StatusCode::OK);

    let acme = hub
        .login_scoped("acme-client", Role::Client, &["acme"])
        .await;
    assert_eq!(hub.get(&path, Some(&acme)).await.status, StatusCode::OK);

    // Another company's user sees nothing, as if it didn't exist.
    hub.place_probe(&TestProbe::new("z").id, "z", "zeta", "main")
        .await;
    let zeta = hub
        .login_scoped("zeta-client", Role::Client, &["zeta"])
        .await;
    let hidden = hub.get(&path, Some(&zeta)).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    let missing = hub
        .get("/api/checks/no-such-check/history", Some(&viewer))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(hub.get(&path, None).await.status, StatusCode::UNAUTHORIZED);
}
