//! SQLite queries for users and sessions, behind [`super::SqliteStorage`]'s
//! `UserStore` implementation.

use std::collections::HashMap;

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{CompanyScope, Role, UserInfo};
use crate::storage::{NewUser, StoredUser, UserUpdate};

const USER_COLUMNS: &str = "id, username, display_name, email, password_hash, role, company_scope,
     is_active, must_change_password, created_at, last_login_at";

fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A `users` row plus its assigned companies.
fn to_user(row: &SqliteRow, companies: Vec<String>) -> anyhow::Result<StoredUser> {
    let role: String = row.get("role");
    let mode: String = row.get("company_scope");
    let created_at: DateTime<Utc> = row.get("created_at");
    let last_login_at: Option<DateTime<Utc>> = row.get("last_login_at");
    Ok(StoredUser {
        info: UserInfo {
            id: row.get("id"),
            username: row.get("username"),
            display_name: row.get("display_name"),
            email: row.get("email"),
            role: Role::parse(&role).with_context(|| format!("unknown role {role:?}"))?,
            scope: CompanyScope::from_parts(&mode, companies)
                .with_context(|| format!("unknown company scope {mode:?}"))?,
            is_active: row.get("is_active"),
            must_change_password: row.get("must_change_password"),
            created_at: fmt_time(created_at),
            last_login_at: last_login_at.map(fmt_time),
        },
        password_hash: row.get("password_hash"),
    })
}

async fn companies_of(pool: &SqlitePool, user_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT company_id FROM user_companies WHERE user_id = ? ORDER BY company_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

async fn user_where(
    pool: &SqlitePool,
    condition: &str,
    value: &str,
) -> anyhow::Result<Option<StoredUser>> {
    let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE {condition}");
    let Some(row) = sqlx::query(&sql).bind(value).fetch_optional(pool).await? else {
        return Ok(None);
    };
    let id: String = row.get("id");
    Ok(Some(to_user(&row, companies_of(pool, &id).await?)?))
}

async fn replace_companies(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    companies: &[String],
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM user_companies WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    for company in companies {
        sqlx::query("INSERT OR IGNORE INTO user_companies (user_id, company_id) VALUES (?, ?)")
            .bind(user_id)
            .bind(company)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub(super) async fn create_user(
    pool: &SqlitePool,
    user: &NewUser,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO users (id, username, display_name, email, password_hash, role,
                            company_scope, must_change_password, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.username)
    .bind(&user.display_name)
    .bind(&user.email)
    .bind(&user.password_hash)
    .bind(user.role.as_str())
    .bind(user.scope.mode())
    .bind(user.must_change_password)
    .bind(now)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("failed to create user {:?}", user.username))?;
    replace_companies(&mut tx, &user.id, &user.scope.companies()).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn user(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<StoredUser>> {
    user_where(pool, "id = ?", id).await
}

pub(super) async fn user_by_username(
    pool: &SqlitePool,
    username: &str,
) -> anyhow::Result<Option<StoredUser>> {
    // The column is COLLATE NOCASE, so `=` ignores case.
    user_where(pool, "username = ?", username).await
}

pub(super) async fn users(pool: &SqlitePool) -> anyhow::Result<Vec<UserInfo>> {
    let mut companies: HashMap<String, Vec<String>> = HashMap::new();
    let pairs: Vec<(String, String)> = sqlx::query_as(
        "SELECT user_id, company_id FROM user_companies ORDER BY user_id, company_id",
    )
    .fetch_all(pool)
    .await?;
    for (user_id, company_id) in pairs {
        companies.entry(user_id).or_default().push(company_id);
    }

    let sql = format!("SELECT {USER_COLUMNS} FROM users ORDER BY username");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter()
        .map(|row| {
            let id: String = row.get("id");
            let assigned = companies.remove(&id).unwrap_or_default();
            to_user(row, assigned).map(|u| u.info)
        })
        .collect()
}

pub(super) async fn user_count(pool: &SqlitePool) -> anyhow::Result<u64> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(n as u64)
}

pub(super) async fn active_owner_count(pool: &SqlitePool) -> anyhow::Result<u64> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'owner' AND is_active = 1")
            .fetch_one(pool)
            .await?;
    Ok(n as u64)
}

pub(super) async fn update_user(
    pool: &SqlitePool,
    id: &str,
    update: &UserUpdate,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE users SET display_name = ?, email = ?, role = ?, company_scope = ?, is_active = ?
         WHERE id = ?",
    )
    .bind(&update.display_name)
    .bind(&update.email)
    .bind(update.role.as_str())
    .bind(update.scope.mode())
    .bind(update.is_active)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    replace_companies(&mut tx, id, &update.scope.companies()).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn set_password(
    pool: &SqlitePool,
    id: &str,
    password_hash: &str,
    must_change_password: bool,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET password_hash = ?, must_change_password = ? WHERE id = ?")
        .bind(password_hash)
        .bind(must_change_password)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn record_login(
    pool: &SqlitePool,
    id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

pub(super) async fn create_session(
    pool: &SqlitePool,
    id_hash: &str,
    user_id: &str,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id_hash, user_id, created_at, last_seen_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id_hash)
    .bind(user_id)
    .bind(now)
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn resume_session(
    pool: &SqlitePool,
    id_hash: &str,
    now: DateTime<Utc>,
    idle_timeout: chrono::Duration,
) -> anyhow::Result<Option<StoredUser>> {
    let user_id: Option<String> = sqlx::query_scalar(
        "SELECT s.user_id FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.id_hash = ? AND s.expires_at > ? AND s.last_seen_at > ? AND u.is_active = 1",
    )
    .bind(id_hash)
    .bind(now)
    .bind(now - idle_timeout)
    .fetch_optional(pool)
    .await?;
    let Some(user_id) = user_id else {
        return Ok(None);
    };
    sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id_hash = ?")
        .bind(now)
        .bind(id_hash)
        .execute(pool)
        .await?;
    user(pool, &user_id).await
}

pub(super) async fn delete_session(pool: &SqlitePool, id_hash: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id_hash = ?")
        .bind(id_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn delete_user_sessions(
    pool: &SqlitePool,
    user_id: &str,
    keep: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND id_hash IS NOT ?")
        .bind(user_id)
        .bind(keep)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn delete_dead_sessions(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    idle_timeout: chrono::Duration,
) -> anyhow::Result<u64> {
    let done = sqlx::query("DELETE FROM sessions WHERE expires_at <= ? OR last_seen_at <= ?")
        .bind(now)
        .bind(now - idle_timeout)
        .execute(pool)
        .await?;
    Ok(done.rows_affected())
}
