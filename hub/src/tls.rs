#![cfg(feature = "ssr")]
use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, KeyPair, SanType, Ia5String};
use axum_server::tls_rustls::RustlsConfig;
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
        println!("[TLS] Loading existing certificate from {}", cert_path);
        let cert_pem = fs::read(cert_path)
            .context(format!("Failed to read certificate from {}", cert_path))?;
        let key_pem = fs::read(key_path)
            .context(format!("Failed to read private key from {}", key_path))?;

        // Display certificate fingerprint for verification
        let fingerprint = get_cert_fingerprint(&cert_pem)?;
        println!("[TLS] Certificate fingerprint (SHA-256): {}", fingerprint);

        Ok((cert_pem, key_pem))
    } else {
        println!("[TLS] Generating new self-signed certificate...");
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
    params.subject_alt_names = vec![
        SanType::DnsName(Ia5String::try_from(host.to_string()).unwrap_or_else(|_| Ia5String::try_from("localhost".to_string()).unwrap())),
        SanType::DnsName(Ia5String::try_from("localhost".to_string()).unwrap()),
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
        SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))),
    ];

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
    let cert = params.self_signed(&key_pair)
        .context("Failed to generate self-signed certificate")?;

    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();

    // Save to disk
    fs::write(cert_path, &cert_pem)
        .context(format!("Failed to write certificate to {}", cert_path))?;
    fs::write(key_path, &key_pem)
        .context(format!("Failed to write private key to {}", key_path))?;

    println!("[TLS] Certificate saved to {}", cert_path);
    println!("[TLS] Private key saved to {}", key_path);

    // Display fingerprint
    let fingerprint = get_cert_fingerprint(&cert_pem)?;
    println!("[TLS] Certificate fingerprint (SHA-256): {}", fingerprint);
    println!("[TLS] IMPORTANT: Copy {} to probe/hub_cert.pem for certificate pinning", cert_path);

    Ok((cert_pem, key_pem))
}

/// Build a RustlsConfig for axum-server from PEM-encoded certificate and key.
pub async fn build_rustls_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<RustlsConfig> {
    // RustlsConfig::from_pem is async
    let config = RustlsConfig::from_pem(cert_pem.to_vec(), key_pem.to_vec())
        .await
        .context("Failed to build TLS config from PEM")?;

    Ok(config)
}

/// Calculate SHA-256 fingerprint of a PEM certificate for verification.
pub fn get_cert_fingerprint(cert_pem: &[u8]) -> Result<String> {
    let mut reader = BufReader::new(cert_pem);
    let certs: Vec<rustls::pki_types::CertificateDer> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificate for fingerprint")?;

    if certs.is_empty() {
        anyhow::bail!("No certificate found in PEM");
    }

    // Calculate SHA-256 hash of the DER-encoded certificate
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(certs[0].as_ref());
    let hash = hasher.finalize();

    // Format as colon-separated hex
    Ok(hash
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":"))
}
