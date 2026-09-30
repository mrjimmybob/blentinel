//! Checks as hub records (M2-04, D-49): managing them through the API, and
//! the config the hub sends a probe in its signed answer to a report.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::ReportAck;
use common::checks::{CheckResult, CheckSpec, ConfigRejection, Health, ProbeConfig};
use hub_lib::model::{CheckInfo, ConfigSync, ProbeChecks};
use serde_json::{Value, json};
use support::*;

fn router_check() -> Value {
    json!({ "name": "Router", "spec": { "kind": "ping", "host": "192.168.1.1" } })
}

async fn add(hub: &TestHub, cookie: &str, probe: &TestProbe, body: Value) -> CheckInfo {
    let response = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(cookie),
            body,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

async fn checks_of(hub: &TestHub, probe: &TestProbe) -> ProbeChecks {
    hub.get_json(&format!("/api/probes/{}/checks", probe.id))
        .await
}

/// Report from `probe` a minute later, running `revision` (and having
/// refused `rejected`), with `results`; the hub's answer.
async fn report(
    hub: &TestHub,
    probe: &TestProbe,
    revision: Option<&str>,
    rejected: Option<ConfigRejection>,
    results: Vec<CheckResult>,
) -> ReportAck {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.config_revision = revision.map(str::to_string);
    report.config_rejected = rejected;
    report.results = results;
    probe.sign(&mut report);
    let (status, ack) = hub.send_report_for_ack(probe, &report).await;
    assert_eq!(status, StatusCode::OK);
    ack.expect("a trusted ack")
}

fn up(check: &CheckInfo) -> CheckResult {
    CheckResult {
        check_id: common::checks::CheckId(check.id.clone()),
        status: Health::Up,
        message: "Responded".into(),
        latency_ms: Some(3),
        metrics: vec![],
    }
}

// ---------------------------------------------------------------------------
// The API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checks_are_added_listed_changed_and_deleted() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let router = add(&hub, &owner, &probe, router_check()).await;
    assert_eq!(router.name, "Router");
    assert_eq!(router.target, "192.168.1.1");
    assert!(router.enabled, "enabled by default");
    assert_eq!(router.updated_by.as_deref(), Some(OWNER));
    let disk = json!({ "name": "  Data disk ", "spec": { "kind": "disk", "path": "/data" } });
    let disk = add(&hub, &owner, &probe, disk).await;
    assert_eq!(disk.name, "Data disk", "trimmed");

    let listed = checks_of(&hub, &probe).await;
    let names: Vec<_> = listed.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Data disk", "Router"]);
    assert_eq!(listed.interval_secs, 60);
    assert_eq!(
        listed.sync,
        ConfigSync::Pending,
        "the probe hasn't reported yet"
    );

    let change = json!({
        "name": "Gateway", "spec": { "kind": "tcp", "host": "192.168.1.1", "port": 443 },
        "enabled": false,
    });
    let changed = hub
        .post_json(&format!("/api/checks/{}", router.id), Some(&owner), change)
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text());
    let changed: CheckInfo = changed.json();
    assert_eq!(
        (
            changed.id.as_str(),
            changed.target.as_str(),
            changed.enabled
        ),
        (router.id.as_str(), "192.168.1.1:443", false),
        "same ID, new target"
    );

    let deleted = hub
        .post_json(
            &format!("/api/checks/{}/delete", disk.id),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(checks_of(&hub, &probe).await.checks.len(), 1);

    let entries: Vec<Value> = hub.get_json("/api/audit?action=check").await;
    let actions: Vec<_> = entries
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "check.delete",
            "check.update",
            "check.create",
            "check.create"
        ]
    );
    assert_eq!(entries[1]["company_id"], "acme");
}

#[tokio::test]
async fn invalid_checks_are_refused_with_the_reason() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let path = format!("/api/probes/{}/checks", probe.id);

    for (body, reason) in [
        (json!({ "name": "", "spec": { "kind": "cpu" } }), "name"),
        (
            json!({ "name": "x", "spec": { "kind": "ping", "host": "router.lan" } }),
            "IP address",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "http", "url": "example.com" } }),
            "http://",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "tcp", "host": "db", "port": 0 } }),
            "port",
        ),
    ] {
        let response = hub.post_json(&path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            response.text().contains(reason),
            "{body}: {}",
            response.text()
        );
    }
    let unknown_kind = json!({ "name": "x", "spec": { "kind": "snmp" } });
    let response = hub.post_json(&path, Some(&owner), unknown_kind).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    let nobody = hub
        .post_json("/api/probes/nobody/checks", Some(&owner), router_check())
        .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);
    assert!(checks_of(&hub, &probe).await.checks.is_empty());
}

#[tokio::test]
async fn viewers_see_checks_but_cant_change_them() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let viewer = hub.login_as(hub_lib::model::Role::Viewer).await;
    let path = format!("/api/probes/{}/checks", probe.id);

    assert_eq!(hub.get(&path, Some(&viewer)).await.status, StatusCode::OK);
    let refused = hub.post_json(&path, Some(&viewer), router_check()).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// The config in the hub's answer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_probe_gets_its_checks_in_the_answer_until_it_runs_them() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let router = add(&hub, &owner, &probe, router_check()).await;

    // A new probe runs nothing: it gets the config.
    let ack = report(&hub, &probe, None, None, vec![]).await;
    let config: ProbeConfig = ack.config.expect("the config is sent");
    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.interval_secs, 60);
    let ids: Vec<_> = config.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, [router.id.as_str()]);
    assert_eq!(
        config.checks[0].spec,
        CheckSpec::Ping {
            host: "192.168.1.1".into()
        }
    );
    assert_eq!(
        checks_of(&hub, &probe).await.config_revision,
        config.revision
    );

    // Running it: nothing more to send, and the hub says so.
    let ack = report(
        &hub,
        &probe,
        Some(&config.revision),
        None,
        vec![up(&router)],
    )
    .await;
    assert_eq!(ack.config, None);
    let state = checks_of(&hub, &probe).await;
    assert_eq!(state.sync, ConfigSync::InSync);
    assert_eq!(
        state.applied_revision.as_deref(),
        Some(config.revision.as_str())
    );

    // Its results are filed under the check.
    let devices: Vec<Value> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["check_id"], router.id.as_str());
    assert_eq!(devices[0]["name"], "Router");
}

#[tokio::test]
async fn any_change_sends_a_new_config() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let router = add(&hub, &owner, &probe, router_check()).await;
    let first = report(&hub, &probe, None, None, vec![])
        .await
        .config
        .unwrap();

    // Disabled: no longer in the config.
    let disabled = json!({ "name": "Router", "spec": router.spec, "enabled": false });
    hub.post_json(
        &format!("/api/checks/{}", router.id),
        Some(&owner),
        disabled,
    )
    .await;
    let second = report(&hub, &probe, Some(&first.revision), None, vec![])
        .await
        .config
        .expect("a new config");
    assert_ne!(second.revision, first.revision);
    assert!(second.checks.is_empty());
    assert_eq!(
        checks_of(&hub, &probe).await.sync,
        ConfigSync::Pending,
        "the probe still runs the first"
    );

    // A new interval.
    let interval = json!({ "name": "hq", "site_id": "hq-site", "interval_secs": 15 });
    let response = hub
        .post_json(&format!("/api/probes/{}", probe.id), Some(&owner), interval)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let third = report(&hub, &probe, Some(&second.revision), None, vec![])
        .await
        .config
        .expect("a new config");
    assert_eq!(third.interval_secs, 15);
}

#[tokio::test]
async fn a_refused_config_is_shown_and_not_sent_again() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    add(&hub, &owner, &probe, router_check()).await;
    let config = report(&hub, &probe, None, None, vec![])
        .await
        .config
        .unwrap();

    let refusal = ConfigRejection {
        revision: config.revision.clone(),
        error: "this probe can't ping".into(),
    };
    let ack = report(&hub, &probe, None, Some(refusal), vec![]).await;
    assert_eq!(ack.config, None, "sending it again would change nothing");
    let state = checks_of(&hub, &probe).await;
    assert_eq!(state.sync, ConfigSync::Rejected);
    assert_eq!(state.config_error.as_deref(), Some("this probe can't ping"));

    // A change makes a new config, which is sent.
    add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "CPU", "spec": { "kind": "cpu" } }),
    )
    .await;
    assert_eq!(checks_of(&hub, &probe).await.sync, ConfigSync::Pending);
    let refusal = ConfigRejection {
        revision: config.revision,
        error: "this probe can't ping".into(),
    };
    let ack = report(&hub, &probe, None, Some(refusal), vec![]).await;
    assert_eq!(ack.config.expect("the new one").checks.len(), 2);
}

#[tokio::test]
async fn a_probe_cant_report_on_another_probes_check() {
    let mine = TestProbe::new("mine");
    let theirs = TestProbe::new("theirs");
    let hub = TestHub::with_alerts(&[&mine, &theirs]).await;
    let owner = hub.login().await;
    let their_check = add(&hub, &owner, &theirs, router_check()).await;

    // `mine` claims `theirs`' router is down.
    let mut down = up(&their_check);
    down.status = Health::Down;
    report(&hub, &mine, None, None, vec![down]).await;

    let devices: Vec<Value> = hub
        .get_json(&format!("/api/probe/{}/devices", mine.id))
        .await;
    assert!(devices.is_empty(), "not filed under anyone's check");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM check_results")
        .fetch_one(hub.pool())
        .await
        .unwrap();
    assert_eq!(stored, 0);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(hub.notifier.subjects().is_empty(), "and no alert");
}

#[tokio::test]
async fn a_probes_checks_survive_enrolling_it_again() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    add(&hub, &owner, &probe, router_check()).await;

    // What `probe enroll` with a new code does to the record.
    let mut record = hub.store().probe_record(&probe.id).await.unwrap().unwrap();
    record.name = "renamed by the code".into();
    record.interval_secs = common::checks::DEFAULT_INTERVAL_SECS;
    hub.store().save_probe_record(&record).await.unwrap();

    assert_eq!(checks_of(&hub, &probe).await.checks.len(), 1);
}
