use std::env;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Configuration file not found at: {0}")]
    NotFound(PathBuf),
    #[error("I/O error reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Invalid(#[from] toml::de::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub agent: AgentConfig,
    #[serde(default)]
    pub resources: Vec<ResourceConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    pub company_id: String,
    pub hub_url: String,
    pub interval: u64,
    pub hub_public_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ResourceConfig {
    pub name: String,
    pub r#type: String,
    pub target: String,
}

pub(crate) fn get_base_dir() -> PathBuf {
    // Returns the directory where the executable is located
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(".")) // Fallback to CWD if all else fails
}

pub fn get_config_path() -> PathBuf {
    get_base_dir().join("blentinel_probe.toml")
}

pub fn load() -> Result<Config, ConfigError> {
    let config_path = get_base_dir().join("blentinel_probe.toml");
    if !config_path.exists() {
        return Err(ConfigError::NotFound(config_path));
    }

    let content = fs::read_to_string(&config_path)?;
    let config: Config = toml::from_str(&content)?;

    if config.agent.hub_url.is_empty() {
        return Err(ConfigError::Validation(
            "agent.hub_url must not be empty".to_string(),
        ));
    }
    if config.agent.interval == 0 {
        return Err(ConfigError::Validation(
            "agent.interval must be greater than 0".to_string(),
        ));
    }
    if config.resources.is_empty() {
        eprintln!("[WARN] No [[resources]] defined. Probe will run but send empty reports.");
    }

    // Validate each resource type
    for r in &config.resources {
        match r.r#type.as_str() {
            "ping" | "http" | "tcp" => {}
            other => {
                return Err(ConfigError::Validation(format!(
                    "Invalid resource type '{}'. Allowed: ping, http, tcp",
                    other
                )));
            }
        }

        if r.name.trim().is_empty() {
            return Err(ConfigError::Validation("resource.name must not be empty".into()));
        }

        if r.target.trim().is_empty() {
            return Err(ConfigError::Validation("resource.target must not be empty".into()));
        }
    }

    Ok(config)
}