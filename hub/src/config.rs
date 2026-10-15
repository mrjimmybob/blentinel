#![cfg(feature = "ssr")]
use serde::Deserialize;

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
    pub backups: BackupsConfig,
    #[serde(default)]
    pub upgrades: UpgradesConfig,
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

    /// The TLS certificate, resolved like the other files (D-61).
    pub fn resolved_tls_cert_path(&self) -> PathBuf {
        self.resolve_path(&self.tls.cert_path)
    }

    pub fn resolved_tls_key_path(&self) -> PathBuf {
        self.resolve_path(&self.tls.key_path)
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

/// Where backups go (M2-12, D-60). When and how many is set on the admin
/// page; only the folder is a bootstrap setting.
#[derive(Debug, Deserialize, Clone)]
pub struct BackupsConfig {
    /// Folder for backups, relative to `state_dir` unless absolute. Point it
    /// at another disk (or a mounted share) so a backup outlives the server.
    #[serde(default = "default_backups_path")]
    pub path: String,
}

fn default_backups_path() -> String {
    "backups".to_string()
}

impl Default for BackupsConfig {
    fn default() -> Self {
        Self {
            path: default_backups_path(),
        }
    }
}

/// Where hub releases come from (M4-04, D-75): for `hub upgrade` and the
/// admin page's "new version" check (which is off until turned on there).
#[derive(Debug, Deserialize, Clone)]
pub struct UpgradesConfig {
    /// A folder of release files: an `https://` URL (GitHub's latest release
    /// by default), or a folder on this server (an offline copy, a mirror).
    /// Only releases signed with a release key this hub trusts are used.
    #[serde(default = "default_release_url")]
    pub release_url: String,
}

fn default_release_url() -> String {
    common::hub_release::DEFAULT_RELEASE_URL.to_string()
}

impl Default for UpgradesConfig {
    fn default() -> Self {
        Self {
            release_url: default_release_url(),
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

    /// The folder backups are written to (`[backups] path`, resolved).
    pub fn backups_dir(&self) -> PathBuf {
        self.server.resolve_path(&self.backups.path)
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

/// Where `hub install` puts a new hub: listen on `host` (all interfaces by
/// default, so probes on other machines reach it), plain HTTP on `port`, the
/// web UI on HTTPS `https_port`, state in `state_dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSettings {
    pub host: String,
    pub port: u16,
    pub https_port: u16,
    pub state_dir: PathBuf,
}

/// The config `hub install` writes on a first install: the `--init` template
/// with these settings, so both stay one definition.
pub fn install_config(settings: &InstallSettings) -> String {
    let state_dir = settings.state_dir.to_string_lossy().replace('\\', "/");
    let mut replaced = std::collections::HashSet::new();
    let mut out = String::with_capacity(DEFAULT_CONFIG_TEMPLATE.len());
    for line in DEFAULT_CONFIG_TEMPLATE.lines() {
        let key = line.split(" = ").next().unwrap_or_default();
        // Only the first of each ([server] comes first).
        let value = match key {
            "host" => Some(format!("\"{}\"", settings.host)),
            "port" => Some(settings.port.to_string()),
            "https_port" => Some(settings.https_port.to_string()),
            "state_dir" => Some(format!("\"{state_dir}\"")),
            _ => None,
        };
        match value {
            Some(value) if replaced.insert(key) => out.push_str(&format!("{key} = {value}")),
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
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
# Backups
# ---------------------------------------------------------------------------
# When backups run and how many are kept is set on the admin page (daily by
# default). Each one holds the hub's private key: put this folder on another
# disk, or copy its files off the server, so a backup outlives the server.
[backups]
path = "backups"

# ---------------------------------------------------------------------------
# Upgrades
# ---------------------------------------------------------------------------
# Where `hub upgrade` (and the admin page's "new version" check, off unless
# turned on there) find hub releases: a URL, or a folder on this server.
# Releases not signed with a release key this hub trusts are refused.
[upgrades]
release_url = "https://github.com/mrjimmybob/blentinel/releases/latest/download"

# Alerts (the mail server, channels, who gets what) and thresholds are set
# in the web UI: the Alerts page, and the admin and company pages (D-64, D-66).
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
    fn an_install_config_is_the_template_with_its_settings() {
        let settings = InstallSettings {
            host: "0.0.0.0".into(),
            port: 8080,
            https_port: 8443,
            state_dir: PathBuf::from(r"C:\ProgramData\Blentinel\Hub"),
        };
        let text = install_config(&settings);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub.toml");
        std::fs::write(&path, &text).unwrap();
        let config = load_from(&path).expect("an install config is valid");

        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.tls.https_port, 8443);
        assert_eq!(
            config.server.state_dir,
            PathBuf::from("C:/ProgramData/Blentinel/Hub")
        );
        assert!(text.contains("# Retention"), "the template's comments stay");
    }

    #[test]
    fn https_can_be_turned_off_explicitly() {
        let toml = format!("{MINIMAL}behind_proxy = true\n[server.tls]\nenabled = false\n");
        let config: HubConfig = toml::from_str(&toml).unwrap();
        assert!(!config.server.tls.enabled);
        assert!(config.server.behind_proxy);
    }
}
