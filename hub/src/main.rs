//! Hub entry point: CLI handling, startup wiring, background tasks, and the
//! listeners. All request handling lives in the library (`hub_lib`).

#[cfg(feature = "ssr")]
use hub_lib::{
    AppState, args, auth, clock, config, hot_reload, identity, logging, routes,
    service::{
        self,
        alerts::{Notifier, SmtpNotifier},
    },
    storage, tls,
};

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();
    logging::init(&args);

    // --init: create config template and exit before any server startup.
    // Does not need a config or state_dir — it creates the template file.
    if args.init {
        match config::create_default_config_file() {
            Ok(true) => {
                println!("Created default configuration at: blentinel_hub.toml");
                println!("Review and edit before starting the hub.");
            }
            Ok(false) => {
                eprintln!("Configuration file already exists at: blentinel_hub.toml");
                eprintln!("Refusing to overwrite existing configuration.");
                eprintln!("Remove or rename the file manually if you want to regenerate it.");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to create configuration file: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Resolve the config path once here so both utility commands and the
    // server start use the same effective path.
    let config_path = args
        .config_path
        .clone()
        .unwrap_or_else(config::get_config_path);

    // --print-public-key / --reset-admin-token: load config for path
    // resolution, perform the action, then exit without starting the server.
    if args.print_public_key || args.reset_admin_token {
        let cfg = match config::load_from(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[ERROR] Failed to load configuration: {}", e);
                std::process::exit(1);
            }
        };

        if args.print_public_key {
            let identity_key_path = cfg.server.resolve_path(&cfg.server.identity_key_path);
            let pub_key_path = identity_key_path.with_extension("pub");
            if let Err(e) = identity::print_and_write_public_key(&identity_key_path, &pub_key_path)
            {
                eprintln!("[ERROR] {e:#}");
                std::process::exit(1);
            }
        }

        if args.reset_admin_token {
            let auth_token_path = cfg.server.resolve_path(&cfg.server.auth_token_path);
            if let Err(e) = auth::reset_token(&auth_token_path) {
                eprintln!("[ERROR] {e:#}");
                std::process::exit(1);
            }
        }

        return;
    }

    if let Err(e) = run(args, config_path).await {
        tracing::error!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(feature = "ssr")]
async fn run(args: args::Args, config_path: std::path::PathBuf) -> anyhow::Result<()> {
    use anyhow::Context;
    use hub_lib::app::{App, shell};
    use leptos::prelude::*;
    use leptos_axum::{LeptosRoutes, generate_route_list};

    // Load hub configuration and wrap in Arc<RwLock> for hot reloading.
    let cfg = std::sync::Arc::new(tokio::sync::RwLock::new(
        config::load_from(&config_path).context("Failed to load hub configuration")?,
    ));
    tracing::info!("Configuration loaded from {}", config_path.display());

    // Leptos needs its own options for the frontend; override site_addr with ours.
    // Deployed mode: serve the web assets (pkg/) from next to the executable,
    // so a service can use its state directory as working directory (D-34).
    let conf = get_configuration(None).context("Failed to read Leptos configuration")?;
    let mut leptos_options = conf.leptos_options;
    if let Some(site_root) = deployed_site_root() {
        leptos_options.site_root = site_root.to_string_lossy().into_owned().into();
    }
    {
        let cfg_read = cfg.read().await;
        leptos_options.site_addr = cfg_read.bind_addr().parse().context(format!(
            "Invalid bind address {}:{}",
            cfg_read.server.host, cfg_read.server.port
        ))?;
    }
    let routes = generate_route_list(App);

    // Resolve runtime state paths relative to state_dir (if configured).
    // Absolute paths are always used as-is; relative paths are anchored to
    // state_dir when set, or to the working directory otherwise.
    let (state_dir, db_path, identity_key_path, auth_token_path) = {
        let cfg_read = cfg.read().await;
        let s = &cfg_read.server;
        (
            s.state_dir.clone(),
            s.resolved_db_path(),
            s.resolve_path(&s.identity_key_path),
            s.resolve_path(&s.auth_token_path),
        )
    };

    // Ensure state_dir exists — create it on first run so that SQLite, identity
    // keys, and auth tokens have a home directory.
    std::fs::create_dir_all(&state_dir).context(format!(
        "Failed to create state directory '{}'. Check permissions or create it manually.",
        state_dir.display()
    ))?;

    let storage: std::sync::Arc<dyn storage::Storage> = std::sync::Arc::new(
        storage::SqliteStorage::open(&db_path)
            .await
            .context(format!(
                "Failed to open database at '{}'",
                db_path.display()
            ))?,
    );
    tracing::info!("Database ready: {}", db_path.display());

    // Hub identity (persistent X25519 key).
    // Public key file is only written on first creation (in identity.rs).
    let hub_secret = identity::load_or_create_hub_key(&identity_key_path)?;

    let whitelist = cfg.read().await.probe_whitelist();
    tracing::info!("Registered probes: {}", whitelist.len());
    for (key, name) in &whitelist {
        tracing::debug!("  {} ({}...)", name, hub_lib::model::short_probe_id(key));
    }

    // Auth
    let admin_token = auth::load_or_create_token(&auth_token_path)?;
    let sessions = auth::new_session_store();

    let clock: std::sync::Arc<dyn clock::Clock> = std::sync::Arc::new(clock::SystemClock);
    let notifier: std::sync::Arc<dyn Notifier> =
        std::sync::Arc::new(SmtpNotifier::new(std::sync::Arc::clone(&cfg)));

    // Clone what the background tasks need before the storage is moved into state.
    let expiry_storage = std::sync::Arc::clone(&storage);
    let expiry_notifier = std::sync::Arc::clone(&notifier);
    let expiry_config = std::sync::Arc::clone(&cfg);
    let expiry_clock = std::sync::Arc::clone(&clock);
    let monitor_storage = std::sync::Arc::clone(&storage);
    let monitor_clock = std::sync::Arc::clone(&clock);
    let monitor_config = std::sync::Arc::clone(&cfg);

    // Assemble state
    let state = AppState {
        leptos_options: leptos_options.clone(),
        storage,
        hub_secret,
        config: std::sync::Arc::clone(&cfg),
        args: args.clone(),
        sessions,
        admin_token,
        clock,
        notifier,
    };

    // TLS Configuration (load or generate certificate if enabled)
    let tls_config = cfg.read().await.server.tls.clone();
    let tls_cert_key = if tls_config.enabled {
        let host = cfg.read().await.server.host.clone();
        Some(tls::load_or_create_tls_cert(
            &tls_config.cert_path,
            &tls_config.key_path,
            &host,
        )?)
    } else {
        None
    };

    // Spawn config file watcher (passes the resolved config path so hot-reload
    // reloads from the same file regardless of whether --config was used).
    tokio::spawn(hot_reload::watch_config(
        std::sync::Arc::clone(&cfg),
        config_path.clone(),
    ));

    // Build the router: the API (hub_lib::routes) plus the Leptos UI on top.
    let app = routes::api_router()
        // Leptos routes + static file fallback (serves /pkg/*)
        .leptos_routes(&state, routes, move || shell(leptos_options.clone()))
        .fallback(leptos_axum::file_and_error_handler::<AppState, _>(shell))
        .with_state(state);

    // Background task: periodically scan for probes that have gone silent.
    // Sleeps first so a hub restart does not instantly expire everyone.
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            let sweep = service::liveness::sweep_expired_probes(
                &expiry_storage,
                &expiry_notifier,
                &expiry_config,
                expiry_clock.now(),
            );
            match sweep.await {
                Ok(expired) => {
                    for (probe_id, company_id) in &expired {
                        tracing::info!(
                            probe = %hub_lib::model::short_probe_id(probe_id),
                            company = %company_id,
                            "Probe stopped reporting"
                        );
                    }
                }
                Err(e) => tracing::error!("Probe expiry check failed: {e:#}"),
            }
        }
    });

    // Spawn DB size monitor (checks every 10 minutes)
    service::retention::spawn_size_monitor(monitor_storage, monitor_config, monitor_clock);

    // Server startup: support HTTP-only, HTTPS-only, or dual-mode
    let (host, port) = {
        let cfg_read = cfg.read().await;
        (cfg_read.server.host.clone(), cfg_read.server.port)
    };

    if let Some((cert_pem, key_pem)) = tls_cert_key {
        let rustls_config = tls::build_rustls_config(&cert_pem, &key_pem).await?;

        if let Some(https_port) = tls_config.https_port {
            // Dual mode: HTTP and HTTPS side by side. If either stops, the hub
            // exits with its error (a spawned task would only panic in the background).
            let http_addr = format!("{}:{}", host, port);
            let https_addr = format!("{}:{}", host, https_port);
            let listener = tokio::net::TcpListener::bind(&http_addr)
                .await
                .context(format!("Failed to bind HTTP to {}", http_addr))?;
            let https_socket = https_addr.parse()?;
            tracing::info!("Blentinel Hub listening on http://{}", http_addr);
            tracing::info!("Blentinel Hub listening on https://{}", https_addr);

            let http_app = app.clone();
            let http = async move {
                axum::serve(listener, http_app.into_make_service())
                    .await
                    .context("HTTP server error")
            };
            let https = async {
                axum_server::bind_rustls(https_socket, rustls_config)
                    .serve(app.into_make_service())
                    .await
                    .context("HTTPS server error")
            };
            tokio::try_join!(http, https)?;
        } else {
            // HTTPS only
            let https_addr = format!("{}:{}", host, port);
            tracing::info!("Blentinel Hub listening on https://{}", https_addr);

            axum_server::bind_rustls(https_addr.parse()?, rustls_config)
                .serve(app.into_make_service())
                .await
                .context("HTTPS server error")?;
        }
    } else {
        // HTTP only (existing behavior)
        let addr = format!("{}:{}", host, port);
        tracing::info!("Blentinel Hub listening on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .context(format!("Failed to bind to {}", addr))?;
        axum::serve(listener, app.into_make_service())
            .await
            .context("Server error")?;
    }

    Ok(())
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}

/// Where a deployed hub's web assets are: the folder containing `pkg/`, looked
/// up next to the executable first, then in the working directory. `None` in a
/// development build, where Leptos serves from its compile-time `target/site`.
#[cfg(feature = "ssr")]
fn deployed_site_root() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [exe_dir, std::path::PathBuf::from(".")]
        .into_iter()
        .find(|dir| dir.join("pkg").is_dir())
}
