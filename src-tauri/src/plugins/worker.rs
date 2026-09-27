//! Subprocess Worker Runner and Host Supervisor for Isolated Plugin Execution.

use super::error::{PluginError, PluginErrorCode, PluginErrorWire};
use super::host_api::execute_plugin_script;
use super::limits::*;
use super::manifest::{PluginManifestV1, PluginPermission};
use super::path_security::{
    compute_package_digest, compute_sha256_hex, scan_package_directory_safety,
    validate_and_canonicalize_contained_path, verify_no_symlinks_in_path,
};
use super::registry::{PluginHookRequestV1, PluginHookResultV1};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecuteRequest {
    pub protocol_version: u32, // Strictly 1
    pub request_id: String,
    pub execution_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub expected_package_digest: String,
    pub expected_manifest_digest: String,
    pub expected_script_digest: String,
    pub hook_request: PluginHookRequestV1,
    pub granted_permissions: Vec<PluginPermission>,
    pub resource_profile_version: u32, // Strictly 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerControlAction {
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerControlMessage {
    pub protocol_version: u32, // Strictly 1
    pub request_id: String,
    pub action: WorkerControlAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerTerminalStatus {
    Success,
    Failed,
    Cancelled,
    ResourceLimitExceeded,
    PermissionDenied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExecuteResponse {
    pub protocol_version: u32, // Strictly 1
    pub request_id: String,
    pub execution_id: String,
    pub status: WorkerTerminalStatus,
    pub result: Option<PluginHookResultV1>,
    pub error: Option<PluginErrorWire>,
}

/// Entrypoint for child worker process (`--run-plugin-worker`).
pub fn run_plugin_worker() {
    let stdout = std::io::stdout();

    // 1. Read and validate canonical private plugin root from parent-controlled env
    let root_os = match std::env::var_os("ANTIGRAVITY_PLUGIN_ROOT") {
        Some(v) => v,
        None => return, // Fail-closed
    };
    let root_path = PathBuf::from(root_os);
    if !root_path.is_absolute() || !root_path.exists() {
        return; // Fail-closed
    }
    let canonical_root = match root_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return,
    };

    // 2. Read Frame 1: WorkerExecuteRequest
    let mut len_buf = [0u8; 4];
    if std::io::stdin().read_exact(&mut len_buf).is_err() {
        return;
    }
    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len > MAX_IPC_REQUEST_BYTES {
        let err_resp = WorkerExecuteResponse {
            protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
            request_id: String::new(),
            execution_id: String::new(),
            status: WorkerTerminalStatus::Failed,
            result: None,
            error: Some(
                PluginError::new(
                    PluginErrorCode::PluginPayloadTooLarge,
                    "İstek yükü üst sınırı aştı.",
                )
                .to_wire(),
            ),
        };
        write_worker_response(&mut stdout.lock(), &err_resp);
        return;
    }

    let mut req_bytes = vec![0u8; payload_len];
    if std::io::stdin().read_exact(&mut req_bytes).is_err() {
        return;
    }

    let req: WorkerExecuteRequest = match serde_json::from_slice(&req_bytes) {
        Ok(r) => r,
        Err(e) => {
            let err_resp = WorkerExecuteResponse {
                protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
                request_id: String::new(),
                execution_id: String::new(),
                status: WorkerTerminalStatus::Failed,
                result: None,
                error: Some(
                    PluginError::new(
                        PluginErrorCode::PluginWorkerProtocolError,
                        format!("Worker JSON ayrıştırma hatası: {}", e),
                    )
                    .to_wire(),
                ),
            };
            write_worker_response(&mut stdout.lock(), &err_resp);
            return;
        }
    };

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let terminal_claimed = Arc::new(AtomicBool::new(false));

    // Spawn dedicated stdin listener for control messages (e.g. Cancel)
    let cancel_flag_clone = cancel_flag.clone();
    let expected_req_id = req.request_id.clone();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut ctrl_len_buf = [0u8; 4];
        while stdin.read_exact(&mut ctrl_len_buf).is_ok() {
            let ctrl_len = u32::from_be_bytes(ctrl_len_buf) as usize;
            if ctrl_len > MAX_IPC_CONTROL_BYTES {
                break;
            }
            let mut ctrl_bytes = vec![0u8; ctrl_len];
            if stdin.read_exact(&mut ctrl_bytes).is_err() {
                break;
            }
            if let Ok(ctrl) = serde_json::from_slice::<WorkerControlMessage>(&ctrl_bytes) {
                if ctrl.protocol_version == CURRENT_PLUGIN_PROTOCOL_VERSION
                    && ctrl.request_id == expected_req_id
                    && ctrl.action == WorkerControlAction::Cancel
                {
                    cancel_flag_clone.store(true, Ordering::SeqCst);
                }
            }
        }
    });

    // 3. 17-Step Mandatory Revalidation before building Rhai Engine
    let target_dir = canonical_root
        .join("installed")
        .join(&req.plugin_id)
        .join(&req.plugin_version);

    let revalidate_result = revalidate_worker_target_directory(&target_dir, &canonical_root, &req);

    let (status, result, error) = match revalidate_result {
        Err(err) => {
            let status = match err.code {
                PluginErrorCode::PluginPermissionDenied => WorkerTerminalStatus::PermissionDenied,
                PluginErrorCode::PluginResourceLimitExceeded => {
                    WorkerTerminalStatus::ResourceLimitExceeded
                }
                _ => WorkerTerminalStatus::Failed,
            };
            (status, None, Some(err.to_wire()))
        }
        Ok((manifest, script_content)) => {
            // Check identity parity between hook request and execute request
            if req.hook_request.plugin_id != req.plugin_id
                || req.hook_request.execution_id != req.execution_id
                || !manifest.hooks.contains(&req.hook_request.hook)
            {
                (
                    WorkerTerminalStatus::Failed,
                    None,
                    Some(
                        PluginError::new(
                            PluginErrorCode::PluginHookNotDeclared,
                            "Kanca kimliği veya deklarasyonu eşleşmiyor.",
                        )
                        .to_wire(),
                    ),
                )
            } else {
                // Execute in isolated Rhai sandbox
                match execute_plugin_script(
                    &script_content,
                    req.hook_request.hook,
                    &req.hook_request.payload,
                    &req.granted_permissions,
                    &req.execution_id,
                    cancel_flag.clone(),
                ) {
                    Ok(hook_result) => (WorkerTerminalStatus::Success, Some(hook_result), None),
                    Err(err) => {
                        let status = match err.code {
                            PluginErrorCode::PluginExecutionCancelled => {
                                WorkerTerminalStatus::Cancelled
                            }
                            PluginErrorCode::PluginResourceLimitExceeded => {
                                WorkerTerminalStatus::ResourceLimitExceeded
                            }
                            PluginErrorCode::PluginPermissionDenied => {
                                WorkerTerminalStatus::PermissionDenied
                            }
                            _ => WorkerTerminalStatus::Failed,
                        };
                        (status, None, Some(err.to_wire()))
                    }
                }
            }
        }
    };

    // Exactly-once terminal response
    if terminal_claimed
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        let resp = WorkerExecuteResponse {
            protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
            request_id: req.request_id,
            execution_id: req.execution_id,
            status,
            result,
            error,
        };
        let mut out = stdout.lock();
        write_worker_response(&mut out, &resp);
    }
}

fn revalidate_worker_target_directory(
    target_dir: &Path,
    canonical_root: &Path,
    req: &WorkerExecuteRequest,
) -> Result<(PluginManifestV1, String), PluginError> {
    // Check versions
    if req.protocol_version != CURRENT_PLUGIN_PROTOCOL_VERSION {
        return Err(PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!(
                "Desteklenmeyen worker protokol sürümü: {}",
                req.protocol_version
            ),
        ));
    }
    if req.resource_profile_version != CURRENT_PLUGIN_RESOURCE_PROFILE_VERSION {
        return Err(PluginError::new(
            PluginErrorCode::PluginResourceLimitExceeded,
            format!(
                "Desteklenmeyen kaynak profil sürümü: {}",
                req.resource_profile_version
            ),
        ));
    }

    // Step 1 & 2 & 3: Containment and symlink checks
    if !target_dir.exists() {
        return Err(PluginError::new(
            PluginErrorCode::PluginNotInstalled,
            "Hedef eklenti dizini bulunamadı.",
        ));
    }
    let canonical_target = target_dir.canonicalize().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Hedef dizin canonicalize edilemedi: {}", e),
        )
    })?;
    if !canonical_target.starts_with(canonical_root) {
        return Err(PluginError::new(
            PluginErrorCode::PluginPathTraversal,
            "Hedef eklenti dizini izin verilen kök dışında.",
        ));
    }

    // Step 4: Package V1 exactly-two-files rule
    scan_package_directory_safety(&canonical_target)?;
    verify_no_symlinks_in_path(&canonical_target)?;

    // Step 5, 6, 7: Read & validate manifest.json
    let manifest_path =
        validate_and_canonicalize_contained_path(&canonical_target, "manifest.json")?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Manifest okunamadı: {}", e),
        )
    })?;
    let manifest = PluginManifestV1::parse_and_validate(&manifest_bytes)?;

    // Step 8, 9, 10, 11: Manifest identity equality
    if manifest.manifest_version != CURRENT_PLUGIN_MANIFEST_VERSION {
        return Err(PluginError::new(
            PluginErrorCode::PluginManifestVersionUnsupported,
            "Manifest sürümü desteklenmiyor.",
        ));
    }
    if manifest.id != req.plugin_id {
        return Err(PluginError::new(
            PluginErrorCode::PluginIdInvalid,
            "Manifest ID eşleşmedi.",
        ));
    }
    if manifest.version != req.plugin_version {
        return Err(PluginError::new(
            PluginErrorCode::PluginVersionInvalid,
            "Manifest sürümü eşleşmedi.",
        ));
    }
    if manifest.main_script != "main.rhai" {
        return Err(PluginError::new(
            PluginErrorCode::PluginManifestInvalid,
            "Ana betik kesinlikle 'main.rhai' olmalıdır.",
        ));
    }

    // Step 12: Declared hook check
    if !manifest.hooks.contains(&req.hook_request.hook) {
        return Err(PluginError::new(
            PluginErrorCode::PluginHookNotDeclared,
            "Kanca eklenti manifestinde bildirilmemiş.",
        ));
    }

    // Step 13: Exact canonical permission set equality
    let disk_perms: BTreeSet<PluginPermission> = manifest.permissions.iter().copied().collect();
    let granted_perms: BTreeSet<PluginPermission> =
        req.granted_permissions.iter().copied().collect();
    if disk_perms != granted_perms {
        return Err(PluginError::new(
            PluginErrorCode::PluginPermissionDenied,
            "Manifest izin kümesi verilen izinlerle tam eşleşmiyor.",
        ));
    }

    // Step 14, 15, 16: Digest parity
    let script_path = validate_and_canonicalize_contained_path(&canonical_target, "main.rhai")?;
    let script_bytes = fs::read(&script_path).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Betik okunamadı: {}", e),
        )
    })?;

    let calc_manifest_digest = compute_sha256_hex(&manifest_bytes);
    let calc_script_digest = compute_sha256_hex(&script_bytes);
    let calc_package_digest = compute_package_digest(&manifest_bytes, &script_bytes);

    if calc_manifest_digest != req.expected_manifest_digest {
        return Err(PluginError::new(
            PluginErrorCode::PluginPackageDigestMismatch,
            "Manifest özeti eşleşmedi.",
        ));
    }
    if calc_script_digest != req.expected_script_digest {
        return Err(PluginError::new(
            PluginErrorCode::PluginPackageDigestMismatch,
            "Betik özeti eşleşmedi.",
        ));
    }
    if calc_package_digest != req.expected_package_digest {
        return Err(PluginError::new(
            PluginErrorCode::PluginPackageDigestMismatch,
            "Paket özeti eşleşmedi.",
        ));
    }

    let script_content = String::from_utf8(script_bytes).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginPathInvalid,
            format!("Betik geçerli UTF-8 değil: {}", e),
        )
    })?;

    Ok((manifest, script_content))
}

fn write_worker_response<W: Write>(writer: &mut W, resp: &WorkerExecuteResponse) {
    if let Ok(bytes) = serde_json::to_vec(resp) {
        let len = (bytes.len() as u32).to_be_bytes();
        let _ = writer.write_all(&len);
        let _ = writer.write_all(&bytes);
        let _ = writer.flush();
    }
}

#[derive(Clone, Debug)]
pub struct PluginWorkerLauncher {
    pub executable: PathBuf,
    pub arguments: Vec<std::ffi::OsString>,
}

impl PluginWorkerLauncher {
    pub fn current_executable() -> Result<Self, PluginError> {
        let executable = std::env::current_exe().map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginWorkerStartFailed,
                format!("Mevcut uygulama yolu tespit edilemedi: {}", e),
            )
        })?;
        Ok(Self {
            executable,
            arguments: vec![std::ffi::OsString::from("--run-plugin-worker")],
        })
    }

    #[cfg(any(test, feature = "plugin-test-support"))]
    pub fn for_test(executable: PathBuf, arguments: Vec<std::ffi::OsString>) -> Self {
        Self {
            executable,
            arguments,
        }
    }
}

/// Host Supervisor: launches worker child process and manages execution lifecycle,
/// timeouts, bounded control messaging, and cooperative/hard cancellation.
#[allow(clippy::too_many_arguments)]
pub async fn supervise_plugin_execution(
    app_data_dir: &Path,
    plugin_id: &str,
    plugin_version: &str,
    expected_package_digest: &str,
    expected_manifest_digest: &str,
    expected_script_digest: &str,
    hook_request: PluginHookRequestV1,
    granted_permissions: Vec<PluginPermission>,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<PluginHookResultV1, PluginError> {
    let launcher = PluginWorkerLauncher::current_executable()?;
    supervise_plugin_execution_with_launcher(
        launcher,
        app_data_dir,
        plugin_id,
        plugin_version,
        expected_package_digest,
        expected_manifest_digest,
        expected_script_digest,
        hook_request,
        granted_permissions,
        cancel_rx,
    )
    .await
}

/// Host Supervisor with explicit worker launcher (used in production and test harnesses).
#[allow(clippy::too_many_arguments)]
pub async fn supervise_plugin_execution_with_launcher(
    launcher: PluginWorkerLauncher,
    app_data_dir: &Path,
    plugin_id: &str,
    plugin_version: &str,
    expected_package_digest: &str,
    expected_manifest_digest: &str,
    expected_script_digest: &str,
    hook_request: PluginHookRequestV1,
    granted_permissions: Vec<PluginPermission>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<PluginHookResultV1, PluginError> {
    let canonical_plugin_root = app_data_dir.join("plugins");
    let request_id = uuid::Uuid::new_v4().hyphenated().to_string();
    let execution_id = hook_request.execution_id.clone();

    let mut cmd = Command::new(&launcher.executable);
    cmd.args(&launcher.arguments)
        .env("ANTIGRAVITY_PLUGIN_ROOT", &canonical_plugin_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerStartFailed,
            format!("Worker alt süreci başlatılamadı: {}", e),
        )
    })?;

    let mut stdin = child.stdin.take().ok_or_else(|| {
        PluginError::new(
            PluginErrorCode::PluginWorkerStartFailed,
            "Worker stdin alınamadı.",
        )
    })?;

    let mut stdout = child.stdout.take().ok_or_else(|| {
        PluginError::new(
            PluginErrorCode::PluginWorkerStartFailed,
            "Worker stdout alınamadı.",
        )
    })?;

    let stderr = child.stderr.take();
    // Drain stderr in background to prevent pipe deadlock
    if let Some(mut err_pipe) = stderr {
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let mut total_drained = 0;
            while let Ok(n) = err_pipe.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                total_drained += n;
                if total_drained > MAX_STDERR_DRAIN_BYTES {
                    break;
                }
            }
        });
    }

    let req = WorkerExecuteRequest {
        protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
        request_id: request_id.clone(),
        execution_id: execution_id.clone(),
        plugin_id: plugin_id.to_string(),
        plugin_version: plugin_version.to_string(),
        expected_package_digest: expected_package_digest.to_string(),
        expected_manifest_digest: expected_manifest_digest.to_string(),
        expected_script_digest: expected_script_digest.to_string(),
        hook_request,
        granted_permissions,
        resource_profile_version: CURRENT_PLUGIN_RESOURCE_PROFILE_VERSION,
    };

    let req_bytes = serde_json::to_vec(&req).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!("Worker isteği serileştirilemedi: {}", e),
        )
    })?;

    // Send length-prefixed request
    let len_prefix = (req_bytes.len() as u32).to_be_bytes();
    stdin.write_all(&len_prefix).await.map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!("Worker stdin yazma hatası: {}", e),
        )
    })?;
    stdin.write_all(&req_bytes).await.map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!("Worker stdin yazma hatası: {}", e),
        )
    })?;
    stdin.flush().await.map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!("Worker stdin flush hatası: {}", e),
        )
    })?;

    let expected_req_id = request_id.clone();
    let expected_exec_id = execution_id.clone();

    let outcome: Result<PluginHookResultV1, PluginError>;

    tokio::select! {
        // Normal completion within timeout
        res = tokio::time::timeout(
            Duration::from_secs(DEFAULT_EXECUTION_TIMEOUT_SECS),
            read_supervisor_response(&mut stdout, &expected_req_id, &expected_exec_id),
        ) => {
            match res {
                Ok(inner_res) => {
                    // Post-terminal exit grace (500 ms) and duplicate response / extra byte detection
                    let mut extra_buf = [0u8; 1];
                    let post_terminal_check = async {
                        tokio::select! {
                            read_res = stdout.read(&mut extra_buf) => {
                                match read_res {
                                    Ok(0) => {
                                        // Clean EOF, wait for child process exit
                                        let _ = child.wait().await;
                                        None
                                    }
                                    Ok(_n) => {
                                        // Extra bytes / duplicate response detected!
                                        let _ = child.kill().await;
                                        let _ = child.wait().await;
                                        Some(PluginError::new(
                                            PluginErrorCode::PluginWorkerProtocolError,
                                            "Worker terminal yanıt sonrasında fazladan veri gönderdi (Protocol Violation).",
                                        ))
                                    }
                                    Err(_) => {
                                        let _ = child.kill().await;
                                        let _ = child.wait().await;
                                        None
                                    }
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_millis(PLUGIN_WORKER_EXIT_GRACE_MS)) => {
                                // Exit grace expired without extra bytes -> kill & reap child
                                let _ = child.kill().await;
                                let _ = child.wait().await;
                                None
                            }
                        }
                    };

                    if let Some(proto_err) = post_terminal_check.await {
                        if inner_res.is_err() {
                            outcome = inner_res;
                        } else {
                            outcome = Err(proto_err);
                        }
                    } else {
                        outcome = inner_res;
                    }
                }
                Err(_) => {
                    // Timeout hit -> Hard kill worker and reap
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    outcome = Err(PluginError::new(
                        PluginErrorCode::PluginExecutionTimeout,
                        format!("Eklenti yürütmesi {} saniye zaman aşımına uğradı.", DEFAULT_EXECUTION_TIMEOUT_SECS),
                    ));
                }
            }
        }
        // Cancellation signal received
        _ = &mut cancel_rx => {
            // Send length-prefixed cancel control message
            let ctrl = WorkerControlMessage {
                protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
                request_id: request_id.clone(),
                action: WorkerControlAction::Cancel,
            };
            if let Ok(ctrl_bytes) = serde_json::to_vec(&ctrl) {
                let ctrl_len = (ctrl_bytes.len() as u32).to_be_bytes();
                let _ = stdin.write_all(&ctrl_len).await;
                let _ = stdin.write_all(&ctrl_bytes).await;
                let _ = stdin.flush().await;
            }
            drop(stdin);

            let grace_result = tokio::time::timeout(
                Duration::from_millis(COOPERATIVE_CANCEL_GRACE_MS),
                child.wait()
            ).await;

            if grace_result.is_err() {
                // Hard kill fallback
                let _ = child.kill().await;
                let _ = child.wait().await;
            }

            outcome = Err(PluginError::new(
                PluginErrorCode::PluginExecutionCancelled,
                "Eklenti yürütmesi kullanıcı tarafından iptal edildi.",
            ));
        }
    }

    outcome
}

async fn read_supervisor_response<R: tokio::io::AsyncRead + Unpin>(
    stdout: &mut R,
    expected_req_id: &str,
    expected_exec_id: &str,
) -> Result<PluginHookResultV1, PluginError> {
    let mut len_buf = [0u8; 4];
    stdout.read_exact(&mut len_buf).await.map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerCrashed,
            format!("Worker yanıt başlığı okunamadı: {}", e),
        )
    })?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    if resp_len > MAX_IPC_RESULT_BYTES {
        return Err(PluginError::new(
            PluginErrorCode::PluginResultTooLarge,
            "Worker yanıtı izin verilen üst sınırı aştı.",
        ));
    }
    let mut resp_bytes = vec![0u8; resp_len];
    stdout.read_exact(&mut resp_bytes).await.map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerCrashed,
            format!("Worker yanıt gövdesi okunamadı: {}", e),
        )
    })?;

    let resp: WorkerExecuteResponse = serde_json::from_slice(&resp_bytes).map_err(|e| {
        PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!("Worker yanıt JSON ayrıştırma hatası: {}", e),
        )
    })?;

    if resp.protocol_version != CURRENT_PLUGIN_PROTOCOL_VERSION {
        return Err(PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            format!(
                "Desteklenmeyen worker yanıt protokol sürümü: {}",
                resp.protocol_version
            ),
        ));
    }

    // Correlation & Invariant check
    if resp.request_id != expected_req_id || resp.execution_id != expected_exec_id {
        return Err(PluginError::new(
            PluginErrorCode::PluginWorkerProtocolError,
            "Worker yanıt kimlik korelasyonu (request_id/execution_id) eşleşmedi.",
        ));
    }

    match resp.status {
        WorkerTerminalStatus::Success => {
            if let Some(res) = resp.result {
                Ok(res)
            } else {
                Err(PluginError::new(
                    PluginErrorCode::PluginWorkerProtocolError,
                    "Başarılı yanıtta sonuç verisi eksik.",
                ))
            }
        }
        WorkerTerminalStatus::Cancelled => Err(PluginError::new(
            PluginErrorCode::PluginExecutionCancelled,
            resp.error
                .map(|e| e.message)
                .unwrap_or_else(|| "Yürütme iptal edildi.".to_string()),
        )),
        WorkerTerminalStatus::ResourceLimitExceeded => Err(PluginError::new(
            PluginErrorCode::PluginResourceLimitExceeded,
            resp.error
                .map(|e| e.message)
                .unwrap_or_else(|| "Kaynak sınırı aşıldı.".to_string()),
        )),
        WorkerTerminalStatus::PermissionDenied => Err(PluginError::new(
            PluginErrorCode::PluginPermissionDenied,
            resp.error
                .map(|e| e.message)
                .unwrap_or_else(|| "İzin reddedildi.".to_string()),
        )),
        WorkerTerminalStatus::Failed => {
            let err_wire = resp.error.unwrap_or_else(|| PluginErrorWire {
                code: PluginErrorCode::PluginExecutionNotFound
                    .as_str()
                    .to_string(),
                message: "Bilinmeyen worker hatası.".to_string(),
            });
            Err(PluginError::new(
                PluginErrorCode::PluginExecutionNotFound,
                format!("[{}] {}", err_wire.code, err_wire.message),
            ))
        }
    }
}
