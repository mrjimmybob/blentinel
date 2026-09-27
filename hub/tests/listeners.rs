//! The hub's listeners move when the config changes (M0-23, D-38): real
//! sockets, `hub_lib::server::serve`, and a config edited while it runs.
#![cfg(feature = "ssr")]

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, routing::get};
use hub_lib::{config, server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, RwLock};

/// A port nothing listens on (bound, then released).
async fn free_port() -> u16 {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    socket.local_addr().unwrap().port()
}

/// `GET /ping` over plain HTTP; `None` if nothing answers in HTTP.
async fn ping(port: u16) -> Option<String> {
    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        stream
            .write_all(b"GET /ping HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.ok()?;
        let text = String::from_utf8_lossy(&response).into_owned();
        text.starts_with("HTTP/1.1 200").then_some(text)
    };
    tokio::time::timeout(Duration::from_secs(3), attempt)
        .await
        .ok()
        .flatten()
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
        let config = Arc::new(RwLock::new(Self::load(&dir, port, "")));
        let changed = Arc::new(Notify::new());
        let app = Router::new().route("/ping", get(|| async { "pong" }));
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

    fn tls_section(&self, https_port: Option<u16>) -> String {
        let https = https_port
            .map(|p| format!("https_port = {p}\n"))
            .unwrap_or_default();
        format!(
            "[server.tls]\nenabled = true\ncert_path = '{}'\nkey_path = '{}'\n{https}",
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

    hub.change(second, "").await;
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
    hub.change(taken, "").await;
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

    let tls = hub.tls_section(None);
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
async fn dual_mode_keeps_http_and_adds_https() {
    let (port, https_port) = (free_port().await, free_port().await);
    let hub = Running::start(port).await;
    eventually("serving HTTP", async || ping(port).await.is_some()).await;

    let tls = hub.tls_section(Some(https_port));
    hub.change(port, &tls).await;
    eventually("HTTPS port open", async || {
        tokio::net::TcpStream::connect(("127.0.0.1", https_port))
            .await
            .is_ok()
    })
    .await;
    assert!(ping(port).await.is_some(), "HTTP still served");
    assert!(
        ping(https_port).await.is_none(),
        "the HTTPS port speaks TLS"
    );
}
