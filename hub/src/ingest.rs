#![cfg(feature = "ssr")]
//! Probe-facing endpoints. Probes are not session-authenticated; they prove
//! identity cryptographically (sealed payload + Ed25519 signature + whitelist).

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use leptos::logging::log;
use x25519_dalek::PublicKey;

use crate::{alerts, db, AppState};

/// `GET /api/handshake`: the hub's X25519 public key, hex encoded.
pub async fn handle_handshake(State(state): State<AppState>) -> impl IntoResponse {
    if state.args.debug {
        println!("[DEBUG] Handshake requested. Serving Hub public key.");
    }
    hex::encode(PublicKey::from(&state.hub_secret).as_bytes())
}

/// `POST /api/report`: receive a sealed, signed `StatusReport` from a probe.
///
/// Responses: 200 stored · 400 not a report · 401 cannot be opened ·
/// 403 unknown probe, missing or bad signature · 500 storage failure.
pub async fn handle_probe_report(State(state): State<AppState>, body: Bytes) -> Response {
    let decrypted = match common::crypto::open_from_probe(&body, &state.hub_secret) {
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

    // Whitelist check: reject any probe we haven't explicitly registered.
    let probe_name = {
        let whitelist = state.config.read().await.probe_whitelist();
        match whitelist.get(&report.probe_id) {
            Some(name) => name.clone(),
            None => {
                log!(
                    "Security: Report from unregistered probe {}. Rejected.",
                    &report.probe_id[..8.min(report.probe_id.len())]
                );
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    };

    // Signature verification (the probe ID is the hex of the probe's public key).
    // Covers the full report: company_id, hostname, site, timestamp, resources.
    // A signature is mandatory: the probe ID and the hub key are both public, so
    // without it anyone could forge reports for a registered probe (D-25).
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

    let short_id = &report.probe_id[..8.min(report.probe_id.len())];
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

    if let Err(e) = db::save_report(&state.pool, &report).await {
        eprintln!("[ERROR] Failed to save report from {}: {}", short_id, e);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let now = state.clock.now();

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
