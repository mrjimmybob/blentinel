#![cfg(feature = "ssr")]
//! The hub's listeners (HTTP, HTTPS, or both), restarted when a config reload
//! changes where the hub listens (D-38).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinSet;

use crate::config::{HubConfig, ServerConfig};
use crate::tls;

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
    /// HTTPS on this port *and* HTTP on `port` (migration mode); `None`: HTTPS only, on `port`.
    https_port: Option<u16>,
}

impl Listen {
    pub fn from_config(server: &ServerConfig) -> Self {
        Self {
            host: server.host.clone(),
            port: server.port,
            tls: server.tls.enabled.then(|| Tls {
                cert_path: server.tls.cert_path.clone(),
                key_path: server.tls.key_path.clone(),
                https_port: server.tls.https_port,
            }),
        }
    }

    fn http_addr(&self) -> Option<String> {
        match &self.tls {
            None => Some(format!("{}:{}", self.host, self.port)),
            Some(tls) => tls
                .https_port
                .map(|_| format!("{}:{}", self.host, self.port)),
        }
    }

    fn https_addr(&self) -> Option<String> {
        self.tls
            .as_ref()
            .map(|tls| format!("{}:{}", self.host, tls.https_port.unwrap_or(self.port)))
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
) -> Result<()> {
    let mut current = Listen::from_config(&config.read().await.server);
    let mut running = start(&app, &current).await?;
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
                match start(&app, &wanted).await {
                    Ok(started) => {
                        running = started;
                        current = wanted;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Can't listen as configured ({e:#}); going back to the previous settings. Fix the config file."
                        );
                        running = start(&app, &current)
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
async fn start(app: &Router, listen: &Listen) -> Result<Running> {
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
            let rustls = tls::build_rustls_config(&cert, &key).await?;
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
        let service = app.clone().into_make_service();
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
        tracing::info!("Blentinel Hub listening on http://{addr}");
    }
    if let Some((socket, rustls, addr)) = https {
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp_rustls(socket, rustls).handle(handle.clone());
        let service = app.clone().into_make_service();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn listen(port: u16, tls: Option<Option<u16>>) -> Listen {
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
        assert_eq!(listen(3000, None).addrs(), ["127.0.0.1:3000"]);
        assert_eq!(listen(3000, Some(None)).addrs(), ["127.0.0.1:3000"]);
        let dual = listen(3000, Some(Some(3443)));
        assert_eq!(dual.http_addr().as_deref(), Some("127.0.0.1:3000"));
        assert_eq!(dual.https_addr().as_deref(), Some("127.0.0.1:3443"));
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
            check_bindable(&listen(port, None), &listen(port, Some(None)))
                .await
                .is_ok()
        );
    }
}
