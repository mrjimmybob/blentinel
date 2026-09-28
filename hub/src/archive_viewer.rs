#![cfg(feature = "ssr")]
//! Read-only views over an archive, with the same JSON shapes as the live API.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};

use crate::AppState;
use crate::api::UptimeQuery;
use crate::auth;
use crate::error::{ApiError, ApiResult};
use crate::model::{CompanyProbe, DashboardCompany, ProbeDevice, UptimeBucket};
use crate::storage::{Storage, UptimeRange};

/// Open archive `archive_id` read-only. Unknown and unreadable archives are
/// both answered with 404; the difference is logged.
async fn open_archive(state: &AppState, archive_id: i64) -> ApiResult<Box<dyn Storage>> {
    let archive_dir = state.config.read().await.retention.archive_path.clone();
    match state
        .storage
        .get()
        .open_archive(&archive_dir, archive_id)
        .await
    {
        Ok(Some(archive)) => Ok(archive),
        Ok(None) => {
            tracing::warn!("Archive {archive_id} requested but not found");
            Err(archive_not_found(archive_id))
        }
        Err(e) => {
            tracing::error!("Failed to open archive {archive_id}: {e:#}");
            Err(archive_not_found(archive_id))
        }
    }
}

fn archive_not_found(archive_id: i64) -> ApiError {
    ApiError::not_found(format!("archive {archive_id} not found or unreadable"))
}

/// GET /api/archive/{archive_id}/companies
pub async fn archive_companies(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path(archive_id): Path<i64>,
) -> ApiResult<Json<Vec<DashboardCompany>>> {
    let archive = open_archive(&state, archive_id).await?;
    let companies = archive
        .dashboard_companies()
        .await
        .map_err(|e| ApiError::internal("archive_companies", e))?;
    Ok(Json(companies))
}

/// GET /api/archive/{archive_id}/company/{company_id}/probes
pub async fn archive_company_probes(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, company_id)): Path<(i64, String)>,
) -> ApiResult<Json<Vec<CompanyProbe>>> {
    let archive = open_archive(&state, archive_id).await?;
    let whitelist = state.config.read().await.probe_whitelist();
    let probes =
        crate::service::dashboard::company_probes(archive.as_ref(), &whitelist, &company_id)
            .await
            .map_err(|e| ApiError::internal("archive_company_probes", e))?;
    Ok(Json(probes))
}

/// GET /api/archive/{archive_id}/company/{company_id}/uptime?range=
///
/// The range ends at the archive's newest report rather than at "now": an
/// archive only holds data older than its cutoff (D-26).
pub async fn archive_company_uptime(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, company_id)): Path<(i64, String)>,
    Query(query): Query<UptimeQuery>,
) -> ApiResult<Json<Vec<UptimeBucket>>> {
    let archive = open_archive(&state, archive_id).await?;
    let range = UptimeRange::parse(query.range.as_deref());
    let end = archive
        .latest_report_at()
        .await
        .map_err(|e| ApiError::internal("archive_company_uptime", e))?
        .unwrap_or_else(|| state.clock.now()); // no reports: nothing to chart anyway
    let uptime = archive
        .company_uptime(&company_id, range, end)
        .await
        .map_err(|e| ApiError::internal("archive_company_uptime", e))?;
    Ok(Json(uptime))
}

/// GET /api/archive/{archive_id}/probe/{probe_id}/devices
pub async fn archive_probe_devices(
    _auth: auth::AuthSession,
    State(state): State<AppState>,
    Path((archive_id, probe_id)): Path<(i64, String)>,
) -> ApiResult<Json<Vec<ProbeDevice>>> {
    let archive = open_archive(&state, archive_id).await?;
    let devices = archive
        .probe_devices(&probe_id)
        .await
        .map_err(|e| ApiError::internal("archive_probe_devices", e))?;
    Ok(Json(devices))
}
