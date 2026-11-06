//! End-to-end plugin V1 proof through the public `PluginState` API and the real
//! app binary's `--run-plugin-worker` entrypoint (no fake helper).
//!
//! Covers the shipped example package `docs/engineering/examples/plugins/hello-log`
//! (install → consent → enable → on_custom_command) plus permission, limit,
//! disable and tamper outcomes as they reach the execution status.

use preview_alternative_lib::plugins::manifest::{PluginHook, PluginPermission};
use preview_alternative_lib::plugins::registry::{ExecutionState, PluginExecutionStatus};
use preview_alternative_lib::plugins::PluginState;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const CUSTOM_COMMAND_PAYLOAD: &str = r#"{"command_name":"hello","parameters":{"n":1}}"#;

fn app_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative"))
}

fn hello_log_package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs/engineering/examples/plugins/hello-log")
}

/// Gate opened explicitly (test authority) so the test means the same thing in any
/// build profile; the profile-driven gate itself is covered by the lib unit tests.
async fn open_state() -> (PluginState, TempDir) {
    let app_data = tempfile::tempdir().unwrap();
    let state = PluginState::with_app_data_dir(app_data.path().to_path_buf())
        .with_worker_executable(app_binary());
    state.set_feature_enabled(true).await;
    (state, app_data)
}

/// Inspect, install, consent with the manifest's own permissions, enable.
async fn install_consent_enable(state: &PluginState, package: &Path) -> String {
    let path = package.to_string_lossy().to_string();
    let inspection = state.inspect_package(&path).await.unwrap();
    let id = inspection.manifest.id.clone();
    state
        .install_package(&path, &inspection.package_digest)
        .await
        .unwrap();
    state
        .grant_consent(
            &id,
            &inspection.package_digest,
            &inspection.manifest.permissions,
        )
        .await
        .unwrap();
    state.set_plugin_enabled(&id, true).await.unwrap();
    id
}

async fn run_to_terminal(
    state: &PluginState,
    plugin_id: &str,
    hook: &str,
    payload: &str,
    execution_id: &str,
) -> PluginExecutionStatus {
    state
        .start_execution(plugin_id, hook, payload, execution_id)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = state.get_execution_status(execution_id).await.unwrap();
        if !matches!(
            status.state,
            ExecutionState::Pending | ExecutionState::Running
        ) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "execution {execution_id} did not finish"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn write_package(dir: &Path, id: &str, permissions: &[&str], script: &str) {
    let manifest = serde_json::json!({
        "manifest_version": 1,
        "id": id,
        "name": id,
        "version": "1.0.0",
        "main_script": "main.rhai",
        "permissions": permissions,
        "hooks": ["on_custom_command"],
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("main.rhai"), script).unwrap();
}

#[tokio::test]
async fn hello_log_example_runs_on_custom_command_through_real_worker() {
    let (state, app_data) = open_state().await;

    let inspection = state
        .inspect_package(&hello_log_package().to_string_lossy())
        .await
        .unwrap();
    assert_eq!(inspection.manifest.id, "hello-log");
    assert_eq!(
        inspection.manifest.permissions,
        vec![
            PluginPermission::PluginLog,
            PluginPermission::UiNotification
        ]
    );
    assert_eq!(
        inspection.manifest.hooks,
        vec![
            PluginHook::OnCustomCommand,
            PluginHook::OnDocumentOpen,
            PluginHook::OnExportComplete,
        ]
    );

    let id = install_consent_enable(&state, &hello_log_package()).await;
    let installed = state.list_installed_plugins().await;
    assert_eq!(installed.len(), 1);
    assert!(installed[0].enabled && installed[0].consent_granted);
    // Files live under <app_data_dir>/plugins/installed/<id>/<version>
    let expected_dir = app_data
        .path()
        .canonicalize()
        .unwrap()
        .join("plugins/installed/hello-log/1.0.0");
    assert_eq!(
        PathBuf::from(&installed[0].install_dir)
            .canonicalize()
            .unwrap(),
        expected_dir
    );

    let status = run_to_terminal(
        &state,
        &id,
        "on_custom_command",
        CUSTOM_COMMAND_PAYLOAD,
        "exec-hello-1",
    )
    .await;
    assert_eq!(status.state, ExecutionState::Completed, "{status:?}");
    assert!(status.error.is_none());
    let result = status.result.expect("completed execution carries a result");
    assert_eq!(result.execution_id, "exec-hello-1");
    assert_eq!(result.text.as_deref(), Some("hello-log tamam"));
    assert_eq!(
        result.logs,
        vec![format!(
            "hello-log: on_custom_command {}",
            r#"{"command_name":"hello","parameters":{"n":1}}"#
        )]
    );
    assert_eq!(
        result.notifications,
        vec!["Merhaba! hello-log eklentisi çalıştı.".to_string()]
    );
    assert!(result.warnings.is_empty());

    // The other declared hooks run too (log only, no notification).
    let status = run_to_terminal(
        &state,
        &id,
        "on_document_open",
        r#"{"page_count":3}"#,
        "exec-hello-2",
    )
    .await;
    assert_eq!(status.state, ExecutionState::Completed, "{status:?}");
    let result = status.result.unwrap();
    assert_eq!(
        result.logs,
        vec![r#"hello-log: on_document_open {"page_count":3}"#.to_string()]
    );
    assert!(result.notifications.is_empty());
    assert_eq!(result.text, None);
}

#[tokio::test]
async fn hello_log_disabled_then_tampered_is_rejected_and_quarantined() {
    let (state, _app_data) = open_state().await;
    let id = install_consent_enable(&state, &hello_log_package()).await;

    // Disabled → execution refused before any worker starts
    state.set_plugin_enabled(&id, false).await.unwrap();
    let err = state
        .start_execution(&id, "on_custom_command", CUSTOM_COMMAND_PAYLOAD, "exec-off")
        .await
        .unwrap_err();
    assert_eq!(err.code, "PLUGIN_NOT_ENABLED");

    // Re-enable, change the installed script on disk → next run is refused and the
    // plugin is quarantined (consent revoked, disabled).
    state.set_plugin_enabled(&id, true).await.unwrap();
    let install_dir = PathBuf::from(&state.list_installed_plugins().await[0].install_dir);
    let mut script = fs::read_to_string(install_dir.join("main.rhai")).unwrap();
    script.push_str("\n// changed after consent\n");
    fs::write(install_dir.join("main.rhai"), script).unwrap();

    let err = state
        .start_execution(
            &id,
            "on_custom_command",
            CUSTOM_COMMAND_PAYLOAD,
            "exec-tamper",
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "PLUGIN_PACKAGE_DIGEST_MISMATCH");
    let entry = &state.list_installed_plugins().await[0];
    assert!(!entry.enabled && !entry.consent_granted);
    assert_eq!(entry.consent_policy_version, 0);

    state.uninstall_plugin(&id).await.unwrap();
}

#[tokio::test]
async fn ungranted_host_api_call_ends_in_permission_denied_status() {
    let (state, _app_data) = open_state().await;
    let pkg = tempfile::tempdir().unwrap();
    // Declares only plugin:log, then calls ui_notify.
    write_package(
        pkg.path(),
        "log-only",
        &["plugin:log"],
        r#"fn on_custom_command(p) { plugin_log("once"); ui_notify("not allowed"); }"#,
    );
    let id = install_consent_enable(&state, pkg.path()).await;

    let status = run_to_terminal(
        &state,
        &id,
        "on_custom_command",
        CUSTOM_COMMAND_PAYLOAD,
        "exec-perm",
    )
    .await;
    assert_eq!(status.state, ExecutionState::Failed, "{status:?}");
    assert!(status.result.is_none());
    assert_eq!(status.error.unwrap().code, "PLUGIN_PERMISSION_DENIED");
}

#[tokio::test]
async fn log_byte_limit_ends_in_resource_limit_status() {
    let (state, _app_data) = open_state().await;
    let pkg = tempfile::tempdir().unwrap();
    // 1100 × 64 bytes > 64 KiB total log budget
    write_package(
        pkg.path(),
        "log-flood",
        &["plugin:log"],
        r#"fn on_custom_command(p) {
            let msg = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
            let i = 0;
            while i < 1100 { plugin_log(msg); i += 1; }
        }"#,
    );
    let id = install_consent_enable(&state, pkg.path()).await;

    let status = run_to_terminal(
        &state,
        &id,
        "on_custom_command",
        CUSTOM_COMMAND_PAYLOAD,
        "exec-log-flood",
    )
    .await;
    assert_eq!(status.state, ExecutionState::Failed, "{status:?}");
    assert_eq!(status.error.unwrap().code, "PLUGIN_RESOURCE_LIMIT_EXCEEDED");
}

#[tokio::test]
async fn notification_limits_end_in_resource_limit_status() {
    let (state, _app_data) = open_state().await;

    // More than 32 notifications
    let pkg = tempfile::tempdir().unwrap();
    write_package(
        pkg.path(),
        "notify-count",
        &["ui:notification"],
        r#"fn on_custom_command(p) {
            let i = 0;
            while i < 33 { ui_notify("n"); i += 1; }
        }"#,
    );
    let id = install_consent_enable(&state, pkg.path()).await;
    let status = run_to_terminal(
        &state,
        &id,
        "on_custom_command",
        CUSTOM_COMMAND_PAYLOAD,
        "exec-notify-count",
    )
    .await;
    assert_eq!(status.state, ExecutionState::Failed, "{status:?}");
    assert_eq!(status.error.unwrap().code, "PLUGIN_RESOURCE_LIMIT_EXCEEDED");

    // One notification over 4 KiB
    let pkg = tempfile::tempdir().unwrap();
    write_package(
        pkg.path(),
        "notify-size",
        &["ui:notification"],
        r#"fn on_custom_command(p) {
            let s = "a";
            while s.len() < 4097 { s = s + s; }
            ui_notify(s);
        }"#,
    );
    let id = install_consent_enable(&state, pkg.path()).await;
    let status = run_to_terminal(
        &state,
        &id,
        "on_custom_command",
        CUSTOM_COMMAND_PAYLOAD,
        "exec-notify-size",
    )
    .await;
    assert_eq!(status.state, ExecutionState::Failed, "{status:?}");
    assert_eq!(status.error.unwrap().code, "PLUGIN_RESOURCE_LIMIT_EXCEEDED");
}
