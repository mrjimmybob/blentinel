use ed25519_dalek::{SigningKey, VerifyingKey};

use rand::rngs::OsRng; // Ensure rand = "0.8" is in Cargo.toml
use std::fs;
use std::path::Path;

const KEY_FILE: &str = "identity.key";

pub fn load_or_create_key() -> SigningKey {
    if Path::new(KEY_FILE).exists() {
        let bytes = fs::read(KEY_FILE).expect("Failed to read identity key");
        let array: [u8; 32] = bytes.try_into().expect("Invalid key length");
        SigningKey::from_bytes(&array)
    } else {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        fs::write(KEY_FILE, signing_key.to_bytes()).expect("Failed to save key");
        
        let verifying_key: VerifyingKey = (&signing_key).into();
        println!("--------------------------------------------------");
        println!("FIRST RUN: New Probe Identity Generated.");
        println!("PUBLIC KEY: {}", hex::encode(verifying_key.as_bytes()));
        println!("Paste the Public Key above into the Blentinel Hub.");
        println!("--------------------------------------------------");
        
        signing_key
    }
}