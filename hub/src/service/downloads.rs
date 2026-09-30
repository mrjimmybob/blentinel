//! The probe binaries the hub offers for download (M2-09, D-53): the files
//! in `<state_dir>/probes/`, named `probe-<target>` (`.exe` on Windows).
//! `hub install` copies them there from a `probes/` folder beside the hub,
//! and `blentinelmake hub publish --with-probes` builds that folder.
//!
//! The "Add probe" wizard shows each file's SHA-256 in its one-line
//! commands, so the download is checked against what the admin sees on the
//! hub's HTTPS page, whatever path it takes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::model::ProbeDownload;

/// The folder, inside the hub's `state_dir`.
pub const PROBES_DIR: &str = "probes";

/// The targets the hub offers, with how the wizard names them (D-13: Linux
/// builds are static musl).
pub const TARGETS: [(&str, &str, &str); 4] = [
    ("x86_64-unknown-linux-musl", "linux", "x86_64"),
    ("aarch64-unknown-linux-musl", "linux", "arm64"),
    ("armv7-unknown-linux-musleabihf", "linux", "armv7"),
    ("x86_64-pc-windows-msvc", "windows", "x86_64"),
];

/// The file name for a target.
pub fn file_name(target: &str) -> String {
    let exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("probe-{target}{exe}")
}

/// SHA-256s already worked out, by file, with the size and time they were for.
type HashCache = HashMap<PathBuf, (u64, SystemTime, String)>;

fn sha256_of(path: &Path) -> std::io::Result<String> {
    static CACHE: OnceLock<Mutex<HashCache>> = OnceLock::new();
    let meta = std::fs::metadata(path)?;
    let stamp = (meta.len(), meta.modified()?);
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((len, modified, hash)) = cache.lock().ok().and_then(|c| c.get(path).cloned())
        && (len, modified) == stamp
    {
        return Ok(hash);
    }
    let hash = hex::encode(Sha256::digest(std::fs::read(path)?));
    if let Ok(mut cache) = cache.lock() {
        cache.insert(path.to_path_buf(), (stamp.0, stamp.1, hash.clone()));
    }
    Ok(hash)
}

/// The probe for `target` in `dir`, if the hub has it.
pub fn find(dir: &Path, target: &str) -> Option<(PathBuf, ProbeDownload)> {
    let (target, os, arch) = TARGETS.iter().find(|(t, ..)| *t == target)?;
    let path = dir.join(file_name(target));
    let size = std::fs::metadata(&path).ok()?.len();
    let sha256 = sha256_of(&path).ok()?;
    Some((
        path,
        ProbeDownload {
            target: target.to_string(),
            os: os.to_string(),
            arch: arch.to_string(),
            size,
            sha256,
        },
    ))
}

/// Every probe the hub has, in [`TARGETS`] order.
pub fn list(dir: &Path) -> Vec<ProbeDownload> {
    TARGETS
        .iter()
        .filter_map(|(target, ..)| find(dir, target).map(|(_, download)| download))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_targets_with_a_file_are_offered() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(dir.path()).is_empty());
        std::fs::write(dir.path().join("probe-x86_64-pc-windows-msvc.exe"), b"MZ").unwrap();
        std::fs::write(dir.path().join("probe-sparc-unknown-linux"), b"?").unwrap();

        let offered = list(dir.path());
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].os, "windows");
        assert_eq!(offered[0].size, 2);
        assert_eq!(offered[0].sha256, hex::encode(Sha256::digest(b"MZ")));
        assert!(find(dir.path(), "sparc-unknown-linux").is_none());
        assert!(
            find(dir.path(), "x86_64-unknown-linux-musl").is_none(),
            "no file"
        );
    }

    #[test]
    fn a_replaced_file_gets_a_new_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(file_name("x86_64-unknown-linux-musl"));
        std::fs::write(&path, b"v1").unwrap();
        let first = sha256_of(&path).unwrap();
        std::fs::write(&path, b"v2, longer").unwrap();
        assert_ne!(sha256_of(&path).unwrap(), first);
    }
}
