#![cfg(feature = "ssr")]
use std::collections::HashMap;
use std::fs;
use std::io::BufReader;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use chrono::{DateTime, Utc};
use rcgen::{CertificateParams, DistinguishedName, Ia5String, KeyPair, SanType};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pemfile::certs;
use time::{Duration, OffsetDateTime};

/// The ALPN protocol of an ACME TLS-ALPN-01 validation (RFC 8737).
const ACME_TLS_ALPN: &[u8] = b"acme-tls/1";

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

/// Generate a new self-signed ECDSA P-256 certificate with 10-year validity
/// (also the next certificate of a rotation, D-61).
pub fn generate_self_signed_cert(
    cert_path: &str,
    key_path: &str,
    host: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("Failed to generate ECDSA key pair")?;
    let cert_pem = self_signed_pem(&key_pair, host)?.into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();

    fs::write(cert_path, &cert_pem)
        .context(format!("Failed to write certificate to {}", cert_path))?;
    common::fs::write_secret_file(std::path::Path::new(key_path), &key_pem)
        .context(format!("Failed to write private key to {}", key_path))?;

    tracing::info!("TLS: Certificate saved to {}", cert_path);
    tracing::info!("TLS: Private key saved to {}", key_path);
    let fingerprint = get_cert_fingerprint(&cert_pem)?;
    tracing::info!("TLS: Certificate fingerprint (SHA-256): {}", fingerprint);

    Ok((cert_pem, key_pem))
}

/// A new self-signed certificate for the key already in `key_pem`: the
/// public key, so what probes pin, stays the same (D-62: ACME turned off).
pub fn self_signed_for_key(key_pem: &[u8], host: &str) -> Result<Vec<u8>> {
    let key_pair = key_pair_of(key_pem)?;
    Ok(self_signed_pem(&key_pair, host)?.into_bytes())
}

/// A self-signed certificate for `key_pair`, valid 10 years, for
/// `localhost`, the loopback addresses and `host` if it is a DNS name.
fn self_signed_pem(key_pair: &KeyPair, host: &str) -> Result<String> {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "Blentinel Hub");
    dn.push(rcgen::DnType::OrganizationName, "Blentinel");
    params.distinguished_name = dn;

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

    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + Duration::days(3650);
    params.is_ca = rcgen::IsCa::NoCa;
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::DigitalSignature,
        rcgen::KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

    let cert = params
        .self_signed(key_pair)
        .context("Failed to generate self-signed certificate")?;
    Ok(cert.pem())
}

/// The key pair in a PEM private key file.
fn key_pair_of(key_pem: &[u8]) -> Result<KeyPair> {
    let pem = std::str::from_utf8(key_pem).context("the private key isn't text (PEM)")?;
    KeyPair::from_pem(pem)
        .context("the private key can't be read (an ECDSA, Ed25519 or RSA key in PEM)")
}

/// A certificate signing request (DER) for `domains`, signed with the key in
/// `key_pem`: the certificate an authority issues for it keeps that key, so
/// what probes pin doesn't change (D-62).
pub fn csr_der(key_pem: &[u8], domains: &[String]) -> Result<Vec<u8>> {
    let key_pair = key_pair_of(key_pem)?;
    let mut params =
        CertificateParams::new(domains.to_vec()).context("the names can't go in a certificate")?;
    let mut dn = DistinguishedName::new();
    if let Some(first) = domains.first() {
        dn.push(rcgen::DnType::CommonName, first.as_str());
    }
    params.distinguished_name = dn;
    let csr = params
        .serialize_request(&key_pair)
        .context("can't make the certificate request")?;
    Ok(csr.der().to_vec())
}

/// Choose rustls's crypto backend. This build contains two (ring and
/// aws-lc-rs, from different dependencies), so rustls can't pick one and
/// would panic; take its default, aws-lc-rs. Later calls are no-ops.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// A certificate chain and its key, as the listener serves them; refused if
/// they don't match.
fn certified_key(cert_pem: &[u8], key_pem: &[u8]) -> Result<CertifiedKey> {
    let (chain, key, provider) = parse_pair(cert_pem, key_pem)?;
    CertifiedKey::from_der(chain, key, provider)
        .context("the certificate and the private key don't match")
}

/// A TLS-ALPN-01 answer as the listener serves it. Not checked like
/// [`certified_key`]: that check parses the certificate, and refuses the
/// critical `acmeIdentifier` extension such an answer must carry. The hub
/// made both halves itself.
fn challenge_key(cert_pem: &[u8], key_pem: &[u8]) -> Result<CertifiedKey> {
    let (chain, key, provider) = parse_pair(cert_pem, key_pem)?;
    let signing_key = provider
        .key_provider
        .load_private_key(key)
        .context("the private key can't be used")?;
    Ok(CertifiedKey::new(chain, signing_key))
}

type Pair = (
    Vec<rustls::pki_types::CertificateDer<'static>>,
    rustls::pki_types::PrivateKeyDer<'static>,
    &'static Arc<rustls::crypto::CryptoProvider>,
);

fn parse_pair(cert_pem: &[u8], key_pem: &[u8]) -> Result<Pair> {
    install_crypto_provider();
    let chain = certs(&mut BufReader::new(cert_pem))
        .collect::<Result<Vec<_>, _>>()
        .context("the certificate can't be read")?;
    anyhow::ensure!(!chain.is_empty(), "no certificate found in PEM");
    let key = rustls_pemfile::private_key(&mut BufReader::new(key_pem))
        .context("the private key can't be read")?
        .context("no private key found in PEM")?;
    let provider =
        rustls::crypto::CryptoProvider::get_default().context("no TLS crypto provider")?;
    Ok((chain, key, provider))
}

/// The first certificate of a PEM file, as DER.
fn first_cert_der(cert_pem: &[u8]) -> Result<rustls::pki_types::CertificateDer<'static>> {
    let mut reader = BufReader::new(cert_pem);
    let certs: Vec<rustls::pki_types::CertificateDer> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse the certificate")?;
    certs
        .into_iter()
        .next()
        .context("No certificate found in PEM")
}

/// SHA-256 of a PEM certificate's DER bytes: what browsers show (the log).
pub fn cert_sha256(cert_pem: &[u8]) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(first_cert_der(cert_pem)?.as_ref()).into())
}

/// SHA-256 of a PEM certificate's public key: what enrolled probes pin (D-61).
pub fn cert_spki_sha256(cert_pem: &[u8]) -> Result<[u8; 32]> {
    Ok(common::pins::spki_sha256(
        first_cert_der(cert_pem)?.as_ref(),
    )?)
}

/// SHA-256 of the public key of a PEM private key: the pin a certificate for
/// it has.
pub fn key_spki_sha256(key_pem: &[u8]) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let spki = key_pair_of(key_pem)?.public_key_der();
    Ok(Sha256::digest(spki).into())
}

/// What the admin page and renewals need to know of a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateDetails {
    /// Its issuer's common name (or the whole name if it has none).
    pub issuer: String,
    /// Its DNS names and IP addresses.
    pub names: Vec<String>,
    /// Issued by itself (issuer and subject are the same name).
    pub self_signed: bool,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
}

impl CertificateDetails {
    /// Its validity period.
    pub fn lifetime(&self) -> chrono::Duration {
        self.not_after - self.not_before
    }
}

/// Read the first certificate of a PEM file.
pub fn certificate_details(cert_pem: &[u8]) -> Result<CertificateDetails> {
    use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};
    let der = first_cert_der(cert_pem)?;
    let (_, cert) =
        X509Certificate::from_der(der.as_ref()).context("the certificate can't be read")?;
    let issuer = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .map_or_else(|| cert.issuer().to_string(), str::to_string);
    let mut names = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            match name {
                GeneralName::DNSName(dns) => names.push(dns.to_string()),
                GeneralName::IPAddress(bytes) => {
                    let ip = match bytes.len() {
                        4 => <[u8; 4]>::try_from(*bytes).ok().map(std::net::IpAddr::from),
                        16 => <[u8; 16]>::try_from(*bytes)
                            .ok()
                            .map(std::net::IpAddr::from),
                        _ => None,
                    };
                    names.extend(ip.map(|ip| ip.to_string()));
                }
                _ => {}
            }
        }
    }
    let time = |t: x509_parser::time::ASN1Time| {
        DateTime::from_timestamp(t.timestamp(), 0)
            .context("the certificate's dates are out of range")
    };
    Ok(CertificateDetails {
        issuer,
        names,
        self_signed: cert.issuer().as_raw() == cert.subject().as_raw(),
        not_before: time(cert.validity().not_before)?,
        not_after: time(cert.validity().not_after)?,
    })
}

/// When a PEM certificate expires.
pub fn cert_not_after(cert_pem: &[u8]) -> Result<time::OffsetDateTime> {
    let not_after = certificate_details(cert_pem)?.not_after;
    Ok(OffsetDateTime::from_unix_timestamp(not_after.timestamp())?)
}

/// The same hash as colon-separated hex, as browsers show it (for the log).
pub fn get_cert_fingerprint(cert_pem: &[u8]) -> Result<String> {
    Ok(cert_sha256(cert_pem)?
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(":"))
}

/// The answers the hub gives an ACME authority while it validates a
/// certificate order (D-62): HTTP-01 key authorizations by token, and
/// TLS-ALPN-01 certificates by name.
#[derive(Default)]
pub struct AcmeChallenges {
    http: Mutex<HashMap<String, String>>,
    tls_alpn: Mutex<HashMap<String, Arc<CertifiedKey>>>,
}

impl std::fmt::Debug for AcmeChallenges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcmeChallenges").finish_non_exhaustive()
    }
}

impl AcmeChallenges {
    /// Answer `GET /.well-known/acme-challenge/{token}` with `key_authorization`.
    pub fn add_http(&self, token: &str, key_authorization: &str) {
        lock(&self.http).insert(token.to_string(), key_authorization.to_string());
    }

    /// What to answer for `token`, if the hub is waiting for it.
    pub fn http(&self, token: &str) -> Option<String> {
        lock(&self.http).get(token).cloned()
    }

    /// Answer an `acme-tls/1` handshake for `domain` with a certificate
    /// carrying the digest of the key authorization (RFC 8737).
    pub fn add_tls_alpn(&self, domain: &str, key_authorization_sha256: &[u8]) -> Result<()> {
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(vec![domain.to_string()])?;
        params.custom_extensions = vec![rcgen::CustomExtension::new_acme_identifier(
            key_authorization_sha256,
        )];
        let cert = params.self_signed(&key_pair)?;
        let certified = challenge_key(cert.pem().as_bytes(), key_pair.serialize_pem().as_bytes())?;
        lock(&self.tls_alpn).insert(domain.to_ascii_lowercase(), Arc::new(certified));
        Ok(())
    }

    fn tls_alpn(&self, domain: &str) -> Option<Arc<CertifiedKey>> {
        lock(&self.tls_alpn)
            .get(&domain.to_ascii_lowercase())
            .cloned()
    }

    /// Forget every answer: the order is over.
    pub fn clear(&self) {
        lock(&self.http).clear();
        lock(&self.tls_alpn).clear();
    }
}

/// Inserts and removes only: a poisoned lock is still consistent.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the HTTPS listener serves: the hub's certificate, or a TLS-ALPN-01
/// answer to an ACME authority that asks with `acme-tls/1`.
#[derive(Debug, Default)]
struct HubCertResolver {
    current: RwLock<Option<Arc<CertifiedKey>>>,
    challenges: Arc<AcmeChallenges>,
}

impl ResolvesServerCert for HubCertResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let acme = hello
            .alpn()
            .is_some_and(|mut protocols| protocols.any(|p| p == ACME_TLS_ALPN));
        if acme {
            return self.challenges.tls_alpn(hello.server_name()?);
        }
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The HTTPS listener's certificate. Replacing it here (a rotation's switch,
/// D-61; a certificate from ACME, D-62) applies to the next handshake:
/// connections keep going, no restart.
#[derive(Clone, Default)]
pub struct TlsHandle(Arc<HubCertResolver>);

impl TlsHandle {
    /// Serve this certificate (chain) and key from now on.
    pub fn set(&self, cert_pem: &[u8], key_pem: &[u8]) -> Result<()> {
        let certified = certified_key(cert_pem, key_pem)?;
        *self
            .0
            .current
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(certified));
        Ok(())
    }

    /// Serve the certificate and key now in these files.
    pub async fn reload(
        &self,
        cert_path: &std::path::Path,
        key_path: &std::path::Path,
    ) -> Result<()> {
        let cert = tokio::fs::read(cert_path)
            .await
            .with_context(|| format!("can't read {}", cert_path.display()))?;
        let key = tokio::fs::read(key_path)
            .await
            .with_context(|| format!("can't read {}", key_path.display()))?;
        self.set(&cert, &key)
            .context("can't load the new TLS certificate")
    }

    /// The certificate served now (its DER), if any.
    pub fn served(&self) -> Option<Vec<u8>> {
        let current = self
            .0
            .current
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        current
            .as_ref()
            .and_then(|key| key.cert.first())
            .map(|der| der.to_vec())
    }

    /// The answers to an ACME authority's challenges.
    pub fn challenges(&self) -> &Arc<AcmeChallenges> {
        &self.0.challenges
    }

    /// The listener's TLS configuration: this handle chooses the certificate.
    pub fn rustls_config(&self) -> RustlsConfig {
        install_crypto_provider();
        let mut config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(self.0.clone());
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), ACME_TLS_ALPN.to_vec()];
        RustlsConfig::from_config(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated(host: &str) -> (tempfile::TempDir, Vec<u8>, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let (cert_pem, key_pem) =
            load_or_create_tls_cert(cert.to_str().unwrap(), key.to_str().unwrap(), host).unwrap();
        (dir, cert_pem, key_pem)
    }

    /// Regression test: rustls has two crypto backends in this build (ring and
    /// aws-lc-rs, from different dependencies) and panics unless one is chosen.
    #[tokio::test]
    async fn a_generated_certificate_is_served() {
        let (_dir, cert_pem, key_pem) = generated("hub.example.com");
        let handle = TlsHandle::default();
        let _config = handle.rustls_config();
        assert_eq!(handle.served(), None);
        handle.set(&cert_pem, &key_pem).unwrap();
        assert_eq!(
            handle.served().unwrap(),
            first_cert_der(&cert_pem).unwrap().to_vec()
        );

        let expires = cert_not_after(&cert_pem).unwrap();
        assert!(expires > time::OffsetDateTime::now_utc() + time::Duration::days(300));
    }

    #[test]
    fn a_certificate_and_another_key_are_refused() {
        let (_a, cert_pem, _) = generated("a.example.com");
        let (_b, _, other_key) = generated("b.example.com");
        assert!(TlsHandle::default().set(&cert_pem, &other_key).is_err());
    }

    #[test]
    fn details_of_a_self_signed_certificate() {
        let (_dir, cert_pem, _) = generated("hub.example.com");
        let details = certificate_details(&cert_pem).unwrap();
        assert!(details.self_signed);
        assert_eq!(details.issuer, "Blentinel Hub");
        assert_eq!(
            details.names,
            ["localhost", "127.0.0.1", "::1", "hub.example.com"]
        );
        assert_eq!(details.lifetime().num_days(), 3650);
    }

    #[test]
    fn a_new_self_signed_certificate_keeps_the_key_and_so_the_pin() {
        let (_dir, cert_pem, key_pem) = generated("hub.example.com");
        let again = self_signed_for_key(&key_pem, "hub.example.com").unwrap();
        assert_ne!(
            cert_sha256(&again).unwrap(),
            cert_sha256(&cert_pem).unwrap()
        );
        assert_eq!(
            cert_spki_sha256(&again).unwrap(),
            cert_spki_sha256(&cert_pem).unwrap()
        );
        assert_eq!(
            key_spki_sha256(&key_pem).unwrap(),
            cert_spki_sha256(&cert_pem).unwrap()
        );
        assert!(TlsHandle::default().set(&again, &key_pem).is_ok());
    }

    #[test]
    fn a_certificate_request_carries_the_names_and_the_key() {
        let (_dir, _, key_pem) = generated("hub.example.com");
        let names = vec!["hub.example.com".to_string(), "b.example.org".to_string()];
        let der = csr_der(&key_pem, &names).unwrap();
        // Signed as an authority would.
        let csr = rcgen::CertificateSigningRequestParams::from_der(&der.into()).unwrap();
        let ca_key = KeyPair::generate().unwrap();
        let ca = CertificateParams::new(Vec::<String>::new())
            .unwrap()
            .self_signed(&ca_key)
            .unwrap();
        let issued = csr.signed_by(&ca, &ca_key).unwrap().pem();
        let details = certificate_details(issued.as_bytes()).unwrap();
        assert_eq!(details.names, names);
        assert!(!details.self_signed);
        assert_eq!(
            cert_spki_sha256(issued.as_bytes()).unwrap(),
            key_spki_sha256(&key_pem).unwrap(),
            "the same key: the same pin"
        );
    }

    #[test]
    fn challenge_answers_are_kept_until_cleared() {
        let challenges = AcmeChallenges::default();
        challenges.add_http("tok", "tok.thumb");
        challenges
            .add_tls_alpn("Hub.Example.com", &[7; 32])
            .unwrap();
        assert_eq!(challenges.http("tok").as_deref(), Some("tok.thumb"));
        assert_eq!(challenges.http("other"), None);
        assert!(challenges.tls_alpn("hub.example.com").is_some());
        challenges.clear();
        assert_eq!(challenges.http("tok"), None);
        assert!(challenges.tls_alpn("hub.example.com").is_none());
    }
}
