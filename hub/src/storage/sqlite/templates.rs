//! SQLite queries for saved check templates, behind
//! [`super::SqliteStorage`]'s `TemplateStore` (M5-12, D-85).

use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::storage::TemplateRecord;

const COLUMNS: &str = "id, name, description, checks, created_at, created_by";

fn to_record(row: &SqliteRow) -> anyhow::Result<TemplateRecord> {
    let checks: String = row.get("checks");
    Ok(TemplateRecord {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        checks: serde_json::from_str(&checks)
            .map_err(|e| anyhow::anyhow!("stored template checks: {e}"))?,
        created_at: row.get("created_at"),
        created_by: row.get("created_by"),
    })
}

pub(super) async fn list(pool: &SqlitePool) -> anyhow::Result<Vec<TemplateRecord>> {
    let sql = format!("SELECT {COLUMNS} FROM check_templates ORDER BY name COLLATE NOCASE");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter().map(to_record).collect()
}

pub(super) async fn get(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<TemplateRecord>> {
    let sql = format!("SELECT {COLUMNS} FROM check_templates WHERE id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(to_record).transpose()
}

/// `false`: a template already has that name (whatever its case).
pub(super) async fn create(pool: &SqlitePool, t: &TemplateRecord) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "INSERT INTO check_templates (id, name, description, checks, created_at, created_by)
         VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(name) DO NOTHING",
    )
    .bind(&t.id)
    .bind(&t.name)
    .bind(&t.description)
    .bind(serde_json::to_string(&t.checks)?)
    .bind(t.created_at)
    .bind(&t.created_by)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn delete(pool: &SqlitePool, id: &str) -> anyhow::Result<bool> {
    let done = sqlx::query("DELETE FROM check_templates WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
