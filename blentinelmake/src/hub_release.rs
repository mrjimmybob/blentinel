//! `release hub`: the files of a GitHub release for `hub.sh` (M2-10, D-57).
//!
//! - the hub for each Linux target (`hub-<target>`), static musl builds;
//! - the signed probe release as `probes.tar.gz` (what `hub install` adds);
//! - `release.json`, the version (what `hub upgrade` and the hub's
//!   "new version" notice read, M4-04, D-75);
//! - `hub.sh`, the installer, with the release public keys in it;
//! - `SHA256SUMS` of all of these and `SHA256SUMS.sig`, its raw Ed25519
//!   signature by the release key, which `hub.sh` checks with `openssl`.
//!
//! Made in three steps (0013 phase 2, D-104), so that compiling and signing
//! can run on different machines: `release binaries` (compile; no key),
//! `release assemble` (an unsigned release; no key), then signing (a release
//! key; compiles nothing). `release hub` runs all three here.

use std::fs;
use std::path::{Path, PathBuf};

use common::hub_release::{
    HubRelease, PROBES_ARCHIVE, RELEASE_FILE, RELEASE_FORMAT, SUMS_FILE, SUMS_SIGNATURE_FILE,
    hub_file, is_commit, parse_sums,
};
use common::release::{PLATFORMS, Platform};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use crate::release;

/// The hub targets `hub.sh` installs (it picks one by `uname -m`).
pub const HUB_TARGETS: [&str; 2] = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

/// The installer, with [`KEYS_PLACEHOLDER`] where the keys go.
const HUB_SCRIPT: &str = include_str!("../templates/hub.sh");
const KEYS_PLACEHOLDER: &str = "@RELEASE_KEYS_PEM@";
const HUB_SCRIPT_FILE: &str = "hub.sh";

/// Where `hub.sh` downloads from by default (`releases/latest/download`).
const RELEASES_URL: &str = "https://github.com/mrjimmybob/blentinel/releases";

/// The DER prefix of an Ed25519 SubjectPublicKeyInfo (RFC 8410); the 32 key
/// bytes follow.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// A hex Ed25519 public key as the PEM `openssl` reads.
pub fn public_key_pem(hex_key: &str) -> Result<String, String> {
    use base64::Engine;
    let key: [u8; 32] = hex::decode(hex_key.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| format!("{hex_key:?} isn't a hex Ed25519 public key"))?;
    let mut der = ED25519_SPKI_PREFIX.to_vec();
    der.extend_from_slice(&key);
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    Ok(format!(
        "-----BEGIN PUBLIC KEY-----\n{b64}\n-----END PUBLIC KEY-----"
    ))
}

/// `hub.sh` trusting `keys` (hex), with Unix line endings whatever the checkout has.
pub fn render_hub_script(keys: &[String]) -> Result<String, String> {
    if keys.is_empty() {
        return Err("hub.sh needs at least one release public key".into());
    }
    let pems = keys
        .iter()
        .map(|k| public_key_pem(k))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    let script = HUB_SCRIPT.replace("\r\n", "\n");
    if !script.contains(KEYS_PLACEHOLDER) {
        return Err("templates/hub.sh has lost its key placeholder".into());
    }
    Ok(script.replace(KEYS_PLACEHOLDER, &pems))
}

/// `SHA256SUMS` for `files` (name, contents), sorted by name, in the format
/// `sha256sum -c` reads.
pub fn sha256_sums(files: &[(String, Vec<u8>)]) -> String {
    let mut files: Vec<&(String, Vec<u8>)> = files.iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
        .iter()
        .map(|(name, bytes)| format!("{}  {name}\n", hex::encode(Sha256::digest(bytes))))
        .collect()
}

/// In an assembled (unsigned) release, the folder of the probes and their
/// `manifest.json`; signing turns it into `probes.tar.gz`.
pub const PROBES_DIR: &str = "probes";

/// `release hub`: `publish/github-release-<version>/`, built, assembled and
/// signed here in three steps: [`binaries`] (the hub for each of
/// `hub_targets`, the probe for each of `probe_targets`), [`assemble`] and
/// [`sign_release`]. CI runs the same steps, but signs in a job of its own
/// that builds nothing (0013 §6.2, D-104). Every file goes into GitHub
/// release `v<version>`.
pub fn build(
    hub_targets: &[String],
    probe_targets: &[String],
    build_hub: impl Fn(&str) -> Result<PathBuf, String>,
    build_probe: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<(), String> {
    // Everything that can fail fast, before the long builds.
    let key = release::signing_key()?;
    let keys = release::public_keys()?;
    if !keys.contains(&hex::encode(key.verifying_key().to_bytes())) {
        return Err(format!(
            "{} has no .pub for its private key: hub.sh couldn't check this release",
            release::key_dir().display()
        ));
    }
    if hub_targets.is_empty() || probe_targets.is_empty() {
        return Err("release hub needs --targets <triple,...> and --probes <triple,...>".into());
    }
    check_targets(hub_targets, probe_targets)?;
    let commit = head_commit()?;
    let version = release::workspace_version()?;

    let bins = Path::new("publish").join(format!("binaries-{version}"));
    binaries(&bins, hub_targets, probe_targets, build_hub, build_probe)?;
    let folder = Path::new("publish").join(format!("github-release-{version}"));
    assemble(&bins, &folder, &version, &commit, &keys)?;
    let sums = sign_release(&folder, &key)?;

    println!(
        "\nHub release {version} (commit {commit}): {}",
        folder.display()
    );
    print!("{sums}");
    println!("\nPublish every file as GitHub release v{version}, e.g.:");
    println!(
        "  gh release create v{version} {}/* --title \"Blentinel {version}\"",
        folder.display()
    );
    println!("A server then installs the hub with:");
    println!("  curl -fsSL {RELEASES_URL}/latest/download/hub.sh | sudo sh");
    Ok(())
}

/// Hub targets `hub.sh` installs and probe targets of [`PLATFORMS`] only.
fn check_targets(hub_targets: &[String], probe_targets: &[String]) -> Result<(), String> {
    for target in hub_targets {
        if !HUB_TARGETS.contains(&target.as_str()) {
            return Err(format!(
                "{target} isn't a hub target hub.sh installs ({})",
                HUB_TARGETS.join(", ")
            ));
        }
    }
    for target in probe_targets {
        if Platform::by_target(target).is_none() {
            return Err(format!(
                "{target} isn't a probe release target ({})",
                PLATFORMS.map(|p| p.target).join(", ")
            ));
        }
    }
    Ok(())
}

/// The commit `HEAD` is, refusing a tree with changes to tracked files: a
/// release says which commit it was built from (0013 §6.4), and it must be
/// that commit's source. (Untracked files don't count.)
pub fn head_commit() -> Result<String, String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let out = std::process::Command::new("git")
            .args(args)
            .output()
            .map_err(|e| format!("Can't run git: {e}"))?;
        if !out.status.success() {
            return Err(format!("git {} failed", args.join(" ")));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return Err(
            "tracked files have uncommitted changes: commit them first, a release names the \
             commit it was built from"
                .into(),
        );
    }
    let commit = git(&["rev-parse", "HEAD"])?;
    if !is_commit(&commit) {
        return Err(format!("git rev-parse HEAD said {commit:?}"));
    }
    Ok(commit)
}

/// The release file name of each binary `release binaries` writes.
fn binary_names() -> Vec<String> {
    HUB_TARGETS
        .iter()
        .map(|t| hub_file(t))
        .chain(PLATFORMS.iter().map(Platform::file_name))
        .collect()
}

/// `release binaries`: build the hub for each of `hub_targets` and the probe
/// for each of `probe_targets` (`build_hub` and `build_probe` return where
/// each binary is) into `out`, under their release names, with their
/// `SHA256SUMS`. Builds only: no key is read. "Build once" (0013 §6.3):
/// these bytes are what every later step assembles, signs and ships.
pub fn binaries(
    out: &Path,
    hub_targets: &[String],
    probe_targets: &[String],
    build_hub: impl Fn(&str) -> Result<PathBuf, String>,
    build_probe: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<(), String> {
    if hub_targets.is_empty() && probe_targets.is_empty() {
        return Err("release binaries needs --hub <triple,...> or --probes <triple,...>".into());
    }
    check_targets(hub_targets, probe_targets)?;
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| format!("Can't clear {}: {e}", out.display()))?;
    }
    fs::create_dir_all(out).map_err(|e| format!("Can't create {}: {e}", out.display()))?;
    let mut files = Vec::new();
    let mut add = |binary: PathBuf, name: String| -> Result<(), String> {
        let bytes =
            fs::read(&binary).map_err(|e| format!("Can't read {}: {e}", binary.display()))?;
        fs::write(out.join(&name), &bytes).map_err(|e| format!("Can't write {name}: {e}"))?;
        files.push((name, bytes));
        Ok(())
    };
    for target in hub_targets {
        add(build_hub(target)?, hub_file(target))?;
    }
    for target in probe_targets {
        let platform = Platform::by_target(target).ok_or("checked above")?;
        add(build_probe(target)?, platform.file_name())?;
    }
    let sums = sha256_sums(&files);
    fs::write(out.join(SUMS_FILE), &sums).map_err(|e| e.to_string())?;
    print!("Binaries in {}:\n{sums}", out.display());
    Ok(())
}

/// The binaries in `dir`, each checked against the folder's `SHA256SUMS`:
/// every file listed, matching, a binary's release name, and nothing else
/// in the folder.
pub fn read_binaries(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let text = fs::read_to_string(dir.join(SUMS_FILE))
        .map_err(|e| format!("Can't read {}: {e}", dir.join(SUMS_FILE).display()))?;
    let sums = parse_sums(&text).map_err(|e| match e {
        common::release::ReleaseError::Invalid(why) => format!("{}: {why}", dir.display()),
        other => other.to_string(),
    })?;
    let known = binary_names();
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == SUMS_FILE {
            continue;
        }
        if !known.contains(&name) {
            return Err(format!("{name} isn't a release binary's name"));
        }
        let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
        if !sums.matches(&name, &bytes) {
            return Err(format!(
                "{name} isn't the file {SUMS_FILE} lists: not using it"
            ));
        }
        files.push((name, bytes));
    }
    if files.len() != text.lines().filter(|l| !l.trim().is_empty()).count() {
        return Err(format!(
            "{SUMS_FILE} lists a file that isn't in {}",
            dir.display()
        ));
    }
    files.sort();
    Ok(files)
}

/// `release assemble`: an unsigned hub release `version` in `out` from the
/// binaries in `from` ([`read_binaries`]): the hubs, `probes/` (the probes
/// and their unsigned `manifest.json`), `release.json` (`version` and
/// `commit`) and `hub.sh` trusting `keys` (hex public keys: the ones the
/// binaries were built with). Reads no private key: [`sign_release`] or
/// CI's signing job finishes it.
pub fn assemble(
    from: &Path,
    out: &Path,
    version: &str,
    commit: &str,
    keys: &[String],
) -> Result<(), String> {
    if !is_commit(commit) {
        return Err(format!("{commit:?} isn't a full commit ID (40 hex digits)"));
    }
    let binaries = read_binaries(from)?;
    let is_hub = |name: &str| name.starts_with("hub-");
    if !binaries.iter().any(|(n, _)| is_hub(n)) || binaries.iter().all(|(n, _)| is_hub(n)) {
        return Err(format!(
            "{} needs a hub and at least one probe: hub.sh installs both",
            from.display()
        ));
    }
    let script = render_hub_script(keys)?;
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| format!("Can't clear {}: {e}", out.display()))?;
    }
    let probes = out.join(PROBES_DIR);
    fs::create_dir_all(&probes).map_err(|e| format!("Can't create {}: {e}", probes.display()))?;
    for (name, bytes) in &binaries {
        let dir = if is_hub(name) { out } else { &probes };
        fs::write(dir.join(name), bytes).map_err(|e| format!("Can't write {name}: {e}"))?;
    }
    release::write_manifest(&probes, version)?;
    fs::write(out.join(HUB_SCRIPT_FILE), script)
        .map_err(|e| format!("Can't write {HUB_SCRIPT_FILE}: {e}"))?;
    let info = HubRelease {
        format: RELEASE_FORMAT,
        version: version.to_string(),
        released: chrono::Utc::now(),
        commit: commit.to_string(),
    };
    let info = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
    fs::write(out.join(RELEASE_FILE), info)
        .map_err(|e| format!("Can't write {RELEASE_FILE}: {e}"))?;
    println!(
        "Assembled release {version} (commit {commit}), unsigned: {}",
        out.display()
    );
    Ok(())
}

/// Sign an assembled release in place with `key`: `manifest.json.sig`, the
/// probes as `probes.tar.gz`, then `SHA256SUMS` of every file and its raw
/// signature. What `.github/scripts/sign-release.sh` does with `openssl` in
/// CI, which a test there checks against `hub.sh`, `hub install` and `hub
/// upgrade`. The text of `SHA256SUMS`.
pub fn sign_release(folder: &Path, key: &SigningKey) -> Result<String, String> {
    let probes = folder.join(PROBES_DIR);
    let manifest = release::sign_manifest_in(&probes, key)?;
    release::write_archive(&probes, &manifest, &folder.join(PROBES_ARCHIVE))?;
    fs::remove_dir_all(&probes).map_err(|e| format!("Can't clear {}: {e}", probes.display()))?;

    let mut files = Vec::new();
    for entry in fs::read_dir(folder).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == SUMS_FILE || name == SUMS_SIGNATURE_FILE {
            continue;
        }
        files.push((name, fs::read(entry.path()).map_err(|e| e.to_string())?));
    }
    let sums = sha256_sums(&files);
    let signature = common::crypto::sign(sums.as_bytes(), key);
    common::hub_release::open_sums(
        sums.as_bytes(),
        &signature,
        &[key.verifying_key().to_bytes()],
    )
    .map_err(|e| format!("The signed SHA256SUMS doesn't verify: {e}"))?;
    fs::write(folder.join(SUMS_FILE), &sums).map_err(|e| e.to_string())?;
    fs::write(folder.join(SUMS_SIGNATURE_FILE), &signature).map_err(|e| e.to_string())?;
    Ok(sums)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_public_key_becomes_the_pem_openssl_reads() {
        // The example key of RFC 8410 §10.1.
        let pem =
            public_key_pem("19bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1")
                .unwrap();
        assert_eq!(
            pem,
            "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----"
        );
        assert!(public_key_pem("abcd").is_err());
    }

    #[test]
    fn the_script_carries_every_key_and_no_placeholder() {
        let keys = ["11".repeat(32), "22".repeat(32)];
        let script = render_hub_script(&keys).unwrap();
        assert!(!script.contains(KEYS_PLACEHOLDER));
        assert!(!script.contains('\r'));
        assert_eq!(script.matches("-----BEGIN PUBLIC KEY-----").count(), 2);
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(RELEASES_URL), "the default the docs show");
        assert!(render_hub_script(&[]).is_err());
    }

    /// Binaries, assembled and signed in steps, open as the hub and the probe
    /// open them; the binaries keep their bytes all the way.
    #[test]
    fn binaries_assembled_then_signed_are_a_release_hub_and_probes_accept() {
        let dir = tempfile::tempdir().unwrap();
        let built = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            path
        };
        let (hub, probe, pi_zero) = (
            built("hub", b"the hub"),
            built("probe", b"a probe"),
            built("probe-v6", b"a probe for ARMv6"),
        );
        let bins = dir.path().join("bins");
        let linux = "x86_64-unknown-linux-musl".to_string();
        let armv6 = "arm-unknown-linux-musleabihf".to_string();
        binaries(
            &bins,
            std::slice::from_ref(&linux),
            &[linux.clone(), armv6.clone()],
            |_| Ok(hub.clone()),
            |t| {
                Ok(if t == armv6 {
                    pi_zero.clone()
                } else {
                    probe.clone()
                })
            },
        )
        .unwrap();

        let key = SigningKey::from_bytes(&[7; 32]);
        let public = key.verifying_key().to_bytes();
        let commit = "5e".repeat(20);
        let release = dir.path().join("release");
        // Assembling reads no private key: it gets only the public ones.
        assemble(&bins, &release, "0.9.0", &commit, &[hex::encode(public)]).unwrap();
        assert!(!release.join(SUMS_SIGNATURE_FILE).exists());
        sign_release(&release, &key).unwrap();

        // What hub.sh and `hub upgrade` check: the signed list, then each file.
        let read = |name: &str| fs::read(release.join(name)).unwrap();
        let sums =
            common::hub_release::open_sums(&read(SUMS_FILE), &read(SUMS_SIGNATURE_FILE), &[public])
                .unwrap();
        let mut names: Vec<String> = fs::read_dir(&release)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                SUMS_FILE,
                SUMS_SIGNATURE_FILE,
                "hub-x86_64-unknown-linux-musl",
                HUB_SCRIPT_FILE,
                PROBES_ARCHIVE,
                RELEASE_FILE
            ]
        );
        for name in &names[2..] {
            assert!(sums.matches(name, &read(name)), "{name}");
        }
        assert_eq!(read("hub-x86_64-unknown-linux-musl"), b"the hub");
        let info = common::hub_release::open_release(&sums, &read(RELEASE_FILE)).unwrap();
        assert_eq!((info.version.as_str(), info.commit), ("0.9.0", commit));

        // What `hub install` and the probe check: the signed probe manifest.
        let probes = read(PROBES_ARCHIVE);
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&probes[..]));
        let mut files = std::collections::BTreeMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
            files.insert(entry.path().unwrap().display().to_string(), bytes);
        }
        let signature = String::from_utf8(files["manifest.json.sig"].clone()).unwrap();
        let manifest =
            common::release::open_manifest(&files["manifest.json"], &signature, &[public]).unwrap();
        assert_eq!(manifest.version, "0.9.0");
        assert_eq!(manifest.artifacts.len(), 2);
        assert_eq!(
            files["probe-arm-unknown-linux-musleabihf"],
            b"a probe for ARMv6"
        );
    }

    #[test]
    fn assembling_refuses_binaries_their_sums_dont_vouch_for() {
        let dir = tempfile::tempdir().unwrap();
        let bins = dir.path().join("bins");
        let file = dir.path().join("binary");
        fs::write(&file, b"bytes").unwrap();
        let linux = vec!["x86_64-unknown-linux-musl".to_string()];
        binaries(
            &bins,
            &linux,
            &linux,
            |_| Ok(file.clone()),
            |_| Ok(file.clone()),
        )
        .unwrap();
        assert!(read_binaries(&bins).is_ok());
        let assemble_to = |out: &str, commit: &str| {
            assemble(
                &bins,
                &dir.path().join(out),
                "0.9.0",
                commit,
                &["11".repeat(32)],
            )
        };
        assert!(assemble_to("a", "abc").is_err(), "an abbreviated commit");

        // A binary changed after it was built.
        fs::write(bins.join("probe-x86_64-unknown-linux-musl"), b"other").unwrap();
        assert!(assemble_to("b", &"5e".repeat(20)).is_err());
        fs::write(bins.join("probe-x86_64-unknown-linux-musl"), b"bytes").unwrap();
        // A file nothing vouches for.
        fs::write(bins.join("probe-aarch64-unknown-linux-musl"), b"bytes").unwrap();
        assert!(assemble_to("c", &"5e".repeat(20)).is_err());
        fs::remove_file(bins.join("probe-aarch64-unknown-linux-musl")).unwrap();
        // A file the sums list is missing.
        fs::remove_file(bins.join("hub-x86_64-unknown-linux-musl")).unwrap();
        assert!(assemble_to("d", &"5e".repeat(20)).is_err());
    }

    #[test]
    fn sums_are_sorted_and_in_sha256sum_format() {
        let sums = sha256_sums(&[
            ("probes.tar.gz".into(), b"p".to_vec()),
            ("hub.sh".into(), Vec::new()),
        ]);
        let lines: Vec<_> = sums.lines().collect();
        assert_eq!(
            lines[0],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  hub.sh"
        );
        assert!(lines[1].ends_with("  probes.tar.gz"));
        assert!(sums.ends_with('\n'));
    }
}
