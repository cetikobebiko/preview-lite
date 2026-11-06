//! Modular Antigravity Plugin System (Prompt 9B V1 - Final Security Closure).

pub mod error;
pub mod host_api;
pub mod install;
pub mod limits;
pub mod manifest;
pub mod path_security;
pub mod registry;
pub mod worker;

use error::{PluginError, PluginErrorCode, PluginErrorWire};
use install::PluginInspectionResult;
use limits::*;
use manifest::{PluginHook, PluginPermission};
use path_security::compute_package_digest;
use registry::{
    ExecutionState, InstalledPluginInfoV1, PluginExecutionRegistry, PluginExecutionStartResult,
    PluginExecutionStatus, PluginHookRequestV1,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;
use worker::PluginWorkerLauncher;

type InstalledPlugins = Arc<RwLock<HashMap<String, InstalledPluginInfoV1>>>;

/// Hashes the installed manifest and script as they are on disk now.
fn installed_package_digest(
    app_data_dir: &Path,
    entry: &InstalledPluginInfoV1,
) -> Result<String, PluginError> {
    let install_dir = install::resolve_installed_plugin_dir(app_data_dir, entry)?;
    let read = |path: PathBuf| {
        fs::read(path).map_err(|_| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                "Eklenti disk dosyaları okunamadı.",
            )
        })
    };
    let manifest_bytes = read(install_dir.join("manifest.json"))?;
    let script_bytes = read(install_dir.join(&entry.main_script))?;
    Ok(compute_package_digest(&manifest_bytes, &script_bytes))
}

/// Logical quarantine: consent revoked and disabled; files stay in place.
fn quarantine_entry(entry: &mut InstalledPluginInfoV1) {
    entry.consent_granted = false;
    entry.consent_policy_version = 0;
    entry.enabled = false;
}

/// Re-hashes the installed files after a failed worker run and quarantines the
/// plugin when they no longer match the consented digest. Catches a change that
/// lands between the host pre-check and the worker's own revalidation.
async fn quarantine_if_tampered(
    app_data_dir: &Path,
    installed_plugins: &InstalledPlugins,
    plugin_id: &str,
) {
    let mut lock = installed_plugins.write().await;
    let tampered = match lock.get(plugin_id) {
        Some(entry) => match installed_package_digest(app_data_dir, entry) {
            Ok(digest) => digest != entry.package_digest,
            // Unreadable or relocated files are not the consented package either.
            Err(_) => true,
        },
        None => false,
    };
    if tampered {
        if let Some(entry) = lock.get_mut(plugin_id) {
            quarantine_entry(entry);
        }
        let _ = install::save_persisted_registry(app_data_dir, &lock);
    }
}

/// Top-level thread-safe state container for the plugin subsystem.
/// Build-profile gate decision for the plugin subsystem.
///
/// The gate's only source is the Rust build profile: development builds
/// (`debug_assertions`) open it, production builds keep it closed. There is no
/// webview-callable command that changes it.
pub const fn plugin_feature_gate_for_build(debug_assertions: bool) -> bool {
    debug_assertions
}

/// Gate value compiled into this binary.
pub const PLUGIN_FEATURE_ENABLED_IN_THIS_BUILD: bool =
    plugin_feature_gate_for_build(cfg!(debug_assertions));

/// Read-only gate observation returned to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginFeatureStatus {
    pub enabled: bool,
    /// `"development"` or `"production"`, matching the frontend registry vocabulary.
    pub build_profile: String,
}

/// Top-level thread-safe state container for the plugin subsystem.
#[derive(Clone)]
pub struct PluginState {
    app_data_dir: Option<PathBuf>,
    installed_plugins: Arc<RwLock<HashMap<String, InstalledPluginInfoV1>>>,
    execution_registry: PluginExecutionRegistry,
    feature_enabled: Arc<AtomicBool>,
    feature_gate_lock: Arc<RwLock<()>>,
    worker_launcher: Option<PluginWorkerLauncher>,
}

impl PluginState {
    /// Production constructor used by Tauri `setup`: the data directory comes from
    /// `app.path().app_data_dir()` and the gate from the build profile.
    pub fn for_app(app_data_dir: Option<PathBuf>) -> Self {
        let state = Self::with_app_data_dir_opt(app_data_dir);
        // Not shared yet, so no gate lock is needed for the initial value.
        state
            .feature_enabled
            .store(PLUGIN_FEATURE_ENABLED_IN_THIS_BUILD, Ordering::SeqCst);
        state
    }

    /// Runs plugin hooks through `executable --run-plugin-worker` instead of the
    /// current executable. Integration tests need this because their current
    /// executable is the test harness, not the app binary. The worker entrypoint
    /// argument is fixed; only the binary path changes.
    pub fn with_worker_executable(mut self, executable: PathBuf) -> Self {
        self.worker_launcher = Some(PluginWorkerLauncher::production_entrypoint(executable));
        self
    }

    pub fn with_app_data_dir(app_data_dir: PathBuf) -> Self {
        Self::with_app_data_dir_opt(Some(app_data_dir))
    }

    pub fn with_app_data_dir_opt(app_data_dir: Option<PathBuf>) -> Self {
        let loaded = match &app_data_dir {
            Some(dir) => install::load_persisted_registry(dir),
            None => HashMap::new(),
        };
        Self {
            app_data_dir,
            installed_plugins: Arc::new(RwLock::new(loaded)),
            execution_registry: PluginExecutionRegistry::new(),
            // Production Default is strictly DISABLED (fail-closed)
            feature_enabled: Arc::new(AtomicBool::new(false)),
            feature_gate_lock: Arc::new(RwLock::new(())),
            worker_launcher: None,
        }
    }

    fn require_app_data_dir(&self) -> Result<&PathBuf, PluginError> {
        self.app_data_dir.as_ref().ok_or_else(|| {
            PluginError::new(
                PluginErrorCode::PluginPathInvalid,
                "Özel uygulama verisi dizini çözümlenemedi.",
            )
        })
    }

    pub async fn set_feature_enabled(&self, enabled: bool) {
        let _guard = self.feature_gate_lock.write().await;
        self.feature_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn is_feature_enabled(&self) -> bool {
        self.feature_enabled.load(Ordering::SeqCst)
    }

    pub fn feature_status(&self) -> PluginFeatureStatus {
        PluginFeatureStatus {
            enabled: self.is_feature_enabled(),
            build_profile: if cfg!(debug_assertions) {
                "development"
            } else {
                "production"
            }
            .to_string(),
        }
    }

    pub fn app_data_dir(&self) -> Option<&Path> {
        self.app_data_dir.as_deref()
    }

    /// Read-only inspection of a candidate package directory.
    pub async fn inspect_package(
        &self,
        package_path: &str,
    ) -> Result<PluginInspectionResult, PluginErrorWire> {
        if !self.is_feature_enabled() {
            return Err(PluginError::feature_disabled().to_wire());
        }

        let path = Path::new(package_path);
        install::inspect_plugin_package_internal(path).map_err(|e| e.to_wire())
    }

    /// Installs a verified plugin package into the AppData private root with staging & atomic publication.
    pub async fn install_package(
        &self,
        package_path: &str,
        expected_digest: &str,
    ) -> Result<InstalledPluginInfoV1, PluginErrorWire> {
        let path = Path::new(package_path);

        // Linearization: acquire gate read lock
        let _gate = self.feature_gate_lock.read().await;
        if !self.is_feature_enabled() {
            return Err(PluginError::feature_disabled().to_wire());
        }

        let app_data_dir = self.require_app_data_dir().map_err(|e| e.to_wire())?;
        let mut lock = self.installed_plugins.write().await;
        install::install_plugin_package_internal(app_data_dir, path, expected_digest, &mut lock)
            .map_err(|e| e.to_wire())
    }

    /// Explicitly grants user review consent for an installed plugin.
    pub async fn grant_consent(
        &self,
        plugin_id: &str,
        expected_digest: &str,
        expected_permissions: &[PluginPermission],
    ) -> Result<InstalledPluginInfoV1, PluginErrorWire> {
        let _gate = self.feature_gate_lock.read().await;
        if !self.is_feature_enabled() {
            return Err(PluginError::feature_disabled().to_wire());
        }

        let app_data_dir = self.require_app_data_dir().map_err(|e| e.to_wire())?;
        let mut lock = self.installed_plugins.write().await;
        install::grant_plugin_consent_internal(
            app_data_dir,
            plugin_id,
            expected_digest,
            expected_permissions,
            &mut lock,
        )
        .map_err(|e| e.to_wire())
    }

    /// Safe uninstallation: cleans up disk and registry even if tampered or feature disabled.
    pub async fn uninstall_plugin(&self, plugin_id: &str) -> Result<(), PluginErrorWire> {
        let app_data_dir = self.require_app_data_dir().map_err(|e| e.to_wire())?;
        if self.execution_registry.is_plugin_busy(plugin_id).await {
            return Err(PluginError::new(
                PluginErrorCode::PluginUninstallBusy,
                format!(
                    "Eklenti şu anda aktif bir yürütmeye sahip olduğu için kaldırılamaz: '{}'",
                    plugin_id
                ),
            )
            .to_wire());
        }

        let mut lock = self.installed_plugins.write().await;
        install::uninstall_plugin_package_internal(app_data_dir, plugin_id, &mut lock)
            .map_err(|e| e.to_wire())
    }

    /// Enables or disables an installed plugin.
    /// Enabling is privilege-increasing (requires feature enabled and explicit consent);
    /// Disabling is privilege-reducing (allowed even if feature is disabled).
    pub async fn set_plugin_enabled(
        &self,
        plugin_id: &str,
        enabled: bool,
    ) -> Result<InstalledPluginInfoV1, PluginErrorWire> {
        let app_data_dir = self.require_app_data_dir().map_err(|e| e.to_wire())?;
        if enabled {
            let _gate = self.feature_gate_lock.read().await;
            if !self.is_feature_enabled() {
                return Err(PluginError::feature_disabled().to_wire());
            }

            let mut lock = self.installed_plugins.write().await;
            let entry = lock.get_mut(plugin_id).ok_or_else(|| {
                PluginError::new(
                    PluginErrorCode::PluginNotInstalled,
                    format!("Eklenti bulunamadı: '{}'", plugin_id),
                )
                .to_wire()
            })?;

            if !entry.consent_granted
                || entry.consent_policy_version != CURRENT_PLUGIN_CONSENT_POLICY_VERSION
            {
                return Err(PluginError::new(
                    PluginErrorCode::PluginConsentRequired,
                    "Eklenti etkinleştirilmeden önce kullanıcı onayı (consent) gereklidir.",
                )
                .to_wire());
            }

            // Verify disk integrity before enabling
            let disk_digest =
                installed_package_digest(app_data_dir, entry).map_err(|e| e.to_wire())?;
            if disk_digest != entry.package_digest {
                quarantine_entry(entry);
                let _ = install::save_persisted_registry(app_data_dir, &lock);
                return Err(PluginError::new(
                    PluginErrorCode::PluginPackageDigestMismatch,
                    "Eklenti disk dosyaları değiştirilmiş, etkinleştirme engellendi.",
                )
                .to_wire());
            }

            entry.enabled = true;
            let cloned = entry.clone();
            install::save_persisted_registry(app_data_dir, &lock).map_err(|e| e.to_wire())?;
            Ok(cloned)
        } else {
            // Privilege-reducing: allowed without gate lock
            let mut lock = self.installed_plugins.write().await;
            if let Some(entry) = lock.get_mut(plugin_id) {
                entry.enabled = false;
                let cloned = entry.clone();
                install::save_persisted_registry(app_data_dir, &lock).map_err(|e| e.to_wire())?;
                Ok(cloned)
            } else {
                Err(PluginError::new(
                    PluginErrorCode::PluginNotInstalled,
                    format!("Eklenti bulunamadı: '{}'", plugin_id),
                )
                .to_wire())
            }
        }
    }

    /// Read-only observation of installed plugins (works even when feature is disabled).
    pub async fn list_installed_plugins(&self) -> Vec<InstalledPluginInfoV1> {
        let lock = self.installed_plugins.read().await;
        lock.values().cloned().collect()
    }

    /// Initiates isolated plugin execution in subprocess worker.
    pub async fn start_execution(
        &self,
        plugin_id: &str,
        hook_name: &str,
        payload_json: &str,
        execution_id: &str,
    ) -> Result<PluginExecutionStartResult, PluginErrorWire> {
        let _gate = self.feature_gate_lock.read().await;
        if !self.is_feature_enabled() {
            return Err(PluginError::feature_disabled().to_wire());
        }

        let app_data_dir = self.require_app_data_dir().map_err(|e| e.to_wire())?;

        if payload_json.len() > MAX_PLUGIN_HOOK_PAYLOAD_BYTES {
            return Err(PluginError::new(
                PluginErrorCode::PluginPayloadTooLarge,
                format!(
                    "Girdi yükü boyutu {} byte; izin verilen üst sınır {} byte.",
                    payload_json.len(),
                    MAX_PLUGIN_HOOK_PAYLOAD_BYTES
                ),
            )
            .to_wire());
        }

        let parsed_payload: serde_json::Value =
            serde_json::from_str(payload_json).map_err(|e| {
                PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    format!("Kanca yükü geçerli JSON değil: {}", e),
                )
                .to_wire()
            })?;

        let hook = match hook_name {
            "on_document_open" => PluginHook::OnDocumentOpen,
            "on_custom_command" => PluginHook::OnCustomCommand,
            "on_export_complete" => PluginHook::OnExportComplete,
            _ => {
                return Err(PluginError::new(
                    PluginErrorCode::PluginHookUnknown,
                    format!("Bilinmeyen veya desteklenmeyen kanca adı: '{}'", hook_name),
                )
                .to_wire());
            }
        };

        // Check if plugin is installed, consented, and enabled
        let plugin = {
            let lock = self.installed_plugins.read().await;
            lock.get(plugin_id).cloned().ok_or_else(|| {
                PluginError::new(
                    PluginErrorCode::PluginNotInstalled,
                    format!("Eklenti bulunamadı: '{}'", plugin_id),
                )
                .to_wire()
            })?
        };

        if !plugin.consent_granted
            || plugin.consent_policy_version != CURRENT_PLUGIN_CONSENT_POLICY_VERSION
        {
            return Err(PluginError::new(
                PluginErrorCode::PluginConsentRequired,
                format!("Eklenti ('{}') için kullanıcı onayı verilmemiş.", plugin_id),
            )
            .to_wire());
        }

        if !plugin.enabled {
            return Err(PluginError::new(
                PluginErrorCode::PluginNotEnabled,
                format!("Eklenti etkin değil: '{}'", plugin_id),
            )
            .to_wire());
        }

        if !plugin.hooks.contains(&hook) {
            return Err(PluginError::new(
                PluginErrorCode::PluginHookNotDeclared,
                format!("Kanca '{}' eklenti manifestinde bildirilmemiş.", hook_name),
            )
            .to_wire());
        }

        // Invariant: single active execution per plugin
        if self.execution_registry.is_plugin_busy(plugin_id).await {
            return Err(PluginError::new(
                PluginErrorCode::PluginExecutionBusy,
                format!(
                    "Eklenti ('{}') için zaten devam eden bir yürütme var.",
                    plugin_id
                ),
            )
            .to_wire());
        }

        // Pre-check disk digest before starting execution
        let disk_digest = installed_package_digest(app_data_dir, &plugin);
        if disk_digest.as_deref().ok() != Some(plugin.package_digest.as_str()) {
            // Quarantine tampered (or unreadable) plugin
            let mut lock = self.installed_plugins.write().await;
            if let Some(entry) = lock.get_mut(plugin_id) {
                quarantine_entry(entry);
                let _ = install::save_persisted_registry(app_data_dir, &lock);
            }
            return Err(disk_digest
                .err()
                .unwrap_or_else(|| {
                    PluginError::new(
                        PluginErrorCode::PluginPackageDigestMismatch,
                        "Eklenti disk dosyaları değiştirilmiş, yürütme engellendi.",
                    )
                })
                .to_wire());
        }

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let exec_id_str = execution_id.to_string();
        let plugin_id_str = plugin_id.to_string();
        let hook_name_str = hook_name.to_string();

        let hook_req = PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: exec_id_str.clone(),
            plugin_id: plugin_id_str.clone(),
            hook,
            payload: parsed_payload,
        };

        let app_data_dir_clone = app_data_dir.clone();

        self.execution_registry
            .register_execution(
                exec_id_str.clone(),
                plugin_id_str.clone(),
                hook_name_str.clone(),
                cancel_tx,
            )
            .await;

        let registry_clone = self.execution_registry.clone();
        let exec_id_for_task = exec_id_str.clone();
        let plugin_version = plugin.version.clone();
        let expected_package_digest = plugin.package_digest.clone();
        let expected_manifest_digest = plugin.manifest_digest.clone();
        let expected_script_digest = plugin.script_digest.clone();
        let granted_permissions = plugin.permissions.clone();
        let installed_plugins = self.installed_plugins.clone();
        let launcher = match &self.worker_launcher {
            Some(launcher) => Ok(launcher.clone()),
            None => PluginWorkerLauncher::current_executable(),
        };

        // Spawn async supervisor task
        tokio::spawn(async move {
            let res = match launcher {
                Ok(launcher) => {
                    worker::supervise_plugin_execution_with_launcher(
                        launcher,
                        &app_data_dir_clone,
                        &plugin_id_str,
                        &plugin_version,
                        &expected_package_digest,
                        &expected_manifest_digest,
                        &expected_script_digest,
                        hook_req,
                        granted_permissions,
                        cancel_rx,
                    )
                    .await
                }
                Err(err) => Err(err),
            };

            if res.is_err() {
                quarantine_if_tampered(&app_data_dir_clone, &installed_plugins, &plugin_id_str)
                    .await;
            }

            match res {
                Ok(hook_result) => {
                    registry_clone
                        .finish_success(&exec_id_for_task, hook_result)
                        .await;
                }
                Err(err) => {
                    let state = match err.code {
                        PluginErrorCode::PluginExecutionCancelled => ExecutionState::Cancelled,
                        PluginErrorCode::PluginExecutionTimeout => ExecutionState::TimedOut,
                        _ => ExecutionState::Failed,
                    };
                    registry_clone
                        .finish_error(&exec_id_for_task, state, err.to_wire())
                        .await;
                }
            }
        });

        Ok(PluginExecutionStartResult {
            execution_id: exec_id_str,
            plugin_id: plugin.id,
            state: ExecutionState::Running,
        })
    }

    /// Cancels an active execution (safe even if feature is disabled).
    pub async fn cancel_execution(&self, execution_id: &str) -> Result<(), PluginErrorWire> {
        let success = self.execution_registry.cancel_execution(execution_id).await;
        if success {
            Ok(())
        } else {
            Err(PluginError::new(
                PluginErrorCode::PluginExecutionNotFound,
                format!(
                    "İptal edilecek aktif yürütme bulunamadı: '{}'",
                    execution_id
                ),
            )
            .to_wire())
        }
    }

    /// Queries execution status (observation, safe even if feature is disabled).
    pub async fn get_execution_status(
        &self,
        execution_id: &str,
    ) -> Result<PluginExecutionStatus, PluginErrorWire> {
        self.execution_registry
            .get_status(execution_id)
            .await
            .ok_or_else(|| {
                PluginError::new(
                    PluginErrorCode::PluginExecutionNotFound,
                    format!("Yürütme durumu bulunamadı: '{}'", execution_id),
                )
                .to_wire()
            })
    }
}

// ---------------------------------------------------------------------------
// Tauri Command Handlers
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn inspect_plugin_package(
    package_path: String,
    state: State<'_, PluginState>,
) -> Result<PluginInspectionResult, PluginErrorWire> {
    state.inspect_package(&package_path).await
}

#[tauri::command]
pub async fn install_plugin_package(
    package_path: String,
    expected_digest: String,
    state: State<'_, PluginState>,
) -> Result<InstalledPluginInfoV1, PluginErrorWire> {
    state.install_package(&package_path, &expected_digest).await
}

#[tauri::command]
pub async fn grant_plugin_consent(
    plugin_id: String,
    expected_package_digest: String,
    expected_permissions: Vec<PluginPermission>,
    state: State<'_, PluginState>,
) -> Result<InstalledPluginInfoV1, PluginErrorWire> {
    state
        .grant_consent(&plugin_id, &expected_package_digest, &expected_permissions)
        .await
}

#[tauri::command]
pub async fn uninstall_plugin(
    plugin_id: String,
    state: State<'_, PluginState>,
) -> Result<(), PluginErrorWire> {
    state.uninstall_plugin(&plugin_id).await
}

#[tauri::command]
pub async fn set_plugin_enabled(
    plugin_id: String,
    enabled: bool,
    state: State<'_, PluginState>,
) -> Result<InstalledPluginInfoV1, PluginErrorWire> {
    state.set_plugin_enabled(&plugin_id, enabled).await
}

#[tauri::command]
pub async fn list_installed_plugins(
    state: State<'_, PluginState>,
) -> Result<Vec<InstalledPluginInfoV1>, PluginErrorWire> {
    Ok(state.list_installed_plugins().await)
}

/// Read-only: reports the build-profile gate. There is deliberately no command that
/// changes it.
#[tauri::command]
pub async fn get_plugin_feature_status(
    state: State<'_, PluginState>,
) -> Result<PluginFeatureStatus, PluginErrorWire> {
    Ok(state.feature_status())
}

#[tauri::command]
pub async fn start_plugin_execution(
    plugin_id: String,
    hook_name: String,
    payload_json: String,
    execution_id: String,
    state: State<'_, PluginState>,
) -> Result<PluginExecutionStartResult, PluginErrorWire> {
    state
        .start_execution(&plugin_id, &hook_name, &payload_json, &execution_id)
        .await
}

#[tauri::command]
pub async fn cancel_plugin_execution(
    execution_id: String,
    state: State<'_, PluginState>,
) -> Result<(), PluginErrorWire> {
    state.cancel_execution(&execution_id).await
}

#[tauri::command]
pub async fn get_plugin_execution_status(
    execution_id: String,
    state: State<'_, PluginState>,
) -> Result<PluginExecutionStatus, PluginErrorWire> {
    state.get_execution_status(&execution_id).await
}

// ---------------------------------------------------------------------------
// Comprehensive Unit and Security Acceptance Test Suite (§39.1 - §39.11)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use manifest::PluginManifestV1;
    use registry::PluginHookResultV1;
    use tempfile::tempdir;

    fn make_valid_manifest_json(id: &str) -> String {
        format!(
            r#"{{
            "manifest_version": 1,
            "id": "{id}",
            "name": "Test Plugin",
            "version": "1.0.0",
            "description": "A secure test plugin",
            "main_script": "main.rhai",
            "permissions": ["ui:notification"],
            "hooks": ["on_custom_command", "on_document_open"]
        }}"#
        )
    }

    fn setup_enabled_state_with_temp_dir() -> (PluginState, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        state.feature_enabled.store(true, Ordering::SeqCst);
        (state, dir)
    }

    // -----------------------------------------------------------------------
    // 1. Backend Feature Gate & Privilege-Direction Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_disabled_default_state_is_fail_closed() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        assert!(!state.is_feature_enabled());
    }

    #[tokio::test]
    async fn test_disabled_start_execution_rejected() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state
            .start_execution("test", "on_custom_command", "{}", "exec-1")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_FEATURE_DISABLED");
    }

    #[tokio::test]
    async fn test_disabled_install_package_rejected() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state
            .install_package("/dummy/path", "dummy_digest")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_FEATURE_DISABLED");
    }

    #[tokio::test]
    async fn test_disabled_grant_consent_rejected() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state
            .grant_consent("test", "dummy", &[PluginPermission::PluginLog])
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_FEATURE_DISABLED");
    }

    #[tokio::test]
    async fn test_disabled_enable_true_rejected() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state.set_plugin_enabled("test", true).await.unwrap_err();
        assert_eq!(err.code, "PLUGIN_FEATURE_DISABLED");
    }

    #[tokio::test]
    async fn test_disabled_inspect_package_rejected() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state.inspect_package("/dummy/path").await.unwrap_err();
        assert_eq!(err.code, "PLUGIN_FEATURE_DISABLED");
    }

    #[tokio::test]
    async fn test_disabled_disable_false_succeeds() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        // For non-existent, returns PluginNotInstalled, but does NOT return PluginFeatureDisabled!
        let err = state.set_plugin_enabled("test", false).await.unwrap_err();
        assert_eq!(err.code, "PLUGIN_NOT_INSTALLED");
    }

    #[tokio::test]
    async fn test_disabled_cancel_active_execution_succeeds() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state
            .cancel_execution("non-existent-exec")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_EXECUTION_NOT_FOUND");
    }

    #[tokio::test]
    async fn test_disabled_uninstall_succeeds() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state.uninstall_plugin("non-existent").await.unwrap_err();
        assert_eq!(err.code, "PLUGIN_NOT_INSTALLED");
    }

    #[tokio::test]
    async fn test_disabled_get_status_succeeds() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let err = state
            .get_execution_status("non-existent")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_EXECUTION_NOT_FOUND");
    }

    #[tokio::test]
    async fn test_disabled_list_installed_plugins_succeeds() {
        let dir = tempdir().unwrap();
        let state = PluginState::with_app_data_dir(dir.path().to_path_buf());
        let list = state.list_installed_plugins().await;
        assert!(list.is_empty());
    }

    // -----------------------------------------------------------------------
    // 2. Package V1, Filesystem & Hardlink Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_manifest_v1_valid_parsing() {
        let json = make_valid_manifest_json("my-test-plugin");
        let manifest =
            PluginManifestV1::parse_and_validate(json.as_bytes()).expect("Valid manifest");
        assert_eq!(manifest.id, "my-test-plugin");
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(manifest.permissions.len(), 1);
        assert_eq!(manifest.main_script, "main.rhai");
    }

    #[test]
    fn test_manifest_v1_rejects_unknown_fields() {
        let json = r#"{
            "manifest_version": 1,
            "id": "test",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai",
            "unauthorized_field": true
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestUnknownField);
    }

    #[test]
    fn test_manifest_v1_rejects_duplicate_keys() {
        let json = r#"{
            "manifest_version": 1,
            "id": "test1",
            "id": "test2",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_manifest_v1_rejects_invalid_id() {
        let json = r#"{
            "manifest_version": 1,
            "id": "INVALID/ID!",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginIdInvalid);
    }

    #[test]
    fn test_manifest_v1_rejects_unknown_permissions() {
        let json = r#"{
            "manifest_version": 1,
            "id": "test-plugin",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai",
            "permissions": ["os:exec_raw_command"]
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPermissionUnknown);
    }

    #[test]
    fn test_manifest_v1_main_script_must_be_main_rhai() {
        let json = r#"{
            "manifest_version": 1,
            "id": "test-plugin",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "other.rhai"
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_manifest_v1_strips_utf8_bom() {
        let raw_json = make_valid_manifest_json("bom-test");
        let mut bom_bytes = vec![0xEF, 0xBB, 0xBF];
        bom_bytes.extend_from_slice(raw_json.as_bytes());

        let manifest =
            PluginManifestV1::parse_and_validate(&bom_bytes).expect("Should strip BOM and parse");
        assert_eq!(manifest.id, "bom-test");
    }

    #[test]
    fn test_package_extra_unsupported_files_rejected() {
        let dir = tempdir().unwrap();
        let manifest_content = make_valid_manifest_json("extra-file-test");
        fs::write(dir.path().join("manifest.json"), manifest_content).unwrap();
        fs::write(dir.path().join("main.rhai"), "fn on_custom_command(p) {}").unwrap();
        fs::write(dir.path().join("unauthorized.dll"), b"MZ").unwrap();

        let err = path_security::scan_package_directory_safety(dir.path()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    #[cfg(unix)]
    fn test_package_hardlink_rejected() {
        let dir = tempdir().unwrap();
        let manifest_content = make_valid_manifest_json("hardlink-test");
        let manifest_path = dir.path().join("manifest.json");
        fs::write(&manifest_path, manifest_content).unwrap();
        fs::write(dir.path().join("main.rhai"), "fn on_custom_command(p) {}").unwrap();

        // Create hardlink to main.rhai
        let hardlink_path = dir.path().join("extra_hardlink.txt");
        let _ = fs::hard_link(dir.path().join("main.rhai"), hardlink_path);

        let err = path_security::scan_package_directory_safety(dir.path()).unwrap_err();
        assert!(
            err.code == PluginErrorCode::PluginHardlinkRejected
                || err.code == PluginErrorCode::PluginManifestInvalid
        );
    }

    #[test]
    fn test_path_security_rejects_traversal() {
        let dir = tempdir().unwrap();
        let err =
            path_security::validate_and_canonicalize_contained_path(dir.path(), "../outside.rhai")
                .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPathTraversal);
    }

    // -----------------------------------------------------------------------
    // 3. Consent, Installation, and Persistence Lifecycle Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_plugin_installation_consent_and_persistence_lifecycle() {
        let (state, _app_data_dir) = setup_enabled_state_with_temp_dir();
        let package_temp = tempdir().unwrap();

        let manifest_content = make_valid_manifest_json("test-counter");
        let script_content = r#"
            fn on_custom_command(payload) {
                plugin_log("Executing counter hook");
                return "counted_42";
            }
        "#;
        fs::write(package_temp.path().join("manifest.json"), &manifest_content).unwrap();
        fs::write(package_temp.path().join("main.rhai"), script_content).unwrap();

        // 1. Inspect
        let inspection = state
            .inspect_package(&package_temp.path().to_string_lossy())
            .await
            .expect("Inspection should succeed");
        assert_eq!(inspection.manifest.id, "test-counter");

        // 2. Install
        let installed = state
            .install_package(
                &package_temp.path().to_string_lossy(),
                &inspection.package_digest,
            )
            .await
            .expect("Installation should succeed");
        assert_eq!(installed.id, "test-counter");
        assert!(!installed.enabled);
        assert!(!installed.consent_granted);

        // 3. Enabling before consent MUST fail
        let enable_err = state
            .set_plugin_enabled("test-counter", true)
            .await
            .unwrap_err();
        assert_eq!(enable_err.code, "PLUGIN_CONSENT_REQUIRED");

        // 4. Grant consent with matching digest and permissions
        let consented = state
            .grant_consent(
                "test-counter",
                &inspection.package_digest,
                &[PluginPermission::UiNotification],
            )
            .await
            .expect("Grant consent should succeed");
        assert!(consented.consent_granted);
        assert_eq!(consented.consent_policy_version, 1);

        // 5. Now enabling succeeds
        let enabled = state
            .set_plugin_enabled("test-counter", true)
            .await
            .expect("Should enable after consent");
        assert!(enabled.enabled);

        // 6. Verify restart persistence
        let restored_state = PluginState::with_app_data_dir_opt(state.app_data_dir.clone());
        let list = restored_state.list_installed_plugins().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "test-counter");
        assert!(list[0].enabled);
        assert!(list[0].consent_granted);

        // 7. Uninstall
        restored_state
            .uninstall_plugin("test-counter")
            .await
            .expect("Uninstall should succeed");
        let empty_list = restored_state.list_installed_plugins().await;
        assert!(empty_list.is_empty());
    }

    #[tokio::test]
    async fn test_digest_mismatch_fails_installation() {
        let (state, _) = setup_enabled_state_with_temp_dir();
        let package_temp = tempdir().unwrap();

        let manifest_content = make_valid_manifest_json("digest-fail");
        fs::write(package_temp.path().join("manifest.json"), manifest_content).unwrap();
        fs::write(
            package_temp.path().join("main.rhai"),
            "fn on_custom_command(p) { return 1; }",
        )
        .unwrap();

        let err = state
            .install_package(
                &package_temp.path().to_string_lossy(),
                "invalid_sha256_digest_00000000000000000000000000000000",
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_PACKAGE_DIGEST_MISMATCH");
    }

    #[tokio::test]
    async fn test_uninstall_tampered_plugin_succeeds_safely() {
        let (state, _) = setup_enabled_state_with_temp_dir();
        let package_temp = tempdir().unwrap();

        let manifest_content = make_valid_manifest_json("tamper-uninstall");
        fs::write(package_temp.path().join("manifest.json"), &manifest_content).unwrap();
        fs::write(
            package_temp.path().join("main.rhai"),
            "fn on_custom_command(p) { return 1; }",
        )
        .unwrap();

        let inspection = state
            .inspect_package(&package_temp.path().to_string_lossy())
            .await
            .unwrap();

        let installed = state
            .install_package(
                &package_temp.path().to_string_lossy(),
                &inspection.package_digest,
            )
            .await
            .unwrap();

        // Tamper disk file
        let script_file = PathBuf::from(&installed.install_dir).join("main.rhai");
        fs::write(&script_file, "malicious tampered content").unwrap();

        // Uninstalling tampered plugin must succeed safely
        state
            .uninstall_plugin("tamper-uninstall")
            .await
            .expect("Uninstalling tampered plugin must succeed");
        assert!(state.list_installed_plugins().await.is_empty());
    }

    // -----------------------------------------------------------------------
    // 4. Rhai Sandbox Resource Limits & Host API Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rhai_infinite_loop_hits_op_limit() {
        let context = host_api::HostApiContext::default();
        let engine = host_api::build_sandboxed_rhai_engine(context);

        let bad_script = "while true { let x = 1; }";
        let res = engine.eval::<()>(bad_script);
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("operation"));
    }

    #[test]
    fn test_rhai_no_blob_no_eval_no_file() {
        let context = host_api::HostApiContext::default();
        let engine = host_api::build_sandboxed_rhai_engine(context);

        // Blob type must NOT be present
        let blob_script = "let b = blob(10);";
        assert!(engine.eval::<()>(blob_script).is_err());

        // Import must NOT be present
        let import_script = "import \"something\" as s;";
        assert!(engine.eval::<()>(import_script).is_err());
    }

    #[test]
    fn test_host_api_permission_enforcement() {
        let script = r#"
            fn on_custom_command(payload) {
                plugin_log("testing log permission");
                return "logged";
            }
        "#;

        // Without plugin:log permission -> Error
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });
        let err = host_api::execute_plugin_script(
            script,
            PluginHook::OnCustomCommand,
            &payload,
            &[PluginPermission::UiNotification],
            "exec-perm",
            cancel_flag.clone(),
        )
        .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPermissionDenied);

        // With plugin:log permission -> Success
        let ok = host_api::execute_plugin_script(
            script,
            PluginHook::OnCustomCommand,
            &payload,
            &[PluginPermission::PluginLog],
            "exec-perm",
            cancel_flag,
        )
        .expect("Should execute with log permission");
        assert_eq!(ok.text, Some("logged".to_string()));
        assert_eq!(ok.logs.len(), 1);
    }

    #[tokio::test]
    async fn test_single_active_execution_busy_invariant() {
        let registry = PluginExecutionRegistry::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        registry
            .register_execution(
                "exec-1".to_string(),
                "busy-plugin".to_string(),
                "hook".to_string(),
                tx,
            )
            .await;

        assert!(registry.is_plugin_busy("busy-plugin").await);
        assert!(!registry.is_plugin_busy("other-plugin").await);

        let hook_res = PluginHookResultV1 {
            schema_version: 1,
            execution_id: "exec-1".to_string(),
            text: Some("ok".to_string()),
            logs: vec![],
            notifications: vec![],
            warnings: vec![],
        };
        registry.finish_success("exec-1", hook_res).await;
        assert!(!registry.is_plugin_busy("busy-plugin").await);
    }

    #[test]
    fn test_manifest_v1_rejects_duplicate_permissions() {
        let json = r#"{
            "manifest_version": 1,
            "id": "dup-perm",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai",
            "permissions": ["ui:notification", "ui:notification"]
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_manifest_v1_rejects_duplicate_hooks() {
        let json = r#"{
            "manifest_version": 1,
            "id": "dup-hook",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai",
            "hooks": ["on_custom_command", "on_custom_command"]
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_manifest_v1_oversized_rejected() {
        let mut large = make_valid_manifest_json("oversized");
        large.push_str(&" ".repeat(70 * 1024));
        let err = PluginManifestV1::parse_and_validate(large.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestTooLarge);
    }

    #[test]
    fn test_package_digest_domain_separated_and_ambiguity_free() {
        let m1 = b"{\"id\":\"a\"}";
        let s1 = b"fn run() {}";
        let d1 = path_security::compute_package_digest(m1, s1);

        let m2 = b"{\"id\":\"b\"}";
        let s2 = b"fn run() {}";
        let d2 = path_security::compute_package_digest(m2, s2);

        assert_ne!(d1, d2);
        assert_eq!(d1.len(), 64);
    }

    #[test]
    fn test_path_relative_main_script_accepted() {
        let dir = tempdir().unwrap();
        let main_rhai = dir.path().join("main.rhai");
        fs::write(&main_rhai, "fn main() {}").unwrap();
        let res = path_security::validate_and_canonicalize_contained_path(dir.path(), "main.rhai");
        assert!(res.is_ok());
    }

    #[test]
    fn test_path_absolute_rejected() {
        let dir = tempdir().unwrap();
        let err =
            path_security::validate_and_canonicalize_contained_path(dir.path(), "/etc/passwd")
                .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPathInvalid);
    }

    #[test]
    fn test_path_parent_traversal_rejected() {
        let dir = tempdir().unwrap();
        let err = path_security::validate_and_canonicalize_contained_path(
            dir.path(),
            "../something.rhai",
        )
        .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPathTraversal);
    }

    #[test]
    fn test_hook_document_open_has_no_file_name_leak() {
        // Valid payload
        let valid_payload = serde_json::json!({ "page_count": 5 });
        assert!(
            host_api::validate_hook_payload(PluginHook::OnDocumentOpen, &valid_payload).is_ok()
        );

        // Forbidden fields like file_name, file_path, session_id must be rejected
        let leaked_payload = serde_json::json!({ "page_count": 5, "file_name": "secret.pdf" });
        let err = host_api::validate_hook_payload(PluginHook::OnDocumentOpen, &leaked_payload)
            .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_hook_document_open_rejects_unexpected_fields() {
        let bad_payload = serde_json::json!({ "page_count": 5, "credential": "password" });
        let err =
            host_api::validate_hook_payload(PluginHook::OnDocumentOpen, &bad_payload).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_payload_oversized_rejected() {
        let huge_str = "x".repeat(300 * 1024);
        let huge_payload =
            serde_json::json!({ "command_name": "cmd", "parameters": { "data": huge_str } });
        let err = host_api::validate_hook_payload(PluginHook::OnCustomCommand, &huge_payload)
            .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPayloadTooLarge);
    }

    #[test]
    fn test_payload_deep_nesting_rejected() {
        let mut nested = serde_json::json!("leaf");
        for _ in 0..12 {
            nested = serde_json::json!([nested]);
        }
        let deep_payload = serde_json::json!({ "command_name": "cmd", "parameters": nested });
        let err = host_api::validate_hook_payload(PluginHook::OnCustomCommand, &deep_payload)
            .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginPayloadTooLarge);
    }

    #[test]
    fn test_rhai_max_variables_limit() {
        let context = host_api::HostApiContext::default();
        let engine = host_api::build_sandboxed_rhai_engine(context);

        // Define more than 1000 variables
        let mut script = String::new();
        for i in 0..1050 {
            script.push_str(&format!("let x{} = {};\n", i, i));
        }
        let res = engine.eval::<()>(&script);
        assert!(res.is_err());
    }

    #[test]
    fn test_rhai_deep_recursion_hits_call_stack_limit() {
        let context = host_api::HostApiContext::default();
        let engine = host_api::build_sandboxed_rhai_engine(context);

        let script = r#"
            fn recurse(n) {
                if n <= 0 { return 0; }
                return recurse(n - 1) + 1;
            }
            recurse(100);
        "#;
        let res = engine.eval::<()>(script);
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("stack"));
    }

    #[test]
    fn test_structured_result_unit_return_accepted() {
        let script = r#"
            fn on_custom_command(payload) {
                // Returns ()
            }
        "#;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });
        let ok = host_api::execute_plugin_script(
            script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-unit",
            cancel_flag,
        )
        .expect("Unit return should be accepted");
        assert_eq!(ok.text, None);
    }

    #[test]
    fn test_structured_result_string_return_accepted() {
        let script = r#"
            fn on_custom_command(payload) {
                return "hello world";
            }
        "#;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });
        let ok = host_api::execute_plugin_script(
            script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-str",
            cancel_flag,
        )
        .expect("String return should be accepted");
        assert_eq!(ok.text, Some("hello world".to_string()));
    }

    #[test]
    fn test_structured_result_array_or_map_return_rejected() {
        let script = r#"
            fn on_custom_command(payload) {
                return [1, 2, 3];
            }
        "#;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });
        let err = host_api::execute_plugin_script(
            script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-arr",
            cancel_flag,
        )
        .unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginResultInvalid);
    }

    #[tokio::test]
    async fn test_app_data_dir_resolution_failure_is_fail_closed() {
        let state = PluginState::with_app_data_dir_opt(None);
        state.feature_enabled.store(true, Ordering::SeqCst);

        // install_package fails closed with PluginPathInvalid
        let err1 = state
            .install_package("/some/path", "dummy_digest")
            .await
            .unwrap_err();
        assert_eq!(err1.code, "PLUGIN_PATH_INVALID");

        // grant_consent fails closed
        let err2 = state
            .grant_consent("test", "digest", &[])
            .await
            .unwrap_err();
        assert_eq!(err2.code, "PLUGIN_PATH_INVALID");

        // start_execution fails closed
        let err3 = state
            .start_execution("test", "on_custom_command", "{}", "exec-1")
            .await
            .unwrap_err();
        assert_eq!(err3.code, "PLUGIN_PATH_INVALID");

        // uninstall fails closed
        let err4 = state.uninstall_plugin("test").await.unwrap_err();
        assert_eq!(err4.code, "PLUGIN_PATH_INVALID");
    }

    #[tokio::test]
    async fn test_consent_reset_on_package_digest_change() {
        let (state, _) = setup_enabled_state_with_temp_dir();
        let package_temp = tempdir().unwrap();

        let manifest_content = make_valid_manifest_json("digest-reset-test");
        fs::write(package_temp.path().join("manifest.json"), &manifest_content).unwrap();
        fs::write(
            package_temp.path().join("main.rhai"),
            "fn on_custom_command(p) {}",
        )
        .unwrap();

        let insp = state
            .inspect_package(&package_temp.path().to_string_lossy())
            .await
            .unwrap();
        let installed = state
            .install_package(&package_temp.path().to_string_lossy(), &insp.package_digest)
            .await
            .unwrap();
        state
            .grant_consent(
                "digest-reset-test",
                &insp.package_digest,
                &[PluginPermission::UiNotification],
            )
            .await
            .unwrap();
        state
            .set_plugin_enabled("digest-reset-test", true)
            .await
            .unwrap();

        // Now tamper main.rhai on disk
        let disk_script = PathBuf::from(&installed.install_dir).join("main.rhai");
        fs::write(
            &disk_script,
            "fn on_custom_command(p) { return \"tampered\"; }",
        )
        .unwrap();

        // Reload persisted registry
        let loaded = install::load_persisted_registry(state.app_data_dir.as_ref().unwrap());
        let entry = loaded.get("digest-reset-test").unwrap();
        assert!(!entry.consent_granted);
        assert_eq!(entry.consent_policy_version, 0);
        assert!(!entry.enabled);
    }

    #[tokio::test]
    async fn test_consent_reset_on_permission_set_change() {
        let (state, _) = setup_enabled_state_with_temp_dir();
        let package_temp = tempdir().unwrap();

        let manifest_content = make_valid_manifest_json("perm-reset-test");
        fs::write(package_temp.path().join("manifest.json"), &manifest_content).unwrap();
        fs::write(
            package_temp.path().join("main.rhai"),
            "fn on_custom_command(p) {}",
        )
        .unwrap();

        let insp = state
            .inspect_package(&package_temp.path().to_string_lossy())
            .await
            .unwrap();
        state
            .install_package(&package_temp.path().to_string_lossy(), &insp.package_digest)
            .await
            .unwrap();

        // Grant consent with mismatched permission set must fail
        let err = state
            .grant_consent(
                "perm-reset-test",
                &insp.package_digest,
                &[PluginPermission::PluginLog], // Mismatched: manifest has UiNotification
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn test_modified_script_on_disk_blocks_execution() {
        let (state, _) = setup_enabled_state_with_temp_dir();
        let package_temp = tempdir().unwrap();

        let manifest_content = make_valid_manifest_json("tamper-exec-test");
        fs::write(package_temp.path().join("manifest.json"), &manifest_content).unwrap();
        fs::write(
            package_temp.path().join("main.rhai"),
            "fn on_custom_command(p) {}",
        )
        .unwrap();

        let insp = state
            .inspect_package(&package_temp.path().to_string_lossy())
            .await
            .unwrap();
        let installed = state
            .install_package(&package_temp.path().to_string_lossy(), &insp.package_digest)
            .await
            .unwrap();
        state
            .grant_consent(
                "tamper-exec-test",
                &insp.package_digest,
                &[PluginPermission::UiNotification],
            )
            .await
            .unwrap();
        state
            .set_plugin_enabled("tamper-exec-test", true)
            .await
            .unwrap();

        // Modify disk script
        let disk_script = PathBuf::from(&installed.install_dir).join("main.rhai");
        fs::write(
            &disk_script,
            "fn on_custom_command(p) { return \"tampered\"; }",
        )
        .unwrap();

        // Attempting to re-enable or validate detects mismatch
        let err = state
            .set_plugin_enabled("tamper-exec-test", true)
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_PACKAGE_DIGEST_MISMATCH");
    }

    #[test]
    fn test_plugin_test_helper_feature_isolation_invariants() {
        // Under default builds, PluginWorkerLauncher only points to current_executable with --run-plugin-worker
        let launcher = worker::PluginWorkerLauncher::current_executable().unwrap();
        assert_eq!(
            launcher.arguments,
            vec![std::ffi::OsString::from("--run-plugin-worker")]
        );
    }

    // -----------------------------------------------------------------------
    // 5. Build-profile gate (5a)
    // -----------------------------------------------------------------------

    #[test]
    fn test_gate_decision_closed_for_production_profile() {
        assert!(!plugin_feature_gate_for_build(false));
        assert!(plugin_feature_gate_for_build(true));
    }

    #[test]
    fn test_gate_constant_follows_debug_assertions() {
        assert_eq!(PLUGIN_FEATURE_ENABLED_IN_THIS_BUILD, cfg!(debug_assertions));
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn test_release_build_gate_is_closed() {
        let dir = tempdir().unwrap();
        let state = PluginState::for_app(Some(dir.path().to_path_buf()));
        assert!(!PLUGIN_FEATURE_ENABLED_IN_THIS_BUILD);
        assert!(!state.is_feature_enabled());
        assert_eq!(
            state.feature_status(),
            PluginFeatureStatus {
                enabled: false,
                build_profile: "production".to_string(),
            }
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_debug_build_gate_is_open() {
        let dir = tempdir().unwrap();
        let state = PluginState::for_app(Some(dir.path().to_path_buf()));
        assert!(state.is_feature_enabled());
        assert_eq!(
            state.feature_status(),
            PluginFeatureStatus {
                enabled: true,
                build_profile: "development".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_for_app_without_data_dir_fails_closed_even_with_gate_open() {
        let state = PluginState::for_app(None);
        state.set_feature_enabled(true).await;
        let err = state
            .install_package("/some/path", "digest")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_PATH_INVALID");
    }

    #[test]
    fn test_plugin_root_is_under_given_app_data_dir() {
        let dir = tempdir().unwrap();
        let state = PluginState::for_app(Some(dir.path().to_path_buf()));
        assert_eq!(state.app_data_dir(), Some(dir.path()));
    }

    // -----------------------------------------------------------------------
    // 6. Permission & lifecycle proof (5a)
    // -----------------------------------------------------------------------

    const TWO_HOOK_MANIFEST_PERMS: [PluginPermission; 1] = [PluginPermission::UiNotification];

    /// Installs `make_valid_manifest_json(id)` (ui:notification; on_custom_command +
    /// on_document_open). With `consent` it also grants consent and enables it.
    async fn install_test_plugin(
        state: &PluginState,
        id: &str,
        consent: bool,
    ) -> (
        PluginInspectionResult,
        InstalledPluginInfoV1,
        tempfile::TempDir,
    ) {
        let package = tempdir().unwrap();
        fs::write(
            package.path().join("manifest.json"),
            make_valid_manifest_json(id),
        )
        .unwrap();
        fs::write(
            package.path().join("main.rhai"),
            "fn on_custom_command(p) {}",
        )
        .unwrap();
        let path = package.path().to_string_lossy().to_string();
        let insp = state.inspect_package(&path).await.unwrap();
        let installed = state
            .install_package(&path, &insp.package_digest)
            .await
            .unwrap();
        if consent {
            state
                .grant_consent(id, &insp.package_digest, &TWO_HOOK_MANIFEST_PERMS)
                .await
                .unwrap();
            state.set_plugin_enabled(id, true).await.unwrap();
        }
        (insp, installed, package)
    }

    async fn entry(state: &PluginState, id: &str) -> InstalledPluginInfoV1 {
        state
            .list_installed_plugins()
            .await
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    }

    fn assert_quarantined(info: &InstalledPluginInfoV1) {
        assert!(!info.consent_granted);
        assert_eq!(info.consent_policy_version, 0);
        assert!(!info.enabled);
    }

    async fn assert_tamper_quarantines_on_execution(file: &str, replacement: &str) {
        let (state, _dir) = setup_enabled_state_with_temp_dir();
        let (insp, installed, _pkg) = install_test_plugin(&state, "tamper-target", true).await;

        fs::write(
            PathBuf::from(&installed.install_dir).join(file),
            replacement,
        )
        .unwrap();

        let payload = r#"{"command_name":"x","parameters":{}}"#;
        let err = state
            .start_execution("tamper-target", "on_custom_command", payload, "exec-t1")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_PACKAGE_DIGEST_MISMATCH");

        // In memory and on disk
        assert_quarantined(&entry(&state, "tamper-target").await);
        let persisted = install::load_persisted_registry(state.app_data_dir().unwrap());
        assert_quarantined(persisted.get("tamper-target").unwrap());

        // Quarantine sticks: no re-enable, no re-consent to the changed files, no run.
        let err = state
            .set_plugin_enabled("tamper-target", true)
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_CONSENT_REQUIRED");
        let err = state
            .grant_consent(
                "tamper-target",
                &insp.package_digest,
                &TWO_HOOK_MANIFEST_PERMS,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_PACKAGE_DIGEST_MISMATCH");
        let err = state
            .start_execution("tamper-target", "on_custom_command", payload, "exec-t2")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_CONSENT_REQUIRED");

        // Uninstall still cleans it up.
        state.uninstall_plugin("tamper-target").await.unwrap();
    }

    #[tokio::test]
    async fn test_script_changed_after_consent_quarantines_plugin() {
        assert_tamper_quarantines_on_execution(
            "main.rhai",
            "fn on_custom_command(p) { return \"tampered\"; }",
        )
        .await;
    }

    #[tokio::test]
    async fn test_manifest_changed_after_consent_quarantines_plugin() {
        // Same id/version, one more permission: still a different package.
        let tampered = make_valid_manifest_json("tamper-target")
            .replace(r#""ui:notification""#, r#""ui:notification", "plugin:log""#);
        assert_tamper_quarantines_on_execution("manifest.json", &tampered).await;
    }

    #[tokio::test]
    async fn test_change_after_precheck_is_quarantined_after_worker_failure() {
        // The worker revalidates digests itself; when it fails, the host re-hashes and
        // quarantines a package that changed after the pre-check.
        let (state, _dir) = setup_enabled_state_with_temp_dir();
        let (_insp, installed, _pkg) = install_test_plugin(&state, "race-target", true).await;
        let app_dir = state.app_data_dir().unwrap().to_path_buf();

        // Unchanged files: a worker failure alone does not quarantine.
        quarantine_if_tampered(&app_dir, &state.installed_plugins, "race-target").await;
        assert!(entry(&state, "race-target").await.enabled);

        fs::write(
            PathBuf::from(&installed.install_dir).join("main.rhai"),
            "fn on_custom_command(p) { 1 }",
        )
        .unwrap();
        quarantine_if_tampered(&app_dir, &state.installed_plugins, "race-target").await;
        assert_quarantined(&entry(&state, "race-target").await);
        let persisted = install::load_persisted_registry(&app_dir);
        assert_quarantined(persisted.get("race-target").unwrap());
    }

    #[tokio::test]
    async fn test_undeclared_hook_rejected() {
        let (state, _dir) = setup_enabled_state_with_temp_dir();
        install_test_plugin(&state, "hook-target", true).await;

        // Known hook, not in this manifest
        let err = state
            .start_execution(
                "hook-target",
                "on_export_complete",
                r#"{"export_format":"png","success":true}"#,
                "exec-h1",
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_HOOK_NOT_DECLARED");

        // Not a hook at all
        let err = state
            .start_execution("hook-target", "on_anything", "{}", "exec-h2")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_HOOK_UNKNOWN");

        // Nothing was registered or left busy
        assert!(!state.execution_registry.is_plugin_busy("hook-target").await);
        assert_eq!(
            state
                .get_execution_status("exec-h1")
                .await
                .unwrap_err()
                .code,
            "PLUGIN_EXECUTION_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn test_unconsented_plugin_cannot_be_enabled_or_executed() {
        let (state, _dir) = setup_enabled_state_with_temp_dir();
        install_test_plugin(&state, "no-consent", false).await;
        let payload = r#"{"command_name":"x","parameters":{}}"#;

        let err = state
            .set_plugin_enabled("no-consent", true)
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_CONSENT_REQUIRED");
        let err = state
            .start_execution("no-consent", "on_custom_command", payload, "exec-c1")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_CONSENT_REQUIRED");

        // A registry file edited to say enabled=true without consent still cannot run.
        let app_dir = state.app_data_dir().unwrap().to_path_buf();
        let mut registry = install::load_persisted_registry(&app_dir);
        registry.get_mut("no-consent").unwrap().enabled = true;
        install::save_persisted_registry(&app_dir, &registry).unwrap();
        let reloaded = PluginState::with_app_data_dir(app_dir);
        reloaded.set_feature_enabled(true).await;
        assert!(entry(&reloaded, "no-consent").await.enabled);
        let err = reloaded
            .start_execution("no-consent", "on_custom_command", payload, "exec-c2")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_CONSENT_REQUIRED");

        // Consent for a stale policy version is not consent either.
        let (state2, _dir2) = setup_enabled_state_with_temp_dir();
        install_test_plugin(&state2, "old-policy", true).await;
        {
            let mut lock = state2.installed_plugins.write().await;
            lock.get_mut("old-policy").unwrap().consent_policy_version =
                CURRENT_PLUGIN_CONSENT_POLICY_VERSION + 1;
        }
        let err = state2
            .start_execution("old-policy", "on_custom_command", payload, "exec-c3")
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_CONSENT_REQUIRED");
    }

    #[tokio::test]
    async fn test_disabled_plugin_execution_rejected() {
        let (state, _dir) = setup_enabled_state_with_temp_dir();
        install_test_plugin(&state, "off-target", true).await;
        state.set_plugin_enabled("off-target", false).await.unwrap();
        let err = state
            .start_execution(
                "off-target",
                "on_custom_command",
                r#"{"command_name":"x","parameters":{}}"#,
                "exec-d1",
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "PLUGIN_NOT_ENABLED");
    }

    #[tokio::test]
    async fn test_gate_closed_after_install_allows_only_restrictive_commands() {
        let (state, _dir) = setup_enabled_state_with_temp_dir();
        let (insp, _installed, pkg) = install_test_plugin(&state, "gate-target", true).await;
        let pkg_path = pkg.path().to_string_lossy().to_string();

        state.set_feature_enabled(false).await;

        // Privilege-increasing commands: all rejected by the gate
        let disabled = "PLUGIN_FEATURE_DISABLED";
        assert_eq!(
            state.inspect_package(&pkg_path).await.unwrap_err().code,
            disabled
        );
        assert_eq!(
            state
                .install_package(&pkg_path, &insp.package_digest)
                .await
                .unwrap_err()
                .code,
            disabled
        );
        assert_eq!(
            state
                .grant_consent(
                    "gate-target",
                    &insp.package_digest,
                    &TWO_HOOK_MANIFEST_PERMS
                )
                .await
                .unwrap_err()
                .code,
            disabled
        );
        assert_eq!(
            state
                .set_plugin_enabled("gate-target", true)
                .await
                .unwrap_err()
                .code,
            disabled
        );
        assert_eq!(
            state
                .start_execution(
                    "gate-target",
                    "on_custom_command",
                    r#"{"command_name":"x","parameters":{}}"#,
                    "exec-g1"
                )
                .await
                .unwrap_err()
                .code,
            disabled
        );

        // Restrictive / observing commands: still work on a real installed plugin
        assert_eq!(state.list_installed_plugins().await.len(), 1);
        assert!(!state.feature_status().enabled);
        assert_eq!(
            state
                .get_execution_status("exec-g1")
                .await
                .unwrap_err()
                .code,
            "PLUGIN_EXECUTION_NOT_FOUND"
        );
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state
            .execution_registry
            .register_execution(
                "exec-live".to_string(),
                "other".to_string(),
                "on_custom_command".to_string(),
                tx,
            )
            .await;
        state.cancel_execution("exec-live").await.unwrap();
        let disabled_entry = state
            .set_plugin_enabled("gate-target", false)
            .await
            .unwrap();
        assert!(!disabled_entry.enabled);
        state.uninstall_plugin("gate-target").await.unwrap();
        assert!(state.list_installed_plugins().await.is_empty());
    }
}
