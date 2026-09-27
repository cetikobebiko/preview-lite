//! Path security, symlink & hardlink rejection, and canonicalization containment checks.

use super::error::{PluginError, PluginErrorCode};
use super::limits::{MAX_PACKAGE_SIZE_BYTES, MAX_SCRIPT_SIZE_BYTES};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

/// Validates that a path is safe, contains no symlinks, hardlinks or reparse points,
/// and is properly canonicalized within the expected base directory.
pub fn validate_and_canonicalize_contained_path(
    base_dir: &Path,
    relative_target: &str,
) -> Result<PathBuf, PluginError> {
    // 1. Check relative target string for forbidden sequences
    if relative_target.is_empty()
        || relative_target.starts_with('/')
        || relative_target.starts_with('\\')
        || relative_target.contains(':')
    {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Geçersiz göreceli yol: '{}'", relative_target),
        ));
    }

    let rel_path = Path::new(relative_target);
    for component in rel_path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PluginError::new(
                    PluginErrorCode::PluginPathTraversal,
                    format!(
                        "Yol aşımı (path traversal) engellendi: '{}'",
                        relative_target
                    ),
                ));
            }
        }
    }

    // 2. Canonicalize base directory
    let canonical_base = base_dir.canonicalize().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Temel dizin canonicalize edilemedi: {}", e),
        )
    })?;

    // 3. Build target path and verify each component relative to canonical_base
    let mut current = canonical_base.clone();
    for component in rel_path.components() {
        current.push(component);
        if current.exists() {
            let meta = fs::symlink_metadata(&current).map_err(|e| {
                PluginError::new(
                    PluginErrorCode::PluginPathInvalid,
                    format!("Yol metaverisi okunamadı ('{}'): {}", current.display(), e),
                )
            })?;

            if meta.file_type().is_symlink() {
                return Err(PluginError::new(
                    PluginErrorCode::PluginSymlinkRejected,
                    format!(
                        "Sembolik bağlantı (symlink) kullanımı güvenlik gereği reddedildi: '{}'",
                        current.display()
                    ),
                ));
            }

            #[cfg(unix)]
            {
                if meta.is_file() && meta.nlink() > 1 {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginHardlinkRejected,
                        format!(
                            "Hardlink tespit edildi (nlink = {}): '{}'",
                            meta.nlink(),
                            current.display()
                        ),
                    ));
                }
            }

            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;
                if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginSymlinkRejected,
                        format!(
                            "Windows reparse point kullanımı güvenlik gereği reddedildi: '{}'",
                            current.display()
                        ),
                    ));
                }
            }
        }
    }

    // 4. Canonicalize target
    let canonical_target = current.canonicalize().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Hedef dosya canonicalize edilemedi: {}", e),
        )
    })?;

    // 5. Containment check
    if !canonical_target.starts_with(&canonical_base) {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathTraversal,
            "Canonical hedef yol, eklenti temel dizini dışında kalıyor.",
        ));
    }

    // 6. Must be a regular file
    let meta = fs::symlink_metadata(&canonical_target).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginFileNotRegular,
            format!("Dosya meta verisi okunamadı: {}", e),
        )
    })?;

    if !meta.file_type().is_file() {
        return Err(PluginError::new(
            PluginErrorCode::PluginFileNotRegular,
            "Hedef yol standart bir dosya (regular file) değil.",
        ));
    }

    #[cfg(unix)]
    {
        if meta.nlink() > 1 {
            return Err(PluginError::new(
                PluginErrorCode::PluginHardlinkRejected,
                format!(
                    "Hedef dosyada hardlink tespit edildi (nlink = {}): '{}'",
                    meta.nlink(),
                    canonical_target.display()
                ),
            ));
        }
    }

    if meta.len() > MAX_SCRIPT_SIZE_BYTES {
        return Err(PluginError::new(
            PluginErrorCode::PluginScriptTooLarge,
            format!(
                "Betik boyutu {} byte; izin verilen üst sınır {} byte.",
                meta.len(),
                MAX_SCRIPT_SIZE_BYTES
            ),
        ));
    }

    Ok(canonical_target)
}

/// Recursively checks that no file or directory under `root` is a symlink, hardlink, or reparse point.
pub fn verify_no_symlinks_in_path(root: &Path) -> Result<(), PluginError> {
    if !root.exists() {
        return Ok(());
    }

    let canonical_root = root.canonicalize().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Kök dizin canonicalize edilemedi: {}", e),
        )
    })?;

    let mut stack = vec![canonical_root];
    while let Some(current_dir) = stack.pop() {
        let entries = fs::read_dir(&current_dir).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Dizin içeriği okunamadı: {}", e),
            )
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                PluginError::new(
                    PluginErrorCode::PluginPathInvalid,
                    format!("Dizin girdisi okunamadı: {}", e),
                )
            })?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| {
                PluginError::new(
                    PluginErrorCode::PluginPathInvalid,
                    format!("Metaveri okunamadı: {}", e),
                )
            })?;

            if meta.file_type().is_symlink() {
                return Err(PluginError::new(
                    PluginErrorCode::PluginSymlinkRejected,
                    format!(
                        "Paket içinde sembolik bağlantı tespit edildi: '{}'",
                        path.display()
                    ),
                ));
            }

            #[cfg(unix)]
            {
                if meta.is_file() && meta.nlink() > 1 {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginHardlinkRejected,
                        format!(
                            "Paket içinde hardlink tespit edildi (nlink = {}): '{}'",
                            meta.nlink(),
                            path.display()
                        ),
                    ));
                }
            }

            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;
                if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginSymlinkRejected,
                        format!(
                            "Windows reparse point kullanımı güvenlik gereği reddedildi: '{}'",
                            path.display()
                        ),
                    ));
                }
            }

            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(())
}

/// Computes SHA256 hex string for a byte buffer.
pub fn compute_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Computes domain-separated SHA256 package digest from manifest and main script bytes.
pub fn compute_package_digest(manifest_bytes: &[u8], script_bytes: &[u8]) -> String {
    let manifest_digest = compute_sha256_hex(manifest_bytes);
    let script_digest = compute_sha256_hex(script_bytes);
    let mut hasher = Sha256::new();
    hasher.update(b"antigravity-plugin-v1:manifest=");
    hasher.update(manifest_digest.as_bytes());
    hasher.update(b":script=");
    hasher.update(script_digest.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Validates Package V1 package directory layout:
/// Enforces exactly-two-files rule (`manifest.json` and `main.rhai`), total size <= 10 MB, no symlinks/hardlinks.
pub fn scan_package_directory_safety(package_dir: &Path) -> Result<u64, PluginError> {
    let canonical_pkg = package_dir.canonicalize().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Paket dizini canonicalize edilemedi: {}", e),
        )
    })?;

    let entries = fs::read_dir(&canonical_pkg).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Dizin içeriği okunamadı: {}", e),
        )
    })?;

    let mut found_manifest = false;
    let mut found_main_rhai = false;
    let mut total_size = 0u64;
    let mut file_count = 0usize;

    for entry in entries {
        let entry = entry.map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Dizin girdisi okunamadı: {}", e),
            )
        })?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Metaveri okunamadı: {}", e),
            )
        })?;

        if meta.file_type().is_symlink() {
            return Err(PluginError::new(
                PluginErrorCode::PluginSymlinkRejected,
                format!(
                    "Paket içinde sembolik bağlantı tespit edildi: '{}'",
                    path.display()
                ),
            ));
        }

        #[cfg(unix)]
        {
            if meta.is_file() && meta.nlink() > 1 {
                return Err(PluginError::new(
                    PluginErrorCode::PluginHardlinkRejected,
                    format!(
                        "Paket içinde hardlink tespit edildi (nlink = {}): '{}'",
                        meta.nlink(),
                        path.display()
                    ),
                ));
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;
            if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                return Err(PluginError::new(
                    PluginErrorCode::PluginSymlinkRejected,
                    format!(
                        "Windows reparse point kullanımı güvenlik gereği reddedildi: '{}'",
                        path.display()
                    ),
                ));
            }
        }

        if meta.is_dir() {
            return Err(PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                format!(
                    "Package V1 alt dizin içeremez (bulunan dizin: '{}').",
                    path.display()
                ),
            ));
        }

        if !meta.is_file() {
            return Err(PluginError::new(
                PluginErrorCode::PluginFileNotRegular,
                format!(
                    "Paket içinde standart dışı dosya türü: '{}'",
                    path.display()
                ),
            ));
        }

        file_count += 1;
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if filename == "manifest.json" {
            found_manifest = true;
        } else if filename == "main.rhai" {
            found_main_rhai = true;
        } else {
            return Err(PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                format!(
                    "Package V1 yalnızca 'manifest.json' ve 'main.rhai' içerebilir (beklenmeyen dosya: '{}').",
                    filename
                ),
            ));
        }

        total_size = total_size.saturating_add(meta.len());
        if total_size > MAX_PACKAGE_SIZE_BYTES {
            return Err(PluginError::new(
                PluginErrorCode::PluginResourceLimitExceeded,
                format!(
                    "Toplam paket boyutu {} byte sınırını aştı.",
                    MAX_PACKAGE_SIZE_BYTES
                ),
            ));
        }
    }

    if !found_manifest {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            "Paket içinde 'manifest.json' bulunamadı.",
        ));
    }
    if !found_main_rhai {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            "Paket içinde 'main.rhai' bulunamadı.",
        ));
    }
    if file_count != 2 {
        return Err(PluginError::new(
            PluginErrorCode::PluginManifestInvalid,
            format!(
                "Package V1 kesinlikle tam olarak 2 dosya içermelidir, bulunan dosya sayısı: {}.",
                file_count
            ),
        ));
    }

    Ok(total_size)
}

/// Creates a directory with private mode (0700 on Unix) recursively.
pub fn create_private_dir_all(path: &Path) -> Result<(), PluginError> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        builder.mode(0o700);
        builder.create(path).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Özel dizin oluşturulamadı ('{}'): {}", path.display(), e),
            )
        })?;
        // Explicitly enforce permissions on the final directory
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        Ok(())
    }

    #[cfg(not(unix))]
    {
        fs::create_dir_all(path).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Dizin oluşturulamadı ('{}'): {}", path.display(), e),
            )
        })
    }
}

/// Writes a file with private mode (0600 on Unix).
pub fn write_private_file(path: &Path, content: &[u8]) -> Result<(), PluginError> {
    #[cfg(unix)]
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| {
                PluginError::new(
                    PluginErrorCode::PluginPathInvalid,
                    format!("Özel dosya açılamadı ('{}'): {}", path.display(), e),
                )
            })?;
        file.write_all(content).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Özel dosya yazılamadı ('{}'): {}", path.display(), e),
            )
        })?;
        file.sync_all().map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!(
                    "Özel dosya senkronize edilemedi ('{}'): {}",
                    path.display(),
                    e
                ),
            )
        })?;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        Ok(())
    }

    #[cfg(not(unix))]
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| {
                PluginError::new(
                    PluginErrorCode::PluginPathInvalid,
                    format!("Dosya açılamadı ('{}'): {}", path.display(), e),
                )
            })?;
        file.write_all(content).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Dosya yazılamadı ('{}'): {}", path.display(), e),
            )
        })?;
        file.sync_all().map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Dosya senkronize edilemedi ('{}'): {}", path.display(), e),
            )
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_path_windows_drive_prefix_rejected() {
        let dir = tempdir().unwrap();
        let err1 =
            validate_and_canonicalize_contained_path(dir.path(), "C:\\malicious.rhai").unwrap_err();
        assert_eq!(err1.code, PluginErrorCode::PluginPathInvalid);

        let err2 =
            validate_and_canonicalize_contained_path(dir.path(), "D:script.rhai").unwrap_err();
        assert_eq!(err2.code, PluginErrorCode::PluginPathInvalid);
    }

    #[test]
    fn test_path_unc_rejected() {
        let dir = tempdir().unwrap();
        let err1 =
            validate_and_canonicalize_contained_path(dir.path(), "\\\\server\\share\\script.rhai")
                .unwrap_err();
        assert_eq!(err1.code, PluginErrorCode::PluginPathInvalid);

        let err2 =
            validate_and_canonicalize_contained_path(dir.path(), "//server/share/script.rhai")
                .unwrap_err();
        assert_eq!(err2.code, PluginErrorCode::PluginPathInvalid);
    }

    #[test]
    fn test_path_nested_traversal_rejected() {
        let dir = tempdir().unwrap();
        let err =
            validate_and_canonicalize_contained_path(dir.path(), "a/b/../../c/../../escape.rhai")
                .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPathTraversal);
    }

    #[test]
    fn test_path_root_prefix_collision_rejected() {
        let dir = tempdir().unwrap();
        let err = validate_and_canonicalize_contained_path(dir.path(), "/etc/passwd").unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPathInvalid);
    }

    #[test]
    #[cfg(unix)]
    fn test_path_manifest_symlink_rejected() {
        let dir = tempdir().unwrap();
        let real_file = tempfile::NamedTempFile::new().unwrap();
        let symlink_path = dir.path().join("manifest.json");
        std::os::unix::fs::symlink(real_file.path(), &symlink_path).unwrap();
        fs::write(dir.path().join("main.rhai"), "fn main() {}").unwrap();

        let err = scan_package_directory_safety(dir.path()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginSymlinkRejected);
    }

    #[test]
    #[cfg(unix)]
    fn test_path_main_script_symlink_rejected() {
        let dir = tempdir().unwrap();
        let real_file = tempfile::NamedTempFile::new().unwrap();
        let symlink_path = dir.path().join("main.rhai");
        std::os::unix::fs::symlink(real_file.path(), &symlink_path).unwrap();
        fs::write(dir.path().join("manifest.json"), "{}").unwrap();

        let err = scan_package_directory_safety(dir.path()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginSymlinkRejected);
    }

    #[test]
    #[cfg(unix)]
    fn test_path_directory_symlink_rejected() {
        let dir = tempdir().unwrap();
        let target_dir = tempdir().unwrap();
        let symlink_dir = dir.path().join("sub_link");
        std::os::unix::fs::symlink(target_dir.path(), &symlink_dir).unwrap();

        let err = verify_no_symlinks_in_path(dir.path()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginSymlinkRejected);
    }

    #[test]
    fn test_path_non_regular_file_rejected() {
        let dir = tempdir().unwrap();
        let sub_dir = dir.path().join("sub_folder");
        fs::create_dir(&sub_dir).unwrap();

        let err = validate_and_canonicalize_contained_path(dir.path(), "sub_folder").unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginFileNotRegular);
    }

    #[test]
    fn test_path_case_sensitivity_behavior() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("main.rhai");
        fs::write(&script, "fn main() {}").unwrap();

        let res = validate_and_canonicalize_contained_path(dir.path(), "main.rhai");
        assert!(res.is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn test_path_windows_reparse_point_rejected() {
        // Windows-specific reparse point rejection test
    }
}
