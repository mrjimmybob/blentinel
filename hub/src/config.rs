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
    #[serde(default)]
    pub tls: TlsConfig,
    /// The hub runs behind a TLS-terminating reverse proxy: treat requests as
    /// HTTPS (cookies get `Secure`) and take the client IP from the last
    /// `X-Forwarded-For` hop. Trusts the proxy: never set it on a hub that
    /// clients reach directly (D-43).
    #[serde(default)]
    pub behind_proxy: bool,
    /// Directory for all runtime state (database, identity key, setup code).
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
        if p.is_absolute() {
            p
        } else {
            self.state_dir.join(path)
        }
    }

    /// Resolved absolute path to the SQLite database file.
    pub fn resolved_db_path(&self) -> PathBuf {
        self.resolve_path(&self.db_path)
    }

    pub fn resolved_identity_key_path(&self) -> PathBuf {
        self.resolve_path(&self.identity_key_path)
    }
}

fn default_identity_key_path() -> String {
    "hub_identity.key".to_string()
}

fn default_probe_timeout_secs() -> u64 {
    300 // 5 minutes
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    /// On by default (D-03, D-43). `false` serves everything over plain HTTP.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_cert_path")]
    pub cert_path: String,
    #[serde(default = "default_key_path")]
    pub key_path: String,
    /// Where HTTPS listens. If it differs from `port` (the default), `port`
    /// keeps plain HTTP for the probe endpoints only; if equal, HTTPS only.
    #[serde(default = "default_https_port")]
    pub https_port: u16,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cert_path: default_cert_path(),
            key_path: default_key_path(),
            https_port: default_https_port(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_https_port() -> u16 {
    3443
}

fn default_cert_path() -> String {
    "hub_tls_cert.pem".to_string()
}

fn default_key_path() -> String {
    "hub_tls_key.pem".to_string()
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

fn default_retention_enabled() -> bool {
    true
}
fn default_retention_auto() -> bool {
    false
}
fn default_archive_older_than_days() -> u32 {
    90
}
fn default_warn_db_size_mb() -> u64 {
    1000
}
fn default_archive_path() -> String {
    "archives".to_string()
}

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

fn default_alerts_enabled() -> bool {
    false
}
fn default_disk_percent() -> u32 {
    90
}
fn default_cpu_percent() -> u32 {
    95
}
fn default_mem_percent() -> u32 {
    90
}
fn default_smtp_port() -> u16 {
    587
}

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

pub(crate) const DEFAULT_CONFIG_TEMPLATE: &str = r#"# ---------------------------------------------------------------------------
# Blentinel Hub Configuration
# ---------------------------------------------------------------------------

[server]
host = "127.0.0.1"
port = 3000

# Required: directory where all runtime state is stored (database, key, setup code).
# Relative paths for db_path and identity_key_path are resolved relative to
# state_dir. Absolute paths always take precedence.
#
# Production example:  state_dir = "/var/lib/blentinel"
# Development example: state_dir = "."
state_dir = "."

db_path = "blentinel.db"
identity_key_path = "hub_identity.key"
probe_timeout_secs = 300

# ---------------------------------------------------------------------------
# HTTPS (on by default, D-43)
# ---------------------------------------------------------------------------
# The web UI and API are served over HTTPS on https_port, with a self-signed
# certificate created on first start. Probes enrolled with a code pin that
# certificate (its fingerprint is in the code). The plain-HTTP `port` serves
# only the probe endpoints (reports are signed and encrypted anyway) and
# redirects everything else to HTTPS.
#
#   https_port = port      HTTPS only, on `port`
#   enabled = false        plain HTTP for everything (behind a TLS proxy:
#                          also set behind_proxy = true under [server])

[server.tls]
enabled = true
cert_path = "hub_tls_cert.pem"
key_path = "hub_tls_key.pem"
https_port = 3443

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
    if config.server.probe_timeout_secs == 0 {
        return Err(ConfigError::Validation(
            "server.probe_timeout_secs must be greater than 0".to_string(),
        ));
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
            tracing::warn!(
                "alerts.default_recipients is empty. No one will receive alerts unless company overrides are configured."
            );
        }
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str =
        "[server]\nhost = '127.0.0.1'\nport = 3000\nstate_dir = '.'\ndb_path = 'hub.db'\n";

    #[test]
    fn https_is_on_by_default_in_split_mode() {
        let config: HubConfig = toml::from_str(MINIMAL).unwrap();
        assert!(
            config.server.tls.enabled,
            "no [server.tls]: HTTPS on (D-43)"
        );
        assert_eq!(config.server.tls.https_port, 3443);
        assert!(!config.server.behind_proxy);

        let template: HubConfig = toml::from_str(DEFAULT_CONFIG_TEMPLATE).unwrap();
        assert!(template.server.tls.enabled);
        assert_eq!(template.server.tls.https_port, 3443);
    }

    #[test]
    fn https_can_be_turned_off_explicitly() {
        let toml = format!("{MINIMAL}behind_proxy = true\n[server.tls]\nenabled = false\n");
        let config: HubConfig = toml::from_str(&toml).unwrap();
        assert!(!config.server.tls.enabled);
        assert!(config.server.behind_proxy);
    }
}
