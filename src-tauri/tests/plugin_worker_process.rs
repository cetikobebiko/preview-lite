use preview_alternative_lib::plugins::limits::*;
use preview_alternative_lib::plugins::manifest::{PluginHook, PluginPermission};
use preview_alternative_lib::plugins::path_security::{compute_package_digest, compute_sha256_hex};
use preview_alternative_lib::plugins::registry::PluginHookRequestV1;
use preview_alternative_lib::plugins::worker::{
    WorkerExecuteRequest, WorkerExecuteResponse, WorkerTerminalStatus,
};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn preview_alternative_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative"))
}

fn write_framed(writer: &mut dyn Write, bytes: &[u8]) {
    let len = (bytes.len() as u32).to_be_bytes();
    writer.write_all(&len).unwrap();
    writer.write_all(bytes).unwrap();
    writer.flush().unwrap();
}

fn read_framed(reader: &mut dyn Read) -> Result<Vec<u8>, std::io::Error> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

struct TestPluginFixture {
    _temp: TempDir,
    root: PathBuf,
    plugin_id: String,
    plugin_version: String,
    package_digest: String,
    manifest_digest: String,
    script_digest: String,
}

fn create_test_plugin(
    plugin_id: &str,
    version: &str,
    script_content: &str,
    permissions: &[PluginPermission],
) -> TestPluginFixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let install_dir = root.join("installed").join(plugin_id).join(version);
    fs::create_dir_all(&install_dir).unwrap();

    let perms_json: Vec<String> = permissions.iter().map(|p| p.as_str().to_string()).collect();
    let manifest_json = serde_json::json!({
        "manifest_version": 1,
        "id": plugin_id,
        "name": "Test Plugin",
        "version": version,
        "main_script": "main.rhai",
        "permissions": perms_json,
        "hooks": ["on_custom_command"]
    });
    let manifest_bytes = serde_json::to_vec(&manifest_json).unwrap();
    let script_bytes = script_content.as_bytes().to_vec();

    fs::write(install_dir.join("manifest.json"), &manifest_bytes).unwrap();
    fs::write(install_dir.join("main.rhai"), &script_bytes).unwrap();

    let manifest_digest = compute_sha256_hex(&manifest_bytes);
    let script_digest = compute_sha256_hex(&script_bytes);
    let package_digest = compute_package_digest(&manifest_bytes, &script_bytes);

    TestPluginFixture {
        _temp: temp,
        root,
        plugin_id: plugin_id.to_string(),
        plugin_version: version.to_string(),
        package_digest,
        manifest_digest,
        script_digest,
    }
}

#[test]
fn test_worker_process_normal_execution_success() {
    let fixture = create_test_plugin(
        "norm-plugin",
        "1.0.0",
        "fn on_custom_command(p) { return \"hello from worker\"; }",
        &[],
    );

    let mut child = Command::new(preview_alternative_bin())
        .arg("--run-plugin-worker")
        .env("ANTIGRAVITY_PLUGIN_ROOT", &fixture.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn plugin worker process");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let req = WorkerExecuteRequest {
        protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
        request_id: "req-norm-1".to_string(),
        execution_id: "exec-norm-1".to_string(),
        plugin_id: fixture.plugin_id.clone(),
        plugin_version: fixture.plugin_version.clone(),
        expected_package_digest: fixture.package_digest.clone(),
        expected_manifest_digest: fixture.manifest_digest.clone(),
        expected_script_digest: fixture.script_digest.clone(),
        hook_request: PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: "exec-norm-1".to_string(),
            plugin_id: fixture.plugin_id.clone(),
            hook: PluginHook::OnCustomCommand,
            payload: serde_json::json!({ "command_name": "cmd", "parameters": {} }),
        },
        granted_permissions: vec![],
        resource_profile_version: 1,
    };

    let req_bytes = serde_json::to_vec(&req).unwrap();
    write_framed(&mut stdin, &req_bytes);

    let resp_bytes = read_framed(&mut stdout).expect("Failed to read response frame");
    let resp: WorkerExecuteResponse = serde_json::from_slice(&resp_bytes).unwrap();

    assert_eq!(resp.protocol_version, CURRENT_PLUGIN_PROTOCOL_VERSION);
    assert_eq!(resp.request_id, "req-norm-1");
    assert_eq!(resp.execution_id, "exec-norm-1");
    assert_eq!(resp.status, WorkerTerminalStatus::Success);

    let res = resp.result.unwrap();
    assert_eq!(res.text, Some("hello from worker".to_string()));

    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn test_worker_process_syntax_error_returns_failure() {
    let fixture = create_test_plugin("bad-syntax", "1.0.0", "fn invalid_syntax {", &[]);

    let mut child = Command::new(preview_alternative_bin())
        .arg("--run-plugin-worker")
        .env("ANTIGRAVITY_PLUGIN_ROOT", &fixture.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn plugin worker process");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let req = WorkerExecuteRequest {
        protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
        request_id: "req-syntax-err".to_string(),
        execution_id: "exec-syntax-err".to_string(),
        plugin_id: fixture.plugin_id.clone(),
        plugin_version: fixture.plugin_version.clone(),
        expected_package_digest: fixture.package_digest.clone(),
        expected_manifest_digest: fixture.manifest_digest.clone(),
        expected_script_digest: fixture.script_digest.clone(),
        hook_request: PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: "exec-syntax-err".to_string(),
            plugin_id: fixture.plugin_id.clone(),
            hook: PluginHook::OnCustomCommand,
            payload: serde_json::json!({ "command_name": "cmd", "parameters": {} }),
        },
        granted_permissions: vec![],
        resource_profile_version: 1,
    };

    write_framed(&mut stdin, &serde_json::to_vec(&req).unwrap());

    let resp_bytes = read_framed(&mut stdout).expect("Failed to read response");
    let resp: WorkerExecuteResponse = serde_json::from_slice(&resp_bytes).unwrap();

    assert_eq!(resp.status, WorkerTerminalStatus::Failed);
    assert!(resp.error.is_some());

    let _ = child.wait();
}

#[test]
fn test_worker_process_infinite_loop_hits_op_limit() {
    let fixture = create_test_plugin(
        "inf-loop",
        "1.0.0",
        "fn on_custom_command(p) { while true {} }",
        &[],
    );

    let mut child = Command::new(preview_alternative_bin())
        .arg("--run-plugin-worker")
        .env("ANTIGRAVITY_PLUGIN_ROOT", &fixture.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn plugin worker process");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let req = WorkerExecuteRequest {
        protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
        request_id: "req-inf-loop".to_string(),
        execution_id: "exec-inf-loop".to_string(),
        plugin_id: fixture.plugin_id.clone(),
        plugin_version: fixture.plugin_version.clone(),
        expected_package_digest: fixture.package_digest.clone(),
        expected_manifest_digest: fixture.manifest_digest.clone(),
        expected_script_digest: fixture.script_digest.clone(),
        hook_request: PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: "exec-inf-loop".to_string(),
            plugin_id: fixture.plugin_id.clone(),
            hook: PluginHook::OnCustomCommand,
            payload: serde_json::json!({ "command_name": "cmd", "parameters": {} }),
        },
        granted_permissions: vec![],
        resource_profile_version: 1,
    };

    write_framed(&mut stdin, &serde_json::to_vec(&req).unwrap());

    let resp_bytes = read_framed(&mut stdout).expect("Failed to read response");
    let resp: WorkerExecuteResponse = serde_json::from_slice(&resp_bytes).unwrap();

    assert_eq!(resp.status, WorkerTerminalStatus::ResourceLimitExceeded);
    let _ = child.wait();
}

#[test]
fn test_worker_process_ui_notify_and_logs_captured() {
    let fixture = create_test_plugin(
        "log-notif",
        "1.0.0",
        r#"
            fn on_custom_command(p) {
                plugin_log("log line 1");
                plugin_log("log line 2");
                ui_notify("user notification 1");
                return "done";
            }
        "#,
        &[
            PluginPermission::PluginLog,
            PluginPermission::UiNotification,
        ],
    );

    let mut child = Command::new(preview_alternative_bin())
        .arg("--run-plugin-worker")
        .env("ANTIGRAVITY_PLUGIN_ROOT", &fixture.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn plugin worker process");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let req = WorkerExecuteRequest {
        protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
        request_id: "req-log-notif".to_string(),
        execution_id: "exec-log-notif".to_string(),
        plugin_id: fixture.plugin_id.clone(),
        plugin_version: fixture.plugin_version.clone(),
        expected_package_digest: fixture.package_digest.clone(),
        expected_manifest_digest: fixture.manifest_digest.clone(),
        expected_script_digest: fixture.script_digest.clone(),
        hook_request: PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: "exec-log-notif".to_string(),
            plugin_id: fixture.plugin_id.clone(),
            hook: PluginHook::OnCustomCommand,
            payload: serde_json::json!({ "command_name": "cmd", "parameters": {} }),
        },
        granted_permissions: vec![
            PluginPermission::PluginLog,
            PluginPermission::UiNotification,
        ],
        resource_profile_version: 1,
    };

    write_framed(&mut stdin, &serde_json::to_vec(&req).unwrap());

    let resp_bytes = read_framed(&mut stdout).expect("Failed to read response");
    let resp: WorkerExecuteResponse = serde_json::from_slice(&resp_bytes).unwrap();

    assert_eq!(resp.status, WorkerTerminalStatus::Success);
    let res = resp.result.unwrap();
    assert_eq!(res.logs, vec!["log line 1", "log line 2"]);
    assert_eq!(res.notifications, vec!["user notification 1"]);

    let _ = child.wait();
}

#[test]
fn test_worker_process_malformed_request_causes_clean_exit() {
    let fixture = create_test_plugin("malform", "1.0.0", "fn on_custom_command(p) {}", &[]);

    let mut child = Command::new(preview_alternative_bin())
        .arg("--run-plugin-worker")
        .env("ANTIGRAVITY_PLUGIN_ROOT", &fixture.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn plugin worker process");

    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(b"not_a_valid_length_prefix");
    let _ = stdin.flush();
    drop(stdin);

    let status = child.wait().unwrap();
    assert!(status.success());
}

/// Runs one worker request for `plugin_id`/`plugin_version` against the fixture root.
fn run_worker_for_identity(
    fixture: &TestPluginFixture,
    plugin_id: &str,
    plugin_version: &str,
) -> WorkerExecuteResponse {
    let mut child = Command::new(preview_alternative_bin())
        .arg("--run-plugin-worker")
        .env("ANTIGRAVITY_PLUGIN_ROOT", &fixture.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn plugin worker process");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let req = WorkerExecuteRequest {
        protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
        request_id: "req-identity".to_string(),
        execution_id: "exec-identity".to_string(),
        plugin_id: plugin_id.to_string(),
        plugin_version: plugin_version.to_string(),
        expected_package_digest: fixture.package_digest.clone(),
        expected_manifest_digest: fixture.manifest_digest.clone(),
        expected_script_digest: fixture.script_digest.clone(),
        hook_request: PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: "exec-identity".to_string(),
            plugin_id: plugin_id.to_string(),
            hook: PluginHook::OnCustomCommand,
            payload: serde_json::json!({ "command_name": "cmd", "parameters": {} }),
        },
        granted_permissions: vec![],
        resource_profile_version: 1,
    };
    write_framed(&mut stdin, &serde_json::to_vec(&req).unwrap());

    let resp_bytes = read_framed(&mut stdout).expect("Failed to read response frame");
    let _ = child.wait();
    serde_json::from_slice(&resp_bytes).unwrap()
}

#[test]
fn test_worker_process_rejects_path_component_version() {
    let fixture = create_test_plugin("dots", "1.0.0", "fn on_custom_command(p) {}", &[]);

    for version in [".", ".."] {
        let resp = run_worker_for_identity(&fixture, "dots", version);
        assert_eq!(resp.status, WorkerTerminalStatus::Failed);
        assert_eq!(resp.error.unwrap().code, "PLUGIN_VERSION_INVALID");
    }
}

#[test]
#[cfg(unix)]
fn test_worker_process_rejects_symlinked_plugin_dir_inside_root() {
    // installed/alias -> installed/hidden: the resolved directory stays under the plugin
    // root but is not installed/<id>/<version>, so it must be refused.
    let fixture = create_test_plugin(
        "alias",
        "1.0.0",
        "fn on_custom_command(p) { return \"ran\"; }",
        &[],
    );
    let installed = fixture.root.join("installed");
    fs::rename(installed.join("alias"), installed.join("hidden")).unwrap();
    std::os::unix::fs::symlink(installed.join("hidden"), installed.join("alias")).unwrap();

    let resp = run_worker_for_identity(&fixture, "alias", "1.0.0");
    assert_eq!(resp.status, WorkerTerminalStatus::Failed);
    assert_eq!(resp.error.unwrap().code, "PLUGIN_PATH_TRAVERSAL");
}
