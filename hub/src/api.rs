#![cfg(feature = "ssr")]
//! Web-UI JSON API handlers. Errors are [`ApiError`]s (JSON bodies, see
//! `docs/reference/api.md`); failures are logged where they are converted.

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::auth;
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice,
    UptimeBucket,
};
use crate::service;
use crate::storage::UptimeRange;

/// The body of a successful action without other output.
fn success() -> Json<Value> {
    Json(json!({ "success": true }))
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// `POST /api/login`: body is the raw admin token as plain text.
pub async fn login(State(state): State<AppState>, body: axum::body::Bytes) -> ApiResult<Response> {
    let token = std::str::from_utf8(&body).unwrap_or("").trim();
    // Read at each login: `--reset-admin-token` and a new `auth_token_path`
    // apply without a restart (D-38).
    let token_path = state.config.read().await.server.resolved_auth_token_path();
    let admin_token = auth::read_token(&token_path).map_err(|e| ApiError::internal("login", e))?;
    if token != admin_token {
        return Err(ApiError::unauthorized());
    }

    let session_id = auth::create_session(&state.sessions);
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Max-Age=86400; Path=/",
        auth::SESSION_COOKIE_NAME,
        session_id
    );
    Ok((StatusCode::OK, [(header::SET_COOKIE, cookie)]).into_response())
}

/// `POST /api/logout`: destroys the current session and clears the cookie.
pub async fn logout(session: auth::AuthSession, State(state): State<AppState>) -> Response {
    auth::destroy_session(&state.sessions, &session.session_id);
    let cookie = format!(
        "{}=; HttpOnly; SameSite=Lax; Max-Age=0; Path=/",
        auth::SESSION_COOKIE_NAME
    );
    (StatusCode::OK, [(header::SET_COOKIE, cookie)]).into_response()
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

pub async fn dashboard_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<DashboardCompany>>> {
    let companies = state
        .storage
        .get()
        .dashboard_companies()
        .await
        .map_err(|e| ApiError::internal("dashboard_companies", e))?;
    Ok(Json(companies))
}

// ---------------------------------------------------------------------------
// Company detail
// ---------------------------------------------------------------------------

pub async fn company_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> ApiResult<Json<Vec<CompanyProbe>>> {
    let whitelist = state.config.read().await.probe_whitelist();
    let probes =
        service::dashboard::company_probes(state.storage.get().as_ref(), &whitelist, &company_id)
            .await
            .map_err(|e| ApiError::internal(&format!("company_probes({company_id})"), e))?;
    Ok(Json(probes))
}

#[derive(Deserialize)]
pub struct UptimeQuery {
    /// Optional range parameter: "24h", "7d", "30d", "all".
    /// Defaults to "24h" if absent or invalid.
    pub range: Option<String>,
}

pub async fn company_uptime(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(company_id): Path<String>,
    Query(query): Query<UptimeQuery>,
) -> ApiResult<Json<Vec<UptimeBucket>>> {
    let range = UptimeRange::parse(query.range.as_deref());
    let uptime = state
        .storage
        .get()
        .company_uptime(&company_id, range, state.clock.now())
        .await
        .map_err(|e| ApiError::internal(&format!("company_uptime({company_id}, {range:?})"), e))?;
    Ok(Json(uptime))
}

// ---------------------------------------------------------------------------
// Probe detail
// ---------------------------------------------------------------------------

pub async fn probe_devices(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(probe_id): Path<String>,
) -> ApiResult<Json<Vec<ProbeDevice>>> {
    let devices = state
        .storage
        .get()
        .probe_devices(&probe_id)
        .await
        .map_err(|e| ApiError::internal(&format!("probe_devices({probe_id})"), e))?;
    Ok(Json(devices))
}

// ---------------------------------------------------------------------------
// Admin — read
// ---------------------------------------------------------------------------

pub async fn admin_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<String>>> {
    let companies = state
        .storage
        .get()
        .company_ids()
        .await
        .map_err(|e| ApiError::internal("admin_companies", e))?;
    Ok(Json(companies))
}

pub async fn admin_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<AdminProbe>>> {
    let probes = state
        .storage
        .get()
        .all_probes()
        .await
        .map_err(|e| ApiError::internal("admin_probes", e))?;
    Ok(Json(probes))
}

// ---------------------------------------------------------------------------
// Admin — destructive (POST with JSON body)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CompanyIdBody {
    pub company_id: String,
}

#[derive(Deserialize)]
pub struct ProbeIdBody {
    pub probe_id: String,
}

pub async fn admin_delete_company_data(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CompanyIdBody>,
) -> ApiResult<Json<Value>> {
    state
        .storage
        .get()
        .delete_company_reports(&body.company_id)
        .await
        .map_err(|e| {
            ApiError::internal(
                &format!("admin_delete_company_data({})", body.company_id),
                e,
            )
        })?;
    Ok(success())
}

pub async fn admin_delete_probe_data(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ProbeIdBody>,
) -> ApiResult<Json<Value>> {
    state
        .storage
        .get()
        .delete_probe_reports(&body.probe_id)
        .await
        .map_err(|e| {
            ApiError::internal(&format!("admin_delete_probe_data({})", body.probe_id), e)
        })?;
    Ok(success())
}

pub async fn admin_remove_probe(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ProbeIdBody>,
) -> ApiResult<Json<Value>> {
    state
        .storage
        .get()
        .remove_probe(&body.probe_id)
        .await
        .map_err(|e| ApiError::internal(&format!("admin_remove_probe({})", body.probe_id), e))?;
    Ok(success())
}

// ---------------------------------------------------------------------------
// Admin — Archive operations
// ---------------------------------------------------------------------------

pub use crate::model::StorageInfo;

pub async fn admin_storage_info(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> Json<StorageInfo> {
    let retention = state.config.read().await.retention.clone();
    Json(service::retention::storage_info(state.storage.get().as_ref(), &retention).await)
}

pub async fn admin_archives(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<ArchiveMetadata>>> {
    let archives = state
        .storage
        .get()
        .archives()
        .await
        .map_err(|e| ApiError::internal("admin_archives", e))?;
    Ok(Json(archives))
}

#[derive(Deserialize)]
pub struct CreateArchiveBody {
    #[serde(default)]
    pub cutoff_days: Option<u32>,
}

pub async fn admin_create_archive(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateArchiveBody>,
) -> ApiResult<Json<Value>> {
    let retention = state.config.read().await.retention.clone();
    let result = service::retention::archive_old_data(
        state.storage.get().as_ref(),
        &retention,
        body.cutoff_days,
        state.clock.now(),
    )
    .await
    .map_err(|e| ApiError::internal("admin_create_archive", e))?;
    Ok(Json(json!({
        "success": true,
        "filename": result.filename,
        "size_mb": result.size_mb,
    })))
}

// ---------------------------------------------------------------------------
// Alert silence management
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateSilenceBody {
    pub resource_key: String,
    pub reason: String,
    pub duration_hours: Option<u32>, // None = forever
}

pub async fn create_silence(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateSilenceBody>,
) -> ApiResult<Json<Value>> {
    let silence_id = service::silences::silence_resource(
        state.storage.get().as_ref(),
        &body.resource_key,
        &body.reason,
        body.duration_hours,
        state.clock.now(),
    )
    .await
    .map_err(|e| ApiError::internal("create_silence", e))?;
    Ok(Json(json!({
        "success": true,
        "silence_id": silence_id,
    })))
}

pub async fn list_silences(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<AlertSilence>>> {
    let silences = state
        .storage
        .get()
        .active_silences(state.clock.now())
        .await
        .map_err(|e| ApiError::internal("list_silences", e))?;
    Ok(Json(silences))
}

#[derive(Deserialize)]
pub struct DeleteSilenceBody {
    pub silence_id: i64,
}

pub async fn delete_silence(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<DeleteSilenceBody>,
) -> ApiResult<Json<Value>> {
    state
        .storage
        .get()
        .delete_silence(body.silence_id)
        .await
        .map_err(|e| ApiError::internal("delete_silence", e))?;
    Ok(success())
}
