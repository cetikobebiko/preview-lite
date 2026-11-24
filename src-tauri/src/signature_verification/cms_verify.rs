//! CAP-05…CAP-08: offline CMS `SignedData` verification (design §8). Worker only.
//!
//! The input is hostile: its shape is pre-scanned with a bounded, iterative TLV walker
//! ([`prescan`]) before any typed decoding. No trust decision is made here: the signer
//! certificate is located and displayed, never chained (G4).

use super::digest::{DigestAlgorithm, DigestError};
use super::limits::{self, MAX_CMS_CERTIFICATES, MAX_DER_DEPTH, MAX_DER_ELEMENTS};
use super::report::{
    AlgorithmPolicyReport, AlgorithmPolicyStatus, AlgorithmStandardReference, CertificateMatch,
    CryptographicIntegrityStatus, FindingSeverity, FormatSpecificationReference,
    SignerIdentityReport, SingleAlgorithmPolicy, ValidationFinding,
};
use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use const_oid::ObjectIdentifier;
use der::asn1::{Any, OctetString};
use der::{Decode, Encode, Tag, Tagged};
use ecdsa::signature::hazmat::PrehashVerifier;
use x509_cert::Certificate;

const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const ID_SIGNING_CERTIFICATE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.12");
const ID_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const ID_SIGNATURE_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");

const ID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const ID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const ID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const ID_MD5: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.5");

const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const MD5_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.4");
const SHA1_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const SHA384_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const SHA512_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const RSASSA_PSS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");
const ID_MGF1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.8");
const EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const ECDSA_WITH_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.1");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_WITH_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const ECDSA_WITH_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.4");
const CURVE_P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const CURVE_P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");

const ID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const ID_ORGANIZATION: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.10");
const ID_COUNTRY: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.6");

/// Algorithm policy (§8 + Gate 1 proposal for RSA, approved 2026-11-24).
pub const MIN_RSA_MODULUS_BITS: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmsOutcome {
    pub integrity: CryptographicIntegrityStatus,
    pub digest_algorithm: Option<String>,
    pub signature_algorithm: Option<String>,
    pub algorithm_policy: AlgorithmPolicyReport,
    pub signer_identity: Option<SignerIdentityReport>,
    pub signer_certificate: CertificateMatch,
    pub cms_signing_time: Option<String>,
    pub signature_timestamp_present: bool,
    /// Signed attributes carry content-type, message-digest and a signing-certificate
    /// attribute (the PAdES B-B signed attributes, §2.6).
    pub pades_signed_attributes: bool,
    pub findings: Vec<ValidationFinding>,
}

impl CmsOutcome {
    fn new() -> Self {
        Self {
            integrity: CryptographicIntegrityStatus::Indeterminate,
            digest_algorithm: None,
            signature_algorithm: None,
            algorithm_policy: AlgorithmPolicyReport {
                digest_policy: not_evaluated(),
                signature_policy: not_evaluated(),
                format_specifications: vec![
                    FormatSpecificationReference::Rfc5652Cms,
                    FormatSpecificationReference::Rfc5280X509,
                    FormatSpecificationReference::Iso32000_1,
                ],
            },
            signer_identity: None,
            signer_certificate: CertificateMatch::NotEvaluated,
            cms_signing_time: None,
            signature_timestamp_present: false,
            pades_signed_attributes: false,
            findings: Vec::new(),
        }
    }

    fn fail(mut self, status: CryptographicIntegrityStatus, code: &str, message: &str) -> Self {
        self.integrity = status;
        self.findings.push(ValidationFinding::new(
            code,
            match status {
                CryptographicIntegrityStatus::Invalid | CryptographicIntegrityStatus::Malformed => {
                    FindingSeverity::Error
                }
                _ => FindingSeverity::Warning,
            },
            message,
        ));
        self
    }
}

fn not_evaluated() -> SingleAlgorithmPolicy {
    SingleAlgorithmPolicy {
        algorithm_identifier: String::new(),
        policy_status: AlgorithmPolicyStatus::NotEvaluated,
        normative_references: Vec::new(),
    }
}

/// The document digest for `algorithm`, or why it is not available (invalid ByteRange,
/// exhausted budget, I/O).
pub type DocumentDigest<'a> = dyn FnMut(DigestAlgorithm) -> Result<Vec<u8>, DigestUnavailable> + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestUnavailable {
    ByteRangeUnusable,
    Digest(DigestError),
}

// ---------------------------------------------------------------------------------------
// Pre-scan
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeError {
    Empty,
    Truncated,
    IndefiniteLength,
    TooDeep,
    TooManyElements,
    /// Bytes after the outer element that are not zero padding.
    TrailingData,
}

/// Walks the DER tree without recursion and returns the length of the outer element. The
/// placeholder after it must be zero padding.
pub fn prescan(bytes: &[u8]) -> Result<usize, ShapeError> {
    if bytes.is_empty() || bytes.iter().all(|byte| *byte == 0) {
        return Err(ShapeError::Empty);
    }
    let (outer_header, outer_length, _) = header(bytes, 0)?;
    let outer_end = outer_header
        .checked_add(outer_length)
        .filter(|end| *end <= bytes.len())
        .ok_or(ShapeError::Truncated)?;
    if bytes[outer_end..].iter().any(|byte| *byte != 0) {
        return Err(ShapeError::TrailingData);
    }
    // Stack of end offsets of the open constructed elements.
    let mut stack: Vec<usize> = Vec::new();
    let mut position = 0usize;
    let mut elements = 0usize;
    while position < outer_end {
        while stack.last().is_some_and(|end| *end == position) {
            stack.pop();
        }
        if position >= outer_end {
            break;
        }
        elements += 1;
        if elements > MAX_DER_ELEMENTS {
            return Err(ShapeError::TooManyElements);
        }
        let (header_length, length, constructed) = header(bytes, position)?;
        let content_start = position
            .checked_add(header_length)
            .ok_or(ShapeError::Truncated)?;
        let end = content_start
            .checked_add(length)
            .ok_or(ShapeError::Truncated)?;
        let parent_end = stack.last().copied().unwrap_or(outer_end);
        if end > parent_end {
            return Err(ShapeError::Truncated);
        }
        if constructed {
            stack.push(end);
            if stack.len() > MAX_DER_DEPTH {
                return Err(ShapeError::TooDeep);
            }
            position = content_start;
        } else {
            position = end;
        }
    }
    Ok(outer_end)
}

/// `(header length, content length, constructed)` of the element at `position`.
fn header(bytes: &[u8], position: usize) -> Result<(usize, usize, bool), ShapeError> {
    let tag = *bytes.get(position).ok_or(ShapeError::Truncated)?;
    let constructed = tag & 0x20 != 0;
    let mut cursor = position + 1;
    if tag & 0x1f == 0x1f {
        // High tag number: base-128 continuation bytes.
        let mut count = 0;
        loop {
            let byte = *bytes.get(cursor).ok_or(ShapeError::Truncated)?;
            cursor += 1;
            count += 1;
            if count > 4 {
                return Err(ShapeError::Truncated);
            }
            if byte & 0x80 == 0 {
                break;
            }
        }
    }
    let first = *bytes.get(cursor).ok_or(ShapeError::Truncated)?;
    cursor += 1;
    let length = if first < 0x80 {
        usize::from(first)
    } else if first == 0x80 {
        return Err(ShapeError::IndefiniteLength);
    } else {
        let count = usize::from(first & 0x7f);
        if count > 4 {
            return Err(ShapeError::Truncated);
        }
        let mut value = 0usize;
        for _ in 0..count {
            let byte = *bytes.get(cursor).ok_or(ShapeError::Truncated)?;
            cursor += 1;
            value = value.checked_mul(256).ok_or(ShapeError::Truncated)? | usize::from(byte);
        }
        value
    };
    Ok((cursor - position, length, constructed))
}

/// The raw `[0] IMPLICIT SignedAttributes` element of the first SignerInfo, as encoded by
/// the signer. `der` re-sorts SET OF on decode, so this is the only way to see the bytes
/// that were signed. Requires a successful [`prescan`].
fn raw_signed_attributes(der: &[u8]) -> Option<&[u8]> {
    // ContentInfo SEQUENCE → content [0] EXPLICIT → SignedData SEQUENCE.
    let content_info = children(der, 0)?;
    let explicit = *content_info.get(1)?;
    let signed_data_start = children(der, explicit.0)?.first()?.0;
    let fields = children(der, signed_data_start)?;
    // The last field of SignedData is signerInfos (SET).
    let signer_infos = *fields.last()?;
    if der.get(signer_infos.0) != Some(&0x31) {
        return None;
    }
    let signer_info = *children(der, signer_infos.0)?.first()?;
    let signer_fields = children(der, signer_info.0)?;
    // version, sid, digestAlgorithm, then [0] signedAttrs if present.
    let candidate = *signer_fields.get(3)?;
    (der.get(candidate.0) == Some(&0xA0)).then(|| &der[candidate.0..candidate.1])
}

/// `(start, end)` of each child of the constructed element at `position`.
fn children(der: &[u8], position: usize) -> Option<Vec<(usize, usize)>> {
    let (header_length, length, constructed) = header(der, position).ok()?;
    if !constructed {
        return None;
    }
    let mut cursor = position + header_length;
    let end = cursor.checked_add(length)?;
    let mut out = Vec::new();
    while cursor < end {
        let (child_header, child_length, _) = header(der, cursor).ok()?;
        let child_end = cursor
            .checked_add(child_header)?
            .checked_add(child_length)?;
        if child_end > end {
            return None;
        }
        out.push((cursor, child_end));
        cursor = child_end;
        if out.len() > 64 {
            return None;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------------------

/// Verifies the detached CMS in `contents` against the document digest.
pub fn verify_cms(contents: &[u8], document_digest: &mut DocumentDigest<'_>) -> CmsOutcome {
    let outcome = CmsOutcome::new();
    let der_length = match prescan(contents) {
        Ok(length) => length,
        Err(error) => {
            let (status, code) = match error {
                ShapeError::TooDeep | ShapeError::TooManyElements => (
                    CryptographicIntegrityStatus::Malformed,
                    limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
                ),
                _ => (
                    CryptographicIntegrityStatus::Malformed,
                    "SIGNATURE_CMS_PARSE_FAILED",
                ),
            };
            let mut outcome = outcome.fail(status, code, "CMS imza verisi ayrıştırılamadı");
            if let Some(last) = outcome.findings.last_mut() {
                last.context = Some(format!("{error:?}"));
            }
            return outcome;
        }
    };
    let der = &contents[..der_length];
    let Ok(content_info) = ContentInfo::from_der(der) else {
        return outcome.fail(
            CryptographicIntegrityStatus::Malformed,
            "SIGNATURE_CMS_PARSE_FAILED",
            "CMS ContentInfo ayrıştırılamadı",
        );
    };
    if content_info.content_type != ID_SIGNED_DATA {
        return outcome.fail(
            CryptographicIntegrityStatus::Malformed,
            "SIGNATURE_CMS_PARSE_FAILED",
            "CMS içeriği SignedData değil",
        );
    }
    let Ok(signed_data) = content_info
        .content
        .to_der()
        .ok()
        .and_then(|bytes| SignedData::from_der(&bytes).ok())
        .ok_or(())
    else {
        return outcome.fail(
            CryptographicIntegrityStatus::Malformed,
            "SIGNATURE_CMS_PARSE_FAILED",
            "CMS SignedData ayrıştırılamadı",
        );
    };
    verify_signed_data(outcome, der, &signed_data, document_digest)
}

fn verify_signed_data(
    mut outcome: CmsOutcome,
    der: &[u8],
    signed_data: &SignedData,
    document_digest: &mut DocumentDigest<'_>,
) -> CmsOutcome {
    let certificates: Vec<&Certificate> = signed_data
        .certificates
        .as_ref()
        .map(|set| {
            set.0
                .iter()
                .filter_map(|choice| match choice {
                    CertificateChoices::Certificate(certificate) => Some(certificate),
                    CertificateChoices::Other(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if certificates.len() > MAX_CMS_CERTIFICATES {
        return outcome.fail(
            CryptographicIntegrityStatus::Indeterminate,
            limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
            "CMS sertifika sayısı sınırı aşıyor",
        );
    }
    let signer_infos: Vec<&SignerInfo> = signed_data.signer_infos.0.iter().collect();
    let [signer] = signer_infos.as_slice() else {
        return outcome.fail(
            CryptographicIntegrityStatus::Indeterminate,
            "SIGNATURE_INPUT_UNSUPPORTED",
            "CMS tam olarak bir imzalayan içermiyor",
        );
    };
    let signer = *signer;
    if signed_data.encap_content_info.econtent_type != ID_DATA
        || signed_data.encap_content_info.econtent.is_some()
    {
        return outcome.fail(
            CryptographicIntegrityStatus::Malformed,
            "SIGNATURE_CMS_NOT_DETACHED",
            "CMS ayrık (detached) imza değil ya da içerik türü id-data değil",
        );
    }
    outcome.signature_timestamp_present =
        signer.unsigned_attrs.as_ref().is_some_and(|attributes| {
            attributes
                .iter()
                .any(|attribute| attribute.oid == ID_SIGNATURE_TIMESTAMP_TOKEN)
        });

    // Signer certificate (CAP-08).
    let candidates = matching_certificates(&signer.sid, &certificates);
    let certificate = match candidates.as_slice() {
        [] => {
            outcome.signer_certificate = CertificateMatch::Missing;
            None
        }
        [certificate] => {
            outcome.signer_certificate = CertificateMatch::Found;
            outcome.signer_identity = Some(identity(certificate));
            Some(*certificate)
        }
        _ => {
            outcome.signer_certificate = CertificateMatch::Ambiguous;
            None
        }
    };

    // Algorithms and policy (CAP-07).
    let digest = digest_policy(&signer.digest_alg.oid);
    outcome.digest_algorithm = Some(digest.name.to_string());
    outcome.algorithm_policy.digest_policy = digest.policy();
    if !signed_data
        .digest_algorithms
        .iter()
        .any(|algorithm| algorithm.oid == signer.digest_alg.oid)
    {
        outcome.findings.push(ValidationFinding::new(
            "CMS_DIGEST_ALGORITHM_NOT_LISTED",
            FindingSeverity::Warning,
            "İmzalayanın özet algoritması SignedData.digestAlgorithms içinde yok",
        ));
    }
    let scheme = certificate.map(|certificate| signature_scheme(signer, certificate));
    if let Some(scheme) = &scheme {
        outcome.signature_algorithm = Some(scheme.name());
        outcome.algorithm_policy.signature_policy = scheme.policy();
    }

    // Signed attributes (CAP-06).
    let attributes = match signed_attributes(der, signer) {
        Ok(attributes) => attributes,
        Err((code, message)) => {
            return outcome.fail(CryptographicIntegrityStatus::Malformed, code, message);
        }
    };
    if let Some(attributes) = &attributes {
        outcome.cms_signing_time = attributes.signing_time.clone();
        outcome.pades_signed_attributes = attributes.content_type_present
            && attributes.message_digest.is_some()
            && (attributes.signing_certificate_v2.is_some() || attributes.signing_certificate_v1);
    }

    match digest.algorithm {
        DigestChoice::Disallowed => {
            return outcome.fail(
                CryptographicIntegrityStatus::UnsupportedAlgorithm,
                "SIGNATURE_ALGORITHM_FORBIDDEN",
                "İmza yasaklı bir özet algoritması kullanıyor",
            );
        }
        DigestChoice::Unknown => {
            return outcome.fail(
                CryptographicIntegrityStatus::UnsupportedAlgorithm,
                "SIGNATURE_ALGORITHM_UNSUPPORTED",
                "Özet algoritması desteklenmiyor",
            );
        }
        DigestChoice::Supported(_) => {}
    }
    let DigestChoice::Supported(digest_algorithm) = digest.algorithm else {
        unreachable!("handled above");
    };
    let Some(certificate) = certificate else {
        let message = match outcome.signer_certificate {
            CertificateMatch::Ambiguous => "İmzalayan sertifikası için birden fazla aday var",
            _ => "İmzalayan sertifikası CMS içinde bulunamadı",
        };
        return outcome.fail(
            CryptographicIntegrityStatus::Indeterminate,
            "SIGNATURE_SIGNER_NOT_FOUND",
            message,
        );
    };
    let scheme = scheme.unwrap_or(Scheme::Unsupported("unknown".into()));
    match scheme.policy().policy_status {
        AlgorithmPolicyStatus::Allowed => {}
        AlgorithmPolicyStatus::Disallowed => {
            return outcome.fail(
                CryptographicIntegrityStatus::UnsupportedAlgorithm,
                "SIGNATURE_ALGORITHM_FORBIDDEN",
                "İmza algoritması ya da anahtar uzunluğu politika tarafından yasaklı",
            );
        }
        _ => {
            return outcome.fail(
                CryptographicIntegrityStatus::UnsupportedAlgorithm,
                "SIGNATURE_ALGORITHM_UNSUPPORTED",
                "İmza algoritması desteklenmiyor",
            );
        }
    }
    if let Some(mismatch) = scheme.digest_mismatch(digest_algorithm) {
        return outcome.fail(
            CryptographicIntegrityStatus::UnsupportedAlgorithm,
            "SIGNATURE_ALGORITHM_UNSUPPORTED",
            mismatch,
        );
    }

    let document = match document_digest(digest_algorithm) {
        Ok(digest) => digest,
        Err(DigestUnavailable::Digest(DigestError::BudgetExceeded)) => {
            return outcome.fail(
                CryptographicIntegrityStatus::Indeterminate,
                limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
                "Belge için özetlenecek bayt sınırı doldu",
            );
        }
        Err(_) => {
            return outcome.fail(
                CryptographicIntegrityStatus::Indeterminate,
                "SIGNATURE_BYTE_RANGE_INVALID",
                "ByteRange geçersiz olduğu için belge özeti hesaplanamadı",
            );
        }
    };

    let signed_message = match &attributes {
        Some(attributes) => {
            if !attributes.content_type_present {
                return outcome.fail(
                    CryptographicIntegrityStatus::Invalid,
                    "CMS_CONTENT_TYPE_ATTRIBUTE_INVALID",
                    "content-type imzalı niteliği eksik ya da id-data değil",
                );
            }
            let Some(message_digest) = &attributes.message_digest else {
                return outcome.fail(
                    CryptographicIntegrityStatus::Invalid,
                    "SIGNATURE_DIGEST_MISMATCH",
                    "message-digest imzalı niteliği eksik",
                );
            };
            if message_digest.as_slice() != document.as_slice() {
                return outcome.fail(
                    CryptographicIntegrityStatus::Invalid,
                    "SIGNATURE_DIGEST_MISMATCH",
                    "message-digest, ByteRange baytlarının özetiyle eşleşmiyor",
                );
            }
            if let Some(expected) = &attributes.signing_certificate_v2 {
                if let Err(message) = check_signing_certificate_v2(expected, certificate) {
                    return outcome.fail(
                        CryptographicIntegrityStatus::Invalid,
                        "CMS_SIGNING_CERTIFICATE_MISMATCH",
                        message,
                    );
                }
            } else if attributes.signing_certificate_v1 {
                outcome.findings.push(ValidationFinding::new(
                    "CMS_SIGNING_CERTIFICATE_V1_NOT_CHECKED",
                    FindingSeverity::Info,
                    "signing-certificate (v1) SHA-1 özeti kullanır; denetlenmedi",
                ));
            }
            digest_algorithm.hash(&attributes.signed_message)
        }
        None => document,
    };
    match scheme.verify(&signed_message, signer.signature.as_bytes()) {
        Ok(()) => {
            outcome.integrity = CryptographicIntegrityStatus::Valid;
            outcome
        }
        Err(()) => outcome.fail(
            CryptographicIntegrityStatus::Invalid,
            "SIGNATURE_VALUE_INVALID",
            "İmza değeri imzalayanın açık anahtarıyla doğrulanamadı",
        ),
    }
}

struct SignedAttributes {
    content_type_present: bool,
    message_digest: Option<Vec<u8>>,
    signing_time: Option<String>,
    signing_certificate_v2: Option<Vec<EssCertIdV2>>,
    signing_certificate_v1: bool,
    /// DER of the attributes with the SET OF tag: the bytes the signature covers.
    signed_message: Vec<u8>,
}

fn signed_attributes(
    der: &[u8],
    signer: &SignerInfo,
) -> Result<Option<SignedAttributes>, (&'static str, &'static str)> {
    let Some(attributes) = &signer.signed_attrs else {
        return Ok(None);
    };
    let raw = raw_signed_attributes(der)
        .ok_or(("SIGNATURE_CMS_PARSE_FAILED", "İmzalı nitelikler bulunamadı"))?;
    let mut signed_message = raw.to_vec();
    signed_message[0] = 0x31;
    let canonical = attributes.to_der().map_err(|_| {
        (
            "SIGNATURE_CMS_PARSE_FAILED",
            "İmzalı nitelikler yeniden kodlanamadı",
        )
    })?;
    if canonical != signed_message {
        return Err((
            "CMS_SIGNED_ATTRIBUTES_NOT_DER",
            "İmzalı niteliklerin kodlaması kanonik DER değil",
        ));
    }
    let mut out = SignedAttributes {
        content_type_present: false,
        message_digest: None,
        signing_time: None,
        signing_certificate_v2: None,
        signing_certificate_v1: false,
        signed_message,
    };
    let mut seen: Vec<ObjectIdentifier> = Vec::new();
    for attribute in attributes.iter() {
        if seen.contains(&attribute.oid) {
            return Err((
                "SIGNATURE_CMS_PARSE_FAILED",
                "Aynı imzalı nitelik birden fazla kez geçiyor",
            ));
        }
        seen.push(attribute.oid);
        let values: Vec<&Any> = attribute.values.iter().collect();
        let single = || -> Result<&Any, (&'static str, &'static str)> {
            match values.as_slice() {
                [value] => Ok(*value),
                _ => Err((
                    "SIGNATURE_CMS_PARSE_FAILED",
                    "İmzalı nitelik tek değer taşımıyor",
                )),
            }
        };
        if attribute.oid == ID_CONTENT_TYPE {
            out.content_type_present = single()?
                .decode_as::<ObjectIdentifier>()
                .is_ok_and(|oid| oid == ID_DATA);
        } else if attribute.oid == ID_MESSAGE_DIGEST {
            out.message_digest = Some(
                single()?
                    .decode_as::<OctetString>()
                    .map_err(|_| ("SIGNATURE_CMS_PARSE_FAILED", "message-digest okunamadı"))?
                    .as_bytes()
                    .to_vec(),
            );
        } else if attribute.oid == ID_SIGNING_TIME {
            out.signing_time = single()?
                .to_der()
                .ok()
                .and_then(|bytes| x509_cert::time::Time::from_der(&bytes).ok())
                .map(|time| time.to_string());
        } else if attribute.oid == ID_SIGNING_CERTIFICATE_V2 {
            let value = single()?
                .to_der()
                .ok()
                .and_then(|bytes| SigningCertificateV2::from_der(&bytes).ok())
                .ok_or((
                    "SIGNATURE_CMS_PARSE_FAILED",
                    "signing-certificate-v2 okunamadı",
                ))?;
            out.signing_certificate_v2 = Some(value.certs);
        } else if attribute.oid == ID_SIGNING_CERTIFICATE {
            out.signing_certificate_v1 = true;
        }
    }
    Ok(Some(out))
}

/// RFC 5035 `SigningCertificateV2`.
#[derive(Clone, Debug, Eq, PartialEq, der::Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
    #[asn1(optional = "true")]
    policies: Option<Any>,
}

/// RFC 5035 `ESSCertIDv2`; `hashAlgorithm` defaults to SHA-256.
#[derive(Clone, Debug, Eq, PartialEq, der::Sequence)]
struct EssCertIdV2 {
    #[asn1(optional = "true")]
    hash_algorithm: Option<spki::AlgorithmIdentifierOwned>,
    cert_hash: OctetString,
    #[asn1(optional = "true")]
    issuer_serial: Option<Any>,
}

fn check_signing_certificate_v2(
    certs: &[EssCertIdV2],
    certificate: &Certificate,
) -> Result<(), &'static str> {
    let first = certs.first().ok_or("signing-certificate-v2 boş")?;
    let oid = first
        .hash_algorithm
        .as_ref()
        .map_or(ID_SHA256, |algorithm| algorithm.oid);
    let algorithm = match oid {
        oid if oid == ID_SHA256 => DigestAlgorithm::Sha256,
        oid if oid == ID_SHA384 => DigestAlgorithm::Sha384,
        oid if oid == ID_SHA512 => DigestAlgorithm::Sha512,
        _ => return Err("signing-certificate-v2 desteklenmeyen bir özet kullanıyor"),
    };
    let encoded = certificate
        .to_der()
        .map_err(|_| "İmzalayan sertifikası yeniden kodlanamadı")?;
    if algorithm.hash(&encoded).as_slice() == first.cert_hash.as_bytes() {
        Ok(())
    } else {
        Err("signing-certificate-v2 özeti imzalayan sertifikasıyla eşleşmiyor")
    }
}

fn matching_certificates<'a>(
    sid: &SignerIdentifier,
    certificates: &[&'a Certificate],
) -> Vec<&'a Certificate> {
    let mut found: Vec<&Certificate> = Vec::new();
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for certificate in certificates {
        let tbs = certificate.tbs_certificate();
        let matches = match sid {
            SignerIdentifier::IssuerAndSerialNumber(isn) => {
                tbs.issuer() == &isn.issuer && tbs.serial_number() == &isn.serial_number
            }
            SignerIdentifier::SubjectKeyIdentifier(ski) => tbs
                .get_extension::<x509_cert::ext::pkix::SubjectKeyIdentifier>()
                .ok()
                .flatten()
                .is_some_and(|(_, extension)| &extension == ski),
        };
        if !matches {
            continue;
        }
        // The same certificate listed twice is one candidate.
        let encoded = certificate.to_der().unwrap_or_default();
        if !seen.contains(&encoded) {
            seen.push(encoded);
            found.push(certificate);
        }
    }
    found
}

fn identity(certificate: &Certificate) -> SignerIdentityReport {
    let tbs = certificate.tbs_certificate();
    let subject = tbs.subject();
    let component = |oid: ObjectIdentifier| -> Option<String> {
        subject.iter().find_map(|attribute| {
            (attribute.oid == oid)
                .then(|| directory_string(&attribute.value))
                .flatten()
                .map(|value| limits::trim(&value, limits::MAX_NAME_COMPONENT_CHARS))
        })
    };
    let encoded = certificate.to_der().unwrap_or_default();
    SignerIdentityReport {
        common_name: component(ID_COMMON_NAME),
        organization: component(ID_ORGANIZATION),
        country: component(ID_COUNTRY),
        certificate_serial: crate::hex_lower(tbs.serial_number().as_bytes()),
        certificate_issuer: limits::trim(&tbs.issuer().to_string(), limits::MAX_CONTEXT_CHARS),
        certificate_validity_start: tbs.validity().not_before.to_string(),
        certificate_validity_end: tbs.validity().not_after.to_string(),
        certificate_sha256_fingerprint: crate::hex_lower(DigestAlgorithm::Sha256.hash(&encoded)),
    }
}

fn directory_string(value: &Any) -> Option<String> {
    let bytes = value.value();
    match value.tag() {
        Tag::Utf8String | Tag::PrintableString | Tag::Ia5String | Tag::VisibleString => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        Tag::TeletexString => Some(bytes.iter().map(|byte| char::from(*byte)).collect()),
        Tag::BmpString => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// Algorithm policy
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DigestChoice {
    Supported(DigestAlgorithm),
    Disallowed,
    Unknown,
}

struct DigestPolicy {
    name: &'static str,
    algorithm: DigestChoice,
}

impl DigestPolicy {
    fn policy(&self) -> SingleAlgorithmPolicy {
        let (status, references) = match self.algorithm {
            DigestChoice::Supported(_) => (
                AlgorithmPolicyStatus::Allowed,
                vec![AlgorithmStandardReference::FipsPub180_4],
            ),
            DigestChoice::Disallowed => (AlgorithmPolicyStatus::Disallowed, Vec::new()),
            DigestChoice::Unknown => (
                AlgorithmPolicyStatus::Indeterminate,
                vec![AlgorithmStandardReference::Unstandardized],
            ),
        };
        SingleAlgorithmPolicy {
            algorithm_identifier: self.name.to_string(),
            policy_status: status,
            normative_references: references,
        }
    }
}

fn digest_policy(oid: &ObjectIdentifier) -> DigestPolicy {
    let (name, algorithm) = match *oid {
        oid if oid == ID_SHA256 => ("SHA-256", DigestChoice::Supported(DigestAlgorithm::Sha256)),
        oid if oid == ID_SHA384 => ("SHA-384", DigestChoice::Supported(DigestAlgorithm::Sha384)),
        oid if oid == ID_SHA512 => ("SHA-512", DigestChoice::Supported(DigestAlgorithm::Sha512)),
        oid if oid == ID_SHA1 => ("SHA-1", DigestChoice::Disallowed),
        oid if oid == ID_MD5 => ("MD5", DigestChoice::Disallowed),
        _ => ("unknown", DigestChoice::Unknown),
    };
    DigestPolicy { name, algorithm }
}

#[derive(Debug, Clone)]
enum Scheme {
    RsaPkcs1v15 {
        key: rsa::PublicKey,
        bits: usize,
        hash: Option<DigestChoice>,
    },
    RsaPss {
        key: rsa::PublicKey,
        bits: usize,
        hash: DigestChoice,
        salt: usize,
    },
    EcdsaP256 {
        point: Vec<u8>,
        hash: Option<DigestChoice>,
    },
    EcdsaP384 {
        point: Vec<u8>,
        hash: Option<DigestChoice>,
    },
    /// RSA key outside the policy range; never verified.
    WeakRsa {
        bits: usize,
    },
    Unsupported(String),
}

impl Scheme {
    fn name(&self) -> String {
        match self {
            Self::RsaPkcs1v15 { bits, .. } => format!("RSA-PKCS1v15-{bits}"),
            Self::RsaPss { bits, .. } => format!("RSA-PSS-{bits}"),
            Self::EcdsaP256 { .. } => "ECDSA-P256".into(),
            Self::EcdsaP384 { .. } => "ECDSA-P384".into(),
            Self::WeakRsa { bits } => format!("RSA-{bits}"),
            Self::Unsupported(name) => limits::trim(name, 64),
        }
    }

    fn policy(&self) -> SingleAlgorithmPolicy {
        let (status, references) = match self {
            Self::RsaPkcs1v15 { hash, .. }
            | Self::EcdsaP256 { hash, .. }
            | Self::EcdsaP384 { hash, .. }
                if matches!(hash, Some(DigestChoice::Disallowed)) =>
            {
                (AlgorithmPolicyStatus::Disallowed, Vec::new())
            }
            Self::RsaPkcs1v15 { .. } | Self::RsaPss { .. } => (
                AlgorithmPolicyStatus::Allowed,
                vec![AlgorithmStandardReference::Pkcs1v2_2],
            ),
            Self::EcdsaP256 { .. } | Self::EcdsaP384 { .. } => (
                AlgorithmPolicyStatus::Allowed,
                vec![
                    AlgorithmStandardReference::Fips186_5,
                    AlgorithmStandardReference::NistSp800_186,
                    AlgorithmStandardReference::AnsiX9_62,
                ],
            ),
            Self::WeakRsa { .. } => (AlgorithmPolicyStatus::Disallowed, Vec::new()),
            Self::Unsupported(_) => (
                AlgorithmPolicyStatus::Indeterminate,
                vec![AlgorithmStandardReference::Unstandardized],
            ),
        };
        SingleAlgorithmPolicy {
            algorithm_identifier: self.name(),
            policy_status: status,
            normative_references: references,
        }
    }

    /// A signature OID that names a hash must name the signer's digest algorithm.
    fn digest_mismatch(&self, digest: DigestAlgorithm) -> Option<&'static str> {
        let named = match self {
            Self::RsaPkcs1v15 { hash, .. }
            | Self::EcdsaP256 { hash, .. }
            | Self::EcdsaP384 { hash, .. } => *hash,
            Self::RsaPss { hash, .. } => Some(*hash),
            _ => None,
        };
        match named {
            None => None,
            Some(DigestChoice::Supported(named)) if named == digest => None,
            Some(_) => Some("İmza algoritmasının özeti imzalayanın özet algoritmasıyla aynı değil"),
        }
    }

    fn verify(&self, message_digest: &[u8], signature: &[u8]) -> Result<(), ()> {
        use rsa::{pkcs1v15, pss};
        let digest = match self {
            Self::RsaPkcs1v15 { hash, .. }
            | Self::EcdsaP256 { hash, .. }
            | Self::EcdsaP384 { hash, .. } => *hash,
            Self::RsaPss { hash, .. } => Some(*hash),
            _ => None,
        };
        // When the signature OID names no hash (rsaEncryption, ecPublicKey) the length of the
        // digest tells it; the caller already matched it to the signer's digest algorithm.
        let algorithm = match digest {
            Some(DigestChoice::Supported(algorithm)) => algorithm,
            Some(_) => return Err(()),
            None => match message_digest.len() {
                32 => DigestAlgorithm::Sha256,
                48 => DigestAlgorithm::Sha384,
                64 => DigestAlgorithm::Sha512,
                _ => return Err(()),
            },
        };
        match self {
            Self::RsaPkcs1v15 { key, .. } => {
                let signature = pkcs1v15::Signature::try_from(signature).map_err(|_| ())?;
                match algorithm {
                    DigestAlgorithm::Sha256 => {
                        pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key.clone())
                            .verify_prehash(message_digest, &signature)
                    }
                    DigestAlgorithm::Sha384 => {
                        pkcs1v15::VerifyingKey::<sha2::Sha384>::new(key.clone())
                            .verify_prehash(message_digest, &signature)
                    }
                    DigestAlgorithm::Sha512 => {
                        pkcs1v15::VerifyingKey::<sha2::Sha512>::new(key.clone())
                            .verify_prehash(message_digest, &signature)
                    }
                }
                .map_err(|_| ())
            }
            Self::RsaPss { key, salt, .. } => {
                let signature = pss::Signature::try_from(signature).map_err(|_| ())?;
                match algorithm {
                    DigestAlgorithm::Sha256 => {
                        pss::VerifyingKey::<sha2::Sha256>::new_with_salt_len(key.clone(), *salt)
                            .verify_prehash(message_digest, &signature)
                    }
                    DigestAlgorithm::Sha384 => {
                        pss::VerifyingKey::<sha2::Sha384>::new_with_salt_len(key.clone(), *salt)
                            .verify_prehash(message_digest, &signature)
                    }
                    DigestAlgorithm::Sha512 => {
                        pss::VerifyingKey::<sha2::Sha512>::new_with_salt_len(key.clone(), *salt)
                            .verify_prehash(message_digest, &signature)
                    }
                }
                .map_err(|_| ())
            }
            Self::EcdsaP256 { point, .. } => {
                let key = ecdsa::VerifyingKey::<p256::NistP256>::from_sec1_bytes(point)
                    .map_err(|_| ())?;
                let signature = ecdsa::der::Signature::<p256::NistP256>::from_bytes(signature)
                    .map_err(|_| ())?;
                key.verify_prehash(message_digest, &signature)
                    .map_err(|_| ())
            }
            Self::EcdsaP384 { point, .. } => {
                let key = ecdsa::VerifyingKey::<p384::NistP384>::from_sec1_bytes(point)
                    .map_err(|_| ())?;
                let signature = ecdsa::der::Signature::<p384::NistP384>::from_bytes(signature)
                    .map_err(|_| ())?;
                key.verify_prehash(message_digest, &signature)
                    .map_err(|_| ())
            }
            Self::WeakRsa { .. } | Self::Unsupported(_) => Err(()),
        }
    }
}

fn signature_scheme(signer: &SignerInfo, certificate: &Certificate) -> Scheme {
    let algorithm = &signer.signature_algorithm;
    let spki = certificate.tbs_certificate().subject_public_key_info();
    let key_bits = spki.subject_public_key.raw_bytes();
    let named_hash = |oid: ObjectIdentifier| -> Option<DigestChoice> {
        match oid {
            oid if oid == SHA256_WITH_RSA || oid == ECDSA_WITH_SHA256 => {
                Some(DigestChoice::Supported(DigestAlgorithm::Sha256))
            }
            oid if oid == SHA384_WITH_RSA || oid == ECDSA_WITH_SHA384 => {
                Some(DigestChoice::Supported(DigestAlgorithm::Sha384))
            }
            oid if oid == SHA512_WITH_RSA || oid == ECDSA_WITH_SHA512 => {
                Some(DigestChoice::Supported(DigestAlgorithm::Sha512))
            }
            oid if oid == SHA1_WITH_RSA || oid == MD5_WITH_RSA || oid == ECDSA_WITH_SHA1 => {
                Some(DigestChoice::Disallowed)
            }
            _ => None,
        }
    };
    let oid = algorithm.oid;
    let is_rsa = oid == RSA_ENCRYPTION
        || oid == SHA1_WITH_RSA
        || oid == MD5_WITH_RSA
        || oid == SHA256_WITH_RSA
        || oid == SHA384_WITH_RSA
        || oid == SHA512_WITH_RSA;
    let is_ecdsa = oid == EC_PUBLIC_KEY
        || oid == ECDSA_WITH_SHA1
        || oid == ECDSA_WITH_SHA256
        || oid == ECDSA_WITH_SHA384
        || oid == ECDSA_WITH_SHA512;
    if is_rsa || oid == RSASSA_PSS {
        if spki.algorithm.oid != RSA_ENCRYPTION {
            return Scheme::Unsupported("RSA key with non-rsaEncryption SPKI".into());
        }
        let Some(bits) = rsa_modulus_bits(key_bits) else {
            return Scheme::Unsupported("malformed RSA key".into());
        };
        if bits < MIN_RSA_MODULUS_BITS {
            return Scheme::WeakRsa { bits };
        }
        if bits > limits::MAX_RSA_MODULUS_BITS {
            return Scheme::Unsupported(format!("RSA-{bits}"));
        }
        let Ok(key) = <rsa::PublicKey as rsa::pkcs8::DecodePublicKey>::from_public_key_der(
            &spki.to_der().unwrap_or_default(),
        ) else {
            return Scheme::Unsupported("malformed RSA key".into());
        };
        if oid == RSASSA_PSS {
            return match pss_parameters(algorithm.parameters.as_ref()) {
                Some((hash, salt)) => Scheme::RsaPss {
                    key,
                    bits,
                    hash,
                    salt,
                },
                None => Scheme::Unsupported("RSA-PSS parameters".into()),
            };
        }
        return Scheme::RsaPkcs1v15 {
            key,
            bits,
            hash: named_hash(oid),
        };
    }
    if is_ecdsa {
        if spki.algorithm.oid != EC_PUBLIC_KEY {
            return Scheme::Unsupported("ECDSA key with non-ecPublicKey SPKI".into());
        }
        let curve = spki
            .algorithm
            .parameters
            .as_ref()
            .and_then(|parameters| parameters.decode_as::<ObjectIdentifier>().ok());
        return match curve {
            Some(curve) if curve == CURVE_P256 => Scheme::EcdsaP256 {
                point: key_bits.to_vec(),
                hash: named_hash(oid),
            },
            Some(curve) if curve == CURVE_P384 => Scheme::EcdsaP384 {
                point: key_bits.to_vec(),
                hash: named_hash(oid),
            },
            Some(curve) => Scheme::Unsupported(format!("ECDSA curve {curve}")),
            None => Scheme::Unsupported("ECDSA curve".into()),
        };
    }
    Scheme::Unsupported(format!("signature algorithm {oid}"))
}

fn rsa_modulus_bits(key_bits: &[u8]) -> Option<usize> {
    let key = rsa::pkcs1::RsaPublicKeyRef::try_from(key_bits).ok()?;
    let modulus = key.modulus.as_bytes();
    let first = modulus.iter().position(|byte| *byte != 0)?;
    let significant = &modulus[first..];
    Some(significant.len() * 8 - significant[0].leading_zeros() as usize)
}

/// `(hash, salt length)` of RSASSA-PSS parameters; the MGF must be MGF1 with the same hash
/// and the trailer field 1 (RFC 4055).
fn pss_parameters(parameters: Option<&Any>) -> Option<(DigestChoice, usize)> {
    let encoded = parameters?.to_der().ok()?;
    pss_from_der(&encoded)
}

fn pss_from_der(encoded: &[u8]) -> Option<(DigestChoice, usize)> {
    let params: rsa::pkcs1::RsaPssParams<der::Any> =
        rsa::pkcs1::RsaPssParams::try_from(encoded).ok()?;
    let hash = digest_policy(&params.hash.oid).algorithm;
    if params.mask_gen.oid != ID_MGF1 {
        return None;
    }
    let mgf_hash = params.mask_gen.parameters.as_ref()?.oid;
    if mgf_hash != params.hash.oid {
        return None;
    }
    if params.trailer_field != rsa::pkcs1::TrailerField::BC {
        return None;
    }
    Some((hash, usize::from(params.salt_len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prescan_measures_the_outer_element_and_allows_zero_padding() {
        let der = [0x30, 0x03, 0x02, 0x01, 0x05];
        assert_eq!(prescan(&der), Ok(5));
        let mut padded = der.to_vec();
        padded.extend_from_slice(&[0; 100]);
        assert_eq!(prescan(&padded), Ok(5));
        padded.push(1);
        assert_eq!(prescan(&padded), Err(ShapeError::TrailingData));
    }

    #[test]
    fn prescan_rejects_hostile_shapes() {
        assert_eq!(prescan(&[]), Err(ShapeError::Empty));
        assert_eq!(prescan(&[0; 16]), Err(ShapeError::Empty));
        assert_eq!(
            prescan(&[0x30, 0x80, 0x00, 0x00]),
            Err(ShapeError::IndefiniteLength)
        );
        assert_eq!(
            prescan(&[0x30, 0x05, 0x02, 0x01]),
            Err(ShapeError::Truncated)
        );
        // A child longer than its parent.
        assert_eq!(
            prescan(&[0x30, 0x03, 0x30, 0x05, 0x00]),
            Err(ShapeError::Truncated)
        );
        // A length that does not fit.
        assert_eq!(
            prescan(&[0x30, 0x89, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
            Err(ShapeError::Truncated)
        );
        // Nesting deeper than the limit.
        let depth = MAX_DER_DEPTH + 1;
        let mut nested = Vec::new();
        for level in 0..depth {
            let remaining = depth - level - 1;
            nested.extend_from_slice(&[0x30, 0x81, (remaining * 3) as u8]);
        }
        assert_eq!(prescan(&nested), Err(ShapeError::TooDeep));
    }

    #[test]
    fn prescan_counts_elements() {
        let count = MAX_DER_ELEMENTS + 1;
        let content_length = count * 2;
        let mut bytes = vec![0x30, 0x83];
        bytes.extend_from_slice(&(content_length as u32).to_be_bytes()[1..]);
        for _ in 0..count {
            bytes.extend_from_slice(&[0x05, 0x00]);
        }
        assert_eq!(prescan(&bytes), Err(ShapeError::TooManyElements));
    }

    #[test]
    fn garbage_cms_is_malformed_not_a_panic() {
        let mut never = |_: DigestAlgorithm| -> Result<Vec<u8>, DigestUnavailable> {
            panic!("digest must not be requested for malformed CMS")
        };
        for input in [
            &[0x30, 0x03, 0x02, 0x01, 0x05][..],
            &[0x30, 0x00],
            &[0x04, 0x02, 0xff, 0xff],
            &[
                0x30, 0x0b, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02,
            ],
        ] {
            let outcome = verify_cms(input, &mut never);
            assert_eq!(
                outcome.integrity,
                CryptographicIntegrityStatus::Malformed,
                "{input:02x?}"
            );
            assert!(!outcome.findings.is_empty());
        }
    }
}
