#![cfg(feature = "ssr")]
//! HTTP handlers for probe commands (M2-06, D-56). Rules:
//! `service::commands`; access: `crate::routes` (viewing: `ViewData`;
//! queuing and cancelling: `ManageCompanies`). A command reaches the probe
//! in the answer to its next report.

use axum::{
    extract::{Path, State},
    response::Json,
};
use serde_json::json;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiJson, ApiResult};
use crate::model::{CommandInfo, CommandRequest};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::commands;

/// `GET /api/probes/{id}/commands`: the probe's latest commands, newest first.
pub async fn list(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(probe_id): Path<String>,
) -> ApiResult<Json<Vec<CommandInfo>>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    Ok(Json(
        commands::probe_commands(store.as_ref(), &scope, &probe_id, state.clock.now()).await?,
    ))
}

/// `POST /api/probes/{id}/commands`: queue a command (or get the same one
/// already waiting).
pub async fn queue(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(probe_id): Path<String>,
    ApiJson(body): ApiJson<CommandRequest>,
) -> ApiResult<Json<CommandInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let now = state.clock.now();
    let (command, queued) = commands::queue(
        store.as_ref(),
        &scope,
        &probe_id,
        &body.spec,
        &me.user.username,
        now,
    )
    .await?;
    if queued {
        let event = Event::new(action::COMMAND_QUEUE)
            .target("probe", &command.probe_id)
            .company(&command.company_id)
            .details(json!({ "command": command.id, "kind": command.spec.kind() }));
        audit::record(
            store.as_ref(),
            &Actor::user(&me.user, info.client_ip),
            event,
            now,
        )
        .await;
    }
    Ok(Json(commands::to_info(&command, now)))
}

/// `POST /api/commands/{id}/cancel`: cancel a command not sent yet.
pub async fn cancel(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<CommandInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let now = state.clock.now();
    let command = commands::cancel(store.as_ref(), &scope, &id, now).await?;
    let event = Event::new(action::COMMAND_CANCEL)
        .target("probe", &command.probe_id)
        .company(&command.company_id)
        .details(json!({ "command": command.id, "kind": command.spec.kind() }));
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        now,
    )
    .await;
    Ok(Json(commands::to_info(&command, now)))
}
