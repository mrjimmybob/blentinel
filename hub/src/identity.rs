#![cfg(feature = "ssr")]
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use x25519_dalek::{PublicKey, StaticSecret};

/// Load or create the Hub's persistent X25519 private key.
///
/// - If the key file **exists**: loads and returns it. The public key file is
///   **not** touched — it was written during the initial creation run.
/// - If the key file **does not exist**: generates a new key pair, writes the
///   private key to `key_path` (owner-only), writes the public key to
///   `<key_path_stem>.pub`, and prints the FIRST RUN banner.
pub fn load_or_create_hub_key(key_path: &Path) -> Result<StaticSecret> {
    let (bytes, is_new) = common::fs::load_or_create_key(key_path, || {
        StaticSecret::random_from_rng(rand::rngs::OsRng).to_bytes()
    })
    .with_context(|| format!("Failed to load the hub identity key {}", key_path.display()))?;
    let secret = StaticSecret::from(bytes);

    if !is_new {
        tracing::info!("Hub identity loaded from {}", key_path.display());
    } else {
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
    }
    Ok(secret)
}

/// Load the existing hub identity key, print its public key to stdout, and
/// write it to `pub_key_path` (creating or overwriting the file).
///
/// Called by the `--print-public-key` CLI flag. Fails if the private key file
/// does not exist.
pub fn print_and_write_public_key(key_path: &Path, pub_key_path: &Path) -> Result<()> {
    if !key_path.exists() {
        anyhow::bail!(
            "Hub identity key not found at {}. Start the hub at least once to generate an identity.",
            key_path.display()
        );
    }

    let bytes = common::fs::load_key(key_path)
        .with_context(|| format!("Failed to load the hub identity key {}", key_path.display()))?;
    let public_key = PublicKey::from(&StaticSecret::from(bytes));
    write_pub_key_file(pub_key_path, &public_key);

    println!("HUB PUBLIC KEY: {}", hex::encode(public_key.as_bytes()));
    println!("Public key written to: {}", pub_key_path.display());
    Ok(())
}

/// Write the hex-encoded public key to `path`.
///
/// Failures are non-fatal warnings — the private key is the authoritative
/// source of identity.
pub fn write_pub_key_file(path: &Path, public_key: &PublicKey) {
    let hex = hex::encode(public_key.as_bytes());
    if let Err(e) = fs::write(path, &hex) {
        tracing::warn!("Could not write public key file {}: {}", path.display(), e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reloads_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub_identity.key");
        let created = load_or_create_hub_key(&path).unwrap();
        let loaded = load_or_create_hub_key(&path).unwrap();
        assert_eq!(created.to_bytes(), loaded.to_bytes());
        assert!(path.with_extension("pub").exists());
    }

    #[test]
    fn a_corrupt_key_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub_identity.key");
        fs::write(&path, b"short").unwrap();
        let error = format!("{:#}", load_or_create_hub_key(&path).err().unwrap());
        assert!(error.contains("must be exactly 32"), "{error}");
    }
}
