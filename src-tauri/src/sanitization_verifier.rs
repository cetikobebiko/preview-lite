//! Independent verification of the private sanitization transform artifact.
//!
//! The verifier runs in a second ephemeral worker process. It accepts only the
//! canonical job artifact identity, independently reopens the serialized PDF,
//! and never mutates or publishes it.

use crate::document_session::SessionJobOperationError;
use crate::sanitization::{
    validate_sanitization_policy, validate_transform_report_for_policy,
    validate_verification_report, SanitizationControlVerification, SanitizationDocumentInvariants,
    SanitizationInspectionWarning, SanitizationPageInvariant, SanitizationPolicy,
    SanitizationProfile, SanitizationTransformReport, SanitizationVerificationCheck,
    SanitizationVerificationFinding, SanitizationVerificationReport,
    SanitizationVerificationStatus, MAX_SANITIZATION_DECODED_STREAM_BYTES,
    MAX_SANITIZATION_OBJECTS, MAX_SANITIZATION_RAW_SCAN_BYTES,
    MAX_SANITIZATION_TOTAL_DECODED_BYTES, MAX_SANITIZATION_TRANSFORM_OUTPUT_BYTES,
    MAX_SANITIZATION_TRAVERSAL_DEPTH, MAX_SANITIZATION_VERIFICATION_FINGERPRINTS,
    MAX_SANITIZATION_VERIFICATION_PAGES, SANITIZATION_CONTROL_REGISTRY_VERSION,
    SANITIZATION_INSPECTION_PROFILE, SANITIZATION_INSPECTION_VERSION,
    SANITIZATION_POLICY_SCHEMA_VERSION, SANITIZATION_REPORT_SCHEMA_VERSION,
    SANITIZATION_TRANSFORM_OUTPUT_NAME, SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION,
    SANITIZATION_VERIFICATION_VERSION,
};
use lopdf::{Dictionary, Document, Object, ObjectId};
use pdfium_render::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(feature = "sanitization-test-support")]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizationVerificationTestBehavior {
    #[default]
    Real,
    CrashDuringTraversal,
    HangDuringTraversal,
    SkipMandatoryRender,
    FailPdfiumReopen,
}

fn verification_error(code: &'static str, message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

fn passed() -> SanitizationVerificationCheck {
    SanitizationVerificationCheck {
        status: SanitizationVerificationStatus::Passed,
        message: None,
    }
}

fn failed(message: impl Into<String>) -> SanitizationVerificationCheck {
    SanitizationVerificationCheck {
        status: SanitizationVerificationStatus::Failed,
        message: Some(message.into()),
    }
}

fn skipped(message: impl Into<String>) -> SanitizationVerificationCheck {
    SanitizationVerificationCheck {
        status: SanitizationVerificationStatus::SkippedWithReason,
        message: Some(message.into()),
    }
}

fn safe_warning(code: &str, message: &str) -> SanitizationInspectionWarning {
    SanitizationInspectionWarning {
        code: code.to_string(),
        safe_message: message.to_string(),
    }
}

fn canonical_artifact(job_dir: &Path) -> Result<PathBuf, SessionJobOperationError> {
    let dir_metadata = fs::symlink_metadata(job_dir).map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Private job directory metadata okunamadı",
        )
    })?;
    if !dir_metadata.file_type().is_dir() || dir_metadata.file_type().is_symlink() {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Private job directory regular non-symlink directory değildir",
        ));
    }
    let canonical_dir = job_dir.canonicalize().map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Private job directory canonicalize edilemedi",
        )
    })?;
    let artifact = job_dir.join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
    let metadata = fs::symlink_metadata(&artifact).map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Canonical transform artifact bulunamadı",
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_SANITIZATION_TRANSFORM_OUTPUT_BYTES
    {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Canonical transform artifact regular/non-empty/size invariantını sağlamıyor",
        ));
    }
    let canonical_artifact = artifact.canonicalize().map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Canonical transform artifact çözümlenemedi",
        )
    })?;
    if canonical_artifact.parent() != Some(canonical_dir.as_path()) {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Canonical transform artifact private job directory dışına çıkıyor",
        ));
    }
    Ok(canonical_artifact)
}

fn resolve_object<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        value => Some(value),
    }
}

fn resolve_dictionary<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    resolve_object(document, object)?.as_dict().ok()
}

fn dictionary_name(dictionary: &Dictionary, key: &[u8]) -> Option<Vec<u8>> {
    dictionary
        .get(key)
        .ok()?
        .as_name()
        .ok()
        .map(ToOwned::to_owned)
}

fn inherited_page_value<'a>(
    document: &'a Document,
    page_id: ObjectId,
    key: &[u8],
) -> Result<&'a Object, SessionJobOperationError> {
    let mut current = page_id;
    let mut visited = HashSet::new();
    for _ in 0..=MAX_SANITIZATION_TRAVERSAL_DEPTH {
        if !visited.insert(current) {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                "Output page tree cycle içeriyor",
            ));
        }
        let dictionary = document.get_dictionary(current).map_err(|_| {
            verification_error(
                "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                "Output page tree dangling reference içeriyor",
            )
        })?;
        if let Ok(value) = dictionary.get(key) {
            return Ok(value);
        }
        current = dictionary
            .get(b"Parent")
            .and_then(Object::as_reference)
            .map_err(|_| {
                verification_error(
                    "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                    "Output page geometry inheritance tamamlanamadı",
                )
            })?;
    }
    Err(verification_error(
        "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
        "Output page tree traversal limiti aşıldı",
    ))
}

fn page_box_size(
    document: &Document,
    page_id: ObjectId,
) -> Result<(f32, f32), SessionJobOperationError> {
    let object = inherited_page_value(document, page_id, b"CropBox")
        .or_else(|_| inherited_page_value(document, page_id, b"MediaBox"))?;
    let array = resolve_object(document, object)
        .and_then(|value| value.as_array().ok())
        .ok_or_else(|| {
            verification_error(
                "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                "Output page box çözümlenemedi",
            )
        })?;
    if array.len() != 4 {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
            "Output page box dört koordinat taşımıyor",
        ));
    }
    let values: Result<Vec<f32>, _> = array.iter().map(Object::as_float).collect();
    let values = values.map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
            "Output page box numeric değildir",
        )
    })?;
    let width = (values[2] - values[0]).abs();
    let height = (values[3] - values[1]).abs();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
            "Output page geometry geçersizdir",
        ));
    }
    Ok((width, height))
}

fn count_named_destinations(document: &Document) -> u32 {
    document
        .objects
        .values()
        .filter_map(|object| match object {
            Object::Dictionary(dictionary) => dictionary.get(b"Names").ok(),
            Object::Stream(stream) => stream.dict.get(b"Names").ok(),
            _ => None,
        })
        .filter_map(|value| resolve_object(document, value))
        .filter_map(|value| value.as_array().ok())
        .map(|array| array.len() / 2)
        .sum::<usize>()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn count_local_goto_actions(document: &Document) -> u32 {
    document
        .objects
        .values()
        .filter_map(|object| match object {
            Object::Dictionary(dictionary) => Some(dictionary),
            Object::Stream(stream) => Some(&stream.dict),
            _ => None,
        })
        .filter(|dictionary| dictionary_name(dictionary, b"S").as_deref() == Some(b"GoTo"))
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn observe_document_invariants(
    document: &Document,
) -> Result<SanitizationDocumentInvariants, SessionJobOperationError> {
    let catalog = document.catalog().map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
            "Output catalog çözümlenemedi",
        )
    })?;
    let page_ids = document.get_pages();
    validate_verification_page_count(page_ids.len())?;
    let mut pages = Vec::with_capacity(page_ids.len());
    for (page_number, page_id) in page_ids {
        let page = document.get_dictionary(page_id).map_err(|_| {
            verification_error(
                "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                "Output page reference çözümlenemedi",
            )
        })?;
        if dictionary_name(page, b"Type").as_deref() != Some(b"Page") {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                "Output page tree member /Page değildir",
            ));
        }
        let (width_points, height_points) = page_box_size(document, page_id)?;
        let rotation_degrees = page
            .get(b"Rotate")
            .ok()
            .and_then(|value| resolve_object(document, value))
            .and_then(|value| value.as_i64().ok())
            .unwrap_or(0)
            .rem_euclid(360) as i32;
        // lopdf 0.34 also fell back to raw bytes on decode failure; only the Result is gone.
        let content = document.get_page_content(page_id);
        pages.push(SanitizationPageInvariant {
            page_index: page_number - 1,
            width_points,
            height_points,
            rotation_degrees,
            content_sha256: format!("{:x}", Sha256::digest(content)),
        });
    }
    let form_field_count = catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|form| form.get(b"Fields").ok())
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_array().ok())
        .map(Vec::len)
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX);
    Ok(SanitizationDocumentInvariants {
        page_count: pages.len().try_into().map_err(|_| {
            verification_error(
                "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
                "Output page count limiti aşıldı",
            )
        })?,
        pages,
        named_destination_count: count_named_destinations(document),
        local_goto_action_count: count_local_goto_actions(document),
        form_field_count,
        outline_root_present: catalog.has(b"Outlines"),
    })
}

fn validate_verification_page_count(page_count: usize) -> Result<(), SessionJobOperationError> {
    if page_count > MAX_SANITIZATION_VERIFICATION_PAGES {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
            "Actual output page count verification limitini aştı",
        ));
    }
    Ok(())
}

fn validate_all_references(document: &Document) -> Result<(), SessionJobOperationError> {
    fn walk(
        document: &Document,
        object: &Object,
        visited: &mut HashSet<ObjectId>,
        nodes: &mut usize,
        depth: usize,
    ) -> Result<(), SessionJobOperationError> {
        if depth > MAX_SANITIZATION_TRAVERSAL_DEPTH {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
                "Object graph traversal depth limiti aşıldı",
            ));
        }
        *nodes += 1;
        if *nodes > MAX_SANITIZATION_OBJECTS * 20 {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
                "Object graph node limiti aşıldı",
            ));
        }
        match object {
            Object::Reference(id) => {
                let resolved = document.objects.get(id).ok_or_else(|| {
                    verification_error(
                        "SANITIZATION_VERIFICATION_PAGE_TREE_INVALID",
                        "Output object graph dangling reference içeriyor",
                    )
                })?;
                if visited.insert(*id) {
                    walk(document, resolved, visited, nodes, depth + 1)?;
                }
            }
            Object::Array(values) => {
                for value in values {
                    walk(document, value, visited, nodes, depth + 1)?;
                }
            }
            Object::Dictionary(dictionary) => {
                for (_, value) in dictionary.iter() {
                    walk(document, value, visited, nodes, depth + 1)?;
                }
            }
            Object::Stream(stream) => {
                for (_, value) in stream.dict.iter() {
                    walk(document, value, visited, nodes, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut visited = HashSet::new();
    let mut nodes = 0usize;
    walk(
        document,
        &Object::Dictionary(document.trailer.clone()),
        &mut visited,
        &mut nodes,
        0,
    )?;
    Ok(())
}

fn count_marker(bytes: &[u8], marker: &[u8]) -> usize {
    if marker.is_empty() {
        return 0;
    }
    bytes
        .windows(marker.len())
        .filter(|window| *window == marker)
        .count()
}

fn verify_full_rewrite(document: &Document, bytes: &[u8]) -> Result<(), SessionJobOperationError> {
    // lopdf 0.45 strips /Prev and /XRefStm on load; the raw trailer is authoritative, and a
    // trailer that cannot be located is not a proven full rewrite.
    if crate::lopdf_compat::trailer_history(bytes, document).is_none_or(|history| history.any()) {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_FULL_REWRITE_FAILED",
            "Output active history pointer içeriyor",
        ));
    }
    if count_marker(bytes, b"%PDF-") != 1
        || count_marker(bytes, b"startxref") != 1
        || count_marker(bytes, b"%%EOF") != 1
    {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_FULL_REWRITE_FAILED",
            "Output tek self-contained PDF serialization değildir",
        ));
    }
    let mut reachable = document.clone();
    let unreachable = reachable.prune_objects();
    let unexpected_unreachable = unreachable.iter().any(|id| {
        !document.objects.get(id).is_some_and(|object| {
            matches!(
                object,
                Object::Stream(stream)
                    if dictionary_name(&stream.dict, b"Type").as_deref() == Some(b"XRef")
            )
        })
    });
    if unexpected_unreachable {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_FULL_REWRITE_FAILED",
            "Output unreachable indirect object içeriyor",
        ));
    }
    validate_all_references(document)
}

fn payload_fingerprints(
    transform: &SanitizationTransformReport,
) -> Result<BTreeMap<String, BTreeSet<String>>, SessionJobOperationError> {
    let mut result = BTreeMap::new();
    let mut count = 0usize;
    for control in &transform.controls {
        let values = result
            .entry(control.control_id.clone())
            .or_insert_with(BTreeSet::new);
        for evidence in &control.evidence {
            if let Some(digest) = &evidence.payload_sha256 {
                values.insert(digest.clone());
                count += 1;
                if count > MAX_SANITIZATION_VERIFICATION_FINGERPRINTS {
                    return Err(verification_error(
                        "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
                        "Payload fingerprint limiti aşıldı",
                    ));
                }
            }
        }
    }
    Ok(result)
}

fn validate_payload_stream_bytes(additional: u64) -> Result<(), SessionJobOperationError> {
    if additional > MAX_SANITIZATION_DECODED_STREAM_BYTES {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
            "Decoded payload stream byte limiti aşıldı",
        ));
    }
    Ok(())
}

fn account_payload_bytes(total: &mut u64, additional: u64) -> Result<(), SessionJobOperationError> {
    *total = total.checked_add(additional).ok_or_else(|| {
        verification_error(
            "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
            "Payload scan byte count overflow",
        )
    })?;
    if *total > MAX_SANITIZATION_TOTAL_DECODED_BYTES {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
            "Decoded payload scan byte limiti aşıldı",
        ));
    }
    Ok(())
}

fn observed_payload_digests(
    document: &Document,
) -> Result<BTreeSet<String>, SessionJobOperationError> {
    fn payload_stream_context(key: &[u8]) -> bool {
        matches!(key, b"JS" | b"Metadata" | b"EF" | b"RF" | b"EmbeddedFile")
    }

    fn stream_requires_decoded_payload_scan(stream: &lopdf::Stream, inherited: bool) -> bool {
        inherited
            || matches!(
                dictionary_name(&stream.dict, b"Type").as_deref(),
                Some(b"EmbeddedFile") | Some(b"Metadata")
            )
    }

    struct PayloadScanState<'a> {
        observed: &'a mut BTreeSet<String>,
        visited: &'a mut HashSet<ObjectId>,
        decoded_bytes: &'a mut u64,
        nodes: &'a mut usize,
    }

    fn scan(
        document: &Document,
        object: &Object,
        state: &mut PayloadScanState<'_>,
        depth: usize,
        inherited_payload_context: bool,
    ) -> Result<(), SessionJobOperationError> {
        if depth > MAX_SANITIZATION_TRAVERSAL_DEPTH {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
                "Payload traversal depth limiti aşıldı",
            ));
        }
        *state.nodes += 1;
        if *state.nodes > MAX_SANITIZATION_OBJECTS * 20 {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
                "Payload traversal node limiti aşıldı",
            ));
        }
        match object {
            Object::Reference(id) => {
                if state.visited.insert(*id) {
                    let value = document.objects.get(id).ok_or_else(|| {
                        verification_error(
                            "SANITIZATION_VERIFICATION_PAYLOAD_SCAN_FAILED",
                            "Payload graph dangling reference içeriyor",
                        )
                    })?;
                    scan(document, value, state, depth + 1, inherited_payload_context)?;
                }
            }
            Object::String(value, _) => {
                account_payload_bytes(state.decoded_bytes, value.len() as u64)?;
                state
                    .observed
                    .insert(format!("{:x}", Sha256::digest(value)));
            }
            Object::Array(values) => {
                for value in values {
                    scan(document, value, state, depth + 1, inherited_payload_context)?;
                }
            }
            Object::Dictionary(dictionary) => {
                for (key, value) in dictionary.iter() {
                    scan(
                        document,
                        value,
                        state,
                        depth + 1,
                        inherited_payload_context || payload_stream_context(key),
                    )?;
                }
            }
            Object::Stream(stream) => {
                // Only streams that can carry transform fingerprint payloads are
                // decoded. Their decoded bytes are the absence-proof authority;
                // encoded bytes are never substituted on decode failure.
                if stream_requires_decoded_payload_scan(stream, inherited_payload_context) {
                    let decoded = if stream.dict.get(b"Filter").is_err() {
                        stream.content.clone()
                    } else {
                        stream.get_plain_content().map_err(|_| {
                            verification_error(
                                "SANITIZATION_VERIFICATION_PAYLOAD_SCAN_FAILED",
                                "Relevant output decoded stream scan tamamlanamadı",
                            )
                        })?
                    };
                    validate_payload_stream_bytes(decoded.len() as u64)?;
                    account_payload_bytes(state.decoded_bytes, decoded.len() as u64)?;
                    state
                        .observed
                        .insert(format!("{:x}", Sha256::digest(decoded)));
                }
                for (key, value) in stream.dict.iter() {
                    scan(
                        document,
                        value,
                        state,
                        depth + 1,
                        inherited_payload_context || payload_stream_context(key),
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut observed = BTreeSet::new();
    let mut visited = HashSet::new();
    let mut decoded_bytes = 0u64;
    let mut nodes = 0usize;
    let mut state = PayloadScanState {
        observed: &mut observed,
        visited: &mut visited,
        decoded_bytes: &mut decoded_bytes,
        nodes: &mut nodes,
    };
    scan(
        document,
        &Object::Dictionary(document.trailer.clone()),
        &mut state,
        0,
        false,
    )?;
    for (id, object) in &document.objects {
        if state.visited.insert(*id) {
            scan(document, object, &mut state, 0, false)?;
        }
    }
    Ok(observed)
}

fn control_results(
    policy: &SanitizationPolicy,
    transform: &SanitizationTransformReport,
    inspection: &crate::sanitization::SanitizationInspectionReport,
    fingerprints: &BTreeMap<String, BTreeSet<String>>,
    observed: &BTreeSet<String>,
) -> Vec<SanitizationControlVerification> {
    policy
        .controls
        .iter()
        .map(|control_id| {
            let before = transform
                .controls
                .iter()
                .find(|control| &control.control_id == control_id)
                .is_some_and(|control| control.detected_before);
            let forbidden_findings_after = inspection
                .controls
                .iter()
                .find(|control| &control.control_id == control_id)
                .map(|control| control.finding_count)
                .unwrap_or(u32::MAX);
            let expected = fingerprints.get(control_id).cloned().unwrap_or_default();
            let remaining = expected.intersection(observed).count() as u32;
            SanitizationControlVerification {
                control_id: control_id.clone(),
                requested: true,
                detected_before: before,
                forbidden_findings_after,
                payload_fingerprints_checked: expected.len() as u32,
                payload_fingerprints_remaining: remaining,
                status: if forbidden_findings_after == 0 && remaining == 0 {
                    SanitizationVerificationStatus::Passed
                } else {
                    SanitizationVerificationStatus::Failed
                },
                warnings: Vec::new(),
            }
        })
        .collect()
}

fn compare_page_geometry(
    expected: &SanitizationDocumentInvariants,
    actual: &SanitizationDocumentInvariants,
) -> Result<(), SessionJobOperationError> {
    if expected.pages.len() != actual.pages.len() {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_PAGE_COUNT_MISMATCH",
            "Output page count expected invariant ile eşleşmiyor",
        ));
    }
    for (expected, actual) in expected.pages.iter().zip(&actual.pages) {
        if expected.page_index != actual.page_index
            || (expected.width_points - actual.width_points).abs() > 0.01
            || (expected.height_points - actual.height_points).abs() > 0.01
            || expected.rotation_degrees != actual.rotation_degrees
        {
            return Err(verification_error(
                "SANITIZATION_VERIFICATION_PAGE_DIMENSION_MISMATCH",
                "Output page geometry expected invariant ile eşleşmiyor",
            ));
        }
    }
    Ok(())
}

fn compare_preservation(
    policy: &SanitizationPolicy,
    expected: &SanitizationDocumentInvariants,
    actual: &SanitizationDocumentInvariants,
) -> Result<(), SessionJobOperationError> {
    if expected.named_destination_count != actual.named_destination_count
        || expected.form_field_count != actual.form_field_count
        || expected.outline_root_present != actual.outline_root_present
        || expected
            .pages
            .iter()
            .zip(&actual.pages)
            .any(|(left, right)| left.content_sha256 != right.content_sha256)
    {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_PRESERVATION_FAILED",
            "Unrequested structural preservation invariantı eşleşmiyor",
        ));
    }
    let origin_actions_requested = policy.controls.iter().any(|control| {
        matches!(
            control.as_str(),
            "additional_actions" | "annotation_actions" | "form_actions"
        )
    });
    if !origin_actions_requested
        && expected.local_goto_action_count != actual.local_goto_action_count
    {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_PRESERVATION_FAILED",
            "Unrequested local GoTo action preservation invariantı eşleşmiyor",
        ));
    }
    Ok(())
}

fn check_from_result(
    result: Result<(), SessionJobOperationError>,
) -> SanitizationVerificationCheck {
    match result {
        Ok(()) => passed(),
        Err(error) => failed(error.to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_sanitized_pdf(
    job_dir: &Path,
    content_generation: u64,
    policy: SanitizationPolicy,
    expected_input_digest: &str,
    expected_transform_digest: &str,
    transform_report: SanitizationTransformReport,
    output_name: &str,
    #[cfg(feature = "sanitization-test-support")]
    test_behavior: SanitizationVerificationTestBehavior,
) -> Result<SanitizationVerificationReport, SessionJobOperationError> {
    validate_sanitization_policy(&policy)?;
    validate_transform_report_for_policy(
        &transform_report,
        &policy,
        expected_input_digest,
        &transform_report.inspection_digest,
    )?;
    if output_name != SANITIZATION_TRANSFORM_OUTPUT_NAME
        || transform_report.output_relative_name != SANITIZATION_TRANSFORM_OUTPUT_NAME
        || transform_report.content_generation != content_generation
        || transform_report.input_digest != expected_input_digest
        || transform_report.output_digest != expected_transform_digest
    {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_REPORT_MISMATCH",
            "Verifier request transform authority ile eşleşmiyor",
        ));
    }
    let artifact = canonical_artifact(job_dir)?;
    let initial_digest = crate::document_io::sha256_file(&artifact).map_err(|_| {
        verification_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH",
            "Verifier artifact digest hesaplayamadı",
        )
    })?;
    if initial_digest != expected_transform_digest {
        return Err(verification_error(
            "SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH",
            "Verifier artifact digest transform authority ile eşleşmiyor",
        ));
    }

    #[cfg(feature = "sanitization-test-support")]
    match test_behavior {
        SanitizationVerificationTestBehavior::CrashDuringTraversal => {
            let _ = fs::write(
                job_dir.join("sanitization-verification-test-phase"),
                b"traversal",
            );
            std::process::abort()
        }
        SanitizationVerificationTestBehavior::HangDuringTraversal => loop {
            let _ = fs::write(
                job_dir.join("sanitization-verification-test-phase"),
                b"traversal",
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        },
        _ => {}
    }

    let bytes = fs::read(&artifact).map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_PARSER_REOPEN_FAILED",
            "Verifier private artifact bytes okuyamadı",
        )
    })?;
    if bytes.len() as u64 > MAX_SANITIZATION_RAW_SCAN_BYTES {
        return Err(verification_error(
            "SANITIZATION_VERIFICATION_LIMIT_EXCEEDED",
            "Verifier raw scan byte limiti aşıldı",
        ));
    }
    let parsed = Document::load(&artifact);
    let parser_reopen = if parsed.is_ok() {
        passed()
    } else {
        failed("Fresh parser private artifact'ı yeniden açamadı")
    };

    let mut full_rewrite = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut incremental_history_removed =
        skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut object_graph_scan = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut removed_payload_scan = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut page_tree_valid = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut page_count_valid = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut page_dimensions_valid = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut preservation_checks = skipped("Parser reopen başarısız olduğu için çalıştırılmadı");
    let mut controls = policy
        .controls
        .iter()
        .map(|control_id| SanitizationControlVerification {
            control_id: control_id.clone(),
            requested: true,
            detected_before: false,
            forbidden_findings_after: u32::MAX,
            payload_fingerprints_checked: 0,
            payload_fingerprints_remaining: u32::MAX,
            status: SanitizationVerificationStatus::SkippedWithReason,
            warnings: vec![safe_warning(
                "SANITIZATION_VERIFICATION_NOT_RUN",
                "Control verification earlier mandatory failure nedeniyle çalıştırılmadı.",
            )],
        })
        .collect::<Vec<_>>();
    let mut unresolved_findings = Vec::new();

    if let Ok(document) = &parsed {
        full_rewrite = check_from_result(verify_full_rewrite(document, &bytes));
        page_tree_valid = check_from_result(validate_all_references(document));
        let observed_invariants = observe_document_invariants(document);
        match observed_invariants {
            Ok(actual) => {
                page_count_valid = if actual.page_count
                    == transform_report.expected_document_invariants.page_count
                {
                    passed()
                } else {
                    failed("Output page count expected input count ile eşleşmiyor")
                };
                page_dimensions_valid = check_from_result(compare_page_geometry(
                    &transform_report.expected_document_invariants,
                    &actual,
                ));
                preservation_checks = check_from_result(compare_preservation(
                    &policy,
                    &transform_report.expected_document_invariants,
                    &actual,
                ));
            }
            Err(error) => {
                page_tree_valid = failed(error.to_string());
            }
        }
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            &artifact,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        );
        match inspection {
            Ok(inspection) => {
                object_graph_scan = passed();
                incremental_history_removed = inspection
                    .controls
                    .iter()
                    .find(|control| control.control_id == "incremental_revisions")
                    .filter(|control| control.finding_count == 0)
                    .map(|_| passed())
                    .unwrap_or_else(|| failed("Output incremental history finding içeriyor"));
                match (
                    payload_fingerprints(&transform_report),
                    observed_payload_digests(document),
                ) {
                    (Ok(fingerprints), Ok(observed)) => {
                        controls = control_results(
                            &policy,
                            &transform_report,
                            &inspection,
                            &fingerprints,
                            &observed,
                        );
                        let remaining = controls
                            .iter()
                            .map(|control| control.payload_fingerprints_remaining as usize)
                            .sum::<usize>();
                        removed_payload_scan = if remaining == 0 {
                            passed()
                        } else {
                            failed("Removed payload fingerprint output object graph'inde bulundu")
                        };
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        removed_payload_scan = failed(error.to_string());
                    }
                }
                for control in &controls {
                    if control.forbidden_findings_after > 0 {
                        unresolved_findings.push(SanitizationVerificationFinding {
                            code: "SANITIZATION_FORBIDDEN_FINDING_REMAINS".to_string(),
                            control_id: Some(control.control_id.clone()),
                            object_id: None,
                            logical_path: None,
                            mandatory: true,
                        });
                    }
                    if control.payload_fingerprints_remaining > 0 {
                        unresolved_findings.push(SanitizationVerificationFinding {
                            code: "SANITIZATION_REMOVED_PAYLOAD_REMAINS".to_string(),
                            control_id: Some(control.control_id.clone()),
                            object_id: None,
                            logical_path: None,
                            mandatory: true,
                        });
                    }
                }
                for finding in crate::sanitization::unresolved_blocking_findings(
                    &inspection,
                    &policy,
                    policy.acknowledgements.acknowledge_signature_invalidation
                        && transform_report.signatures_invalidated,
                )? {
                    unresolved_findings.push(SanitizationVerificationFinding {
                        code: "SANITIZATION_POST_TRANSFORM_BLOCKING_FINDING".to_string(),
                        control_id: Some(finding.control_id),
                        object_id: None,
                        logical_path: None,
                        mandatory: true,
                    });
                }
            }
            Err(error) => {
                object_graph_scan = failed(error.to_string());
            }
        }
    }

    #[cfg(feature = "sanitization-test-support")]
    let force_pdfium_failure = matches!(
        test_behavior,
        SanitizationVerificationTestBehavior::FailPdfiumReopen
    );
    #[cfg(not(feature = "sanitization-test-support"))]
    let force_pdfium_failure = false;
    let pdfium = if force_pdfium_failure {
        Err("deterministic test failure".to_string())
    } else {
        crate::pdf_engine::get_pdfium()
    };
    let pdfium_document = match &pdfium {
        Ok(pdfium) => crate::pdf_engine::open_pdfium_doc(pdfium, &artifact.to_string_lossy(), None)
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.clone()),
    };
    let pdfium_reopen = if pdfium_document.is_ok() {
        passed()
    } else {
        failed("PDFium private artifact'ı yeniden açamadı")
    };
    #[cfg(feature = "sanitization-test-support")]
    let skip_render = matches!(
        test_behavior,
        SanitizationVerificationTestBehavior::SkipMandatoryRender
    );
    #[cfg(not(feature = "sanitization-test-support"))]
    let skip_render = false;
    let render_smoke = if skip_render {
        skipped("Deterministic mandatory skipped test seam")
    } else if let Ok(document) = &pdfium_document {
        let page_count = usize::from(document.pages().len());
        let result = (|| {
            if page_count == 0 {
                return Err(verification_error(
                    "SANITIZATION_VERIFICATION_RENDER_FAILED",
                    "PDFium output page tree boştur",
                ));
            }
            validate_verification_page_count(page_count)?;
            let mut samples = BTreeSet::from([0usize, page_count - 1]);
            if page_count > 2 {
                samples.insert(page_count / 2);
            }
            for page_index in samples {
                let page = document.pages().get(page_index as u16).map_err(|_| {
                    verification_error(
                        "SANITIZATION_VERIFICATION_RENDER_FAILED",
                        "PDFium sampled page'i açamadı",
                    )
                })?;
                if !page.width().value.is_finite()
                    || !page.height().value.is_finite()
                    || page.width().value <= 0.0
                    || page.height().value <= 0.0
                {
                    return Err(verification_error(
                        "SANITIZATION_VERIFICATION_RENDER_FAILED",
                        "PDFium sampled page geometry geçersiz",
                    ));
                }
                page.render_with_config(&PdfRenderConfig::new().set_target_width(64))
                    .map_err(|_| {
                        verification_error(
                            "SANITIZATION_VERIFICATION_RENDER_FAILED",
                            "PDFium bounded render smoke başarısız",
                        )
                    })?;
            }
            Ok(())
        })();
        check_from_result(result)
    } else {
        skipped("PDFium reopen başarısız olduğu için render çalıştırılmadı")
    };

    let final_digest = crate::document_io::sha256_file(&artifact).map_err(|_| {
        verification_error(
            "SANITIZATION_VERIFICATION_ARTIFACT_INVALID",
            "Final artifact digest hesaplanamadı",
        )
    })?;
    let artifact_unchanged = if final_digest == initial_digest {
        passed()
    } else {
        failed("Private artifact verification sırasında değişti")
    };
    let mandatory = [
        &full_rewrite,
        &incremental_history_removed,
        &object_graph_scan,
        &removed_payload_scan,
        &parser_reopen,
        &pdfium_reopen,
        &page_tree_valid,
        &page_count_valid,
        &page_dimensions_valid,
        &preservation_checks,
        &render_smoke,
        &artifact_unchanged,
    ];
    let mut blockers = Vec::new();
    if unresolved_findings.iter().any(|finding| {
        finding.mandatory && finding.code == "SANITIZATION_POST_TRANSFORM_BLOCKING_FINDING"
    }) {
        blockers.push(safe_warning(
            "SANITIZATION_POST_TRANSFORM_BLOCKING_CONTENT",
            "Fresh output inspection blocking content tespit etti.",
        ));
    }
    if mandatory
        .iter()
        .any(|check| check.status != SanitizationVerificationStatus::Passed)
        || controls
            .iter()
            .any(|control| control.status != SanitizationVerificationStatus::Passed)
    {
        blockers.push(safe_warning(
            "SANITIZATION_VERIFICATION_MANDATORY_CHECK_FAILED",
            "Bir veya daha fazla mandatory independent verification check geçmedi.",
        ));
    }
    let is_verified = blockers.is_empty()
        && unresolved_findings.iter().all(|finding| !finding.mandatory)
        && mandatory
            .iter()
            .all(|check| check.status == SanitizationVerificationStatus::Passed)
        && controls
            .iter()
            .all(|control| control.status == SanitizationVerificationStatus::Passed);
    let report = SanitizationVerificationReport {
        schema_version: SANITIZATION_VERIFICATION_REPORT_SCHEMA_VERSION.to_string(),
        policy_schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
        control_registry_version: SANITIZATION_CONTROL_REGISTRY_VERSION.to_string(),
        verification_version: SANITIZATION_VERIFICATION_VERSION.to_string(),
        inspection_version: SANITIZATION_INSPECTION_VERSION.to_string(),
        input_digest: expected_input_digest.to_string(),
        transform_digest: final_digest,
        profile: match policy.profile {
            SanitizationProfile::VerifiedLimitedV1 => SanitizationProfile::VerifiedLimitedV1,
            SanitizationProfile::CustomSupportedV1 => SanitizationProfile::CustomSupportedV1,
        },
        controls,
        full_rewrite,
        incremental_history_removed,
        object_graph_scan,
        removed_payload_scan,
        parser_reopen,
        pdfium_reopen,
        page_tree_valid,
        page_count_valid,
        page_dimensions_valid,
        preservation_checks,
        render_smoke,
        artifact_unchanged,
        unresolved_findings,
        blockers,
        warnings: Vec::new(),
        is_verified,
    };
    validate_verification_report(&report)?;
    Ok(report)
}

#[cfg(all(
    test,
    feature = "sanitization-test-support",
    target_os = "macos",
    target_arch = "aarch64"
))]
mod tests {
    use super::*;
    use crate::sanitization::{
        inspection_digest, validate_verification_report_for_transform,
        SanitizationAcknowledgements, SanitizationProfile, SANITIZATION_TRANSFORM_OUTPUT_NAME,
    };
    use lopdf::{dictionary, Stream};
    use std::io::Write;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sanitization")
            .join(name)
    }

    fn policy(controls: &[&str]) -> SanitizationPolicy {
        policy_with_signature_acknowledgement(controls, false)
    }

    fn policy_with_signature_acknowledgement(
        controls: &[&str],
        acknowledge_signature_invalidation: bool,
    ) -> SanitizationPolicy {
        let mut controls: Vec<String> = controls.iter().map(|value| (*value).to_string()).collect();
        if !controls
            .iter()
            .any(|value| value == "incremental_revisions")
        {
            controls.push("incremental_revisions".to_string());
        }
        SanitizationPolicy {
            schema_version: SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            profile: SanitizationProfile::CustomSupportedV1,
            controls,
            acknowledgements: SanitizationAcknowledgements {
                acknowledge_signature_invalidation,
            },
            expected_inspection_version: SANITIZATION_INSPECTION_VERSION.to_string(),
        }
    }

    fn transform(
        source: &Path,
        job_dir: &Path,
        policy: &SanitizationPolicy,
    ) -> SanitizationTransformReport {
        let inspection = crate::sanitization_inspection::inspect_snapshot(
            source,
            None,
            SANITIZATION_REPORT_SCHEMA_VERSION,
            SANITIZATION_POLICY_SCHEMA_VERSION,
            SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let inspection_digest = inspection_digest(&inspection).unwrap();
        crate::sanitization_transform::execute_transform(
            crate::sanitization_transform::SanitizationTransformExecutionArgs {
                job_dir,
                snapshot_path: source,
                credential: None,
                content_generation: 1,
                policy: policy.clone(),
                expected_input_digest: &inspection.input_digest,
                expected_inspection_digest: &inspection_digest,
                output_name: SANITIZATION_TRANSFORM_OUTPUT_NAME,
                #[cfg(feature = "sanitization-test-support")]
                test_behavior:
                    crate::sanitization_transform::SanitizationTransformTestBehavior::Real,
            },
        )
        .unwrap()
    }

    fn rebind(report: &mut SanitizationTransformReport, artifact: &Path) {
        report.output_size_bytes = fs::metadata(artifact).unwrap().len();
        report.output_digest = crate::document_io::sha256_file(artifact).unwrap();
    }

    fn verify(
        job_dir: &Path,
        policy: &SanitizationPolicy,
        report: &SanitizationTransformReport,
    ) -> Result<SanitizationVerificationReport, SessionJobOperationError> {
        verify_sanitized_pdf(
            job_dir,
            1,
            policy.clone(),
            &report.input_digest,
            &report.output_digest,
            report.clone(),
            SANITIZATION_TRANSFORM_OUTPUT_NAME,
            SanitizationVerificationTestBehavior::Real,
        )
    }

    #[test]
    fn fake_transform_success_with_reintroduced_javascript_and_fingerprint_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.pdf");
        let mut input = Document::load(fixture("clean-minimal.pdf")).unwrap();
        let action = input.add_object(dictionary! {
            "S" => "JavaScript",
            "JS" => Object::string_literal("PRIVATE_SANITIZATION_JS_PAYLOAD"),
        });
        input
            .catalog_mut()
            .unwrap()
            .set("OpenAction", Object::Reference(action));
        input.save(&source).unwrap();
        let policy = policy(&["document_javascript"]);
        let mut report = transform(&source, temp.path(), &policy);
        let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let expected_fingerprint = report
            .controls
            .iter()
            .find(|control| control.control_id == "document_javascript")
            .unwrap()
            .evidence
            .iter()
            .find_map(|evidence| evidence.payload_sha256.clone())
            .unwrap();
        let mut corrupted = Document::load(&artifact).unwrap();
        let action = corrupted.add_object(dictionary! {
            "S" => "JavaScript",
            "JS" => Object::string_literal("PRIVATE_SANITIZATION_JS_PAYLOAD"),
        });
        corrupted
            .catalog_mut()
            .unwrap()
            .set("OpenAction", Object::Reference(action));
        corrupted.save(&artifact).unwrap();
        rebind(&mut report, &artifact);

        let verification = verify(temp.path(), &policy, &report).unwrap();
        assert!(!verification.is_verified);
        let javascript = verification
            .controls
            .iter()
            .find(|control| control.control_id == "document_javascript")
            .unwrap();
        assert!(javascript.forbidden_findings_after > 0);
        assert!(javascript.payload_fingerprints_remaining > 0);
        assert_eq!(
            expected_fingerprint,
            format!("{:x}", Sha256::digest(b"PRIVATE_SANITIZATION_JS_PAYLOAD"))
        );
    }

    #[test]
    fn wrong_digest_is_rejected_before_independent_parse() {
        let temp = tempfile::tempdir().unwrap();
        let policy = policy(&[]);
        let report = transform(&fixture("clean-minimal.pdf"), temp.path(), &policy);
        let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&artifact)
            .unwrap()
            .write_all(b"tamper")
            .unwrap();
        let input_digest = report.input_digest.clone();
        let output_digest = report.output_digest.clone();
        let error = verify_sanitized_pdf(
            temp.path(),
            1,
            policy,
            &input_digest,
            &output_digest,
            report,
            SANITIZATION_TRANSFORM_OUTPUT_NAME,
            SanitizationVerificationTestBehavior::Real,
        )
        .unwrap_err();
        match error {
            SessionJobOperationError::Failed(error) => assert_eq!(
                error.code,
                "SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH"
            ),
            other => panic!("expected typed digest failure: {other:?}"),
        }
    }

    #[test]
    fn old_revision_marker_and_dangling_reference_are_independently_rejected() {
        for corruption in ["old_revision", "dangling_reference"] {
            let temp = tempfile::tempdir().unwrap();
            let policy = policy(&[]);
            let mut report = transform(&fixture("clean-minimal.pdf"), temp.path(), &policy);
            let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
            if corruption == "old_revision" {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&artifact)
                    .unwrap()
                    .write_all(b"\n%PDF-1.4\n% PRIVATE_OLD_REVISION_MARKER\n")
                    .unwrap();
            } else {
                let mut document = Document::load(&artifact).unwrap();
                document
                    .catalog_mut()
                    .unwrap()
                    .set("VerifierDangling", Object::Reference((65_000, 0)));
                document.save(&artifact).unwrap();
            }
            rebind(&mut report, &artifact);
            let verification = verify(temp.path(), &policy, &report).unwrap();
            assert!(!verification.is_verified);
            assert!(
                verification.full_rewrite.status == SanitizationVerificationStatus::Failed
                    || verification.page_tree_valid.status
                        == SanitizationVerificationStatus::Failed
            );
        }
    }

    #[test]
    fn reintroduced_embedded_stream_is_found_even_when_transform_claims_success() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture("embedded-files-af.pdf");
        let policy = policy(&[
            "embedded_files",
            "associated_files",
            "file_attachment_annotations",
        ]);
        let mut report = transform(&source, temp.path(), &policy);
        let source_document = Document::load(&source).unwrap();
        let removed_payload = source_document
            .objects
            .values()
            .find_map(|object| match object {
                Object::Stream(stream)
                    if dictionary_name(&stream.dict, b"Type").as_deref()
                        == Some(b"EmbeddedFile") =>
                {
                    Some(stream.get_plain_content().unwrap())
                }
                _ => None,
            })
            .unwrap();
        let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let mut corrupted = Document::load(&artifact).unwrap();
        corrupted.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile" },
            removed_payload,
        ));
        corrupted.save(&artifact).unwrap();
        rebind(&mut report, &artifact);
        let verification = verify(temp.path(), &policy, &report).unwrap();
        assert!(!verification.is_verified);
        let embedded = verification
            .controls
            .iter()
            .find(|control| control.control_id == "embedded_files")
            .unwrap();
        assert!(embedded.payload_fingerprints_remaining > 0);
        assert_eq!(
            verification.full_rewrite.status,
            SanitizationVerificationStatus::Failed
        );
    }

    #[test]
    fn fresh_xfa_and_optional_content_blockers_cannot_be_ignored() {
        for (control_id, key) in [("xfa", "AcroForm"), ("optional_content", "OCProperties")] {
            let temp = tempfile::tempdir().unwrap();
            let policy = policy(&[]);
            let mut report = transform(&fixture("clean-minimal.pdf"), temp.path(), &policy);
            let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
            let mut document = Document::load(&artifact).unwrap();
            document.catalog_mut().unwrap().set(
                key,
                Object::Dictionary(if key == "AcroForm" {
                    dictionary! { "XFA" => Object::string_literal("post-transform-xfa") }
                } else {
                    dictionary! { "OCGs" => Vec::<Object>::new() }
                }),
            );
            document.save(&artifact).unwrap();
            rebind(&mut report, &artifact);

            let verification = verify(temp.path(), &policy, &report).unwrap();
            assert!(!verification.is_verified, "control={control_id}");
            assert!(verification
                .blockers
                .iter()
                .any(|blocker| blocker.code == "SANITIZATION_POST_TRANSFORM_BLOCKING_CONTENT"));
            assert!(verification.unresolved_findings.iter().any(|finding| {
                finding.mandatory
                    && finding.code == "SANITIZATION_POST_TRANSFORM_BLOCKING_FINDING"
                    && finding.control_id.as_deref() == Some(control_id)
            }));
        }
    }

    #[test]
    fn acknowledged_signature_invalidation_remains_a_valid_transform_path() {
        let temp = tempfile::tempdir().unwrap();
        let policy = policy_with_signature_acknowledgement(&[], true);
        let report = transform(&fixture("signature.pdf"), temp.path(), &policy);
        assert!(report.signatures_invalidated);
        let verification = verify(temp.path(), &policy, &report).unwrap();
        assert!(verification.is_verified, "{verification:?}");
        assert!(verification.blockers.is_empty());
    }

    #[test]
    fn undecodable_reachable_stream_fails_the_decoded_payload_scan() {
        let temp = tempfile::tempdir().unwrap();
        let policy = policy(&[]);
        let mut report = transform(&fixture("clean-minimal.pdf"), temp.path(), &policy);
        let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let mut document = Document::load(&artifact).unwrap();
        let bad_stream = document.add_object(Stream::new(
            dictionary! { "Filter" => "RunLengthDecode" },
            b"not-a-deflate-stream".to_vec(),
        ));
        document
            .catalog_mut()
            .unwrap()
            .set("Metadata", Object::Reference(bad_stream));
        document.save(&artifact).unwrap();
        rebind(&mut report, &artifact);

        let verification = verify(temp.path(), &policy, &report).unwrap();
        assert_eq!(
            verification.parser_reopen.status,
            SanitizationVerificationStatus::Passed
        );
        assert_eq!(
            verification.removed_payload_scan.status,
            SanitizationVerificationStatus::Failed
        );
        assert!(!verification.is_verified);
    }

    #[test]
    fn page_count_mismatch_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let policy = policy(&[]);
        let mut report = transform(&fixture("clean-minimal.pdf"), temp.path(), &policy);
        let artifact = temp.path().join(SANITIZATION_TRANSFORM_OUTPUT_NAME);
        let mut document = Document::load(&artifact).unwrap();
        let page_id = *document.get_pages().get(&1).unwrap();
        let pages_id = document
            .catalog()
            .unwrap()
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        let pages = document.get_dictionary_mut(pages_id).unwrap();
        pages
            .get_mut(b"Kids")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(Object::Reference(page_id));
        pages.set("Count", 2);
        document.save(&artifact).unwrap();
        rebind(&mut report, &artifact);
        let verification = verify(temp.path(), &policy, &report).unwrap();
        assert!(!verification.is_verified);
        assert_eq!(
            verification.page_count_valid.status,
            SanitizationVerificationStatus::Failed
        );
    }

    #[test]
    fn malformed_and_mismatched_verification_reports_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let policy = policy(&[]);
        let transform = transform(&fixture("clean-minimal.pdf"), temp.path(), &policy);
        let valid = verify(temp.path(), &policy, &transform).unwrap();
        assert!(valid.is_verified);

        let mut candidates = Vec::new();
        let mut wrong_schema = valid.clone();
        wrong_schema.schema_version = "sanitization-verification/0".to_string();
        candidates.push(wrong_schema);
        let mut wrong_verifier = valid.clone();
        wrong_verifier.verification_version = "sanitization-verifier-v0".to_string();
        candidates.push(wrong_verifier);
        let mut wrong_policy = valid.clone();
        wrong_policy.policy_schema_version = "sanitization-policy-v0".to_string();
        candidates.push(wrong_policy);
        let mut missing_control = valid.clone();
        missing_control.controls.clear();
        candidates.push(missing_control);
        let mut duplicate_control = valid.clone();
        duplicate_control.controls.push(valid.controls[0].clone());
        candidates.push(duplicate_control);
        let mut unknown_control = valid.clone();
        unknown_control.controls[0].control_id = "unknown_control".to_string();
        candidates.push(unknown_control);
        let mut true_with_failed = valid.clone();
        true_with_failed.full_rewrite = failed("deterministic failed mandatory check");
        candidates.push(true_with_failed);
        let mut true_with_blocker = valid.clone();
        true_with_blocker.blockers.push(safe_warning(
            "SANITIZATION_TEST_BLOCKER",
            "Deterministic structural blocker.",
        ));
        candidates.push(true_with_blocker);
        for candidate in candidates {
            assert!(validate_verification_report(&candidate).is_err());
        }

        let mut wrong_profile = valid.clone();
        wrong_profile.profile = SanitizationProfile::VerifiedLimitedV1;
        assert!(
            validate_verification_report_for_transform(&wrong_profile, &transform, &policy)
                .is_err()
        );
        let mut wrong_input_digest = valid.clone();
        wrong_input_digest.input_digest = "0".repeat(64);
        assert!(validate_verification_report_for_transform(
            &wrong_input_digest,
            &transform,
            &policy
        )
        .is_err());
        let mut wrong_transform_digest = valid;
        wrong_transform_digest.transform_digest = "0".repeat(64);
        assert!(validate_verification_report_for_transform(
            &wrong_transform_digest,
            &transform,
            &policy
        )
        .is_err());
    }

    #[test]
    fn representative_named_destination_goto_form_outline_and_page_content_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("preservation-source.pdf");
        let mut document = Document::load(fixture("clean-minimal.pdf")).unwrap();
        let page_id = *document.get_pages().get(&1).unwrap();
        let javascript = document.add_object(dictionary! {
            "S" => "JavaScript",
            "JS" => Object::string_literal("REMOVE_ONLY_THIS_SCRIPT"),
        });
        document
            .get_dictionary_mut(page_id)
            .unwrap()
            .set("A", Object::Reference(javascript));
        let local_goto = document.add_object(dictionary! {
            "S" => "GoTo",
            "D" => vec![Object::Reference(page_id), Object::Name(b"Fit".to_vec())],
        });
        let field = document.add_object(dictionary! {
            "FT" => "Tx",
            "T" => Object::string_literal("preserved-field"),
        });
        let acro_form = document.add_object(dictionary! {
            "Fields" => vec![Object::Reference(field)],
        });
        let destinations = document.add_object(dictionary! {
            "Names" => vec![
                Object::string_literal("preserved-destination"),
                Object::Array(vec![Object::Reference(page_id), Object::Name(b"Fit".to_vec())]),
            ],
        });
        let names = document.add_object(dictionary! {
            "Dests" => Object::Reference(destinations),
        });
        let outlines = document.add_object(dictionary! {
            "Type" => "Outlines",
            "Count" => 0,
        });
        let catalog = document.catalog_mut().unwrap();
        catalog.set("OpenAction", Object::Reference(local_goto));
        catalog.set("AcroForm", Object::Reference(acro_form));
        catalog.set("Names", Object::Reference(names));
        catalog.set("Outlines", Object::Reference(outlines));
        document.save(&source).unwrap();

        let policy = policy(&["document_javascript"]);
        let report = transform(&source, temp.path(), &policy);
        assert_eq!(
            report.expected_document_invariants.named_destination_count,
            1
        );
        assert_eq!(
            report.expected_document_invariants.local_goto_action_count,
            1
        );
        assert_eq!(report.expected_document_invariants.form_field_count, 1);
        assert!(report.expected_document_invariants.outline_root_present);
        let verification = verify(temp.path(), &policy, &report).unwrap();
        assert!(verification.is_verified);
        assert_eq!(
            verification.preservation_checks.status,
            SanitizationVerificationStatus::Passed
        );
    }

    #[test]
    fn incremental_object_stream_and_xref_stream_inputs_verify_as_single_full_rewrites() {
        for fixture_name in [
            "incremental-revision.pdf",
            "object-xref-stream.pdf",
            "incremental-xref-stream-chain.pdf",
            "hybrid-xref.pdf",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let policy = policy(&[]);
            let transform = transform(&fixture(fixture_name), temp.path(), &policy);
            let verification = verify(temp.path(), &policy, &transform).unwrap();
            assert!(
                verification.is_verified,
                "fixture={fixture_name} report={verification:?}"
            );
            assert_eq!(
                verification.full_rewrite.status,
                SanitizationVerificationStatus::Passed,
                "fixture={fixture_name}"
            );
            assert_eq!(
                verification.incremental_history_removed.status,
                SanitizationVerificationStatus::Passed,
                "fixture={fixture_name}"
            );
            assert_eq!(
                verification.parser_reopen.status,
                SanitizationVerificationStatus::Passed,
                "fixture={fixture_name}"
            );
            assert_eq!(
                verification.pdfium_reopen.status,
                SanitizationVerificationStatus::Passed,
                "fixture={fixture_name}"
            );
        }
    }

    #[test]
    fn verifier_payload_budget_accepts_exact_boundary_and_rejects_plus_one_and_overflow() {
        validate_payload_stream_bytes(MAX_SANITIZATION_DECODED_STREAM_BYTES).unwrap();
        assert!(validate_payload_stream_bytes(MAX_SANITIZATION_DECODED_STREAM_BYTES + 1).is_err());
        let mut total = 0;
        for _ in 0..(MAX_SANITIZATION_TOTAL_DECODED_BYTES / MAX_SANITIZATION_DECODED_STREAM_BYTES) {
            account_payload_bytes(&mut total, MAX_SANITIZATION_DECODED_STREAM_BYTES).unwrap();
        }
        assert_eq!(total, MAX_SANITIZATION_TOTAL_DECODED_BYTES);
        assert!(account_payload_bytes(&mut total, 1).is_err());
        let mut overflow = u64::MAX;
        assert!(account_payload_bytes(&mut overflow, 1).is_err());
    }

    #[test]
    fn actual_output_page_budget_accepts_exact_boundary_and_rejects_plus_one() {
        validate_verification_page_count(MAX_SANITIZATION_VERIFICATION_PAGES).unwrap();
        assert!(validate_verification_page_count(MAX_SANITIZATION_VERIFICATION_PAGES + 1).is_err());
    }
}
