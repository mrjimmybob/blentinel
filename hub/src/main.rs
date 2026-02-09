use leptos::prelude::*;
use hub_lib::app::{App, shell};

#[cfg(feature = "ssr")]
use axum::{
    routing::{get, post},
    response::IntoResponse,
    extract::State,
    Router,
};

#[cfg(feature = "ssr")]
use x25519_dalek::PublicKey;

#[cfg(feature = "ssr")]
use anyhow::Context;

#[cfg(feature = "ssr")]
mod args;

#[cfg(feature = "ssr")]
mod config;

#[cfg(feature = "ssr")]
mod crypto;

#[cfg(feature = "ssr")]
mod identity;

#[cfg(feature = "ssr")]
mod db;

#[cfg(feature = "ssr")]
mod auth;

#[cfg(feature = "ssr")]
mod api;

#[cfg(feature = "ssr")]
mod hot_reload;

#[cfg(feature = "ssr")]
mod tls;

#[cfg(feature = "ssr")]
mod archive;

#[cfg(feature = "ssr")]
mod archive_viewer;

#[cfg(feature = "ssr")]
mod alerts;

#[cfg(feature = "ssr")]
use leptos::logging::log;

#[cfg(feature = "ssr")]
use leptos_axum::{generate_route_list, LeptosRoutes};

#[cfg(feature = "ssr")]
#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub pool: sqlx::SqlitePool,
    pub hub_secret: x25519_dalek::StaticSecret,
    pub config: std::sync::Arc<tokio::sync::RwLock<config::HubConfig>>,
    pub args: args::Args,
    pub sessions:    auth::SessionStore,
    pub admin_token: String,
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();

    if let Err(e) = run(args).await {
        eprintln!("\n[ERROR] {:#}", e);
        std::process::exit(1);
    }
}

#[cfg(feature = "ssr")]
async fn run(args: args::Args) -> anyhow::Result<()> {
    // Load hub configuration and wrap in Arc<RwLock> for hot reloading
    let cfg = std::sync::Arc::new(tokio::sync::RwLock::new(
        config::load().context("Failed to load hub configuration")?
    ));
    if args.verbose { println!("Configuration loaded from blentinel_hub.toml"); }

    // Leptos needs its own options for the frontend; override site_addr with ours
    let conf = get_configuration(None).unwrap();
    let mut leptos_options = conf.leptos_options;
    {
        let cfg_read = cfg.read().await;
        leptos_options.site_addr = cfg_read.bind_addr().parse()
            .context(format!("Invalid bind address {}:{}", cfg_read.server.host, cfg_read.server.port))?;
    }
    let routes = generate_route_list(App);

    // Database
    let (db_path, identity_key_path, auth_token_path) = {
        let cfg_read = cfg.read().await;
        (
            cfg_read.server.db_path.clone(),
            cfg_read.server.identity_key_path.clone(),
            cfg_read.server.auth_token_path.clone(),
        )
    };

    let db_opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(db_opts)
        .await
        .context(format!("Failed to connect to database: {}", db_path))?;
    db::setup_tables(&pool).await.context("Failed to setup database tables")?;
    if args.verbose { println!("Database initialized: {}", db_path); }

    // Hub identity (persistent X25519 key)
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
    if args.verbose { println!("Auth token loaded from {}", auth_token_path); }
    let sessions = auth::new_session_store();

    // Clone what the background tasks need before pool is moved into state.
    let expiry_pool = pool.clone();
    let expiry_config = std::sync::Arc::clone(&cfg);
    let expiry_args = args.clone();
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

    // Spawn config file watcher
    tokio::spawn(hot_reload::watch_config(std::sync::Arc::clone(&cfg)));

    // Build the router
    let app = Router::new()
        // Probe endpoints (no auth — probes authenticate via crypto)
        .route("/api/handshake", get(handle_handshake))
        .route("/api/report", post(handle_probe_report))
        // Auth
        .route("/api/login",  post(handle_login))
        .route("/api/logout", post(handle_logout))
        // Dashboard & detail (require session)
        .route("/api/dashboard/companies",              get(api::dashboard_companies))
        .route("/api/company/{company_id}/probes",       get(api::company_probes))
        .route("/api/company/{company_id}/uptime",       get(api::company_uptime))
        .route("/api/probe/{probe_id}/devices",          get(api::probe_devices))
        // Admin
        .route("/api/admin/companies",                  get(api::admin_companies))
        .route("/api/admin/probes",                     get(api::admin_probes))
        .route("/api/admin/delete-company-data",        post(api::admin_delete_company_data))
        .route("/api/admin/delete-probe-data",          post(api::admin_delete_probe_data))
        .route("/api/admin/remove-probe",               post(api::admin_remove_probe))
        .route("/api/admin/storage-info",               get(api::admin_storage_info))
        .route("/api/admin/archives",                   get(api::admin_archives))
        .route("/api/admin/archive",                    post(api::admin_create_archive))
        // Alert silence management
        .route("/api/silences",                         get(api::list_silences))
        .route("/api/silence",                          post(api::create_silence))
        .route("/api/silence/delete",                   post(api::delete_silence))
        // Archive viewer (read-only historical access)
        .route("/api/archive/{archive_id}/companies",                    get(archive_viewer::archive_companies))
        .route("/api/archive/{archive_id}/company/{company_id}/probes",  get(archive_viewer::archive_company_probes))
        .route("/api/archive/{archive_id}/company/{company_id}/uptime",  get(archive_viewer::archive_company_uptime))
        .route("/api/archive/{archive_id}/probe/{probe_id}/devices",     get(archive_viewer::archive_probe_devices))
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
            match db::check_expired_probes(&expiry_pool, timeout).await {
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
                            if let Err(e) = alerts::evaluate_probe_expiry(&alert_pool, alert_config, &alert_expired, timeout).await {
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
            if args.verbose {
                println!("\n--- BLENTINEL HUB LISTENING (HTTP) on {} ---", http_addr);
            }
            tokio::spawn(async move {
                let listener = tokio::net::TcpListener::bind(&http_addr).await
                    .expect(&format!("Failed to bind HTTP to {}", http_addr));
                axum::serve(listener, http_app.into_make_service()).await
                    .expect("HTTP server error");
            });

            let https_addr = format!("{}:{}", host, https_port);
            if args.verbose {
                println!("--- BLENTINEL HUB LISTENING (HTTPS) on {} ---", https_addr);
            }
            axum_server::bind_rustls(https_addr.parse()?, rustls_config)
                .serve(app.into_make_service())
                .await
                .context("HTTPS server error")?;
        } else {
            // HTTPS only
            let https_addr = format!("{}:{}", host, port);
            if args.verbose {
                println!("\n--- BLENTINEL HUB LISTENING (HTTPS) on {} ---", https_addr);
            }
            axum_server::bind_rustls(https_addr.parse()?, rustls_config)
                .serve(app.into_make_service())
                .await
                .context("HTTPS server error")?;
        }
    } else {
        // HTTP only (existing behavior)
        let addr = format!("{}:{}", host, port);
        if args.verbose {
            println!("\n--- BLENTINEL HUB LISTENING (HTTP) on {} ---", addr);
        }
        let listener = tokio::net::TcpListener::bind(&addr).await
            .context(format!("Failed to bind to {}", addr))?;
        axum::serve(listener, app.into_make_service())
            .await
            .context("Server error")?;
    }

    Ok(())
}


#[cfg(feature = "ssr")]
async fn handle_login(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let token = std::str::from_utf8(&body)
        .unwrap_or("")
        .trim();

    if token == state.admin_token {
        let session_id = auth::create_session(&state.sessions);
        let cookie = format!(
            "{}={}; HttpOnly; SameSite=Lax; Max-Age=86400; Path=/",
            auth::SESSION_COOKIE_NAME, session_id
        );
        axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header("Set-Cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap()
    } else {
        axum::http::StatusCode::UNAUTHORIZED.into_response()
    }
}

#[cfg(feature = "ssr")]
async fn handle_logout(
    session: auth::AuthSession,
    State(state): State<AppState>,
) -> axum::response::Response {
    auth::destroy_session(&state.sessions, &session.session_id);
    let cookie = format!(
        "{}=; HttpOnly; SameSite=Lax; Max-Age=0; Path=/",
        auth::SESSION_COOKIE_NAME
    );
    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("Set-Cookie", cookie)
        .body(axum::body::Body::empty())
        .unwrap()
}

#[cfg(feature = "ssr")]
async fn handle_handshake(State(state): State<AppState>) -> impl IntoResponse {
    if state.args.debug {
        println!("[DEBUG] Handshake requested. Serving Hub public key.");
    }
    let public_key = PublicKey::from(&state.hub_secret);
    hex::encode(public_key.as_bytes())
}


// The receiver endpoint for probe reports
#[cfg(feature = "ssr")]
async fn handle_probe_report(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    // Decrypt using the Hub's persistent private key
    let decrypted = match crypto::SecureSeal::decrypt_from_probe(&body, &state.hub_secret) {
        Ok(d) => d,
        Err(_e) => {
            log!("Security: Decryption failed. Potential unauthorized probe or corrupted data.");
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };

    if state.args.debug {
        let pretty = serde_json::from_slice::<serde_json::Value>(&decrypted)
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or_else(|_| format!("{:?}", decrypted));
        println!("[DEBUG] Decrypted payload:\n{}", pretty);
    }

    // Deserialize the StatusReport
    let mut report: common::models::StatusReport = match serde_json::from_slice(&decrypted) {
        Ok(r) => r,
        Err(_) => return axum::http::StatusCode::BAD_REQUEST.into_response(),
    };

    // Whitelist check — reject any probe we haven't explicitly registered
    let probe_name = {
        let config_read = state.config.read().await;
        let whitelist = config_read.probe_whitelist();

        if !whitelist.contains_key(&report.probe_id) {
            log!("Security: Report from unregistered probe {}. Rejected.", &report.probe_id[..8.min(report.probe_id.len())]);
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }

        whitelist.get(&report.probe_id).cloned().unwrap_or_else(|| "unknown".to_string())
    };

    // Signature Verification (Probe ID is the Hex of the Probe's Public Key)
    // Verifies the full report: company_id, hostname, site, timestamp, resources, etc.
    if let Some(sig) = report.signature.take() {
        let signable = report.to_signable();
        let signed_data = serde_json::to_vec(&signable).unwrap();
        if let Err(_e) = crypto::SecureSeal::verify(&signed_data, &sig, &report.probe_id) {
            log!("Security Alert: Invalid signature from {}!", report.probe_id);
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
        report.signature = Some(sig);
    }

    if state.args.verbose {
        println!("[{}] Report received from {} ({}) — {} resources",
            report.timestamp.format("%H:%M:%S"),
            &probe_name,
            &report.probe_id[..8],
            report.resources.len());
    }

    if state.args.debug {
        println!("[DEBUG] Saving report: probe={}, company={}, resources={}",
            &report.probe_id[..8], report.company_id, report.resources.len());
    }

    // Save to DB
    match db::save_report(&state.pool, &report).await {
        Ok(_) => {
            // Refresh this probe's heartbeat — marks it active and records now.
            // A failure here does not invalidate the report (already persisted).
            if let Err(e) = db::upsert_heartbeat(&state.pool, &report.probe_id, &report.company_id).await {
                eprintln!("[ERROR] Failed to update heartbeat for {}: {}", &report.probe_id[..8], e);
            }

            // Evaluate alerts for this report
            // Spawn as task so it doesn't block the response
            let alert_pool = state.pool.clone();
            let alert_config = std::sync::Arc::clone(&state.config);
            let alert_report = report.clone();
            tokio::spawn(async move {
                if let Err(e) = alerts::evaluate_alerts_for_report(&alert_pool, alert_config, &alert_report).await {
                    eprintln!("[ALERT ERROR] Failed to evaluate alerts: {}", e);
                }
            });

            if state.args.verbose {
                println!("[{}] Report saved for {} ({}).",
                    report.timestamp.format("%H:%M:%S"),
                    &probe_name,
                    &report.probe_id[..8]);
            }
            axum::http::StatusCode::OK.into_response()
        }
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}


#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
