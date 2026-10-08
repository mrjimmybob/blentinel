use chrono::{DateTime, Utc};
use common::ack::{AckError, ReportAck};
use ed25519_dalek::SigningKey;
use reqwest::{Client, StatusCode};
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

use crate::outbox::Delivery;

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The hub answered with an error status; the text is its reason, if it sent one.
    #[error("Hub answered {0}{1}")]
    ApiError(StatusCode, String),

    #[error("Network timeout after multiple attempts")]
    Timeout,
}

/// The hub's answer to a report, as received.
#[derive(Debug, Clone)]
pub struct HubAnswer {
    pub status: StatusCode,
    pub body: Vec<u8>,
    /// `application/octet-stream`: a sealed [`ReportAck`]. Otherwise a JSON
    /// error (`common::protocol::ErrorBody`), or a proxy's page.
    pub is_ack: bool,
}

#[derive(Clone)]
pub struct HubTransport {
    client: Client,
    base_url: String,
}

impl HubTransport {
    /// A client for the hub at `base_url`. With `tls_pins` (from the
    /// enrollment, D-61), HTTPS trusts only certificates with those public
    /// keys; without any, HTTPS uses the system's trusted authorities (a hub
    /// behind a proxy with a public certificate). `http://` is plain HTTP.
    pub fn new(base_url: &str, tls_pins: Vec<[u8; 32]>) -> anyhow::Result<Self> {
        use anyhow::Context;
        // Strip any trailing slash so we don't end up with double slashes
        let base_url = base_url.trim_end_matches('/').to_string();
        let builder = Client::builder().timeout(Duration::from_secs(10));
        let builder = if base_url.starts_with("https://") {
            if tls_pins.is_empty() {
                builder
            } else {
                builder.tls_backend_preconfigured(crate::tls::pinned_client_config(tls_pins)?)
            }
        } else if base_url.starts_with("http://") {
            builder
        } else {
            anyhow::bail!("the hub URL must start with http:// or https:// (got \"{base_url}\")");
        };
        let client = builder.build().context("Failed to build the HTTP client")?;
        Ok(Self { client, base_url })
    }

    /// Reach the hub (DNS, TCP, TLS with the pinned certificate) without
    /// sending anything, and read its clock from the answer's `Date`
    /// (`probe doctor`). A `GET` on the report endpoint: the hub answers
    /// "method not allowed", on the plain port too, and logs nothing.
    pub async fn hub_date(&self) -> Result<Option<DateTime<Utc>>, TransportError> {
        let response = self
            .client
            .get(format!("{}/api/report", self.base_url))
            .send()
            .await?;
        Ok(response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| DateTime::parse_from_rfc2822(v).ok())
            .map(|t| t.with_timezone(&Utc)))
    }

    /// Send a sealed report. Only failing to reach the hub is an error; any
    /// answer, error statuses included, is returned for [`classify`].
    pub async fn ship_report(&self, sealed_report: &[u8]) -> Result<HubAnswer, TransportError> {
        let response = self
            .client
            .post(format!("{}/api/report", self.base_url))
            .header("Content-Type", "application/octet-stream")
            .header(
                common::protocol::PROTOCOL_HEADER,
                common::protocol::PROTOCOL_VERSION.to_string(),
            )
            .body(sealed_report.to_vec())
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    TransportError::Timeout
                } else {
                    TransportError::Http(e)
                }
            })?;

        let status = response.status();
        let is_ack = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes().starts_with(b"application/octet-stream"));
        let body = response.bytes().await?.to_vec();
        Ok(HubAnswer {
            status,
            body,
            is_ack,
        })
    }

    /// Download `path` from the hub into `to`, refusing more than `max_bytes`
    /// (the size the signed manifest states) or taking longer than `timeout`.
    /// The file's SHA-256 (hex), worked out while it is written.
    pub async fn download(
        &self,
        path: &str,
        to: &std::path::Path,
        max_bytes: u64,
        timeout: Duration,
    ) -> anyhow::Result<String> {
        use anyhow::{Context, bail};
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncWriteExt;

        let mut response = self
            .client
            .get(format!("{}{path}", self.base_url))
            .timeout(timeout) // the client's 10 s is for reports
            .send()
            .await
            .context("can't reach the hub")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            bail!("the hub answered {status}{}", hub_reason(&body));
        }
        let mut file = tokio::fs::File::create(to)
            .await
            .with_context(|| format!("can't write {}", to.display()))?;
        let (mut hasher, mut written) = (Sha256::new(), 0u64);
        while let Some(chunk) = response.chunk().await.context("the download broke off")? {
            written += chunk.len() as u64;
            if written > max_bytes {
                bail!("the download is larger than the {max_bytes} bytes the release states");
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.sync_all().await?;
        Ok(hex::encode(hasher.finalize()))
    }

    /// Redeem an enrollment code: `POST /api/enroll` with the sealed request.
    pub async fn enroll(
        &self,
        sealed_request: Vec<u8>,
    ) -> Result<common::enrollment::EnrollResponse, TransportError> {
        let response = self
            .client
            .post(format!("{}/api/enroll", self.base_url))
            .header("Content-Type", "application/octet-stream")
            .header(
                common::protocol::PROTOCOL_HEADER,
                common::protocol::PROTOCOL_VERSION.to_string(),
            )
            .body(sealed_request)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            return Err(TransportError::ApiError(status, hub_reason(&body)));
        }
        Ok(response.json().await?)
    }
}

/// The reason in a hub error response (`common::protocol::ErrorBody`), as
/// ": <message> (<code>)", or "" if the body isn't one (e.g. a proxy's error page).
fn hub_reason(body: &[u8]) -> String {
    serde_json::from_slice::<common::protocol::ErrorBody>(body)
        .map(|error| format!(": {error}"))
        .unwrap_or_default()
}

/// Map the hub's answer to what the outbox should do with the report.
/// `open_ack` opens and checks a sealed ack (`common::ack::open_ack`).
///
/// Mirrors the hub's documented responses (`hub/src/ingest.rs`):
///
/// - 200: delivered, with the hub's signed ack. An ack that can't be trusted
///   is logged and ignored (the report was stored either way).
/// - 409: the hub already has it, so it counts as delivered.
/// - 403 with a **signed** ack saying `revoked`: [`Delivery::Revoked`]. A
///   plain 403 could come from anyone on the path, so it only drops the report.
/// - Other 4xx are permanent (except 408 and 429); 5xx and network errors are
///   transient.
pub fn classify(
    answer: Result<HubAnswer, TransportError>,
    open_ack: impl FnOnce(&[u8]) -> Result<ReportAck, AckError>,
) -> Delivery {
    let answer = match answer {
        Ok(answer) => answer,
        Err(e) => return Delivery::Retry(e.to_string()),
    };
    let ack = answer.is_ack.then(|| open_ack(&answer.body));
    let status = answer.status;

    if status.is_success() {
        return Delivery::Delivered(match ack {
            Some(Ok(ack)) => Some(Box::new(ack)),
            Some(Err(e)) => {
                warn!("The hub stored the report, but its answer can't be trusted: {e}");
                None
            }
            None => None, // an older hub, or a proxy that dropped the body
        });
    }
    if status == StatusCode::CONFLICT {
        return Delivery::Delivered(None);
    }
    let reason = match ack {
        Some(Ok(ack)) if ack.revoked => return Delivery::Revoked,
        Some(Err(e)) => format!(": its answer can't be trusted: {e}"),
        _ => hub_reason(&answer.body),
    };
    let reason = TransportError::ApiError(status, reason).to_string();
    if status.is_client_error()
        && status != StatusCode::REQUEST_TIMEOUT
        && status != StatusCode::TOO_MANY_REQUESTS
    {
        Delivery::Rejected(reason)
    } else {
        Delivery::Retry(reason)
    }
}

/// How far the probe's clock is from the hub's, in seconds, if more than
/// `CLOCK_WARNING_SECS`: the hub refuses reports beyond `MAX_CLOCK_SKEW_SECS`.
pub fn clock_skew(hub_time: DateTime<Utc>, now: DateTime<Utc>) -> Option<i64> {
    let skew = (now - hub_time).num_seconds();
    (skew.abs() > common::protocol::CLOCK_WARNING_SECS).then_some(skew)
}

/// Delivers reports to the hub and checks its answers with the keys from
/// the enrollment: its current signing key, and the next one while the hub
/// rotates (D-61).
pub struct HubSender<'a> {
    pub transport: &'a HubTransport,
    pub probe_key: &'a SigningKey,
    pub hub_signing_keys: &'a [[u8; 32]],
}

#[async_trait::async_trait]
impl crate::outbox::ReportSender for HubSender<'_> {
    async fn deliver(&self, sealed_report: &[u8]) -> Delivery {
        classify(self.transport.ship_report(sealed_report).await, |body| {
            common::ack::open_ack_signed_by_any(
                body,
                self.probe_key,
                self.hub_signing_keys,
                sealed_report,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(code: u16, body: &[u8], is_ack: bool) -> Result<HubAnswer, TransportError> {
        Ok(HubAnswer {
            status: StatusCode::from_u16(code).unwrap(),
            body: body.to_vec(),
            is_ack,
        })
    }

    fn ack(revoked: bool) -> ReportAck {
        ReportAck {
            report_sha256: String::new(),
            hub_time: Utc::now(),
            revoked,
            config: None,
            update: None,
            commands: vec![],
            rotation: None,
        }
    }

    /// An `open_ack` that trusts everything; the crypto is tested in `common::ack`.
    fn trusted(revoked: bool) -> impl FnOnce(&[u8]) -> Result<ReportAck, AckError> {
        move |_| Ok(ack(revoked))
    }

    fn forged(_: &[u8]) -> Result<ReportAck, AckError> {
        Err(AckError::BadSignature)
    }

    fn status(code: u16) -> Delivery {
        classify(answer(code, b"", false), forged)
    }

    #[test]
    fn success_and_conflict_count_as_delivered() {
        let delivered = classify(answer(200, b"sealed", true), trusted(false));
        assert!(matches!(delivered, Delivery::Delivered(Some(_))));
        assert_eq!(status(200), Delivery::Delivered(None));
        assert_eq!(status(409), Delivery::Delivered(None));
    }

    #[test]
    fn an_untrusted_ack_is_ignored_but_the_report_counts_as_delivered() {
        assert_eq!(
            classify(answer(200, b"sealed", true), forged),
            Delivery::Delivered(None)
        );
    }

    #[test]
    fn only_a_signed_revocation_stops_the_probe() {
        assert_eq!(
            classify(answer(403, b"sealed", true), trusted(true)),
            Delivery::Revoked
        );
        let forged_revocation = classify(answer(403, b"sealed", true), forged);
        assert!(
            matches!(&forged_revocation, Delivery::Rejected(r) if r.contains("can't be trusted")),
            "{forged_revocation:?}"
        );
        let plain =
            br#"{"error":{"code":"probe_revoked","message":"report from revoked probe 42d51d7a"}}"#;
        assert!(matches!(
            classify(answer(403, plain, false), trusted(true)),
            Delivery::Rejected(_)
        ));
    }

    #[test]
    fn permanent_client_errors_are_rejected() {
        for code in [400, 401, 403, 422] {
            assert!(matches!(status(code), Delivery::Rejected(_)), "{code}");
        }
    }

    #[test]
    fn a_rejection_carries_the_hubs_reason() {
        let body = br#"{"error":{"code":"unknown_probe","message":"report from unregistered probe 42d51d7a"}}"#;
        assert_eq!(
            classify(answer(403, body, false), forged),
            Delivery::Rejected(
                "Hub answered 403 Forbidden: report from unregistered probe 42d51d7a (unknown_probe)"
                    .to_string()
            )
        );
    }

    #[test]
    fn transient_failures_are_retried() {
        for code in [408, 429, 500, 502, 503] {
            assert!(matches!(status(code), Delivery::Retry(_)), "{code}");
        }
        assert!(matches!(
            classify(Err(TransportError::Timeout), forged),
            Delivery::Retry(_)
        ));
    }

    /// A stand-in hub answering every request with `status` and a sealed ack.
    async fn fake_hub(status: u16, body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0u8; 16 * 1024];
                let _ = socket.read(&mut request).await;
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/octet-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        url
    }

    #[tokio::test]
    async fn the_hubs_signed_answer_is_opened_and_checked_end_to_end() {
        use crate::outbox::ReportSender;
        use rand::rngs::OsRng;

        let hub_secret = x25519_dalek::StaticSecret::random_from_rng(OsRng);
        let hub_signing_key = common::crypto::hub_signing_key(&hub_secret)
            .verifying_key()
            .to_bytes();
        let probe = SigningKey::generate(&mut OsRng);
        let probe_id = hex::encode(probe.verifying_key().as_bytes());
        let report = b"sealed report".to_vec();
        let answer = |revoked| ReportAck {
            report_sha256: common::ack::report_sha256(&report),
            hub_time: "2026-09-29T10:00:00Z".parse().unwrap(),
            revoked,
            config: None,
            update: None,
            commands: vec![],
            rotation: None,
        };

        for (status, revoked, expected) in [
            (
                200,
                false,
                Delivery::Delivered(Some(Box::new(answer(false)))),
            ),
            (403, true, Delivery::Revoked),
        ] {
            let body = common::ack::seal_ack(&answer(revoked), &hub_secret, &probe_id).unwrap();
            let transport = HubTransport::new(&fake_hub(status, body).await, Vec::new()).unwrap();
            let sender = HubSender {
                transport: &transport,
                probe_key: &probe,
                hub_signing_keys: &[hub_signing_key],
            };
            assert_eq!(sender.deliver(&report).await, expected);

            // The same answer given to another report is not trusted.
            let replayed = sender.deliver(b"another report").await;
            assert_ne!(replayed, expected, "{status}");
        }
    }

    #[test]
    fn clock_skew_is_reported_beyond_a_minute_either_way() {
        let hub: DateTime<Utc> = "2026-09-29T10:00:00Z".parse().unwrap();
        assert_eq!(clock_skew(hub, hub + chrono::Duration::seconds(59)), None);
        assert_eq!(
            clock_skew(hub, hub + chrono::Duration::seconds(90)),
            Some(90)
        );
        assert_eq!(
            clock_skew(hub, hub - chrono::Duration::seconds(120)),
            Some(-120)
        );
    }
}
