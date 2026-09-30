//! The audit log (M1-06, D-46): actions leave entries with who, when, from
//! where; Admins read and export them.
#![cfg(feature = "ssr")]

mod support;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use hub_lib::model::{AuditEntry, Role};
use serde_json::json;
use support::*;

async fn audit(hub: &TestHub, query: &str) -> Vec<AuditEntry> {
    hub.get_json(&format!("/api/audit{query}")).await
}

fn actions(entries: &[AuditEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.action.as_str()).collect()
}

#[tokio::test]
async fn logins_and_failed_logins_are_recorded_with_the_client_address() {
    let hub = TestHub::new(&[]).await;
    let body = json!({ "username": OWNER, "password": "wrong-password" });
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-blentinel", "1")
        .body(Body::from(body.to_string()))
        .unwrap();
    let addr = std::net::SocketAddr::from(([192, 0, 2, 7], 50000));
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(addr));
    assert_eq!(hub.send(request).await.status, StatusCode::UNAUTHORIZED);

    let entries = audit(&hub, "?action=login").await; // get_json logs in as the Owner
    let failed = entries.iter().find(|e| e.action == "login.failed").unwrap();
    assert_eq!(
        failed.username.as_deref(),
        Some(OWNER),
        "the name that was tried"
    );
    assert_eq!(failed.actor_ip.as_deref(), Some("192.0.2.7"));
    assert!(entries.iter().any(|e| e.action == "login.success"));
}

#[tokio::test]
async fn user_management_is_recorded_without_secrets() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;
    let created = hub
        .post_json(
            "/api/users",
            Some(&owner),
            json!({ "username": "tech", "display_name": "", "role": "technician" }),
        )
        .await;
    let user: hub_lib::model::TemporaryPassword = created.json();
    let id = &user.user.id;
    hub.post_json(
        &format!("/api/users/{id}/reset-password"),
        Some(&owner),
        json!({}),
    )
    .await;
    hub.post_json(&format!("/api/users/{id}/logout"), Some(&owner), json!({}))
        .await;

    let entries = audit(&hub, "?action=user").await;
    assert_eq!(
        actions(&entries),
        ["user.force_logout", "user.reset_password", "user.create"]
    );
    let create = &entries[2];
    assert_eq!(create.username.as_deref(), Some(OWNER), "who did it");
    assert_eq!(create.target_id.as_deref(), Some(id.as_str()));
    let details = create.details.as_deref().unwrap();
    assert!(details.contains("technician"), "{details}");
    let everything = serde_json::to_string(&audit(&hub, "").await).unwrap();
    assert!(
        !everything.contains(&user.temporary_password),
        "temporary passwords are never logged"
    );
}

#[tokio::test]
async fn company_silence_and_data_actions_are_recorded_with_their_company() {
    let probe = TestProbe::new("hq");
    let hub = TestHub::new(&[&probe]).await;
    hub.report_now(&probe, "acme", vec![ping("gw", "10.0.0.1", true)])
        .await;
    let owner = hub.login().await;
    hub.post_json(
        "/api/companies/acme",
        Some(&owner),
        json!({ "name": "ACME" }),
    )
    .await;
    let silence = json!({
        "check_id": check_id(&probe.id, "gw"), "reason": "maintenance", "duration_hours": 1,
    });
    let created: serde_json::Value = hub
        .post_json("/api/silence", Some(&owner), silence)
        .await
        .json();
    let silence_id = created["silence_id"].as_i64().unwrap();
    hub.post_json(
        "/api/silence/delete",
        Some(&owner),
        json!({ "silence_id": silence_id }),
    )
    .await;
    hub.post_json(
        "/api/admin/delete-probe-data",
        Some(&owner),
        json!({ "probe_id": probe.id }),
    )
    .await;

    let entries = audit(&hub, "?target=acme").await;
    assert_eq!(
        actions(&entries),
        [
            "data.delete_probe",
            "silence.delete",
            "silence.create",
            "company.update",
        ]
    );
    assert!(
        entries
            .iter()
            .all(|e| e.company_id.as_deref() == Some("acme"))
    );
    assert!(
        entries[2]
            .details
            .as_deref()
            .unwrap()
            .contains("maintenance")
    );
}

#[tokio::test]
async fn the_console_is_recorded_as_the_hub() {
    let hub = TestHub::new(&[]).await;
    let store = hub.state.storage.get();
    let command = hub_lib::args::UserCommand::ResetPassword {
        username: OWNER.into(),
    };
    hub_lib::user_cli::run(store.as_ref(), &command, hub.now())
        .await
        .unwrap();
    // The Owner's sessions ended; log in with a new user to read the log.
    let entries: Vec<AuditEntry> = {
        let cookie = hub.login_as(Role::Admin).await;
        hub.get("/api/audit?action=user", Some(&cookie))
            .await
            .json()
    };
    assert_eq!(entries[0].action, "user.reset_password");
    assert_eq!(entries[0].username, None);
    assert!(entries[0].details.as_deref().unwrap().contains("hub user"));
}

#[tokio::test]
async fn filters_are_validated_and_csv_is_exported() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let bad = hub.get("/api/audit?from=yesterday", Some(&owner)).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.error_code(), "invalid_query");

    let csv = hub.get("/api/audit.csv?action=login", Some(&owner)).await;
    assert_eq!(csv.status, StatusCode::OK);
    assert!(
        csv.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/csv")
    );
    assert!(
        csv.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("blentinel-audit-")
    );
    let text = csv.text();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("id,at,user,ip,action,target_type,target_id,company,details")
    );
    assert!(lines.next().unwrap().contains("login.success"));
}

#[tokio::test]
async fn only_admins_read_the_log() {
    let hub = TestHub::new(&[]).await;
    for role in [Role::Technician, Role::Viewer] {
        let cookie = hub.login_as(role).await;
        let response = hub.get("/api/audit", Some(&cookie)).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{role:?}");
    }
}
