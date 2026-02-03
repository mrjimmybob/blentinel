use anyhow::{Context, Result};
use reqwest::Certificate;

/// Embedded hub TLS certificate for certificate pinning.
/// This file should contain the hub's `hub_tls_cert.pem`.
/// If missing, HTTPS will not work.
pub static HUB_CERTIFICATE_PEM: &[u8] = include_bytes!("../hub_cert.pem");

/// Get the pinned hub certificate for HTTPS connections.
/// Returns an error if the certificate is missing or invalid.
pub fn get_pinned_cert() -> Result<Certificate> {
    // Sanity check: ensure the certificate was actually embedded
    if HUB_CERTIFICATE_PEM.is_empty() || HUB_CERTIFICATE_PEM.len() < 100 {
        anyhow::bail!(
            "No hub certificate embedded. Copy hub_tls_cert.pem to probe/hub_cert.pem and rebuild."
        );
    }

    Certificate::from_pem(HUB_CERTIFICATE_PEM)
        .context("Failed to parse embedded hub certificate")
}
