#![cfg(feature = "ssr")]
//! HTTP handlers for enrollment (probe-facing) and for probe records and
//! enrollment codes (UI). Rules: `service::probes`; access: `crate::routes`.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use common::enrollment::EnrollResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use x25519_dalek::PublicKey;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    CreateCodeRequest, CreatedCode, EnrollmentCodeInfo, ProbeInfo, UpdateProbeRequest,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::probes::{self, EnrollError, HubIdentity, ProbeError};

impl From<ProbeError> for ApiError {
    fn from(error: ProbeError) -> Self {
        match error {
            ProbeError::NotFound => ApiError::not_found("not found"),
            ProbeError::Invalid(message) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid", message)
            }
            ProbeError::Exists(_) => {
                ApiError::new(StatusCode::CONFLICT, "exists", error.to_string())
            }
            ProbeError::Storage(e) => ApiError::internal("probe operation", e),
        }
    }
}

/// Username under which failed enrollments are rate-limited (D-43).
const ENROLL_LIMIT_KEY: &str = "#enroll";

// ---------------------------------------------------------------------------
// POST /api/enroll (probes)
// ---------------------------------------------------------------------------

/// A probe redeems an enrollment code. Body: a `SignedEnrollRequest`, sealed
/// for the hub's key like a report. Answer: an `EnrollResponse`.
pub async fn enroll(
    State(state): State<AppState>,
    info: RequestInfo,
    body: Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Like every probe-facing answer, it names the hub's protocol version.
    let response = match enroll_inner(state, info, body).await {
        Ok(json) => json.into_response(),
        Err(error) => error.into_response(),
    };
    crate::ingest::with_protocol_header(response)
}

async fn enroll_inner(
    state: AppState,
    info: RequestInfo,
    body: Bytes,
) -> ApiResult<Json<EnrollResponse>> {
    let now = state.clock.now();
    state
        .login_limiter
        .check(info.client_ip, ENROLL_LIMIT_KEY, now)
        .map_err(|wait| ApiError::too_many_attempts(wait.num_seconds().max(1) as u64))?;
    let store = state.storage.get();
    let (record, request) =
        match probes::enroll(store.as_ref(), &state.hub_secret.get(), &body, now).await {
            Ok(ok) => ok,
            Err(error) => {
                let (status, code) = match &error {
                    EnrollError::Storage(e) => return Err(ApiError::internal("enroll", e)),
                    EnrollError::Undecryptable => (StatusCode::UNAUTHORIZED, "undecryptable"),
                    EnrollError::Malformed => (StatusCode::BAD_REQUEST, "invalid_body"),
                    EnrollError::BadSignature => (StatusCode::FORBIDDEN, "bad_signature"),
                    EnrollError::ClockSkew => {
                        (StatusCode::UNPROCESSABLE_ENTITY, "timestamp_rejected")
                    }
                    EnrollError::InvalidCode => {
                        state
                            .login_limiter
                            .failure(info.client_ip, ENROLL_LIMIT_KEY, now);
                        (StatusCode::FORBIDDEN, "invalid_code")
                    }
                };
                tracing::warn!("Enrollment refused: {error}");
                return Err(ApiError::new(status, code, error.to_string()));
            }
        };

    let company = store
        .company(&record.company_id)
        .await
        .map_err(|e| ApiError::internal("enroll", e))?
        .ok_or_else(|| ApiError::internal("enroll", "the code's company is gone"))?;
    let site_name = company
        .sites
        .iter()
        .find(|s| s.id == record.site_id)
        .map_or_else(|| record.site_id.clone(), |s| s.name.clone());
    tracing::info!(probe = %crate::model::short_probe_id(&record.id), name = %record.name, company = %record.company_id, "Probe enrolled");
    let event = Event::new(action::PROBE_ENROLL)
        .target("probe", &record.id)
        .company(&record.company_id)
        .details(json!({
            "name": record.name, "site": record.site_id, "code": request.token_id,
            "hostname": request.hostname, "os": request.os, "version": request.probe_version,
        }));
    audit::record(
        store.as_ref(),
        &Actor::anonymous(None, info.client_ip),
        event,
        now,
    )
    .await;
    Ok(Json(EnrollResponse {
        probe_id: record.id,
        name: record.name,
        company_id: record.company_id,
        company_name: company.company.name,
        site_id: record.site_id,
        site_name,
    }))
}

// ---------------------------------------------------------------------------
// Probe records
// ---------------------------------------------------------------------------

/// `GET /api/probes`: the user's companies' probes (revoked ones too).
pub async fn list(
    State(state): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<ProbeInfo>>> {
    let list = state
        .storage
        .get()
        .probe_records(&me.user.effective_scope())
        .await
        .map_err(|e| ApiError::internal("list probes", e))?;
    Ok(Json(list))
}

/// `POST /api/probes/{id}`: rename, or move to another site of its company.
pub async fn update(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateProbeRequest>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let record = probes::update_probe(
        store.as_ref(),
        &me.user.effective_scope(),
        &id,
        &body.name,
        &body.site_id,
    )
    .await?;
    let event = Event::new(action::PROBE_UPDATE)
        .target("probe", &id)
        .company(&record.company_id)
        .details(json!({ "name": record.name, "site": record.site_id }));
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        state.clock.now(),
    )
    .await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/probes/{id}/revoke`: refuse its reports from now on.
pub async fn revoke(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let now = state.clock.now();
    let record = probes::revoke_probe(store.as_ref(), &me.user.effective_scope(), &id, now).await?;
    let event = Event::new(action::PROBE_REVOKE)
        .target("probe", &id)
        .company(&record.company_id)
        .details(json!({ "name": record.name }));
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        now,
    )
    .await;
    Ok(Json(json!({ "success": true })))
}

// ---------------------------------------------------------------------------
// Enrollment codes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CodesQuery {
    pub company: Option<String>,
}

/// `GET /api/enrollment-codes?company=`: codes that can still be used.
pub async fn list_codes(
    State(state): State<AppState>,
    me: CurrentUser,
    Query(query): Query<CodesQuery>,
) -> ApiResult<Json<Vec<EnrollmentCodeInfo>>> {
    let mut codes = state
        .storage
        .get()
        .enrollment_tokens(&me.user.effective_scope(), state.clock.now())
        .await
        .map_err(|e| ApiError::internal("list codes", e))?;
    if let Some(company) = query.company {
        codes.retain(|c| c.company_id == company);
    }
    Ok(Json(codes))
}

/// The hub's keys and, if it serves HTTPS itself, its certificate's hash.
async fn hub_identity(state: &AppState) -> HubIdentity {
    let hub_secret = state.hub_secret.get();
    let hub_key = *PublicKey::from(&hub_secret).as_bytes();
    let hub_signing_key = common::crypto::hub_signing_key(&hub_secret)
        .verifying_key()
        .to_bytes();
    let tls = state.config.read().await.server.tls.clone();
    let tls_cert_sha256 = if tls.enabled {
        match std::fs::read(&tls.cert_path).map(|pem| crate::tls::cert_sha256(&pem)) {
            Ok(Ok(hash)) => Some(hash),
            Ok(Err(e)) => {
                tracing::error!("Can't read the TLS certificate for enrollment codes: {e:#}");
                None
            }
            Err(e) => {
                tracing::error!("Can't read {} for enrollment codes: {e}", tls.cert_path);
                None
            }
        }
    } else {
        None
    };
    HubIdentity {
        hub_key,
        hub_signing_key,
        tls_cert_sha256,
    }
}

/// `POST /api/enrollment-codes`: "Add probe". The code is in this answer only.
pub async fn create_code(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<CreateCodeRequest>,
) -> ApiResult<Json<CreatedCode>> {
    let store = state.storage.get();
    let now = state.clock.now();
    let hub = hub_identity(&state).await;
    let created = probes::create_code(store.as_ref(), &me.user, body, &hub, now).await?;
    let event = Event::new(action::CODE_CREATE)
        .target("code", &created.info.id)
        .company(&created.info.company_id)
        .details(json!({
            "site": created.info.site_id, "max_uses": created.info.max_uses,
            "expires_at": created.info.expires_at, "probe_name": created.info.probe_name,
        }));
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        now,
    )
    .await;
    Ok(Json(created))
}

/// `POST /api/enrollment-codes/{id}/revoke`.
pub async fn revoke_code(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let now = state.clock.now();
    let company = probes::revoke_code(store.as_ref(), &me.user.effective_scope(), &id, now).await?;
    let event = Event::new(action::CODE_REVOKE)
        .target("code", &id)
        .company(company);
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        now,
    )
    .await;
    Ok(Json(json!({ "success": true })))
}
