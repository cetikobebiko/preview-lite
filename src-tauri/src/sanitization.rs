use crate::document_session::{SessionJobOperationError, SessionJobOutcome};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SANITIZATION_REPORT_SCHEMA_VERSION: &str = "1.0.0";
pub const SANITIZATION_INSPECTION_VERSION: &str = "sanitization-inspection-v1";
pub const SANITIZATION_POLICY_SCHEMA_VERSION: &str = "sanitization-policy-v1";
pub const SANITIZATION_INSPECTION_PROFILE: &str = "minimum-profile-inspection-v1";

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
    pub document_facts: SanitizationDocumentFacts,
    pub controls: Vec<SanitizationControlInspection>,
    pub blocking_findings: Vec<SanitizationBlockingFinding>,
    pub warnings: Vec<SanitizationInspectionWarning>,
    pub limits: SanitizationInspectionLimits,
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
    if manifest.schema_version != "sanitization-controls/1" {
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
    {
        return Err(inspection_failed(
            "Sanitization inspection report sürümü eşleşmedi",
        ));
    }
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
    let blocking_by_control: BTreeSet<_> = report
        .blocking_findings
        .iter()
        .map(|finding| finding.control_id.as_str())
        .collect();
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
        if !descriptor.blocking_for_minimum_profile
            || finding.category != descriptor.category
            || finding.evidence_count == 0
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
                    },
                    std::time::Duration::from_secs(180),
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn policy_schema_is_strict() {
        assert!(validate_policy_schema_version(SANITIZATION_POLICY_SCHEMA_VERSION).is_ok());
        assert!(validate_policy_schema_version("future").is_err());
    }
}
