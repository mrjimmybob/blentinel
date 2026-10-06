//! `hub doctor`: what's wrong with this hub, and how to fix it (D-52).
//! Reads only. Exits non-zero if anything fails.

use std::fmt::Write as _;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use crate::config;
use crate::storage;

use super::{ServiceState, answers, local_address, platform, ports};

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
    /// What to do about it (for warnings and failures).
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

/// Free space below which the database is at risk.
const DISK_FAIL_BYTES: u64 = 500 * 1024 * 1024;
const DISK_WARN_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Certificate lifetime left that deserves a warning; less for a short-lived
/// certificate (a quarter of its life: one from an ACME authority is renewed
/// with a third left, D-62, so only a late renewal warns).
const CERT_WARN_DAYS: i64 = 30;

/// Every check, in order. Stops after the config if it can't be read.
pub async fn run(config_path: &Path) -> Vec<Check> {
    let config = match config::load_from(config_path) {
        Ok(config) => config,
        Err(e) => {
            return vec![Check::fail(
                format!("Config {}: {e}", config_path.display()),
                "fix the file, or point to it with --config",
            )];
        }
    };
    let mut checks = vec![Check::ok(format!(
        "Config {} is valid",
        config_path.display()
    ))];
    let service = platform::service_state();
    checks.push(service_check(&service));
    checks.push(state_dir_check(&config.server.state_dir));
    checks.push(key_check(&config.server.resolved_identity_key_path()));
    checks.push(database_check(&config.server.resolved_db_path()).await);
    if config.server.tls.enabled {
        checks.push(certificate_check(
            &config.server.resolve_path(&config.server.tls.cert_path),
            time::OffsetDateTime::now_utc(),
        ));
    }
    let running = service == ServiceState::Running;
    for (what, port) in ports(&config) {
        checks.push(port_check(&config.server.host, port, what, running));
    }
    checks.push(disk_check(&config.server.state_dir));
    checks
}

/// The findings as text, worst level.
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
            "The hub isn't installed as a service",
            "run `hub install` as root/Administrator (or start the hub yourself)",
        ),
        other => Check::fail(
            format!("The service is {other}"),
            if cfg!(windows) {
                "see hub.log in the data folder; Start-Service BlentinelHub"
            } else {
                "journalctl -u blentinel-hub; sudo systemctl start blentinel-hub"
            },
        ),
    }
}

/// Exists, writable, and closed to other users.
pub fn state_dir_check(dir: &Path) -> Check {
    if !dir.is_dir() {
        return Check::fail(
            format!("Data folder {} doesn't exist", dir.display()),
            "`hub install` creates it, or create it and give the hub's user access",
        );
    }
    let probe = dir.join(".doctor-write-test");
    if let Err(e) = std::fs::write(&probe, b"") {
        return Check::fail(
            format!("Data folder {} can't be written: {e}", dir.display()),
            "run `hub doctor` as root/Administrator, or give the hub's user access",
        );
    }
    let _ = std::fs::remove_file(&probe);
    match private(dir) {
        Ok(true) => Check::ok(format!("Data folder {} is private", dir.display())),
        Ok(false) => Check::warn(
            format!(
                "Data folder {} can be read by other users (keys, database)",
                dir.display()
            ),
            "`hub install` restricts it (Linux: chmod 700; Windows: SYSTEM and Administrators only)",
        ),
        Err(e) => Check::warn(
            format!("Data folder permissions unknown: {e}"),
            "check them by hand",
        ),
    }
}

#[cfg(target_os = "linux")]
fn private(dir: &Path) -> anyhow::Result<bool> {
    super::linux::private_mode(dir)
}
#[cfg(windows)]
fn private(dir: &Path) -> anyhow::Result<bool> {
    Ok(super::windows::admins_only(&super::windows::dacl_sddl(
        dir,
    )?))
}
#[cfg(not(any(target_os = "linux", windows)))]
fn private(dir: &Path) -> anyhow::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    Ok(std::fs::metadata(dir)?.permissions().mode() & 0o077 == 0)
}

/// The hub's identity key: there (or created at the first start), 32 bytes.
pub fn key_check(path: &Path) -> Check {
    if !path.exists() {
        return Check::warn(
            format!("Identity key {} doesn't exist yet", path.display()),
            "the hub creates it at its first start; if it had one, restore it (`hub restore`): probes trust that key",
        );
    }
    match common::fs::load_key(path) {
        Ok(_) => Check::ok(format!("Identity key {} is readable", path.display())),
        Err(e) => Check::fail(
            format!("Identity key {}: {e}", path.display()),
            "restore it from a backup (`hub restore`); a new key means enrolling every probe again",
        ),
    }
}

/// Opens, is consistent, and isn't from a newer hub.
pub async fn database_check(path: &Path) -> Check {
    if !path.exists() {
        return Check::warn(
            format!("Database {} doesn't exist yet", path.display()),
            "the hub creates it at its first start",
        );
    }
    let store = match storage::open_read_only(path).await {
        Ok(store) => store,
        Err(e) => {
            return Check::fail(
                format!("Database {} can't be opened: {e:#}", path.display()),
                "check its permissions, or restore a backup (`hub restore`)",
            );
        }
    };
    let (schema, latest) = (
        store.schema_version().await.unwrap_or(0),
        storage::latest_schema_version(),
    );
    if schema > latest {
        return Check::fail(
            format!("Database schema {schema} is newer than this hub's ({latest})"),
            "run the newer hub that made it, or restore an older backup",
        );
    }
    match store.check_integrity().await {
        Ok(Ok(())) if schema < latest => Check::ok(format!(
            "Database is consistent (schema {schema}; updated to {latest} at the next start)"
        )),
        Ok(Ok(())) => Check::ok(format!("Database is consistent (schema {schema})")),
        Ok(Err(problems)) => Check::fail(
            format!("Database is damaged: {problems}"),
            "restore a backup (`hub restore`)",
        ),
        Err(e) => Check::fail(format!("Database check failed: {e:#}"), "restore a backup"),
    }
}

/// The HTTPS certificate: readable and not about to expire.
pub fn certificate_check(path: &Path, now: time::OffsetDateTime) -> Check {
    let Ok(pem) = std::fs::read(path) else {
        return Check::warn(
            format!("TLS certificate {} doesn't exist yet", path.display()),
            "the hub creates a self-signed one at its first start",
        );
    };
    const RENEW: &str = "renew it on the admin page: Renew now (HTTPS certificate) if an ACME \
                         authority issues it, or rotate it (Hub keys); probes follow either way";
    let details = crate::tls::certificate_details(&pem);
    match details
        .as_ref()
        .map(|d| (d.not_after, d.lifetime().num_days()))
    {
        Ok((not_after, lifetime_days)) => {
            let expires =
                time::OffsetDateTime::from_unix_timestamp(not_after.timestamp()).unwrap_or(now);
            let days_left = (expires - now).whole_days();
            if expires <= now {
                Check::fail(
                    format!("TLS certificate expired on {}", expires.date()),
                    RENEW,
                )
            } else if days_left < CERT_WARN_DAYS.min(lifetime_days / 4) {
                Check::warn(
                    format!(
                        "TLS certificate expires in {days_left} days ({})",
                        expires.date()
                    ),
                    RENEW,
                )
            } else {
                Check::ok(format!("TLS certificate valid until {}", expires.date()))
            }
        }
        Err(e) => Check::fail(
            format!("TLS certificate {}: {e:#}", path.display()),
            "replace the file with a PEM certificate",
        ),
    }
}

/// With the hub running, the port answers; without it, the port is free.
pub fn port_check(host: &str, port: u16, what: &str, hub_running: bool) -> Check {
    let label = format!("Port {port} ({what})");
    let answering = answers(host, port);
    match (hub_running, answering) {
        (true, true) => Check::ok(format!("{label} answers")),
        (true, false) => Check::fail(
            format!("{label}: the service runs but nothing answers"),
            "see the hub's log",
        ),
        (false, true) => Check::warn(
            format!("{label} is taken while the service isn't running"),
            "a hub started by hand, or another program: stop it before starting the service",
        ),
        (false, false) => {
            let bind = local_address(host, port).map(|mut addr| {
                if host == "0.0.0.0" || host.is_empty() {
                    addr.set_ip(std::net::Ipv4Addr::UNSPECIFIED.into());
                }
                TcpListener::bind(addr)
            });
            match bind {
                Some(Ok(_)) => Check::ok(format!("{label} is free")),
                Some(Err(e)) => Check::fail(
                    format!("{label} can't be used: {e}"),
                    "choose another port in the config, or run as a user allowed to use it",
                ),
                None => Check::fail(
                    format!("{label}: host {host:?} doesn't resolve"),
                    "fix `host` in the config",
                ),
            }
        }
    }
}

/// Free space where the data lives.
pub fn disk_check(state_dir: &Path) -> Check {
    let dir = state_dir
        .canonicalize()
        .map(without_verbatim_prefix)
        .unwrap_or_else(|_| PathBuf::from(state_dir));
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .iter()
        .filter(|d| dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    let Some(disk) = disk else {
        return Check::warn("Free disk space unknown", "check it by hand");
    };
    let free = disk.available_space();
    let text = format!(
        "{:.1} GB free on {}",
        free as f64 / 1_073_741_824.0,
        disk.mount_point().display()
    );
    if free < DISK_FAIL_BYTES {
        Check::fail(text, "free space, or archive old data (Admin page)")
    } else if free < DISK_WARN_BYTES {
        Check::warn(text, "free space soon, or archive old data (Admin page)")
    } else {
        Check::ok(text)
    }
}

/// Windows' `canonicalize` gives `\\?\C:\…`, which never starts with a
/// disk's mount point `C:\`: drop the prefix (not from UNC paths).
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with("UNC") => PathBuf::from(plain),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_is_found_for_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_ne!(disk_check(dir.path()).what, "Free disk space unknown");
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(r"\\?\C:\data")),
            PathBuf::from(r"C:\data")
        );
        let unc = PathBuf::from(r"\\?\UNC\server\share");
        assert_eq!(without_verbatim_prefix(unc.clone()), unc);
    }

    #[test]
    fn the_worst_finding_decides() {
        let (text, worst) = report(&[
            Check::ok("fine"),
            Check::warn("hmm", "look"),
            Check::ok("fine too"),
        ]);
        assert_eq!(worst, Level::Warn);
        assert!(text.contains("[WARN] hmm\n       → look"), "{text}");
        assert_eq!(report(&[]).1, Level::Ok);
    }

    #[test]
    fn a_missing_or_short_key_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("hub_identity.key");
        assert_eq!(key_check(&key).level, Level::Warn);
        std::fs::write(&key, b"short").unwrap();
        assert_eq!(key_check(&key).level, Level::Fail);
        std::fs::write(&key, [1u8; 32]).unwrap();
        assert_eq!(key_check(&key).level, Level::Ok);
    }

    #[test]
    fn a_missing_data_folder_fails_and_a_writable_one_passes_the_write_test() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(state_dir_check(&dir.path().join("nope")).level, Level::Fail);
        // A temp folder is writable; whether it's private depends on the OS.
        assert_ne!(state_dir_check(dir.path()).level, Level::Fail);
    }

    #[test]
    fn certificates_warn_when_renewal_is_late_and_fail_when_expired() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
        assert_eq!(
            certificate_check(&cert, time::OffsetDateTime::now_utc()).level,
            Level::Warn
        );
        crate::tls::load_or_create_tls_cert(
            cert.to_str().unwrap(),
            key.to_str().unwrap(),
            "hub.example.com",
        )
        .unwrap();
        let expires = crate::tls::cert_not_after(&std::fs::read(&cert).unwrap()).unwrap();
        let at = |days: i64| expires - time::Duration::days(days);
        assert_eq!(certificate_check(&cert, at(365)).level, Level::Ok);
        assert_eq!(certificate_check(&cert, at(29)).level, Level::Warn);
        assert_eq!(certificate_check(&cert, at(-1)).level, Level::Fail);
    }

    #[test]
    fn a_90_day_certificate_warns_only_once_its_renewal_is_late() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("c.pem");
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["hub.example.com".into()]).unwrap();
        let issued = time::OffsetDateTime::now_utc();
        params.not_before = issued;
        params.not_after = issued + time::Duration::days(90);
        std::fs::write(&cert, params.self_signed(&key).unwrap().pem()).unwrap();
        let expires = issued + time::Duration::days(90);
        let at = |days: i64| expires - time::Duration::days(days);
        // Renewed with 30 days left (a third): no warning then...
        assert_eq!(certificate_check(&cert, at(30)).level, Level::Ok);
        assert_eq!(certificate_check(&cert, at(23)).level, Level::Ok);
        // ...only once the renewal is about a week late.
        assert_eq!(certificate_check(&cert, at(21)).level, Level::Warn);
    }

    #[test]
    fn a_free_port_passes_and_a_taken_one_is_explained() {
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        assert_eq!(port_check("127.0.0.1", port, "x", true).level, Level::Ok);
        assert_eq!(port_check("127.0.0.1", port, "x", false).level, Level::Warn);
        drop(taken);
        assert_eq!(port_check("127.0.0.1", port, "x", false).level, Level::Ok);
        assert_eq!(port_check("127.0.0.1", port, "x", true).level, Level::Fail);
    }

    #[tokio::test]
    async fn a_database_from_a_newer_hub_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub.db");
        assert_eq!(
            database_check(&path).await.level,
            Level::Warn,
            "not created yet"
        );
        storage::open(&path).await.unwrap();
        assert_eq!(database_check(&path).await.level, Level::Ok);

        let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
             VALUES (9999, 'future', CURRENT_TIMESTAMP, 1, x'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let check = database_check(&path).await;
        assert_eq!(check.level, Level::Fail, "{check:?}");
        assert!(check.what.contains("newer"));
    }
}
