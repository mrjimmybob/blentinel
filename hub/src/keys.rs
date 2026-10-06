#![cfg(feature = "ssr")]
//! The hub's credentials as probes pin them (D-47, D-61): its X25519 key,
//! the signing key derived from it, and its TLS certificate's public key.
//!
//! While a rotation announces, the next key and certificate wait in files
//! beside the current ones (`hub_identity.next.key`, `hub_tls_cert.next.pem`,
//! `hub_tls_key.next.pem`). After a key switch, the previous key stays as
//! `hub_identity.prev.key` long enough to open reports probes sealed for it
//! before they switched (their buffer's age limit).

use std::path::{Path, PathBuf};

use common::pins::HubPins;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::config::HubConfig;

/// Where the hub's key and certificate files are: current, next, previous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFiles {
    pub identity: PathBuf,
    pub identity_next: PathBuf,
    pub identity_prev: PathBuf,
    pub tls_cert: PathBuf,
    pub tls_cert_next: PathBuf,
    pub tls_key: PathBuf,
    pub tls_key_next: PathBuf,
    /// The ACME account's key, made when a certificate is first ordered
    /// (D-62), in the data folder.
    pub acme_account: PathBuf,
}

impl KeyFiles {
    pub fn of(config: &HubConfig) -> Self {
        let server = &config.server;
        let identity = server.resolved_identity_key_path();
        let tls_cert = server.resolved_tls_cert_path();
        let tls_key = server.resolved_tls_key_path();
        Self {
            identity_next: sibling(&identity, "next"),
            identity_prev: sibling(&identity, "prev"),
            tls_cert_next: sibling(&tls_cert, "next"),
            tls_key_next: sibling(&tls_key, "next"),
            acme_account: server.resolve_path("hub_acme_account.json"),
            identity,
            tls_cert,
            tls_key,
        }
    }
}

/// `hub_identity.key` → `hub_identity.next.key` (`tag` before the extension).
pub fn sibling(path: &Path, tag: &str) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let name = match path.extension() {
        Some(ext) => format!("{stem}.{tag}.{}", ext.to_string_lossy()),
        None => format!("{stem}.{tag}"),
    };
    path.with_file_name(name)
}

/// The pins a probe holds for a hub with `secret` and, if it serves HTTPS
/// itself, a certificate whose public key hashes to `tls_spki_sha256`.
pub fn pins_of(secret: &StaticSecret, tls_spki_sha256: Option<[u8; 32]>) -> HubPins {
    HubPins {
        hub_key: *PublicKey::from(secret).as_bytes(),
        signing_key: common::crypto::hub_signing_key(secret)
            .verifying_key()
            .to_bytes(),
        tls_spki_sha256,
    }
}

/// What the hub tells probes about its pins, and the old key it still opens.
#[derive(Clone)]
pub struct Credentials {
    /// What enrollment codes carry and probes should hold.
    pub current: HubPins,
    /// The pins a rotation announces (`None`: no rotation waiting).
    pub next: Option<HubPins>,
    /// The key before the last switch, while probes may still have reports
    /// sealed for it.
    pub prev_secret: Option<StaticSecret>,
}

impl Credentials {
    /// No rotation: the current key, and the TLS pin if any.
    pub fn current_only(secret: &StaticSecret, tls_spki_sha256: Option<[u8; 32]>) -> Self {
        Self {
            current: pins_of(secret, tls_spki_sha256),
            next: None,
            prev_secret: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_and_prev_files_sit_beside_the_current_ones() {
        assert_eq!(
            sibling(Path::new("/var/lib/blentinel/hub_identity.key"), "next"),
            Path::new("/var/lib/blentinel/hub_identity.next.key")
        );
        assert_eq!(
            sibling(Path::new("hub_tls_cert.pem"), "prev"),
            Path::new("hub_tls_cert.prev.pem")
        );
        assert_eq!(sibling(Path::new("key"), "next"), Path::new("key.next"));
    }
}
