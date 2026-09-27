#![cfg(feature = "ssr")]
//! Probe-facing HTTP endpoints. The rules live in [`crate::service::ingest`];
//! this module only maps them to HTTP (status codes, headers, CLI-flag output).

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use common::protocol::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use leptos::logging::log;
use x25519_dalek::PublicKey;

use crate::AppState;
use crate::model::short_probe_id;
use crate::service::ingest::{self, IngestDeps, IngestError, timestamp_reason};

/// `GET /api/handshake`: the hub's X25519 public key, hex encoded.
pub async fn handle_handshake(State(state): State<AppState>) -> Response {
    if state.args.debug {
        println!("[DEBUG] Handshake requested. Serving Hub public key.");
    }
    let key = hex::encode(PublicKey::from(&state.hub_secret).as_bytes());
    with_protocol_header(key.into_response())
}

/// `POST /api/report`: receive a sealed, signed `StatusReport` from a probe.
///
/// | Status | Meaning | What the probe does |
/// |---|---|---|
/// | 200 | stored | delete from its buffer |
/// | 409 | already have this or a newer report (replay / duplicate delivery) | delete: nothing to resend |
/// | 400 | not a report, or unsupported protocol version | drop (permanent) |
/// | 401 | cannot be opened (wrong hub key / corrupted) | drop (permanent) |
/// | 403 | unknown probe, missing or bad signature | drop (permanent) |
/// | 422 | timestamp outside the accepted window (clock skew / too old) | drop (permanent) |
/// | 500 | storage failure | keep and retry |
///
/// Every response carries the hub's protocol version in [`PROTOCOL_HEADER`].
pub async fn handle_probe_report(State(state): State<AppState>, body: Bytes) -> Response {
    let deps = IngestDeps {
        storage: &state.storage,
        config: &state.config,
        hub_secret: &state.hub_secret,
        notifier: &state.notifier,
    };
    let response = match ingest::ingest_report(deps, &body, state.clock.now()).await {
        Ok(accepted) => {
            let report = &accepted.report;
            if state.args.verbose {
                println!(
                    "[{}] Report saved for {} ({}) — {} resources",
                    report.timestamp.format("%H:%M:%S"),
                    accepted.probe_name,
                    short_probe_id(&report.probe_id),
                    report.resources.len()
                );
            }
            if state.args.debug {
                let pretty = serde_json::to_string_pretty(report).unwrap_or_default();
                println!("[DEBUG] Accepted report:\n{}", pretty);
            }
            StatusCode::OK.into_response()
        }
        Err(error) => rejection_response(&error),
    };
    with_protocol_header(response)
}

/// Map an ingest failure to its HTTP response, logging it.
fn rejection_response(error: &IngestError) -> Response {
    match error {
        IngestError::Storage(_) => eprintln!("[ERROR] {}", error),
        IngestError::Undecryptable
        | IngestError::UnknownProbe(_)
        | IngestError::MissingSignature(_)
        | IngestError::BadSignature(_) => log!("Security: rejected report: {}", error),
        _ => log!("Rejected report: {}", error),
    }
    match error {
        IngestError::Undecryptable => StatusCode::UNAUTHORIZED.into_response(),
        IngestError::NotAReport => StatusCode::BAD_REQUEST.into_response(),
        IngestError::UnsupportedVersion(version) => (
            StatusCode::BAD_REQUEST,
            format!("unsupported protocol version {version}"),
        )
            .into_response(),
        IngestError::UnknownProbe(_)
        | IngestError::MissingSignature(_)
        | IngestError::BadSignature(_) => StatusCode::FORBIDDEN.into_response(),
        IngestError::TimestampRejected { rejection, .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            timestamp_reason(*rejection),
        )
            .into_response(),
        IngestError::NotNewer { .. } => (
            StatusCode::CONFLICT,
            "already have this or a newer report from this probe",
        )
            .into_response(),
        IngestError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn with_protocol_header(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(PROTOCOL_HEADER, HeaderValue::from(PROTOCOL_VERSION));
    response
}
