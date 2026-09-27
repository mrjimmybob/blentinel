//! Hub entry point: CLI handling, startup wiring, background tasks, and the
//! listeners. All request handling lives in the library (`hub_lib`).

#[cfg(feature = "ssr")]
use hub_lib::{
    alerts, archive, args, auth, clock, config, db, hot_reload, identity, routes, tls, AppState,
};

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();

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
    let config_path = args.config_path
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
            identity::print_and_write_public_key(&identity_key_path, &pub_key_path);
        }

        if args.reset_admin_token {
            let auth_token_path = cfg.server.resolve_path(&cfg.server.auth_token_path);
            auth::reset_token(&auth_token_path);
        }

        return;
    }

    if let Err(e) = run(args, config_path).await {
        eprintln!("\n[ERROR] {:#}", e);
        std::process::exit(1);
    }
}

#[cfg(feature = "ssr")]
async fn run(args: args::Args, config_path: std::path::PathBuf) -> anyhow::Result<()> {
    use anyhow::Context;
    use hub_lib::app::{shell, App};
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};

    // Load hub configuration and wrap in Arc<RwLock> for hot reloading.
    let cfg = std::sync::Arc::new(tokio::sync::RwLock::new(
        config::load_from(&config_path).context("Failed to load hub configuration")?
    ));
    if args.verbose { println!("Configuration loaded from {}", config_path.display()); }

    // Leptos needs its own options for the frontend; override site_addr with ours.
    // Also detect deployed mode: if pkg/ exists in the current directory, serve
    // from "." instead of the compile-time "target/site".
    let conf = get_configuration(None).context("Failed to read Leptos configuration")?;
    let mut leptos_options = conf.leptos_options;
    if std::path::Path::new("pkg").exists() {
        leptos_options.site_root = ".".into();
    }
    {
        let cfg_read = cfg.read().await;
        leptos_options.site_addr = cfg_read.bind_addr().parse()
            .context(format!("Invalid bind address {}:{}", cfg_read.server.host, cfg_read.server.port))?;
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

    let pool = db::open(&db_path).await.context(format!(
        "Failed to open database at '{}'",
        db_path.display()
    ))?;
    if args.verbose { println!("Database initialized: {}", db_path.display()); }

    // Hub identity (persistent X25519 key).
    // Public key file is only written on first creation (in identity.rs).
    let hub_secret = identity::load_or_create_hub_key(&identity_key_path);

    // Display initial probe whitelist
    if args.verbose {
        let whitelist = cfg.read().await.probe_whitelist();
        println!("Registered probes: {}", whitelist.len());
        for (key, name) in &whitelist {
            println!("  {} ({}...)", name, &key[..8]);
        }
    }

    // Auth
    let admin_token = auth::load_or_create_token(&auth_token_path);
    if args.verbose { println!("Auth token loaded from {}", auth_token_path.display()); }
    let sessions = auth::new_session_store();

    let clock: std::sync::Arc<dyn clock::Clock> = std::sync::Arc::new(clock::SystemClock);

    // Clone what the background tasks need before pool is moved into state.
    let expiry_pool = pool.clone();
    let expiry_config = std::sync::Arc::clone(&cfg);
    let expiry_args = args.clone();
    let expiry_clock = std::sync::Arc::clone(&clock);
    let monitor_pool = pool.clone();
    let monitor_config = std::sync::Arc::clone(&cfg);

    // Assemble state
    let state = AppState {
        leptos_options: leptos_options.clone(),
        pool,
        hub_secret,
        config: std::sync::Arc::clone(&cfg),
        args: args.clone(),
        sessions,
        admin_token,
        clock,
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
    tokio::spawn(hot_reload::watch_config(std::sync::Arc::clone(&cfg), config_path.clone()));

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
            // Read timeout from config each iteration (hot reloadable)
            let timeout = expiry_config.read().await.server.probe_timeout_secs;
            let now = expiry_clock.now();
            match db::check_expired_probes(&expiry_pool, timeout, now).await {
                Ok(expired) => {
                    if !expired.is_empty() {
                        for (probe_id, company_id) in &expired {
                            if expiry_args.verbose {
                                println!("[EXPIRED] Probe {} (company: {}) has not reported within {}s.",
                                    &probe_id[..8.min(probe_id.len())], company_id, timeout);
                            }
                        }

                        // Evaluate probe expiry alerts
                        let alert_pool = expiry_pool.clone();
                        let alert_config = std::sync::Arc::clone(&expiry_config);
                        let alert_expired = expired.clone();
                        tokio::spawn(async move {
                            if let Err(e) = alerts::evaluate_probe_expiry(&alert_pool, alert_config, &alert_expired, timeout, now).await {
                                eprintln!("[ALERT ERROR] Failed to evaluate probe expiry alerts: {}", e);
                            }
                        });
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Expiry check failed: {}", e);
                }
            }
        }
    });

    // Spawn DB size monitor (checks every 10 minutes)
    archive::spawn_db_size_monitor(monitor_pool, monitor_config).await;

    // Server startup: support HTTP-only, HTTPS-only, or dual-mode
    let (host, port) = {
        let cfg_read = cfg.read().await;
        (cfg_read.server.host.clone(), cfg_read.server.port)
    };

    if let Some((cert_pem, key_pem)) = tls_cert_key {
        let rustls_config = tls::build_rustls_config(&cert_pem, &key_pem).await?;

        if let Some(https_port) = tls_config.https_port {
            // Dual mode: spawn HTTP in background, run HTTPS on main thread
            let http_app = app.clone();
            let http_addr = format!("{}:{}", host, port);
            println!("🌐 Blentinel Hub listening on http://{}", http_addr);

            tokio::spawn(async move {
                let listener = tokio::net::TcpListener::bind(&http_addr).await
                    .unwrap_or_else(|e| panic!("Failed to bind HTTP to {}: {}", http_addr, e));
                axum::serve(listener, http_app.into_make_service()).await
                    .expect("HTTP server error");
            });

            let https_addr = format!("{}:{}", host, https_port);
            println!("🔒 Blentinel Hub listening on https://{}", https_addr);

            axum_server::bind_rustls(https_addr.parse()?, rustls_config)
                .serve(app.into_make_service())
                .await
                .context("HTTPS server error")?;
        } else {
            // HTTPS only
            let https_addr = format!("{}:{}", host, port);
            println!("🔒 Blentinel Hub listening on https://{}", https_addr);

            axum_server::bind_rustls(https_addr.parse()?, rustls_config)
                .serve(app.into_make_service())
                .await
                .context("HTTPS server error")?;
        }
    } else {
        // HTTP only (existing behavior)
        let addr = format!("{}:{}", host, port);
        println!("🌐 Blentinel Hub listening on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await
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
