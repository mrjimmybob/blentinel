//! Network discovery (M5-18, D-101): off for a company until someone who
//! manages the hub's data allows it; then a sweep is a command, what the
//! probe found comes with a report, and the hub suggests checks from it.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::ReportAck;
use common::checks::CheckSpec;
use common::commands::CommandSpec;
use common::discovery::{Device, Discovery};
use hub_lib::model::{AddedChecks, AuditEntry, CommandInfo, CompanyDetail, NetworkInfo, Role};
use serde_json::json;
use support::*;

fn sweep(at: chrono::DateTime<chrono::Utc>) -> Discovery {
    Discovery {
        started_at: at - Duration::seconds(9),
        finished_at: at,
        networks: vec!["192.168.1.0/24".into()],
        scanned: 253,
        devices: vec![
            Device {
                address: "192.168.1.20".into(),
                name: Some("nas.lan".into()),
                mac: Some("00:11:32:AA:BB:CC".into()),
                pinged: true,
                ports: vec![5000, 445, 31337],
            },
            Device {
                address: "192.168.1.1".into(),
                name: None,
                mac: None,
                pinged: true,
                ports: vec![],
            },
        ],
        note: None,
    }
}

/// A report from `probe`, a minute later, carrying `found`; the hub's answer.
async fn report(hub: &TestHub, probe: &TestProbe, found: Option<Discovery>) -> ReportAck {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.discovery = found;
    probe.sign(&mut report);
    let (status, ack) = hub.send_report_for_ack(probe, &report).await;
    assert_eq!(status, StatusCode::OK);
    ack.expect("a trusted ack")
}

async fn allow(hub: &TestHub, cookie: &str, allowed: bool) -> TestResponse {
    hub.post_json(
        "/api/companies/acme/discovery",
        Some(cookie),
        json!({ "allowed": allowed }),
    )
    .await
}

async fn discover(hub: &TestHub, cookie: &str, probe: &TestProbe) -> TestResponse {
    hub.post_json(
        &format!("/api/probes/{}/commands", probe.id),
        Some(cookie),
        json!({ "kind": "discover" }),
    )
    .await
}

async fn network_of(hub: &TestHub, probe: &TestProbe) -> NetworkInfo {
    hub.get_json(&format!("/api/probes/{}/network", probe.id))
        .await
}

#[tokio::test]
async fn discovery_is_off_until_an_admin_allows_it_for_the_company() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, None).await;

    let detail: CompanyDetail = hub.get_json("/api/companies/acme").await;
    assert!(!detail.company.discovery_allowed);
    assert!(!network_of(&hub, &probe).await.allowed);

    // Off: nobody can ask for a sweep, not even the Owner.
    let refused = discover(&hub, &owner, &probe).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text());
    assert!(refused.text().contains("isn't allowed for this company"));
    assert!(report(&hub, &probe, None).await.commands.is_empty());

    // Who sends commands (a Technician) can't allow it; an Admin can.
    let technician = hub.login_as(Role::Technician).await;
    assert_eq!(
        allow(&hub, &technician, true).await.status,
        StatusCode::FORBIDDEN
    );
    let admin = hub.login_as(Role::Admin).await;
    let allowed = allow(&hub, &admin, true).await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.text());
    assert!(allowed.json::<CompanyDetail>().company.discovery_allowed);
    assert_eq!(
        allow(&hub, &admin, true).await.status,
        StatusCode::OK,
        "allowing twice is fine"
    );
    let nowhere = hub
        .post_json(
            "/api/companies/nobody/discovery",
            Some(&owner),
            json!({ "allowed": true }),
        )
        .await;
    assert_eq!(nowhere.status, StatusCode::NOT_FOUND);

    // Now the Technician asks, and the command rides in the next answer.
    let asked = discover(&hub, &technician, &probe).await;
    assert_eq!(asked.status, StatusCode::OK, "{}", asked.text());
    let command: CommandInfo = asked.json();
    assert_eq!(command.kind, "discover");
    let ack = report(&hub, &probe, None).await;
    assert_eq!(ack.commands.len(), 1);
    assert_eq!(ack.commands[0].spec, CommandSpec::Discover);

    let audit: Vec<AuditEntry> = hub.get_json("/api/audit").await;
    let entry = audit
        .iter()
        .find(|e| e.action == "company.discovery")
        .expect("allowing it is in the audit log");
    assert_eq!(entry.company_id.as_deref(), Some("acme"));

    // Forbidden again: no more sweeps.
    assert_eq!(allow(&hub, &admin, false).await.status, StatusCode::OK);
    assert_eq!(
        discover(&hub, &technician, &probe).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn what_a_probe_found_is_kept_shown_and_suggests_checks() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, None).await;
    assert_eq!(allow(&hub, &owner, true).await.status, StatusCode::OK);

    let before = network_of(&hub, &probe).await;
    assert!(before.allowed);
    assert_eq!(before.reported_at, None);
    assert!(before.devices.is_empty());

    report(&hub, &probe, Some(sweep(hub.now()))).await;
    // A later report without one changes nothing.
    report(&hub, &probe, None).await;
    let found = network_of(&hub, &probe).await;
    assert!(found.reported_at.is_some());
    assert_eq!(found.networks, ["192.168.1.0/24"]);
    assert_eq!((found.scanned, found.seconds), (253, 9));
    // In the order of their addresses, within the hub's bounds: a port a
    // sweep doesn't try is dropped, the hardware address is put in one form.
    let addresses: Vec<&str> = found
        .devices
        .iter()
        .map(|d| d.device.address.as_str())
        .collect();
    assert_eq!(addresses, ["192.168.1.1", "192.168.1.20"]);
    let nas = &found.devices[1];
    assert_eq!(nas.device.mac.as_deref(), Some("00:11:32:aa:bb:cc"));
    assert_eq!(nas.device.ports, [445, 5000]);
    assert_eq!(nas.checks, 0);
    let names: Vec<&str> = nas.suggestions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["nas", "nas Windows file sharing", "nas NAS web interface"]
    );

    // Adding them is the batch the machine's suggestions use.
    let checks: Vec<_> = nas
        .suggestions
        .iter()
        .map(|s| json!({ "name": s.name, "spec": s.spec }))
        .collect();
    let added = hub
        .post_json(
            &format!("/api/probes/{}/checks/batch", probe.id),
            Some(&owner),
            json!({ "checks": checks }),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text());
    assert_eq!(added.json::<AddedChecks>().added.len(), 3);
    let after = network_of(&hub, &probe).await;
    assert_eq!(after.devices[1].checks, 3);
    assert!(after.devices[1].suggestions.is_empty());
    assert_eq!(
        after.devices[0].suggestions[0].spec,
        CheckSpec::ping("192.168.1.1")
    );
}

#[tokio::test]
async fn a_sweep_nobody_allowed_is_dropped_and_others_dont_see_one() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    report(&hub, &probe, None).await;

    // Not allowed: what the probe sends is dropped, the report still counts.
    report(&hub, &probe, Some(sweep(hub.now()))).await;
    assert_eq!(network_of(&hub, &probe).await.reported_at, None);

    assert_eq!(allow(&hub, &owner, true).await.status, StatusCode::OK);
    report(&hub, &probe, Some(sweep(hub.now()))).await;
    assert_eq!(network_of(&hub, &probe).await.devices.len(), 2);

    // Forbidden later: what was found stays, to be read.
    assert_eq!(allow(&hub, &owner, false).await.status, StatusCode::OK);
    let kept = network_of(&hub, &probe).await;
    assert!(!kept.allowed);
    assert_eq!(kept.devices.len(), 2);

    // A user of another company sees no such probe, and can't allow it here.
    let made = hub
        .post_json(
            "/api/companies",
            Some(&owner),
            json!({ "id": "other-company", "name": "Other" }),
        )
        .await;
    assert!(made.status.is_success(), "{}", made.text());
    let other = hub
        .login_scoped("olga", Role::Technician, &["other-company"])
        .await;
    let path = format!("/api/probes/{}/network", probe.id);
    assert_eq!(
        hub.get(&path, Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_ne!(allow(&hub, &other, true).await.status, StatusCode::OK);
    assert_eq!(hub.get(&path, None).await.status, StatusCode::UNAUTHORIZED);
}
