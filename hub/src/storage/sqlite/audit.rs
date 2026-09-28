//! SQLite queries for the audit log, behind [`super::SqliteStorage`]'s
//! `AuditStore` implementation.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::model::AuditEntry;
use crate::storage::{AuditFilter, NewAuditEntry};

pub(super) async fn append(pool: &SqlitePool, e: &NewAuditEntry) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (at, user_id, username, actor_ip, action, target_type,
                                target_id, company_id, details)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(e.at)
    .bind(&e.user_id)
    .bind(&e.username)
    .bind(&e.actor_ip)
    .bind(&e.action)
    .bind(&e.target_type)
    .bind(&e.target_id)
    .bind(&e.company_id)
    .bind(&e.details)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn entries(
    pool: &SqlitePool,
    filter: &AuditFilter,
) -> anyhow::Result<Vec<AuditEntry>> {
    // Every condition is optional; `?n IS NULL OR …` keeps one static query.
    let rows = sqlx::query(
        "SELECT id, at, username, actor_ip, action, target_type, target_id, company_id, details
         FROM audit_log
         WHERE (?1 IS NULL OR username = ?1 COLLATE NOCASE)
           AND (?2 IS NULL OR action = ?2 OR action LIKE ?2 || '.%')
           AND (?3 IS NULL OR target_id = ?3 OR company_id = ?3)
           AND (?4 IS NULL OR at >= ?4)
           AND (?5 IS NULL OR at < ?5)
           AND (?6 IS NULL OR id < ?6)
         ORDER BY id DESC
         LIMIT ?7",
    )
    .bind(&filter.username)
    .bind(&filter.action_prefix)
    .bind(&filter.target)
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.before_id)
    .bind(i64::from(filter.limit))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let at: DateTime<Utc> = row.get("at");
            AuditEntry {
                id: row.get("id"),
                at: at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                username: row.get("username"),
                actor_ip: row.get("actor_ip"),
                action: row.get("action"),
                target_type: row.get("target_type"),
                target_id: row.get("target_id"),
                company_id: row.get("company_id"),
                details: row.get("details"),
            }
        })
        .collect())
}
