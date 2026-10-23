//! What the updater remembers between runs, in the probe's folder, and the
//! pure decisions made from it (no network, no processes).
//!
//! | File | While | Holds |
//! |---|---|---|
//! | `update.json` | a new version runs unconfirmed | from, to, when, how many starts |
//! | `update-failed.json` | until another version is installed | the last version that failed, why, how often |
//! | `updates-off` | the local brake is on (`probe updates off`) | nothing |
//! | `staging-releases` | a staging probe (`probe install --staging`) | nothing |

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use common::models::UpdateFailure;
use common::release::{Manifest, Platform, ReleaseError, Rollback, Trust, Version};
use serde::{Deserialize, Serialize};

/// The pending update, while the new version hasn't delivered a report.
pub const PENDING_FILE: &str = "update.json";
/// The last update that failed.
pub const FAILED_FILE: &str = "update-failed.json";
/// The local emergency brake: refuse every update while this file exists.
pub const BRAKE_FILE: &str = "updates-off";
/// A staging probe (0013 §6.2, D-106): it also accepts releases signed with
/// a staging key while this file exists. Only `probe install --staging`, run
/// on the machine, writes it; nothing the hub sends can (a hub must not
/// widen what its probes accept).
pub const STAGING_FILE: &str = "staging-releases";

/// Starts a new version gets to deliver its first report; at the next one it
/// is rolled back (a crash loop).
pub const MAX_STARTS: u32 = 3;
/// Download attempts (network, hash) for one version before giving up on it.
pub const MAX_ATTEMPTS: u32 = 3;
/// How long a new version has to deliver a report, at least.
const MIN_CONFIRM_WINDOW: Duration = Duration::from_secs(15 * 60);
/// … or this many report intervals, if that is longer.
const CONFIRM_INTERVALS: u32 = 3;

/// A switch to a new version, waiting for its first delivered report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub from: String,
    pub to: String,
    pub since: DateTime<Utc>,
    /// How many times `to` has started.
    pub starts: u32,
}

/// The last update that didn't work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    pub version: String,
    pub error: String,
    /// Failed download attempts; once it reaches [`MAX_ATTEMPTS`], or for a
    /// failure that would happen again (`last`), the version isn't retried.
    pub attempts: u32,
    /// No point retrying: bad signature, self-test failed, rolled back, …
    pub last: bool,
}

impl Failed {
    /// Is this version given up on (and so reported to the hub)?
    pub fn is_final(&self) -> bool {
        self.last || self.attempts >= MAX_ATTEMPTS
    }

    pub fn as_report(&self) -> Option<UpdateFailure> {
        self.is_final().then(|| UpdateFailure {
            version: self.version.clone(),
            error: self.error.clone(),
        })
    }
}

/// The updater's files in the probe's folder.
#[derive(Debug, Clone)]
pub struct Files {
    pub dir: PathBuf,
}

impl Files {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Result<Option<T>> {
        let path = self.dir.join(name);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .with_context(|| format!("{} is damaged", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("can't read {}", path.display())),
        }
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let path = self.dir.join(name);
        let json = serde_json::to_vec_pretty(value).context("serializing the update state")?;
        common::fs::write_atomic(&path, json)
            .with_context(|| format!("can't write {}", path.display()))
    }

    fn remove(&self, name: &str) -> Result<()> {
        let path = self.dir.join(name);
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("can't delete {}", path.display()))
            }
            _ => Ok(()),
        }
    }

    pub fn pending(&self) -> Result<Option<Pending>> {
        self.read(PENDING_FILE)
    }
    pub fn set_pending(&self, pending: &Pending) -> Result<()> {
        self.write(PENDING_FILE, pending)
    }
    pub fn clear_pending(&self) -> Result<()> {
        self.remove(PENDING_FILE)
    }

    pub fn failed(&self) -> Result<Option<Failed>> {
        self.read(FAILED_FILE)
    }
    pub fn set_failed(&self, failed: &Failed) -> Result<()> {
        self.write(FAILED_FILE, failed)
    }
    pub fn clear_failed(&self) -> Result<()> {
        self.remove(FAILED_FILE)
    }

    pub fn brake_on(&self) -> bool {
        self.dir.join(BRAKE_FILE).exists()
    }

    /// Turn the brake on or off. Turning it off forgets a refusal made
    /// because of it, so the hub offers that version again.
    pub fn set_brake(&self, on: bool) -> Result<()> {
        if on {
            let path = self.dir.join(BRAKE_FILE);
            std::fs::write(
                &path,
                b"Updates are off on this machine: `probe updates on`.\n",
            )
            .with_context(|| format!("can't write {}", path.display()))
        } else {
            if self
                .failed()
                .ok()
                .flatten()
                .is_some_and(|f| f.error == BRAKE_REASON)
            {
                self.clear_failed()?;
            }
            self.remove(BRAKE_FILE)
        }
    }

    /// Whether this probe accepts staging-signed releases ([`STAGING_FILE`]).
    pub fn trust(&self) -> Trust {
        if self.dir.join(STAGING_FILE).exists() {
            Trust::Staging
        } else {
            Trust::Production
        }
    }

    /// Make this a staging probe, or a production one again.
    pub fn set_staging(&self, on: bool) -> Result<()> {
        if on {
            let path = self.dir.join(STAGING_FILE);
            std::fs::write(
                &path,
                b"A staging probe: it also accepts releases signed with a staging key. \
                  `probe install --no-staging` undoes it.\n",
            )
            .with_context(|| format!("can't write {}", path.display()))
        } else {
            self.remove(STAGING_FILE)
        }
    }
}

/// What the brake makes the probe answer an offer with.
pub const BRAKE_REASON: &str = "updates are turned off on this machine (probe updates on)";

/// What a starting probe must do about an update in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnStart {
    /// Nothing pending, or this is the new version and it may go on.
    Run(Option<Pending>),
    /// The new version keeps failing to start: put the old one back.
    RollBack { pending: Pending, reason: String },
}

/// The watchdog's first decision, made as a probe starts: count the start of
/// a new version, and give up on it after [`MAX_STARTS`]. A pending update
/// for another version (a rollback or an install by hand) is dropped.
pub fn on_start(pending: Option<Pending>, own: &str) -> (OnStart, Option<Pending>) {
    match pending {
        Some(mut pending) if pending.to == own => {
            pending.starts += 1;
            if pending.starts > MAX_STARTS {
                let reason = format!(
                    "{} started {} times without delivering a report",
                    pending.to, MAX_STARTS
                );
                (OnStart::RollBack { pending, reason }, None)
            } else {
                (OnStart::Run(Some(pending.clone())), Some(pending))
            }
        }
        _ => (OnStart::Run(None), None),
    }
}

/// How long a new version has to deliver a report.
pub fn confirm_window(interval: Duration) -> Duration {
    MIN_CONFIRM_WINDOW.max(interval * CONFIRM_INTERVALS)
}

/// Has a pending update run out of time to deliver a report?
pub fn overdue(pending: &Pending, interval: Duration, now: DateTime<Utc>) -> bool {
    let window = chrono::Duration::from_std(confirm_window(interval))
        .unwrap_or_else(|_| chrono::Duration::hours(1));
    now - pending.since > window
}

/// What to do with an offered release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Download this binary.
    Install(Box<Manifest>),
    /// Don't, and remember why (a refusal the hub should see).
    Refuse { version: String, error: String },
    /// Not now, nothing to remember (already tried, or already this version).
    Ignore,
}

/// Decide about a release whose signature has been checked (`opened`), for a
/// probe at version `own` on `platform`. An older release is installed only
/// with a signed `rollback` that names `own` (D-74).
pub fn decide(
    opened: Result<Manifest, ReleaseError>,
    rollback: Option<Result<Rollback, ReleaseError>>,
    offered_version: &str,
    own: &str,
    platform: Option<&Platform>,
    failed: Option<&Failed>,
    brake_on: bool,
) -> Decision {
    let refuse = |error: String| Decision::Refuse {
        version: offered_version.to_string(),
        error,
    };
    if failed.is_some_and(|f| f.version == offered_version && f.is_final()) {
        return Decision::Ignore;
    }
    if brake_on {
        return refuse(BRAKE_REASON.into());
    }
    let manifest = match opened {
        Ok(manifest) => manifest,
        Err(e) => return refuse(e.to_string()),
    };
    let (Some(offered), Some(current)) = (manifest.parsed_version(), Version::parse(own)) else {
        return refuse(format!("can't compare {} with {own}", manifest.version));
    };
    if offered == current {
        return Decision::Ignore;
    }
    if offered < current {
        let why = match rollback {
            None => "and no signed rollback allows going back".to_string(),
            Some(Err(e)) => format!("and its rollback can't be trusted: {e}"),
            Some(Ok(r)) if r.version != manifest.version => {
                format!("and its rollback is for {}", r.version)
            }
            Some(Ok(r)) if !r.allows(own) => {
                format!("and its rollback is only from {}", r.from.join(", "))
            }
            Some(Ok(_)) => String::new(),
        };
        if !why.is_empty() {
            return refuse(format!(
                "{offered} is older than this probe ({current}), {why}"
            ));
        }
    }
    match platform {
        Some(p) if manifest.artifact_for(p).is_some() => Decision::Install(Box::new(manifest)),
        Some(p) => refuse(format!("the release has no binary for {}", p.target)),
        None => refuse("this probe's platform isn't one releases are built for".into()),
    }
}

/// The record after an attempt at `version` failed with `error`; `last` for
/// a failure that would happen again.
pub fn record_failure(
    previous: Option<Failed>,
    version: &str,
    error: String,
    last: bool,
) -> Failed {
    let attempts = match previous {
        Some(p) if p.version == version => p.attempts + 1,
        _ => 1,
    };
    Failed {
        version: version.to_string(),
        error,
        attempts,
        last,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::release::{Artifact, MANIFEST_FORMAT};

    fn manifest(version: &str) -> Manifest {
        let platform = Platform::current().unwrap();
        Manifest {
            format: MANIFEST_FORMAT,
            version: version.into(),
            released: "2026-11-02T10:00:00Z".parse().unwrap(),
            protocol: 5,
            artifacts: vec![Artifact {
                target: platform.target.into(),
                file: platform.file_name(),
                sha256: "ab".repeat(32),
                size: 1,
            }],
        }
    }

    fn decide_for(version: &str, failed: Option<&Failed>, brake: bool) -> Decision {
        decide(
            Ok(manifest(version)),
            None,
            version,
            "0.2.0",
            Platform::current(),
            failed,
            brake,
        )
    }

    fn rollback(version: &str, from: &[&str]) -> Rollback {
        Rollback {
            format: MANIFEST_FORMAT,
            version: version.into(),
            from: from.iter().map(|v| v.to_string()).collect(),
            signed: "2026-11-03T10:00:00Z".parse().unwrap(),
        }
    }

    fn go_back(rollback: Option<Result<Rollback, ReleaseError>>) -> Decision {
        decide(
            Ok(manifest("0.1.9")),
            rollback,
            "0.1.9",
            "0.2.0",
            Platform::current(),
            None,
            false,
        )
    }

    #[test]
    fn an_older_release_is_installed_only_with_a_rollback_naming_this_version() {
        assert!(matches!(
            go_back(Some(Ok(rollback("0.1.9", &["0.2.0"])))),
            Decision::Install(m) if m.version == "0.1.9"
        ));
        for (rollback, why) in [
            (None, "no signed rollback"),
            (Some(Err(ReleaseError::BadSignature)), "can't be trusted"),
            (Some(Ok(rollback("0.1.8", &["0.2.0"]))), "is for 0.1.8"),
            (Some(Ok(rollback("0.1.9", &["0.2.1"]))), "only from 0.2.1"),
        ] {
            assert!(
                matches!(go_back(rollback), Decision::Refuse { ref error, .. } if error.contains(why)),
                "{why}"
            );
        }
    }

    #[test]
    fn a_newer_signed_release_is_installed() {
        assert!(matches!(
            decide_for("0.3.0", None, false),
            Decision::Install(m) if m.version == "0.3.0"
        ));
    }

    #[test]
    fn downgrades_the_brake_and_bad_signatures_are_refused() {
        assert!(matches!(
            decide_for("0.1.9", None, false),
            Decision::Refuse { error, .. } if error.contains("older than this probe")
        ));
        assert_eq!(decide_for("0.2.0", None, false), Decision::Ignore);
        assert_eq!(
            decide_for("0.3.0", None, true),
            Decision::Refuse {
                version: "0.3.0".into(),
                error: BRAKE_REASON.into()
            }
        );
        assert!(matches!(
            decide(Err(ReleaseError::BadSignature), None, "0.3.0", "0.2.0", Platform::current(), None, false),
            Decision::Refuse { version, .. } if version == "0.3.0"
        ));
        assert!(matches!(
            decide(
                Ok(manifest("0.3.0")),
                None,
                "0.3.0",
                "0.2.0",
                None,
                None,
                false
            ),
            Decision::Refuse { .. }
        ));
    }

    #[test]
    fn a_version_given_up_on_is_not_tried_again() {
        let mut failed = record_failure(None, "0.3.0", "hash mismatch".into(), false);
        assert!(!failed.is_final());
        assert!(matches!(
            decide_for("0.3.0", Some(&failed), false),
            Decision::Install(_)
        ));
        for _ in 1..MAX_ATTEMPTS {
            failed = record_failure(Some(failed), "0.3.0", "hash mismatch".into(), false);
        }
        assert!(failed.is_final());
        assert_eq!(decide_for("0.3.0", Some(&failed), false), Decision::Ignore);
        // A newer version starts over.
        assert!(matches!(
            decide_for("0.3.1", Some(&failed), false),
            Decision::Install(_)
        ));
        let rolled_back = record_failure(None, "0.3.1", "rolled back".into(), true);
        assert_eq!(rolled_back.attempts, 1);
        assert!(rolled_back.as_report().is_some());
    }

    #[test]
    fn a_new_version_that_keeps_crashing_is_rolled_back() {
        let pending = Pending {
            from: "0.2.0".into(),
            to: "0.3.0".into(),
            since: "2026-11-02T10:00:00Z".parse().unwrap(),
            starts: 0,
        };
        let mut current = Some(pending.clone());
        for start in 1..=MAX_STARTS {
            let (action, next) = on_start(current, "0.3.0");
            assert!(matches!(action, OnStart::Run(Some(ref p)) if p.starts == start));
            current = next;
        }
        let (action, next) = on_start(current, "0.3.0");
        assert!(
            matches!(action, OnStart::RollBack { ref reason, .. } if reason.contains("3 times"))
        );
        assert_eq!(next, None);

        // The old version (rolled back by hand) or another one: drop it.
        assert_eq!(
            on_start(Some(pending.clone()), "0.2.0"),
            (OnStart::Run(None), None)
        );
        assert_eq!(on_start(None, "0.3.0"), (OnStart::Run(None), None));
    }

    #[test]
    fn a_new_version_must_report_within_its_window() {
        let since: DateTime<Utc> = "2026-11-02T10:00:00Z".parse().unwrap();
        let pending = Pending {
            from: "0.2.0".into(),
            to: "0.3.0".into(),
            since,
            starts: 1,
        };
        let minute = Duration::from_secs(60);
        assert_eq!(confirm_window(minute), MIN_CONFIRM_WINDOW);
        assert_eq!(confirm_window(minute * 10), minute * 30);
        assert!(!overdue(
            &pending,
            minute,
            since + chrono::Duration::minutes(15)
        ));
        assert!(overdue(
            &pending,
            minute,
            since + chrono::Duration::minutes(16)
        ));
        assert!(!overdue(
            &pending,
            minute * 10,
            since + chrono::Duration::minutes(29)
        ));
    }

    #[test]
    fn the_files_round_trip_and_the_brake_forgets_its_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let files = Files::new(dir.path());
        assert_eq!(files.pending().unwrap(), None);
        let pending = Pending {
            from: "0.2.0".into(),
            to: "0.3.0".into(),
            since: "2026-11-02T10:00:00Z".parse().unwrap(),
            starts: 2,
        };
        files.set_pending(&pending).unwrap();
        assert_eq!(files.pending().unwrap(), Some(pending));
        files.clear_pending().unwrap();
        files.clear_pending().unwrap(); // already gone: fine

        files.set_brake(true).unwrap();
        assert!(files.brake_on());
        files
            .set_failed(&record_failure(None, "0.3.0", BRAKE_REASON.into(), true))
            .unwrap();
        files.set_brake(false).unwrap();
        assert!(!files.brake_on());
        assert_eq!(files.failed().unwrap(), None);

        std::fs::write(dir.path().join(PENDING_FILE), b"{").unwrap();
        assert!(files.pending().is_err());
    }

    /// D-106: a release signed with a staging key opens only on a probe
    /// made a staging probe on its own machine, and not after it is undone.
    #[test]
    fn a_staging_release_opens_only_on_a_staging_probe() {
        use common::release::{ReleaseKeys, open_manifest, sign_manifest};
        use ed25519_dalek::SigningKey;
        let staging = SigningKey::from_bytes(&[7; 32]);
        let keys = ReleaseKeys {
            production: vec![SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes()],
            staging: vec![staging.verifying_key().to_bytes()],
        };
        // A fixed platform, not this machine's: the signature doesn't depend
        // on it, and macOS has none (ROADMAP M0-30).
        let linux = Platform::by_target("x86_64-unknown-linux-musl").unwrap();
        let manifest = Manifest {
            format: MANIFEST_FORMAT,
            version: "0.3.0-dev.10".into(),
            released: "2026-11-02T10:00:00Z".parse().unwrap(),
            protocol: 5,
            artifacts: vec![Artifact {
                target: linux.target.into(),
                file: linux.file_name(),
                sha256: "ab".repeat(32),
                size: 1,
            }],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let signature = sign_manifest(&bytes, &staging);
        let dir = tempfile::tempdir().unwrap();
        let files = Files::new(dir.path());
        let opens =
            |files: &Files| open_manifest(&bytes, &signature, &keys.trusted(files.trust())).is_ok();

        assert_eq!(files.trust(), Trust::Production);
        assert!(!opens(&files));
        files.set_staging(true).unwrap();
        assert_eq!(files.trust(), Trust::Staging);
        assert!(opens(&files));
        files.set_staging(false).unwrap();
        files.set_staging(false).unwrap(); // already a production probe: fine
        assert!(!opens(&files));
    }
}
