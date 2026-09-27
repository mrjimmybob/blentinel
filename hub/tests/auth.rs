//! Admin-token login, sessions, and which routes require a session.
#![cfg(feature = "ssr")]

mod support;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use serde_json::json;
use support::*;

/// Every session-protected route. Keep in sync with `hub_lib::routes::api_router`
/// (M1 replaces this list with a check generated from the route table).
const PROTECTED: &[(&str, &str)] = &[
    ("POST", "/api/logout"),
    ("GET", "/api/dashboard/companies"),
    ("GET", "/api/company/acme/probes"),
    ("GET", "/api/company/acme/uptime"),
    ("GET", "/api/probe/abc/devices"),
    ("GET", "/api/admin/companies"),
    ("GET", "/api/admin/probes"),
    ("POST", "/api/admin/delete-company-data"),
    ("POST", "/api/admin/delete-probe-data"),
    ("POST", "/api/admin/remove-probe"),
    ("GET", "/api/admin/storage-info"),
    ("GET", "/api/admin/archives"),
    ("POST", "/api/admin/archive"),
    ("GET", "/api/silences"),
    ("POST", "/api/silence"),
    ("POST", "/api/silence/delete"),
    ("GET", "/api/archive/1/companies"),
    ("GET", "/api/archive/1/company/acme/probes"),
    ("GET", "/api/archive/1/company/acme/uptime"),
    ("GET", "/api/archive/1/probe/abc/devices"),
];

async fn login_with(hub: &TestHub, token: &str) -> TestResponse {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/login")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(token.to_string()))
        .unwrap();
    hub.send(request).await
}

fn request(method: &str, path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if method == "POST" {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(Body::from(json!({}).to_string())).unwrap();
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn correct_token_creates_an_http_only_session_cookie() {
    let hub = TestHub::new(&[]).await;

    let response = login_with(&hub, ADMIN_TOKEN).await;

    assert_eq!(response.status, StatusCode::OK);
    let set_cookie = response.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.starts_with("blentinel_session="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Max-Age=86400"));
}

#[tokio::test]
async fn surrounding_whitespace_in_the_token_is_ignored() {
    let hub = TestHub::new(&[]).await;

    let response = login_with(&hub, &format!("  {ADMIN_TOKEN}\n")).await;

    assert_eq!(response.status, StatusCode::OK);
}

#[tokio::test]
async fn wrong_or_empty_token_is_unauthorized() {
    let hub = TestHub::new(&[]).await;

    assert_eq!(
        login_with(&hub, "wrong").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(login_with(&hub, "").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(login_with(&hub, "wrong").await.error_code(), "unauthorized");
    assert!(login_with(&hub, "wrong")
        .await
        .headers
        .get(header::SET_COOKIE)
        .is_none());
}

#[tokio::test]
async fn every_protected_route_rejects_requests_without_a_session() {
    let hub = TestHub::new(&[]).await;

    for (method, path) in PROTECTED {
        let response = hub.send(request(method, path, None)).await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must require a session"
        );
        assert_eq!(response.error_code(), "unauthorized", "{method} {path}");
    }
}

#[tokio::test]
async fn every_protected_route_rejects_an_unknown_session() {
    let hub = TestHub::new(&[]).await;

    for (method, path) in PROTECTED {
        let response = hub
            .send(request(method, path, Some("blentinel_session=forged")))
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must reject a forged session"
        );
        assert_eq!(response.error_code(), "unauthorized", "{method} {path}");
    }
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

#[tokio::test]
async fn logout_invalidates_the_session_and_clears_the_cookie() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;
    assert_eq!(
        hub.get("/api/dashboard/companies", Some(&cookie))
            .await
            .status,
        StatusCode::OK
    );

    let response = hub
        .send(request("POST", "/api/logout", Some(&cookie)))
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert!(response.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        hub.get("/api/dashboard/companies", Some(&cookie))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn session_cookie_is_found_among_other_cookies() {
    let hub = TestHub::new(&[]).await;
    let cookie = hub.login().await;

    let combined = format!("theme=dark; {cookie}; other=1");
    let response = hub.get("/api/dashboard/companies", Some(&combined)).await;

    assert_eq!(response.status, StatusCode::OK);
}
