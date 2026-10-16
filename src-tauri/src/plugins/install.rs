//! AppData Private Installation Root, Staging, Atomic Publication, and Restart Persistence.

use super::error::{PluginError, PluginErrorCode};
use super::limits::{
    CURRENT_PLUGIN_CONSENT_POLICY_VERSION, CURRENT_PLUGIN_REGISTRY_SCHEMA_VERSION,
};
use super::manifest::{
    validate_plugin_id, validate_plugin_version, PluginManifestV1, PluginPermission,
};
use super::path_security::{
    compute_package_digest, compute_sha256_hex, create_private_real_dir, is_link_like,
    scan_package_directory_safety, validate_and_canonicalize_contained_path,
    verify_no_symlinks_in_path, write_private_file,
};
use super::registry::{InstalledPluginInfoV1, PluginRegistryEnvelopeV1};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInspectionResult {
    pub manifest: PluginManifestV1,
    pub package_digest: String,
    pub manifest_digest: String,
    pub script_digest: String,
    pub script_size: u64,
    pub package_size: u64,
}

/// Inspects a candidate plugin package directory before installation.
pub fn inspect_plugin_package_internal(
    package_dir: &Path,
) -> Result<PluginInspectionResult, PluginError> {
    // 1. Safety check for directory tree (enforces Package V1 exactly-two-files rule)
    let package_size = scan_package_directory_safety(package_dir)?;

    // 2. Locate and validate manifest.json
    let manifest_path = validate_and_canonicalize_contained_path(package_dir, "manifest.json")?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Manifest okunamadı: {}", e),
        )
    })?;
    let manifest = PluginManifestV1::parse_and_validate(&manifest_bytes)?;

    // 3. Locate and validate main script (Package V1: main.rhai)
    let script_path = validate_and_canonicalize_contained_path(package_dir, &manifest.main_script)?;
    let script_bytes = fs::read(&script_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Ana betik okunamadı: {}", e),
        )
    })?;

    // 4. Compute digests
    let manifest_digest = compute_sha256_hex(&manifest_bytes);
    let script_digest = compute_sha256_hex(&script_bytes);
    let package_digest = compute_package_digest(&manifest_bytes, &script_bytes);

    Ok(PluginInspectionResult {
        manifest,
        package_digest,
        manifest_digest,
        script_digest,
        script_size: script_bytes.len() as u64,
        package_size,
    })
}

/// Installs a verified plugin package into the AppData private root with staging and atomic rename.
pub fn install_plugin_package_internal(
    app_data_dir: &Path,
    package_dir: &Path,
    expected_digest: &str,
    current_registry: &mut HashMap<String, InstalledPluginInfoV1>,
) -> Result<InstalledPluginInfoV1, PluginError> {
    // 1. Perform full inspection
    let inspection = inspect_plugin_package_internal(package_dir)?;

    // 2. Verify digest matches user-reviewed expected digest
    if inspection.package_digest != expected_digest {
        return Err(PluginError::new(
            PluginErrorCode::PluginPackageDigestMismatch,
            format!(
                "Eklenti paket özeti (digest) eşleşmedi. Beklenen: '{}', Hesaplanan: '{}'",
                expected_digest, inspection.package_digest
            ),
        ));
    }

    let manifest = inspection.manifest;
    let plugin_id = manifest.id.clone();
    let version = manifest.version.clone();

    // 3. Check for conflict in registry
    if let Some(existing) = current_registry.get(&plugin_id) {
        if existing.version == version {
            return Err(PluginError::new(
                PluginErrorCode::PluginInstallConflict,
                format!(
                    "Bu eklenti ve sürüm ('{}' v{}) zaten yüklü.",
                    plugin_id, version
                ),
            ));
        }
    }

    // 4. Prepare AppData directories with private permissions (0700)
    let plugins_root = app_data_dir.join("plugins");
    let staging_root = plugins_root.join("staging");
    let installed_root = plugins_root.join("installed");

    // Every level must be a real directory so staged files and the published plugin
    // cannot be redirected outside the private root through a planted symlink.
    create_private_real_dir(&plugins_root)?;
    create_private_real_dir(&staging_root)?;
    create_private_real_dir(&installed_root)?;

    let staging_id = uuid::Uuid::new_v4().hyphenated().to_string();
    let staged_dir = staging_root.join(&staging_id);
    create_private_real_dir(&staged_dir)?;

    // 5. Copy manifest and script into staging with private permissions (0600)
    let manifest_src = package_dir.join("manifest.json");
    let manifest_dest = staged_dir.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_src).map_err(|e| {
        let _ = fs::remove_dir_all(&staged_dir);
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Manifest okunamadı: {}", e),
        )
    })?;
    if let Err(e) = write_private_file(&manifest_dest, &manifest_bytes) {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(e);
    }

    let script_src = package_dir.join(&manifest.main_script);
    let script_dest = staged_dir.join(&manifest.main_script);
    let script_bytes = fs::read(&script_src).map_err(|e| {
        let _ = fs::remove_dir_all(&staged_dir);
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Betik okunamadı: {}", e),
        )
    })?;
    if let Err(e) = write_private_file(&script_dest, &script_bytes) {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(e);
    }

    // 6. Verify staged directory safety and invariants
    if let Err(e) = scan_package_directory_safety(&staged_dir) {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(e);
    }

    // Verify digest of staged files matches expected digest
    let staged_manifest_bytes = fs::read(&manifest_dest).map_err(|e| {
        let _ = fs::remove_dir_all(&staged_dir);
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Staged manifest okunamadı: {}", e),
        )
    })?;
    let staged_script_bytes = fs::read(&script_dest).map_err(|e| {
        let _ = fs::remove_dir_all(&staged_dir);
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Staged betik okunamadı: {}", e),
        )
    })?;
    let staged_digest = compute_package_digest(&staged_manifest_bytes, &staged_script_bytes);
    if staged_digest != inspection.package_digest {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(PluginError::new(
            PluginErrorCode::PluginPackageDigestMismatch,
            "Staging aşamasında paket bütünlüğü doğrulanamadı.",
        ));
    }

    // 7. Atomic target directory placement
    let plugin_target_parent = installed_root.join(&plugin_id);
    if let Err(e) = create_private_real_dir(&plugin_target_parent) {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(e);
    }
    let plugin_target_dir = plugin_target_parent.join(&version);

    if plugin_target_dir.exists() {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(PluginError::new(
            PluginErrorCode::PluginInstallConflict,
            format!(
                "Hedef eklenti dizini ('{}') zaten mevcut.",
                plugin_target_dir.display()
            ),
        ));
    }

    if let Err(e) = fs::rename(&staged_dir, &plugin_target_dir) {
        let _ = fs::remove_dir_all(&staged_dir);
        return Err(PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Eklenti hedef konuma taşınamadı: {}", e),
        ));
    }

    // 8. Create InstalledPluginInfoV1 record
    let installed_info = InstalledPluginInfoV1 {
        id: plugin_id.clone(),
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        permissions: manifest.permissions,
        hooks: manifest.hooks,
        main_script: manifest.main_script,
        enabled: false, // Default is disabled until user explicitly consents and enables
        installed_at_rfc3339: chrono_now_rfc3339(),
        install_dir: plugin_target_dir.to_string_lossy().to_string(),
        package_digest: inspection.package_digest,
        manifest_digest: inspection.manifest_digest,
        script_digest: inspection.script_digest,
        reviewed_package_digest: String::new(),
        reviewed_permissions: Vec::new(),
        consent_granted: false,
        consent_policy_version: 0,
    };

    let backup_registry = current_registry.clone();
    current_registry.insert(plugin_id.clone(), installed_info.clone());

    if let Err(err) = save_persisted_registry(app_data_dir, current_registry) {
        // Rollback: delete published directory, restore old registry
        let _ = fs::remove_dir_all(&plugin_target_dir);
        let _ = fs::remove_dir(&plugin_target_parent);
        *current_registry = backup_registry;
        return Err(err);
    }

    Ok(installed_info)
}

/// Resolves the on-disk directory of a registry entry without trusting its `install_dir`.
///
/// `install_dir` is persisted in `registry.json` and may have been tampered with, so it is
/// accepted only if it canonicalizes to exactly `plugins/installed/<id>/<version>`, with
/// `id`, `version` and `main_script` re-validated against the manifest rules. Since the
/// expected path is built from the canonical root plus two plain components, equality also
/// rules out symlinks at the `<id>` and `<version>` levels. Returns the canonical directory.
pub fn resolve_installed_plugin_dir(
    app_data_dir: &Path,
    info: &InstalledPluginInfoV1,
) -> Result<PathBuf, PluginError> {
    validate_plugin_id(&info.id)?;
    validate_plugin_version(&info.version)?;
    if info.main_script != "main.rhai" {
        return Err(PluginError::new(
            PluginErrorCode::PluginManifestInvalid,
            format!(
                "Registry kaydındaki ana betik adı geçersiz: '{}'",
                info.main_script
            ),
        ));
    }

    let installed_root = app_data_dir.join("plugins").join("installed");
    let canonical_root = installed_root.canonicalize().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Eklenti kurulum kökü canonicalize edilemedi: {}", e),
        )
    })?;
    let expected_dir = canonical_root.join(&info.id).join(&info.version);

    let canonical_install = PathBuf::from(&info.install_dir)
        .canonicalize()
        .map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Eklenti kurulum dizini canonicalize edilemedi: {}", e),
            )
        })?;
    if canonical_install != expected_dir {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathTraversal,
            format!(
                "Registry kaydındaki kurulum dizini ('{}') beklenen eklenti dizini değil.",
                info.install_dir
            ),
        ));
    }

    let meta = fs::symlink_metadata(&expected_dir).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Eklenti kurulum dizini metaverisi okunamadı: {}", e),
        )
    })?;
    if !meta.is_dir() {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            "Eklenti kurulum yolu bir dizin değil.",
        ));
    }

    Ok(expected_dir)
}

/// Grants explicit user consent for an installed plugin after reviewing permissions and digest.
pub fn grant_plugin_consent_internal(
    app_data_dir: &Path,
    plugin_id: &str,
    expected_package_digest: &str,
    expected_permissions: &[PluginPermission],
    current_registry: &mut HashMap<String, InstalledPluginInfoV1>,
) -> Result<InstalledPluginInfoV1, PluginError> {
    let entry = current_registry.get_mut(plugin_id).ok_or_else(|| {
        PluginError::new(
            PluginErrorCode::PluginNotInstalled,
            format!("Eklenti bulunamadı: '{}'", plugin_id),
        )
    })?;

    // Verify disk files on disk
    let install_dir = resolve_installed_plugin_dir(app_data_dir, entry)?;
    let manifest_path = install_dir.join("manifest.json");
    let script_path = install_dir.join(&entry.main_script);

    let manifest_bytes = fs::read(&manifest_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Yüklü manifest okunamadı: {}", e),
        )
    })?;
    let script_bytes = fs::read(&script_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Yüklü betik okunamadı: {}", e),
        )
    })?;

    let disk_digest = compute_package_digest(&manifest_bytes, &script_bytes);
    if disk_digest != expected_package_digest || disk_digest != entry.package_digest {
        // Tampered! Reset consent
        entry.consent_granted = false;
        entry.consent_policy_version = 0;
        entry.enabled = false;
        let _ = save_persisted_registry(app_data_dir, current_registry);
        return Err(PluginError::new(
            PluginErrorCode::PluginPackageDigestMismatch,
            "Eklenti disk dosyaları onaylanan paket özetiyle eşleşmiyor (değiştirilmiş olabilir).",
        ));
    }

    let manifest = PluginManifestV1::parse_and_validate(&manifest_bytes)?;
    let disk_perms: BTreeSet<PluginPermission> = manifest.permissions.into_iter().collect();
    let expected_perms_set: BTreeSet<PluginPermission> =
        expected_permissions.iter().copied().collect();

    if disk_perms != expected_perms_set {
        return Err(PluginError::new(
            PluginErrorCode::PluginPermissionDenied,
            "Onaylanmak istenen izin kümesi eklenti manifestiyle tam eşleşmiyor.",
        ));
    }

    entry.consent_granted = true;
    entry.consent_policy_version = CURRENT_PLUGIN_CONSENT_POLICY_VERSION;
    entry.reviewed_package_digest = expected_package_digest.to_string();
    entry.reviewed_permissions = expected_permissions.to_vec();

    let updated = entry.clone();
    save_persisted_registry(app_data_dir, current_registry)?;

    Ok(updated)
}

/// Uninstalls an installed plugin and removes its physical files safely.
pub fn uninstall_plugin_package_internal(
    app_data_dir: &Path,
    plugin_id: &str,
    current_registry: &mut HashMap<String, InstalledPluginInfoV1>,
) -> Result<(), PluginError> {
    let entry = current_registry.remove(plugin_id).ok_or_else(|| {
        PluginError::new(
            PluginErrorCode::PluginNotInstalled,
            format!("Kaldırılacak eklenti bulunamadı: '{}'", plugin_id),
        )
    })?;

    // Touch disk only when install_dir is exactly plugins/installed/<id>/<version>;
    // otherwise drop the registry entry and leave the filesystem alone.
    if let Ok(plugin_dir) = resolve_installed_plugin_dir(app_data_dir, &entry) {
        let _ = remove_installed_plugin_dir(app_data_dir, &plugin_dir, &entry.id, &entry.version);
    }

    save_persisted_registry(app_data_dir, current_registry)?;
    Ok(())
}

/// Deletes a resolved `plugins/installed/<id>/<version>` directory.
///
/// The directory is first moved into a fresh quarantine under `plugins/staging` and only
/// deleted there, so a swap of `<id>` for a symlink after `resolve_installed_plugin_dir`
/// cannot redirect the recursive delete. On Unix the move is done relative to directory
/// descriptors opened with `O_NOFOLLOW`. Leftover quarantines are cleaned on next load.
fn remove_installed_plugin_dir(
    app_data_dir: &Path,
    plugin_dir: &Path,
    id: &str,
    version: &str,
) -> Result<(), PluginError> {
    let io_err = |e: std::io::Error| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Eklenti dizini kaldırılamadı: {}", e),
        )
    };

    let staging_root = app_data_dir.join("plugins").join("staging");
    create_private_real_dir(&staging_root)?;
    let staging_root = staging_root.canonicalize().map_err(io_err)?;
    let quarantine_name = format!("uninstall-{}", uuid::Uuid::new_v4().hyphenated());
    let quarantine = staging_root.join(&quarantine_name);

    // plugin_dir is <canonical installed root>/<id>/<version>
    let installed_root = plugin_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io_err(std::io::ErrorKind::NotFound.into()))?;

    #[cfg(unix)]
    unix_dirfd::quarantine_plugin_dir(installed_root, &staging_root, id, version, &quarantine_name)
        .map_err(io_err)?;

    #[cfg(not(unix))]
    {
        let _ = version;
        fs::rename(plugin_dir, &quarantine).map_err(io_err)?;
        let _ = fs::remove_dir(installed_root.join(id));
    }

    fs::remove_dir_all(&quarantine).map_err(io_err)
}

/// Removes a symlink or junction itself, never its target.
fn remove_link(path: &Path) {
    if fs::remove_file(path).is_err() {
        // Windows directory symlinks and junctions are removed as directories
        let _ = fs::remove_dir(path);
    }
}

#[cfg(unix)]
mod unix_dirfd {
    use std::ffi::{CStr, CString};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn c_string(bytes: &[u8]) -> io::Result<CString> {
        CString::new(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    }

    /// Opens a directory without following a symlink in its final component.
    fn open_dir_nofollow(dirfd: Option<RawFd>, name: &CStr) -> io::Result<OwnedFd> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `name` is a valid NUL-terminated string and `dirfd` an open descriptor.
        let fd = unsafe {
            match dirfd {
                Some(dirfd) => libc::openat(dirfd, name.as_ptr(), flags),
                None => libc::open(name.as_ptr(), flags),
            }
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` was just returned by open/openat and is owned by nobody else.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// Moves `<installed_root>/<id>/<version>` to `<staging_root>/<quarantine_name>` using
    /// descriptors pinned with `O_NOFOLLOW`, then removes `<id>` if it is now empty.
    pub(super) fn quarantine_plugin_dir(
        installed_root: &Path,
        staging_root: &Path,
        id: &str,
        version: &str,
        quarantine_name: &str,
    ) -> io::Result<()> {
        let root_fd = open_dir_nofollow(None, &c_string(installed_root.as_os_str().as_bytes())?)?;
        let staging_fd = open_dir_nofollow(None, &c_string(staging_root.as_os_str().as_bytes())?)?;
        let id_c = c_string(id.as_bytes())?;
        let version_c = c_string(version.as_bytes())?;
        let quarantine_c = c_string(quarantine_name.as_bytes())?;

        let id_fd = open_dir_nofollow(Some(root_fd.as_raw_fd()), &id_c)?;
        // <version> must currently be a real directory inside the pinned <id> directory
        drop(open_dir_nofollow(Some(id_fd.as_raw_fd()), &version_c)?);

        // SAFETY: all descriptors are open directories and the names are NUL-terminated.
        // renameat never follows a symlink in the moved entry itself.
        let rc = unsafe {
            libc::renameat(
                id_fd.as_raw_fd(),
                version_c.as_ptr(),
                staging_fd.as_raw_fd(),
                quarantine_c.as_ptr(),
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }

        // Best effort: drop the now-empty <id> directory (ENOTEMPTY when other versions remain)
        // SAFETY: `root_fd` is an open directory and `id_c` is NUL-terminated.
        unsafe {
            libc::unlinkat(root_fd.as_raw_fd(), id_c.as_ptr(), libc::AT_REMOVEDIR);
        }
        Ok(())
    }
}

/// Loads the persisted plugin registry from `$APPDATA/plugins/registry.json` and cleans staging leftovers.
pub fn load_persisted_registry(app_data_dir: &Path) -> HashMap<String, InstalledPluginInfoV1> {
    let plugins_root = app_data_dir.join("plugins");
    match fs::symlink_metadata(&plugins_root) {
        Ok(meta) if is_link_like(&meta) || !meta.is_dir() => {
            // A redirected plugins root is not ours to clean or trust: fail closed
            return HashMap::new();
        }
        Ok(_) => {}
        Err(_) => return HashMap::new(),
    }

    // Clean up orphaned staging and uninstall-quarantine leftovers from previous runs
    let staging_root = plugins_root.join("staging");
    if let Ok(meta) = fs::symlink_metadata(&staging_root) {
        if is_link_like(&meta) {
            // Never follow a planted staging link; drop the link itself
            remove_link(&staging_root);
        } else if meta.is_dir() {
            if let Ok(entries) = fs::read_dir(&staging_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    match fs::symlink_metadata(&path) {
                        Ok(m) if is_link_like(&m) => remove_link(&path),
                        Ok(m) if m.is_dir() => {
                            let _ = fs::remove_dir_all(&path);
                        }
                        Ok(_) => {
                            let _ = fs::remove_file(&path);
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    let registry_file = plugins_root.join("registry.json");
    if !registry_file.exists() {
        return HashMap::new();
    }

    let bytes = match fs::read(&registry_file) {
        Ok(b) => b,
        Err(_) => return HashMap::new(),
    };

    let envelope: PluginRegistryEnvelopeV1 =
        match serde_json::from_slice::<PluginRegistryEnvelopeV1>(&bytes) {
            Ok(env) => {
                if env.schema_version != CURRENT_PLUGIN_REGISTRY_SCHEMA_VERSION {
                    // Unsupported registry schema version -> fail closed
                    return HashMap::new();
                }
                env
            }
            Err(_) => {
                // Syntactically corrupt registry JSON -> fail closed, do NOT overwrite or panic
                return HashMap::new();
            }
        };

    let mut valid_map = HashMap::new();

    for mut info in envelope.entries {
        // install_dir is untrusted: it must be exactly plugins/installed/<id>/<version>
        let canonical_install = match resolve_installed_plugin_dir(app_data_dir, &info) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Verify files exist and check integrity
        let manifest_path = canonical_install.join("manifest.json");
        let script_path = canonical_install.join(&info.main_script);

        if !manifest_path.is_file() || !script_path.is_file() {
            continue;
        }

        // Check symlinks
        if verify_no_symlinks_in_path(&canonical_install).is_err() {
            continue;
        }

        let manifest_bytes = match fs::read(&manifest_path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let script_bytes = match fs::read(&script_path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let current_digest = compute_package_digest(&manifest_bytes, &script_bytes);
        if current_digest != info.package_digest {
            // Tampered entry: quarantine by disabling and revoking consent
            info.consent_granted = false;
            info.consent_policy_version = 0;
            info.enabled = false;
        }

        valid_map.insert(info.id.clone(), info);
    }

    valid_map
}

/// Persists registry atomically via a temporary file and rename with 0600 permissions.
pub fn save_persisted_registry(
    app_data_dir: &Path,
    current_registry: &HashMap<String, InstalledPluginInfoV1>,
) -> Result<(), PluginError> {
    let plugins_root = app_data_dir.join("plugins");
    create_private_real_dir(&plugins_root)?;

    let registry_file = plugins_root.join("registry.json");
    let tmp_file = plugins_root.join(format!("registry.json.tmp.{}", uuid::Uuid::new_v4()));

    let envelope = PluginRegistryEnvelopeV1 {
        schema_version: CURRENT_PLUGIN_REGISTRY_SCHEMA_VERSION,
        entries: current_registry.values().cloned().collect(),
    };

    let json_bytes = serde_json::to_vec_pretty(&envelope).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginManifestInvalid,
            format!("Registry serileştirme hatası: {}", e),
        )
    })?;

    write_private_file(&tmp_file, &json_bytes)?;

    #[cfg(not(windows))]
    {
        fs::rename(&tmp_file, &registry_file).map_err(|e| {
            let _ = fs::remove_file(&tmp_file);
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!("Registry dosyası kaydedilemedi: {}", e),
            )
        })?;
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let wide_tmp: Vec<u16> = tmp_file.as_os_str().encode_wide().chain(Some(0)).collect();
        let wide_target: Vec<u16> = registry_file
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();

        let success = unsafe {
            MoveFileExW(
                wide_tmp.as_ptr(),
                wide_target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };

        if success == 0 {
            let err = std::io::Error::last_os_error();
            let _ = fs::remove_file(&tmp_file);
            return Err(PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                format!(
                    "Windows MoveFileExW registry dosyası kaydedilemedi: {}",
                    err
                ),
            ));
        }
    }

    Ok(())
}

fn chrono_now_rfc3339() -> String {
    let now = std::time::SystemTime::now();
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}.{:03}Z",
        since_epoch.as_secs(),
        since_epoch.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn install_fixture(
        app_data_dir: &Path,
        id: &str,
        registry: &mut HashMap<String, InstalledPluginInfoV1>,
    ) -> InstalledPluginInfoV1 {
        let package = tempdir().unwrap();
        let manifest = format!(
            r#"{{
            "manifest_version": 1,
            "id": "{id}",
            "name": "Test Plugin",
            "version": "1.0.0",
            "main_script": "main.rhai",
            "hooks": ["on_custom_command"]
        }}"#
        );
        fs::write(package.path().join("manifest.json"), manifest).unwrap();
        fs::write(
            package.path().join("main.rhai"),
            "fn on_custom_command(p) { return 1; }",
        )
        .unwrap();
        let inspection = inspect_plugin_package_internal(package.path()).unwrap();
        install_plugin_package_internal(
            app_data_dir,
            package.path(),
            &inspection.package_digest,
            registry,
        )
        .unwrap()
    }

    /// Persists `entry` as the only registry entry and reloads the registry from disk.
    fn reload_with_entry(
        app_data_dir: &Path,
        entry: InstalledPluginInfoV1,
    ) -> HashMap<String, InstalledPluginInfoV1> {
        let mut registry = HashMap::new();
        registry.insert(entry.id.clone(), entry);
        save_persisted_registry(app_data_dir, &registry).unwrap();
        load_persisted_registry(app_data_dir)
    }

    fn copy_package_files(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for name in ["manifest.json", "main.rhai"] {
            fs::write(to.join(name), fs::read(from.join(name)).unwrap()).unwrap();
        }
    }

    #[test]
    fn test_registry_load_accepts_valid_install_dir() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let info = install_fixture(app.path(), "valid-plugin", &mut registry);

        let loaded = reload_with_entry(app.path(), info);
        assert!(loaded.contains_key("valid-plugin"));
    }

    #[test]
    fn test_registry_load_rejects_corrupt_install_dir() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let victim = install_fixture(app.path(), "victim", &mut registry);
        let other = install_fixture(app.path(), "other", &mut registry);
        let installed_root = app.path().join("plugins").join("installed");
        let victim_dir = PathBuf::from(&victim.install_dir);

        // Directories that hold a digest-valid copy of the victim package but are not
        // plugins/installed/victim/1.0.0.
        let outside = tempdir().unwrap();
        copy_package_files(&victim_dir, outside.path());
        copy_package_files(&victim_dir, &installed_root);
        let sibling_version = installed_root.join("victim").join("2.0.0");
        copy_package_files(&victim_dir, &sibling_version);

        let corrupt_dirs = [
            outside.path().to_path_buf(),
            installed_root.clone(),
            sibling_version,
            victim_dir.join(".."),
            PathBuf::from(&other.install_dir),
            PathBuf::from("relative/plugins/installed/victim/1.0.0"),
            PathBuf::new(),
        ];
        for corrupt in corrupt_dirs {
            let mut entry = victim.clone();
            entry.install_dir = corrupt.to_string_lossy().to_string();
            let loaded = reload_with_entry(app.path(), entry);
            assert!(
                loaded.is_empty(),
                "install_dir {corrupt:?} must be rejected on registry load"
            );
        }
    }

    #[test]
    fn test_registry_load_rejects_path_component_identity_fields() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let victim = install_fixture(app.path(), "victim", &mut registry);

        for version in [".", "..", "1..2", ".1", "-1"] {
            let mut entry = victim.clone();
            entry.version = version.to_string();
            assert!(
                reload_with_entry(app.path(), entry).is_empty(),
                "registry version {version:?} must be rejected"
            );
        }
        for id in [".", "..", "a/b", ""] {
            let mut entry = victim.clone();
            entry.id = id.to_string();
            assert!(
                reload_with_entry(app.path(), entry).is_empty(),
                "registry id {id:?} must be rejected"
            );
        }
        for main_script in ["../main.rhai", "/etc/passwd", "other.rhai"] {
            let mut entry = victim.clone();
            entry.main_script = main_script.to_string();
            assert!(
                reload_with_entry(app.path(), entry).is_empty(),
                "registry main_script {main_script:?} must be rejected"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_registry_load_rejects_symlinked_install_dir() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let victim = install_fixture(app.path(), "victim", &mut registry);
        let other = install_fixture(app.path(), "other", &mut registry);
        let victim_dir = PathBuf::from(&victim.install_dir);

        // Replace victim/1.0.0 with a symlink to a digest-valid copy inside the installed root.
        let decoy = app
            .path()
            .join("plugins")
            .join("installed")
            .join("other")
            .join("decoy");
        copy_package_files(&victim_dir, &decoy);
        fs::remove_dir_all(&victim_dir).unwrap();
        std::os::unix::fs::symlink(&decoy, &victim_dir).unwrap();

        let loaded = reload_with_entry(app.path(), victim);
        assert!(loaded.is_empty());
        assert!(PathBuf::from(&other.install_dir)
            .join("main.rhai")
            .is_file());
    }

    #[test]
    fn test_uninstall_with_corrupt_install_dir_leaves_disk_untouched() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let victim = install_fixture(app.path(), "victim", &mut registry);
        let other = install_fixture(app.path(), "other", &mut registry);
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("keep.txt"), "keep").unwrap();

        for corrupt in [
            other.install_dir.clone(),
            outside
                .path()
                .join("keep.txt")
                .to_string_lossy()
                .to_string(),
            app.path()
                .join("plugins")
                .join("installed")
                .to_string_lossy()
                .to_string(),
        ] {
            let mut entry = victim.clone();
            entry.install_dir = corrupt.clone();
            registry.insert("victim".to_string(), entry);

            uninstall_plugin_package_internal(app.path(), "victim", &mut registry).unwrap();
            assert!(!registry.contains_key("victim"));
            assert!(
                PathBuf::from(&other.install_dir)
                    .join("main.rhai")
                    .is_file(),
                "uninstall with install_dir {corrupt:?} must not delete another plugin"
            );
            assert!(outside.path().join("keep.txt").is_file());
            assert!(PathBuf::from(&victim.install_dir)
                .join("main.rhai")
                .is_file());
        }

        // A valid entry is removed from disk together with its now-empty id directory.
        registry.insert("victim".to_string(), victim.clone());
        uninstall_plugin_package_internal(app.path(), "victim", &mut registry).unwrap();
        let victim_dir = PathBuf::from(&victim.install_dir);
        assert!(!victim_dir.exists());
        assert!(!victim_dir.parent().unwrap().exists());
    }

    #[test]
    fn test_uninstall_leaves_no_quarantine_behind() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let first = install_fixture(app.path(), "first", &mut registry);

        uninstall_plugin_package_internal(app.path(), "first", &mut registry).unwrap();
        assert!(!PathBuf::from(&first.install_dir).exists());
        let staging = app.path().join("plugins").join("staging");
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    }

    #[test]
    #[cfg(unix)]
    fn test_quarantine_refuses_symlinked_id_dir() {
        // Simulates the uninstall race: <id> swapped for a symlink after resolution.
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let victim = install_fixture(app.path(), "victim", &mut registry);
        let outside = tempdir().unwrap();
        let outside_version = outside.path().join("1.0.0");
        fs::create_dir(&outside_version).unwrap();
        fs::write(outside_version.join("keep.txt"), "keep").unwrap();

        let plugins = app.path().join("plugins").canonicalize().unwrap();
        let installed = plugins.join("installed");
        fs::remove_dir_all(installed.join("victim")).unwrap();
        std::os::unix::fs::symlink(outside.path(), installed.join("victim")).unwrap();

        let err = unix_dirfd::quarantine_plugin_dir(
            &installed,
            &plugins.join("staging"),
            &victim.id,
            &victim.version,
            "uninstall-test",
        )
        .unwrap_err();
        // Linux reports ELOOP for O_NOFOLLOW on a symlink; macOS reports ENOTDIR with O_DIRECTORY
        assert!(
            matches!(err.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)),
            "unexpected error: {err:?}"
        );
        assert!(outside_version.join("keep.txt").is_file());
        assert!(!plugins.join("staging").join("uninstall-test").exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_install_refuses_symlinked_staging_root() {
        let app = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let plugins = app.path().join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        std::os::unix::fs::symlink(outside.path(), plugins.join("staging")).unwrap();

        let package = tempdir().unwrap();
        fs::write(
            package.path().join("manifest.json"),
            r#"{"manifest_version": 1, "id": "p", "name": "P", "version": "1.0.0", "main_script": "main.rhai"}"#,
        )
        .unwrap();
        fs::write(package.path().join("main.rhai"), "fn x() {}").unwrap();
        let digest = inspect_plugin_package_internal(package.path())
            .unwrap()
            .package_digest;

        let mut registry = HashMap::new();
        let err =
            install_plugin_package_internal(app.path(), package.path(), &digest, &mut registry)
                .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginSymlinkRejected);
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(registry.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn test_load_drops_symlinked_staging_without_following_it() {
        let app = tempdir().unwrap();
        let mut registry = HashMap::new();
        let info = install_fixture(app.path(), "kept", &mut registry);
        save_persisted_registry(app.path(), &registry).unwrap();

        let outside = tempdir().unwrap();
        fs::write(outside.path().join("keep.txt"), "keep").unwrap();
        let staging = app.path().join("plugins").join("staging");
        fs::remove_dir_all(&staging).unwrap();
        std::os::unix::fs::symlink(outside.path(), &staging).unwrap();

        let loaded = load_persisted_registry(app.path());
        assert!(loaded.contains_key(&info.id));
        assert!(outside.path().join("keep.txt").is_file());
        assert!(
            fs::symlink_metadata(&staging).is_err(),
            "staging link must be removed"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_load_fails_closed_on_symlinked_plugins_root() {
        let app = tempdir().unwrap();
        let real = tempdir().unwrap();
        let mut registry = HashMap::new();
        install_fixture(real.path(), "elsewhere", &mut registry);
        save_persisted_registry(real.path(), &registry).unwrap();
        fs::create_dir_all(real.path().join("plugins").join("staging").join("leftover")).unwrap();

        std::os::unix::fs::symlink(real.path().join("plugins"), app.path().join("plugins"))
            .unwrap();

        assert!(load_persisted_registry(app.path()).is_empty());
        assert!(real
            .path()
            .join("plugins")
            .join("staging")
            .join("leftover")
            .is_dir());
    }
}
