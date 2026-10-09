//! Hub upgrades (M4-04, proposal 0012 §8, D-75): finding a newer hub
//! release, and checking it is genuine.
//!
//! A release is a folder of files ([`common::hub_release`]): a URL (GitHub's
//! latest release by default, `[upgrades] release_url`) or a folder on the
//! server. `SHA256SUMS` must be signed by a release key compiled into this
//! hub, and every file used must match it, so neither the host nor the
//! network is trusted (D-41).
//!
//! - **The notice:** off by default, since it means contacting an outside
//!   server. Turned on (admin page), the hub checks once a day and shows
//!   "Blentinel X is available" with the command to run. It never upgrades.
//! - **The upgrade itself** is `hub upgrade`, run by an admin on the server
//!   (`ops::hub_upgrade`): the service runs as an unprivileged user that
//!   can't replace its own program, by design (D-52).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use common::hub_release::{
    HubRelease, RELEASE_FILE, SUMS_FILE, SUMS_SIGNATURE_FILE, Sums, open_release, open_sums,
};
use common::release::Version;

use crate::model::HubUpgradeInfo;
use crate::storage::Storage;

/// This hub's version.
pub const RUNNING: &str = env!("CARGO_PKG_VERSION");
/// The largest `SHA256SUMS` or `release.json` read.
const MAX_INDEX_BYTES: usize = 1024 * 1024;
/// The largest hub binary or probe archive read.
pub const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;
/// How long one download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How often the notice looks, once turned on.
pub const CHECK_EVERY: chrono::Duration = chrono::Duration::hours(24);

/// Where release files are read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Url(String),
    Folder(PathBuf),
}

impl Source {
    /// `https://…` (or `http://…`), `file://…`, or a folder path.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if text.starts_with("https://") || text.starts_with("http://") {
            Self::Url(text.trim_end_matches('/').to_string())
        } else if let Some(path) = text.strip_prefix("file://") {
            Self::Folder(PathBuf::from(path))
        } else {
            Self::Folder(PathBuf::from(text))
        }
    }

    /// Read file `name` of the release, at most `max` bytes.
    pub async fn read(&self, name: &str, max: usize) -> Result<Vec<u8>> {
        let bytes = match self {
            Self::Folder(dir) => {
                let path = dir.join(name);
                let len = tokio::fs::metadata(&path)
                    .await
                    .with_context(|| format!("can't read {}", path.display()))?
                    .len();
                if len > max as u64 {
                    bail!("{} is larger than {max} bytes", path.display());
                }
                tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("can't read {}", path.display()))?
            }
            Self::Url(base) => {
                let url = format!("{base}/{name}");
                let client = reqwest::Client::builder()
                    .timeout(DOWNLOAD_TIMEOUT)
                    .user_agent(concat!("blentinel-hub/", env!("CARGO_PKG_VERSION")))
                    .build()?;
                let mut response = client
                    .get(&url)
                    .send()
                    .await
                    .with_context(|| format!("can't download {url}"))?
                    .error_for_status()
                    .with_context(|| format!("can't download {url}"))?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    if bytes.len() + chunk.len() > max {
                        bail!("{url} is larger than {max} bytes");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                bytes
            }
        };
        Ok(bytes)
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Url(url) => f.write_str(url),
            Self::Folder(dir) => write!(f, "{}", dir.display()),
        }
    }
}

/// A release's signed list of files and its version.
#[derive(Debug, Clone)]
pub struct Index {
    pub release: HubRelease,
    pub sums: Sums,
}

/// Read and check the index of the release at `source` against `keys`.
pub async fn read_index(source: &Source, keys: &[[u8; 32]]) -> Result<Index> {
    let sums = source.read(SUMS_FILE, MAX_INDEX_BYTES).await?;
    let signature = source.read(SUMS_SIGNATURE_FILE, MAX_INDEX_BYTES).await?;
    let sums =
        open_sums(&sums, &signature, keys).with_context(|| format!("{SUMS_FILE} of {source}"))?;
    let release = source.read(RELEASE_FILE, MAX_INDEX_BYTES).await?;
    let release =
        open_release(&sums, &release).with_context(|| format!("{RELEASE_FILE} of {source}"))?;
    Ok(Index { release, sums })
}

/// Read file `name` of the release and check it against the signed list.
pub async fn read_checked(source: &Source, index: &Index, name: &str) -> Result<Vec<u8>> {
    if index.sums.sha256(name).is_none() {
        bail!("release {} has no {name}", index.release.version);
    }
    let bytes = source.read(name, MAX_FILE_BYTES).await?;
    if !index.sums.matches(name, &bytes) {
        bail!("{name} from {source} isn't the file the release signs: not using it");
    }
    Ok(bytes)
}

/// Is `version` newer than the hub running?
pub fn is_newer(version: &str) -> bool {
    match (Version::parse(version), Version::parse(RUNNING)) {
        (Some(found), Some(running)) => found > running,
        _ => false,
    }
}

/// Check `source` now, and record what was found.
pub async fn check_now(
    storage: &dyn Storage,
    source: &Source,
    keys: &[[u8; 32]],
    now: DateTime<Utc>,
) -> Result<()> {
    match read_index(source, keys).await {
        Ok(index) => {
            let version = index.release.version;
            if is_newer(&version) {
                tracing::info!(%version, "A newer hub release is available: `hub upgrade` on the server");
            }
            storage
                .record_upgrade_check(now, Some(&version), None)
                .await
        }
        Err(e) => {
            tracing::warn!("Checking for a newer hub release failed: {e:#}");
            storage
                .record_upgrade_check(now, None, Some(&format!("{e:#}")))
                .await
        }
    }
}

/// Is a check due at `now`: turned on, and none in the last day?
pub fn check_due(enabled: bool, checked_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    enabled && checked_at.is_none_or(|at| now - at >= CHECK_EVERY)
}

/// The background check: if it is turned on and due, check.
pub async fn sweep(
    storage: &dyn Storage,
    release_url: &str,
    keys: &[[u8; 32]],
    now: DateTime<Utc>,
) -> Result<()> {
    let check = storage.upgrade_check().await?;
    if check_due(check.enabled, check.checked_at, now) {
        check_now(storage, &Source::parse(release_url), keys, now).await?;
    }
    Ok(())
}

/// The command an admin runs on the server to upgrade.
pub fn upgrade_command() -> String {
    let exe = crate::ops::layout::Layout::system().exe;
    if cfg!(windows) {
        format!("\"{}\" upgrade", exe.display())
    } else {
        format!("sudo {} upgrade", exe.display())
    }
}

/// What the admin page shows.
pub async fn info(storage: &dyn Storage, release_url: &str) -> Result<HubUpgradeInfo> {
    let check = storage.upgrade_check().await?;
    let available = check
        .latest_version
        .clone()
        .filter(|version| is_newer(version));
    Ok(HubUpgradeInfo {
        running: RUNNING.to_string(),
        check_enabled: check.enabled,
        checked_at: check
            .checked_at
            .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string()),
        latest: check.latest_version,
        available,
        error: check.error,
        release_url: release_url.to_string(),
        command: upgrade_command(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_urls_or_folders() {
        assert_eq!(
            Source::parse("https://example.com/rel/"),
            Source::Url("https://example.com/rel".into())
        );
        assert_eq!(
            Source::parse("file:///srv/rel"),
            Source::Folder("/srv/rel".into())
        );
        assert_eq!(Source::parse("/srv/rel"), Source::Folder("/srv/rel".into()));
    }

    #[test]
    fn only_a_newer_version_is_an_upgrade() {
        assert!(is_newer("999.0.0"));
        assert!(!is_newer(RUNNING));
        assert!(!is_newer("0.0.1"));
        assert!(!is_newer("latest"));
    }

    #[test]
    fn the_check_runs_daily_once_turned_on() {
        let now: DateTime<Utc> = "2026-10-09T12:00:00Z".parse().unwrap();
        assert!(!check_due(false, None, now));
        assert!(check_due(true, None, now));
        assert!(!check_due(
            true,
            Some(now - chrono::Duration::hours(23)),
            now
        ));
        assert!(check_due(
            true,
            Some(now - chrono::Duration::hours(24)),
            now
        ));
    }
}
