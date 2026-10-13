//! Checks as hub records (M2-04, D-49): managing them through the API, and
//! the config the hub sends a probe in its signed answer to a report.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::ReportAck;
use common::checks::{CheckResult, CheckSpec, ConfigRejection, Health, ProbeConfig};
use hub_lib::model::{AlertAfter, CheckInfo, ConfigSync, ProbeChecks};
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
    assert_eq!(router.alert_after, AlertAfter::default(), "2 in a row");
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
        "enabled": false, "fail_after": 3, "recover_after": null,
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
    assert_eq!(
        (
            changed.alert_after.fail_after,
            changed.alert_after.recover_after
        ),
        (Some(3), None)
    );
    let listed = hub
        .get(&format!("/api/probes/{}/checks", probe.id), Some(&owner))
        .await
        .json::<Value>();
    let gateway = &listed["checks"][1];
    assert_eq!(
        (&gateway["fail_after"], &gateway["recover_after"]),
        (&json!(3), &Value::Null)
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
            json!({ "name": "x", "spec": { "kind": "ping", "host": "router lan" } }),
            "IP address",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "dns", "name": "example.com", "server": "dns.lan" } }),
            "DNS server",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "http", "url": "example.com" } }),
            "http://",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "http", "url": "https://example.com", "status": "2xx" } }),
            "status code",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "http", "url": "https://example.com",
                "method": "head", "contains": "Welcome" } }),
            "HEAD",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "http", "url": "https://example.com",
                "headers": [{ "name": "X Y", "value": "1" }] } }),
            "header name",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "tcp", "host": "db", "port": 0 } }),
            "port",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "service", "name": "" } }),
            "service name",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "cpu" }, "fail_after": 0 }),
            "1 to 10 failed results",
        ),
        (
            json!({ "name": "x", "spec": { "kind": "cpu" }, "recover_after": 11 }),
            "1 to 10 good results",
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
    // A database check: an engine the hub knows, a host, a port or none.
    for (spec, status) in [
        (
            json!({ "kind": "database", "engine": "oracle", "host": "db.lan" }),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({ "kind": "database", "engine": "mysql", "host": "db.lan", "port": 0 }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "kind": "database", "engine": "mongodb", "host": "" }),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let body = json!({ "name": "x", "spec": spec });
        let response = hub.post_json(&path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, status, "{body}");
    }
    // An event log check: names as Windows has them, and sane numbers.
    for (spec, reason) in [
        (json!({ "kind": "eventlog", "log": "" }), "log"),
        (
            json!({ "kind": "eventlog", "log": "System", "source": "disk' or '1'='1" }),
            "names as Windows shows them",
        ),
        (
            json!({ "kind": "eventlog", "log": "System", "minutes": 0 }),
            "1-44640 minutes",
        ),
        (
            json!({ "kind": "eventlog", "log": "System", "count": 5000 }),
            "1-1000",
        ),
    ] {
        let body = json!({ "name": "x", "spec": spec });
        let response = hub.post_json(&path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
        assert!(response.text().contains(reason), "{}", response.text());
    }
    let unknown_level = json!({ "name": "x", "spec": {
        "kind": "eventlog", "log": "System", "level": "loud" } });
    let response = hub.post_json(&path, Some(&owner), unknown_level).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    // A process check: a program's name, and numbers that make sense.
    for (spec, reason) in [
        (json!({ "kind": "process", "name": "" }), "process name"),
        (
            json!({ "kind": "process", "name": "/usr/sbin/nginx" }),
            "not its path",
        ),
        (
            json!({ "kind": "process", "name": "nginx", "min": 3, "max": 2 }),
            "fewest not above",
        ),
    ] {
        let body = json!({ "name": "x", "spec": spec });
        let response = hub.post_json(&path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
        assert!(response.text().contains(reason), "{}", response.text());
    }
    // An interface check: a name as the system shows it.
    for spec in [
        json!({ "kind": "interface", "name": "" }),
        json!({ "kind": "interface", "name": "../eth0" }),
    ] {
        let body = json!({ "name": "x", "spec": spec });
        let response = hub.post_json(&path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            response.text().contains("interface name"),
            "{}",
            response.text()
        );
    }
    // An updates check: its limits are days, within a year.
    for spec in [
        json!({ "kind": "updates", "security_days": 366 }),
        json!({ "kind": "updates", "stale_days": 1000 }),
    ] {
        let body = json!({ "name": "x", "spec": spec });
        let response = hub.post_json(&path, Some(&owner), body.clone()).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            response.text().contains("0-365 days"),
            "{}",
            response.text()
        );
    }
    let unknown_kind = json!({ "name": "x", "spec": { "kind": "oracle" } });
    let response = hub.post_json(&path, Some(&owner), unknown_kind).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    let nobody = hub
        .post_json("/api/probes/nobody/checks", Some(&owner), router_check())
        .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);
    assert!(checks_of(&hub, &probe).await.checks.is_empty());

    // An http check's expectations are kept as sent (M5-02), and left out
    // where they are the default.
    let spec = json!({
        "kind": "http", "url": "https://example.com/login", "status": "200, 401",
        "contains": "Sign in", "follow_redirects": false, "verify": false,
        "headers": [{ "name": "Accept-Language", "value": "en" }],
    });
    let created = hub
        .post_json(
            &path,
            Some(&owner),
            json!({ "name": "Login page", "spec": spec }),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text());
    let created: Value = created.json();
    assert_eq!(created["spec"], spec);
    assert_eq!(created["target"], "https://example.com/login");
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
    assert_eq!(config.checks[0].spec, CheckSpec::ping("192.168.1.1"));
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
    let hub = TestHub::new(&[&mine, &theirs]).await;
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

#[tokio::test]
async fn a_new_probe_gets_copies_of_another_probes_checks() {
    let old = TestProbe::new("old");
    let new = TestProbe::new("new");
    let hub = TestHub::new(&[&old, &new]).await;
    let owner = hub.login().await;
    let router = add(&hub, &owner, &old, router_check()).await;
    let disabled =
        json!({ "name": "CPU", "spec": { "kind": "cpu" }, "enabled": false, "fail_after": 5 });
    add(&hub, &owner, &old, disabled).await;

    let path = format!("/api/probes/{}/checks/copy", new.id);
    let response = hub
        .post_json(&path, Some(&owner), json!({ "from_probe_id": old.id }))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let copies: Vec<CheckInfo> = response.json();

    assert_eq!(copies.len(), 2);
    let listed = checks_of(&hub, &new).await.checks;
    let summary: Vec<_> = listed
        .iter()
        .map(|c| (c.name.as_str(), c.enabled, c.probe_id.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("CPU", false, new.id.as_str()),
            ("Router", true, new.id.as_str())
        ]
    );
    assert!(listed.iter().all(|c| c.id != router.id), "new IDs");
    assert_eq!(
        listed[0].alert_after.fail_after,
        Some(5),
        "when it alerts too"
    );
    assert_eq!(
        checks_of(&hub, &old).await.checks.len(),
        2,
        "the source keeps its own"
    );

    let onto_itself = hub
        .post_json(&path, Some(&owner), json!({ "from_probe_id": new.id }))
        .await;
    assert_eq!(onto_itself.status, StatusCode::BAD_REQUEST);
    let entries: Vec<Value> = hub.get_json("/api/audit?action=check.copy").await;
    assert_eq!(entries.len(), 1);
}

/// M5-11, D-89: the probe reports what its machine waits for; the hub makes
/// a Warning of it by the check's own limits, and never Down.
#[tokio::test]
async fn waiting_updates_are_judged_by_the_checks_own_limits() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    // Without limits in the request, the usual ones; they are in the answer.
    let usual = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Updates", "spec": { "kind": "updates" } }),
    )
    .await;
    assert_eq!(usual.spec, CheckSpec::updates());
    assert_eq!(usual.target, "localhost");
    // A second one that waits a week for security updates, and says nothing
    // of restarts.
    let patient = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Updates, weekly", "spec": {
            "kind": "updates", "security_days": 7, "reboot_days": null, "stale_days": 30
        } }),
    )
    .await;
    assert_eq!(
        patient.spec,
        CheckSpec::Updates {
            security_days: Some(7),
            reboot_days: None,
            stale_days: Some(30),
        }
    );

    let result = |check: &CheckInfo, numbers: &[(&str, f64)]| CheckResult {
        check_id: common::checks::CheckId(check.id.clone()),
        status: Health::Up,
        message: "2 security updates wait; a restart is needed".into(),
        latency_ms: None,
        metrics: numbers
            .iter()
            .map(|(key, value)| Metric::new(key, *value, ""))
            .collect(),
    };
    let waiting = [
        (metric::UPDATES, 5.0),
        (metric::SECURITY, 2.0),
        (metric::SECURITY_DAYS, 3.0),
        (metric::REBOOT, 1.0),
        (metric::REBOOT_DAYS, 3.0),
        (metric::LOOKED_DAYS, 0.5),
    ];
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&usual, &waiting), result(&patient, &waiting)],
    )
    .await;
    let devices: Vec<hub_lib::model::ProbeDevice> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;
    let of = |id: &str| devices.iter().find(|d| d.check_id == id).expect("a device");
    assert_eq!(of(&usual.id).status, hub_lib::model::Status::Warning);
    assert_eq!(
        of(&usual.id).reason.as_deref(),
        Some("2 security updates wait; a restart is needed to finish updates")
    );
    // Three days is within a week, and restarts aren't its concern.
    assert_eq!(of(&patient.id).status, hub_lib::model::Status::Up);
    assert_eq!(of(&patient.id).reason, None);

    // The probe gets the check with its limits (it doesn't use them: the
    // hub judges), and nothing secret.
    let config = report(&hub, &probe, None, None, vec![])
        .await
        .config
        .expect("a config");
    assert_eq!(config.validate(), Ok(()));
    assert!(config.checks.iter().any(|c| c.spec == CheckSpec::updates()));
}

/// M5-13, D-90: a process check is the probe's to judge (enough run, or
/// not); the hub keeps what it says and has no thresholds for it.
#[tokio::test]
async fn a_process_check_is_kept_as_typed_and_judged_by_the_probe() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let plain = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "SQL Server", "spec": { "kind": "process", "name": "sqlservr" } }),
    )
    .await;
    assert_eq!(plain.spec, CheckSpec::process("sqlservr"));
    assert_eq!(plain.target, "sqlservr");
    let workers = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Billing", "spec": {
            "kind": "process", "name": "java", "command": "billing.jar", "min": 2, "max": 4
        } }),
    )
    .await;
    assert_eq!(workers.target, "java (billing.jar)");
    // It has no thresholds: limits on it are refused.
    let limited = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(&owner),
            json!({ "name": "x", "spec": { "kind": "process", "name": "nginx" },
                "limits": { "warning": 5 } }),
        )
        .await;
    assert_eq!(
        limited.status,
        StatusCode::BAD_REQUEST,
        "{}",
        limited.text()
    );

    let result = |check: &CheckInfo, status: Health, message: &str, count: f64| CheckResult {
        check_id: common::checks::CheckId(check.id.clone()),
        status,
        message: message.into(),
        latency_ms: None,
        metrics: vec![
            Metric::new(metric::PROCESSES, count, ""),
            Metric::new(metric::CPU, 12.5, metric::PERCENT),
            Metric::new(metric::MEMORY, 2048.0, metric::MEGABYTES),
        ],
    };
    report(
        &hub,
        &probe,
        None,
        None,
        vec![
            result(
                &plain,
                Health::Up,
                "sqlservr: 1 running, 12.5% CPU, 2048 MB",
                1.0,
            ),
            result(
                &workers,
                Health::Down,
                "java (billing.jar): 1 running, fewer than the 2 expected",
                1.0,
            ),
        ],
    )
    .await;
    let devices: Vec<hub_lib::model::ProbeDevice> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;
    let of = |id: &str| devices.iter().find(|d| d.check_id == id).expect("a device");
    assert_eq!(of(&plain.id).status, hub_lib::model::Status::Up);
    assert_eq!(of(&workers.id).status, hub_lib::model::Status::Down);
    assert_eq!(
        of(&workers.id).message.as_deref(),
        Some("java (billing.jar): 1 running, fewer than the 2 expected")
    );

    // The probe gets both as typed.
    let config = report(&hub, &probe, None, None, vec![])
        .await
        .config
        .expect("a config");
    assert_eq!(config.validate(), Ok(()));
    assert!(config.checks.iter().any(|c| c.spec
        == CheckSpec::Process {
            name: "java".into(),
            command: "billing.jar".into(),
            min: 2,
            max: Some(4),
        }));
}

/// M5-16, D-93: a failing drive is the probe's Down; a drive's warning
/// sign is the hub's Warning, in the probe's words.
#[tokio::test]
async fn a_smart_check_warns_on_a_drives_warning_sign() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let smart = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Disk health", "spec": { "kind": "smart", "ignore": ["WDC WD40*"] } }),
    )
    .await;
    assert_eq!(
        smart.spec,
        CheckSpec::Smart {
            ignore: vec!["WDC WD40*".into()]
        }
    );
    assert_eq!(smart.target, "disk health but WDC WD40*");
    // It has no thresholds: limits on it are refused.
    let limited = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(&owner),
            json!({ "name": "x", "spec": { "kind": "smart" }, "limits": { "warning": 5 } }),
        )
        .await;
    assert_eq!(
        limited.status,
        StatusCode::BAD_REQUEST,
        "{}",
        limited.text()
    );

    let result = |status: Health, message: &str, failing: f64, warnings: f64| CheckResult {
        check_id: common::checks::CheckId(smart.id.clone()),
        status,
        message: message.into(),
        latency_ms: None,
        metrics: vec![
            Metric::new(metric::FAILING, failing, ""),
            Metric::new(metric::WARNINGS, warnings, ""),
            Metric::new(metric::DRIVES, 3.0, ""),
            Metric::new(metric::TEMPERATURE, 41.0, metric::CELSIUS),
        ],
    };
    let shown = || {
        let hub = &hub;
        let path = format!("/api/probe/{}/devices", probe.id);
        let id = smart.id.clone();
        async move {
            let devices: Vec<hub_lib::model::ProbeDevice> = hub.get_json(&path).await;
            let device = devices
                .into_iter()
                .find(|d| d.check_id == id)
                .expect("a device");
            (device.status, device.message)
        }
    };

    let healthy = "3 disks healthy, the hottest at 41 °C";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(Health::Up, healthy, 0.0, 0.0)],
    )
    .await;
    assert_eq!(
        shown().await,
        (hub_lib::model::Status::Up, Some(healthy.to_string()))
    );

    let sectors = "ST4000DM004 (serial ZFN0): 8 bad sectors; 2 other disks healthy";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(Health::Up, sectors, 0.0, 1.0)],
    )
    .await;
    assert_eq!(
        shown().await,
        (hub_lib::model::Status::Warning, Some(sectors.to_string()))
    );

    let failing = "ST3000DM007 (serial W1F2): the drive says it is failing (SMART)";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(Health::Down, failing, 1.0, 1.0)],
    )
    .await;
    assert_eq!(
        shown().await,
        (hub_lib::model::Status::Down, Some(failing.to_string()))
    );
}

/// M5-15, D-92: an interface check is the probe's to judge (its link is up
/// or it isn't); the hub keeps what it says and has no thresholds for it.
#[tokio::test]
async fn an_interface_check_is_kept_as_typed_and_judged_by_the_probe() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let add_interface = |name: &'static str| {
        let body = json!({ "name": name, "spec": { "kind": "interface", "name": name } });
        add(&hub, &owner, &probe, body)
    };
    let (lan, wifi) = (
        add_interface("Ethernet").await,
        add_interface("Wi-Fi").await,
    );
    assert_eq!(
        lan.spec,
        CheckSpec::Interface {
            name: "Ethernet".into()
        }
    );
    assert_eq!(lan.target, "Ethernet");
    // It has no thresholds: limits on it are refused.
    let limited = hub
        .post_json(
            &format!("/api/probes/{}/checks", probe.id),
            Some(&owner),
            json!({ "name": "x", "spec": { "kind": "interface", "name": "eth0" },
                "limits": { "warning": 5 } }),
        )
        .await;
    assert_eq!(
        limited.status,
        StatusCode::BAD_REQUEST,
        "{}",
        limited.text()
    );

    let megabits = |key: &str, value: f64| Metric::new(key, value, metric::MEGABITS_PER_SECOND);
    let up = CheckResult {
        check_id: common::checks::CheckId(lan.id.clone()),
        status: Health::Up,
        message: "Ethernet: up at 1 Gbit/s, in 2.4 Mbit/s, out 200 kbit/s".into(),
        latency_ms: None,
        metrics: vec![
            megabits(metric::RECEIVED, 2.4),
            megabits(metric::SENT, 0.2),
            Metric::new(metric::ERRORS, 0.0, ""),
            megabits(metric::SPEED, 1000.0),
        ],
    };
    let down = CheckResult {
        check_id: common::checks::CheckId(wifi.id.clone()),
        status: Health::Down,
        message: "Wi-Fi: the link is down".into(),
        latency_ms: None,
        metrics: vec![],
    };
    report(&hub, &probe, None, None, vec![up, down]).await;
    let devices: Vec<hub_lib::model::ProbeDevice> = hub
        .get_json(&format!("/api/probe/{}/devices", probe.id))
        .await;
    let of = |id: &str| devices.iter().find(|d| d.check_id == id).expect("a device");
    assert_eq!(of(&lan.id).status, hub_lib::model::Status::Up);
    assert_eq!(
        of(&lan.id).message.as_deref(),
        Some("Ethernet: up at 1 Gbit/s, in 2.4 Mbit/s, out 200 kbit/s")
    );
    assert_eq!(of(&wifi.id).status, hub_lib::model::Status::Down);
    assert_eq!(
        of(&wifi.id).message.as_deref(),
        Some("Wi-Fi: the link is down")
    );

    // The probe gets both as typed.
    let config = report(&hub, &probe, None, None, vec![])
        .await
        .config
        .expect("a config");
    assert_eq!(config.validate(), Ok(()));
    for name in ["Ethernet", "Wi-Fi"] {
        let spec = CheckSpec::Interface { name: name.into() };
        assert!(config.checks.iter().any(|c| c.spec == spec), "{name}");
    }
}

/// M5-14, D-91: the probe counts the events and says which was the latest;
/// the hub says whether that is trouble, as the check asks.
#[tokio::test]
async fn events_are_judged_as_the_check_asks() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    // Disk errors: a Warning. A backup's success: must be there, else Down.
    let disk = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Disk errors", "spec": {
            "kind": "eventlog", "log": "System", "source": "disk", "ids": [7, 51, 153],
            "level": "warning", "minutes": 1440, "warning": true
        } }),
    )
    .await;
    assert_eq!(disk.target, "System (disk)");
    let backup = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Backup ran", "spec": {
            "kind": "eventlog", "log": "Microsoft-Windows-Backup", "ids": [4],
            "level": "information", "minutes": 1560, "wanted": true
        } }),
    )
    .await;

    let result = |check: &CheckInfo, events: f64, message: &str| CheckResult {
        check_id: common::checks::CheckId(check.id.clone()),
        status: Health::Up,
        message: message.into(),
        latency_ms: None,
        metrics: vec![Metric::new(metric::EVENTS, events, "")],
    };
    let devices = || async {
        let devices: Vec<hub_lib::model::ProbeDevice> = hub
            .get_json(&format!("/api/probe/{}/devices", probe.id))
            .await;
        devices
    };
    let found = "2 events in the last 1 day; the latest: disk 51 at 2026-10-12 14:02 UTC: An error was detected";
    let none = "No such event in the last 26 hours";
    let ran = "1 event in the last 26 hours; the latest: Microsoft-Windows-Backup 4 at 2026-10-12 02:30 UTC: The backup operation has finished successfully.";

    // A good night: no disk errors, and the backup's event is there.
    report(
        &hub,
        &probe,
        None,
        None,
        vec![
            result(&disk, 0.0, "No such event in the last 1 day"),
            result(&backup, 1.0, ran),
        ],
    )
    .await;
    for device in devices().await {
        assert_eq!(device.status, hub_lib::model::Status::Up, "{}", device.name);
        assert_eq!(device.reason, None);
    }

    // A bad one: the disk complains, and no backup ran.
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&disk, 2.0, found), result(&backup, 0.0, none)],
    )
    .await;
    let after = devices().await;
    let of = |id: &str| after.iter().find(|d| d.check_id == id).expect("a device");
    assert_eq!(of(&disk.id).status, hub_lib::model::Status::Warning);
    assert_eq!(of(&disk.id).reason.as_deref(), Some(found));
    assert_eq!(of(&backup.id).status, hub_lib::model::Status::Down);
    assert_eq!(of(&backup.id).reason.as_deref(), Some(none));
}

/// M5-16b, D-94: a temperature check is judged by limits in degrees, the
/// built-in ones or its own, and the reason names the sensor.
#[tokio::test]
async fn a_temperature_check_is_judged_by_its_limits_in_degrees() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let all = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Temperature", "spec": { "kind": "temperature" } }),
    )
    .await;
    assert_eq!(all.spec, CheckSpec::Temperature { sensors: vec![] });
    assert_eq!(all.target, "hottest sensor");
    let drives = add(
        &hub,
        &owner,
        &probe,
        json!({
            "name": "Drive temperature",
            "spec": { "kind": "temperature", "sensors": ["nvme*", "drivetemp*"] },
            "limits": { "warning": 55, "down": 70 },
        }),
    )
    .await;
    assert_eq!(drives.target, "hottest of nvme*, drivetemp*");
    // Degrees, 0 to 150; a name can't be blank.
    for body in [
        json!({ "name": "x", "spec": { "kind": "temperature" }, "limits": { "warning": 200 } }),
        json!({ "name": "x", "spec": { "kind": "temperature", "sensors": [" "] } }),
    ] {
        let refused = hub
            .post_json(
                &format!("/api/probes/{}/checks", probe.id),
                Some(&owner),
                body,
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{}",
            refused.text()
        );
    }

    let result = |id: &str, message: &str, celsius: f64| CheckResult {
        check_id: common::checks::CheckId(id.to_string()),
        status: Health::Up,
        message: message.into(),
        latency_ms: None,
        metrics: vec![
            Metric::new(metric::TEMPERATURE, celsius, metric::CELSIUS),
            Metric::new(metric::SENSORS, 2.0, ""),
        ],
    };
    let shown = |id: &str| {
        let hub = &hub;
        let path = format!("/api/probe/{}/devices", probe.id);
        let id = id.to_string();
        async move {
            let devices: Vec<hub_lib::model::ProbeDevice> = hub.get_json(&path).await;
            let device = devices
                .into_iter()
                .find(|d| d.check_id == id)
                .expect("a device");
            (device.status, device.reason.unwrap_or_default())
        }
    };

    let cpu = "coretemp Package id 0 is the hottest of 2 sensors, at 73 °C";
    let drive = "nvme Composite is the hottest of 2 sensors, at 58 °C";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&all.id, cpu, 73.0), result(&drives.id, drive, 58.0)],
    )
    .await;
    // Fine by the built-in 90 °C; over the drives' own 55 °C.
    assert_eq!(
        shown(&all.id).await,
        (hub_lib::model::Status::Up, String::new())
    );
    let (status, reason) = shown(&drives.id).await;
    assert_eq!(status, hub_lib::model::Status::Warning);
    assert_eq!(
        reason,
        format!("temperature 58 °C is over the warning limit (55 °C, from the check): {drive}")
    );

    let hot = "coretemp Package id 0 is the hottest of 2 sensors, at 94 °C";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&all.id, hot, 94.0), result(&drives.id, drive, 71.0)],
    )
    .await;
    let (status, reason) = shown(&all.id).await;
    assert_eq!(status, hub_lib::model::Status::Warning);
    assert!(
        reason.starts_with(
            "temperature 94 °C is over the warning limit (90 °C, from the built-in default): "
        ),
        "{reason}"
    );
    assert_eq!(shown(&drives.id).await.0, hub_lib::model::Status::Down);
}

/// M5-16c, D-95: a mail check is kept as typed, and judged by its latency
/// and by the days its certificate has left.
#[tokio::test]
async fn a_mail_check_is_judged_by_its_certificates_days_too() {
    use common::checks::{MailProtocol, MailSecurity, Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let smtp = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Mail", "spec": { "kind": "mail", "protocol": "smtp", "host": "mail.example.com" } }),
    )
    .await;
    assert_eq!(
        smtp.spec,
        CheckSpec::Mail {
            protocol: MailProtocol::Smtp,
            host: "mail.example.com".into(),
            port: None,
            security: MailSecurity::Starttls,
            verify: true,
        }
    );
    assert_eq!(smtp.target, "mail.example.com:25 (SMTP)");
    let imap = add(
        &hub,
        &owner,
        &probe,
        json!({
            "name": "Mailboxes",
            "spec": {
                "kind": "mail", "protocol": "imap", "host": "mail.example.com",
                "security": "tls", "verify": false,
            },
            "limits": { "warning": 500 },
        }),
    )
    .await;
    assert_eq!(imap.target, "mail.example.com:993 (IMAP)");
    for spec in [
        json!({ "kind": "mail", "protocol": "x400", "host": "mail.example.com" }),
        json!({ "kind": "mail", "protocol": "smtp", "host": "smtp://mail" }),
        json!({ "kind": "mail", "protocol": "smtp", "host": "mail", "port": 0 }),
        json!({ "kind": "mail", "protocol": "smtp", "host": "mail", "security": "ssl" }),
    ] {
        let refused = hub
            .post_json(
                &format!("/api/probes/{}/checks", probe.id),
                Some(&owner),
                json!({ "name": "x", "spec": spec }),
            )
            .await;
        assert!(
            refused.status.is_client_error(),
            "{}: {}",
            refused.status,
            refused.text()
        );
    }

    let result = |id: &str, latency_ms: u64, days: f64| CheckResult {
        check_id: common::checks::CheckId(id.to_string()),
        status: Health::Up,
        message: "ready".into(),
        latency_ms: Some(latency_ms),
        metrics: vec![Metric::new(metric::DAYS_LEFT, days, metric::DAYS)],
    };
    let shown = |id: &str| {
        let hub = &hub;
        let path = format!("/api/probe/{}/devices", probe.id);
        let id = id.to_string();
        async move {
            let devices: Vec<hub_lib::model::ProbeDevice> = hub.get_json(&path).await;
            let device = devices
                .into_iter()
                .find(|d| d.check_id == id)
                .expect("a device");
            (device.status, device.reason.unwrap_or_default())
        }
    };

    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&smtp.id, 120, 60.0), result(&imap.id, 800, 60.0)],
    )
    .await;
    assert_eq!(
        shown(&smtp.id).await,
        (hub_lib::model::Status::Up, String::new())
    );
    let (status, reason) = shown(&imap.id).await;
    assert_eq!(status, hub_lib::model::Status::Warning);
    assert!(reason.starts_with("latency 800 ms is over"), "{reason}");

    // The certificate runs out: a Warning at 21 days, Down at 7.
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&smtp.id, 120, 12.0), result(&imap.id, 120, 3.0)],
    )
    .await;
    assert_eq!(
        shown(&smtp.id).await,
        (
            hub_lib::model::Status::Warning,
            "certificate expiry in 12 days is under the warning limit (21 days, from the \
             built-in default)"
                .to_string()
        )
    );
    assert_eq!(shown(&imap.id).await.0, hub_lib::model::Status::Down);
}

/// M5-16d, D-96: an ntp check is judged by how far the clock is off, in
/// seconds, and the reason says which way and from which server.
#[tokio::test]
async fn an_ntp_check_is_judged_by_the_clocks_offset() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let pool = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "Clock", "spec": { "kind": "ntp", "host": "pool.ntp.org" } }),
    )
    .await;
    assert_eq!(
        pool.spec,
        CheckSpec::Ntp {
            host: "pool.ntp.org".into(),
            port: None,
        }
    );
    assert_eq!(pool.target, "pool.ntp.org");
    let strict = add(
        &hub,
        &owner,
        &probe,
        json!({
            "name": "Domain clock",
            "spec": { "kind": "ntp", "host": "dc01.lan", "port": 1123 },
            "limits": { "warning": 0.5, "down": "off" },
        }),
    )
    .await;
    assert_eq!(strict.target, "dc01.lan:1123");
    for body in [
        json!({ "name": "x", "spec": { "kind": "ntp", "host": "" } }),
        json!({ "name": "x", "spec": { "kind": "ntp", "host": "pool.ntp.org", "port": 0 } }),
        // A limit is a day at most.
        json!({ "name": "x", "spec": { "kind": "ntp", "host": "pool.ntp.org" }, "limits": { "warning": 100000 } }),
    ] {
        let refused = hub
            .post_json(
                &format!("/api/probes/{}/checks", probe.id),
                Some(&owner),
                body,
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{}",
            refused.text()
        );
    }

    let result = |id: &str, message: &str, seconds: f64| CheckResult {
        check_id: common::checks::CheckId(id.to_string()),
        status: Health::Up,
        message: message.into(),
        latency_ms: Some(18),
        metrics: vec![
            Metric::new(metric::OFFSET, seconds, metric::SECONDS),
            Metric::new(metric::STRATUM, 2.0, ""),
        ],
    };
    let shown = |id: &str| {
        let hub = &hub;
        let path = format!("/api/probe/{}/devices", probe.id);
        let id = id.to_string();
        async move {
            let devices: Vec<hub_lib::model::ProbeDevice> = hub.get_json(&path).await;
            let device = devices
                .into_iter()
                .find(|d| d.check_id == id)
                .expect("a device");
            (device.status, device.reason.unwrap_or_default())
        }
    };

    let fine = "This machine's clock is 700 ms ahead of pool.ntp.org (stratum 2)";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(&pool.id, fine, 0.7), result(&strict.id, fine, 0.7)],
    )
    .await;
    assert_eq!(
        shown(&pool.id).await,
        (hub_lib::model::Status::Up, String::new())
    );
    assert_eq!(
        shown(&strict.id).await,
        (
            hub_lib::model::Status::Warning,
            format!("clock offset 0.7 s is over the warning limit (0.5 s, from the check): {fine}")
        )
    );

    let off = "This machine's clock is 2 h 5 min behind pool.ntp.org (stratum 2)";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![
            result(&pool.id, off, 7500.0),
            result(&strict.id, off, 7500.0),
        ],
    )
    .await;
    assert_eq!(
        shown(&pool.id).await,
        (
            hub_lib::model::Status::Down,
            format!(
                "clock offset 7500 s is over the down limit (60 s, from the built-in default): {off}"
            )
        )
    );
    // Its Down limit is off: a Warning however far.
    assert_eq!(shown(&strict.id).await.0, hub_lib::model::Status::Warning);
}

/// M5-16e, D-97: a UPS on battery is the hub's Warning, in the probe's
/// words; its own low-battery alarm is the probe's Down.
#[tokio::test]
async fn a_nut_check_warns_while_the_ups_is_on_battery() {
    use common::checks::{Metric, metric};
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let ups = add(
        &hub,
        &owner,
        &probe,
        json!({ "name": "UPS", "spec": { "kind": "nut", "host": "nas.lan", "ups": "rack" } }),
    )
    .await;
    assert_eq!(
        ups.spec,
        CheckSpec::Nut {
            host: "nas.lan".into(),
            port: None,
            ups: "rack".into(),
        }
    );
    assert_eq!(ups.target, "rack@nas.lan");
    for body in [
        json!({ "name": "x", "spec": { "kind": "nut", "host": "nas.lan", "ups": "my ups" } }),
        json!({ "name": "x", "spec": { "kind": "nut", "host": "" } }),
        // It has no thresholds: limits on it are refused.
        json!({ "name": "x", "spec": { "kind": "nut", "host": "nas.lan" }, "limits": { "warning": 50 } }),
    ] {
        let refused = hub
            .post_json(
                &format!("/api/probes/{}/checks", probe.id),
                Some(&owner),
                body,
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{}",
            refused.text()
        );
    }

    let result = |status: Health, message: &str, warnings: f64| CheckResult {
        check_id: common::checks::CheckId(ups.id.clone()),
        status,
        message: message.into(),
        latency_ms: Some(12),
        metrics: vec![
            Metric::new(metric::CHARGE, 85.0, metric::PERCENT),
            Metric::new(metric::RUNTIME, 31.2, metric::MINUTES),
            Metric::new(metric::LOAD, 23.0, metric::PERCENT),
            Metric::new(metric::WARNINGS, warnings, ""),
        ],
    };
    let shown = || {
        let hub = &hub;
        let path = format!("/api/probe/{}/devices", probe.id);
        let id = ups.id.clone();
        async move {
            let devices: Vec<hub_lib::model::ProbeDevice> = hub.get_json(&path).await;
            let device = devices
                .into_iter()
                .find(|d| d.check_id == id)
                .expect("a device");
            (device.status, device.message)
        }
    };

    let online = "Back-UPS ES 700G: on mains power, battery 100 %, 42 min of runtime, load 23 %";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(Health::Up, online, 0.0)],
    )
    .await;
    assert_eq!(
        shown().await,
        (hub_lib::model::Status::Up, Some(online.to_string()))
    );

    let outage = "Back-UPS ES 700G: on battery, battery 85 %, 31 min of runtime, load 23 %";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(Health::Up, outage, 1.0)],
    )
    .await;
    assert_eq!(
        shown().await,
        (hub_lib::model::Status::Warning, Some(outage.to_string()))
    );

    let low = "Back-UPS ES 700G: on battery, battery low, battery 9 %, 2 min of runtime, load 23 %";
    report(
        &hub,
        &probe,
        None,
        None,
        vec![result(Health::Down, low, 1.0)],
    )
    .await;
    assert_eq!(
        shown().await,
        (hub_lib::model::Status::Down, Some(low.to_string()))
    );
}
