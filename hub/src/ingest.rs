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
/// | 200 | | stored; the body is a sealed [`ReportAck`], with the probe's config if it runs another one (D-49), an update offer (D-55), pending commands (D-56) and the hub's next pins or its switch (D-61) | delete from its buffer; apply the config; run the commands; take the pins |
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
    // The current database and keys (a reload or a rotation may replace them).
    let (storage, hub_secret) = (state.storage.get(), state.hub_secret.get());
    let credentials = state.hub_pins.get();
    let releases_dir = crate::releases::releases_dir(&state).await;
    let release_keys = state.trusted_release_keys().await;
    let deps = IngestDeps {
        storage: &storage,
        config: &state.config,
        hub_secret: &hub_secret,
        credentials: &credentials,
        notifier: &state.notifier,
        releases_dir: &releases_dir,
        release_keys: &release_keys,
        data_key: &state.delivery.key,
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
                commands = accepted.commands.len(),
                "Report saved"
            );
            tracing::trace!(
                "Accepted report:\n{}",
                serde_json::to_string_pretty(report).unwrap_or_default()
            );
            let mut answer = ack_for(&body, now, false);
            answer.config = accepted.config;
            answer.update = accepted.update;
            answer.commands = accepted.commands;
            answer.rotation = accepted.rotation;
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

/// `POST /api/probe/commands`: a probe asks for its commands between
/// reports (M4-03, D-76). The body is a sealed, signed
/// `common::poll::CommandPoll`.
///
/// | Status | Error code | Meaning |
/// |---|---|---|
/// | 200 | | the body is a sealed `PollAnswer`: the commands waiting, and when to ask again. Held up to `HOLD_SECS` while none are |
/// | 400 | `not_a_poll`, `unsupported_protocol_version` | not a poll |
/// | 401 | `undecryptable` | can't be opened |
/// | 403 | `unknown_probe`, `probe_revoked`, `bad_signature` | not an enrolled, active probe |
/// | 422 | `timestamp_rejected` | its time is too far from the hub's |
/// | 500 | `internal` | storage failure |
///
/// A probe treats every failure alike: it asks again later, and its
/// reports, which carry the same commands, go on as before.
pub async fn handle_command_poll(State(state): State<AppState>, body: Bytes) -> Response {
    use crate::service::poll::{self, PollRefusal};
    use common::poll::PollError;

    let (storage, hub_secret) = (state.storage.get(), state.hub_secret.get());
    let credentials = state.hub_pins.get();
    let secrets: Vec<&StaticSecret> = std::iter::once(&hub_secret)
        .chain(credentials.prev_secret.as_ref())
        .collect();
    let clock = state.clock.clone();
    let outcome = poll::answer(
        storage.as_ref(),
        &state.command_waiters,
        &secrets,
        &body,
        || clock.now(),
    )
    .await;
    let response = match outcome {
        Ok((poll, answer)) => {
            if !answer.commands.is_empty() {
                tracing::debug!(
                    probe = %short_probe_id(&poll.probe_id),
                    commands = answer.commands.len(),
                    "Commands sent to a probe that asked"
                );
            }
            let sealed = common::poll::seal_answer(
                &common::poll::PollAnswer {
                    poll_sha256: ack::report_sha256(&body),
                    hub_time: state.clock.now(),
                    commands: answer.commands,
                    next_poll_secs: answer.next_poll_secs,
                },
                &hub_secret,
                &poll.probe_id,
            );
            match sealed {
                Ok(sealed) => (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
                    sealed,
                )
                    .into_response(),
                Err(e) => ApiError::internal("sealing a poll answer", e).into_response(),
            }
        }
        Err(refusal) => {
            let (status, code) = match &refusal {
                PollRefusal::Storage(_) => {
                    return with_protocol_header(
                        ApiError::internal("Answering a command poll failed", &refusal)
                            .into_response(),
                    );
                }
                PollRefusal::Poll(PollError::Undecryptable) => {
                    (StatusCode::UNAUTHORIZED, "undecryptable")
                }
                PollRefusal::Poll(PollError::NotAPoll) => (StatusCode::BAD_REQUEST, "not_a_poll"),
                PollRefusal::Poll(PollError::UnsupportedVersion(_)) => {
                    (StatusCode::BAD_REQUEST, "unsupported_protocol_version")
                }
                PollRefusal::Poll(PollError::BadSignature) => {
                    (StatusCode::FORBIDDEN, "bad_signature")
                }
                PollRefusal::Poll(PollError::BadTime) => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "timestamp_rejected")
                }
                PollRefusal::UnknownProbe(_) => (StatusCode::FORBIDDEN, "unknown_probe"),
                PollRefusal::Revoked(_) => (StatusCode::FORBIDDEN, "probe_revoked"),
            };
            tracing::warn!("Refused a command poll: {refusal}");
            ApiError::new(status, code, refusal.to_string()).into_response()
        }
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
        commands: vec![],
        rotation: None,
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
