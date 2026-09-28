//! SQLite queries for companies and sites, behind [`super::SqliteStorage`]'s
//! `CompanyStore` implementation.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::model::{
    CompanyDetail, CompanyInfo, CompanyScope, SiteInfo, UpdateCompanyRequest, UpdateSiteRequest,
};
use crate::storage::Registered;

fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

const COMPANY_QUERY: &str = "SELECT c.id, c.name, c.notes, c.contact_name, c.contact_email,
        c.contact_phone, c.created_at, c.reviewed_at, c.archived_at,
        (SELECT COUNT(*) FROM sites s WHERE s.company_id = c.id) AS site_count,
        (SELECT COUNT(*) FROM probe_heartbeats h WHERE h.company_id = c.id) AS probe_count
     FROM companies c";

fn to_company(row: &SqliteRow) -> CompanyInfo {
    let created_at: DateTime<Utc> = row.get("created_at");
    let reviewed_at: Option<DateTime<Utc>> = row.get("reviewed_at");
    let archived_at: Option<DateTime<Utc>> = row.get("archived_at");
    CompanyInfo {
        id: row.get("id"),
        name: row.get("name"),
        notes: row.get("notes"),
        contact_name: row.get("contact_name"),
        contact_email: row.get("contact_email"),
        contact_phone: row.get("contact_phone"),
        is_new: reviewed_at.is_none(),
        archived: archived_at.is_some(),
        created_at: fmt_time(created_at),
        site_count: row.get("site_count"),
        probe_count: row.get("probe_count"),
    }
}

pub(super) async fn register(
    pool: &SqlitePool,
    company_id: &str,
    site: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Registered> {
    let mut tx = pool.begin().await?;
    let company =
        sqlx::query("INSERT OR IGNORE INTO companies (id, name, created_at) VALUES (?1, ?1, ?2)")
            .bind(company_id)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
    let site = if site.is_empty() {
        false
    } else {
        sqlx::query(
            "INSERT OR IGNORE INTO sites (company_id, id, name, created_at) VALUES (?1, ?2, ?2, ?3)",
        )
        .bind(company_id)
        .bind(site)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0
    };
    tx.commit().await?;
    Ok(Registered { company, site })
}

pub(super) async fn companies(
    pool: &SqlitePool,
    scope: &CompanyScope,
) -> anyhow::Result<Vec<CompanyInfo>> {
    let (condition, binds) = super::scope::condition("c.id", scope);
    let sql = format!("{COMPANY_QUERY} WHERE {condition} ORDER BY c.name COLLATE NOCASE, c.id");
    let mut query = sqlx::query(&sql);
    for id in &binds {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows.iter().map(to_company).collect())
}

pub(super) async fn company(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<CompanyDetail>> {
    let sql = format!("{COMPANY_QUERY} WHERE c.id = ?");
    let Some(row) = sqlx::query(&sql).bind(id).fetch_optional(pool).await? else {
        return Ok(None);
    };
    let sites = sqlx::query(
        "SELECT id, name, address, notes, reviewed_at FROM sites
         WHERE company_id = ? ORDER BY name COLLATE NOCASE, id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?
    .iter()
    .map(|r| {
        let reviewed_at: Option<DateTime<Utc>> = r.get("reviewed_at");
        SiteInfo {
            id: r.get("id"),
            name: r.get("name"),
            address: r.get("address"),
            notes: r.get("notes"),
            is_new: reviewed_at.is_none(),
        }
    })
    .collect();
    Ok(Some(CompanyDetail {
        company: to_company(&row),
        sites,
    }))
}

pub(super) async fn update_company(
    pool: &SqlitePool,
    id: &str,
    fields: &UpdateCompanyRequest,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE companies SET name = ?, notes = ?, contact_name = ?, contact_email = ?,
                              contact_phone = ?, reviewed_at = ?
         WHERE id = ?",
    )
    .bind(&fields.name)
    .bind(&fields.notes)
    .bind(&fields.contact_name)
    .bind(&fields.contact_email)
    .bind(&fields.contact_phone)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn set_company_archived(
    pool: &SqlitePool,
    id: &str,
    archived: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    // Archiving is a decision about the company, so it also counts as reviewed.
    let done = sqlx::query(
        "UPDATE companies SET archived_at = ?, reviewed_at = COALESCE(reviewed_at, ?) WHERE id = ?",
    )
    .bind(archived.then_some(now))
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub(super) async fn update_site(
    pool: &SqlitePool,
    company_id: &str,
    site_id: &str,
    fields: &UpdateSiteRequest,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE sites SET name = ?, address = ?, notes = ?, reviewed_at = ?
         WHERE company_id = ? AND id = ?",
    )
    .bind(&fields.name)
    .bind(&fields.address)
    .bind(&fields.notes)
    .bind(now)
    .bind(company_id)
    .bind(site_id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}
