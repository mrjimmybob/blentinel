//! `hub backup` and `hub restore` (D-52).
//!
//! A backup is one `.tar.gz`: a consistent copy of the database (made while
//! the hub runs), the identity key, the TLS certificate and key, the config,
//! and `manifest.json`. It holds the hub's private key, so it is written
//! readable by its owner only: keep it like the key itself.
//!
//! Restore puts the files where the *current* config says (a new server has
//! its own paths), moves what was there aside first, and keeps the backed-up
//! config beside the current one for reference.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::HubConfig;
use crate::storage::{self, ArchiveStore};

/// What's inside a backup, by entry name.
const MANIFEST: &str = "manifest.json";
const DATABASE: &str = "blentinel.db";
const IDENTITY_KEY: &str = "hub_identity.key";
const TLS_CERT: &str = "hub_tls_cert.pem";
const TLS_KEY: &str = "hub_tls_key.pem";
const CONFIG: &str = "blentinel_hub.toml";

/// The format of this file; restore refuses others.
const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub hub_version: String,
    pub created_at: DateTime<Utc>,
    /// The database's schema: a hub older than this can't open it.
    pub schema_version: i64,
}

/// Write a backup of the hub `config` describes, its database read through
/// `store`, to `out` (a new file). Works while the hub runs. Returns what it
/// wrote.
pub async fn backup(
    store: &dyn ArchiveStore,
    config: &HubConfig,
    config_path: &Path,
    out: &Path,
    now: DateTime<Utc>,
) -> Result<Manifest> {
    if out.exists() {
        bail!("{} already exists", out.display());
    }
    let manifest = Manifest {
        format: FORMAT,
        hub_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: now,
        schema_version: store.schema_version().await?,
    };

    // The database copy goes to a scratch folder next to the backup.
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("can't create {}", parent.display()))?;
    let scratch = tempfile::Builder::new()
        .prefix(".hub-backup-")
        .tempdir_in(parent)?;
    let db_copy = scratch.path().join(DATABASE);
    store
        .backup_to(&db_copy)
        .await
        .context("can't copy the database")?;

    let server = &config.server;
    let others = vec![
        (IDENTITY_KEY, server.resolved_identity_key_path()),
        (TLS_CERT, server.resolve_path(&server.tls.cert_path)),
        (TLS_KEY, server.resolve_path(&server.tls.key_path)),
        (CONFIG, config_path.to_path_buf()),
    ];
    let (out, written) = (out.to_path_buf(), manifest.clone());
    // Compressing a large database takes a while: off the async threads.
    tokio::task::spawn_blocking(move || {
        let result = write_archive(&out, &written, &db_copy, &others, now);
        if result.is_err() {
            let _ = std::fs::remove_file(&out); // no half-written backup
        }
        drop(scratch);
        result
    })
    .await
    .context("the backup task failed")??;
    Ok(manifest)
}

/// The `.tar.gz` itself: the manifest, the database copy (streamed, not
/// read into memory), and whichever of `others` exist.
fn write_archive(
    out: &Path,
    manifest: &Manifest,
    db_copy: &Path,
    others: &[(&str, PathBuf)],
    now: DateTime<Utc>,
) -> Result<()> {
    let file = common::fs::create_secret_file(out)
        .with_context(|| format!("can't create {}", out.display()))?;
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::default(),
    ));
    let mut add = |name: &str, size: u64, data: &mut dyn Read| -> Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(0o600);
        header.set_mtime(now.timestamp().max(0) as u64);
        header.set_cksum();
        archive.append_data(&mut header, name, data)?;
        Ok(())
    };
    let manifest = serde_json::to_vec_pretty(manifest)?;
    add(MANIFEST, manifest.len() as u64, &mut manifest.as_slice())?;
    let mut db = std::fs::File::open(db_copy)?;
    add(DATABASE, db.metadata()?.len(), &mut db)?;
    for (name, path) in others {
        if let Ok(bytes) = std::fs::read(path) {
            add(name, bytes.len() as u64, &mut bytes.as_slice())?;
        }
    }
    archive.into_inner()?.finish()?.sync_all()?;
    Ok(())
}

/// A backup's files other than the manifest: (entry name, contents).
type Entries = Vec<(&'static str, Vec<u8>)>;

/// The entries of a backup, by name (only the names a backup has).
fn read_backup(path: &Path) -> Result<(Manifest, Entries)> {
    let file =
        std::fs::File::open(path).with_context(|| format!("can't open {}", path.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut entries = Vec::new();
    let mut manifest = None;
    let known = [MANIFEST, DATABASE, IDENTITY_KEY, TLS_CERT, TLS_KEY, CONFIG];
    for entry in archive.entries().context("not a hub backup (.tar.gz)")? {
        let mut entry = entry.context("the backup is damaged")?;
        let name = entry.path()?.to_string_lossy().into_owned();
        // Fixed names only: nothing in the archive chooses where files go.
        let Some(name) = known.iter().find(|k| **k == name) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if *name == MANIFEST {
            manifest = Some(
                serde_json::from_slice::<Manifest>(&bytes).context("its manifest is damaged")?,
            );
        } else {
            entries.push((*name, bytes));
        }
    }
    let manifest = manifest.context("not a hub backup (no manifest.json)")?;
    Ok((manifest, entries))
}

/// Restore `backup` into the hub `config` describes. The hub must be
/// stopped (`hub_running` false). Returns the folder the previous files
/// were moved to, if any.
pub fn restore(
    backup: &Path,
    config: &HubConfig,
    config_path: &Path,
    hub_running: bool,
    now: DateTime<Utc>,
) -> Result<Option<PathBuf>> {
    if hub_running {
        bail!(
            "the hub is running: stop it first ({})",
            if cfg!(windows) {
                "Stop-Service BlentinelHub"
            } else {
                "sudo systemctl stop blentinel-hub"
            }
        );
    }
    let (manifest, entries) = read_backup(backup)?;
    if manifest.format != FORMAT {
        bail!(
            "this backup's format ({}) isn't one this hub reads",
            manifest.format
        );
    }
    let latest = storage::latest_schema_version();
    if manifest.schema_version > latest {
        bail!(
            "this backup was made by a newer hub ({}, schema {}); this hub knows schema {latest}",
            manifest.hub_version,
            manifest.schema_version
        );
    }
    if !entries.iter().any(|(name, _)| *name == DATABASE) {
        bail!("this backup has no database");
    }

    let server = &config.server;
    let db = server.resolved_db_path();
    let destinations: Vec<(&str, PathBuf)> = vec![
        (DATABASE, db.clone()),
        (IDENTITY_KEY, server.resolved_identity_key_path()),
        (TLS_CERT, server.resolve_path(&server.tls.cert_path)),
        (TLS_KEY, server.resolve_path(&server.tls.key_path)),
    ];

    // Move aside what the restore replaces (and the database's WAL files).
    let mut replaced = Vec::new();
    for (name, path) in &destinations {
        if entries.iter().any(|(n, _)| n == name) {
            replaced.push(path.clone());
        }
    }
    replaced.extend(wal_files(&db));
    replaced.push(server.resolved_identity_key_path().with_extension("pub"));
    let aside = server
        .state_dir
        .join(format!("before-restore-{}", now.format("%Y%m%d-%H%M%S")));
    let moved_any = move_aside(&replaced, &aside)?;

    for (name, bytes) in &entries {
        if *name == CONFIG {
            let reference = PathBuf::from(format!("{}.from-backup", config_path.display()));
            common::fs::write_secret_file(&reference, bytes)?;
            continue;
        }
        let Some((_, path)) = destinations.iter().find(|(n, _)| n == name) else {
            continue;
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        common::fs::write_secret_file(path, bytes)
            .with_context(|| format!("can't write {}", path.display()))?;
        if *name == IDENTITY_KEY
            && let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice())
        {
            // The .pub beside it (moved aside above) matches the restored key.
            let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(key));
            crate::identity::write_pub_key_file(&path.with_extension("pub"), &public);
        }
    }
    Ok(moved_any.then_some(aside))
}

/// Put back only the database from `backup`, after a failed schema upgrade
/// (D-60): the database there (and its WAL files) moves to a
/// `failed-upgrade-…` folder in the data folder, which is returned. Its
/// connections must be closed.
pub fn restore_database(backup: &Path, config: &HubConfig, now: DateTime<Utc>) -> Result<PathBuf> {
    let (_, entries) = read_backup(backup)?;
    let Some((_, bytes)) = entries.iter().find(|(name, _)| *name == DATABASE) else {
        bail!("{} has no database", backup.display());
    };
    let db = config.server.resolved_db_path();
    let aside = config
        .server
        .state_dir
        .join(format!("failed-upgrade-{}", now.format("%Y%m%d-%H%M%S")));
    let mut failed = vec![db.clone()];
    failed.extend(wal_files(&db));
    move_aside(&failed, &aside)?;
    common::fs::write_secret_file(&db, bytes)
        .with_context(|| format!("can't write {}", db.display()))?;
    Ok(aside)
}

/// SQLite's write-ahead log and shared-memory files beside `db`.
fn wal_files(db: &Path) -> [PathBuf; 2] {
    ["-wal", "-shm"].map(|suffix| PathBuf::from(format!("{}{suffix}", db.display())))
}

/// Move whichever of `paths` exist into `folder` (created if needed).
/// `true` if anything moved.
fn move_aside(paths: &[PathBuf], folder: &Path) -> Result<bool> {
    let mut moved_any = false;
    for path in paths.iter().filter(|p| p.exists()) {
        std::fs::create_dir_all(folder)?;
        let name = path.file_name().context("a file name")?;
        std::fs::rename(path, folder.join(name))
            .with_context(|| format!("can't move {} aside", path.display()))?;
        moved_any = true;
    }
    Ok(moved_any)
}

/// A backup's manifest, for `hub restore` to say what it restores.
pub fn manifest_of(backup: &Path) -> Result<Manifest> {
    Ok(read_backup(backup)?.0)
}
