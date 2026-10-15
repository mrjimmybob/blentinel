#![cfg(feature = "ssr")]
//! HTTP handlers for the Alerts page (M3-03, D-66): the hub's mail server and
//! address, alert channels (with "Send test") and routing rules. Rules:
//! `service::notify::channels`; access: `crate::routes` (Owners and Admins:
//! channels hold secrets).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::json;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    AlertSettings, AlertsOverview, ChannelInfo, ChannelRequest, DigestSettings, RoutesRequest,
    TestResult,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::digest::{self, DigestError};
use crate::service::notify::channels::{self, ChannelError};

impl From<ChannelError> for ApiError {
    fn from(error: ChannelError) -> Self {
        match error {
            ChannelError::NotFound => ApiError::not_found("no such channel"),
            ChannelError::Invalid(message) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_alerts", message)
            }
            ChannelError::Storage(e) => ApiError::internal("alert channels", format!("{e:#}")),
        }
    }
}

impl From<DigestError> for ApiError {
    fn from(error: DigestError) -> Self {
        match error {
            DigestError::Invalid(message) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_alerts", message)
            }
            DigestError::Storage(e) => ApiError::internal("digest", format!("{e:#}")),
        }
    }
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

/// `GET /api/admin/alerts`.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<AlertsOverview>> {
    let overview = channels::overview(state.storage.get().as_ref())
        .await
        .map_err(ChannelError::Storage)?;
    Ok(Json(overview))
}

/// `POST /api/admin/alerts/settings`: the mail server and the hub's address.
pub async fn save_settings(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<AlertSettings>,
) -> ApiResult<Json<serde_json::Value>> {
    channels::save_settings(
        state.storage.get().as_ref(),
        &state.delivery,
        &body,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::ALERTS_SETTINGS).details(json!({
        "hub_url": body.hub_url, "smtp_server": body.smtp_server, "smtp_port": body.smtp_port,
        "smtp_security": body.smtp_security, "smtp_from": body.smtp_from,
        "password_changed": body.smtp_password.is_some(),
    }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// What the audit log keeps of a channel: never its secret.
fn channel_details(request: &ChannelRequest) -> serde_json::Value {
    json!({
        "name": request.name, "kind": request.kind, "enabled": request.enabled,
        "target": request.target, "secret_changed": request.secret.as_deref().is_some_and(|s| !s.trim().is_empty()),
    })
}

/// `POST /api/admin/alerts/channels`: a new channel.
pub async fn create(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<ChannelRequest>,
) -> ApiResult<Json<ChannelInfo>> {
    let channel = channels::create(
        state.storage.get().as_ref(),
        &state.delivery,
        &body,
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::CHANNEL_CREATE)
        .target("channel", channel.id.to_string())
        .details(channel_details(&body));
    record(&state, &me, &info, event).await;
    Ok(Json(channel))
}

/// `POST /api/admin/alerts/channels/{id}`: change a channel.
pub async fn update(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<ChannelRequest>,
) -> ApiResult<Json<ChannelInfo>> {
    let channel =
        channels::update(state.storage.get().as_ref(), &state.delivery, id, &body).await?;
    let event = Event::new(action::CHANNEL_UPDATE)
        .target("channel", id.to_string())
        .details(channel_details(&body));
    record(&state, &me, &info, event).await;
    Ok(Json(channel))
}

/// `POST /api/admin/alerts/channels/{id}/delete`.
pub async fn delete(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = channels::delete(state.storage.get().as_ref(), id, state.clock.now()).await?;
    let event = Event::new(action::CHANNEL_DELETE)
        .target("channel", id.to_string())
        .details(json!({ "name": name }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/alerts/channels/{id}/test`: "Send test", now, once.
pub async fn test(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<TestResult>> {
    let result = channels::test(state.storage.get().as_ref(), &state.delivery, id).await?;
    Ok(Json(result))
}

/// `POST /api/admin/alerts/routes`: replace the routing rules.
pub async fn save_routes(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<RoutesRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    channels::save_rules(state.storage.get().as_ref(), &body).await?;
    let event = Event::new(action::ROUTES_UPDATE).details(json!({ "rules": body.rules }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/alerts/digest`: when the daily digest goes, and where (D-71).
pub async fn save_digest(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<DigestSettings>,
) -> ApiResult<Json<serde_json::Value>> {
    digest::save(
        state.storage.get().as_ref(),
        &body,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::DIGEST_SETTINGS).details(json!({
        "hour": body.hour, "timezone": body.timezone, "channels": body.channels,
    }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/admin/alerts/digest/send`: "Send digest now".
pub async fn send_digest(State(state): State<AppState>) -> ApiResult<Json<TestResult>> {
    let result = digest::send_now(
        state.storage.get().as_ref(),
        &state.delivery,
        state.clock.now(),
    )
    .await?;
    Ok(Json(result))
}
