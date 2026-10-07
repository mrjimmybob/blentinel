#![cfg(feature = "ssr")]
//! HTTP handlers for incidents (M3-05, D-67): the list, one incident with its
//! timeline, acknowledging and notes. Rules: `service::incidents`; access:
//! `crate::routes` (viewing: `ViewData`, scoped; acknowledging and notes:
//! `Silence`, as for silences).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde_json::json;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{IncidentDetail, IncidentInfo, IncidentNote, IncidentsQuery, Role};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::incidents::{self, IncidentError};

impl From<IncidentError> for ApiError {
    fn from(error: IncidentError) -> Self {
        match error {
            IncidentError::NotFound => ApiError::not_found("no such incident"),
            IncidentError::Invalid(message) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_incident", message)
            }
            IncidentError::Conflict(message) => {
                ApiError::new(StatusCode::CONFLICT, "incident_conflict", message)
            }
            IncidentError::Storage(e) => ApiError::internal("incidents", format!("{e:#}")),
        }
    }
}

/// `GET /api/incidents?state=open|closed|all`: newest first.
pub async fn list(
    State(state): State<AppState>,
    me: CurrentUser,
    Query(query): Query<IncidentsQuery>,
) -> ApiResult<Json<Vec<IncidentInfo>>> {
    let list = incidents::list(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        &query,
        state.clock.now(),
    )
    .await?;
    Ok(Json(list))
}

/// `GET /api/incidents/{id}`: with its timeline (Clients: without notes).
pub async fn detail(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<IncidentDetail>> {
    let detail = incidents::detail(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        id,
        me.user.role != Role::Client,
        state.clock.now(),
    )
    .await?;
    Ok(Json(detail))
}

/// `POST /api/incidents/{id}/acknowledge`: "I'm on it", with a note.
pub async fn acknowledge(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<IncidentNote>,
) -> ApiResult<Json<IncidentInfo>> {
    let incident = incidents::acknowledge(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        id,
        &me.user.username,
        &body,
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::INCIDENT_ACKNOWLEDGE)
        .target("incident", id.to_string())
        .company(&incident.company_id)
        .details(json!({ "title": incident.title(), "note": body.text.trim() }));
    record(&state, &me, &info, event).await;
    Ok(Json(incident))
}

/// `POST /api/incidents/{id}/notes`: a note on its timeline.
pub async fn add_note(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<i64>,
    ApiJson(body): ApiJson<IncidentNote>,
) -> ApiResult<Json<IncidentInfo>> {
    let incident = incidents::add_note(
        state.storage.get().as_ref(),
        &me.user.effective_scope(),
        id,
        &me.user.username,
        &body,
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::INCIDENT_NOTE)
        .target("incident", id.to_string())
        .company(&incident.company_id)
        .details(json!({ "title": incident.title(), "note": body.text.trim() }));
    record(&state, &me, &info, event).await;
    Ok(Json(incident))
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
