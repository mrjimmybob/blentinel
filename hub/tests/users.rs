//! User management and first-run setup (proposal 0001, D-42).
#![cfg(feature = "ssr")]

mod support;

use axum::http::StatusCode;
use hub_lib::model::{Role, TemporaryPassword, UserInfo};
use serde_json::{Value, json};
use support::*;

const DASHBOARD: &str = "/api/dashboard/companies";

async fn create(hub: &TestHub, cookie: &str, username: &str, role: &str) -> TestResponse {
    let body = json!({ "username": username, "display_name": "", "role": role });
    hub.post_json("/api/users", Some(cookie), body).await
}

async fn user_id(hub: &TestHub, username: &str) -> String {
    let store = hub.state.storage.get();
    store
        .user_by_username(username)
        .await
        .unwrap()
        .unwrap()
        .info
        .id
}

fn update(role: &str, is_active: bool) -> Value {
    json!({ "display_name": "Edited", "role": role, "is_active": is_active })
}

// ---------------------------------------------------------------------------
// Creating users
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_user_gets_a_temporary_password_and_must_change_it() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let response = create(&hub, &owner, "tech", "technician").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let created: TemporaryPassword = response.json();
    assert_eq!(created.user.role, Role::Technician);
    assert_eq!(
        created.user.display_name, "tech",
        "defaults to the username"
    );
    assert!(created.user.must_change_password);
    assert!(created.temporary_password.len() >= 12);

    let login = hub.try_login("tech", &created.temporary_password).await;
    assert_eq!(login.status, StatusCode::OK);
    let cookie = session_cookie(&login.headers).unwrap();
    let blocked = hub.get(DASHBOARD, Some(&cookie)).await;
    assert_eq!(blocked.error_code(), "password_change_required");

    let change = json!({
        "current_password": created.temporary_password,
        "new_password": "a brand new password",
    });
    let response = hub
        .post_json("/api/me/password", Some(&cookie), change)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        hub.get(DASHBOARD, Some(&cookie)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        hub.try_login("tech", "a brand new password").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn usernames_are_unique_and_validated() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let taken = create(&hub, &owner, "OWNER", "viewer").await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    assert_eq!(taken.error_code(), "username_taken");
    let invalid = create(&hub, &owner, "has space", "viewer").await;
    assert_eq!(invalid.error_code(), "invalid_username");
}

#[tokio::test]
async fn users_limited_to_some_companies_are_refused_until_m1_04() {
    let hub = TestHub::new(&[]).await;
    let owner = hub.login().await;

    let client = create(&hub, &owner, "client", "client").await;
    assert_eq!(client.status, StatusCode::BAD_REQUEST);
    assert_eq!(client.error_code(), "not_supported");
    let body = json!({
        "username": "scoped", "display_name": "", "role": "viewer",
        "all_companies": false, "companies": ["acme"],
    });
    let scoped = hub.post_json("/api/users", Some(&owner), body).await;
    assert_eq!(scoped.error_code(), "not_supported");
}

#[tokio::test]
async fn only_owners_manage_owners() {
    let hub = TestHub::new(&[]).await;
    let admin = hub.login_as(Role::Admin).await;
    let owner_id = user_id(&hub, OWNER).await;

    let create_owner = create(&hub, &admin, "boss", "owner").await;
    assert_eq!(create_owner.status, StatusCode::FORBIDDEN);
    for path in [
        format!("/api/users/{owner_id}/reset-password"),
        format!("/api/users/{owner_id}/logout"),
    ] {
        let response = hub.post_json(&path, Some(&admin), json!({})).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{path}");
    }
    let edit_owner = hub
        .post_json(
            &format!("/api/users/{owner_id}"),
            Some(&admin),
            update("viewer", true),
        )
        .await;
    assert_eq!(edit_owner.status, StatusCode::FORBIDDEN);

    // An Admin may manage other non-Owners, including Admins.
    assert_eq!(
        create(&hub, &admin, "admin2", "admin").await.status,
        StatusCode::OK
    );
}

// ---------------------------------------------------------------------------
// Editing users
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_last_active_owner_cannot_be_demoted_or_deactivated() {
    let hub = TestHub::new(&[]).await;
    hub.seed_user("owner2", Role::Owner).await;
    let owner = hub.login().await;
    let id = user_id(&hub, "owner2").await;

    let demote = hub
        .post_json(
            &format!("/api/users/{id}"),
            Some(&owner),
            update("admin", true),
        )
        .await;
    assert_eq!(demote.status, StatusCode::OK, "another Owner remains");

    let self_id = user_id(&hub, OWNER).await;
    let last = hub
        .post_json(
            &format!("/api/users/{self_id}"),
            Some(&owner),
            update("admin", true),
        )
        .await;
    assert_eq!(last.status, StatusCode::CONFLICT);
    assert_eq!(last.error_code(), "last_owner");
}

#[tokio::test]
async fn nobody_deactivates_themselves() {
    let hub = TestHub::new(&[]).await;
    hub.seed_user("owner2", Role::Owner).await;
    let owner = hub.login().await;
    let self_id = user_id(&hub, OWNER).await;

    let response = hub
        .post_json(
            &format!("/api/users/{self_id}"),
            Some(&owner),
            update("owner", false),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn deactivating_a_user_ends_their_sessions_and_blocks_login() {
    let hub = TestHub::new(&[]).await;
    let viewer = hub.login_as(Role::Viewer).await;
    let owner = hub.login().await;
    let id = user_id(&hub, "viewer").await;

    let response = hub
        .post_json(
            &format!("/api/users/{id}"),
            Some(&owner),
            update("viewer", false),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let user: UserInfo = response.json();
    assert!(!user.is_active);

    assert_eq!(
        hub.get(DASHBOARD, Some(&viewer)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        hub.try_login("viewer", PASSWORD).await.error_code(),
        "invalid_credentials"
    );
}

#[tokio::test]
async fn a_role_change_applies_to_existing_sessions() {
    let hub = TestHub::new(&[]).await;
    let tech = hub.login_as(Role::Technician).await;
    let owner = hub.login().await;
    let id = user_id(&hub, "technician").await;
    let silence = json!({ "resource_key": "k", "reason": "test", "duration_hours": 1 });
    assert_eq!(
        hub.post_json("/api/silence", Some(&tech), silence.clone())
            .await
            .status,
        StatusCode::OK
    );

    hub.post_json(
        &format!("/api/users/{id}"),
        Some(&owner),
        update("viewer", true),
    )
    .await;

    let response = hub.post_json("/api/silence", Some(&tech), silence).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_password_reset_logs_the_user_out_and_forces_a_change() {
    let hub = TestHub::new(&[]).await;
    let viewer = hub.login_as(Role::Viewer).await;
    let owner = hub.login().await;
    let id = user_id(&hub, "viewer").await;

    let response = hub
        .post_json(
            &format!("/api/users/{id}/reset-password"),
            Some(&owner),
            json!({}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let reset: TemporaryPassword = response.json();

    assert_eq!(
        hub.get(DASHBOARD, Some(&viewer)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        hub.try_login("viewer", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    let login = hub.try_login("viewer", &reset.temporary_password).await;
    let user: UserInfo = login.json();
    assert!(user.must_change_password);
}

#[tokio::test]
async fn force_logout_ends_every_session_of_a_user() {
    let hub = TestHub::new(&[]).await;
    let first = hub.login_as(Role::Viewer).await;
    let second = hub.login_as(Role::Viewer).await;
    let owner = hub.login().await;
    let id = user_id(&hub, "viewer").await;

    let response = hub
        .post_json(&format!("/api/users/{id}/logout"), Some(&owner), json!({}))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    for cookie in [first, second] {
        assert_eq!(
            hub.get(DASHBOARD, Some(&cookie)).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn the_user_list_shows_everyone_without_secrets() {
    let hub = TestHub::new(&[]).await;
    hub.seed_user("viewer", Role::Viewer).await;

    let response = hub.get("/api/users", Some(&hub.login().await)).await;
    let users: Vec<UserInfo> = response.json();

    let names: Vec<_> = users.iter().map(|u| u.username.as_str()).collect();
    assert_eq!(names, ["owner", "viewer"]);
    assert!(!response.text().contains("argon2"));
}

// ---------------------------------------------------------------------------
// Own account
// ---------------------------------------------------------------------------

#[tokio::test]
async fn changing_your_password_needs_the_current_one_and_ends_other_sessions() {
    let hub = TestHub::new(&[]).await;
    let here = hub.login().await;
    let elsewhere = hub.login().await;

    let wrong = json!({ "current_password": "nope", "new_password": "long enough password" });
    let response = hub.post_json("/api/me/password", Some(&here), wrong).await;
    assert_eq!(response.error_code(), "wrong_password");
    let weak = json!({ "current_password": PASSWORD, "new_password": "short" });
    let response = hub.post_json("/api/me/password", Some(&here), weak).await;
    assert_eq!(response.error_code(), "weak_password");

    let good = json!({ "current_password": PASSWORD, "new_password": "long enough password" });
    let response = hub.post_json("/api/me/password", Some(&here), good).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(hub.get(DASHBOARD, Some(&here)).await.status, StatusCode::OK);
    assert_eq!(
        hub.get(DASHBOARD, Some(&elsewhere)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn logging_out_other_sessions_keeps_this_one() {
    let hub = TestHub::new(&[]).await;
    let here = hub.login().await;
    let elsewhere = hub.login().await;

    let response = hub
        .post_json("/api/me/logout-others", Some(&here), json!({}))
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(hub.get(DASHBOARD, Some(&here)).await.status, StatusCode::OK);
    assert_eq!(
        hub.get(DASHBOARD, Some(&elsewhere)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------
// First run
// ---------------------------------------------------------------------------

fn setup_body(code: &str) -> Value {
    json!({
        "code": code, "username": "founder", "display_name": "The Founder",
        "password": "founder-password",
    })
}

#[tokio::test]
async fn setup_creates_the_first_owner_once_with_the_code() {
    let hub = TestHub::without_users().await;
    let code_file = hub.dir().join("setup.code");
    let code = std::fs::read_to_string(&code_file).expect("setup code written");
    let code = code.trim();
    assert_eq!(code.len(), 8);

    let status: Value = hub.get("/api/setup", None).await.json();
    assert_eq!(status["needed"], true);

    let wrong = hub
        .post_json("/api/setup", None, setup_body("WRONG123"))
        .await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.error_code(), "wrong_setup_code");

    // Codes are case-insensitive, to be easy to type.
    let response = hub
        .post_json("/api/setup", None, setup_body(&code.to_lowercase()))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let owner: UserInfo = response.json();
    assert_eq!(owner.role, Role::Owner);
    assert_eq!(owner.display_name, "The Founder");
    let cookie = session_cookie(&response.headers).expect("logged in right away");
    assert_eq!(
        hub.get(DASHBOARD, Some(&cookie)).await.status,
        StatusCode::OK
    );
    assert!(!code_file.exists(), "the code file is deleted once used");

    let status: Value = hub.get("/api/setup", None).await.json();
    assert_eq!(status["needed"], false);
    let again = hub.post_json("/api/setup", None, setup_body(code)).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.error_code(), "already_set_up");
}

#[tokio::test]
async fn setup_enforces_the_password_rule() {
    let hub = TestHub::without_users().await;
    let code = std::fs::read_to_string(hub.dir().join("setup.code")).unwrap();
    let mut body = setup_body(code.trim());
    body["password"] = json!("short");

    let response = hub.post_json("/api/setup", None, body).await;

    assert_eq!(response.error_code(), "weak_password");
    let status: Value = hub.get("/api/setup", None).await.json();
    assert_eq!(status["needed"], true, "still not set up");
}

#[tokio::test]
async fn a_hub_with_users_has_no_setup_code() {
    let hub = TestHub::new(&[]).await;
    assert!(!hub.dir().join("setup.code").exists());
    let response = hub
        .post_json("/api/setup", None, setup_body("ANYTHING"))
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// hub user … (the console path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_console_can_create_users_and_reset_passwords() {
    use hub_lib::args::UserCommand;
    let hub = TestHub::new(&[]).await;
    let store = hub.state.storage.get();
    let run = |command: UserCommand| {
        let store = store.clone();
        let now = hub.now();
        async move { hub_lib::user_cli::run(store.as_ref(), &command, now).await }
    };

    let created = run(UserCommand::Create {
        username: "rescue".into(),
        role: Role::Owner,
        display_name: None,
    })
    .await
    .unwrap();
    assert!(created.contains("Temporary password:"), "{created}");

    let viewer = hub.login().await;
    let output = run(UserCommand::ResetPassword {
        username: OWNER.into(),
    })
    .await
    .unwrap();
    let password = output
        .lines()
        .find_map(|l| l.strip_prefix("Temporary password: "))
        .unwrap();
    assert_eq!(
        hub.get(DASHBOARD, Some(&viewer)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(hub.try_login(OWNER, password).await.status, StatusCode::OK);

    let list = run(UserCommand::List).await.unwrap();
    assert!(list.contains("rescue") && list.contains("owner"), "{list}");
    assert!(
        run(UserCommand::ResetPassword {
            username: "ghost".into()
        })
        .await
        .is_err()
    );
}
