#![cfg(feature = "ssr")]

use sqlx::SqlitePool;
use anyhow::Result;

/// Generate a stable resource key for alert state tracking
pub fn generate_resource_key(
    company_id: &str,
    probe_id: &str,
    resource_name: &str,
    target: &str,
) -> String {
    format!("{}:{}:{}:{}", company_id, probe_id, resource_name, target)
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
) -> Result<()> {
    crate::db::upsert_alert_state(pool, resource_key, status, alert_sent).await
}
