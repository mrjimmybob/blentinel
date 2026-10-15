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
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    AdminProbe, AlertSilence, ArchiveMetadata, CompanyProbe, CreateSilenceRequest,
    DashboardCompany, ProbeDevice, UptimeBucket,
};
use crate::service::audit::{self, Actor, Event, action};
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
    let probes = service::dashboard::company_probes(state.storage.get().as_ref(), &company_id)
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
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<CompanyIdBody>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    store
        .delete_company_reports(&body.company_id)
        .await
        .map_err(|e| {
            ApiError::internal(
                &format!("admin_delete_company_data({})", body.company_id),
                e,
            )
        })?;
    let event = Event::new(action::DATA_DELETE_COMPANY)
        .target("company", &body.company_id)
        .company(&body.company_id);
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(store.as_ref(), &actor, event, state.clock.now()).await;
    Ok(success())
}

pub async fn admin_delete_probe_data(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<ProbeIdBody>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let company = store.probe_company(&body.probe_id).await.ok().flatten();
    store
        .delete_probe_reports(&body.probe_id)
        .await
        .map_err(|e| {
            ApiError::internal(&format!("admin_delete_probe_data({})", body.probe_id), e)
        })?;
    record_probe_event(
        &state,
        &me,
        &info,
        action::DATA_DELETE_PROBE,
        &body.probe_id,
        company,
    )
    .await;
    Ok(success())
}

/// Audit an action on a probe, with its company (looked up before the action).
async fn record_probe_event(
    state: &AppState,
    me: &CurrentUser,
    info: &RequestInfo,
    what: &'static str,
    probe_id: &str,
    company: Option<String>,
) {
    let mut event = Event::new(what).target("probe", probe_id);
    if let Some(company) = company {
        event = event.company(company);
    }
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(
        state.storage.get().as_ref(),
        &actor,
        event,
        state.clock.now(),
    )
    .await;
}

pub async fn admin_remove_probe(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<ProbeIdBody>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let company = store.probe_company(&body.probe_id).await.ok().flatten();
    store
        .remove_probe(&body.probe_id)
        .await
        .map_err(|e| ApiError::internal(&format!("admin_remove_probe({})", body.probe_id), e))?;
    store
        .delete_probe_record(&body.probe_id)
        .await
        .map_err(|e| ApiError::internal(&format!("admin_remove_probe({})", body.probe_id), e))?;
    record_probe_event(
        &state,
        &me,
        &info,
        action::PROBE_REMOVE,
        &body.probe_id,
        company,
    )
    .await;
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
    info: RequestInfo,
    me: CurrentUser,
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
    let event = Event::new(action::ARCHIVE_CREATE)
        .target("archive", &result.filename)
        .details(json!({ "cutoff_days": body.cutoff_days, "size_mb": result.size_mb }));
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(
        state.storage.get().as_ref(),
        &actor,
        event,
        state.clock.now(),
    )
    .await;
    Ok(Json(json!({
        "success": true,
        "filename": result.filename,
        "size_mb": result.size_mb,
    })))
}

// ---------------------------------------------------------------------------
// Audit log (M1-06, D-46)
// ---------------------------------------------------------------------------

async fn audit_entries(
    state: &AppState,
    query: &crate::model::AuditQuery,
) -> ApiResult<Vec<crate::model::AuditEntry>> {
    let filter = audit::filter(query).map_err(|message| {
        ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "invalid_query",
            message,
        )
    })?;
    state
        .storage
        .get()
        .audit_entries(&filter)
        .await
        .map_err(|e| ApiError::internal("audit_entries", e))
}

/// `GET /api/audit?user=&action=&target=&from=&to=&before_id=&limit=`, newest first.
pub async fn audit_log(
    State(state): State<AppState>,
    Query(query): Query<crate::model::AuditQuery>,
) -> ApiResult<Json<Vec<crate::model::AuditEntry>>> {
    Ok(Json(audit_entries(&state, &query).await?))
}

/// `GET /api/audit.csv` with the same filters (up to 1000 entries per file).
pub async fn audit_csv(
    State(state): State<AppState>,
    Query(mut query): Query<crate::model::AuditQuery>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    query.limit = Some(query.limit.unwrap_or(audit::MAX_LIMIT));
    let entries = audit_entries(&state, &query).await?;
    let filename = format!(
        "blentinel-audit-{}.csv",
        state.clock.now().format("%Y%m%d-%H%M%S")
    );
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        audit::to_csv(&entries),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Alert silence management
// ---------------------------------------------------------------------------

/// `POST /api/silence`: silence a check, a probe, a site or a whole company
/// of the user's, now (D-65).
pub async fn create_silence(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<CreateSilenceRequest>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let silence_id = service::maintenance::silence(
        store.as_ref(),
        &me.user.effective_scope(),
        &body,
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::SILENCE_CREATE)
        .target("silence", silence_id.to_string())
        .company(&body.company_id)
        .details(json!({
            "scope": body.scope,
            "reason": body.reason,
            "duration_hours": body.duration_hours,
        }));
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(
        state.storage.get().as_ref(),
        &actor,
        event,
        state.clock.now(),
    )
    .await;
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

/// Active silences in the user's companies.
async fn visible_silences(state: &AppState, me: &CurrentUser) -> ApiResult<Vec<AlertSilence>> {
    let scope = me.user.effective_scope();
    let mut silences = state
        .storage
        .get()
        .active_silences(state.clock.now())
        .await
        .map_err(|e| ApiError::internal("list_silences", e))?;
    silences.retain(|s| scope.allows(&s.company_id));
    Ok(silences)
}

#[derive(Deserialize)]
pub struct DeleteSilenceBody {
    pub silence_id: i64,
}

pub async fn delete_silence(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<DeleteSilenceBody>,
) -> ApiResult<Json<Value>> {
    // Only silences the user can see (an expired one is gone anyway).
    let visible = visible_silences(&state, &me).await?;
    let Some(silence) = visible.iter().find(|s| s.id == body.silence_id) else {
        return Err(ApiError::not_found("no such silence"));
    };
    let event = Event::new(action::SILENCE_DELETE)
        .target("silence", silence.id.to_string())
        .company(&silence.company_id)
        .details(json!({ "scope": silence.scope, "reason": silence.reason }));
    state
        .storage
        .get()
        .delete_silence(body.silence_id)
        .await
        .map_err(|e| ApiError::internal("delete_silence", e))?;
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(
        state.storage.get().as_ref(),
        &actor,
        event,
        state.clock.now(),
    )
    .await;
    Ok(success())
}
