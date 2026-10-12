//! SQLite queries for probe records and enrollment codes, behind
//! [`super::SqliteStorage`]'s `ProbeRecordStore` implementation (D-47).

use chrono::{DateTime, Utc};
use common::checks::ConfigRejection;
use common::models::{ProbeHealth, UpdateFailure};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{CompanyScope, EnrollmentCodeInfo, ProbeInfo};
use crate::storage::{
    NewEnrollmentToken, PROBE_REVOKED, ProbeRecord, RedeemedToken, ReportedState,
};

/// The failed update in a row's `update_failed_*` columns.
fn update_failed(row: &SqliteRow) -> Option<UpdateFailure> {
    let version: Option<String> = row.get("update_failed_version");
    let error: Option<String> = row.get("update_failed_error");
    version.map(|version| UpdateFailure {
        version,
        error: error.unwrap_or_default(),
    })
}

/// The health in a row's `health` column (JSON); an unreadable one is none.
fn health(row: &SqliteRow) -> Option<ProbeHealth> {
    let json: Option<String> = row.get("health");
    json.and_then(|json| serde_json::from_str(&json).ok())
}

fn config_rejected(row: &SqliteRow) -> Option<ConfigRejection> {
    let revision: Option<String> = row.get("config_rejected_revision");
    let error: Option<String> = row.get("config_error");
    revision.map(|revision| ConfigRejection {
        revision,
        error: error.unwrap_or_default(),
    })
}

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
        interval_secs: row.get("interval_secs"),
        config_revision: row.get("config_revision"),
        config_rejected: config_rejected(row),
        update_failed: update_failed(row),
        canary: row.get("update_canary"),
    }
}

pub(super) async fn record(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<ProbeRecord>> {
    let row = sqlx::query(
        "SELECT id, name, company_id, site_id, state, hostname, os, arch, probe_version,
                enrolled_at, token_id, interval_secs, config_revision,
                config_rejected_revision, config_error, update_failed_version, update_failed_error,
                update_canary
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
                p.enrolled_at, h.last_seen_at, h.status AS heartbeat, p.interval_secs,
                p.update_failed_version, p.update_failed_error, p.config_revision,
                p.config_rejected_revision, p.config_error, p.health, p.update_canary
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
            let heartbeat: Option<String> = row.get("heartbeat");
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
                interval_secs: row.get("interval_secs"),
                update_failed: update_failed(row),
                silent: heartbeat.as_deref() == Some("expired"),
                config_revision: row.get("config_revision"),
                config_rejected: config_rejected(row),
                config_sync: None,
                health: health(row),
                canary: row.get("update_canary"),
                update_status: None,
            }
        })
        .collect())
}

pub(super) async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    site_id: &str,
    interval_secs: u32,
) -> anyhow::Result<bool> {
    // The site must belong to the probe's company.
    let done = sqlx::query(
        "UPDATE probes SET name = ?1, site_id = ?2, interval_secs = ?4
         WHERE id = ?3
           AND EXISTS (SELECT 1 FROM sites s WHERE s.company_id = probes.company_id AND s.id = ?2)",
    )
    .bind(name)
    .bind(site_id)
    .bind(id)
    .bind(interval_secs)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn record_state(
    pool: &SqlitePool,
    id: &str,
    state: &ReportedState<'_>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE probes SET config_revision = ?, config_rejected_revision = ?, config_error = ?,
                probe_version = COALESCE(?, probe_version),
                update_failed_version = ?, update_failed_error = ?,
                pins_current = CASE WHEN ? THEN ? ELSE pins_current END,
                pins_next = CASE WHEN ? THEN ? ELSE pins_next END,
                health = COALESCE(?, health)
         WHERE id = ?",
    )
    .bind(state.config_revision)
    .bind(state.config_rejected.map(|r| r.revision.as_str()))
    .bind(state.config_rejected.map(|r| r.error.as_str()))
    .bind(state.probe_version)
    .bind(state.update_failed.map(|f| f.version.as_str()))
    .bind(state.update_failed.map(|f| f.error.as_str()))
    .bind(state.pins.is_some())
    .bind(state.pins.map(|p| p.current.as_str()))
    .bind(state.pins.is_some())
    .bind(state.pins.and_then(|p| p.next.as_deref()))
    .bind(
        state
            .health
            .map(|h| serde_json::to_string(h).expect("health serializes")),
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn record_inventory(
    pool: &SqlitePool,
    id: &str,
    inventory: &common::inventory::Inventory,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE probes SET inventory = ?, inventory_at = ? WHERE id = ?")
        .bind(serde_json::to_string(inventory)?)
        .bind(at)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The stored inventory; an unreadable one is none.
pub(super) async fn inventory(
    pool: &SqlitePool,
    id: &str,
) -> anyhow::Result<Option<(common::inventory::Inventory, DateTime<Utc>)>> {
    let row = sqlx::query("SELECT inventory, inventory_at FROM probes WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|row| {
        let json: Option<String> = row.get("inventory");
        let at: Option<DateTime<Utc>> = row.get("inventory_at");
        Some((serde_json::from_str(&json?).ok()?, at?))
    }))
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
    // 0 in the column: no limit (it is NOT NULL).
    .bind(t.max_uses.map_or(0, i64::from))
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
         WHERE revoked_at IS NULL AND (max_uses = 0 OR uses < max_uses)
           AND expires_at > ? AND {in_scope}
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
                max_uses: Some(row.get::<i64, _>("max_uses")).filter(|&n| n > 0),
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
           AND expires_at > ? AND (max_uses = 0 OR uses < max_uses)
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
