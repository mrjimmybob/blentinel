#![cfg(feature = "ssr")]

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use anyhow::Result;

/// Stable key of a monitored resource (defined once, in `crate::model`).
pub use crate::model::resource_key;

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
pub async fn rearm_probe_expiry_alert(pool: &SqlitePool, company_id: &str, probe_id: &str) -> Result<()> {
    crate::db::set_alert_status_if_present(pool, &probe_expiry_key(company_id, probe_id), PROBE_REPORTING).await
}

/// Check if an alert should be sent based on previous state
pub async fn should_send_alert(
    pool: &SqlitePool,
    resource_key: &str,
    current_status: &str,
) -> Result<bool> {
    let previous = crate::db::get_alert_state(pool, resource_key).await?;

    match previous {
        None => {
            // First time seeing this resource - send alert if it's down or threshold exceeded
            Ok(current_status != "Up")
        }
        Some(prev) => {
            // Send alert if status changed
            Ok(prev.last_status != current_status)
        }
    }
}

/// Update alert state after evaluation
pub async fn update_alert_state(
    pool: &SqlitePool,
    resource_key: &str,
    status: &str,
    alert_sent: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    crate::db::upsert_alert_state(pool, resource_key, status, alert_sent, now).await
}
