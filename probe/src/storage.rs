use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

/// The black box's file, in the probe's folder.
pub const FILE_NAME: &str = "blentinel_blackbox.db";

pub struct BlackBox {
    pool: SqlitePool,
}

impl BlackBox {
    /// Open the black box in the probe's own directory (FP-2: all state lives there).
    pub async fn new(probe_public_key: &str, hub_public_key: &str) -> anyhow::Result<Self> {
        let db_path = crate::config::get_base_dir().join(FILE_NAME);
        Self::open(&db_path, probe_public_key, hub_public_key).await
    }

    /// How many reports wait in the black box at `db_path`, read-only
    /// (`probe status`; `None` if there's no black box).
    pub async fn pending_at(db_path: &std::path::Path) -> anyhow::Result<Option<i64>> {
        if !db_path.exists() {
            return Ok(None);
        }
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .read_only(true);
        let pool = SqlitePool::connect_with(options).await?;
        let count = sqlx::query_scalar("SELECT COUNT(*) FROM pending_reports")
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
        pool.close().await;
        Ok(Some(count))
    }

    /// Open (or create) the black box at an explicit path.
    pub async fn open(
        db_path: &std::path::Path,
        probe_public_key: &str,
        hub_public_key: &str,
    ) -> anyhow::Result<Self> {
        let db_path = db_path.to_path_buf();

        // If the DB already exists, validate stored identity fingerprints.
        // A mismatch means queued payloads are cryptographically invalid.
        if db_path.exists() && Self::needs_reset(&db_path, probe_public_key, hub_public_key).await {
            eprintln!(
                "\u{26A0} Identity or hub key changed. \
                     Existing blentinel_blackbox.db is invalid (hub_public_key mismatch). Reinitializing."
            );
            std::fs::remove_file(&db_path)?;
        }

        let db_url = format!("sqlite://{}", db_path.to_string_lossy());
        let options = SqliteConnectOptions::from_str(&db_url)?.create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await?;

        // Create tables
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pending_reports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload BLOB NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

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

    /// The hub switched to a new key (D-61): reports already queued were
    /// sealed for the old one, which the hub keeps for them, so they stay.
    pub async fn set_hub_key(&self, hub_public_key: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT OR REPLACE INTO meta (key, value) VALUES ('hub_public_key', ?)")
            .bind(hub_public_key)
            .execute(&self.pool)
            .await?;
        Ok(())
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
            "SELECT key, value FROM meta WHERE key IN ('probe_public_key', 'hub_public_key')",
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

    /// Up to `limit` queued payloads, oldest first (delivery order).
    pub async fn oldest(&self, limit: i64) -> anyhow::Result<Vec<(i64, Vec<u8>)>> {
        let rows = sqlx::query_as::<_, (i64, Vec<u8>)>(
            "SELECT id, payload FROM pending_reports ORDER BY id ASC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Number of undelivered reports.
    pub async fn count(&self) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM pending_reports")
            .fetch_one(&self.pool)
            .await?)
    }

    /// Enforce the buffer limits: drop reports older than `max_age_secs` (the hub
    /// would reject them anyway) and, if still over `max_rows`, the oldest extras.
    /// Returns how many reports were dropped.
    pub async fn prune(&self, max_age_secs: i64, max_rows: i64) -> anyhow::Result<u64> {
        let too_old =
            sqlx::query("DELETE FROM pending_reports WHERE created_at < datetime('now', ?)")
                .bind(format!("-{max_age_secs} seconds"))
                .execute(&self.pool)
                .await?
                .rows_affected();
        let over_cap = sqlx::query(
            "DELETE FROM pending_reports
             WHERE id NOT IN (SELECT id FROM pending_reports ORDER BY id DESC LIMIT ?)",
        )
        .bind(max_rows)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(too_old + over_cap)
    }

    /// Queue a payload with an explicit (UTC, `YYYY-MM-DD HH:MM:SS`) creation time.
    #[cfg(test)]
    pub async fn queue_report_created_at(
        &self,
        payload: &[u8],
        created_at: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO pending_reports (payload, created_at) VALUES (?, ?)")
            .bind(payload)
            .bind(created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Forget every queued report (the probe was revoked: they won't be accepted).
    pub async fn clear(&self) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM pending_reports")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_report(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM pending_reports WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
