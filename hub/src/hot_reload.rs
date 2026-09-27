#![cfg(feature = "ssr")]

use crate::config::{self, HubConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Reload `config_path` whenever it changes. Runs forever (spawn it).
///
/// The explicit path works with both the default location and `--config`.
pub async fn watch_config(config: Arc<RwLock<HubConfig>>, config_path: PathBuf) {
    let path = config_path.clone();
    common::watch::watch_file(config_path, move || {
        let config = Arc::clone(&config);
        let path = path.clone();
        async move { reload(&config, &path).await }
    })
    .await;
}

/// Load the file and swap it in. An invalid file is logged and the previous
/// configuration kept.
async fn reload(config: &RwLock<HubConfig>, path: &Path) {
    tracing::info!("Config file changed, reloading");
    let new_config = match config::load_from(path) {
        Ok(new_config) => new_config,
        Err(e) => {
            tracing::error!("Config reload failed, keeping the previous configuration: {e}");
            return;
        }
    };

    let mut cfg = config.write().await;
    let warnings = restart_required_changes(&cfg, &new_config);
    let whitelist_info = match new_config.probes.len() as i64 - cfg.probes.len() as i64 {
        0 => "probe whitelist unchanged".to_string(),
        added if added > 0 => format!("{added} probe(s) added to the whitelist"),
        removed => format!("{} probe(s) removed from the whitelist", -removed),
    };
    *cfg = new_config;
    drop(cfg);

    tracing::info!("Configuration reloaded: {whitelist_info}");
    for warning in warnings {
        tracing::warn!("{warning}");
    }
}

/// Settings that changed but only take effect after a restart (ROADMAP M0-23
/// makes them reloadable).
fn restart_required_changes(old: &HubConfig, new: &HubConfig) -> Vec<String> {
    let (old, new) = (&old.server, &new.server);
    let mut changes = Vec::new();
    let mut check = |name: &str, before: String, after: String| {
        if before != after {
            changes.push(format!(
                "server.{name} changed from '{before}' to '{after}'; restart the hub to apply it"
            ));
        }
    };
    check("host", old.host.clone(), new.host.clone());
    check("port", old.port.to_string(), new.port.to_string());
    check("db_path", old.db_path.clone(), new.db_path.clone());
    check(
        "identity_key_path",
        old.identity_key_path.clone(),
        new.identity_key_path.clone(),
    );
    check(
        "auth_token_path",
        old.auth_token_path.clone(),
        new.auth_token_path.clone(),
    );
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_restart_settings_are_reported() {
        // The --init template: a complete, valid config.
        let old: HubConfig = toml::from_str(config::DEFAULT_CONFIG_TEMPLATE).unwrap();
        assert!(restart_required_changes(&old, &old.clone()).is_empty());

        let mut new = old.clone();
        new.server.port = old.server.port + 1;
        new.probes.clear(); // the whitelist reloads live: not reported
        let changes = restart_required_changes(&old, &new);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(changes[0].starts_with("server.port changed"), "{changes:?}");
    }
}
