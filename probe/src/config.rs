use common::models::ResourceType;
use serde::Deserialize;
use std::env;
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
    #[serde(default = "default_site")]
    pub site: String,
}

fn default_site() -> String {
    "main".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct ResourceConfig {
    pub name: String,
    pub r#type: ResourceType,
    pub target: String,
}

// pub(crate) fn get_base_dir() -> PathBuf {
//     env::current_exe()
//         .ok()
//         .and_then(|p| p.parent().map(|p| p.to_path_buf()))
//         .unwrap_or_else(|| PathBuf::from(".")) // Fallback to CWD if all else fails
// }

// Returns the directory where the executable is located
pub(crate) fn get_base_dir() -> PathBuf {
    env::current_exe()
        .expect("Failed to determine executable path")
        .parent()
        .expect("Executable has no parent directory")
        .to_path_buf()
}

pub fn get_config_path() -> PathBuf {
    get_base_dir().join("blentinel_probe.toml")
}

/// Write a fully-commented configuration template to `blentinel_probe.toml`.
///
/// The file is written into `get_base_dir()` (next to the executable).
/// If the file already exists this is a no-op (returns `Ok(false)`).
/// The caller is responsible for printing status messages.
pub fn create_default_config_file() -> Result<bool, ConfigError> {
    let path = get_config_path();
    if path.exists() {
        return Ok(false);
    }

    fs::write(&path, DEFAULT_CONFIG_TEMPLATE)?;
    Ok(true)
}

const DEFAULT_CONFIG_TEMPLATE: &str = r#"# ---------------------------------------------------------------------------
# Blentinel Probe Configuration
# ---------------------------------------------------------------------------

[agent]
# Unique company identifier (must match hub expectation)
company_id = "COMPANY_NAME"

# Hub URL (HTTP or HTTPS)
hub_url = "http://127.0.0.1:3000"

# Reporting interval in seconds
interval = 60

# Optional: Hub Ed25519 public key (hex, 32 bytes)
# If set, probe verifies hub signature (recommended in production)
# hub_public_key = "PUT_32_BYTE_HEX_PUBLIC_KEY_HERE"

# Logical site name (e.g., main, datacenter-1, branch-office)
site = "main"


# ---------------------------------------------------------------------------
# Monitored Resources
# ---------------------------------------------------------------------------

# ---- Ping (ICMP) ----
[[resources]]
name = "Router"
type = "ping"
target = "192.168.1.1"

# ---- HTTP ----
[[resources]]
name = "Company Website"
type = "http"
target = "https://example.com"

# ---- TCP ----
[[resources]]
name = "Database"
type = "tcp"
target = "192.168.1.50:5432"

# ---- Local Disk ----
[[resources]]
name = "System Disk"
type = "local_disk"
target = "/"
"#;

pub fn load() -> Result<Config, ConfigError> {
    let config_path = get_base_dir().join("blentinel_probe.toml");
    if !config_path.exists() {
        return Err(ConfigError::NotFound(config_path));
    }
    parse(&fs::read_to_string(&config_path)?)
}

/// Parse and validate a probe configuration (the checks `load` applies).
pub fn parse(content: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(content)?;

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
        if r.name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "resource.name must not be empty".into(),
            ));
        }

        if r.target.trim().is_empty() {
            return Err(ConfigError::Validation(
                "resource.target must not be empty".into(),
            ));
        }
    }

    Ok(config)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_init_template_is_a_valid_config() {
        parse(DEFAULT_CONFIG_TEMPLATE).expect("`probe --init` template must parse and validate");
    }

    #[test]
    fn repository_example_config_is_valid() {
        let example = include_str!("../blentinel_probe.example.toml");
        let config = parse(example).expect("probe/blentinel_probe.example.toml must stay valid");
        assert!(!config.resources.is_empty());
    }

    #[test]
    fn empty_hub_url_is_rejected() {
        let invalid = "[agent]\ncompany_id = \"c\"\nhub_url = \"\"\ninterval = 60\n";
        assert!(matches!(parse(invalid), Err(ConfigError::Validation(_))));
    }
}
