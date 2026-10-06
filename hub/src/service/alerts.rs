//! Alerting: deciding when a resource or probe deserves a notification, and
//! sending it through a [`Notifier`].

pub mod email;
pub mod engine;
pub mod flap;
pub mod silence;
pub mod state;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::HubConfig;
pub use email::AlertEmail;
pub use engine::{evaluate_alerts_for_report, evaluate_probe_expiry, spawn_in_order};

/// Delivers an alert. SMTP in production; tests record what would be sent.
/// (Proposal 0005 adds webhook/Teams/Slack channels as further implementations.)
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn send(&self, email: AlertEmail) -> anyhow::Result<()>;
}

/// Sends alerts by e-mail, reading the SMTP settings from the (hot-reloadable)
/// hub config at send time.
pub struct SmtpNotifier {
    config: Arc<RwLock<HubConfig>>,
}

impl SmtpNotifier {
    pub fn new(config: Arc<RwLock<HubConfig>>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Notifier for SmtpNotifier {
    async fn send(&self, email: AlertEmail) -> anyhow::Result<()> {
        let smtp = self.config.read().await.alerts.smtp.clone();
        email::send_alert_email(&smtp, email).await
    }
}
