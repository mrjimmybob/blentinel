use leptos::prelude::*;
use hub::app::{App, shell};

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
use leptos::logging::log;

#[cfg(feature = "ssr")]
use leptos_axum::{generate_route_list, LeptosRoutes};

#[cfg(feature = "ssr")]
#[derive(Clone, axum::extract::FromRef)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub pool: sqlx::SqlitePool,
    pub hub_secret: x25519_dalek::StaticSecret,
    /// Allowed probes: Ed25519 public key (hex) → configured name
    pub whitelist: std::collections::HashMap<String, String>,
    pub args: args::Args,
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
    // Load hub configuration
    let cfg = config::load().context("Failed to load hub configuration")?;
    if args.verbose { println!("Configuration loaded from blentinel_hub.toml"); }

    // Leptos needs its own options for the frontend; override site_addr with ours
    let conf = get_configuration(None).unwrap();
    let mut leptos_options = conf.leptos_options;
    leptos_options.site_addr = cfg.bind_addr().parse()
        .context(format!("Invalid bind address {}:{}", cfg.server.host, cfg.server.port))?;
    let routes = generate_route_list(App);

    // Database
    let db_opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&cfg.server.db_path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(db_opts)
        .await
        .context(format!("Failed to connect to database: {}", cfg.server.db_path))?;
    db::setup_tables(&pool).await.context("Failed to setup database tables")?;
    if args.verbose { println!("Database initialized: {}", cfg.server.db_path); }

    // Hub identity (persistent X25519 key)
    let hub_secret = identity::load_or_create_hub_key(&cfg.server.identity_key_path);

    // Probe whitelist
    let whitelist = cfg.probe_whitelist();
    if args.verbose {
        println!("Registered probes: {}", whitelist.len());
        for (key, name) in &whitelist {
            println!("  {} ({}...)", name, &key[..8]);
        }
    }

    // Clone what the background expiry checker needs before pool is moved into state.
    let expiry_pool    = pool.clone();
    let expiry_timeout = cfg.server.probe_timeout_secs;
    let expiry_args    = args.clone();

    // Assemble state
    let state = AppState {
        leptos_options: leptos_options.clone(),
        pool,
        hub_secret,
        whitelist,
        args: args.clone(),
    };

    // Build the router
    let app = Router::new()
        .route("/api/handshake", get(handle_handshake))
        .route("/api/report", post(handle_probe_report))
        .leptos_routes(&state, routes, move || shell(leptos_options.clone()))
        .with_state(state);

    let addr = cfg.bind_addr();
    if args.verbose { println!("\n--- BLENTINEL HUB LISTENING on {} ---", addr); }
    let listener = tokio::net::TcpListener::bind(&addr).await
        .context(format!("Failed to bind to {}", addr))?;

    // Background task: periodically scan for probes that have gone silent.
    // Sleeps first so a hub restart does not instantly expire everyone.
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            match db::check_expired_probes(&expiry_pool, expiry_timeout).await {
                Ok(expired) => {
                    for (probe_id, company_id) in &expired {
                        if expiry_args.verbose {
                            println!("[EXPIRED] Probe {} (company: {}) has not reported within {}s.",
                                &probe_id[..8.min(probe_id.len())], company_id, expiry_timeout);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Expiry check failed: {}", e);
                }
            }
        }
    });

    axum::serve(listener, app.into_make_service()).await
        .context("Server error")?;

    Ok(())
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
    if !state.whitelist.contains_key(&report.probe_id) {
        log!("Security: Report from unregistered probe {}. Rejected.", &report.probe_id[..8.min(report.probe_id.len())]);
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }

    // Signature Verification (Probe ID is the Hex of the Probe's Public Key)
    if let Some(sig) = report.signature.take() {
        let signed_data = serde_json::to_vec(&report.resources).unwrap();
        if let Err(_e) = crypto::SecureSeal::verify(&signed_data, &sig, &report.probe_id) {
            log!("Security Alert: Invalid signature from {}!", report.probe_id);
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
        report.signature = Some(sig);
    }

    let probe_name = state.whitelist.get(&report.probe_id)
        .map(|s| s.as_str())
        .unwrap_or("unknown");

    if state.args.verbose {
        println!("[{}] Report received from {} ({}) — {} resources",
            report.timestamp.format("%H:%M:%S"),
            probe_name,
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

            if state.args.verbose {
                println!("[{}] Report saved for {} ({}).",
                    report.timestamp.format("%H:%M:%S"),
                    probe_name,
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
