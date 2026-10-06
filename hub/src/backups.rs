#![cfg(feature = "ssr")]
//! HTTP handlers for the admin page's "Backups" (M2-12, D-60): the
//! schedule, "Back up now", the list, deleting one. Rules:
//! `service::backups`; access: `crate::routes`.
//!
//! A backup holds the hub's private key, so downloading one asks for the
//! user's password again and then gives a one-time link (D-60).

use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    BackupDownloadLink, BackupDownloadRequest, BackupFile, BackupKind, BackupOverview,
    BackupSchedule,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::backups::{self, BackupError};
use crate::service::users::{self, UserError};

impl From<BackupError> for ApiError {
    fn from(error: BackupError) -> Self {
        match &error {
            BackupError::Invalid(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_schedule",
                error.to_string(),
            ),
            BackupError::NotFound(_) => ApiError::not_found(error.to_string()),
            BackupError::Exists(_) => {
                ApiError::new(StatusCode::CONFLICT, "backup_exists", error.to_string())
            }
            BackupError::Failed(e) => ApiError::internal("backup", format!("{e:#}")),
        }
    }
}

/// `GET /api/admin/backups`: the schedule, the last scheduled run, the files.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<BackupOverview>> {
    let config = state.config.read().await.clone();
    let store = state.storage.get();
    Ok(Json(
        backups::overview(store.as_ref(), &config, state.clock.now()).await?,
    ))
}

/// `POST /api/admin/backups`: back up now. Answers once the file is written.
pub async fn create(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<BackupFile>> {
    let config = state.config.read().await.clone();
    let store = state.storage.get();
    let now = state.clock.now();
    let file = backups::make(
        store.as_ref(),
        &config,
        &state.config_path,
        BackupKind::Manual,
        now,
    )
    .await?;
    let event = Event::new(action::BACKUP_CREATE)
        .target("backup", &file.name)
        .details(json!({ "size_bytes": file.size_bytes }));
    record(&state, &me, &info, event).await;
    Ok(Json(file))
}

/// `POST /api/admin/backups/schedule`: when the daily backup runs, how many
/// are kept, or off.
pub async fn set_schedule(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(schedule): ApiJson<BackupSchedule>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    backups::set_schedule(
        store.as_ref(),
        &schedule,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::BACKUP_SCHEDULE).details(json!({
        "enabled": schedule.enabled,
        "hour_utc": schedule.hour_utc,
        "keep": schedule.keep,
    }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/backups/{name}/delete`.
pub async fn delete(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let config = state.config.read().await.clone();
    backups::delete(&config, &name)?;
    let event = Event::new(action::BACKUP_DELETE).target("backup", &name);
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/backups/{name}/download`: the user's own password again
/// (wrong ones count like failed logins, D-43), then a one-time link to the
/// file, valid for a minute, for this user only.
pub async fn request_download(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<BackupDownloadRequest>,
) -> ApiResult<Json<BackupDownloadLink>> {
    let now = state.clock.now();
    let username = me.user.username.clone();
    state
        .login_limiter
        .check(info.client_ip, &username, now)
        .map_err(|wait| ApiError::too_many_attempts(wait.num_seconds().max(1) as u64))?;
    backups::find(&state.config.read().await.clone(), &name)?;
    let store = state.storage.get();
    match users::confirm_password(store.as_ref(), &me.user.id, &body.password).await {
        Ok(()) => state.login_limiter.success(info.client_ip, &username),
        Err(e) => {
            if matches!(e, UserError::WrongCurrentPassword) {
                state.login_limiter.failure(info.client_ip, &username, now);
                let event = Event::new(action::BACKUP_DOWNLOAD_FAILED).target("backup", &name);
                record(&state, &me, &info, event).await;
            }
            return Err(e.into());
        }
    }
    let token = state.backup_downloads.issue(&me.user.id, &name, now);
    let event = Event::new(action::BACKUP_DOWNLOAD).target("backup", &name);
    record(&state, &me, &info, event).await;
    Ok(Json(BackupDownloadLink {
        url: format!("/api/admin/backups/download/{token}"),
    }))
}

/// `GET /api/admin/backups/download/{token}`: the file, streamed. The link
/// works once, for the user it was made for.
pub async fn download(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(token): Path<String>,
) -> ApiResult<Response> {
    let Some(name) = state
        .backup_downloads
        .redeem(&token, &me.user.id, state.clock.now())
    else {
        return Err(ApiError::not_found(
            "this download link has expired or was used: download the backup again",
        ));
    };
    let path = backups::find(&state.config.read().await.clone(), &name)?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::internal("backup download", e))?;
    let size = file
        .metadata()
        .await
        .map_err(|e| ApiError::internal("backup download", e))?
        .len();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response())
}

async fn record(state: &AppState, me: &CurrentUser, info: &RequestInfo, event: Event) {
    audit::record(
        state.storage.get().as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        state.clock.now(),
    )
    .await;
}
