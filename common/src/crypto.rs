//! Probe ↔ hub payload protection, shared by both sides so there is exactly
//! one implementation of the wire format.
//!
//! Wire format of a sealed payload (probe → hub):
//!
//! ```text
//! [ probe ephemeral X25519 public key (32) ][ nonce (12) ][ ChaCha20-Poly1305 ciphertext + tag ]
//! ```
//!
//! The symmetric key is the X25519 shared secret between the probe's ephemeral
//! key and the hub's static key, so only the hub can open a sealed payload.
//! Authenticity of the *sender* comes from the separate Ed25519 signature
//! ([`sign`] / [`verify`]) over the canonical report, not from the seal.

use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

const PUBLIC_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// Smallest possible sealed payload: key + nonce (the ciphertext may be empty
/// only in theory; the AEAD tag check rejects it anyway).
const MIN_SEALED_LEN: usize = PUBLIC_KEY_LEN + NONCE_LEN;

/// Errors from sealing, opening, signing or verifying payloads.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("sealed payload too short ({0} bytes, need at least {MIN_SEALED_LEN})")]
    PayloadTooShort(usize),
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed (wrong key or tampered payload)")]
    Decrypt,
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
}

/// Encrypt `data` so that only the holder of `hub_public_key`'s secret can read it.
/// A fresh ephemeral key and nonce are generated for every call.
pub fn seal_for_hub(
    data: &[u8],
    hub_public_key: &[u8; PUBLIC_KEY_LEN],
) -> Result<Vec<u8>, CryptoError> {
    let probe_secret = EphemeralSecret::random_from_rng(OsRng);
    let probe_public = PublicKey::from(&probe_secret);
    let shared = probe_secret.diffie_hellman(&PublicKey::from(*hub_public_key));

    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = ChaCha20Poly1305::new(Key::from_slice(shared.as_bytes()))
        .encrypt(Nonce::from_slice(&nonce), data)
        .map_err(|_| CryptoError::Encrypt)?;

    let mut sealed = Vec::with_capacity(MIN_SEALED_LEN + ciphertext.len());
    sealed.extend_from_slice(probe_public.as_bytes());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

/// Decrypt a payload produced by [`seal_for_hub`] using the hub's static secret.
pub fn open_from_probe(sealed: &[u8], hub_secret: &StaticSecret) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < MIN_SEALED_LEN {
        return Err(CryptoError::PayloadTooShort(sealed.len()));
    }
    let (probe_public, rest) = sealed.split_at(PUBLIC_KEY_LEN);
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);

    let probe_public: [u8; PUBLIC_KEY_LEN] = probe_public
        .try_into()
        .map_err(|_| CryptoError::PayloadTooShort(sealed.len()))?;
    let shared = hub_secret.diffie_hellman(&PublicKey::from(probe_public));

    ChaCha20Poly1305::new(Key::from_slice(shared.as_bytes()))
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Decrypt)
}

/// Sign `data` with the probe's Ed25519 identity key.
pub fn sign(data: &[u8], key: &SigningKey) -> Vec<u8> {
    key.sign(data).to_bytes().to_vec()
}

/// Verify an Ed25519 signature. `public_key_hex` is the probe ID (hex of its verifying key).
pub fn verify(data: &[u8], signature: &[u8], public_key_hex: &str) -> Result<(), CryptoError> {
    let key_bytes: [u8; PUBLIC_KEY_LEN] = hex::decode(public_key_hex)
        .map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))?
        .try_into()
        .map_err(|_| CryptoError::InvalidPublicKey("expected 32 bytes".into()))?;
    let key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))?;
    let signature = Signature::from_slice(signature)
        .map_err(|e| CryptoError::InvalidSignature(e.to_string()))?;
    key.verify(data, &signature)
        .map_err(|e| CryptoError::InvalidSignature(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub_keys() -> (StaticSecret, [u8; 32]) {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = *PublicKey::from(&secret).as_bytes();
        (secret, public)
    }

    #[test]
    fn seal_then_open_round_trips() {
        let (secret, public) = hub_keys();
        let sealed = seal_for_hub(b"report", &public).unwrap();
        assert_eq!(open_from_probe(&sealed, &secret).unwrap(), b"report");
    }

    #[test]
    fn each_seal_uses_fresh_key_and_nonce() {
        let (_, public) = hub_keys();
        let a = seal_for_hub(b"same", &public).unwrap();
        let b = seal_for_hub(b"same", &public).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn open_rejects_tampered_ciphertext() {
        let (secret, public) = hub_keys();
        let mut sealed = seal_for_hub(b"report", &public).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(matches!(
            open_from_probe(&sealed, &secret),
            Err(CryptoError::Decrypt)
        ));
    }

    #[test]
    fn open_rejects_payload_for_another_hub() {
        let (_, public) = hub_keys();
        let (other_secret, _) = hub_keys();
        let sealed = seal_for_hub(b"report", &public).unwrap();
        assert!(matches!(
            open_from_probe(&sealed, &other_secret),
            Err(CryptoError::Decrypt)
        ));
    }

    #[test]
    fn open_rejects_short_payload() {
        let (secret, _) = hub_keys();
        assert!(matches!(
            open_from_probe(&[0u8; 43], &secret),
            Err(CryptoError::PayloadTooShort(43))
        ));
    }

    #[test]
    fn signature_verifies_and_detects_changes() {
        let key = SigningKey::generate(&mut OsRng);
        let probe_id = hex::encode(key.verifying_key().as_bytes());
        let sig = sign(b"payload", &key);

        assert!(verify(b"payload", &sig, &probe_id).is_ok());
        assert!(matches!(
            verify(b"payl0ad", &sig, &probe_id),
            Err(CryptoError::InvalidSignature(_))
        ));
    }

    #[test]
    fn verify_rejects_malformed_public_key() {
        let key = SigningKey::generate(&mut OsRng);
        let sig = sign(b"payload", &key);
        assert!(matches!(
            verify(b"payload", &sig, "not-hex"),
            Err(CryptoError::InvalidPublicKey(_))
        ));
        assert!(matches!(
            verify(b"payload", &sig, "abcd"),
            Err(CryptoError::InvalidPublicKey(_))
        ));
    }
}
