//! Incidents (M3-05, D-67): opened, changed and closed by the alert engine,
//! one per outage; acknowledged and noted over the API; scoped; Clients
//! don't see what people wrote.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::{CheckId, CheckSpec};
use hub_lib::model::{AlertKind, IncidentDetail, IncidentEventKind, IncidentInfo, Role, Status};
use hub_lib::service::{liveness, silences};
use serde_json::json;
use support::*;

fn nas(up: bool) -> Vec<Res> {
    vec![ping("NAS", "10.0.0.20", up)]
}

fn disk(percent: f64) -> Vec<Res> {
    vec![metric("C:", CheckSpec::Disk { path: None }, percent, "%")]
}

async fn list(hub: &TestHub, state: &str) -> Vec<IncidentInfo> {
    hub.get_json(&format!("/api/incidents?state={state}")).await
}

async fn detail(hub: &TestHub, id: i64) -> IncidentDetail {
    hub.get_json(&format!("/api/incidents/{id}")).await
}

fn kinds(detail: &IncidentDetail) -> Vec<IncidentEventKind> {
    detail.events.iter().map(|e| e.kind).collect()
}

/// A check that went down (two results in a row) and is still down: its incident.
async fn outage(hub: &TestHub, probe: &TestProbe) -> IncidentInfo {
    for up in [true, false, false] {
        hub.evaluate_report(probe, nas(up)).await;
    }
    let open = list(hub, "open").await;
    assert_eq!(open.len(), 1);
    open[0].clone()
}

#[tokio::test]
async fn an_outage_is_one_incident_from_its_alert_to_its_recovery() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    let incident = outage(&hub, &probe).await;
    assert_eq!(incident.title(), "NAS");
    assert_eq!(
        (incident.status, incident.severity),
        (Status::Down, Status::Down)
    );
    assert_eq!(
        incident.check_id.as_deref(),
        Some(check_id(&probe.id, "NAS").as_str())
    );
    assert_eq!(incident.site_id, "hq-site");
    let down = &hub.notifier.sent()[0];
    assert_eq!(
        down.incident.map(|i| i.id),
        Some(incident.id),
        "the alert links to it"
    );
    assert_eq!(
        down.link("https://hub.example.com/").as_deref(),
        Some(format!("https://hub.example.com/incidents/{}", incident.id).as_str())
    );

    // Still down: the same incident, nothing new.
    hub.evaluate_report(&probe, nas(false)).await;
    for _ in 0..2 {
        hub.evaluate_report(&probe, nas(true)).await;
    }

    assert!(list(&hub, "open").await.is_empty());
    let closed = list(&hub, "closed").await;
    assert_eq!(closed.len(), 1);
    assert_eq!(
        closed[0].duration, "3 min",
        "from the alert to the recovery"
    );
    let timeline = detail(&hub, incident.id).await;
    assert_eq!(
        kinds(&timeline),
        [IncidentEventKind::Opened, IncidentEventKind::Closed]
    );
    assert!(
        timeline.events[1].text.starts_with("Recovered"),
        "{:?}",
        timeline.events[1]
    );

    let recovered = hub.notifier.sent().pop().unwrap();
    assert_eq!(recovered.kind, AlertKind::Recovered);
    assert_eq!(recovered.lasted().as_deref(), Some("3 min"));
    assert!(recovered.text(None).contains("Lasted: 3 min"));
    // One tag on the outage's alerts, for one ticket in a PSA (D-72).
    let tag = format!("[BLENTINEL #{}]", incident.id);
    let subjects = hub.notifier.subjects();
    assert_eq!(subjects.len(), 2);
    assert!(subjects.iter().all(|s| s.starts_with(&tag)), "{subjects:?}");

    // The next outage is a new incident.
    outage(&hub, &probe).await;
    assert_eq!(list(&hub, "all").await.len(), 2);
}

#[tokio::test]
async fn warning_then_down_is_one_incident_at_its_worst() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await; // built in: warning 90 %, down 97 %

    for percent in [95.0, 95.0, 98.0, 98.0, 92.0, 92.0] {
        hub.evaluate_report(&probe, disk(percent)).await;
    }

    let open = list(&hub, "open").await;
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].status, open[0].severity),
        (Status::Warning, Status::Down)
    );
    let timeline = detail(&hub, open[0].id).await;
    assert_eq!(
        kinds(&timeline),
        [
            IncidentEventKind::Opened,
            IncidentEventKind::Changed,
            IncidentEventKind::Changed
        ]
    );
    assert!(
        timeline.events[1].text.starts_with("Down: disk usage 98%"),
        "{:?}",
        timeline.events[1]
    );
}

#[tokio::test]
async fn a_silenced_outage_is_an_incident_nobody_was_told_about() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
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
    silences::silence_check(hub.store(), &key, "known", Some(24), hub.now())
        .await
        .unwrap();

    let incident = outage(&hub, &probe).await;

    assert!(hub.notifier.subjects().is_empty());
    let timeline = detail(&hub, incident.id).await;
    assert!(
        timeline.events[0]
            .text
            .ends_with("(silenced: nobody was told)")
    );
}

#[tokio::test]
async fn a_probe_that_stops_reporting_is_one_incident_until_it_reports_again() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let (storage, notifier, config) = (
        hub.state.storage.get(),
        hub.state.notifier.clone(),
        hub.state.config.clone(),
    );

    hub.report_now(&probe, "acme", nas(true)).await;
    hub.clock
        .advance(Duration::seconds(PROBE_TIMEOUT_SECS as i64 + 1));
    liveness::sweep_expired_probes(&storage, &notifier, &config, hub.now())
        .await
        .unwrap();
    hub.wait_for_alerts(1).await;

    let open = list(&hub, "open").await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].check_id, None);
    assert_eq!(open[0].title(), "Probe hq stopped reporting");
    assert_eq!(
        hub.notifier.sent()[0].incident.map(|i| i.id),
        Some(open[0].id)
    );

    hub.clock.advance(Duration::minutes(10));
    hub.report_now(&probe, "acme", nas(true)).await;

    let closed = list(&hub, "closed").await;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].duration, "10 min");
    let timeline = detail(&hub, closed[0].id).await;
    assert_eq!(
        timeline.events.last().unwrap().text,
        "The probe reports again"
    );
}

#[tokio::test]
async fn deleting_a_check_closes_its_incident() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let incident = outage(&hub, &probe).await;

    let response = hub
        .post_json(
            &format!(
                "/api/checks/{}/delete",
                incident.check_id.as_deref().unwrap()
            ),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    let closed = detail(&hub, incident.id).await;
    assert!(!closed.incident.is_open());
    let last = closed.events.last().unwrap();
    assert_eq!(
        (last.text.as_str(), last.username.as_deref()),
        ("The check was deleted", Some(OWNER))
    );
}

#[tokio::test]
async fn a_technician_acknowledges_once_and_adds_notes() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let incident = outage(&hub, &probe).await;
    let technician = hub.login_as(Role::Technician).await;
    let path = |what: &str| format!("/api/incidents/{}/{what}", incident.id);

    let response = hub
        .post_json(
            &path("acknowledge"),
            Some(&technician),
            json!({ "text": "On my way" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let acked: IncidentInfo = response.json();
    assert_eq!(acked.acknowledged_by.as_deref(), Some("technician"));

    let again = hub
        .post_json(&path("acknowledge"), Some(&technician), json!({}))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.error_code(), "incident_conflict");
    assert!(again.text().contains("technician acknowledged it already"));

    let empty = hub
        .post_json(&path("notes"), Some(&technician), json!({ "text": "  " }))
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.error_code(), "invalid_incident");
    let note = hub
        .post_json(
            &path("notes"),
            Some(&technician),
            json!({ "text": "The NAS PSU died" }),
        )
        .await;
    assert_eq!(note.status, StatusCode::OK);

    let timeline = detail(&hub, incident.id).await;
    assert_eq!(
        kinds(&timeline),
        [
            IncidentEventKind::Opened,
            IncidentEventKind::Acknowledged,
            IncidentEventKind::Note
        ]
    );
    assert_eq!(timeline.events[1].text, "On my way");

    let audit: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_log WHERE action LIKE 'incident.%' ORDER BY id",
    )
    .fetch_all(hub.pool())
    .await
    .unwrap();
    assert_eq!(audit, ["incident.acknowledge", "incident.note"]);
}

#[tokio::test]
async fn a_closed_incident_cant_be_acknowledged_but_takes_notes() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let incident = outage(&hub, &probe).await;
    for _ in 0..2 {
        hub.evaluate_report(&probe, nas(true)).await;
    }
    let owner = hub.login().await;

    let ack = hub
        .post_json(
            &format!("/api/incidents/{}/acknowledge", incident.id),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(ack.status, StatusCode::CONFLICT);
    assert!(ack.text().contains("it is closed"));
    let note = hub
        .post_json(
            &format!("/api/incidents/{}/notes", incident.id),
            Some(&owner),
            json!({ "text": "Replaced the PSU" }),
        )
        .await;
    assert_eq!(note.status, StatusCode::OK);
}

#[tokio::test]
async fn viewers_see_incidents_but_cant_act_on_them() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let incident = outage(&hub, &probe).await;
    let viewer = hub.login_as(Role::Viewer).await;

    let seen: Vec<IncidentInfo> = hub.get("/api/incidents", Some(&viewer)).await.json();
    assert_eq!(seen.len(), 1);
    let response = hub
        .post_json(
            &format!("/api/incidents/{}/acknowledge", incident.id),
            Some(&viewer),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn incidents_are_scoped_and_clients_dont_see_notes() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let incident = outage(&hub, &probe).await;
    let owner = hub.login().await;
    hub.post_json(
        &format!("/api/incidents/{}/acknowledge", incident.id),
        Some(&owner),
        json!({ "text": "internal: blame the vendor" }),
    )
    .await;
    hub.post_json(
        &format!("/api/incidents/{}/notes", incident.id),
        Some(&owner),
        json!({ "text": "internal: ticket 42" }),
    )
    .await;

    let client = hub.login_scoped("client", Role::Client, &["acme"]).await;
    let seen: IncidentDetail = hub
        .get(&format!("/api/incidents/{}", incident.id), Some(&client))
        .await
        .json();
    assert_eq!(
        kinds(&seen),
        [IncidentEventKind::Opened, IncidentEventKind::Acknowledged],
        "no notes"
    );
    assert_eq!(seen.events[1].text, "", "no acknowledgement note");

    // Another company's technician.
    hub.place_probe("zeta-probe", "z", "zeta", "z-site").await;
    let other = hub.login_scoped("other", Role::Technician, &["zeta"]).await;
    let list: Vec<IncidentInfo> = hub.get("/api/incidents", Some(&other)).await.json();
    assert!(list.is_empty());
    let response = hub
        .get(&format!("/api/incidents/{}", incident.id), Some(&other))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = hub
        .post_json(
            &format!("/api/incidents/{}/notes", incident.id),
            Some(&other),
            json!({ "text": "x" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_list_state_is_refused() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let response = hub.get("/api/incidents?state=lost", Some(&owner)).await;

    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.error_code(), "invalid_incident");
}

#[tokio::test]
async fn removing_a_probe_forgets_its_incidents() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    outage(&hub, &probe).await;

    hub.store().remove_probe(&probe.id).await.unwrap();

    assert!(list(&hub, "all").await.is_empty());
}
