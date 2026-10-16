//! Integration tests for external validation contract (11I-E5).
//!
//! Verifies:
//! - Fail-closed semantics for required and optional validators
//! - Tool absence classification (`ToolUnavailableOffline`)
//! - Strict distinction between internal verification and external validation
//! - Mock harness isolation (`is_mock: true`)
//! - Bounded path safety and command injection protection
//! - Exact-artifact output validation evidence generation and serialization

use preview_alternative_lib::external_validation::{
    validate_artifact_output, validate_input_pdf_path, ExecutionLayer, ExternalValidationConfig,
    ExternalValidationEvidence, MockVerapdfHarness, OverallValidationStatus, ProfileId,
    Requiredness, ToolAvailability, ToolIdentity, ToolResolutionMethod, ValidationFeature,
    ValidationOutcome, ValidatorId, ValidatorRequirement, VerapdfConfig, VerapdfHarness,
    MIN_VERAPDF_VERSION,
};
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf")
}

#[test]
fn contract_documented_requirements_are_optional_and_artifact_smoke() {
    let requirements = ExternalValidationConfig::documented_requirements();
    assert_eq!(requirements.len(), 2);

    let ua1 = requirements
        .iter()
        .find(|r| r.id == ValidatorId("verapdf-pdf-ua-1".to_string()))
        .expect("verapdf-pdf-ua-1 requirement must be present");
    assert_eq!(ua1.feature, ValidationFeature::PdfUaMachineValidation);
    assert_eq!(ua1.requiredness, Requiredness::OptionalDiagnostic);
    assert_eq!(ua1.execution_layer, ExecutionLayer::ArtifactSmoke);
    assert_eq!(ua1.min_version.as_deref(), Some(MIN_VERAPDF_VERSION));

    let ua2 = requirements
        .iter()
        .find(|r| r.id == ValidatorId("verapdf-pdf-ua-2".to_string()))
        .expect("verapdf-pdf-ua-2 requirement must be present");
    assert_eq!(ua2.feature, ValidationFeature::PdfUaMachineValidation);
    assert_eq!(ua2.requiredness, Requiredness::OptionalDiagnostic);
    assert_eq!(ua2.execution_layer, ExecutionLayer::ArtifactSmoke);
}

#[test]
fn contract_required_validator_failure_is_strictly_fail_closed() {
    let required_req = ValidatorRequirement {
        id: ValidatorId("mandatory-pdfa".to_string()),
        feature: ValidationFeature::Other("pdfa_export".to_string()),
        validator: "verapdf".to_string(),
        profile: ProfileId("PDF/A-2b".to_string()),
        requiredness: Requiredness::RequiredForClaim,
        execution_layer: ExecutionLayer::ArtifactSmoke,
        min_version: Some(MIN_VERAPDF_VERSION.to_string()),
        max_version: None,
        timeout_secs: 30,
        max_output_bytes: 1024 * 1024,
        extra_args: Vec::new(),
    };
    let reqs = vec![required_req];

    // Case 1: Missing tool offline -> NonCompliant
    let offline_outcome = ValidationOutcome {
        validator_id: ValidatorId("mandatory-pdfa".to_string()),
        tool_identity: None,
        status: ToolAvailability::ToolUnavailableOffline,
        profile: ProfileId("PDF/A-2b".to_string()),
        machine_result: None,
        stdout_truncated: None,
        stderr_truncated: Some("Tool not found offline".to_string()),
        duration_ms: 0,
        error: Some("Tool not found offline".to_string()),
        is_mock: false,
    };
    let status_offline = OverallValidationStatus::compute(&reqs, &[offline_outcome]);
    assert_eq!(status_offline, OverallValidationStatus::NonCompliant);

    // Case 2: Timeout -> NonCompliant
    let timeout_outcome = ValidationOutcome {
        validator_id: ValidatorId("mandatory-pdfa".to_string()),
        tool_identity: None,
        status: ToolAvailability::Timeout,
        profile: ProfileId("PDF/A-2b".to_string()),
        machine_result: None,
        stdout_truncated: None,
        stderr_truncated: None,
        duration_ms: 30_000,
        error: Some("Execution timed out".to_string()),
        is_mock: false,
    };
    assert_eq!(
        OverallValidationStatus::compute(&reqs, &[timeout_outcome]),
        OverallValidationStatus::NonCompliant
    );

    // Case 3: Version unsupported -> NonCompliant
    let unsupported_outcome = ValidationOutcome {
        validator_id: ValidatorId("mandatory-pdfa".to_string()),
        tool_identity: Some(ToolIdentity {
            name: "verapdf".to_string(),
            version: "1.0.0".to_string(),
            profile_version: None,
            invocation_class: "CLI".to_string(),
            resolution_method: ToolResolutionMethod::PathLookup,
        }),
        status: ToolAvailability::VersionUnsupported,
        profile: ProfileId("PDF/A-2b".to_string()),
        machine_result: None,
        stdout_truncated: None,
        stderr_truncated: None,
        duration_ms: 10,
        error: Some("Version 1.0.0 < 1.30.2".to_string()),
        is_mock: false,
    };
    assert_eq!(
        OverallValidationStatus::compute(&reqs, &[unsupported_outcome]),
        OverallValidationStatus::NonCompliant
    );

    // Case 4: Compliant when all required pass
    let pass_outcome = ValidationOutcome {
        validator_id: ValidatorId("mandatory-pdfa".to_string()),
        tool_identity: Some(ToolIdentity {
            name: "verapdf".to_string(),
            version: "1.30.2".to_string(),
            profile_version: None,
            invocation_class: "CLI --format xml".to_string(),
            resolution_method: ToolResolutionMethod::PathLookup,
        }),
        status: ToolAvailability::Pass,
        profile: ProfileId("PDF/A-2b".to_string()),
        machine_result: None,
        stdout_truncated: None,
        stderr_truncated: None,
        duration_ms: 1500,
        error: None,
        is_mock: false,
    };
    assert_eq!(
        OverallValidationStatus::compute(&reqs, &[pass_outcome]),
        OverallValidationStatus::Compliant
    );
}

#[test]
fn contract_optional_diagnostic_absence_does_not_block_release() {
    let reqs = ExternalValidationConfig::documented_requirements();

    let offline_outcome_1 = ValidationOutcome {
        validator_id: ValidatorId("verapdf-pdf-ua-1".to_string()),
        tool_identity: None,
        status: ToolAvailability::ToolUnavailableOffline,
        profile: ProfileId("PDF/UA-1".to_string()),
        machine_result: None,
        stdout_truncated: None,
        stderr_truncated: Some("Offline".to_string()),
        duration_ms: 0,
        error: Some("Offline".to_string()),
        is_mock: false,
    };
    let offline_outcome_2 = ValidationOutcome {
        validator_id: ValidatorId("verapdf-pdf-ua-2".to_string()),
        tool_identity: None,
        status: ToolAvailability::ToolUnavailableOffline,
        profile: ProfileId("PDF/UA-2".to_string()),
        machine_result: None,
        stdout_truncated: None,
        stderr_truncated: Some("Offline".to_string()),
        duration_ms: 0,
        error: Some("Offline".to_string()),
        is_mock: false,
    };

    // When all requirements are optional, tool unavailability yields DiagnosticOnly (NOT NonCompliant)
    let status = OverallValidationStatus::compute(&reqs, &[offline_outcome_1, offline_outcome_2]);
    assert_eq!(status, OverallValidationStatus::DiagnosticOnly);
}

#[test]
fn contract_mock_results_always_carry_is_mock_true() {
    let mock = MockVerapdfHarness::new();
    let outcome_pass = mock.validate(Path::new("dummy.pdf"), "PDF/A-2b").unwrap();
    assert!(outcome_pass.is_mock);
    assert_eq!(outcome_pass.status, ToolAvailability::Pass);

    let mut mock_offline = MockVerapdfHarness::new();
    mock_offline.should_be_offline = true;
    let outcome_offline = mock_offline
        .validate(Path::new("dummy.pdf"), "PDF/A-2b")
        .unwrap();
    assert!(outcome_offline.is_mock);
    assert_eq!(
        outcome_offline.status,
        ToolAvailability::ToolUnavailableOffline
    );
}

#[test]
fn contract_path_safety_rejects_dangerous_paths() {
    // 1. Non-existent path
    let bad_path = Path::new("/var/tmp/non_existent_pdf_file_98765.pdf");
    assert!(validate_input_pdf_path(bad_path).is_err());

    // 2. Directory instead of file
    let dir_path = fixtures_dir();
    assert!(validate_input_pdf_path(&dir_path).is_err());

    // 3. Valid fixture path succeeds and canonicalizes
    let valid_path = fixtures_dir().join("release-smoke.pdf");
    let canonical = validate_input_pdf_path(&valid_path).expect("Canonical path must succeed");
    assert!(canonical.is_file());
    assert!(canonical.is_absolute());
}

#[test]
fn contract_validate_artifact_output_with_real_fixture_offline() {
    let fixture_path = fixtures_dir().join("release-smoke.pdf");
    assert!(
        fixture_path.exists(),
        "release-smoke.pdf fixture must exist"
    );

    let config = ExternalValidationConfig {
        requirements: ExternalValidationConfig::documented_requirements(),
        ..Default::default()
    };

    // Use default harness pointing to non-existent executable. In this test environment,
    // it fail-closed reports ToolUnavailableOffline.
    let harness = VerapdfHarness::with_config(VerapdfConfig {
        executable: "verapdf_non_existent_binary".to_string(),
        jvm_args: vec![],
        use_xml_output: true,
        timeout_secs: 10,
        max_output_bytes: 1024 * 1024,
    });

    let evidence = validate_artifact_output(
        &fixture_path,
        "release_smoke",
        "core-release-smoke",
        &config,
        &harness,
    )
    .expect("validate_artifact_output should succeed and record offline outcomes");

    assert_eq!(evidence.operation_type, "release_smoke");
    assert_eq!(evidence.source_fixture_id, "core-release-smoke");
    assert_eq!(
        evidence.output_sha256,
        "727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611"
    );
    assert_eq!(evidence.output_byte_length, 4520);
    assert_eq!(evidence.outcomes.len(), 2);

    for outcome in &evidence.outcomes {
        assert_eq!(outcome.status, ToolAvailability::ToolUnavailableOffline);
        assert!(outcome.status.is_unavailable());
        assert!(!outcome.status.is_pass());
        assert!(!outcome.is_mock);
    }

    // Because all documented requirements are OptionalDiagnostic, the overall status is DiagnosticOnly
    assert_eq!(
        evidence.overall_status,
        OverallValidationStatus::DiagnosticOnly
    );

    // Serialization round-trip
    let json = serde_json::to_string_pretty(&evidence).expect("Must serialize evidence");
    let deserialized: ExternalValidationEvidence =
        serde_json::from_str(&json).expect("Must deserialize evidence");
    assert_eq!(deserialized.output_sha256, evidence.output_sha256);
    assert_eq!(deserialized.overall_status, evidence.overall_status);
}
