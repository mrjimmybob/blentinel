#![cfg(feature = "ssr")]

use super::flap::FlapState;
use crate::storage::AlertStore;
use anyhow::Result;
use chrono::{DateTime, Utc};
use common::checks::{CheckId, Health};

// Alert state is kept per key: a check's ID for its Up/Down state (D-49),
// and the keys below for the rest.

/// Alert-state key for a check's usage threshold (disk/CPU/memory %).
/// Persisted: changing the format orphans existing state.
pub fn threshold_key(check_id: &CheckId) -> String {
    format!("{check_id}-threshold")
}

/// Status under [`threshold_key`] after the threshold alert was sent.
pub const THRESHOLD_EXCEEDED: &str = "threshold_exceeded";
/// Status under [`threshold_key`] after the metric dropped back below the limit.
pub const THRESHOLD_OK: &str = "threshold_ok";

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

/// Check if an alert should be sent based on previous state
pub async fn should_send_alert(
    store: &dyn AlertStore,
    key: &str,
    current_status: &str,
) -> Result<bool> {
    let previous = store.alert_state(key).await?;

    match previous {
        None => {
            // First time seeing this key: alert if it's down or over its threshold.
            Ok(current_status != Health::Up.as_str())
        }
        Some(prev) => {
            // Send alert if status changed
            Ok(prev.last_status != current_status)
        }
    }
}

/// Update alert state after evaluation
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

/// How a flap state (D-63) is stored: the confirmed status's text when
/// failing and when fine (`Down`/`Up`, or a threshold's).
#[derive(Debug, Clone, Copy)]
pub struct Statuses {
    pub failing: &'static str,
    pub fine: &'static str,
}

/// A check's Up/Down state.
pub const UP_DOWN: Statuses = Statuses {
    failing: "Down",
    fine: "Up",
};

/// A check's usage threshold state, under [`threshold_key`].
pub const THRESHOLD: Statuses = Statuses {
    failing: THRESHOLD_EXCEEDED,
    fine: THRESHOLD_OK,
};

/// The flap state stored under `key`, if any.
pub async fn flap_state(
    store: &dyn AlertStore,
    key: &str,
    statuses: Statuses,
) -> Result<Option<FlapState>> {
    Ok(store.alert_state(key).await?.map(|s| FlapState {
        failing: s.last_status == statuses.failing,
        streak: s.streak,
    }))
}

/// Store a flap state under `key`; `alert_sent` stamps when it was told.
pub async fn save_flap_state(
    store: &dyn AlertStore,
    key: &str,
    statuses: Statuses,
    state: FlapState,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    let status = match state.failing {
        true => statuses.failing,
        false => statuses.fine,
    };
    store
        .upsert_alert_state(key, status, state.streak, alert_sent, now)
        .await
}
