//! Alerting: deciding when a check or probe deserves a notification, and
//! handing it to a [`Notifier`]. Where it goes (channels and routing rules)
//! is `service::notify`'s business (D-66).

pub mod engine;
pub mod flap;
pub mod silence;
pub mod state;

use async_trait::async_trait;

pub use engine::{evaluate_alerts_for_report, evaluate_probe_expiry, spawn_in_order};

use crate::service::notify::Notification;

/// Takes an alert. In production `notify::Dispatcher`, which routes it to
/// channels; tests record what would be sent.
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, notification: Notification) -> anyhow::Result<()>;
}
