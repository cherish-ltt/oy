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

/// Create or overwrite a file with 600 permissions atomically (Unix)
/// or via fallback (non-Unix). Eliminates the race window between write and chmod.
pub fn write_file_with_permissions_600(path: &Path, content: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(content)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content)?;
        set_file_permissions_600(path)?;
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

    #[test]
    fn test_write_file_with_permissions_600_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("new_secret_file.txt");
        let content = b"secret data";
        write_file_with_permissions_600(&file_path, content).unwrap();
        assert!(file_path.exists(), "File should be created");
        let read_back = std::fs::read(&file_path).unwrap();
        assert_eq!(read_back, content);
    }

    #[cfg(unix)]
    #[test]
    fn test_write_file_with_permissions_600_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("new_mode_test.txt");
        let content = b"test";
        // 文件不存在，write_file_with_permissions_600 会创建新文件并设 0o600 权限
        write_file_with_permissions_600(&file_path, content).unwrap();
        assert!(file_path.exists(), "File should be created");
        let meta = std::fs::metadata(&file_path).unwrap();
        let mode = meta.permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "expected 0o600, got {:#o}", mode);
    }
}
