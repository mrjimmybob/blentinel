//! Sending alerts out (M3-03, proposal 0005 §2.6, D-66): routing rules pick
//! the channels for each [`Notification`], and each channel (e-mail, a
//! webhook, Teams, Slack, Discord, Telegram) gets it in its own shape.
//!
//! A delivery runs in the background, tried up to three times (a minute,
//! then four, apart); the channel records how its last delivery went, which
//! the alerts page shows. Retries live in memory: a restart drops them.

pub mod channels;
pub mod message;
pub mod payload;
pub mod transport;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

pub use message::{CheckRef, IncidentRef, Notification, Nudge};
use payload::{MailServer, OpenChannel};
use transport::Transport;

use crate::clock::Clock;
use crate::model::{AlertKind, follow_up, route};
use crate::secrets::DataKey;
use crate::service::alerts::Notifier;
use crate::state::Reloadable;
use crate::storage::{Storage, StoredChannel};

/// The waits before the second and third tries.
pub const RETRIES: [Duration; 2] = [Duration::from_secs(60), Duration::from_secs(240)];

/// What delivering needs: the data key for secrets, a transport, a clock.
pub struct Delivery {
    pub key: Arc<DataKey>,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    /// The waits between tries ([`RETRIES`]; tests: none).
    pub retries: Vec<Duration>,
}

/// The hub's address for links, and its mail server (if set).
pub struct Sending {
    pub hub_url: String,
    pub mail: Option<MailServer>,
}

impl Delivery {
    /// The settings a delivery needs, the mail password opened.
    pub async fn sending(&self, store: &dyn Storage) -> anyhow::Result<Sending> {
        let Some(s) = store.alert_settings().await? else {
            return Ok(Sending {
                hub_url: String::new(),
                mail: None,
            });
        };
        let password = match &s.smtp_password_sealed {
            Some(sealed) => self.key.open(sealed)?,
            None => String::new(),
        };
        Ok(Sending {
            hub_url: s.hub_url,
            mail: (!s.smtp_server.is_empty()).then_some(MailServer {
                server: s.smtp_server,
                port: s.smtp_port,
                security: s.smtp_security,
                username: s.smtp_username,
                password,
                from: s.smtp_from,
            }),
        })
    }

    /// A stored channel, its secret opened.
    pub fn open(&self, channel: &StoredChannel) -> anyhow::Result<OpenChannel> {
        Ok(OpenChannel {
            id: channel.id,
            name: channel.name.clone(),
            kind: channel.kind,
            target: channel.target.clone(),
            secret: channel
                .secret_sealed
                .as_deref()
                .map(|s| self.key.open(s))
                .transpose()?,
        })
    }

    /// Deliver `n` to `channel` once, and record how it went.
    pub async fn try_once(
        &self,
        store: &dyn Storage,
        channel: &OpenChannel,
        n: &Notification,
        sending: &Sending,
    ) -> anyhow::Result<()> {
        let link = n.link(&sending.hub_url);
        let result = async {
            let outgoing = payload::outgoing(
                channel,
                n,
                link.as_deref(),
                sending.mail.as_ref(),
                self.clock.now(),
            )?;
            self.transport.deliver(&outgoing).await
        }
        .await;
        let error = result.as_ref().err().map(|e| format!("{e:#}"));
        store
            .record_delivery(channel.id, self.clock.now(), error.as_deref())
            .await?;
        result
    }
}

/// Delivers notifications as the routing rules say (production's [`Notifier`]).
pub struct Dispatcher {
    storage: Reloadable<Arc<dyn Storage>>,
    delivery: Arc<Delivery>,
}

impl Dispatcher {
    pub fn new(storage: Reloadable<Arc<dyn Storage>>, delivery: Arc<Delivery>) -> Self {
        Self { storage, delivery }
    }

    /// Route `n` and deliver it to each channel, in the background. Returns
    /// the deliveries, for tests to wait on.
    pub async fn dispatch(
        &self,
        n: Notification,
    ) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>> {
        let store = self.storage.get();
        let ids = match &n.nudge {
            // A reminder or an escalation names its channels (D-69).
            Some(nudge) => nudge.channels().to_vec(),
            None => {
                let rules = store.route_rules().await?;
                let check_kind = n.check.as_ref().map(|c| c.kind.as_str());
                let mut ids = route(&rules, n.kind, &n.company_id, &n.site_id, check_kind);
                // The recovery of an escalated incident goes where it was escalated.
                if let (AlertKind::Recovered, Some(as_kind)) =
                    (n.kind, n.incident.and_then(|i| i.escalated))
                {
                    let follow = follow_up(&rules, as_kind, &n.company_id, &n.site_id, check_kind);
                    for id in follow.escalate.map(|(_, ids)| ids).unwrap_or_default() {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
                ids
            }
        };
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sending = Arc::new(self.delivery.sending(store.as_ref()).await?);
        let n = Arc::new(n);
        let mut tasks = Vec::new();
        for id in ids {
            let Some(stored) = store.channel(id).await? else {
                continue;
            };
            if !stored.enabled {
                continue;
            }
            let channel = match self.delivery.open(&stored) {
                Ok(channel) => channel,
                Err(e) => {
                    let error = format!("{e:#}");
                    store
                        .record_delivery(id, self.delivery.clock.now(), Some(&error))
                        .await?;
                    tracing::error!(channel = %stored.name, "Can't deliver an alert: {error}");
                    continue;
                }
            };
            let (store, delivery, sending, n) = (
                Arc::clone(&store),
                Arc::clone(&self.delivery),
                Arc::clone(&sending),
                Arc::clone(&n),
            );
            tasks.push(tokio::spawn(async move {
                let waits = std::iter::once(Duration::ZERO).chain(delivery.retries.iter().copied());
                for (attempt, wait) in waits.enumerate() {
                    tokio::time::sleep(wait).await;
                    match delivery.try_once(store.as_ref(), &channel, &n, &sending).await {
                        Ok(()) => return,
                        Err(e) => tracing::warn!(
                            channel = %channel.name,
                            attempt = attempt + 1,
                            "Alert not delivered: {e:#}"
                        ),
                    }
                }
                tracing::error!(channel = %channel.name, "Alert not delivered after every try: {}", n.subject());
            }));
        }
        Ok(tasks)
    }
}

#[async_trait]
impl Notifier for Dispatcher {
    async fn notify(&self, notification: Notification) -> anyhow::Result<()> {
        self.dispatch(notification).await.map(|_| ())
    }
}
