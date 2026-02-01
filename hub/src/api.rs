#![cfg(feature = "ssr")]

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Json},
};
use serde::Deserialize;

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
            // Fill in probe_name from the whitelist
            for probe in &mut probes {
                probe.probe_name = state.whitelist
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
