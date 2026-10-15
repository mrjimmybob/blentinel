//! The signed probe releases the hub holds (M2-15, proposal 0012 §4, D-55),
//! in `<state_dir>/releases/<version>/`: `manifest.json`, its signature, and
//! the binaries it lists.
//!
//! The hub can't sign (D-41): it only checks a release against the release
//! keys compiled into it before taking it, and serves it as it came. From
//! them it offers:
//!
//! - **updates**: the version the admin approved, in its answer to a report
//!   from a probe running another version (none until one is approved);
//! - **downloads** for "Add probe" (D-53): the approved version, or else the
//!   newest one, with each binary's SHA-256 from the signed manifest.
//!
//! A release may also hold a signed rollback (`rollback.json`, D-74): which
//! newer versions may go back to it. It comes with the release, or later on
//! its own (`blentinelmake release rollback`), and a newer one replaces it.
//! Which probe is offered what, and when, is `service::rollout`.

use std::path::{Path, PathBuf};

use common::ack::{SignedRollback, UpdateOffer};
use common::protocol::{MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION};
use common::release::{
    MANIFEST_FILE, Manifest, Platform, ROLLBACK_FILE, ROLLBACK_SIGNATURE_FILE, ReleaseError,
    Rollback, SIGNATURE_FILE, Version, open_manifest, open_rollback,
};
use sha2::{Digest, Sha256};

use crate::model::{ProbeDownload, ProbeRelease};

/// The folder, inside the hub's `state_dir`.
pub const RELEASES_DIR: &str = "releases";
/// The largest release archive the hub takes (four probes are about 40 MB).
pub const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

/// Why a release wasn't added (or can't be found).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Release(#[from] ReleaseError),
    #[error(
        "its probes speak protocol {0}, this hub {MIN_SUPPORTED_PROTOCOL_VERSION} to {PROTOCOL_VERSION}"
    )]
    Protocol(u16),
    #[error("{0} is missing")]
    Missing(String),
    #[error("{0} isn't the file the manifest signs (size or SHA-256 differ)")]
    Mismatch(String),
    #[error("version {0} is already on the hub, built differently; remove it first")]
    Exists(String),
    #[error("not a release archive: {0}")]
    Archive(String),
    #[error("no release {0} on this hub")]
    NotFound(String),
    #[error("the rollback is for {rollback}, the release is {release}")]
    RollbackMismatch { rollback: String, release: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A release on the hub, as found on disk.
#[derive(Debug, Clone)]
pub struct Stored {
    pub manifest: Manifest,
    /// Its signed rollback, if it has one that checks out.
    pub rollback: Option<Rollback>,
    pub dir: PathBuf,
}

/// Add the release in folder `src` to `root`, checked against `keys`. The
/// release it now holds; adding the same release again changes nothing but
/// its rollback. A folder holding only a rollback adds it to the release on
/// the hub it is for.
pub fn add_from_dir(root: &Path, src: &Path, keys: &[[u8; 32]]) -> Result<Manifest, StoreError> {
    let read = |name: &str| {
        std::fs::read(src.join(name)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::Missing(name.to_string()),
            _ => e.into(),
        })
    };
    let rollback = if src.join(ROLLBACK_FILE).exists() {
        let bytes = read(ROLLBACK_FILE)?;
        let signature = String::from_utf8_lossy(&read(ROLLBACK_SIGNATURE_FILE)?).into_owned();
        let parsed = open_rollback(&bytes, &signature, keys)?;
        Some((parsed, bytes, signature))
    } else {
        None
    };
    if !src.join(MANIFEST_FILE).exists()
        && let Some((parsed, bytes, signature)) = &rollback
    {
        let stored = find(root, &parsed.version, keys)
            .ok_or_else(|| StoreError::NotFound(parsed.version.clone()))?;
        write_rollback(&stored.dir, bytes, signature)?;
        return Ok(stored.manifest);
    }
    let manifest_bytes = read(MANIFEST_FILE)?;
    let signature = String::from_utf8_lossy(&read(SIGNATURE_FILE)?).into_owned();
    let manifest = open_manifest(&manifest_bytes, &signature, keys)?;
    if !(MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&manifest.protocol) {
        return Err(StoreError::Protocol(manifest.protocol));
    }
    for artifact in &manifest.artifacts {
        let bytes = read(&artifact.file)?;
        if bytes.len() as u64 != artifact.size
            || !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(&artifact.sha256)
        {
            return Err(StoreError::Mismatch(artifact.file.clone()));
        }
    }
    if let Some((parsed, _, _)) = &rollback
        && parsed.version != manifest.version
    {
        return Err(StoreError::RollbackMismatch {
            rollback: parsed.version.clone(),
            release: manifest.version,
        });
    }

    let target = root.join(&manifest.version);
    if target.exists() {
        return match std::fs::read(target.join(MANIFEST_FILE)) {
            Ok(existing) if existing == manifest_bytes => {
                if let Some((_, bytes, signature)) = &rollback {
                    write_rollback(&target, bytes, signature)?;
                }
                Ok(manifest)
            }
            _ => Err(StoreError::Exists(manifest.version)),
        };
    }
    // Copied into a staging folder, then renamed: a half-copied release is
    // never offered.
    std::fs::create_dir_all(root)?;
    let staging = tempfile::Builder::new()
        .prefix(".incoming-")
        .tempdir_in(root)?;
    for name in [MANIFEST_FILE, SIGNATURE_FILE]
        .into_iter()
        .chain(manifest.artifacts.iter().map(|a| a.file.as_str()))
    {
        std::fs::copy(src.join(name), staging.path().join(name))?;
    }
    if let Some((_, bytes, signature)) = &rollback {
        write_rollback(staging.path(), bytes, signature)?;
    }
    std::fs::rename(staging.keep(), &target)?;
    Ok(manifest)
}

/// Put a checked rollback in release folder `dir`, replacing one there.
/// The signature goes first: until the rollback itself is replaced, the
/// pair doesn't check out and the release is offered without one.
fn write_rollback(dir: &Path, rollback: &[u8], signature: &str) -> Result<(), StoreError> {
    common::fs::write_atomic(&dir.join(ROLLBACK_SIGNATURE_FILE), signature.as_bytes())?;
    common::fs::write_atomic(&dir.join(ROLLBACK_FILE), rollback)?;
    Ok(())
}

/// Add a release from a `.tar.gz` holding its files at the top level (what
/// `blentinelmake release build` writes). Only plain file names are read:
/// the archive can't choose where anything goes.
pub fn add_from_archive(
    root: &Path,
    archive: &[u8],
    keys: &[[u8; 32]],
) -> Result<Manifest, StoreError> {
    std::fs::create_dir_all(root)?;
    let unpacked = tempfile::Builder::new()
        .prefix(".upload-")
        .tempdir_in(root)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let bad = |e: std::io::Error| StoreError::Archive(e.to_string());
    for entry in tar.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(bad)?.into_owned();
        let name = match path.to_str() {
            Some(name) if is_plain_name(name.trim_start_matches("./")) => {
                name.trim_start_matches("./").to_string()
            }
            _ => {
                return Err(StoreError::Archive(format!(
                    "unexpected entry {}",
                    path.display()
                )));
            }
        };
        let mut file = std::fs::File::create(unpacked.path().join(&name))?;
        std::io::copy(&mut entry, &mut file).map_err(bad)?;
    }
    add_from_dir(root, unpacked.path(), keys)
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':'])
}

/// Every release in `root` that still checks out against `keys`, newest first.
pub fn list(root: &Path, keys: &[[u8; 32]]) -> Vec<Stored> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut releases: Vec<Stored> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|name| find(root, name, keys))
        })
        .collect();
    releases.sort_by_key(|stored| std::cmp::Reverse(version_of(stored)));
    releases
}

fn version_of(stored: &Stored) -> Option<Version> {
    stored.manifest.parsed_version()
}

/// Release `version` in `root`, if it is there and checks out.
pub fn find(root: &Path, version: &str, keys: &[[u8; 32]]) -> Option<Stored> {
    find_with_offer(root, version, keys).map(|(stored, _)| stored)
}

/// A release as read from its folder, with the texts as signed.
struct Read {
    manifest: Manifest,
    offer: UpdateOffer,
    rollback: Option<(Rollback, SignedRollback)>,
}

/// The release in folder `version` (never a path), if it checks out; with
/// its rollback, if that does too and is for this version.
fn read(root: &Path, version: &str, keys: &[[u8; 32]]) -> Option<Read> {
    Version::parse(version)?; // also keeps `..` and separators out
    let dir = root.join(version);
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    let signature = std::fs::read_to_string(dir.join(SIGNATURE_FILE)).ok()?;
    let manifest = open_manifest(text.as_bytes(), &signature, keys).ok()?;
    if manifest.version != version {
        return None;
    }
    let rollback = (|| {
        let rollback = std::fs::read_to_string(dir.join(ROLLBACK_FILE)).ok()?;
        let signature = std::fs::read_to_string(dir.join(ROLLBACK_SIGNATURE_FILE)).ok()?;
        let parsed = open_rollback(rollback.as_bytes(), &signature, keys).ok()?;
        (parsed.version == version).then_some((
            parsed,
            SignedRollback {
                rollback,
                signature,
            },
        ))
    })();
    Some(Read {
        manifest,
        offer: UpdateOffer {
            manifest: text,
            signature,
            rollback: rollback.as_ref().map(|(_, signed)| signed.clone()),
        },
        rollback,
    })
}

/// The offer for `version`: its manifest, signature and rollback, as signed.
pub fn offer(root: &Path, version: &str, keys: &[[u8; 32]]) -> Option<UpdateOffer> {
    read(root, version, keys).map(|read| read.offer)
}

/// Release `version` and its offer, read once.
pub fn find_with_offer(
    root: &Path,
    version: &str,
    keys: &[[u8; 32]],
) -> Option<(Stored, UpdateOffer)> {
    let read = read(root, version, keys)?;
    Some((
        Stored {
            manifest: read.manifest,
            rollback: read.rollback.map(|(rollback, _)| rollback),
            dir: root.join(version),
        },
        read.offer,
    ))
}

/// Delete release `version`.
pub fn remove(root: &Path, version: &str, keys: &[[u8; 32]]) -> Result<(), StoreError> {
    let stored = find(root, version, keys).ok_or_else(|| StoreError::NotFound(version.into()))?;
    std::fs::remove_dir_all(stored.dir)?;
    Ok(())
}

/// The binary `file` of release `version`, if the manifest lists it.
pub fn binary(root: &Path, version: &str, file: &str, keys: &[[u8; 32]]) -> Option<PathBuf> {
    let stored = find(root, version, keys)?;
    stored
        .manifest
        .artifacts
        .iter()
        .any(|a| a.file == file)
        .then(|| stored.dir.join(file))
}

/// The release "Add probe" installs: the approved one, else the newest.
pub fn for_install(root: &Path, approved: Option<&str>, keys: &[[u8; 32]]) -> Option<Stored> {
    approved
        .and_then(|v| find(root, v, keys))
        .or_else(|| list(root, keys).into_iter().next())
}

/// The downloads a release offers, in [`common::release::PLATFORMS`] order.
pub fn downloads(stored: &Stored) -> Vec<ProbeDownload> {
    common::release::PLATFORMS
        .iter()
        .filter_map(|platform| {
            let artifact = stored.manifest.artifact_for(platform)?;
            Some(ProbeDownload {
                target: platform.target.to_string(),
                os: platform.os.to_string(),
                arch: platform.arch.to_string(),
                size: artifact.size,
                sha256: artifact.sha256.clone(),
                version: stored.manifest.version.clone(),
            })
        })
        .collect()
}

/// The path of the binary for `target` in a release, if it has one.
pub fn download_path(stored: &Stored, target: &str) -> Option<PathBuf> {
    let platform = Platform::by_target(target)?;
    stored
        .manifest
        .artifact_for(platform)
        .map(|a| stored.dir.join(&a.file))
}

/// What the admin page shows of a release.
pub fn describe(stored: &Stored) -> ProbeRelease {
    ProbeRelease {
        version: stored.manifest.version.clone(),
        released: stored
            .manifest
            .released
            .format("%Y-%m-%d %H:%M UTC")
            .to_string(),
        protocol: stored.manifest.protocol,
        platforms: downloads(stored)
            .into_iter()
            .map(|d| format!("{} {}", d.os, d.arch))
            .collect(),
        rollback_from: stored
            .rollback
            .as_ref()
            .map(|r| r.from.clone())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::release::{Artifact, MANIFEST_FORMAT, sign_manifest};
    use ed25519_dalek::SigningKey;

    const TARGET: &str = "x86_64-unknown-linux-musl";

    /// A signed release of `version` in a folder, and the key that trusts it.
    fn release(version: &str, binary: &[u8]) -> (tempfile::TempDir, [u8; 32]) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let dir = tempfile::tempdir().unwrap();
        let platform = Platform::by_target(TARGET).unwrap();
        let manifest = Manifest {
            format: MANIFEST_FORMAT,
            version: version.into(),
            released: "2026-10-04T10:00:00Z".parse().unwrap(),
            protocol: PROTOCOL_VERSION,
            artifacts: vec![Artifact {
                target: TARGET.into(),
                file: platform.file_name(),
                sha256: hex::encode(Sha256::digest(binary)),
                size: binary.len() as u64,
            }],
        };
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        std::fs::write(dir.path().join(MANIFEST_FILE), &bytes).unwrap();
        std::fs::write(dir.path().join(SIGNATURE_FILE), sign_manifest(&bytes, &key)).unwrap();
        std::fs::write(dir.path().join(platform.file_name()), binary).unwrap();
        (dir, key.verifying_key().to_bytes())
    }

    #[test]
    fn a_signed_release_is_added_listed_offered_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let (v1, key) = release("0.1.0", b"probe one");
        let (v2, _) = release("0.2.0", b"probe two");
        add_from_dir(root.path(), v1.path(), &[key]).unwrap();
        add_from_dir(root.path(), v2.path(), &[key]).unwrap();
        add_from_dir(root.path(), v2.path(), &[key]).unwrap(); // again: no change

        let listed: Vec<_> = list(root.path(), &[key])
            .into_iter()
            .map(|s| s.manifest.version)
            .collect();
        assert_eq!(listed, ["0.2.0", "0.1.0"]);

        let newest = for_install(root.path(), None, &[key]).unwrap();
        assert_eq!(newest.manifest.version, "0.2.0");
        let approved = for_install(root.path(), Some("0.1.0"), &[key]).unwrap();
        let downloads = downloads(&approved);
        assert_eq!(downloads.len(), 1);
        assert_eq!(
            downloads[0].sha256,
            hex::encode(Sha256::digest(b"probe one"))
        );
        assert_eq!(
            std::fs::read(download_path(&approved, TARGET).unwrap()).unwrap(),
            b"probe one"
        );

        let offer = offer(root.path(), "0.2.0", &[key]).unwrap();
        assert!(open_manifest(offer.manifest.as_bytes(), &offer.signature, &[key]).is_ok());
        assert!(
            binary(
                root.path(),
                "0.2.0",
                "probe-x86_64-unknown-linux-musl",
                &[key]
            )
            .is_some()
        );
        assert!(binary(root.path(), "0.2.0", "manifest.json", &[key]).is_none());
        assert!(
            binary(
                root.path(),
                "../0.2.0",
                "probe-x86_64-unknown-linux-musl",
                &[key]
            )
            .is_none()
        );

        remove(root.path(), "0.1.0", &[key]).unwrap();
        assert!(find(root.path(), "0.1.0", &[key]).is_none());
        assert!(matches!(
            remove(root.path(), "0.1.0", &[key]),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn untrusted_altered_or_clashing_releases_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let (v1, key) = release("0.1.0", b"probe");
        let stranger = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
        assert!(matches!(
            add_from_dir(root.path(), v1.path(), &[stranger]),
            Err(StoreError::Release(ReleaseError::BadSignature))
        ));
        std::fs::write(v1.path().join("probe-x86_64-unknown-linux-musl"), b"probf").unwrap();
        assert!(matches!(
            add_from_dir(root.path(), v1.path(), &[key]),
            Err(StoreError::Mismatch(_))
        ));
        std::fs::remove_file(v1.path().join("probe-x86_64-unknown-linux-musl")).unwrap();
        assert!(matches!(
            add_from_dir(root.path(), v1.path(), &[key]),
            Err(StoreError::Missing(_))
        ));

        let (good, _) = release("0.1.0", b"probe");
        add_from_dir(root.path(), good.path(), &[key]).unwrap();
        let (rebuilt, _) = release("0.1.0", b"another build");
        assert!(matches!(
            add_from_dir(root.path(), rebuilt.path(), &[key]),
            Err(StoreError::Exists(_))
        ));
        // Nothing half-copied left behind.
        let folders = std::fs::read_dir(root.path()).unwrap().count();
        assert_eq!(folders, 1);
    }

    /// Write a rollback to `version` from `from` into `dir`, signed with the
    /// test release key.
    fn write_signed_rollback(dir: &Path, version: &str, from: &[&str]) {
        let rollback = Rollback {
            format: MANIFEST_FORMAT,
            version: version.into(),
            from: from.iter().map(|v| v.to_string()).collect(),
            signed: "2026-10-05T10:00:00Z".parse().unwrap(),
        };
        let bytes = serde_json::to_vec_pretty(&rollback).unwrap();
        let key = SigningKey::from_bytes(&[7; 32]);
        std::fs::write(dir.join(ROLLBACK_FILE), &bytes).unwrap();
        std::fs::write(
            dir.join(ROLLBACK_SIGNATURE_FILE),
            common::release::sign_rollback(&bytes, &key),
        )
        .unwrap();
    }

    #[test]
    fn a_rollback_comes_with_its_release_or_later_and_rides_in_the_offer() {
        let root = tempfile::tempdir().unwrap();
        let (v1, key) = release("0.1.0", b"probe one");
        add_from_dir(root.path(), v1.path(), &[key]).unwrap();
        assert_eq!(find(root.path(), "0.1.0", &[key]).unwrap().rollback, None);
        assert_eq!(offer(root.path(), "0.1.0", &[key]).unwrap().rollback, None);

        // Later, on its own.
        let alone = tempfile::tempdir().unwrap();
        write_signed_rollback(alone.path(), "0.1.0", &["0.2.0"]);
        add_from_dir(root.path(), alone.path(), &[key]).unwrap();
        let stored = find(root.path(), "0.1.0", &[key]).unwrap();
        assert_eq!(stored.rollback.unwrap().from, ["0.2.0"]);
        assert_eq!(
            describe(&find(root.path(), "0.1.0", &[key]).unwrap()).rollback_from,
            ["0.2.0"]
        );
        let signed = offer(root.path(), "0.1.0", &[key])
            .unwrap()
            .rollback
            .unwrap();
        let opened =
            common::release::open_rollback(signed.rollback.as_bytes(), &signed.signature, &[key]);
        assert!(opened.unwrap().allows("0.2.0"));

        // A newer one, with the release again, replaces it.
        write_signed_rollback(v1.path(), "0.1.0", &["0.2.0", "0.2.1"]);
        add_from_dir(root.path(), v1.path(), &[key]).unwrap();
        let from = find(root.path(), "0.1.0", &[key])
            .unwrap()
            .rollback
            .unwrap()
            .from;
        assert_eq!(from, ["0.2.0", "0.2.1"]);

        // One for a release the hub hasn't got, or for another release.
        let stray = tempfile::tempdir().unwrap();
        write_signed_rollback(stray.path(), "0.0.9", &["0.1.0"]);
        assert!(matches!(
            add_from_dir(root.path(), stray.path(), &[key]),
            Err(StoreError::NotFound(_))
        ));
        let (v2, _) = release("0.2.0", b"probe two");
        write_signed_rollback(v2.path(), "0.1.0", &["0.2.0"]);
        assert!(matches!(
            add_from_dir(root.path(), v2.path(), &[key]),
            Err(StoreError::RollbackMismatch { .. })
        ));
        // Unsigned by a trusted key.
        let stranger = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
        assert!(matches!(
            add_from_dir(root.path(), alone.path(), &[stranger]),
            Err(StoreError::Release(_))
        ));
    }

    #[test]
    fn an_archive_with_paths_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (v1, key) = release("0.1.0", b"probe");
        let archive = |names: &[(&str, &str)]| {
            let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::fast(),
            ));
            for (path, file) in names {
                tar.append_path_with_name(v1.path().join(file), path)
                    .unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap()
        };
        let files = [
            MANIFEST_FILE,
            SIGNATURE_FILE,
            "probe-x86_64-unknown-linux-musl",
        ];
        let flat: Vec<_> = files.iter().map(|f| (*f, *f)).collect();
        let added = add_from_archive(root.path(), &archive(&flat), &[key]).unwrap();
        assert_eq!(added.version, "0.1.0");

        let nested = [("sub/manifest.json", MANIFEST_FILE)];
        assert!(matches!(
            add_from_archive(root.path(), &archive(&nested), &[key]),
            Err(StoreError::Archive(_))
        ));
        assert!(matches!(
            add_from_archive(root.path(), b"not gzip", &[key]),
            Err(StoreError::Archive(_))
        ));
    }
}
