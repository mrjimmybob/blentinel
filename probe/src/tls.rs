//! Certificate pinning (D-01, D-47, D-61): an enrolled probe trusts only a
//! certificate whose public key (SPKI) has the SHA-256 from its enrollment
//! code, or the next one its hub announced while rotating. No certificate is
//! built into the binary, so every hub works with the same official probe.
//!
//! Everything else the probe reaches over TLS (a hub behind a proxy with a
//! public certificate, the targets of `http` and `tls` checks) is trusted by
//! Mozilla's list of authorities, compiled in ([`public_roots`]). The probe
//! never asks the machine's certificate store (FP-2, D-78): every HTTP client
//! gets one of the configurations made here.

use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

/// The crypto provider of every TLS connection the probe makes.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// What browsers trust: Mozilla's list of authorities, compiled in.
pub fn public_roots() -> RootCertStore {
    RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
}

/// A TLS client configuration that trusts certificates issued under
/// [`public_roots`], for the name connected to.
pub fn public_client_config() -> Result<ClientConfig> {
    Ok(ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .context("TLS protocol versions")?
        .with_root_certificates(public_roots())
        .with_no_client_auth())
}

/// Accepts any certificate: for checks told not to verify. The server must
/// still prove it holds the certificate's key.
#[derive(Debug)]
struct AnyCertificate {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A TLS client configuration that accepts any certificate (an `http` check
/// with `verify` off). Never for the hub.
pub fn unverified_client_config() -> Result<ClientConfig> {
    let provider = provider();
    let verifier = AnyCertificate {
        algorithms: provider.signature_verification_algorithms,
    };
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("TLS protocol versions")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}
/// Accepts only a certificate whose public key has one of these SHA-256s
/// (`common::pins::spki_sha256`), and still checks that the server proves
/// it holds that key. The name in the certificate isn't checked: the pin is
/// stronger, and its expiry neither (the hub replaces it by rotation).
#[derive(Debug)]
struct PinnedCertificate {
    spki_sha256: Vec<[u8; 32]>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let pinned = common::pins::spki_sha256(end_entity.as_ref())
            .is_ok_and(|spki| self.spki_sha256.contains(&spki));
        if pinned {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the hub's certificate isn't the one this probe pins \
                 (enroll again with a new code if the hub's certificate was replaced)"
                    .into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A TLS client configuration that trusts only certificates whose public
/// key has one of `spki_sha256`.
pub fn pinned_client_config(spki_sha256: Vec<[u8; 32]>) -> Result<ClientConfig> {
    let provider = provider();
    let verifier = PinnedCertificate {
        spki_sha256,
        algorithms: provider.signature_verification_algorithms,
    };
    Ok(ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .context("TLS protocol versions")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A self-signed certificate like the hub's, and its pin.
    fn certificate() -> (rcgen::CertifiedKey, [u8; 32]) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin = common::pins::spki_sha256(certified.cert.der()).unwrap();
        (certified, pin)
    }

    /// An HTTPS server answering "pinned-ok" with `certified`; returns its URL.
    async fn https_server(certified: rcgen::CertifiedKey) -> String {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let key =
            rustls::pki_types::PrivateKeyDer::try_from(certified.key_pair.serialize_der()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "https://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        );
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(socket).await else {
                        return;
                    };
                    let mut request = [0u8; 1024];
                    let _ = tls.read(&mut request).await;
                    let body = "pinned-ok";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        url
    }

    async fn get(url: &str, pins: Vec<[u8; 32]>) -> reqwest::Result<String> {
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(pinned_client_config(pins).unwrap())
            .build()?;
        client.get(url).send().await?.text().await
    }

    #[tokio::test]
    async fn the_pinned_certificate_is_accepted_and_any_other_refused() {
        let (certified, pin) = certificate();
        let url = https_server(certified).await;

        assert_eq!(get(&url, vec![pin]).await.unwrap(), "pinned-ok");

        let (_, other) = certificate();
        assert!(
            get(&url, vec![other]).await.is_err(),
            "a different certificate"
        );
        assert_eq!(
            get(&url, vec![other, pin]).await.unwrap(),
            "pinned-ok",
            "while rotating, either pin will do"
        );
    }
}
