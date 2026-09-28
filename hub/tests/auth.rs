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

    // /api/report authenticates cryptographically: garbage is 401 from decryption, not from session auth.
    assert_eq!(
        hub.post_bytes("/api/report", vec![0; 64]).await.status,
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------
// Cookies over HTTPS, and slowing down guessing (M1-05, D-43)
// ---------------------------------------------------------------------------

/// A login request as a listener would deliver it: `https` marks the HTTPS
/// listener, `ip` is the client's address.
fn login_request(username: &str, password: &str, https: bool, ip: [u8; 4]) -> Request<Body> {
    use hub_lib::auth::Transport;
    let body = serde_json::json!({ "username": username, "password": password });
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-blentinel", "1")
        .body(Body::from(body.to_string()))
        .unwrap();
    let transport = if https {
        Transport::Https
    } else {
        Transport::Http
    };
    request.extensions_mut().insert(transport);
    let addr = std::net::SocketAddr::from((ip, 50000));
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(addr));
    request
}

const CLIENT: [u8; 4] = [192, 0, 2, 1];

fn set_cookie(response: &TestResponse) -> String {
    response.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn the_cookie_is_secure_exactly_when_the_request_came_over_https() {
    let hub = TestHub::new(&[]).await;

    let https = hub.send(login_request(OWNER, PASSWORD, true, CLIENT)).await;
    assert!(
        set_cookie(&https).contains("; Secure"),
        "{}",
        set_cookie(&https)
    );
    let http = hub
        .send(login_request(OWNER, PASSWORD, false, CLIENT))
        .await;
    assert!(
        !set_cookie(&http).contains("Secure"),
        "plain HTTP (TLS turned off)"
    );

    // Behind a TLS proxy the hub sees HTTP, but the browser used HTTPS.
    hub.state.config.write().await.server.behind_proxy = true;
    let proxied = hub
        .send(login_request(OWNER, PASSWORD, false, CLIENT))
        .await;
    assert!(set_cookie(&proxied).contains("; Secure"));
}

#[tokio::test]
async fn repeated_wrong_passwords_lock_the_username_for_that_client() {
    let hub = TestHub::new(&[]).await;
    let attempt = |password: &'static str, ip| hub.send(login_request(OWNER, password, false, ip));

    for _ in 0..6 {
        let response = attempt("wrong-password", CLIENT).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
    // Locked: even the right password is refused, without being checked.
    let locked = attempt(PASSWORD, CLIENT).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(locked.error_code(), "too_many_attempts");
    assert_eq!(locked.headers[header::RETRY_AFTER], "30");
    // Another client isn't affected.
    assert_eq!(
        attempt(PASSWORD, [192, 0, 2, 2]).await.status,
        StatusCode::OK
    );

    hub.clock.advance(Duration::seconds(30));
    assert_eq!(attempt(PASSWORD, CLIENT).await.status, StatusCode::OK);
    // The success cleared the count: one more failure doesn't lock.
    attempt("wrong-password", CLIENT).await;
    assert_eq!(attempt(PASSWORD, CLIENT).await.status, StatusCode::OK);
}

#[tokio::test]
async fn trying_many_usernames_from_one_client_locks_the_client() {
    let hub = TestHub::new(&[]).await;

    for n in 0..20 {
        let response = hub
            .send(login_request(
                &format!("guess{n}"),
                "password123",
                false,
                CLIENT,
            ))
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "attempt {n}");
    }
    let response = hub
        .send(login_request(OWNER, PASSWORD, false, CLIENT))
        .await;
    assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn wrong_setup_codes_are_limited_too() {
    let hub = TestHub::without_users().await;
    let body = serde_json::json!({
        "code": "WRONG123", "username": "x", "display_name": "", "password": "long-enough-password",
    });
    for _ in 0..6 {
        let response = hub.post_json("/api/setup", None, body.clone()).await;
        assert_eq!(response.error_code(), "wrong_setup_code");
    }
    let response = hub.post_json("/api/setup", None, body).await;
    assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);
}
