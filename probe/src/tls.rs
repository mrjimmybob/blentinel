//! Certificate pinning (D-01, D-47): an enrolled probe trusts exactly one
//! certificate for its hub, the one whose SHA-256 was in its enrollment code.
//! No certificate is built into the binary, so every hub works with the same
//! official probe.

use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// Accepts only the certificate with this SHA-256 (of its DER bytes), and
/// still checks that the server proves it holds that certificate's key.
/// The name in the certificate isn't checked: the pin is stronger.
#[derive(Debug)]
struct PinnedCertificate {
    sha256: [u8; 32],
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
        if Sha256::digest(end_entity.as_ref()).as_slice() == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the hub's certificate doesn't match the one in the enrollment code \
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

/// A TLS client configuration that trusts only the certificate with `sha256`.
pub fn pinned_client_config(sha256: [u8; 32]) -> Result<ClientConfig> {
    let provider: Arc<CryptoProvider> = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = PinnedCertificate {
        sha256,
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

    /// A self-signed certificate like the hub's, and its SHA-256.
    fn certificate() -> (rcgen::CertifiedKey, [u8; 32]) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let sha = Sha256::digest(certified.cert.der().as_ref()).into();
        (certified, sha)
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

    async fn get(url: &str, pin: [u8; 32]) -> reqwest::Result<String> {
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(pinned_client_config(pin).unwrap())
            .build()?;
        client.get(url).send().await?.text().await
    }

    #[tokio::test]
    async fn the_pinned_certificate_is_accepted_and_any_other_refused() {
        let (certified, sha) = certificate();
        let url = https_server(certified).await;

        assert_eq!(get(&url, sha).await.unwrap(), "pinned-ok");

        let (_, other) = certificate();
        assert!(get(&url, other).await.is_err(), "a different certificate");
    }
}
