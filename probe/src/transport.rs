use reqwest::{Client, StatusCode};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The hub answered with an error status; the text is its reason, if it sent one.
    #[error("Hub answered {0}{1}")]
    ApiError(StatusCode, String),

    #[error("Network timeout after multiple attempts")]
    Timeout,

    #[error("Handshake failed: could not retrieve or verify Hub key")]
    HandshakeFailed,

    #[error("Failed to decode hex data: {0}")]
    Hex(#[from] hex::FromHexError),
}

pub struct HubTransport {
    client: Client,
    base_url: String,
}

impl HubTransport {
    /// A client for the hub at `base_url` (`http://…` or `https://…`; HTTPS
    /// pins the certificate embedded at build time).
    pub fn new(base_url: &str) -> anyhow::Result<Self> {
        use anyhow::Context;
        // Strip any trailing slash so we don't end up with double slashes
        let base_url = base_url.trim_end_matches('/').to_string();
        let builder = Client::builder().timeout(Duration::from_secs(10));
        let builder = if base_url.starts_with("https://") {
            builder.add_root_certificate(crate::tls::get_pinned_cert()?)
        } else if base_url.starts_with("http://") {
            builder
        } else {
            anyhow::bail!("hub_url must start with http:// or https:// (got \"{base_url}\")");
        };
        let client = builder.build().context("Failed to build the HTTP client")?;
        Ok(Self { client, base_url })
    }

    /// Sends the encrypted binary payload to the Hub
    pub async fn ship_report(&self, encrypted_payload: Vec<u8>) -> Result<(), TransportError> {
        let response = self
            .client
            .post(format!("{}/api/report", self.base_url))
            .header("Content-Type", "application/octet-stream")
            .header(
                common::protocol::PROTOCOL_HEADER,
                common::protocol::PROTOCOL_VERSION.to_string(),
            )
            .body(encrypted_payload)
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
        if status.is_success() {
            Ok(())
        } else {
            Err(TransportError::ApiError(status, hub_reason(response).await))
        }
    }

    pub async fn fetch_hub_pk(&self) -> Result<[u8; 32], TransportError> {
        let resp = self
            .client
            .get(format!("{}/api/handshake", self.base_url))
            .send()
            .await?
            .text()
            .await?;

        let bytes = hex::decode(resp.trim())?;

        bytes
            .try_into()
            .map_err(|_| TransportError::HandshakeFailed)
    }
}
/// The reason in a hub error response (`common::protocol::ErrorBody`), as
/// ": <message> (<code>)", or "" if the body isn't one (e.g. a proxy's error page).
async fn hub_reason(response: reqwest::Response) -> String {
    let body = response.bytes().await.unwrap_or_default();
    serde_json::from_slice::<common::protocol::ErrorBody>(&body)
        .map(|error| format!(": {error}"))
        .unwrap_or_default()
}

/// Map a delivery attempt to what the outbox should do with the report.
///
/// Mirrors the hub's documented responses (`hub/src/ingest.rs`): 409 means the
/// hub already has the report, so it counts as delivered. Other 4xx are
/// permanent (except 408 Request Timeout and 429 Too Many Requests). 5xx and
/// network errors are transient.
pub fn classify(result: Result<(), TransportError>) -> crate::outbox::Delivery {
    use crate::outbox::Delivery;
    match result {
        Ok(()) => Delivery::Delivered,
        Err(TransportError::ApiError(StatusCode::CONFLICT, _)) => Delivery::Delivered,
        Err(error @ TransportError::ApiError(status, _))
            if status.is_client_error()
                && status != StatusCode::REQUEST_TIMEOUT
                && status != StatusCode::TOO_MANY_REQUESTS =>
        {
            Delivery::Rejected(error.to_string())
        }
        Err(e) => Delivery::Retry(e.to_string()),
    }
}

#[async_trait::async_trait]
impl crate::outbox::ReportSender for HubTransport {
    async fn deliver(&self, sealed_report: &[u8]) -> crate::outbox::Delivery {
        classify(self.ship_report(sealed_report.to_vec()).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::Delivery;

    fn status(code: u16) -> Delivery {
        classify(Err(TransportError::ApiError(
            StatusCode::from_u16(code).unwrap(),
            String::new(),
        )))
    }

    #[test]
    fn success_and_conflict_count_as_delivered() {
        assert_eq!(classify(Ok(())), Delivery::Delivered);
        assert_eq!(status(409), Delivery::Delivered);
    }

    #[test]
    fn permanent_client_errors_are_rejected() {
        for code in [400, 401, 403, 422] {
            assert!(matches!(status(code), Delivery::Rejected(_)), "{code}");
        }
    }

    #[test]
    fn a_rejection_carries_the_hubs_reason() {
        let error = TransportError::ApiError(
            StatusCode::FORBIDDEN,
            ": report from unregistered probe 42d51d7a (unknown_probe)".to_string(),
        );
        assert_eq!(
            classify(Err(error)),
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
            classify(Err(TransportError::Timeout)),
            Delivery::Retry(_)
        ));
    }
}
