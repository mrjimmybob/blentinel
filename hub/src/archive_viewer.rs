#![cfg(feature = "ssr")]

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Json},
};
use sqlx::SqlitePool;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use crate::auth;
use crate::db;
use crate::AppState;

/// Open an archive database in read-only mode
async fn open_archive_read_only(archive_path: &str) -> Result<SqlitePool> {
    // Verify the archive file exists
    if !std::path::Path::new(archive_path).exists() {
        bail!("Archive file does not exist: {}", archive_path);
    }

    // Open with read-only flag
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(archive_path)
        .read_only(true)
        .create_if_missing(false);

    SqlitePool::connect_with(opts)
        .await
        .context("Failed to open archive database")
}

/// Get the full path to an archive file from archive ID
async fn get_archive_path(main_pool: &SqlitePool, archive_id: i64, archive_dir: &str) -> Result<String> {
    let archives = db::get_archives(main_pool).await?;

    let archive = archives
        .iter()
        .find(|a| a.id == archive_id)
        .ok_or_else(|| anyhow::anyhow!("Archive with ID {} not found", archive_id))?;

    let full_path = PathBuf::from(archive_dir)
        .join(&archive.filename)
        .to_string_lossy()
        .to_string();

    Ok(full_path)
}

// ---------------------------------------------------------------------------
// Archive Viewer API Endpoints
// ---------------------------------------------------------------------------

/// GET /api/archive/{archive_id}/companies
pub async fn archive_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(archive_id): Path<i64>,
) -> axum::response::Response {
    let archive_dir = {
        let cfg = state.config.read().await;
        cfg.retention.archive_path.clone()
    };

    match get_archive_path(&state.pool, archive_id, &archive_dir).await {
        Ok(archive_path) => {
            match open_archive_read_only(&archive_path).await {
                Ok(archive_pool) => {
                    match db::get_dashboard_companies(&archive_pool).await {
                        Ok(data) => Json(data).into_response(),
                        Err(e) => {
                            eprintln!("[ERROR] archive_companies query failed: {}", e);
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Query failed").into_response()
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to open archive {}: {}", archive_id, e);
                    (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
                }
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Archive lookup failed: {}", e);
            (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
        }
    }
}

/// GET /api/archive/{archive_id}/company/{company_id}/probes
pub async fn archive_company_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, company_id)): Path<(i64, String)>,
) -> axum::response::Response {
    let archive_dir = {
        let cfg = state.config.read().await;
        cfg.retention.archive_path.clone()
    };

    match get_archive_path(&state.pool, archive_id, &archive_dir).await {
        Ok(archive_path) => {
            match open_archive_read_only(&archive_path).await {
                Ok(archive_pool) => {
                    match db::get_company_probes(&archive_pool, &company_id).await {
                        Ok(mut data) => {
                            // Fill in probe names from whitelist
                            let whitelist = state.config.read().await.probe_whitelist();
                            for probe in &mut data {
                                probe.probe_name = whitelist
                                    .get(&probe.probe_id)
                                    .cloned()
                                    .unwrap_or_else(|| probe.probe_id[..8.min(probe.probe_id.len())].to_string());
                            }
                            Json(data).into_response()
                        }
                        Err(e) => {
                            eprintln!("[ERROR] archive_company_probes query failed: {}", e);
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Query failed").into_response()
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to open archive {}: {}", archive_id, e);
                    (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
                }
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Archive lookup failed: {}", e);
            (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
        }
    }
}

/// GET /api/archive/{archive_id}/company/{company_id}/uptime
pub async fn archive_company_uptime(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, company_id)): Path<(i64, String)>,
) -> axum::response::Response {
    let archive_dir = {
        let cfg = state.config.read().await;
        cfg.retention.archive_path.clone()
    };

    match get_archive_path(&state.pool, archive_id, &archive_dir).await {
        Ok(archive_path) => {
            match open_archive_read_only(&archive_path).await {
                Ok(archive_pool) => {
                    match db::get_company_uptime_history(&archive_pool, &company_id).await {
                        Ok(data) => Json(data).into_response(),
                        Err(e) => {
                            eprintln!("[ERROR] archive_company_uptime query failed: {}", e);
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Query failed").into_response()
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to open archive {}: {}", archive_id, e);
                    (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
                }
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Archive lookup failed: {}", e);
            (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
        }
    }
}

/// GET /api/archive/{archive_id}/probe/{probe_id}/devices
pub async fn archive_probe_devices(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, probe_id)): Path<(i64, String)>,
) -> axum::response::Response {
    let archive_dir = {
        let cfg = state.config.read().await;
        cfg.retention.archive_path.clone()
    };

    match get_archive_path(&state.pool, archive_id, &archive_dir).await {
        Ok(archive_path) => {
            match open_archive_read_only(&archive_path).await {
                Ok(archive_pool) => {
                    match db::get_probe_devices(&archive_pool, &probe_id).await {
                        Ok(data) => Json(data).into_response(),
                        Err(e) => {
                            eprintln!("[ERROR] archive_probe_devices query failed: {}", e);
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Query failed").into_response()
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to open archive {}: {}", archive_id, e);
                    (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
                }
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Archive lookup failed: {}", e);
            (axum::http::StatusCode::NOT_FOUND, "Archive not found").into_response()
        }
    }
}
