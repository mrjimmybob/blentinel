#![cfg(feature = "ssr")]
//! Web-UI JSON API handlers. Errors are [`ApiError`]s (JSON bodies, see
//! `docs/reference/api.md`); failures are logged where they are converted.
//! Who may call each one is declared in `crate::routes` and checked before
//! the handler runs.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, DashboardCompany, ProbeDevice,
    UptimeBucket,
};
use crate::service::{self, scope};
use crate::storage::UptimeRange;

/// The body of a successful action without other output.
fn success() -> Json<Value> {
    Json(json!({ "success": true }))
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

pub async fn dashboard_companies(
    State(state): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<DashboardCompany>>> {
    let companies = state
        .storage
        .get()
        .dashboard_companies(&me.user.effective_scope())
        .await
        .map_err(|e| ApiError::internal("dashboard_companies", e))?;
    Ok(Json(companies))
}

// ---------------------------------------------------------------------------
// Company detail
// ---------------------------------------------------------------------------

pub async fn company_probes(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(company_id): Path<String>,
) -> ApiResult<Json<Vec<CompanyProbe>>> {
    scope::require(&me.user.effective_scope(), &company_id)?;
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
    State(state): State<AppState>,
    me: CurrentUser,
    Path(company_id): Path<String>,
    Query(query): Query<UptimeQuery>,
) -> ApiResult<Json<Vec<UptimeBucket>>> {
    scope::require(&me.user.effective_scope(), &company_id)?;
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
    State(state): State<AppState>,
    me: CurrentUser,
    Path(probe_id): Path<String>,
) -> ApiResult<Json<Vec<ProbeDevice>>> {
    let store = state.storage.get();
    scope::require_probe(store.as_ref(), &me.user.effective_scope(), &probe_id)
        .await
        .map_err(|e| ApiError::internal("probe_devices scope", e))??;
    let devices = store
        .probe_devices(&probe_id)
        .await
        .map_err(|e| ApiError::internal(&format!("probe_devices({probe_id})"), e))?;
    Ok(Json(devices))
}

// ---------------------------------------------------------------------------
// Admin — read
// ---------------------------------------------------------------------------

pub async fn admin_companies(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    let companies = state
        .storage
        .get()
        .company_ids()
        .await
        .map_err(|e| ApiError::internal("admin_companies", e))?;
    Ok(Json(companies))
}

pub async fn admin_probes(State(state): State<AppState>) -> ApiResult<Json<Vec<AdminProbe>>> {
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

pub async fn admin_storage_info(State(state): State<AppState>) -> Json<StorageInfo> {
    let retention = state.config.read().await.retention.clone();
    Json(service::retention::storage_info(state.storage.get().as_ref(), &retention).await)
}

pub async fn admin_archives(
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
    State(state): State<AppState>,
    me: CurrentUser,
    ApiJson(body): ApiJson<CreateSilenceBody>,
) -> ApiResult<Json<Value>> {
    scope::require_resource(&me.user.effective_scope(), &body.resource_key)?;
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
    State(state): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<AlertSilence>>> {
    Ok(Json(visible_silences(&state, &me).await?))
}

/// Active silences of resources in the user's companies.
async fn visible_silences(state: &AppState, me: &CurrentUser) -> ApiResult<Vec<AlertSilence>> {
    let scope = me.user.effective_scope();
    let mut silences = state
        .storage
        .get()
        .active_silences(state.clock.now())
        .await
        .map_err(|e| ApiError::internal("list_silences", e))?;
    silences.retain(|s| scope::require_resource(&scope, &s.scope_id).is_ok());
    Ok(silences)
}

#[derive(Deserialize)]
pub struct DeleteSilenceBody {
    pub silence_id: i64,
}

pub async fn delete_silence(
    State(state): State<AppState>,
    me: CurrentUser,
    ApiJson(body): ApiJson<DeleteSilenceBody>,
) -> ApiResult<Json<Value>> {
    // Only silences the user can see (an expired one is gone anyway).
    let visible = visible_silences(&state, &me).await?;
    if !visible.iter().any(|s| s.id == body.silence_id) {
        return Err(ApiError::not_found("no such silence"));
    }
    state
        .storage
        .get()
        .delete_silence(body.silence_id)
        .await
        .map_err(|e| ApiError::internal("delete_silence", e))?;
    Ok(success())
}
