//! The managed probe rollout (M4-01, D-74): canaries first, then every
//! probe; pause, resume, promote; each company's policy and update window;
//! signed rollbacks.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::{ReportAck, UpdateOffer};
use common::models::UpdateFailure;
use common::release::{MANIFEST_FORMAT, ROLLBACK_FILE, ROLLBACK_SIGNATURE_FILE, Rollback};
use hub_lib::model::{ProbeUpdates, Role, RolloutStage, UpdateStatus};
use hub_lib::service::rollout;
use support::*;

const LINUX: &str = "x86_64-unknown-linux-musl";

/// Report from `probe` a minute later as a probe at `version`; the release
/// the hub offers it, if any.
async fn offered(
    hub: &TestHub,
    probe: &TestProbe,
    version: &str,
    failed: Option<UpdateFailure>,
) -> Option<UpdateOffer> {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.probe_version = Some(version.into());
    report.update_failed = failed;
    probe.sign(&mut report);
    let (status, ack) = hub.send_report_for_ack(probe, &report).await;
    assert_eq!(status, StatusCode::OK);
    let ack: ReportAck = ack.expect("a trusted ack");
    ack.update
}

/// The version an offer is for.
fn version_of(offer: &UpdateOffer) -> String {
    let manifest: serde_json::Value = serde_json::from_str(&offer.manifest).unwrap();
    manifest["version"].as_str().unwrap().to_string()
}

async fn post(hub: &TestHub, cookie: &str, path: &str, body: serde_json::Value) -> TestResponse {
    hub.post_json(path, Some(cookie), body).await
}

async fn approve(hub: &TestHub, cookie: &str, version: &str, soak: u32) -> TestResponse {
    post(
        hub,
        cookie,
        "/api/admin/probe-updates/approve",
        serde_json::json!({ "version": version, "soak_minutes": soak }),
    )
    .await
}

async fn make_canary(hub: &TestHub, cookie: &str, probe: &TestProbe) {
    let done = post(
        hub,
        cookie,
        &format!("/api/admin/probe-updates/probes/{}/canary", probe.id),
        serde_json::json!({ "canary": true }),
    )
    .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text());
}

async fn advance(hub: &TestHub) {
    rollout::advance(hub.store(), hub.now()).await.unwrap();
}

async fn overview(hub: &TestHub) -> ProbeUpdates {
    hub.get_json("/api/admin/probe-updates").await
}

fn status_of(updates: &ProbeUpdates, probe: &TestProbe) -> UpdateStatus {
    updates
        .probes
        .iter()
        .find(|p| p.id == probe.id)
        .and_then(|p| p.update_status.clone())
        .expect("listed with a status")
}

#[tokio::test]
async fn canaries_get_a_release_first_then_every_probe_after_the_soak() {
    let (canary, other) = (TestProbe::new("canary"), TestProbe::new("other"));
    let hub = TestHub::new(&[&canary, &other]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let owner = hub.login().await;
    make_canary(&hub, &owner, &canary).await;
    assert_eq!(
        approve(&hub, &owner, "0.2.0", 30).await.status,
        StatusCode::OK
    );

    let updates = overview(&hub).await;
    let r = updates.rollout.as_ref().unwrap();
    assert_eq!(
        (r.stage, r.canaries, r.canaries_ready),
        (RolloutStage::Canary, 1, 0)
    );
    assert!(updates.probes.iter().any(|p| p.id == canary.id && p.canary));

    assert!(offered(&hub, &canary, "0.1.0", None).await.is_some());
    assert!(offered(&hub, &other, "0.1.0", None).await.is_none());
    assert_eq!(
        status_of(&overview(&hub).await, &other),
        UpdateStatus::AwaitingCanaries {
            version: "0.2.0".into()
        }
    );

    // The canary now runs it; the soak starts.
    offered(&hub, &canary, "0.2.0", None).await;
    advance(&hub).await;
    let updates = overview(&hub).await;
    let r = updates.rollout.as_ref().unwrap();
    assert_eq!(r.canaries_ready, 1);
    assert!(r.promote_at.is_some());
    assert_eq!(status_of(&updates, &canary), UpdateStatus::UpToDate);
    hub.clock.advance(Duration::minutes(29));
    advance(&hub).await;
    assert!(
        offered(&hub, &other, "0.1.0", None).await.is_none(),
        "still soaking"
    );

    hub.clock.advance(Duration::minutes(1));
    advance(&hub).await;
    let r = overview(&hub).await.rollout.unwrap();
    assert_eq!(r.stage, RolloutStage::All);
    assert!(
        r.promoted_at.is_some() && r.promoted_by.is_none(),
        "by the hub"
    );
    let offer = offered(&hub, &other, "0.1.0", None)
        .await
        .expect("now everyone");
    assert_eq!(version_of(&offer), "0.2.0");
}

#[tokio::test]
async fn without_canaries_a_release_goes_to_every_probe_at_once() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let owner = hub.login().await;
    approve(&hub, &owner, "0.2.0", 60).await;
    assert_eq!(
        overview(&hub).await.rollout.unwrap().stage,
        RolloutStage::All
    );
    assert!(offered(&hub, &probe, "0.1.0", None).await.is_some());
}

#[tokio::test]
async fn a_canary_that_fails_pauses_the_rollout_until_someone_acts() {
    let (canary, other) = (TestProbe::new("canary"), TestProbe::new("other"));
    let hub = TestHub::new(&[&canary, &other]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let owner = hub.login().await;
    make_canary(&hub, &owner, &canary).await;
    approve(&hub, &owner, "0.2.0", 0).await;

    let failure = UpdateFailure {
        version: "0.2.0".into(),
        error: "rolled back: started 3 times without delivering a report".into(),
    };
    offered(&hub, &canary, "0.1.0", Some(failure)).await;
    advance(&hub).await;
    let r = overview(&hub).await.rollout.unwrap();
    let paused = r.paused.expect("paused by the hub");
    assert_eq!(paused.by, None);
    assert!(
        paused.reason.contains("canary didn't install 0.2.0"),
        "{}",
        paused.reason
    );
    assert!(offered(&hub, &other, "0.1.0", None).await.is_none());

    // An admin sends it to everyone anyway.
    let promoted = post(
        &hub,
        &owner,
        "/api/admin/probe-updates/promote",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(promoted.status, StatusCode::OK);
    let r = overview(&hub).await.rollout.unwrap();
    assert_eq!(
        (r.stage, r.paused, r.promoted_by.as_deref()),
        (RolloutStage::All, None, Some(OWNER))
    );
    assert!(offered(&hub, &other, "0.1.0", None).await.is_some());
    // The canary that refused it isn't offered it again.
    assert!(
        offered(
            &hub,
            &canary,
            "0.1.0",
            Some(UpdateFailure {
                version: "0.2.0".into(),
                error: "rolled back".into(),
            })
        )
        .await
        .is_none()
    );

    let actions: Vec<String> = hub
        .get_json::<Vec<serde_json::Value>>("/api/audit?action=rollout")
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        actions.contains(&"rollout.pause".to_string()),
        "{actions:?}"
    );
    assert!(actions.contains(&"rollout.promote".to_string()));
}

#[tokio::test]
async fn admins_pause_and_resume_a_rollout() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let owner = hub.login().await;
    let pause = "/api/admin/probe-updates/pause";
    let resume = "/api/admin/probe-updates/resume";
    assert_eq!(
        post(&hub, &owner, pause, serde_json::json!({}))
            .await
            .status,
        StatusCode::NOT_FOUND,
        "nothing rolled out"
    );
    approve(&hub, &owner, "0.2.0", 60).await;
    assert_eq!(
        post(&hub, &owner, pause, serde_json::json!({}))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        post(&hub, &owner, pause, serde_json::json!({}))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert!(offered(&hub, &probe, "0.1.0", None).await.is_none());
    let paused = overview(&hub).await.rollout.unwrap().paused.unwrap();
    assert_eq!(paused.by.as_deref(), Some(OWNER));
    assert_eq!(
        post(&hub, &owner, resume, serde_json::json!({}))
            .await
            .status,
        StatusCode::OK
    );
    assert!(offered(&hub, &probe, "0.1.0", None).await.is_some());
    assert_eq!(
        post(
            &hub,
            &owner,
            "/api/admin/probe-updates/promote",
            serde_json::json!({})
        )
        .await
        .status,
        StatusCode::CONFLICT,
        "already to every probe"
    );
}

#[tokio::test]
async fn a_company_can_take_no_updates_be_pinned_or_wait_for_its_window() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    hub.add_release("0.3.0", &[(LINUX, b"probe three")]);
    let owner = hub.login().await;
    approve(&hub, &owner, "0.3.0", 60).await;
    let policy = "/api/admin/probe-updates/companies/acme";

    assert_eq!(
        post(&hub, &owner, policy, serde_json::json!({ "mode": "off" }))
            .await
            .status,
        StatusCode::OK
    );
    assert!(offered(&hub, &probe, "0.1.0", None).await.is_none());
    assert_eq!(
        status_of(&overview(&hub).await, &probe),
        UpdateStatus::CompanyOff
    );

    // Pinned: that release, whatever is rolled out.
    post(
        &hub,
        &owner,
        policy,
        serde_json::json!({ "mode": "pin", "version": "0.2.0" }),
    )
    .await;
    let offer = offered(&hub, &probe, "0.1.0", None).await.unwrap();
    assert_eq!(version_of(&offer), "0.2.0");
    let updates = overview(&hub).await;
    let acme = updates
        .companies
        .iter()
        .find(|c| c.company_id == "acme")
        .unwrap();
    assert_eq!(acme.policy.version.as_deref(), Some("0.2.0"));
    // … and that release can't be removed meanwhile.
    let delete = post(
        &hub,
        &owner,
        "/api/admin/probe-releases/0.2.0/delete",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(delete.status, StatusCode::CONFLICT);

    // A window that is closed now (the site is in UTC; the clock near 12:00).
    let closed = serde_json::json!({
        "mode": "follow",
        "window": { "start": "02:00", "end": "05:00" }
    });
    assert_eq!(
        post(&hub, &owner, policy, closed).await.status,
        StatusCode::OK
    );
    assert!(offered(&hub, &probe, "0.1.0", None).await.is_none());
    assert!(matches!(
        status_of(&overview(&hub).await, &probe),
        UpdateStatus::OutsideWindow { .. }
    ));
    let open = serde_json::json!({
        "mode": "follow",
        "window": { "start": "11:00", "end": "13:00" }
    });
    post(&hub, &owner, policy, open).await;
    let offer = offered(&hub, &probe, "0.1.0", None).await.unwrap();
    assert_eq!(version_of(&offer), "0.3.0");

    // What the hub refuses.
    for (body, status) in [
        (
            serde_json::json!({ "mode": "pin" }),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({ "mode": "pin", "version": "9.9.9" }),
            StatusCode::NOT_FOUND,
        ),
        (
            serde_json::json!({ "mode": "follow", "window": { "start": "02:00", "end": "02:00" } }),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(
            post(&hub, &owner, policy, body.clone()).await.status,
            status,
            "{body}"
        );
    }
    assert_eq!(
        post(
            &hub,
            &owner,
            "/api/admin/probe-updates/companies/nobody",
            serde_json::json!({ "mode": "off" })
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

/// A folder holding a signed rollback to `version` from `from`.
fn signed_rollback(version: &str, from: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let rollback = Rollback {
        format: MANIFEST_FORMAT,
        version: version.into(),
        from: from.iter().map(|v| v.to_string()).collect(),
        signed: "2026-03-10T11:00:00Z".parse().unwrap(),
    };
    let bytes = serde_json::to_vec_pretty(&rollback).unwrap();
    std::fs::write(dir.path().join(ROLLBACK_FILE), &bytes).unwrap();
    std::fs::write(
        dir.path().join(ROLLBACK_SIGNATURE_FILE),
        common::release::sign_rollback(&bytes, &release_key()),
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn an_older_release_is_offered_only_with_a_signed_rollback() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let owner = hub.login().await;
    approve(&hub, &owner, "0.2.0", 60).await;
    assert!(offered(&hub, &probe, "0.3.0", None).await.is_none());
    assert_eq!(
        status_of(&overview(&hub).await, &probe),
        UpdateStatus::NoRollback {
            version: "0.2.0".into()
        }
    );

    // The rollback, uploaded on its own like a release.
    let rollback = signed_rollback("0.2.0", &["0.3.0"]);
    let added = hub
        .post_file(
            "/api/admin/probe-releases",
            Some(&owner),
            release_archive(rollback.path()),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text());
    let updates = overview(&hub).await;
    assert_eq!(updates.releases[0].rollback_from, ["0.3.0"]);

    let offer = offered(&hub, &probe, "0.3.0", None)
        .await
        .expect("may go back");
    let signed = offer.rollback.expect("with its rollback");
    let opened = common::release::open_rollback(
        signed.rollback.as_bytes(),
        &signed.signature,
        &[release_key().verifying_key().to_bytes()],
    )
    .unwrap();
    assert!(opened.allows("0.3.0"));
    // Not from a version it doesn't name.
    assert!(offered(&hub, &probe, "0.3.1", None).await.is_none());
}

#[tokio::test]
async fn only_admins_steer_the_rollout() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.add_release("0.2.0", &[(LINUX, b"probe two")]);
    let technician = hub.login_as(Role::Technician).await;
    for (path, body) in [
        ("/api/admin/probe-updates/pause", serde_json::json!({})),
        ("/api/admin/probe-updates/resume", serde_json::json!({})),
        ("/api/admin/probe-updates/promote", serde_json::json!({})),
        (
            &*format!("/api/admin/probe-updates/probes/{}/canary", probe.id),
            serde_json::json!({ "canary": true }),
        ),
        (
            "/api/admin/probe-updates/companies/acme",
            serde_json::json!({ "mode": "off" }),
        ),
    ] {
        assert_eq!(
            post(&hub, &technician, path, body).await.status,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
}
