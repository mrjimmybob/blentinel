//! SQLite queries for probe records and enrollment codes, behind
//! [`super::SqliteStorage`]'s `ProbeRecordStore` implementation (D-47).

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{CompanyScope, EnrollmentCodeInfo, ProbeInfo};
use crate::storage::{NewEnrollmentToken, PROBE_REVOKED, ProbeRecord, RedeemedToken};

fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn to_record(row: &SqliteRow) -> ProbeRecord {
    ProbeRecord {
        id: row.get("id"),
        name: row.get("name"),
        company_id: row.get("company_id"),
        site_id: row.get("site_id"),
        state: row.get("state"),
        hostname: row.get("hostname"),
        os: row.get("os"),
        arch: row.get("arch"),
        probe_version: row.get("probe_version"),
        enrolled_at: row.get("enrolled_at"),
        token_id: row.get("token_id"),
    }
}

pub(super) async fn record(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<ProbeRecord>> {
    let row = sqlx::query(
        "SELECT id, name, company_id, site_id, state, hostname, os, arch, probe_version,
                enrolled_at, token_id
         FROM probes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(to_record))
}

pub(super) async fn save(pool: &SqlitePool, r: &ProbeRecord) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO probes (id, name, company_id, site_id, state, hostname, os, arch,
                             probe_version, enrolled_at, revoked_at, token_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name, company_id = excluded.company_id, site_id = excluded.site_id,
             state = excluded.state, hostname = excluded.hostname, os = excluded.os,
             arch = excluded.arch, probe_version = excluded.probe_version,
             enrolled_at = excluded.enrolled_at, revoked_at = NULL, token_id = excluded.token_id",
    )
    .bind(&r.id)
    .bind(&r.name)
    .bind(&r.company_id)
    .bind(&r.site_id)
    .bind(&r.state)
    .bind(&r.hostname)
    .bind(&r.os)
    .bind(&r.arch)
    .bind(&r.probe_version)
    .bind(r.enrolled_at)
    .bind(&r.token_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn list(
    pool: &SqlitePool,
    scope: &CompanyScope,
) -> anyhow::Result<Vec<ProbeInfo>> {
    let (in_scope, binds) = super::scope::condition("p.company_id", scope);
    let sql = format!(
        "SELECT p.id, p.name, p.company_id, c.name AS company_name, p.site_id,
                s.name AS site_name, p.state, p.hostname, p.os, p.arch, p.probe_version,
                p.enrolled_at, h.last_seen_at
         FROM probes p
         JOIN companies c ON c.id = p.company_id
         JOIN sites s ON s.company_id = p.company_id AND s.id = p.site_id
         LEFT JOIN probe_heartbeats h ON h.probe_id = p.id
         WHERE {in_scope}
         ORDER BY c.name COLLATE NOCASE, p.name COLLATE NOCASE"
    );
    let mut query = sqlx::query(&sql);
    for id in &binds {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let enrolled_at: DateTime<Utc> = row.get("enrolled_at");
            let last_seen_at: Option<DateTime<Utc>> = row.get("last_seen_at");
            ProbeInfo {
                id: row.get("id"),
                name: row.get("name"),
                company_id: row.get("company_id"),
                company_name: row.get("company_name"),
                site_id: row.get("site_id"),
                site_name: row.get("site_name"),
                state: row.get("state"),
                hostname: row.get("hostname"),
                os: row.get("os"),
                arch: row.get("arch"),
                probe_version: row.get("probe_version"),
                enrolled_at: fmt_time(enrolled_at),
                last_seen_at: last_seen_at.map(fmt_time),
            }
        })
        .collect())
}

pub(super) async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    site_id: &str,
) -> anyhow::Result<bool> {
    // The site must belong to the probe's company.
    let done = sqlx::query(
        "UPDATE probes SET name = ?1, site_id = ?2
         WHERE id = ?3
           AND EXISTS (SELECT 1 FROM sites s WHERE s.company_id = probes.company_id AND s.id = ?2)",
    )
    .bind(name)
    .bind(site_id)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn revoke(
    pool: &SqlitePool,
    id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE probes SET state = ?, revoked_at = ? WHERE id = ?")
        .bind(PROBE_REVOKED)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn delete(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM probes WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Enrollment codes
// ---------------------------------------------------------------------------

pub(super) async fn create_token(
    pool: &SqlitePool,
    t: &NewEnrollmentToken,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO enrollment_tokens (id, secret_hash, company_id, site_id, probe_name,
                                        created_by, created_at, expires_at, max_uses)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&t.id)
    .bind(&t.secret_hash)
    .bind(&t.company_id)
    .bind(&t.site_id)
    .bind(&t.probe_name)
    .bind(&t.created_by)
    .bind(now)
    .bind(t.expires_at)
    .bind(i64::from(t.max_uses))
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn tokens(
    pool: &SqlitePool,
    scope: &CompanyScope,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<EnrollmentCodeInfo>> {
    let (in_scope, binds) = super::scope::condition("company_id", scope);
    let sql = format!(
        "SELECT id, company_id, site_id, probe_name, created_by, created_at, expires_at,
                max_uses, uses
         FROM enrollment_tokens
         WHERE revoked_at IS NULL AND uses < max_uses AND expires_at > ? AND {in_scope}
         ORDER BY created_at DESC"
    );
    let mut query = sqlx::query(&sql).bind(now);
    for id in &binds {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let created_at: DateTime<Utc> = row.get("created_at");
            let expires_at: DateTime<Utc> = row.get("expires_at");
            EnrollmentCodeInfo {
                id: row.get("id"),
                company_id: row.get("company_id"),
                site_id: row.get("site_id"),
                probe_name: row.get("probe_name"),
                created_by: row.get("created_by"),
                created_at: fmt_time(created_at),
                expires_at: fmt_time(expires_at),
                max_uses: row.get("max_uses"),
                uses: row.get("uses"),
            }
        })
        .collect())
}

pub(super) async fn token_company(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT company_id FROM enrollment_tokens WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub(super) async fn revoke_token(
    pool: &SqlitePool,
    id: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE enrollment_tokens SET revoked_at = COALESCE(revoked_at, ?) WHERE id = ?",
    )
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn redeem(
    pool: &SqlitePool,
    id: &str,
    secret_hash: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<RedeemedToken>> {
    // One statement: two probes racing for the last use can't both win.
    let row = sqlx::query(
        "UPDATE enrollment_tokens SET uses = uses + 1
         WHERE id = ? AND secret_hash = ? AND revoked_at IS NULL
           AND expires_at > ? AND uses < max_uses
         RETURNING company_id, site_id, probe_name",
    )
    .bind(id)
    .bind(secret_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| RedeemedToken {
        company_id: row.get("company_id"),
        site_id: row.get("site_id"),
        probe_name: row.get("probe_name"),
    }))
}
