//! Rotating the hub's certificate and key over the protocol (M2-13, D-61):
//! the next pins ride in signed answers, the hub switches when every probe
//! holds them (or when told to), and probes on the old pins are told.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::ack::ReportAck;
use common::models::PinDigests;
use common::pins::HubPins;
use hub_lib::keys::KeyFiles;
use hub_lib::model::{EnrollmentCodeInfo, HubKeysOverview, Role, RotationState};
use hub_lib::service::rotation::{self, Tick};
use serde_json::json;
use support::*;

const START: &str = "/api/admin/hub-keys/rotation";
const SWITCH: &str = "/api/admin/hub-keys/rotation/switch";
const CANCEL: &str = "/api/admin/hub-keys/rotation/cancel";

fn pins(hub: &TestHub) -> (HubPins, Option<HubPins>) {
    let credentials = hub.state.hub_pins.get();
    (credentials.current, credentials.next)
}

async fn overview(hub: &TestHub) -> HubKeysOverview {
    hub.get_json("/api/admin/hub-keys").await
}

/// A v7 report from `probe` saying it holds `current` (and `next`), sealed
/// for `current`'s key; the hub's answer, checked against both signing keys.
async fn report(
    hub: &TestHub,
    probe: &TestProbe,
    current: &HubPins,
    next: Option<&HubPins>,
) -> (StatusCode, Option<ReportAck>) {
    hub.clock.advance(Duration::minutes(1));
    let mut report = probe.unsigned_report("acme", hub.now(), vec![]);
    report.pins = Some(PinDigests {
        current: current.digest(),
        next: next.map(HubPins::digest),
    });
    probe.sign(&mut report);
    let keys: Vec<[u8; 32]> = std::iter::once(current.signing_key)
        .chain(next.map(|n| n.signing_key))
        .collect();
    let (status, ack) = hub
        .send_report_pinned(probe, &report, &current.hub_key, &keys)
        .await;
    (status, ack.ok())
}

async fn start(hub: &TestHub, cookie: &str, tls: bool, key: bool) -> TestResponse {
    hub.post_json(
        START,
        Some(cookie),
        json!({ "tls": tls, "key": key, "window_days": 14 }),
    )
    .await
}

async fn actions(hub: &TestHub) -> Vec<String> {
    hub.get_json::<Vec<serde_json::Value>>("/api/audit?action=rotation")
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap_or("").to_string())
        .collect()
}

#[tokio::test]
async fn a_key_rotation_is_announced_then_switched_once_every_probe_holds_it() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let (old, _) = pins(&hub);

    let (status, ack) = report(&hub, &probe, &old, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ack.unwrap().rotation, None, "nothing to rotate");

    let started = start(&hub, &owner, false, true).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.text());
    let (current, next) = pins(&hub);
    assert_eq!(current, old, "the hub still uses its key");
    let new = next.expect("announced");
    assert_ne!(new.hub_key, old.hub_key);
    assert_eq!(new.tls_spki_sha256, old.tls_spki_sha256);
    assert_eq!(overview(&hub).await.rotation.unwrap().waiting.len(), 1);

    // The answer announces the next pins, signed with the current key.
    let (_, ack) = report(&hub, &probe, &old, None).await;
    let rotation = ack.unwrap().rotation.expect("announced");
    assert!(!rotation.switched);
    assert_eq!(HubPins::try_from(&rotation.pins).unwrap(), new);

    // The probe holds them: nothing more to say; it's ready.
    let (_, ack) = report(&hub, &probe, &old, Some(&new)).await;
    assert_eq!(ack.unwrap().rotation, None);
    let progress = overview(&hub).await.rotation.unwrap();
    assert_eq!((progress.ready, progress.waiting.len()), (1, 0));

    // A code made now carries the old pins: it goes at the switch.
    let code = hub
        .post_json(
            "/api/enrollment-codes",
            Some(&owner),
            json!({ "company_id": "acme", "site_id": "hq-site", "hub_url": "http://hub:3000" }),
        )
        .await;
    assert_eq!(code.status, StatusCode::OK, "{}", code.text());

    assert_eq!(
        hub_lib::rotation::check_once(&hub.state).await.unwrap(),
        Tick::AllReady
    );
    let (current, next) = pins(&hub);
    assert_eq!((current, next), (new, None), "switched");
    assert_eq!(hub.public_key(), new.hub_key);
    let keys = overview(&hub).await;
    assert!(keys.rotation.is_none());
    assert_eq!(keys.history[0].state, RotationState::Switched);
    assert_eq!(keys.history[0].ended_by, None, "by the hub itself");
    let codes: Vec<EnrollmentCodeInfo> = hub.get_json("/api/enrollment-codes").await;
    assert!(codes.is_empty(), "codes with the old pins are revoked");

    // A report the probe sealed before it switched still opens (the old key
    // is kept); the answer, signed with the new key, says the hub switched.
    let (status, ack) = report(&hub, &probe, &old, Some(&new)).await;
    assert_eq!(status, StatusCode::OK);
    let told = ack
        .expect("signed with the next key it holds")
        .rotation
        .unwrap();
    assert!(told.switched);
    assert_eq!(HubPins::try_from(&told.pins).unwrap(), new);

    // Switched: nothing more to say.
    let (status, ack) = report(&hub, &probe, &new, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ack.unwrap().rotation, None);

    let actions = actions(&hub).await;
    assert!(actions.contains(&"rotation.start".into()), "{actions:?}");
    assert!(actions.contains(&"rotation.switch".into()));
}

#[tokio::test]
async fn an_overdue_rotation_is_flagged_not_forced_and_switch_now_cuts_off_the_rest() {
    let (ready, older) = (TestProbe::new("ready"), TestProbe::new("older"));
    let hub = TestHub::new(&[&ready, &older]).await;
    let owner = hub.login().await;
    let (old, _) = pins(&hub);
    // `older` is a v6 probe: it says nothing about pins.
    hub.report_now(&older, "acme", vec![]).await;
    start(&hub, &owner, false, true).await;
    let new = pins(&hub).1.unwrap();
    report(&hub, &ready, &old, Some(&new)).await;

    assert_eq!(
        hub_lib::rotation::check_once(&hub.state).await.unwrap(),
        Tick::Nothing
    );
    hub.clock.advance(Duration::days(15));
    let owner = hub.login().await; // the old session ran out
    assert_eq!(
        hub_lib::rotation::check_once(&hub.state).await.unwrap(),
        Tick::Overdue { waiting: 1 }
    );
    assert_eq!(
        hub_lib::rotation::check_once(&hub.state).await.unwrap(),
        Tick::Nothing,
        "said once"
    );
    let progress = overview(&hub).await.rotation.expect("still waiting");
    assert!(progress.overdue);
    assert_eq!(progress.waiting.len(), 1);
    assert_eq!(progress.waiting[0].name, "older");
    assert!(
        progress.waiting[0].reason.contains("older probe"),
        "{progress:?}"
    );

    let switched = hub.post_json(SWITCH, Some(&owner), json!({})).await;
    assert_eq!(switched.status, StatusCode::OK, "{}", switched.text());
    assert_eq!(switched.json::<serde_json::Value>()["probes_not_ready"], 1);
    assert_eq!(pins(&hub).0, new);

    // The probe that wasn't ready can't trust the hub's answers any more.
    let (_, ack) = report(&hub, &older, &old, None).await;
    assert!(ack.is_none(), "signed with a key it doesn't hold");
}

#[tokio::test]
async fn cancelling_drops_the_next_key_and_changes_nothing() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let (old, _) = pins(&hub);
    start(&hub, &owner, false, true).await;
    let files = KeyFiles::of(&hub.state.config.read().await.clone());
    assert!(files.identity_next.exists());

    let cancelled = hub.post_json(CANCEL, Some(&owner), json!({})).await;
    assert_eq!(cancelled.status, StatusCode::OK);
    assert!(!files.identity_next.exists());
    assert_eq!(pins(&hub), (old, None));
    let (_, ack) = report(&hub, &probe, &old, None).await;
    assert_eq!(ack.unwrap().rotation, None);
    assert_eq!(
        overview(&hub).await.history[0].state,
        RotationState::Cancelled
    );
}

#[tokio::test]
async fn one_rotation_at_a_time_and_bad_requests_are_refused() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    for (body, code) in [
        (
            json!({ "tls": false, "key": false, "window_days": 14 }),
            "invalid_rotation",
        ),
        (
            json!({ "tls": false, "key": true, "window_days": 0 }),
            "invalid_rotation",
        ),
        (
            json!({ "tls": true, "key": false, "window_days": 14 }),
            "invalid_rotation",
        ),
    ] {
        let refused = hub.post_json(START, Some(&owner), body).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.error_code(), code);
    }
    for path in [SWITCH, CANCEL] {
        let nothing = hub.post_json(path, Some(&owner), json!({})).await;
        assert_eq!(nothing.status, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(
        start(&hub, &owner, false, true).await.status,
        StatusCode::OK
    );
    let busy = start(&hub, &owner, false, true).await;
    assert_eq!(busy.status, StatusCode::CONFLICT);
    assert_eq!(busy.error_code(), "rotation_in_progress");

    let viewer = hub.login_as(Role::Viewer).await;
    let forbidden = hub.post_json(CANCEL, Some(&viewer), json!({})).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
}

/// Turn on HTTPS with a certificate in the hub's folder, as after a reload.
async fn with_https(hub: &TestHub) -> std::path::PathBuf {
    let cert = hub.dir().join("hub_tls_cert.pem");
    let key = hub.dir().join("hub_tls_key.pem");
    hub_lib::tls::load_or_create_tls_cert(
        cert.to_str().unwrap(),
        key.to_str().unwrap(),
        "127.0.0.1",
    )
    .unwrap();
    {
        let mut config = hub.state.config.write().await;
        config.server.tls.enabled = true;
        config.server.tls.cert_path = cert.to_string_lossy().into_owned();
        config.server.tls.key_path = key.to_string_lossy().into_owned();
    }
    hub_lib::rotation::refresh(&hub.state).await.unwrap();
    cert
}

#[tokio::test]
async fn a_certificate_rotation_swaps_the_files_and_new_codes_pin_the_new_key() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    let owner = hub.login().await;
    let cert = with_https(&hub).await;
    let (old, _) = pins(&hub);
    assert!(old.tls_spki_sha256.is_some());

    assert_eq!(
        start(&hub, &owner, true, false).await.status,
        StatusCode::OK
    );
    let new = pins(&hub).1.unwrap();
    assert_eq!(new.hub_key, old.hub_key, "only the certificate");
    assert_ne!(new.tls_spki_sha256, old.tls_spki_sha256);
    let next_cert =
        std::fs::read(KeyFiles::of(&hub.state.config.read().await.clone()).tls_cert_next).unwrap();

    report(&hub, &probe, &old, Some(&new)).await;
    hub_lib::rotation::check_once(&hub.state).await.unwrap();
    assert_eq!(pins(&hub).0, new);
    assert_eq!(
        std::fs::read(&cert).unwrap(),
        next_cert,
        "the new certificate is served"
    );
    let keys = KeyFiles::of(&hub.state.config.read().await.clone());
    assert!(!keys.identity_prev.exists(), "the key didn't change");

    let code = hub
        .post_json(
            "/api/enrollment-codes",
            Some(&owner),
            json!({ "company_id": "acme", "site_id": "hq-site", "hub_url": "https://hub:3443" }),
        )
        .await
        .json::<serde_json::Value>();
    let code = common::enrollment::EnrollmentCode::decode(code["code"].as_str().unwrap()).unwrap();
    assert_eq!(code.tls_spki_sha256, new.tls_spki_sha256);
}

#[tokio::test]
async fn the_previous_key_goes_once_no_probe_can_still_use_it() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    start(&hub, &owner, false, true).await;
    hub_lib::rotation::check_once(&hub.state).await.unwrap(); // no probes: all ready
    let files = KeyFiles::of(&hub.state.config.read().await.clone());
    assert!(files.identity_prev.exists());
    assert!(hub.state.hub_pins.get().prev_secret.is_some());

    hub.clock.advance(Duration::days(6));
    hub_lib::rotation::check_once(&hub.state).await.unwrap();
    assert!(
        files.identity_prev.exists(),
        "a probe may still hold a report for it"
    );
    hub.clock.advance(rotation::prev_key_retention());
    hub_lib::rotation::check_once(&hub.state).await.unwrap();
    assert!(!files.identity_prev.exists());
    assert!(hub.state.hub_pins.get().prev_secret.is_none());
}

#[tokio::test]
async fn a_switch_cut_short_is_finished_at_startup() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    start(&hub, &owner, false, true).await;
    let config = hub.state.config.read().await.clone();
    let files = KeyFiles::of(&config);
    let next = std::fs::read(&files.identity_next).unwrap();
    // The hub stopped right after moving the new key in place.
    std::fs::rename(&files.identity_next, &files.identity).unwrap();

    assert!(
        rotation::recover(hub.store(), &config, hub.now())
            .await
            .unwrap()
    );
    assert_eq!(std::fs::read(&files.identity).unwrap(), next);
    assert!(rotation::active(hub.store()).await.unwrap().is_none());
    assert!(
        !rotation::recover(hub.store(), &config, hub.now())
            .await
            .unwrap(),
        "nothing left to finish"
    );
}
