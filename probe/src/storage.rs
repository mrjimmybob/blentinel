use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

pub struct BlackBox {
    pool: SqlitePool,
}

impl BlackBox {
    pub async fn new() -> anyhow::Result<Self> {
        let db_path = crate::config::get_base_dir().join("blentinel_blackbox.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());

        let options = SqliteConnectOptions::from_str(&db_url)?
            .create_if_missing(true);
        
        let pool = SqlitePool::connect_with(options).await?;
        
        // Create the table for "Pending Reports"
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pending_reports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload BLOB NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await?;

        Ok(Self { pool })
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