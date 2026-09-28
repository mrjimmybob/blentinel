//! Probes as records, added by enrollment codes (M2-01, proposal 0002, D-47).
//!
//! "Add probe" creates a code for a company and site; the probe redeems it
//! with `POST /api/enroll`, which creates its record. The record, not the
//! probe, decides where its reports belong (D-17). Also: creating companies
//! and sites by hand, which replaces their registration by reports (D-44).

use chrono::{DateTime, Duration, Utc};
use common::enrollment::{EnrollRequest, EnrollmentCode, SignedEnrollRequest};
use rand::Rng;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use x25519_dalek::StaticSecret;

use crate::model::{
    CompanyScope, CreateCodeRequest, CreatedCode, EnrollmentCodeInfo, UserInfo, is_valid_slug,
};
use crate::service::scope;
use crate::storage::{NewEnrollmentToken, PROBE_ACTIVE, ProbeRecord, Storage};

/// The most probes one code may enroll.
pub const MAX_USES: u32 = 1000;
/// The longest a code may stay valid (30 days).
pub const MAX_VALID_HOURS: u32 = 720;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// Unknown, or outside the user's companies (deliberately the same).
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("{0} already exists")]
    Exists(String),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

pub type ProbeResult<T> = Result<T, ProbeError>;

fn invalid(message: impl Into<String>) -> ProbeError {
    ProbeError::Invalid(message.into())
}

fn secret_hash(secret: &[u8]) -> String {
    hex::encode(Sha256::digest(secret))
}

fn check_id_and_name(what: &str, id: &str, name: &str) -> ProbeResult<()> {
    if !is_valid_slug(id) {
        return Err(invalid(format!(
            "{what} IDs are 1-64 lowercase letters, digits, '-', '_' or '.'"
        )));
    }
    if name.trim().is_empty() {
        return Err(invalid(format!("the {what} name can't be empty")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Companies and sites, created by hand
// ---------------------------------------------------------------------------

pub async fn create_company(
    store: &dyn Storage,
    id: &str,
    name: &str,
    now: DateTime<Utc>,
) -> ProbeResult<()> {
    check_id_and_name("company", id, name)?;
    if !store.create_company(id, name.trim(), now).await? {
        return Err(ProbeError::Exists(format!("company {id:?}")));
    }
    Ok(())
}

pub async fn create_site(
    store: &dyn Storage,
    scope: &CompanyScope,
    company_id: &str,
    id: &str,
    name: &str,
    now: DateTime<Utc>,
) -> ProbeResult<()> {
    scope::require(scope, company_id).map_err(|_| ProbeError::NotFound)?;
    if store.company(company_id).await?.is_none() {
        return Err(ProbeError::NotFound);
    }
    check_id_and_name("site", id, name)?;
    if !store.create_site(company_id, id, name.trim(), now).await? {
        return Err(ProbeError::Exists(format!("site {id:?}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Enrollment codes ("Add probe")
// ---------------------------------------------------------------------------

/// What the hub puts in every code about itself.
pub struct HubIdentity {
    pub hub_key: [u8; 32],
    /// The public half of its signing key (`common::crypto::hub_signing_key`).
    pub hub_signing_key: [u8; 32],
    /// SHA-256 of its TLS certificate, if it serves HTTPS.
    pub tls_cert_sha256: Option<[u8; 32]>,
}

/// Create a code for `request.company_id`/`site_id`. The code is returned
/// only now; the hub keeps only its secret's hash.
pub async fn create_code(
    store: &dyn Storage,
    actor: &UserInfo,
    request: CreateCodeRequest,
    hub: &HubIdentity,
    now: DateTime<Utc>,
) -> ProbeResult<CreatedCode> {
    scope::require(&actor.effective_scope(), &request.company_id)
        .map_err(|_| ProbeError::NotFound)?;
    let company = store
        .company(&request.company_id)
        .await?
        .ok_or(ProbeError::NotFound)?;
    if !company.sites.iter().any(|s| s.id == request.site_id) {
        return Err(invalid(format!(
            "no site {:?} in this company",
            request.site_id
        )));
    }
    if !(1..=MAX_USES).contains(&request.max_uses) {
        return Err(invalid(format!("a code can enroll 1 to {MAX_USES} probes")));
    }
    if !(1..=MAX_VALID_HOURS).contains(&request.valid_hours) {
        return Err(invalid(format!(
            "a code can be valid for 1 to {MAX_VALID_HOURS} hours"
        )));
    }
    let hub_url = request.hub_url.trim().trim_end_matches('/').to_string();
    let https = hub_url.starts_with("https://");
    if !(https || hub_url.starts_with("http://")) || hub_url.len() > 200 {
        return Err(invalid("the hub URL must start with https:// or http://"));
    }

    let mut token_id = [0u8; 8];
    let mut secret = [0u8; 16];
    OsRng.fill(&mut token_id);
    OsRng.fill(&mut secret);
    let probe_name = request
        .probe_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let token = NewEnrollmentToken {
        id: hex::encode(token_id),
        secret_hash: secret_hash(&secret),
        company_id: request.company_id,
        site_id: request.site_id,
        probe_name,
        created_by: Some(actor.username.clone()),
        expires_at: now + Duration::hours(i64::from(request.valid_hours)),
        max_uses: request.max_uses,
    };
    store.create_enrollment_token(&token, now).await?;

    // The pin only makes sense for the hub's own HTTPS (not behind a proxy).
    let code = EnrollmentCode {
        hub_url,
        hub_key: hub.hub_key,
        hub_signing_key: hub.hub_signing_key,
        tls_cert_sha256: hub.tls_cert_sha256.filter(|_| https),
        token_id,
        secret,
    };
    let info = EnrollmentCodeInfo {
        id: token.id,
        company_id: token.company_id,
        site_id: token.site_id,
        probe_name: token.probe_name,
        created_by: token.created_by,
        created_at: now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        expires_at: token.expires_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        max_uses: i64::from(token.max_uses),
        uses: 0,
    };
    Ok(CreatedCode {
        code: code.encode(),
        info,
    })
}

/// Revoke a code of one of the user's companies.
pub async fn revoke_code(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
    now: DateTime<Utc>,
) -> ProbeResult<String> {
    let company = store
        .enrollment_token_company(id)
        .await?
        .ok_or(ProbeError::NotFound)?;
    scope::require(scope, &company).map_err(|_| ProbeError::NotFound)?;
    store.revoke_enrollment_token(id, now).await?;
    Ok(company)
}

// ---------------------------------------------------------------------------
// Probe records
// ---------------------------------------------------------------------------

/// The probe, if it exists and belongs to one of the user's companies.
pub async fn scoped_record(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
) -> ProbeResult<ProbeRecord> {
    let record = store.probe_record(id).await?.ok_or(ProbeError::NotFound)?;
    scope::require(scope, &record.company_id).map_err(|_| ProbeError::NotFound)?;
    Ok(record)
}

pub async fn update_probe(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
    name: &str,
    site_id: &str,
) -> ProbeResult<ProbeRecord> {
    scoped_record(store, scope, id).await?;
    if name.trim().is_empty() {
        return Err(invalid("the probe name can't be empty"));
    }
    if !store.update_probe_record(id, name.trim(), site_id).await? {
        return Err(invalid(format!(
            "no site {site_id:?} in the probe's company"
        )));
    }
    scoped_record(store, scope, id).await
}

pub async fn revoke_probe(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
    now: DateTime<Utc>,
) -> ProbeResult<ProbeRecord> {
    let record = scoped_record(store, scope, id).await?;
    store.revoke_probe_record(id, now).await?;
    Ok(record)
}

// ---------------------------------------------------------------------------
// Enrollment (POST /api/enroll)
// ---------------------------------------------------------------------------

/// Why an enrollment was refused.
#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("the request can't be opened (not sealed for this hub)")]
    Undecryptable,
    #[error("not an enrollment request")]
    Malformed,
    #[error("the request isn't signed by the probe it names")]
    BadSignature,
    #[error("the probe's clock is more than 5 minutes off the hub's")]
    ClockSkew,
    /// Unknown, wrong secret, expired, used up or revoked: one answer on purpose.
    #[error("invalid or expired enrollment code")]
    InvalidCode,
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// Open, verify and redeem an enrollment request; create or replace the
/// probe's record in the code's company and site.
pub async fn enroll(
    store: &dyn Storage,
    hub_secret: &StaticSecret,
    sealed: &[u8],
    now: DateTime<Utc>,
) -> Result<(ProbeRecord, EnrollRequest), EnrollError> {
    let cleartext = common::crypto::open_from_probe(sealed, hub_secret)
        .map_err(|_| EnrollError::Undecryptable)?;
    let signed: SignedEnrollRequest =
        serde_json::from_slice(&cleartext).map_err(|_| EnrollError::Malformed)?;
    let request = signed.request;
    let signature = hex::decode(&signed.signature).map_err(|_| EnrollError::Malformed)?;
    let bytes = serde_json::to_vec(&request).map_err(|_| EnrollError::Malformed)?;
    common::crypto::verify(&bytes, &signature, &request.probe_id)
        .map_err(|_| EnrollError::BadSignature)?;

    let sent_at = DateTime::parse_from_rfc3339(&request.timestamp)
        .map_err(|_| EnrollError::Malformed)?
        .with_timezone(&Utc);
    if (now - sent_at).num_seconds().abs() > common::protocol::MAX_CLOCK_SKEW_SECS {
        return Err(EnrollError::ClockSkew);
    }

    let secret = hex::decode(&request.secret).map_err(|_| EnrollError::InvalidCode)?;
    let token = store
        .redeem_enrollment_token(&request.token_id, &secret_hash(&secret), now)
        .await?
        .ok_or(EnrollError::InvalidCode)?;

    let limit = |s: &str| s.chars().take(100).collect::<String>();
    let hostname = limit(&request.hostname);
    let record = ProbeRecord {
        id: request.probe_id.clone(),
        name: token.probe_name.unwrap_or_else(|| hostname.clone()),
        company_id: token.company_id,
        site_id: token.site_id,
        state: PROBE_ACTIVE.to_string(),
        hostname: Some(hostname),
        os: Some(limit(&request.os)),
        arch: Some(limit(&request.arch)),
        probe_version: Some(limit(&request.probe_version)),
        enrolled_at: now,
        token_id: Some(request.token_id.clone()),
    };
    store.save_probe_record(&record).await?;
    Ok((record, request))
}
