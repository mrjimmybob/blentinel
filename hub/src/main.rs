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
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\n[ERROR] {:#}", e);
        std::process::exit(1);
    }
}

#[cfg(feature = "ssr")]
async fn run() -> anyhow::Result<()> {
    // Load hub configuration
    let cfg = config::load().context("Failed to load hub configuration")?;
    println!("Configuration loaded from blentinel_hub.toml");

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
    println!("Database initialized: {}", cfg.server.db_path);

    // Hub identity (persistent X25519 key)
    let hub_secret = identity::load_or_create_hub_key(&cfg.server.identity_key_path);

    // Probe whitelist
    let whitelist = cfg.probe_whitelist();
    println!("Registered probes: {}", whitelist.len());
    for (key, name) in &whitelist {
        println!("  {} ({}...)", name, &key[..8]);
    }

    // Assemble state
    let state = AppState {
        leptos_options: leptos_options.clone(),
        pool,
        hub_secret,
        whitelist,
    };

    // Build the router
    let app = Router::new()
        .route("/api/handshake", get(handle_handshake))
        .route("/api/report", post(handle_probe_report))
        .leptos_routes(&state, routes, move || shell(leptos_options.clone()))
        .with_state(state);

    let addr = cfg.bind_addr();
    println!("\n--- BLENTINEL HUB LISTENING on {} ---", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .context(format!("Failed to bind to {}", addr))?;
    axum::serve(listener, app.into_make_service()).await
        .context("Server error")?;

    Ok(())
}


#[cfg(feature = "ssr")]
async fn handle_handshake(State(state): State<AppState>) -> impl IntoResponse {
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

    // Final step - Save to DB
    match db::save_report(&state.pool, &report).await {
        Ok(_) => axum::http::StatusCode::OK.into_response(),
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}


#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
