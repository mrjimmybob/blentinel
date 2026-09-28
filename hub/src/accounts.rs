#![cfg(feature = "ssr")]
//! HTTP handlers for login, first-run setup, your own account, and user
//! management. The rules are in `service::users` and `service::setup`; this
//! file only maps them to HTTP. Access per route: `crate::routes`.

use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::{self, CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    ChangePasswordRequest, CreateUserRequest, LoginRequest, SetupRequest, SetupStatus,
    TemporaryPassword, UpdateUserRequest, UserInfo,
};
use crate::service::login_limit;
use crate::service::setup::SetupError;
use crate::service::users::{self, UserError, UserFields};

impl From<UserError> for ApiError {
    fn from(error: UserError) -> Self {
        let message = error.to_string();
        let (status, code) = match error {
            UserError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid_credentials"),
            UserError::WrongCurrentPassword => (StatusCode::BAD_REQUEST, "wrong_password"),
            UserError::WeakPassword => (StatusCode::BAD_REQUEST, "weak_password"),
            UserError::InvalidUsername => (StatusCode::BAD_REQUEST, "invalid_username"),
            UserError::UsernameTaken(_) => (StatusCode::CONFLICT, "username_taken"),
            UserError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            UserError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            UserError::LastOwner => (StatusCode::CONFLICT, "last_owner"),
            UserError::InvalidScope(_) => (StatusCode::BAD_REQUEST, "invalid_scope"),
            UserError::UnknownCompany(_) => (StatusCode::BAD_REQUEST, "unknown_company"),
            UserError::Storage(e) => return ApiError::internal("user operation", e),
        };
        ApiError::new(status, code, message)
    }
}

impl From<SetupError> for ApiError {
    fn from(error: SetupError) -> Self {
        match error {
            SetupError::AlreadyDone => {
                ApiError::new(StatusCode::CONFLICT, "already_set_up", error.to_string())
            }
            SetupError::WrongCode => {
                ApiError::new(StatusCode::FORBIDDEN, "wrong_setup_code", error.to_string())
            }
            SetupError::User(e) => e.into(),
        }
    }
}

fn success() -> Json<Value> {
    Json(json!({ "success": true }))
}

/// The user as JSON, with a new session cookie.
fn logged_in(token: &str, user: UserInfo, secure: bool) -> Response {
    (
        StatusCode::OK,
        [(header::SET_COOKIE, auth::session_cookie(token, secure))],
        Json(user),
    )
        .into_response()
}

/// Refuse the attempt while `(client, name)` is locked (D-43).
fn check_limit(state: &AppState, info: &RequestInfo, name: &str) -> ApiResult<()> {
    state
        .login_limiter
        .check(info.client_ip, name, state.clock.now())
        .map_err(|wait| ApiError::too_many_attempts(wait.num_seconds().max(1) as u64))
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

/// `POST /api/auth/login`: `{username, password}` → the user, and a session cookie.
pub async fn login(
    State(state): State<AppState>,
    info: RequestInfo,
    ApiJson(body): ApiJson<LoginRequest>,
) -> ApiResult<Response> {
    check_limit(&state, &info, &body.username)?;
    let store = state.storage.get();
    let now = state.clock.now();
    match users::login(store.as_ref(), &body.username, &body.password, now).await {
        Ok((token, user)) => {
            state.login_limiter.success(info.client_ip, &body.username);
            Ok(logged_in(&token, user, info.secure))
        }
        Err(e) => {
            if matches!(e, UserError::InvalidCredentials) {
                let limiter = &state.login_limiter;
                limiter.failure(info.client_ip, &body.username, now);
            }
            Err(e.into())
        }
    }
}

/// `POST /api/auth/logout`: end this session and clear the cookie.
pub async fn logout(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Response> {
    users::logout(state.storage.get().as_ref(), &me.token)
        .await
        .map_err(|e| ApiError::internal("logout", e))?;
    Ok((
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            auth::cleared_session_cookie(info.secure),
        )],
        success(),
    )
        .into_response())
}

/// `GET /api/auth/me`.
pub async fn me(me: CurrentUser) -> Json<UserInfo> {
    Json(me.user)
}

// ---------------------------------------------------------------------------
// First run
// ---------------------------------------------------------------------------

/// `GET /api/setup`: whether the first Owner still has to be created.
pub async fn setup_status(State(state): State<AppState>) -> ApiResult<Json<SetupStatus>> {
    let count = state
        .storage
        .get()
        .user_count()
        .await
        .map_err(|e| ApiError::internal("setup_status", e))?;
    Ok(Json(SetupStatus { needed: count == 0 }))
}

/// `POST /api/setup`: create the first Owner with the setup code, and log in.
pub async fn setup(
    State(state): State<AppState>,
    info: RequestInfo,
    ApiJson(body): ApiJson<SetupRequest>,
) -> ApiResult<Response> {
    check_limit(&state, &info, login_limit::SETUP)?;
    let store = state.storage.get();
    let now = state.clock.now();
    let completed = state.setup.complete(
        store.as_ref(),
        &body.code,
        &body.username,
        &body.display_name,
        &body.password,
        now,
    );
    let owner = match completed.await {
        Ok(owner) => owner,
        Err(e) => {
            if matches!(e, SetupError::WrongCode) {
                let limiter = &state.login_limiter;
                limiter.failure(info.client_ip, login_limit::SETUP, now);
            }
            return Err(e.into());
        }
    };
    let (token, user) = users::login(store.as_ref(), &owner.username, &body.password, now).await?;
    Ok(logged_in(&token, user, info.secure))
}

// ---------------------------------------------------------------------------
// Own account
// ---------------------------------------------------------------------------

/// `POST /api/me/password`: change your password; your other sessions end.
pub async fn change_password(
    State(state): State<AppState>,
    me: CurrentUser,
    ApiJson(body): ApiJson<ChangePasswordRequest>,
) -> ApiResult<Json<Value>> {
    users::change_own_password(
        state.storage.get().as_ref(),
        &me.user.id,
        &me.token,
        &body.current_password,
        &body.new_password,
    )
    .await?;
    Ok(success())
}

/// `POST /api/me/logout-others`: end your other sessions.
pub async fn logout_others(
    State(state): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Value>> {
    users::logout_other_sessions(state.storage.get().as_ref(), &me.user.id, &me.token)
        .await
        .map_err(|e| ApiError::internal("logout_others", e))?;
    Ok(success())
}

// ---------------------------------------------------------------------------
// User management
// ---------------------------------------------------------------------------

/// `GET /api/users`.
pub async fn list_users(State(state): State<AppState>) -> ApiResult<Json<Vec<UserInfo>>> {
    let list = state
        .storage
        .get()
        .users()
        .await
        .map_err(|e| ApiError::internal("list_users", e))?;
    Ok(Json(list))
}

/// `POST /api/users`: create a user with a temporary password (returned once).
pub async fn create_user(
    State(state): State<AppState>,
    me: CurrentUser,
    ApiJson(body): ApiJson<CreateUserRequest>,
) -> ApiResult<Json<TemporaryPassword>> {
    let fields = UserFields {
        display_name: body.display_name,
        email: body.email,
        role: body.role,
        scope: body.scope,
    };
    let created = users::create_user(
        state.storage.get().as_ref(),
        Some(&me.user),
        &body.username,
        fields,
        None,
        state.clock.now(),
    )
    .await?;
    tracing::info!(by = %me.user.username, user = %created.user.username, role = created.user.role.as_str(), "User created");
    Ok(Json(created))
}

/// `POST /api/users/{id}`: edit a user.
pub async fn update_user(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateUserRequest>,
) -> ApiResult<Json<UserInfo>> {
    let fields = UserFields {
        display_name: body.display_name,
        email: body.email,
        role: body.role,
        scope: body.scope,
    };
    let user = users::update_user(
        state.storage.get().as_ref(),
        Some(&me.user),
        &id,
        fields,
        body.is_active,
    )
    .await?;
    tracing::info!(by = %me.user.username, user = %user.username, "User updated");
    Ok(Json(user))
}

/// `POST /api/users/{id}/reset-password`: a new temporary password (returned once).
pub async fn reset_password(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<TemporaryPassword>> {
    let reset = users::reset_password(state.storage.get().as_ref(), Some(&me.user), &id).await?;
    tracing::info!(by = %me.user.username, user = %reset.user.username, "Password reset");
    Ok(Json(reset))
}

/// `POST /api/users/{id}/logout`: end all of a user's sessions.
pub async fn force_logout(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    users::force_logout(state.storage.get().as_ref(), Some(&me.user), &id).await?;
    Ok(success())
}
