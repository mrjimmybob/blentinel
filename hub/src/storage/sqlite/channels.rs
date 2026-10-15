//! Alert settings, channels and routing rules (M3-03, D-66). Secrets arrive
//! and leave sealed; this layer never sees them in the clear.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{ChannelKind, DigestSettings, MailSecurity, RouteRule};
use crate::storage::{StoredAlertSettings, StoredChannel};

pub(super) async fn settings(pool: &SqlitePool) -> anyhow::Result<Option<StoredAlertSettings>> {
    let row = sqlx::query(
        "SELECT hub_url, smtp_server, smtp_port, smtp_security, smtp_username, smtp_from,
                smtp_password_sealed
         FROM alert_settings WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let security: String = row.get("smtp_security");
        Ok(StoredAlertSettings {
            hub_url: row.get("hub_url"),
            smtp_server: row.get("smtp_server"),
            smtp_port: u16::try_from(row.get::<i64, _>("smtp_port"))?,
            smtp_security: MailSecurity::parse(&security)
                .ok_or_else(|| anyhow::anyhow!("unknown mail security {security}"))?,
            smtp_username: row.get("smtp_username"),
            smtp_from: row.get("smtp_from"),
            smtp_password_sealed: row.get("smtp_password_sealed"),
        })
    })
    .transpose()
}

pub(super) async fn set_settings(
    pool: &SqlitePool,
    s: &StoredAlertSettings,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO alert_settings (id, hub_url, smtp_server, smtp_port, smtp_security,
             smtp_username, smtp_from, smtp_password_sealed, updated_by, updated_at)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET hub_url = excluded.hub_url,
             smtp_server = excluded.smtp_server, smtp_port = excluded.smtp_port,
             smtp_security = excluded.smtp_security, smtp_username = excluded.smtp_username,
             smtp_from = excluded.smtp_from, smtp_password_sealed = excluded.smtp_password_sealed,
             updated_by = excluded.updated_by, updated_at = excluded.updated_at",
    )
    .bind(&s.hub_url)
    .bind(&s.smtp_server)
    .bind(i64::from(s.smtp_port))
    .bind(s.smtp_security.as_str())
    .bind(&s.smtp_username)
    .bind(&s.smtp_from)
    .bind(&s.smtp_password_sealed)
    .bind(updated_by)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

const COLUMNS: &str =
    "id, name, kind, enabled, target, secret_sealed, last_delivery_at, last_error";

fn to_channel(row: &SqliteRow) -> anyhow::Result<StoredChannel> {
    let kind: String = row.get("kind");
    Ok(StoredChannel {
        id: row.get("id"),
        name: row.get("name"),
        kind: ChannelKind::parse(&kind)
            .ok_or_else(|| anyhow::anyhow!("unknown channel kind {kind}"))?,
        enabled: row.get("enabled"),
        target: row.get("target"),
        secret_sealed: row.get("secret_sealed"),
        last_delivery_at: row.get("last_delivery_at"),
        last_error: row.get("last_error"),
    })
}

pub(super) async fn channels(pool: &SqlitePool) -> anyhow::Result<Vec<StoredChannel>> {
    let sql = format!("SELECT {COLUMNS} FROM alert_channels ORDER BY name COLLATE NOCASE, id");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter().map(to_channel).collect()
}

pub(super) async fn channel(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<StoredChannel>> {
    let sql = format!("SELECT {COLUMNS} FROM alert_channels WHERE id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.as_ref().map(to_channel).transpose()
}

pub(super) async fn create(
    pool: &SqlitePool,
    c: &StoredChannel,
    now: DateTime<Utc>,
) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO alert_channels (name, kind, enabled, target, secret_sealed, created_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&c.name)
    .bind(c.kind.as_str())
    .bind(c.enabled)
    .bind(&c.target)
    .bind(&c.secret_sealed)
    .bind(now)
    .fetch_one(pool)
    .await?)
}

pub(super) async fn update(pool: &SqlitePool, c: &StoredChannel) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE alert_channels SET name = ?, kind = ?, enabled = ?, target = ?, secret_sealed = ?
         WHERE id = ?",
    )
    .bind(&c.name)
    .bind(c.kind.as_str())
    .bind(c.enabled)
    .bind(&c.target)
    .bind(&c.secret_sealed)
    .bind(c.id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn delete(pool: &SqlitePool, id: i64) -> anyhow::Result<bool> {
    let done = sqlx::query("DELETE FROM alert_channels WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn record_delivery(
    pool: &SqlitePool,
    id: i64,
    at: DateTime<Utc>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE alert_channels SET last_delivery_at = ?, last_error = ? WHERE id = ?")
        .bind(at)
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn rules(pool: &SqlitePool) -> anyhow::Result<Vec<RouteRule>> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT rule FROM alert_routes ORDER BY position")
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|json| {
            serde_json::from_str(json).map_err(|e| anyhow::anyhow!("stored rule {json:?}: {e}"))
        })
        .collect()
}

pub(super) async fn set_rules(pool: &SqlitePool, rules: &[RouteRule]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM alert_routes")
        .execute(&mut *tx)
        .await?;
    for (position, rule) in rules.iter().enumerate() {
        sqlx::query("INSERT INTO alert_routes (position, rule) VALUES (?, ?)")
            .bind(position as i64)
            .bind(serde_json::to_string(rule)?)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn digest(
    pool: &SqlitePool,
) -> anyhow::Result<Option<(DigestSettings, Option<String>)>> {
    let row =
        sqlx::query("SELECT hour, timezone, channels, last_sent FROM digest_settings WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    row.map(|row| {
        let channels: String = row.get("channels");
        let settings = DigestSettings {
            hour: u8::try_from(row.get::<i64, _>("hour"))?,
            timezone: row.get("timezone"),
            channels: serde_json::from_str(&channels)
                .map_err(|e| anyhow::anyhow!("stored digest channels {channels:?}: {e}"))?,
        };
        Ok((settings, row.get("last_sent")))
    })
    .transpose()
}

pub(super) async fn set_digest(
    pool: &SqlitePool,
    s: &DigestSettings,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO digest_settings (id, hour, timezone, channels, updated_by, updated_at)
         VALUES (1, ?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET hour = excluded.hour, timezone = excluded.timezone,
             channels = excluded.channels, updated_by = excluded.updated_by,
             updated_at = excluded.updated_at",
    )
    .bind(i64::from(s.hour))
    .bind(s.timezone.trim())
    .bind(serde_json::to_string(&s.channels)?)
    .bind(updated_by)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn digest_sent(pool: &SqlitePool, date: &str) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE digest_settings SET last_sent = ?1
         WHERE id = 1 AND (last_sent IS NULL OR last_sent <> ?1)",
    )
    .bind(date)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}
