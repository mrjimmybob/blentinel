//! The probe's link to its hub: where it is (`hub_url`) and the key reports
//! are sealed for (`hub_public_key`, or fetched by handshake). Both follow the
//! config file while the probe runs (M0-23, D-38).

use anyhow::{Context, Result};
use tokio::sync::watch;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

use crate::config::AgentConfig;
use crate::transport::HubTransport;

/// How long to wait between handshake attempts at startup.
const HANDSHAKE_RETRY: Duration = Duration::from_secs(120);

pub struct HubLink {
    /// The settings this link was built from, to notice when they change.
    hub_url: String,
    pinned_key: Option<String>,
    pub transport: HubTransport,
    pub hub_key: [u8; 32],
}

impl HubLink {
    /// The first link, at startup. A bad `hub_url` or `hub_public_key` is an
    /// error; an unreachable hub is retried every 2 minutes. `None` if the
    /// probe is told to shut down while waiting.
    pub async fn connect(
        agent: &AgentConfig,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<Option<Self>> {
        let transport = HubTransport::new(&agent.hub_url)?;
        let hub_key = match &agent.hub_public_key {
            Some(hex) => parse_key(hex)?,
            None => {
                info!("Hub public key not in config, initiating handshake with hub");
                loop {
                    match transport.fetch_hub_pk().await {
                        Ok(key) => break key,
                        Err(e) => {
                            warn!(
                                "Hub handshake failed: {e}. Hub may not be running, retrying in 120s"
                            );
                            tokio::select! {
                                _ = sleep(HANDSHAKE_RETRY) => {}
                                changed = shutdown.changed() => {
                                    if changed.is_err() || *shutdown.borrow() {
                                        return Ok(None);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        info!("Hub key ready");
        Ok(Some(Self::from_parts(agent, transport, hub_key)))
    }

    /// Follow a changed `hub_url` or `hub_public_key`. If the new settings
    /// can't be used (bad URL or key, or the new hub doesn't answer the
    /// handshake), keep the current link and log why; the next call retries.
    pub async fn follow(&mut self, agent: &AgentConfig) {
        if agent.hub_url == self.hub_url && agent.hub_public_key == self.pinned_key {
            return;
        }
        match Self::try_new(agent).await {
            Ok(link) => {
                info!(hub_url = %agent.hub_url, "Hub connection updated from the config");
                *self = link;
            }
            Err(e) => error!(
                "Can't use the new hub settings, keeping the current connection ({}): {e:#}",
                self.hub_url
            ),
        }
    }

    /// One attempt, no retries.
    async fn try_new(agent: &AgentConfig) -> Result<Self> {
        let transport = HubTransport::new(&agent.hub_url)?;
        let hub_key = match &agent.hub_public_key {
            Some(hex) => parse_key(hex)?,
            None => transport
                .fetch_hub_pk()
                .await
                .with_context(|| format!("handshake with {} failed", agent.hub_url))?,
        };
        Ok(Self::from_parts(agent, transport, hub_key))
    }

    fn from_parts(agent: &AgentConfig, transport: HubTransport, hub_key: [u8; 32]) -> Self {
        Self {
            hub_url: agent.hub_url.clone(),
            pinned_key: agent.hub_public_key.clone(),
            transport,
            hub_key,
        }
    }
}

/// `hub_public_key` from the config: 64 hex characters.
fn parse_key(hex_key: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_key.trim())
        .context("Invalid hub_public_key in config (expected 64 hex characters)")?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("hub_public_key must be 32 bytes (64 hex characters)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn agent(url: &str, key: Option<[u8; 32]>) -> AgentConfig {
        AgentConfig {
            company_id: "acme".into(),
            hub_url: url.into(),
            interval: 60,
            hub_public_key: key.map(hex::encode),
            site: "main".into(),
        }
    }

    async fn pinned(url: &str, key: [u8; 32]) -> HubLink {
        let (_tx, mut rx) = watch::channel(false);
        HubLink::connect(&agent(url, Some(key)), &mut rx)
            .await
            .unwrap()
            .unwrap()
    }

    /// A stand-in hub that answers every request with its key, like /api/handshake.
    async fn fake_hub(key: [u8; 32]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = socket.read(&mut request).await;
                let body = hex::encode(key);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        url
    }

    #[tokio::test]
    async fn a_new_pinned_key_is_used() {
        let mut link = pinned("http://127.0.0.1:9", [1; 32]).await;
        link.follow(&agent("http://127.0.0.1:9", Some([2; 32])))
            .await;
        assert_eq!(link.hub_key, [2; 32]);
    }

    #[tokio::test]
    async fn unusable_settings_keep_the_current_link() {
        let mut link = pinned("http://127.0.0.1:9", [1; 32]).await;

        let mut bad_key = agent("http://127.0.0.1:9", None);
        bad_key.hub_public_key = Some("not hex".into());
        link.follow(&bad_key).await;
        link.follow(&agent("hub.example.com:3000", Some([2; 32])))
            .await; // no scheme

        assert_eq!(link.hub_key, [1; 32]);
        assert_eq!(link.hub_url, "http://127.0.0.1:9");
    }

    #[tokio::test]
    async fn removing_the_pinned_key_handshakes_with_the_new_hub() {
        let mut link = pinned("http://127.0.0.1:9", [1; 32]).await;
        let url = fake_hub([7; 32]).await;

        link.follow(&agent(&url, None)).await;

        assert_eq!(link.hub_key, [7; 32]);
        assert_eq!(link.hub_url, url);
    }

    #[tokio::test]
    async fn an_unreachable_new_hub_keeps_the_current_link_and_retries_later() {
        let mut link = pinned("http://127.0.0.1:9", [1; 32]).await;
        let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let unreachable = format!("http://{}", silent.local_addr().unwrap());
        drop(silent); // nothing listens there now

        link.follow(&agent(&unreachable, None)).await;
        assert_eq!(link.hub_key, [1; 32], "kept");

        // Settings unchanged since then, but the link still differs: retried.
        let url = fake_hub([7; 32]).await;
        link.follow(&agent(&url, None)).await;
        assert_eq!(link.hub_key, [7; 32]);
    }
}
