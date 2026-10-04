//! Self-update from the hub (M2-15, proposal 0012, D-41, D-55).
//!
//! The hub offers the release the admin approved in its signed answer to a
//! report. The probe installs it only if **the release** is signed by a key
//! compiled into it (`common::release::trusted_keys`), is newer than itself
//! and has a binary for its platform, and the local brake is off. Then:
//!
//! 1. download to `probe.new` beside itself, while monitoring goes on, and
//!    check the SHA-256 against the signed manifest;
//! 2. run `probe.new --self-test`: nothing is replaced if it fails;
//! 3. keep the running binary as `probe.prev`, rename the new one into place,
//!    note the switch in `update.json`, and restart ([`Restart`]);
//! 4. watchdog: the new version must deliver a report within 15 minutes (or
//!    3 intervals) and may start at most 3 times before it does; otherwise
//!    `probe.prev` comes back, and the version isn't tried again.

pub mod binary;
pub mod self_test;
pub mod state;

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use common::ack::UpdateOffer;
use common::models::UpdateFailure;
use common::release::{Manifest, Platform};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::transport::HubTransport;
use binary::{Attempt, Binaries};
use state::{Decision, Failed, Files, OnStart, Pending};

/// This build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long to wait after a download that may work next time.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The probe must restart: it switched to a new version, or back to the old.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Restart;

/// The exit code that asks the service manager for a restart: systemd's
/// unit has `Restart=always`; a Windows service stops with it as its
/// service-specific error, which its recovery action answers.
pub const EXIT_RESTART: i32 = 75;

/// Start the probe at `exe` again, outside a Windows service. Under systemd,
/// exit and let it start the unit again; run by hand, put the program at
/// `exe` in this one's place with the same arguments.
pub fn relaunch(exe: &Path) -> ! {
    if std::env::var_os("INVOCATION_ID").is_some() {
        std::process::exit(EXIT_RESTART); // systemd sets INVOCATION_ID for its units
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = std::process::Command::new(exe).args(&args).exec();
        eprintln!("[ERROR] can't start {}: {e}", exe.display());
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        match std::process::Command::new(exe).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                eprintln!("[ERROR] can't start {}: {e}", exe.display());
                std::process::exit(1);
            }
        }
    }
}

/// A download and self-test running in the background.
struct Fetch {
    manifest: Manifest,
    task: JoinHandle<Result<(), Attempt>>,
}

/// The updater, owned by the monitoring loop.
pub struct Updater {
    files: Files,
    bins: Binaries,
    keys: Vec<[u8; 32]>,
    pending: Option<Pending>,
    failed: Option<Failed>,
    fetch: Option<Fetch>,
    /// No new attempt before this (after one that may work next time).
    quiet_until: Option<Instant>,
}

impl Updater {
    /// The updater for the probe in `dir`, after the watchdog's start-up
    /// check: `Some(Restart)` if it just rolled back a version that keeps
    /// failing to start.
    pub fn start(dir: &Path) -> Result<(Self, Option<Restart>)> {
        let files = Files::new(dir);
        let mut updater = Self {
            bins: Binaries::current()?,
            keys: common::release::trusted_keys(),
            pending: None,
            failed: files.failed().unwrap_or_else(|e| {
                warn!("{e:#}; forgetting it");
                None
            }),
            fetch: None,
            quiet_until: None,
            files,
        };
        let recorded = updater.files.pending().unwrap_or_else(|e| {
            warn!("{e:#}; forgetting it");
            None
        });
        let had_pending = recorded.is_some();
        let (action, keep) = state::on_start(recorded, VERSION);
        match &keep {
            Some(pending) => updater.files.set_pending(pending)?,
            None if had_pending => updater.files.clear_pending()?,
            None => {}
        }
        updater.pending = keep;
        if let Some(pending) = &updater.pending {
            info!(
                from = %pending.from,
                start = pending.starts,
                "Running the new version; it is kept once it delivers a report"
            );
        }
        if let OnStart::RollBack { pending, reason } = action {
            let restart = updater.roll_back(&pending, &reason);
            return Ok((updater, restart));
        }
        Ok((updater, None))
    }

    /// The failed update to tell the hub about, if any.
    pub fn failure(&self) -> Option<UpdateFailure> {
        self.failed.as_ref().and_then(Failed::as_report)
    }

    /// A release offered by the hub. Starts a download if it should be
    /// installed; returns at once either way.
    pub fn offer(&mut self, offer: &UpdateOffer, transport: &HubTransport) {
        if self.pending.is_some() || self.fetch.is_some() {
            return;
        }
        if self.quiet_until.is_some_and(|t| Instant::now() < t) {
            return;
        }
        let opened =
            common::release::open_manifest(offer.manifest.as_bytes(), &offer.signature, &self.keys);
        // The version, for the record, even from a manifest that didn't open.
        let version = serde_json::from_str::<serde_json::Value>(&offer.manifest)
            .ok()
            .and_then(|m| m["version"].as_str().map(str::to_string))
            .unwrap_or_else(|| "?".into());
        match state::decide(
            opened,
            &version,
            VERSION,
            Platform::current(),
            self.failed.as_ref(),
            self.files.brake_on(),
        ) {
            Decision::Ignore => {}
            Decision::Refuse { version, error } => {
                warn!(%version, "Not installing the update the hub offers: {error}");
                self.remember_failure(&version, error, true);
            }
            Decision::Install(manifest) => self.begin_fetch(*manifest, transport),
        }
    }

    fn begin_fetch(&mut self, manifest: Manifest, transport: &HubTransport) {
        let Some(artifact) = Platform::current()
            .and_then(|p| manifest.artifact_for(p))
            .cloned()
        else {
            return; // `decide` checked
        };
        info!(
            version = %manifest.version,
            bytes = artifact.size,
            "Downloading the update the hub offers"
        );
        let (transport, bins, for_task) = (transport.clone(), self.bins.clone(), manifest.clone());
        let task = tokio::spawn(async move {
            binary::fetch_and_test(&transport, &for_task, &artifact, &bins).await
        });
        self.fetch = Some(Fetch { manifest, task });
    }

    /// After each report: confirm or roll back a new version, and switch to
    /// a downloaded one. `delivered`: the hub took a report this cycle.
    pub async fn after_report(
        &mut self,
        delivered: bool,
        interval: Duration,
        now: DateTime<Utc>,
    ) -> Option<Restart> {
        if let Some(pending) = self.pending.clone() {
            if delivered {
                self.confirm(&pending);
            } else if state::overdue(&pending, interval, now) {
                let reason = format!(
                    "{} delivered no report within {} minutes",
                    pending.to,
                    state::confirm_window(interval).as_secs() / 60
                );
                return self.roll_back(&pending, &reason);
            }
        }
        let finished = self.fetch.as_ref().is_some_and(|f| f.task.is_finished());
        if !finished {
            return None;
        }
        let fetch = self.fetch.take()?;
        let result = fetch
            .task
            .await
            .unwrap_or_else(|e| Err(Attempt::GiveUp(format!("the download task failed: {e}"))));
        match result {
            Ok(()) => self.switch(&fetch.manifest.version, now),
            Err(Attempt::Retry(error)) => {
                warn!(version = %fetch.manifest.version, "Update not installed, will try again: {error}");
                self.quiet_until = Some(Instant::now() + RETRY_AFTER);
                self.remember_failure(&fetch.manifest.version, error, false);
                None
            }
            Err(Attempt::GiveUp(error)) => {
                error!(version = %fetch.manifest.version, "Update not installed: {error}");
                self.remember_failure(&fetch.manifest.version, error, true);
                None
            }
        }
    }

    fn switch(&mut self, version: &str, now: DateTime<Utc>) -> Option<Restart> {
        let pending = Pending {
            from: VERSION.into(),
            to: version.into(),
            since: now,
            starts: 0,
        };
        // Noted first: a crash between the two leaves the old version
        // running, which drops a note that isn't about it.
        let switched = self
            .files
            .set_pending(&pending)
            .and_then(|()| binary::switch(&self.bins));
        match switched {
            Ok(()) => {
                info!(from = VERSION, to = %version, "Switched to the new version; restarting");
                self.pending = Some(pending);
                Some(Restart)
            }
            Err(e) => {
                let _ = self.files.clear_pending();
                let _ = std::fs::remove_file(&self.bins.new);
                error!(%version, "Can't switch to the new version: {e:#}");
                self.remember_failure(version, format!("can't switch: {e:#}"), true);
                None
            }
        }
    }

    fn confirm(&mut self, pending: &Pending) {
        info!(from = %pending.from, to = %pending.to, "Update confirmed: the new version reports to the hub");
        if let Err(e) = self.files.clear_pending() {
            error!("{e:#}");
        }
        self.pending = None;
        if self.failed.take().is_some()
            && let Err(e) = self.files.clear_failed()
        {
            error!("{e:#}");
        }
    }

    /// Put the previous version back. `Some(Restart)` if it is in place.
    fn roll_back(&mut self, pending: &Pending, reason: &str) -> Option<Restart> {
        error!(from = %pending.to, to = %pending.from, "Rolling back: {reason}");
        self.pending = None;
        if let Err(e) = self.files.clear_pending() {
            error!("{e:#}");
        }
        self.remember_failure(&pending.to, format!("rolled back: {reason}"), true);
        match binary::roll_back(&self.bins) {
            Ok(()) => Some(Restart),
            Err(e) => {
                error!("Can't roll back, staying on {}: {e:#}", pending.to);
                None
            }
        }
    }

    fn remember_failure(&mut self, version: &str, error: String, last: bool) {
        let failed = state::record_failure(self.failed.take(), version, error, last);
        if let Err(e) = self.files.set_failed(&failed) {
            error!("{e:#}");
        }
        self.failed = Some(failed);
    }
}
