//! Dashboard views that combine storage with hub configuration.
//!
//! Plain reads with no rules (companies, devices, uptime) go straight from the
//! handler to storage (ENGINEERING.md §2.2); anything that adds a rule or
//! combines sources lives here, so the live API and the archive viewer share it.

use std::collections::HashMap;

use crate::model::CompanyProbe;
use crate::storage::{ProbeStore, StorageResult};

/// A company's probes, named from the hub's probe whitelist (`probe_id → name`).
/// Unnamed probes keep an empty name; the UI falls back to
/// [`CompanyProbe::display_name`].
pub async fn company_probes(
    store: &dyn ProbeStore,
    whitelist: &HashMap<String, String>,
    company_id: &str,
) -> StorageResult<Vec<CompanyProbe>> {
    let mut probes = store.company_probes(company_id).await?;
    for probe in &mut probes {
        probe.probe_name = whitelist.get(&probe.probe_id).cloned().unwrap_or_default();
    }
    Ok(probes)
}
