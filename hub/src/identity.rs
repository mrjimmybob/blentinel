#![cfg(feature = "ssr")]
use x25519_dalek::{StaticSecret, PublicKey};
use std::fs;
use std::path::Path;

/// Load or create the Hub's persistent X25519 private key.
/// `key_path` is read from the hub config (server.identity_key_path).
pub fn load_or_create_hub_key(key_path: &str) -> StaticSecret {
    if Path::new(key_path).exists() {
        let bytes = fs::read(key_path).expect("Failed to read hub identity key");
        let array: [u8; 32] = bytes
            .try_into()
            .expect("Invalid hub key length (expected 32 bytes)");

        println!("Hub identity loaded from {}", key_path);
        StaticSecret::from(array)
    } else {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        fs::write(key_path, secret.to_bytes())
            .expect("Failed to save hub identity key");

        let public_key = PublicKey::from(&secret);
        println!("\n================================================");
        println!("FIRST RUN: New Hub Identity Generated");
        println!("================================================");
        println!("HUB PUBLIC KEY: {}", hex::encode(public_key.as_bytes()));
        println!("\nProbes can use this key in their config file");
        println!("to skip the handshake step (optional).");
        println!("Key saved to: {}", key_path);
        println!("================================================\n");

        secret
    }
}