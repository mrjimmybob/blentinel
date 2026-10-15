//! Companies and sites (proposal 0002 §4.1, D-44): the rules for editing
//! them. Records are created by reports (`service::ingest`), not by hand.

use chrono::{DateTime, Utc};

use crate::model::{CompanyDetail, UpdateCompanyRequest, UpdateSiteRequest};
use crate::storage::Storage;

#[derive(Debug, thiserror::Error)]
pub enum CompanyError {
    #[error("no such company")]
    NotFound,
    #[error("no such site")]
    SiteNotFound,
    #[error("the name can't be empty")]
    EmptyName,
    #[error("{0}")]
    BadTimezone(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

pub type CompanyResult<T> = Result<T, CompanyError>;

/// Trim text; an empty optional field is stored as none.
fn optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn required_name(name: &str) -> CompanyResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CompanyError::EmptyName);
    }
    Ok(name.to_string())
}

pub async fn company(store: &dyn Storage, id: &str) -> CompanyResult<CompanyDetail> {
    store.company(id).await?.ok_or(CompanyError::NotFound)
}

pub async fn update_company(
    store: &dyn Storage,
    id: &str,
    fields: UpdateCompanyRequest,
    now: DateTime<Utc>,
) -> CompanyResult<CompanyDetail> {
    let fields = UpdateCompanyRequest {
        name: required_name(&fields.name)?,
        notes: optional(fields.notes),
        contact_name: optional(fields.contact_name),
        contact_email: optional(fields.contact_email),
        contact_phone: optional(fields.contact_phone),
    };
    if !store.update_company(id, &fields, now).await? {
        return Err(CompanyError::NotFound);
    }
    company(store, id).await
}

pub async fn set_archived(
    store: &dyn Storage,
    id: &str,
    archived: bool,
    now: DateTime<Utc>,
) -> CompanyResult<CompanyDetail> {
    if !store.set_company_archived(id, archived, now).await? {
        return Err(CompanyError::NotFound);
    }
    company(store, id).await
}

pub async fn update_site(
    store: &dyn Storage,
    company_id: &str,
    site_id: &str,
    fields: UpdateSiteRequest,
    now: DateTime<Utc>,
) -> CompanyResult<CompanyDetail> {
    let fields = UpdateSiteRequest {
        name: required_name(&fields.name)?,
        address: optional(fields.address),
        notes: optional(fields.notes),
        timezone: crate::service::maintenance::zone(&fields.timezone)
            .map_err(CompanyError::BadTimezone)?
            .name()
            .to_string(),
    };
    company(store, company_id).await?; // 404 for the company before the site
    if !store.update_site(company_id, site_id, &fields, now).await? {
        return Err(CompanyError::SiteNotFound);
    }
    company(store, company_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_required() {
        assert_eq!(required_name("  ACME Ltd ").unwrap(), "ACME Ltd");
        assert!(matches!(required_name("   "), Err(CompanyError::EmptyName)));
    }

    #[test]
    fn empty_optional_fields_are_none() {
        assert_eq!(optional(Some("  ".into())), None);
        assert_eq!(optional(Some(" x ".into())), Some("x".into()));
        assert_eq!(optional(None), None);
    }
}
