//! The hub's answer to a report (protocol v3, M2-03, D-48): a [`ReportAck`],
//! signed with the hub's Ed25519 key and sealed for the probe.
//!
//! It is the body of `200` (stored) and of `403 probe_revoked`:
//!
//! ```text
//! seal_for_probe( [ hub Ed25519 signature (64) ][ ack JSON ] )
//! ```
//!
//! - **Signed** with [`crate::crypto::hub_signing_key`], whose public half the
//!   probe got in its enrollment code: nobody else can make an ack, even on
//!   plain HTTP (anyone can *seal* for a probe, its key is public).
//! - **Sealed** for the probe: acks carry its checks (M2-04) and later its
//!   commands (M2-06), which are nobody else's business.
//! - **Bound to one report** by the report's SHA-256, so a captured ack
//!   can't be replayed as the answer to another report.
//! - The signature covers the JSON **as sent**, so a newer hub can add fields
//!   that an older probe ignores without breaking verification.

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::StaticSecret;

use crate::checks::ProbeConfig;
use crate::crypto::{self, CryptoError};

const SIGNATURE_LEN: usize = 64;

/// What the hub tells a probe about one report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAck {
    /// SHA-256 (hex) of the sealed report this answers ([`report_sha256`]).
    pub report_sha256: String,
    /// The hub's clock, so the probe notices clock skew before the hub starts
    /// refusing its reports.
    pub hub_time: DateTime<Utc>,
    /// The hub revoked this probe: stop reporting until it is enrolled again.
    #[serde(default)]
    pub revoked: bool,
    /// What the probe should run from now on: sent only when the report said
    /// it runs another revision (D-49). Covered by the ack's signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ProbeConfig>,
    /// A release the admin approved, offered while the probe runs another
    /// version (protocol v5, D-55). The probe trusts it for its own release
    /// signature, not for this ack's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateOffer>,
}

/// A signed release, as the hub stores it: the probe checks the signature
/// against its compiled-in release keys, then downloads its binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOffer {
    /// The text of `manifest.json`, exactly as signed.
    pub manifest: String,
    /// The text of `manifest.json.sig` (hex).
    pub signature: String,
}

/// Why an ack can't be trusted.
#[derive(Debug, thiserror::Error)]
pub enum AckError {
    #[error("the answer can't be opened: {0}")]
    Crypto(#[from] CryptoError),
    #[error("the answer isn't signed by the hub")]
    BadSignature,
    #[error("the answer isn't an ack")]
    Malformed,
    #[error("the answer is for another report")]
    WrongReport,
}

/// The value of [`ReportAck::report_sha256`] for a sealed report.
pub fn report_sha256(sealed_report: &[u8]) -> String {
    hex::encode(Sha256::digest(sealed_report))
}

/// Hub side: sign `ack` with the hub's key and seal it for `probe_id`.
pub fn seal_ack(
    ack: &ReportAck,
    hub_secret: &StaticSecret,
    probe_id: &str,
) -> Result<Vec<u8>, CryptoError> {
    let json = serde_json::to_vec(ack).expect("an ack always serializes");
    let mut signed = crypto::sign(&json, &crypto::hub_signing_key(hub_secret));
    signed.extend_from_slice(&json);
    crypto::seal_for_probe(&signed, probe_id)
}

/// Probe side: open an ack with the probe's identity key, check it is signed
/// by `hub_signing_key` and answers `sealed_report`.
pub fn open_ack(
    body: &[u8],
    probe_key: &SigningKey,
    hub_signing_key: &[u8; 32],
    sealed_report: &[u8],
) -> Result<ReportAck, AckError> {
    let signed = crypto::open_from_hub(body, probe_key)?;
    let (signature, json) = signed
        .split_at_checked(SIGNATURE_LEN)
        .ok_or(AckError::Malformed)?;
    crypto::verify_with_key(json, signature, hub_signing_key)
        .map_err(|_| AckError::BadSignature)?;
    let ack: ReportAck = serde_json::from_slice(json).map_err(|_| AckError::Malformed)?;
    if ack.report_sha256 != report_sha256(sealed_report) {
        return Err(AckError::WrongReport);
    }
    Ok(ack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    struct Keys {
        hub: StaticSecret,
        hub_signing: [u8; 32],
        probe: SigningKey,
        probe_id: String,
    }

    fn keys() -> Keys {
        let hub = StaticSecret::random_from_rng(OsRng);
        let probe = SigningKey::generate(&mut OsRng);
        Keys {
            hub_signing: crypto::hub_signing_key(&hub).verifying_key().to_bytes(),
            probe_id: hex::encode(probe.verifying_key().as_bytes()),
            hub,
            probe,
        }
    }

    fn ack_for(report: &[u8]) -> ReportAck {
        ReportAck {
            report_sha256: report_sha256(report),
            hub_time: "2026-09-29T10:00:00Z".parse().unwrap(),
            revoked: false,
            config: None,
            update: None,
        }
    }

    #[test]
    fn a_sealed_ack_opens_for_its_probe_and_report() {
        let k = keys();
        let body = seal_ack(&ack_for(b"report"), &k.hub, &k.probe_id).unwrap();
        let ack = open_ack(&body, &k.probe, &k.hub_signing, b"report").unwrap();
        assert_eq!(ack, ack_for(b"report"));
    }

    #[test]
    fn an_ack_for_another_report_is_refused() {
        let k = keys();
        let body = seal_ack(&ack_for(b"old report"), &k.hub, &k.probe_id).unwrap();
        assert!(matches!(
            open_ack(&body, &k.probe, &k.hub_signing, b"new report"),
            Err(AckError::WrongReport)
        ));
    }

    #[test]
    fn an_ack_from_another_hub_is_refused() {
        let k = keys();
        let impostor = StaticSecret::random_from_rng(OsRng);
        let body = seal_ack(&ack_for(b"report"), &impostor, &k.probe_id).unwrap();
        assert!(matches!(
            open_ack(&body, &k.probe, &k.hub_signing, b"report"),
            Err(AckError::BadSignature)
        ));
    }

    #[test]
    fn an_ack_for_another_probe_cant_be_opened() {
        let k = keys();
        let other = keys();
        let body = seal_ack(&ack_for(b"report"), &k.hub, &other.probe_id).unwrap();
        assert!(matches!(
            open_ack(&body, &k.probe, &k.hub_signing, b"report"),
            Err(AckError::Crypto(_))
        ));
    }

    #[test]
    fn fields_from_a_newer_hub_are_ignored_and_the_signature_still_holds() {
        let k = keys();
        let json = format!(
            r#"{{"report_sha256":"{}","hub_time":"2026-09-29T10:00:00Z","revoked":true,"commands":[{{"kind":"run_now"}}]}}"#,
            report_sha256(b"report")
        );
        let mut signed = crypto::sign(json.as_bytes(), &crypto::hub_signing_key(&k.hub));
        signed.extend_from_slice(json.as_bytes());
        let body = crypto::seal_for_probe(&signed, &k.probe_id).unwrap();

        let ack = open_ack(&body, &k.probe, &k.hub_signing, b"report").unwrap();
        assert!(ack.revoked);
    }

    #[test]
    fn a_config_rides_in_the_signed_ack() {
        use crate::checks::{CheckDefinition, CheckId, CheckSpec};
        let k = keys();
        let mut ack = ack_for(b"report");
        ack.config = Some(ProbeConfig {
            revision: "r1".into(),
            interval_secs: 30,
            checks: vec![CheckDefinition {
                id: CheckId("c1".into()),
                name: "CPU".into(),
                spec: CheckSpec::Cpu,
            }],
        });
        let body = seal_ack(&ack, &k.hub, &k.probe_id).unwrap();
        let opened = open_ack(&body, &k.probe, &k.hub_signing, b"report").unwrap();
        assert_eq!(opened.config, ack.config);
    }
}
