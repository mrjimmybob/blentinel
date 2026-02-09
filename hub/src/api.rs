#![cfg(feature = "ssr")]

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::db;
use crate::AppState;

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

pub async fn dashboard_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    match db::get_dashboard_companies(&state.pool).await {
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
    match db::get_company_probes(&state.pool, &company_id).await {
        Ok(mut probes) => {
            // Fill in probe_name from the whitelist (read from config)
            let whitelist = state.config.read().await.probe_whitelist();
            for probe in &mut probes {
                probe.probe_name = whitelist
                    .get(&probe.probe_id)
                    .cloned()
                    .unwrap_or_default();
            }
            Json(probes).into_response()
        }
        Err(e) => {
            eprintln!("[ERROR] company_probes({}): {}", company_id, e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

pub async fn company_uptime(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(company_id): Path<String>,
) -> axum::response::Response {
    match db::get_company_uptime_history(&state.pool, &company_id).await {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] company_uptime({}): {}", company_id, e);
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
    match db::get_probe_devices(&state.pool, &probe_id).await {
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
    match db::get_all_companies(&state.pool).await {
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
    match db::get_all_probes(&state.pool).await {
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
    match db::delete_company_data(&state.pool, &body.company_id).await {
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
    match db::delete_probe_data(&state.pool, &body.probe_id).await {
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
    match db::remove_probe(&state.pool, &body.probe_id).await {
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

#[derive(Deserialize, Serialize)]
pub struct StorageInfo {
    pub current_db_size_mb: u64,
    pub warn_threshold_mb: u64,
    pub retention_days: u32,
    pub last_archive_date: Option<String>,
    pub last_archive_size_mb: Option<i64>,
    pub warning: bool,
}

pub async fn admin_storage_info(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    let (db_path, warn_threshold, retention_days) = {
        let cfg = state.config.read().await;
        (
            cfg.server.db_path.clone(),
            cfg.retention.warn_db_size_mb,
            cfg.retention.archive_older_than_days,
        )
    };

    let current_size = match db::get_db_size_mb(&db_path).await {
        Ok(size) => size,
        Err(_) => 0,
    };

    let (last_archive_date, last_archive_size) = match db::get_archives(&state.pool).await {
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
    match db::get_archives(&state.pool).await {
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
    let (days, db_path, archive_path) = {
        let cfg = state.config.read().await;
        (
            body.cutoff_days.unwrap_or(cfg.retention.archive_older_than_days),
            cfg.server.db_path.clone(),
            cfg.retention.archive_path.clone(),
        )
    };

    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
    let request = crate::archive::engine::ArchiveRequest {
        main_db_path: db_path,
        archive_path_dir: archive_path,
        cutoff_date: cutoff,
    };

    match crate::archive::engine::create_archive(&state.pool, request).await {
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
    let expires_at = body.duration_hours.map(|hours| {
        chrono::Utc::now() + chrono::Duration::hours(hours as i64)
    });

    match db::create_silence(
        &state.pool,
        "resource",
        &body.resource_key,
        &body.reason,
        expires_at,
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
    match db::get_active_silences(&state.pool).await {
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
    match db::delete_silence(&state.pool, body.silence_id).await {
        Ok(()) => Json(serde_json::json!({
            "success": true,
        })).into_response(),
        Err(e) => {
            eprintln!("[ERROR] delete_silence: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete silence").into_response()
        }
    }
}
