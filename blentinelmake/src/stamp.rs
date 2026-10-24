//! `release stamp <N>`: an internal development build's version (proposal
//! 0013 §6.1 and §10.4 C, D-106, D-107).
//!
//! Every Blentinel binary takes its version from `CARGO_PKG_VERSION`, so the
//! Build workflow stamps the workspace before compiling: `[workspace.package]
//! version` in `Cargo.toml` and the workspace's own entries in `Cargo.lock`
//! become `<version>-dev.<N>`, N being the workflow's run number. The lock
//! file is stamped too, so `cargo --locked` still holds. Both files are
//! changed on the runner only; nothing stamped is ever committed.

use std::fs;

use common::release::Version;

const MANIFEST: &str = "Cargo.toml";
const LOCK: &str = "Cargo.lock";

/// `release stamp <N>`: stamp the workspace in the current folder and print
/// the new version (alone on stdout, for `$(…)`).
pub fn stamp(build: &str) -> Result<(), String> {
    let manifest = fs::read_to_string(MANIFEST)
        .map_err(|e| format!("Run this from the workspace folder ({MANIFEST}: {e})"))?;
    let lock = fs::read_to_string(LOCK).map_err(|e| format!("{LOCK}: {e}"))?;
    let base = crate::release::workspace_version()?;
    let version = dev_version(&base, build)?;
    let manifest = stamp_manifest(&manifest, &base, &version)?;
    let lock = stamp_lock(&lock, &base, &version, workspace_members(&manifest)?)?;
    fs::write(MANIFEST, manifest).map_err(|e| format!("Can't write {MANIFEST}: {e}"))?;
    fs::write(LOCK, lock).map_err(|e| format!("Can't write {LOCK}: {e}"))?;
    println!("{version}");
    Ok(())
}

/// `<base>-dev.<build>`. The base must be a plain release version (a stamped
/// workspace isn't stamped again), and the build a positive whole number
/// written without a leading zero, as SemVer requires of a numeric part.
pub fn dev_version(base: &str, build: &str) -> Result<String, String> {
    if Version::parse(base).is_none() || base.contains(['-', '+']) {
        return Err(format!(
            "The workspace version {base} isn't a plain MAJOR.MINOR.PATCH: stamped already?"
        ));
    }
    match build.parse::<u64>() {
        Ok(n) if n > 0 && n.to_string() == build => {}
        _ => {
            return Err(format!(
                "The build number must be a positive whole number with no leading zero, not {build:?}"
            ));
        }
    }
    let version = format!("{base}-dev.{build}");
    Version::parse(&version)
        .map(|_| version.clone())
        .ok_or_else(|| format!("{version} isn't a version"))
}

/// `Cargo.toml` with `[workspace.package] version` set to `version`. Only
/// that line changes, line endings included.
fn stamp_manifest(text: &str, base: &str, version: &str) -> Result<String, String> {
    let mut section = String::new();
    let mut stamped = 0;
    let out: String = text
        .split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                section = trimmed.to_string();
            } else if section == "[workspace.package]" && is_version_line(trimmed, base) {
                stamped += 1;
                return line.replacen(&format!("\"{base}\""), &format!("\"{version}\""), 1);
            }
            line.to_string()
        })
        .collect();
    match stamped {
        1 => Ok(out),
        n => Err(format!(
            "{MANIFEST}: expected one `version = \"{base}\"` in [workspace.package], found {n}"
        )),
    }
}

/// `Cargo.lock` with the workspace's own packages set to `version`: those
/// with no `source` line (a crate from a registry or git always has one).
/// Every member must be found, or the lock and the manifest would disagree.
fn stamp_lock(text: &str, base: &str, version: &str, members: usize) -> Result<String, String> {
    let mut out = String::with_capacity(text.len() + 64);
    let mut stamped = 0;
    for (i, block) in text.split("\n[[package]]").enumerate() {
        if i > 0 {
            out.push_str("\n[[package]]");
        }
        let local = i > 0 && !block.split('\n').any(|l| l.starts_with("source = "));
        if !local {
            out.push_str(block);
            continue;
        }
        for line in block.split_inclusive('\n') {
            if is_version_line(line.trim(), base) {
                stamped += 1;
                out.push_str(&line.replacen(&format!("\"{base}\""), &format!("\"{version}\""), 1));
            } else {
                out.push_str(line);
            }
        }
    }
    if stamped != members {
        return Err(format!(
            "{LOCK}: {members} workspace members, but {stamped} of its packages are local at {base}: run `cargo metadata` and commit the lock file"
        ));
    }
    Ok(out)
}

/// `version = "<base>"`, spaced as cargo writes it.
fn is_version_line(trimmed: &str, base: &str) -> bool {
    trimmed == format!("version = \"{base}\"")
}

/// How many crates `[workspace] members` lists.
fn workspace_members(manifest: &str) -> Result<usize, String> {
    let doc: toml::Table = manifest.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    doc.get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .map(Vec::len)
        .ok_or_else(|| format!("{MANIFEST} has no [workspace] members"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_version_is_the_base_and_the_build_number() {
        assert_eq!(dev_version("0.1.0", "7").unwrap(), "0.1.0-dev.7");
        assert_eq!(dev_version("1.20.3", "1234").unwrap(), "1.20.3-dev.1234");
    }

    #[test]
    fn dev_builds_order_by_their_number_and_before_the_release() {
        let v = |n: &str| Version::parse(&dev_version("0.1.0", n).unwrap()).unwrap();
        assert!(v("10") > v("9"));
        assert!(v("9") > v("1"));
        assert!(v("1000") < Version::parse("0.1.0").unwrap());
    }

    #[test]
    fn a_bad_build_number_is_refused() {
        for build in [
            "",
            "0",
            "07",
            "-1",
            "1.5",
            "x",
            "1 ",
            "99999999999999999999999",
        ] {
            assert!(dev_version("0.1.0", build).is_err(), "{build:?} was taken");
        }
    }

    #[test]
    fn only_a_plain_version_is_stamped() {
        for base in ["0.1.0-dev.3", "0.1.0+meta", "0.1", "x.y.z", ""] {
            assert!(dev_version(base, "3").is_err(), "{base:?} was stamped");
        }
    }

    #[test]
    fn the_manifest_changes_only_the_workspace_version() {
        let text = "[workspace]\r\nmembers = [\"a\"]\r\n\r\n[workspace.package]\r\nversion = \"0.1.0\"\r\nedition = \"2024\"\r\n\r\n[workspace.dependencies]\r\nx = { version = \"0.1.0\" }\r\n[package]\r\nversion = \"0.1.0\"\r\n";
        let out = stamp_manifest(text, "0.1.0", "0.1.0-dev.4").unwrap();
        assert_eq!(
            out,
            text.replacen(
                "version = \"0.1.0\"\r\nedition",
                "version = \"0.1.0-dev.4\"\r\nedition",
                1
            )
        );
        let doc: toml::Table = out.parse().unwrap();
        assert_eq!(
            doc["workspace"]["package"]["version"].as_str(),
            Some("0.1.0-dev.4")
        );
    }

    #[test]
    fn a_manifest_without_the_version_is_refused() {
        assert!(
            stamp_manifest(
                "[workspace.package]\nversion = \"0.2.0\"\n",
                "0.1.0",
                "0.1.0-dev.1"
            )
            .is_err()
        );
        assert!(
            stamp_manifest("[package]\nversion = \"0.1.0\"\n", "0.1.0", "0.1.0-dev.1").is_err()
        );
    }

    #[test]
    fn the_lock_changes_only_the_local_packages() {
        let text = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"0.1.0\"\ndependencies = [\n \"common\",\n]\n\n[[package]]\nname = \"common\"\nversion = \"0.1.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"00\"\n\n[[package]]\nname = \"common\"\nversion = \"0.1.0\"\n";
        let out = stamp_lock(text, "0.1.0", "0.1.0-dev.2", 2).unwrap();
        let doc: toml::Table = out.parse().unwrap();
        assert_eq!(doc["version"].as_integer(), Some(4));
        let versions: Vec<_> = doc["package"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p.get("source").is_some(), p["version"].as_str().unwrap()))
            .collect();
        assert_eq!(
            versions,
            [
                (false, "0.1.0-dev.2"),
                (true, "0.1.0"),
                (false, "0.1.0-dev.2")
            ]
        );
        assert!(
            stamp_lock(text, "0.1.0", "0.1.0-dev.2", 3).is_err(),
            "a member wasn't found"
        );
    }

    /// The workspace as it is: every member stamped in both files.
    #[test]
    fn the_real_workspace_stamps() {
        let manifest = include_str!("../../Cargo.toml");
        let lock = include_str!("../../Cargo.lock");
        let doc: toml::Table = manifest.parse().unwrap();
        let base = doc["workspace"]["package"]["version"].as_str().unwrap();
        let version = dev_version(base, "12").unwrap();
        let manifest = stamp_manifest(manifest, base, &version).unwrap();
        let members = workspace_members(&manifest).unwrap();
        let lock = stamp_lock(lock, base, &version, members).unwrap();
        assert_eq!(
            lock.matches(&format!("version = \"{version}\"")).count(),
            members
        );
    }
}
