use x25519_dalek::{EphemeralSecret, PublicKey};
use ed25519_dalek::{SigningKey, Signer};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, KeyInit, aead::{Aead, OsRng}};
use rand::RngCore;

pub struct SecureSeal;

impl SecureSeal {
    // This is the "Sandwich" maker
    pub fn encrypt_for_hub(
        data: &[u8], 
        hub_public_key_bytes: &[u8; 32]
    ) -> Result<Vec<u8>, anyhow::Error> {
        let probe_secret = EphemeralSecret::random_from_rng(OsRng);
        let probe_public = PublicKey::from(&probe_secret);
        let hub_public = PublicKey::from(*hub_public_key_bytes);
        let shared_secret = probe_secret.diffie_hellman(&hub_public);
        
        // Call our internal helper
        let ciphertext_with_nonce = Self::encrypt(data, shared_secret.as_bytes())?;

        let mut final_payload = probe_public.as_bytes().to_vec();
        final_payload.extend(ciphertext_with_nonce);
        
        Ok(final_payload)
    }

    // --- INTERNAL HELPERS ---

    fn encrypt(data: &[u8], key_bytes: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        let key = Key::from_slice(key_bytes);
        let cipher = ChaCha20Poly1305::new(key);
        
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // This returns a Vec<u8> containing [Ciphertext + 16-byte Auth Tag]
        let mut ciphertext = cipher.encrypt(nonce, data)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

        // Prepend the nonce so the receiver has it: [Nonce(12)] + [Ciphertext(?)]
        let mut result = nonce_bytes.to_vec();
        result.append(&mut ciphertext);
        Ok(result)
    }

    pub fn sign(data: &[u8], key: &SigningKey) -> Vec<u8> {
        key.sign(data).to_bytes().to_vec()
    }

}