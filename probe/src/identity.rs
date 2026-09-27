use crate::config::get_base_dir;
use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;

/// The probe's Ed25519 identity (`identity.key` next to the binary), created
/// on first run. Returns the key and whether it was just created.
pub fn load_or_create_key() -> Result<(SigningKey, bool)> {
    let key_path = get_base_dir().join("identity.key");
    let (bytes, is_new) =
        common::fs::load_or_create_key(&key_path, || SigningKey::generate(&mut OsRng).to_bytes())
            .with_context(|| format!("Failed to load the probe identity {}", key_path.display()))?;
    Ok((SigningKey::from_bytes(&bytes), is_new))
}
