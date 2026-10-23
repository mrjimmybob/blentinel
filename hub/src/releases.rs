#![cfg(feature = "ssr")]
//! HTTP handlers for probe releases (M2-15, D-55) and their rollout (M4-01,
//! D-74): adding and removing releases, rolling one out (canaries, pause,
//! promote), canary probes and company policies (admin page), and serving
//! the binaries, to probes that update and to machines being set up (D-53).
//! Rules: `service::releases`, `service::rollout`; access: `crate::routes`.

use std::path::PathBuf;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    ApproveRequest, CanaryRequest, CompanyUpdatePolicy, ProbeDownload, ProbeRelease, ProbeUpdates,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::releases::{self, StoreError};
use crate::service::rollout::{self, AdminChange, RolloutError};

impl From<RolloutError> for ApiError {
    fn from(error: RolloutError) -> Self {
        match error {
            RolloutError::NotFound(why) => ApiError::not_found(why),
            RolloutError::Invalid(why) => ApiError::new(StatusCode::BAD_REQUEST, "invalid", why),
            RolloutError::Conflict(why) => {
                ApiError::new(StatusCode::CONFLICT, "rollout_state", why)
            }
            RolloutError::Storage(e) => ApiError::internal("probe rollout", e),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        let code = match &error {
            StoreError::NotFound(_) => return ApiError::not_found(error.to_string()),
            StoreError::Io(e) => return ApiError::internal("probe release", e),
            StoreError::Exists(_) => {
                return ApiError::new(StatusCode::CONFLICT, "release_exists", error.to_string());
            }
            StoreError::Release(_) => "untrusted_release",
            StoreError::Protocol(_) => "unsupported_protocol_version",
            StoreError::Missing(_)
            | StoreError::Mismatch(_)
            | StoreError::Archive(_)
            | StoreError::RollbackMismatch { .. } => "invalid_release",
        };
        ApiError::new(StatusCode::BAD_REQUEST, code, error.to_string())
    }
}

/// Where the hub keeps its releases.
pub async fn releases_dir(state: &AppState) -> PathBuf {
    let config = state.config.read().await;
    config.server.state_dir.join(releases::RELEASES_DIR)
}

/// Run file work off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| ApiError::internal("probe release", e))
}

/// The version being rolled out, if any.
async fn approved(state: &AppState) -> ApiResult<Option<String>> {
    state
        .storage
        .get()
        .rollout()
        .await
        .map(|r| r.map(|r| r.version))
        .map_err(|e| ApiError::internal("probe rollout", e))
}

/// The release folder and keys, for `service::rollout`.
async fn release_store(state: &AppState) -> (PathBuf, std::sync::Arc<Vec<[u8; 32]>>) {
    (
        releases_dir(state).await,
        state.trusted_release_keys().await,
    )
}

/// The release "Add probe" offers, if any.
async fn install_release(state: &AppState) -> ApiResult<Option<releases::Stored>> {
    let root = releases_dir(state).await;
    let approved = approved(state).await?;
    let keys = state.trusted_release_keys().await;
    blocking(move || releases::for_install(&root, approved.as_deref(), &keys)).await
}

/// `GET /api/probe-downloads`: the probe binaries this hub offers, for the
/// "Add probe" wizard's one-line commands.
pub async fn list_downloads(State(state): State<AppState>) -> ApiResult<Json<Vec<ProbeDownload>>> {
    Ok(Json(
        install_release(&state)
            .await?
            .map(|stored| releases::downloads(&stored))
            .unwrap_or_default(),
    ))
}

/// `GET /download/probe/{target}`: the probe "Add probe" installs. No
/// session: the machine being set up has none. Binaries aren't secret; the
/// wizard's commands check the SHA-256 shown on the hub's page.
pub async fn download(
    State(state): State<AppState>,
    Path(target): Path<String>,
) -> ApiResult<Response> {
    let stored = install_release(&state).await?;
    let found = stored.as_ref().and_then(|stored| {
        let path = releases::download_path(stored, &target)?;
        let info = releases::downloads(stored)
            .into_iter()
            .find(|d| d.target == target)?;
        Some((path, info))
    });
    let Some((path, info)) = found else {
        return Err(ApiError::not_found(
            "this hub has no probe for that platform",
        ));
    };
    let name = if info.os == "windows" {
        "probe.exe"
    } else {
        "probe"
    };
    serve_file(path, name, Some(info.sha256)).await
}

/// `GET /download/releases/{version}/{file}`: a binary of a release, for a
/// probe that updates. The probe checks it against the signed manifest.
pub async fn download_release_file(
    State(state): State<AppState>,
    Path((version, file)): Path<(String, String)>,
) -> ApiResult<Response> {
    let root = releases_dir(&state).await;
    let keys = state.trusted_release_keys().await;
    let found = {
        let file = file.clone();
        blocking(move || releases::binary(&root, &version, &file, &keys)).await?
    };
    let Some(path) = found else {
        return Err(ApiError::not_found("no such release file on this hub"));
    };
    serve_file(path, &file, None).await
}

async fn serve_file(path: PathBuf, name: &str, sha256: Option<String>) -> ApiResult<Response> {
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::internal("download", e))?;
    let mut response = (
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    )
        .into_response();
    if let Some(sha256) = sha256.and_then(|s| s.parse().ok()) {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static("x-checksum-sha256"),
            sha256,
        );
    }
    Ok(response)
}

/// `GET /api/admin/probe-updates`: releases, the rollout, each probe's
/// version and update status, and each company's policy.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<ProbeUpdates>> {
    let root = releases_dir(&state).await;
    let keys = state.trusted_release_keys().await;
    let has_release_key = !keys.is_empty();
    let listed = blocking(move || releases::list(&root, &keys)).await?;
    let (rollout, probes, companies) =
        rollout::overview(state.storage.get().as_ref(), &listed, state.clock.now())
            .await
            .map_err(|e| ApiError::internal("probe updates", e))?;
    Ok(Json(ProbeUpdates {
        rollout,
        releases: listed.iter().map(releases::describe).collect(),
        has_release_key,
        probes,
        companies,
    }))
}

/// `POST /api/admin/probe-releases`: add a signed release; the body is the
/// `.tar.gz` that `blentinelmake release build` writes.
pub async fn add(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    body: Bytes,
) -> ApiResult<Json<ProbeRelease>> {
    let root = releases_dir(&state).await;
    let keys = state.trusted_release_keys().await;
    let manifest = {
        let root = root.clone();
        blocking(move || releases::add_from_archive(&root, &body, &keys)).await??
    };
    record(&state, &me, &info, action::RELEASE_ADD, &manifest.version).await;
    let keys = state.trusted_release_keys().await;
    let version = manifest.version.clone();
    let stored = blocking(move || releases::find(&root, &version, &keys))
        .await?
        .ok_or_else(|| ApiError::internal("probe release", "added release not found"))?;
    Ok(Json(releases::describe(&stored)))
}

/// `POST /api/admin/probe-releases/{version}/delete`. The release being
/// rolled out, or a company is pinned to, can't be removed: probes would be
/// offered nothing to download.
pub async fn remove(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(version): Path<String>,
) -> ApiResult<Json<Value>> {
    let in_use = rollout::versions_in_use(state.storage.get().as_ref())
        .await
        .map_err(|e| ApiError::internal("probe rollout", e))?;
    if in_use.contains(&version) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "release_approved",
            "probes are offered this release (rolled out, or a company is pinned to it): \
             change that first",
        ));
    }
    let root = releases_dir(&state).await;
    let keys = state.trusted_release_keys().await;
    {
        let version = version.clone();
        blocking(move || releases::remove(&root, &version, &keys)).await??;
    }
    record(&state, &me, &info, action::RELEASE_REMOVE, &version).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/probe-updates/approve`: roll this release out, to the
/// canaries first if there are any, or (`null`) stop.
pub async fn approve(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(request): ApiJson<ApproveRequest>,
) -> ApiResult<Json<Value>> {
    let started = rollout::approve(
        state.storage.get().as_ref(),
        release_store(&state).await,
        request.version,
        request.soak_minutes,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    let target = started.as_ref().map_or("none", |r| r.version.as_str());
    record(&state, &me, &info, action::RELEASE_APPROVE, target).await;
    Ok(Json(json!({ "success": true })))
}

async fn change_rollout(
    state: AppState,
    info: RequestInfo,
    me: CurrentUser,
    change: AdminChange,
    what: &'static str,
) -> ApiResult<Json<Value>> {
    let version = rollout::change(
        state.storage.get().as_ref(),
        change,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    record(&state, &me, &info, what, &version).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/probe-updates/pause`: offer the release to no more probes.
pub async fn pause(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<Value>> {
    change_rollout(state, info, me, AdminChange::Pause, action::ROLLOUT_PAUSE).await
}

/// `POST /api/admin/probe-updates/resume`.
pub async fn resume(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<Value>> {
    change_rollout(state, info, me, AdminChange::Resume, action::ROLLOUT_RESUME).await
}

/// `POST /api/admin/probe-updates/promote`: to every probe now, without
/// waiting for the canaries.
pub async fn promote(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<Value>> {
    change_rollout(
        state,
        info,
        me,
        AdminChange::Promote,
        action::ROLLOUT_PROMOTE,
    )
    .await
}

/// `POST /api/admin/probe-updates/probes/{id}/canary`.
pub async fn set_canary(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(probe_id): Path<String>,
    ApiJson(request): ApiJson<CanaryRequest>,
) -> ApiResult<Json<Value>> {
    let storage = state.storage.get();
    rollout::set_canary(storage.as_ref(), &probe_id, request.canary).await?;
    audit::record(
        storage.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        Event::new(action::PROBE_CANARY)
            .target("probe", &probe_id)
            .details(json!({ "canary": request.canary })),
        state.clock.now(),
    )
    .await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/probe-updates/companies/{id}`: a company's update policy.
pub async fn set_company_policy(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(company_id): Path<String>,
    ApiJson(policy): ApiJson<CompanyUpdatePolicy>,
) -> ApiResult<Json<Value>> {
    let storage = state.storage.get();
    let details = serde_json::to_value(&policy).unwrap_or(Value::Null);
    rollout::set_policy(
        storage.as_ref(),
        release_store(&state).await,
        &company_id,
        policy,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    audit::record(
        storage.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        Event::new(action::UPDATE_POLICY)
            .target("company", &company_id)
            .company(&company_id)
            .details(details),
        state.clock.now(),
    )
    .await;
    Ok(Json(json!({ "success": true })))
}

async fn record(
    state: &AppState,
    me: &CurrentUser,
    info: &RequestInfo,
    what: &'static str,
    version: &str,
) {
    audit::record(
        state.storage.get().as_ref(),
        &Actor::user(&me.user, info.client_ip),
        Event::new(what).target("release", version),
        state.clock.now(),
    )
    .await;
}
