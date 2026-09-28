//! Accepting a report from a probe: every rule between "bytes arrived" and
//! "report stored", with no HTTP in sight (ENGINEERING.md §2.2).
//!
//! Order matters and is part of the security design:
//! 1. open the seal (only this hub can)
//! 2. parse, check the protocol version
//! 3. report must be signed by the key it names (D-25), and that key must be
//!    an enrolled, active probe; the probe's record decides its company and
//!    site, whatever the report says (D-17, D-47)
//! 4. timestamp within the accepted window, checked *after* the signature so
//!    forged traffic learns nothing about it (D-28)
//! 5. store only if newer than the probe's latest report, atomically (D-28)
//! 6. heartbeat, re-arm the offline alert (D-27), evaluate alerts in the background

use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::models::StatusReport;
use common::protocol::{
    self, MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION, TimestampRejection,
};
use tokio::sync::RwLock;
use x25519_dalek::StaticSecret;

use crate::config::HubConfig;
use crate::model::short_probe_id;
use crate::service::alerts::{self, Notifier};
use crate::storage::{SaveOutcome, Storage};

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
    pub report: StatusReport,
    /// The probe's name (from its record).
    pub probe_name: String,
}

/// What the ingest flow needs from the hub.
pub struct IngestDeps<'a> {
    pub storage: &'a Arc<dyn Storage>,
    pub config: &'a Arc<RwLock<HubConfig>>,
    pub hub_secret: &'a StaticSecret,
    pub notifier: &'a Arc<dyn Notifier>,
}

/// Steps 1–3a: open, parse, check the version, and verify the signature
/// against the key the report names. Pure (no I/O); the probe record is
/// checked next, in [`ingest_report`].
pub fn authenticate(sealed: &[u8], hub_secret: &StaticSecret) -> Result<StatusReport, IngestError> {
    let cleartext = common::crypto::open_from_probe(sealed, hub_secret)
        .map_err(|_| IngestError::Undecryptable)?;
    let mut report: StatusReport =
        serde_json::from_slice(&cleartext).map_err(|_| IngestError::NotAReport)?;

    let version = report.effective_protocol_version();
    if !(MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&version) {
        return Err(IngestError::UnsupportedVersion(version));
    }

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
    let mut report = authenticate(sealed, deps.hub_secret)?;
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
    report.company_id = record.company_id;
    report.site = record.site_id;
    let accepted = Accepted {
        report,
        probe_name: record.name,
    };
    let report = &accepted.report;
    let short_id = short_id.as_str();

    protocol::check_report_timestamp(report.timestamp, now).map_err(|rejection| {
        IngestError::TimestampRejected {
            probe: short_id.to_string(),
            timestamp: report.timestamp,
            rejection,
        }
    })?;

    match deps
        .storage
        .save_report(report)
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

    // Alerts run in the background so the probe gets its answer immediately.
    let storage = Arc::clone(deps.storage);
    let notifier = Arc::clone(deps.notifier);
    let config = Arc::clone(deps.config);
    let for_alerts = report.clone();
    tokio::spawn(async move {
        if let Err(e) = alerts::evaluate_alerts_for_report(
            storage.as_ref(),
            notifier.as_ref(),
            &config,
            &for_alerts,
            now,
        )
        .await
        {
            tracing::error!("Failed to evaluate alerts: {}", e);
        }
    });

    Ok(accepted)
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
                probe_id: self.probe_id.clone(),
                company_id: "acme".into(),
                hostname: "h".into(),
                site: "s".into(),
                timestamp: Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap(),
                interval_seconds: 60,
                resources: vec![],
                signature: None,
                ephemeral_public_key: None,
                protocol_version: Some(PROTOCOL_VERSION),
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
        let not_a_report = common::crypto::seal_for_hub(b"{}", &hub_public).unwrap();
        assert!(matches!(
            authenticate(&not_a_report, hub_secret),
            Err(IngestError::NotAReport)
        ));

        let mut future = f.report();
        future.protocol_version = Some(PROTOCOL_VERSION + 1);
        f.sign(&mut future);
        assert!(
            matches!(f.authenticate(&future), Err(IngestError::UnsupportedVersion(v)) if v == PROTOCOL_VERSION + 1)
        );

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
    fn timestamp_rejections_explain_themselves() {
        assert!(timestamp_reason(TimestampRejection::InFuture).contains("future"));
        assert!(timestamp_reason(TimestampRejection::TooOld).contains("older"));
    }
}
