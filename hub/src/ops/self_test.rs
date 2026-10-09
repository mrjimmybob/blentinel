//! `hub self-test` (M4-04, proposal 0012 §8, D-75): run by `hub upgrade` on
//! the **new** hub before anything is switched. It must:
//!
//! - start and print its version (the first line, which `hub upgrade`
//!   compares with the release's);
//! - read the config and the hub's key;
//! - upgrade a **copy** of the database to its schema (the real one isn't
//!   touched), and find it consistent.
//!
//! So a hub that can't start, or whose migrations fail on this database,
//! is never switched to.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::storage;

/// What the first line of a passing self-test says.
pub fn banner(version: &str) -> String {
    format!("Blentinel hub {version}")
}

/// Run the self-test against the hub configured at `config_path`.
pub async fn run(config_path: &Path) -> Result<()> {
    println!("{}", banner(env!("CARGO_PKG_VERSION")));
    let config = crate::config::load_from(config_path)
        .map_err(|e| anyhow::anyhow!("{}: {e}", config_path.display()))?;
    let key = config.server.resolved_identity_key_path();
    common::fs::load_key(&key).with_context(|| format!("can't read the key {}", key.display()))?;

    let db = config.server.resolved_db_path();
    if !db.exists() {
        println!("No database yet: nothing to upgrade.");
        return Ok(());
    }
    let work = tempfile::Builder::new()
        .prefix(".self-test-")
        .tempdir_in(&config.server.state_dir)
        .context("can't make a folder for the copy of the database")?;
    let copy = work.path().join("copy.db");
    let store = storage::open_read_only(&db)
        .await
        .with_context(|| format!("can't open {}", db.display()))?;
    let copied = store.backup_to(&copy).await;
    let from = store.schema_version().await.unwrap_or(0);
    store.close().await;
    copied.context("can't copy the database")?;

    let upgraded = storage::open(&copy)
        .await
        .context("upgrading a copy of the database failed")?;
    let schema = upgraded.schema_version().await;
    let integrity = upgraded.check_integrity().await;
    upgraded.close().await;
    let schema = schema?;
    let latest = storage::latest_schema_version();
    if schema != latest {
        bail!("the copy of the database is at schema {schema}, not {latest}");
    }
    if let Err(problem) = integrity? {
        bail!("the upgraded copy isn't consistent: {problem}");
    }
    println!("Database: a copy upgrades from schema {from} to {latest} and checks out.");
    Ok(())
}
