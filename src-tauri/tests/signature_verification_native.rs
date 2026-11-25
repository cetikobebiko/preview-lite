//! NATIVE_PLATFORM: offline signature verification of the TEST ONLY G2 fixtures with the real
//! PDFium runtime (Prompt 11 G2). Expected results are known by construction
//! (`signature_support`); cryptography is not mocked.

use parking_lot::Mutex;
use preview_alternative_lib::signature_verification::inspect::inspect_signatures;
use preview_alternative_lib::signature_verification::*;
use std::path::PathBuf;

static PDFIUM_TEST_MUTEX: Mutex<()> = Mutex::new(());

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("signature")
        .join(name)
}

fn inspect_with(name: &str, credential: Option<&str>) -> PdfSignatureVerificationReport {
    let _guard = PDFIUM_TEST_MUTEX.lock();
    let path = fixture(name);
    let report = inspect_signatures(&path, credential, "session-test", 7)
        .unwrap_or_else(|(code, message)| panic!("{name}: {code} {message}"));
    validate_report(&report, "session-test", 7).unwrap_or_else(|error| panic!("{name}: {error:?}"));
    report
}

fn inspect(name: &str) -> PdfSignatureVerificationReport {
    inspect_with(name, None)
}

fn only(report: &PdfSignatureVerificationReport) -> &PdfSignatureReport {
    assert_eq!(report.signatures.len(), 1, "{report:#?}");
    &report.signatures[0]
}

fn codes(findings: &[ValidationFinding]) -> Vec<&str> {
    findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect()
}

/// OPTION 1 invariants hold for every signature of every report.
fn assert_option1(report: &PdfSignatureVerificationReport) {
    for signature in &report.signatures {
        assert_eq!(signature.trust_status, TrustChainStatus::NotChecked);
        assert_eq!(signature.revocation_status, RevocationStatus::NotChecked);
        assert_eq!(signature.pades_validated_level, PadesLevel::None);
    }
}

fn assert_valid(signature: &PdfSignatureReport, algorithm: &str, level: PadesLevel) {
    assert_eq!(
        signature.byte_range_status,
        ByteRangeStatus::Valid,
        "{signature:#?}"
    );
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Valid,
        "{signature:#?}"
    );
    assert_eq!(signature.signature_algorithm.as_deref(), Some(algorithm));
    assert_eq!(signature.pades_detected_level, level);
    assert_eq!(signature.signer_certificate, CertificateMatch::Found);
    assert_eq!(
        signature.algorithm_policy.digest_policy.policy_status,
        AlgorithmPolicyStatus::Allowed
    );
    assert_eq!(
        signature.algorithm_policy.signature_policy.policy_status,
        AlgorithmPolicyStatus::Allowed
    );
}

#[test]
fn unsigned_documents_have_no_signatures() {
    let report = inspect("sig-01-unsigned.pdf");
    assert_eq!(report.total_signatures, 0);
    assert_eq!(report.inspection_status, InspectionStatus::Complete);
    assert_eq!(
        report.document_modification_status,
        DocumentModificationStatus::Indeterminate
    );
    assert!(codes(&report.global_findings).contains(&"NO_SIGNATURES"));

    let report = inspect("sig-03-empty-signature-field.pdf");
    assert_eq!(report.total_signatures, 0);
    assert_eq!(report.discovery.empty_signature_field_count, 1);
    // PDFium lists the empty field as a signature object (G2 spike); it is not a signature.
    assert_eq!(report.discovery.pdfium_signature_object_count, Some(1));
}

#[test]
fn valid_signatures_of_every_supported_algorithm_verify() {
    for (name, algorithm, digest) in [
        (
            "sig-09-rsa-pkcs1-sha256.pdf",
            "RSA-PKCS1v15-2048",
            "SHA-256",
        ),
        ("sig-10-rsa-pss-sha384.pdf", "RSA-PSS-2048", "SHA-384"),
        ("sig-11-ecdsa-p256-sha256.pdf", "ECDSA-P256", "SHA-256"),
        ("sig-12-ecdsa-p384-sha384.pdf", "ECDSA-P384", "SHA-384"),
    ] {
        let report = inspect(name);
        assert_option1(&report);
        let signature = only(&report);
        assert_valid(signature, algorithm, PadesLevel::PadesBB);
        assert_eq!(
            signature.digest_algorithm.as_deref(),
            Some(digest),
            "{name}"
        );
        assert_eq!(
            signature.document_modification,
            ModificationScope::NoLaterRevision,
            "{name}"
        );
        assert_eq!(signature.revision_index, Some(0), "{name}");
        assert_eq!(
            signature.field_name.as_deref(),
            Some("Signature1"),
            "{name}"
        );
        assert!(
            signature.claimed_signing_time.cms_signing_time.is_some(),
            "{name}"
        );
        let identity = signature.signer_identity.as_ref().unwrap();
        assert_eq!(
            identity.organization.as_deref(),
            Some("TEST ONLY"),
            "{name}"
        );
        assert_eq!(identity.country.as_deref(), Some("TR"), "{name}");
        assert_eq!(
            report.document_modification_status,
            DocumentModificationStatus::NoModifications,
            "{name}"
        );
        assert_eq!(
            report.inspection_status,
            InspectionStatus::Complete,
            "{name}"
        );
    }
}

#[test]
fn adbe_pkcs7_without_signed_attributes_verifies_but_is_not_pades() {
    let report = inspect("sig-09b-adbe-pkcs7-no-signed-attributes.pdf");
    let signature = only(&report);
    assert_valid(signature, "RSA-PKCS1v15-2048", PadesLevel::None);
    assert_eq!(signature.claimed_signing_time.cms_signing_time, None);
}

#[test]
fn two_signatures_are_verified_independently() {
    let report = inspect("sig-16-two-signatures.pdf");
    assert_option1(&report);
    assert_eq!(report.total_signatures, 2);
    let [first, second] = report.signatures.as_slice() else {
        panic!()
    };
    assert_valid(first, "RSA-PKCS1v15-2048", PadesLevel::PadesBB);
    assert_valid(second, "ECDSA-P256", PadesLevel::PadesBB);
    assert_eq!(first.revision_index, Some(0));
    assert_eq!(second.revision_index, Some(1));
    assert_eq!(
        first.document_modification,
        ModificationScope::LaterRevisionUnclassified
    );
    assert_eq!(
        first.later_revisions,
        vec![LaterRevision {
            revision_index: 1,
            kind: LaterRevisionKind::Signed
        }]
    );
    assert_eq!(
        second.document_modification,
        ModificationScope::NoLaterRevision
    );
    assert_ne!(first.signature_id, second.signature_id);
    assert_eq!(
        report.document_modification_status,
        DocumentModificationStatus::NoModifications
    );
}

#[test]
fn tampering_is_invalid() {
    for (name, code) in [
        (
            "sig-19-modified-after-signing.pdf",
            "SIGNATURE_DIGEST_MISMATCH",
        ),
        ("sig-21-bad-signature-value.pdf", "SIGNATURE_VALUE_INVALID"),
        (
            "sig-22-message-digest-mismatch.pdf",
            "SIGNATURE_DIGEST_MISMATCH",
        ),
        (
            "sig-24-content-type-missing.pdf",
            "CMS_CONTENT_TYPE_ATTRIBUTE_INVALID",
        ),
        ("sig-wrong-key.pdf", "SIGNATURE_VALUE_INVALID"),
    ] {
        let report = inspect(name);
        let signature = only(&report);
        assert_eq!(
            signature.byte_range_status,
            ByteRangeStatus::Valid,
            "{name}"
        );
        assert_eq!(
            signature.cryptographic_integrity,
            CryptographicIntegrityStatus::Invalid,
            "{name}"
        );
        assert!(
            codes(&signature.findings).contains(&code),
            "{name}: {:?}",
            signature.findings
        );
    }
}

#[test]
fn weak_algorithms_are_reported_not_verified() {
    let report = inspect("sig-27-sha1-digest.pdf");
    let signature = only(&report);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::UnsupportedAlgorithm
    );
    assert_eq!(
        signature.algorithm_policy.digest_policy.policy_status,
        AlgorithmPolicyStatus::Disallowed
    );
    assert!(codes(&signature.findings).contains(&"SIGNATURE_ALGORITHM_FORBIDDEN"));

    let report = inspect("sig-rsa-1024-weak-key.pdf");
    let signature = only(&report);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::UnsupportedAlgorithm
    );
    assert_eq!(
        signature.algorithm_policy.signature_policy.policy_status,
        AlgorithmPolicyStatus::Disallowed
    );
    assert_eq!(signature.signature_algorithm.as_deref(), Some("RSA-1024"));
}

#[test]
fn signer_certificate_must_be_found_exactly_once() {
    let report = inspect("sig-signer-certificate-missing.pdf");
    let signature = only(&report);
    assert_eq!(signature.signer_certificate, CertificateMatch::Missing);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Indeterminate
    );
    assert!(signature.signer_identity.is_none());

    let report = inspect("sig-signer-certificate-ambiguous.pdf");
    let signature = only(&report);
    assert_eq!(signature.signer_certificate, CertificateMatch::Ambiguous);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Indeterminate
    );
}

#[test]
fn later_revisions_and_trailing_bytes_do_not_break_the_signature() {
    let report = inspect("sig-unsigned-revision-after-signing.pdf");
    let signature = only(&report);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Valid
    );
    assert_eq!(
        signature.document_modification,
        ModificationScope::LaterRevisionUnclassified
    );
    assert_eq!(
        signature.later_revisions,
        vec![LaterRevision {
            revision_index: 1,
            kind: LaterRevisionKind::Unsigned
        }]
    );
    assert!(codes(&signature.findings).contains(&"UNSIGNED_LATER_REVISION"));
    assert_eq!(
        report.document_modification_status,
        DocumentModificationStatus::Indeterminate
    );

    let report = inspect("sig-trailing-bytes.pdf");
    let signature = only(&report);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Valid
    );
    assert_eq!(
        signature.document_modification,
        ModificationScope::LaterRevisionUnclassified
    );
    assert!(report.revisions.trailing_bytes > 0);
    assert!(codes(&report.global_findings).contains(&"TRAILING_BYTES_AFTER_LAST_REVISION"));
}

#[test]
fn docmdp_is_reported_and_later_changes_are_not_classified() {
    let report = inspect("sig-39-docmdp-p1.pdf");
    let signature = only(&report);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Valid
    );
    let docmdp = signature.docmdp.as_ref().unwrap();
    assert_eq!(
        docmdp.permission,
        Some(CertificationPermission::NoChangesPermitted)
    );
    assert!(docmdp.certification_signature);
    assert_eq!(
        report.certification_permission,
        Some(CertificationPermission::NoChangesPermitted)
    );

    let report = inspect("sig-39b-docmdp-p1-later-revision.pdf");
    let signature = only(&report);
    assert_eq!(
        signature.document_modification,
        ModificationScope::LaterRevisionUnclassified
    );
    assert!(codes(&signature.findings).contains(&"DOCMDP_P1_LATER_REVISION"));
}

#[test]
fn timestamps_are_detected_not_verified() {
    let report = inspect("sig-doc-timestamp.pdf");
    let signature = only(&report);
    assert_eq!(signature.kind, SignatureKind::DocumentTimestamp);
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Indeterminate
    );
    assert_eq!(
        signature.signature_timestamp,
        Some(TimestampReport {
            timestamp_type: TimestampType::DocumentTimestamp,
            timestamp_status: TimestampStatus::NotVerified,
        })
    );
    assert_eq!(report.discovery.document_timestamp_count, 1);

    let report = inspect("sig-signature-timestamp-attribute.pdf");
    let signature = only(&report);
    assert_valid(signature, "RSA-PKCS1v15-2048", PadesLevel::PadesBT);
    assert_eq!(
        signature
            .signature_timestamp
            .as_ref()
            .map(|timestamp| timestamp.timestamp_status),
        Some(TimestampStatus::NotVerified)
    );
}

#[test]
fn canonical_discovery_finds_what_pdfium_does_not_list() {
    let report = inspect("sig-signed-kid-field.pdf");
    let signature = only(&report);
    assert_eq!(signature.field_name.as_deref(), Some("Parent.Kid"));
    assert_eq!(
        signature.cryptographic_integrity,
        CryptographicIntegrityStatus::Valid
    );
    assert!(codes(&report.global_findings).contains(&"SIGNATURE_NOT_VISIBLE_TO_PDFIUM"));

    let report = inspect("sig-unreferenced-signature-dictionary.pdf");
    assert_eq!(report.total_signatures, 0);
    assert_eq!(report.discovery.unreferenced_signature_dictionary_count, 1);
    assert!(codes(&report.global_findings).contains(&"SIGNATURE_DICTIONARY_UNREFERENCED"));
}

#[test]
fn hostile_byte_ranges_are_invalid() {
    for (name, code) in [
        (
            "sig-49-byterange-overlapping.pdf",
            "SIGNATURE_BYTE_RANGE_INVALID",
        ),
        (
            "sig-50-byterange-out-of-bounds.pdf",
            "SIGNATURE_BYTE_RANGE_INVALID",
        ),
        (
            "sig-51-byterange-negative.pdf",
            "SIGNATURE_BYTE_RANGE_INVALID",
        ),
        (
            "sig-52-byterange-uncovered-gap.pdf",
            "SIGNATURE_BYTE_RANGE_GAP_DETECTED",
        ),
        (
            "sig-53-byterange-two-exclusions.pdf",
            "SIGNATURE_BYTE_RANGE_INVALID",
        ),
    ] {
        let report = inspect(name);
        let signature = only(&report);
        assert_eq!(
            signature.byte_range_status,
            ByteRangeStatus::Invalid,
            "{name}"
        );
        assert_eq!(
            signature.document_modification,
            ModificationScope::ByteRangeInvalid,
            "{name}"
        );
        assert_ne!(
            signature.cryptographic_integrity,
            CryptographicIntegrityStatus::Valid,
            "{name}"
        );
        assert!(
            codes(&signature.findings).contains(&code),
            "{name}: {:?}",
            signature.findings
        );
        assert_eq!(
            report.document_modification_status,
            DocumentModificationStatus::Indeterminate,
            "{name}"
        );
    }
}

#[test]
fn hostile_contents_are_malformed() {
    for name in ["sig-54-empty-contents.pdf", "sig-55-malformed-der.pdf"] {
        let report = inspect(name);
        let signature = only(&report);
        assert_eq!(
            signature.cryptographic_integrity,
            CryptographicIntegrityStatus::Malformed,
            "{name}"
        );
        assert!(
            codes(&signature.findings).contains(&"SIGNATURE_CMS_PARSE_FAILED"),
            "{name}"
        );
    }
}

#[test]
fn existing_signature_marked_fixtures_have_an_uncovered_gap() {
    // Design §7 note: `/ByteRange [0 100 200 300]` does not cover the file.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    for path in [
        root.join(["sanit", "ization"].concat())
            .join("signature.pdf"),
        root.join(["redac", "tion"].concat())
            .join("signature-marked-byterange.pdf"),
    ] {
        let _guard = PDFIUM_TEST_MUTEX.lock();
        let report = inspect_signatures(&path, None, "s", 1).unwrap();
        let signature = only(&report);
        assert_eq!(
            signature.byte_range_status,
            ByteRangeStatus::Invalid,
            "{path:?}"
        );
        assert!(
            codes(&signature.findings).contains(&"SIGNATURE_BYTE_RANGE_GAP_DETECTED"),
            "{path:?}: {:?}",
            signature.findings
        );
        assert_ne!(
            signature.cryptographic_integrity,
            CryptographicIntegrityStatus::Valid
        );
    }
}

#[test]
fn encrypted_documents_need_the_session_credential() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pdf")
        .join("encrypted.pdf");
    let _guard = PDFIUM_TEST_MUTEX.lock();
    let without = inspect_signatures(&path, None, "s", 1).unwrap();
    assert_eq!(without.inspection_status, InspectionStatus::Indeterminate);
    assert!(codes(&without.global_findings).contains(&"SIGNATURE_DOCUMENT_UNREADABLE"));
    let with = inspect_signatures(&path, Some("fixture-password"), "s", 1).unwrap();
    assert_ne!(with.inspection_status, InspectionStatus::Indeterminate);
    assert_eq!(with.total_signatures, 0);
}

#[test]
fn reports_are_deterministic() {
    let first = inspect("sig-16-two-signatures.pdf");
    let second = inspect("sig-16-two-signatures.pdf");
    assert_eq!(first, second);
}
