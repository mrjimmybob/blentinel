//! Hub entry point: CLI handling, startup wiring, background tasks, and the
//! listeners. All request handling lives in the library (`hub_lib`).
// The UI's view types are deeply nested (as in lib.rs).
#![recursion_limit = "512"]

#[cfg(all(feature = "ssr", windows))]
mod windows_service;

#[cfg(feature = "ssr")]
use hub_lib::{
    AppState, args, clock, config, hot_reload, identity, logging, ops, routes, server,
    service::{self, alerts::Notifier},
    state::Reloadable,
    user_cli, web_assets,
};

#[cfg(feature = "ssr")]
fn main() {
    // --version and --help exit inside parse(); no async context needed.
    let args = args::parse();

    // Started by the Windows Service Control Manager (`hub install`, D-52):
    // it runs the hub until the service stops. From a terminal this returns
    // at once, and the hub runs as a normal program.
    #[cfg(windows)]
    if is_server_start(&args) && windows_service::run_if_service(&args, &config_path(&args)) {
        return;
    }

    logging::init(&args);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime");
    runtime.block_on(cli_main(args));
}

/// Without a command: start the server (not `--init`, `user`, `install`, …).
#[cfg(all(feature = "ssr", windows))]
fn is_server_start(args: &args::Args) -> bool {
    !args.init && !args.print_public_key && args.user.is_none() && args.ops.is_none()
}

/// `--config`, else `blentinel_hub.toml` in the working directory, else the
/// installed hub's (D-52).
#[cfg(feature = "ssr")]
fn config_path(args: &args::Args) -> std::path::PathBuf {
    args.config_path.clone().unwrap_or_else(|| {
        let working_dir = std::env::current_dir().unwrap_or_default();
        ops::layout::default_config_path(&working_dir, &ops::layout::Layout::system().config)
    })
}

#[cfg(feature = "ssr")]
async fn cli_main(args: args::Args) {
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
    let config_path = config_path(&args);

    // hub install, status, backup, …: server administration, then exit.
    if let Some(command) = &args.ops {
        std::process::exit(ops::run(command, &config_path).await);
    }

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
        let result = async {
            let store = ops::upgrade::open_database(&cfg, &config_path, chrono::Utc::now()).await?;
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

/// Run the hub server until it fails (or the process ends).
#[cfg(feature = "ssr")]
pub(crate) async fn run(args: args::Args, config_path: std::path::PathBuf) -> anyhow::Result<()> {
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
    let conf = get_configuration(None).context("Failed to read Leptos configuration")?;
    let mut leptos_options = conf.leptos_options;
    if web_assets::embedded() {
        tracing::info!("Web UI: embedded in the binary");
    } else {
        // A development build (cargo leptos watch): the files on disk.
        tracing::info!(site_root = %leptos_options.site_root, "Web UI: served from disk");
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
    let storage = {
        let config = cfg.read().await;
        ops::upgrade::open_database(&config, &config_path, chrono::Utc::now())
            .await
            .context(format!(
                "Failed to open database at '{}'",
                db_path.display()
            ))?
    };
    tracing::info!("Database ready: {}", db_path.display());

    // A key rotation's switch cut short by a stop is finished first (D-61).
    {
        let config = cfg.read().await;
        service::rotation::recover(storage.as_ref(), &config, chrono::Utc::now())
            .await
            .context("Failed to finish an interrupted key rotation")?;
    }

    // Hub identity (persistent X25519 key); the .pub file is written on creation.
    let hub_secret = identity::load_or_create_hub_key(&server.resolved_identity_key_path())?;

    // The certificate exists before probes are told its pin (the listener
    // would otherwise create it later).
    if server.tls.enabled {
        hub_lib::tls::load_or_create_tls_cert(
            &server.resolved_tls_cert_path().to_string_lossy(),
            &server.resolved_tls_key_path().to_string_lossy(),
            &server.host,
        )?;
    }
    let credentials = {
        let config = cfg.read().await;
        service::rotation::credentials(storage.as_ref(), &config, &hub_secret)
            .await
            .context("Failed to read the hub's keys")?
    };

    let clock: std::sync::Arc<dyn clock::Clock> = std::sync::Arc::new(clock::SystemClock);
    // Alerts go where the routing rules say; channel secrets are sealed with
    // the data key (D-66).
    let data_key = {
        let config = cfg.read().await;
        hub_lib::secrets::DataKey::load_or_create(&hub_lib::keys::KeyFiles::of(&config).data_key)?
    };
    let delivery = std::sync::Arc::new(service::notify::Delivery {
        key: std::sync::Arc::new(data_key),
        transport: std::sync::Arc::new(service::notify::transport::NetTransport::new()?),
        clock: std::sync::Arc::clone(&clock),
        retries: service::notify::RETRIES.to_vec(),
    });
    let storage = Reloadable::new(storage);
    let notifier: std::sync::Arc<dyn Notifier> = std::sync::Arc::new(
        service::notify::Dispatcher::new(storage.clone(), std::sync::Arc::clone(&delivery)),
    );

    let state = AppState {
        leptos_options: leptos_options.clone(),
        storage,
        hub_secret: Reloadable::new(hub_secret),
        config: std::sync::Arc::clone(&cfg),
        args: args.clone(),
        setup: std::sync::Arc::new(service::setup::SetupCode::default()),
        login_limiter: std::sync::Arc::default(),
        clock,
        notifier,
        delivery,
        listeners_changed: std::sync::Arc::new(tokio::sync::Notify::new()),
        release_keys: std::sync::Arc::new(common::release::trusted_keys()),
        hub_pins: Reloadable::new(credentials),
        tls: hub_lib::tls::TlsHandle::default(),
        acme: std::sync::Arc::new(hub_lib::acme::Acme::new(std::sync::Arc::new(
            service::acme::AcmeClient,
        ))),
        backup_downloads: std::sync::Arc::default(),
        config_path: config_path.clone(),
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

    // Key rotation: switch once every probe is ready, flag it when overdue (D-61).
    hub_lib::rotation::spawn_checks(state.clone());

    // The certificate from Let's Encrypt, renewed when due (D-62).
    hub_lib::acme::spawn_renewals(state.clone());

    // The daily backup, as set on the admin page (D-60).
    service::backups::spawn_scheduler(
        state.storage.clone(),
        std::sync::Arc::clone(&state.config),
        config_path.clone(),
        std::sync::Arc::clone(&state.clock),
    );

    // The router: the API (hub_lib::routes) plus the Leptos UI on top.
    let listeners_changed = std::sync::Arc::clone(&state.listeners_changed);
    let tls_handle = state.tls.clone();
    // Anything else: an embedded web asset, else a file from the site root on
    // disk (development builds) or the 404 page.
    let disk_or_404 = leptos_axum::file_and_error_handler::<AppState, _>(shell);
    let fallback = move |uri: axum::http::Uri,
                         state: axum::extract::State<AppState>,
                         request: axum::extract::Request| {
        let disk_or_404 = disk_or_404.clone();
        async move {
            match web_assets::serve(uri.path(), request.headers()) {
                Some(response) => response,
                None => disk_or_404(uri, state, request).await,
            }
        }
    };
    let app = routes::api_router(&state)
        .leptos_routes(&state, routes, move || shell(leptos_options.clone()))
        .fallback(fallback)
        .with_state(state);

    // HTTP, HTTPS or both, as [server] says; moved when a reload changes it.
    server::serve(app, cfg, listeners_changed, tls_handle).await
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
