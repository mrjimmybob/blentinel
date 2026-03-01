use crate::config::get_base_dir;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::fs;

pub fn load_or_create_key() -> (SigningKey, bool) {
    let key_path = get_base_dir().join("identity.key");

    if key_path.exists() {
        let bytes = fs::read(&key_path).expect("Failed to read identity key");
        let array: [u8; 32] = bytes.try_into().expect("Invalid key length");
        (SigningKey::from_bytes(&array), false)
    } else {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        fs::write(&key_path, signing_key.to_bytes()).expect("Failed to save identity key");
        (signing_key, true)
    }
}
