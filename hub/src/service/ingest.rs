//! Accepting a report from a probe: every rule between "bytes arrived" and
//! "report stored", with no HTTP in sight (ENGINEERING.md §2.2).
//!
//! Order matters and is part of the security design:
//! 1. open the seal (only this hub can)
//! 2. parse, check the protocol version
//! 3. report must be signed by the key it names (D-25), and that key must be
//!    an enrolled, active probe; the probe's record decides its company and
//!    site, whatever the report says (D-17, D-47), and only results of the
//!    probe's own checks are kept (D-49)
//! 4. timestamp within the accepted window, checked *after* the signature so
//!    forged traffic learns nothing about it (D-28)
//! 5. store only if newer than the probe's latest report, atomically (D-28)
//! 6. heartbeat, what config the probe runs, the results of its commands
//!    (D-56), re-arm the offline alert (D-27), evaluate alerts in the background
//! 7. the config to send back, if the probe runs another one (D-49), the
//!    approved release, if it runs another version (D-55), and its pending
//!    commands (D-56)

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::ack::UpdateOffer;
use common::checks::{CheckId, ProbeConfig};
use common::commands::Command;
use common::models::{ReportVersion, StatusReport, UpdateFailure};
use common::protocol::{
    self, MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION, TimestampRejection,
};
use tokio::sync::RwLock;
use x25519_dalek::StaticSecret;

use crate::config::HubConfig;
use crate::model::short_probe_id;
use crate::service::alerts::{self, Notifier};
use crate::service::checks;
use crate::service::commands;
use crate::service::releases;
use crate::service::{incidents, maintenance, thresholds};
use crate::storage::{CheckRecord, ReportedState, SaveOutcome, Storage};

/// Why a report was not accepted. Each variant maps to one HTTP status in the
/// handler; the `Display` text is what gets logged.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("decryption failed (not sealed for this hub, or corrupted)")]
    Undecryptable,
    #[error("payload is not a status report")]
    NotAReport,
    #[error(
        "unsupported protocol version {0} (supported {MIN_SUPPORTED_PROTOCOL_VERSION}..={PROTOCOL_VERSION})"
    )]
    UnsupportedVersion(u16),
    #[error("report from unregistered probe {0} (enroll it with a code)")]
    UnknownProbe(String),
    #[error("report from revoked probe {}", short_probe_id(.0))]
    Revoked(String),
    #[error("unsigned report claiming to be probe {0}")]
    MissingSignature(String),
    #[error("invalid signature for probe {0}")]
    BadSignature(String),
    #[error("report from {probe} at {timestamp}: {}", timestamp_reason(*.rejection))]
    TimestampRejected {
        probe: String,
        timestamp: DateTime<Utc>,
        rejection: TimestampRejection,
    },
    #[error(
        "report from {probe} at {timestamp} is not newer than its latest stored report (replay or duplicate delivery)"
    )]
    NotNewer {
        probe: String,
        timestamp: DateTime<Utc>,
    },
    #[error("storage failure: {0:#}")]
    Storage(#[source] anyhow::Error),
}

/// Human-readable reason for a timestamp rejection (also sent to the probe).
pub fn timestamp_reason(rejection: TimestampRejection) -> &'static str {
    match rejection {
        TimestampRejection::InFuture => "timestamp is in the future (probe clock ahead of hub?)",
        TimestampRejection::TooOld => "timestamp is older than any probe buffers reports",
    }
}

/// A report that passed every check and was stored.
#[derive(Debug, Clone)]
pub struct Accepted {
    /// As stored: filed under the probe's company and site, with only the
    /// results of its own checks.
    pub report: StatusReport,
    /// The probe's name (from its record).
    pub probe_name: String,
    /// What the probe should run from now on, if it runs something else.
    pub config: Option<ProbeConfig>,
    /// The approved release, if the probe runs another version (D-55).
    pub update: Option<UpdateOffer>,
    /// Commands waiting for the probe (D-56).
    pub commands: Vec<Command>,
    /// The hub's next pins, or that it switched (D-61).
    pub rotation: Option<common::pins::Rotation>,
}

/// What the ingest flow needs from the hub.
pub struct IngestDeps<'a> {
    pub storage: &'a Arc<dyn Storage>,
    pub config: &'a Arc<RwLock<HubConfig>>,
    pub hub_secret: &'a StaticSecret,
    /// The pins the hub tells probes, and the previous key (D-61).
    pub credentials: &'a crate::keys::Credentials,
    pub notifier: &'a Arc<dyn Notifier>,
    /// Where the probe releases are (`service::releases`).
    pub releases_dir: &'a Path,
    /// The release keys the hub trusts.
    pub release_keys: &'a Arc<Vec<[u8; 32]>>,
}

/// Steps 1–3a: open, parse, check the version, and verify the signature
/// against the key the report names. Pure (no I/O); the probe record is
/// checked next, in [`ingest_report`].
pub fn authenticate(sealed: &[u8], hub_secret: &StaticSecret) -> Result<StatusReport, IngestError> {
    authenticate_with_any(sealed, &[hub_secret])
}

/// [`authenticate`] with the current key, then the previous one: after a
/// key switch, a probe may still deliver reports it sealed before (D-61).
pub fn authenticate_with_any(
    sealed: &[u8],
    hub_secrets: &[&StaticSecret],
) -> Result<StatusReport, IngestError> {
    let cleartext = hub_secrets
        .iter()
        .find_map(|secret| common::crypto::open_from_probe(sealed, secret).ok())
        .ok_or(IngestError::Undecryptable)?;
    // The version first, so an older or newer probe is told why.
    let version: ReportVersion =
        serde_json::from_slice(&cleartext).map_err(|_| IngestError::NotAReport)?;
    let version = version.protocol_version.unwrap_or(1);
    if !(MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&version) {
        return Err(IngestError::UnsupportedVersion(version));
    }
    let mut report: StatusReport =
        serde_json::from_slice(&cleartext).map_err(|_| IngestError::NotAReport)?;

    // The signature covers the canonical report without the signature itself.
    let signature = report
        .signature
        .take()
        .ok_or_else(|| IngestError::MissingSignature(report.probe_id.clone()))?;
    let signed_bytes =
        serde_json::to_vec(&report.to_signable()).map_err(|_| IngestError::NotAReport)?;
    common::crypto::verify(&signed_bytes, &signature, &report.probe_id)
        .map_err(|_| IngestError::BadSignature(report.probe_id.clone()))?;
    report.signature = Some(signature);
    Ok(report)
}

/// The full flow (steps 1–6) for a sealed report received at `now`.
pub async fn ingest_report(
    deps: IngestDeps<'_>,
    sealed: &[u8],
    now: DateTime<Utc>,
) -> Result<Accepted, IngestError> {
    let secrets: Vec<&StaticSecret> = std::iter::once(deps.hub_secret)
        .chain(deps.credentials.prev_secret.as_ref())
        .collect();
    let mut report = authenticate_with_any(sealed, &secrets)?;
    let short_id = short_probe_id(&report.probe_id).to_string();

    // Step 3b: an enrolled, active probe. Its record, not the report, says
    // which company and site it reports for.
    let record = deps
        .storage
        .probe_record(&report.probe_id)
        .await
        .map_err(IngestError::Storage)?
        .ok_or_else(|| IngestError::UnknownProbe(short_id.clone()))?;
    if !record.is_active() {
        return Err(IngestError::Revoked(report.probe_id.clone()));
    }
    report.company_id = record.company_id.clone();
    report.site = record.site_id.clone();
    let probe_checks = deps
        .storage
        .probe_checks(&report.probe_id)
        .await
        .map_err(IngestError::Storage)?;
    keep_own_results(&mut report, &probe_checks);
    let report = &report;
    let short_id = short_id.as_str();

    protocol::check_report_timestamp(report.timestamp, now).map_err(|rejection| {
        IngestError::TimestampRejected {
            probe: short_id.to_string(),
            timestamp: report.timestamp,
            rejection,
        }
    })?;

    // The hub's verdict on each result (D-64), stored with it.
    let mut evaluations = thresholds::evaluate_report(deps.storage.as_ref(), report, &probe_checks)
        .await
        .map_err(IngestError::Storage)?;
    // During a maintenance window: no alerts, and not counted for uptime (D-65).
    maintenance::mark_report(deps.storage.as_ref(), report, &mut evaluations, now)
        .await
        .map_err(IngestError::Storage)?;
    match deps
        .storage
        .save_report(report, &evaluations)
        .await
        .map_err(IngestError::Storage)?
    {
        SaveOutcome::Stored => {}
        SaveOutcome::NotNewer => {
            return Err(IngestError::NotNewer {
                probe: short_id.to_string(),
                timestamp: report.timestamp,
            });
        }
    }

    // From here on the report is persisted; follow-up failures are logged, not returned.
    if let Err(e) = deps
        .storage
        .record_heartbeat(&report.probe_id, &report.company_id, now)
        .await
    {
        tracing::error!("Failed to update heartbeat for {}: {}", short_id, e);
    }
    if let Err(e) = deps
        .storage
        .record_probe_state(
            &report.probe_id,
            &ReportedState {
                config_revision: report.config_revision.as_deref(),
                config_rejected: report.config_rejected.as_ref(),
                probe_version: report.probe_version.as_deref(),
                update_failed: report.update_failed.as_ref(),
                pins: report.pins.as_ref(),
            },
        )
        .await
    {
        tracing::error!("Failed to record the config of {}: {}", short_id, e);
    }
    if let Some(version) = &report.probe_version
        && record.probe_version.as_ref() != Some(version)
    {
        tracing::info!(
            probe = %short_id,
            from = record.probe_version.as_deref().unwrap_or("?"),
            to = %version,
            "The probe runs another version"
        );
    }
    if let Some(failed) = &report.update_failed
        && record.update_failed.as_ref() != Some(failed)
    {
        tracing::warn!(
            probe = %short_id,
            version = %failed.version,
            "The probe didn't install the update: {}",
            failed.error
        );
    }
    if let Err(e) = commands::record_results(
        deps.storage.as_ref(),
        &report.probe_id,
        &report.command_results,
        now,
    )
    .await
    {
        tracing::error!(
            "Failed to record the command results of {}: {}",
            short_id,
            e
        );
    }
    if let Some(rejected) = &report.config_rejected {
        tracing::warn!(
            probe = %short_id,
            revision = %rejected.revision,
            "The probe refused its config: {}",
            rejected.error
        );
    }
    // Re-arm here, not in the alert task: that task exits early when alerts are
    // disabled, and the state must stay correct for when they are enabled (D-27).
    if let Err(e) = alerts::state::rearm_probe_expiry_alert(
        deps.storage.as_ref(),
        &report.company_id,
        &report.probe_id,
    )
    .await
    {
        tracing::error!("Failed to re-arm expiry alert for {}: {}", short_id, e);
    }
    // Its "stopped reporting" incident, if any, is over (D-67).
    if let Err(e) =
        incidents::on_probe_reporting(deps.storage.as_ref(), &report.probe_id, now).await
    {
        tracing::error!("Failed to close the incident of {}: {}", short_id, e);
    }

    // Alerts run in the background so the probe gets its answer immediately,
    // one report of a probe at a time, in order (D-63).
    alerts::spawn_in_order(
        Arc::clone(deps.storage),
        Arc::clone(deps.notifier),
        report.clone(),
        evaluations,
        probe_checks.clone(),
        now,
    );

    let desired = checks::desired_config(record.interval_secs, &probe_checks);
    let update = update_to_offer(&deps, report).await;
    // After the results above, so a command just done isn't sent again.
    let commands = match commands::to_send(deps.storage.as_ref(), &record, now).await {
        Ok(commands) => commands,
        Err(e) => {
            tracing::error!("Failed to read the commands for {}: {}", short_id, e);
            Vec::new()
        }
    };
    let rotation = super::rotation::for_probe(
        deps.credentials,
        report.pins.as_ref(),
        report.protocol_version,
    );
    Ok(Accepted {
        update,
        commands,
        rotation,
        config: checks::config_to_send(
            desired,
            report.config_revision.as_deref(),
            report.config_rejected.as_ref(),
        ),
        report: report.clone(),
        probe_name: record.name,
    })
}

/// The approved release for a probe that runs another version and hasn't
/// given up on it. A v4 probe (no version) can't update itself.
async fn update_to_offer(deps: &IngestDeps<'_>, report: &StatusReport) -> Option<UpdateOffer> {
    let runs = report.probe_version.as_deref()?;
    let approved = match deps.storage.approved_probe_version().await {
        Ok(approved) => approved?,
        Err(e) => {
            tracing::error!("Can't read the approved probe version: {e:#}");
            return None;
        }
    };
    if !should_offer(&approved.version, runs, report.update_failed.as_ref()) {
        return None;
    }
    let (root, keys) = (
        deps.releases_dir.to_path_buf(),
        Arc::clone(deps.release_keys),
    );
    let version = approved.version.clone();
    let offer = tokio::task::spawn_blocking(move || releases::offer(&root, &version, &keys))
        .await
        .ok()
        .flatten();
    if offer.is_none() {
        tracing::error!(
            version = %approved.version,
            "The approved probe release is missing or no longer checks out; nothing is offered"
        );
    }
    offer
}

/// Offer `approved` to a probe that `runs` another version, unless that's the
/// version it last refused (it would refuse it again: D-49's rule for configs).
pub fn should_offer(approved: &str, runs: &str, failed: Option<&UpdateFailure>) -> bool {
    approved != runs && failed.is_none_or(|f| f.version != approved)
}

/// Drop results that aren't for one of the probe's checks (deleted since,
/// or another probe's: a probe can't report on someone else's check), and
/// repeats of the same check.
fn keep_own_results(report: &mut StatusReport, own: &[CheckRecord]) {
    let own: HashSet<&CheckId> = own.iter().map(|c| &c.id).collect();
    let mut seen = HashSet::new();
    let before = report.results.len();
    report
        .results
        .retain(|r| own.contains(&r.check_id) && seen.insert(r.check_id.clone()));
    if report.results.len() < before {
        tracing::debug!(
            probe = %short_probe_id(&report.probe_id),
            dropped = before - report.results.len(),
            "Ignored results for checks the probe doesn't have"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use x25519_dalek::PublicKey;

    struct Fixture {
        hub_secret: StaticSecret,
        probe_key: SigningKey,
        probe_id: String,
    }

    fn fixture() -> Fixture {
        let probe_key = SigningKey::generate(&mut OsRng);
        let probe_id = hex::encode(probe_key.verifying_key().as_bytes());
        Fixture {
            hub_secret: StaticSecret::random_from_rng(OsRng),
            probe_key,
            probe_id,
        }
    }

    impl Fixture {
        fn report(&self) -> StatusReport {
            StatusReport {
                protocol_version: PROTOCOL_VERSION,
                probe_id: self.probe_id.clone(),
                company_id: "acme".into(),
                hostname: "h".into(),
                site: "s".into(),
                timestamp: Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap(),
                interval_seconds: 60,
                results: vec![],
                config_revision: None,
                config_rejected: None,
                probe_version: None,
                update_failed: None,
                command_results: vec![],
                pins: None,
                signature: None,
            }
        }
        fn sign(&self, report: &mut StatusReport) {
            let bytes = serde_json::to_vec(&report.to_signable()).unwrap();
            report.signature = Some(common::crypto::sign(&bytes, &self.probe_key));
        }
        fn seal(&self, report: &StatusReport) -> Vec<u8> {
            let hub_public = *PublicKey::from(&self.hub_secret).as_bytes();
            common::crypto::seal_for_hub(&serde_json::to_vec(report).unwrap(), &hub_public).unwrap()
        }
        fn authenticate(&self, report: &StatusReport) -> Result<StatusReport, IngestError> {
            authenticate(&self.seal(report), &self.hub_secret)
        }
    }

    #[test]
    fn a_report_signed_by_the_key_it_names_is_authenticated() {
        let f = fixture();
        let mut report = f.report();
        f.sign(&mut report);

        let accepted = f.authenticate(&report).unwrap();

        assert_eq!(accepted.probe_id, f.probe_id);
        assert!(
            accepted.signature.is_some(),
            "signature is kept on the accepted report"
        );
    }

    #[test]
    fn every_rejection_has_its_own_error() {
        let f = fixture();
        let hub_secret = &f.hub_secret;

        assert!(matches!(
            authenticate(&[0; 10], hub_secret),
            Err(IngestError::Undecryptable)
        ));

        let hub_public = *PublicKey::from(hub_secret).as_bytes();
        let supported_but_empty =
            format!(r#"{{"protocol_version":{MIN_SUPPORTED_PROTOCOL_VERSION}}}"#);
        for not_a_report in [&b"not json"[..], supported_but_empty.as_bytes()] {
            let sealed = common::crypto::seal_for_hub(not_a_report, &hub_public).unwrap();
            assert!(matches!(
                authenticate(&sealed, hub_secret),
                Err(IngestError::NotAReport)
            ));
        }

        let mut future = f.report();
        future.protocol_version = PROTOCOL_VERSION + 1;
        f.sign(&mut future);
        assert!(
            matches!(f.authenticate(&future), Err(IngestError::UnsupportedVersion(v)) if v == PROTOCOL_VERSION + 1)
        );

        // An older probe's report is refused for its version, not as malformed.
        let v3 = br#"{"protocol_version":3,"probe_id":"p","resources":[]}"#;
        let sealed = common::crypto::seal_for_hub(v3, &hub_public).unwrap();
        assert!(matches!(
            authenticate(&sealed, hub_secret),
            Err(IngestError::UnsupportedVersion(3))
        ));

        let unsigned = f.report();
        assert!(matches!(
            f.authenticate(&unsigned),
            Err(IngestError::MissingSignature(_))
        ));

        let mut tampered = f.report();
        f.sign(&mut tampered);
        tampered.company_id = "other".into();
        assert!(matches!(
            f.authenticate(&tampered),
            Err(IngestError::BadSignature(_))
        ));

        // Signed by one key, claiming another.
        let mut impostor = f.report();
        impostor.probe_id = "ab".repeat(32);
        f.sign(&mut impostor);
        assert!(matches!(
            f.authenticate(&impostor),
            Err(IngestError::BadSignature(_))
        ));
    }

    #[test]
    fn only_results_of_the_probes_own_checks_are_kept() {
        use common::checks::{CheckResult, CheckSpec, Health};
        let own = |id: &str| CheckRecord {
            id: CheckId(id.into()),
            probe_id: "p".into(),
            company_id: "acme".into(),
            name: id.into(),
            spec: CheckSpec::Cpu,
            enabled: true,
            alert_after: Default::default(),
            limits: Default::default(),
            created_at: Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap(),
            updated_by: None,
        };
        let result = |id: &str| CheckResult {
            check_id: CheckId(id.into()),
            status: Health::Up,
            message: String::new(),
            latency_ms: None,
            metrics: vec![],
        };
        let mut report = fixture().report();
        report.results = vec![
            result("mine"),
            result("theirs"),
            result("mine"),
            result("also"),
        ];
        keep_own_results(&mut report, &[own("mine"), own("also")]);
        let kept: Vec<_> = report.results.iter().map(|r| r.check_id.as_str()).collect();
        assert_eq!(kept, ["mine", "also"]);
    }

    #[test]
    fn timestamp_rejections_explain_themselves() {
        assert!(timestamp_reason(TimestampRejection::InFuture).contains("future"));
        assert!(timestamp_reason(TimestampRejection::TooOld).contains("older"));
    }

    #[test]
    fn the_approved_release_is_offered_until_installed_or_refused() {
        let refused = |version: &str| UpdateFailure {
            version: version.into(),
            error: "rolled back".into(),
        };
        assert!(should_offer("0.3.0", "0.2.0", None));
        assert!(!should_offer("0.3.0", "0.3.0", None), "already runs it");
        assert!(!should_offer("0.3.0", "0.2.0", Some(&refused("0.3.0"))));
        assert!(should_offer("0.3.1", "0.2.0", Some(&refused("0.3.0"))));
    }
}
