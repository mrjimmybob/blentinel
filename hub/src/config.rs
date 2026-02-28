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
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub alerts: AlertsConfig,
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
    #[serde(default = "default_auth_token_path")]
    pub auth_token_path: String,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Directory for all runtime state (database, identity keys, auth tokens).
    ///
    /// **Required.** All relative paths in `[server]` are resolved relative
    /// to this directory. Absolute paths are always used as-is.
    ///
    /// Example values:
    ///   - Production:  `/var/lib/blentinel`
    ///   - Development: `.`  (current working directory)
    pub state_dir: PathBuf,
}

impl ServerConfig {
    /// Resolve `path` against `state_dir`.
    ///
    /// - Absolute paths pass through unchanged.
    /// - Relative paths are joined to `state_dir`.
    pub fn resolve_path(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() { p } else { self.state_dir.join(path) }
    }
}

fn default_identity_key_path() -> String {
    "hub_identity.key".to_string()
}

fn default_probe_timeout_secs() -> u64 {
    300 // 5 minutes
}

fn default_auth_token_path() -> String {
    "hub_auth.token".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cert_path")]
    pub cert_path: String,
    #[serde(default = "default_key_path")]
    pub key_path: String,
    pub https_port: Option<u16>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: default_cert_path(),
            key_path: default_key_path(),
            https_port: None,
        }
    }
}

fn default_cert_path() -> String {
    "hub_tls_cert.pem".to_string()
}

fn default_key_path() -> String {
    "hub_tls_key.pem".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProbeConfig {
    pub name: String,
    pub public_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RetentionConfig {
    #[serde(default = "default_retention_enabled")]
    pub enabled: bool,
    #[serde(default = "default_retention_auto")]
    pub auto: bool,
    #[serde(default = "default_archive_older_than_days")]
    pub archive_older_than_days: u32,
    #[serde(default = "default_warn_db_size_mb")]
    pub warn_db_size_mb: u64,
    #[serde(default = "default_archive_path")]
    pub archive_path: String,
}

fn default_retention_enabled() -> bool { true }
fn default_retention_auto() -> bool { false }
fn default_archive_older_than_days() -> u32 { 90 }
fn default_warn_db_size_mb() -> u64 { 1000 }
fn default_archive_path() -> String { "archives".to_string() }

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            enabled: default_retention_enabled(),
            auto: default_retention_auto(),
            archive_older_than_days: default_archive_older_than_days(),
            warn_db_size_mb: default_warn_db_size_mb(),
            archive_path: default_archive_path(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AlertsConfig {
    #[serde(default = "default_alerts_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub technicians: Vec<Technician>,
    #[serde(default)]
    pub default_recipients: Vec<String>,
    #[serde(default)]
    pub thresholds: ThresholdConfig,
    #[serde(default)]
    pub smtp: SmtpConfig,
    #[serde(default)]
    pub company_overrides: HashMap<String, CompanyAlertOverride>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Technician {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ThresholdConfig {
    #[serde(default = "default_disk_percent")]
    pub disk_percent: u32,
    #[serde(default = "default_cpu_percent")]
    pub cpu_percent: u32,
    #[serde(default = "default_mem_percent")]
    pub mem_percent: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SmtpConfig {
    #[serde(default)]
    pub server: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub from: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CompanyAlertOverride {
    pub alert_emails: Vec<String>,
    pub thresholds: Option<ThresholdConfig>,
}

fn default_alerts_enabled() -> bool { false }
fn default_disk_percent() -> u32 { 90 }
fn default_cpu_percent() -> u32 { 95 }
fn default_mem_percent() -> u32 { 90 }
fn default_smtp_port() -> u16 { 587 }

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            enabled: default_alerts_enabled(),
            technicians: vec![],
            default_recipients: vec![],
            thresholds: ThresholdConfig::default(),
            smtp: SmtpConfig::default(),
            company_overrides: HashMap::new(),
        }
    }
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            disk_percent: default_disk_percent(),
            cpu_percent: default_cpu_percent(),
            mem_percent: default_mem_percent(),
        }
    }
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            port: default_smtp_port(),
            username: String::new(),
            password: String::new(),
            from: String::new(),
        }
    }
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

pub fn get_config_path() -> PathBuf {
    PathBuf::from(CONFIG_FILE)
}

/// Write a fully-commented configuration template to `blentinel_hub.toml`.
///
/// If the file already exists this is a no-op (returns `Ok(())`).
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
# Blentinel Hub Configuration
# ---------------------------------------------------------------------------

[server]
host = "127.0.0.1"
port = 3000

# Required: directory where all runtime state is stored (database, keys, tokens).
# Relative paths for db_path, identity_key_path, and auth_token_path are resolved
# relative to state_dir. Absolute paths always take precedence.
#
# Production example:  state_dir = "/var/lib/blentinel"
# Development example: state_dir = "."
state_dir = "."

db_path = "blentinel.db"
identity_key_path = "hub_identity.key"
auth_token_path = "hub_auth.token"
probe_timeout_secs = 300

# ---------------------------------------------------------------------------
# TLS Configuration (Optional)
# ---------------------------------------------------------------------------
# Uncomment to enable HTTPS
#
# [server.tls]
# enabled = true
# cert_path = "hub_tls_cert.pem"
# key_path = "hub_tls_key.pem"
# https_port = 3443

# ---------------------------------------------------------------------------
# Authorized Probes
# ---------------------------------------------------------------------------
# Each probe must be registered here.
#
# [[probes]]
# name = "SERVER-1"
# public_key = "PUT_32_BYTE_HEX_PUBLIC_KEY_HERE"

# ---------------------------------------------------------------------------
# Retention
# ---------------------------------------------------------------------------
[retention]
enabled = true
auto = false
archive_older_than_days = 90
warn_db_size_mb = 1000
archive_path = "archives"

# ---------------------------------------------------------------------------
# Alerts
# ---------------------------------------------------------------------------
[alerts]
enabled = false
default_recipients = []

# [[alerts.technicians]]
# name = "Alice"
# email = "alice@example.com"
# phone = "+123456789"

[alerts.thresholds]
disk_percent = 90
cpu_percent = 95
mem_percent = 90

[alerts.smtp]
server = ""
port = 587
username = ""
password = ""
from = ""

# [alerts.company_overrides."COMPANY_ID"]
# alert_emails = ["ops@example.com"]
#
# [alerts.company_overrides."COMPANY_ID".thresholds]
# disk_percent = 85
"#;

/// Load configuration from the default path (`blentinel_hub.toml`).
///
/// Convenience wrapper around [`load_from`] for callers that always use
/// the default config location (tests, tooling, etc.).
#[allow(dead_code)]
pub fn load() -> Result<HubConfig, ConfigError> {
    load_from(&PathBuf::from(CONFIG_FILE))
}

/// Load and validate hub configuration from an explicit file path.
///
/// Use this when a custom `--config` flag was provided; otherwise prefer
/// the zero-argument [`load`].
pub fn load_from(config_path: &std::path::Path) -> Result<HubConfig, ConfigError> {
    if !config_path.exists() {
        return Err(ConfigError::NotFound(config_path.to_path_buf()));
    }

    let content = fs::read_to_string(config_path)?;
    let config: HubConfig = toml::from_str(&content)?;

    // --- server section ---
    if config.server.state_dir.as_os_str().is_empty() {
        return Err(ConfigError::Validation(
            "server.state_dir must not be empty. Set it to the runtime state directory (e.g. state_dir = \"/var/lib/blentinel\")".to_string(),
        ));
    }
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
    if config.server.auth_token_path.is_empty() {
        return Err(ConfigError::Validation(
            "server.auth_token_path must not be empty".to_string(),
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

    // Validate retention configuration
    if config.retention.enabled {
        if config.retention.archive_older_than_days == 0 {
            return Err(ConfigError::Validation(
                "retention.archive_older_than_days must be greater than 0".to_string(),
            ));
        }
        if config.retention.warn_db_size_mb == 0 {
            return Err(ConfigError::Validation(
                "retention.warn_db_size_mb must be greater than 0".to_string(),
            ));
        }
        if config.retention.archive_path.is_empty() {
            return Err(ConfigError::Validation(
                "retention.archive_path must not be empty".to_string(),
            ));
        }
    }

    // Validate alerts configuration
    if config.alerts.enabled {
        if config.alerts.smtp.server.is_empty() {
            return Err(ConfigError::Validation(
                "alerts.smtp.server must not be empty when alerts are enabled".to_string(),
            ));
        }
        if config.alerts.smtp.from.is_empty() {
            return Err(ConfigError::Validation(
                "alerts.smtp.from must not be empty when alerts are enabled".to_string(),
            ));
        }
        if config.alerts.default_recipients.is_empty() {
            eprintln!("[WARN] alerts.default_recipients is empty. No one will receive alerts unless company overrides are configured.");
        }
    }

    Ok(config)
}
