#![cfg(feature = "ssr")]
//! HTTP handlers for a company's maintenance page (M3-04, D-65): its windows
//! and silences. Rules: `service::maintenance`; access: `crate::routes`
//! (viewing: `ViewData`; changing: `Silence`). Silences themselves are
//! created and removed through `/api/silence` (`crate::api`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::json;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{MaintenanceOverview, MaintenanceWindow, NewWindowRequest};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::maintenance::{self, MaintenanceError};

impl From<MaintenanceError> for ApiError {
    fn from(error: MaintenanceError) -> Self {
        match error {
            MaintenanceError::NotFound => ApiError::not_found("not found"),
            MaintenanceError::Invalid(message) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_maintenance", message)
            }
            MaintenanceError::Storage(e) => ApiError::internal("maintenance", e),
        }
    }
}

/// `GET /api/companies/{id}/maintenance`.
pub async fn overview(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<MaintenanceOverview>> {
    let overview = maintenance::overview(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &id,
        state.clock.now(),
    )
    .await?;
    Ok(Json(overview))
}

/// `POST /api/companies/{id}/maintenance`: a new window.
pub async fn create(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<NewWindowRequest>,
) -> ApiResult<Json<MaintenanceWindow>> {
    let window = maintenance::create_window(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &id,
        &body,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::MAINTENANCE_CREATE)
        .target("maintenance", window.id.to_string())
        .company(&id)
        .details(json!({
            "scope": window.scope, "reason": window.reason, "timezone": window.timezone,
            "schedule": window.schedule,
        }));
    record(&state, &me, &info, event).await;
    Ok(Json(window))
}

/// `POST /api/maintenance/{id}/delete`.
pub async fn delete(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let window =
        maintenance::delete_window(state.storage.get().as_ref(), &me.user.effective_scope(), id)
            .await?;
    let event = Event::new(action::MAINTENANCE_DELETE)
        .target("maintenance", window.id.to_string())
        .company(&window.company_id)
        .details(json!({ "scope": window.scope, "reason": window.reason }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
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
