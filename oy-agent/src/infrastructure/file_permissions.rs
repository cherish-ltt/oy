use std::fs;
use std::io;
use std::path::Path;

/// Set file permissions to 600 (owner read/write only) on Unix.
/// On Windows this is a no-op.
pub fn set_file_permissions_600(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    // Windows: no permission setting needed
    #[cfg(windows)]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_set_permissions_600_does_not_error() {
        let tmp = NamedTempFile::new().unwrap();
        let result = set_file_permissions_600(tmp.path());
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_set_permissions_600_unix_actual_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = NamedTempFile::new().unwrap();
        set_file_permissions_600(tmp.path()).unwrap();
        let meta = fs::metadata(tmp.path()).unwrap();
        let mode = meta.permissions().mode();
        // Only check the owner bits; allow for other bits potentially set by the filesystem
        assert_eq!(
            mode & 0o777,
            0o600,
            "expected permissions 0o600, got {:#o}",
            mode
        );
    }
}
