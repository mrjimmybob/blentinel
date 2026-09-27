#![cfg(feature = "ssr")]
//! Probe-facing HTTP endpoints. The rules live in [`crate::service::ingest`];
//! this module only maps them to HTTP (status codes, error codes, headers).

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use common::protocol::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use x25519_dalek::PublicKey;

use crate::AppState;
use crate::error::ApiError;
use crate::model::short_probe_id;
use crate::service::ingest::{self, IngestDeps, IngestError};

/// `GET /api/handshake`: the hub's X25519 public key, hex encoded.
pub async fn handle_handshake(State(state): State<AppState>) -> Response {
    tracing::debug!("Handshake requested; serving the hub public key");
    let key = hex::encode(PublicKey::from(&state.hub_secret.get()).as_bytes());
    with_protocol_header(key.into_response())
}

/// `POST /api/report`: receive a sealed, signed `StatusReport` from a probe.
///
/// | Status | Error code | Meaning | What the probe does |
/// |---|---|---|---|
/// | 200 | | stored | delete from its buffer |
/// | 409 | `not_newer` | already have this or a newer report (replay / duplicate delivery) | delete: nothing to resend |
/// | 400 | `not_a_report`, `unsupported_protocol_version` | not a report, or unsupported protocol version | drop (permanent) |
/// | 401 | `undecryptable` | cannot be opened (wrong hub key / corrupted) | drop (permanent) |
/// | 403 | `unknown_probe`, `missing_signature`, `bad_signature` | unknown probe, missing or bad signature | drop (permanent) |
/// | 422 | `timestamp_rejected` | timestamp outside the accepted window (clock skew / too old) | drop (permanent) |
/// | 500 | `internal` | storage failure | keep and retry |
///
/// Errors have the JSON body `common::protocol::ErrorBody`. Every response
/// carries the hub's protocol version in [`PROTOCOL_HEADER`].
pub async fn handle_probe_report(State(state): State<AppState>, body: Bytes) -> Response {
    // The current database and key (a config reload may replace them).
    let (storage, hub_secret) = (state.storage.get(), state.hub_secret.get());
    let deps = IngestDeps {
        storage: &storage,
        config: &state.config,
        hub_secret: &hub_secret,
        notifier: &state.notifier,
    };
    let response = match ingest::ingest_report(deps, &body, state.clock.now()).await {
        Ok(accepted) => {
            let report = &accepted.report;
            tracing::debug!(
                probe = %short_probe_id(&report.probe_id),
                name = %accepted.probe_name,
                resources = report.resources.len(),
                "Report saved"
            );
            tracing::trace!(
                "Accepted report:\n{}",
                serde_json::to_string_pretty(report).unwrap_or_default()
            );
            StatusCode::OK.into_response()
        }
        Err(error) => rejection(&error).into_response(),
    };
    with_protocol_header(response)
}

/// Map an ingest failure to its HTTP error, logging it.
fn rejection(error: &IngestError) -> ApiError {
    let (status, code) = match error {
        IngestError::Storage(_) => return ApiError::internal("Storing a report failed", error),
        IngestError::Undecryptable => (StatusCode::UNAUTHORIZED, "undecryptable"),
        IngestError::NotAReport => (StatusCode::BAD_REQUEST, "not_a_report"),
        IngestError::UnsupportedVersion(_) => {
            (StatusCode::BAD_REQUEST, "unsupported_protocol_version")
        }
        IngestError::UnknownProbe(_) => (StatusCode::FORBIDDEN, "unknown_probe"),
        IngestError::MissingSignature(_) => (StatusCode::FORBIDDEN, "missing_signature"),
        IngestError::BadSignature(_) => (StatusCode::FORBIDDEN, "bad_signature"),
        IngestError::TimestampRejected { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, "timestamp_rejected")
        }
        IngestError::NotNewer { .. } => (StatusCode::CONFLICT, "not_newer"),
    };
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            tracing::warn!("Security: rejected report: {error}")
        }
        _ => tracing::warn!("Rejected report: {error}"),
    }
    ApiError::new(status, code, error.to_string())
}

fn with_protocol_header(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(PROTOCOL_HEADER, HeaderValue::from(PROTOCOL_VERSION));
    response
}
