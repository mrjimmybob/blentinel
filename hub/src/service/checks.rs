//! Checks as hub records (M2-04, proposal 0003 §4.1, D-15, D-49).
//!
//! A probe runs what the hub says: its report interval and its enabled
//! checks, the [`ProbeConfig`] [`desired_config`] builds. The config's
//! revision is a digest of its content, so it needs no counter and means the
//! same thing on any hub: the probe echoes the revision it runs in each
//! report, and the hub sends the config in its answer only when they differ
//! ([`config_to_send`]).
//!
//! Who may change checks: `ManageCompanies` within the user's company scope,
//! like renaming a probe; seeing them: anyone who sees the probe.

use chrono::{DateTime, Utc};
use common::checks::{
    self, CheckDefinition, CheckId, ConfigRejection, MAX_CHECKS_PER_PROBE, ProbeConfig,
};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

use crate::model::{CheckInfo, CheckRequest, CompanyScope, ConfigSync, ProbeChecks};
use crate::service::probes::{ProbeError, ProbeResult, scoped_record};
use crate::service::scope;
use crate::storage::{CheckRecord, ProbeRecord, Storage};

/// Hex digits of the SHA-256 kept as a config's revision (64 bits).
const REVISION_HEX_LEN: usize = 16;

fn fmt_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// A new, random check ID (16 bytes, hex).
fn new_check_id() -> CheckId {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    CheckId(hex::encode(bytes))
}

/// The check as the API shows it.
pub fn to_info(check: &CheckRecord) -> CheckInfo {
    CheckInfo {
        id: check.id.to_string(),
        probe_id: check.probe_id.clone(),
        name: check.name.clone(),
        target: check.spec.target(),
        spec: check.spec.clone(),
        enabled: check.enabled,
        alert_after: check.alert_after,
        created_at: fmt_time(check.created_at),
        updated_at: fmt_time(check.updated_at),
        updated_by: check.updated_by.clone(),
    }
}

// ---------------------------------------------------------------------------
// The config a probe runs
// ---------------------------------------------------------------------------

/// What the probe should run: `interval_secs` and the enabled `checks`, by
/// ID, with a revision derived from exactly that.
pub fn desired_config(interval_secs: u32, checks: &[CheckRecord]) -> ProbeConfig {
    let mut definitions: Vec<CheckDefinition> = checks
        .iter()
        .filter(|c| c.enabled)
        .map(|c| CheckDefinition {
            id: c.id.clone(),
            name: c.name.clone(),
            spec: c.spec.clone(),
        })
        .collect();
    definitions.sort_by(|a, b| a.id.cmp(&b.id));
    let mut config = ProbeConfig {
        revision: String::new(),
        interval_secs,
        checks: definitions,
    };
    // An empty revision while hashing: the digest covers everything else.
    let json = serde_json::to_vec(&config).expect("a config always serializes");
    config.revision = hex::encode(Sha256::digest(json))[..REVISION_HEX_LEN].to_string();
    config
}

/// The config to put in the answer to a report that says the probe runs
/// `running` and last refused `rejected`: `desired` if it's neither, since
/// sending again what the probe refused would change nothing.
pub fn config_to_send(
    desired: ProbeConfig,
    running: Option<&str>,
    rejected: Option<&ConfigRejection>,
) -> Option<ProbeConfig> {
    let refused = rejected.is_some_and(|r| r.revision == desired.revision);
    (running != Some(desired.revision.as_str()) && !refused).then_some(desired)
}

/// Does a probe that reported `record`'s config state run `desired_revision`?
pub fn sync_state(record: &ProbeRecord, desired_revision: &str) -> ConfigSync {
    if record.config_revision.as_deref() == Some(desired_revision) {
        ConfigSync::InSync
    } else if record
        .config_rejected
        .as_ref()
        .is_some_and(|r| r.revision == desired_revision)
    {
        ConfigSync::Rejected
    } else {
        ConfigSync::Pending
    }
}

// ---------------------------------------------------------------------------
// Managing checks
// ---------------------------------------------------------------------------

/// A probe's checks and whether it runs them.
pub async fn probe_checks(
    store: &dyn Storage,
    scope: &CompanyScope,
    probe_id: &str,
) -> ProbeResult<ProbeChecks> {
    let record = scoped_record(store, scope, probe_id).await?;
    let checks = store.probe_checks(probe_id).await?;
    let desired = desired_config(record.interval_secs, &checks);
    let sync = sync_state(&record, &desired.revision);
    Ok(ProbeChecks {
        probe_id: record.id.clone(),
        interval_secs: record.interval_secs,
        applied_revision: record.config_revision.clone(),
        config_error: (sync == ConfigSync::Rejected)
            .then(|| record.config_rejected.as_ref().map(|r| r.error.clone()))
            .flatten(),
        config_revision: desired.revision,
        sync,
        checks: checks.iter().map(to_info).collect(),
    })
}

fn validate(request: &CheckRequest) -> ProbeResult<()> {
    checks::validate_name(request.name.trim())
        .and_then(|()| request.spec.validate())
        .map_err(|e| ProbeError::Invalid(e.to_string()))?;
    request.alert_after.validate().map_err(ProbeError::Invalid)
}

/// Add a check to a probe of one of the user's companies.
pub async fn create_check(
    store: &dyn Storage,
    scope: &CompanyScope,
    probe_id: &str,
    request: &CheckRequest,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<CheckRecord> {
    let record = scoped_record(store, scope, probe_id).await?;
    validate(request)?;
    if store.probe_checks(probe_id).await?.len() >= MAX_CHECKS_PER_PROBE {
        return Err(ProbeError::Invalid(format!(
            "a probe runs at most {MAX_CHECKS_PER_PROBE} checks"
        )));
    }
    let check = CheckRecord {
        id: new_check_id(),
        probe_id: record.id,
        company_id: record.company_id,
        name: request.name.trim().to_string(),
        spec: request.spec.clone(),
        enabled: request.enabled,
        alert_after: request.alert_after,
        created_at: now,
        updated_at: now,
        updated_by: Some(by.to_string()),
    };
    store.create_check(&check).await?;
    Ok(check)
}

/// Give probe `to` a copy of every check of probe `from` (new IDs, same
/// names, specs and enabled flags), for setting up a similar machine. Both
/// probes must be in the user's companies. Returns the new checks.
pub async fn copy_checks(
    store: &dyn Storage,
    scope: &CompanyScope,
    to: &str,
    from: &str,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<(ProbeRecord, Vec<CheckRecord>)> {
    let target = scoped_record(store, scope, to).await?;
    scoped_record(store, scope, from).await?;
    if to == from {
        return Err(ProbeError::Invalid(
            "choose another probe to copy from".into(),
        ));
    }
    let sources = store.probe_checks(from).await?;
    let existing = store.probe_checks(to).await?.len();
    if existing + sources.len() > MAX_CHECKS_PER_PROBE {
        return Err(ProbeError::Invalid(format!(
            "a probe runs at most {MAX_CHECKS_PER_PROBE} checks"
        )));
    }
    let mut copies = Vec::with_capacity(sources.len());
    for source in sources {
        let copy = CheckRecord {
            id: new_check_id(),
            probe_id: target.id.clone(),
            company_id: target.company_id.clone(),
            created_at: now,
            updated_at: now,
            updated_by: Some(by.to_string()),
            ..source
        };
        store.create_check(&copy).await?;
        copies.push(copy);
    }
    Ok((target, copies))
}

/// A check of one of the user's companies.
pub async fn scoped_check(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
) -> ProbeResult<CheckRecord> {
    let check = store
        .check(&CheckId(id.to_string()))
        .await?
        .ok_or(ProbeError::NotFound)?;
    scope::require(scope, &check.company_id).map_err(|_| ProbeError::NotFound)?;
    Ok(check)
}

/// Replace a check's name, spec, enabled flag and when it alerts. Returns it before and after.
pub async fn update_check(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
    request: &CheckRequest,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<(CheckRecord, CheckRecord)> {
    let before = scoped_check(store, scope, id).await?;
    validate(request)?;
    let after = CheckRecord {
        name: request.name.trim().to_string(),
        spec: request.spec.clone(),
        enabled: request.enabled,
        alert_after: request.alert_after,
        updated_at: now,
        updated_by: Some(by.to_string()),
        ..before.clone()
    };
    if !store.update_check(&after).await? {
        return Err(ProbeError::NotFound);
    }
    Ok((before, after))
}

/// Delete a check and its silence (its results stay in the history).
pub async fn delete_check(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
) -> ProbeResult<CheckRecord> {
    let check = scoped_check(store, scope, id).await?;
    store.delete_check(&check.id).await?;
    store.clear_check_silence(&check.id).await?;
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::CheckSpec;

    fn check(id: &str, enabled: bool) -> CheckRecord {
        let at = "2026-09-30T10:00:00Z".parse().unwrap();
        CheckRecord {
            id: CheckId(id.into()),
            probe_id: "p".into(),
            company_id: "acme".into(),
            name: format!("check {id}"),
            spec: CheckSpec::Cpu,
            enabled,
            alert_after: Default::default(),
            created_at: at,
            updated_at: at,
            updated_by: None,
        }
    }

    #[test]
    fn the_config_has_the_enabled_checks_by_id_and_a_stable_revision() {
        let config = desired_config(60, &[check("b", true), check("a", true), check("c", false)]);
        let ids: Vec<_> = config.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(config.revision.len(), REVISION_HEX_LEN);
        assert_eq!(config.validate(), Ok(()));

        // Same content, same revision, whatever the order of the records.
        let again = desired_config(60, &[check("a", true), check("b", true)]);
        assert_eq!(again.revision, config.revision);
    }

    #[test]
    fn when_a_check_alerts_isnt_part_of_the_probes_config() {
        let base = desired_config(60, &[check("a", true)]).revision;
        let mut patient = check("a", true);
        patient.alert_after.fail_after = Some(5);
        assert_eq!(desired_config(60, &[patient]).revision, base);
    }

    #[test]
    fn any_change_is_a_new_revision() {
        let base = desired_config(60, &[check("a", true)]).revision;
        assert_ne!(desired_config(30, &[check("a", true)]).revision, base);
        let mut renamed = check("a", true);
        renamed.name = "renamed".into();
        assert_ne!(desired_config(60, &[renamed]).revision, base);
        assert_ne!(desired_config(60, &[]).revision, base);
    }

    #[test]
    fn the_config_is_sent_only_to_a_probe_running_something_else() {
        let desired = desired_config(60, &[check("a", true)]);
        let revision = desired.revision.clone();
        assert!(config_to_send(desired.clone(), None, None).is_some());
        assert!(config_to_send(desired.clone(), Some("old"), None).is_some());
        assert!(config_to_send(desired.clone(), Some(&revision), None).is_none());

        // Not again once refused; a newer one is sent.
        let refused = ConfigRejection {
            revision: revision.clone(),
            error: "no".into(),
        };
        assert!(config_to_send(desired.clone(), Some("old"), Some(&refused)).is_none());
        let older_refusal = ConfigRejection {
            revision: "older".into(),
            error: "no".into(),
        };
        assert!(config_to_send(desired, Some("old"), Some(&older_refusal)).is_some());
    }
}
