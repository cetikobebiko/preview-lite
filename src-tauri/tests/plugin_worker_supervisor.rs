#[cfg(not(feature = "plugin-test-support"))]
#[test]
fn test_plugin_worker_supervisor_feature_disabled_noop() {
    // Under default cargo test builds without `--features plugin-test-support`,
    // this test target compiles cleanly without referencing CARGO_BIN_EXE_plugin-worker-test-helper.
}

#[cfg(feature = "plugin-test-support")]
mod supervisor_adversarial_tests {
    use preview_alternative_lib::plugins::error::PluginErrorCode;
    use preview_alternative_lib::plugins::limits::*;
    use preview_alternative_lib::plugins::manifest::PluginHook;
    use preview_alternative_lib::plugins::registry::PluginHookRequestV1;
    use preview_alternative_lib::plugins::worker::{
        supervise_plugin_execution_with_launcher, PluginWorkerLauncher,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn helper_launcher(mode: &str) -> PluginWorkerLauncher {
        PluginWorkerLauncher::for_test(
            PathBuf::from(env!("CARGO_BIN_EXE_plugin-worker-test-helper")),
            vec![OsString::from("--mode"), OsString::from(mode)],
        )
    }

    fn sample_hook_request(exec_id: &str) -> PluginHookRequestV1 {
        PluginHookRequestV1 {
            schema_version: CURRENT_PLUGIN_HOOK_SCHEMA_VERSION,
            execution_id: exec_id.to_string(),
            plugin_id: "test-plugin".to_string(),
            hook: PluginHook::OnCustomCommand,
            payload: serde_json::json!({ "command_name": "cmd", "parameters": {} }),
        }
    }

    #[tokio::test]
    async fn test_supervisor_valid_response_success() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("valid-response");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-valid");

        let res = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await;

        assert!(res.is_ok());
        let outcome = res.unwrap();
        assert_eq!(outcome.text, Some("helper test output".to_string()));
    }

    #[tokio::test]
    async fn test_supervisor_wrong_protocol_version_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("wrong-protocol-version");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-proto");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerProtocolError);
    }

    #[tokio::test]
    async fn test_supervisor_wrong_request_id_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("wrong-request-id");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-req-id");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerProtocolError);
    }

    #[tokio::test]
    async fn test_supervisor_wrong_execution_id_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("wrong-execution-id");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-expected-id");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerProtocolError);
    }

    #[tokio::test]
    async fn test_supervisor_malformed_length_prefix_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("malformed-length-prefix");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-malformed-len");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginResultTooLarge);
    }

    #[tokio::test]
    async fn test_supervisor_oversized_response_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("oversized-response");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-oversize");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginResultTooLarge);
    }

    #[tokio::test]
    async fn test_supervisor_truncated_frame_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("truncated-frame");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-trunc");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerCrashed);
    }

    #[tokio::test]
    async fn test_supervisor_malformed_json_rejected() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("malformed-json");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-bad-json");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerProtocolError);
    }

    #[tokio::test]
    async fn test_supervisor_exit_before_response_crashed() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("exit-before-response");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-exit-early");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerCrashed);
    }

    #[tokio::test]
    async fn test_supervisor_duplicate_terminal_response_protocol_violation() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("duplicate-terminal-response");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-dup");

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginWorkerProtocolError);
    }

    #[tokio::test]
    async fn test_supervisor_stderr_flood_survived() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("stderr-flood");
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-flood");

        let res = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_supervisor_cooperative_cancel() {
        let temp = tempdir().unwrap();
        let launcher = helper_launcher("cancel-cooperative");
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let hook_req = sample_hook_request("exec-cancel");

        // Trigger cancel after slight delay
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = cancel_tx.send(());
        });

        let err = supervise_plugin_execution_with_launcher(
            launcher,
            temp.path(),
            "test-plugin",
            "1.0.0",
            "dummy_pkg_digest",
            "dummy_mf_digest",
            "dummy_sc_digest",
            hook_req,
            vec![],
            cancel_rx,
        )
        .await
        .unwrap_err();

        assert_eq!(err.code, PluginErrorCode::PluginExecutionCancelled);
    }
}
