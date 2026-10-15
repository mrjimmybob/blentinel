//! Opening the database with an upgrade safety net (M2-12, proposal 0004
//! §4.5, D-60): when a new hub finds an older schema, it backs the hub up
//! first, then migrates; if the migration fails, it puts the database back
//! as it was and refuses to start, saying so.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::config::HubConfig;
use crate::model::BackupKind;
use crate::service::backups;
use crate::storage::{self, Storage};

/// Open (creating and migrating if needed) the database `config` names. An
/// existing database with an older schema is backed up before it is
/// migrated (a `-pre-upgrade` backup in the backups folder).
pub async fn open_database(
    config: &HubConfig,
    config_path: &Path,
    now: DateTime<Utc>,
) -> Result<Arc<dyn Storage>> {
    let db = config.server.resolved_db_path();
    let latest = storage::latest_schema_version();
    let Some(from) = older_schema(&db, latest).await? else {
        return storage::open(&db).await;
    };

    // Through a read-only connection: nothing changes before the backup exists.
    let store = storage::open_read_only(&db).await?;
    let made = backups::make(
        store.as_ref(),
        config,
        config_path,
        BackupKind::PreUpgrade,
        now,
    )
    .await;
    store.close().await;
    let backup =
        made.context("can't back up the database before upgrading it; nothing was changed")?;
    let backup = config.backups_dir().join(backup.name);
    tracing::info!(
        backup = %backup.display(),
        "Upgrading the database from schema {from} to {latest}; backed up first"
    );

    match storage::open(&db).await {
        Ok(store) => Ok(store),
        Err(error) => {
            let aside =
                crate::ops::backup::restore_database(&backup, config, now).with_context(|| {
                    format!(
                        "upgrading the database from schema {from} to {latest} failed ({error:#}), \
                     and putting it back failed too: stop the hub, then `hub restore {}`",
                        backup.display()
                    )
                })?;
            bail!(
                "upgrading the database from schema {from} to {latest} failed: {error:#}. \
                 It was put back as it was, from {}; the database that failed is in {}. \
                 Run the previous hub version, and report this",
                backup.display(),
                aside.display()
            )
        }
    }
}

/// The schema of the database at `db` if this hub would migrate it: it
/// exists and has an older schema. A new or empty file, or one that can't be
/// read here, is left to `storage::open` (which creates it, or says why not).
async fn older_schema(db: &Path, latest: i64) -> Result<Option<i64>> {
    if !db.exists() {
        return Ok(None);
    }
    let Ok(store) = storage::open_read_only(db).await else {
        return Ok(None);
    };
    let version = store.schema_version().await;
    store.close().await;
    Ok(version.ok().filter(|v| (1..latest).contains(v)))
}
