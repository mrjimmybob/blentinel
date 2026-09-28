#![cfg(feature = "ssr")]
//! The one error type every HTTP handler returns.
//!
//! Every error response has the same JSON body, [`ErrorBody`]:
//! `{"error": {"code": "…", "message": "…"}}`. Handlers return
//! `Result<_, ApiError>` and use `?`; they never build error responses by hand.

use axum::{
    Json,
    extract::{FromRequest, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use common::protocol::ErrorBody;

pub type ApiResult<T> = Result<T, ApiError>;

/// An HTTP error: status, stable machine-readable `code`, human `message`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    /// Seconds for a `Retry-After` header (429).
    retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    /// Too many failed attempts: try again in `wait_secs` (429, `Retry-After`).
    pub fn too_many_attempts(wait_secs: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            format!("too many failed attempts; try again in {wait_secs} s"),
        );
        error.retry_after = Some(wait_secs);
        error
    }

    /// No valid session cookie.
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "not logged in, or the session has expired",
        )
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// A failure on the hub's side. `cause` is logged with `context`, but never
    /// sent to the client: it can contain file paths or SQL.
    pub fn internal(context: &str, cause: impl std::fmt::Display) -> Self {
        tracing::error!("{context}: {cause:#}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error; see the hub log",
        )
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response =
            (self.status, Json(ErrorBody::new(self.code, self.message))).into_response();
        if let Some(secs) = self.retry_after {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, secs.into());
        }
        response
    }
}

/// Outside the user's company scope: answered like a missing item (D-45).
impl From<crate::service::scope::OutOfScope> for ApiError {
    fn from(_: crate::service::scope::OutOfScope) -> Self {
        Self::not_found("not found")
    }
}

/// A request body that isn't the expected JSON (400, 415 or 422, as axum decides).
impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::new(rejection.status(), "invalid_body", rejection.body_text())
    }
}

/// `axum::Json` for request bodies, rejecting a bad body with an [`ApiError`]
/// so it gets the same JSON error shape as everything else.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;

    async fn body_of(error: ApiError) -> (StatusCode, ErrorBody) {
        let response = error.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn renders_status_and_json_body() {
        let (status, body) = body_of(ApiError::not_found("archive 7 not found")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, ErrorBody::new("not_found", "archive 7 not found"));
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_the_cause() {
        let (status, body) = body_of(ApiError::internal(
            "test",
            "no such table: reports at /var/lib/x.db",
        ))
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.error.code, "internal");
        assert!(!body.error.message.contains("/var/lib"));
    }
}
