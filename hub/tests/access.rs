//! Authorization over **every** API route, generated from the route table
//! (`hub_lib::routes::access_table`), so a new route is covered automatically
//! (proposal 0001 §5, D-42).
#![cfg(feature = "ssr")]

mod support;

use std::collections::HashSet;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use hub_lib::auth::Access;
use hub_lib::model::{Role, authorize};
use hub_lib::routes::{RouteAccess, access_table};
use support::*;

/// A request to `route` with its path parameters filled in and, for POST, an
/// empty JSON body (enough to get past authorization, never a valid action)
/// and the CSRF header the web UI sends.
fn request(route: &RouteAccess, cookie: Option<&str>) -> Request<Body> {
    let mut request = request_without_csrf(route, cookie);
    if route.method == Method::POST {
        request
            .headers_mut()
            .insert("x-blentinel", "1".parse().unwrap());
    }
    request
}

fn request_without_csrf(route: &RouteAccess, cookie: Option<&str>) -> Request<Body> {
    let mut path = String::new();
    let mut in_param = false;
    for c in route.path.chars() {
        match c {
            '{' => in_param = true,
            '}' => {
                in_param = false;
                path.push('x');
            }
            c if !in_param => path.push(c),
            _ => {}
        }
    }
    let mut builder = Request::builder().method(route.method.clone()).uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if route.method == Method::POST {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(Body::from("{}")).unwrap();
    }
    builder.body(Body::empty()).unwrap()
}

fn session_routes() -> Vec<RouteAccess> {
    access_table()
        .into_iter()
        .filter(|r| matches!(r.access, Access::LoggedIn | Access::Perm(_)))
        .collect()
}

#[test]
fn the_table_lists_each_route_once_and_only_login_setup_and_probes_are_open() {
    let table = access_table();
    let unique: HashSet<_> = table.iter().map(|r| (r.method.clone(), r.path)).collect();
    assert_eq!(unique.len(), table.len(), "duplicate route");

    let open: Vec<_> = table
        .iter()
        .filter(|r| matches!(r.access, Access::Public | Access::Probe))
        .map(|r| format!("{} {}", r.method, r.path))
        .collect();
    assert_eq!(
        open,
        [
            "POST /api/report",
            "POST /api/enroll",
            "POST /api/auth/login",
            "GET /api/setup",
            "POST /api/setup",
        ],
        "a new unauthenticated route needs a deliberate decision"
    );
}

#[tokio::test]
async fn every_session_route_rejects_missing_and_forged_sessions() {
    let hub = TestHub::new(&[]).await;

    for route in session_routes() {
        for cookie in [None, Some("blentinel_session=forged")] {
            let response = hub.send(request(&route, cookie)).await;
            assert_eq!(
                response.status,
                StatusCode::UNAUTHORIZED,
                "{} {} with {cookie:?}",
                route.method,
                route.path
            );
            assert_eq!(response.error_code(), "unauthorized");
        }
    }
}

#[tokio::test]
async fn every_route_allows_exactly_the_roles_its_permission_names() {
    let hub = TestHub::new(&[]).await;
    // Client can't be created until company scoping exists (M1-04).
    let roles = [Role::Owner, Role::Admin, Role::Technician, Role::Viewer];
    for role in roles.into_iter().filter(|r| *r != Role::Owner) {
        hub.seed_user(role.as_str(), role).await;
    }

    for route in session_routes() {
        for role in roles {
            let username = if role == Role::Owner {
                OWNER
            } else {
                role.as_str()
            };
            // A fresh session each time: some routes end sessions (logout).
            let cookie = hub.session_for(username).await;
            let response = hub.send(request(&route, Some(&cookie))).await;
            let allowed = match route.access {
                Access::Perm(permission) => authorize(role, permission),
                _ => true,
            };
            let what = format!("{role:?} {} {}", route.method, route.path);
            if allowed {
                assert!(
                    !matches!(
                        response.status,
                        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                    ),
                    "{what} must be allowed: {} {}",
                    response.status,
                    response.text()
                );
            } else {
                assert_eq!(response.status, StatusCode::FORBIDDEN, "{what}");
                assert_eq!(response.error_code(), "forbidden", "{what}");
            }
        }
    }
}

#[tokio::test]
async fn a_pending_password_change_blocks_everything_but_the_account_basics() {
    let hub = TestHub::new(&[]).await;
    let store = hub.state.storage.get();
    let owner = store.user_by_username(OWNER).await.unwrap().unwrap();
    store
        .set_password(&owner.info.id, &owner.password_hash, true)
        .await
        .unwrap();

    for route in session_routes() {
        let cookie = hub.session_for(OWNER).await;
        let response = hub.send(request(&route, Some(&cookie))).await;
        let what = format!("{} {}", route.method, route.path);
        if route.access == Access::LoggedIn {
            assert_ne!(response.status, StatusCode::FORBIDDEN, "{what}");
            assert_ne!(response.status, StatusCode::UNAUTHORIZED, "{what}");
        } else {
            assert_eq!(response.status, StatusCode::FORBIDDEN, "{what}");
            assert_eq!(response.error_code(), "password_change_required", "{what}");
        }
    }
}

#[tokio::test]
async fn every_post_except_the_probes_needs_the_csrf_header() {
    let hub = TestHub::new(&[]).await;

    for route in access_table()
        .into_iter()
        .filter(|r| r.method == Method::POST && r.access != Access::Probe)
    {
        // A valid session, so the header is the only thing missing.
        let cookie = hub.session_for(OWNER).await;
        let response = hub.send(request_without_csrf(&route, Some(&cookie))).await;
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "POST {}",
            route.path
        );
        assert_eq!(response.error_code(), "csrf", "POST {}", route.path);
    }
    // Probes can't send it and don't need it: their reports are signed.
    let report = access_table()
        .into_iter()
        .find(|r| r.path == "/api/report")
        .unwrap();
    let response = hub.send(request_without_csrf(&report, None)).await;
    assert_ne!(response.error_code(), "csrf");
}
