use pdfium_render::prelude::*;
use preview_alternative_lib::job_worker::JOB_WORKER_PROTOCOL_VERSION;
use preview_alternative_lib::secure_redaction::{
    SecureRedactionPlan, GEOMETRY_EPSILON, MAX_PIXELS_PER_PAGE, MAX_RASTER_DPI, MIN_RASTER_DPI,
    REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS,
};
use preview_alternative_lib::secure_redaction_raster::{
    build_lossless_raster_payload_for_page, calculate_raster_dimensions,
    map_pdf_rect_to_device_pixel_rect, render_affected_page_raster, verify_opaque_black_overwrite,
    RedactionAnnotationPolicy, RedactionBackgroundPolicy, RedactionRasterSpec,
    RedactionWidgetFormPolicy, MAX_RENDER_BYTES,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

// Each high-DPI worker independently holds several bounded raster buffers.
// Keep the two characterizations from racing each other in Rust's default
// parallel test runner; production worker concurrency is unchanged.
static HIGH_DPI_WORKER_TEST_MUTEX: Mutex<()> = Mutex::new(());

fn run_worker(
    directory: &std::path::Path,
    id: u64,
    mut request_body: serde_json::Value,
) -> serde_json::Value {
    // Production requests are authoritative job envelopes. Keep older process
    // test fixtures focused on their command under test while exercising the
    // same required envelope shape.
    if let Some(request) = request_body.as_object_mut() {
        request
            .entry("authoritative_job_id")
            .or_insert_with(|| json!(format!("test-job-{id}")));
        request
            .entry("authoritative_session_id")
            .or_insert_with(|| json!(format!("test-session-{id}")));
    }
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
fn sanitization_inspection_worker_returns_a_bounded_read_only_report_without_output_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sanitization/unsupported-active-content.pdf"
        ),
        &snapshot,
    )
    .unwrap();
    let before = fs::read(&snapshot).unwrap();
    let response = run_worker(
        directory.path(),
        2,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "command": { "InspectDocumentSanitization": {
                "schema_version": "1.1.0",
                "policy_schema_version": "sanitization-policy-v1",
                "inspection_profile": "minimum-profile-inspection-v1"
            }}
        }),
    );
    let report = &response["result"]["Ok"]["SanitizationInspection"];
    assert_eq!(report["inspectionVersion"], "sanitization-inspection-v1");
    assert_eq!(report["documentFacts"]["xfaDetected"], true);
    assert_eq!(report["documentFacts"]["optionalContentDetected"], true);
    assert_eq!(report["documentFacts"]["richMediaDetected"], true);
    assert!(report["blockingFindings"].as_array().unwrap().len() >= 3);
    let encoded = serde_json::to_vec(report).unwrap();
    for marker in [
        b"SANITIZATION_XFA_PAYLOAD_001".as_slice(),
        b"SANITIZATION_RICHMEDIA_PAYLOAD_001".as_slice(),
        b"SANITIZATION_OPTIONAL_CONTENT_001".as_slice(),
    ] {
        assert!(!encoded.windows(marker.len()).any(|window| window == marker));
    }
    assert_eq!(fs::read(&snapshot).unwrap(), before);
    let file_names: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(file_names.iter().all(|name| {
        let name = name.to_string_lossy();
        name == "snapshot.pdf"
            || name.starts_with("worker-request-")
            || name.starts_with("worker-response-")
    }));
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
            "authoritative_job_id": "worker-test-job",
            "authoritative_session_id": "worker-test-session",
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

#[test]
fn ephemeral_job_worker_rejects_protocol_version_mismatch() {
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

    let old_version = JOB_WORKER_PROTOCOL_VERSION - 1;
    let response = run_worker(
        directory.path(),
        40,
        json!({
            "protocol_version": old_version,
            "snapshot_path": snapshot,
            "credential": null,
            "command": "ValidateSnapshot"
        }),
    );

    assert_eq!(response["protocol_version"], JOB_WORKER_PROTOCOL_VERSION);
    assert_eq!(
        response["result"]["Err"]["code"],
        "SESSION_JOB_WORKER_PROTOCOL_ERROR"
    );
}

#[test]
fn ephemeral_job_worker_executes_plan_secure_redaction() {
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

    let policy = json!({
        "schema_version": "1.0.0",
        "profile": "secure_rasterized_pages_v1",
        "strategy": "rasterized_pages",
        "raster_dpi": 300,
        "appearance": {
            "kind": "solid",
            "color": "black",
            "opacity": 1.0
        },
        "safety_margin_points": 0.0,
        "interactive_content_policy": "strip",
        "signature_policy": "block",
        "encryption_policy": "block"
    });

    let acknowledgements = json!({
        "schema_version": "1.0.0",
        "rasterized_pages_accepted": true,
        "text_search_loss_accepted": true,
        "accessibility_loss_accepted": true,
        "annotation_loss_accepted": true,
        "form_loss_accepted": true,
        "signature_invalidation_accepted": true,
        "encryption_change_accepted": true
    });

    let region = json!({
        "id": "r1",
        "page_index": 0,
        "normalized_top_left_rect": {
            "x": 0.1,
            "y": 0.1,
            "width": 0.2,
            "height": 0.2
        },
        "source": "manual"
    });

    let response = run_worker(
        directory.path(),
        41,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "authoritative_job_id": "worker-test-job",
            "authoritative_session_id": "worker-test-session",
            "command": {
                "PlanSecureRedaction": {
                    "content_generation": 1,
                    "policy": policy.clone(),
                    "acknowledgements": acknowledgements.clone(),
                    "regions": [region.clone()]
                }
            }
        }),
    );

    assert_eq!(response["protocol_version"], JOB_WORKER_PROTOCOL_VERSION);
    let plan = &response["result"]["Ok"]["SecureRedactionPlan"];
    assert_eq!(plan["schema_version"], "1.0.0");
    assert_eq!(plan["plan_version"], "secure_redaction_plan_v2");
    assert_eq!(plan["content_generation"], 1);
    assert!(plan["executable"].as_bool().unwrap());
    assert!(plan["ready_for_execution"].as_bool().unwrap());

    let input_digest = hex_sha256(&snapshot);
    let transform = run_worker(
        directory.path(),
        42,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "authoritative_job_id": "worker-test-job",
            "authoritative_session_id": "worker-test-session",
            "command": {
                "ApplySecureRedaction": {
                    "content_generation": 1,
                    "policy": policy.clone(),
                    "acknowledgements": acknowledgements.clone(),
                    "regions": [region.clone()],
                    "expected_input_digest": input_digest,
                    "expected_plan_digest": plan["plan_digest"],
                    "output_name": "secure-redaction-transform.pdf"
                }
            }
        }),
    );
    let report = &transform["result"]["Ok"]["SecureRedactionTransform"];
    assert_eq!(report["status"], "transformed_unverified");
    assert_eq!(report["affected_pages"], json!([0]));
    assert!(directory
        .path()
        .join("secure-redaction-transform.pdf")
        .is_file());

    // The payload is written as the only PDF image object without a lossy
    // encoder or an implicit resize. Inspect the reopened object itself so a
    // later page-render failure can be localized to placement/sampling rather
    // than payload conversion.
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let typed_plan: SecureRedactionPlan =
            serde_json::from_value(plan.clone()).map_err(|error| error.to_string())?;
        let source = pdfium
            .load_pdf_from_file(&snapshot, None)
            .map_err(|error| error.to_string())?;
        let expected = build_lossless_raster_payload_for_page(&typed_plan, &source, 0, None)
            .map_err(|error| error.to_string())?;
        let output = pdfium
            .load_pdf_from_file(
                &directory.path().join("secure-redaction-transform.pdf"),
                None,
            )
            .map_err(|error| error.to_string())?;
        let page = output.pages().get(0).map_err(|error| error.to_string())?;
        let object = page.objects().get(0).map_err(|error| error.to_string())?;
        let image = object
            .as_image_object()
            .ok_or_else(|| "reopened object is not an image".to_string())?;
        let raw = image
            .get_raw_image()
            .map_err(|error| error.to_string())?
            .to_rgba8();
        assert_eq!(
            (raw.width(), raw.height()),
            (expected.width, expected.height)
        );
        let rect = &expected.region_mappings[0].device_pixel_rect;
        let pixel = raw.get_pixel(
            rect.left as u32 + rect.width / 2,
            rect.top as u32 + rect.height / 2,
        );
        assert_eq!(pixel.0, [0, 0, 0, 255]);
        Ok::<(), String>(())
    })
    .unwrap();

    // A second, terminal worker process has no access to transform PDFium
    // handles or in-memory state. It must reopen the artifact independently.
    let verification = run_worker(
        directory.path(),
        43,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "authoritative_job_id": "worker-test-job",
            "authoritative_session_id": "worker-test-session",
            "command": { "VerifySecureRedaction": {
                "content_generation": 1,
                "policy": policy,
                "acknowledgements": acknowledgements,
                "regions": [region],
                "transform_report": report
            }}
        }),
    );
    let verification_report = &verification["result"]["Ok"]["SecureRedactionVerification"];
    assert_eq!(
        verification_report["verification_version"],
        "secure_redaction_verifier_v1"
    );
    assert_eq!(
        verification_report["is_verified"], true,
        "independent verification diagnostics: {verification_report}"
    );
    assert_eq!(
        verification_report["verification_checks"]
            .as_array()
            .unwrap()[11]["status"],
        "passed"
    );
    assert_eq!(
        verification_report["verification_checks"]
            .as_array()
            .unwrap()
            .len(),
        14
    );
}

fn hex_sha256(path: &std::path::Path) -> String {
    let bytes = fs::read(path).unwrap();
    format!("{:x}", Sha256::digest(bytes))
}

fn secure_redaction_request_parts() -> (Value, Value, Value) {
    (
        json!({
            "schema_version": "1.0.0", "profile": "secure_rasterized_pages_v1", "strategy": "rasterized_pages", "raster_dpi": 72,
            "appearance": { "kind": "solid", "color": "black", "opacity": 1.0 }, "safety_margin_points": 0.0,
            "interactive_content_policy": "strip", "signature_policy": "block", "encryption_policy": "block"
        }),
        json!({
            "schema_version": "1.0.0", "rasterized_pages_accepted": true, "text_search_loss_accepted": true,
            "accessibility_loss_accepted": true, "annotation_loss_accepted": true, "form_loss_accepted": true,
            "signature_invalidation_accepted": true, "encryption_change_accepted": true
        }),
        json!([{ "id": "r0", "page_index": 0, "normalized_top_left_rect": { "x": 0.1, "y": 0.1, "width": 0.1, "height": 0.1 }, "source": "manual" }]),
    )
}

#[test]
fn secure_redaction_worker_rejects_stale_digests_and_never_clobbers_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/redaction/simple-text.pdf"),
        &snapshot,
    )
    .unwrap();
    let (policy, acknowledgements, regions) = secure_redaction_request_parts();
    let plan_response = run_worker(
        directory.path(),
        160,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "command": { "PlanSecureRedaction": { "content_generation": 1, "policy": policy.clone(), "acknowledgements": acknowledgements.clone(), "regions": regions.clone() } }
        }),
    );
    let plan_digest = plan_response["result"]["Ok"]["SecureRedactionPlan"]["plan_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let actual_input_digest = hex_sha256(&snapshot);
    let apply_request = |expected_input_digest: String, expected_plan_digest: String| {
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "command": { "ApplySecureRedaction": {
                "content_generation": 1, "policy": policy.clone(), "acknowledgements": acknowledgements.clone(), "regions": regions.clone(),
                "expected_input_digest": expected_input_digest, "expected_plan_digest": expected_plan_digest, "output_name": "secure-redaction-transform.pdf"
            }}
        })
    };

    let stale = run_worker(
        directory.path(),
        161,
        apply_request("b".repeat(64), plan_digest.clone()),
    );
    assert_eq!(stale["result"]["Err"]["code"], "REDACTION_PLAN_STALE");
    let final_path = directory.path().join("secure-redaction-transform.pdf");
    assert!(!final_path.exists());

    let mismatched_plan = run_worker(
        directory.path(),
        162,
        apply_request(actual_input_digest.clone(), "c".repeat(64)),
    );
    assert_eq!(
        mismatched_plan["result"]["Err"]["code"],
        "REDACTION_PLAN_DIGEST_MISMATCH"
    );
    assert!(!final_path.exists());

    let preserved = b"must-not-be-overwritten";
    fs::write(&final_path, preserved).unwrap();
    let existing = run_worker(
        directory.path(),
        163,
        apply_request(actual_input_digest.clone(), plan_digest.clone()),
    );
    assert_eq!(
        existing["result"]["Err"]["code"],
        "SESSION_JOB_WORKER_PATH_REJECTED"
    );
    assert_eq!(fs::read(&final_path).unwrap(), preserved);
    fs::remove_file(&final_path).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let target = directory.path().join("symlink-target.pdf");
        let target_bytes = b"symlink-target-must-remain-unchanged";
        fs::write(&target, target_bytes).unwrap();
        symlink(&target, &final_path).unwrap();
        let symlink_response = run_worker(
            directory.path(),
            164,
            apply_request(actual_input_digest, plan_digest),
        );
        assert_eq!(
            symlink_response["result"]["Err"]["code"],
            "SESSION_JOB_WORKER_PATH_REJECTED"
        );
        assert_eq!(fs::read(&target).unwrap(), target_bytes);
    }

    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| entry
        .unwrap()
        .path()
        .extension()
        .is_none_or(|ext| ext != "tmp")));
}

#[test]
fn secure_redaction_worker_mixes_rasterized_and_imported_pages_in_one_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/redaction/mixed-secure-redaction-pages.pdf"),
        &snapshot,
    )
    .unwrap();
    let (policy, acknowledgements, regions) = secure_redaction_request_parts();
    let plan_response = run_worker(
        directory.path(),
        170,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "mixed-transform-job", "authoritative_session_id": "mixed-transform-session",
            "command": { "PlanSecureRedaction": {
                "content_generation": 1, "policy": policy.clone(), "acknowledgements": acknowledgements.clone(), "regions": regions.clone()
            }}
        }),
    );
    let plan = plan_response["result"]["Ok"]["SecureRedactionPlan"].clone();
    assert!(plan["executable"].as_bool().unwrap());
    assert!(plan["ready_for_execution"].as_bool().unwrap());
    assert_eq!(plan["page_plans"][0]["strategy"], "rasterize_page");
    assert_eq!(plan["page_plans"][1]["strategy"], "import_unaffected_page");

    let transform = run_worker(
        directory.path(),
        171,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "mixed-transform-job", "authoritative_session_id": "mixed-transform-session",
            "command": { "ApplySecureRedaction": {
                "content_generation": 1, "policy": policy, "acknowledgements": acknowledgements, "regions": regions,
                "expected_input_digest": hex_sha256(&snapshot), "expected_plan_digest": plan["plan_digest"],
                "output_name": "secure-redaction-transform.pdf"
            }}
        }),
    );
    let report = &transform["result"]["Ok"]["SecureRedactionTransform"];
    assert_eq!(report["status"], "transformed_unverified");
    assert_eq!(report["affected_pages"], json!([0]));
    assert_eq!(report["unaffected_pages"], json!([1]));
    assert_eq!(report["page_count"], 2);
    assert_eq!(
        report["transform_evidence"]["affected_page_original_objects_not_imported"],
        true
    );
    assert_eq!(
        report["transform_evidence"]["unaffected_pages_imported"],
        true
    );

    let output_path = directory.path().join("secure-redaction-transform.pdf");
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let typed_plan: SecureRedactionPlan = serde_json::from_value(plan.clone())
            .map_err(|error| error.to_string())?;
        let source = pdfium
            .load_pdf_from_file(&snapshot, None)
            .map_err(|error| error.to_string())?;
        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        assert_eq!(source.pages().len(), 2);
        assert_eq!(output.pages().len(), source.pages().len());

        let source_affected = source.pages().get(0).map_err(|error| error.to_string())?;
        let output_affected = output.pages().get(0).map_err(|error| error.to_string())?;
        assert!(source_affected.text().map_err(|error| error.to_string())?.all().contains("AFFECTED ORIGINAL TEXT MARKER"));
        assert!(source_affected
            .objects()
            .iter()
            .any(|object| object.object_type() == PdfPageObjectType::Text));
        assert!(
            !source_affected.annotations().is_empty(),
            "fixture must retain its controlled link annotation before transform"
        );
        assert!(output_affected
            .text()
            .map_err(|error| error.to_string())?
            .all()
            .trim()
            .is_empty());
        let output_object_types: Vec<_> = output_affected
            .objects()
            .iter()
            .map(|object| object.object_type())
            .collect();
        assert_eq!(output_object_types, vec![PdfPageObjectType::Image]);
        assert!(output_affected.annotations().is_empty());

        let expected = build_lossless_raster_payload_for_page(&typed_plan, &source, 0, None)
            .map_err(|error| error.to_string())?;
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: typed_plan.policy.raster_dpi,
            rotation: typed_plan.page_plans[0].geometry.rotation,
            viewport_width: expected.width,
            viewport_height: expected.height,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy: RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy: RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let (actual, _) = render_affected_page_raster(&output_affected, &spec, None)
            .map_err(|error| error.to_string())?;
        for mapping in &expected.region_mappings {
            let rect = &mapping.device_pixel_rect;
            for y in rect.top as u32..rect.top as u32 + rect.height {
                for x in rect.left as u32..rect.left as u32 + rect.width {
                    let offset = ((y * actual.width + x) * 4) as usize;
                    assert_eq!(
                        &actual.pixels[offset..offset + 4],
                        &[0, 0, 0, 255],
                        "saved affected page must retain opaque-black redaction coverage at ({x}, {y})"
                    );
                }
            }
        }

        let source_unaffected = source.pages().get(1).map_err(|error| error.to_string())?;
        let output_unaffected = output.pages().get(1).map_err(|error| error.to_string())?;
        assert!(output_unaffected
            .text()
            .map_err(|error| error.to_string())?
            .all()
            .contains("UNTOUCHED PAGE TEXT MARKER"));
        assert_eq!(source_unaffected.rotation().map_err(|error| error.to_string())?, output_unaffected.rotation().map_err(|error| error.to_string())?);
        for (source_box, output_box) in [
            (
                source_unaffected.boundaries().media().map_err(|error| error.to_string())?,
                output_unaffected.boundaries().media().map_err(|error| error.to_string())?,
            ),
            (
                source_unaffected.boundaries().crop().map_err(|error| error.to_string())?,
                output_unaffected.boundaries().crop().map_err(|error| error.to_string())?,
            ),
        ] {
            assert!((source_box.bounds.left().value - output_box.bounds.left().value).abs() <= 1e-6);
            assert!((source_box.bounds.bottom().value - output_box.bounds.bottom().value).abs() <= 1e-6);
            assert!((source_box.bounds.right().value - output_box.bounds.right().value).abs() <= 1e-6);
            assert!((source_box.bounds.top().value - output_box.bounds.top().value).abs() <= 1e-6);
        }
        let landmark = output_unaffected
            .render(600, 800, None)
            .map_err(|error| error.to_string())?
            .as_rgba_bytes();
        let landmark_offset = ((630 * 600 + 470) * 4) as usize;
        assert!(
            landmark[landmark_offset] < 30
                && landmark[landmark_offset + 1] > 220
                && landmark[landmark_offset + 2] < 30,
            "unaffected page's deterministic green landmark must survive import"
        );
        Ok::<(), String>(())
    })
    .unwrap();
}

#[test]
fn secure_redaction_worker_emits_a_full_rewrite_without_a_prev_chain() {
    // The source deliberately contains the literal `/Prev` inside a stream;
    // inspect the parsed trailer instead of treating byte presence as proof.
    let (directory, _) = transform_single_page_geometry_fixture("stream-literal-prev.pdf", 180);
    let source = fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/redaction/stream-literal-prev.pdf"),
    )
    .unwrap();
    let output_path = directory.path().join("secure-redaction-transform.pdf");
    let output = fs::read(&output_path).unwrap();
    assert!(output.starts_with(b"%PDF-"));
    assert!(!output.starts_with(&source));
    assert!(!output.windows(source.len()).any(|window| window == source));

    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        pdfium
            .load_pdf_from_file(&output_path, None)
            .map(|_| ())
            .map_err(|error| error.to_string())
    })
    .unwrap();
    let output_doc = lopdf::Document::load(&output_path).expect("lopdf must reopen full rewrite");
    assert!(
        output_doc.trailer.get(b"Prev").is_err(),
        "output trailer must not retain a prior-revision chain"
    );
}

#[test]
fn secure_redaction_worker_full_rewrite_drops_incremental_unreachable_objects() {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/redaction/incremental-revision.pdf");
    let source = fs::read(&source_path).unwrap();
    assert!(source
        .windows(b"UNREACHABLE_OLD_OBJECT_MARKER".len())
        .any(|window| window == b"UNREACHABLE_OLD_OBJECT_MARKER"));

    let (directory, plan) = transform_single_page_geometry_fixture("incremental-revision.pdf", 190);
    assert!(plan["executable"].as_bool().unwrap());
    assert!(plan["ready_for_execution"].as_bool().unwrap());
    let output_path = directory.path().join("secure-redaction-transform.pdf");
    let output = fs::read(&output_path).unwrap();
    assert!(output.starts_with(b"%PDF-"));
    assert!(!output.windows(source.len()).any(|window| window == source));
    assert!(
        !output
            .windows(b"UNREACHABLE_OLD_OBJECT_MARKER".len())
            .any(|window| window == b"UNREACHABLE_OLD_OBJECT_MARKER"),
        "unreachable payload from the source revision must not be carried into the new document"
    );

    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let output_doc = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        assert_eq!(output_doc.pages().len(), 1);
        Ok::<(), String>(())
    })
    .unwrap();
    let output_doc = lopdf::Document::load(&output_path).expect("lopdf must reopen full rewrite");
    assert!(
        output_doc.trailer.get(b"Prev").is_err(),
        "output trailer must not retain a prior-revision chain"
    );
}

/// Test-only record of a production worker protocol pipeline. In particular,
/// verification is deliberately not part of the transform worker invocation:
/// callers must invoke `verify()` after the transform process has exited.
struct SecureRedactionWorkerPipeline {
    directory: tempfile::TempDir,
    snapshot: PathBuf,
    policy: Value,
    acknowledgements: Value,
    regions: Value,
    plan: Value,
    transform_report: Value,
    raster_characterization: Option<(u32, u32, u64, usize)>,
    job_id: String,
    session_id: String,
}

impl SecureRedactionWorkerPipeline {
    fn output_path(&self) -> PathBuf {
        self.directory
            .path()
            .join(preview_alternative_lib::job_worker::SECURE_REDACTION_TRANSFORM_OUTPUT_NAME)
    }

    fn verify(&self, id: u64, transform_report: Value) -> Value {
        let response = run_worker(
            self.directory.path(),
            id,
            json!({
                "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
                "snapshot_path": self.snapshot,
                "credential": null,
                "authoritative_job_id": self.job_id,
                "authoritative_session_id": self.session_id,
                "command": { "VerifySecureRedaction": {
                    "content_generation": 1,
                    "policy": self.policy,
                    "acknowledgements": self.acknowledgements,
                    "regions": self.regions,
                    "transform_report": transform_report
                }}
            }),
        );
        response["result"]["Ok"]["SecureRedactionVerification"].clone()
    }

    fn verify_untouched(&self, id: u64) -> Value {
        self.verify(id, self.transform_report.clone())
    }
}

fn assert_all_required_verification_checks_passed(verification: &Value) {
    assert_eq!(
        verification["verification_version"], "secure_redaction_verifier_v1",
        "verification report: {verification}"
    );
    assert_eq!(
        verification["is_verified"], true,
        "verification report: {verification}"
    );
    assert!(
        verification["blockers"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "verification report: {verification}"
    );
    let checks = verification["verification_checks"]
        .as_array()
        .expect("production verifier must return checks");
    assert_eq!(
        checks.len(),
        REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS.len(),
        "verification report: {verification}"
    );
    for (check, expected_code) in checks
        .iter()
        .zip(REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS)
    {
        assert_eq!(
            check["code"], expected_code,
            "verification report: {verification}"
        );
        assert_eq!(
            check["status"], "passed",
            "verification report: {verification}"
        );
    }
}

/// Characterizes the bounded, non-sensitive dimensions that the production
/// planner and raster authority derive for the simple fixture before its
/// transform worker is started.
fn assert_simple_fixture_runnable_at_dpi(plan: &Value, dpi: u32) -> (u32, u32, u64, usize) {
    let typed_plan: SecureRedactionPlan =
        serde_json::from_value(plan.clone()).expect("worker plan must deserialize");
    assert_eq!(typed_plan.policy.raster_dpi, dpi, "plan: {plan}");
    let affected_page = typed_plan
        .page_plans
        .iter()
        .find(|page| page.page_index == 0)
        .expect("simple fixture must plan page zero");
    let (width, height) = calculate_raster_dimensions(&affected_page.geometry, dpi)
        .expect("simple fixture must fit the production raster resource policy");
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .expect("simple fixture pixel count must not overflow");
    let rgba_bytes = usize::try_from(pixels)
        .expect("simple fixture pixel count must fit usize")
        .checked_mul(4)
        .expect("simple fixture RGBA byte count must not overflow");
    assert!(
        pixels <= MAX_PIXELS_PER_PAGE,
        "DPI {dpi}: {width}x{height} = {pixels} pixels exceeds production per-page limit {MAX_PIXELS_PER_PAGE}"
    );
    assert!(
        rgba_bytes <= MAX_RENDER_BYTES,
        "DPI {dpi}: {rgba_bytes} RGBA bytes exceeds production render limit {MAX_RENDER_BYTES}"
    );
    (width, height, pixels, rgba_bytes)
}

/// Reads only PDF geometry and raster metadata from the serialized artifact.
/// Its diagnostic intentionally contains no document text or image payload.
fn high_dpi_observation_diagnostics(
    pipeline: &SecureRedactionWorkerPipeline,
    dpi: u32,
    expected_width: u32,
    expected_height: u32,
    pixels: u64,
    rgba_bytes: usize,
) -> String {
    let typed_plan: SecureRedactionPlan =
        serde_json::from_value(pipeline.plan.clone()).expect("worker plan must deserialize");
    let geometry = &typed_plan.page_plans[0].geometry;
    let expected = &geometry.effective_box;
    let output_size = fs::metadata(pipeline.output_path())
        .expect("transformed artifact metadata must be available")
        .len();
    let (actual, embedded_width, embedded_height) =
        preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
            let output = pdfium
                .load_pdf_from_file(&pipeline.output_path(), None)
                .map_err(|error| error.to_string())?;
            let page = output.pages().get(0).map_err(|error| error.to_string())?;
            let object = page.objects().get(0).map_err(|error| error.to_string())?;
            let bounds = object
                .bounds()
                .map_err(|error| error.to_string())?
                .to_rect();
            let image = object
                .as_image_object()
                .ok_or_else(|| "affected output object must be an image".to_string())?;
            let raw = image
                .get_raw_image()
                .map_err(|error| error.to_string())?
                .to_rgba8();
            Ok::<_, String>((
                (
                    bounds.left().value as f64,
                    bounds.bottom().value as f64,
                    bounds.right().value as f64,
                    bounds.top().value as f64,
                ),
                raw.width(),
                raw.height(),
            ))
        })
        .expect("artifact must expose its affected PDFium image observation");
    assert_eq!(
        (embedded_width, embedded_height),
        (expected_width, expected_height),
        "DPI {dpi}: actual embedded dimensions must equal calculate_raster_dimensions()"
    );

    let expected_edges = [
        expected.left,
        expected.bottom,
        expected.left + expected.width,
        expected.bottom + expected.height,
    ];
    let actual_edges = [actual.0, actual.1, actual.2, actual.3];
    let page_coordinate_scale = expected_edges
        .into_iter()
        .chain([expected.width, expected.height, 1.0])
        .map(f64::abs)
        .fold(1.0_f64, f64::max);
    let f32_scale = page_coordinate_scale as f32;
    let ulp_tolerance = (f32::from_bits(f32_scale.to_bits() + 1) as f64) - f32_scale as f64;
    let deltas: Vec<f64> = actual_edges
        .iter()
        .zip(expected_edges.iter())
        .map(|(actual, expected)| actual - expected)
        .collect();
    let ratios: Vec<f64> = deltas
        .iter()
        .map(|delta| delta.abs() / ulp_tolerance)
        .collect();
    format!(
        "DPI={dpi}; page_coordinate_scale={page_coordinate_scale:.12}; expected left/bottom/right/top=({:.12}, {:.12}, {:.12}, {:.12}); actual left/bottom/right/top=({:.12}, {:.12}, {:.12}, {:.12}); delta per edge=({:.12}, {:.12}, {:.12}, {:.12}); derived_f32_ulp_tolerance={ulp_tolerance:.12}; delta/tolerance ratio=({:.3}, {:.3}, {:.3}, {:.3}); raster width={expected_width}; raster height={expected_height}; pixels/page={pixels}; raw RGBA bytes={rgba_bytes}; output artifact size={output_size}; rotation={:?}",
        expected_edges[0], expected_edges[1], expected_edges[2], expected_edges[3],
        actual_edges[0], actual_edges[1], actual_edges[2], actual_edges[3],
        deltas[0], deltas[1], deltas[2], deltas[3],
        ratios[0], ratios[1], ratios[2], ratios[3],
        geometry.rotation,
    )
}

fn verification_check<'a>(verification: &'a Value, code: &str) -> &'a Value {
    verification["verification_checks"]
        .as_array()
        .expect("production verifier must return checks")
        .iter()
        .find(|check| check["code"] == code)
        .unwrap_or_else(|| panic!("missing production verification check {code}: {verification}"))
}

fn run_secure_redaction_worker_pipeline(
    fixture: &str,
    regions: Value,
    id: u64,
) -> SecureRedactionWorkerPipeline {
    run_secure_redaction_worker_pipeline_at_dpi(fixture, regions, id, 72)
}

fn run_secure_redaction_worker_pipeline_at_dpi(
    fixture: &str,
    regions: Value,
    id: u64,
    raster_dpi: u32,
) -> SecureRedactionWorkerPipeline {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/redaction")
            .join(fixture),
        &snapshot,
    )
    .unwrap();
    let (mut policy, acknowledgements, _) = secure_redaction_request_parts();
    policy["raster_dpi"] = json!(raster_dpi);
    let job_id = format!("acceptance-job-{id}");
    let session_id = format!("acceptance-session-{id}");
    let plan_response = run_worker(
        directory.path(),
        id,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "authoritative_job_id": job_id,
            "authoritative_session_id": session_id,
            "command": { "PlanSecureRedaction": {
                "content_generation": 1,
                "policy": policy,
                "acknowledgements": acknowledgements,
                "regions": regions
            }}
        }),
    );
    let plan = plan_response["result"]["Ok"]["SecureRedactionPlan"].clone();
    assert!(plan["executable"].as_bool().unwrap(), "plan: {plan}");
    assert!(
        plan["ready_for_execution"].as_bool().unwrap(),
        "plan: {plan}"
    );
    // This must happen before the transform worker begins: the high-DPI
    // acceptance fixture is only run when its real planned geometry fits the
    // current production resource envelope.
    let raster_characterization = (fixture == "simple-text.pdf"
        && (raster_dpi == 450 || raster_dpi == MAX_RASTER_DPI))
        .then(|| assert_simple_fixture_runnable_at_dpi(&plan, raster_dpi));
    let transform_response = run_worker(
        directory.path(),
        id + 1,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION,
            "snapshot_path": snapshot,
            "credential": null,
            "authoritative_job_id": job_id,
            "authoritative_session_id": session_id,
            "command": { "ApplySecureRedaction": {
                "content_generation": 1,
                "policy": policy,
                "acknowledgements": acknowledgements,
                "regions": regions,
                "expected_input_digest": hex_sha256(&snapshot),
                "expected_plan_digest": plan["plan_digest"],
                "output_name": preview_alternative_lib::job_worker::SECURE_REDACTION_TRANSFORM_OUTPUT_NAME
            }}
        }),
    );
    let transform_report = transform_response["result"]["Ok"]["SecureRedactionTransform"].clone();
    assert_eq!(transform_report["status"], "transformed_unverified");
    assert!(directory
        .path()
        .join(preview_alternative_lib::job_worker::SECURE_REDACTION_TRANSFORM_OUTPUT_NAME)
        .is_file());
    SecureRedactionWorkerPipeline {
        directory,
        snapshot,
        policy,
        acknowledgements,
        regions,
        plan,
        transform_report,
        raster_characterization,
        job_id,
        session_id,
    }
}

fn single_page_acceptance_regions() -> Value {
    json!([{
        "id": "r0",
        "page_index": 0,
        "normalized_top_left_rect": { "x": 0.1, "y": 0.1, "width": 0.1, "height": 0.1 },
        "source": "manual"
    }])
}

/// Changes exactly one opaque-black pixel in the affected page's embedded
/// raster. The coordinate is derived from the canonical plan payload rather
/// than being a hard-coded PDF/device coordinate.
fn corrupt_one_covered_raster_pixel(pipeline: &SecureRedactionWorkerPipeline) -> (u32, u32) {
    let typed_plan: SecureRedactionPlan = serde_json::from_value(pipeline.plan.clone()).unwrap();
    let output_path = pipeline.output_path();
    let staging_path = pipeline
        .directory
        .path()
        .join("corrupted-raster-artifact.pdf");
    let coordinate = preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let source = pdfium
            .load_pdf_from_file(&pipeline.snapshot, None)
            .map_err(|error| error.to_string())?;
        let expected = build_lossless_raster_payload_for_page(&typed_plan, &source, 0, None)
            .map_err(|error| error.to_string())?;
        let rect = &expected.region_mappings[0].device_pixel_rect;
        assert!(
            rect.width >= 3 && rect.height >= 3,
            "coverage rect must have an interior pixel"
        );
        let x = rect.left as u32 + rect.width / 2;
        let y = rect.top as u32 + rect.height / 2;
        assert!(
            x > rect.left as u32
                && x < rect.left as u32 + rect.width - 1
                && y > rect.top as u32
                && y < rect.top as u32 + rect.height - 1,
            "mutation coordinate must be strictly inside canonical coverage"
        );

        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        let mut page = output.pages().get(0).map_err(|error| error.to_string())?;
        let mut object = page
            .objects_mut()
            .get(0)
            .map_err(|error| error.to_string())?;
        let image = object
            .as_image_object_mut()
            .ok_or_else(|| "affected output object must be an image".to_string())?;
        let mut raw = image
            .get_raw_image()
            .map_err(|error| error.to_string())?
            .to_rgba8();
        assert_eq!(
            (raw.width(), raw.height()),
            (expected.width, expected.height)
        );
        assert_eq!(raw.get_pixel(x, y).0, [0, 0, 0, 255]);
        raw.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        image
            .set_image(&image::DynamicImage::ImageRgba8(raw))
            .map_err(|error| error.to_string())?;
        page.regenerate_content()
            .map_err(|error| error.to_string())?;
        output
            .save_to_file(&staging_path)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((x, y))
    })
    .unwrap();
    fs::rename(&staging_path, &output_path).unwrap();
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        let page = output.pages().get(0).map_err(|error| error.to_string())?;
        let object = page.objects().get(0).map_err(|error| error.to_string())?;
        let image = object
            .as_image_object()
            .ok_or_else(|| "serialized affected output object must be an image".to_string())?;
        assert_eq!(
            image
                .get_raw_image()
                .map_err(|error| error.to_string())?
                .to_rgba8()
                .get_pixel(coordinate.0, coordinate.1)
                .0,
            [255, 255, 255, 255],
            "serialized embedded raster must retain the one-pixel mutation"
        );
        Ok::<_, String>(())
    })
    .unwrap();
    coordinate
}

/// Mirrors the sole affected-page image about the effective page's vertical
/// center. The axis-aligned bounds and embedded bytes remain unchanged, but
/// asymmetric canonical content moves in the reopened render.
fn mirror_affected_image_placement(pipeline: &SecureRedactionWorkerPipeline) {
    let output_path = pipeline.output_path();
    let staging_path = pipeline
        .directory
        .path()
        .join("mirrored-placement-artifact.pdf");
    let original_pixels = preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        let mut page = output.pages().get(0).map_err(|error| error.to_string())?;
        let mut object = page
            .objects_mut()
            .get(0)
            .map_err(|error| error.to_string())?;
        let bounds = object
            .bounds()
            .map_err(|error| error.to_string())?
            .to_rect();
        let raw = object
            .as_image_object()
            .ok_or_else(|| "affected output object must be an image".to_string())?
            .get_raw_image()
            .map_err(|error| error.to_string())?
            .to_rgba8()
            .into_raw();
        object
            .transform(
                -1.0,
                0.0,
                0.0,
                1.0,
                bounds.left().value + bounds.right().value,
                0.0,
            )
            .map_err(|error| error.to_string())?;
        let mirrored_bounds = object
            .bounds()
            .map_err(|error| error.to_string())?
            .to_rect();
        assert!((mirrored_bounds.left().value - bounds.left().value).abs() <= 1e-6);
        assert!((mirrored_bounds.right().value - bounds.right().value).abs() <= 1e-6);
        assert!((mirrored_bounds.bottom().value - bounds.bottom().value).abs() <= 1e-6);
        assert!((mirrored_bounds.top().value - bounds.top().value).abs() <= 1e-6);
        page.regenerate_content()
            .map_err(|error| error.to_string())?;
        output
            .save_to_file(&staging_path)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(raw)
    })
    .unwrap();
    fs::rename(&staging_path, &output_path).unwrap();
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        let page = output.pages().get(0).map_err(|error| error.to_string())?;
        let object = page.objects().get(0).map_err(|error| error.to_string())?;
        let serialized_pixels = object
            .as_image_object()
            .ok_or_else(|| "serialized affected output object must be an image".to_string())?
            .get_raw_image()
            .map_err(|error| error.to_string())?
            .to_rgba8()
            .into_raw();
        assert_eq!(serialized_pixels, original_pixels);
        Ok::<_, String>(())
    })
    .unwrap();
}

/// Moves the affected image by a value intentionally far beyond the f32 ULP
/// observation budget. The embedded raster remains intact; this isolates the
/// structural page-object placement check.
fn translate_affected_image_placement(
    pipeline: &SecureRedactionWorkerPipeline,
    delta_x: f32,
    delta_y: f32,
) {
    let output_path = pipeline.output_path();
    let staging_path = pipeline
        .directory
        .path()
        .join("translated-placement-artifact.pdf");
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        let mut page = output.pages().get(0).map_err(|error| error.to_string())?;
        let mut object = page
            .objects_mut()
            .get(0)
            .map_err(|error| error.to_string())?;
        object
            .transform(1.0, 0.0, 0.0, 1.0, delta_x, delta_y)
            .map_err(|error| error.to_string())?;
        page.regenerate_content()
            .map_err(|error| error.to_string())?;
        output
            .save_to_file(&staging_path)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })
    .unwrap();
    fs::rename(&staging_path, &output_path).unwrap();
}

fn transform_single_page_geometry_fixture(fixture: &str, id: u64) -> (tempfile::TempDir, Value) {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/redaction")
            .join(fixture),
        &snapshot,
    )
    .unwrap();
    let policy = json!({
        "schema_version": "1.0.0", "profile": "secure_rasterized_pages_v1", "strategy": "rasterized_pages", "raster_dpi": 72,
        "appearance": { "kind": "solid", "color": "black", "opacity": 1.0 }, "safety_margin_points": 0.0,
        "interactive_content_policy": "strip", "signature_policy": "block", "encryption_policy": "block"
    });
    let acknowledgements = json!({
        "schema_version": "1.0.0", "rasterized_pages_accepted": true, "text_search_loss_accepted": true,
        "accessibility_loss_accepted": true, "annotation_loss_accepted": true, "form_loss_accepted": true,
        "signature_invalidation_accepted": true, "encryption_change_accepted": true
    });
    let regions = json!([{ "id": "r0", "page_index": 0, "normalized_top_left_rect": { "x": 0.1, "y": 0.1, "width": 0.1, "height": 0.1 }, "source": "manual" }]);
    let plan_response = run_worker(
        directory.path(),
        id,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "geometry-test-job", "authoritative_session_id": "geometry-test-session",
            "command": { "PlanSecureRedaction": { "content_generation": 1, "policy": policy.clone(), "acknowledgements": acknowledgements.clone(), "regions": regions.clone() } }
        }),
    );
    let plan = plan_response["result"]["Ok"]["SecureRedactionPlan"].clone();
    assert!(plan["executable"].as_bool().unwrap());
    let expected_plan_digest = plan["plan_digest"].as_str().unwrap().to_string();
    let transformed = run_worker(
        directory.path(),
        id + 1,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "geometry-test-job", "authoritative_session_id": "geometry-test-session",
            "command": { "ApplySecureRedaction": {
                "content_generation": 1, "policy": policy, "acknowledgements": acknowledgements, "regions": regions,
                "expected_input_digest": hex_sha256(&snapshot), "expected_plan_digest": expected_plan_digest, "output_name": "secure-redaction-transform.pdf"
            }}
        }),
    );
    assert_eq!(
        transformed["result"]["Ok"]["SecureRedactionTransform"]["status"],
        "transformed_unverified"
    );
    (directory, plan)
}

#[test]
fn secure_redaction_worker_round_trips_rotated_page_geometry() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = directory.path().join("snapshot.pdf");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/redaction/rotations.pdf"
        ),
        &snapshot,
    )
    .unwrap();
    let policy = json!({
        "schema_version": "1.0.0", "profile": "secure_rasterized_pages_v1", "strategy": "rasterized_pages", "raster_dpi": 72,
        "appearance": { "kind": "solid", "color": "black", "opacity": 1.0 }, "safety_margin_points": 0.0,
        "interactive_content_policy": "strip", "signature_policy": "block", "encryption_policy": "block"
    });
    let acknowledgements = json!({
        "schema_version": "1.0.0", "rasterized_pages_accepted": true, "text_search_loss_accepted": true,
        "accessibility_loss_accepted": true, "annotation_loss_accepted": true, "form_loss_accepted": true,
        "signature_invalidation_accepted": true, "encryption_change_accepted": true
    });
    let regions: Vec<_> = (0..4)
        .map(|page_index| json!({
            "id": format!("r{page_index}"), "page_index": page_index,
            "normalized_top_left_rect": { "x": 0.7, "y": 0.7, "width": 0.1, "height": 0.1 }, "source": "manual"
        }))
        .collect();
    let plan_response = run_worker(
        directory.path(),
        50,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "rotation-test-job", "authoritative_session_id": "rotation-test-session",
            "command": { "PlanSecureRedaction": { "content_generation": 1, "policy": policy.clone(), "acknowledgements": acknowledgements.clone(), "regions": regions.clone() } }
        }),
    );
    let plan = &plan_response["result"]["Ok"]["SecureRedactionPlan"];
    assert!(plan["executable"].as_bool().unwrap());
    let transform_response = run_worker(
        directory.path(),
        51,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "rotation-test-job", "authoritative_session_id": "rotation-test-session",
            "command": { "ApplySecureRedaction": {
                "content_generation": 1, "policy": policy.clone(), "acknowledgements": acknowledgements.clone(), "regions": regions.clone(),
                "expected_input_digest": hex_sha256(&snapshot), "expected_plan_digest": plan["plan_digest"], "output_name": "secure-redaction-transform.pdf"
            }}
        }),
    );
    assert_eq!(
        transform_response["result"]["Ok"]["SecureRedactionTransform"]["status"],
        "transformed_unverified"
    );
    let verification_response = run_worker(
        directory.path(),
        52,
        json!({
            "protocol_version": JOB_WORKER_PROTOCOL_VERSION, "snapshot_path": snapshot, "credential": null,
            "authoritative_job_id": "rotation-test-job", "authoritative_session_id": "rotation-test-session",
            "command": { "VerifySecureRedaction": {
                "content_generation": 1, "policy": policy, "acknowledgements": acknowledgements, "regions": regions,
                "transform_report": transform_response["result"]["Ok"]["SecureRedactionTransform"]
            }}
        }),
    );
    let verification = &verification_response["result"]["Ok"]["SecureRedactionVerification"];
    assert_eq!(verification["is_verified"], true, "{verification}");

    let output_path = directory.path().join("secure-redaction-transform.pdf");
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
        let typed_plan: SecureRedactionPlan = serde_json::from_value(plan.clone())
            .map_err(|error| error.to_string())?;
        let source = pdfium
            .load_pdf_from_file(&snapshot, None)
            .map_err(|error| error.to_string())?;
        let output = pdfium
            .load_pdf_from_file(&output_path, None)
            .map_err(|error| error.to_string())?;
        for (page_index, rotation) in [
            PdfPageRenderRotation::None,
            PdfPageRenderRotation::Degrees90,
            PdfPageRenderRotation::Degrees180,
            PdfPageRenderRotation::Degrees270,
        ]
        .into_iter()
        .enumerate()
        {
            let page = output
                .pages()
                .get(page_index as u16)
                .map_err(|error| error.to_string())?;
            let media = page
                .boundaries()
                .media()
                .map_err(|error| error.to_string())?;
            assert!((media.bounds.left().value - 0.0).abs() <= 1e-6);
            assert!((media.bounds.bottom().value - 0.0).abs() <= 1e-6);
            assert!((media.bounds.right().value - 600.0).abs() <= 1e-6);
            assert!((media.bounds.top().value - 800.0).abs() <= 1e-6);
            assert_eq!(
                page.rotation().map_err(|error| error.to_string())?,
                rotation
            );

            let expected = build_lossless_raster_payload_for_page(
                &typed_plan,
                &source,
                page_index as u32,
                None,
            )
            .map_err(|error| error.to_string())?;
            let spec = RedactionRasterSpec {
                page_index: page_index as u32,
                dpi: typed_plan.policy.raster_dpi,
                rotation: typed_plan.page_plans[page_index].geometry.rotation,
                viewport_width: expected.width,
                viewport_height: expected.height,
                background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
                annotation_policy: RedactionAnnotationPolicy::DoNotRender,
                widget_form_policy: RedactionWidgetFormPolicy::BlockedIfPresent,
            };
            let (actual, output_config) = render_affected_page_raster(&page, &spec, None)
                .map_err(|error| error.to_string())?;
            for mapping in &expected.region_mappings {
                let canonical = typed_plan.page_plans[page_index]
                    .canonical_regions
                    .iter()
                    .find(|region| region.region_id == mapping.canonical_region_id)
                    .expect("canonical mapping must remain plan-bound");
                let (output_rect, _) = map_pdf_rect_to_device_pixel_rect(
                    &page,
                    &canonical.applied_pdf_point_rect,
                    &output_config,
                    actual.width,
                    actual.height,
                )
                .map_err(|error| error.to_string())?;
                verify_opaque_black_overwrite(&actual, &[output_rect])
                    .map_err(|error| error.to_string())?;
                let rect = &canonical.applied_pdf_point_rect;
                for (x, y) in [
                    (rect.left, rect.bottom),
                    (rect.left + rect.width, rect.bottom),
                    (rect.left, rect.bottom + rect.height),
                    (rect.left + rect.width, rect.bottom + rect.height),
                ] {
                    let (pixel_x, pixel_y) = page
                        .points_to_pixels(
                            PdfPoints::new(x as f32),
                            PdfPoints::new(y as f32),
                            &output_config,
                        )
                        .map_err(|error| error.to_string())?;
                    let (roundtrip_x, roundtrip_y) = page
                        .pixels_to_points(pixel_x, pixel_y, &output_config)
                        .map_err(|error| error.to_string())?;
                    assert!((roundtrip_x.value as f64 - x).abs() <= 1e-3);
                    assert!((roundtrip_y.value as f64 - y).abs() <= 1e-3);
                }
                let rect = &mapping.device_pixel_rect;
                for y in rect.top as u32..(rect.top as u32 + rect.height) {
                    for x in rect.left as u32..(rect.left as u32 + rect.width) {
                        let offset = ((y * actual.width + x) * 4) as usize;
                        assert_eq!(
                            &actual.pixels[offset..offset + 4],
                            &[0, 0, 0, 255],
                            "saved artifact must preserve opaque-black coverage for page {page_index} at ({x}, {y})"
                        );
                    }
                }
            }
            let landmark = (0..expected.height)
                .flat_map(|y| (0..expected.width).map(move |x| (x, y)))
                .find(|(x, y)| {
                    let in_redaction = expected.region_mappings.iter().any(|mapping| {
                        let rect = &mapping.device_pixel_rect;
                        *x >= rect.left as u32
                            && *x < rect.left as u32 + rect.width
                            && *y >= rect.top as u32
                            && *y < rect.top as u32 + rect.height
                    });
                    let offset = ((*y * expected.width + *x) * 4) as usize;
                    !in_redaction && expected.pixels[offset] < 128
                })
                .expect("source fixture must contain a non-redacted dark landmark");
            let offset = ((landmark.1 * expected.width + landmark.0) * 4) as usize;
            for channel in 0..3 {
                assert!(
                    (actual.pixels[offset + channel] as i16 - expected.pixels[offset + channel] as i16)
                        .abs()
                        <= 8,
                    "saved output changed rotation/orientation at page {page_index}, landmark {landmark:?}"
                );
            }
        }
        Ok::<(), String>(())
    })
    .unwrap();
}

#[test]
fn secure_redaction_worker_round_trips_crop_and_nonzero_origins() {
    for (offset, fixture) in [
        "cropbox-mediabox-diff.pdf",
        "explicit-print-boxes.pdf",
        "nonzero-origin.pdf",
        "negative-origin.pdf",
    ]
    .into_iter()
    .enumerate()
    {
        let (directory, plan) =
            transform_single_page_geometry_fixture(fixture, 70 + (offset as u64 * 2));
        let expected = &plan["page_plans"][0]["geometry"];
        let output_path = directory.path().join("secure-redaction-transform.pdf");
        preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
            let output = pdfium
                .load_pdf_from_file(&output_path, None)
                .map_err(|error| error.to_string())?;
            let page = output.pages().get(0).map_err(|error| error.to_string())?;
            let media = page
                .boundaries()
                .media()
                .map_err(|error| error.to_string())?;
            let assert_box = |actual: PdfRect, expected: &Value| {
                assert!(
                    (actual.left().value as f64 - expected["left"].as_f64().unwrap()).abs() <= 1e-6
                );
                assert!(
                    (actual.bottom().value as f64 - expected["bottom"].as_f64().unwrap()).abs()
                        <= 1e-6
                );
                assert!(
                    (actual.right().value as f64
                        - (expected["left"].as_f64().unwrap()
                            + expected["width"].as_f64().unwrap()))
                    .abs()
                        <= 1e-6
                );
                assert!(
                    (actual.top().value as f64
                        - (expected["bottom"].as_f64().unwrap()
                            + expected["height"].as_f64().unwrap()))
                    .abs()
                        <= 1e-6
                );
            };
            assert_box(media.bounds, &expected["media_box"]);
            match (&expected["crop_box"], page.boundaries().crop()) {
                (Value::Null, Err(_)) => {}
                (expected_crop, Ok(actual_crop)) => assert_box(actual_crop.bounds, expected_crop),
                _ => panic!("saved CropBox presence must match canonical geometry"),
            }
            for (name, expected_box, actual_box) in [
                ("TrimBox", &expected["trim_box"], page.boundaries().trim()),
                (
                    "BleedBox",
                    &expected["bleed_box"],
                    page.boundaries().bleed(),
                ),
                ("ArtBox", &expected["art_box"], page.boundaries().art()),
            ] {
                match (expected_box, actual_box) {
                    (Value::Null, Err(_)) => {}
                    (expected, Ok(actual)) => assert_box(actual.bounds, expected),
                    _ => panic!("saved {name} presence must match canonical geometry"),
                }
            }
            Ok::<(), String>(())
        })
        .unwrap();
    }
}

#[test]
fn secure_redaction_supported_geometry_fixtures_pass_separate_verifier_acceptance() {
    // Exercise each rotation as its own Plan -> Transform -> terminal Verify
    // pipeline. The remaining pages are intentionally retained as unaffected
    // pages, so this also keeps the import-equivalence verifier path covered.
    for (offset, (rotation, page_index)) in [(0, 0), (90, 1), (180, 2), (270, 3)]
        .into_iter()
        .enumerate()
    {
        let pipeline = run_secure_redaction_worker_pipeline(
            "rotations.pdf",
            json!([
                {
                    "id": format!("rotation-{rotation}"),
                    "page_index": page_index,
                    "normalized_top_left_rect": { "x": 0.7, "y": 0.7, "width": 0.1, "height": 0.1 },
                    "source": "manual"
                },
                {
                    "id": format!("rotation-{rotation}-edge"),
                    "page_index": page_index,
                    "normalized_top_left_rect": { "x": 0.82, "y": 0.03, "width": 0.16, "height": 0.08 },
                    "source": "manual"
                }
            ]),
            300 + (offset as u64 * 10),
        );
        assert_eq!(
            pipeline.plan["page_plans"][page_index]["geometry"]["rotation"], rotation,
            "fixture page must retain its claimed rotation"
        );
        assert_all_required_verification_checks_passed(
            &pipeline.verify_untouched(302 + (offset as u64 * 10)),
        );
    }

    for (offset, fixture) in [
        "cropbox-mediabox-diff.pdf",
        "nonzero-origin.pdf",
        "negative-origin.pdf",
    ]
    .into_iter()
    .enumerate()
    {
        let pipeline = run_secure_redaction_worker_pipeline(
            fixture,
            single_page_acceptance_regions(),
            400 + (offset as u64 * 10),
        );
        let geometry = &pipeline.plan["page_plans"][0]["geometry"];
        match fixture {
            "cropbox-mediabox-diff.pdf" => {
                assert_ne!(geometry["media_box"], geometry["crop_box"]);
            }
            "nonzero-origin.pdf" => {
                assert!(
                    geometry["media_box"]["left"].as_f64().unwrap() != 0.0
                        || geometry["media_box"]["bottom"].as_f64().unwrap() != 0.0
                );
            }
            "negative-origin.pdf" => {
                assert!(
                    geometry["media_box"]["left"].as_f64().unwrap() < 0.0
                        || geometry["media_box"]["bottom"].as_f64().unwrap() < 0.0
                );
            }
            _ => unreachable!(),
        }
        let verification = pipeline.verify_untouched(402 + (offset as u64 * 10));
        assert_all_required_verification_checks_passed(&verification);
        assert_eq!(
            verification_check(&verification, "PAGE_GEOMETRY")["status"],
            "passed"
        );
        assert_eq!(
            verification_check(&verification, "AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE")["status"],
            "passed"
        );
    }
}

#[test]
fn secure_redaction_object_bounds_observation_passes_representative_dpis() {
    // Object bounds are in page points, so their f32 precision budget must not
    // be coupled to raster density. These are separate plan, transform, and
    // verifier workers; the high-DPI characterizations below exercise the
    // same protocol at 450 and the policy maximum.
    for (offset, dpi) in [MIN_RASTER_DPI, 300].into_iter().enumerate() {
        let pipeline = run_secure_redaction_worker_pipeline_at_dpi(
            "simple-text.pdf",
            single_page_acceptance_regions(),
            550 + (offset as u64 * 10),
            dpi,
        );
        let verification = pipeline.verify_untouched(552 + (offset as u64 * 10));
        assert_eq!(
            verification_check(&verification, "AFFECTED_PAGE_IMAGE_ONLY")["status"],
            "passed",
            "DPI {dpi}: {verification}"
        );
    }
}

fn assert_high_dpi_worker_acceptance(dpi: u32, id: u64) {
    assert_eq!(GEOMETRY_EPSILON, 1e-6, "canonical geometry epsilon changed");
    let pipeline = run_secure_redaction_worker_pipeline_at_dpi(
        "simple-text.pdf",
        single_page_acceptance_regions(),
        id,
        dpi,
    );
    let (width, height, pixels, rgba_bytes) = pipeline
        .raster_characterization
        .expect("high-DPI simple fixture must have been preflighted before transform");
    // Do not retain a high-DPI raw image in the test process while the separate
    // verifier worker performs its own exact embedded and reopened-render
    // observations. The verifier response is available even when a required
    // check fails, so we can still attach the complete bounded diagnostics
    // below before making acceptance assertions.
    let verification = pipeline.verify_untouched(id + 2);
    let diagnostics =
        high_dpi_observation_diagnostics(&pipeline, dpi, width, height, pixels, rgba_bytes);
    eprintln!("secure-redaction high-DPI characterization: {diagnostics}");
    assert_all_required_verification_checks_passed(&verification);
    for code in [
        "AFFECTED_PAGE_IMAGE_ONLY",
        "AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE",
        "PAGE_GEOMETRY",
        "ARTIFACT_FINAL_DIGEST_STABILITY",
    ] {
        assert_eq!(
            verification_check(&verification, code)["status"],
            "passed",
            "{diagnostics}; verification={verification}"
        );
    }
}

#[test]
fn secure_redaction_worker_accepts_high_dpi_450() {
    let _guard = HIGH_DPI_WORKER_TEST_MUTEX.lock().unwrap();
    assert_high_dpi_worker_acceptance(450, 570);
}

#[test]
fn secure_redaction_worker_accepts_policy_max_dpi_600() {
    let _guard = HIGH_DPI_WORKER_TEST_MUTEX.lock().unwrap();
    assert_high_dpi_worker_acceptance(MAX_RASTER_DPI, 580);
}

#[test]
fn secure_redaction_explicit_print_boxes_pass_separate_verifier_when_supported() {
    let pipeline = run_secure_redaction_worker_pipeline(
        "explicit-print-boxes.pdf",
        single_page_acceptance_regions(),
        500,
    );
    let geometry = &pipeline.plan["page_plans"][0]["geometry"];
    assert!(!geometry["trim_box"].is_null());
    assert!(!geometry["bleed_box"].is_null());
    assert!(!geometry["art_box"].is_null());
    let verification = pipeline.verify_untouched(502);
    assert_all_required_verification_checks_passed(&verification);
    assert_eq!(
        verification_check(&verification, "PAGE_GEOMETRY")["status"],
        "passed"
    );
}

#[test]
fn secure_redaction_verifier_rejects_a_stale_digest_after_artifact_tamper() {
    let pipeline = run_secure_redaction_worker_pipeline(
        "simple-text.pdf",
        single_page_acceptance_regions(),
        600,
    );
    assert_all_required_verification_checks_passed(&pipeline.verify_untouched(602));

    // Appending a PDF comment changes only the serialized artifact bytes. The
    // transform report deliberately retains its original digest and size.
    use std::io::Write;
    let mut artifact = fs::OpenOptions::new()
        .append(true)
        .open(pipeline.output_path())
        .unwrap();
    artifact
        .write_all(b"\n% integration-test digest tamper\n")
        .unwrap();
    drop(artifact);

    let verification = pipeline.verify_untouched(603);
    assert_eq!(verification["is_verified"], false, "{verification}");
    assert_eq!(
        verification_check(&verification, "ARTIFACT_DIGEST_AND_SIZE")["status"],
        "failed",
        "{verification}"
    );
}

#[test]
fn secure_redaction_verifier_rejects_one_pixel_coverage_corruption_after_digest_rebinding() {
    let pipeline = run_secure_redaction_worker_pipeline(
        "simple-text.pdf",
        single_page_acceptance_regions(),
        700,
    );
    assert_all_required_verification_checks_passed(&pipeline.verify_untouched(702));

    let (x, y) = corrupt_one_covered_raster_pixel(&pipeline);
    let mut rebound_report = pipeline.transform_report.clone();
    rebound_report["output_digest"] = json!(hex_sha256(&pipeline.output_path()));
    rebound_report["output_size_bytes"] =
        json!(fs::metadata(pipeline.output_path()).unwrap().len());

    let verification = pipeline.verify(703, rebound_report);
    assert_eq!(verification["is_verified"], false, "{verification}");
    assert_eq!(
        verification_check(&verification, "ARTIFACT_DIGEST_AND_SIZE")["status"],
        "passed",
        "{verification}"
    );
    assert_eq!(
        verification_check(&verification, "AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE")["status"],
        "failed",
        "{verification}"
    );
    let coverage = verification_check(&verification, "AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE");
    assert!(
        coverage["message"].as_str().is_some_and(|message| {
            message.contains("observation=embedded_raster")
                && message.contains("page_index 0")
                && message.contains("canonical_region_id")
                && message.contains("expected [0, 0, 0, 255]")
                && message.contains(&format!("({x}, {y})"))
        }),
        "coverage diagnostic must identify the mutated canonical pixel: {coverage}"
    );
}

#[test]
fn secure_redaction_verifier_rejects_reopened_placement_corruption_with_embedded_pixels_unchanged()
{
    let pipeline = run_secure_redaction_worker_pipeline(
        "simple-text.pdf",
        single_page_acceptance_regions(),
        800,
    );
    assert_all_required_verification_checks_passed(&pipeline.verify_untouched(802));

    mirror_affected_image_placement(&pipeline);
    let mut rebound_report = pipeline.transform_report.clone();
    rebound_report["output_digest"] = json!(hex_sha256(&pipeline.output_path()));
    rebound_report["output_size_bytes"] =
        json!(fs::metadata(pipeline.output_path()).unwrap().len());

    let verification = pipeline.verify(803, rebound_report);
    assert_eq!(verification["is_verified"], false, "{verification}");
    assert_eq!(
        verification_check(&verification, "ARTIFACT_DIGEST_AND_SIZE")["status"],
        "passed",
        "{verification}"
    );
    assert_eq!(
        verification_check(&verification, "AFFECTED_PAGE_IMAGE_ONLY")["status"],
        "passed",
        "placement mirror must preserve the structural page bounds: {verification}"
    );
    let coverage = verification_check(&verification, "AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE");
    assert_eq!(coverage["status"], "failed", "{verification}");
    assert!(
        coverage["message"].as_str().is_some_and(|message| {
            message.contains("observation=reopened_render")
                && message.contains("page_index 0")
                && message.contains("canonical_region_id")
                && message.contains("mapped_output_device_rect")
                && message.contains("expected [0, 0, 0, 255]")
        }),
        "coverage diagnostic must prove the reopened observation rejected unchanged embedded pixels: {coverage}"
    );
}

#[test]
fn secure_redaction_verifier_rejects_large_object_bounds_placement_translation() {
    let pipeline = run_secure_redaction_worker_pipeline(
        "simple-text.pdf",
        single_page_acceptance_regions(),
        900,
    );
    assert_all_required_verification_checks_passed(&pipeline.verify_untouched(902));

    // 0.01 point is orders of magnitude above one f32 ULP at this fixture's
    // page scale, while preserving the embedded image bytes.
    translate_affected_image_placement(&pipeline, 0.01, 0.0);
    let mut rebound_report = pipeline.transform_report.clone();
    rebound_report["output_digest"] = json!(hex_sha256(&pipeline.output_path()));
    rebound_report["output_size_bytes"] =
        json!(fs::metadata(pipeline.output_path()).unwrap().len());

    let verification = pipeline.verify(903, rebound_report);
    assert_eq!(verification["is_verified"], false, "{verification}");
    assert_eq!(
        verification_check(&verification, "ARTIFACT_DIGEST_AND_SIZE")["status"],
        "passed",
        "{verification}"
    );
    let placement = verification_check(&verification, "AFFECTED_PAGE_IMAGE_ONLY");
    assert_eq!(placement["status"], "failed", "{verification}");
    assert!(
        placement["message"].as_str().is_some_and(|message| {
            message.contains("actual left=")
                && message.contains("absolute delta left=")
                && message.contains("inward")
                && message.contains("f32 observation tolerance=")
        }),
        "placement diagnostic must characterize the rejected object-bounds translation: {placement}"
    );
}
