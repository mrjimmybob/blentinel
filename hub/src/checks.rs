#![cfg(feature = "ssr")]
//! HTTP handlers for checks (M2-04, D-49). Rules: `service::checks`; access:
//! `crate::routes` (viewing: `ViewData`; changing: `ManageCompanies`). Every
//! change reaches the probe in the answer to its next report.

use axum::{
    extract::{Path, State},
    response::Json,
};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiJson, ApiResult};
use crate::model::{CheckInfo, CheckRequest, CopyChecksRequest, ProbeChecks};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::checks;
use crate::storage::CheckRecord;

/// What the audit log keeps of a check.
fn audit_details(check: &CheckRecord) -> Value {
    json!({
        "probe": check.probe_id, "name": check.name, "spec": check.spec, "enabled": check.enabled,
        "fail_after": check.alert_after.fail_after, "recover_after": check.alert_after.recover_after,
        "limits": check.limits,
    })
}

/// One check as the API shows it.
async fn info(store: &dyn crate::storage::Storage, check: &CheckRecord) -> ApiResult<CheckInfo> {
    let mut infos = checks::infos(store, &check.probe_id, std::slice::from_ref(check)).await?;
    Ok(infos.remove(0))
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

/// `GET /api/probes/{id}/checks`: the probe's checks and whether it runs them.
pub async fn list(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(probe_id): Path<String>,
) -> ApiResult<Json<ProbeChecks>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    Ok(Json(
        checks::probe_checks(store.as_ref(), &scope, &probe_id).await?,
    ))
}

/// `POST /api/probes/{id}/checks`: add a check.
pub async fn create(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(probe_id): Path<String>,
    ApiJson(body): ApiJson<CheckRequest>,
) -> ApiResult<Json<CheckInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let now = state.clock.now();
    let check = checks::create_check(
        store.as_ref(),
        &scope,
        &probe_id,
        &body,
        &me.user.username,
        now,
    )
    .await?;
    let event = Event::new(action::CHECK_CREATE)
        .target("check", check.id.as_str())
        .company(&check.company_id)
        .details(audit_details(&check));
    record(&state, &me, &info, event).await;
    Ok(Json(self::info(store.as_ref(), &check).await?))
}

/// `POST /api/probes/{id}/checks/copy`: add copies of another probe's checks.
pub async fn copy(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(probe_id): Path<String>,
    ApiJson(body): ApiJson<CopyChecksRequest>,
) -> ApiResult<Json<Vec<CheckInfo>>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let now = state.clock.now();
    let (target, copies) = checks::copy_checks(
        store.as_ref(),
        &scope,
        &probe_id,
        &body.from_probe_id,
        &me.user.username,
        now,
    )
    .await?;
    let event = Event::new(action::CHECK_COPY)
        .target("probe", &target.id)
        .company(&target.company_id)
        .details(json!({ "from": body.from_probe_id, "checks": copies.len() }));
    record(&state, &me, &info, event).await;
    Ok(Json(
        checks::infos(store.as_ref(), &target.id, &copies).await?,
    ))
}

/// `POST /api/checks/{id}`: replace a check's name, spec and enabled flag.
pub async fn update(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CheckRequest>,
) -> ApiResult<Json<CheckInfo>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let now = state.clock.now();
    let (before, after) =
        checks::update_check(store.as_ref(), &scope, &id, &body, &me.user.username, now).await?;
    let event = Event::new(action::CHECK_UPDATE)
        .target("check", after.id.as_str())
        .company(&after.company_id)
        .details(json!({ "before": audit_details(&before), "after": audit_details(&after) }));
    record(&state, &me, &info, event).await;
    Ok(Json(self::info(store.as_ref(), &after).await?))
}

/// `POST /api/checks/{id}/delete`: remove a check (its history stays).
pub async fn delete(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let store = state.storage.get();
    let scope = me.user.effective_scope();
    let check = checks::delete_check(
        store.as_ref(),
        &scope,
        &id,
        &me.user.username,
        state.clock.now(),
    )
    .await?;
    let event = Event::new(action::CHECK_DELETE)
        .target("check", check.id.as_str())
        .company(&check.company_id)
        .details(audit_details(&check));
    record(&state, &me, &info, event).await;
    Ok(Json(json!({ "success": true })))
}
