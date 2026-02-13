use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

pub struct BlackBox {
    pool: SqlitePool,
}

impl BlackBox {
    pub async fn new(probe_public_key: &str, hub_public_key: &str) -> anyhow::Result<Self> {
        let db_path = crate::config::get_base_dir().join("blentinel_blackbox.db");

        // If the DB already exists, validate stored identity fingerprints.
        // A mismatch means queued payloads are cryptographically invalid.
        if db_path.exists() {
            if Self::needs_reset(&db_path, probe_public_key, hub_public_key).await {
                eprintln!(
                    "\u{26A0} Identity or hub key changed. \
                     Existing blentinel_blackbox.db is invalid (hub_public_key mismatch). Reinitializing."
                );
                std::fs::remove_file(&db_path)?;
            }
        }

        let db_url = format!("sqlite://{}", db_path.to_string_lossy());
        let options = SqliteConnectOptions::from_str(&db_url)?
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await?;

        // Create tables
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pending_reports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload BLOB NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )"
        ).execute(&pool).await?;

        // Store current fingerprints (INSERT OR REPLACE handles both
        // fresh DBs and the rare case where only one key changed)
        sqlx::query("INSERT OR REPLACE INTO meta (key, value) VALUES ('probe_public_key', ?)")
            .bind(probe_public_key)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT OR REPLACE INTO meta (key, value) VALUES ('hub_public_key', ?)")
            .bind(hub_public_key)
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    /// Open an existing DB, read its stored fingerprints, and compare with
    /// the current keys.  Returns `true` if the DB should be deleted.
    ///
    /// Any error (corrupt DB, missing meta table, bad schema) is treated
    /// as a mismatch — the safest response is to start fresh.
    async fn needs_reset(db_path: &std::path::Path, probe_pk: &str, hub_pk: &str) -> bool {
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());
        let options = match SqliteConnectOptions::from_str(&db_url) {
            Ok(o) => o,
            Err(_) => return true,
        };

        let pool = match SqlitePool::connect_with(options).await {
            Ok(p) => p,
            Err(_) => return true,
        };

        let result = sqlx::query_as::<_, (String, String)>(
            "SELECT key, value FROM meta WHERE key IN ('probe_public_key', 'hub_public_key')"
        )
        .fetch_all(&pool)
        .await;

        // Close before we potentially delete the file (critical on Windows)
        pool.close().await;

        let rows = match result {
            Ok(r) => r,
            Err(_) => return true, // meta table missing or corrupt
        };

        if rows.is_empty() {
            return true; // legacy DB without fingerprints
        }

        for (key, value) in &rows {
            match key.as_str() {
                "probe_public_key" if value != probe_pk => return true,
                "hub_public_key" if value != hub_pk => return true,
                _ => {}
            }
        }

        false
    }

    pub async fn queue_report(&self, payload: &[u8]) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO pending_reports (payload) VALUES (?)")
            .bind(payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_queued_reports(&self) -> anyhow::Result<Vec<(i64, Vec<u8>)>> {
        let rows = sqlx::query_as::<_, (i64, Vec<u8>)>("SELECT id, payload FROM pending_reports ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn delete_report(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM pending_reports WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
