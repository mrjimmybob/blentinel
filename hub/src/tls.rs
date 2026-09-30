#![cfg(feature = "ssr")]
use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertificateParams, DistinguishedName, Ia5String, KeyPair, SanType};
use rustls_pemfile::certs;
use std::fs;
use std::io::BufReader;
use time::{Duration, OffsetDateTime};

/// Load existing TLS certificate and key, or generate new ones if missing.
/// Returns (cert_pem, key_pem) as byte vectors.
pub fn load_or_create_tls_cert(
    cert_path: &str,
    key_path: &str,
    host: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let cert_exists = std::path::Path::new(cert_path).exists();
    let key_exists = std::path::Path::new(key_path).exists();

    if cert_exists && key_exists {
        tracing::info!("TLS: Loading existing certificate from {}", cert_path);
        let cert_pem = fs::read(cert_path)
            .context(format!("Failed to read certificate from {}", cert_path))?;
        let key_pem =
            fs::read(key_path).context(format!("Failed to read private key from {}", key_path))?;

        // Display certificate fingerprint for verification
        let fingerprint = get_cert_fingerprint(&cert_pem)?;
        tracing::info!("TLS: Certificate fingerprint (SHA-256): {}", fingerprint);

        Ok((cert_pem, key_pem))
    } else {
        tracing::info!("TLS: Generating new self-signed certificate...");
        generate_self_signed_cert(cert_path, key_path, host)
    }
}

/// Generate a new self-signed ECDSA P-256 certificate with 10-year validity.
fn generate_self_signed_cert(
    cert_path: &str,
    key_path: &str,
    host: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    // Generate ECDSA P-256 key pair
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("Failed to generate ECDSA key pair")?;

    let mut params = CertificateParams::default();

    // Set subject distinguished name
    let mut dn = DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "Blentinel Hub");
    dn.push(rcgen::DnType::OrganizationName, "Blentinel");
    params.distinguished_name = dn;

    // Subject Alternative Names (SAN)
    // `host` becomes a DNS name only if it's a valid one (e.g. not an IP or a
    // name with non-ASCII characters); "localhost" is always included.
    let dns_name = |name: &str| {
        Ia5String::try_from(name.to_string())
            .map(SanType::DnsName)
            .with_context(|| format!("Invalid DNS name for the certificate: {name}"))
    };
    params.subject_alt_names = vec![
        dns_name("localhost")?,
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
    ];
    if host != "localhost"
        && let Ok(name) = dns_name(host)
    {
        params.subject_alt_names.push(name);
    }

    // 10-year validity
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + Duration::days(3650);

    // Basic constraints for end-entity certificate
    params.is_ca = rcgen::IsCa::NoCa;
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::DigitalSignature,
        rcgen::KeyUsagePurpose::KeyEncipherment,
    ];

    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

    // Generate certificate using from_params with the key pair
    let cert = params
        .self_signed(&key_pair)
        .context("Failed to generate self-signed certificate")?;

    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();

    // Save to disk
    fs::write(cert_path, &cert_pem)
        .context(format!("Failed to write certificate to {}", cert_path))?;
    common::fs::write_secret_file(std::path::Path::new(key_path), &key_pem)
        .context(format!("Failed to write private key to {}", key_path))?;

    tracing::info!("TLS: Certificate saved to {}", cert_path);
    tracing::info!("TLS: Private key saved to {}", key_path);

    // Display fingerprint
    let fingerprint = get_cert_fingerprint(&cert_pem)?;
    tracing::info!("TLS: Certificate fingerprint (SHA-256): {}", fingerprint);
    tracing::info!(
        "TLS: IMPORTANT: Copy {} to probe/hub_cert.pem for certificate pinning",
        cert_path
    );

    Ok((cert_pem, key_pem))
}

/// Build a RustlsConfig for axum-server from PEM-encoded certificate and key.
pub async fn build_rustls_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<RustlsConfig> {
    // This build contains two rustls crypto backends (ring and aws-lc-rs, from
    // different dependencies), so rustls can't pick one and would panic.
    // Choose its default, aws-lc-rs; later calls are no-ops (already set).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // RustlsConfig::from_pem is async
    let config = RustlsConfig::from_pem(cert_pem.to_vec(), key_pem.to_vec())
        .await
        .context("Failed to build TLS config from PEM")?;

    Ok(config)
}

/// SHA-256 of a PEM certificate's DER bytes: what enrolled probes pin (D-47).
pub fn cert_sha256(cert_pem: &[u8]) -> Result<[u8; 32]> {
    let mut reader = BufReader::new(cert_pem);
    let certs: Vec<rustls::pki_types::CertificateDer> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificate for fingerprint")?;
    let cert = certs.first().context("No certificate found in PEM")?;
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(cert.as_ref()).into())
}

/// When a PEM certificate expires.
pub fn cert_not_after(cert_pem: &[u8]) -> Result<time::OffsetDateTime> {
    let pem = std::str::from_utf8(cert_pem).context("the certificate isn't text (PEM)")?;
    let params =
        rcgen::CertificateParams::from_ca_cert_pem(pem).context("the certificate can't be read")?;
    Ok(params.not_after)
}

/// The same hash as colon-separated hex, as browsers show it (for the log).
pub fn get_cert_fingerprint(cert_pem: &[u8]) -> Result<String> {
    Ok(cert_sha256(cert_pem)?
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: rustls has two crypto backends in this build (ring and
    /// aws-lc-rs, from different dependencies) and panics unless one is chosen.
    #[tokio::test]
    async fn a_generated_certificate_builds_a_tls_config() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let (cert_pem, key_pem) = load_or_create_tls_cert(
            cert.to_str().unwrap(),
            key.to_str().unwrap(),
            "hub.example.com",
        )
        .unwrap();
        assert!(build_rustls_config(&cert_pem, &key_pem).await.is_ok());

        let expires = cert_not_after(&cert_pem).unwrap();
        assert!(expires > time::OffsetDateTime::now_utc() + time::Duration::days(300));
    }
}
