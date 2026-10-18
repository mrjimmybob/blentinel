//! SQLite queries for probe commands, behind [`super::SqliteStorage`]'s
//! `CommandStore` (M2-06, D-56).

use chrono::{DateTime, Utc};
use common::commands::CommandSpec;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::CommandState;
use crate::storage::CommandRecord;

const COLUMNS: &str = "c.id, c.probe_id, p.company_id, c.spec, c.created_by, c.created_at,
                       c.expires_at, c.state, c.delivered_at, c.completed_at, c.result";

fn to_record(row: &SqliteRow) -> anyhow::Result<CommandRecord> {
    let spec: &str = row.get("spec");
    let state: &str = row.get("state");
    Ok(CommandRecord {
        id: row.get("id"),
        probe_id: row.get("probe_id"),
        company_id: row.get("company_id"),
        spec: serde_json::from_str::<CommandSpec>(spec)
            .map_err(|e| anyhow::anyhow!("stored command spec {spec:?}: {e}"))?,
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        state: CommandState::parse(state)
            .ok_or_else(|| anyhow::anyhow!("stored command state {state:?}"))?,
        delivered_at: row.get("delivered_at"),
        completed_at: row.get("completed_at"),
        result: row.get("result"),
    })
}

pub(super) async fn create(pool: &SqlitePool, c: &CommandRecord) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO probe_commands
             (id, probe_id, kind, spec, created_by, created_at, expires_at, state,
              delivered_at, completed_at, result)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&c.id)
    .bind(&c.probe_id)
    .bind(c.spec.kind())
    .bind(serde_json::to_string(&c.spec)?)
    .bind(&c.created_by)
    .bind(c.created_at)
    .bind(c.expires_at)
    .bind(c.state.as_str())
    .bind(c.delivered_at)
    .bind(c.completed_at)
    .bind(&c.result)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn get(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<CommandRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM probe_commands c JOIN probes p ON p.id = c.probe_id WHERE c.id = ?"
    );
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(to_record).transpose()
}

pub(super) async fn of_probe(
    pool: &SqlitePool,
    probe_id: &str,
    limit: u32,
) -> anyhow::Result<Vec<CommandRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM probe_commands c JOIN probes p ON p.id = c.probe_id
         WHERE c.probe_id = ? ORDER BY c.created_at DESC, c.rowid DESC LIMIT ?"
    );
    let rows = sqlx::query(&sql)
        .bind(probe_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter().map(to_record).collect()
}

pub(super) async fn pending(
    pool: &SqlitePool,
    probe_id: &str,
    now: DateTime<Utc>,
    limit: u32,
) -> anyhow::Result<Vec<CommandRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM probe_commands c JOIN probes p ON p.id = c.probe_id
         WHERE c.probe_id = ? AND c.state IN ('queued', 'delivered') AND c.expires_at > ?
         ORDER BY c.created_at, c.rowid LIMIT ?"
    );
    let rows = sqlx::query(&sql)
        .bind(probe_id)
        .bind(now)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter().map(to_record).collect()
}

pub(super) async fn mark_delivered(
    pool: &SqlitePool,
    ids: &[String],
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut tx = super::write_transaction(pool).await?;
    for id in ids {
        sqlx::query(
            "UPDATE probe_commands SET state = 'delivered', delivered_at = ?
             WHERE id = ? AND state = 'queued'",
        )
        .bind(now)
        .bind(id)
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn complete(
    pool: &SqlitePool,
    probe_id: &str,
    id: &str,
    state: CommandState,
    result: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        matches!(state, CommandState::Succeeded | CommandState::Failed),
        "a command completes as succeeded or failed, not {}",
        state.as_str()
    );
    let done = sqlx::query(
        "UPDATE probe_commands SET state = ?, result = ?, completed_at = ?
         WHERE id = ? AND probe_id = ? AND state IN ('queued', 'delivered')",
    )
    .bind(state.as_str())
    .bind(result)
    .bind(now)
    .bind(id)
    .bind(probe_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn cancel(
    pool: &SqlitePool,
    id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE probe_commands SET state = 'cancelled', completed_at = ?
         WHERE id = ? AND state = 'queued'",
    )
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}
