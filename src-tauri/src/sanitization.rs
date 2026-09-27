use crate::document_session::{SessionJobOperationError, SessionJobOutcome};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const SANITIZATION_REPORT_SCHEMA_VERSION: &str = "1.1.0";
pub const SANITIZATION_INSPECTION_VERSION: &str = "sanitization-inspection-v1";
pub const SANITIZATION_POLICY_SCHEMA_VERSION: &str = "sanitization-policy-v1";
pub const SANITIZATION_INSPECTION_PROFILE: &str = "minimum-profile-inspection-v1";
pub const SANITIZATION_CONTROL_REGISTRY_VERSION: &str = "sanitization-controls/1";
pub const SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION: &str = "sanitization-transform/1";
pub const SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION: &str = "sanitization-verification/1";
pub const SANITIZATION_VERIFICATION_VERSION: &str = "sanitization-verifier-v1";
pub const SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION: &str = "sanitization-publication/1";
pub const SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION: &str = "sanitization-execution/1";
pub const SANITIZATION_TRANSFORM_OUTPUT_NAME: &str = "sanitized-transform.pdf";
pub const MAX_SANITIZATION_POLICY_CONTROLS: usize = 32;
pub const MAX_SANITIZATION_TRANSFORM_EVIDENCE: usize = 10_000;
pub const MAX_SANITIZATION_TRANSFORM_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_SANITIZATION_VERIFICATION_FINGERPRINTS: usize = 10_000;
pub const MAX_SANITIZATION_VERIFICATION_PAGES: usize = 10_000;

pub const MAX_SANITIZATION_OBJECTS: usize = 100_000;
pub const MAX_SANITIZATION_TRAVERSAL_DEPTH: usize = 64;
pub const MAX_SANITIZATION_NAME_TREE_DEPTH: usize = 32;
pub const MAX_SANITIZATION_ACTION_CHAIN_DEPTH: usize = 64;
pub const MAX_SANITIZATION_ARRAY_ELEMENTS: usize = 100_000;
pub const MAX_SANITIZATION_DICTIONARY_ENTRIES: usize = 10_000;
pub const MAX_SANITIZATION_DECODED_STREAM_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SANITIZATION_TOTAL_DECODED_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_SANITIZATION_EMBEDDED_FILES: usize = 1_024;
pub const MAX_SANITIZATION_FINDINGS: usize = 10_000;
pub const MAX_SANITIZATION_EVIDENCE_PER_CONTROL: usize = 8;
pub const MAX_SANITIZATION_BLOCKING_FINDINGS: usize = 128;
pub const MAX_SANITIZATION_WARNINGS: usize = 256;
pub const MAX_SANITIZATION_REPORT_BYTES: usize = 1024 * 1024;
pub const MAX_SANITIZATION_RAW_SCAN_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_SANITIZATION_SAFE_STRING_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionSupport {
    SupportedForInspection,
    Experimental,
    Unsupported,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FutureRemovalSupport {
    Supported,
    SupportedWithVisualChange,
    SupportedWithFunctionalChange,
    Candidate,
    CandidateWithVisualChange,
    CandidateWithFunctionalChange,
    Experimental,
    Unsupported,
    Unverified,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ControlManifest {
    schema_version: String,
    controls: Vec<SanitizationControlDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationControlDescriptor {
    pub control_id: String,
    pub category: String,
    pub inspection_support: InspectionSupport,
    pub future_removal_support: FutureRemovalSupport,
    pub blocking_for_minimum_profile: bool,
    pub expected_visual_impact: String,
    pub expected_functional_impact: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationDocumentFacts {
    pub encrypted: bool,
    pub signed: bool,
    pub certification_signature_detected: bool,
    pub field_mdp_detected: bool,
    pub incremental_revisions_detected: bool,
    pub xfa_detected: bool,
    pub optional_content_detected: bool,
    pub rich_media_detected: bool,
    pub acro_form_detected: bool,
    pub calculation_order_detected: bool,
    pub document_identifier_present: bool,
    pub object_streams_detected: bool,
    pub xref_streams_detected: bool,
    pub page_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationFindingEvidence {
    pub category: String,
    pub object_id: Option<String>,
    pub logical_path: String,
    pub event_or_action_type: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationControlInspection {
    pub control_id: String,
    pub category: String,
    pub inspection_support: InspectionSupport,
    pub future_removal_support: FutureRemovalSupport,
    pub present: bool,
    pub finding_count: u32,
    pub expected_visual_impact: String,
    pub expected_functional_impact: String,
    pub blocking_reason: Option<String>,
    pub representative_evidence: Vec<SanitizationFindingEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationBlockingFinding {
    pub code: String,
    pub control_id: String,
    pub category: String,
    pub safe_message: String,
    pub evidence_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationInspectionWarning {
    pub code: String,
    pub safe_message: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationInspectionLimits {
    pub visited_objects: u32,
    pub decoded_bytes: u64,
    pub findings: u32,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationInspectionReport {
    pub schema_version: String,
    pub inspection_version: String,
    pub policy_schema_version: String,
    pub control_registry_version: String,
    pub input_digest: String,
    pub document_facts: SanitizationDocumentFacts,
    pub controls: Vec<SanitizationControlInspection>,
    pub blocking_findings: Vec<SanitizationBlockingFinding>,
    pub warnings: Vec<SanitizationInspectionWarning>,
    pub limits: SanitizationInspectionLimits,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationProfile {
    VerifiedLimitedV1,
    CustomSupportedV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationAcknowledgements {
    pub acknowledge_signature_invalidation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationPolicy {
    pub schema_version: String,
    pub profile: SanitizationProfile,
    pub controls: Vec<String>,
    pub acknowledgements: SanitizationAcknowledgements,
    pub expected_inspection_version: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationTransformStatus {
    RemovedCandidate,
    NotFound,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationTransformEvidence {
    pub logical_path: String,
    pub object_id: Option<String>,
    pub payload_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationControlTransformResult {
    pub control_id: String,
    pub requested: bool,
    pub detected_before: bool,
    pub removed_count: u32,
    pub transform_status: SanitizationTransformStatus,
    pub evidence: Vec<SanitizationTransformEvidence>,
    pub warnings: Vec<SanitizationInspectionWarning>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationFullRewriteEvidence {
    pub source_bytes_copied: bool,
    pub incremental_save_used: bool,
    pub active_prev_present: bool,
    pub active_xref_stm_present: bool,
    pub parser_reopen_passed: bool,
    pub unreachable_objects_pruned: u32,
    pub self_check_passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationPageInvariant {
    pub page_index: u32,
    pub width_points: f32,
    pub height_points: f32,
    pub rotation_degrees: i32,
    pub content_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationDocumentInvariants {
    pub page_count: u32,
    pub pages: Vec<SanitizationPageInvariant>,
    pub named_destination_count: u32,
    pub local_goto_action_count: u32,
    pub form_field_count: u32,
    pub outline_root_present: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationTransformReport {
    pub schema_version: String,
    pub policy_schema_version: String,
    pub control_registry_version: String,
    pub profile: SanitizationProfile,
    pub input_digest: String,
    pub output_digest: String,
    pub output_size_bytes: u64,
    pub output_relative_name: String,
    pub inspection_version: String,
    pub inspection_digest: String,
    pub content_generation: u64,
    pub controls: Vec<SanitizationControlTransformResult>,
    pub expected_document_invariants: SanitizationDocumentInvariants,
    pub full_rewrite_evidence: SanitizationFullRewriteEvidence,
    pub warnings: Vec<SanitizationInspectionWarning>,
    pub signatures_invalidated: bool,
    pub certification_lost: bool,
    pub status: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationVerificationStatus {
    Passed,
    Failed,
    SkippedWithReason,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationVerificationCheck {
    pub status: SanitizationVerificationStatus,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationControlVerification {
    pub control_id: String,
    pub requested: bool,
    pub detected_before: bool,
    pub forbidden_findings_after: u32,
    pub payload_fingerprints_checked: u32,
    pub payload_fingerprints_remaining: u32,
    pub status: SanitizationVerificationStatus,
    pub warnings: Vec<SanitizationInspectionWarning>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationVerificationFinding {
    pub code: String,
    pub control_id: Option<String>,
    pub object_id: Option<String>,
    pub logical_path: Option<String>,
    pub mandatory: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationVerificationReport {
    pub schema_version: String,
    pub policy_schema_version: String,
    pub control_registry_version: String,
    pub verification_version: String,
    pub inspection_version: String,
    pub input_digest: String,
    pub transform_digest: String,
    pub profile: SanitizationProfile,
    pub controls: Vec<SanitizationControlVerification>,
    pub full_rewrite: SanitizationVerificationCheck,
    pub incremental_history_removed: SanitizationVerificationCheck,
    pub object_graph_scan: SanitizationVerificationCheck,
    pub removed_payload_scan: SanitizationVerificationCheck,
    pub parser_reopen: SanitizationVerificationCheck,
    pub pdfium_reopen: SanitizationVerificationCheck,
    pub page_tree_valid: SanitizationVerificationCheck,
    pub page_count_valid: SanitizationVerificationCheck,
    pub page_dimensions_valid: SanitizationVerificationCheck,
    pub preservation_checks: SanitizationVerificationCheck,
    pub render_smoke: SanitizationVerificationCheck,
    pub artifact_unchanged: SanitizationVerificationCheck,
    pub unresolved_findings: Vec<SanitizationVerificationFinding>,
    pub blockers: Vec<SanitizationInspectionWarning>,
    pub warnings: Vec<SanitizationInspectionWarning>,
    pub is_verified: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationPublicationCommitState {
    NewCommitted,
    NewCommittedDurabilityUnconfirmed,
}

impl SanitizationPublicationCommitState {
    pub(crate) fn from_document_io(
        state: crate::document_io::CommitState,
    ) -> Result<Self, SessionJobOperationError> {
        match state {
            crate::document_io::CommitState::NewCommitted => Ok(Self::NewCommitted),
            crate::document_io::CommitState::NewCommittedDurabilityUnconfirmed => {
                Ok(Self::NewCommittedDurabilityUnconfirmed)
            }
            _ => Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_PUBLICATION_STATE_INVALID",
                "Successful publication report definitive committed state gerektirir",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationPublicationReport {
    pub schema_version: String,
    pub published: bool,
    pub target_path: String,
    pub published_digest: String,
    pub commit_state: SanitizationPublicationCommitState,
    pub warnings: Vec<SanitizationInspectionWarning>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationAssurance {
    NotPublished,
    VerifiedLimited,
    VerifiedCustom,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SanitizationExecutionReport {
    pub schema_version: String,
    pub policy_schema_version: String,
    pub profile: SanitizationProfile,
    pub assurance: SanitizationAssurance,
    pub transform: SanitizationTransformReport,
    pub verification: SanitizationVerificationReport,
    pub publication: Option<SanitizationPublicationReport>,
    pub controls: Vec<String>,
    pub warnings: Vec<SanitizationInspectionWarning>,
}

fn inspection_failed(message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code("SANITIZATION_INSPECTION_FAILED", message)
}

pub fn control_descriptors(
) -> Result<BTreeMap<String, SanitizationControlDescriptor>, SessionJobOperationError> {
    let manifest: ControlManifest = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/features/sanitization/sanitization-controls-v1.json"
    )))
    .map_err(|_| inspection_failed("Sanitization control manifest okunamadı"))?;
    if manifest.schema_version != SANITIZATION_CONTROL_REGISTRY_VERSION {
        return Err(inspection_failed(
            "Sanitization control manifest sürümü desteklenmiyor",
        ));
    }
    let mut controls = BTreeMap::new();
    for descriptor in manifest.controls {
        validate_safe_string(&descriptor.control_id, "controlId")?;
        validate_safe_string(&descriptor.category, "category")?;
        validate_safe_string(&descriptor.expected_visual_impact, "expectedVisualImpact")?;
        validate_safe_string(
            &descriptor.expected_functional_impact,
            "expectedFunctionalImpact",
        )?;
        if controls
            .insert(descriptor.control_id.clone(), descriptor)
            .is_some()
        {
            return Err(inspection_failed(
                "Sanitization control manifest duplicate control içeriyor",
            ));
        }
    }
    if controls.is_empty() {
        return Err(inspection_failed(
            "Sanitization control manifest boş olamaz",
        ));
    }
    Ok(controls)
}

pub fn validate_policy_schema_version(version: &str) -> Result<(), SessionJobOperationError> {
    if version != SANITIZATION_POLICY_SCHEMA_VERSION {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_POLICY_VERSION_UNSUPPORTED",
            "Sanitization policy schema sürümü desteklenmiyor",
        ));
    }
    Ok(())
}

pub fn supported_transform_controls() -> Result<BTreeSet<String>, SessionJobOperationError> {
    Ok(control_descriptors()?
        .into_values()
        .filter(|descriptor| {
            matches!(
                descriptor.future_removal_support,
                FutureRemovalSupport::Supported
                    | FutureRemovalSupport::SupportedWithVisualChange
                    | FutureRemovalSupport::SupportedWithFunctionalChange
            )
        })
        .map(|descriptor| descriptor.control_id)
        .collect())
}

pub fn validate_sanitization_policy(
    policy: &SanitizationPolicy,
) -> Result<(), SessionJobOperationError> {
    validate_policy_schema_version(&policy.schema_version)?;
    if policy.expected_inspection_version != SANITIZATION_INSPECTION_VERSION {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_STALE_INSPECTION",
            "Policy inspection sürümü current authority ile eşleşmiyor",
        ));
    }
    if policy.controls.is_empty() || policy.controls.len() > MAX_SANITIZATION_POLICY_CONTROLS {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_POLICY_INVALID",
            "Sanitization policy control sayısı geçersiz",
        ));
    }
    let manifest = control_descriptors()?;
    let supported = supported_transform_controls()?;
    let mut seen = BTreeSet::new();
    for control in &policy.controls {
        validate_safe_string(control, "policy.control")?;
        if !manifest.contains_key(control) {
            return Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_CONTROL_UNKNOWN",
                "Sanitization policy unknown control içeriyor",
            ));
        }
        if !seen.insert(control.as_str()) {
            return Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_CONTROL_DUPLICATE",
                "Sanitization policy duplicate control içeriyor",
            ));
        }
        if !supported.contains(control) {
            return Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_CONTROL_UNSUPPORTED",
                "Sanitization policy production transform desteği olmayan control içeriyor",
            ));
        }
    }
    if !seen.contains("incremental_revisions") {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_POLICY_INVALID",
            "Full-rewrite profiles incremental_revisions control'ünü içermelidir",
        ));
    }
    if seen.contains("object_metadata")
        && (!seen.contains("xmp_metadata") || !seen.contains("page_metadata"))
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_POLICY_INVALID",
            "object_metadata current scanner semantics altında xmp_metadata ve page_metadata gerektirir",
        ));
    }
    if policy.profile == SanitizationProfile::VerifiedLimitedV1
        && seen != supported.iter().map(String::as_str).collect()
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_POLICY_INVALID",
            "verified_limited_v1 bütün mandatory control'leri içermelidir",
        ));
    }
    Ok(())
}

/// Returns scanner blockers that remain unresolved for the requested phase.
///
/// The inspection report and control registry remain the canonical source of
/// blocker taxonomy. Acknowledged signature invalidation is an input-only
/// exception. Verification may pass that exception only when the transform
/// report has already bound the output to an explicitly acknowledged signature
/// invalidation; a reintroduced signature on an otherwise unsigned transform
/// remains a blocker.
pub fn unresolved_blocking_findings(
    inspection: &SanitizationInspectionReport,
    policy: &SanitizationPolicy,
    allow_acknowledged_signature_invalidation: bool,
) -> Result<Vec<SanitizationBlockingFinding>, SessionJobOperationError> {
    validate_sanitization_policy(policy)?;
    validate_inspection_report(inspection)?;
    let descriptors = control_descriptors()?;
    Ok(inspection
        .blocking_findings
        .iter()
        .filter(|finding| {
            descriptors
                .get(&finding.control_id)
                .is_some_and(|descriptor| descriptor.blocking_for_minimum_profile)
                && !(allow_acknowledged_signature_invalidation
                    && policy.acknowledgements.acknowledge_signature_invalidation
                    && finding.control_id == "digital_signatures")
        })
        .cloned()
        .collect())
}

pub fn inspection_digest(
    report: &SanitizationInspectionReport,
) -> Result<String, SessionJobOperationError> {
    validate_inspection_report(report)?;
    let bytes = serde_json::to_vec(report)
        .map_err(|_| inspection_failed("Inspection digest serialize edilemedi"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_sha256(value: &str, field: &str) -> Result<(), SessionJobOperationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(inspection_failed(format!(
            "{field} geçerli SHA-256 değildir"
        )));
    }
    Ok(())
}

fn validate_safe_string(value: &str, field: &str) -> Result<(), SessionJobOperationError> {
    if value.is_empty() || value.len() > MAX_SANITIZATION_SAFE_STRING_BYTES {
        return Err(inspection_failed(format!(
            "Sanitization report {field} sınırı geçersiz"
        )));
    }
    Ok(())
}

pub fn validate_inspection_report(
    report: &SanitizationInspectionReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != SANITIZATION_REPORT_SCHEMA_VERSION
        || report.inspection_version != SANITIZATION_INSPECTION_VERSION
        || report.policy_schema_version != SANITIZATION_POLICY_SCHEMA_VERSION
        || report.control_registry_version != SANITIZATION_CONTROL_REGISTRY_VERSION
    {
        return Err(inspection_failed(
            "Sanitization inspection report sürümü eşleşmedi",
        ));
    }
    validate_sha256(&report.input_digest, "inspection.inputDigest")?;
    let descriptors = control_descriptors()?;
    if report.controls.len() != descriptors.len() {
        return Err(inspection_failed(
            "Sanitization inspection report control seti eksik",
        ));
    }
    if report.blocking_findings.len() > MAX_SANITIZATION_BLOCKING_FINDINGS
        || report.warnings.len() > MAX_SANITIZATION_WARNINGS
        || report.limits.visited_objects as usize > MAX_SANITIZATION_OBJECTS
        || report.limits.decoded_bytes > MAX_SANITIZATION_TOTAL_DECODED_BYTES
        || report.limits.findings as usize > MAX_SANITIZATION_FINDINGS
        || report.limits.truncated
    {
        return Err(inspection_failed(
            "Sanitization inspection report limit invariantı geçersiz",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut blocking_by_control = BTreeSet::new();
    for finding in &report.blocking_findings {
        if !blocking_by_control.insert(finding.control_id.as_str()) {
            return Err(inspection_failed(
                "Sanitization inspection report duplicate blocking finding içeriyor",
            ));
        }
    }
    let mut total_findings = 0usize;
    for control in &report.controls {
        let descriptor = descriptors
            .get(&control.control_id)
            .ok_or_else(|| inspection_failed("Unknown sanitization control ID"))?;
        if !seen.insert(control.control_id.as_str()) {
            return Err(inspection_failed(
                "Sanitization inspection report duplicate control içeriyor",
            ));
        }
        if control.category != descriptor.category
            || control.inspection_support != descriptor.inspection_support
            || control.future_removal_support != descriptor.future_removal_support
            || control.expected_visual_impact != descriptor.expected_visual_impact
            || control.expected_functional_impact != descriptor.expected_functional_impact
            || control.representative_evidence.len() > MAX_SANITIZATION_EVIDENCE_PER_CONTROL
            || control.finding_count as usize > MAX_SANITIZATION_FINDINGS
            || control.present != (control.finding_count > 0)
        {
            return Err(inspection_failed(
                "Sanitization inspection control invariantı geçersiz",
            ));
        }
        let should_block = descriptor.blocking_for_minimum_profile && control.present;
        if should_block != control.blocking_reason.is_some()
            || should_block != blocking_by_control.contains(control.control_id.as_str())
        {
            return Err(inspection_failed(
                "Sanitization blocking control invariantı geçersiz",
            ));
        }
        if let Some(reason) = &control.blocking_reason {
            validate_safe_string(reason, "blockingReason")?;
        }
        for evidence in &control.representative_evidence {
            validate_safe_string(&evidence.category, "evidence.category")?;
            validate_safe_string(&evidence.logical_path, "evidence.logicalPath")?;
            if let Some(object_id) = &evidence.object_id {
                validate_safe_string(object_id, "evidence.objectId")?;
            }
            if let Some(action_type) = &evidence.event_or_action_type {
                validate_safe_string(action_type, "evidence.eventOrActionType")?;
            }
        }
        total_findings = total_findings
            .checked_add(control.finding_count as usize)
            .ok_or_else(|| inspection_failed("Sanitization finding count overflow"))?;
    }
    if total_findings != report.limits.findings as usize {
        return Err(inspection_failed(
            "Sanitization total finding count invariantı geçersiz",
        ));
    }
    for finding in &report.blocking_findings {
        let descriptor = descriptors
            .get(&finding.control_id)
            .ok_or_else(|| inspection_failed("Unknown blocking control ID"))?;
        let control = report
            .controls
            .iter()
            .find(|control| control.control_id == finding.control_id)
            .ok_or_else(|| inspection_failed("Blocking finding control sonucu bulunamadı"))?;
        if !descriptor.blocking_for_minimum_profile
            || finding.category != descriptor.category
            || finding.evidence_count == 0
            || !control.present
            || control.finding_count == 0
            || finding.evidence_count > control.finding_count
        {
            return Err(inspection_failed(
                "Sanitization blocking finding invariantı geçersiz",
            ));
        }
        validate_safe_string(&finding.code, "blocking.code")?;
        validate_safe_string(&finding.safe_message, "blocking.safeMessage")?;
    }
    for warning in &report.warnings {
        validate_safe_string(&warning.code, "warning.code")?;
        validate_safe_string(&warning.safe_message, "warning.safeMessage")?;
    }
    let encoded = serde_json::to_vec(report)
        .map_err(|_| inspection_failed("Sanitization report serialize edilemedi"))?;
    if encoded.len() > MAX_SANITIZATION_REPORT_BYTES {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_REPORT_LIMIT_EXCEEDED",
            "Sanitization inspection report boyut sınırını aştı",
        ));
    }
    Ok(())
}

pub fn validate_transform_report(
    report: &SanitizationTransformReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != SANITIZATION_TRANSFORM_REPORT_SCHEMA_VERSION
        || report.policy_schema_version != SANITIZATION_POLICY_SCHEMA_VERSION
        || report.control_registry_version != SANITIZATION_CONTROL_REGISTRY_VERSION
        || report.inspection_version != SANITIZATION_INSPECTION_VERSION
        || report.output_relative_name != SANITIZATION_TRANSFORM_OUTPUT_NAME
        || report.status != "transformed_unverified"
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_TRANSFORM_REPORT_INVALID",
            "Sanitization transform report version veya invariant eşleşmedi",
        ));
    }
    validate_sha256(&report.input_digest, "transform.inputDigest")?;
    validate_sha256(&report.output_digest, "transform.outputDigest")?;
    validate_sha256(&report.inspection_digest, "transform.inspectionDigest")?;
    if report.output_size_bytes == 0
        || report.output_size_bytes > MAX_SANITIZATION_TRANSFORM_OUTPUT_BYTES
        || report.controls.is_empty()
        || report.controls.len() > MAX_SANITIZATION_POLICY_CONTROLS
        || report.warnings.len() > MAX_SANITIZATION_WARNINGS
        || report.full_rewrite_evidence.source_bytes_copied
        || report.full_rewrite_evidence.incremental_save_used
        || report.full_rewrite_evidence.active_prev_present
        || report.full_rewrite_evidence.active_xref_stm_present
        || !report.full_rewrite_evidence.parser_reopen_passed
        || !report.full_rewrite_evidence.self_check_passed
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_TRANSFORM_REPORT_INVALID",
            "Sanitization transform report safety invariantı geçersiz",
        ));
    }
    validate_document_invariants(&report.expected_document_invariants)?;
    if report.certification_lost && !report.signatures_invalidated {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_TRANSFORM_REPORT_INVALID",
            "Certification loss signature invalidation olmadan raporlanamaz",
        ));
    }
    let supported = supported_transform_controls()?;
    let mut seen = BTreeSet::new();
    let mut evidence_count = 0usize;
    for control in &report.controls {
        if !control.requested
            || !supported.contains(&control.control_id)
            || !seen.insert(control.control_id.as_str())
            || control.detected_before != (control.removed_count > 0)
            || control.evidence.len() > MAX_SANITIZATION_EVIDENCE_PER_CONTROL
            || control.warnings.len() > MAX_SANITIZATION_WARNINGS
            || (control.removed_count == 0
                && control.transform_status != SanitizationTransformStatus::NotFound)
            || (control.removed_count > 0
                && control.transform_status != SanitizationTransformStatus::RemovedCandidate)
        {
            return Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_TRANSFORM_REPORT_INVALID",
                "Sanitization control transform sonucu geçersiz",
            ));
        }
        evidence_count = evidence_count
            .checked_add(control.evidence.len())
            .ok_or_else(|| inspection_failed("Transform evidence count overflow"))?;
        for evidence in &control.evidence {
            validate_safe_string(&evidence.logical_path, "transform.evidence.logicalPath")?;
            if let Some(object_id) = &evidence.object_id {
                validate_safe_string(object_id, "transform.evidence.objectId")?;
            }
            if let Some(digest) = &evidence.payload_sha256 {
                validate_sha256(digest, "transform.evidence.payloadSha256")?;
            }
        }
    }
    if evidence_count > MAX_SANITIZATION_TRANSFORM_EVIDENCE {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_LIMIT_EXCEEDED",
            "Sanitization transform evidence sınırı aşıldı",
        ));
    }
    let encoded = serde_json::to_vec(report)
        .map_err(|_| inspection_failed("Transform report serialize edilemedi"))?;
    if encoded.len() > MAX_SANITIZATION_REPORT_BYTES {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_REPORT_LIMIT_EXCEEDED",
            "Sanitization transform report boyut sınırını aştı",
        ));
    }
    Ok(())
}

fn validate_document_invariants(
    invariants: &SanitizationDocumentInvariants,
) -> Result<(), SessionJobOperationError> {
    if invariants.page_count == 0
        || invariants.pages.len() != invariants.page_count as usize
        || invariants.pages.len() > MAX_SANITIZATION_VERIFICATION_PAGES
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_TRANSFORM_REPORT_INVALID",
            "Expected document page invariant seti geçersiz",
        ));
    }
    for (index, page) in invariants.pages.iter().enumerate() {
        if page.page_index != index as u32
            || !page.width_points.is_finite()
            || !page.height_points.is_finite()
            || page.width_points <= 0.0
            || page.height_points <= 0.0
            || !matches!(page.rotation_degrees, 0 | 90 | 180 | 270)
        {
            return Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_TRANSFORM_REPORT_INVALID",
                "Expected page geometry invariantı geçersiz",
            ));
        }
        validate_sha256(&page.content_sha256, "transform.page.contentSha256")?;
    }
    Ok(())
}

fn validate_verification_check(
    check: &SanitizationVerificationCheck,
) -> Result<(), SessionJobOperationError> {
    match check.status {
        SanitizationVerificationStatus::Passed if check.message.is_some() => {
            Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_VERIFICATION_REPORT_INVALID",
                "Passed verification check message taşımamalıdır",
            ))
        }
        SanitizationVerificationStatus::Failed
        | SanitizationVerificationStatus::SkippedWithReason
            if check.message.as_deref().is_none_or(str::is_empty) =>
        {
            Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_VERIFICATION_REPORT_INVALID",
                "Failed/skipped verification check bounded reason taşımalıdır",
            ))
        }
        _ => {
            if let Some(message) = &check.message {
                validate_safe_string(message, "verification.check.message")?;
            }
            Ok(())
        }
    }
}

pub fn validate_verification_report(
    report: &SanitizationVerificationReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION
        || report.policy_schema_version != SANITIZATION_POLICY_SCHEMA_VERSION
        || report.control_registry_version != SANITIZATION_CONTROL_REGISTRY_VERSION
        || report.verification_version != SANITIZATION_VERIFICATION_VERSION
        || report.inspection_version != SANITIZATION_INSPECTION_VERSION
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_VERIFICATION_REPORT_INVALID",
            "Sanitization verification report version invariantı geçersiz",
        ));
    }
    validate_sha256(&report.input_digest, "verification.inputDigest")?;
    validate_sha256(&report.transform_digest, "verification.transformDigest")?;
    let mandatory = [
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
    for check in mandatory {
        validate_verification_check(check)?;
    }
    if report.controls.is_empty()
        || report.controls.len() > MAX_SANITIZATION_POLICY_CONTROLS
        || report.blockers.len() > MAX_SANITIZATION_BLOCKING_FINDINGS
        || report.warnings.len() > MAX_SANITIZATION_WARNINGS
        || report.unresolved_findings.len() > MAX_SANITIZATION_FINDINGS
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_VERIFICATION_REPORT_INVALID",
            "Sanitization verification report collection sınırı geçersiz",
        ));
    }
    let supported = supported_transform_controls()?;
    let mut seen = BTreeSet::new();
    for control in &report.controls {
        if !control.requested
            || !supported.contains(&control.control_id)
            || !seen.insert(control.control_id.as_str())
            || control.warnings.len() > MAX_SANITIZATION_WARNINGS
            || (control.status == SanitizationVerificationStatus::Passed
                && (control.forbidden_findings_after != 0
                    || control.payload_fingerprints_remaining != 0))
        {
            return Err(SessionJobOperationError::failed_with_code(
                "SANITIZATION_VERIFICATION_REPORT_INVALID",
                "Sanitization control verification invariantı geçersiz",
            ));
        }
    }
    let descriptors = control_descriptors()?;
    for finding in &report.unresolved_findings {
        validate_safe_string(&finding.code, "verification.finding.code")?;
        if let Some(control_id) = &finding.control_id {
            if !descriptors.contains_key(control_id) {
                return Err(SessionJobOperationError::failed_with_code(
                    "SANITIZATION_VERIFICATION_REPORT_INVALID",
                    "Verification finding canonical control registry ile eşleşmiyor",
                ));
            }
        }
        for value in [&finding.object_id, &finding.logical_path]
            .into_iter()
            .flatten()
        {
            validate_safe_string(value, "verification.finding.evidence")?;
        }
    }
    for message in report.blockers.iter().chain(&report.warnings) {
        validate_safe_string(&message.code, "verification.message.code")?;
        validate_safe_string(&message.safe_message, "verification.message.safeMessage")?;
    }
    let computed_verified = mandatory
        .iter()
        .all(|check| check.status == SanitizationVerificationStatus::Passed)
        && report.blockers.is_empty()
        && !report
            .unresolved_findings
            .iter()
            .any(|finding| finding.mandatory)
        && report
            .controls
            .iter()
            .all(|control| control.status == SanitizationVerificationStatus::Passed);
    if report.is_verified != computed_verified {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_VERIFICATION_REPORT_INVALID",
            "isVerified mandatory verification invariantı ile eşleşmiyor",
        ));
    }
    let encoded = serde_json::to_vec(report)
        .map_err(|_| inspection_failed("Verification report serialize edilemedi"))?;
    if encoded.len() > MAX_SANITIZATION_REPORT_BYTES {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_REPORT_LIMIT_EXCEEDED",
            "Sanitization verification report boyut sınırını aştı",
        ));
    }
    Ok(())
}

pub fn validate_verification_report_for_transform(
    report: &SanitizationVerificationReport,
    transform: &SanitizationTransformReport,
    policy: &SanitizationPolicy,
) -> Result<(), SessionJobOperationError> {
    validate_verification_report(report)?;
    validate_transform_report_for_policy(
        transform,
        policy,
        &transform.input_digest,
        &transform.inspection_digest,
    )?;
    if report.policy_schema_version != policy.schema_version
        || report.profile != policy.profile
        || report.input_digest != transform.input_digest
        || report.transform_digest != transform.output_digest
        || report.inspection_version != transform.inspection_version
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_VERIFICATION_REPORT_MISMATCH",
            "Verification report transform authority ile eşleşmiyor",
        ));
    }
    let requested: BTreeSet<&str> = policy.controls.iter().map(String::as_str).collect();
    let reported: BTreeSet<&str> = report
        .controls
        .iter()
        .map(|control| control.control_id.as_str())
        .collect();
    if requested != reported {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_VERIFICATION_REPORT_MISMATCH",
            "Verification report requested control seti ile eşleşmiyor",
        ));
    }
    Ok(())
}

pub fn validate_publication_report(
    report: &SanitizationPublicationReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != SANITIZATION_PUBLICATION_REPORT_SCHEMA_VERSION
        || !report.published
        || report.target_path.trim().is_empty()
        || report.target_path.len() > 4096
        || report.target_path.contains('\0')
        || report.warnings.len() > MAX_SANITIZATION_WARNINGS
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_PUBLICATION_REPORT_INVALID",
            "Sanitization publication report invariantı geçersiz",
        ));
    }
    validate_sha256(&report.published_digest, "publication.publishedDigest")?;
    for warning in &report.warnings {
        validate_safe_string(&warning.code, "publication.warning.code")?;
        validate_safe_string(&warning.safe_message, "publication.warning.safeMessage")?;
    }
    Ok(())
}

pub fn validate_execution_report(
    report: &SanitizationExecutionReport,
    policy: &SanitizationPolicy,
) -> Result<(), SessionJobOperationError> {
    validate_sanitization_policy(policy)?;
    validate_transform_report_for_policy(
        &report.transform,
        policy,
        &report.transform.input_digest,
        &report.transform.inspection_digest,
    )?;
    validate_verification_report_for_transform(&report.verification, &report.transform, policy)?;
    if report.schema_version != SANITIZATION_EXECUTION_REPORT_SCHEMA_VERSION
        || report.policy_schema_version != policy.schema_version
        || report.profile != policy.profile
        || report.controls != policy.controls
        || report.warnings.len() > MAX_SANITIZATION_WARNINGS
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_EXECUTION_REPORT_INVALID",
            "Sanitization execution report request correlation invariantı geçersiz",
        ));
    }
    for warning in &report.warnings {
        validate_safe_string(&warning.code, "execution.warning.code")?;
        validate_safe_string(&warning.safe_message, "execution.warning.safeMessage")?;
    }

    match &report.publication {
        Some(publication) => {
            validate_publication_report(publication)?;
            let expected_assurance = match report.profile {
                SanitizationProfile::VerifiedLimitedV1 => SanitizationAssurance::VerifiedLimited,
                SanitizationProfile::CustomSupportedV1 => SanitizationAssurance::VerifiedCustom,
            };
            if !report.verification.is_verified
                || !report.verification.blockers.is_empty()
                || report.assurance != expected_assurance
                || publication.published_digest != report.transform.output_digest
                || publication.published_digest != report.verification.transform_digest
            {
                return Err(SessionJobOperationError::failed_with_code(
                    "SANITIZATION_EXECUTION_REPORT_INVALID",
                    "Published execution exact verified digest ve assurance invariantını karşılamıyor",
                ));
            }
        }
        None => {
            if report.assurance != SanitizationAssurance::NotPublished {
                return Err(SessionJobOperationError::failed_with_code(
                    "SANITIZATION_EXECUTION_REPORT_INVALID",
                    "Publication olmadan verified assurance üretilemez",
                ));
            }
        }
    }
    let encoded = serde_json::to_vec(report)
        .map_err(|_| inspection_failed("Execution report serialize edilemedi"))?;
    if encoded.len() > MAX_SANITIZATION_REPORT_BYTES {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_REPORT_LIMIT_EXCEEDED",
            "Sanitization execution report boyut sınırını aştı",
        ));
    }
    Ok(())
}

/// Validates that a structurally valid worker report is correlated to the
/// authoritative transform request held by the parent. This is intentionally
/// separate from `validate_transform_report`: worker self-validation cannot
/// establish request/report identity at the parent trust boundary.
pub fn validate_transform_report_for_policy(
    report: &SanitizationTransformReport,
    policy: &SanitizationPolicy,
    expected_input_digest: &str,
    expected_inspection_digest: &str,
) -> Result<(), SessionJobOperationError> {
    validate_transform_report(report)?;
    validate_sanitization_policy(policy)?;
    if report.policy_schema_version != policy.schema_version
        || report.profile != policy.profile
        || report.inspection_version != policy.expected_inspection_version
        || report.input_digest != expected_input_digest
        || report.inspection_digest != expected_inspection_digest
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_TRANSFORM_REPORT_MISMATCH",
            "Sanitization transform report authoritative request ile eşleşmedi",
        ));
    }

    let requested: BTreeSet<&str> = policy.controls.iter().map(String::as_str).collect();
    let reported: BTreeSet<&str> = report
        .controls
        .iter()
        .map(|control| control.control_id.as_str())
        .collect();
    if requested != reported {
        return Err(SessionJobOperationError::failed_with_code(
            "SANITIZATION_TRANSFORM_REPORT_MISMATCH",
            "Sanitization transform report requested control seti ile eşleşmedi",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn inspect_document_sanitization(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy_schema_version: String,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<
    SessionJobOutcome<SanitizationInspectionReport>,
    crate::document_session::DocumentSessionError,
> {
    inspect_document_sanitization_core(
        job_id,
        session_id,
        expected_content_generation,
        policy_schema_version,
        &store,
        std::time::Duration::from_secs(180),
        #[cfg(feature = "sanitization-test-support")]
        crate::sanitization_inspection::SanitizationInspectionTestBehavior::Real,
    )
    .await
}

async fn inspect_document_sanitization_core(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy_schema_version: String,
    store: &crate::document_session::DocumentSessionStore,
    timeout: std::time::Duration,
    #[cfg(feature = "sanitization-test-support")]
    test_behavior: crate::sanitization_inspection::SanitizationInspectionTestBehavior,
) -> Result<
    SessionJobOutcome<SanitizationInspectionReport>,
    crate::document_session::DocumentSessionError,
> {
    validate_policy_schema_version(&policy_schema_version).map_err(|error| match error {
        SessionJobOperationError::Failed(failure) => {
            crate::document_session::DocumentSessionError::new(failure.code, failure.message)
        }
        SessionJobOperationError::Cancelled(_) => {
            crate::document_session::DocumentSessionError::new(
                "SESSION_JOB_CANCELLED",
                "Sanitization inspection başlamadan iptal edildi",
            )
        }
    })?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::InspectDocumentSanitization {
                        schema_version: SANITIZATION_REPORT_SCHEMA_VERSION.to_string(),
                        policy_schema_version,
                        inspection_profile: SANITIZATION_INSPECTION_PROFILE.to_string(),
                        #[cfg(feature = "sanitization-test-support")]
                        test_behavior,
                    },
                    timeout,
                )?;
                let crate::job_worker::JobResult::SanitizationInspection(report) = result else {
                    return Err(inspection_failed(
                        "Sanitization worker beklenmeyen result variant döndürdü",
                    ));
                };
                validate_inspection_report(&report)?;
                Ok(report)
            },
        )
        .await
}

#[cfg(feature = "sanitization-test-support")]
pub async fn inspect_document_sanitization_core_with_test_support(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy_schema_version: String,
    store: &crate::document_session::DocumentSessionStore,
    timeout: std::time::Duration,
    test_behavior: crate::sanitization_inspection::SanitizationInspectionTestBehavior,
) -> Result<
    SessionJobOutcome<SanitizationInspectionReport>,
    crate::document_session::DocumentSessionError,
> {
    inspect_document_sanitization_core(
        job_id,
        session_id,
        expected_content_generation,
        policy_schema_version,
        store,
        timeout,
        test_behavior,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsupported_report() -> SanitizationInspectionReport {
        crate::sanitization_inspection::inspect_snapshot(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/sanitization/unsupported-active-content.pdf"),
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap()
    }

    #[test]
    fn control_manifest_is_unique_and_separates_inspection_from_removal() {
        let controls = control_descriptors().unwrap();
        assert!(controls.len() >= 25);
        let xfa = &controls["xfa"];
        assert_eq!(
            xfa.inspection_support,
            InspectionSupport::SupportedForInspection
        );
        assert_eq!(
            xfa.future_removal_support,
            FutureRemovalSupport::Unsupported
        );
        assert!(xfa.blocking_for_minimum_profile);
        let supported = supported_transform_controls().unwrap();
        assert_eq!(supported.len(), 16);
        assert!(supported.contains("document_javascript"));
        assert!(supported.contains("incremental_revisions"));
        assert!(!supported.contains("document_identifier"));
        assert!(!supported.contains("optional_content"));
    }

    #[test]
    fn policy_schema_is_strict() {
        assert!(validate_policy_schema_version(SANITIZATION_POLICY_SCHEMA_VERSION).is_ok());
        assert!(validate_policy_schema_version("future").is_err());
        let supported = supported_transform_controls().unwrap();
        let policy = SanitizationPolicy {
            schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            profile: SanitizationProfile::VerifiedLimitedV1,
            controls: supported.into_iter().collect(),
            acknowledgements: SanitizationAcknowledgements {
                acknowledge_signature_invalidation: false,
            },
            expected_inspection_version: SANITIZATION_INSPECTION_VERSION.to_string(),
        };
        assert!(validate_sanitization_policy(&policy).is_ok());
        let mut wire = serde_json::to_value(&policy).unwrap();
        wire.as_object_mut().unwrap().insert(
            "removeHiddenLayers".to_string(),
            serde_json::Value::Bool(true),
        );
        assert!(serde_json::from_value::<SanitizationPolicy>(wire).is_err());
        let mut unknown_profile = serde_json::to_value(&policy).unwrap();
        unknown_profile["profile"] = serde_json::Value::String("future".to_string());
        assert!(serde_json::from_value::<SanitizationPolicy>(unknown_profile).is_err());
    }

    #[test]
    fn report_rejects_duplicate_blockers_and_cross_field_evidence_contradictions() {
        let mut report = unsupported_report();
        let valid_blocker = &mut report.blocking_findings[0];
        let valid_control = report
            .controls
            .iter_mut()
            .find(|control| control.control_id == valid_blocker.control_id)
            .unwrap();
        report.limits.findings = report.limits.findings - valid_control.finding_count + 3;
        valid_control.finding_count = 3;
        valid_control.present = true;
        valid_blocker.evidence_count = 1;
        validate_inspection_report(&report).unwrap();

        let mut duplicate = report.clone();
        duplicate
            .blocking_findings
            .push(duplicate.blocking_findings[0].clone());
        assert!(validate_inspection_report(&duplicate).is_err());

        let mut excessive_evidence = report.clone();
        let blocker = &mut excessive_evidence.blocking_findings[0];
        let control = excessive_evidence
            .controls
            .iter()
            .find(|control| control.control_id == blocker.control_id)
            .unwrap();
        blocker.evidence_count = control.finding_count + 1;
        assert!(validate_inspection_report(&excessive_evidence).is_err());

        let mut absent = report;
        let blocker_control_id = absent.blocking_findings[0].control_id.clone();
        let control = absent
            .controls
            .iter_mut()
            .find(|control| control.control_id == blocker_control_id)
            .unwrap();
        absent.limits.findings -= control.finding_count;
        control.present = false;
        control.finding_count = 0;
        control.blocking_reason = None;
        control.representative_evidence.clear();
        assert!(validate_inspection_report(&absent).is_err());
    }
}
