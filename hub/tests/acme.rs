//! The hub's certificate from an ACME authority such as Let's Encrypt
//! (M2-14, D-62): issued for the hub's own key, so what probes pin never
//! changes; renewed with a third of its life left; retried with back-off;
//! back to self-signed when turned off; the next certificate of a rotation
//! issued before the switch. The authority is [`TestAuthority`].
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use hub_lib::keys::KeyFiles;
use hub_lib::model::{AuditEntry, CertificateOverview, Role};
use hub_lib::service::acme::Outcome;
use hub_lib::tls;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde_json::{Value, json};
use support::*;

const CERTIFICATE: &str = "/api/admin/certificate";
const RENEW: &str = "/api/admin/certificate/renew";

fn settings(enabled: bool) -> Value {
    json!({
        "enabled": enabled,
        "domains": ["Hub.Example.com"],
        "email": "ops@example.com",
        "authority": "lets_encrypt_staging",
        "directory_url": "",
        "ca_pem": "",
        "challenge": "tls_alpn",
        "terms_agreed": true,
    })
}

async fn overview(hub: &TestHub) -> CertificateOverview {
    hub.get_json(CERTIFICATE).await
}

async fn save(hub: &TestHub, owner: &str, body: Value) -> TestResponse {
    hub.post_json(CERTIFICATE, Some(owner), body).await
}

/// The certificate file the hub serves, and its key's pin.
fn served(hub: &TestHub, cert: &std::path::Path) -> (tls::CertificateDetails, [u8; 32]) {
    let pem = std::fs::read(cert).unwrap();
    let details = tls::certificate_details(&pem).unwrap();
    // The listener serves exactly that file's certificate.
    let der = CertificateDer::pem_slice_iter(&pem)
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(hub.state.tls.served().as_deref(), Some(der.as_ref()));
    (details, tls::cert_spki_sha256(&pem).unwrap())
}

async fn actions(hub: &TestHub) -> Vec<AuditEntry> {
    hub.get_json::<Vec<AuditEntry>>("/api/audit?action=certificate")
        .await
}

#[tokio::test]
async fn off_by_default_with_the_self_signed_certificate() {
    let hub = TestHub::new(&[]).await;
    let cert = hub.with_https().await;

    let page = overview(&hub).await;
    assert!(page.tls_enabled && !page.settings.enabled);
    let current = page.certificate.unwrap();
    assert!(current.self_signed);
    assert_eq!(current.issuer, "Blentinel Hub");
    assert_eq!(page.next_attempt_at, None, "off: nothing to do");
    assert_eq!(page.http_port, Some(3000));

    // Off, the hub never touches the file (it may be the admin's own).
    let before = std::fs::read(&cert).unwrap();
    assert_eq!(
        hub_lib::acme::check_once(&hub.state).await.unwrap(),
        Outcome::Nothing
    );
    assert_eq!(std::fs::read(&cert).unwrap(), before);
    assert!(hub.authority.orders().is_empty());
}

#[tokio::test]
async fn turning_it_on_issues_a_certificate_for_the_same_key_so_probes_see_no_change() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let cert = hub.with_https().await;
    let pin = hub.state.hub_pins.get().current;

    let saved = save(&hub, &owner, settings(true)).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    assert_eq!(saved.json::<Value>()["error"], Value::Null);
    assert_eq!(hub.authority.orders(), [["hub.example.com"]], "normalized");

    let (details, spki) = served(&hub, &cert);
    assert!(!details.self_signed);
    assert_eq!(details.issuer, TestAuthority::ISSUER);
    assert_eq!(details.names, ["hub.example.com"]);
    assert_eq!(Some(spki), pin.tls_spki_sha256, "the pin holds");
    hub_lib::rotation::refresh(&hub.state).await.unwrap();
    assert_eq!(hub.state.hub_pins.get().current, pin);

    let page = overview(&hub).await;
    assert!(page.settings.enabled);
    assert_eq!(page.last_error, None);
    assert!(page.last_issued_at.is_some());
    assert_eq!(
        page.next_attempt_at.as_deref(),
        Some("2026-05-09 12:00 UTC"),
        "60 days of 90"
    );
    let audited: Vec<_> = actions(&hub).await.into_iter().map(|e| e.action).collect();
    assert_eq!(audited, ["certificate.issue", "certificate.settings"]);

    // Saving the same settings again doesn't spend an issuance.
    save(&hub, &owner, settings(true)).await;
    assert_eq!(hub.authority.orders().len(), 1);
}

#[tokio::test]
async fn a_certificate_is_renewed_with_a_third_of_its_life_left() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let cert = hub.with_https().await;
    save(&hub, &owner, settings(true)).await;
    let first = served(&hub, &cert).0;

    hub.clock.advance(Duration::days(59));
    assert_eq!(
        hub_lib::acme::check_once(&hub.state).await.unwrap(),
        Outcome::Nothing
    );
    hub.clock.advance(Duration::days(1));
    assert_eq!(
        hub_lib::acme::check_once(&hub.state).await.unwrap(),
        Outcome::Issued {
            count: 1,
            serving: true
        }
    );
    let renewed = served(&hub, &cert).0;
    assert_eq!(renewed.not_before, first.not_before + Duration::days(60));
    assert_eq!(hub.authority.orders().len(), 2);
    let by_hub = actions(&hub).await;
    assert_eq!(by_hub[0].action, "certificate.issue");
    assert_eq!(by_hub[0].username, None, "the hub itself");
}

#[tokio::test]
async fn a_failure_is_shown_and_retried_later_and_later() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let cert = hub.with_https().await;
    let refused = "Connection refused: hub.example.com:443";
    hub.authority.fail_with(Some(refused));

    let saved = save(&hub, &owner, settings(true)).await;
    assert_eq!(saved.status, StatusCode::OK, "the settings are saved");
    assert!(
        saved.json::<Value>()["error"]
            .as_str()
            .unwrap()
            .contains(refused)
    );
    let page = overview(&hub).await;
    assert!(page.settings.enabled);
    assert!(page.last_error.unwrap().contains(refused));
    assert_eq!(
        page.next_attempt_at.as_deref(),
        Some("2026-03-10 13:00 UTC")
    );
    assert!(
        served(&hub, &cert).0.self_signed,
        "still serving the old one"
    );

    // The hourly look waits an hour, then two...
    let wait = hub_lib::acme::check_once(&hub.state).await.unwrap();
    assert!(matches!(wait, Outcome::Waiting(_)), "{wait:?}");
    hub.clock.advance(Duration::hours(1));
    let again = hub_lib::acme::check_once(&hub.state).await.unwrap();
    assert!(matches!(again, Outcome::Failed(_)), "{again:?}");
    hub.clock.advance(Duration::minutes(90));
    let wait = hub_lib::acme::check_once(&hub.state).await.unwrap();
    assert!(matches!(wait, Outcome::Waiting(_)), "doubled: {wait:?}");

    // ..."Renew now" doesn't wait, and says why it failed.
    let renew = hub.post_json(RENEW, Some(&owner), json!({})).await;
    assert_eq!(renew.status, StatusCode::BAD_GATEWAY);
    assert_eq!(renew.error_code(), "certificate_not_issued");
    assert!(renew.text().contains(refused));

    // Fixed (say the port now reaches the hub): saving again tries again.
    hub.authority.fail_with(None);
    let saved = save(&hub, &owner, settings(true)).await;
    assert_eq!(saved.json::<Value>()["error"], Value::Null);
    assert!(!served(&hub, &cert).0.self_signed);
    assert_eq!(overview(&hub).await.last_error, None);
    // Working: saving the same settings doesn't spend an issuance...
    let orders = hub.authority.orders().len();
    save(&hub, &owner, settings(true)).await;
    assert_eq!(hub.authority.orders().len(), orders);
    // ...and Renew now asks anyway.
    let renew = hub.post_json(RENEW, Some(&owner), json!({})).await;
    assert_eq!(renew.status, StatusCode::OK, "{}", renew.text());
    assert_eq!(hub.authority.orders().len(), orders + 1);
    let audited: Vec<_> = actions(&hub).await.into_iter().map(|e| e.action).collect();
    assert_eq!(
        audited,
        [
            "certificate.issue",
            "certificate.settings",
            "certificate.issue",
            "certificate.settings",
            "certificate.issue_failed",
            "certificate.issue_failed",
            "certificate.issue_failed",
            "certificate.settings",
        ]
    );
}

#[tokio::test]
async fn turning_it_off_goes_back_to_self_signed_with_the_same_key() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let cert = hub.with_https().await;
    save(&hub, &owner, settings(true)).await;
    let (_, spki) = served(&hub, &cert);

    let off = save(&hub, &owner, settings(false)).await;
    assert_eq!(off.status, StatusCode::OK);
    let (details, again) = served(&hub, &cert);
    assert!(details.self_signed);
    assert_eq!(again, spki, "the pin holds");
    assert_eq!(hub.authority.orders().len(), 1);

    let renew = hub.post_json(RENEW, Some(&owner), json!({})).await;
    assert_eq!(renew.status, StatusCode::CONFLICT);
    assert_eq!(renew.error_code(), "acme_off");
}

#[tokio::test]
async fn settings_are_checked_against_how_the_hub_listens() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    // TLS is off in test hubs until `with_https`: a proxy handles certificates.
    let refused = save(&hub, &owner, settings(true)).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(refused.error_code(), "invalid_certificate_settings");
    assert!(refused.text().contains("proxy"), "{}", refused.text());
    assert_eq!(
        save(&hub, &owner, settings(false)).await.status,
        StatusCode::OK,
        "off can always be saved"
    );

    hub.with_https().await;
    let mut ip = settings(true);
    ip["domains"] = json!(["192.0.2.10"]);
    assert_eq!(save(&hub, &owner, ip).await.status, StatusCode::BAD_REQUEST);
    let mut no_terms = settings(true);
    no_terms["terms_agreed"] = json!(false);
    assert_eq!(
        save(&hub, &owner, no_terms).await.status,
        StatusCode::BAD_REQUEST
    );

    // HTTPS only (no plain-HTTP port): HTTP-01 can't be answered.
    {
        let mut config = hub.state.config.write().await;
        config.server.tls.https_port = config.server.port;
    }
    let mut http = settings(true);
    http["challenge"] = json!("http");
    let refused = save(&hub, &owner, http).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.text().contains("TLS-ALPN"), "{}", refused.text());
    assert!(hub.authority.orders().is_empty());

    let viewer = hub.login_as(Role::Viewer).await;
    assert_eq!(
        save(&hub, &viewer, settings(false)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_rotation_gets_its_next_certificate_before_the_switch() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let cert = hub.with_https().await;
    save(&hub, &owner, settings(true)).await;

    let started = hub
        .post_json(
            "/api/admin/hub-keys/rotation",
            Some(&owner),
            json!({ "tls": true, "key": false, "window_days": 14 }),
        )
        .await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.text());
    let files = KeyFiles::of(&hub.state.config.read().await.clone());
    let next_pin = hub.state.hub_pins.get().next.unwrap().tls_spki_sha256;

    // The next certificate is self-signed until the hub looks.
    assert_eq!(
        hub_lib::acme::check_once(&hub.state).await.unwrap(),
        Outcome::Issued {
            count: 1,
            serving: false
        },
        "only the next one was due"
    );
    let next = std::fs::read(&files.tls_cert_next).unwrap();
    assert!(!tls::certificate_details(&next).unwrap().self_signed);
    assert_eq!(Some(tls::cert_spki_sha256(&next).unwrap()), next_pin);
    let page = overview(&hub).await;
    assert!(!page.next_certificate.unwrap().self_signed);

    let switched = hub
        .post_json(
            "/api/admin/hub-keys/rotation/switch",
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(switched.status, StatusCode::OK);
    let (details, spki) = served(&hub, &cert);
    assert!(
        !details.self_signed,
        "a trusted certificate from the switch on"
    );
    assert_eq!(Some(spki), next_pin);
    assert_eq!(overview(&hub).await.next_certificate, None);
}

#[tokio::test]
async fn the_hub_answers_an_http_challenge_while_an_order_waits() {
    let hub = TestHub::new(&[]).await;
    let path = "/.well-known/acme-challenge/t0ken";
    assert_eq!(hub.get(path, None).await.status, StatusCode::NOT_FOUND);

    hub.state
        .tls
        .challenges()
        .add_http("t0ken", "t0ken.thumbprint");
    let answer = hub.get(path, None).await;
    assert_eq!(answer.status, StatusCode::OK, "no session needed");
    assert_eq!(answer.text(), "t0ken.thumbprint");
    assert_eq!(
        hub.get("/.well-known/acme-challenge/other", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    hub.state.tls.challenges().clear();
    assert_eq!(hub.get(path, None).await.status, StatusCode::NOT_FOUND);
}
