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
    self, CheckDefinition, CheckId, CheckSpec, ConfigRejection, MAX_CHECKS_PER_PROBE,
    MAX_SECRET_CHARS, ProbeConfig, Secrets,
};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

use crate::model::{CheckInfo, CheckRequest, CompanyScope, ConfigSync, ProbeChecks, ProbeInfo};
use crate::secrets::DataKey;
use crate::service::probes::{ProbeError, ProbeResult, scoped_record};
use crate::service::scope;
use crate::service::thresholds::{Levels, validate_check_limits};
use crate::storage::{CheckRecord, ProbeRecord, SealedSecrets, Storage, StorageResult};

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

/// The check as the API shows it, with the thresholds in force from
/// `levels` (the levels above it, D-64).
pub fn to_info(check: &CheckRecord, levels: &Levels) -> CheckInfo {
    CheckInfo {
        limits: check.limits,
        limits_in_force: levels.for_check(check),
        id: check.id.to_string(),
        probe_id: check.probe_id.clone(),
        name: check.name.clone(),
        target: check.spec.target(),
        spec: check.spec.clone(),
        enabled: check.enabled,
        alert_after: check.alert_after,
        depends_on: check.depends_on.as_ref().map(|d| d.to_string()),
        secrets_set: check.secrets.keys().cloned().collect(),
        created_at: fmt_time(check.created_at),
        updated_at: fmt_time(check.updated_at),
        updated_by: check.updated_by.clone(),
    }
}

// ---------------------------------------------------------------------------
// The config a probe runs
// ---------------------------------------------------------------------------

/// What the probe should run: `interval_secs` and the enabled `checks`, by
/// ID, with a revision derived from exactly that. The checks' secrets are
/// not in it yet ([`with_secrets`] puts them in, to send it): the revision
/// covers them **as sealed**, so it changes when one is saved again, and
/// says nothing about what they are to whoever reads it (D-86).
pub fn desired_config(interval_secs: u32, checks: &[CheckRecord]) -> ProbeConfig {
    let mut enabled: Vec<&CheckRecord> = checks.iter().filter(|c| c.enabled).collect();
    enabled.sort_by(|a, b| a.id.cmp(&b.id));
    let mut config = ProbeConfig {
        revision: String::new(),
        interval_secs,
        checks: enabled
            .iter()
            .map(|c| CheckDefinition {
                id: c.id.clone(),
                name: c.name.clone(),
                spec: c.spec.clone(),
                secrets: Secrets::default(),
            })
            .collect(),
    };
    // An empty revision while hashing: the digest covers everything else.
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(&config).expect("a config always serializes"));
    for check in enabled.iter().filter(|c| !c.secrets.is_empty()) {
        digest.update(check.id.as_str());
        for (name, sealed) in &check.secrets {
            digest.update(name);
            digest.update(sealed);
        }
    }
    config.revision = hex::encode(digest.finalize())[..REVISION_HEX_LEN].to_string();
    config
}

/// `config` with each check's secrets opened, as the probe gets it (in the
/// hub's sealed answer). One that can't be opened (a database restored
/// without its `hub_data.key`) is left out and logged: the probe then
/// refuses the config and says which check lacks what.
pub fn with_secrets(mut config: ProbeConfig, checks: &[CheckRecord], key: &DataKey) -> ProbeConfig {
    for definition in &mut config.checks {
        let Some(record) = checks.iter().find(|c| c.id == definition.id) else {
            continue;
        };
        for (name, sealed) in &record.secrets {
            match key.open(sealed) {
                Ok(value) => {
                    definition.secrets.0.insert(name.clone(), value);
                }
                Err(e) => tracing::error!(
                    check = %record.id,
                    "Can't open the check's {name}: {e:#}. Save it again on the check."
                ),
            }
        }
    }
    config
}

/// A check's secrets after a request: those it `has` that `spec` still
/// needs, with the ones `given` over them (an empty one keeps what there
/// is), sealed. And whether they are all `spec` needs: a check that runs
/// must have them all.
fn resolve_secrets(
    key: &DataKey,
    spec: &CheckSpec,
    has: &SealedSecrets,
    given: Option<&Secrets>,
) -> ProbeResult<(SealedSecrets, Result<(), String>)> {
    let needed = spec.secret_names();
    let mut sealed: SealedSecrets = has
        .iter()
        .filter(|(name, _)| needed.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut plain = Secrets::default();
    for (name, value) in &sealed {
        plain.0.insert(name.clone(), key.open(value)?);
    }
    for (name, value) in given.map(|g| &g.0).into_iter().flatten() {
        if !needed.contains(&name.as_str()) {
            return Err(ProbeError::Invalid(format!(
                "this check has no secret named {name}"
            )));
        }
        if value.is_empty() {
            continue;
        }
        if value.chars().count() > MAX_SECRET_CHARS {
            return Err(ProbeError::Invalid(format!(
                "a secret takes at most {MAX_SECRET_CHARS} characters"
            )));
        }
        sealed.insert(name.clone(), key.seal(value)?);
        plain.0.insert(name.clone(), value.clone());
    }
    let complete = spec.validate_secrets(&plain).map_err(|e| e.to_string());
    Ok((sealed, complete))
}

/// The secrets of a check that `request` makes or changes. Enabled, it
/// needs them all; disabled, it may wait for them.
fn request_secrets(
    key: &DataKey,
    request: &CheckRequest,
    has: &SealedSecrets,
) -> ProbeResult<SealedSecrets> {
    let (sealed, complete) = resolve_secrets(key, &request.spec, has, request.secrets.as_ref())?;
    match complete {
        Err(why) if request.enabled => Err(ProbeError::Invalid(why)),
        _ => Ok(sealed),
    }
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

/// Does a probe that said it runs `running` and last refused `rejected`
/// run `desired_revision`?
pub fn sync_state(
    running: Option<&str>,
    rejected: Option<&ConfigRejection>,
    desired_revision: &str,
) -> ConfigSync {
    if running == Some(desired_revision) {
        ConfigSync::InSync
    } else if rejected.is_some_and(|r| r.revision == desired_revision) {
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
    let sync = sync_state(
        record.config_revision.as_deref(),
        record.config_rejected.as_ref(),
        &desired.revision,
    );
    let levels = Levels::of(store, &record.company_id, &record.site_id).await?;
    Ok(ProbeChecks {
        probe_id: record.id.clone(),
        interval_secs: record.interval_secs,
        applied_revision: record.config_revision.clone(),
        config_error: (sync == ConfigSync::Rejected)
            .then(|| record.config_rejected.as_ref().map(|r| r.error.clone()))
            .flatten(),
        config_revision: desired.revision,
        sync,
        checks: checks.iter().map(|c| to_info(c, &levels)).collect(),
    })
}

/// The probes in `scope`, each with whether it runs its current checks:
/// the fleet view (M4-02, D-73).
pub async fn fleet(store: &dyn Storage, scope: &CompanyScope) -> StorageResult<Vec<ProbeInfo>> {
    let mut probes = store.probe_records(scope).await?;
    for probe in &mut probes {
        let checks = store.probe_checks(&probe.id).await?;
        let desired = desired_config(probe.interval_secs, &checks);
        probe.config_sync = Some(sync_state(
            probe.config_revision.as_deref(),
            probe.config_rejected.as_ref(),
            &desired.revision,
        ));
    }
    Ok(probes)
}

fn validate(request: &CheckRequest) -> ProbeResult<()> {
    checks::validate_name(request.name.trim())
        .and_then(|()| request.spec.validate())
        .map_err(|e| ProbeError::Invalid(e.to_string()))?;
    request
        .alert_after
        .validate()
        .map_err(ProbeError::Invalid)?;
    validate_check_limits(&request.spec, &request.limits).map_err(ProbeError::Invalid)
}

/// The check `request` depends on, among its probe's `checks` (D-68).
fn parent(
    checks: &[CheckRecord],
    id: Option<&CheckId>,
    request: &CheckRequest,
) -> ProbeResult<Option<CheckId>> {
    match request.depends_on.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(parent) => crate::service::dependencies::validate(checks, id, parent)
            .map(Some)
            .map_err(ProbeError::Invalid),
    }
}

/// Checks as the API shows them, all of one probe (`probe_id`).
pub async fn infos(
    store: &dyn Storage,
    probe_id: &str,
    checks: &[CheckRecord],
) -> ProbeResult<Vec<CheckInfo>> {
    let record = store
        .probe_record(probe_id)
        .await?
        .ok_or(ProbeError::NotFound)?;
    let levels = Levels::of(store, &record.company_id, &record.site_id).await?;
    Ok(checks.iter().map(|c| to_info(c, &levels)).collect())
}

/// Add a check to a probe of one of the user's companies.
pub async fn create_check(
    store: &dyn Storage,
    key: &DataKey,
    scope: &CompanyScope,
    probe_id: &str,
    request: &CheckRequest,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<CheckRecord> {
    let record = scoped_record(store, scope, probe_id).await?;
    validate(request)?;
    let checks = store.probe_checks(probe_id).await?;
    if checks.len() >= MAX_CHECKS_PER_PROBE {
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
        limits: request.limits,
        depends_on: parent(&checks, None, request)?,
        secrets: request_secrets(key, request, &SealedSecrets::new())?,
        created_at: now,
        updated_at: now,
        updated_by: Some(by.to_string()),
    };
    store.create_check(&check).await?;
    Ok(check)
}

/// Add several checks to a probe of one of the user's companies: all of
/// them or, if one is wrong, none. A check the probe already has (the same
/// settings, under any name) is left out, so a template or a suggestion
/// accepted twice doesn't double it. Returns the probe, the new checks and
/// how many were left out. A check that needs a secret it wasn't given (a
/// template keeps none) is added **disabled**: it runs once someone gives
/// it one.
pub async fn add_checks(
    store: &dyn Storage,
    key: &DataKey,
    scope: &CompanyScope,
    probe_id: &str,
    requests: &[CheckRequest],
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<(ProbeRecord, Vec<CheckRecord>, usize)> {
    let record = scoped_record(store, scope, probe_id).await?;
    if requests.is_empty() {
        return Err(ProbeError::Invalid("no checks to add".into()));
    }
    for request in requests {
        validate(request).map_err(|e| match e {
            ProbeError::Invalid(why) => ProbeError::Invalid(format!("{}: {why}", request.name)),
            other => other,
        })?;
    }
    let existing = store.probe_checks(probe_id).await?;
    let mut specs: Vec<&common::checks::CheckSpec> = existing.iter().map(|c| &c.spec).collect();
    let mut new = Vec::new();
    for request in requests {
        if !specs.contains(&&request.spec) {
            specs.push(&request.spec);
            new.push(request);
        }
    }
    if existing.len() + new.len() > MAX_CHECKS_PER_PROBE {
        return Err(ProbeError::Invalid(format!(
            "a probe runs at most {MAX_CHECKS_PER_PROBE} checks"
        )));
    }
    let mut added = Vec::with_capacity(new.len());
    for request in &new {
        let (secrets, complete) = resolve_secrets(
            key,
            &request.spec,
            &SealedSecrets::new(),
            request.secrets.as_ref(),
        )?;
        let check = CheckRecord {
            id: new_check_id(),
            probe_id: record.id.clone(),
            company_id: record.company_id.clone(),
            name: request.name.trim().to_string(),
            spec: request.spec.clone(),
            enabled: request.enabled && complete.is_ok(),
            alert_after: request.alert_after,
            limits: request.limits,
            // Among checks made together, none is behind another yet.
            depends_on: None,
            secrets,
            created_at: now,
            updated_at: now,
            updated_by: Some(by.to_string()),
        };
        store.create_check(&check).await?;
        added.push(check);
    }
    let skipped = requests.len() - added.len();
    Ok((record, added, skipped))
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
    // New IDs, and the links between them follow (D-68): created without,
    // then linked, so a parent exists before a check points to it.
    let ids: std::collections::HashMap<CheckId, CheckId> = sources
        .iter()
        .map(|s| (s.id.clone(), new_check_id()))
        .collect();
    let mut copies = Vec::with_capacity(sources.len());
    for source in sources {
        let copy = CheckRecord {
            id: ids[&source.id].clone(),
            probe_id: target.id.clone(),
            company_id: target.company_id.clone(),
            depends_on: source.depends_on.as_ref().and_then(|d| ids.get(d)).cloned(),
            created_at: now,
            updated_at: now,
            updated_by: Some(by.to_string()),
            ..source
        };
        store
            .create_check(&CheckRecord {
                depends_on: None,
                ..copy.clone()
            })
            .await?;
        copies.push(copy);
    }
    for copy in copies.iter().filter(|c| c.depends_on.is_some()) {
        store.update_check(copy).await?;
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
    key: &DataKey,
    scope: &CompanyScope,
    id: &str,
    request: &CheckRequest,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<(CheckRecord, CheckRecord)> {
    let before = scoped_check(store, scope, id).await?;
    validate(request)?;
    let checks = store.probe_checks(&before.probe_id).await?;
    let after = CheckRecord {
        name: request.name.trim().to_string(),
        spec: request.spec.clone(),
        enabled: request.enabled,
        alert_after: request.alert_after,
        limits: request.limits,
        depends_on: parent(&checks, Some(&before.id), request)?,
        secrets: request_secrets(key, request, &before.secrets)?,
        updated_at: now,
        updated_by: Some(by.to_string()),
        ..before.clone()
    };
    if !store.update_check(&after).await? {
        return Err(ProbeError::NotFound);
    }
    Ok((before, after))
}

/// Delete a check and its silence, and close its open incident (its results
/// and incidents stay in the history).
pub async fn delete_check(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<CheckRecord> {
    let check = scoped_check(store, scope, id).await?;
    store.delete_check(&check.id).await?;
    store.clear_check_silence(&check.id).await?;
    crate::service::incidents::on_check_deleted(store, &check.probe_id, check.id.as_str(), by, now)
        .await?;
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
            limits: Default::default(),
            depends_on: None,
            secrets: Default::default(),
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

    /// D-86: the revision covers a check's secrets as they are sealed, the
    /// config for the probe has them opened, and one that can't be opened
    /// is left out (the probe then says which check lacks what).
    #[test]
    fn secrets_change_the_revision_and_are_opened_for_the_probe() {
        let key = DataKey::from_bytes([7; 32]);
        let plain = check("a", true);
        let base = desired_config(60, std::slice::from_ref(&plain));
        let mut secret = plain.clone();
        secret
            .secrets
            .insert("community".into(), key.seal("public").unwrap());
        let sealed = desired_config(60, std::slice::from_ref(&secret));
        assert_ne!(sealed.revision, base.revision);
        // Not in what the revision is shown with, nor in the config before it is sent.
        assert!(sealed.checks[0].secrets.is_empty());
        // Sealed again (the same text): another revision, which says nothing of the text.
        let mut again = plain.clone();
        again
            .secrets
            .insert("community".into(), key.seal("public").unwrap());
        assert_ne!(
            desired_config(60, std::slice::from_ref(&again)).revision,
            sealed.revision
        );
        // A disabled check's secrets aren't the probe's business.
        let mut off = secret.clone();
        off.enabled = false;
        assert_eq!(
            desired_config(60, &[off]).revision,
            desired_config(60, &[]).revision
        );

        let sent = with_secrets(sealed.clone(), std::slice::from_ref(&secret), &key);
        assert_eq!(sent.checks[0].secrets.get("community"), Some("public"));
        assert_eq!(sent.revision, sealed.revision);
        let other_key = DataKey::from_bytes([8; 32]);
        let unopened = with_secrets(sealed, &[secret], &other_key);
        assert!(unopened.checks[0].secrets.is_empty());
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
