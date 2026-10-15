use crate::config::get_base_dir;
use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::path::Path;

/// The file holding the probe's Ed25519 identity, in its folder.
pub const KEY_FILE: &str = "identity.key";

/// The probe's Ed25519 identity (`identity.key` next to the binary), created
/// on first run. Returns the key and whether it was just created.
pub fn load_or_create_key() -> Result<(SigningKey, bool)> {
    load_or_create_key_in(&get_base_dir())
}

/// The identity in `dir` (`probe install` works on the installed folder).
pub fn load_or_create_key_in(dir: &Path) -> Result<(SigningKey, bool)> {
    let key_path = dir.join(KEY_FILE);
    let (bytes, is_new) =
        common::fs::load_or_create_key(&key_path, || SigningKey::generate(&mut OsRng).to_bytes())
            .with_context(|| format!("Failed to load the probe identity {}", key_path.display()))?;
    Ok((SigningKey::from_bytes(&bytes), is_new))
}

/// The probe ID (hex of the public key) for the identity in `dir`, if there is one.
pub fn probe_id_in(dir: &Path) -> Result<Option<String>> {
    let key_path = dir.join(KEY_FILE);
    if !key_path.exists() {
        return Ok(None);
    }
    let bytes = common::fs::load_key(&key_path)
        .with_context(|| format!("can't read {}", key_path.display()))?;
    Ok(Some(hex::encode(
        SigningKey::from_bytes(&bytes).verifying_key().as_bytes(),
    )))
}
