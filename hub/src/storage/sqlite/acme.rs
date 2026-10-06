//! The ACME settings and the last attempt (M2-14, D-62).

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::model::{AcmeAuthority, AcmeChallenge, AcmeSettings};
use crate::storage::AcmeStatus;

pub(super) async fn settings(pool: &SqlitePool) -> anyhow::Result<Option<AcmeSettings>> {
    let row = sqlx::query(
        "SELECT enabled, domains, email, authority, directory_url, ca_pem, challenge, terms_agreed
         FROM acme_settings WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let authority: String = row.get("authority");
        let challenge: String = row.get("challenge");
        Ok(AcmeSettings {
            enabled: row.get("enabled"),
            domains: serde_json::from_str(row.get("domains"))?,
            email: row.get("email"),
            authority: AcmeAuthority::parse(&authority)
                .ok_or_else(|| anyhow::anyhow!("unknown ACME authority {authority}"))?,
            directory_url: row.get("directory_url"),
            ca_pem: row.get("ca_pem"),
            challenge: AcmeChallenge::parse(&challenge)
                .ok_or_else(|| anyhow::anyhow!("unknown ACME challenge {challenge}"))?,
            terms_agreed: row.get("terms_agreed"),
        })
    })
    .transpose()
}

pub(super) async fn set_settings(
    pool: &SqlitePool,
    settings: &AcmeSettings,
    updated_by: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO acme_settings (id, enabled, domains, email, authority, directory_url, ca_pem,
             challenge, terms_agreed, updated_by, updated_at)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled,
             domains = excluded.domains, email = excluded.email,
             authority = excluded.authority, directory_url = excluded.directory_url,
             ca_pem = excluded.ca_pem, challenge = excluded.challenge,
             terms_agreed = excluded.terms_agreed,
             updated_by = excluded.updated_by, updated_at = excluded.updated_at",
    )
    .bind(settings.enabled)
    .bind(serde_json::to_string(&settings.domains)?)
    .bind(&settings.email)
    .bind(settings.authority.as_str())
    .bind(&settings.directory_url)
    .bind(&settings.ca_pem)
    .bind(settings.challenge.as_str())
    .bind(settings.terms_agreed)
    .bind(updated_by)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn status(pool: &SqlitePool) -> anyhow::Result<Option<AcmeStatus>> {
    let row = sqlx::query(
        "SELECT last_attempt_at, last_error, failures, last_issued_at FROM acme_status WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        Ok(AcmeStatus {
            last_attempt_at: row.get("last_attempt_at"),
            last_error: row.get("last_error"),
            failures: u32::try_from(row.get::<i64, _>("failures"))?,
            last_issued_at: row.get("last_issued_at"),
        })
    })
    .transpose()
}

pub(super) async fn record_attempt(
    pool: &SqlitePool,
    at: DateTime<Utc>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let issued_at = error.is_none().then_some(at);
    sqlx::query(
        "INSERT INTO acme_status (id, last_attempt_at, last_error, failures, last_issued_at)
         VALUES (1, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET last_attempt_at = excluded.last_attempt_at,
             last_error = excluded.last_error,
             failures = CASE WHEN excluded.last_error IS NULL THEN 0 ELSE failures + 1 END,
             last_issued_at = COALESCE(excluded.last_issued_at, last_issued_at)",
    )
    .bind(at)
    .bind(error)
    .bind(i64::from(error.is_some()))
    .bind(issued_at)
    .execute(pool)
    .await?;
    Ok(())
}
