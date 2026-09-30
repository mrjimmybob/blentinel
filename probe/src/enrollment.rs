//! Enrollment (M2-01, D-47): `probe enroll <code>` redeems the code from the
//! hub's "Add probe" and writes `enrollment.json` next to `identity.key`.
//! That file is how the probe knows its hub: URL, key, certificate pin, and
//! the company and site the hub put it in. It is written by the probe, not
//! by hand; enrolling again (a new code) replaces it, even while the probe runs.
//! When the hub revokes the probe, the file is set aside
//! (`enrollment.revoked.json`) and the probe waits for a new enrollment.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use common::enrollment::{EnrollRequest, EnrollmentCode, SignedEnrollRequest};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::transport::HubTransport;

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
    /// Hex of the SHA-256 of the hub's TLS certificate, if pinned.
    pub tls_cert_sha256: Option<String>,
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
        self.tls_cert_sha256
            .as_deref()
            .map(hex_array)
            .transpose()
            .context("tls_cert_sha256 in enrollment.json")
    }
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
    enrollment.hub_key()?;
    enrollment.hub_signing_key()?;
    enrollment.tls_pin()?;
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

/// Redeem `code` for this probe's identity and save the enrollment in `dir`.
pub async fn enroll(code: &str, key: &SigningKey, dir: &Path) -> Result<Enrollment> {
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

    let transport = HubTransport::new(&code.hub_url, code.tls_cert_sha256)?;
    let answer = transport
        .enroll(sealed)
        .await
        .with_context(|| format!("enrollment with {} failed", code.hub_url))?;

    let enrollment = Enrollment {
        hub_url: code.hub_url,
        hub_public_key: hex::encode(code.hub_key),
        hub_signing_key: hex::encode(code.hub_signing_key),
        tls_cert_sha256: code.tls_cert_sha256.map(hex::encode),
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0u8; 16 * 1024];
                let _ = socket.read(&mut request).await;
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
            tls_cert_sha256: None,
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

        let enrollment = enroll(&code_for(&url), &key, dir.path()).await.unwrap();

        assert_eq!(enrollment.company_name, "ACME");
        assert_eq!(enrollment.hub_url, url);
        assert_eq!(enrollment.hub_key().unwrap(), [5; 32]);
        assert_eq!(load(dir.path()).unwrap(), Some(enrollment));
    }

    #[tokio::test]
    async fn a_refused_code_leaves_nothing_behind() {
        let body =
            r#"{"error":{"code":"invalid_code","message":"invalid or expired enrollment code"}}"#;
        let url = fake_hub(403, body.to_string()).await;
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);

        let error = enroll(&code_for(&url), &key, dir.path()).await.unwrap_err();

        assert!(
            format!("{error:#}").contains("invalid or expired"),
            "{error:#}"
        );
        assert_eq!(load(dir.path()).unwrap(), None);
    }

    #[tokio::test]
    async fn a_mistyped_code_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let error = enroll("BLT1-ABC", &key, dir.path()).await.unwrap_err();
        assert!(error.to_string().contains("damaged"), "{error}");
    }
}
