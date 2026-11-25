//! Worker entry of `JobCommand::InspectSignatures` (design §6.1): every signature of the
//! immutable snapshot is checked independently. Read-only: nothing is written, the session
//! is not touched, no network is used.

use super::byte_range::{self, ByteRangeViolation, CheckedByteRange};
use super::cms_verify::{self, CmsOutcome, DigestUnavailable};
use super::digest::{self, DigestAlgorithm};
use super::discovery::{self, DiscoveredSignature, DiscoveryError, SignatureDiscovery};
use super::limits::{self, MAX_REVISIONS, MAX_TOTAL_HASHED_BYTES};
use super::report::*;
use super::revisions::{self, RevisionScanError, Revisions};
use std::fs::File;
use std::path::Path;

/// A job-level failure: `(code, message)`.
pub type InspectionFailure = (String, String);

fn failure(code: &str, message: &str) -> InspectionFailure {
    (code.to_string(), message.to_string())
}

pub fn inspect_signatures(
    snapshot: &Path,
    credential: Option<&str>,
    session_id: &str,
    content_generation: u64,
) -> Result<PdfSignatureVerificationReport, InspectionFailure> {
    let file_length = std::fs::metadata(snapshot)
        .map_err(|_| failure("SESSION_JOB_PDF_OPEN_FAILED", "Snapshot okunamadı"))?
        .len();
    let document_digest = crate::document_io::sha256_file(snapshot).map_err(|_| {
        failure(
            "SESSION_JOB_PDF_OPEN_FAILED",
            "Snapshot özeti hesaplanamadı",
        )
    })?;
    let mut file = File::open(snapshot)
        .map_err(|_| failure("SESSION_JOB_PDF_OPEN_FAILED", "Snapshot açılamadı"))?;
    let revisions =
        revisions::scan_revisions(&mut file, file_length, MAX_REVISIONS).map_err(|error| {
            match error {
                RevisionScanError::TooManyRevisions => failure(
                    limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
                    "Belgedeki revizyon sayısı sınırı aşıyor",
                ),
                RevisionScanError::Io => {
                    failure("SESSION_JOB_PDF_OPEN_FAILED", "Snapshot okunamadı")
                }
            }
        })?;
    let pdfium = crate::pdf_engine::get_pdfium()
        .map_err(|message| failure("SESSION_JOB_PDF_ENGINE_UNAVAILABLE", &message))?;
    let bindings = pdfium.bindings();
    let pdfium_signatures = snapshot
        .to_str()
        .and_then(|path| crate::incremental_save::RawDocument::open(bindings, path, credential))
        .map(|document| discovery::read_pdfium_signatures(bindings, document.handle))
        .transpose()
        .map_err(too_many)?;
    let discovery = discovery::combine(
        pdfium_signatures,
        discovery::read_structure_at(snapshot, credential),
    )
    .map_err(too_many)?;
    Ok(build_report(
        &mut file,
        file_length,
        document_digest,
        session_id,
        content_generation,
        &revisions,
        discovery,
    ))
}

fn too_many(_: DiscoveryError) -> InspectionFailure {
    failure(
        limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
        "Belgedeki imza sayısı sınırı aşıyor",
    )
}

/// Builds the report from already gathered sources. Separate from PDFium so the
/// arithmetic, CMS and classification are testable without the engine.
pub fn build_report(
    file: &mut File,
    file_length: u64,
    document_digest: String,
    session_id: &str,
    content_generation: u64,
    revisions: &Revisions,
    discovery: SignatureDiscovery,
) -> PdfSignatureVerificationReport {
    let mut global_findings = discovery.findings.clone();
    let opened = discovery.pdfium_object_count.is_some()
        || discovery.structural_status != StructuralStatus::Unavailable;
    if !opened {
        global_findings.push(ValidationFinding::new(
            "SIGNATURE_DOCUMENT_UNREADABLE",
            FindingSeverity::Error,
            "Belge açılamadı (parola ya da bozuk dosya); imzalar doğrulanmadı",
        ));
    }
    if revisions.ignored_markers > 0 {
        global_findings.push(
            ValidationFinding::new(
                "REVISION_MARKER_IGNORED",
                FindingSeverity::Info,
                "Geçerli bir startxref'e bağlanmayan %%EOF işaretleri yok sayıldı",
            )
            .with_context(format!("count={}", revisions.ignored_markers)),
        );
    }
    if revisions.trailing_bytes() > 0 {
        global_findings.push(
            ValidationFinding::new(
                "TRAILING_BYTES_AFTER_LAST_REVISION",
                FindingSeverity::Warning,
                "Son revizyondan sonra dosyada fazladan bayt var",
            )
            .with_context(format!("bytes={}", revisions.trailing_bytes())),
        );
    }

    // First pass: ByteRange and revision of every signature.
    let checked: Vec<(ByteRangeCheck, &DiscoveredSignature)> = discovery
        .signatures
        .iter()
        .map(|signature| {
            (
                check_byte_range(file, file_length, revisions, signature),
                signature,
            )
        })
        .collect();
    let revision_owners: Vec<(u32, SignatureKind)> = checked
        .iter()
        .filter_map(|(check, signature)| {
            check
                .revision_index
                .map(|index| (index, kind_of(signature)))
        })
        .collect();

    let mut budget = MAX_TOTAL_HASHED_BYTES;
    let mut signatures = Vec::with_capacity(checked.len());
    for (check, signature) in &checked {
        signatures.push(signature_report(
            file,
            revisions,
            &discovery,
            signature,
            check,
            &revision_owners,
            &mut budget,
        ));
    }
    signatures.sort_by(|left, right| {
        (
            left.revision_index.unwrap_or(u32::MAX),
            &left.byte_range,
            &left.field_name,
        )
            .cmp(&(
                right.revision_index.unwrap_or(u32::MAX),
                &right.byte_range,
                &right.field_name,
            ))
    });

    let document_modification_status = if signatures.is_empty() {
        global_findings.push(ValidationFinding::new(
            "NO_SIGNATURES",
            FindingSeverity::Info,
            "Belgede imzalı imza alanı yok",
        ));
        DocumentModificationStatus::Indeterminate
    } else if signatures
        .iter()
        .any(|signature| signature.byte_range_status != ByteRangeStatus::Valid)
    {
        DocumentModificationStatus::Indeterminate
    } else {
        let last_revision = revisions
            .ends
            .len()
            .checked_sub(1)
            .and_then(|index| u32::try_from(index).ok());
        let latest_covers_end = signatures
            .iter()
            .filter_map(|signature| signature.revision_index)
            .max()
            == last_revision;
        if latest_covers_end && revisions.trailing_bytes() == 0 {
            DocumentModificationStatus::NoModifications
        } else {
            // Whether a later change is permitted is not defined (design §2.4): not guessed.
            DocumentModificationStatus::Indeterminate
        }
    };
    let certification_permission = signatures
        .iter()
        .filter_map(|signature| signature.docmdp.as_ref())
        .find(|docmdp| docmdp.certification_signature)
        .or_else(|| {
            signatures
                .iter()
                .filter_map(|signature| signature.docmdp.as_ref())
                .next()
        })
        .and_then(|docmdp| docmdp.permission);

    let inspection_status = if !opened {
        InspectionStatus::Indeterminate
    } else if discovery.structural_status == StructuralStatus::Complete
        && discovery.pdfium_object_count.is_some()
    {
        InspectionStatus::Complete
    } else {
        InspectionStatus::Partial
    };
    global_findings.truncate(limits::MAX_GLOBAL_FINDINGS);

    PdfSignatureVerificationReport {
        schema_version: SIGNATURE_REPORT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        content_generation,
        document_digest,
        file_length,
        inspection_status,
        total_signatures: discovery.signature_count(),
        discovery: DiscoverySummary {
            signed_signature_count: discovery.signature_count()
                - discovery.document_timestamp_count(),
            document_timestamp_count: discovery.document_timestamp_count(),
            empty_signature_field_count: discovery.empty_signature_fields,
            unreferenced_signature_dictionary_count: discovery.unreferenced_signature_dictionaries,
            pdfium_signature_object_count: discovery.pdfium_object_count,
            structural_status: discovery.structural_status,
        },
        revisions: RevisionSummary {
            revision_count: u32::try_from(revisions.ends.len()).unwrap_or(u32::MAX),
            revision_ends: revisions.ends.clone(),
            trailing_bytes: revisions.trailing_bytes(),
        },
        signatures,
        document_modification_status,
        certification_permission,
        global_findings,
    }
}

fn kind_of(signature: &DiscoveredSignature) -> SignatureKind {
    if signature.is_document_timestamp {
        SignatureKind::DocumentTimestamp
    } else {
        SignatureKind::Signature
    }
}

struct ByteRangeCheck {
    range: Option<CheckedByteRange>,
    violations: Vec<ByteRangeViolation>,
    /// The `/Contents` bytes read from the ByteRange gap.
    gap_contents: Option<Vec<u8>>,
    revision_index: Option<u32>,
}

fn check_byte_range(
    file: &mut File,
    file_length: u64,
    revisions: &Revisions,
    signature: &DiscoveredSignature,
) -> ByteRangeCheck {
    let mut check = ByteRangeCheck {
        range: None,
        violations: Vec::new(),
        gap_contents: None,
        revision_index: None,
    };
    let range = match byte_range::check_arithmetic(&signature.byte_range, file_length) {
        Ok(range) => range,
        Err(violation) => {
            check.violations.push(violation);
            return check;
        }
    };
    check.range = Some(range);
    match byte_range::read_contents_gap(file, &range, limits::MAX_CONTENTS_BYTES) {
        Ok(decoded) => {
            let consistent = match &signature.contents {
                Some(reported) => {
                    decoded.starts_with(reported)
                        && decoded[reported.len()..].iter().all(|byte| *byte == 0)
                }
                None => true,
            };
            if !consistent {
                check.violations.push(ByteRangeViolation::ContentsMismatch);
            }
            check.gap_contents = Some(decoded);
        }
        Err(violation) => check.violations.push(violation),
    }
    match revisions.index_ending_at(range.covered_end()) {
        Some(index) => check.revision_index = u32::try_from(index).ok(),
        None => check.violations.push(ByteRangeViolation::NotAtRevisionEnd),
    }
    check
}

fn signature_report(
    file: &mut File,
    revisions: &Revisions,
    discovery: &SignatureDiscovery,
    signature: &DiscoveredSignature,
    check: &ByteRangeCheck,
    revision_owners: &[(u32, SignatureKind)],
    budget: &mut u64,
) -> PdfSignatureReport {
    let mut findings: Vec<ValidationFinding> = check
        .violations
        .iter()
        .map(|violation| {
            ValidationFinding::new(
                violation.code(),
                FindingSeverity::Error,
                violation.message(),
            )
        })
        .collect();
    let byte_range_status = if check.violations.is_empty() {
        ByteRangeStatus::Valid
    } else {
        ByteRangeStatus::Invalid
    };
    let kind = kind_of(signature);
    let cms_bytes: Vec<u8> = check
        .gap_contents
        .clone()
        .or_else(|| signature.contents.clone())
        .unwrap_or_default();

    let mut cms = if kind == SignatureKind::DocumentTimestamp {
        let mut outcome = not_verified_cms();
        outcome.findings.push(ValidationFinding::new(
            "DOCUMENT_TIMESTAMP_NOT_VERIFIED",
            FindingSeverity::Info,
            "Belge zaman damgası bulundu; RFC 3161 doğrulaması bu sürümde yapılmıyor",
        ));
        outcome
    } else {
        match signature.sub_filter.as_deref() {
            Some("adbe.pkcs7.detached" | "ETSI.CAdES.detached") => {
                let range = check.range;
                let mut document_digest =
                    |algorithm: DigestAlgorithm| -> Result<Vec<u8>, DigestUnavailable> {
                        let range = range.ok_or(DigestUnavailable::ByteRangeUnusable)?;
                        digest::digest_ranges(file, &range.ranges(), algorithm, budget)
                            .map_err(DigestUnavailable::Digest)
                    };
                cms_verify::verify_cms(&cms_bytes, &mut document_digest)
            }
            Some("adbe.pkcs7.sha1") => {
                let mut outcome = not_verified_cms();
                outcome.integrity = CryptographicIntegrityStatus::UnsupportedAlgorithm;
                outcome.digest_algorithm = Some("SHA-1".into());
                outcome.algorithm_policy.digest_policy = SingleAlgorithmPolicy {
                    algorithm_identifier: "SHA-1".into(),
                    policy_status: AlgorithmPolicyStatus::Disallowed,
                    normative_references: Vec::new(),
                };
                outcome.findings.push(ValidationFinding::new(
                    "SIGNATURE_ALGORITHM_FORBIDDEN",
                    FindingSeverity::Warning,
                    "adbe.pkcs7.sha1 SHA-1 kullanır; doğrulanmadı",
                ));
                outcome
            }
            other => {
                let mut outcome = not_verified_cms();
                outcome.integrity = CryptographicIntegrityStatus::UnsupportedAlgorithm;
                outcome.findings.push(
                    ValidationFinding::new(
                        "SIGNATURE_SUBFILTER_UNSUPPORTED",
                        FindingSeverity::Warning,
                        "İmzanın SubFilter değeri desteklenmiyor",
                    )
                    .with_context(other.unwrap_or("(none)")),
                );
                outcome
            }
        }
    };
    if signature.sub_filter.as_deref() == Some("ETSI.CAdES.detached") {
        cms.algorithm_policy
            .format_specifications
            .push(FormatSpecificationReference::EtsiEn319142_1Pades);
    }
    if kind == SignatureKind::DocumentTimestamp {
        cms.algorithm_policy
            .format_specifications
            .push(FormatSpecificationReference::Rfc3161Tsp);
    }
    findings.append(&mut cms.findings);

    let later_revisions: Vec<LaterRevision> = match check.revision_index {
        Some(own) => (own + 1..u32::try_from(revisions.ends.len()).unwrap_or(u32::MAX))
            .map(|index| LaterRevision {
                revision_index: index,
                kind: match revision_owners.iter().find(|(owner, _)| *owner == index) {
                    Some((_, SignatureKind::DocumentTimestamp)) => {
                        LaterRevisionKind::DocumentTimestamp
                    }
                    Some((_, SignatureKind::Signature)) => LaterRevisionKind::Signed,
                    None => LaterRevisionKind::Unsigned,
                },
            })
            .collect(),
        None => Vec::new(),
    };
    if later_revisions
        .iter()
        .any(|revision| revision.kind == LaterRevisionKind::Unsigned)
    {
        findings.push(ValidationFinding::new(
            "UNSIGNED_LATER_REVISION",
            FindingSeverity::Warning,
            "İmzadan sonra imzasız bir revizyon eklenmiş; bu imzayı kriptografik olarak geçersiz kılmaz",
        ));
    }
    let document_modification = if byte_range_status != ByteRangeStatus::Valid {
        ModificationScope::ByteRangeInvalid
    } else if later_revisions.is_empty() && revisions.trailing_bytes() == 0 {
        ModificationScope::NoLaterRevision
    } else {
        ModificationScope::LaterRevisionUnclassified
    };

    let docmdp = docmdp_report(signature, &mut findings);
    if docmdp.as_ref().and_then(|docmdp| docmdp.permission)
        == Some(CertificationPermission::NoChangesPermitted)
        && document_modification == ModificationScope::LaterRevisionUnclassified
    {
        findings.push(ValidationFinding::new(
            "DOCMDP_P1_LATER_REVISION",
            FindingSeverity::Warning,
            "Sertifika imzası değişikliğe izin vermiyor (P=1) ama sonrasında revizyon var; izin durumu sınıflandırılmadı",
        ));
    }

    let pades_detected_level = if kind == SignatureKind::Signature
        && signature.sub_filter.as_deref() == Some("ETSI.CAdES.detached")
        && cms.pades_signed_attributes
    {
        if !cms.signature_timestamp_present {
            PadesLevel::PadesBB
        } else if !discovery.dss_present {
            PadesLevel::PadesBT
        } else if later_revisions
            .iter()
            .any(|revision| revision.kind == LaterRevisionKind::DocumentTimestamp)
        {
            PadesLevel::PadesBLTA
        } else {
            PadesLevel::PadesBLT
        }
    } else {
        PadesLevel::None
    };

    let contents_digest = crate::hex_lower(DigestAlgorithm::Sha256.hash(&cms_bytes));
    let signature_id = signature_identity(
        signature.field_name.as_deref(),
        check.revision_index,
        &signature.byte_range,
        &contents_digest,
    );
    findings.truncate(limits::MAX_FINDINGS_PER_SIGNATURE);
    PdfSignatureReport {
        signature_id,
        field_name: signature.field_name.clone(),
        kind,
        revision_index: check.revision_index,
        byte_range: signature.byte_range.iter().copied().take(8).collect(),
        byte_range_status,
        contents_digest,
        sub_filter: signature.sub_filter.clone(),
        cryptographic_integrity: cms.integrity,
        digest_algorithm: cms.digest_algorithm,
        signature_algorithm: cms.signature_algorithm,
        document_modification,
        later_revisions,
        signer_identity: cms.signer_identity,
        signer_certificate: cms.signer_certificate,
        claimed_signing_time: ClaimedSigningTime {
            pdf_m: signature.pdf_m.clone(),
            cms_signing_time: cms.cms_signing_time,
        },
        signature_timestamp: match kind {
            SignatureKind::DocumentTimestamp => Some(TimestampReport {
                timestamp_type: TimestampType::DocumentTimestamp,
                timestamp_status: TimestampStatus::NotVerified,
            }),
            SignatureKind::Signature => {
                cms.signature_timestamp_present.then_some(TimestampReport {
                    timestamp_type: TimestampType::SignatureTimestamp,
                    timestamp_status: TimestampStatus::NotVerified,
                })
            }
        },
        pades_detected_level,
        pades_validated_level: PadesLevel::None,
        algorithm_policy: cms.algorithm_policy,
        revocation_status: RevocationStatus::NotChecked,
        trust_status: TrustChainStatus::NotChecked,
        docmdp,
        reason: signature.reason.clone(),
        findings,
    }
}

fn not_verified_cms() -> CmsOutcome {
    let mut outcome =
        cms_verify::verify_cms(&[], &mut |_| Err(DigestUnavailable::ByteRangeUnusable));
    outcome.integrity = CryptographicIntegrityStatus::Indeterminate;
    outcome.findings.clear();
    outcome
}

fn permission(value: i64) -> Option<CertificationPermission> {
    match value {
        1 => Some(CertificationPermission::NoChangesPermitted),
        2 => Some(CertificationPermission::FormFillAndSigningPermitted),
        3 => Some(CertificationPermission::FormFillSigningAndAnnotations),
        _ => None,
    }
}

fn docmdp_report(
    signature: &DiscoveredSignature,
    findings: &mut Vec<ValidationFinding>,
) -> Option<DocMdpReport> {
    let from_pdfium = signature
        .pdfium_docmdp
        .filter(|value| *value != 0)
        .and_then(|value| permission(i64::from(value)));
    let from_structure = signature.structural_docmdp_p.map(permission);
    if from_pdfium.is_none()
        && from_structure.is_none()
        && signature.field_mdp.is_none()
        && !signature.certification
    {
        return None;
    }
    let permission = match (from_pdfium, from_structure) {
        (Some(pdfium), Some(Some(structure))) if pdfium != structure => {
            findings.push(ValidationFinding::new(
                "DOCMDP_SOURCE_MISMATCH",
                FindingSeverity::Warning,
                "PDFium ve belge yapısı farklı DocMDP izni bildiriyor",
            ));
            None
        }
        (Some(pdfium), _) => Some(pdfium),
        (None, Some(structure)) => {
            if structure.is_none() {
                findings.push(ValidationFinding::new(
                    "DOCMDP_PERMISSION_UNKNOWN",
                    FindingSeverity::Warning,
                    "DocMDP /P değeri tanımlı değil",
                ));
            }
            structure
        }
        (None, None) => None,
    };
    Some(DocMdpReport {
        permission,
        certification_signature: signature.certification,
        field_mdp: signature.field_mdp.clone(),
    })
}

fn signature_identity(
    field_name: Option<&str>,
    revision_index: Option<u32>,
    byte_range: &[i64],
    contents_digest: &str,
) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(field_name.unwrap_or("").as_bytes());
    input.push(0);
    input.extend_from_slice(&revision_index.map_or(-1i64, i64::from).to_be_bytes());
    for value in byte_range {
        input.extend_from_slice(&value.to_be_bytes());
    }
    input.extend_from_slice(contents_digest.as_bytes());
    let digest = DigestAlgorithm::Sha256.hash(&input);
    crate::hex_lower(&digest[..16])
}
