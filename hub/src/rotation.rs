#![cfg(feature = "ssr")]
//! The admin page's "Hub keys" (M2-13, D-61): the hub's pins, and rotating
//! its certificate and key. Rules: `service::rotation`; this module maps
//! them to HTTP, and applies a switch to the running hub (its key, its
//! HTTPS listener, what it tells probes).

use axum::{extract::State, http::StatusCode, response::Json};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{HubKeysOverview, RotationInfo, StartRotationRequest};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::rotation::{self, RotationError, Switched, Tick};

/// One switch at a time: the button and the hub's own check may race.
static SWITCHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How often the hub looks whether every probe is ready.
const TICK: std::time::Duration = std::time::Duration::from_secs(60);

impl From<RotationError> for ApiError {
    fn from(error: RotationError) -> Self {
        match &error {
            RotationError::Invalid(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_rotation",
                error.to_string(),
            ),
            RotationError::Busy => ApiError::new(
                StatusCode::CONFLICT,
                "rotation_in_progress",
                error.to_string(),
            ),
            RotationError::NotFound => ApiError::not_found(error.to_string()),
            RotationError::Failed(e) => ApiError::internal("rotation", format!("{e:#}")),
        }
    }
}

/// Work out again what the hub tells probes (after a start, cancel or
/// switch, at startup, after a config reload).
pub async fn refresh(state: &AppState) -> anyhow::Result<()> {
    let config = state.config.read().await.clone();
    let credentials = rotation::credentials(
        state.storage.get().as_ref(),
        &config,
        &state.hub_secret.get(),
    )
    .await?;
    state.hub_pins.set(credentials);
    Ok(())
}

/// Switch, and make the running hub use the new key and certificate.
/// `by`: the user who clicked "Switch now"; `None`: the hub (all ready).
pub async fn apply_switch(state: &AppState, by: Option<&str>) -> Result<Switched, RotationError> {
    let _one_at_a_time = SWITCHING.lock().await;
    let config = state.config.read().await.clone();
    let switched =
        rotation::switch(state.storage.get().as_ref(), &config, by, state.clock.now()).await?;
    if let Some(secret) = &switched.new_secret {
        state.hub_secret.set(secret.clone());
    }
    if switched.tls_changed {
        let server = &config.server;
        state
            .tls
            .reload(
                &server.resolved_tls_cert_path(),
                &server.resolved_tls_key_path(),
            )
            .await?;
    }
    refresh(state).await?;
    Ok(switched)
}

/// `GET /api/admin/hub-keys`.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<HubKeysOverview>> {
    let config = state.config.read().await.clone();
    let overview = rotation::overview(
        state.storage.get().as_ref(),
        &config,
        &state.hub_pins.get(),
        state.clock.now(),
    )
    .await
    .map_err(|e| ApiError::internal("hub keys", format!("{e:#}")))?;
    Ok(Json(overview))
}

/// `POST /api/admin/hub-keys/rotation`: start a rotation.
pub async fn start(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(request): ApiJson<StartRotationRequest>,
) -> ApiResult<Json<RotationInfo>> {
    let config = state.config.read().await.clone();
    let started = rotation::start(
        state.storage.get().as_ref(),
        &config,
        &request,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    refresh(&state).await.map_err(RotationError::Failed)?;
    let event = Event::new(action::ROTATION_START)
        .target("rotation", started.id.to_string())
        .details(serde_json::json!({
            "tls": request.tls, "key": request.key, "window_days": request.window_days,
        }));
    record(&state, Actor::user(&me.user, info.client_ip), event).await;
    Ok(Json(current(&state).await?))
}

/// `POST /api/admin/hub-keys/rotation/switch`: "Switch now". Probes that
/// don't hold the new pins can't reach the hub afterwards.
pub async fn switch(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let waiting = current(&state).await.map(|r| r.waiting.len()).unwrap_or(0);
    let switched = apply_switch(&state, Some(&me.user.username)).await?;
    let event = Event::new(action::ROTATION_SWITCH)
        .target("rotation", switched.rotation.id.to_string())
        .details(serde_json::json!({
            "probes_not_ready": waiting, "codes_revoked": switched.codes_revoked,
        }));
    record(&state, Actor::user(&me.user, info.client_ip), event).await;
    Ok(Json(serde_json::json!({
        "success": true,
        "probes_not_ready": waiting,
        "codes_revoked": switched.codes_revoked,
    })))
}

/// `POST /api/admin/hub-keys/rotation/cancel`.
pub async fn cancel(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let config = state.config.read().await.clone();
    let cancelled = rotation::cancel(
        state.storage.get().as_ref(),
        &config,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    refresh(&state).await.map_err(RotationError::Failed)?;
    let event = Event::new(action::ROTATION_CANCEL).target("rotation", cancelled.id.to_string());
    record(&state, Actor::user(&me.user, info.client_ip), event).await;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// The rotation in progress, as the page shows it.
async fn current(state: &AppState) -> Result<RotationInfo, RotationError> {
    let config = state.config.read().await.clone();
    rotation::overview(
        state.storage.get().as_ref(),
        &config,
        &state.hub_pins.get(),
        state.clock.now(),
    )
    .await?
    .rotation
    .ok_or(RotationError::NotFound)
}

async fn record(state: &AppState, actor: Actor, event: Event) {
    audit::record(
        state.storage.get().as_ref(),
        &actor,
        event,
        state.clock.now(),
    )
    .await;
}

/// One look: switch when every probe is ready, say once when overdue.
pub async fn check_once(state: &AppState) -> anyhow::Result<Tick> {
    let config = state.config.read().await.clone();
    let tick = rotation::tick(
        state.storage.get().as_ref(),
        &config,
        &state.hub_pins.get(),
        state.clock.now(),
    )
    .await?;
    match tick {
        Tick::Nothing => {}
        Tick::AllReady => {
            let switched = apply_switch(state, None).await?;
            let event = Event::new(action::ROTATION_SWITCH)
                .target("rotation", switched.rotation.id.to_string())
                .details(serde_json::json!({
                    "probes_not_ready": 0, "codes_revoked": switched.codes_revoked,
                }));
            record(state, Actor::hub(), event).await;
        }
        Tick::Overdue { waiting } => tracing::warn!(
            waiting,
            "The key rotation's window has passed and {waiting} probe(s) don't hold the new keys: \
             see the admin page (Hub keys) to switch now or wait"
        ),
    }
    // A key switch's previous key may have gone: follow the files.
    refresh(state).await?;
    Ok(tick)
}

/// Run [`check_once`] every minute, for as long as the hub runs.
pub fn spawn_checks(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            if let Err(e) = check_once(&state).await {
                tracing::error!("Key rotation check failed: {e:#}");
            }
        }
    });
}
