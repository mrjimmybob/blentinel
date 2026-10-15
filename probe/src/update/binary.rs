//! The probe's own executable: fetching a new one, testing it, switching to
//! it, and back (proposal 0012 §5.3–5.6).
//!
//! Every file stays in the probe's folder, so a switch is a rename on one
//! file system. Both Linux and Windows let a running executable be renamed;
//! Windows doesn't let it be replaced or deleted, so there the running file
//! is moved aside first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use common::release::{Artifact, Manifest};

use crate::transport::HubTransport;

/// How long a download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How long `--self-test` may take.
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(60);
/// `--self-test`'s exit code when the binary works but the hub can't be
/// reached: worth another try later, unlike any other failure.
pub const SELF_TEST_HUB_UNREACHABLE: i32 = 2;

/// The executable and the files beside it the updater uses: `probe.new`,
/// `probe.prev`, `probe.failed` (`probe.new.exe`, … on Windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binaries {
    pub exe: PathBuf,
    pub new: PathBuf,
    pub prev: PathBuf,
    pub failed: PathBuf,
}

impl Binaries {
    pub fn of(exe: &Path) -> Self {
        let beside = |tag: &str| {
            let stem = exe.file_stem().unwrap_or_default().to_string_lossy();
            let name = match exe.extension() {
                Some(ext) => format!("{stem}.{tag}.{}", ext.to_string_lossy()),
                None => format!("{stem}.{tag}"),
            };
            exe.with_file_name(name)
        };
        Self {
            exe: exe.to_path_buf(),
            new: beside("new"),
            prev: beside("prev"),
            failed: beside("failed"),
        }
    }

    /// The running executable's files.
    pub fn current() -> Result<Self> {
        Ok(Self::of(
            &std::env::current_exe().context("can't tell where this program is")?,
        ))
    }
}

/// Why fetching or testing a new version failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    /// Might work next time (network, hub down): try again later.
    Retry(String),
    /// Would fail again (a broken binary): give up on this version.
    GiveUp(String),
}

/// Download the binary for `artifact` into `bins.new`, check it against the
/// signed manifest, and run its self-test. Nothing is replaced yet.
pub async fn fetch_and_test(
    transport: &HubTransport,
    manifest: &Manifest,
    artifact: &Artifact,
    bins: &Binaries,
) -> Result<(), Attempt> {
    let path = format!("/download/releases/{}/{}", manifest.version, artifact.file);
    let sha256 = transport
        .download(&path, &bins.new, artifact.size, DOWNLOAD_TIMEOUT)
        .await
        .map_err(|e| {
            discard(&bins.new);
            Attempt::Retry(format!("download failed: {e:#}"))
        })?;
    if !sha256.eq_ignore_ascii_case(&artifact.sha256) {
        discard(&bins.new);
        return Err(Attempt::Retry(format!(
            "the download's SHA-256 ({sha256}) isn't the signed one"
        )));
    }
    make_executable(&bins.new).map_err(|e| Attempt::GiveUp(format!("{e:#}")))?;
    self_test(&bins.new, &manifest.version)
        .await
        .inspect_err(|_| discard(&bins.new))
}

fn discard(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("can't make {} executable", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// Run `exe --self-test`: it must say it is `version` and exit 0 in time.
pub async fn self_test(exe: &Path, version: &str) -> Result<(), Attempt> {
    let run = tokio::process::Command::new(exe)
        .arg("--self-test")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(SELF_TEST_TIMEOUT, run).await {
        Err(_) => {
            return Err(Attempt::GiveUp(format!(
                "the new version's self-test took longer than {} s",
                SELF_TEST_TIMEOUT.as_secs()
            )));
        }
        Ok(Err(e)) => return Err(Attempt::GiveUp(format!("the new version won't start: {e}"))),
        Ok(Ok(output)) => output,
    };
    let said = String::from_utf8_lossy(&output.stdout);
    let tail = |text: &str| text.trim().lines().last().unwrap_or("").to_string();
    let reason = || {
        let stderr = tail(&String::from_utf8_lossy(&output.stderr));
        if stderr.is_empty() {
            tail(&said)
        } else {
            stderr
        }
    };
    match output.status.code() {
        Some(0) if said.lines().next() == Some(&self_test_banner(version)) => Ok(()),
        Some(0) => Err(Attempt::GiveUp(format!(
            "the new binary isn't version {version} (it says \"{}\")",
            said.lines().next().unwrap_or("")
        ))),
        Some(SELF_TEST_HUB_UNREACHABLE) => Err(Attempt::Retry(format!(
            "the new version's self-test can't reach the hub: {}",
            reason()
        ))),
        _ => Err(Attempt::GiveUp(format!(
            "the new version's self-test failed ({}): {}",
            output.status,
            reason()
        ))),
    }
}

/// The first line `--self-test` prints.
pub fn self_test_banner(version: &str) -> String {
    format!("blentinel-probe {version}")
}

/// Make `bins.new` the executable, keeping the current one as `bins.prev`.
pub fn switch(bins: &Binaries) -> Result<()> {
    remove_if_present(&bins.prev)?;
    #[cfg(unix)]
    {
        // A second name for the running binary, then one atomic rename:
        // there is never a moment without an executable to start.
        if std::fs::hard_link(&bins.exe, &bins.prev).is_err() {
            std::fs::copy(&bins.exe, &bins.prev)
                .with_context(|| format!("can't keep {}", bins.prev.display()))?;
        }
        std::fs::rename(&bins.new, &bins.exe)
            .with_context(|| format!("can't replace {}", bins.exe.display()))
    }
    #[cfg(not(unix))]
    {
        swap_in(&bins.exe, &bins.prev, &bins.new)
    }
}

/// Put `bins.prev` back as the executable; the failed one is discarded.
pub fn roll_back(bins: &Binaries) -> Result<()> {
    if !bins.prev.exists() {
        bail!(
            "there is no previous version ({}) to go back to",
            bins.prev.display()
        );
    }
    #[cfg(unix)]
    {
        std::fs::rename(&bins.prev, &bins.exe)
            .with_context(|| format!("can't restore {}", bins.exe.display()))
    }
    #[cfg(not(unix))]
    {
        remove_if_present(&bins.failed)?;
        swap_in(&bins.exe, &bins.failed, &bins.prev)
    }
}

/// Windows: move the running `exe` to `aside`, then `incoming` to `exe`;
/// if that second step fails, move `exe` back.
#[cfg(not(unix))]
fn swap_in(exe: &Path, aside: &Path, incoming: &Path) -> Result<()> {
    std::fs::rename(exe, aside).with_context(|| format!("can't move {} aside", exe.display()))?;
    if let Err(e) = std::fs::rename(incoming, exe) {
        let _ = std::fs::rename(aside, exe);
        return Err(e).with_context(|| format!("can't put the new version at {}", exe.display()));
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("can't delete {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_sit_beside_the_executable() {
        let bins = Binaries::of(Path::new("/var/lib/blentinel-probe/probe"));
        assert_eq!(bins.new, Path::new("/var/lib/blentinel-probe/probe.new"));
        assert_eq!(bins.prev, Path::new("/var/lib/blentinel-probe/probe.prev"));
        let bins = Binaries::of(Path::new("probe.exe"));
        assert_eq!(bins.new, Path::new("probe.new.exe"));
        assert_eq!(bins.failed, Path::new("probe.failed.exe"));
    }

    fn folder_with(exe: &[u8], new: &[u8]) -> (tempfile::TempDir, Binaries) {
        let dir = tempfile::tempdir().unwrap();
        let bins = Binaries::of(&dir.path().join("probe"));
        std::fs::write(&bins.exe, exe).unwrap();
        std::fs::write(&bins.new, new).unwrap();
        (dir, bins)
    }

    #[test]
    fn switching_keeps_the_old_version_and_rolling_back_restores_it() {
        let (_dir, bins) = folder_with(b"v1", b"v2");
        std::fs::write(&bins.prev, b"v0").unwrap(); // from an earlier update
        switch(&bins).unwrap();
        assert_eq!(std::fs::read(&bins.exe).unwrap(), b"v2");
        assert_eq!(std::fs::read(&bins.prev).unwrap(), b"v1");
        assert!(!bins.new.exists());

        roll_back(&bins).unwrap();
        assert_eq!(std::fs::read(&bins.exe).unwrap(), b"v1");
        assert!(!bins.prev.exists());
        assert!(roll_back(&bins).is_err(), "nothing left to go back to");
        assert_eq!(std::fs::read(&bins.exe).unwrap(), b"v1");
    }

    #[test]
    fn a_failed_switch_leaves_the_executable_in_place() {
        let (_dir, bins) = folder_with(b"v1", b"v2");
        std::fs::remove_file(&bins.new).unwrap();
        assert!(switch(&bins).is_err());
        assert_eq!(std::fs::read(&bins.exe).unwrap(), b"v1");
    }

    #[tokio::test]
    async fn a_binary_that_isnt_a_probe_fails_its_self_test() {
        let (_dir, bins) = folder_with(b"v1", b"not a program");
        make_executable(&bins.new).unwrap();
        assert!(matches!(
            self_test(&bins.new, "0.3.0").await,
            Err(Attempt::GiveUp(_))
        ));
    }
}
