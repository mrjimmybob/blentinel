#![cfg(feature = "ssr")]
use std::fs;
use std::path::Path;
use x25519_dalek::{PublicKey, StaticSecret};

/// Load or create the Hub's persistent X25519 private key.
///
/// - If the key file **exists**: loads and returns it. The public key file is
///   **not** touched — it was written during the initial creation run.
/// - If the key file **does not exist**: generates a new key pair, writes the
///   private key to `key_path`, writes the public key to
///   `<key_path_stem>.pub`, and prints the FIRST RUN banner.
pub fn load_or_create_hub_key(key_path: &Path) -> StaticSecret {
    if key_path.exists() {
        let bytes = fs::read(key_path).expect("Failed to read hub identity key");
        let array: [u8; 32] = bytes
            .try_into()
            .expect("Invalid hub key length (expected 32 bytes)");

        println!("Hub identity loaded from {}", key_path.display());
        StaticSecret::from(array)
    } else {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        fs::write(key_path, secret.to_bytes()).expect("Failed to save hub identity key");

        let public_key = PublicKey::from(&secret);
        let pub_key_hex = hex::encode(public_key.as_bytes());

        // Derive the public key file path alongside the private key.
        let pub_key_path = key_path.with_extension("pub");
        write_pub_key_file(&pub_key_path, &public_key);

        println!("\n================================================");
        println!("FIRST RUN: New Hub Identity Generated");
        println!("================================================");
        println!("HUB PUBLIC KEY:      {}", pub_key_hex);
        println!("\nProbes can use this key in their config file");
        println!("to skip the handshake step (optional).");
        println!("Key saved to:        {}", key_path.display());
        println!("Public key saved to: {}", pub_key_path.display());
        println!("================================================\n");

        secret
    }
}

/// Load the existing hub identity key, print its public key to stdout, and
/// write it to `pub_key_path` (creating or overwriting the file).
///
/// Called by the `--print-public-key` CLI flag. Exits with an error message
/// if the private key file does not exist.
pub fn print_and_write_public_key(key_path: &Path, pub_key_path: &Path) {
    if !key_path.exists() {
        eprintln!(
            "[ERROR] Hub identity key not found at {}.\n\
             Start the hub at least once to generate an identity.",
            key_path.display()
        );
        std::process::exit(1);
    }

    let bytes = fs::read(key_path).expect("Failed to read hub identity key");
    let array: [u8; 32] = bytes
        .try_into()
        .expect("Invalid hub key length (expected 32 bytes)");

    let secret = StaticSecret::from(array);
    let public_key = PublicKey::from(&secret);
    let pub_key_hex = hex::encode(public_key.as_bytes());

    write_pub_key_file(pub_key_path, &public_key);

    println!("HUB PUBLIC KEY: {}", pub_key_hex);
    println!("Public key written to: {}", pub_key_path.display());
}

/// Write the hex-encoded public key to `path`.
///
/// Failures are non-fatal warnings — the private key is the authoritative
/// source of identity.
pub fn write_pub_key_file(path: &Path, public_key: &PublicKey) {
    let hex = hex::encode(public_key.as_bytes());
    if let Err(e) = fs::write(path, &hex) {
        eprintln!(
            "[WARN] Could not write public key file {}: {}",
            path.display(),
            e
        );
    }
}
