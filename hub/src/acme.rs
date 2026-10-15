#![cfg(feature = "ssr")]
//! The admin page's "HTTPS certificate" (M2-14, D-62): the hub's certificate
//! from Let's Encrypt or another ACME authority. Rules: `service::acme`;
//! this module maps them to HTTP, answers the authority's HTTP-01 requests,
//! renews in the background, and makes the HTTPS listener serve what was
//! issued (in place, no restart).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use tokio::sync::Notify;

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{AcmeSettings, CertificateOverview, CertificateSaved};
use crate::service::acme::{self, AcmeError, CertificateAuthority, Change, Outcome};
use crate::service::audit::{self, Actor, Event, action};

/// Where an ACME authority asks for HTTP-01 answers (RFC 8555 §8.3). The
/// plain-HTTP listener lets it through (`crate::server`).
pub const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

/// The authority the hub asks, and a way to make it look now.
pub struct Acme {
    authority: Arc<dyn CertificateAuthority>,
    wake: Notify,
}

impl Acme {
    pub fn new(authority: Arc<dyn CertificateAuthority>) -> Self {
        Self {
            authority,
            wake: Notify::new(),
        }
    }

    /// Look now rather than at the next hourly tick (a rotation announced a
    /// new certificate key: its certificate is wanted before the switch).
    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

impl From<AcmeError> for ApiError {
    fn from(error: AcmeError) -> Self {
        match &error {
            AcmeError::Invalid(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_certificate_settings",
                error.to_string(),
            ),
            AcmeError::Failed(e) => ApiError::internal("certificate", format!("{e:#}")),
        }
    }
}

/// Look whether a certificate is due (or, `force`, ask now), and serve what
/// was issued.
pub async fn run(state: &AppState, force: bool) -> anyhow::Result<Outcome> {
    let config = state.config.read().await.clone();
    let outcome = acme::run(
        state.storage.get().as_ref(),
        &config,
        state.acme.authority.as_ref(),
        state.tls.challenges(),
        force,
        state.clock.now(),
    )
    .await?;
    if let Outcome::Issued { serving: true, .. } = outcome {
        let server = &config.server;
        state
            .tls
            .reload(
                &server.resolved_tls_cert_path(),
                &server.resolved_tls_key_path(),
            )
            .await?;
    }
    Ok(outcome)
}

/// `GET /api/admin/certificate`.
pub async fn overview(State(state): State<AppState>) -> ApiResult<Json<CertificateOverview>> {
    Ok(Json(current(&state).await?))
}

async fn current(state: &AppState) -> ApiResult<CertificateOverview> {
    let config = state.config.read().await.clone();
    acme::overview(state.storage.get().as_ref(), &config, state.clock.now())
        .await
        .map_err(|e| ApiError::internal("certificate", format!("{e:#}")))
}

/// `POST /api/admin/certificate`: save the settings. Switched on (or new
/// names, or another authority): the certificate is asked for now, and the
/// answer says how that went; the settings stay saved either way.
/// Switched off: back to a self-signed certificate with the same key.
pub async fn save(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(settings): ApiJson<AcmeSettings>,
) -> ApiResult<Json<CertificateSaved>> {
    let config = state.config.read().await.clone();
    let store = state.storage.get();
    let (saved, change) = acme::save(
        store.as_ref(),
        &config,
        settings,
        Some(&me.user.username),
        state.clock.now(),
    )
    .await?;
    let actor = Actor::user(&me.user, info.client_ip);
    let event = Event::new(action::CERTIFICATE_SETTINGS).details(json!({
        "enabled": saved.enabled, "domains": saved.domains, "directory": saved.directory(),
        "challenge": saved.challenge.as_str(),
    }));
    record(&state, actor.clone(), event).await;
    let error = match change {
        Change::Nothing => None,
        Change::SelfSigned => {
            if acme::back_to_self_signed(store.as_ref(), &config)
                .await
                .map_err(AcmeError::Failed)?
            {
                let server = &config.server;
                state
                    .tls
                    .reload(
                        &server.resolved_tls_cert_path(),
                        &server.resolved_tls_key_path(),
                    )
                    .await
                    .map_err(AcmeError::Failed)?;
            }
            None
        }
        Change::Issue => issue_now(&state, actor).await?,
    };
    Ok(Json(CertificateSaved {
        success: true,
        error,
    }))
}

/// `POST /api/admin/certificate/renew`: "Renew now". 502 with the
/// authority's reason if it failed.
pub async fn renew(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let config = state.config.read().await.clone();
    let settings = acme::settings(state.storage.get().as_ref())
        .await
        .map_err(AcmeError::Failed)?;
    if !settings.enabled {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "acme_off",
            "the hub doesn't get its certificate from an ACME authority: turn it on first",
        ));
    }
    acme::check_listeners(&config, settings.challenge).map_err(AcmeError::Invalid)?;
    match issue_now(&state, Actor::user(&me.user, info.client_ip)).await? {
        None => Ok(Json(json!({ "success": true }))),
        Some(error) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "certificate_not_issued",
            error,
        )),
    }
}

/// Ask now; `Some(reason)` if the authority didn't issue. Audited.
async fn issue_now(state: &AppState, actor: Actor) -> Result<Option<String>, AcmeError> {
    let outcome = run(state, true).await?;
    audit_outcome(state, actor, &outcome).await;
    Ok(match outcome {
        Outcome::Failed(error) => Some(error),
        _ => None,
    })
}

async fn audit_outcome(state: &AppState, actor: Actor, outcome: &Outcome) {
    let event = match outcome {
        Outcome::Issued { count, .. } => {
            Event::new(action::CERTIFICATE_ISSUE).details(json!({ "certificates": count }))
        }
        Outcome::Failed(error) => {
            Event::new(action::CERTIFICATE_ISSUE_FAILED).details(json!({ "error": error }))
        }
        Outcome::Nothing | Outcome::Waiting(_) => return,
    };
    record(state, actor, event).await;
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

/// `GET /.well-known/acme-challenge/{token}`: the answer to an HTTP-01
/// challenge the hub is waiting on; 404 otherwise.
pub async fn http_challenge(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    match state.tls.challenges().http(&token) {
        Some(answer) => ([(header::CONTENT_TYPE, "text/plain")], answer).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// One look, as the background task makes it: renew what is due.
pub async fn check_once(state: &AppState) -> anyhow::Result<Outcome> {
    let outcome = run(state, false).await?;
    audit_outcome(state, Actor::hub(), &outcome).await;
    Ok(outcome)
}

/// Renew when due, looking every hour (and soon after the start, and when
/// woken), for as long as the hub runs.
pub fn spawn_renewals(state: AppState) {
    tokio::spawn(async move {
        // Not at once: the listeners start first (a TLS-ALPN challenge needs them).
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        loop {
            if let Err(e) = check_once(&state).await {
                tracing::error!("Certificate renewal check failed: {e:#}");
            }
            tokio::select! {
                () = tokio::time::sleep(acme::TICK) => {}
                () = state.acme.wake.notified() => {}
            }
        }
    });
}
