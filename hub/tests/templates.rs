//! The probe's machine, suggested checks and check templates (M5-12, D-85)
//! through the API.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::checks::CheckSpec;
use common::inventory::{Interface, Inventory, Os, OsFamily, Service, Volume};
use hub_lib::model::{AddedChecks, MachineInfo, ProbeChecks, Role, TemplateInfo};
use serde_json::{Value, json};
use support::*;

fn linux_machine() -> Inventory {
    Inventory {
        os: Os {
            family: OsFamily::Linux,
            name: "Ubuntu 26.04".into(),
            version: "6.14.0".into(),
            arch: "x86_64".into(),
        },
        volumes: vec![Volume {
            mount: "/".into(),
            file_system: "ext4".into(),
            total_mb: 50_000,
        }],
        services: vec![
            Service {
                name: "nginx.service".into(),
                label: Some("A high performance web server".into()),
            },
            Service {
                name: "systemd-journald.service".into(),
                label: None,
            },
        ],
        interfaces: vec![Interface {
            name: "eth0".into(),
            addresses: vec!["192.168.1.20".into()],
        }],
        gateway: Some("192.168.1.1".into()),
    }
}

/// A report from `probe`, a minute later, carrying `inventory`.
async fn report_with(hub: &TestHub, probe: &TestProbe, inventory: Option<Inventory>) {
    report_from(hub, probe, "acme", inventory).await;
}

/// The same from a probe of `company` (the harness files a probe under the
/// company its report names).
async fn report_from(
    hub: &TestHub,
    probe: &TestProbe,
    company: &str,
    inventory: Option<Inventory>,
) {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report(company, hub.now(), vec![]);
    report.inventory = inventory;
    probe.sign(&mut report);
    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

async fn machine_of(hub: &TestHub, probe: &TestProbe) -> MachineInfo {
    hub.get_json(&format!("/api/probes/{}/machine", probe.id))
        .await
}

async fn checks_of(hub: &TestHub, probe: &TestProbe) -> ProbeChecks {
    hub.get_json(&format!("/api/probes/{}/checks", probe.id))
        .await
}

fn names(checks: &ProbeChecks) -> Vec<&str> {
    checks.checks.iter().map(|c| c.name.as_str()).collect()
}

/// The probe says what its machine is made of; the hub keeps the latest,
/// shows it, and suggests checks from it. A report without one changes
/// nothing.
#[tokio::test]
async fn a_probes_inventory_is_kept_and_suggests_checks() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;

    report_with(&hub, &probe, None).await;
    let before = machine_of(&hub, &probe).await;
    assert_eq!(before, MachineInfo::default(), "nothing known yet");

    report_with(&hub, &probe, Some(linux_machine())).await;
    let reported_at = hub.now();
    report_with(&hub, &probe, None).await;

    let machine = machine_of(&hub, &probe).await;
    assert_eq!(machine.inventory, Some(linux_machine()));
    assert_eq!(
        machine.reported_at.as_deref(),
        Some(
            reported_at
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
                .as_str()
        )
    );
    let suggested: Vec<&str> = machine
        .suggestions
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        suggested,
        ["CPU", "Memory", "Disks", "Uptime", "Gateway", "nginx"]
    );
}

/// What the probe sent is kept within bounds, whatever it sent (FP-3).
#[tokio::test]
async fn an_oversized_inventory_is_cut_to_size() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let mut huge = linux_machine();
    huge.gateway = Some("somewhere".into());
    huge.services = (0..3000)
        .map(|i| Service {
            name: format!("svc{i}.service"),
            label: None,
        })
        .collect();
    report_with(&hub, &probe, Some(huge)).await;

    let kept = machine_of(&hub, &probe).await.inventory.unwrap();
    assert_eq!(kept.services.len(), common::inventory::MAX_SERVICES);
    assert_eq!(kept.gateway, None, "not an address");
}

/// Accepting suggestions adds them in one request; accepting them again
/// adds nothing, and they are no longer suggested.
#[tokio::test]
async fn suggested_checks_are_added_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report_with(&hub, &probe, Some(linux_machine())).await;

    let suggestions = machine_of(&hub, &probe).await.suggestions;
    let body = json!({
        "checks": suggestions.iter().map(|s| json!({ "name": s.name, "spec": s.spec })).collect::<Vec<_>>()
    });
    let path = format!("/api/probes/{}/checks/batch", probe.id);
    let response = hub.post_json(&path, Some(&owner), body.clone()).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let added: AddedChecks = response.json();
    assert_eq!((added.added.len(), added.skipped), (6, 0));

    let checks = checks_of(&hub, &probe).await;
    assert_eq!(
        names(&checks),
        ["CPU", "Disks", "Gateway", "Memory", "nginx", "Uptime"]
    );
    assert!(machine_of(&hub, &probe).await.suggestions.is_empty());

    let again: AddedChecks = hub.post_json(&path, Some(&owner), body).await.json();
    assert_eq!((again.added.len(), again.skipped), (0, 6));
    assert_eq!(checks_of(&hub, &probe).await.checks.len(), 6);

    // One wrong check: none is added.
    let wrong = json!({ "checks": [
        { "name": "Fine", "spec": { "kind": "tcp", "host": "10.0.0.5", "port": 22 } },
        { "name": "Wrong", "spec": { "kind": "tcp", "host": "", "port": 22 } },
    ]});
    let refused = hub.post_json(&path, Some(&owner), wrong).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.text().contains("Wrong: "), "{}", refused.text());
    assert_eq!(checks_of(&hub, &probe).await.checks.len(), 6);

    let entries: Vec<Value> = hub.get_json("/api/audit?action=check.batch").await;
    assert_eq!(entries.len(), 2);
}

async fn templates(hub: &TestHub) -> Vec<TemplateInfo> {
    hub.get_json("/api/check-templates").await
}

/// A built-in template asks for its value and makes its checks from it.
#[tokio::test]
async fn a_built_in_template_adds_its_checks_with_the_value_given() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;

    let all = templates(&hub).await;
    let website = all.iter().find(|t| t.name == "Website").expect("built in");
    assert!(website.builtin);
    assert_eq!(website.parameters[0].key, "host");

    let path = format!("/api/probes/{}/checks/template", probe.id);
    let missing = hub
        .post_json(&path, Some(&owner), json!({ "template_id": website.id }))
        .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert!(
        missing.text().contains("Host name is needed"),
        "{}",
        missing.text()
    );

    let body = json!({ "template_id": website.id, "values": { "host": "shop.example.com" } });
    let response = hub.post_json(&path, Some(&owner), body.clone()).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let added: AddedChecks = response.json();
    assert_eq!(added.added.len(), 3);
    let checks = checks_of(&hub, &probe).await;
    assert_eq!(
        names(&checks),
        [
            "shop.example.com certificate",
            "shop.example.com DNS",
            "shop.example.com page"
        ]
    );
    assert_eq!(
        checks.checks[2].spec,
        CheckSpec::http("https://shop.example.com")
    );

    // Applied twice: nothing doubles.
    let again: AddedChecks = hub.post_json(&path, Some(&owner), body).await.json();
    assert_eq!((again.added.len(), again.skipped), (0, 3));

    let unknown = hub
        .post_json(
            &path,
            Some(&owner),
            json!({ "template_id": "builtin:nope" }),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let entries: Vec<Value> = hub.get_json("/api/audit?action=check.template").await;
    assert_eq!(entries.len(), 2);
}

/// A probe's checks saved as a template, applied to another probe, deleted.
#[tokio::test]
async fn a_probes_checks_are_saved_as_a_template_and_applied_elsewhere() {
    let (first, second) = (TestProbe::new("first"), TestProbe::new("second"));
    let hub = TestHub::new(&[&first, &second]).await;
    let owner = hub.login().await;
    hub.add_check(&first, "Router", CheckSpec::ping("192.168.1.1"))
        .await;
    hub.add_check(&first, "CPU", CheckSpec::Cpu).await;

    let request = json!({ "name": "Branch office", "description": "Router and CPU", "from_probe_id": first.id });
    let response = hub
        .post_json("/api/check-templates", Some(&owner), request.clone())
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let saved: TemplateInfo = response.json();
    assert!(!saved.builtin);
    assert_eq!(saved.checks.len(), 2);
    assert_eq!(templates(&hub).await.last().unwrap().name, "Branch office");

    // A name is taken once, whatever its case; a built-in's name too.
    let mut same = request.clone();
    same["name"] = json!("branch OFFICE");
    let taken = hub
        .post_json("/api/check-templates", Some(&owner), same)
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    let mut builtin = request.clone();
    builtin["name"] = json!("website");
    let taken = hub
        .post_json("/api/check-templates", Some(&owner), builtin)
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    // A probe without checks has nothing to save.
    let mut empty = request.clone();
    empty["name"] = json!("Empty");
    empty["from_probe_id"] = json!(second.id);
    let nothing = hub
        .post_json("/api/check-templates", Some(&owner), empty)
        .await;
    assert_eq!(nothing.status, StatusCode::BAD_REQUEST);

    let path = format!("/api/probes/{}/checks/template", second.id);
    let applied: AddedChecks = hub
        .post_json(&path, Some(&owner), json!({ "template_id": saved.id }))
        .await
        .json();
    assert_eq!(applied.added.len(), 2);
    assert_eq!(names(&checks_of(&hub, &second).await), ["CPU", "Router"]);

    let delete = format!("/api/check-templates/{}/delete", saved.id);
    let deleted = hub.post_json(&delete, Some(&owner), json!({})).await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert!(templates(&hub).await.iter().all(|t| t.builtin));
    // The checks made from it stay.
    assert_eq!(checks_of(&hub, &second).await.checks.len(), 2);
    let gone = hub.post_json(&delete, Some(&owner), json!({})).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let builtin = hub
        .post_json(
            "/api/check-templates/builtin:machine/delete",
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(builtin.status, StatusCode::BAD_REQUEST);

    for action in ["template.create", "template.delete"] {
        let entries: Vec<Value> = hub.get_json(&format!("/api/audit?action={action}")).await;
        assert_eq!(entries.len(), 1, "{action}");
    }
}

/// The machine of a probe is shown to those who see the probe: a user
/// limited to some companies doesn't reach another company's through it.
#[tokio::test]
async fn a_probes_machine_is_shown_within_the_users_companies() {
    let (mine, theirs) = (TestProbe::new("mine"), TestProbe::new("theirs"));
    let hub = TestHub::new(&[&mine, &theirs]).await;
    let scoped = hub.login_scoped("sam", Role::Technician, &["acme"]).await;
    report_with(&hub, &mine, Some(linux_machine())).await;
    report_from(&hub, &theirs, "globex", Some(linux_machine())).await;

    let own = hub
        .get(&format!("/api/probes/{}/machine", mine.id), Some(&scoped))
        .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text());
    assert!(own.json::<MachineInfo>().inventory.is_some());
    let other = hub
        .get(&format!("/api/probes/{}/machine", theirs.id), Some(&scoped))
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
}

/// A saved template shows what a probe checks, and templates are hub-wide:
/// a technician limited to some companies has the built-in ones only, for
/// their own probes, and never reaches another company's probe through one.
#[tokio::test]
async fn a_scoped_user_applies_built_in_templates_only() {
    let (mine, theirs) = (TestProbe::new("mine"), TestProbe::new("theirs"));
    let hub = TestHub::new(&[&mine, &theirs]).await;
    let owner = hub.login().await;
    let scoped = hub.login_scoped("sam", Role::Technician, &["acme"]).await;
    hub.add_check(&mine, "CPU", CheckSpec::Cpu).await;
    report_from(&hub, &theirs, "globex", Some(linux_machine())).await;

    let save = json!({ "name": "Mine", "from_probe_id": mine.id });
    let refused = hub
        .post_json("/api/check-templates", Some(&scoped), save.clone())
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text());
    let saved: TemplateInfo = hub
        .post_json("/api/check-templates", Some(&owner), save)
        .await
        .json();
    let delete = format!("/api/check-templates/{}/delete", saved.id);
    let refused = hub.post_json(&delete, Some(&scoped), json!({})).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    // Nor does it see a saved one, or apply it.
    let listed: Vec<TemplateInfo> = hub.get("/api/check-templates", Some(&scoped)).await.json();
    assert!(!listed.is_empty() && listed.iter().all(|t| t.builtin));
    let apply_saved = hub
        .post_json(
            &format!("/api/probes/{}/checks/template", mine.id),
            Some(&scoped),
            json!({ "template_id": saved.id }),
        )
        .await;
    assert_eq!(apply_saved.status, StatusCode::NOT_FOUND);

    let body = json!({ "template_id": "builtin:machine" });
    let own = hub
        .post_json(
            &format!("/api/probes/{}/checks/template", mine.id),
            Some(&scoped),
            body.clone(),
        )
        .await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text());
    let added: AddedChecks = own.json();
    assert_eq!(
        (added.added.len(), added.skipped),
        (3, 1),
        "it had a CPU check"
    );

    let batch = json!({ "checks": [{ "name": "CPU", "spec": { "kind": "cpu" } }] });
    for (path, body) in [
        (format!("/api/probes/{}/checks/template", theirs.id), body),
        (format!("/api/probes/{}/checks/batch", theirs.id), batch),
    ] {
        let other = hub.post_json(&path, Some(&scoped), body).await;
        assert_eq!(other.status, StatusCode::NOT_FOUND, "{path}");
    }
}
