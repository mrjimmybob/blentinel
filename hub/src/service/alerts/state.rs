#![cfg(feature = "ssr")]

use super::flap::FlapState;
use crate::model::Status;
use crate::storage::AlertStore;
use anyhow::Result;
use chrono::{DateTime, Utc};

// Alert state is kept per key: a check's ID for its status (D-49, D-64:
// Up, Warning or Down, thresholds included), and the key below for probes.

/// Alert-state key for "probe stopped reporting" alerts.
pub fn probe_expiry_key(company_id: &str, probe_id: &str) -> String {
    format!("probe_expiry:{}:{}", company_id, probe_id)
}

/// Status stored under [`probe_expiry_key`] once the "probe offline" alert was sent.
pub const PROBE_EXPIRED: &str = "expired";
/// Status stored under [`probe_expiry_key`] after the probe reports again.
pub const PROBE_REPORTING: &str = "active";

/// Re-arm the "probe offline" alert because the probe is reporting again, so
/// its *next* outage alerts too (D-27). No-op if this probe never alerted.
pub async fn rearm_probe_expiry_alert(
    store: &dyn AlertStore,
    company_id: &str,
    probe_id: &str,
) -> Result<()> {
    store
        .set_alert_status_if_present(&probe_expiry_key(company_id, probe_id), PROBE_REPORTING)
        .await
}

/// Should the probe-expiry alert under `key` be sent, given its new status?
/// Yes the first time it isn't Up, and whenever it changes.
pub async fn should_send_alert(
    store: &dyn AlertStore,
    key: &str,
    current_status: &str,
) -> Result<bool> {
    Ok(match store.alert_state(key).await? {
        None => current_status != Status::Up.as_str(),
        Some(prev) => prev.last_status != current_status,
    })
}

/// Record the probe-expiry state under `key`.
pub async fn update_alert_state(
    store: &dyn AlertStore,
    key: &str,
    status: &str,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    store
        .upsert_alert_state(key, status, 0, alert_sent, now)
        .await
}

/// A check's alert state, if it has one.
pub async fn flap_state(store: &dyn AlertStore, key: &str) -> Result<Option<FlapState>> {
    Ok(store.alert_state(key).await?.map(|s| FlapState {
        // Anything unexpected counts as Up: the next results set it right.
        confirmed: Status::parse(&s.last_status).unwrap_or(Status::Up),
        streak: s.streak,
    }))
}

/// Store a check's alert state; `alert_sent` stamps when people were told.
pub async fn save_flap_state(
    store: &dyn AlertStore,
    key: &str,
    state: FlapState,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    store
        .upsert_alert_state(key, state.confirmed.as_str(), state.streak, alert_sent, now)
        .await
}
