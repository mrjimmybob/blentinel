//! Hub releases (M2-10, D-57; read by the hub itself since M4-04, D-75): the
//! files `blentinelmake release hub` writes for one version, published as a
//! GitHub release (or any folder):
//!
//! - `hub-<target>`: the hub for each target (`.exe` for Windows);
//! - `probes.tar.gz`: the matching signed probe release;
//! - `release.json`: which version this is ([`HubRelease`]);
//! - `hub.sh`, the one-line installer;
//! - `SHA256SUMS` of every file, and `SHA256SUMS.sig`, its raw Ed25519
//!   signature by a release key ([`open_sums`]).
//!
//! One signature covers everything, checked against the release keys
//! compiled in (`release::trusted_keys`): neither the download host nor the
//! network is trusted, by `hub.sh` or by `hub upgrade`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::release::{ReleaseError, Version};

/// The list of SHA-256s.
pub const SUMS_FILE: &str = "SHA256SUMS";
/// Its raw 64-byte signature (what `openssl pkeyutl` reads, for `hub.sh`).
pub const SUMS_SIGNATURE_FILE: &str = "SHA256SUMS.sig";
/// Which version the release is.
pub const RELEASE_FILE: &str = "release.json";
/// The signed probe release the hub adds after installing or upgrading.
pub const PROBES_ARCHIVE: &str = "probes.tar.gz";
/// Where hub releases are published: the latest one's files.
pub const DEFAULT_RELEASE_URL: &str =
    "https://github.com/mrjimmybob/blentinel/releases/latest/download";
/// The format of `release.json` this build reads and writes.
pub const RELEASE_FORMAT: u32 = 1;

/// `release.json`: covered by `SHA256SUMS`, so as trusted as the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubRelease {
    /// [`RELEASE_FORMAT`].
    pub format: u32,
    /// The hub (and probe) version inside.
    pub version: String,
    pub released: DateTime<Utc>,
}

impl HubRelease {
    pub fn parsed_version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }
}

/// The file name of the hub for `target` in a release.
pub fn hub_file(target: &str) -> String {
    if target.contains("windows") {
        format!("hub-{target}.exe")
    } else {
        format!("hub-{target}")
    }
}

/// The release target of the hub running now (Linux hubs are static musl
/// builds, D-57), if releases are built for it.
pub fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// A checked `SHA256SUMS`: each file's SHA-256 (hex), by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sums(BTreeMap<String, String>);

impl Sums {
    /// The signed SHA-256 of `name`, if the release has that file.
    pub fn sha256(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Does `bytes` match the signed SHA-256 of `name`?
    pub fn matches(&self, name: &str, bytes: &[u8]) -> bool {
        use sha2::{Digest, Sha256};
        self.sha256(name)
            .is_some_and(|sha| hex::encode(Sha256::digest(bytes)).eq_ignore_ascii_case(sha))
    }
}

/// `SHA256SUMS` in the format `sha256sum` writes (`<hex>  <name>`). A line
/// that isn't one, or a name that is a path, makes the whole list invalid.
pub fn parse_sums(text: &str) -> Result<Sums, ReleaseError> {
    let mut sums = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let invalid = || ReleaseError::Invalid(format!("not a SHA256SUMS line: {line:?}"));
        let (sha, name) = line.split_once("  ").ok_or_else(invalid)?;
        let plain = !name.is_empty() && !name.contains(['/', '\\']) && name != "..";
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) || !plain {
            return Err(invalid());
        }
        sums.insert(name.to_string(), sha.to_ascii_lowercase());
    }
    Ok(Sums(sums))
}

/// Check `SHA256SUMS` against its raw signature and `keys`, then parse it.
pub fn open_sums(sums: &[u8], signature: &[u8], keys: &[[u8; 32]]) -> Result<Sums, ReleaseError> {
    if keys.is_empty() {
        return Err(ReleaseError::NoTrustedKeys);
    }
    if !keys
        .iter()
        .any(|key| crate::crypto::verify_with_key(sums, signature, key).is_ok())
    {
        return Err(ReleaseError::BadSignature);
    }
    let text = std::str::from_utf8(sums)
        .map_err(|_| ReleaseError::Invalid("SHA256SUMS isn't text".into()))?;
    parse_sums(text)
}

/// Check `release.json` against the checked `sums`, then parse it.
pub fn open_release(sums: &Sums, release: &[u8]) -> Result<HubRelease, ReleaseError> {
    if !sums.matches(RELEASE_FILE, release) {
        return Err(ReleaseError::Invalid(format!(
            "{RELEASE_FILE} isn't the file SHA256SUMS signs"
        )));
    }
    let parsed: HubRelease = serde_json::from_slice(release)
        .map_err(|e| ReleaseError::Invalid(format!("not a {RELEASE_FILE}: {e}")))?;
    if parsed.format != RELEASE_FORMAT {
        return Err(ReleaseError::Invalid(format!(
            "unknown format {}",
            parsed.format
        )));
    }
    if parsed.parsed_version().is_none() {
        return Err(ReleaseError::Invalid(format!(
            "\"{}\" isn't a version",
            parsed.version
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use sha2::{Digest, Sha256};

    fn line(name: &str, bytes: &[u8]) -> String {
        format!("{}  {name}\n", hex::encode(Sha256::digest(bytes)))
    }

    fn release_json(version: &str) -> Vec<u8> {
        serde_json::to_vec(&HubRelease {
            format: RELEASE_FORMAT,
            version: version.into(),
            released: "2026-10-09T10:00:00Z".parse().unwrap(),
        })
        .unwrap()
    }

    #[test]
    fn signed_sums_open_and_vouch_for_each_file() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let trusted = [key.verifying_key().to_bytes()];
        let release = release_json("0.5.0");
        let sums = format!(
            "{}{}",
            line(RELEASE_FILE, &release),
            line("hub-x86_64-unknown-linux-musl", b"hub")
        );
        let signature = crate::crypto::sign(sums.as_bytes(), &key);
        let opened = open_sums(sums.as_bytes(), &signature, &trusted).unwrap();
        assert!(opened.matches("hub-x86_64-unknown-linux-musl", b"hub"));
        assert!(!opened.matches("hub-x86_64-unknown-linux-musl", b"hum"));
        assert!(!opened.matches("probes.tar.gz", b""));
        assert_eq!(open_release(&opened, &release).unwrap().version, "0.5.0");
        assert!(open_release(&opened, &release_json("0.6.0")).is_err());

        let stranger = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes();
        assert_eq!(
            open_sums(sums.as_bytes(), &signature, &[stranger]),
            Err(ReleaseError::BadSignature)
        );
        assert_eq!(
            open_sums(sums.as_bytes(), &signature, &[]),
            Err(ReleaseError::NoTrustedKeys)
        );
        let tampered = sums.replace("0", "1");
        assert_eq!(
            open_sums(tampered.as_bytes(), &signature, &trusted),
            Err(ReleaseError::BadSignature)
        );
    }

    #[test]
    fn sums_lines_must_be_plain_names_and_hex() {
        assert!(parse_sums(&line("hub.sh", b"")).is_ok());
        for bad in [
            format!("{}  ../hub", "a".repeat(64)),
            format!("{}  dir/hub", "a".repeat(64)),
            format!("{} hub", "a".repeat(64)),
            format!("{}  hub", "z".repeat(64)),
            "abc  hub".to_string(),
        ] {
            assert!(parse_sums(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hub_files_are_named_per_target() {
        assert_eq!(
            hub_file("x86_64-unknown-linux-musl"),
            "hub-x86_64-unknown-linux-musl"
        );
        assert_eq!(
            hub_file("x86_64-pc-windows-msvc"),
            "hub-x86_64-pc-windows-msvc.exe"
        );
        assert!(current_target().is_some(), "the machines tests run on");
    }
}
