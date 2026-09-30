//! Company scoping for single items (D-45). Lists are filtered in storage;
//! requests for one company, probe, check or silence check here first.
//!
//! Outside the scope the answer is "not found", never "forbidden", so a
//! user can't find out which other companies exist.

use crate::model::CompanyScope;
use crate::storage::{ProbeStore, StorageResult};

/// Out of scope, or not there at all (deliberately indistinguishable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not found")]
pub struct OutOfScope;

/// `Ok` if `scope` covers `company_id`.
pub fn require(scope: &CompanyScope, company_id: &str) -> Result<(), OutOfScope> {
    if scope.allows(company_id) {
        Ok(())
    } else {
        Err(OutOfScope)
    }
}

/// `Ok` if the probe exists and its company is in `scope`.
pub async fn require_probe(
    store: &dyn ProbeStore,
    scope: &CompanyScope,
    probe_id: &str,
) -> StorageResult<Result<(), OutOfScope>> {
    Ok(match store.probe_company(probe_id).await? {
        Some(company) => require(scope, &company),
        None => Err(OutOfScope),
    })
}
