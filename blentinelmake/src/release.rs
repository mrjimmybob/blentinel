//! Signed probe releases (M2-15, proposal 0012 §4, D-41, D-55).
//!
//! - `release keygen`: an Ed25519 key pair in the key folder. The private
//!   key never leaves this machine (the hub can't sign, D-41).
//! - Every hub and probe build gets the public keys of the key folder in
//!   `BLENTINEL_RELEASE_KEYS` ([`apply_keys`]): the probe installs only
//!   releases signed with one of them, and the hub takes only those.
//! - `release sign <folder>`: `manifest.json` (each `probe-<target>` binary's
//!   SHA-256) and `manifest.json.sig` for the binaries in a folder.
//! - `release build`: build the probes, sign them, and write the folder and
//!   the `.tar.gz` the hub's admin page takes.
//! - `release rollback`: a signed `rollback.json` letting probes on newer
//!   versions go back to an older release (D-74).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::release::{
    Artifact, MANIFEST_FILE, MANIFEST_FORMAT, Manifest, PLATFORMS, Platform, RELEASE_KEYS_ENV,
    ROLLBACK_FILE, ROLLBACK_SIGNATURE_FILE, Rollback, SIGNATURE_FILE,
};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

/// Overrides where the release keys are kept.
const KEY_DIR_ENV: &str = "BLENTINEL_RELEASE_KEY_DIR";
const PRIVATE_KEY: &str = "release.key";
const PUBLIC_KEY: &str = "release.pub";

/// The key folder: `$BLENTINEL_RELEASE_KEY_DIR`, else `blentinel/release-keys`
/// in the user's configuration folder (outside the source tree, so the
/// private key is never committed or shipped with it).
pub fn key_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(KEY_DIR_ENV) {
        return dir.into();
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("blentinel")
        .join("release-keys")
}

/// `release keygen`: create the key pair (never replacing one).
pub fn keygen() -> Result<(), String> {
    let dir = key_dir();
    let private = dir.join(PRIVATE_KEY);
    if private.exists() {
        return Err(format!(
            "{} already exists. Every probe built so far trusts it; keep it. \
             To rotate, move it aside first: probes trust the keys of every \
             .pub file in the folder.",
            private.display()
        ));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    let key = SigningKey::generate(&mut rand::rngs::OsRng);
    common::fs::write_secret_file(&private, key.to_bytes())
        .map_err(|e| format!("Can't write {}: {e}", private.display()))?;
    let public = hex::encode(key.verifying_key().to_bytes());
    fs::write(dir.join(PUBLIC_KEY), format!("{public}\n"))
        .map_err(|e| format!("Can't write the public key: {e}"))?;
    println!("Release key created in {}", dir.display());
    println!(
        "  Private: {PRIVATE_KEY}  (keep it here, or on a token; back it up; never on the hub)"
    );
    println!("  Public:  {PUBLIC_KEY}   {public}");
    println!("Hubs and probes built from now on trust it: build them again.");
    Ok(())
}

/// The public keys of every `.pub` file in the key folder, as hex.
pub fn public_keys() -> Result<Vec<String>, String> {
    let dir = key_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut keys = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "pub") {
            continue;
        }
        let text =
            fs::read_to_string(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
        let key = text.trim().to_string();
        if common::release::parse_keys(&key).len() != 1 {
            return Err(format!("{} isn't a hex Ed25519 public key", path.display()));
        }
        keys.push(key);
    }
    keys.sort();
    Ok(keys)
}

/// Give a hub or probe build the release keys to compile in.
pub fn apply_keys(command: &mut Command) -> Result<(), String> {
    let keys = public_keys()?;
    if keys.is_empty() {
        println!(
            "Note: no release key in {} (blentinelmake release keygen): this build can't update probes.",
            key_dir().display()
        );
    }
    command.env(RELEASE_KEYS_ENV, keys.join(","));
    Ok(())
}

/// The private key, for signing.
pub fn signing_key() -> Result<SigningKey, String> {
    let path = key_dir().join(PRIVATE_KEY);
    let bytes = common::fs::load_key(&path).map_err(|e| {
        format!(
            "Can't read the release key {} ({e}). Create one with `blentinelmake release keygen`.",
            path.display()
        )
    })?;
    Ok(SigningKey::from_bytes(&bytes))
}

/// The workspace version (`[workspace.package]` in Cargo.toml), read now
/// rather than compiled in, so a stale `blentinelmake` can't mislabel a release.
pub fn workspace_version() -> Result<String, String> {
    let text = fs::read_to_string("Cargo.toml")
        .map_err(|e| format!("Run this from the workspace folder (Cargo.toml: {e})"))?;
    let doc: toml::Table = text.parse().map_err(|e| format!("Cargo.toml: {e}"))?;
    doc.get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Cargo.toml has no [workspace.package] version".to_string())
}

/// `release sign <folder>`: sign the `probe-<target>` binaries in `folder`
/// as release `version`.
pub fn sign(folder: &Path, version: &str) -> Result<Manifest, String> {
    let key = signing_key()?;
    let mut artifacts = Vec::new();
    for platform in &PLATFORMS {
        let file = platform.file_name();
        let Ok(bytes) = fs::read(folder.join(&file)) else {
            continue;
        };
        artifacts.push(Artifact {
            target: platform.target.to_string(),
            file,
            sha256: hex::encode(Sha256::digest(&bytes)),
            size: bytes.len() as u64,
        });
    }
    if artifacts.is_empty() {
        return Err(format!(
            "No probe binaries in {} (named probe-<target>, e.g. probe-x86_64-unknown-linux-musl)",
            folder.display()
        ));
    }
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        version: version.to_string(),
        released: chrono::Utc::now(),
        protocol: common::protocol::PROTOCOL_VERSION,
        artifacts,
    };
    manifest.validate().map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    let signature = common::release::sign_manifest(&bytes, &key);
    // Checked as a probe will check it, before anything is shipped.
    common::release::open_manifest(&bytes, &signature, &[key.verifying_key().to_bytes()])
        .map_err(|e| format!("The signed manifest doesn't verify: {e}"))?;
    fs::write(folder.join(MANIFEST_FILE), &bytes).map_err(|e| e.to_string())?;
    fs::write(folder.join(SIGNATURE_FILE), format!("{signature}\n")).map_err(|e| e.to_string())?;
    println!(
        "Signed release {version}: {}",
        manifest
            .artifacts
            .iter()
            .map(|a| a.file.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(manifest)
}

/// Build the probe for each of `targets` with `build` (which returns the
/// binary's path) into `folder`, then sign them.
pub fn build_into(
    folder: &Path,
    targets: &[String],
    build: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<Manifest, String> {
    signing_key()?; // fail before a long build, not after
    let version = workspace_version()?;
    fs::create_dir_all(folder).map_err(|e| format!("Can't create {}: {e}", folder.display()))?;
    for target in targets {
        let platform = Platform::by_target(target).ok_or_else(|| {
            format!(
                "{target} isn't a release target ({})",
                PLATFORMS.map(|p| p.target).join(", ")
            )
        })?;
        let binary = build(target)?;
        fs::copy(&binary, folder.join(platform.file_name()))
            .map_err(|e| format!("Can't copy {}: {e}", binary.display()))?;
    }
    sign(folder, &version)
}

/// `release build --targets …`: `publish/probe-release-<version>/` and its
/// `.tar.gz`, for `hub release add` or the admin page.
pub fn build(
    targets: &[String],
    build: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<(), String> {
    let version = workspace_version()?;
    let folder = Path::new("publish").join(format!("probe-release-{version}"));
    if folder.exists() {
        fs::remove_dir_all(&folder)
            .map_err(|e| format!("Can't clear {}: {e}", folder.display()))?;
    }
    let manifest = build_into(&folder, targets, build)?;
    let archive = folder.with_extension("tar.gz");
    write_archive(&folder, &manifest, &archive)?;
    println!("\nRelease {version}:");
    println!("  {}", folder.display());
    println!(
        "  {}  (admin page > Probe updates > Add release)",
        archive.display()
    );
    Ok(())
}

/// `release rollback --to <version> --from <version,...>`: let probes on the
/// `from` versions go back to release `to` (D-74), signed with the release
/// key. Writes `publish/probe-rollback-<to>/` and its `.tar.gz`, which the
/// hub adds to its release `to` (the admin page's Add release, or `hub
/// release add <folder>`).
pub fn rollback(to: &str, from: &[String]) -> Result<(), String> {
    let key = signing_key()?;
    let rollback = Rollback {
        format: MANIFEST_FORMAT,
        version: to.to_string(),
        from: from.to_vec(),
        signed: chrono::Utc::now(),
    };
    rollback.validate().map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&rollback).map_err(|e| e.to_string())?;
    let signature = common::release::sign_rollback(&bytes, &key);
    common::release::open_rollback(&bytes, &signature, &[key.verifying_key().to_bytes()])
        .map_err(|e| format!("The signed rollback doesn't verify: {e}"))?;

    let folder = Path::new("publish").join(format!("probe-rollback-{to}"));
    fs::create_dir_all(&folder).map_err(|e| format!("Can't create {}: {e}", folder.display()))?;
    fs::write(folder.join(ROLLBACK_FILE), &bytes).map_err(|e| e.to_string())?;
    fs::write(
        folder.join(ROLLBACK_SIGNATURE_FILE),
        format!("{signature}\n"),
    )
    .map_err(|e| e.to_string())?;
    let archive = folder.with_extension("tar.gz");
    archive_files(&folder, &[ROLLBACK_FILE, ROLLBACK_SIGNATURE_FILE], &archive)?;
    println!("Signed rollback to {to} from {}:", from.join(", "));
    println!("  {}", folder.display());
    println!(
        "  {}  (admin page > Probe updates > Add release; then approve {to})",
        archive.display()
    );
    Ok(())
}

/// The release's files, at the top level of a `.tar.gz`.
pub fn write_archive(folder: &Path, manifest: &Manifest, archive: &Path) -> Result<(), String> {
    let names: Vec<&str> = [MANIFEST_FILE, SIGNATURE_FILE]
        .into_iter()
        .chain(manifest.artifacts.iter().map(|a| a.file.as_str()))
        .collect();
    archive_files(folder, &names, archive)
}

/// Files `names` of `folder`, at the top level of a `.tar.gz`.
fn archive_files(folder: &Path, names: &[&str], archive: &Path) -> Result<(), String> {
    let file = fs::File::create(archive)
        .map_err(|e| format!("Can't create {}: {e}", archive.display()))?;
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::best(),
    ));
    for &name in names {
        tar.append_path_with_name(folder.join(name), name)
            .map_err(|e| format!("Can't archive {name}: {e}"))?;
    }
    tar.into_inner()
        .and_then(|gz| gz.finish())
        .map_err(|e| format!("Can't write {}: {e}", archive.display()))?;
    Ok(())
}
