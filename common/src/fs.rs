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
