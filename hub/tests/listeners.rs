//! The hub's listeners move when the config changes (M0-23, D-38): real
//! sockets, `hub_lib::server::serve`, and a config edited while it runs.
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
            .route("/api/handshake", get(|| async { "hub-key" }));
        tokio::spawn(server::serve(app, config.clone(), changed.clone()));
        Self {
            dir,
            config,
            changed,
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
    let probe = http_get(port, "/api/handshake")
        .await
        .expect("HTTP answers");
    assert!(probe.starts_with("HTTP/1.1 200"), "{probe}");
    assert!(probe.ends_with("hub-key"), "{probe}");
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
    let key = https_get(hub.dir.path(), https_port, "/api/handshake").await;
    assert_eq!(key, Some((200, "hub-key".to_string())));
}
