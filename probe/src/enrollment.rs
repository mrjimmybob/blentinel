//! Enrollment (M2-01, D-47): `probe enroll <code>` redeems the code from the
//! hub's "Add probe" and writes `enrollment.json` next to `identity.key`.
//! That file is how the probe knows its hub: URL, key, certificate pin, and
//! the company and site the hub put it in. It is written by the probe, not
//! by hand; enrolling again (a new code) replaces it, even while the probe runs.
//! When the hub revokes the probe, the file is set aside
//! (`enrollment.revoked.json`), and so is its identity
//! (`identity.revoked.key`): the probe waits for a new enrollment, which
//! makes it a new identity (D-116).
//!
//! The hub changes its pins by announcing the next ones in its signed
//! answers (D-61): they are kept in `next` until the hub says it switched,
//! then replace the current ones ([`Enrollment::rotated`]).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use common::enrollment::{EnrollRequest, EnrollmentCode, SignedEnrollRequest};
use common::models::PinDigests;
use common::pins::{HubPins, Rotation, WirePins};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::transport::{HubTransport, TransportError};

pub const FILE_NAME: &str = "enrollment.json";
const REVOKED_FILE_NAME: &str = "enrollment.revoked.json";

/// What the probe keeps from its enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrollment {
    pub hub_url: String,
    /// Hex of the hub's X25519 key: reports are sealed for it.
    pub hub_public_key: String,
    /// Hex of the hub's Ed25519 key: its answers are signed with it (D-48).
    pub hub_signing_key: String,
    /// Hex of the SHA-256 of the public key of the hub's TLS certificate,
    /// if pinned (D-61).
    pub tls_spki_sha256: Option<String>,
    /// The pins the hub announced, held until it switches to them (D-61).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<WirePins>,
    pub probe_name: String,
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
    pub enrolled_at: String,
}

impl Enrollment {
    pub fn hub_key(&self) -> Result<[u8; 32]> {
        hex_array(&self.hub_public_key).context("hub_public_key in enrollment.json")
    }

    pub fn hub_signing_key(&self) -> Result<[u8; 32]> {
        hex_array(&self.hub_signing_key).context("hub_signing_key in enrollment.json")
    }

    pub fn tls_pin(&self) -> Result<Option<[u8; 32]>> {
        self.tls_spki_sha256
            .as_deref()
            .map(hex_array)
            .transpose()
            .context("tls_spki_sha256 in enrollment.json")
    }

    /// The pins the probe trusts now.
    pub fn pins(&self) -> Result<HubPins> {
        Ok(HubPins {
            hub_key: self.hub_key()?,
            signing_key: self.hub_signing_key()?,
            tls_spki_sha256: self.tls_pin()?,
        })
    }

    /// The pins the hub announced, if any.
    pub fn next_pins(&self) -> Result<Option<HubPins>> {
        self.next
            .as_ref()
            .map(HubPins::try_from)
            .transpose()
            .context("next in enrollment.json")
    }

    /// The certificates (public keys) the probe accepts: current, then next.
    pub fn tls_pins(&self) -> Result<Vec<[u8; 32]>> {
        let next = self.next_pins()?.and_then(|p| p.tls_spki_sha256);
        Ok(self.tls_pin()?.into_iter().chain(next).collect())
    }

    /// The keys the hub's answers may be signed with: current, then next.
    pub fn signing_keys(&self) -> Result<Vec<[u8; 32]>> {
        let next = self.next_pins()?.map(|p| p.signing_key);
        Ok(std::iter::once(self.hub_signing_key()?)
            .chain(next)
            .collect())
    }

    /// What the probe tells the hub it holds (protocol v7).
    pub fn pin_digests(&self) -> Result<PinDigests> {
        Ok(PinDigests {
            current: self.pins()?.digest(),
            next: self.next_pins()?.map(|p| p.digest()),
        })
    }

    /// This enrollment after a rotation from a **trusted** ack (one signed
    /// with a key it pins); `None` if nothing changes. A probe that doesn't
    /// pin a certificate keeps not pinning one.
    pub fn rotated(&self, rotation: &Rotation) -> Result<Option<(Self, RotationStep)>> {
        let mut offered = HubPins::try_from(&rotation.pins).context("the hub's next pins")?;
        if self.tls_spki_sha256.is_none() {
            offered = offered.without_tls();
        }
        let current = self.pins()?;
        let mut rotated = self.clone();
        if !rotation.switched {
            if self.next_pins()? == Some(offered) || current == offered {
                return Ok(None);
            }
            rotated.next = Some(WirePins::from(&offered));
            return Ok(Some((rotated, RotationStep::Announced)));
        }
        if current == offered {
            if self.next.is_none() {
                return Ok(None);
            }
            rotated.next = None; // told it switched to what it already has
            let step = RotationStep::Switched {
                hub_key_changed: false,
            };
            return Ok(Some((rotated, step)));
        }
        let wire = WirePins::from(&offered);
        rotated.hub_public_key = wire.hub_key;
        rotated.hub_signing_key = wire.signing_key;
        rotated.tls_spki_sha256 = wire.tls_spki_sha256;
        rotated.next = None;
        let hub_key_changed = offered.hub_key != current.hub_key;
        Ok(Some((rotated, RotationStep::Switched { hub_key_changed })))
    }
}

/// What a rotation did to the enrollment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationStep {
    /// The hub's next pins are held, beside the current ones.
    Announced,
    /// The hub switched: its next pins are now the only ones.
    Switched { hub_key_changed: bool },
}

/// Apply a rotation from a trusted ack to the enrollment in `dir`.
pub fn follow_rotation(
    dir: &Path,
    current: &Enrollment,
    rotation: &Rotation,
) -> Result<Option<(Enrollment, RotationStep)>> {
    let Some((rotated, step)) = current.rotated(rotation)? else {
        return Ok(None);
    };
    save(dir, &rotated)?;
    Ok(Some((rotated, step)))
}

fn hex_array(text: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(text.trim())?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| anyhow::anyhow!("expected 32 bytes"))
}

pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// The enrollment in `dir`; `None` if the probe was never enrolled.
pub fn load(dir: &Path) -> Result<Option<Enrollment>> {
    let path = path_in(dir);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("can't read {}", path.display()))?;
    let enrollment: Enrollment = serde_json::from_str(&text)
        .with_context(|| format!("{} is damaged; enroll again", path.display()))?;
    enrollment.pins()?;
    enrollment.next_pins()?;
    Ok(Some(enrollment))
}

/// Write it atomically (a running probe may be reading the file).
fn save(dir: &Path, enrollment: &Enrollment) -> Result<()> {
    let path = path_in(dir);
    common::fs::write_atomic(&path, serde_json::to_string_pretty(enrollment)?)
        .with_context(|| format!("can't write {}", path.display()))
}

/// The hub revoked this probe: move the enrollment aside (kept for reference)
/// so the probe waits for a new one.
pub fn set_aside_revoked(dir: &Path) -> Result<()> {
    let revoked = dir.join(REVOKED_FILE_NAME);
    std::fs::rename(path_in(dir), &revoked)
        .with_context(|| format!("can't move {} aside", path_in(dir).display()))
}

/// Redeem `code` for the identity in `dir` (made if there is none) and save
/// the enrollment there (D-116):
///
/// - `409 probe_revoked`: the hub deleted this identity, which never comes
///   back. It is set aside, and a new one tries once more with the same code
///   (the refusal left it unused).
/// - `409 probe_in_another_company`: this identity is active in another
///   company on that hub. The probe stops: the old company still expects it,
///   so it must be deleted there first.
pub async fn enroll(code: &str, dir: &Path) -> Result<Enrollment> {
    let (key, _) = crate::identity::load_or_create_key_in(dir)?;
    match enroll_as(code, &key, dir).await {
        Err(e) if refused_with(&e, "probe_revoked") => {
            eprintln!("The hub deleted this probe's identity: making a new one and trying again.");
            crate::identity::set_aside_revoked(dir)?;
            let (key, _) = crate::identity::load_or_create_key_in(dir)?;
            enroll_as(code, &key, dir).await
        }
        Err(e) if refused_with(&e, "probe_in_another_company") => Err(e.context(
            "this probe is active in another company on that hub: delete it on its company's \
             page there first, then enroll it again",
        )),
        other => other,
    }
}

/// Whether `error` is the hub refusing with `code`.
fn refused_with(error: &anyhow::Error, code: &str) -> bool {
    matches!(
        error.downcast_ref::<TransportError>(),
        Some(TransportError::Refused(_, body)) if body.error.code == code
    )
}

/// Redeem `code` for `key` and save the enrollment in `dir`.
async fn enroll_as(code: &str, key: &SigningKey, dir: &Path) -> Result<Enrollment> {
    let code = EnrollmentCode::decode(code).map_err(|e| anyhow::anyhow!("{e}"))?;
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let request = EnrollRequest {
        token_id: hex::encode(code.token_id),
        secret: hex::encode(code.secret),
        probe_id: hex::encode(key.verifying_key().as_bytes()),
        hostname,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        probe_version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now().to_rfc3339(),
    };
    let signature = hex::encode(common::crypto::sign(&serde_json::to_vec(&request)?, key));
    let signed = serde_json::to_vec(&SignedEnrollRequest { request, signature })?;
    let sealed = common::crypto::seal_for_hub(&signed, &code.hub_key)?;

    let pins = code.tls_spki_sha256.into_iter().collect();
    let transport = HubTransport::new(&code.hub_url, pins)?;
    let answer = transport
        .enroll(sealed)
        .await
        .with_context(|| format!("enrollment with {} failed", code.hub_url))?;

    let enrollment = Enrollment {
        hub_url: code.hub_url,
        hub_public_key: hex::encode(code.hub_key),
        hub_signing_key: hex::encode(code.hub_signing_key),
        tls_spki_sha256: code.tls_spki_sha256.map(hex::encode),
        next: None,
        probe_name: answer.name,
        company_id: answer.company_id,
        company_name: answer.company_name,
        site_id: answer.site_id,
        site_name: answer.site_name,
        enrolled_at: Utc::now().to_rfc3339(),
    };
    save(dir, &enrollment)?;
    Ok(enrollment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A stand-in hub whose `/api/enroll` answers with `body` and `status`.
    async fn fake_hub(status: u16, body: String) -> String {
        fake_hub_answering(vec![(status, body)]).await
    }

    /// A stand-in hub answering with each of `answers` in turn, then the last
    /// one again.
    async fn fake_hub_answering(answers: Vec<(u16, String)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut answers = answers.into_iter().peekable();
            let mut last = None;
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0u8; 16 * 1024];
                let _ = socket.read(&mut request).await;
                if let Some(next) = answers.next() {
                    last = Some(next);
                }
                let (status, body) = last.clone().expect("an answer");
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        url
    }

    fn code_for(url: &str) -> String {
        EnrollmentCode {
            hub_url: url.to_string(),
            hub_key: [5; 32],
            hub_signing_key: [6; 32],
            tls_spki_sha256: None,
            token_id: [1; 8],
            secret: [2; 16],
        }
        .encode()
    }

    #[tokio::test]
    async fn enrolling_writes_the_hub_and_the_placement() {
        let answer = serde_json::json!({
            "probe_id": "x", "name": "Front desk", "company_id": "acme",
            "company_name": "ACME", "site_id": "hq", "site_name": "HQ",
        });
        let url = fake_hub(200, answer.to_string()).await;
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);

        let enrollment = enroll_as(&code_for(&url), &key, dir.path()).await.unwrap();

        assert_eq!(enrollment.company_name, "ACME");
        assert_eq!(enrollment.hub_url, url);
        assert_eq!(enrollment.hub_key().unwrap(), [5; 32]);
        assert_eq!(load(dir.path()).unwrap(), Some(enrollment));
    }

    fn placed() -> String {
        serde_json::json!({
            "probe_id": "x", "name": "Front desk", "company_id": "acme",
            "company_name": "ACME", "site_id": "hq", "site_name": "HQ",
        })
        .to_string()
    }

    fn refusal(code: &str) -> String {
        format!(r#"{{"error":{{"code":"{code}","message":"refused"}}}}"#)
    }

    fn key_in(dir: &Path, file: &str) -> Vec<u8> {
        std::fs::read(dir.join(file)).unwrap()
    }

    /// D-116: a deleted identity never comes back; the probe sets it aside
    /// and tries once more as a new one, with the same code.
    #[tokio::test]
    async fn a_revoked_identity_is_set_aside_and_a_new_one_enrolls() {
        let url = fake_hub_answering(vec![(409, refusal("probe_revoked")), (200, placed())]).await;
        let dir = tempfile::tempdir().unwrap();
        crate::identity::load_or_create_key_in(dir.path()).unwrap();
        let old = key_in(dir.path(), crate::identity::KEY_FILE);

        let enrollment = enroll(&code_for(&url), dir.path()).await.unwrap();

        assert_eq!(enrollment.company_name, "ACME");
        assert_eq!(key_in(dir.path(), crate::identity::REVOKED_KEY_FILE), old);
        assert_ne!(
            key_in(dir.path(), crate::identity::KEY_FILE),
            old,
            "a new identity"
        );
        assert_eq!(load(dir.path()).unwrap(), Some(enrollment));
    }

    /// Only once: a second refusal is the answer.
    #[tokio::test]
    async fn a_new_identity_refused_again_is_not_tried_a_third_time() {
        let url = fake_hub(409, refusal("probe_revoked")).await;
        let dir = tempfile::tempdir().unwrap();

        let error = enroll(&code_for(&url), dir.path()).await.unwrap_err();

        assert!(format!("{error:#}").contains("probe_revoked"), "{error:#}");
        assert_eq!(load(dir.path()).unwrap(), None);
    }

    /// The old company still expects it: the probe keeps its identity and says
    /// what to do.
    #[tokio::test]
    async fn a_probe_active_in_another_company_keeps_its_identity_and_says_so() {
        let url = fake_hub(409, refusal("probe_in_another_company")).await;
        let dir = tempfile::tempdir().unwrap();
        crate::identity::load_or_create_key_in(dir.path()).unwrap();
        let old = key_in(dir.path(), crate::identity::KEY_FILE);

        let error = enroll(&code_for(&url), dir.path()).await.unwrap_err();

        assert!(
            format!("{error:#}").contains("delete it on its company's page"),
            "{error:#}"
        );
        assert_eq!(key_in(dir.path(), crate::identity::KEY_FILE), old);
        assert!(!dir.path().join(crate::identity::REVOKED_KEY_FILE).exists());
        assert_eq!(load(dir.path()).unwrap(), None);
    }

    #[tokio::test]
    async fn a_refused_code_leaves_nothing_behind() {
        let body =
            r#"{"error":{"code":"invalid_code","message":"invalid or expired enrollment code"}}"#;
        let url = fake_hub(403, body.to_string()).await;
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);

        let error = enroll_as(&code_for(&url), &key, dir.path())
            .await
            .unwrap_err();

        assert!(
            format!("{error:#}").contains("invalid or expired"),
            "{error:#}"
        );
        assert_eq!(load(dir.path()).unwrap(), None);
    }

    fn enrolled(tls: bool) -> Enrollment {
        Enrollment {
            hub_url: "https://hub.example.com:3443".into(),
            hub_public_key: hex::encode([1; 32]),
            hub_signing_key: hex::encode([2; 32]),
            tls_spki_sha256: tls.then(|| hex::encode([3; 32])),
            next: None,
            probe_name: "pc".into(),
            company_id: "acme".into(),
            company_name: "ACME".into(),
            site_id: "hq".into(),
            site_name: "HQ".into(),
            enrolled_at: "2026-10-06T00:00:00Z".into(),
        }
    }

    fn rotation(key: u8, switched: bool) -> Rotation {
        Rotation {
            pins: WirePins::from(&HubPins {
                hub_key: [key; 32],
                signing_key: [key + 1; 32],
                tls_spki_sha256: Some([key + 2; 32]),
            }),
            switched,
        }
    }

    #[test]
    fn announced_pins_are_held_beside_the_current_ones() {
        let before = enrolled(true);
        let (after, step) = before.rotated(&rotation(7, false)).unwrap().unwrap();
        assert_eq!(step, RotationStep::Announced);
        assert_eq!(
            after.pins().unwrap(),
            before.pins().unwrap(),
            "still the current ones"
        );
        assert_eq!(after.tls_pins().unwrap(), vec![[3; 32], [9; 32]]);
        assert_eq!(after.signing_keys().unwrap(), vec![[2; 32], [8; 32]]);
        let digests = after.pin_digests().unwrap();
        assert_eq!(
            digests.next,
            Some(after.next_pins().unwrap().unwrap().digest())
        );
        assert!(
            after.rotated(&rotation(7, false)).unwrap().is_none(),
            "once"
        );
    }

    #[test]
    fn a_switch_makes_the_next_pins_the_only_ones() {
        let (held, _) = enrolled(true)
            .rotated(&rotation(7, false))
            .unwrap()
            .unwrap();
        let (after, step) = held.rotated(&rotation(7, true)).unwrap().unwrap();
        assert_eq!(
            step,
            RotationStep::Switched {
                hub_key_changed: true
            }
        );
        assert_eq!(after.hub_key().unwrap(), [7; 32]);
        assert_eq!(after.tls_pins().unwrap(), vec![[9; 32]]);
        assert_eq!(after.next, None);
        assert!(
            after.rotated(&rotation(7, true)).unwrap().is_none(),
            "already switched"
        );
    }

    #[test]
    fn a_probe_without_a_certificate_pin_doesnt_start_pinning_one() {
        let (after, _) = enrolled(false)
            .rotated(&rotation(7, true))
            .unwrap()
            .unwrap();
        assert_eq!(after.tls_pin().unwrap(), None);
        assert_eq!(after.hub_key().unwrap(), [7; 32]);
    }

    #[tokio::test]
    async fn a_mistyped_code_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let error = enroll_as("BLT1-ABC", &key, dir.path()).await.unwrap_err();
        assert!(error.to_string().contains("damaged"), "{error}");
    }
}
