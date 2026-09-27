use crate::config::{self, Config};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

/// Reload the config file whenever it changes. Runs forever (spawn it); an
/// invalid file is logged and the previous configuration kept.
pub async fn watch_config(config: Arc<RwLock<Config>>) {
    common::watch::watch_file(config::get_config_path(), move || {
        let config = Arc::clone(&config);
        async move { reload(&config).await }
    })
    .await;
}

async fn reload(config: &RwLock<Config>) {
    info!("Config file changed, reloading");
    match config::load() {
        Ok(new_config) => {
            let mut cfg = config.write().await;
            *cfg = new_config;
            info!(
                hub_url = %cfg.agent.hub_url,
                interval_secs = cfg.agent.interval,
                resources = cfg.resources.len(),
                "Configuration reloaded successfully"
            );
        }
        Err(e) => error!(
            "Failed to reload config: {}. Keeping previous configuration",
            e
        ),
    }
}
