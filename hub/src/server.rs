#![cfg(feature = "ssr")]
//! The hub's listeners (HTTP, HTTPS, or both), restarted when a config reload
//! changes where the hub listens (D-38).
//!
//! Default is split mode (D-43): HTTPS on `https_port` for everything, and
//! plain HTTP on `port` for the probe endpoints only (their reports are
//! signed and encrypted end to end) and an ACME authority's HTTP-01 requests
//! (D-62); anything else there is redirected to HTTPS, so passwords and
//! session cookies never cross plain HTTP.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinSet;

use crate::auth::{Access, Transport};
use crate::config::{HubConfig, ServerConfig};
use crate::{routes, tls};

/// How long requests in flight get to finish when the listeners move.
const DRAIN: Duration = Duration::from_secs(5);

/// Where and how the hub listens: the part of `[server]` the listeners use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listen {
    host: String,
    port: u16,
    tls: Option<Tls>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tls {
    cert_path: String,
    key_path: String,
    /// Where HTTPS listens. Different from `port`: `port` keeps plain HTTP
    /// for the probe endpoints (split mode). Equal: HTTPS only.
    https_port: u16,
}

impl Listen {
    pub fn from_config(server: &ServerConfig) -> Self {
        Self {
            host: server.host.clone(),
            port: server.port,
            tls: server.tls.enabled.then(|| Tls {
                cert_path: server
                    .resolved_tls_cert_path()
                    .to_string_lossy()
                    .into_owned(),
                key_path: server
                    .resolved_tls_key_path()
                    .to_string_lossy()
                    .into_owned(),
                https_port: server.tls.https_port,
            }),
        }
    }

    fn http_addr(&self) -> Option<String> {
        match &self.tls {
            Some(tls) if tls.https_port == self.port => None,
            _ => Some(format!("{}:{}", self.host, self.port)),
        }
    }

    fn https_addr(&self) -> Option<String> {
        self.tls
            .as_ref()
            .map(|tls| format!("{}:{}", self.host, tls.https_port))
    }

    /// In split mode, the HTTPS port that plain-HTTP requests other than the
    /// probe endpoints are redirected to.
    fn redirect_to(&self) -> Option<u16> {
        self.tls
            .as_ref()
            .filter(|tls| tls.https_port != self.port)
            .map(|tls| tls.https_port)
    }

    fn addrs(&self) -> Vec<String> {
        self.http_addr()
            .into_iter()
            .chain(self.https_addr())
            .collect()
    }
}

/// Fail if `new` needs an address that can't be bound now. Addresses `old`
/// already listens on are skipped: the hub frees them itself when it switches.
/// Called while preparing a reload, so a bad port refuses the whole reload.
pub async fn check_bindable(old: &Listen, new: &Listen) -> Result<()> {
    let taken = old.addrs();
    for addr in new.addrs().iter().filter(|a| !taken.contains(a)) {
        tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("can't listen on {addr}"))?;
    }
    Ok(())
}

/// Serve `app` as `[server]` says, until a listener fails. When `changed`
/// fires and the listening settings differ, the listeners move: the old ones
/// stop (requests in flight get [`DRAIN`] to finish), the new ones start. If
/// the new ones can't start, the hub goes back to the previous settings.
pub async fn serve(
    app: Router,
    config: Arc<RwLock<HubConfig>>,
    changed: Arc<Notify>,
    tls_handle: tls::TlsHandle,
) -> Result<()> {
    let mut current = Listen::from_config(&config.read().await.server);
    let mut running = start(&app, &current, &tls_handle).await?;
    loop {
        tokio::select! {
            ended = running.tasks.join_next() => {
                return match ended {
                    Some(Ok(Err(e))) => Err(e),
                    Some(Err(e)) => Err(e).context("listener task failed"),
                    _ => Err(anyhow::anyhow!("a listener stopped unexpectedly")),
                };
            }
            () = changed.notified() => {
                let wanted = Listen::from_config(&config.read().await.server);
                if wanted == current {
                    continue;
                }
                running.stop().await;
                match start(&app, &wanted, &tls_handle).await {
                    Ok(started) => {
                        running = started;
                        current = wanted;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Can't listen as configured ({e:#}); going back to the previous settings. Fix the config file."
                        );
                        running = start(&app, &current, &tls_handle)
                            .await
                            .context("the previous listening settings failed too")?;
                    }
                }
            }
        }
    }
}

/// Listeners that are running, and how to stop them.
struct Running {
    tasks: JoinSet<Result<()>>,
    stop: Vec<Box<dyn FnOnce() + Send>>,
}

impl Running {
    /// Stop accepting, let requests in flight finish (up to [`DRAIN`]), then drop the rest.
    async fn stop(mut self) {
        for stop in self.stop.drain(..) {
            stop();
        }
        let drained = tokio::time::timeout(DRAIN + Duration::from_secs(1), async {
            while self.tasks.join_next().await.is_some() {}
        });
        if drained.await.is_err() {
            self.tasks.abort_all();
        }
    }
}

/// Bind every address first, then start serving: a bind error leaves nothing running.
async fn start(app: &Router, listen: &Listen, tls_handle: &tls::TlsHandle) -> Result<Running> {
    let http = match listen.http_addr() {
        Some(addr) => Some((
            tokio::net::TcpListener::bind(&addr)
                .await
                .with_context(|| format!("can't listen on {addr}"))?,
            addr,
        )),
        None => None,
    };
    let https = match (&listen.tls, listen.https_addr()) {
        (Some(tls), Some(addr)) => {
            let (cert, key) =
                tls::load_or_create_tls_cert(&tls.cert_path, &tls.key_path, &listen.host)?;
            // A rotation (D-61) or ACME (D-62) replaces it in place.
            tls_handle.set(&cert, &key)?;
            let rustls = tls_handle.rustls_config();
            let socket = std::net::TcpListener::bind(&addr)
                .with_context(|| format!("can't listen on {addr}"))?;
            socket.set_nonblocking(true)?;
            Some((socket, rustls, addr))
        }
        _ => None,
    };

    let mut running = Running {
        tasks: JoinSet::new(),
        stop: Vec::new(),
    };
    if let Some((listener, addr)) = http {
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let mut http_app = app.clone().layer(axum::Extension(Transport::Http));
        if let Some(https_port) = listen.redirect_to() {
            let fallback_host = listen.host.clone();
            http_app = http_app.layer(axum::middleware::from_fn(move |req, next| {
                probes_only(req, next, https_port, fallback_host.clone())
            }));
        }
        let service = http_app.into_make_service_with_connect_info::<SocketAddr>();
        running.tasks.spawn(async move {
            axum::serve(listener, service)
                .with_graceful_shutdown(async {
                    let _ = stop_rx.await;
                })
                .await
                .context("HTTP server error")
        });
        running.stop.push(Box::new(move || {
            let _ = stop_tx.send(());
        }));
        match listen.redirect_to() {
            Some(port) => tracing::info!(
                "Blentinel Hub listening on http://{addr} for probes (everything else goes to HTTPS on port {port})"
            ),
            None => tracing::info!("Blentinel Hub listening on http://{addr}"),
        }
    }
    if let Some((socket, rustls, addr)) = https {
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp_rustls(socket, rustls)
            .context("the HTTPS socket can't be used")?
            .handle(handle.clone());
        let service = app
            .clone()
            .layer(axum::Extension(Transport::Https))
            .into_make_service_with_connect_info::<SocketAddr>();
        running
            .tasks
            .spawn(async move { server.serve(service).await.context("HTTPS server error") });
        running
            .stop
            .push(Box::new(move || handle.graceful_shutdown(Some(DRAIN))));
        tracing::info!("Blentinel Hub listening on https://{addr}");
    }
    Ok(running)
}

/// Split mode, on the plain-HTTP listener: let the probe endpoints through
/// and send everything else to the same path on HTTPS (308 keeps the method).
async fn probes_only(
    request: Request,
    next: Next,
    https_port: u16,
    fallback_host: String,
) -> Response {
    let path = request.uri().path();
    let is_probe_route = routes::access_table()
        .iter()
        .any(|r| r.access == Access::Probe && path_matches(r.path, path));
    // An ACME authority asks for HTTP-01 answers over plain HTTP (D-62).
    if is_probe_route || path.starts_with(crate::acme::CHALLENGE_PREFIX) {
        return next.run(request).await;
    }
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(host_without_port)
        .unwrap_or(fallback_host);
    let path_and_query = request.uri().path_and_query().map_or("/", |p| p.as_str());
    let location = format!("https://{host}:{https_port}{path_and_query}");
    (
        StatusCode::PERMANENT_REDIRECT,
        [(header::LOCATION, location)],
    )
        .into_response()
}

/// Does `path` match a route pattern such as `/download/probe/{target}`?
/// (A `{…}` segment matches any one non-empty segment.)
fn path_matches(pattern: &str, path: &str) -> bool {
    let (pattern, path): (Vec<_>, Vec<_>) =
        (pattern.split('/').collect(), path.split('/').collect());
    pattern.len() == path.len()
        && pattern
            .iter()
            .zip(&path)
            .all(|(p, s)| *p == *s || (p.starts_with('{') && p.ends_with('}') && !s.is_empty()))
}

/// `example.com:3000` → `example.com`; `[::1]:3000` → `[::1]`.
fn host_without_port(host: &str) -> String {
    match host.rfind(']') {
        Some(end) => host[..=end].to_string(),
        None => host.split(':').next().unwrap_or(host).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_patterns_match_one_segment_per_parameter() {
        assert!(path_matches("/api/report", "/api/report"));
        assert!(path_matches(
            "/download/probe/{target}",
            "/download/probe/x86_64-pc-windows-msvc"
        ));
        assert!(!path_matches(
            "/download/probe/{target}",
            "/download/probe/"
        ));
        assert!(!path_matches(
            "/download/probe/{target}",
            "/download/probe/a/b"
        ));
        assert!(!path_matches("/api/report", "/api/reports"));
    }

    fn listen(port: u16, tls: Option<u16>) -> Listen {
        Listen {
            host: "127.0.0.1".into(),
            port,
            tls: tls.map(|https_port| Tls {
                cert_path: "c.pem".into(),
                key_path: "k.pem".into(),
                https_port,
            }),
        }
    }

    #[test]
    fn addresses_for_each_mode() {
        let plain = listen(3000, None);
        assert_eq!(plain.addrs(), ["127.0.0.1:3000"]);
        assert_eq!(plain.redirect_to(), None);

        let https_only = listen(3000, Some(3000));
        assert_eq!(https_only.http_addr(), None);
        assert_eq!(https_only.https_addr().as_deref(), Some("127.0.0.1:3000"));
        assert_eq!(https_only.redirect_to(), None);

        let split = listen(3000, Some(3443));
        assert_eq!(split.http_addr().as_deref(), Some("127.0.0.1:3000"));
        assert_eq!(split.https_addr().as_deref(), Some("127.0.0.1:3443"));
        assert_eq!(split.redirect_to(), Some(3443));
    }

    #[test]
    fn redirects_keep_the_host_name_but_not_its_port() {
        assert_eq!(host_without_port("hub.example.com:3000"), "hub.example.com");
        assert_eq!(host_without_port("hub.example.com"), "hub.example.com");
        assert_eq!(host_without_port("192.0.2.10:3000"), "192.0.2.10");
        assert_eq!(host_without_port("[::1]:3000"), "[::1]");
    }

    #[tokio::test]
    async fn an_address_in_use_is_not_bindable_but_our_own_is() {
        let busy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = busy.local_addr().unwrap().port();
        // Another program holds it: refused.
        assert!(
            check_bindable(&listen(1, None), &listen(port, None))
                .await
                .is_err()
        );
        // We hold it ourselves: fine, we free it when switching.
        assert!(
            check_bindable(&listen(port, None), &listen(port, Some(port)))
                .await
                .is_ok()
        );
    }
}
