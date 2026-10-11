//! The probe's directory, and the config its hub gave it (D-49).
//!
//! The probe has no config file to edit. Its checks and report interval are
//! hub records, sent in the hub's signed answer to a report whenever the
//! probe runs another revision. The last one it accepted is kept in
//! `config.json`, so the probe runs it again after a restart, hub or no hub
//! (FP-4).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use common::checks::{ConfigRejection, DEFAULT_INTERVAL_SECS, ProbeConfig};
use tracing::{error, info};

/// The file, in the probe's directory, holding the last config it accepted.
pub const FILE_NAME: &str = "config.json";

/// The directory the probe keeps all of its files in: next to its executable.
pub(crate) fn get_base_dir() -> PathBuf {
    std::env::current_exe()
        .expect("Failed to determine executable path")
        .parent()
        .expect("Executable has no parent directory")
        .to_path_buf()
}

pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// What the probe runs: the config it accepted last, if any, and the last
/// one it refused (reported to the hub until another arrives).
#[derive(Debug, Default)]
pub struct ActiveConfig {
    pub config: Option<ProbeConfig>,
    pub rejected: Option<ConfigRejection>,
}

impl ActiveConfig {
    /// The cached config in `dir`. A damaged file is logged and ignored:
    /// the hub sends the config again on the first report.
    pub fn load(dir: &Path) -> Self {
        let config = match load_cached(dir) {
            Ok(config) => config,
            Err(e) => {
                error!("{e:#}; waiting for the hub to send the checks again");
                None
            }
        };
        Self {
            config,
            rejected: None,
        }
    }

    /// Seconds between two reports: the config's, else the default.
    pub fn interval(&self) -> Duration {
        let secs = self
            .config
            .as_ref()
            .map_or(DEFAULT_INTERVAL_SECS, |c| c.interval_secs);
        Duration::from_secs(u64::from(secs))
    }

    pub fn revision(&self) -> Option<String> {
        self.config.as_ref().map(|c| c.revision.clone())
    }

    /// Take the config the hub sent, if it's usable, and save it in `dir`.
    /// `true` if the probe now runs something new. A config it can't use is
    /// remembered as rejected and the current one kept.
    pub fn apply(&mut self, dir: &Path, offered: ProbeConfig) -> bool {
        if self.config.as_ref() == Some(&offered) {
            return false;
        }
        if let Err(e) = offered.validate() {
            error!(
                revision = %offered.revision,
                "The hub sent a config this probe can't use; keeping the current one: {e}"
            );
            self.rejected = Some(ConfigRejection {
                revision: offered.revision,
                error: e.to_string(),
            });
            return false;
        }
        if let Err(e) = save(dir, &offered) {
            // Runs anyway; after a restart the hub sends it again.
            error!("{e:#}");
        }
        info!(
            revision = %offered.revision,
            checks = offered.checks.len(),
            interval_secs = offered.interval_secs,
            "New config from the hub"
        );
        self.config = Some(offered);
        self.rejected = None;
        true
    }
}

fn load_cached(dir: &Path) -> Result<Option<ProbeConfig>> {
    let path = path_in(dir);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("can't read {}", path.display()))?;
    let config: ProbeConfig =
        serde_json::from_str(&text).with_context(|| format!("{} is damaged", path.display()))?;
    config
        .validate()
        .with_context(|| format!("{} can't be used", path.display()))?;
    Ok(Some(config))
}

fn save(dir: &Path, config: &ProbeConfig) -> Result<()> {
    let path = path_in(dir);
    common::fs::write_atomic(&path, serde_json::to_string_pretty(config)?)
        .with_context(|| format!("can't save {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::{CheckDefinition, CheckId, CheckSpec};

    fn config(revision: &str, host: &str) -> ProbeConfig {
        ProbeConfig {
            revision: revision.into(),
            interval_secs: 30,
            checks: vec![CheckDefinition {
                id: CheckId("c1".into()),
                name: "Router".into(),
                spec: CheckSpec::ping(host),
            }],
        }
    }

    #[test]
    fn without_a_config_the_probe_reports_nothing_every_minute() {
        let dir = tempfile::tempdir().unwrap();
        let active = ActiveConfig::load(dir.path());
        assert!(active.config.is_none() && active.revision().is_none());
        assert_eq!(active.interval(), Duration::from_secs(60));
    }

    #[test]
    fn an_accepted_config_is_run_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut active = ActiveConfig::load(dir.path());
        assert!(active.apply(dir.path(), config("r1", "10.0.0.1")));
        assert!(
            !active.apply(dir.path(), config("r1", "10.0.0.1")),
            "same again"
        );
        assert_eq!(active.interval(), Duration::from_secs(30));

        let restarted = ActiveConfig::load(dir.path());
        assert_eq!(restarted.config, Some(config("r1", "10.0.0.1")));
    }

    #[test]
    fn an_unusable_config_is_rejected_and_the_current_one_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut active = ActiveConfig::load(dir.path());
        active.apply(dir.path(), config("r1", "10.0.0.1"));

        assert!(!active.apply(dir.path(), config("r2", "not an address")));
        assert_eq!(active.revision().as_deref(), Some("r1"));
        let rejected = active.rejected.clone().unwrap();
        assert_eq!(rejected.revision, "r2");
        assert!(rejected.error.contains("IP address"), "{}", rejected.error);
        assert_eq!(
            ActiveConfig::load(dir.path()).revision().as_deref(),
            Some("r1")
        );

        // A good one clears the rejection.
        assert!(active.apply(dir.path(), config("r3", "10.0.0.3")));
        assert!(active.rejected.is_none());
    }

    #[test]
    fn a_damaged_cache_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_in(dir.path()), "{not json").unwrap();
        assert!(ActiveConfig::load(dir.path()).config.is_none());
    }
}
