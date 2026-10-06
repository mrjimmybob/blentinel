//! The hub's listeners move when the config changes (M0-23, D-38): real
//! sockets, `hub_lib::server::serve`, and a config edited while it runs.
//! Also what an ACME authority sees of them (D-62).
#![cfg(feature = "ssr")]

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, routing::get};
use hub_lib::{config, server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, RwLock};

/// `[server.tls]` for plain HTTP on `port` (the opt-out; the default is split mode).
const PLAIN: &str = "[server.tls]\nenabled = false\n";

/// A port nothing listens on (bound, then released).
async fn free_port() -> u16 {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    socket.local_addr().unwrap().port()
}

/// `GET path` over plain HTTP: the raw response, or `None` if nothing answers in HTTP.
async fn http_get(port: u16, path: &str) -> Option<String> {
    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: test:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.ok()?;
        let text = String::from_utf8_lossy(&response).into_owned();
        text.starts_with("HTTP/1.1").then_some(text)
    };
    tokio::time::timeout(Duration::from_secs(3), attempt)
        .await
        .ok()
        .flatten()
}

/// `GET /ping` over plain HTTP answered with 200.
async fn ping(port: u16) -> Option<String> {
    http_get(port, "/ping")
        .await
        .filter(|r| r.starts_with("HTTP/1.1 200"))
}

/// `GET path` over HTTPS, trusting the hub's own certificate (as a probe built
/// with it does): the status and body, or `None` if no HTTPS answer.
async fn https_get(dir: &std::path::Path, port: u16, path: &str) -> Option<(u16, String)> {
    let pem = std::fs::read(dir.join("cert.pem")).ok()?;
    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).ok()?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client
        .get(format!("https://127.0.0.1:{port}{path}"))
        .send()
        .await
        .ok()?;
    let status = response.status().as_u16();
    Some((status, response.text().await.ok()?))
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    for _ in 0..50 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out: {what}");
}

struct Running {
    dir: tempfile::TempDir,
    config: Arc<RwLock<config::HubConfig>>,
    changed: Arc<Notify>,
    tls: hub_lib::tls::TlsHandle,
}

impl Running {
    /// Start the listeners for a `[server]` section with `extra` lines.
    async fn start(port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(RwLock::new(Self::load(&dir, port, PLAIN)));
        let changed = Arc::new(Notify::new());
        let app = Router::new()
            .route("/ping", get(|| async { "pong" }))
            // A probe endpoint: split mode keeps it on plain HTTP.
            .route("/api/report", get(|| async { "hub-key" }))
            // A probe binary download, a route with a parameter (D-53).
            .route("/download/probe/{target}", get(|| async { "probe-binary" }))
            // An ACME authority's HTTP-01 request (D-62).
            .route(
                "/.well-known/acme-challenge/{token}",
                get(|| async { "key-authorization" }),
            );
        let tls = hub_lib::tls::TlsHandle::default();
        tokio::spawn(server::serve(
            app,
            config.clone(),
            changed.clone(),
            tls.clone(),
        ));
        Self {
            dir,
            config,
            changed,
            tls,
        }
    }

    fn load(dir: &tempfile::TempDir, port: u16, tls: &str) -> config::HubConfig {
        let path = dir.path().join("blentinel_hub.toml");
        std::fs::write(
            &path,
            format!(
                "[server]\nhost = '127.0.0.1'\nport = {port}\nstate_dir = '{}'\ndb_path = 'hub.db'\n{tls}",
                dir.path().display()
            ),
        )
        .unwrap();
        config::load_from(&path).unwrap()
    }

    /// Change the config under the running listeners, as a reload does.
    async fn change(&self, port: u16, tls: &str) {
        *self.config.write().await = Self::load(&self.dir, port, tls);
        self.changed.notify_one();
    }

    fn tls_section(&self, https_port: u16) -> String {
        format!(
            "[server.tls]\nenabled = true\ncert_path = '{}'\nkey_path = '{}'\nhttps_port = {https_port}\n",
            self.dir.path().join("cert.pem").display(),
            self.dir.path().join("key.pem").display(),
        )
    }
}

#[tokio::test]
async fn the_hub_moves_to_a_new_port() {
    let (first, second) = (free_port().await, free_port().await);
    let hub = Running::start(first).await;
    eventually("serving on the first port", async || {
        ping(first).await.is_some()
    })
    .await;

    hub.change(second, PLAIN).await;
    eventually("serving on the new port", async || {
        ping(second).await.is_some()
    })
    .await;
    eventually("the old port closed", async || ping(first).await.is_none()).await;
}

#[tokio::test]
async fn a_port_that_fails_at_the_switch_falls_back_to_the_old_one() {
    let first = free_port().await;
    let busy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let taken = busy.local_addr().unwrap().port();
    let hub = Running::start(first).await;
    eventually("serving", async || ping(first).await.is_some()).await;

    // Normally the reload refuses this (reload.rs); here it slips through.
    hub.change(taken, PLAIN).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    eventually("still serving on the old port", async || {
        ping(first).await.is_some()
    })
    .await;
}

#[tokio::test]
async fn tls_can_be_switched_on_in_place() {
    let port = free_port().await;
    let hub = Running::start(port).await;
    eventually("serving HTTP", async || ping(port).await.is_some()).await;

    let tls = hub.tls_section(port); // https_port == port: HTTPS only
    hub.change(port, &tls).await;
    eventually("certificate created", async || {
        hub.dir.path().join("cert.pem").exists()
    })
    .await;
    // HTTPS only now: plain HTTP on that port gets no HTTP answer.
    eventually("plain HTTP no longer served", async || {
        ping(port).await.is_none()
    })
    .await;
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn split_mode_keeps_probes_on_http_and_sends_everything_else_to_https() {
    let (port, https_port) = (free_port().await, free_port().await);
    let hub = Running::start(port).await;
    eventually("serving HTTP", async || ping(port).await.is_some()).await;

    let tls = hub.tls_section(https_port);
    hub.change(port, &tls).await;
    eventually("HTTPS serving", async || {
        https_get(hub.dir.path(), https_port, "/ping")
            .await
            .is_some()
    })
    .await;

    // Plain HTTP: the probe endpoint still answers...
    let probe = http_get(port, "/api/report").await.expect("HTTP answers");
    assert!(probe.starts_with("HTTP/1.1 200"), "{probe}");
    assert!(probe.ends_with("hub-key"), "{probe}");
    // ...and so do probe downloads (Windows PowerShell 5.1 can't follow a 308).
    let download = http_get(port, "/download/probe/x86_64-pc-windows-msvc")
        .await
        .expect("HTTP answers");
    assert!(download.starts_with("HTTP/1.1 200"), "{download}");
    // ...and so does an ACME authority checking a name (HTTP-01, D-62)...
    let acme = http_get(port, "/.well-known/acme-challenge/t0ken")
        .await
        .expect("HTTP answers");
    assert!(acme.starts_with("HTTP/1.1 200"), "{acme}");
    assert!(acme.ends_with("key-authorization"), "{acme}");
    // ...everything else is redirected to the same path on HTTPS.
    let other = http_get(port, "/ping?x=1").await.expect("HTTP answers");
    assert!(other.starts_with("HTTP/1.1 308"), "{other}");
    assert!(
        other
            .to_lowercase()
            .contains(&format!("location: https://test:{https_port}/ping?x=1")),
        "{other}"
    );

    // HTTPS serves everything, probe endpoints included.
    let ping = https_get(hub.dir.path(), https_port, "/ping").await;
    assert_eq!(ping, Some((200, "pong".to_string())));
    let key = https_get(hub.dir.path(), https_port, "/api/report").await;
    assert_eq!(key, Some((200, "hub-key".to_string())));
}

/// Trusts any certificate and signature, as an ACME authority reads a
/// TLS-ALPN-01 answer (self-signed by design; rustls's own signature check
/// would refuse its critical acmeIdentifier extension).
#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// A TLS handshake with `name` offering `alpn`: the protocol agreed and
/// the certificate served.
async fn handshake(port: u16, name: &str, alpn: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    hub_lib::tls::install_crypto_provider();
    let provider = rustls::crypto::CryptoProvider::get_default()?.clone();
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
        .with_no_client_auth();
    config.alpn_protocols = vec![alpn.to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    let server_name = rustls::pki_types::ServerName::try_from(name.to_string()).ok()?;
    let tls = tokio::time::timeout(
        Duration::from_secs(3),
        connector.connect(server_name, stream),
    )
    .await
    .ok()?
    .ok()?;
    let (_, session) = tls.get_ref();
    Some((
        session.alpn_protocol()?.to_vec(),
        session.peer_certificates()?.first()?.to_vec(),
    ))
}

/// The `acmeIdentifier` extension (RFC 8737) of a certificate, if it has one.
fn acme_identifier(der: &[u8]) -> Option<Vec<u8>> {
    use x509_parser::prelude::{FromDer, X509Certificate};
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    cert.extensions()
        .iter()
        .find(|e| e.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
        .map(|e| {
            assert!(e.critical, "the acmeIdentifier extension is critical");
            e.value.to_vec()
        })
}

#[tokio::test]
async fn the_https_listener_answers_an_acme_tls_alpn_challenge() {
    let port = free_port().await;
    let hub = Running::start(port).await;
    eventually("serving HTTP", async || ping(port).await.is_some()).await;
    let tls = hub.tls_section(port); // HTTPS only: the authority comes to 443
    hub.change(port, &tls).await;
    eventually("HTTPS serving", async || {
        handshake(port, "hub.example.com", b"http/1.1")
            .await
            .is_some()
    })
    .await;

    // No challenge waiting: an authority's handshake gets nothing.
    assert_eq!(
        handshake(port, "hub.example.com", b"acme-tls/1").await,
        None
    );

    let digest = [9u8; 32];
    hub.tls
        .challenges()
        .add_tls_alpn("hub.example.com", &digest)
        .unwrap();
    let (alpn, cert) = handshake(port, "hub.example.com", b"acme-tls/1")
        .await
        .expect("the challenge is answered");
    assert_eq!(alpn, b"acme-tls/1");
    // The extension holds the digest as a DER OCTET STRING.
    let value = acme_identifier(&cert).expect("an acmeIdentifier");
    assert_eq!(value[..2], [0x04, 32]);
    assert_eq!(value[2..], digest);

    // Browsers and probes still get the hub's own certificate meanwhile.
    let (alpn, cert) = handshake(port, "hub.example.com", b"http/1.1")
        .await
        .expect("the hub's certificate");
    assert_eq!(alpn, b"http/1.1");
    assert_eq!(acme_identifier(&cert), None);
    assert_eq!(Some(cert), hub.tls.served());

    // Another name, or after the order: nothing.
    assert_eq!(
        handshake(port, "other.example.com", b"acme-tls/1").await,
        None
    );
    hub.tls.challenges().clear();
    assert_eq!(
        handshake(port, "hub.example.com", b"acme-tls/1").await,
        None
    );
}
