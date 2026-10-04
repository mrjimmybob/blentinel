//! Probe releases and updates (M2-15, D-55): adding, approving and removing
//! signed releases, offering the approved one in the answer to a report, and
//! serving its files to a probe.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::ReportAck;
use common::models::UpdateFailure;
use common::release::{Platform, open_manifest};
use ed25519_dalek::SigningKey;
use hub_lib::model::{ProbeRelease, ProbeUpdates, Role};
use support::*;

const LINUX: &str = "x86_64-unknown-linux-musl";

/// Report from `probe` a minute later, as a probe at `version` that last
/// failed to install `failed`; the hub's answer.
async fn report(
    hub: &TestHub,
    probe: &TestProbe,
    version: Option<&str>,
    failed: Option<UpdateFailure>,
) -> ReportAck {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.probe_version = version.map(str::to_string);
    report.update_failed = failed;
    probe.sign(&mut report);
    let (status, ack) = hub.send_report_for_ack(probe, &report).await;
    assert_eq!(status, StatusCode::OK);
    ack.expect("a trusted ack")
}

async fn approve(hub: &TestHub, cookie: &str, version: Option<&str>) -> TestResponse {
    hub.post_json(
        "/api/admin/probe-updates/approve",
        Some(cookie),
        serde_json::json!({ "version": version }),
    )
    .await
}

#[tokio::test]
async fn nothing_is_offered_until_a_release_is_approved() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    assert_eq!(report(&hub, &probe, Some("0.1.0"), None).await.update, None);

    let owner = hub.login().await;
    assert_eq!(
        approve(&hub, &owner, Some("0.2.0")).await.status,
        StatusCode::OK
    );
    let offer = report(&hub, &probe, Some("0.1.0"), None)
        .await
        .update
        .expect("the approved release is offered");
    let manifest = open_manifest(
        offer.manifest.as_bytes(),
        &offer.signature,
        &[release_key().verifying_key().to_bytes()],
    )
    .expect("signed with the release key, as added");
    assert_eq!(manifest.version, "0.2.0");

    // Its binary is served to probes, without a session.
    let file = Platform::by_target(LINUX).unwrap().file_name();
    let download = hub
        .get(&format!("/download/releases/0.2.0/{file}"), None)
        .await;
    assert_eq!(download.status, StatusCode::OK);
    assert_eq!(&download.body[..], b"probe two");
    for missing in [
        "/download/releases/0.2.0/manifest.json.sig".to_string(),
        format!("/download/releases/0.9.0/{file}"),
        format!("/download/releases/..%2F0.2.0/{file}"),
    ] {
        assert_eq!(
            hub.get(&missing, None).await.status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }

    // Stopping updates.
    assert_eq!(approve(&hub, &owner, None).await.status, StatusCode::OK);
    assert_eq!(report(&hub, &probe, Some("0.1.0"), None).await.update, None);
}

#[tokio::test]
async fn a_probe_isnt_offered_what_it_runs_or_refused() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let owner = hub.login().await;
    approve(&hub, &owner, Some("0.2.0")).await;

    assert_eq!(report(&hub, &probe, Some("0.2.0"), None).await.update, None);
    let refused = UpdateFailure {
        version: "0.2.0".into(),
        error: "rolled back: no report within 15 minutes".into(),
    };
    let ack = report(&hub, &probe, Some("0.1.0"), Some(refused.clone())).await;
    assert_eq!(ack.update, None);
    // A v4 probe (no version) can't update itself.
    assert_eq!(report(&hub, &probe, None, None).await.update, None);

    // The admin page shows what each probe runs and what it refused.
    report(&hub, &probe, Some("0.1.0"), Some(refused.clone())).await;
    let overview: ProbeUpdates = hub.get_json("/api/admin/probe-updates").await;
    assert_eq!(overview.approved.as_deref(), Some("0.2.0"));
    assert_eq!(overview.approved_by.as_deref(), Some(OWNER));
    assert!(overview.has_release_key);
    let listed = &overview.probes[0];
    assert_eq!(listed.probe_version.as_deref(), Some("0.1.0"));
    assert_eq!(listed.update_failed.as_ref(), Some(&refused));
}

#[tokio::test]
async fn admins_upload_approve_and_remove_releases() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let folder = signed_release("0.3.0", &[(LINUX, b"probe three")]);

    let added = hub
        .post_file(
            "/api/admin/probe-releases",
            Some(&owner),
            release_archive(folder.path()),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text());
    let added: ProbeRelease = added.json();
    assert_eq!(added.version, "0.3.0");
    assert_eq!(added.platforms, ["linux x86_64"]);
    let again = hub
        .post_file(
            "/api/admin/probe-releases",
            Some(&owner),
            release_archive(folder.path()),
        )
        .await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "the same release again changes nothing"
    );

    let overview: ProbeUpdates = hub.get_json("/api/admin/probe-updates").await;
    assert_eq!(overview.releases.len(), 1);
    assert_eq!(overview.approved, None, "off by default");

    assert_eq!(
        approve(&hub, &owner, Some("9.9.9")).await.status,
        StatusCode::NOT_FOUND
    );
    approve(&hub, &owner, Some("0.3.0")).await;
    let delete = "/api/admin/probe-releases/0.3.0/delete";
    let refused = hub
        .post_json(delete, Some(&owner), serde_json::json!({}))
        .await;
    assert_eq!(
        refused.status,
        StatusCode::CONFLICT,
        "the approved one stays"
    );
    approve(&hub, &owner, None).await;
    let deleted = hub
        .post_json(delete, Some(&owner), serde_json::json!({}))
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    let overview: ProbeUpdates = hub.get_json("/api/admin/probe-updates").await;
    assert!(overview.releases.is_empty());

    let actions: Vec<String> = hub
        .get_json::<Vec<serde_json::Value>>("/api/audit?action=release")
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(actions.contains(&"release.add".to_string()), "{actions:?}");
    assert!(actions.contains(&"release.approve".to_string()));
    assert!(actions.contains(&"release.remove".to_string()));
}

#[tokio::test]
async fn releases_not_signed_by_a_trusted_key_are_refused() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let stranger = SigningKey::from_bytes(&[1; 32]);
    let folder = signed_release_with(&stranger, "0.3.0", &[(LINUX, b"probe")]);
    let refused = hub
        .post_file(
            "/api/admin/probe-releases",
            Some(&owner),
            release_archive(folder.path()),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.error_code(), "untrusted_release");

    let garbage = hub
        .post_file(
            "/api/admin/probe-releases",
            Some(&owner),
            b"not an archive".to_vec(),
        )
        .await;
    assert_eq!(garbage.error_code(), "invalid_release");
}

#[tokio::test]
async fn only_admins_manage_releases() {
    let hub = TestHub::new(&[]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let technician = hub.login_as(Role::Technician).await;
    assert_eq!(
        approve(&hub, &technician, Some("0.2.0")).await.status,
        StatusCode::FORBIDDEN
    );
    let upload = hub
        .post_file("/api/admin/probe-releases", Some(&technician), vec![])
        .await;
    assert_eq!(upload.status, StatusCode::FORBIDDEN);
}
