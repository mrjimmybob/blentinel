#![cfg(feature = "ssr")]
//! Who is making a request, and whether they may (proposal 0001, D-42).
//!
//! Every API route declares an [`Access`] in `crate::routes`; [`enforce`]
//! checks it before the handler runs: it resumes the session from the cookie,
//! applies the forced password change, and asks [`crate::model::authorize`].
//! Handlers that need the user take a [`CurrentUser`].

use axum::extract::{FromRequestParts, Request, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;

use crate::AppState;
use crate::error::ApiError;
use crate::model::{Permission, UserInfo, authorize};
use crate::service::users;

pub const SESSION_COOKIE_NAME: &str = "blentinel_session";

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
/// never sends it with a request started by another site. `Secure` follows
/// with HTTPS by default (M1-05).
pub fn session_cookie(token: &str) -> String {
    let max_age = users::ABSOLUTE_TIMEOUT.num_seconds();
    format!("{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Max-Age={max_age}; Path=/")
}

/// The `Set-Cookie` value that removes the session cookie.
pub fn cleared_session_cookie() -> String {
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Strict; Max-Age=0; Path=/")
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
    fn cookies_are_http_only_and_strict() {
        let cookie = session_cookie("tok");
        assert!(cookie.starts_with("blentinel_session=tok;"));
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert!(cleared_session_cookie().contains("Max-Age=0"));
    }
}
