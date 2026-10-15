#![cfg(feature = "ssr")]
//! Who is making a request, and whether they may (proposal 0001, D-42).
//!
//! Every API route declares an [`Access`] in `crate::routes`; [`enforce`]
//! checks it before the handler runs: the CSRF header on POSTs, then the
//! session from the cookie, the forced password change, and
//! [`crate::model::authorize`]. Handlers that need the user take a
//! [`CurrentUser`]; those that need how the request arrived take [`RequestInfo`].

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::AppState;
use crate::error::ApiError;
use crate::model::{Permission, UserInfo, authorize};
use crate::service::users;

pub const SESSION_COOKIE_NAME: &str = "blentinel_session";

/// The header every POST from the web UI (and scripts) must carry (D-43). A
/// browser can't add it to a cross-site form post, and a cross-site `fetch`
/// with it needs a CORS preflight the hub never grants.
pub const CSRF_HEADER: &str = "x-blentinel";

/// How a request reached the hub, set by the listener (`crate::server`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Http,
    Https,
}

/// How the request arrived: whether it is protected by TLS (directly, or at
/// a trusted proxy) and who sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestInfo {
    /// HTTPS to the hub, or `behind_proxy` (the proxy terminates TLS).
    pub secure: bool,
    /// The client's address; `None` if unknown (e.g. in-process tests).
    pub client_ip: Option<IpAddr>,
}

impl FromRequestParts<AppState> for RequestInfo {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let behind_proxy = state.config.read().await.server.behind_proxy;
        let https = parts.extensions.get::<Transport>() == Some(&Transport::Https);
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        let forwarded = || {
            parts
                .headers
                .get("x-forwarded-for")?
                .to_str()
                .ok()?
                .rsplit(',')
                .next()?
                .trim()
                .parse()
                .ok()
        };
        Ok(RequestInfo {
            secure: https || behind_proxy,
            client_ip: if behind_proxy {
                forwarded().or(peer)
            } else {
                peer
            },
        })
    }
}

/// What a route requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Anyone (login, setup: they check what they need themselves).
    Public,
    /// Probes: authenticated by the report's signature, not a session.
    Probe,
    /// Any logged-in user, even one who must still change their password
    /// (who am I, change password, log out).
    LoggedIn,
    /// A logged-in user with this permission and no pending password change.
    Perm(Permission),
}

/// The logged-in user of this request, put in place by [`enforce`].
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user: UserInfo,
    /// The raw session cookie value (to log out or keep this session).
    pub token: String,
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        // Missing only if a route declared `Public` but wants a user: a bug,
        // answered like any request without a session.
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or_else(ApiError::unauthorized)
    }
}

/// Route middleware: let the request through only if it meets `access`.
pub async fn enforce(
    State(state): State<AppState>,
    access: Access,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if access != Access::Probe
        && request.method() == Method::POST
        && request
            .headers()
            .get(CSRF_HEADER)
            .is_none_or(|v| v.as_bytes() != b"1")
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "csrf",
            "POST requests need the header X-Blentinel: 1",
        ));
    }

    let permission = match access {
        Access::Public | Access::Probe => return Ok(next.run(request).await),
        Access::LoggedIn => None,
        Access::Perm(p) => Some(p),
    };

    let token = request
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_session_cookie)
        .map(str::to_owned)
        .ok_or_else(ApiError::unauthorized)?;
    let user = users::resume(state.storage.get().as_ref(), &token, state.clock.now())
        .await
        .map_err(|e| ApiError::internal("resume session", e))?
        .ok_or_else(ApiError::unauthorized)?;

    if let Some(permission) = permission {
        if user.must_change_password {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "password_change_required",
                "set a new password first (My account)",
            ));
        }
        if !authorize(user.role, permission) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden",
                format!("your role ({}) can't do this", user.role.label()),
            ));
        }
    }

    request.extensions_mut().insert(CurrentUser { user, token });
    Ok(next.run(request).await)
}

/// The `Set-Cookie` value for a new session. `SameSite=Strict`: the browser
/// never sends it with a request started by another site. `Secure` (only
/// sent over HTTPS) whenever the request itself was secure.
pub fn session_cookie(token: &str, secure: bool) -> String {
    let max_age = users::ABSOLUTE_TIMEOUT.num_seconds();
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict{secure}; Max-Age={max_age}; Path=/"
    )
}

/// The `Set-Cookie` value that removes the session cookie.
pub fn cleared_session_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Strict{secure}; Max-Age=0; Path=/")
}

/// Extract the blentinel_session cookie value from a Cookie header string.
pub fn extract_session_cookie(cookie_header: &str) -> Option<&str> {
    cookie_header
        .split(';')
        .filter_map(|part| part.trim().strip_prefix("blentinel_session="))
        .map(str::trim)
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_session_cookie_among_others() {
        assert_eq!(
            extract_session_cookie("theme=dark; blentinel_session=abc ; x=1"),
            Some("abc")
        );
        assert_eq!(extract_session_cookie("theme=dark"), None);
        assert_eq!(extract_session_cookie("blentinel_session="), None);
    }

    #[test]
    fn cookies_are_http_only_strict_and_secure_over_https() {
        let cookie = session_cookie("tok", false);
        assert!(cookie.starts_with("blentinel_session=tok;"));
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert!(!cookie.contains("Secure"));
        assert!(session_cookie("tok", true).contains("; Secure"));
        assert!(cleared_session_cookie(true).contains("Max-Age=0"));
    }
}
