//! Digest-bound atomic publication for independently verified sanitization artifacts.
//!
//! PDF semantics remain owned by the transform and verifier workers. This
//! module's main-process boundary handles only session/path authority,
//! filesystem identity, byte digests, cancellation boundaries and typed disk
//! truth through the shared `document_io` primitive.

use crate::document_session::{
    DocumentSessionError, SessionJobFailure, SessionJobOperationError, SessionJobOutcome,
    VerifiedArtifactExportPaths,
};
use crate::sanitization::{
    inspection_digest, validate_execution_report, validate_inspection_report,
    validate_sanitization_policy, validate_verification_report_for_transform,
    SanitizationAssurance, SanitizationExecutionReport, SanitizationPolicy,
    SanitizationPublicationCommitState, SanitizationPublicationReport, SanitizationTransformReport,
    SanitizationVerificationReport, SanitizationVerificationStatus,
    SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION, SANITIZATION_INSPECTION_PROFILE,
    SANITIZATION_POLICY_SCHEMA_VERSION, SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION,
    SANITIZATION_REPORT_SCHEMA_VERSION, SANITIZATION_TRANSFORM_OUTPUT_NAME,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug)]
struct SanitizationPublicationTarget {
    canonical_target: PathBuf,
    expectation: crate::document_io::VerifiedArtifactTargetExpectation,
}

fn operation_error(code: &'static str, message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

fn map_operation_error(error: SessionJobOperationError) -> DocumentSessionError {
    match error {
        SessionJobOperationError::Failed(failure) => {
            DocumentSessionError::new(failure.code, failure.message)
        }
        SessionJobOperationError::Cancelled(_) => DocumentSessionError::new(
            "SESSION_JOB_CANCELLED",
            "Sanitization execution başlamadan iptal edildi",
        ),
    }
}

fn target_preflight(
    target_path: &str,
    paths: &VerifiedArtifactExportPaths,
) -> Result<SanitizationPublicationTarget, DocumentSessionError> {
    if target_path.trim().is_empty() || target_path.len() > 4096 || target_path.contains('\0') {
        return Err(DocumentSessionError::new(
            "SANITIZATION_TARGET_INVALID",
            "Sanitization publication target geçerli bir yol olmalıdır",
        ));
    }
    let (canonical_target, expectation) =
        crate::document_io::capture_verified_artifact_target_expectation(Path::new(target_path))
            .map_err(|detail| {
                DocumentSessionError::new("SANITIZATION_TARGET_PREFLIGHT_FAILED", detail)
            })?;

    for protected in [&paths.original_path, &paths.working_path] {
        let same = crate::document_io::same_persistence_target(&canonical_target, protected)
            .map_err(|detail| {
                DocumentSessionError::new(
                    "SANITIZATION_TARGET_PREFLIGHT_FAILED",
                    format!("Protected session path identity doğrulanamadı: {detail}"),
                )
            })?;
        if same {
            return Err(DocumentSessionError::new(
                "SANITIZATION_TARGET_PROTECTED_PATH",
                "Sanitization export original veya working PDF üzerine yazılamaz",
            ));
        }
    }

    for private_root in [&paths.session_dir, &paths.jobs_root] {
        let canonical_root = private_root.canonicalize().map_err(|error| {
            DocumentSessionError::new(
                "SANITIZATION_TARGET_PREFLIGHT_FAILED",
                format!("Private session root çözümlenemedi: {error}"),
            )
        })?;
        if canonical_target.starts_with(&canonical_root) {
            return Err(DocumentSessionError::new(
                "SANITIZATION_TARGET_PRIVATE_PATH",
                "Sanitization export private session veya job dizinine yazılamaz",
            ));
        }
    }

    let parent = canonical_target.parent().ok_or_else(|| {
        DocumentSessionError::new(
            "SANITIZATION_TARGET_PREFLIGHT_FAILED",
            "Sanitization target parent dizini belirlenemedi",
        )
    })?;
    let parent_metadata = fs::metadata(parent).map_err(|error| {
        DocumentSessionError::new(
            "SANITIZATION_TARGET_PREFLIGHT_FAILED",
            format!("Sanitization target parent metadata okunamadı: {error}"),
        )
    })?;
    if !parent_metadata.is_dir() || parent_metadata.permissions().readonly() {
        return Err(DocumentSessionError::new(
            "SANITIZATION_TARGET_PREFLIGHT_FAILED",
            "Sanitization target parent kullanılabilir bir yazma dizini değil",
        ));
    }

    Ok(SanitizationPublicationTarget {
        canonical_target,
        expectation,
    })
}

fn mandatory_verification_passed(report: &SanitizationVerificationReport) -> bool {
    let checks = [
        &report.full_rewrite,
        &report.incremental_history_removed,
        &report.object_graph_scan,
        &report.removed_payload_scan,
        &report.parser_reopen,
        &report.pdfium_reopen,
        &report.page_tree_valid,
        &report.page_count_valid,
        &report.page_dimensions_valid,
        &report.preservation_checks,
        &report.render_smoke,
        &report.artifact_unchanged,
    ];
    report.is_verified
        && report.blockers.is_empty()
        && report
            .unresolved_findings
            .iter()
            .all(|finding| !finding.mandatory)
        && checks
            .iter()
            .all(|check| check.status == SanitizationVerificationStatus::Passed)
        && report
            .controls
            .iter()
            .all(|control| control.status == SanitizationVerificationStatus::Passed)
}

/// Publication-only production core. It does not parse or reopen the PDF.
#[allow(clippy::too_many_arguments)]
fn execute_verified_sanitization_publication(
    context: &crate::document_session::SessionJobContext,
    transform: &SanitizationTransformReport,
    verification: &SanitizationVerificationReport,
    policy: &SanitizationPolicy,
    artifact: &Path,
    target: &SanitizationPublicationTarget,
    corrupt_staged_temp: bool,
    #[cfg(feature = "sanitization-test-support")]
    publication_fault: crate::document_io::VerifiedArtifactPublicationTestFault,
) -> Result<SanitizationPublicationReport, SessionJobOperationError> {
    validate_verification_report_for_transform(verification, transform, policy)?;
    if !mandatory_verification_passed(verification) {
        return Err(operation_error(
            "SANITIZATION_VERIFICATION_GATE_FAILED",
            "Independent verification publication için bütün mandatory koşulları geçmelidir",
        ));
    }

    let job_dir = context.snapshot_path.parent().ok_or_else(|| {
        operation_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Private job directory bulunamadı",
        )
    })?;
    let canonical_job_dir = job_dir.canonicalize().map_err(|_| {
        operation_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Private job directory canonicalize edilemedi",
        )
    })?;
    let artifact_metadata = fs::symlink_metadata(artifact).map_err(|_| {
        operation_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Verified artifact metadata okunamadı",
        )
    })?;
    if !artifact_metadata.file_type().is_file()
        || artifact_metadata.file_type().is_symlink()
        || artifact != canonical_job_dir.join(SANITIZATION_TRANSFORM_OUTPUT_NAME)
    {
        return Err(operation_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Verified artifact canonical private regular-file invariantını karşılamıyor",
        ));
    }

    let actual_artifact_digest = crate::document_io::sha256_file(artifact).map_err(|_| {
        operation_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_INVALID",
            "Verified artifact SHA-256 okunamadı",
        )
    })?;
    if transform.output_digest != verification.transform_digest
        || transform.output_digest != actual_artifact_digest
    {
        return Err(operation_error(
            "SANITIZATION_OUTPUT_DIGEST_MISMATCH",
            "Transform/verifier/actual artifact digest chain eşleşmedi",
        ));
    }

    #[cfg(feature = "sanitization-test-support")]
    let commit = crate::document_io::atomic_publish_verified_artifact_with_fault_for_acceptance(
        "sanitization_publish",
        &context.snapshot_path,
        &target.canonical_target,
        artifact,
        &verification.transform_digest,
        target.expectation.clone(),
        "SANITIZATION_TARGET_CONFLICT",
        "SANITIZATION_OUTPUT_DIGEST_MISMATCH",
        corrupt_staged_temp,
        publication_fault,
    );
    #[cfg(not(feature = "sanitization-test-support"))]
    let commit = {
        let _ = corrupt_staged_temp;
        crate::document_io::atomic_publish_verified_artifact(
            "sanitization_publish",
            &context.snapshot_path,
            &target.canonical_target,
            artifact,
            &verification.transform_digest,
            target.expectation.clone(),
            crate::document_io::PublicationErrorCodes {
                conflict_code: "SANITIZATION_TARGET_CONFLICT",
                digest_mismatch_code: "SANITIZATION_OUTPUT_DIGEST_MISMATCH",
            },
        )
    };
    let commit = commit.map_err(|error| {
        let code = if error.detail.contains("SANITIZATION_TARGET_CONFLICT")
            || error.detail.contains("PRECONDITION_TARGET_CHANGED")
        {
            "SANITIZATION_TARGET_CONFLICT"
        } else if error.detail.contains("SANITIZATION_OUTPUT_DIGEST_MISMATCH") {
            "SANITIZATION_OUTPUT_DIGEST_MISMATCH"
        } else {
            "SANITIZATION_PUBLICATION_FAILED"
        };
        SessionJobOperationError::Failed(SessionJobFailure::publication_failure(code, error))
    })?;

    // `document_io` computes this digest from the committed target while the
    // shared namespace/identity locks are still held. Re-reading after it
    // returns would create a new, weaker TOCTOU window.
    let published_digest = commit.committed_digest.clone();
    if published_digest != verification.transform_digest {
        let error = crate::document_io::DocumentIoError::new(
            "sanitization_publish",
            crate::document_io::DocumentIoStage::PostCommitVerify,
            crate::document_io::CommitState::Indeterminate,
            "Commit sonrası target exact verifier digest'ini taşımıyor",
        )
        .with_replacement_digest(published_digest);
        return Err(SessionJobOperationError::Failed(
            SessionJobFailure::publication_failure(
                "SANITIZATION_POSTCOMMIT_DIGEST_MISMATCH",
                error,
            ),
        ));
    }

    let report = SanitizationPublicationReport {
        schema_version: SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION.to_string(),
        published: true,
        target_path: target.canonical_target.to_string_lossy().into_owned(),
        published_digest,
        commit_state: SanitizationPublicationCommitState::from_document_io(commit.state)?,
        warnings: Vec::new(),
    };
    crate::sanitization::validate_publication_report(&report)?;
    Ok(report)
}

#[cfg(feature = "sanitization-test-support")]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SanitizationExecutionTestPhase {
    AfterSnapshotCreated,
    AfterInspectionValidated,
    AfterTransformValidated,
    AfterVerifierValidated,
    BeforePublication,
    PublicationStarted,
    AfterPublication,
}

#[cfg(not(feature = "sanitization-test-support"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SanitizationExecutionTestPhase {
    AfterSnapshotCreated,
    AfterInspectionValidated,
    AfterTransformValidated,
    AfterVerifierValidated,
    BeforePublication,
    PublicationStarted,
    AfterPublication,
}

#[cfg(feature = "sanitization-test-support")]
pub type SanitizationExecutionPhaseHook =
    Arc<dyn Fn(SanitizationExecutionTestPhase, Option<&Path>) + Send + Sync + 'static>;

#[cfg(feature = "sanitization-test-support")]
pub type SanitizationExecutionResultHook = Arc<
    dyn Fn(crate::job_worker::JobResult) -> crate::job_worker::JobResult + Send + Sync + 'static,
>;

#[cfg(feature = "sanitization-test-support")]
#[doc(hidden)]
#[derive(Clone)]
pub struct SanitizationExecutionTestSupport {
    pub phase_hook: Option<SanitizationExecutionPhaseHook>,
    pub transform_behavior: crate::sanitization_transform::SanitizationTransformTestBehavior,
    pub verification_behavior: crate::sanitization_verifier::SanitizationVerificationTestBehavior,
    pub transform_result_hook: Option<SanitizationExecutionResultHook>,
    pub verifier_result_hook: Option<SanitizationExecutionResultHook>,
    pub corrupt_staged_temp: bool,
    pub publication_fault: crate::document_io::VerifiedArtifactPublicationTestFault,
    pub transform_timeout: Duration,
    pub verification_timeout: Duration,
}

#[cfg(feature = "sanitization-test-support")]
impl Default for SanitizationExecutionTestSupport {
    fn default() -> Self {
        Self {
            phase_hook: None,
            transform_behavior: Default::default(),
            verification_behavior: Default::default(),
            transform_result_hook: None,
            verifier_result_hook: None,
            corrupt_staged_temp: false,
            publication_fault: Default::default(),
            transform_timeout: Duration::from_secs(300),
            verification_timeout: Duration::from_secs(300),
        }
    }
}

#[cfg(not(feature = "sanitization-test-support"))]
#[derive(Clone, Default)]
struct SanitizationExecutionTestSupport;

fn notify_phase(
    support: Option<&SanitizationExecutionTestSupport>,
    phase: SanitizationExecutionTestPhase,
    artifact: Option<&Path>,
) {
    #[cfg(feature = "sanitization-test-support")]
    if let Some(hook) = support.and_then(|support| support.phase_hook.as_ref()) {
        hook(phase, artifact);
    }
    #[cfg(not(feature = "sanitization-test-support"))]
    let _ = (support, phase, artifact);
}

/// Internal/testable full backend chain. This is intentionally not a Tauri
/// command and does not activate the frontend feature.
#[allow(clippy::too_many_arguments)]
async fn execute_document_sanitization_core_internal(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy: SanitizationPolicy,
    target_path: String,
    store: &crate::document_session::DocumentSessionStore,
    test_support: Option<Arc<SanitizationExecutionTestSupport>>,
) -> Result<SessionJobOutcome<SanitizationExecutionReport>, DocumentSessionError> {
    validate_sanitization_policy(&policy).map_err(map_operation_error)?;
    let export_guard = store
        .acquire_verified_artifact_export_guard(&session_id, expected_content_generation)
        .await?;
    let publication_target = target_preflight(&target_path, &export_guard.paths)?;

    let report_policy = policy.clone();
    let outcome = store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::AfterSnapshotCreated,
                    None,
                );
                context.cancellation.check_cancelled()?;
                let input_digest = crate::document_io::sha256_file(&context.snapshot_path)
                    .map_err(|_| {
                        operation_error(
                            "SANITIZATION_SNAPSHOT_DIGEST_FAILED",
                            "Immutable snapshot SHA-256 okunamadı",
                        )
                    })?;

                let inspection_result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::InspectDocumentSanitization {
                        schema_version: SANITIZATION_REPORT_SCHEMA_VERSION.to_string(),
                        policy_schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
                        inspection_profile: SANITIZATION_INSPECTION_PROFILE.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior: crate::sanitization_inspection::SanitizationInspectionTestBehavior::Real,
                    },
                    Duration::from_secs(180),
                )?;
                let inspection = match inspection_result {
                    crate::job_worker::JobResult::SanitizationInspection(report) => report,
                    _ => {
                        return Err(operation_error(
                            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                            "Sanitization inspection worker beklenmeyen sonuç döndürdü",
                        ))
                    }
                };
                validate_inspection_report(&inspection)?;
                if inspection.input_digest != input_digest {
                    return Err(operation_error(
                        "SANITIZATION_INSPECTION_REPORT_MISMATCH",
                        "Inspection report immutable snapshot digest'iyle eşleşmiyor",
                    ));
                }
                let expected_inspection_digest = inspection_digest(&inspection)?;
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::AfterInspectionValidated,
                    None,
                );
                context.cancellation.check_cancelled()?;

                #[cfg(feature = "sanitization-test-support")]
                let transform_timeout = test_support
                    .as_ref()
                    .map_or(Duration::from_secs(300), |support| support.transform_timeout);
                #[cfg(not(feature = "sanitization-test-support"))]
                let transform_timeout = Duration::from_secs(300);
                #[cfg_attr(not(feature = "sanitization-test-support"), allow(unused_mut))]
                let mut transform_result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::SanitizePdf {
                        content_generation: context.captured_content_generation,
                        policy: policy.clone(),
                        expected_input_digest: input_digest.clone(),
                        expected_inspection_digest: expected_inspection_digest.clone(),
                        output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior: test_support
                            .as_ref()
                            .map_or(Default::default(), |support| support.transform_behavior),
                    },
                    transform_timeout,
                )?;
                #[cfg(feature = "sanitization-test-support")]
                if let Some(hook) = test_support
                    .as_ref()
                    .and_then(|support| support.transform_result_hook.as_ref())
                {
                    transform_result = hook(transform_result);
                }
                let (transform, artifact) =
                    crate::sanitization_transform::validate_transform_worker_result(
                        &context,
                        transform_result,
                        &policy,
                        &input_digest,
                        &expected_inspection_digest,
                    )?;
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::AfterTransformValidated,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;

                #[cfg(feature = "sanitization-test-support")]
                let verification_timeout = test_support.as_ref().map_or(
                    Duration::from_secs(300),
                    |support| support.verification_timeout,
                );
                #[cfg(not(feature = "sanitization-test-support"))]
                let verification_timeout = Duration::from_secs(300);
                #[cfg_attr(not(feature = "sanitization-test-support"), allow(unused_mut))]
                let mut verification_result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::VerifySanitizedPdf {
                        content_generation: context.captured_content_generation,
                        policy: policy.clone(),
                        expected_input_digest: input_digest,
                        expected_transform_digest: transform.output_digest.clone(),
                        transform_report: transform.clone(),
                        output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior: test_support.as_ref().map_or(
                            Default::default(),
                            |support| support.verification_behavior,
                        ),
                    },
                    verification_timeout,
                )?;
                #[cfg(feature = "sanitization-test-support")]
                if let Some(hook) = test_support
                    .as_ref()
                    .and_then(|support| support.verifier_result_hook.as_ref())
                {
                    verification_result = hook(verification_result);
                }
                let verification = match verification_result {
                    crate::job_worker::JobResult::SanitizationVerification(report) => report,
                    _ => {
                        return Err(operation_error(
                            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                            "Sanitization verifier beklenmeyen sonuç döndürdü",
                        ))
                    }
                };
                validate_verification_report_for_transform(&verification, &transform, &policy)?;
                if !mandatory_verification_passed(&verification) {
                    return Err(operation_error(
                        "SANITIZATION_VERIFICATION_GATE_FAILED",
                        "Independent sanitization verification mandatory gate'i geçmedi",
                    ));
                }
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::AfterVerifierValidated,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::BeforePublication,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;

                // Disk truth wins after this checkpoint. No cooperative cancel
                // is observed once atomic publication starts.
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::PublicationStarted,
                    Some(&artifact),
                );
                #[cfg(feature = "sanitization-test-support")]
                let corrupt_staged_temp = test_support
                    .as_ref()
                    .is_some_and(|support| support.corrupt_staged_temp);
                #[cfg(not(feature = "sanitization-test-support"))]
                let corrupt_staged_temp = false;
                #[cfg(feature = "sanitization-test-support")]
                let publication_fault = test_support
                    .as_ref()
                    .map_or(Default::default(), |support| support.publication_fault);
                let publication = execute_verified_sanitization_publication(
                    &context,
                    &transform,
                    &verification,
                    &policy,
                    &artifact,
                    &publication_target,
                    corrupt_staged_temp,
                    #[cfg(feature = "sanitization-test-support")]
                    publication_fault,
                )?;
                notify_phase(
                    test_support.as_deref(),
                    SanitizationExecutionTestPhase::AfterPublication,
                    Some(&artifact),
                );

                let assurance = match policy.profile {
                    crate::sanitization::SanitizationProfile::VerifiedLimitedV1 => {
                        SanitizationAssurance::VerifiedLimited
                    }
                    crate::sanitization::SanitizationProfile::CustomSupportedV1 => {
                        SanitizationAssurance::VerifiedCustom
                    }
                };
                let report = SanitizationExecutionReport {
                    schema_version: SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION.to_string(),
                    policy_schema_version: policy.schema_version.clone(),
                    profile: policy.profile,
                    assurance,
                    warnings: verification.warnings.clone(),
                    controls: policy.controls.clone(),
                    transform,
                    verification,
                    publication: Some(publication),
                };
                validate_execution_report(&report, &policy)?;
                Ok(report)
            },
        )
        .await;
    drop(export_guard);
    let outcome = outcome?;
    if let SessionJobOutcome::Completed { value, .. } = &outcome {
        validate_execution_report(value, &report_policy).map_err(map_operation_error)?;
    }
    Ok(outcome)
}

pub async fn execute_document_sanitization_core(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy: SanitizationPolicy,
    target_path: String,
    store: &crate::document_session::DocumentSessionStore,
) -> Result<SessionJobOutcome<SanitizationExecutionReport>, DocumentSessionError> {
    execute_document_sanitization_core_internal(
        job_id,
        session_id,
        expected_content_generation,
        policy,
        target_path,
        store,
        None,
    )
    .await
}

/// Canonical versioned frontend entry point. Feature activation remains owned
/// by the frontend registry; this command never accepts the legacy broad
/// boolean option model or a source/input path.
#[tauri::command]
pub async fn execute_document_sanitization(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy: SanitizationPolicy,
    target_path: String,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<SessionJobOutcome<SanitizationExecutionReport>, DocumentSessionError> {
    execute_document_sanitization_core(
        job_id,
        session_id,
        expected_content_generation,
        policy,
        target_path,
        &store,
    )
    .await
}

#[cfg(feature = "sanitization-test-support")]
#[doc(hidden)]
pub async fn execute_document_sanitization_core_with_test_support(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy: SanitizationPolicy,
    target_path: String,
    store: &crate::document_session::DocumentSessionStore,
    test_support: Option<Arc<SanitizationExecutionTestSupport>>,
) -> Result<SessionJobOutcome<SanitizationExecutionReport>, DocumentSessionError> {
    execute_document_sanitization_core_internal(
        job_id,
        session_id,
        expected_content_generation,
        policy,
        target_path,
        store,
        test_support,
    )
    .await
}
