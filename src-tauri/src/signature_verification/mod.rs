//! Offline signature verification (Prompt 11 G2; pades-architecture-design.md v1.3.2).
//!
//! Answers: is each signature structurally and cryptographically sound, and what happened
//! to the document after it was signed? It makes **no trust decision**: trust and
//! revocation are always `NotChecked` (OPTION 1, §19).
//!
//! Placement (§6.1): everything that parses the PDF or CMS runs in the ephemeral job worker
//! (`JobCommand::InspectSignatures`). The main process only bound-checks the typed report
//! ([`validate_report`]); it never parses DER.

pub mod byte_range;
pub mod cms_verify;
pub mod digest;
pub mod discovery;
pub mod inspect;
pub mod limits;
pub mod report;
pub mod revisions;
pub mod structure;

pub use report::*;

use crate::document_session::{SessionJobOperationError, SessionJobOutcome};

fn invalid_report(message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code("SIGNATURE_REPORT_INVALID", message)
}

/// Main-process check of the worker's report: identity, schema and every bound. Anything
/// outside the limits means the worker misbehaved; the report is rejected, not trimmed.
pub fn validate_report(
    report: &PdfSignatureVerificationReport,
    session_id: &str,
    content_generation: u64,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != SIGNATURE_REPORT_SCHEMA_VERSION {
        return Err(invalid_report("İmza raporu şema sürümü eşleşmedi"));
    }
    if report.session_id != session_id || report.content_generation != content_generation {
        return Err(invalid_report(
            "İmza raporu başka bir oturuma ya da sürüme ait",
        ));
    }
    if report.signatures.len() > limits::MAX_SIGNATURES
        || report.global_findings.len() > limits::MAX_GLOBAL_FINDINGS
        || report.revisions.revision_ends.len() > limits::MAX_REVISIONS
        || report.total_signatures as usize != report.signatures.len()
    {
        return Err(invalid_report("İmza raporu sınırların dışında"));
    }
    let hex_ok = |value: &str, length: usize| {
        value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    if !hex_ok(&report.document_digest, 64) {
        return Err(invalid_report("İmza raporunda belge özeti geçersiz"));
    }
    let short = |value: &Option<String>, max: usize| {
        value
            .as_ref()
            .is_none_or(|value| value.chars().count() <= max + 1)
    };
    let findings_ok = |findings: &[ValidationFinding]| {
        findings.iter().all(|finding| {
            finding.code.len() <= 64
                && finding.message.chars().count() <= limits::MAX_CONTEXT_CHARS
                && short(&finding.context, limits::MAX_CONTEXT_CHARS)
        })
    };
    if !findings_ok(&report.global_findings) {
        return Err(invalid_report("İmza raporu bulguları sınırların dışında"));
    }
    for signature in &report.signatures {
        let identity_ok = signature.signer_identity.as_ref().is_none_or(|identity| {
            short(&identity.common_name, limits::MAX_NAME_COMPONENT_CHARS)
                && short(&identity.organization, limits::MAX_NAME_COMPONENT_CHARS)
                && short(&identity.country, limits::MAX_NAME_COMPONENT_CHARS)
                && identity.certificate_issuer.chars().count() <= limits::MAX_CONTEXT_CHARS + 1
                && identity.certificate_serial.len() <= 128
                && hex_ok(&identity.certificate_sha256_fingerprint, 64)
        });
        if !hex_ok(&signature.signature_id, 32)
            || !hex_ok(&signature.contents_digest, 64)
            || signature.byte_range.len() > 8
            || signature.findings.len() > limits::MAX_FINDINGS_PER_SIGNATURE
            || !findings_ok(&signature.findings)
            || !short(&signature.field_name, limits::MAX_FIELD_NAME_CHARS)
            || !short(&signature.reason, limits::MAX_REASON_CHARS)
            || !short(&signature.sub_filter, limits::MAX_NAME_COMPONENT_CHARS)
            || !identity_ok
            || signature.later_revisions.len() > limits::MAX_REVISIONS
        {
            return Err(invalid_report(
                "İmza raporundaki bir imza sınırların dışında",
            ));
        }
        // OPTION 1 invariants: never a trust or validated-level claim.
        if signature.trust_status != TrustChainStatus::NotChecked
            || signature.revocation_status != RevocationStatus::NotChecked
            || signature.pades_validated_level != PadesLevel::None
        {
            return Err(invalid_report(
                "İmza raporu kapsam dışı bir güven iddiası taşıyor",
            ));
        }
    }
    let encoded =
        serde_json::to_vec(report).map_err(|_| invalid_report("İmza raporu kodlanamadı"))?;
    if encoded.len() > limits::MAX_REPORT_JSON_BYTES {
        return Err(invalid_report("İmza raporu boyut sınırını aşıyor"));
    }
    Ok(())
}

/// Read-only inspection of the session snapshot's signatures (CAP-01…08, 12, 13).
#[tauri::command]
pub async fn inspect_session_signatures(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<
    SessionJobOutcome<PdfSignatureVerificationReport>,
    crate::document_session::DocumentSessionError,
> {
    inspect_session_signatures_core(
        job_id,
        session_id,
        expected_content_generation,
        &store,
        std::time::Duration::from_secs(180),
    )
    .await
}

pub async fn inspect_session_signatures_core(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    store: &crate::document_session::DocumentSessionStore,
    timeout: std::time::Duration,
) -> Result<
    SessionJobOutcome<PdfSignatureVerificationReport>,
    crate::document_session::DocumentSessionError,
> {
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let generation = context.captured_content_generation;
                let owner = context.owner_session_id.clone();
                let result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::InspectSignatures {
                        schema_version: SIGNATURE_REPORT_SCHEMA_VERSION,
                        content_generation: generation,
                    },
                    timeout,
                )
                .map_err(|error| match error {
                    // A panic aborts the worker (release `panic = "abort"`): typed outcome.
                    SessionJobOperationError::Failed(failure)
                        if failure.code == "SESSION_JOB_WORKER_CRASHED" =>
                    {
                        SessionJobOperationError::failed_with_code(
                            limits::SIGNATURE_INPUT_UNSUPPORTED,
                            "İmza doğrulayıcısı bu belgede beklenmedik biçimde durdu",
                        )
                    }
                    other => other,
                })?;
                let crate::job_worker::JobResult::SignatureInspection(report) = result else {
                    return Err(invalid_report("İmza worker'ı beklenmeyen sonuç döndürdü"));
                };
                validate_report(&report, &owner, generation)?;
                Ok(report)
            },
        )
        .await
}
