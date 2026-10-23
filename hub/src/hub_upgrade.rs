#![cfg(feature = "ssr")]
//! HTTP handlers for the admin page's "Hub version" (M4-04, D-75): the
//! version running, the opt-in check for a newer release, and the command
//! that upgrades. Rules: `service::hub_upgrades`; access: `crate::routes`.
//!
//! The web UI never upgrades the hub: the service can't replace its own
//! program (it runs unprivileged, D-52), and an upgrade is an admin's
//! decision on the server (`hub upgrade`, proposal 0012 §8).

use axum::{extract::State, response::Json};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{HubUpgradeInfo, UpgradeCheckRequest};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::hub_upgrades::{self, Source};

async fn release_url(state: &AppState) -> String {
    state.config.read().await.upgrades.release_url.clone()
}

async fn current(state: &AppState) -> ApiResult<Json<HubUpgradeInfo>> {
    let url = release_url(state).await;
    hub_upgrades::info(state.storage.get().as_ref(), &url)
        .await
        .map(Json)
        .map_err(|e| ApiError::internal("hub upgrade", e))
}

/// `GET /api/admin/hub-upgrade`.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<HubUpgradeInfo>> {
    current(&state).await
}

/// `POST /api/admin/hub-upgrade`: turn the daily check on or off. Turned
/// on, it checks within a minute.
pub async fn set_check(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(request): ApiJson<UpgradeCheckRequest>,
) -> ApiResult<Json<HubUpgradeInfo>> {
    let storage = state.storage.get();
    storage
        .set_upgrade_check_enabled(request.enabled, Some(&me.user.username), state.clock.now())
        .await
        .map_err(|e| ApiError::internal("hub upgrade", e))?;
    audit::record(
        storage.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        Event::new(action::UPGRADE_CHECK)
            .details(serde_json::json!({ "enabled": request.enabled })),
        state.clock.now(),
    )
    .await;
    current(&state).await
}

/// `POST /api/admin/hub-upgrade/check`: check now, whether or not the daily
/// check is on.
pub async fn check_now(State(state): State<AppState>) -> ApiResult<Json<HubUpgradeInfo>> {
    let source = Source::parse(&release_url(&state).await);
    hub_upgrades::check_now(
        state.storage.get().as_ref(),
        &source,
        &state.trusted_release_keys().await,
        state.clock.now(),
    )
    .await
    .map_err(|e| ApiError::internal("hub upgrade", e))?;
    current(&state).await
}
