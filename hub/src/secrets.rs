#![cfg(feature = "ssr")]
//! Secrets kept in the database (D-66): the mail server's password, chat
//! webhook URLs, webhook signing keys, bot tokens. They are sealed with the
//! hub's **data key** (`hub_data.key`, 32 random bytes beside the other key
//! files, in backups), ChaCha20-Poly1305 with a random nonce each: a copy of
//! the database alone doesn't give them away.
//!
//! The data key is not the hub's identity key: that one changes when it
//! rotates (D-61), and the stored secrets must not.

use std::path::Path;

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

const NONCE_LEN: usize = 12;

/// The key that seals the database's secrets.
pub struct DataKey(ChaCha20Poly1305);

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKey(…)")
    }
}

impl DataKey {
    /// The key in `path`, made (owner-only) the first time.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let (bytes, _) = common::fs::load_or_create_key(path, || {
            let mut bytes = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
            bytes
        })
        .with_context(|| format!("can't read or create {}", path.display()))?;
        Ok(Self::from_bytes(bytes))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(ChaCha20Poly1305::new(Key::from_slice(&bytes)))
    }

    /// `plaintext`, sealed: hex of the nonce and the ciphertext.
    pub fn seal(&self, plaintext: &str) -> Result<String> {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = self
            .0
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow::anyhow!("can't seal a secret"))?;
        let mut out = nonce.to_vec();
        out.extend(sealed);
        Ok(hex::encode(out))
    }

    /// What [`DataKey::seal`] sealed. Fails if it was sealed with another key
    /// (a database restored without its `hub_data.key`) or altered.
    pub fn open(&self, sealed: &str) -> Result<String> {
        let bytes = hex::decode(sealed).context("a stored secret isn't hex")?;
        anyhow::ensure!(bytes.len() > NONCE_LEN, "a stored secret is too short");
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        let plain = self
            .0
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| {
                anyhow::anyhow!(
                    "a stored secret can't be opened: the hub's data key (hub_data.key) isn't the one it was saved with"
                )
            })?;
        String::from_utf8(plain).context("a stored secret isn't text")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_round_trips_and_never_seals_the_same_way_twice() {
        let key = DataKey::from_bytes([7; 32]);
        let a = key.seal("hunter2").unwrap();
        let b = key.seal("hunter2").unwrap();
        assert_ne!(a, b, "a fresh nonce each time");
        assert!(!a.contains("hunter2"));
        assert_eq!(key.open(&a).unwrap(), "hunter2");
        assert_eq!(key.open(&key.seal("").unwrap()).unwrap(), "");
    }

    #[test]
    fn another_key_or_an_altered_secret_is_refused() {
        let sealed = DataKey::from_bytes([7; 32]).seal("token").unwrap();
        assert!(
            DataKey::from_bytes([8; 32])
                .open(&sealed)
                .unwrap_err()
                .to_string()
                .contains("hub_data.key")
        );
        let mut altered = sealed.into_bytes();
        let last = altered.len() - 1;
        altered[last] = if altered[last] == b'0' { b'1' } else { b'0' };
        let altered = String::from_utf8(altered).unwrap();
        assert!(DataKey::from_bytes([7; 32]).open(&altered).is_err());
        assert!(DataKey::from_bytes([7; 32]).open("zz").is_err());
    }

    #[test]
    fn the_key_file_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub_data.key");
        let sealed = DataKey::load_or_create(&path).unwrap().seal("x").unwrap();
        assert_eq!(
            DataKey::load_or_create(&path)
                .unwrap()
                .open(&sealed)
                .unwrap(),
            "x"
        );
    }
}
