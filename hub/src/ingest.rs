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

use common::ack::{self, ReportAck};
use x25519_dalek::StaticSecret;

use crate::AppState;
use crate::error::ApiError;
use crate::model::short_probe_id;
use crate::service::ingest::{self, IngestDeps, IngestError};

/// `POST /api/report`: receive a sealed, signed `StatusReport` from a probe.
///
/// | Status | Error code | Meaning | What the probe does |
/// |---|---|---|---|
/// | 200 | | stored; the body is a sealed [`ReportAck`], with the probe's config if it runs another one (D-49) | delete from its buffer; apply the config |
/// | 409 | `not_newer` | already have this or a newer report (replay / duplicate delivery) | delete: nothing to resend |
/// | 400 | `not_a_report`, `unsupported_protocol_version` | not a report, or unsupported protocol version | drop (permanent) |
/// | 401 | `undecryptable` | cannot be opened (wrong hub key / corrupted) | drop (permanent) |
/// | 403 | `unknown_probe`, `missing_signature`, `bad_signature` | not enrolled, missing or bad signature | drop (permanent) |
/// | 403 | (`probe_revoked`) | revoked; the body is a sealed [`ReportAck`] with `revoked` | stop reporting until enrolled again |
/// | 422 | `timestamp_rejected` | timestamp outside the accepted window (clock skew / too old) | drop (permanent) |
/// | 500 | `internal` | storage failure | keep and retry |
///
/// Other errors have the JSON body `common::protocol::ErrorBody`. Every
/// response carries the hub's protocol version in [`PROTOCOL_HEADER`]. Acks
/// go only to a probe whose signature checked out (D-48).
pub async fn handle_probe_report(State(state): State<AppState>, body: Bytes) -> Response {
    // The current database and key (a config reload may replace them).
    let (storage, hub_secret) = (state.storage.get(), state.hub_secret.get());
    let releases_dir = crate::releases::releases_dir(&state).await;
    let deps = IngestDeps {
        storage: &storage,
        config: &state.config,
        hub_secret: &hub_secret,
        notifier: &state.notifier,
        releases_dir: &releases_dir,
        release_keys: &state.release_keys,
    };
    let now = state.clock.now();
    let response = match ingest::ingest_report(deps, &body, now).await {
        Ok(accepted) => {
            let report = &accepted.report;
            tracing::debug!(
                probe = %short_probe_id(&report.probe_id),
                name = %accepted.probe_name,
                results = report.results.len(),
                config_sent = accepted.config.is_some(),
                update_offered = accepted.update.is_some(),
                "Report saved"
            );
            tracing::trace!(
                "Accepted report:\n{}",
                serde_json::to_string_pretty(report).unwrap_or_default()
            );
            let mut answer = ack_for(&body, now, false);
            answer.config = accepted.config;
            answer.update = accepted.update;
            ack_response(StatusCode::OK, &answer, &hub_secret, &report.probe_id)
        }
        Err(IngestError::Revoked(ref probe_id)) => {
            tracing::warn!(
                probe = %short_probe_id(probe_id),
                "Security: rejected report from a revoked probe"
            );
            let answer = ack_for(&body, now, true);
            ack_response(StatusCode::FORBIDDEN, &answer, &hub_secret, probe_id)
        }
        Err(error) => rejection(&error).into_response(),
    };
    with_protocol_header(response)
}

fn ack_for(sealed_report: &[u8], now: chrono::DateTime<chrono::Utc>, revoked: bool) -> ReportAck {
    ReportAck {
        report_sha256: ack::report_sha256(sealed_report),
        hub_time: now,
        revoked,
        config: None,
        update: None,
    }
}

/// `status` with the ack, signed and sealed for the probe, as the body.
fn ack_response(
    status: StatusCode,
    answer: &ReportAck,
    hub_secret: &StaticSecret,
    probe_id: &str,
) -> Response {
    match ack::seal_ack(answer, hub_secret, probe_id) {
        Ok(sealed) => (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            sealed,
        )
            .into_response(),
        // Can't happen: the probe ID was a valid key to check the signature.
        Err(e) => {
            tracing::error!("Can't seal the answer for probe {probe_id}: {e}");
            status.into_response()
        }
    }
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
        IngestError::Revoked(_) => (StatusCode::FORBIDDEN, "probe_revoked"), // answered with an ack
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

pub(crate) fn with_protocol_header(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(PROTOCOL_HEADER, HeaderValue::from(PROTOCOL_VERSION));
    response
}
