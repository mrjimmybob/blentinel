//! Dashboard views that combine storage with hub configuration.
//!
//! Plain reads with no rules (companies, devices, uptime) go straight from the
//! handler to storage (ENGINEERING.md §2.2); anything that adds a rule or
//! combines sources lives here, so the live API and the archive viewer share it.

use crate::model::CompanyProbe;
use crate::storage::{ProbeStore, StorageResult};

/// A company's probes. Unnamed probes keep an empty name; the UI falls back
/// to [`CompanyProbe::display_name`].
pub async fn company_probes(
    store: &dyn ProbeStore,
    company_id: &str,
) -> StorageResult<Vec<CompanyProbe>> {
    store.company_probes(company_id).await
}
