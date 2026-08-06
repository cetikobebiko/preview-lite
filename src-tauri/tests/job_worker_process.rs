use preview_alternative_lib::job_worker::JOB_WORKER_PROTOCOL_VERSION;
use serde_json::json;
use std::fs;
use std::process::Command;

fn run_worker(
    directory: &std::path::Path,
    id: u64,
    request_body: serde_json::Value,
) -> serde_json::Value {
    let request = directory.join(format!("worker-request-{id}.json"));
    let response = directory.join(format!("worker-response-{id}.json"));
    fs::write(&request, serde_json::to_vec(&request_body).unwrap()).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_preview-alternative"))
        .arg("--run-job-worker")
        .arg("--job-request")
        .arg(&request)
        .arg("--job-response")
        .arg(&response)
        .status()
        .unwrap();
    assert!(status.success());
    serde_json::from_slice(&fs::read(response).unwrap()).unwrap()
}

#[test]
fn ephemeral_job_worker_validates_an_immutable_snapshot_and_exits() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf/minimal.pdf"
        ),
        &snapshot,
    )
    .unwrap();
    let response = run_worker(
        directory.path(),
        1,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": "ValidateSnapshot"
        }),
    );
    assert_eq!(response["protocol_version"], JOB_WORKER_PROTOCOL_VERSION);
    assert_eq!(response["result"]["Ok"], "SnapshotValid");
}

#[test]
fn mutation_and_reopen_validation_run_as_separate_ephemeral_workers() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf/minimal.pdf"
        ),
        &snapshot,
    )
    .unwrap();

    let mutation = run_worker(
        directory.path(),
        10,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "MutatePdf": {
                    "recipe": {
                        "RotatePage": {
                            "page_index": 0,
                            "degrees": 90
                        }
                    },
                    "output_name": "mutation-output.pdf"
                }
            }
        }),
    );
    assert_eq!(
        mutation["result"]["Ok"]["File"],
        directory
            .path()
            .join("mutation-output.pdf")
            .to_string_lossy()
            .as_ref()
    );

    let validation = run_worker(
        directory.path(),
        11,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "ValidateMutationOutput": {
                    "output_name": "mutation-output.pdf"
                }
            }
        }),
    );
    assert_eq!(validation["result"]["Ok"], "MutationValidated");
}

#[test]
fn split_and_merge_outputs_are_worker_owned_and_reopen_validated() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    let additional = directory.path().join("merge-input-0.pdf");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf/minimal.pdf"
        ),
        &snapshot,
    )
    .unwrap();
    fs::copy(&snapshot, &additional).unwrap();

    let split = run_worker(
        directory.path(),
        20,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "SplitPdf": {
                    "ranges": [[0, 0]]
                }
            }
        }),
    );
    let split_path = split["result"]["Ok"]["Files"][0].as_str().unwrap();
    assert!(std::path::Path::new(split_path).is_file());
    let split_validation = run_worker(
        directory.path(),
        21,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "ValidateMutationOutput": {
                    "output_name": "split_0_0.pdf"
                }
            }
        }),
    );
    assert_eq!(split_validation["result"]["Ok"], "MutationValidated");

    let merged = run_worker(
        directory.path(),
        22,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "MergePdf": {
                    "additional_input_names": ["merge-input-0.pdf"],
                    "output_name": "merge-output.pdf"
                }
            }
        }),
    );
    assert_eq!(
        merged["result"]["Ok"]["File"],
        directory
            .path()
            .join("merge-output.pdf")
            .to_string_lossy()
            .as_ref()
    );
    let merge_validation = run_worker(
        directory.path(),
        23,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "ValidateMutationOutput": {
                    "output_name": "merge-output.pdf"
                }
            }
        }),
    );
    assert_eq!(merge_validation["result"]["Ok"], "MutationValidated");
}

#[test]
fn ephemeral_job_worker_scans_sensitive_data_with_page_geometry_v2() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pdf/reorder-rich.pdf"
        ),
        &snapshot,
    )
    .unwrap();

    let response = run_worker(
        directory.path(),
        30,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": {
                "ScanSensitiveData": {
                    "patterns": ["\\w+".to_string()]
                }
            }
        }),
    );

    assert_eq!(response["protocol_version"], JOB_WORKER_PROTOCOL_VERSION);
    let report = &response["result"]["Ok"]["SensitiveReport"];
    assert_eq!(report["schema_version"], 2);
    let items = report["items"]
        .as_array()
        .expect("items should be an array");
    assert!(!items.is_empty());
    let first = &items[0];
    assert_eq!(first["geometry_status"], "resolved");
    assert!(first["page_geometry"].is_object());
    assert_eq!(first["page_geometry"]["coordinate_space"], "pdf_user_space");
    assert_eq!(first["page_geometry"]["unit"], "pdf_point");
    assert_eq!(first["page_geometry"]["origin"], "bottom_left");
    assert!(!first["source_rects_pdf_points"]
        .as_array()
        .unwrap()
        .is_empty());
}
