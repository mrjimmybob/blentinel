//! `probe doctor`: what's wrong with this probe, and how to fix it (D-53).
//! Reads only, and sends no report. Exits non-zero if anything fails.

use std::fmt::Write as _;
use std::path::Path;

use chrono::{DateTime, Utc};
use common::protocol::MAX_CLOCK_SKEW_SECS;
use common::service::ServiceState;

use super::platform;
use crate::transport::HubTransport;
use crate::{config, enrollment, identity, storage};

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub level: Level,
    pub what: String,
    pub fix: Option<String>,
}

impl Check {
    fn ok(what: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            what: what.into(),
            fix: None,
        }
    }
    fn warn(what: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            level: Level::Warn,
            what: what.into(),
            fix: Some(fix.into()),
        }
    }
    fn fail(what: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            level: Level::Fail,
            what: what.into(),
            fix: Some(fix.into()),
        }
    }
}

/// Clock difference worth a warning (the hub refuses at [`MAX_CLOCK_SKEW_SECS`]).
const SKEW_WARN_SECS: i64 = 60;

/// Every check, in order.
pub async fn run(dir: &Path) -> Vec<Check> {
    let mut checks = vec![
        service_check(&platform::service_state().await),
        folder_check(dir),
        identity_check(dir),
    ];
    let enrolled = match enrollment::load(dir) {
        Ok(Some(e)) => {
            checks.push(Check::ok(format!(
                "Enrolled with {} as \"{}\" ({}, {})",
                e.hub_url, e.probe_name, e.company_name, e.site_name
            )));
            Some(e)
        }
        Ok(None) => {
            checks.push(Check::fail(
                "Not enrolled",
                "get a code from the hub's Add probe and run `probe install --code <code>`",
            ));
            None
        }
        Err(e) => {
            checks.push(Check::fail(
                format!("The enrollment can't be read: {e:#}"),
                "enroll again: `probe install --code <code>`",
            ));
            None
        }
    };
    if let Some(e) = &enrolled {
        checks.extend(hub_checks(e).await);
    }
    checks.push(config_check(dir));
    checks.push(buffer_check(dir).await);
    checks
}

/// The findings as text, and the worst level.
pub fn report(checks: &[Check]) -> (String, Level) {
    let mut out = String::new();
    for check in checks {
        let tag = match check.level {
            Level::Ok => "OK  ",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        };
        let _ = writeln!(out, "[{tag}] {}", check.what);
        if let Some(fix) = &check.fix {
            let _ = writeln!(out, "       → {fix}");
        }
    }
    let worst = checks.iter().map(|c| c.level).max().unwrap_or(Level::Ok);
    (out, worst)
}

fn service_check(state: &ServiceState) -> Check {
    match state {
        ServiceState::Running => Check::ok("The service is running"),
        ServiceState::NotInstalled => Check::warn(
            "The probe isn't installed as a service",
            "run `probe install --code <code>` as root/Administrator (or run the probe yourself)",
        ),
        other => Check::fail(
            format!("The service is {other}"),
            if cfg!(windows) {
                "see blentinel_probe.log in the probe's folder; Start-Service BlentinelProbe"
            } else {
                "journalctl -u blentinel-probe"
            },
        ),
    }
}

fn folder_check(dir: &Path) -> Check {
    if !dir.is_dir() {
        return Check::fail(
            format!("Folder {} doesn't exist", dir.display()),
            "`probe install --code <code>` creates it",
        );
    }
    match platform::private(dir) {
        Ok(true) => Check::ok(format!("Folder {} is private", dir.display())),
        Ok(false) => Check::warn(
            format!(
                "Folder {} can be read by other users (the identity key)",
                dir.display()
            ),
            "`probe install` restricts it",
        ),
        Err(e) => Check::warn(
            format!("Folder permissions unknown: {e:#}"),
            "check them by hand",
        ),
    }
}

fn identity_check(dir: &Path) -> Check {
    match identity::probe_id_in(dir) {
        Ok(Some(id)) => Check::ok(format!("Identity {}…", &id[..8])),
        Ok(None) => Check::warn(
            "No identity yet",
            "enrollment creates it: `probe install --code <code>`",
        ),
        Err(e) => Check::fail(
            format!("The identity can't be read: {e:#}"),
            "run as root/Administrator; if it's damaged, enroll again (the probe gets a new ID)",
        ),
    }
}

/// The hub answers through the pinned connection, and its clock agrees.
async fn hub_checks(e: &enrollment::Enrollment) -> Vec<Check> {
    let transport = match e
        .tls_pin()
        .and_then(|pin| HubTransport::new(&e.hub_url, pin))
    {
        Ok(transport) => transport,
        Err(err) => {
            return vec![Check::fail(
                format!("The hub address can't be used: {err:#}"),
                "enroll again with a new code",
            )];
        }
    };
    match transport.hub_date().await {
        Ok(date) => vec![
            Check::ok(format!("The hub at {} answers", e.hub_url)),
            skew_check(date, Utc::now()),
        ],
        Err(err) => vec![Check::fail(
            format!("The hub at {} can't be reached: {err}", e.hub_url),
            "check DNS, the firewall (outbound to the hub's port) and that the hub runs; \
             a certificate error means the hub's certificate changed: enroll with a new code",
        )],
    }
}

/// How far this machine's clock is from the hub's.
pub fn skew_check(hub_time: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Check {
    let Some(hub_time) = hub_time else {
        return Check::warn(
            "The hub's clock is unknown (no Date header)",
            "check the clock by hand",
        );
    };
    let skew = (now - hub_time).num_seconds();
    if skew.abs() > MAX_CLOCK_SKEW_SECS {
        Check::fail(
            format!("This clock is {skew} s off the hub's: the hub refuses its reports"),
            "sync the clock (NTP)",
        )
    } else if skew.abs() > SKEW_WARN_SECS {
        Check::warn(
            format!("This clock is {skew} s off the hub's"),
            "sync the clock (NTP)",
        )
    } else {
        Check::ok("The clock agrees with the hub's")
    }
}

fn config_check(dir: &Path) -> Check {
    match config::ActiveConfig::load(dir).config {
        Some(c) if c.checks.is_empty() => Check::warn(
            "It has no checks",
            "add them on the hub: the probe's page (its name on the company's page)",
        ),
        Some(c) => Check::ok(format!(
            "{} checks, every {} s",
            c.checks.len(),
            c.interval_secs
        )),
        None => Check::warn(
            "No checks received yet",
            "the hub sends them with the answer to the next report",
        ),
    }
}

async fn buffer_check(dir: &Path) -> Check {
    match storage::BlackBox::pending_at(&dir.join(storage::FILE_NAME)).await {
        Ok(Some(n)) if n > 0 => Check::warn(
            format!("{n} reports wait for the hub"),
            "they're delivered once it's reachable again",
        ),
        Ok(_) => Check::ok("No reports waiting"),
        Err(e) => Check::warn(
            format!("The buffer can't be read: {e:#}"),
            "run as root/Administrator",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_skew_warns_after_a_minute_and_fails_past_the_hubs_limit() {
        let hub: DateTime<Utc> = "2026-09-30T10:00:00Z".parse().unwrap();
        let at = |secs| hub + chrono::Duration::seconds(secs);
        assert_eq!(skew_check(Some(hub), at(30)).level, Level::Ok);
        assert_eq!(skew_check(Some(hub), at(-90)).level, Level::Warn);
        assert_eq!(
            skew_check(Some(hub), at(MAX_CLOCK_SKEW_SECS + 1)).level,
            Level::Fail
        );
        assert_eq!(skew_check(None, hub).level, Level::Warn);
    }

    #[tokio::test]
    async fn an_empty_folder_reports_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let checks = run(dir.path()).await;
        let not_enrolled = checks.iter().find(|c| c.what == "Not enrolled").unwrap();
        assert_eq!(not_enrolled.level, Level::Fail);
        assert!(
            not_enrolled
                .fix
                .as_deref()
                .unwrap()
                .contains("probe install --code")
        );
        assert_eq!(report(&checks).1, Level::Fail);
    }

    #[tokio::test]
    async fn a_reachable_hub_is_found_through_its_address() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0u8; 4096];
                let _ = socket.read(&mut request).await;
                let date = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT");
                let head = format!(
                    "HTTP/1.1 405 Method Not Allowed\r\ndate: {date}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                );
                let _ = socket.write_all(head.as_bytes()).await;
            }
        });
        let transport = HubTransport::new(&url, None).unwrap();
        let date = transport.hub_date().await.unwrap();
        assert_eq!(skew_check(date, Utc::now()).level, Level::Ok);
    }
}
