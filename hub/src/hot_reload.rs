#![cfg(feature = "ssr")]
//! Config hot reload: every setting applies without a restart (D-38).
//!
//! A reload is all-or-nothing: the new file is validated and everything it
//! needs (database, hub key, listening address) is prepared
//! first. Only if all of that works are the old ones replaced.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use x25519_dalek::StaticSecret;

use crate::config::{self, HubConfig};
use crate::identity;
use crate::server::{self, Listen};
use crate::state::AppState;
use crate::storage::{self, Storage};

/// Reload `config_path` whenever it changes. Runs forever (spawn it).
pub async fn watch_config(state: AppState, config_path: PathBuf) {
    let path = config_path.clone();
    common::watch::watch_file(config_path, move || {
        let state = state.clone();
        let path = path.clone();
        async move {
            if let Err(e) = reload(&state, &path).await {
                tracing::error!("Config reload failed; nothing changed: {e:#}");
            }
        }
    })
    .await;
}

/// Load `path` and apply it to the running hub, or change nothing on error.
pub async fn reload(state: &AppState, path: &Path) -> Result<()> {
    tracing::info!("Config file changed, reloading");
    let new = config::load_from(path)?;
    let old = state.config.read().await.clone();
    let prepared = prepare(&old, &new).await?;

    // Commit: nothing below can fail.
    let mut applied = prepared.applied;
    if let Some(storage) = prepared.storage {
        // Users live in the database: an empty one needs the setup wizard.
        if let Err(e) = state
            .setup
            .ensure(storage.as_ref(), &new.server.state_dir)
            .await
        {
            tracing::error!("Could not create a setup code for the new database: {e:#}");
        }
        state.storage.set(storage);
    }
    if let Some(secret) = prepared.hub_secret {
        state.hub_secret.set(secret);
    }
    applied.push(whitelist_change(&old, &new));
    let listeners_changed = Listen::from_config(&old.server) != Listen::from_config(&new.server);
    *state.config.write().await = new;
    if listeners_changed {
        state.listeners_changed.notify_one();
    }

    tracing::info!("Configuration reloaded: {}", applied.join("; "));
    Ok(())
}

/// What a new config needs, made ready without touching the running hub.
struct Prepared {
    storage: Option<Arc<dyn Storage>>,
    hub_secret: Option<StaticSecret>,
    /// Human-readable list of what changes, for the log.
    applied: Vec<String>,
}

async fn prepare(old: &HubConfig, new: &HubConfig) -> Result<Prepared> {
    let (before, after) = (&old.server, &new.server);
    let mut prepared = Prepared {
        storage: None,
        hub_secret: None,
        applied: Vec::new(),
    };

    if after.state_dir != before.state_dir {
        std::fs::create_dir_all(&after.state_dir)
            .with_context(|| format!("can't create state_dir {}", after.state_dir.display()))?;
    }

    let db_path = after.resolved_db_path();
    if db_path != before.resolved_db_path() {
        prepared.storage = Some(
            storage::open(&db_path)
                .await
                .with_context(|| format!("can't open the database {}", db_path.display()))?,
        );
        prepared
            .applied
            .push(format!("database is now {}", db_path.display()));
    }

    let key_path = after.resolved_identity_key_path();
    if key_path != before.resolved_identity_key_path() {
        prepared.hub_secret = Some(identity::load_or_create_hub_key(&key_path)?);
        prepared.applied.push(format!(
            "hub key is now {} (probes using the handshake need the new key)",
            key_path.display()
        ));
    }

    let (listen_before, listen_after) = (Listen::from_config(before), Listen::from_config(after));
    if listen_after != listen_before {
        server::check_bindable(&listen_before, &listen_after).await?;
        prepared.applied.push("listeners restarting".to_string());
    }

    Ok(prepared)
}

fn whitelist_change(old: &HubConfig, new: &HubConfig) -> String {
    match new.probes.len() as i64 - old.probes.len() as i64 {
        0 => "probe whitelist unchanged".to_string(),
        added if added > 0 => format!("{added} probe(s) added to the whitelist"),
        removed => format!("{} probe(s) removed from the whitelist", -removed),
    }
}
