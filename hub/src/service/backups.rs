//! Backups the hub makes by itself (M2-12, proposal 0004 §4.5, D-60): the
//! daily backup set on the admin page, "Back up now", and which files are
//! kept. The file format is `ops::backup` (the same as `hub backup`).
//!
//! A backup's kind is in its file name, so the folder is the only record of
//! what exists: files copied in or deleted by hand are seen as they are.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDateTime, NaiveTime, Utc};
use tokio::sync::RwLock;

use crate::clock::Clock;
use crate::config::HubConfig;
use crate::model::{BackupFile, BackupKind, BackupOverview, BackupSchedule};
use crate::ops;
use crate::state::Reloadable;
use crate::storage::{BackupRun, Storage};

/// Every backup's name starts with this, then `YYYYMMDD-HHMMSS`.
const PREFIX: &str = "blentinel-hub-";
const SUFFIX: &str = ".tar.gz";
const STAMP: &str = "%Y%m%d-%H%M%S";

/// Backups made before an upgrade that are kept: enough to step back a few
/// upgrades, without filling the disk over the years.
pub const KEEP_PRE_UPGRADE: usize = 3;

/// How often the scheduler looks whether a backup is due.
const SCHEDULER_TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// One backup at a time: they read the whole database and compress it.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0}")]
    Invalid(String),
    #[error("no backup named {0}")]
    NotFound(String),
    #[error("{0} already exists: a backup was made this second")]
    Exists(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        Self::Failed(e.into())
    }
}

fn kind_suffix(kind: BackupKind) -> &'static str {
    match kind {
        BackupKind::Manual => "",
        BackupKind::Scheduled => "-scheduled",
        BackupKind::PreUpgrade => "-pre-upgrade",
    }
}

/// The file name of a `kind` backup made at `at`.
pub fn file_name(kind: BackupKind, at: DateTime<Utc>) -> String {
    format!("{PREFIX}{}{}{SUFFIX}", at.format(STAMP), kind_suffix(kind))
}

/// When and why a backup was made, from its name; `None`: not a hub backup's
/// name (which also keeps any path out of the API's names).
pub fn parse_name(name: &str) -> Option<(DateTime<Utc>, BackupKind)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (stamp, tail) = rest.split_at_checked(15)?; // YYYYMMDD-HHMMSS
    let kind = [
        BackupKind::Manual,
        BackupKind::Scheduled,
        BackupKind::PreUpgrade,
    ]
    .into_iter()
    .find(|kind| kind_suffix(*kind) == tail)?;
    let at = NaiveDateTime::parse_from_str(stamp, STAMP).ok()?.and_utc();
    Some((at, kind))
}

/// Where a `kind` backup made at `now` goes.
pub fn path_for(config: &HubConfig, kind: BackupKind, now: DateTime<Utc>) -> PathBuf {
    config.backups_dir().join(file_name(kind, now))
}

/// The backups in `dir`, newest first (none if the folder doesn't exist).
pub fn list(dir: &Path) -> std::io::Result<Vec<BackupFile>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((at, kind)) = parse_name(&name) else {
            continue;
        };
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        found.push((
            at,
            BackupFile {
                name,
                kind,
                created_at: at.format("%Y-%m-%d %H:%M UTC").to_string(),
                size_bytes: metadata.len(),
            },
        ));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    Ok(found.into_iter().map(|(_, file)| file).collect())
}

/// The `kind` backups beyond the newest `keep`, to delete. `files` is newest
/// first, as [`list`] returns it.
pub fn to_prune(files: &[BackupFile], kind: BackupKind, keep: usize) -> Vec<String> {
    files
        .iter()
        .filter(|f| f.kind == kind)
        .skip(keep)
        .map(|f| f.name.clone())
        .collect()
}

/// Delete the `kind` backups beyond the newest `keep`. A file that can't be
/// deleted is logged and left: the next backup tries again.
pub fn prune(dir: &Path, kind: BackupKind, keep: usize) -> std::io::Result<()> {
    for name in to_prune(&list(dir)?, kind, keep) {
        match std::fs::remove_file(dir.join(&name)) {
            Ok(()) => tracing::info!(backup = %name, "Old backup deleted"),
            Err(e) => tracing::warn!(backup = %name, "Can't delete an old backup: {e}"),
        }
    }
    Ok(())
}

/// The latest time at or before `now` the daily backup was due: today at
/// `hour_utc`, or yesterday if that's still to come.
pub fn latest_slot(hour_utc: u8, now: DateTime<Utc>) -> DateTime<Utc> {
    let today = now.date_naive().and_time(NaiveTime::MIN).and_utc()
        + Duration::hours(i64::from(hour_utc.min(23)));
    if today <= now {
        today
    } else {
        today - Duration::days(1)
    }
}

/// Is the daily backup due? Yes if it hasn't run since its latest slot, so a
/// hub that was off at that hour catches up when it starts.
pub fn is_due(schedule: &BackupSchedule, last: Option<&BackupRun>, now: DateTime<Utc>) -> bool {
    schedule.enabled && last.is_none_or(|run| run.ran_at < latest_slot(schedule.hour_utc, now))
}

/// When the next scheduled backup runs (`now` if due); `None` if off.
pub fn next_run(
    schedule: &BackupSchedule,
    last: Option<&BackupRun>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if !schedule.enabled {
        return None;
    }
    if is_due(schedule, last, now) {
        return Some(now);
    }
    Some(latest_slot(schedule.hour_utc, now) + Duration::days(1))
}

/// The schedule in force: the one set on the admin page, or the default.
pub async fn schedule(store: &dyn Storage) -> anyhow::Result<BackupSchedule> {
    Ok(store.backup_schedule().await?.unwrap_or_default())
}

/// Set the schedule, if it's valid.
pub async fn set_schedule(
    store: &dyn Storage,
    schedule: &BackupSchedule,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), BackupError> {
    schedule.validate().map_err(BackupError::Invalid)?;
    store.set_backup_schedule(schedule, updated_by, now).await?;
    Ok(())
}

/// Make a `kind` backup now, then delete that kind's old ones (scheduled:
/// beyond the schedule's `keep`; before an upgrade: [`KEEP_PRE_UPGRADE`];
/// manual: never). Returns the new file.
pub async fn make(
    store: &dyn Storage,
    config: &HubConfig,
    config_path: &Path,
    kind: BackupKind,
    now: DateTime<Utc>,
) -> Result<BackupFile, BackupError> {
    let _one_at_a_time = RUNNING.lock().await;
    let out = path_for(config, kind, now);
    if out.exists() {
        return Err(BackupError::Exists(file_name(kind, now)));
    }
    ops::backup::backup(store, config, config_path, &out, now).await?;
    let keep = match kind {
        BackupKind::Manual => None,
        BackupKind::Scheduled => Some(schedule(store).await?.keep as usize),
        BackupKind::PreUpgrade => Some(KEEP_PRE_UPGRADE),
    };
    let dir = config.backups_dir();
    if let Some(keep) = keep {
        prune(&dir, kind, keep)?;
    }
    let name = file_name(kind, now);
    list(&dir)?
        .into_iter()
        .find(|f| f.name == name)
        .ok_or_else(|| BackupError::Failed(anyhow::anyhow!("{name} vanished once written")))
}

/// Make the scheduled backup if it's due, and record how it went. `Ok(None)`:
/// not due.
pub async fn run_if_due(
    store: &dyn Storage,
    config: &HubConfig,
    config_path: &Path,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<Result<BackupFile, String>>> {
    let schedule = schedule(store).await?;
    let last = store.last_backup_run().await?;
    if !is_due(&schedule, last.as_ref(), now) {
        return Ok(None);
    }
    let made = make(store, config, config_path, BackupKind::Scheduled, now)
        .await
        .map_err(|e| format!("{:#}", anyhow::Error::from(e)));
    // Recorded either way: a failure waits for the next day's slot (or
    // "Back up now") instead of retrying every minute; the page shows it.
    store
        .record_backup_run(&BackupRun {
            ran_at: now,
            error: made.as_ref().err().cloned(),
        })
        .await?;
    Ok(Some(made))
}

/// Everything the admin page shows about backups.
pub async fn overview(
    store: &dyn Storage,
    config: &HubConfig,
    now: DateTime<Utc>,
) -> Result<BackupOverview, BackupError> {
    let schedule = schedule(store).await?;
    let last = store.last_backup_run().await?;
    let dir = config.backups_dir();
    let format = |at: DateTime<Utc>| at.format("%Y-%m-%d %H:%M UTC").to_string();
    Ok(BackupOverview {
        schedule,
        folder: dir.display().to_string(),
        next_at: next_run(&schedule, last.as_ref(), now).map(format),
        last_run_at: last.as_ref().map(|run| format(run.ran_at)),
        last_error: last.and_then(|run| run.error),
        backups: list(&dir)?,
    })
}

/// Delete the backup called `name` from the backups folder.
pub fn delete(config: &HubConfig, name: &str) -> Result<(), BackupError> {
    if parse_name(name).is_none() {
        return Err(BackupError::NotFound(name.to_string()));
    }
    match std::fs::remove_file(config.backups_dir().join(name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(BackupError::NotFound(name.to_string()))
        }
        Err(e) => Err(e.into()),
    }
}

/// The path of the backup called `name`, if there is one.
pub fn find(config: &HubConfig, name: &str) -> Result<PathBuf, BackupError> {
    let path = config.backups_dir().join(name);
    if parse_name(name).is_some() && path.is_file() {
        Ok(path)
    } else {
        Err(BackupError::NotFound(name.to_string()))
    }
}

/// How long a download link works once the password was confirmed.
pub const DOWNLOAD_LINK_TTL: Duration = Duration::seconds(60);

/// One-time download links (D-60): issued once a user confirmed their
/// password, used once, by that user, within [`DOWNLOAD_LINK_TTL`]. In
/// memory: a restart forgets them, which only means asking again.
#[derive(Debug, Default)]
pub struct DownloadTickets {
    /// By the SHA-256 of the link's token (the token itself isn't kept).
    tickets: std::sync::Mutex<std::collections::HashMap<String, Ticket>>,
}

#[derive(Debug)]
struct Ticket {
    user_id: String,
    name: String,
    expires_at: DateTime<Utc>,
}

impl DownloadTickets {
    fn tickets(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Ticket>> {
        // Inserts and removes only: a poisoned lock is still consistent.
        self.tickets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A token that lets `user_id` download `name` once, until `now` + the TTL.
    pub fn issue(&self, user_id: &str, name: &str, now: DateTime<Utc>) -> String {
        let token = crate::service::users::random_string(32);
        let mut tickets = self.tickets();
        tickets.retain(|_, t| t.expires_at > now);
        tickets.insert(
            crate::service::users::session_hash(&token),
            Ticket {
                user_id: user_id.to_string(),
                name: name.to_string(),
                expires_at: now + DOWNLOAD_LINK_TTL,
            },
        );
        token
    }

    /// The backup `token` is for, if it's `user_id`'s and still valid. Used
    /// up either way.
    pub fn redeem(&self, token: &str, user_id: &str, now: DateTime<Utc>) -> Option<String> {
        let ticket = self
            .tickets()
            .remove(&crate::service::users::session_hash(token))?;
        (ticket.user_id == user_id && ticket.expires_at > now).then_some(ticket.name)
    }
}

/// Make the scheduled backup when it's due, checking every minute, for as
/// long as the hub runs.
pub fn spawn_scheduler(
    storage: Reloadable<Arc<dyn Storage>>,
    config: Arc<RwLock<HubConfig>>,
    config_path: PathBuf,
    clock: Arc<dyn Clock>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SCHEDULER_TICK).await;
            let config = config.read().await.clone(); // hot-reloadable
            // The current database: a reload may have switched it (D-38).
            let storage = storage.get();
            match run_if_due(storage.as_ref(), &config, &config_path, clock.now()).await {
                Ok(None) => {}
                Ok(Some(Ok(file))) => {
                    tracing::info!(backup = %file.name, "Scheduled backup written")
                }
                Ok(Some(Err(e))) => tracing::error!("Scheduled backup failed: {e}"),
                Err(e) => tracing::error!("Scheduled backup check failed: {e:#}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, h, m, 0).unwrap()
    }

    fn ran(at: DateTime<Utc>) -> BackupRun {
        BackupRun {
            ran_at: at,
            error: None,
        }
    }

    #[test]
    fn names_round_trip_with_their_kind() {
        for kind in [
            BackupKind::Manual,
            BackupKind::Scheduled,
            BackupKind::PreUpgrade,
        ] {
            let name = file_name(kind, at(3, 0));
            assert_eq!(parse_name(&name), Some((at(3, 0), kind)), "{name}");
        }
        assert_eq!(
            file_name(BackupKind::Scheduled, at(3, 0)),
            "blentinel-hub-20261006-030000-scheduled.tar.gz"
        );
    }

    #[test]
    fn other_names_are_not_backups() {
        for name in [
            "blentinel.db",
            "blentinel-hub-20261006-030000.tar",
            "blentinel-hub-20261006-030000-weekly.tar.gz",
            "blentinel-hub-2026100-030000.tar.gz",
            "../blentinel-hub-20261006-030000.tar.gz",
            "blentinel-hub-20261006-030000.tar.gz/../x",
            "blentinel-hub-20261306-030000.tar.gz",
        ] {
            assert_eq!(parse_name(name), None, "{name}");
        }
    }

    fn file(kind: BackupKind, day: u32) -> BackupFile {
        let at = Utc.with_ymd_and_hms(2026, 10, day, 3, 0, 0).unwrap();
        BackupFile {
            name: file_name(kind, at),
            kind,
            created_at: String::new(),
            size_bytes: 1,
        }
    }

    #[test]
    fn pruning_keeps_the_newest_of_one_kind_only() {
        // Newest first, as `list` returns them.
        let files = [
            file(BackupKind::Scheduled, 5),
            file(BackupKind::Manual, 4),
            file(BackupKind::Scheduled, 4),
            file(BackupKind::PreUpgrade, 3),
            file(BackupKind::Scheduled, 3),
            file(BackupKind::Manual, 1),
        ];
        assert_eq!(
            to_prune(&files, BackupKind::Scheduled, 2),
            vec![files[4].name.clone()]
        );
        assert!(to_prune(&files, BackupKind::Scheduled, 3).is_empty());
        assert!(to_prune(&files, BackupKind::PreUpgrade, 1).is_empty());
    }

    #[test]
    fn the_latest_slot_is_today_once_the_hour_has_come() {
        assert_eq!(latest_slot(3, at(3, 0)), at(3, 0));
        assert_eq!(latest_slot(3, at(10, 0)), at(3, 0));
        assert_eq!(latest_slot(3, at(2, 59)), at(3, 0) - Duration::days(1));
    }

    #[test]
    fn a_backup_is_due_once_a_day_and_catches_up_after_downtime() {
        let schedule = BackupSchedule::default(); // 03:00
        assert!(is_due(&schedule, None, at(1, 0)), "never ran: due now");
        assert!(!is_due(&schedule, Some(&ran(at(3, 0))), at(23, 0)));
        assert!(
            is_due(
                &schedule,
                Some(&ran(at(3, 0) - Duration::days(2))),
                at(9, 0)
            ),
            "the hub was off at 03:00: it runs when it's back"
        );
        assert!(!is_due(
            &schedule,
            Some(&ran(at(3, 1))),
            at(3, 1) + Duration::hours(23)
        ));
        assert!(is_due(
            &schedule,
            Some(&ran(at(3, 1))),
            at(3, 0) + Duration::days(1)
        ));
    }

    #[test]
    fn a_failed_run_counts_as_a_run() {
        let failed = BackupRun {
            ran_at: at(3, 0),
            error: Some("disk full".into()),
        };
        assert!(!is_due(&BackupSchedule::default(), Some(&failed), at(4, 0)));
    }

    #[test]
    fn next_run_follows_the_schedule() {
        let mut schedule = BackupSchedule::default();
        let last = ran(at(3, 0));
        assert_eq!(
            next_run(&schedule, Some(&last), at(9, 0)),
            Some(at(3, 0) + Duration::days(1))
        );
        assert_eq!(next_run(&schedule, None, at(9, 0)), Some(at(9, 0)));
        schedule.hour_utc = 20; // moved later the same day: tonight
        assert_eq!(next_run(&schedule, Some(&last), at(9, 0)), Some(at(20, 0)));
        schedule.enabled = false;
        assert_eq!(next_run(&schedule, Some(&last), at(9, 0)), None);
    }

    #[test]
    fn a_download_link_works_once_for_its_user_within_a_minute() {
        let tickets = DownloadTickets::default();
        let name = "blentinel-hub-20261006-030000.tar.gz";
        let token = tickets.issue("u1", name, at(3, 0));
        assert_eq!(
            tickets.redeem(&token, "u1", at(3, 0)),
            Some(name.to_string())
        );
        assert_eq!(tickets.redeem(&token, "u1", at(3, 0)), None, "once");

        let token = tickets.issue("u1", name, at(3, 0));
        assert_eq!(tickets.redeem(&token, "u2", at(3, 0)), None, "another user");
        assert_eq!(
            tickets.redeem(&token, "u1", at(3, 0)),
            None,
            "used up anyway"
        );

        let token = tickets.issue("u1", name, at(3, 0));
        assert_eq!(tickets.redeem(&token, "u1", at(3, 1)), None, "expired");
        assert_eq!(tickets.redeem("made-up", "u1", at(3, 0)), None);
    }

    #[test]
    fn schedules_are_validated() {
        let ok = BackupSchedule::default();
        assert!(ok.validate().is_ok());
        assert!(BackupSchedule { hour_utc: 24, ..ok }.validate().is_err());
        assert!(BackupSchedule { keep: 0, ..ok }.validate().is_err());
        assert!(
            BackupSchedule {
                keep: BackupSchedule::MAX_KEEP + 1,
                ..ok
            }
            .validate()
            .is_err()
        );
    }
}
