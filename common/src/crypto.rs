//! Probe ↔ hub payload protection, shared by both sides so there is exactly
//! one implementation of the wire format.
//!
//! Wire format of a sealed payload, in both directions:
//!
//! ```text
//! [ sender's ephemeral X25519 public key (32) ][ nonce (12) ][ ChaCha20-Poly1305 ciphertext + tag ]
//! ```
//!
//! The symmetric key is the X25519 shared secret between the ephemeral key
//! and the recipient's static key, so only the recipient can open it:
//!
//! - **probe → hub** ([`seal_for_hub`] / [`open_from_probe`]): the hub's X25519 key;
//! - **hub → probe** ([`seal_for_probe`] / [`open_from_hub`]): the X25519 form
//!   of the probe's Ed25519 identity, so a probe needs no second key (D-48).
//!
//! Authenticity of the *sender* comes from a separate Ed25519 signature
//! ([`sign`] / [`verify`]), not from the seal: anyone can seal for a public key.
//! The hub signs with a key derived from its identity ([`hub_signing_key`]).

use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};
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
    seal(data, hub_public_key)
}

/// Decrypt a payload produced by [`seal_for_hub`] using the hub's static secret.
pub fn open_from_probe(sealed: &[u8], hub_secret: &StaticSecret) -> Result<Vec<u8>, CryptoError> {
    open(sealed, hub_secret)
}

/// Encrypt `data` so that only the probe `probe_id` (hex of its Ed25519
/// public key) can read it.
pub fn seal_for_probe(data: &[u8], probe_id: &str) -> Result<Vec<u8>, CryptoError> {
    seal(data, &verifying_key(probe_id)?.to_montgomery().to_bytes())
}

/// Decrypt a payload produced by [`seal_for_probe`] with the probe's identity key.
pub fn open_from_hub(sealed: &[u8], probe_key: &SigningKey) -> Result<Vec<u8>, CryptoError> {
    open(sealed, &StaticSecret::from(probe_key.to_scalar_bytes()))
}

/// The hub's Ed25519 signing key, derived from its X25519 identity secret
/// (domain-separated SHA-256), so the hub has one key file to keep and back
/// up. Its public half is in every enrollment code.
pub fn hub_signing_key(hub_secret: &StaticSecret) -> SigningKey {
    let seed: [u8; 32] = Sha256::new()
        .chain_update(b"blentinel hub signing key v1\0")
        .chain_update(hub_secret.as_bytes())
        .finalize()
        .into();
    SigningKey::from_bytes(&seed)
}

fn seal(data: &[u8], recipient: &[u8; PUBLIC_KEY_LEN]) -> Result<Vec<u8>, CryptoError> {
    let ephemeral = EphemeralSecret::random_from_rng(OsRng);
    let ephemeral_public = PublicKey::from(&ephemeral);
    let shared = ephemeral.diffie_hellman(&PublicKey::from(*recipient));

    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = ChaCha20Poly1305::new(Key::from_slice(shared.as_bytes()))
        .encrypt(Nonce::from_slice(&nonce), data)
        .map_err(|_| CryptoError::Encrypt)?;

    let mut sealed = Vec::with_capacity(MIN_SEALED_LEN + ciphertext.len());
    sealed.extend_from_slice(ephemeral_public.as_bytes());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

fn open(sealed: &[u8], secret: &StaticSecret) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < MIN_SEALED_LEN {
        return Err(CryptoError::PayloadTooShort(sealed.len()));
    }
    let (sender_public, rest) = sealed.split_at(PUBLIC_KEY_LEN);
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);

    let sender_public: [u8; PUBLIC_KEY_LEN] = sender_public
        .try_into()
        .map_err(|_| CryptoError::PayloadTooShort(sealed.len()))?;
    let shared = secret.diffie_hellman(&PublicKey::from(sender_public));

    ChaCha20Poly1305::new(Key::from_slice(shared.as_bytes()))
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Decrypt)
}

/// Sign `data` with an Ed25519 key (a probe's identity, or the hub's signing key).
pub fn sign(data: &[u8], key: &SigningKey) -> Vec<u8> {
    key.sign(data).to_bytes().to_vec()
}

/// Verify an Ed25519 signature. `public_key_hex` is the probe ID (hex of its verifying key).
pub fn verify(data: &[u8], signature: &[u8], public_key_hex: &str) -> Result<(), CryptoError> {
    verify_with(data, signature, &verifying_key(public_key_hex)?)
}

/// Verify an Ed25519 signature by the raw public key `public_key` (e.g. the
/// hub's signing key from an enrollment code).
pub fn verify_with_key(
    data: &[u8],
    signature: &[u8],
    public_key: &[u8; PUBLIC_KEY_LEN],
) -> Result<(), CryptoError> {
    let key = VerifyingKey::from_bytes(public_key)
        .map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))?;
    verify_with(data, signature, &key)
}

fn verifying_key(public_key_hex: &str) -> Result<VerifyingKey, CryptoError> {
    let key_bytes: [u8; PUBLIC_KEY_LEN] = hex::decode(public_key_hex)
        .map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))?
        .try_into()
        .map_err(|_| CryptoError::InvalidPublicKey("expected 32 bytes".into()))?;
    VerifyingKey::from_bytes(&key_bytes).map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))
}

fn verify_with(data: &[u8], signature: &[u8], key: &VerifyingKey) -> Result<(), CryptoError> {
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
    fn a_probe_opens_what_the_hub_seals_for_it_and_nobody_else_does() {
        let probe = SigningKey::generate(&mut OsRng);
        let other = SigningKey::generate(&mut OsRng);
        let probe_id = hex::encode(probe.verifying_key().as_bytes());
        let sealed = seal_for_probe(b"ack", &probe_id).unwrap();
        assert_eq!(open_from_hub(&sealed, &probe).unwrap(), b"ack");
        assert!(matches!(
            open_from_hub(&sealed, &other),
            Err(CryptoError::Decrypt)
        ));
    }

    #[test]
    fn the_hub_signing_key_is_stable_per_identity_and_distinct() {
        let (secret, _) = hub_keys();
        let (other, _) = hub_keys();
        let key = hub_signing_key(&secret);
        assert_eq!(key.to_bytes(), hub_signing_key(&secret).to_bytes());
        assert_ne!(key.to_bytes(), hub_signing_key(&other).to_bytes());
        assert_ne!(key.to_bytes(), secret.to_bytes());

        let public = key.verifying_key().to_bytes();
        let signature = sign(b"ack", &key);
        assert!(verify_with_key(b"ack", &signature, &public).is_ok());
        assert!(verify_with_key(b"acK", &signature, &public).is_err());
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
