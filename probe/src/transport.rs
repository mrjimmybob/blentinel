use reqwest::{Client, StatusCode};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Hub returned error status: {0}")]
    ApiError(StatusCode),

    #[error("Network timeout after multiple attempts")]
    Timeout,

    // ADD THESE TWO:
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
    pub fn new(base_url: String) -> Self {
        // Strip any trailing slash so we don't end up with double slashes
        let base_url = base_url.trim_end_matches('/').to_string();
        let is_https = base_url.starts_with("https://");

        let client = if is_https {
            // HTTPS mode: use certificate pinning
            let cert = crate::tls::get_pinned_cert()
                .expect("HTTPS URL requires embedded hub certificate");

            Client::builder()
                .add_root_certificate(cert)
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTPS client")
        } else {
            // HTTP mode: standard client
            Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTP client")
        };

        Self { client, base_url }
    }

    /// Sends the encrypted binary payload to the Hub
    pub async fn ship_report(&self, encrypted_payload: Vec<u8>) -> Result<(), TransportError> {
        let response = self.client
            .post(format!("{}/api/report", self.base_url))
            .header("Content-Type", "application/octet-stream")
            .header("X-Blentinel-Version", "1.0")
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

        if response.status().is_success() {
            Ok(())
        } else {
            Err(TransportError::ApiError(response.status()))
        }
    }


    pub async fn fetch_hub_pk(&self) -> Result<[u8; 32], TransportError> {
        let resp = self.client
            .get(format!("{}/api/handshake", self.base_url))
            .send()
            .await?
            .text()
            .await?;

        let bytes = hex::decode(resp.trim())?;

        bytes.try_into()
            .map_err(|_| TransportError::HandshakeFailed)
    }

}