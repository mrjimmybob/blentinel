//! Hub entry point: CLI handling, startup wiring, background tasks, and the
//! listeners. All request handling lives in the library (`hub_lib`).
// The UI's view types are deeply nested (as in lib.rs).
#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
use hub_lib::{
    AppState, args, clock, config, hot_reload, identity, logging, routes, server,
    service::{
        self,
        alerts::{Notifier, SmtpNotifier},
    },
    state::Reloadable,
    storage, user_cli,
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

    // --print-public-key: load config for path resolution, print, and exit
    // without starting the server.
    if args.print_public_key {
        let cfg = load_config_or_exit(&config_path);
        let identity_key_path = cfg.server.resolve_path(&cfg.server.identity_key_path);
        let pub_key_path = identity_key_path.with_extension("pub");
        if let Err(e) = identity::print_and_write_public_key(&identity_key_path, &pub_key_path) {
            eprintln!("[ERROR] {e:#}");
            std::process::exit(1);
        }
        return;
    }

    // hub user …: account recovery on the server, then exit.
    if let Some(command) = &args.user {
        let cfg = load_config_or_exit(&config_path);
        let db_path = cfg.server.resolved_db_path();
        let result = async {
            let store = storage::open(&db_path).await?;
            user_cli::run(store.as_ref(), command, chrono::Utc::now())
                .await
                .map_err(anyhow::Error::from)
        };
        match result.await {
            Ok(output) => print!("{output}"),
            Err(e) => {
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

/// Load the config for a one-off command, or print why not and exit.
#[cfg(feature = "ssr")]
fn load_config_or_exit(path: &std::path::Path) -> config::HubConfig {
    config::load_from(path).unwrap_or_else(|e| {
        eprintln!("[ERROR] Failed to load configuration: {e}");
        std::process::exit(1);
    })
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

    // Runtime state lives in state_dir (created on first run); relative paths
    // in [server] are anchored there.
    let server = cfg.read().await.server.clone();
    std::fs::create_dir_all(&server.state_dir).context(format!(
        "Failed to create state directory '{}'. Check permissions or create it manually.",
        server.state_dir.display()
    ))?;

    let db_path = server.resolved_db_path();
    let storage = storage::open(&db_path).await.context(format!(
        "Failed to open database at '{}'",
        db_path.display()
    ))?;
    tracing::info!("Database ready: {}", db_path.display());

    // Hub identity (persistent X25519 key); the .pub file is written on creation.
    let hub_secret = identity::load_or_create_hub_key(&server.resolved_identity_key_path())?;

    let clock: std::sync::Arc<dyn clock::Clock> = std::sync::Arc::new(clock::SystemClock);
    let notifier: std::sync::Arc<dyn Notifier> =
        std::sync::Arc::new(SmtpNotifier::new(std::sync::Arc::clone(&cfg)));

    let state = AppState {
        leptos_options: leptos_options.clone(),
        storage: Reloadable::new(storage),
        hub_secret: Reloadable::new(hub_secret),
        config: std::sync::Arc::clone(&cfg),
        args: args.clone(),
        setup: std::sync::Arc::new(service::setup::SetupCode::default()),
        login_limiter: std::sync::Arc::default(),
        clock,
        notifier,
        listeners_changed: std::sync::Arc::new(tokio::sync::Notify::new()),
    };

    // No users yet: create the one-time setup code for the first Owner.
    state
        .setup
        .ensure(state.storage.get().as_ref(), &server.state_dir)
        .await
        .context("Failed to create the setup code")?;

    // Every setting in the config file applies without a restart (D-38).
    tokio::spawn(hot_reload::watch_config(state.clone(), config_path.clone()));

    // Background task: periodically scan for probes that have gone silent.
    // Sleeps first so a hub restart does not instantly expire everyone.
    let sweeper = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            let storage = sweeper.storage.get(); // the current database (D-38)
            let sweep = service::liveness::sweep_expired_probes(
                &storage,
                &sweeper.notifier,
                &sweeper.config,
                sweeper.clock.now(),
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

    // Remove login sessions that ended (hourly).
    let sessions = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            let storage = sessions.storage.get();
            if let Err(e) =
                service::users::delete_dead_sessions(storage.as_ref(), sessions.clock.now()).await
            {
                tracing::error!("Session cleanup failed: {e:#}");
            }
        }
    });

    // DB size monitor (checks every 10 minutes).
    service::retention::spawn_size_monitor(
        state.storage.clone(),
        std::sync::Arc::clone(&state.config),
        std::sync::Arc::clone(&state.clock),
    );

    // The router: the API (hub_lib::routes) plus the Leptos UI on top.
    let listeners_changed = std::sync::Arc::clone(&state.listeners_changed);
    let app = routes::api_router(&state)
        .leptos_routes(&state, routes, move || shell(leptos_options.clone()))
        .fallback(leptos_axum::file_and_error_handler::<AppState, _>(shell))
        .with_state(state);

    // HTTP, HTTPS or both, as [server] says; moved when a reload changes it.
    server::serve(app, cfg, listeners_changed).await
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
