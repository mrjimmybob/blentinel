//! Probe health telemetry and the fleet view (M4-02, D-73): what a probe
//! says about itself in its reports, and `GET /api/probes` with it.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::models::{ProbeHealth, ProbeProblem};
use hub_lib::model::{ConfigSync, ProbeChecks, ProbeInfo};
use serde_json::json;
use support::*;

/// Report from `probe` a minute later, running `revision`, with `health`.
async fn report(
    hub: &TestHub,
    probe: &TestProbe,
    revision: &str,
    protocol: u16,
    health: Option<ProbeHealth>,
) {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.protocol_version = protocol;
    report.config_revision = Some(revision.into());
    report.health = health;
    probe.sign(&mut report);
    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

async fn listed(hub: &TestHub) -> ProbeInfo {
    let probes: Vec<ProbeInfo> = hub.get_json("/api/probes").await;
    assert_eq!(probes.len(), 1);
    probes.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_probes_health_is_listed_with_what_needs_attention() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let desired: ProbeChecks = hub
        .get_json(&format!("/api/probes/{}/checks", probe.id))
        .await;

    let well = ProbeHealth {
        started_at: hub.now() - Duration::hours(2),
        queued_reports: 0,
        clock_offset_secs: Some(1),
        last_error: None,
    };
    report(
        &hub,
        &probe,
        &desired.config_revision,
        8,
        Some(well.clone()),
    )
    .await;
    let fine = listed(&hub).await;
    assert_eq!(fine.health.as_ref(), Some(&well));
    assert_eq!(fine.config_sync, Some(ConfigSync::InSync));
    assert!(fine.concerns().is_empty(), "{:?}", fine.concerns());

    let unwell = ProbeHealth {
        queued_reports: 3,
        clock_offset_secs: Some(-90),
        last_error: Some(ProbeProblem {
            at: hub.now(),
            message: "Can't reach the hub: timed out".into(),
        }),
        ..well
    };
    report(
        &hub,
        &probe,
        &desired.config_revision,
        8,
        Some(unwell.clone()),
    )
    .await;
    // A check added since: the probe gets it with its next answer.
    let added = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(&owner),
            json!({ "name": "Router", "spec": { "kind": "ping", "host": "192.168.1.1" } }),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text());

    let probe_info = listed(&hub).await;
    assert_eq!(probe_info.health.as_ref(), Some(&unwell));
    assert_eq!(probe_info.config_sync, Some(ConfigSync::Pending));
    assert_eq!(
        probe_info.concerns(),
        ["3 reports held back", "Clock off by -90 s", "Had a problem"]
    );
}

/// A report that says nothing about the probe's health: what the last one
/// said stays, and a probe that never said anything has none.
#[tokio::test]
async fn a_report_without_health_keeps_the_last() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    report(&hub, &probe, "r1", 8, None).await;
    assert_eq!(listed(&hub).await.health, None);

    let health = ProbeHealth {
        started_at: hub.now(),
        queued_reports: 0,
        clock_offset_secs: None,
        last_error: None,
    };
    report(&hub, &probe, "r1", 9, Some(health.clone())).await;
    report(&hub, &probe, "r1", 8, None).await;
    assert_eq!(listed(&hub).await.health, Some(health));
}

/// The health is signed: a report whose health was changed after signing
/// is refused like any other tampered report.
#[tokio::test]
async fn tampered_health_is_refused() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.health = Some(ProbeHealth {
        started_at: hub.now(),
        queued_reports: 50,
        clock_offset_secs: None,
        last_error: None,
    });
    probe.sign(&mut report);
    report.health.as_mut().unwrap().queued_reports = 0;
    assert_eq!(hub.send_report(&report).await, StatusCode::FORBIDDEN);
}
