#![cfg(feature = "ssr")]
//! Read-only views over an archive, with the same JSON shapes as the live API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};

use crate::AppState;
use crate::auth;
use crate::storage::{Storage, UptimeRange};

/// Open archive `archive_id` read-only; `None` (logged) if it is unknown or unreadable.
async fn open_archive(state: &AppState, archive_id: i64) -> Option<Box<dyn Storage>> {
    let archive_dir = state.config.read().await.retention.archive_path.clone();
    match state.storage.open_archive(&archive_dir, archive_id).await {
        Ok(Some(archive)) => Some(archive),
        Ok(None) => {
            eprintln!(
                "[ERROR] Archive lookup failed: archive {} not found",
                archive_id
            );
            None
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to open archive {}: {}", archive_id, e);
            None
        }
    }
}

/// Every archive lookup failure is answered the same way.
fn archive_not_found() -> Response {
    (StatusCode::NOT_FOUND, "Archive not found").into_response()
}

/// Turn a query result into the JSON response (500 on failure).
fn respond<T: serde::Serialize>(what: &str, result: anyhow::Result<T>) -> Response {
    match result {
        Ok(data) => Json(data).into_response(),
        Err(e) => {
            eprintln!("[ERROR] {} query failed: {}", what, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Query failed").into_response()
        }
    }
}

/// GET /api/archive/{archive_id}/companies
pub async fn archive_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(archive_id): Path<i64>,
) -> Response {
    match open_archive(&state, archive_id).await {
        Some(archive) => respond("archive_companies", archive.dashboard_companies().await),
        None => archive_not_found(),
    }
}

/// GET /api/archive/{archive_id}/company/{company_id}/probes
pub async fn archive_company_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, company_id)): Path<(i64, String)>,
) -> Response {
    match open_archive(&state, archive_id).await {
        Some(archive) => {
            let whitelist = state.config.read().await.probe_whitelist();
            respond(
                "archive_company_probes",
                crate::service::dashboard::company_probes(archive.as_ref(), &whitelist, &company_id).await,
            )
        }
        None => archive_not_found(),
    }
}

/// GET /api/archive/{archive_id}/company/{company_id}/uptime
pub async fn archive_company_uptime(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, company_id)): Path<(i64, String)>,
) -> Response {
    match open_archive(&state, archive_id).await {
        // Known issue (DECISIONS.md D-26): "24h" is relative to *now*, but an archive
        // only holds data older than its cutoff, so this is always empty.
        Some(archive) => respond(
            "archive_company_uptime",
            archive
                .company_uptime(&company_id, UptimeRange::Hours24, state.clock.now())
                .await,
        ),
        None => archive_not_found(),
    }
}

/// GET /api/archive/{archive_id}/probe/{probe_id}/devices
pub async fn archive_probe_devices(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, probe_id)): Path<(i64, String)>,
) -> Response {
    match open_archive(&state, archive_id).await {
        Some(archive) => respond(
            "archive_probe_devices",
            archive.probe_devices(&probe_id).await,
        ),
        None => archive_not_found(),
    }
}
