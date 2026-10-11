//! The `service` check (M5-06, D-83): is a service of the probe's machine
//! running? A Windows service, asked of the Service Control Manager; a
//! systemd unit, asked of systemd over D-Bus. Both natively (FP-2): no
//! `sc`, no `systemctl`. The probe only asks: it never starts or stops
//! anything (proposal 0006 #3, D-59).
//!
//! Where the probe isn't let onto the system bus (see
//! [`platform::from_files`]), it reads what systemd writes about its running
//! services instead.

use super::Outcome;

/// Is the service called `name` running?
pub async fn check(name: &str) -> Outcome {
    platform::check(name).await
}

/// What a systemd unit's states are worth. `load`: `loaded`, `not-found`,
/// `masked`, …; `active`: `active`, `inactive`, `failed`, `activating`, …;
/// `sub`: the detail (`running`, `dead`, `exited`).
#[cfg(any(target_os = "linux", test))]
fn systemd_outcome(unit: &str, load: &str, active: &str, sub: &str) -> Outcome {
    match (load, active) {
        ("not-found", _) => Outcome::down(format!("No unit named {unit}")),
        ("masked", _) => Outcome::down(format!("{unit} is masked (it can't be started)")),
        (_, "active" | "reloading") => Outcome::up(format!("{unit} is {active} ({sub})")),
        _ => Outcome::down(format!("{unit} is {active} ({sub})")),
    }
}

/// A unit as systemd names it: `nginx` is `nginx.service`.
#[cfg(any(target_os = "linux", test))]
fn unit_name(name: &str) -> String {
    if name.contains('.') {
        name.to_string()
    } else {
        format!("{name}.service")
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::Path;

    use super::{Outcome, systemd_outcome, unit_name};

    /// Exists where systemd runs the machine (as `sd_booted()` checks).
    const SYSTEMD_RUNS: &str = "/run/systemd/system";
    /// Where systemd keeps a link per active unit that has run something,
    /// named `invocation:<unit>`: there from its start until it stops or
    /// fails. Anyone may read it.
    const ACTIVE_UNITS: &str = "/run/systemd/units";
    /// Where unit files are, to tell a stopped unit from a name nobody has.
    const UNIT_FOLDERS: [&str; 8] = [
        "/etc/systemd/system",
        "/run/systemd/system",
        "/run/systemd/transient",
        "/run/systemd/generator",
        "/run/systemd/generator.late",
        "/usr/local/lib/systemd/system",
        "/usr/lib/systemd/system",
        "/lib/systemd/system",
    ];

    pub async fn check(name: &str) -> Outcome {
        let unit = unit_name(name);
        match crate::ops::unit_states(&unit).await {
            Ok(Some(states)) => systemd_outcome(&unit, &states.load, &states.active, &states.sub),
            Ok(None) if Path::new(SYSTEMD_RUNS).exists() => {
                from_files(&unit, Path::new(ACTIVE_UNITS), &UNIT_FOLDERS)
            }
            Ok(None) => Outcome::down(format!(
                "Can't ask about {unit}: this machine doesn't run systemd"
            )),
            Err(e) => Outcome::down(format!("Can't ask systemd about {unit}: {e:#}")),
        }
    }

    /// A service's state without asking systemd: from the files it writes.
    ///
    /// For a probe that can't open the system bus. That was every installed
    /// probe on Debian and Ubuntu before D-84: its unit ran it as a
    /// throwaway user (`DynamicUser`), and their bus daemon, dbus-daemon,
    /// turns such users away (it can't look them up). A probe that updated
    /// itself still runs under that unit until `probe install` is run again.
    /// systemd's link for each active service says as much as the check
    /// needs: running or not. Only services: a timer or a mount has no such
    /// link, and says so.
    pub(super) fn from_files(unit: &str, active_units: &Path, unit_folders: &[&str]) -> Outcome {
        if !unit.ends_with(".service") {
            return Outcome::down(format!(
                "Can't tell whether {unit} is active: this probe's account isn't let onto \
                 the system bus, and without it only services can be read \
                 (run `probe install` again on this machine)"
            ));
        }
        // The link itself, wherever it points.
        let active = active_units
            .join(format!("invocation:{unit}"))
            .symlink_metadata()
            .is_ok();
        if active {
            return Outcome::up(format!("{unit} is active"));
        }
        // An instance (`postgresql@16-main.service`) comes from its template.
        let template = unit
            .split_once('@')
            .map(|(name, _)| format!("{name}@.service"));
        let known = unit_folders.iter().any(|folder| {
            let folder = Path::new(folder);
            folder.join(unit).exists() || template.as_ref().is_some_and(|t| folder.join(t).exists())
        });
        if known {
            Outcome::down(format!("{unit} is not active"))
        } else {
            Outcome::down(format!("No unit named {unit}"))
        }
    }
}

/// What the Service Control Manager says of a service: is it running, and
/// the state in words.
#[cfg(any(windows, test))]
fn windows_outcome(name: &str, running: bool, state: &str) -> Outcome {
    if running {
        Outcome::up(format!("{name} is {state}"))
    } else {
        Outcome::down(format!("{name} is {state}"))
    }
}

#[cfg(windows)]
mod platform {
    use windows_service::service::{ServiceAccess, ServiceState};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    use super::{Outcome, windows_outcome};

    /// Windows error codes: no such service; not allowed.
    const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
    const ERROR_ACCESS_DENIED: i32 = 5;

    pub async fn check(name: &str) -> Outcome {
        let asked = name.to_string();
        // The Service Control Manager's calls block.
        tokio::task::spawn_blocking(move || query(&asked))
            .await
            .unwrap_or_else(|_| Outcome::down("Asking the Service Control Manager failed"))
    }

    fn query(name: &str) -> Outcome {
        let status = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .and_then(|manager| manager.open_service(name, ServiceAccess::QUERY_STATUS))
            .and_then(|service| service.query_status());
        match status {
            Ok(status) => {
                let state = match status.current_state {
                    ServiceState::Running => "running",
                    ServiceState::Stopped => "stopped",
                    ServiceState::StartPending => "starting",
                    ServiceState::StopPending => "stopping",
                    ServiceState::Paused => "paused",
                    ServiceState::PausePending => "pausing",
                    ServiceState::ContinuePending => "resuming",
                };
                windows_outcome(name, status.current_state == ServiceState::Running, state)
            }
            Err(windows_service::Error::Winapi(e)) => match e.raw_os_error() {
                Some(ERROR_SERVICE_DOES_NOT_EXIST) => Outcome::down(format!(
                    "No service named {name} (use its service name, not its display name)"
                )),
                Some(ERROR_ACCESS_DENIED) => {
                    Outcome::down(format!("The probe may not ask about {name}"))
                }
                _ => Outcome::down(format!("Can't ask about {name}: {e}")),
            },
            Err(e) => Outcome::down(format!("Can't ask about {name}: {e}")),
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::Outcome;

    pub async fn check(_name: &str) -> Outcome {
        Outcome::down("Service checks work on Windows and on Linux with systemd")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    #[test]
    fn a_systemd_unit_is_up_while_it_is_active() {
        let judged = |load, active, sub| {
            let outcome = systemd_outcome("nginx.service", load, active, sub);
            (outcome.status, outcome.message)
        };
        assert_eq!(
            judged("loaded", "active", "running"),
            (Health::Up, "nginx.service is active (running)".into())
        );
        // A service that ran and stays "active" without a process.
        assert_eq!(judged("loaded", "active", "exited").0, Health::Up);
        assert_eq!(judged("loaded", "reloading", "reload").0, Health::Up);
        assert_eq!(
            judged("loaded", "failed", "failed"),
            (Health::Down, "nginx.service is failed (failed)".into())
        );
        assert_eq!(judged("loaded", "inactive", "dead").0, Health::Down);
        assert_eq!(judged("loaded", "activating", "start").0, Health::Down);
        assert_eq!(
            judged("not-found", "inactive", "dead"),
            (Health::Down, "No unit named nginx.service".into())
        );
        assert!(judged("masked", "inactive", "dead").1.contains("masked"));
    }

    #[test]
    fn a_unit_without_a_suffix_is_a_service() {
        assert_eq!(unit_name("nginx"), "nginx.service");
        assert_eq!(
            unit_name("postgresql@16-main"),
            "postgresql@16-main.service"
        );
        assert_eq!(unit_name("backup.timer"), "backup.timer");
    }

    #[test]
    fn a_windows_service_is_up_only_while_it_runs() {
        let running = windows_outcome("Spooler", true, "running");
        assert_eq!(
            (running.status, running.message.as_str()),
            (Health::Up, "Spooler is running")
        );
        assert_eq!(
            windows_outcome("Spooler", false, "stopped").status,
            Health::Down
        );
        assert_eq!(
            windows_outcome("Spooler", false, "starting").message,
            "Spooler is starting"
        );
    }

    /// The real Service Control Manager: the event log always runs, and
    /// nothing has this name.
    #[cfg(windows)]
    #[tokio::test]
    async fn this_machines_services_are_asked() {
        let event_log = check("EventLog").await;
        assert_eq!(event_log.status, Health::Up, "{}", event_log.message);
        assert_eq!(event_log.message, "EventLog is running");

        let none = check("Blentinel No Such Service").await;
        assert_eq!(none.status, Health::Down);
        assert!(
            none.message.starts_with("No service named"),
            "{}",
            none.message
        );
    }

    /// The real systemd, where there is one to ask.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn this_machines_units_are_asked() {
        let none = check("blentinel-no-such-unit").await;
        assert_eq!(none.status, Health::Down);
        // Where systemd runs the machine (this folder says so), it must
        // answer, also to an account without rights (the probe's own
        // user, or the throwaway one of an older unit): the unit is unknown. Elsewhere (a build
        // container) the check says it can't ask.
        let systemd_runs = std::path::Path::new("/run/systemd/system").exists();
        let answered = none.message.starts_with("No unit named");
        assert!(
            answered || (!systemd_runs && none.message.starts_with("Can't ask")),
            "{}",
            none.message
        );
        // Where systemd answers, its journal is a unit that always runs.
        if none.message.starts_with("No unit named") {
            let journal = check("systemd-journald").await;
            assert_eq!(journal.status, Health::Up, "{}", journal.message);
            // "active (running)" from systemd itself; "active" from its
            // files, where this account isn't let onto the bus.
            assert!(
                journal
                    .message
                    .starts_with("systemd-journald.service is active"),
                "{}",
                journal.message
            );
        }
    }

    /// What systemd's files say of a service, in folders made for the test.
    #[cfg(target_os = "linux")]
    #[test]
    fn without_the_bus_a_service_is_read_from_systemds_files() {
        let dir = tempfile::tempdir().unwrap();
        let (links, units) = (dir.path().join("units"), dir.path().join("system"));
        std::fs::create_dir_all(&links).unwrap();
        std::fs::create_dir_all(&units).unwrap();
        for unit in ["nginx.service", "stopped.service", "postgresql@.service"] {
            std::fs::write(units.join(unit), "[Service]\n").unwrap();
        }
        // As systemd makes them: a link to the invocation's ID, not to a file.
        for unit in ["nginx.service", "postgresql@16-main.service"] {
            let link = links.join(format!("invocation:{unit}"));
            std::os::unix::fs::symlink("03c7d635925f4695bc3c6d6bc73f816f", link).unwrap();
        }
        let folders = [units.to_str().unwrap()];
        let read = |unit: &str| {
            let outcome = platform::from_files(unit, &links, &folders);
            (outcome.status, outcome.message)
        };

        assert_eq!(
            read("nginx.service"),
            (Health::Up, "nginx.service is active".into())
        );
        assert_eq!(read("postgresql@16-main.service").0, Health::Up);
        assert_eq!(
            read("stopped.service"),
            (Health::Down, "stopped.service is not active".into())
        );
        // An instance that isn't running, of a template that exists.
        assert_eq!(
            read("postgresql@17-main.service").1,
            "postgresql@17-main.service is not active"
        );
        assert_eq!(
            read("ngnix.service"),
            (Health::Down, "No unit named ngnix.service".into())
        );
        let timer = read("backup.timer");
        assert_eq!(timer.0, Health::Down);
        assert!(
            timer.1.starts_with("Can't tell whether backup.timer"),
            "{}",
            timer.1
        );
    }
}
