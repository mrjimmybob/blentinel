//! Enrollment (M2-01, D-47): "Add probe" gives a code; a probe redeems it
//! with `POST /api/enroll` and from then on reports for the company and site
//! the hub chose, whatever its reports say.
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use chrono::Duration;
use common::enrollment::{EnrollRequest, EnrollResponse, EnrollmentCode, SignedEnrollRequest};
use hub_lib::model::{CreatedCode, EnrollmentCodeInfo, ProbeInfo};
use serde_json::{Value, json};
use support::*;

/// A hub with company `acme` (site `hq`) and the Owner's session.
async fn hub_with_acme() -> (TestHub, String) {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    hub.post_json(
        "/api/companies",
        Some(&owner),
        json!({ "id": "acme", "name": "ACME" }),
    )
    .await;
    hub.post_json(
        "/api/companies/acme/sites",
        Some(&owner),
        json!({ "id": "hq", "name": "Headquarters" }),
    )
    .await;
    (hub, owner)
}

async fn create_code(hub: &TestHub, cookie: &str, body: Value) -> TestResponse {
    hub.post_json("/api/enrollment-codes", Some(cookie), body)
        .await
}

async fn new_code(hub: &TestHub, cookie: &str, max_uses: u32) -> CreatedCode {
    let body = json!({
        "company_id": "acme", "site_id": "hq", "max_uses": max_uses,
        "hub_url": "https://hub.example.com:3443",
    });
    let response = create_code(hub, cookie, body).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    response.json()
}

/// What `probe enroll <code>` sends, from `probe`.
fn enroll_request(hub: &TestHub, probe: &TestProbe, code: &str) -> Vec<u8> {
    let code = EnrollmentCode::decode(code).expect("a valid code");
    let request = EnrollRequest {
        token_id: hex::encode(code.token_id),
        secret: hex::encode(code.secret),
        probe_id: probe.id.clone(),
        hostname: format!("{}-host", probe.name),
        os: "linux".into(),
        arch: "x86_64".into(),
        probe_version: "0.1.0".into(),
        timestamp: hub.now().to_rfc3339(),
    };
    signed_and_sealed(hub, probe, request)
}

fn signed_and_sealed(hub: &TestHub, probe: &TestProbe, request: EnrollRequest) -> Vec<u8> {
    let signature = hex::encode(probe.sign_bytes(&serde_json::to_vec(&request).unwrap()));
    seal(
        &SignedEnrollRequest { request, signature },
        &hub.public_key(),
    )
}

async fn enroll(hub: &TestHub, probe: &TestProbe, code: &str) -> TestResponse {
    hub.post_bytes("/api/enroll", enroll_request(hub, probe, code))
        .await
}

#[tokio::test]
async fn a_probe_enrolls_with_a_code_and_the_hub_decides_its_company() {
    let (hub, owner) = hub_with_acme().await;
    let created = new_code(&hub, &owner, 1).await;

    // The code tells the probe everything it needs to trust this hub.
    let code = EnrollmentCode::decode(&created.code).unwrap();
    assert_eq!(code.hub_key, hub.public_key());
    assert_eq!(code.hub_signing_key, hub.signing_key());
    assert_eq!(code.hub_url, "https://hub.example.com:3443");

    let probe = TestProbe::new("pc1");
    let response = enroll(&hub, &probe, &created.code).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let enrolled: EnrollResponse = response.json();
    assert_eq!(enrolled.probe_id, probe.id);
    assert_eq!(
        enrolled.name, "pc1-host",
        "the hostname, as no name was given"
    );
    assert_eq!(
        (enrolled.company_name.as_str(), enrolled.site_name.as_str()),
        ("ACME", "Headquarters")
    );

    // Its reports count for acme/hq, whatever company and site they claim (D-17).
    let report = probe.signed_report(
        "someone-else",
        hub.now(),
        vec![ping("gw", "10.0.0.1", true)],
    );
    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
    let probes: Vec<ProbeInfo> = hub.get_json("/api/probes").await;
    assert_eq!(probes[0].company_id, "acme");
    assert_eq!(probes[0].site_id, "hq");
    assert_eq!(probes[0].os.as_deref(), Some("linux"));
    assert!(probes[0].last_seen_at.is_some());
    let stored: String = sqlx::query_scalar("SELECT company_id FROM reports")
        .fetch_one(hub.pool())
        .await
        .unwrap();
    assert_eq!(stored, "acme");
    let companies: Vec<Value> = hub.get_json("/api/companies").await;
    assert_eq!(companies.len(), 1, "no company is created from a report");
}

#[tokio::test]
async fn codes_are_single_use_by_default_and_expire() {
    let (hub, owner) = hub_with_acme().await;
    let once = new_code(&hub, &owner, 1).await;
    assert_eq!(
        enroll(&hub, &TestProbe::new("a"), &once.code).await.status,
        StatusCode::OK
    );
    let reused = enroll(&hub, &TestProbe::new("b"), &once.code).await;
    assert_eq!(reused.status, StatusCode::FORBIDDEN);
    assert_eq!(reused.error_code(), "invalid_code");

    let twice = new_code(&hub, &owner, 2).await;
    for name in ["c", "d"] {
        assert_eq!(
            enroll(&hub, &TestProbe::new(name), &twice.code)
                .await
                .status,
            StatusCode::OK
        );
    }
    assert_eq!(
        enroll(&hub, &TestProbe::new("e"), &twice.code)
            .await
            .error_code(),
        "invalid_code"
    );

    let later = new_code(&hub, &owner, 1).await;
    hub.clock.advance(Duration::hours(25));
    let probe = TestProbe::new("f");
    let expired = enroll(&hub, &probe, &later.code).await;
    assert_eq!(
        expired.error_code(),
        "invalid_code",
        "valid 24 hours by default"
    );

    let codes: Vec<EnrollmentCodeInfo> = hub.get_json("/api/enrollment-codes").await;
    assert!(codes.is_empty(), "used-up and expired codes aren't listed");
}

#[tokio::test]
async fn a_revoked_code_or_a_wrong_secret_is_refused() {
    let (hub, owner) = hub_with_acme().await;
    let created = new_code(&hub, &owner, 5).await;
    let mut code = EnrollmentCode::decode(&created.code).unwrap();
    code.secret[0] ^= 1;
    let forged = enroll(&hub, &TestProbe::new("x"), &code.encode()).await;
    assert_eq!(forged.error_code(), "invalid_code");

    let codes: Vec<EnrollmentCodeInfo> = hub.get_json("/api/enrollment-codes").await;
    assert_eq!(codes.len(), 1);
    let revoke = format!("/api/enrollment-codes/{}/revoke", codes[0].id);
    assert_eq!(
        hub.post_json(&revoke, Some(&owner), json!({})).await.status,
        StatusCode::OK
    );
    let revoked = enroll(&hub, &TestProbe::new("y"), &created.code).await;
    assert_eq!(revoked.error_code(), "invalid_code");
}

#[tokio::test]
async fn the_request_must_be_signed_by_the_probe_and_timely() {
    let (hub, owner) = hub_with_acme().await;
    let created = new_code(&hub, &owner, 5).await;
    let code = EnrollmentCode::decode(&created.code).unwrap();
    let (probe, impostor) = (TestProbe::new("p"), TestProbe::new("i"));
    let request = |probe_id: &str, at: chrono::DateTime<chrono::Utc>| EnrollRequest {
        token_id: hex::encode(code.token_id),
        secret: hex::encode(code.secret),
        probe_id: probe_id.into(),
        hostname: "h".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        probe_version: "0.1.0".into(),
        timestamp: at.to_rfc3339(),
    };

    // Signed by one key, naming another: a probe can't enroll someone else's key.
    let sealed = signed_and_sealed(&hub, &impostor, request(&probe.id, hub.now()));
    let response = hub.post_bytes("/api/enroll", sealed).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.error_code(), "bad_signature");

    let late = signed_and_sealed(
        &hub,
        &probe,
        request(&probe.id, hub.now() - Duration::minutes(10)),
    );
    let response = hub.post_bytes("/api/enroll", late).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    let garbage = hub.post_bytes("/api/enroll", vec![0u8; 64]).await;
    assert_eq!(garbage.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_revoked_probe_is_refused_until_enrolled_again() {
    let (hub, owner) = hub_with_acme().await;
    let probe = TestProbe::new("pc1");
    let first = new_code(&hub, &owner, 1).await;
    enroll(&hub, &probe, &first.code).await;

    let revoke = format!("/api/probes/{}/revoke", probe.id);
    assert_eq!(
        hub.post_json(&revoke, Some(&owner), json!({})).await.status,
        StatusCode::OK
    );
    // The probe is told so in a signed answer, so it can stop reporting
    // (a plain 403 could come from anyone on the path).
    let report = probe.signed_report("acme", hub.now(), vec![]);
    let (status, ack) = hub.send_report_for_ack(&probe, &report).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(ack.expect("a signed ack").revoked);

    let again = new_code(&hub, &owner, 1).await;
    assert_eq!(
        enroll(&hub, &probe, &again.code).await.status,
        StatusCode::OK
    );
    hub.clock.advance(Duration::seconds(60));
    let report = probe.signed_report("acme", hub.now(), vec![]);
    assert_eq!(hub.send_report(&report).await, StatusCode::OK);
}

#[tokio::test]
async fn probes_can_be_renamed_moved_within_their_company_and_timed() {
    let (hub, owner) = hub_with_acme().await;
    hub.post_json(
        "/api/companies/acme/sites",
        Some(&owner),
        json!({ "id": "branch", "name": "Branch" }),
    )
    .await;
    let probe = TestProbe::new("pc1");
    let code = new_code(&hub, &owner, 1).await;
    enroll(&hub, &probe, &code.code).await;

    let path = format!("/api/probes/{}", probe.id);
    let moved = hub
        .post_json(
            &path,
            Some(&owner),
            json!({ "name": "Front desk", "site_id": "branch", "interval_secs": 30 }),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text());
    let probes: Vec<ProbeInfo> = hub.get_json("/api/probes").await;
    assert_eq!(
        (
            probes[0].name.as_str(),
            probes[0].site_name.as_str(),
            probes[0].interval_secs
        ),
        ("Front desk", "Branch", 30)
    );

    for bad in [
        json!({ "name": "x", "site_id": "nowhere", "interval_secs": 30 }),
        json!({ "name": "x", "site_id": "hq", "interval_secs": 1 }),
        json!({ "name": "x", "site_id": "hq", "interval_secs": 86_400 }),
    ] {
        let refused = hub.post_json(&path, Some(&owner), bad.clone()).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn code_requests_are_validated() {
    let (hub, owner) = hub_with_acme().await;
    let base = json!({ "company_id": "acme", "site_id": "hq", "hub_url": "https://h:3443" });
    let with = |key: &str, value: Value| {
        let mut body = base.clone();
        body[key] = value;
        body
    };
    for (body, expected) in [
        (with("site_id", json!("nowhere")), StatusCode::BAD_REQUEST),
        (with("max_uses", json!(0)), StatusCode::BAD_REQUEST),
        (with("valid_hours", json!(10_000)), StatusCode::BAD_REQUEST),
        (with("hub_url", json!("ftp://h")), StatusCode::BAD_REQUEST),
        (with("company_id", json!("nobody")), StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            create_code(&hub, &owner, body.clone()).await.status,
            expected,
            "{body}"
        );
    }
}

#[tokio::test]
async fn https_codes_pin_the_hubs_certificate() {
    let (hub, owner) = hub_with_acme().await;
    let cert = hub.dir().join("cert.pem");
    let key = hub.dir().join("key.pem");
    let (pem, _) = hub_lib::tls::load_or_create_tls_cert(
        cert.to_str().unwrap(),
        key.to_str().unwrap(),
        "127.0.0.1",
    )
    .unwrap();
    {
        let mut config = hub.state.config.write().await;
        config.server.tls.enabled = true;
        config.server.tls.cert_path = cert.to_string_lossy().into_owned();
    }
    let expected = hub_lib::tls::cert_sha256(&pem).unwrap();

    let https = new_code(&hub, &owner, 1).await;
    assert_eq!(
        EnrollmentCode::decode(&https.code).unwrap().tls_cert_sha256,
        Some(expected)
    );
    let body = json!({ "company_id": "acme", "site_id": "hq", "hub_url": "http://h:3000" });
    let http: CreatedCode = create_code(&hub, &owner, body).await.json();
    assert_eq!(
        EnrollmentCode::decode(&http.code).unwrap().tls_cert_sha256,
        None,
        "no pin for plain HTTP"
    );
}

#[tokio::test]
async fn enrollment_and_codes_are_audited() {
    let (hub, owner) = hub_with_acme().await;
    let code = new_code(&hub, &owner, 1).await;
    let probe = TestProbe::new("pc1");
    enroll(&hub, &probe, &code.code).await;

    let entries: Vec<hub_lib::model::AuditEntry> = hub.get_json("/api/audit?target=acme").await;
    let actions: Vec<&str> = entries.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(
        actions,
        [
            "probe.enroll",
            "code.create",
            "site.create",
            "company.create"
        ]
    );
    let enroll = &entries[0];
    assert_eq!(enroll.target_id.as_deref(), Some(probe.id.as_str()));
    assert!(enroll.details.as_deref().unwrap().contains("pc1-host"));
    assert!(
        !serde_json::to_string(&entries)
            .unwrap()
            .contains(&code.code),
        "the code itself is never logged"
    );
}
