//! `release hub`: the files of a GitHub release for `hub.sh` (M2-10, D-57).
//!
//! - the hub for each Linux target (`hub-<target>`), static musl builds;
//! - the signed probe release as `probes.tar.gz` (what `hub install` adds);
//! - `release.json`, the version (what `hub upgrade` and the hub's
//!   "new version" notice read, M4-04, D-75);
//! - `hub.sh`, the installer, with the release public keys in it;
//! - `SHA256SUMS` of all of these and `SHA256SUMS.sig`, its raw Ed25519
//!   signature by the release key, which `hub.sh` checks with `openssl`.

use std::fs;
use std::path::{Path, PathBuf};

use common::hub_release::{
    HubRelease, PROBES_ARCHIVE, RELEASE_FILE, RELEASE_FORMAT, SUMS_FILE, SUMS_SIGNATURE_FILE,
    hub_file,
};
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

/// `release hub`: `publish/github-release-<version>/` with the hub for each
/// of `hub_targets` (`build_hub` returns the binary's path), the probes for
/// `probe_targets` as a signed release (`build_probe`), `hub.sh`, and the
/// signed `SHA256SUMS`. Every file goes into GitHub release `v<version>`.
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
    if hub_targets.is_empty() {
        return Err("release hub needs --targets <triple,...>".into());
    }
    for target in hub_targets {
        if !HUB_TARGETS.contains(&target.as_str()) {
            return Err(format!(
                "{target} isn't a hub target hub.sh installs ({})",
                HUB_TARGETS.join(", ")
            ));
        }
    }
    if probe_targets.is_empty() {
        return Err("release hub needs --probes <triple,...> (the probes Add probe offers)".into());
    }
    let script = render_hub_script(&keys)?;
    let version = release::workspace_version()?;
    let folder = Path::new("publish").join(format!("github-release-{version}"));
    if folder.exists() {
        fs::remove_dir_all(&folder)
            .map_err(|e| format!("Can't clear {}: {e}", folder.display()))?;
    }

    let probes = folder.join("probe-release");
    let manifest = release::build_into(&probes, probe_targets, build_probe)?;
    release::write_archive(&probes, &manifest, &folder.join(PROBES_ARCHIVE))?;
    fs::remove_dir_all(&probes).map_err(|e| format!("Can't clear {}: {e}", probes.display()))?;

    for target in hub_targets {
        let binary = build_hub(target)?;
        fs::copy(&binary, folder.join(hub_file(target)))
            .map_err(|e| format!("Can't copy {}: {e}", binary.display()))?;
    }
    fs::write(folder.join(HUB_SCRIPT_FILE), script)
        .map_err(|e| format!("Can't write {HUB_SCRIPT_FILE}: {e}"))?;
    let info = HubRelease {
        format: RELEASE_FORMAT,
        version: version.clone(),
        released: chrono::Utc::now(),
    };
    let info = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
    fs::write(folder.join(RELEASE_FILE), info)
        .map_err(|e| format!("Can't write {RELEASE_FILE}: {e}"))?;

    let mut files = Vec::new();
    for entry in fs::read_dir(&folder).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        files.push((name, fs::read(entry.path()).map_err(|e| e.to_string())?));
    }
    let sums = sha256_sums(&files);
    let signature = common::crypto::sign(sums.as_bytes(), &key);
    common::crypto::verify_with_key(sums.as_bytes(), &signature, &key.verifying_key().to_bytes())
        .map_err(|_| "The signature of SHA256SUMS doesn't verify".to_string())?;
    fs::write(folder.join(SUMS_FILE), &sums).map_err(|e| e.to_string())?;
    fs::write(folder.join(SUMS_SIGNATURE_FILE), &signature).map_err(|e| e.to_string())?;

    println!("\nHub release {version}: {}", folder.display());
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
