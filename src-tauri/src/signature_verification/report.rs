//! Typed offline signature verification report (pades-architecture-design.md §2.4, v1.3.2).
//!
//! The job worker builds this report; the main process only bound-checks it
//! ([`super::validate_report`]) and forwards it to the UI. It never carries raw certificates,
//! CMS or DER bytes: only trimmed display fields and hex digests.
//!
//! OPTION 1 (§19): `trust_status` and `revocation_status` are always `NotChecked`,
//! `pades_validated_level` is always `None`, timestamps are `NotVerified`.

use serde::{Deserialize, Serialize};

pub const SIGNATURE_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PdfSignatureVerificationReport {
    pub schema_version: u32,
    pub session_id: String,
    pub content_generation: u64,
    /// SHA-256 of the whole snapshot (lowercase hex).
    pub document_digest: String,
    pub file_length: u64,
    pub inspection_status: InspectionStatus,
    /// Signed signature fields, document timestamps included.
    pub total_signatures: u32,
    pub discovery: DiscoverySummary,
    pub revisions: RevisionSummary,
    /// Sorted by revision, ByteRange, then field name; never by discovery order.
    pub signatures: Vec<PdfSignatureReport>,
    pub document_modification_status: DocumentModificationStatus,
    pub certification_permission: Option<CertificationPermission>,
    pub global_findings: Vec<ValidationFinding>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectionStatus {
    /// Every source the verifier uses could read the document.
    Complete,
    /// A structural source (lopdf or the revision scan) was unavailable or disagreed.
    Partial,
    /// The document could not be opened: nothing was verified.
    Indeterminate,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DiscoverySummary {
    pub signed_signature_count: u32,
    pub document_timestamp_count: u32,
    pub empty_signature_field_count: u32,
    pub unreferenced_signature_dictionary_count: u32,
    /// `FPDF_GetSignatureCount`; `None` when PDFium could not open the document.
    pub pdfium_signature_object_count: Option<u32>,
    pub structural_status: StructuralStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuralStatus {
    Complete,
    /// lopdf could not read the document (too large, unparsable, credential refused).
    Unavailable,
    /// PDFium and lopdf disagreed; see the `DISCOVERY_SOURCE_MISMATCH` finding.
    Inconsistent,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RevisionSummary {
    /// `startxref … %%EOF` markers. A linearized file has one more marker than revisions.
    pub revision_count: u32,
    /// End offset of each revision (after `%%EOF` and its end-of-line).
    pub revision_ends: Vec<u64>,
    /// Bytes after the last revision end.
    pub trailing_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PdfSignatureReport {
    /// sha256(field name | revision | ByteRange | sha256(/Contents)), first 16 bytes, hex.
    pub signature_id: String,
    pub field_name: Option<String>,
    pub kind: SignatureKind,
    /// Index into `revisions.revision_ends` of the revision this signature covers.
    pub revision_index: Option<u32>,
    pub byte_range: Vec<i64>,
    pub byte_range_status: ByteRangeStatus,
    /// SHA-256 of the decoded `/Contents` bytes (lowercase hex).
    pub contents_digest: String,
    pub sub_filter: Option<String>,
    pub cryptographic_integrity: CryptographicIntegrityStatus,
    pub digest_algorithm: Option<String>,
    pub signature_algorithm: Option<String>,
    pub document_modification: ModificationScope,
    pub later_revisions: Vec<LaterRevision>,
    pub signer_identity: Option<SignerIdentityReport>,
    pub signer_certificate: CertificateMatch,
    pub claimed_signing_time: ClaimedSigningTime,
    pub signature_timestamp: Option<TimestampReport>,
    pub pades_detected_level: PadesLevel,
    pub pades_validated_level: PadesLevel,
    pub algorithm_policy: AlgorithmPolicyReport,
    pub revocation_status: RevocationStatus,
    pub trust_status: TrustChainStatus,
    pub docmdp: Option<DocMdpReport>,
    pub reason: Option<String>,
    pub findings: Vec<ValidationFinding>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignatureKind {
    Signature,
    DocumentTimestamp,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CryptographicIntegrityStatus {
    Valid,
    Invalid,
    Indeterminate,
    UnsupportedAlgorithm,
    Malformed,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ByteRangeStatus {
    Valid,
    Invalid,
    Indeterminate,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustChainStatus {
    Trusted,
    Untrusted,
    Indeterminate,
    NotChecked,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RevocationStatus {
    Good,
    Revoked,
    Unknown,
    NotChecked,
    NetworkFailed,
    EvidenceExpired,
    MalformedEvidence,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentModificationStatus {
    NoModifications,
    PermittedModificationsOnly,
    UnauthorizedModificationsDetected,
    Indeterminate,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CertificationPermission {
    NoChangesPermitted,
    FormFillAndSigningPermitted,
    FormFillSigningAndAnnotations,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModificationScope {
    NoLaterRevision,
    LaterRevisionPermitted,
    LaterRevisionNotPermitted,
    LaterRevisionUnclassified,
    ByteRangeInvalid,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LaterRevision {
    pub revision_index: u32,
    pub kind: LaterRevisionKind,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaterRevisionKind {
    /// A later signature covers this revision.
    Signed,
    DocumentTimestamp,
    /// No signature ends at this revision.
    Unsigned,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PadesLevel {
    None,
    PadesBB,
    PadesBT,
    PadesBLT,
    PadesBLTA,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AlgorithmPolicyReport {
    pub digest_policy: SingleAlgorithmPolicy,
    pub signature_policy: SingleAlgorithmPolicy,
    pub format_specifications: Vec<FormatSpecificationReference>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SingleAlgorithmPolicy {
    pub algorithm_identifier: String,
    pub policy_status: AlgorithmPolicyStatus,
    pub normative_references: Vec<AlgorithmStandardReference>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmPolicyStatus {
    Allowed,
    Deprecated,
    Disallowed,
    Indeterminate,
    NotEvaluated,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmStandardReference {
    Fips186_5,
    FipsPub180_4,
    NistSp800_186,
    Pkcs1v2_2,
    AnsiX9_62,
    Unstandardized,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FormatSpecificationReference {
    Rfc5652Cms,
    Rfc5280X509,
    EtsiEn319142_1Pades,
    Iso32000_1,
    Iso32000_2,
    Rfc3161Tsp,
}

/// Display fields of the signer certificate. These are the signer's claims: the certificate
/// is not validated against any trust anchor (G4).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignerIdentityReport {
    pub common_name: Option<String>,
    pub organization: Option<String>,
    pub country: Option<String>,
    pub certificate_serial: String,
    pub certificate_issuer: String,
    pub certificate_validity_start: String,
    pub certificate_validity_end: String,
    pub certificate_sha256_fingerprint: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CertificateMatch {
    Found,
    Missing,
    Ambiguous,
    /// The CMS could not be read far enough to look.
    NotEvaluated,
}

/// Both values are claims of the signer (§2.5): neither is a trusted time.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimedSigningTime {
    /// `/M` of the signature dictionary.
    pub pdf_m: Option<String>,
    /// CMS `signing-time` signed attribute.
    pub cms_signing_time: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TimestampReport {
    pub timestamp_type: TimestampType,
    pub timestamp_status: TimestampStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimestampType {
    SignatureTimestamp,
    DocumentTimestamp,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimestampStatus {
    Valid,
    Invalid,
    Expired,
    UntrustedTsa,
    DigestMismatch,
    Indeterminate,
    /// v1.3.2: present, but RFC 3161 validation is G5.
    NotVerified,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DocMdpReport {
    pub permission: Option<CertificationPermission>,
    /// The catalog `/Perms /DocMDP` entry points at this signature.
    pub certification_signature: bool,
    pub field_mdp: Option<FieldMdpReport>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FieldMdpReport {
    /// `All`, `Include` or `Exclude`; other values are kept as written (trimmed).
    pub action: String,
    pub field_count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ValidationFinding {
    pub code: String,
    pub severity: FindingSeverity,
    pub message: String,
    pub context: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Info,
    Warning,
    Error,
    Fatal,
}

impl ValidationFinding {
    pub(crate) fn new(code: &str, severity: FindingSeverity, message: &str) -> Self {
        Self {
            code: code.to_string(),
            severity,
            message: message.to_string(),
            context: None,
        }
    }

    pub(crate) fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(super::limits::trim(
            &context.into(),
            super::limits::MAX_CONTEXT_CHARS,
        ));
        self
    }
}
