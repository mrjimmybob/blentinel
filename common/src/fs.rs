//! File helpers shared by the probe and the hub.

use std::io::Write;
use std::path::Path;

/// Write a secret (private key, token) readable only by its owner.
///
/// On Unix the file is created with mode `0600`, so the secret is never on
/// disk with wider permissions, and an existing file is tightened to `0600`
/// before it is overwritten. On Windows the file inherits its folder's ACL.
pub fn write_secret_file(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents.as_ref())
}

/// Replace `path` with `contents` atomically: write `<path>.tmp`, flush it
/// to disk, then rename it over `path`. A reader (or a crash) sees the old
/// file or the new one, never half of it.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = std::path::PathBuf::from(temp);
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(contents.as_ref())?;
        file.sync_all()?;
    } // closed before the rename (Windows can't rename an open file)
    std::fs::rename(&temp, path)
}

/// A 32-byte private key stored raw in a file (the probe's Ed25519 identity,
/// the hub's X25519 key).
///
/// Loads `path` if it exists; otherwise saves `generate()` there with
/// [`write_secret_file`]. Returns the key and whether it was just created.
/// A file of any other length is an error, never silently replaced: replacing
/// a key changes the probe's ID or breaks every probe's sealing.
pub fn load_or_create_key(
    path: &Path,
    generate: impl FnOnce() -> [u8; 32],
) -> std::io::Result<([u8; 32], bool)> {
    if path.exists() {
        Ok((load_key(path)?, false))
    } else {
        let key = generate();
        write_secret_file(path, key)?;
        Ok((key, true))
    }
}

/// Load an existing 32-byte key file (see [`load_or_create_key`]).
pub fn load_key(path: &Path) -> std::io::Result<[u8; 32]> {
    let bytes = std::fs::read(path)?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{} is {} bytes; a key file must be exactly 32. Restore it from a backup, or delete it to create a new identity.",
                path.display(),
                bytes.len()
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("blentinel-fs-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("secret")
    }

    #[test]
    fn a_key_is_created_once_then_loaded() {
        let path = temp_path("key");
        let _ = std::fs::remove_file(&path);
        let (created, is_new) = load_or_create_key(&path, || [7; 32]).unwrap();
        assert!(is_new);
        let (loaded, is_new) = load_or_create_key(&path, || panic!("must not regenerate")).unwrap();
        assert!(!is_new);
        assert_eq!(created, loaded);
    }

    #[test]
    fn a_key_file_of_the_wrong_length_is_an_error_and_kept() {
        let path = temp_path("badkey");
        std::fs::write(&path, b"short").unwrap();
        let error = load_or_create_key(&path, || [7; 32]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("5 bytes"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"short");
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temp_file() {
        let path = temp_path("atomic").with_file_name("config.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert!(!path.with_file_name("config.json.tmp").exists());
    }

    #[test]
    fn writes_and_overwrites_contents() {
        let path = temp_path("contents");
        write_secret_file(&path, b"first-secret").unwrap();
        write_secret_file(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
    }

    #[cfg(unix)]
    #[test]
    fn secret_is_owner_only_even_when_overwriting_a_loose_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("mode");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();

        write_secret_file(&path, b"new").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        std::fs::remove_file(&path).unwrap();
        write_secret_file(&path, b"fresh").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
