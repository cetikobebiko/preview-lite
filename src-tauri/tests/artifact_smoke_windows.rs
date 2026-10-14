//! Artifact Smoke Test — Windows (11I-E4)
//!
//! Exact artifact smoke test for Windows .msi and .nsis packages.
//! This test invokes the Node.js harness (artifact-smoke-windows.mjs) which implements
//! the full smoke lifecycle: install → launch → runtime resolution → smoke document →
//! hardening tests → shutdown → uninstall.
//!
//! Run with: ARTIFACT_SMOKE_WINDOWS_PATH=<path> cargo test --test artifact_smoke_windows -- --ignored

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const SMOKE_FIXTURE_ID: &str = "core-release-smoke";

fn artifact_path_from_env() -> Option<PathBuf> {
    env::var("ARTIFACT_SMOKE_WINDOWS_PATH")
        .ok()
        .map(PathBuf::from)
}

fn artifact_type_from_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "msi" => Some("WINDOWS_MSI".to_string()),
        "exe" => Some("WINDOWS_NSIS".to_string()),
        _ => None,
    }
}

fn harness_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("artifact-smoke-windows.mjs")
}

fn node_executable() -> String {
    env::var("NODE_EXE").unwrap_or_else(|_| "node".to_string())
}

#[test]
#[ignore = "requires ARTIFACT_SMOKE_WINDOWS_PATH env var pointing to real .msi or .exe"]
fn artifact_smoke_windows() {
    let artifact_path = artifact_path_from_env().expect("ARTIFACT_SMOKE_WINDOWS_PATH not set");
    let artifact_type = artifact_type_from_path(&artifact_path)
        .expect("Unknown artifact type (must be .msi or .exe)");

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
        let evidence_name = format!(
            "windows-smoke-evidence-{}.json",
            artifact_type.to_lowercase()
        );
        cmd.arg("--output")
            .arg(join(&output_dir, evidence_name.as_str()));
    }

    let status = cmd
        .status()
        .expect("Failed to execute Windows smoke harness");

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        panic!("Windows artifact smoke harness exited with code {}", code);
    }
}

#[test]
#[ignore = "requires ARTIFACT_SMOKE_WINDOWS_PATH env var"]
fn artifact_smoke_windows_evidence_structure() {
    let artifact_path = artifact_path_from_env().expect("ARTIFACT_SMOKE_WINDOWS_PATH not set");
    let artifact_type = artifact_type_from_path(&artifact_path).expect("Unknown artifact type");

    let harness = harness_script_path();
    assert!(harness.exists());

    let output = Command::new(node_executable())
        .arg(&harness)
        .arg("--artifact")
        .arg(&artifact_path)
        .arg("--artifact-type")
        .arg(&artifact_type)
        .output()
        .expect("Failed to execute harness");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let evidence: serde_json::Value =
        serde_json::from_str(&stdout).expect("Harness output is not valid JSON");

    assert_eq!(evidence["schemaVersion"], "1.0.0");
    assert_eq!(evidence["platform"], "windows");
    assert!(["x86_64", "arm64"].contains(&evidence["architecture"].as_str().unwrap_or("")));
    assert!(
        ["WINDOWS_MSI", "WINDOWS_NSIS"].contains(&evidence["artifactType"].as_str().unwrap_or(""))
    );

    assert!(evidence["artifactIdentity"]["filePath"].is_string());
    assert!(evidence["artifactIdentity"]["buildProvenance"].is_string());

    assert!([
        "PASS",
        "FAIL",
        "NOT_APPLICABLE",
        "TOOL_UNAVAILABLE",
        "ARTIFACT_UNAVAILABLE"
    ]
    .contains(&evidence["installResult"]["status"].as_str().unwrap_or("")));
    assert!(evidence["installResult"]["method"].is_string());

    assert!(["PASS", "FAIL", "NOT_RUN"]
        .contains(&evidence["launchResult"]["status"].as_str().unwrap_or("")));
    assert!(evidence["launchResult"]["executablePath"].is_string());

    assert!(["PASS", "FAIL", "NOT_RUN"].contains(
        &evidence["runtimeResolutionResult"]["status"]
            .as_str()
            .unwrap_or("")
    ));
    assert!([
        "packaged",
        "development_override",
        "staged_development",
        "unknown"
    ]
    .contains(
        &evidence["runtimeResolutionResult"]["source"]
            .as_str()
            .unwrap_or("")
    ));

    assert!(["PASS", "FAIL", "NOT_RUN"].contains(
        &evidence["smokeDocumentResult"]["status"]
            .as_str()
            .unwrap_or("")
    ));
    assert_eq!(
        evidence["smokeDocumentResult"]["fixtureId"],
        SMOKE_FIXTURE_ID
    );
    assert!(evidence["smokeDocumentResult"]["workerUsed"].is_boolean());
    assert_eq!(
        evidence["smokeDocumentResult"]["workerSpawnMethod"],
        "production_ipc"
    );

    assert!(["PASS", "FAIL", "NOT_RUN"]
        .contains(&evidence["shutdownResult"]["status"].as_str().unwrap_or("")));
    assert!(evidence["shutdownResult"]["cleanExit"].is_boolean());

    assert!(evidence["externalValidationResults"].is_array());

    assert!(evidence["hardeningResult"].is_object());
    assert!(["PASS", "FAIL", "NOT_RUN", "NOT_APPLICABLE"]
        .contains(&evidence["hardeningResult"]["status"].as_str().unwrap_or("")));

    assert!(["PASS", "FAIL"].contains(&evidence["overallStatus"].as_str().unwrap_or("")));
    assert!(evidence["timestamp"].is_string());
    assert!(evidence["durationTotalMs"].is_number());
}

fn join<P: AsRef<std::path::Path>>(base: P, rel: &str) -> PathBuf {
    base.as_ref().join(rel)
}
