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
    let rollout = overview.rollout.as_ref().expect("a rollout");
    assert_eq!(rollout.version, "0.2.0");
    assert_eq!(rollout.approved_by.as_deref(), Some(OWNER));
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
    assert_eq!(overview.approved(), None, "off by default");

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

/// 0013 §6.2, D-106: a release signed with a staging key is taken only by a
/// hub whose own config says it is a staging hub, and a reload applies it.
#[tokio::test]
async fn a_staging_signed_release_is_taken_only_by_a_staging_hub() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let folder = signed_release_with(&staging_key(), "0.3.0-dev.12", &[(LINUX, b"probe")]);
    let upload = || {
        hub.post_file(
            "/api/admin/probe-releases",
            Some(&owner),
            release_archive(folder.path()),
        )
    };

    let refused = upload().await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.error_code(), "untrusted_release");

    let toml = std::fs::read_to_string(hub.config_path()).unwrap();
    hub.reload_with(&format!("{toml}\n[upgrades]\nstaging = true\n"))
        .await
        .expect("reload");
    let taken = upload().await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.text());

    // Back to a production hub: the staging release isn't trusted, so not offered.
    hub.reload_with(&toml).await.expect("reload");
    let overview: ProbeUpdates = hub
        .get("/api/admin/probe-updates", Some(&owner))
        .await
        .json();
    assert!(overview.releases.is_empty(), "{:?}", overview.releases);
}

/// 0013 §3.1 gap 5, §10.4 (D-107): `hub release approve` starts the rollout
/// from the server, as the admin page's Approve does; the rollout and the
/// audit log name the server, not a user.
#[tokio::test]
async fn the_server_approves_a_release_and_the_audit_log_says_so() {
    use hub_lib::ops::{SERVER_APPROVER, approve_on_server};
    use hub_lib::storage::AuditFilter;
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.3.0", &[(LINUX, b"probe")]);
    let config = hub.state.config.read().await.clone();
    let keys = hub.state.trusted_release_keys().await;
    assert_eq!(report(&hub, &probe, Some("0.2.0"), None).await.update, None);

    let rollout = approve_on_server(hub.store(), &config, keys.clone(), "0.3.0", hub.now())
        .await
        .expect("approved");
    assert_eq!(rollout.version, "0.3.0");
    // The running hub offers it from the next report, as after the admin page.
    let offer = report(&hub, &probe, Some("0.2.0"), None)
        .await
        .update
        .expect("the approved release is offered");
    let manifest = open_manifest(
        offer.manifest.as_bytes(),
        &offer.signature,
        &[release_key().verifying_key().to_bytes()],
    )
    .unwrap();
    assert_eq!(manifest.version, "0.3.0");
    let stored = hub.store().rollout().await.unwrap().expect("a rollout");
    assert_eq!(stored.version, "0.3.0");
    assert_eq!(stored.approved_by.as_deref(), Some(SERVER_APPROVER));

    let entries = hub
        .store()
        .audit_entries(&AuditFilter {
            action_prefix: Some("release.approve".into()),
            limit: 10,
            ..AuditFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let entry = &entries[0];
    assert_eq!(entry.username, None, "the server, not a user");
    assert_eq!(entry.target_id.as_deref(), Some("0.3.0"));
    assert!(
        entry
            .details
            .as_deref()
            .unwrap_or_default()
            .contains("hub release approve"),
        "{entry:?}"
    );

    // Only a release on this hub.
    let error = approve_on_server(hub.store(), &config, keys, "9.9.9", hub.now())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no release 9.9.9"), "{error:#}");
}

/// A staging-signed release is approved only on a staging hub (D-106).
#[tokio::test]
async fn the_server_approves_a_staging_release_only_on_a_staging_hub() {
    use hub_lib::ops::approve_on_server;
    let hub = TestHub::new(&[]).await;
    let folder = signed_release_with(&staging_key(), "0.4.0-dev.3", &[(LINUX, b"probe")]);
    hub_lib::service::releases::add_from_dir(
        &hub.dir().join(hub_lib::service::releases::RELEASES_DIR),
        folder.path(),
        &[staging_key().verifying_key().to_bytes()],
    )
    .expect("stored");

    let config = hub.state.config.read().await.clone();
    let production = hub.state.trusted_release_keys().await;
    assert!(
        approve_on_server(hub.store(), &config, production, "0.4.0-dev.3", hub.now())
            .await
            .is_err(),
        "a production hub doesn't trust it"
    );

    hub.state.config.write().await.upgrades.staging = true;
    let config = hub.state.config.read().await.clone();
    let staging = hub.state.trusted_release_keys().await;
    let rollout = approve_on_server(hub.store(), &config, staging, "0.4.0-dev.3", hub.now())
        .await
        .expect("a staging hub does");
    assert_eq!(rollout.version, "0.4.0-dev.3");
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
