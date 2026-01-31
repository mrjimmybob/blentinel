#![cfg(feature = "ssr")]
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct HubConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub probes: Vec<ProbeConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub db_path: String,
    #[serde(default = "default_identity_key_path")]
    pub identity_key_path: String,
    /// Seconds of silence before a probe is marked expired.
    /// Should be set to at least 2-3× the longest probe interval.
    #[serde(default = "default_probe_timeout_secs")]
    pub probe_timeout_secs: u64,
}

fn default_identity_key_path() -> String {
    "hub_identity.key".to_string()
}

fn default_probe_timeout_secs() -> u64 {
    300 // 5 minutes
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProbeConfig {
    pub name: String,
    pub public_key: String,
}

// ---------------------------------------------------------------------------
// Derived helpers
// ---------------------------------------------------------------------------

impl HubConfig {
    /// Returns a map of public_key (hex) -> name for O(1) whitelist lookups.
    pub fn probe_whitelist(&self) -> HashMap<String, String> {
        self.probes
            .iter()
            .map(|p| (p.public_key.clone(), p.name.clone()))
            .collect()
    }

    /// The address string the server binds to.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}

// ---------------------------------------------------------------------------
// Loader & validation
// ---------------------------------------------------------------------------

const CONFIG_FILE: &str = "blentinel_hub.toml";

pub fn load() -> Result<HubConfig, ConfigError> {
    let config_path = PathBuf::from(CONFIG_FILE);
    if !config_path.exists() {
        return Err(ConfigError::NotFound(config_path));
    }

    let content = fs::read_to_string(&config_path)?;
    let config: HubConfig = toml::from_str(&content)?;

    // --- server section ---
    if config.server.host.is_empty() {
        return Err(ConfigError::Validation(
            "server.host must not be empty".to_string(),
        ));
    }
    if config.server.db_path.is_empty() {
        return Err(ConfigError::Validation(
            "server.db_path must not be empty".to_string(),
        ));
    }
    if config.server.identity_key_path.is_empty() {
        return Err(ConfigError::Validation(
            "server.identity_key_path must not be empty".to_string(),
        ));
    }
    if config.server.probe_timeout_secs == 0 {
        return Err(ConfigError::Validation(
            "server.probe_timeout_secs must be greater than 0".to_string(),
        ));
    }

    // --- probes section ---
    for (i, probe) in config.probes.iter().enumerate() {
        if probe.name.is_empty() {
            return Err(ConfigError::Validation(format!(
                "probes[{}].name must not be empty",
                i
            )));
        }
        if probe.public_key.is_empty() {
            return Err(ConfigError::Validation(format!(
                "probes[{}].public_key must not be empty (probe '{}')",
                i, probe.name
            )));
        }

        // Must be valid hex that decodes to exactly 32 bytes (Ed25519 public key)
        match hex::decode(&probe.public_key) {
            Ok(bytes) if bytes.len() == 32 => {}
            Ok(bytes) => {
                return Err(ConfigError::Validation(format!(
                    "probes[{}] '{}': public_key must be 32 bytes, got {}",
                    i,
                    probe.name,
                    bytes.len()
                )));
            }
            Err(e) => {
                return Err(ConfigError::Validation(format!(
                    "probes[{}] '{}': public_key is not valid hex: {}",
                    i, probe.name, e
                )));
            }
        }
    }

    if config.probes.is_empty() {
        eprintln!(
            "[WARN] No [[probes]] defined. No probes will be accepted until registered."
        );
    }

    Ok(config)
}
