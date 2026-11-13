//! The monthly availability report (M6-05): `GET /api/companies/{id}/report`
//! and its CSV, from the rollups, incidents and maintenance windows.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use hub_lib::model::{MonthlyReport, Role, Status, StatusCounts};
use serde_json::json;
use support::*;

/// The hub's clock is 2026-03-10T12:00:00Z. Seeded, oldest first:
/// January: `a` Up. February: `a` Up and Down, `b` Up. March: `a` Down.
async fn hub_with_months() -> (TestHub, TestProbe) {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let send = |timestamp: &str, resources| {
        let report = probe.signed_report("acme", at(timestamp), resources);
        let hub = &hub;
        async move { hub.seed_report(&report).await }
    };
    send("2026-01-20T10:00:00Z", vec![ping("a", "1", true)]).await;
    send(
        "2026-02-03T10:00:00Z",
        vec![ping("a", "1", true), ping("b", "2", true)],
    )
    .await;
    send("2026-02-27T23:30:00Z", vec![ping("a", "1", false)]).await;
    send("2026-03-02T08:00:00Z", vec![ping("a", "1", false)]).await;
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
async fn a_month_has_each_sites_and_checks_availability_and_the_month_before() {
    let (hub, _) = hub_with_months().await;
    let report: MonthlyReport = hub
        .get_json("/api/companies/acme/report?month=2026-02")
        .await;

    assert_eq!(report.company_id, "acme");
    assert_eq!(report.month.param(), "2026-02");
    assert!(!report.partial && report.exact);
    assert_eq!(report.totals, counts(2, 1));
    assert_eq!(report.previous_pct, Some(100.0), "January: a was Up");
    assert_eq!(report.sites.len(), 1);
    let site = &report.sites[0];
    assert_eq!(site.name, "hq-site");
    assert_eq!(
        (site.from.as_str(), site.to.as_str()),
        ("2026-02-01 00:00", "2026-03-01 00:00")
    );
    let rows: Vec<(&str, StatusCounts, Option<f64>, Option<f64>)> = site
        .checks
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.totals,
                c.availability_pct,
                c.previous_pct,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("a", counts(1, 1), Some(50.0), Some(100.0)),
            ("b", counts(1, 0), Some(100.0), None),
        ]
    );
    assert_eq!(site.checks[0].kind, "ping");
    assert_eq!(site.checks[0].probe_name, "hq");

    // Last month, unless asked; this month so far.
    let default: MonthlyReport = hub.get_json("/api/companies/acme/report").await;
    assert_eq!(default.month.param(), "2026-02");
    let march: MonthlyReport = hub
        .get_json("/api/companies/acme/report?month=2026-03&target=99.9")
        .await;
    assert!(march.partial);
    assert_eq!(march.target_pct, Some(99.9));
    assert_eq!(march.totals, counts(0, 1));
    assert_eq!(march.previous_pct, Some(200.0 / 3.0));
}

#[tokio::test]
async fn a_site_reports_its_own_local_month() {
    let (hub, _) = hub_with_months().await;
    // Another probe of the same site: Down at 28 February 23:30 UTC, which
    // is already 1 March 00:30 in Madrid (UTC+1).
    let late = TestProbe::new("hq2");
    hub.place_probe(&late.id, "hq2", "acme", "hq-site").await;
    hub.seed_report(&late.signed_report(
        "acme",
        at("2026-02-28T23:30:00Z"),
        vec![ping("c", "3", false)],
    ))
    .await;
    let february = || async {
        hub.get_json::<MonthlyReport>("/api/companies/acme/report?month=2026-02")
            .await
    };
    assert_eq!(
        february().await.sites[0].totals,
        counts(2, 2),
        "UTC: February"
    );

    let owner = hub.login().await;
    let set = hub
        .post_json(
            "/api/companies/acme/sites/hq-site",
            Some(&owner),
            json!({ "name": "hq-site", "timezone": "Europe/Madrid" }),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text());

    let report = february().await;
    let site = &report.sites[0];
    assert_eq!(site.timezone, "Europe/Madrid");
    assert_eq!(
        (site.from.as_str(), site.to.as_str()),
        ("2026-02-01 00:00", "2026-03-01 00:00"),
        "local times"
    );
    assert_eq!(site.totals, counts(2, 1), "Madrid: March, not February");
    let march: MonthlyReport = hub
        .get_json("/api/companies/acme/report?month=2026-03")
        .await;
    assert_eq!(march.sites[0].totals, counts(0, 2));
}

#[tokio::test]
async fn incidents_have_their_cause_and_downtime_and_maintenance_is_listed() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let window = hub
        .post_json(
            "/api/companies/acme/maintenance",
            Some(&owner),
            json!({
                "scope": { "kind": "company" },
                "reason": "patching",
                "timezone": "UTC",
                "schedule": { "every": "weekly", "days": [1], "start": "22:00", "minutes": 60 },
            }),
        )
        .await;
    assert_eq!(window.status, StatusCode::OK, "{}", window.text());
    // Two failures in a row alert; recovering closes it (a minute a report).
    for up in [false, false, false, true, true, true] {
        hub.evaluate_report(&probe, vec![ping("a", "1", up)]).await;
    }

    let report: MonthlyReport = hub
        .get_json("/api/companies/acme/report?month=2026-03")
        .await;
    assert_eq!(report.incident_count, 1);
    let incident = &report.incidents[0];
    assert_eq!(incident.check_name.as_deref(), Some("a"));
    assert_eq!(incident.severity, Status::Down);
    assert!(incident.closed_at.is_some());
    assert!(incident.duration_secs > 0);
    assert!(
        incident.cause.contains("Request timed out"),
        "what the hub saw: {}",
        incident.cause
    );
    assert_eq!(report.downtime_secs, incident.duration_secs);
    let a = &report.sites[0].checks[0];
    assert_eq!(
        (a.incident_count, a.downtime_secs),
        (1, incident.duration_secs)
    );

    // Mondays 22:00 in March 2026 until now (the 10th): the 2nd and 9th.
    let starts: Vec<&str> = report
        .maintenance
        .iter()
        .map(|m| m.start.as_str())
        .collect();
    assert_eq!(starts, ["2026-03-02 22:00", "2026-03-09 22:00"]);
    assert_eq!(report.maintenance[0].scope, "The whole company");
    assert_eq!(report.maintenance[0].reason, "patching");
}

#[tokio::test]
async fn the_csv_has_a_row_per_check() {
    let (hub, _) = hub_with_months().await;
    let owner = hub.login().await;
    let csv = hub
        .get("/api/companies/acme/report.csv?month=2026-02", Some(&owner))
        .await;
    assert_eq!(csv.status, StatusCode::OK);
    assert_eq!(csv.headers["content-type"], "text/csv; charset=utf-8");
    assert_eq!(
        csv.headers["content-disposition"],
        "attachment; filename=\"blentinel-acme-2026-02.csv\""
    );
    let text = csv.text();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("site,probe,check,kind,availability_pct"));
    assert_eq!(
        lines[1],
        "hq-site,hq,a,ping,50.000,100.000,1,0,1,0,0,0,0,false"
    );
    assert_eq!(lines[2], "hq-site,hq,b,ping,100.000,,1,0,0,0,0,0,0,false");
}

#[tokio::test]
async fn a_report_is_only_for_the_users_companies() {
    let (hub, _) = hub_with_months().await;
    hub.place_probe(&TestProbe::new("z").id, "z", "zeta", "main")
        .await;
    let path = "/api/companies/acme/report?month=2026-02";
    let csv = "/api/companies/acme/report.csv?month=2026-02";

    let viewer = hub.login_as(Role::Viewer).await;
    assert_eq!(hub.get(path, Some(&viewer)).await.status, StatusCode::OK);
    let client = hub
        .login_scoped("acme-client", Role::Client, &["acme"])
        .await;
    assert_eq!(hub.get(path, Some(&client)).await.status, StatusCode::OK);
    assert_eq!(hub.get(csv, Some(&client)).await.status, StatusCode::OK);

    let other = hub
        .login_scoped("zeta-client", Role::Client, &["zeta"])
        .await;
    assert_eq!(
        hub.get(path, Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        hub.get(csv, Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    let missing = hub.get("/api/companies/nobody/report", Some(&viewer)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(hub.get(path, None).await.status, StatusCode::UNAUTHORIZED);

    for bad in ["month=2026-04", "month=2026-13", "target=0"] {
        let refused = hub
            .get(&format!("/api/companies/acme/report?{bad}"), Some(&viewer))
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(refused.error_code(), "invalid_report", "{bad}");
    }
}
