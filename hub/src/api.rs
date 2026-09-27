#![cfg(feature = "ssr")]

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Json},
};
use serde::Deserialize;

use crate::auth;
use crate::service;
use crate::storage::UptimeRange;
use crate::AppState;

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// `POST /api/login`: body is the raw admin token as plain text.
pub async fn login(State(state): State<AppState>, body: axum::body::Bytes) -> axum::response::Response {
    let token = std::str::from_utf8(&body).unwrap_or("").trim();

    if token != state.admin_token {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    let session_id = auth::create_session(&state.sessions);
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Max-Age=86400; Path=/",
        auth::SESSION_COOKIE_NAME, session_id
    );
    (axum::http::StatusCode::OK, [(axum::http::header::SET_COOKIE, cookie)]).into_response()
}

/// `POST /api/logout`: destroys the current session and clears the cookie.
pub async fn logout(session: auth::AuthSession, State(state): State<AppState>) -> axum::response::Response {
    auth::destroy_session(&state.sessions, &session.session_id);
    let cookie = format!("{}=; HttpOnly; SameSite=Lax; Max-Age=0; Path=/", auth::SESSION_COOKIE_NAME);
    (axum::http::StatusCode::OK, [(axum::http::header::SET_COOKIE, cookie)]).into_response()
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

pub async fn dashboard_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    match state.storage.dashboard_companies().await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] dashboard_companies: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Company detail
// ---------------------------------------------------------------------------

pub async fn company_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> axum::response::Response {
    let whitelist = state.config.read().await.probe_whitelist();
    match service::dashboard::company_probes(state.storage.as_ref(), &whitelist, &company_id).await {
        Ok(probes) => Json(probes).into_response(),
        Err(e) => {
            eprintln!("[ERROR] company_probes({}): {}", company_id, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
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
) -> axum::response::Response {
    let range = UptimeRange::parse(query.range.as_deref());
    match state.storage.company_uptime(&company_id, range, state.clock.now()).await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] company_uptime({}, range={:?}): {}", company_id, range, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Probe detail
// ---------------------------------------------------------------------------

pub async fn probe_devices(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(probe_id): Path<String>,
) -> axum::response::Response {
    match state.storage.probe_devices(&probe_id).await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] probe_devices({}): {}", probe_id, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Admin — read
// ---------------------------------------------------------------------------

pub async fn admin_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    match state.storage.company_ids().await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_companies: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

pub async fn admin_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    match state.storage.all_probes().await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_probes: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
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
    axum::Json(body): axum::Json<CompanyIdBody>,
) -> axum::response::Response {
    match state.storage.delete_company_reports(&body.company_id).await {
        Ok(()) => (axum::http::StatusCode::OK, "OK").into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_delete_company_data({}): {}", body.company_id, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

pub async fn admin_delete_probe_data(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<ProbeIdBody>,
) -> axum::response::Response {
    match state.storage.delete_probe_reports(&body.probe_id).await {
        Ok(()) => (axum::http::StatusCode::OK, "OK").into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_delete_probe_data({}): {}", body.probe_id, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

pub async fn admin_remove_probe(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<ProbeIdBody>,
) -> axum::response::Response {
    match state.storage.remove_probe(&body.probe_id).await {
        Ok(()) => (axum::http::StatusCode::OK, "OK").into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_remove_probe({}): {}", body.probe_id, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Admin — Archive operations
// ---------------------------------------------------------------------------

pub use crate::model::StorageInfo;

pub async fn admin_storage_info(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    let (warn_threshold, retention_days) = {
        let cfg = state.config.read().await;
        (cfg.retention.warn_db_size_mb, cfg.retention.archive_older_than_days)
    };

    let current_size = state.storage.size_mb().await.unwrap_or(0);

    let (last_archive_date, last_archive_size) = match state.storage.archives().await {
        Ok(archives) if !archives.is_empty() => (
            Some(archives[0].created_at.clone()),
            Some(archives[0].size_mb)
        ),
        _ => (None, None),
    };

    let info = StorageInfo {
        current_db_size_mb: current_size,
        warn_threshold_mb: warn_threshold,
        retention_days,
        last_archive_date,
        last_archive_size_mb: last_archive_size,
        warning: current_size >= warn_threshold,
    };

    Json(info).into_response()
}

pub async fn admin_archives(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    match state.storage.archives().await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_archives: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct CreateArchiveBody {
    #[serde(default)]
    pub cutoff_days: Option<u32>,
}

pub async fn admin_create_archive(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateArchiveBody>,
) -> axum::response::Response {
    let (days, archive_path) = {
        let cfg = state.config.read().await;
        (
            body.cutoff_days.unwrap_or(cfg.retention.archive_older_than_days),
            cfg.retention.archive_path.clone(),
        )
    };

    let now = state.clock.now();
    let cutoff = now - chrono::Duration::days(days as i64);
    match state.storage.create_archive(&archive_path, cutoff, now).await {
        Ok(result) => Json(serde_json::json!({
            "success": true,
            "filename": result.filename,
            "size_mb": result.size_mb,
        })).into_response(),
        Err(e) => {
            eprintln!("[ERROR] admin_create_archive: {:#}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
             format!("Archive failed: {}", e)).into_response()
        }
    }
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
    axum::Json(body): axum::Json<CreateSilenceBody>,
) -> axum::response::Response {
    let now = state.clock.now();
    let expires_at = body.duration_hours.map(|hours| now + chrono::Duration::hours(hours as i64));

    match state.storage.create_silence(
        "resource",
        &body.resource_key,
        &body.reason,
        expires_at,
        now,
    ).await {
        Ok(silence_id) => Json(serde_json::json!({
            "success": true,
            "silence_id": silence_id,
        })).into_response(),
        Err(e) => {
            eprintln!("[ERROR] create_silence: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to create silence").into_response()
        }
    }
}

pub async fn list_silences(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    match state.storage.active_silences(state.clock.now()).await {
        Ok(silences) => Json(silences).into_response(),
        Err(e) => {
            eprintln!("[ERROR] list_silences: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to list silences").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct DeleteSilenceBody {
    pub silence_id: i64,
}

pub async fn delete_silence(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<DeleteSilenceBody>,
) -> axum::response::Response {
    match state.storage.delete_silence(body.silence_id).await {
        Ok(()) => Json(serde_json::json!({
            "success": true,
        })).into_response(),
        Err(e) => {
            eprintln!("[ERROR] delete_silence: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete silence").into_response()
        }
    }
}
