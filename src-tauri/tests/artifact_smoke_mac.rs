//! Artifact Smoke Test — macOS (11I-E2)
//!
//! Exact artifact smoke test for macOS `.dmg` and `.app` bundle.
//! This test invokes the Node.js harness (artifact-smoke-macos.mjs) which implements
//! the full smoke lifecycle: mount/copy → bundle layout validation → launch →
//! runtime resolution → smoke document → shutdown → cleanup.
//!
//! Run with: ARTIFACT_SMOKE_MACOS_PATH=<path> cargo test --test artifact_smoke_mac -- --ignored

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn artifact_path_from_env() -> Option<PathBuf> {
    env::var("ARTIFACT_SMOKE_MACOS_PATH")
        .ok()
        .map(PathBuf::from)
}

fn artifact_type_from_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "dmg" => Some("MACOS_DMG".to_string()),
        "app" => Some("MACOS_APP_BUNDLE".to_string()),
        _ => None,
    }
}

fn harness_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("artifact-smoke-macos.mjs")
}

fn node_executable() -> String {
    env::var("NODE_EXE").unwrap_or_else(|_| "node".to_string())
}

#[test]
#[ignore = "requires ARTIFACT_SMOKE_MACOS_PATH env var pointing to real .dmg or .app"]
fn artifact_smoke_macos() {
    let artifact_path = artifact_path_from_env().expect("ARTIFACT_SMOKE_MACOS_PATH not set");
    let artifact_type = artifact_type_from_path(&artifact_path)
        .expect("Unknown artifact type (must be .dmg or .app)");

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
        .arg(&artifact_type)
        .arg("--verbose");

    if let Ok(output_dir) = env::var("ARTIFACT_SMOKE_OUTPUT_DIR") {
        let evidence_name = format!("macos-smoke-evidence-{}.json", artifact_type.to_lowercase());
        cmd.arg("--output").arg(join(&output_dir, &evidence_name));
    }

    let status = cmd.status().expect("Failed to execute macOS smoke harness");

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        panic!("macOS artifact smoke harness exited with code {}", code);
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
        .arg("/nonexistent/PreviewLite.dmg")
        .output()
        .expect("Failed to run harness with missing artifact");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn macos_packaging_contract_validator_passes() {
    let validator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("validate-macos-packaging-contract.mjs");
    assert!(validator.exists());
    let status = Command::new(node_executable())
        .arg(&validator)
        .status()
        .expect("Failed to run validate-macos-packaging-contract.mjs");
    assert!(status.success());
}

#[test]
fn fast_local_macos_harness_tests_pass() {
    let test_runner = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("test-macos-smoke-harness.mjs");
    assert!(test_runner.exists());
    let status = Command::new(node_executable())
        .arg(&test_runner)
        .status()
        .expect("Failed to run test-macos-smoke-harness.mjs");
    assert!(status.success());
}

fn join<P: AsRef<std::path::Path>>(base: P, rel: &str) -> PathBuf {
    base.as_ref().join(rel)
}
