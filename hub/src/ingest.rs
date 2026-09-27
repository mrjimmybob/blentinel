#![cfg(feature = "ssr")]
//! Probe-facing endpoints. Probes are not session-authenticated; they prove
//! identity cryptographically (sealed payload + Ed25519 signature + whitelist).

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use common::protocol::{
    self, TimestampRejection, MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_HEADER, PROTOCOL_VERSION,
};
use leptos::logging::log;
use x25519_dalek::PublicKey;

use crate::{alerts, db, AppState};

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
    with_protocol_header(process_report(&state, &body).await)
}

fn with_protocol_header(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(PROTOCOL_HEADER, HeaderValue::from(PROTOCOL_VERSION));
    response
}

async fn process_report(state: &AppState, body: &[u8]) -> Response {
    let decrypted = match common::crypto::open_from_probe(body, &state.hub_secret) {
        Ok(d) => d,
        Err(_e) => {
            log!("Security: Decryption failed. Potential unauthorized probe or corrupted data.");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };

    if state.args.debug {
        let pretty = serde_json::from_slice::<serde_json::Value>(&decrypted)
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or_else(|_| format!("{:?}", decrypted));
        println!("[DEBUG] Decrypted payload:\n{}", pretty);
    }

    let mut report: common::models::StatusReport = match serde_json::from_slice(&decrypted) {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let version = report.effective_protocol_version();
    if !(MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&version) {
        log!(
            "Rejected report with unsupported protocol version {} (supported {}..={}).",
            version,
            MIN_SUPPORTED_PROTOCOL_VERSION,
            PROTOCOL_VERSION
        );
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported protocol version {version}"),
        )
            .into_response();
    }

    // Whitelist check: reject any probe we haven't explicitly registered.
    let probe_name = {
        let whitelist = state.config.read().await.probe_whitelist();
        match whitelist.get(&report.probe_id) {
            Some(name) => name.clone(),
            None => {
                log!(
                    "Security: Report from unregistered probe {}. Rejected.",
                    crate::model::short_probe_id(&report.probe_id)
                );
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    };

    // Signature verification (the probe ID is the hex of the probe's public key).
    // Covers the full report: company_id, hostname, site, timestamp, resources,
    // protocol_version. A signature is mandatory: the probe ID and the hub key are
    // both public, so without it anyone could forge reports (D-25).
    let Some(sig) = report.signature.take() else {
        log!(
            "Security Alert: Unsigned report claiming to be {}. Rejected.",
            report.probe_id
        );
        return StatusCode::FORBIDDEN.into_response();
    };
    let signed_data = match serde_json::to_vec(&report.to_signable()) {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if common::crypto::verify(&signed_data, &sig, &report.probe_id).is_err() {
        log!(
            "Security Alert: Invalid signature from {}!",
            report.probe_id
        );
        return StatusCode::FORBIDDEN.into_response();
    }
    report.signature = Some(sig);

    let short_id = crate::model::short_probe_id(&report.probe_id);
    let now = state.clock.now();

    // Replay protection, part 1: the (signed) timestamp must be recent. Checked
    // after the signature so an attacker can't probe the window with fake reports.
    if let Err(rejection) = protocol::check_report_timestamp(report.timestamp, now) {
        let reason = match rejection {
            TimestampRejection::InFuture => {
                "timestamp is in the future (probe clock ahead of hub?)"
            }
            TimestampRejection::TooOld => "timestamp is older than any probe buffers reports",
        };
        log!(
            "Rejected report from {} at {}: {}",
            short_id,
            report.timestamp,
            reason
        );
        return (StatusCode::UNPROCESSABLE_ENTITY, reason).into_response();
    }

    if state.args.verbose {
        println!(
            "[{}] Report received from {} ({}) — {} resources",
            report.timestamp.format("%H:%M:%S"),
            probe_name,
            short_id,
            report.resources.len()
        );
    }

    if state.args.debug {
        println!(
            "[DEBUG] Saving report: probe={}, company={}, resources={}",
            short_id,
            report.company_id,
            report.resources.len()
        );
    }

    // Replay protection, part 2: stored only if strictly newer than this probe's
    // latest report (atomic check-and-insert in the repository).
    match db::save_report(&state.pool, &report).await {
        Ok(db::SaveOutcome::Stored) => {}
        Ok(db::SaveOutcome::NotNewer) => {
            log!(
                "Report from {} at {} is not newer than its latest stored report (replay or duplicate delivery). Ignored.",
                short_id,
                report.timestamp
            );
            return (
                StatusCode::CONFLICT,
                "already have this or a newer report from this probe",
            )
                .into_response();
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to save report from {}: {}", short_id, e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Refresh this probe's heartbeat: marks it active and records now.
    // A failure here does not invalidate the report (already persisted).
    if let Err(e) =
        db::upsert_heartbeat(&state.pool, &report.probe_id, &report.company_id, now).await
    {
        eprintln!("[ERROR] Failed to update heartbeat for {}: {}", short_id, e);
    }

    // The probe is alive: re-arm its "probe offline" alert so the next outage
    // alerts too (D-27). Done here, not in the alert task, because that task
    // exits early when alerts are disabled and the state must stay correct.
    if let Err(e) =
        alerts::state::rearm_probe_expiry_alert(&state.pool, &report.company_id, &report.probe_id)
            .await
    {
        eprintln!(
            "[ERROR] Failed to re-arm expiry alert for {}: {}",
            short_id, e
        );
    }

    // Evaluate alerts in the background so the probe gets its response immediately.
    let alert_pool = state.pool.clone();
    let alert_config = std::sync::Arc::clone(&state.config);
    let alert_report = report.clone();
    tokio::spawn(async move {
        if let Err(e) =
            alerts::evaluate_alerts_for_report(&alert_pool, alert_config, &alert_report, now).await
        {
            eprintln!("[ALERT ERROR] Failed to evaluate alerts: {}", e);
        }
    });

    if state.args.verbose {
        println!(
            "[{}] Report saved for {} ({}).",
            report.timestamp.format("%H:%M:%S"),
            probe_name,
            short_id
        );
    }
    StatusCode::OK.into_response()
}
