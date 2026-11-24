// Offline signature verification report (Prompt 11 G2; pades-architecture-design.md §2.4).
// The job worker builds it and the main process bound-checks it; this is the UI-side shape
// check. OPTION 1 (§19): trust and revocation are never evaluated, so a report claiming
// otherwise is rejected instead of shown.

export const SIGNATURE_REPORT_SCHEMA_VERSION = 1;

const INTEGRITY = ["valid", "invalid", "indeterminate", "unsupported_algorithm", "malformed"];
const BYTE_RANGE = ["valid", "invalid", "indeterminate"];
const MODIFICATION_SCOPE = [
  "no_later_revision",
  "later_revision_permitted",
  "later_revision_not_permitted",
  "later_revision_unclassified",
  "byte_range_invalid",
];
const DOCUMENT_MODIFICATION = [
  "no_modifications",
  "permitted_modifications_only",
  "unauthorized_modifications_detected",
  "indeterminate",
];
const PADES = ["none", "pades_b_b", "pades_b_t", "pades_b_l_t", "pades_b_l_t_a"];
const INSPECTION = ["complete", "partial", "indeterminate"];
const CERTIFICATE = ["found", "missing", "ambiguous", "not_evaluated"];
const KIND = ["signature", "document_timestamp"];

function fail(message) {
  const error = new Error(`İmza raporu geçersiz: ${message}`);
  error.code = "SIGNATURE_REPORT_INVALID";
  throw error;
}

function oneOf(value, allowed, field) {
  if (!allowed.includes(value)) fail(`${field}=${String(value)}`);
  return value;
}

function text(value, field, { optional = false, max = 1024 } = {}) {
  if (value === null || value === undefined) {
    if (optional) return null;
    fail(`${field} eksik`);
  }
  if (typeof value !== "string" || value.length > max) fail(`${field} metin değil`);
  return value;
}

function findings(list, field) {
  if (!Array.isArray(list) || list.length > 64) fail(`${field} listesi`);
  return Object.freeze(
    list.map((finding, index) =>
      Object.freeze({
        code: text(finding?.code, `${field}[${index}].code`, { max: 64 }),
        severity: oneOf(finding?.severity, ["info", "warning", "error", "fatal"], `${field}[${index}].severity`),
        message: text(finding?.message, `${field}[${index}].message`),
        context: text(finding?.context, `${field}[${index}].context`, { optional: true }),
      }),
    ),
  );
}

function identity(value) {
  if (value === null || value === undefined) return null;
  return Object.freeze({
    commonName: text(value.common_name, "common_name", { optional: true }),
    organization: text(value.organization, "organization", { optional: true }),
    country: text(value.country, "country", { optional: true }),
    serial: text(value.certificate_serial, "certificate_serial", { max: 128 }),
    issuer: text(value.certificate_issuer, "certificate_issuer"),
    validFrom: text(value.certificate_validity_start, "certificate_validity_start", { max: 64 }),
    validTo: text(value.certificate_validity_end, "certificate_validity_end", { max: 64 }),
    fingerprint: text(value.certificate_sha256_fingerprint, "certificate_sha256_fingerprint", { max: 64 }),
  });
}

function signature(value, index) {
  const field = `signatures[${index}]`;
  if (!value || typeof value !== "object") fail(field);
  if (value.trust_status !== "not_checked" || value.revocation_status !== "not_checked") {
    fail(`${field} kapsam dışı güven iddiası`);
  }
  if (value.pades_validated_level !== "none") fail(`${field} doğrulanmış PAdES iddiası`);
  const docmdp = value.docmdp
    ? Object.freeze({
        permission: value.docmdp.permission ?? null,
        certification: value.docmdp.certification_signature === true,
        fieldMdp: value.docmdp.field_mdp
          ? Object.freeze({
              action: text(value.docmdp.field_mdp.action, "field_mdp.action", { max: 64 }),
              fieldCount: Number(value.docmdp.field_mdp.field_count) || 0,
            })
          : null,
      })
    : null;
  return Object.freeze({
    id: text(value.signature_id, `${field}.signature_id`, { max: 32 }),
    fieldName: text(value.field_name, `${field}.field_name`, { optional: true }),
    kind: oneOf(value.kind, KIND, `${field}.kind`),
    revisionIndex: Number.isInteger(value.revision_index) ? value.revision_index : null,
    byteRangeStatus: oneOf(value.byte_range_status, BYTE_RANGE, `${field}.byte_range_status`),
    integrity: oneOf(value.cryptographic_integrity, INTEGRITY, `${field}.cryptographic_integrity`),
    modification: oneOf(value.document_modification, MODIFICATION_SCOPE, `${field}.document_modification`),
    laterRevisions: Object.freeze(
      (Array.isArray(value.later_revisions) ? value.later_revisions : []).map((revision) =>
        Object.freeze({
          index: revision.revision_index,
          kind: oneOf(revision.kind, ["signed", "document_timestamp", "unsigned"], `${field}.later_revisions`),
        }),
      ),
    ),
    subFilter: text(value.sub_filter, `${field}.sub_filter`, { optional: true }),
    digestAlgorithm: text(value.digest_algorithm, `${field}.digest_algorithm`, { optional: true, max: 64 }),
    signatureAlgorithm: text(value.signature_algorithm, `${field}.signature_algorithm`, { optional: true, max: 64 }),
    digestPolicy: value.algorithm_policy?.digest_policy?.policy_status ?? "not_evaluated",
    signaturePolicy: value.algorithm_policy?.signature_policy?.policy_status ?? "not_evaluated",
    signer: identity(value.signer_identity),
    signerCertificate: oneOf(value.signer_certificate, CERTIFICATE, `${field}.signer_certificate`),
    claimedTime: Object.freeze({
      pdf: text(value.claimed_signing_time?.pdf_m, `${field}.pdf_m`, { optional: true, max: 64 }),
      cms: text(value.claimed_signing_time?.cms_signing_time, `${field}.cms_signing_time`, { optional: true, max: 64 }),
    }),
    timestamp: value.signature_timestamp
      ? Object.freeze({
          type: value.signature_timestamp.timestamp_type,
          status: value.signature_timestamp.timestamp_status,
        })
      : null,
    padesDetected: oneOf(value.pades_detected_level, PADES, `${field}.pades_detected_level`),
    docmdp,
    reason: text(value.reason, `${field}.reason`, { optional: true }),
    findings: findings(value.findings ?? [], `${field}.findings`),
  });
}

export function normalizeSignatureReport(raw) {
  if (!raw || typeof raw !== "object") fail("rapor nesne değil");
  if (raw.schema_version !== SIGNATURE_REPORT_SCHEMA_VERSION) fail("şema sürümü");
  if (!Array.isArray(raw.signatures) || raw.signatures.length > 256) fail("signatures");
  return Object.freeze({
    sessionId: text(raw.session_id, "session_id"),
    contentGeneration: raw.content_generation,
    inspectionStatus: oneOf(raw.inspection_status, INSPECTION, "inspection_status"),
    totalSignatures: raw.total_signatures,
    emptyFieldCount: raw.discovery?.empty_signature_field_count ?? 0,
    unreferencedCount: raw.discovery?.unreferenced_signature_dictionary_count ?? 0,
    revisionCount: raw.revisions?.revision_count ?? 0,
    trailingBytes: raw.revisions?.trailing_bytes ?? 0,
    documentModification: oneOf(raw.document_modification_status, DOCUMENT_MODIFICATION, "document_modification_status"),
    certificationPermission: raw.certification_permission ?? null,
    signatures: Object.freeze(raw.signatures.map(signature)),
    findings: findings(raw.global_findings ?? [], "global_findings"),
  });
}
