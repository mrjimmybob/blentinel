#![cfg_attr(not(feature = "ssr"), allow(unused_imports, dead_code))]
#[cfg(feature = "ssr")]
use x25519_dalek::{StaticSecret, PublicKey};
use ed25519_dalek::{VerifyingKey, Signature, Verifier};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, KeyInit, aead::Aead};

pub struct SecureSeal;

impl SecureSeal {
    pub fn decrypt_from_probe(
        payload: &[u8], 
        hub_static_secret: &StaticSecret // Use the type directly
    ) -> anyhow::Result<Vec<u8>> {
        if payload.len() < 44 { // 32 (PK) + 12 (Nonce)
            return Err(anyhow::anyhow!("Payload too short"));
        }

        let (probe_pk_bytes, remainder) = payload.split_at(32);
        let (nonce_bytes, ciphertext) = remainder.split_at(12);

        let probe_public = PublicKey::from(<[u8; 32]>::try_from(probe_pk_bytes)?);
        let shared_secret = hub_static_secret.diffie_hellman(&probe_public);

        // FIX: Don't call Self::decrypt, call the cipher directly
        let key = Key::from_slice(shared_secret.as_bytes());
        let cipher = ChaCha20Poly1305::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher.decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))
    }
    
    // Ensure this verify function is INSIDE the impl block
    pub fn verify(data: &[u8], sig_bytes: &[u8], public_key_hex: &str) -> anyhow::Result<()> {
        let pk_bytes = hex::decode(public_key_hex)?;
        let pk = VerifyingKey::from_bytes(&pk_bytes.try_into().map_err(|_| anyhow::anyhow!("Invalid PK"))?)?;
        let sig = Signature::from_slice(sig_bytes)?;
        pk.verify(data, &sig).map_err(|e| anyhow::anyhow!(e))
    }
}