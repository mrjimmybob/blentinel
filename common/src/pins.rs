//! What a probe pins of its hub, and how pins change (M2-13, D-61).
//!
//! A probe trusts its hub by three values from its enrollment code:
//!
//! - the hub's **X25519 key**, which reports are sealed for;
//! - the hub's **Ed25519 signing key**, which signs its answers (D-48);
//! - the **SHA-256 of the public key (SPKI) of the hub's TLS certificate**,
//!   if it connects over HTTPS to the hub's own certificate. The public key,
//!   not the whole certificate (refines D-47): a certificate renewed with the
//!   same key (as Let's Encrypt can do) keeps the same pin.
//!
//! The hub changes them by announcing the next ones in its signed answers;
//! the probe keeps both until the hub says it switched. Reports say which
//! pins the probe holds, as [`HubPins::digest`]s
//! ([`crate::models::PinDigests`]), so the hub knows who is ready.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The values a probe pins of its hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubPins {
    pub hub_key: [u8; 32],
    pub signing_key: [u8; 32],
    /// `None`: the probe doesn't pin a certificate (plain HTTP, or a proxy
    /// with a public certificate).
    pub tls_spki_sha256: Option<[u8; 32]>,
}

impl HubPins {
    /// One value for comparing pin sets: SHA-256 (hex) of the keys and the
    /// TLS pin, in that order, with a marker byte for a missing TLS pin.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.hub_key);
        hasher.update(self.signing_key);
        match &self.tls_spki_sha256 {
            Some(pin) => {
                hasher.update([1]);
                hasher.update(pin);
            }
            None => hasher.update([0]),
        }
        hex::encode(hasher.finalize())
    }

    /// The same pins for a probe that doesn't pin a certificate.
    pub fn without_tls(self) -> Self {
        Self {
            tls_spki_sha256: None,
            ..self
        }
    }

    /// Does `digest` (from a probe) name these pins, with or without the TLS pin?
    pub fn matches(&self, digest: &str) -> bool {
        digest == self.digest() || digest == self.without_tls().digest()
    }
}

/// [`HubPins`] on the wire (hex), in the hub's answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePins {
    pub hub_key: String,
    pub signing_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_spki_sha256: Option<String>,
}

impl From<&HubPins> for WirePins {
    fn from(pins: &HubPins) -> Self {
        Self {
            hub_key: hex::encode(pins.hub_key),
            signing_key: hex::encode(pins.signing_key),
            tls_spki_sha256: pins.tls_spki_sha256.map(hex::encode),
        }
    }
}

impl TryFrom<&WirePins> for HubPins {
    type Error = PinError;

    fn try_from(wire: &WirePins) -> Result<Self, PinError> {
        Ok(Self {
            hub_key: from_hex(&wire.hub_key)?,
            signing_key: from_hex(&wire.signing_key)?,
            tls_spki_sha256: wire.tls_spki_sha256.as_deref().map(from_hex).transpose()?,
        })
    }
}

/// A hub's answer about its pins (protocol v7), signed with the ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rotation {
    /// The pins the hub will switch to (or, with `switched`, uses now).
    pub pins: WirePins,
    /// The hub has switched to `pins`: drop the old ones.
    #[serde(default)]
    pub switched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PinError {
    #[error("a pin isn't 32 bytes of hex")]
    BadHex,
    #[error("the certificate can't be read: {0}")]
    BadCertificate(String),
}

fn from_hex(text: &str) -> Result<[u8; 32], PinError> {
    let bytes = hex::decode(text.trim()).map_err(|_| PinError::BadHex)?;
    bytes.try_into().map_err(|_| PinError::BadHex)
}

/// SHA-256 of the SubjectPublicKeyInfo of a DER certificate: the TLS pin.
pub fn spki_sha256(cert_der: &[u8]) -> Result<[u8; 32], PinError> {
    let der = rustls_pki_types::CertificateDer::from(cert_der);
    let cert = webpki::EndEntityCert::try_from(&der)
        .map_err(|e| PinError::BadCertificate(format!("{e:?}")))?;
    Ok(Sha256::digest(cert.subject_public_key_info().as_ref()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(tls: Option<[u8; 32]>) -> HubPins {
        HubPins {
            hub_key: [1; 32],
            signing_key: [2; 32],
            tls_spki_sha256: tls,
        }
    }

    #[test]
    fn digests_differ_by_every_pin() {
        let base = pins(Some([3; 32]));
        let digests = [
            base.digest(),
            pins(Some([4; 32])).digest(),
            pins(None).digest(),
            HubPins {
                hub_key: [9; 32],
                ..base
            }
            .digest(),
            HubPins {
                signing_key: [9; 32],
                ..base
            }
            .digest(),
        ];
        let distinct: std::collections::HashSet<_> = digests.iter().collect();
        assert_eq!(distinct.len(), digests.len());
        assert_eq!(base.digest(), pins(Some([3; 32])).digest(), "stable");
    }

    #[test]
    fn a_probe_without_a_tls_pin_matches_too() {
        let hub = pins(Some([3; 32]));
        assert!(hub.matches(&hub.digest()));
        assert!(hub.matches(&pins(None).digest()));
        assert!(!hub.matches(&pins(Some([4; 32])).digest()));
    }

    #[test]
    fn wire_pins_round_trip_and_bad_hex_is_refused() {
        let hub = pins(Some([3; 32]));
        assert_eq!(HubPins::try_from(&WirePins::from(&hub)), Ok(hub));
        let mut bad = WirePins::from(&hub);
        bad.hub_key = "abc".into();
        assert_eq!(HubPins::try_from(&bad), Err(PinError::BadHex));
    }

    #[test]
    fn the_tls_pin_is_the_public_key_so_a_renewal_with_the_same_key_keeps_it() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = |name: &str| {
            rcgen::CertificateParams::new(vec![name.to_string()])
                .unwrap()
                .self_signed(&key)
                .unwrap()
        };
        let (first, renewed) = (cert("hub.example.com"), cert("hub.example.org"));
        assert_ne!(first.der().as_ref(), renewed.der().as_ref());
        assert_eq!(
            spki_sha256(first.der()).unwrap(),
            spki_sha256(renewed.der()).unwrap()
        );
        let other = rcgen::generate_simple_self_signed(vec!["hub".into()]).unwrap();
        assert_ne!(
            spki_sha256(other.cert.der()).unwrap(),
            spki_sha256(first.der()).unwrap()
        );
        assert!(spki_sha256(b"not a certificate").is_err());
    }
}
