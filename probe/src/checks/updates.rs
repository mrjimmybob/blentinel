//! The `updates` check (M5-11, D-89): the updates the probe's machine waits
//! for, as its own system knows them. How many, how many of them security
//! updates, whether the machine must restart to finish one, and when the
//! system last looked.
//!
//! **The probe only reads** (D-59): it installs nothing, downloads nothing
//! and doesn't make the system look for updates; the system's own timers do
//! that. It runs no tool (FP-2): Windows is asked through the Windows Update
//! Agent's API and the registry, Linux from the file Ubuntu's
//! `update-notifier` writes, else through PackageKit over D-Bus where it is
//! installed.
//!
//! Reading the list is slow (seconds on Windows; PackageKit is a service
//! started for it), so it has its own pace: once an hour ([`SEARCH_EVERY`]),
//! whatever the check's interval. Whether a restart is needed is read at
//! every run: it costs nothing, and it should clear as the machine comes
//! back.

use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use common::checks::{Metric, metric};

use super::Outcome;

/// How often the system's list of updates is read.
const SEARCH_EVERY: Duration = Duration::from_secs(60 * 60);
/// How soon it is read again after a failure.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// How long a run of the check waits for a reading under way, before it
/// answers with what it knew.
const SEARCH_WAIT: Duration = Duration::from_secs(20);
/// The longest a reading may take.
const SEARCH_LIMIT: Duration = Duration::from_secs(10 * 60);

const SECONDS_PER_DAY: f64 = 86_400.0;

/// What the system's list of updates says.
#[derive(Debug, Clone, PartialEq)]
struct Found {
    /// Updates that wait to be installed.
    updates: u32,
    /// How many of them are security updates.
    security: u32,
    /// Since when the oldest of the security updates has been there to
    /// install, where the system says (Windows: the day it was released).
    security_since: Option<SystemTime>,
    /// When the system last looked for updates, where it says.
    looked: Option<SystemTime>,
}

/// Whether the machine must restart to finish an update.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Restart {
    needed: bool,
    /// Since when, where the system says.
    since: Option<SystemTime>,
}

/// What a run of the check has to go by.
#[derive(Debug, Clone, PartialEq)]
enum Listed {
    /// The list, and since when security updates have waited.
    Known(Found, Option<SystemTime>),
    /// The list can't be read: why.
    Failed(String),
    /// The first reading isn't done yet.
    Searching,
}

#[derive(Default)]
struct Memory {
    /// The last reading, and when it ended.
    last: Option<(Instant, Result<Found, String>)>,
    searching: bool,
    /// When this probe first saw security updates waiting: for the systems
    /// that don't say since when. Forgotten when the probe restarts.
    security_seen: Option<SystemTime>,
}

impl Memory {
    fn due(&self, every: Duration, retry: Duration) -> bool {
        match &self.last {
            None => true,
            Some((at, Ok(_))) => at.elapsed() >= every,
            Some((at, Err(_))) => at.elapsed() >= retry,
        }
    }

    fn note(&mut self, found: Result<Found, String>, now: SystemTime) {
        match &found {
            Ok(found) if found.security > 0 => {
                self.security_seen.get_or_insert(now);
            }
            Ok(_) => self.security_seen = None,
            // A failed reading says nothing about what waits.
            Err(_) => {}
        }
        self.last = Some((Instant::now(), found));
        self.searching = false;
    }

    fn listed(&self) -> Listed {
        match &self.last {
            Some((_, Ok(found))) => Listed::Known(
                found.clone(),
                found
                    .security_since
                    .or(self.security_seen)
                    .filter(|_| found.security > 0),
            ),
            Some((_, Err(why))) => Listed::Failed(why.clone()),
            None => Listed::Searching,
        }
    }
}

/// The list of updates, read at its own pace and remembered between runs.
struct Watch {
    memory: Mutex<Memory>,
    every: Duration,
    retry: Duration,
}

impl Watch {
    fn new(every: Duration, retry: Duration) -> Arc<Self> {
        Arc::new(Self {
            memory: Mutex::new(Memory::default()),
            every,
            retry,
        })
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, Memory> {
        // A reading that panicked must not end the check for good.
        self.memory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// What is known of the list. Starts a reading (`search`) when one is
    /// due and none runs, and waits up to `wait` for it.
    async fn list<F, Fut>(self: &Arc<Self>, search: F, wait: Duration) -> Listed
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Found, String>> + Send + 'static,
    {
        {
            let mut memory = self.memory();
            if memory.due(self.every, self.retry) && !memory.searching {
                memory.searching = true;
                let watch = Arc::clone(self);
                tokio::spawn(async move {
                    // In a task of its own: whatever happens to it, the
                    // reading ends and the next one can start.
                    let found = tokio::time::timeout(SEARCH_LIMIT, tokio::spawn(search())).await;
                    let found = match found {
                        Ok(Ok(found)) => found,
                        Ok(Err(_)) => Err("reading the system's updates failed".to_string()),
                        Err(_) => Err(format!(
                            "reading the system's updates didn't end within {} minutes",
                            SEARCH_LIMIT.as_secs() / 60
                        )),
                    };
                    watch.memory().note(found, SystemTime::now());
                });
            }
        }
        let deadline = Instant::now() + wait;
        loop {
            {
                let memory = self.memory();
                if !memory.searching || Instant::now() >= deadline {
                    return memory.listed();
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

static WATCH: LazyLock<Arc<Watch>> = LazyLock::new(|| Watch::new(SEARCH_EVERY, RETRY_AFTER));

/// What the machine's system says of its updates.
pub async fn check() -> Outcome {
    let restart = platform::restart();
    let listed = WATCH.list(platform::search, SEARCH_WAIT).await;
    describe(&listed, restart, SystemTime::now())
}

/// How long ago `since` was; no time for one ahead of the clock.
fn elapsed(since: SystemTime, now: SystemTime) -> Duration {
    now.duration_since(since).unwrap_or_default()
}

/// A length of time in days, to one decimal.
fn days(time: Duration) -> f64 {
    (time.as_secs_f64() / SECONDS_PER_DAY * 10.0).round() / 10.0
}

/// A length of time in words: `40 minutes`, `5 hours`, `3 days`.
fn ago(time: Duration) -> String {
    let count = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    let seconds = time.as_secs();
    if seconds >= 2 * 86_400 {
        count(seconds / 86_400, "day")
    } else if seconds >= 3600 {
        count(seconds / 3600, "hour")
    } else {
        count(seconds / 60, "minute")
    }
}

fn describe(listed: &Listed, restart: Option<Restart>, now: SystemTime) -> Outcome {
    let mut metrics = Vec::new();
    let mut parts = Vec::new();
    let mut failed = None;
    match listed {
        Listed::Known(found, security_since) => {
            metrics.push(Metric::new(metric::UPDATES, f64::from(found.updates), ""));
            metrics.push(Metric::new(metric::SECURITY, f64::from(found.security), ""));
            parts.push(match (found.updates, found.security) {
                (0, _) => "No updates wait".to_string(),
                (1, 0) => "1 update waits".to_string(),
                (1, _) => "1 security update waits".to_string(),
                (n, 0) => format!("{n} updates wait"),
                (n, s) if n == s => format!("{n} security updates wait"),
                (n, s) => format!("{n} updates wait, {s} of them security updates"),
            });
            if found.security > 0 {
                let days = security_since.map_or(0.0, |since| days(elapsed(since, now)));
                metrics.push(Metric::new(metric::SECURITY_DAYS, days, metric::DAYS));
            }
        }
        Listed::Failed(why) => failed = Some(format!("Can't tell which updates wait: {why}")),
        Listed::Searching => parts.push("Reading the system's updates for the first time".into()),
    }
    if let Some(restart) = restart {
        metrics.push(Metric::new(
            metric::REBOOT,
            if restart.needed { 1.0 } else { 0.0 },
            "",
        ));
        if restart.needed {
            let waited = restart.since.map(|since| elapsed(since, now));
            metrics.push(Metric::new(
                metric::REBOOT_DAYS,
                waited.map_or(0.0, days),
                metric::DAYS,
            ));
            parts.push(match waited {
                Some(waited) if waited.as_secs() >= 86_400 => {
                    format!("a restart has been needed for {}", ago(waited))
                }
                _ => "a restart is needed".to_string(),
            });
        }
    }
    if let Listed::Known(found, _) = listed
        && let Some(looked) = found.looked
    {
        let since = elapsed(looked, now);
        metrics.push(Metric::new(metric::LOOKED_DAYS, days(since), metric::DAYS));
        parts.push(format!("the system last looked {} ago", ago(since)));
    }
    let outcome = match failed {
        Some(mut why) => {
            for part in &parts {
                why.push_str("; ");
                why.push_str(part);
            }
            Outcome::down(why)
        }
        None => Outcome::up(parts.join("; ")),
    };
    outcome.with_metrics(metrics)
}

/// What Ubuntu's `update-notifier` wrote in `updates-available`: how many
/// updates, and how many of them security updates. `None`: not that text
/// (the file is written in the system's language; only English is read).
#[cfg(any(target_os = "linux", test))]
fn parse_updates_available(text: &str) -> Option<(u32, u32)> {
    let number_before = |marker: &str| {
        text.lines().find_map(|line| {
            let (number, rest) = line.trim().split_once(' ')?;
            rest.starts_with(marker)
                .then(|| number.parse::<u32>().ok())?
        })
    };
    let updates =
        number_before("updates can be applied").or_else(|| number_before("update can be applied"));
    let security = number_before("of these updates are standard security")
        .or_else(|| number_before("of these updates is a standard security"))
        .or_else(|| number_before("of these updates are security"))
        .or_else(|| number_before("of these updates is a security"));
    Some((updates?, security.unwrap_or(0)))
}

/// PackageKit's word for a package of its list of updates (`info`, as
/// `PkInfoEnum`): a security update, another update, or one that isn't
/// offered now (held back). Newer versions carry a severity in the upper
/// half of the number.
#[cfg(any(target_os = "linux", test))]
fn packagekit_update(info: u32) -> Option<bool> {
    const SECURITY: u32 = 8;
    const BLOCKED: u32 = 9;
    let (kind, severity) = (info & 0xFFFF, info >> 16);
    if kind == BLOCKED {
        None
    } else {
        Some(kind == SECURITY || severity == SECURITY)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use zbus::export::ordered_stream::OrderedStreamExt;
    use zbus::zvariant::OwnedObjectPath;

    use super::{Found, Restart, packagekit_update, parse_updates_available};

    /// Written by Ubuntu's `update-notifier-common` after every `apt
    /// update` and package change.
    const UPDATES_AVAILABLE: &str = "/var/lib/update-notifier/updates-available";
    /// Touched by Debian and Ubuntu when an installed package wants the
    /// machine restarted.
    const REBOOT_REQUIRED: &str = "/run/reboot-required";
    /// There on the systems that write [`REBOOT_REQUIRED`].
    const DPKG_STATUS: &str = "/var/lib/dpkg/status";
    /// What `apt update` touches when it ends well, and the lists it writes.
    const APT_LOOKED: [&str; 2] = [
        "/var/lib/apt/periodic/update-success-stamp",
        "/var/lib/apt/lists",
    ];
    /// PackageKit's number for "refresh the lists" (`PK_ROLE_ENUM_REFRESH_CACHE`).
    const ROLE_REFRESH_CACHE: u32 = 13;
    /// PackageKit's "no filter" (`PK_FILTER_ENUM_NONE`, as a bit).
    const FILTER_NONE: u64 = 1 << 1;
    /// PackageKit's "it ended well" (`PK_EXIT_ENUM_SUCCESS`).
    const EXIT_SUCCESS: u32 = 1;
    /// PackageKit answers "never" with the largest number; a time beyond
    /// this isn't one.
    const LONGEST_SINCE: u32 = 10 * 365 * 86_400;

    #[zbus::proxy(
        interface = "org.freedesktop.PackageKit",
        default_service = "org.freedesktop.PackageKit",
        default_path = "/org/freedesktop/PackageKit"
    )]
    trait PackageKit {
        fn create_transaction(&self) -> zbus::Result<OwnedObjectPath>;
        fn get_time_since_action(&self, role: u32) -> zbus::Result<u32>;
    }

    #[zbus::proxy(
        interface = "org.freedesktop.PackageKit.Transaction",
        default_service = "org.freedesktop.PackageKit"
    )]
    trait Transaction {
        fn get_updates(&self, filter: u64) -> zbus::Result<()>;
    }

    fn modified(path: &str) -> Option<SystemTime> {
        std::fs::metadata(path).ok()?.modified().ok()
    }

    pub fn restart() -> Option<Restart> {
        // Only Debian's family says so in a file anyone may read.
        if !Path::new(DPKG_STATUS).exists() {
            return None;
        }
        let since = modified(REBOOT_REQUIRED);
        Some(Restart {
            needed: since.is_some() || Path::new(REBOOT_REQUIRED).exists(),
            since,
        })
    }

    pub async fn search() -> Result<Found, String> {
        let looked = APT_LOOKED.iter().filter_map(|path| modified(path)).max();
        // Ubuntu's own count first: it is what the system would install
        // now. PackageKit also counts the updates Ubuntu hands out to a
        // part of its machines at a time, before this one's turn; and
        // reading a file wakes no service.
        let written = std::fs::read_to_string(UPDATES_AVAILABLE).ok();
        if let Some((updates, security)) = written.as_deref().and_then(parse_updates_available) {
            return Ok(Found {
                updates,
                security,
                security_since: None,
                // The file is as old as what it says.
                looked: looked.max(modified(UPDATES_AVAILABLE)),
            });
        }
        match packagekit().await {
            Ok((updates, security, refreshed)) => Ok(Found {
                updates,
                security,
                security_since: None,
                looked: looked.max(refreshed),
            }),
            Err(why) => Err(format!(
                "this system keeps no list of its updates that the probe can read \
                 (no update-notifier file; PackageKit: {why})"
            )),
        }
    }

    /// PackageKit's list of updates: how many, how many of them security
    /// updates, and when it last refreshed its lists (if it ever did). It
    /// reads what the system already downloaded; nothing is refreshed.
    async fn packagekit() -> Result<(u32, u32, Option<SystemTime>), String> {
        let short = |e: zbus::Error| match e {
            zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("ServiceUnknown") => {
                "not installed".to_string()
            }
            e => e.to_string(),
        };
        let connection = zbus::Connection::system()
            .await
            .map_err(|_| "the probe isn't let onto the system bus".to_string())?;
        let daemon = PackageKitProxy::new(&connection).await.map_err(short)?;
        let path = daemon.create_transaction().await.map_err(short)?;
        let transaction = TransactionProxy::builder(&connection)
            .path(path)
            .map_err(short)?
            .build()
            .await
            .map_err(short)?;
        // Listening before asking: the answers come as signals.
        let mut signals = transaction
            .inner()
            .receive_all_signals()
            .await
            .map_err(short)?;
        transaction.get_updates(FILTER_NONE).await.map_err(short)?;

        let (mut updates, mut security) = (0, 0);
        let mut error = None;
        while let Some(message) = signals.next().await {
            let header = message.header();
            let body = message.body();
            match header.member().map(|member| member.as_str()) {
                Some("Package") => {
                    if let Ok((info, _id, _summary)) = body.deserialize::<(u32, &str, &str)>()
                        && let Some(is_security) = packagekit_update(info)
                    {
                        updates += 1;
                        security += u32::from(is_security);
                    }
                }
                Some("ErrorCode") => {
                    if let Ok((_code, details)) = body.deserialize::<(u32, &str)>() {
                        error = Some(details.lines().next().unwrap_or_default().to_string());
                    }
                }
                Some("Finished") => {
                    let exit = body.deserialize::<(u32, u32)>().map(|(exit, _)| exit);
                    if exit == Ok(EXIT_SUCCESS) {
                        break;
                    }
                    return Err(error.unwrap_or_else(|| "its search failed".to_string()));
                }
                _ => {}
            }
        }
        let refreshed = daemon
            .get_time_since_action(ROLE_REFRESH_CACHE)
            .await
            .ok()
            .filter(|seconds| *seconds < LONGEST_SINCE)
            .and_then(|seconds| {
                SystemTime::now().checked_sub(Duration::from_secs(u64::from(seconds)))
            });
        Ok((updates, security, refreshed))
    }
}

/// A date as Windows' automation writes it (days since 1899-12-30, UTC):
/// `None` for "never" (zero) and anything before 1970.
#[cfg(any(windows, test))]
fn from_ole_date(date: f64) -> Option<SystemTime> {
    const DAYS_TO_1970: f64 = 25_569.0;
    let seconds = (date - DAYS_TO_1970) * SECONDS_PER_DAY;
    (seconds.is_finite() && seconds > 0.0)
        .then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(seconds))
}

/// A Windows file time (100 ns since 1601): `None` before 1970.
#[cfg(any(windows, test))]
fn from_file_time(ticks: u64) -> Option<SystemTime> {
    const TICKS_TO_1970: u64 = 116_444_736_000_000_000;
    let since = ticks
        .checked_sub(TICKS_TO_1970)
        .filter(|since| *since > 0)?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(since.saturating_mul(100)))
}

#[cfg(windows)]
mod platform {
    use std::time::SystemTime;

    use windows::Win32::Foundation::{ERROR_SUCCESS, FILETIME, VARIANT_FALSE};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_WOW64_64KEY, RegCloseKey, RegOpenKeyExW,
        RegQueryInfoKeyW,
    };
    use windows::Win32::System::UpdateAgent::{
        AutomaticUpdates, IAutomaticUpdates2, IUpdate, IUpdateSession, UpdateSession,
    };
    use windows::core::{BSTR, PCWSTR, PWSTR, w};

    use super::{Found, Restart, from_file_time, from_ole_date};

    /// What Windows would install by itself: not installed, not hidden by
    /// someone, and not one of the optional ones (drivers, previews).
    const WAITING: &str = "IsInstalled=0 and IsHidden=0 and BrowseOnly=0";
    /// A VARIANT that holds a date.
    const VT_DATE: u16 = 7;
    /// The category "Security Updates", as Windows Update numbers it.
    const SECURITY_UPDATES: &str = "0fa1201d-4330-4fa8-8ae9-b877473b6441";
    /// The keys that are there while Windows waits for a restart: for
    /// Windows Update, and for its servicing of components.
    const RESTART_KEYS: [PCWSTR; 2] = [
        w!(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate\\Auto Update\\RebootRequired"
        ),
        w!(
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Component Based Servicing\\RebootPending"
        ),
    ];

    /// When the registry key at `path` was last written. `None`: no such key.
    fn key_written(path: PCWSTR) -> Option<Option<SystemTime>> {
        let mut key = HKEY::default();
        let mut written = FILETIME::default();
        // SAFETY: plain registry calls with valid pointers to locals; the
        // key opened is closed before returning.
        unsafe {
            let opened = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                path,
                0,
                KEY_QUERY_VALUE | KEY_WOW64_64KEY,
                &mut key,
            );
            if opened != ERROR_SUCCESS {
                return None;
            }
            let asked = RegQueryInfoKeyW(
                key,
                PWSTR::null(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&mut written),
            );
            let _ = RegCloseKey(key);
            if asked != ERROR_SUCCESS {
                return Some(None);
            }
        }
        let ticks = (u64::from(written.dwHighDateTime) << 32) | u64::from(written.dwLowDateTime);
        Some(from_file_time(ticks))
    }

    pub fn restart() -> Option<Restart> {
        let waiting: Vec<Option<SystemTime>> =
            RESTART_KEYS.into_iter().filter_map(key_written).collect();
        Some(Restart {
            needed: !waiting.is_empty(),
            // The longest wait.
            since: waiting.into_iter().flatten().min(),
        })
    }

    pub async fn search() -> Result<Found, String> {
        // The Windows Update Agent's calls block, for seconds.
        tokio::task::spawn_blocking(|| {
            // SAFETY: COM is set up for this thread before it is used, and
            // closed after, once every COM object made in `ask` is dropped.
            unsafe {
                let started = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                let found = ask().map_err(|e| format!("Windows Update says: {}", e.message()));
                if started {
                    CoUninitialize();
                }
                found
            }
        })
        .await
        .unwrap_or_else(|_| Err("asking Windows Update failed".to_string()))
    }

    /// Ask the Windows Update Agent what it already knows: no search on the
    /// network (`Online` off), nothing downloaded.
    unsafe fn ask() -> windows::core::Result<Found> {
        unsafe {
            let session: IUpdateSession =
                CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER)?;
            let searcher = session.CreateUpdateSearcher()?;
            searcher.SetOnline(VARIANT_FALSE)?;
            let updates = searcher.Search(&BSTR::from(WAITING))?.Updates()?;
            let count = updates.Count()?;
            let mut security = 0;
            let mut security_since: Option<SystemTime> = None;
            for index in 0..count {
                let update = updates.get_Item(index)?;
                if is_security(&update) {
                    security += 1;
                    let released = update
                        .LastDeploymentChangeTime()
                        .ok()
                        .and_then(from_ole_date);
                    security_since = match (security_since, released) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
            }
            Ok(Found {
                updates: count.max(0) as u32,
                security,
                security_since,
                looked: looked(),
            })
        }
    }

    /// A security update: Microsoft rates it, or files it under "Security
    /// Updates" (the monthly cumulative updates are).
    unsafe fn is_security(update: &IUpdate) -> bool {
        unsafe {
            if update
                .MsrcSeverity()
                .is_ok_and(|severity| !severity.is_empty())
            {
                return true;
            }
            let Ok(categories) = update.Categories() else {
                return false;
            };
            (0..categories.Count().unwrap_or(0)).any(|index| {
                categories
                    .get_Item(index)
                    .and_then(|category| category.CategoryID())
                    .is_ok_and(|id| id.to_string().eq_ignore_ascii_case(SECURITY_UPDATES))
            })
        }
    }

    /// When Windows last looked for updates and reached the service.
    unsafe fn looked() -> Option<SystemTime> {
        unsafe {
            let automatic: IAutomaticUpdates2 =
                CoCreateInstance(&AutomaticUpdates, None, CLSCTX_INPROC_SERVER).ok()?;
            let date = automatic.Results().ok()?.LastSearchSuccessDate().ok()?;
            // A date (`VT_DATE`), or nothing when it never looked.
            let raw = date.as_raw().Anonymous.Anonymous;
            (raw.vt == VT_DATE)
                .then(|| from_ole_date(raw.Anonymous.date))
                .flatten()
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::{Found, Restart};

    pub fn restart() -> Option<Restart> {
        None
    }

    pub async fn search() -> Result<Found, String> {
        Err("updates checks work on Windows and on Linux".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;
    use std::sync::atomic::{AtomicU32, Ordering};

    const DAY: Duration = Duration::from_secs(86_400);

    fn found(updates: u32, security: u32) -> Found {
        Found {
            updates,
            security,
            security_since: None,
            looked: None,
        }
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value)
    }

    #[test]
    fn the_message_says_what_waits_and_the_numbers_go_with_it() {
        let now = SystemTime::now();
        let known = |found: Found, since: Option<SystemTime>, restart| {
            describe(&Listed::Known(found, since), restart, now)
        };
        let fine = known(
            Found {
                looked: Some(now - Duration::from_secs(5 * 3600)),
                ..found(0, 0)
            },
            None,
            Some(Restart {
                needed: false,
                since: None,
            }),
        );
        assert_eq!(fine.status, Health::Up);
        assert_eq!(
            fine.message,
            "No updates wait; the system last looked 5 hours ago"
        );
        assert_eq!(value(&fine, metric::UPDATES), Some(0.0));
        assert_eq!(value(&fine, metric::SECURITY), Some(0.0));
        assert_eq!(value(&fine, metric::REBOOT), Some(0.0));
        assert_eq!(value(&fine, metric::LOOKED_DAYS), Some(0.2));
        assert_eq!(value(&fine, metric::SECURITY_DAYS), None);
        assert_eq!(value(&fine, metric::REBOOT_DAYS), None);

        let waiting = known(
            Found {
                looked: Some(now - 20 * DAY),
                ..found(7, 2)
            },
            Some(now - 9 * DAY),
            Some(Restart {
                needed: true,
                since: Some(now - 3 * DAY),
            }),
        );
        // The probe only reports: the hub's limits make a warning of it.
        assert_eq!(waiting.status, Health::Up);
        assert_eq!(
            waiting.message,
            "7 updates wait, 2 of them security updates; a restart has been needed for 3 days; \
             the system last looked 20 days ago"
        );
        assert_eq!(value(&waiting, metric::UPDATES), Some(7.0));
        assert_eq!(value(&waiting, metric::SECURITY), Some(2.0));
        assert_eq!(value(&waiting, metric::SECURITY_DAYS), Some(9.0));
        assert_eq!(value(&waiting, metric::REBOOT), Some(1.0));
        assert_eq!(value(&waiting, metric::REBOOT_DAYS), Some(3.0));
        assert_eq!(value(&waiting, metric::LOOKED_DAYS), Some(20.0));

        // The wordings of the counts; a restart since an unknown time, and
        // a system that doesn't say whether it needs one.
        for (updates, security, text) in [
            (1, 0, "1 update waits"),
            (1, 1, "1 security update waits"),
            (4, 0, "4 updates wait"),
            (4, 4, "4 security updates wait"),
        ] {
            assert_eq!(known(found(updates, security), None, None).message, text);
        }
        let unknown_since = known(
            found(0, 0),
            None,
            Some(Restart {
                needed: true,
                since: None,
            }),
        );
        assert_eq!(
            unknown_since.message,
            "No updates wait; a restart is needed"
        );
        assert_eq!(value(&unknown_since, metric::REBOOT_DAYS), Some(0.0));
        let silent = known(found(2, 1), None, None);
        assert_eq!(value(&silent, metric::REBOOT), None);
        // Security updates seen only now have waited no time.
        assert_eq!(value(&silent, metric::SECURITY_DAYS), Some(0.0));
    }

    #[test]
    fn a_list_that_cant_be_read_is_down_and_the_first_reading_is_not() {
        let now = SystemTime::now();
        let restart = Some(Restart {
            needed: true,
            since: None,
        });
        let failed = describe(&Listed::Failed("no list".into()), restart, now);
        assert_eq!(failed.status, Health::Down);
        assert_eq!(
            failed.message,
            "Can't tell which updates wait: no list; a restart is needed"
        );
        // What is known all the same is reported.
        assert_eq!(value(&failed, metric::REBOOT), Some(1.0));
        assert_eq!(value(&failed, metric::UPDATES), None);

        let first = describe(&Listed::Searching, None, now);
        assert_eq!(first.status, Health::Up);
        assert_eq!(
            first.message,
            "Reading the system's updates for the first time"
        );
        assert!(first.metrics.is_empty());
    }

    #[test]
    fn times_are_told_in_the_unit_that_fits() {
        let seconds = Duration::from_secs;
        assert_eq!(ago(seconds(0)), "0 minutes");
        assert_eq!(ago(seconds(119)), "1 minute");
        assert_eq!(ago(seconds(43 * 60)), "43 minutes");
        assert_eq!(ago(seconds(3600)), "1 hour");
        assert_eq!(ago(DAY * 3 / 2), "36 hours");
        assert_eq!(ago(DAY * 2), "2 days");
        assert_eq!(ago(DAY * 31 - seconds(1)), "30 days");
        let now = SystemTime::now();
        assert_eq!(days(elapsed(now - DAY * 3 / 2, now)), 1.5);
        assert_eq!(days(seconds(5 * 3600)), 0.2);
        // A time ahead of the clock (a clock set back) is no time ago.
        assert_eq!(elapsed(now + DAY, now), Duration::ZERO);
    }

    #[tokio::test]
    async fn the_list_is_read_at_its_own_pace_and_remembered() {
        let watch = Watch::new(Duration::from_millis(150), Duration::from_millis(150));
        let readings = Arc::new(AtomicU32::new(0));
        let read = |updates: u32| {
            let readings = Arc::clone(&readings);
            move || async move {
                readings.fetch_add(1, Ordering::SeqCst);
                Ok(found(updates, 0))
            }
        };
        let wait = Duration::from_secs(2);
        let known = |updates| Listed::Known(found(updates, 0), None);
        assert_eq!(watch.list(read(3), wait).await, known(3));
        // Asked again at once: what was read, not a new reading.
        assert_eq!(watch.list(read(4), wait).await, known(3));
        assert_eq!(readings.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(watch.list(read(5), wait).await, known(5));
        assert_eq!(readings.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_slow_reading_doesnt_hold_the_check_and_what_was_known_stands() {
        let watch = Watch::new(Duration::from_millis(100), Duration::from_millis(100));
        let slow = |updates: u32| {
            move || async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(found(updates, 0))
            }
        };
        let short = Duration::from_millis(30);
        // The first reading isn't done: the check says so, and doesn't
        // start a second one beside it.
        assert_eq!(watch.list(slow(1), short).await, Listed::Searching);
        assert_eq!(watch.list(slow(2), short).await, Listed::Searching);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            watch.list(slow(3), short).await,
            Listed::Known(found(1, 0), None)
        );
        // Due again, and slow again: the last reading stands meanwhile.
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(
            watch.list(slow(4), short).await,
            Listed::Known(found(1, 0), None)
        );
    }

    #[tokio::test]
    async fn a_failed_or_broken_reading_is_said_and_tried_again() {
        let watch = Watch::new(Duration::from_secs(60), Duration::from_millis(80));
        let wait = Duration::from_secs(2);
        let failed = watch
            .list(|| async { Err("no list".to_string()) }, wait)
            .await;
        assert_eq!(failed, Listed::Failed("no list".into()));
        tokio::time::sleep(Duration::from_millis(120)).await;
        // A reading that panics ends too, and the next one runs.
        #[allow(unreachable_code)]
        let broken = watch
            .list(
                || async {
                    panic!("a broken reading");
                    Ok(found(0, 0))
                },
                wait,
            )
            .await;
        assert_eq!(
            broken,
            Listed::Failed("reading the system's updates failed".into())
        );
        tokio::time::sleep(Duration::from_millis(120)).await;
        let read = watch.list(|| async { Ok(found(2, 0)) }, wait).await;
        assert_eq!(read, Listed::Known(found(2, 0), None));
    }

    #[test]
    fn security_updates_wait_since_the_system_says_or_since_the_probe_saw_them() {
        let start = SystemTime::now() - 10 * DAY;
        let mut memory = Memory::default();
        memory.note(Ok(found(3, 1)), start);
        assert_eq!(memory.listed(), Listed::Known(found(3, 1), Some(start)));
        // Still waiting later, more of them: since the first time.
        memory.note(Ok(found(5, 2)), start + 4 * DAY);
        assert_eq!(memory.listed(), Listed::Known(found(5, 2), Some(start)));
        // A failed reading says nothing; the time isn't forgotten for it.
        memory.note(Err("busy".into()), start + 5 * DAY);
        assert_eq!(memory.listed(), Listed::Failed("busy".into()));
        memory.note(Ok(found(5, 2)), start + 6 * DAY);
        assert_eq!(memory.listed(), Listed::Known(found(5, 2), Some(start)));
        // Installed: forgotten, and the next ones start anew.
        memory.note(Ok(found(1, 0)), start + 7 * DAY);
        assert_eq!(memory.listed(), Listed::Known(found(1, 0), None));
        memory.note(Ok(found(2, 1)), start + 8 * DAY);
        assert_eq!(
            memory.listed(),
            Listed::Known(found(2, 1), Some(start + 8 * DAY))
        );
        // The system's own date counts before the probe's.
        let released = Found {
            security_since: Some(start - 30 * DAY),
            ..found(2, 1)
        };
        memory.note(Ok(released.clone()), start + 9 * DAY);
        assert_eq!(
            memory.listed(),
            Listed::Known(released, Some(start - 30 * DAY))
        );
    }

    #[test]
    fn ubuntus_file_and_packagekits_numbers_are_read() {
        let ubuntu = "Expanded Security Maintenance for Applications is not enabled.\n\n\
            12 updates can be applied immediately.\n\
            5 of these updates are standard security updates.\n\
            To see these additional updates run: apt list --upgradable\n\n\
            Enable ESM Apps to receive additional future security updates.\n";
        assert_eq!(parse_updates_available(ubuntu), Some((12, 5)));
        assert_eq!(
            parse_updates_available(
                "\n1 update can be applied immediately.\n\
                 1 of these updates is a standard security update.\n"
            ),
            Some((1, 1))
        );
        assert_eq!(
            parse_updates_available("0 updates can be applied immediately.\n"),
            Some((0, 0))
        );
        // Older wording, and a count without security updates.
        assert_eq!(
            parse_updates_available(
                "3 updates can be applied immediately.\n2 of these updates are security updates.\n"
            ),
            Some((3, 2))
        );
        assert_eq!(
            parse_updates_available("4 updates can be applied immediately.\n"),
            Some((4, 0))
        );
        // Another language, or another file: not read as "none".
        assert_eq!(
            parse_updates_available("12 Aktualisierungen können sofort angewendet werden.\n"),
            None
        );
        assert_eq!(parse_updates_available(""), None);

        // PackageKit: normal, security, blocked, and a severity above.
        assert_eq!(packagekit_update(5), Some(false));
        assert_eq!(packagekit_update(8), Some(true));
        assert_eq!(packagekit_update(9), None);
        assert_eq!(packagekit_update(5 | (8 << 16)), Some(true));
        assert_eq!(packagekit_update(6 | (3 << 16)), Some(false));
    }

    #[test]
    fn windows_dates_become_times() {
        let unix = |time: SystemTime| {
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        // 2026-10-12 00:00 UTC is day 46307 of Windows' automation dates.
        assert_eq!(from_ole_date(46_307.0).map(unix), Some(1_791_763_200));
        assert_eq!(from_ole_date(46_307.5).map(unix), Some(1_791_806_400));
        // "Never" is zero (1899).
        assert_eq!(from_ole_date(0.0), None);
        assert_eq!(from_ole_date(f64::NAN), None);
        assert_eq!(
            from_file_time(116_444_736_000_000_000 + 10_000_000).map(unix),
            Some(1)
        );
        assert_eq!(from_file_time(0), None);
    }

    /// The real thing, by hand (`--ignored --nocapture`): what this machine
    /// says of its updates.
    #[tokio::test]
    #[ignore = "asks this machine's own system: run by hand"]
    async fn this_machine_says_what_it_waits_for() {
        let start = Instant::now();
        let restart = platform::restart();
        let found = platform::search().await;
        println!("restart: {restart:?}");
        println!("found in {:.1?}: {found:?}", start.elapsed());
        let listed = match found {
            Ok(found) => Listed::Known(found, None),
            Err(why) => Listed::Failed(why),
        };
        let outcome = describe(&listed, restart, SystemTime::now());
        println!("{:?}: {}", outcome.status, outcome.message);
        for metric in &outcome.metrics {
            println!("  {} = {} {}", metric.key, metric.value, metric.unit);
        }
        if let Ok(expected) = std::env::var("UPDATES_TEST_EXPECT") {
            // "updates,security", as the system's own tool counts them.
            let counted = format!(
                "{},{}",
                value(&outcome, metric::UPDATES).unwrap_or(-1.0),
                value(&outcome, metric::SECURITY).unwrap_or(-1.0)
            );
            assert_eq!(counted, expected);
        }
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
    }
}
