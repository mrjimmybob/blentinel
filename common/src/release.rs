//! Signed probe releases (M2-15, proposal 0012, D-41, D-55).
//!
//! A release is a folder: the probe binaries, `manifest.json` listing each
//! one's SHA-256, and `manifest.json.sig`, an Ed25519 signature of the
//! manifest's bytes (hex). One signature covers every binary.
//!
//! The private key stays on the machine that builds and signs (`blentinelmake
//! release keygen`); the public keys are compiled into the probe and the hub
//! ([`trusted_keys`]). The hub checks a release when it is added, and the
//! probe checks it again before installing it: neither the hub nor the
//! network is trusted with what a probe runs.
//!
//! A probe never installs an older version, so a hacked hub can't replay an
//! old, vulnerable release. Going back takes a **signed rollback** (M4-01,
//! D-74): `rollback.json` beside a release names the versions that may
//! return to it, signed with a release key too.

use std::cmp::Ordering;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The manifest's file name in a release folder.
pub const MANIFEST_FILE: &str = "manifest.json";
/// The signature's file name in a release folder.
pub const SIGNATURE_FILE: &str = "manifest.json.sig";
/// The manifest format this build reads and writes.
pub const MANIFEST_FORMAT: u32 = 1;

/// A platform the hub offers probes for (D-13: Linux builds are static musl).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// Rust target triple, e.g. `x86_64-unknown-linux-musl`.
    pub target: &'static str,
    /// `linux` or `windows`.
    pub os: &'static str,
    /// How the UI names the architecture: `x86_64`, `arm64`, `armv7`.
    pub arch: &'static str,
    /// [`std::env::consts::ARCH`] on that platform, so a probe finds its own
    /// binary whatever target it was built for (a `gnu` build takes `musl`).
    std_arch: &'static str,
}

/// Every platform, in the order the UI lists them.
pub const PLATFORMS: [Platform; 4] = [
    Platform {
        target: "x86_64-unknown-linux-musl",
        os: "linux",
        arch: "x86_64",
        std_arch: "x86_64",
    },
    Platform {
        target: "aarch64-unknown-linux-musl",
        os: "linux",
        arch: "arm64",
        std_arch: "aarch64",
    },
    Platform {
        target: "armv7-unknown-linux-musleabihf",
        os: "linux",
        arch: "armv7",
        std_arch: "arm",
    },
    Platform {
        target: "x86_64-pc-windows-msvc",
        os: "windows",
        arch: "x86_64",
        std_arch: "x86_64",
    },
];

impl Platform {
    /// The platform for a target triple, if it is one of [`PLATFORMS`].
    pub fn by_target(target: &str) -> Option<&'static Platform> {
        PLATFORMS.iter().find(|p| p.target == target)
    }

    /// The platform this program runs on, if it is one of [`PLATFORMS`].
    pub fn current() -> Option<&'static Platform> {
        PLATFORMS
            .iter()
            .find(|p| p.os == std::env::consts::OS && p.std_arch == std::env::consts::ARCH)
    }

    /// The binary's file name in a release: `probe-<target>`, `.exe` on Windows.
    pub fn file_name(&self) -> String {
        let exe = if self.os == "windows" { ".exe" } else { "" };
        format!("probe-{}{exe}", self.target)
    }
}

/// A version number: `MAJOR.MINOR.PATCH`, optionally `-pre` (SemVer order:
/// a pre-release comes before its release).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    numbers: [u64; 3],
    pre: Option<String>,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        // The pre-release part ends up in download paths: letters, digits, `.`, `-`.
        let pre_ok = |pre: &str| {
            !pre.is_empty()
                && pre
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        };
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) if pre_ok(pre) => (core, Some(pre.to_string())),
            Some(_) => return None,
            None => (text, None),
        };
        let mut parts = core.split('.').map(|n| n.parse::<u64>().ok());
        let numbers = [parts.next()??, parts.next()??, parts.next()??];
        if parts.next().is_some() {
            return None;
        }
        Some(Self { numbers, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers
            .cmp(&other.numbers)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.numbers;
        write!(f, "{major}.{minor}.{patch}")?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// What a release holds. Signed as the bytes of `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`MANIFEST_FORMAT`].
    pub format: u32,
    /// The probe version inside (the workspace version it was built from).
    pub version: String,
    pub released: DateTime<Utc>,
    /// The protocol version these probes speak; a hub refuses a release
    /// its probes can't report to.
    pub protocol: u16,
    pub artifacts: Vec<Artifact>,
}

/// One probe binary in a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// One of [`PLATFORMS`].
    pub target: String,
    /// Always [`Platform::file_name`]: a manifest can't name other files.
    pub file: String,
    /// Hex SHA-256 of the file.
    pub sha256: String,
    pub size: u64,
}

/// Why a release can't be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReleaseError {
    #[error("this build trusts no release key (built without `blentinelmake release keygen`)")]
    NoTrustedKeys,
    #[error("the release isn't signed by a trusted release key")]
    BadSignature,
    #[error("the manifest isn't valid: {0}")]
    Invalid(String),
}

impl Manifest {
    /// The parsed [`Manifest::version`].
    pub fn parsed_version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }

    /// The binary for `platform`, if the release has one.
    pub fn artifact_for(&self, platform: &Platform) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.target == platform.target)
    }

    /// Checks a well-formed manifest passes: known format, a version, known
    /// targets each named canonically and listed once, SHA-256 in hex.
    pub fn validate(&self) -> Result<(), ReleaseError> {
        let invalid = |why: String| Err(ReleaseError::Invalid(why));
        if self.format != MANIFEST_FORMAT {
            return invalid(format!("unknown format {}", self.format));
        }
        if self.parsed_version().is_none() {
            return invalid(format!("\"{}\" isn't a version", self.version));
        }
        if self.artifacts.is_empty() {
            return invalid("it lists no binaries".into());
        }
        for (i, artifact) in self.artifacts.iter().enumerate() {
            let Some(platform) = Platform::by_target(&artifact.target) else {
                return invalid(format!("unknown target {}", artifact.target));
            };
            if artifact.file != platform.file_name() {
                return invalid(format!(
                    "{} must be named {}",
                    artifact.file,
                    platform.file_name()
                ));
            }
            if artifact.sha256.len() != 64
                || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return invalid(format!("{}: the SHA-256 isn't hex", artifact.file));
            }
            if self.artifacts[..i]
                .iter()
                .any(|a| a.target == artifact.target)
            {
                return invalid(format!("{} is listed twice", artifact.target));
            }
        }
        Ok(())
    }
}

/// The release public keys compiled into this build: the hex keys in the
/// `BLENTINEL_RELEASE_KEYS` environment variable at build time (comma or
/// space separated; `blentinelmake` sets it from its key folder). Empty
/// when built without one: such a build accepts no release.
pub fn trusted_keys() -> Vec<[u8; 32]> {
    parse_keys(option_env!("BLENTINEL_RELEASE_KEYS").unwrap_or(""))
}

/// The environment variable [`trusted_keys`] is read from at build time.
pub const RELEASE_KEYS_ENV: &str = "BLENTINEL_RELEASE_KEYS";

/// Hex keys separated by commas or white space; anything else is skipped
/// (`blentinelmake` checks them before a build).
pub fn parse_keys(text: &str) -> Vec<[u8; 32]> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|k| hex::decode(k).ok()?.try_into().ok())
        .collect()
}

/// Sign `manifest` (its JSON bytes) with a release key; the signature as
/// written to [`SIGNATURE_FILE`].
pub fn sign_manifest(manifest: &[u8], key: &ed25519_dalek::SigningKey) -> String {
    hex::encode(crate::crypto::sign(manifest, key))
}

/// Check `manifest` (the bytes of `manifest.json`) against `signature` (the
/// text of `manifest.json.sig`) and `keys`, then parse and validate it.
pub fn open_manifest(
    manifest: &[u8],
    signature: &str,
    keys: &[[u8; 32]],
) -> Result<Manifest, ReleaseError> {
    if keys.is_empty() {
        return Err(ReleaseError::NoTrustedKeys);
    }
    let signature = hex::decode(signature.trim()).map_err(|_| ReleaseError::BadSignature)?;
    if !keys
        .iter()
        .any(|key| crate::crypto::verify_with_key(manifest, &signature, key).is_ok())
    {
        return Err(ReleaseError::BadSignature);
    }
    let parsed: Manifest = serde_json::from_slice(manifest)
        .map_err(|e| ReleaseError::Invalid(format!("not a manifest: {e}")))?;
    parsed.validate()?;
    Ok(parsed)
}

/// The signed rollback's file name in a release folder.
pub const ROLLBACK_FILE: &str = "rollback.json";
/// Its signature's file name.
pub const ROLLBACK_SIGNATURE_FILE: &str = "rollback.json.sig";
/// Prefixed to a rollback's bytes before signing, so no signature over a
/// manifest (or anything else signed with a release key) is ever one over
/// a rollback, and the other way round.
const ROLLBACK_DOMAIN: &[u8] = b"blentinel probe rollback v1\n";

/// Permission, signed with a release key, for probes running one of `from`
/// to install the older release `version` (M4-01, D-74).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rollback {
    /// [`MANIFEST_FORMAT`].
    pub format: u32,
    /// The release probes go back to.
    pub version: String,
    /// The versions that may go back to it, each newer than `version`.
    pub from: Vec<String>,
    pub signed: DateTime<Utc>,
}

impl Rollback {
    /// May a probe running `own` go back to [`Rollback::version`]?
    pub fn allows(&self, own: &str) -> bool {
        self.from.iter().any(|from| from == own)
    }

    /// A known format, versions that parse, and each `from` newer.
    pub fn validate(&self) -> Result<(), ReleaseError> {
        let invalid = |why: String| Err(ReleaseError::Invalid(why));
        if self.format != MANIFEST_FORMAT {
            return invalid(format!("unknown format {}", self.format));
        }
        let Some(to) = Version::parse(&self.version) else {
            return invalid(format!("\"{}\" isn't a version", self.version));
        };
        if self.from.is_empty() {
            return invalid("it names no version to go back from".into());
        }
        for from in &self.from {
            match Version::parse(from) {
                Some(v) if v > to => {}
                Some(_) => return invalid(format!("{from} isn't newer than {to}")),
                None => return invalid(format!("\"{from}\" isn't a version")),
            }
        }
        Ok(())
    }
}

/// Sign `rollback` (its JSON bytes) with a release key; the signature as
/// written to [`ROLLBACK_SIGNATURE_FILE`].
pub fn sign_rollback(rollback: &[u8], key: &ed25519_dalek::SigningKey) -> String {
    hex::encode(crate::crypto::sign(&rollback_message(rollback), key))
}

fn rollback_message(rollback: &[u8]) -> Vec<u8> {
    [ROLLBACK_DOMAIN, rollback].concat()
}

/// Check `rollback` (the bytes of `rollback.json`) against `signature` and
/// `keys`, then parse and validate it.
pub fn open_rollback(
    rollback: &[u8],
    signature: &str,
    keys: &[[u8; 32]],
) -> Result<Rollback, ReleaseError> {
    if keys.is_empty() {
        return Err(ReleaseError::NoTrustedKeys);
    }
    let signature = hex::decode(signature.trim()).map_err(|_| ReleaseError::BadSignature)?;
    let message = rollback_message(rollback);
    if !keys
        .iter()
        .any(|key| crate::crypto::verify_with_key(&message, &signature, key).is_ok())
    {
        return Err(ReleaseError::BadSignature);
    }
    let parsed: Rollback = serde_json::from_slice(rollback)
        .map_err(|e| ReleaseError::Invalid(format!("not a rollback: {e}")))?;
    parsed.validate()?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn rollback(version: &str, from: &[&str]) -> Rollback {
        Rollback {
            format: MANIFEST_FORMAT,
            version: version.into(),
            from: from.iter().map(|v| v.to_string()).collect(),
            signed: "2026-11-03T10:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn a_signed_rollback_opens_and_names_who_may_go_back() {
        let key = SigningKey::generate(&mut OsRng);
        let trusted = [key.verifying_key().to_bytes()];
        let bytes = serde_json::to_vec(&rollback("0.4.1", &["0.4.2", "0.4.3"])).unwrap();
        let signature = sign_rollback(&bytes, &key);
        let opened = open_rollback(&bytes, &signature, &trusted).unwrap();
        assert!(opened.allows("0.4.2") && opened.allows("0.4.3"));
        assert!(!opened.allows("0.4.4") && !opened.allows("0.4.1"));

        let stranger = SigningKey::generate(&mut OsRng).verifying_key().to_bytes();
        assert_eq!(
            open_rollback(&bytes, &signature, &[stranger]),
            Err(ReleaseError::BadSignature)
        );
        assert_eq!(
            open_rollback(&bytes, &signature, &[]),
            Err(ReleaseError::NoTrustedKeys)
        );
        let widened = serde_json::to_vec(&rollback("0.4.1", &["0.4.2", "0.4.9"])).unwrap();
        assert_eq!(
            open_rollback(&widened, &signature, &trusted),
            Err(ReleaseError::BadSignature)
        );
    }

    #[test]
    fn a_manifest_signature_is_never_a_rollback_signature() {
        // The same bytes, signed as a manifest: refused as a rollback, and
        // the other way round (the signatures are domain separated).
        let key = SigningKey::generate(&mut OsRng);
        let trusted = [key.verifying_key().to_bytes()];
        let bytes = serde_json::to_vec(&rollback("0.4.1", &["0.4.2"])).unwrap();
        assert_eq!(
            open_rollback(&bytes, &sign_manifest(&bytes, &key), &trusted),
            Err(ReleaseError::BadSignature)
        );
        let manifest_bytes = serde_json::to_vec(&manifest()).unwrap();
        assert_eq!(
            open_manifest(
                &manifest_bytes,
                &sign_rollback(&manifest_bytes, &key),
                &trusted
            ),
            Err(ReleaseError::BadSignature)
        );
    }

    #[test]
    fn a_rollback_goes_only_to_an_older_version() {
        let key = SigningKey::generate(&mut OsRng);
        let trusted = [key.verifying_key().to_bytes()];
        for bad in [
            rollback("0.4.1", &[]),
            rollback("0.4.1", &["0.4.1"]),
            rollback("0.4.1", &["0.4.0"]),
            rollback("0.4.1", &["latest"]),
            rollback("old", &["0.4.2"]),
        ] {
            let bytes = serde_json::to_vec(&bad).unwrap();
            assert!(
                matches!(
                    open_rollback(&bytes, &sign_rollback(&bytes, &key), &trusted),
                    Err(ReleaseError::Invalid(_))
                ),
                "{bad:?}"
            );
        }
    }

    fn manifest() -> Manifest {
        Manifest {
            format: MANIFEST_FORMAT,
            version: "0.4.2".into(),
            released: "2026-11-02T10:00:00Z".parse().unwrap(),
            protocol: 5,
            artifacts: vec![Artifact {
                target: "x86_64-pc-windows-msvc".into(),
                file: "probe-x86_64-pc-windows-msvc.exe".into(),
                sha256: "ab".repeat(32),
                size: 10,
            }],
        }
    }

    fn signed(manifest: &Manifest) -> (Vec<u8>, String, [u8; 32]) {
        let key = SigningKey::generate(&mut OsRng);
        let bytes = serde_json::to_vec(manifest).unwrap();
        let signature = sign_manifest(&bytes, &key);
        (bytes, signature, key.verifying_key().to_bytes())
    }

    #[test]
    fn versions_order_like_semver() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.4.10") > v("0.4.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.5.0") > v("0.5.0-rc1"));
        assert!(v("0.5.0-rc2") > v("0.5.0-rc1"));
        assert_eq!(v("0.4.2"), v("0.4.2"));
        assert_eq!(v("0.5.0-rc1").to_string(), "0.5.0-rc1");
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.x.3",
            "1.2.3-",
            "v1.2.3",
            "1.2.3-a/b",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_signed_manifest_opens_with_a_trusted_key() {
        let (bytes, signature, key) = signed(&manifest());
        let other = SigningKey::generate(&mut OsRng).verifying_key().to_bytes();
        let opened = open_manifest(&bytes, &format!("{signature}\n"), &[other, key]).unwrap();
        assert_eq!(opened, manifest());
    }

    #[test]
    fn untrusted_keys_and_tampering_are_refused() {
        let (bytes, signature, key) = signed(&manifest());
        let stranger = SigningKey::generate(&mut OsRng).verifying_key().to_bytes();
        assert_eq!(
            open_manifest(&bytes, &signature, &[stranger]),
            Err(ReleaseError::BadSignature)
        );
        assert_eq!(
            open_manifest(&bytes, &signature, &[]),
            Err(ReleaseError::NoTrustedKeys)
        );
        assert_eq!(
            open_manifest(&bytes, "not hex", &[key]),
            Err(ReleaseError::BadSignature)
        );
        // Another binary's hash, same signature.
        let mut tampered = manifest();
        tampered.artifacts[0].sha256 = "cd".repeat(32);
        let tampered = serde_json::to_vec(&tampered).unwrap();
        assert_eq!(
            open_manifest(&tampered, &signature, &[key]),
            Err(ReleaseError::BadSignature)
        );
    }

    #[test]
    fn a_manifest_names_only_known_binaries() {
        let check = |change: fn(&mut Manifest)| {
            let mut m = manifest();
            change(&mut m);
            let (bytes, signature, key) = signed(&m);
            open_manifest(&bytes, &signature, &[key])
        };
        assert!(check(|_| {}).is_ok());
        for change in [
            (|m: &mut Manifest| m.format = 2) as fn(&mut Manifest),
            |m| m.version = "latest".into(),
            |m| m.artifacts.clear(),
            |m| m.artifacts[0].target = "sparc-sun-solaris".into(),
            |m| m.artifacts[0].file = "../hub.exe".into(),
            |m| m.artifacts[0].sha256 = "zz".repeat(32),
            |m| m.artifacts.push(m.artifacts[0].clone()),
        ] {
            assert!(matches!(check(change), Err(ReleaseError::Invalid(_))));
        }
    }

    #[test]
    fn platforms_and_file_names() {
        let windows = Platform::by_target("x86_64-pc-windows-msvc").unwrap();
        assert_eq!(windows.file_name(), "probe-x86_64-pc-windows-msvc.exe");
        assert_eq!(
            Platform::by_target("armv7-unknown-linux-musleabihf")
                .unwrap()
                .file_name(),
            "probe-armv7-unknown-linux-musleabihf"
        );
        assert!(Platform::by_target("x86_64-unknown-linux-gnu").is_none());
        // The machines the tests run on are ones the hub offers probes for.
        assert!(Platform::current().is_some());
        assert_eq!(manifest().artifact_for(windows).map(|a| a.size), Some(10));
    }

    #[test]
    fn keys_are_read_from_hex_lists() {
        let a = "11".repeat(32);
        let b = "22".repeat(32);
        assert_eq!(
            parse_keys(&format!("{a}, {b}\nnot-a-key")),
            vec![[0x11; 32], [0x22; 32]]
        );
        assert!(parse_keys("").is_empty());
    }
}
