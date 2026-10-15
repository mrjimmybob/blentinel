#![cfg(feature = "ssr")]
//! HTTP handlers for a probe's machine, its suggested checks and check
//! templates (M5-12, D-85). Rules: `service::inventory`, `service::templates`;
//! access: `crate::routes`.

use axum::{
    extract::{Path, State},
    response::Json,
};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiJson, ApiResult};
use crate::model::{
    AddChecksRequest, AddedChecks, ApplyTemplateRequest, MachineInfo, NetworkInfo,
    NewTemplateRequest, TemplateInfo,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::{checks, inventory, templates};

async fn record(state: &AppState, me: &CurrentUser, info: &RequestInfo, event: Event) {
    audit::record(
        state.storage.get().as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        state.clock.now(),
    )
    .await;
}

/// `GET /api/probes/{id}/machine`: what the probe's machine is made of, and
/// the checks that suggests.
pub async fn machine(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(probe_id): Path<String>,
) -> ApiResult<Json<MachineInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    Ok(Json(
        inventory::machine(store.as_ref(), &scope, &probe_id).await?,
    ))
}

/// `GET /api/probes/{id}/network`: whether the probe may look at its
/// network, and what its last sweep found (D-101).
pub async fn network(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(probe_id): Path<String>,
) -> ApiResult<Json<NetworkInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    Ok(Json(
        crate::service::discovery::network(store.as_ref(), &scope, &probe_id).await?,
    ))
}

/// `POST /api/probes/{id}/checks/batch`: add several checks at once (the
/// suggested ones). A check the probe already has is skipped.
pub async fn add_checks(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(probe_id): Path<String>,
    ApiJson(body): ApiJson<AddChecksRequest>,
) -> ApiResult<Json<AddedChecks>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let (probe, added, skipped) = checks::add_checks(
        store.as_ref(),
        &state.delivery.key,
        &scope,
        &probe_id,
        &body.checks,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    let names: Vec<&str> = added.iter().map(|c| c.name.as_str()).collect();
    let event = Event::new(action::CHECK_BATCH)
        .target("probe", &probe.id)
        .company(&probe.company_id)
        .details(json!({ "added": names, "skipped": skipped }));
    record(&state, &me, &info, event).await;
    Ok(Json(AddedChecks {
        added: checks::infos(store.as_ref(), &probe.id, &added).await?,
        skipped,
    }))
}

/// `GET /api/check-templates`: the built-in templates and, for a user who
/// sees every company, the saved ones.
pub async fn list(
    State(state): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<TemplateInfo>>> {
    let scope = me.user.effective_scope();
    Ok(Json(
        templates::list(state.storage.get().as_ref(), &scope).await?,
    ))
}

/// `POST /api/check-templates`: save a probe's checks as a template.
pub async fn create(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<NewTemplateRequest>,
) -> ApiResult<Json<TemplateInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let template = templates::create_from_probe(
        store.as_ref(),
        &scope,
        &body,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::TEMPLATE_CREATE)
        .target("template", &template.id)
        .details(json!({
            "name": template.name, "from_probe": body.from_probe_id,
            "checks": template.checks.len(),
        }));
    record(&state, &me, &info, event).await;
    Ok(Json(template))
}

/// `POST /api/check-templates/{id}/delete`: delete a saved template.
pub async fn delete(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let template = templates::delete(store.as_ref(), &scope, &id).await?;
    let event = Event::new(action::TEMPLATE_DELETE)
        .target("template", &template.id)
        .details(json!({ "name": template.name }));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/probes/{id}/checks/template`: add a template's checks to a probe.
pub async fn apply(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(probe_id): Path<String>,
    ApiJson(body): ApiJson<ApplyTemplateRequest>,
) -> ApiResult<Json<AddedChecks>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let applied = templates::apply(
        store.as_ref(),
        &state.delivery.key,
        &scope,
        &probe_id,
        &body,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    // What was typed for the template, but for its secrets.
    let values: std::collections::BTreeMap<&String, &String> = body
        .values
        .iter()
        .filter(|(key, _)| {
            !applied
                .template
                .parameters
                .iter()
                .any(|parameter| parameter.secret && parameter.key == **key)
        })
        .collect();
    let event = Event::new(action::CHECK_TEMPLATE)
        .target("probe", &applied.probe.id)
        .company(&applied.probe.company_id)
        .details(json!({
            "template": applied.template.name, "values": values,
            "added": applied.added.len(), "skipped": applied.skipped,
        }));
    record(&state, &me, &info, event).await;
    Ok(Json(AddedChecks {
        added: checks::infos(store.as_ref(), &applied.probe.id, &applied.added).await?,
        skipped: applied.skipped,
    }))
}
