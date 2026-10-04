#![cfg(feature = "ssr")]
//! HTTP handlers for probe releases (M2-15, D-55): adding, approving and
//! removing them (admin page), and serving their binaries, to probes that
//! update and to machines being set up (D-53). Rules: `service::releases`;
//! access: `crate::routes`.

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
use crate::model::{ApproveRequest, CompanyScope, ProbeDownload, ProbeRelease, ProbeUpdates};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::releases::{self, StoreError};
use crate::storage::ApprovedVersion;

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
            StoreError::Missing(_) | StoreError::Mismatch(_) | StoreError::Archive(_) => {
                "invalid_release"
            }
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

async fn approved(state: &AppState) -> ApiResult<Option<ApprovedVersion>> {
    state
        .storage
        .get()
        .approved_probe_version()
        .await
        .map_err(|e| ApiError::internal("approved probe version", e))
}

/// The release "Add probe" offers, if any.
async fn install_release(state: &AppState) -> ApiResult<Option<releases::Stored>> {
    let root = releases_dir(state).await;
    let approved = approved(state).await?.map(|a| a.version);
    let keys = state.release_keys.clone();
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
    let keys = state.release_keys.clone();
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

/// `GET /api/admin/probe-updates`: releases, the approved one, and which
/// version each probe runs.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<ProbeUpdates>> {
    let root = releases_dir(&state).await;
    let keys = state.release_keys.clone();
    let has_release_key = !keys.is_empty();
    let listed = blocking(move || releases::list(&root, &keys)).await?;
    let approval = approved(&state).await?;
    let probes = state
        .storage
        .get()
        .probe_records(&CompanyScope::All)
        .await
        .map_err(|e| ApiError::internal("probe updates", e))?;
    Ok(Json(ProbeUpdates {
        approved: approval.as_ref().map(|a| a.version.clone()),
        approved_by: approval.as_ref().and_then(|a| a.approved_by.clone()),
        approved_at: approval.map(|a| a.approved_at.format("%Y-%m-%d %H:%M UTC").to_string()),
        releases: listed.iter().map(releases::describe).collect(),
        has_release_key,
        probes,
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
    let keys = state.release_keys.clone();
    let manifest = {
        let root = root.clone();
        blocking(move || releases::add_from_archive(&root, &body, &keys)).await??
    };
    record(&state, &me, &info, action::RELEASE_ADD, &manifest.version).await;
    let keys = state.release_keys.clone();
    let version = manifest.version.clone();
    let stored = blocking(move || releases::find(&root, &version, &keys))
        .await?
        .ok_or_else(|| ApiError::internal("probe release", "added release not found"))?;
    Ok(Json(releases::describe(&stored)))
}

/// `POST /api/admin/probe-releases/{version}/delete`. The approved release
/// can't be removed: probes would be offered nothing to download.
pub async fn remove(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(version): Path<String>,
) -> ApiResult<Json<Value>> {
    if approved(&state)
        .await?
        .is_some_and(|a| a.version == version)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "release_approved",
            "this is the approved release: approve another (or none) first",
        ));
    }
    let root = releases_dir(&state).await;
    let keys = state.release_keys.clone();
    {
        let version = version.clone();
        blocking(move || releases::remove(&root, &version, &keys)).await??;
    }
    record(&state, &me, &info, action::RELEASE_REMOVE, &version).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/probe-updates/approve`: offer this release to every
/// probe, or (`null`) none.
pub async fn approve(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(request): ApiJson<ApproveRequest>,
) -> ApiResult<Json<Value>> {
    let approval = match request.version {
        None => None,
        Some(version) => {
            let root = releases_dir(&state).await;
            let keys = state.release_keys.clone();
            let found = {
                let version = version.clone();
                blocking(move || releases::find(&root, &version, &keys)).await?
            };
            if found.is_none() {
                return Err(ApiError::not_found(format!(
                    "no release {version} on this hub"
                )));
            }
            Some(ApprovedVersion {
                version,
                approved_by: Some(me.user.username.clone()),
                approved_at: state.clock.now(),
            })
        }
    };
    state
        .storage
        .get()
        .set_approved_probe_version(approval.as_ref())
        .await
        .map_err(|e| ApiError::internal("approve probe release", e))?;
    let target = approval.as_ref().map_or("none", |a| a.version.as_str());
    record(&state, &me, &info, action::RELEASE_APPROVE, target).await;
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
