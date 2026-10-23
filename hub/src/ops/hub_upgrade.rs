//! `hub upgrade` (M4-04, proposal 0012 §8, D-75): an admin upgrades the
//! installed hub to a newer release, never the hub by itself. It:
//!
//! 1. reads the release (`[upgrades] release_url`, or `--from`) and checks
//!    `SHA256SUMS` against the release keys compiled into this hub, then
//!    downloads the hub for this machine and checks its SHA-256;
//! 2. runs the new hub's `self-test` beside the running one: it upgrades a
//!    *copy* of the database (`ops::self_test`). Nothing has changed yet;
//! 3. backs the hub up (a `-pre-upgrade` backup, D-60);
//! 4. stops the service, keeps the running program as `hub.prev`, puts the
//!    new one in its place, and starts the service;
//! 5. waits for the hub to run and answer for a while. If it doesn't, it
//!    puts back the program **and the database** from that backup, and
//!    starts the old hub again;
//! 6. adds the release's probes, ready to approve (D-74).
//!
//! The machine-facing steps are behind [`Machine`], so every path can be
//! tested without a service.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use common::hub_release::{PROBES_ARCHIVE, current_target, hub_file};

use super::layout::Layout;
use super::{ServiceState, answers, needs_admin, platform};
use crate::config::HubConfig;
use crate::model::BackupKind;
use crate::service::hub_upgrades::{self, Index, Source};
use crate::service::{backups, releases};
use crate::storage;

/// How long the new hub's self-test may take (a large database is copied).
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How long the new hub has to start answering.
const START_TIMEOUT: Duration = Duration::from_secs(90);
/// … and then how long it must keep running and answering.
const STEADY_FOR: Duration = Duration::from_secs(15);

/// `hub upgrade` options.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpgradeOptions {
    /// Where the release is, instead of `[upgrades] release_url`.
    pub from: Option<String>,
}

/// What the upgrade does to the machine it runs on.
pub trait Machine {
    fn stop(&self) -> Result<()>;
    fn start(&self) -> Result<()>;
    /// Does the hub start answering, and keep running and answering?
    fn healthy(&self, config: &HubConfig) -> bool;
    /// Run `binary self-test`: the first line it printed, or why it failed.
    fn self_test(&self, binary: &Path, config_path: &Path) -> Result<String>;
    /// Give the service its files back after root wrote some.
    fn secure(&self, state_dir: &Path) -> Result<()>;
}

/// The installed hub's service on this machine.
struct System {
    layout: Layout,
}

impl Machine for System {
    fn stop(&self) -> Result<()> {
        platform::stop_service()
    }

    fn start(&self) -> Result<()> {
        platform::install_service(&self.layout, true)
    }

    fn healthy(&self, config: &HubConfig) -> bool {
        let up = || {
            platform::service_state() == ServiceState::Running
                && answers(&config.server.host, config.server.port)
        };
        let deadline = Instant::now() + START_TIMEOUT;
        while !up() {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        // A hub that crashes soon after starting is restarted by the service
        // manager: it must stay up, not just come up once.
        let steady = Instant::now() + STEADY_FOR;
        while Instant::now() < steady {
            std::thread::sleep(Duration::from_secs(1));
            if !up() {
                return false;
            }
        }
        true
    }

    fn self_test(&self, binary: &Path, config_path: &Path) -> Result<String> {
        let mut child = Command::new(binary)
            .arg("self-test")
            .arg("--config")
            .arg(config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("can't run {}", binary.display()))?;
        let deadline = Instant::now() + SELF_TEST_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                bail!(
                    "the new hub's self-test didn't finish in {} minutes",
                    SELF_TEST_TIMEOUT.as_secs() / 60
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let mut out = String::new();
        let mut err = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut out)?;
        }
        if let Some(mut stderr) = child.stderr.take() {
            stderr.read_to_string(&mut err)?;
        }
        if !status.success() {
            bail!("the new hub's self-test failed: {}", err.trim());
        }
        Ok(out.lines().next().unwrap_or_default().to_string())
    }

    fn secure(&self, state_dir: &Path) -> Result<()> {
        platform::secure_state_dir(state_dir)
    }
}

/// A release downloaded and checked, its hub staged beside the running one.
pub struct Prepared {
    pub version: String,
    /// The new hub, at `<program>.new`.
    pub staged: PathBuf,
    /// The release's probes (`probes.tar.gz`), if it has them.
    pub probes: Option<Vec<u8>>,
}

/// Read and check the release at `source`, and stage its hub for this
/// machine beside `exe`. Refuses a release that isn't newer.
pub async fn prepare(source: &Source, keys: &[[u8; 32]], exe: &Path) -> Result<Prepared> {
    let index: Index = hub_upgrades::read_index(source, keys).await?;
    let version = index.release.version.clone();
    if !hub_upgrades::is_newer(&version) {
        bail!(
            "the release at {source} is {version}; this hub is {}: nothing to upgrade",
            hub_upgrades::RUNNING
        );
    }
    let target =
        current_target().context("releases have no hub for this operating system or processor")?;
    let name = hub_file(target);
    if index.sums.sha256(&name).is_none() {
        bail!(
            "release {version} has no hub for {target}{}",
            if cfg!(windows) {
                ": download the new hub.exe and run `hub.exe install` from it"
            } else {
                ""
            }
        );
    }
    let binary = hub_upgrades::read_checked(source, &index, &name).await?;
    let probes = match index.sums.sha256(PROBES_ARCHIVE) {
        Some(_) => Some(hub_upgrades::read_checked(source, &index, PROBES_ARCHIVE).await?),
        None => None,
    };
    let staged = exe.with_extension("new");
    std::fs::write(&staged, &binary)
        .with_context(|| format!("can't write {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(Prepared {
        version,
        staged,
        probes,
    })
}

/// How an upgrade ended (a failure that left the old hub running is one).
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Upgraded {
        backup: PathBuf,
        probes: Option<String>,
    },
    /// The new hub didn't come up: the old one is back, with its database.
    RolledBack {
        backup: PathBuf,
        /// The database the new hub left behind.
        aside: PathBuf,
    },
}

/// Self-test, back up, switch, and check (or roll back) the hub at `exe`.
pub async fn apply(
    machine: &dyn Machine,
    exe: &Path,
    prepared: &Prepared,
    config: &HubConfig,
    config_path: &Path,
    now: DateTime<Utc>,
) -> Result<Outcome> {
    // 2. The new hub, on a copy of the database: nothing changed so far.
    let tested = machine.self_test(&prepared.staged, config_path);
    let expected = super::self_test::banner(&prepared.version);
    match tested {
        Ok(first) if first.trim() == expected => {}
        Ok(first) => {
            let _ = std::fs::remove_file(&prepared.staged);
            bail!("the new hub says {first:?}, not {expected:?}: not upgrading");
        }
        Err(e) => {
            let _ = std::fs::remove_file(&prepared.staged);
            return Err(e.context("not upgrading; nothing was changed"));
        }
    }

    // 3. A backup to come back to.
    let db = config.server.resolved_db_path();
    let store = storage::open_read_only(&db)
        .await
        .with_context(|| format!("can't open {}", db.display()))?;
    let made = backups::make(
        store.as_ref(),
        config,
        config_path,
        BackupKind::PreUpgrade,
        now,
    )
    .await;
    store.close().await;
    let backup = config.backups_dir().join(
        made.context("can't back up the hub; nothing was changed")?
            .name,
    );
    machine.secure(&config.server.state_dir)?;

    // 4. The switch.
    machine.stop()?;
    let previous = exe.with_extension("prev");
    if previous.exists() {
        std::fs::remove_file(&previous)
            .with_context(|| format!("can't delete {}", previous.display()))?;
    }
    std::fs::rename(exe, &previous)
        .with_context(|| format!("can't move {} aside", exe.display()))?;
    if let Err(e) = std::fs::rename(&prepared.staged, exe) {
        std::fs::rename(&previous, exe)?;
        machine.start()?;
        return Err(anyhow::Error::from(e)
            .context("can't put the new hub in place; the old one runs again"));
    }

    // 5. Does it run?
    if machine.start().is_ok() && machine.healthy(config) {
        let probes = match &prepared.probes {
            Some(archive) => add_probes(config, archive)?,
            None => None,
        };
        machine.secure(&config.server.state_dir)?;
        return Ok(Outcome::Upgraded { backup, probes });
    }

    // It doesn't: the old program and database back.
    machine.stop()?;
    let failed = exe.with_extension("failed");
    let _ = std::fs::remove_file(&failed);
    std::fs::rename(exe, &failed)
        .with_context(|| format!("can't move the new hub aside ({})", exe.display()))?;
    std::fs::rename(&previous, exe)
        .with_context(|| format!("can't put back {}", previous.display()))?;
    let aside = super::backup::restore_database(&backup, config, now).with_context(|| {
        format!(
            "the new hub didn't start, the old program is back, but putting the database back \
             failed: `hub restore {}`",
            backup.display()
        )
    })?;
    machine.secure(&config.server.state_dir)?;
    machine.start()?;
    if !machine.healthy(config) {
        bail!(
            "the new hub didn't start; the old program and database are back, but it doesn't \
             start either: `hub doctor`, and `hub restore {}` if needed",
            backup.display()
        );
    }
    Ok(Outcome::RolledBack { backup, aside })
}

/// Add the release's probes; their version.
fn add_probes(config: &HubConfig, archive: &[u8]) -> Result<Option<String>> {
    let root = config.server.state_dir.join(releases::RELEASES_DIR);
    match releases::add_from_archive(&root, archive, &config.release_keys()) {
        Ok(manifest) => Ok(Some(manifest.version)),
        // The hub is upgraded: say so, and how to add the probes by hand.
        Err(e) => {
            eprintln!("[WARN] the release's probes weren't added ({e}); `hub release add` them");
            Ok(None)
        }
    }
}

/// `hub upgrade`.
pub async fn upgrade(options: &UpgradeOptions, config_path: &Path) -> Result<()> {
    if !platform::is_admin() {
        return Err(needs_admin("upgrade"));
    }
    let layout = Layout::system();
    if platform::service_state() == ServiceState::NotInstalled || !layout.exe.exists() {
        bail!(
            "no installed hub here: `hub upgrade` upgrades the hub `hub install` set up \
             (a hub run by hand: stop it and run the new one)"
        );
    }
    let config = crate::config::load_from(config_path)
        .map_err(|e| anyhow::anyhow!("{}: {e}", config_path.display()))?;
    let source = Source::parse(
        options
            .from
            .as_deref()
            .unwrap_or(&config.upgrades.release_url),
    );
    println!("Release: {source}");
    let prepared = prepare(&source, &config.release_keys(), &layout.exe).await?;
    println!(
        "Upgrading from {} to {}: signature and checksums verified.",
        hub_upgrades::RUNNING,
        prepared.version
    );
    let machine = System {
        layout: layout.clone(),
    };
    match apply(
        &machine,
        &layout.exe,
        &prepared,
        &config,
        config_path,
        Utc::now(),
    )
    .await?
    {
        Outcome::Upgraded { backup, probes } => {
            println!(
                "Upgraded to {}. The hub runs and answers.",
                prepared.version
            );
            println!("  Backup from before: {}", backup.display());
            println!(
                "  The old program: {} (`hub restore` and that program undo the upgrade)",
                layout.exe.with_extension("prev").display()
            );
            if let Some(version) = probes {
                println!(
                    "  Probes: release {version} added; approve it on the admin page (Probe updates) to update them"
                );
            }
            Ok(())
        }
        Outcome::RolledBack { backup, aside } => bail!(
            "the new hub didn't keep running, so the old one is back with its database \
             (from {}). The database the new hub left is in {}; its log says why \
             (journalctl -u {}).",
            backup.display(),
            aside.display(),
            super::layout::SERVICE_NAME
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A machine whose new hub passes or fails its self-test and start.
    struct Fake {
        self_test: Result<String, String>,
        /// Whether each start is healthy, in order.
        healthy: RefCell<Vec<bool>>,
        log: RefCell<Vec<String>>,
        /// The first start ruins this file, as a new hub's failed upgrade
        /// of the database might.
        ruin_on_start: RefCell<Option<PathBuf>>,
    }

    impl Fake {
        fn new(self_test: Result<&str, &str>, healthy: &[bool]) -> Self {
            Self {
                self_test: self_test.map(str::to_string).map_err(str::to_string),
                healthy: RefCell::new(healthy.iter().rev().copied().collect()),
                log: RefCell::default(),
                ruin_on_start: RefCell::default(),
            }
        }
        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    impl Machine for Fake {
        fn stop(&self) -> Result<()> {
            self.log.borrow_mut().push("stop".into());
            Ok(())
        }
        fn start(&self) -> Result<()> {
            self.log.borrow_mut().push("start".into());
            if let Some(path) = self.ruin_on_start.borrow_mut().take() {
                std::fs::write(path, b"not a database any more").unwrap();
            }
            Ok(())
        }
        fn healthy(&self, _: &HubConfig) -> bool {
            self.healthy.borrow_mut().pop().unwrap_or(false)
        }
        fn self_test(&self, _: &Path, _: &Path) -> Result<String> {
            self.self_test.clone().map_err(|e| anyhow::anyhow!(e))
        }
        fn secure(&self, _: &Path) -> Result<()> {
            Ok(())
        }
    }

    /// A hub in a folder: its config, a database with one company, the
    /// running program (`hub`, holding "old") and a staged one ("new").
    struct Hub {
        _dir: tempfile::TempDir,
        config: HubConfig,
        config_path: PathBuf,
        exe: PathBuf,
        prepared: Prepared,
    }

    async fn hub() -> Hub {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("blentinel_hub.toml");
        std::fs::write(
            &config_path,
            format!(
                "[server]\nhost = '127.0.0.1'\nport = 1\nstate_dir = '{}'\ndb_path = 'hub.db'\n",
                dir.path().display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        let config = crate::config::load_from(&config_path).unwrap();
        let store = storage::open(&config.server.resolved_db_path())
            .await
            .unwrap();
        store
            .create_company("acme", "Acme", Utc::now())
            .await
            .unwrap();
        store.close().await;
        let exe = dir.path().join("hub");
        std::fs::write(&exe, b"old").unwrap();
        let staged = exe.with_extension("new");
        std::fs::write(&staged, b"new").unwrap();
        Hub {
            config,
            config_path,
            exe,
            prepared: Prepared {
                version: "9.0.0".into(),
                staged,
                probes: None,
            },
            _dir: dir,
        }
    }

    async fn companies(hub: &Hub) -> usize {
        let store = storage::open_read_only(&hub.config.server.resolved_db_path())
            .await
            .unwrap();
        let n = store
            .companies(&crate::model::CompanyScope::All)
            .await
            .unwrap()
            .len();
        store.close().await;
        n
    }

    async fn run(hub: &Hub, machine: &Fake) -> Result<Outcome> {
        apply(
            machine,
            &hub.exe,
            &hub.prepared,
            &hub.config,
            &hub.config_path,
            Utc::now(),
        )
        .await
    }

    #[tokio::test]
    async fn a_hub_that_passes_its_self_test_and_runs_is_switched_to() {
        let hub = hub().await;
        let machine = Fake::new(Ok("Blentinel hub 9.0.0"), &[true]);
        let outcome = run(&hub, &machine).await.unwrap();
        assert!(matches!(outcome, Outcome::Upgraded { ref backup, .. } if backup.exists()));
        assert_eq!(std::fs::read(&hub.exe).unwrap(), b"new");
        assert_eq!(
            std::fs::read(hub.exe.with_extension("prev")).unwrap(),
            b"old"
        );
        assert!(!hub.prepared.staged.exists());
        assert_eq!(machine.log(), ["stop", "start"]);
    }

    #[tokio::test]
    async fn a_failed_self_test_changes_nothing() {
        let hub = hub().await;
        for machine in [
            Fake::new(Err("migration 0099 failed"), &[]),
            Fake::new(Ok("Blentinel hub 8.0.0"), &[]),
        ] {
            std::fs::write(&hub.prepared.staged, b"new").unwrap();
            assert!(run(&hub, &machine).await.is_err());
            assert_eq!(std::fs::read(&hub.exe).unwrap(), b"old");
            assert!(!hub.prepared.staged.exists(), "the new hub is deleted");
            assert!(machine.log().is_empty(), "the service wasn't touched");
        }
        let backups = std::fs::read_dir(hub.config.backups_dir()).map_or(0, |d| d.count());
        assert_eq!(backups, 0, "no backup either");
    }

    #[tokio::test]
    async fn a_hub_that_doesnt_run_is_rolled_back_with_its_database() {
        let hub = hub().await;
        // The new hub ruins the database as it starts, then doesn't run.
        let machine = Fake::new(Ok("Blentinel hub 9.0.0"), &[false, true]);
        *machine.ruin_on_start.borrow_mut() = Some(hub.config.server.resolved_db_path());
        let outcome = run(&hub, &machine).await.unwrap();
        let Outcome::RolledBack { backup, aside } = outcome else {
            panic!("expected a rollback");
        };
        assert!(backup.exists() && aside.exists());
        assert_eq!(std::fs::read(&hub.exe).unwrap(), b"old");
        assert_eq!(
            std::fs::read(hub.exe.with_extension("failed")).unwrap(),
            b"new"
        );
        assert_eq!(companies(&hub).await, 1, "the database from the backup");
        assert_eq!(machine.log(), ["stop", "start", "stop", "start"]);
    }

    #[tokio::test]
    async fn when_the_old_hub_doesnt_run_either_it_says_what_to_do() {
        let hub = hub().await;
        let machine = Fake::new(Ok("Blentinel hub 9.0.0"), &[false, false]);
        let error = run(&hub, &machine).await.unwrap_err().to_string();
        assert!(error.contains("hub restore"), "{error}");
        assert_eq!(std::fs::read(&hub.exe).unwrap(), b"old");
    }
}
