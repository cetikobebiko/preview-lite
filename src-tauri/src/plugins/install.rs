//! AppData Private Installation Root, Staging, Atomic Publication, and Restart Persistence.

use super::error::{PluginError, PluginErrorCode};
use super::limits::{
    CURRENT_PLUGIN_CONSENT_POLICY_VERSION, CURRENT_PLUGIN_REGISTRY_SCHEMA_VERSION,
};
use super::manifest::{PluginManifestV1, PluginPermission};
use super::path_security::{
    compute_package_digest, compute_sha256_hex, create_private_dir_all,
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

    create_private_dir_all(&staging_root)?;
    create_private_dir_all(&installed_root)?;

    let staging_id = uuid::Uuid::new_v4().hyphenated().to_string();
    let staged_dir = staging_root.join(&staging_id);
    create_private_dir_all(&staged_dir)?;

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
    create_private_dir_all(&plugin_target_parent)?;
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
    let install_dir = PathBuf::from(&entry.install_dir);
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

    let installed_root = app_data_dir.join("plugins").join("installed");
    let plugin_dir = PathBuf::from(&entry.install_dir);

    // Verify containment inside installed_root before deleting
    if plugin_dir.exists() {
        if let Ok(canonical_dir) = plugin_dir.canonicalize() {
            if let Ok(canonical_root) = installed_root.canonicalize() {
                if canonical_dir.starts_with(&canonical_root) {
                    if let Ok(meta) = fs::symlink_metadata(&canonical_dir) {
                        if !meta.file_type().is_symlink() {
                            let _ = fs::remove_dir_all(&canonical_dir);
                        }
                    }
                }
            }
        }
    }

    // Also remove parent directory if empty
    if let Some(parent) = plugin_dir.parent() {
        if parent.exists() {
            let _ = fs::remove_dir(parent);
        }
    }

    save_persisted_registry(app_data_dir, current_registry)?;
    Ok(())
}

/// Loads the persisted plugin registry from `$APPDATA/plugins/registry.json` and cleans staging leftovers.
pub fn load_persisted_registry(app_data_dir: &Path) -> HashMap<String, InstalledPluginInfoV1> {
    let plugins_root = app_data_dir.join("plugins");
    let staging_root = plugins_root.join("staging");
    if staging_root.exists() {
        // Clean up orphaned staging leftovers from previous runs
        if let Ok(entries) = fs::read_dir(&staging_root) {
            for entry in entries.flatten() {
                let _ = fs::remove_dir_all(entry.path());
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

    let installed_root = plugins_root.join("installed");
    let mut valid_map = HashMap::new();

    for mut info in envelope.entries {
        let install_path = PathBuf::from(&info.install_dir);
        if !install_path.exists() {
            continue;
        }

        // Verify containment under installed root
        let canonical_install = match install_path.canonicalize() {
            Ok(c) => c,
            Err(_) => continue,
        };
        let canonical_root = match installed_root.canonicalize() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !canonical_install.starts_with(&canonical_root) {
            continue;
        }

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
    create_private_dir_all(&plugins_root)?;

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
