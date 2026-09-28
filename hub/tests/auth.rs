//! Login, logout and sessions (proposal 0001, D-42). Which routes need which
//! permission: `access.rs`. Managing users and first-run setup: `users.rs`.
#![cfg(feature = "ssr")]

mod support;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use chrono::Duration;
use hub_lib::model::{Role, UserInfo};
use support::*;
use tower::ServiceExt;

const DASHBOARD: &str = "/api/dashboard/companies";

async fn status(hub: &TestHub, path: &str, cookie: &str) -> StatusCode {
    hub.get(path, Some(cookie)).await.status
}

#[tokio::test]
async fn login_returns_the_user_and_an_http_only_strict_cookie() {
    let hub = TestHub::new(&[]).await;

    let response = hub.try_login(OWNER, PASSWORD).await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let user: UserInfo = response.json();
    assert_eq!(user.username, OWNER);
    assert_eq!(user.role, Role::Owner);
    assert!(user.last_login_at.is_some());
    let set_cookie = response.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.starts_with("blentinel_session="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    assert!(
        set_cookie.contains("Max-Age=604800"),
        "7 days: {set_cookie}"
    );
    assert!(
        !response.text().contains("argon2"),
        "the password hash never leaves the hub"
    );
}

#[tokio::test]
async fn usernames_are_case_insensitive() {
    let hub = TestHub::new(&[]).await;
    assert_eq!(
        hub.try_login("OWNER", PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn wrong_password_and_unknown_user_get_the_same_answer() {
    let hub = TestHub::new(&[]).await;

    let wrong = hub.try_login(OWNER, "not-the-password").await;
    let unknown = hub.try_login("nobody", PASSWORD).await;

    for response in [&wrong, &unknown] {
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.error_code(), "invalid_credentials");
        assert!(response.headers.get(header::SET_COOKIE).is_none());
    }
    assert_eq!(wrong.text(), unknown.text());
}

#[tokio::test]
async fn the_session_identifies_the_user() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login_as(Role::Viewer).await;

    let me: UserInfo = hub.get("/api/auth/me", Some(&cookie)).await.json();

    assert_eq!(me.username, "viewer");
    assert_eq!(me.role, Role::Viewer);
}

#[tokio::test]
async fn logout_ends_the_session_and_clears_the_cookie() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;
    let other = hub.login().await;
    assert_eq!(status(&hub, DASHBOARD, &cookie).await, StatusCode::OK);

    let response = hub
        .post_json("/api/auth/logout", Some(&cookie), serde_json::json!({}))
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert!(
        response.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        status(&hub, DASHBOARD, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&hub, DASHBOARD, &other).await,
        StatusCode::OK,
        "only this session"
    );
}

#[tokio::test]
async fn sessions_are_stored_hashed_and_survive_a_restart() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;
    let token = cookie.trim_start_matches("blentinel_session=");

    let stored: Vec<String> = sqlx::query_scalar("SELECT id_hash FROM sessions")
        .fetch_all(hub.pool())
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_ne!(stored[0], token, "the raw cookie value is never stored");

    // A new router over the same database: what the hub sees after a restart.
    let request = Request::builder()
        .method(Method::GET)
        .uri(DASHBOARD)
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let response = hub.restarted_router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn sessions_expire_after_12_idle_hours() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    hub.clock.advance(Duration::hours(11));
    assert_eq!(status(&hub, DASHBOARD, &cookie).await, StatusCode::OK);
    hub.clock.advance(Duration::hours(11));
    assert_eq!(
        status(&hub, DASHBOARD, &cookie).await,
        StatusCode::OK,
        "each request restarts the idle timer"
    );
    hub.clock.advance(Duration::hours(13));
    assert_eq!(
        status(&hub, DASHBOARD, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn sessions_end_7_days_after_login_however_active() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    for _ in 0..16 {
        hub.clock.advance(Duration::hours(10));
        assert_eq!(status(&hub, DASHBOARD, &cookie).await, StatusCode::OK);
    }
    hub.clock.advance(Duration::hours(10)); // 170 h after login
    assert_eq!(
        status(&hub, DASHBOARD, &cookie).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn the_session_cookie_is_found_among_other_cookies() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    let combined = format!("theme=dark; {cookie}; other=1");
    assert_eq!(status(&hub, DASHBOARD, &combined).await, StatusCode::OK);
}

#[tokio::test]
async fn probe_endpoints_do_not_require_a_session() {
    let hub = TestHub::new(&[]).await;

    assert_eq!(hub.get("/api/handshake", None).await.status, StatusCode::OK);
    // /api/report authenticates cryptographically: garbage is 401 from decryption, not from session auth.
    assert_eq!(
        hub.post_bytes("/api/report", vec![0; 64]).await.status,
        StatusCode::UNAUTHORIZED
    );
}
