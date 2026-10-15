#![cfg(feature = "ssr")]
//! HTTP handlers for thresholds above the check (M3-01, D-64): the hub's
//! default (admin page), a company's and a site's. Rules:
//! `service::thresholds`; access: `crate::routes`. A check's own limits are
//! part of the check (`crate::checks`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::json;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{Thresholds, ThresholdsView};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::thresholds::{self, ThresholdError};

impl From<ThresholdError> for ApiError {
    fn from(error: ThresholdError) -> Self {
        match error {
            ThresholdError::NotFound => ApiError::not_found("not found"),
            ThresholdError::Invalid(message) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_thresholds", message)
            }
            ThresholdError::Storage(e) => ApiError::internal("thresholds", e),
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

/// `GET /api/admin/thresholds`: the hub's default.
pub async fn global(State(state): State<AppState>) -> ApiResult<Json<ThresholdsView>> {
    let view = thresholds::global(state.storage.get().as_ref())
        .await
        .map_err(ThresholdError::Storage)?;
    Ok(Json(view))
}

/// `POST /api/admin/thresholds`.
pub async fn set_global(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<Thresholds>,
) -> ApiResult<Json<serde_json::Value>> {
    thresholds::set_global(
        state.storage.get().as_ref(),
        &body,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::THRESHOLDS_UPDATE)
        .target("thresholds", "hub")
        .details(json!(body));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `GET /api/companies/{id}/thresholds`.
pub async fn company(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<ThresholdsView>> {
    let view = thresholds::company(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &id,
    )
    .await?;
    Ok(Json(view))
}

/// `POST /api/companies/{id}/thresholds`.
pub async fn set_company(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Thresholds>,
) -> ApiResult<Json<serde_json::Value>> {
    thresholds::set_company(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &id,
        &body,
    )
    .await?;
    let event = Event::new(action::THRESHOLDS_UPDATE)
        .target("company", &id)
        .company(&id)
        .details(json!(body));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `GET /api/companies/{id}/sites/{site}/thresholds`.
pub async fn site(
    State(state): State<AppState>,
    me: CurrentUser,
    Path((id, site)): Path<(String, String)>,
) -> ApiResult<Json<ThresholdsView>> {
    let view = thresholds::site(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &id,
        &site,
    )
    .await?;
    Ok(Json(view))
}

/// `POST /api/companies/{id}/sites/{site}/thresholds`.
pub async fn set_site(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path((id, site)): Path<(String, String)>,
    ApiJson(body): ApiJson<Thresholds>,
) -> ApiResult<Json<serde_json::Value>> {
    thresholds::set_site(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &id,
        &site,
        &body,
    )
    .await?;
    let event = Event::new(action::THRESHOLDS_UPDATE)
        .target("site", &site)
        .company(&id)
        .details(json!(body));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}
