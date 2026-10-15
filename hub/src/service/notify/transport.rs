//! Delivering an [`Outgoing`]: HTTPS POSTs and e-mail. A trait, so tests
//! record instead of sending.
//!
//! Errors never contain a URL: a chat webhook's or Telegram's URL is itself
//! a secret (D-66), and errors are shown on the alerts page and logged.

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;

use super::payload::{MailServer, Outgoing};
use crate::model::MailSecurity;

/// How long one delivery may take.
const TIMEOUT: Duration = Duration::from_secs(20);

#[async_trait]
pub trait Transport: Send + Sync {
    async fn deliver(&self, outgoing: &Outgoing) -> anyhow::Result<()>;
}

/// The real thing: reqwest (rustls) for HTTPS, lettre for e-mail.
pub struct NetTransport {
    http: reqwest::Client,
}

impl NetTransport {
    pub fn new() -> anyhow::Result<Self> {
        crate::tls::install_crypto_provider();
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .user_agent(concat!("Blentinel-Hub/", env!("CARGO_PKG_VERSION")))
                .build()
                .context("can't make the HTTP client")?,
        })
    }

    async fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &str,
    ) -> anyhow::Result<()> {
        let mut request = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("can't reach the receiver: {}", e.without_url()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let answer: String = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        anyhow::bail!("the receiver answered {status}: {}", answer.trim())
    }
}

/// Send one e-mail (lettre's SMTP client is blocking: off the async threads).
async fn send_mail(
    server: &MailServer,
    to: &[String],
    subject: &str,
    body: &str,
) -> anyhow::Result<()> {
    let (server, to, subject, body) = (
        server.clone(),
        to.to_vec(),
        subject.to_string(),
        body.to_string(),
    );
    tokio::task::spawn_blocking(move || {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{Message, SmtpTransport, Transport as _};

        let mut message =
            Message::builder().from(server.from.parse().map_err(|e| {
                anyhow::anyhow!("the sender {} isn't an address: {e}", server.from)
            })?);
        for recipient in &to {
            message = message.to(recipient
                .parse()
                .map_err(|e| anyhow::anyhow!("{recipient} isn't an address: {e}"))?);
        }
        let message = message
            .subject(subject)
            .body(body)
            .context("can't make the e-mail")?;
        let builder = match server.security {
            MailSecurity::Starttls => SmtpTransport::starttls_relay(&server.server),
            MailSecurity::Tls => SmtpTransport::relay(&server.server),
            MailSecurity::None => Ok(SmtpTransport::builder_dangerous(&server.server)),
        }
        .with_context(|| format!("can't use the mail server {}", server.server))?
        .port(server.port)
        .timeout(Some(TIMEOUT));
        let builder = match server.username.is_empty() {
            true => builder,
            false => builder.credentials(Credentials::new(server.username, server.password)),
        };
        builder
            .build()
            .send(&message)
            .map_err(|e| anyhow::anyhow!("the mail server refused it: {e}"))?;
        Ok(())
    })
    .await
    .context("the e-mail task stopped")?
}

#[async_trait]
impl Transport for NetTransport {
    async fn deliver(&self, outgoing: &Outgoing) -> anyhow::Result<()> {
        match outgoing {
            Outgoing::Http { url, headers, body } => self.post(url, headers, body).await,
            Outgoing::Email {
                server,
                to,
                subject,
                body,
            } => send_mail(server, to, subject, body).await,
        }
    }
}
