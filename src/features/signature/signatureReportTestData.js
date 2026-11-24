// Test data for the signature verification report (Vitest only; never imported by the app).

export function rawSignature(overrides = {}) {
  return {
    signature_id: "0123456789abcdef0123456789abcdef",
    field_name: "Signature1",
    kind: "signature",
    revision_index: 0,
    byte_range: [0, 10, 20, 30],
    byte_range_status: "valid",
    contents_digest: "a".repeat(64),
    sub_filter: "ETSI.CAdES.detached",
    cryptographic_integrity: "valid",
    digest_algorithm: "SHA-256",
    signature_algorithm: "RSA-PKCS1v15-2048",
    document_modification: "no_later_revision",
    later_revisions: [],
    signer_identity: {
      common_name: "TEST ONLY RSA Signer",
      organization: "TEST ONLY",
      country: "TR",
      certificate_serial: "01",
      certificate_issuer: "CN=TEST ONLY RSA Signer,O=TEST ONLY,C=TR",
      certificate_validity_start: "2026-01-01T00:00:00Z",
      certificate_validity_end: "2036-01-01T00:00:00Z",
      certificate_sha256_fingerprint: "b".repeat(64),
    },
    signer_certificate: "found",
    claimed_signing_time: { pdf_m: "D:20260101000000Z", cms_signing_time: "2026-01-01T00:00:00Z" },
    signature_timestamp: null,
    pades_detected_level: "pades_b_b",
    pades_validated_level: "none",
    algorithm_policy: {
      digest_policy: { algorithm_identifier: "SHA-256", policy_status: "allowed", normative_references: [] },
      signature_policy: { algorithm_identifier: "RSA-PKCS1v15-2048", policy_status: "allowed", normative_references: [] },
      format_specifications: [],
    },
    revocation_status: "not_checked",
    trust_status: "not_checked",
    docmdp: null,
    reason: null,
    findings: [],
    ...overrides,
  };
}

export function rawReport(overrides = {}) {
  return {
    schema_version: 1,
    session_id: "session-1",
    content_generation: 3,
    document_digest: "c".repeat(64),
    file_length: 1000,
    inspection_status: "complete",
    total_signatures: 1,
    discovery: {
      signed_signature_count: 1,
      document_timestamp_count: 0,
      empty_signature_field_count: 0,
      unreferenced_signature_dictionary_count: 0,
      pdfium_signature_object_count: 1,
      structural_status: "complete",
    },
    revisions: { revision_count: 1, revision_ends: [1000], trailing_bytes: 0 },
    signatures: [rawSignature()],
    document_modification_status: "no_modifications",
    certification_permission: null,
    global_findings: [],
    ...overrides,
  };
}
