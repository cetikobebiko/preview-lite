//! Artifact Smoke Test — Linux (11I-E3)
//!
//! Exact artifact smoke test for Linux .deb packages.
//! This test invokes the Node.js harness (artifact-smoke-linux.mjs) which implements
//! the full smoke lifecycle: package inspection → extract → layout validation →
//! launch → runtime resolution → smoke document → shutdown → cleanup.
//!
//! Run with: ARTIFACT_SMOKE_LINUX_PATH=<path> cargo test --test artifact_smoke_linux -- --ignored

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn artifact_path_from_env() -> Option<PathBuf> {
    env::var("ARTIFACT_SMOKE_LINUX_PATH")
        .ok()
        .map(PathBuf::from)
}

fn harness_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("artifact-smoke-linux.mjs")
}

fn node_executable() -> String {
    env::var("NODE_EXE").unwrap_or_else(|_| "node".to_string())
}

#[test]
#[ignore = "requires ARTIFACT_SMOKE_LINUX_PATH env var pointing to real .deb"]
fn artifact_smoke_linux() {
    let artifact_path = artifact_path_from_env().expect("ARTIFACT_SMOKE_LINUX_PATH not set");

    let harness = harness_script_path();
    assert!(
        harness.exists(),
        "Harness script not found at {}",
        harness.display()
    );

    let mut cmd = Command::new(node_executable());
    cmd.arg(&harness)
        .arg("--artifact")
        .arg(&artifact_path)
        .arg("--artifact-type")
        .arg("LINUX_DEB")
        .arg("--verbose");

    if let Ok(output_dir) = env::var("ARTIFACT_SMOKE_OUTPUT_DIR") {
        cmd.arg("--output").arg(format!(
            "{}/linux-smoke-evidence-linux_deb.json",
            output_dir
        ));
    }

    let status = cmd.status().expect("Failed to execute Linux smoke harness");

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        panic!("Linux artifact smoke harness exited with code {}", code);
    }
}

#[test]
fn harness_script_exists() {
    let harness = harness_script_path();
    assert!(
        harness.exists(),
        "Harness script must exist at {}",
        harness.display()
    );
}

#[test]
fn harness_script_help_exits_zero() {
    let harness = harness_script_path();
    let status = Command::new(node_executable())
        .arg(&harness)
        .arg("--help")
        .status()
        .expect("Failed to run harness --help");
    assert!(status.success());
}

#[test]
fn harness_script_missing_artifact_exits_with_code_2() {
    let harness = harness_script_path();
    let output = Command::new(node_executable())
        .arg(&harness)
        .arg("--artifact")
        .arg("/nonexistent/preview-lite.deb")
        .output()
        .expect("Failed to run harness with missing artifact");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn linux_packaging_contract_validator_passes() {
    let validator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("validate-linux-packaging-contract.mjs");
    assert!(validator.exists());
    let status = Command::new(node_executable())
        .arg(&validator)
        .status()
        .expect("Failed to run validate-linux-packaging-contract.mjs");
    assert!(status.success());
}

#[test]
fn fast_local_linux_harness_tests_pass() {
    let test_runner = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("test-linux-smoke-harness.mjs");
    assert!(test_runner.exists());
    let status = Command::new(node_executable())
        .arg(&test_runner)
        .status()
        .expect("Failed to run test-linux-smoke-harness.mjs");
    assert!(status.success());
}
