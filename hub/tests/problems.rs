//! What is wrong now (M6-03): `GET /api/problems` from reports ingested as a
//! probe sends them, with the incidents the alert engine opens, filtered,
//! grouped by company › site, and scoped to the user's companies.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::CheckSpec;
use hub_lib::model::{IncidentInfo, ProblemsView, Role, Status};
use hub_lib::service::alerts;
use serde_json::json;
use support::*;

/// Every check of `probe`, reported now, a minute after the last report.
async fn report(hub: &TestHub, probe: &TestProbe, company: &str, resources: Vec<Res>) {
    hub.clock.advance(Duration::minutes(1));
    hub.report_now(probe, company, resources).await;
}

fn hq_checks(nas_up: bool) -> Vec<Res> {
    vec![
        ping("NAS", "10.0.0.20", nas_up),
        ping("Router", "10.0.0.1", true),
        // Over the built-in 90% warning limit.
        metric("C:", CheckSpec::Disk { path: None }, 95.0, "%"),
    ]
}

/// Acme: `hq` (its NAS down three times, its disk at 95%) and `br`, which
/// reports once and then stops. Zeta: `z`, its printer down once, lately.
async fn hub_with_problems() -> (TestHub, TestProbe, TestProbe, TestProbe) {
    let hq = TestProbe::new("hq");
    let br = TestProbe::new("br");
    let z = TestProbe::new("z");
    let hub = TestHub::new(&[&hq, &br]).await;
    hub.place_probe(&z.id, "z", "zeta", "main").await;

    report(&hub, &br, "acme", vec![ping("Printer", "10.0.0.30", true)]).await;
    report(&hub, &hq, "acme", hq_checks(false)).await;
    report(&hub, &hq, "acme", hq_checks(false)).await;
    // br has been silent for over the probe timeout: the sweep expires it
    // and opens its incident. The hub opens it in a task of its own, so the
    // test runs that step and waits for it. hq and z report again.
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    let expired = hub.sweep_expired().await;
    let (storage, notifier) = (hub.state.storage.get(), hub.state.notifier.clone());
    alerts::evaluate_probe_expiry(
        storage.as_ref(),
        notifier.as_ref(),
        &expired,
        PROBE_TIMEOUT_SECS,
        hub.now(),
    )
    .await
    .unwrap();
    report(&hub, &hq, "acme", hq_checks(false)).await;
    report(&hub, &z, "zeta", vec![ping("Printer", "10.9.0.30", false)]).await;
    (hub, hq, br, z)
}

/// `(company, site, probe, check, status)` in the order shown.
fn shown(view: &ProblemsView) -> Vec<(String, String, String, Option<String>, Status)> {
    let mut rows = Vec::new();
    for c in &view.companies {
        for s in &c.sites {
            for p in &s.problems {
                rows.push((
                    c.company_id.clone(),
                    s.site_id.clone(),
                    p.probe_name.clone(),
                    p.check_name.clone(),
                    p.status,
                ));
            }
        }
    }
    rows
}

fn row(
    company: &str,
    site: &str,
    probe: &str,
    check: Option<&str>,
    status: Status,
) -> (String, String, String, Option<String>, Status) {
    (
        company.into(),
        site.into(),
        probe.into(),
        check.map(str::to_string),
        status,
    )
}

#[tokio::test]
async fn whats_wrong_now_is_grouped_by_company_and_site_with_its_incident() {
    let (hub, ..) = hub_with_problems().await;
    let view: ProblemsView = hub.get_json("/api/problems").await;

    assert_eq!(
        shown(&view),
        [
            row("acme", "hq-site", "hq", Some("NAS"), Status::Down),
            row("acme", "hq-site", "hq", Some("C:"), Status::Warning),
            row("acme", "br-site", "br", None, Status::Unknown),
            row("zeta", "main", "z", Some("Printer"), Status::Down),
        ],
        "the Router is Up; br stopped reporting, listed once"
    );
    assert_eq!(
        (view.total, view.down, view.warning, view.unknown),
        (4, 2, 1, 1)
    );
    assert_eq!(view.kinds, ["disk", "ping"]);

    let hq = &view.companies[0].sites[0].problems;
    assert_eq!(hq[0].target.as_deref(), Some("10.0.0.20"));
    assert_eq!(hq[0].message.as_deref(), Some("Request timed out"));
    assert!(hq[1].reason.is_some(), "why the disk is a Warning");
    let br = &view.companies[0].sites[1].problems[0];
    assert!(br.last_seen_at.is_some());
    assert!(br.incident.is_some(), "the probe's own incident");

    // Two failures in a row opened the NAS's incident; nobody is on it yet.
    let nas = hq[0].incident.clone().expect("an open incident");
    assert_eq!(nas.acknowledged_by, None);
    assert!(
        view.companies[1].sites[0].problems[0].incident.is_none(),
        "one failure: no incident yet"
    );

    // Someone acknowledges it: the list says who.
    let technician = hub.login_as(Role::Technician).await;
    let ack = hub
        .post_json(
            &format!("/api/incidents/{}/acknowledge", nas.id),
            Some(&technician),
            json!({ "text": "On my way" }),
        )
        .await;
    assert_eq!(ack.status, StatusCode::OK, "{}", ack.text());
    let incident: IncidentInfo = ack.json();
    assert_eq!(incident.check_name.as_deref(), Some("NAS"));
    let view: ProblemsView = hub.get_json("/api/problems").await;
    let nas = view.companies[0].sites[0].problems[0]
        .incident
        .clone()
        .unwrap();
    assert_eq!(nas.acknowledged_by.as_deref(), Some("technician"));
    assert!(nas.acknowledged_at.is_some());
}

#[tokio::test]
async fn search_status_and_kind_filter_the_list() {
    let (hub, ..) = hub_with_problems().await;
    let get = |query: &str| {
        let hub = &hub;
        let path = format!("/api/problems?{query}");
        async move { shown(&hub.get_json::<ProblemsView>(&path).await) }
    };

    assert_eq!(
        get("q=nas").await,
        [row("acme", "hq-site", "hq", Some("NAS"), Status::Down)]
    );
    assert_eq!(
        get("q=10.9.0").await,
        [row("zeta", "main", "z", Some("Printer"), Status::Down)],
        "by target"
    );
    assert_eq!(get("q=printer").await.len(), 1, "br's printer is Up");
    assert_eq!(
        get("q=acme%20br").await.len(),
        1,
        "every word: the stopped probe"
    );
    assert_eq!(
        get("status=warning").await,
        [row("acme", "hq-site", "hq", Some("C:"), Status::Warning)]
    );
    assert_eq!(
        get("status=unknown").await,
        [row("acme", "br-site", "br", None, Status::Unknown)]
    );
    assert_eq!(get("status=down,warning").await.len(), 3);
    assert_eq!(
        get("kind=disk").await,
        [row("acme", "hq-site", "hq", Some("C:"), Status::Warning)]
    );
    assert_eq!(get("kind=ping&status=down&q=zeta").await.len(), 1);
    assert!(get("q=nothing-like-this").await.is_empty());

    let owner = hub.login().await;
    let refused = hub.get("/api/problems?status=up", Some(&owner)).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.error_code(), "invalid_problems");
}

#[tokio::test]
async fn a_user_sees_only_their_companies_problems() {
    let (hub, ..) = hub_with_problems().await;
    let companies = |view: &ProblemsView| {
        view.companies
            .iter()
            .map(|c| c.company_id.clone())
            .collect::<Vec<_>>()
    };
    let as_user = |cookie: String| {
        let hub = &hub;
        async move {
            let response = hub.get("/api/problems", Some(&cookie)).await;
            assert_eq!(response.status, StatusCode::OK, "{}", response.text());
            response.json::<ProblemsView>()
        }
    };

    let viewer = as_user(hub.login_as(Role::Viewer).await).await;
    assert_eq!(companies(&viewer), ["acme", "zeta"]);

    // A Client of Acme: Acme's problems only, totals and kinds included.
    let acme = as_user(
        hub.login_scoped("acme-client", Role::Client, &["acme"])
            .await,
    )
    .await;
    assert_eq!(companies(&acme), ["acme"]);
    assert_eq!(acme.total, 3);
    let zeta = as_user(
        hub.login_scoped("zeta-client", Role::Client, &["zeta"])
            .await,
    )
    .await;
    assert_eq!(companies(&zeta), ["zeta"]);
    assert_eq!(
        (zeta.total, zeta.kinds.clone()),
        (1, vec!["ping".to_string()])
    );
    // Searching for another company's check finds nothing.
    let hub_ref = &hub;
    let cookie = hub.session_for("zeta-client").await;
    let search = hub_ref.get("/api/problems?q=nas", Some(&cookie)).await;
    assert!(search.json::<ProblemsView>().companies.is_empty());

    assert_eq!(
        hub.get("/api/problems", None).await.status,
        StatusCode::UNAUTHORIZED
    );
}
