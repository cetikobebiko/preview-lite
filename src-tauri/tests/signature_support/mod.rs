//! TEST ONLY deterministic generator of signed PDF fixtures (Prompt 11 G2, Gate 1 §H option 1).
//!
//! Everything here is test code: production never signs (pades-architecture-design.md §19).
//! Keys are fixed: RSA keys are the committed `fixtures/signature/keys/TEST-ONLY-*.pkcs1.der`
//! files, EC keys are scalars derived from fixed labels. Signatures are deterministic:
//! RSA PKCS#1 v1.5, RSA-PSS with a fixed salt source, ECDSA with RFC 6979 nonces. No `rand`.
//! Certificates and CMS `SignedData` are encoded field by field with `der` (no builders).
//!
//! The PDFs use classic xref tables and fixed-width ByteRange placeholders, so the bytes of
//! every fixture depend only on this file and the keys.
#![allow(dead_code)]

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use const_oid::ObjectIdentifier;
use der::asn1::{Any, BitString, OctetString, SetOfVec, Uint, UtcTime};
use der::{Decode, Encode, Sequence};
use ecdsa::signature::hazmat::{PrehashSigner, RandomizedPrehashSigner};
use sha2::Digest;
use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use x509_cert::attr::Attribute;
use x509_cert::name::Name;
use x509_cert::time::Time;
use x509_cert::Certificate;

pub const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
pub const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const ID_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const ID_SIGNATURE_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
const ID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const ID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const ID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const SHA1_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const RSASSA_PSS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");
const EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_WITH_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const CURVE_P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const CURVE_P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("signature")
}

// ---------------------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------------------

#[derive(Clone)]
pub enum Key {
    Rsa(rsa::PrivateKey),
    P256(ecdsa::SigningKey<p256::NistP256>),
    P384(ecdsa::SigningKey<p384::NistP384>),
}

pub fn rsa_key(name: &str) -> Key {
    let path = fixtures_dir()
        .join("keys")
        .join(format!("TEST-ONLY-{name}.pkcs1.der"));
    let der = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    Key::Rsa(<rsa::PrivateKey as rsa::pkcs1::DecodeRsaPrivateKey>::from_pkcs1_der(&der).unwrap())
}

pub fn p256_key(label: &str) -> Key {
    let scalar = sha2::Sha256::digest(format!("TEST ONLY P-256 {label}").as_bytes());
    Key::P256(ecdsa::SigningKey::from_bytes(&scalar).unwrap())
}

pub fn p384_key(label: &str) -> Key {
    let scalar = sha2::Sha384::digest(format!("TEST ONLY P-384 {label}").as_bytes());
    Key::P384(ecdsa::SigningKey::from_bytes(&scalar).unwrap())
}

impl Key {
    fn spki(&self) -> SubjectPublicKeyInfoOwned {
        match self {
            Key::Rsa(key) => {
                let document =
                    rsa::pkcs8::EncodePublicKey::to_public_key_der(&key.to_public_key()).unwrap();
                SubjectPublicKeyInfoOwned::from_der(document.as_bytes()).unwrap()
            }
            Key::P256(key) => ec_spki(
                CURVE_P256,
                key.verifying_key().to_sec1_point(false).as_bytes(),
            ),
            Key::P384(key) => ec_spki(
                CURVE_P384,
                key.verifying_key().to_sec1_point(false).as_bytes(),
            ),
        }
    }

    fn certificate_algorithm(&self) -> AlgorithmIdentifierOwned {
        match self {
            Key::Rsa(_) => algorithm(SHA256_WITH_RSA, Some(Any::null())),
            Key::P256(_) | Key::P384(_) => algorithm(ECDSA_WITH_SHA256, None),
        }
    }

    /// Signs a SHA-256 prehash for certificates.
    fn sign_certificate(&self, tbs: &[u8]) -> Vec<u8> {
        let prehash = sha2::Sha256::digest(tbs);
        self.sign_prehash(Scheme::Pkcs1v15, Hash::Sha256, &prehash)
    }

    fn sign_prehash(&self, scheme: Scheme, hash: Hash, prehash: &[u8]) -> Vec<u8> {
        use rsa::{pkcs1v15, pss};
        match (self, scheme) {
            (Key::Rsa(key), Scheme::Pkcs1v15) => {
                let signature = match hash {
                    Hash::Sha256 => {
                        pkcs1v15::SigningKey::<sha2::Sha256>::new(key.clone()).sign_prehash(prehash)
                    }
                    Hash::Sha384 => {
                        pkcs1v15::SigningKey::<sha2::Sha384>::new(key.clone()).sign_prehash(prehash)
                    }
                    Hash::Sha512 => {
                        pkcs1v15::SigningKey::<sha2::Sha512>::new(key.clone()).sign_prehash(prehash)
                    }
                    // SHA-1 fixtures are rejected by policy before verification; any bytes do.
                    Hash::Sha1 => pkcs1v15::SigningKey::<sha2::Sha256>::new_unprefixed(key.clone())
                        .sign_prehash(prehash),
                };
                rsa::signature::SignatureEncoding::to_vec(&signature.unwrap())
            }
            (Key::Rsa(key), Scheme::Pss { salt }) => {
                let mut rng = FixedSalt(0);
                let signature = match hash {
                    Hash::Sha384 => {
                        pss::SigningKey::<sha2::Sha384>::new_with_salt_len(key.clone(), salt)
                            .sign_prehash_with_rng(&mut rng, prehash)
                    }
                    _ => pss::SigningKey::<sha2::Sha256>::new_with_salt_len(key.clone(), salt)
                        .sign_prehash_with_rng(&mut rng, prehash),
                };
                rsa::signature::SignatureEncoding::to_vec(&signature.unwrap())
            }
            (Key::P256(key), _) => {
                let signature: ecdsa::der::Signature<p256::NistP256> =
                    key.sign_prehash(prehash).unwrap();
                signature.as_bytes().to_vec()
            }
            (Key::P384(key), _) => {
                let signature: ecdsa::der::Signature<p384::NistP384> =
                    key.sign_prehash(prehash).unwrap();
                signature.as_bytes().to_vec()
            }
        }
    }
}

/// Deterministic "RNG" for the PSS salt: a counter. TEST ONLY.
struct FixedSalt(u8);

impl rsa::rand_core::TryRng for FixedSalt {
    type Error = core::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut bytes = [0u8; 4];
        self.try_fill_bytes(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let mut bytes = [0u8; 8];
        self.try_fill_bytes(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        for byte in dst {
            *byte = self.0;
            self.0 = self.0.wrapping_add(1);
        }
        Ok(())
    }
}

impl rsa::rand_core::TryCryptoRng for FixedSalt {}

fn ec_spki(curve: ObjectIdentifier, point: &[u8]) -> SubjectPublicKeyInfoOwned {
    SubjectPublicKeyInfoOwned {
        algorithm: algorithm(EC_PUBLIC_KEY, Some(Any::encode_from(&curve).unwrap())),
        subject_public_key: BitString::from_bytes(point).unwrap(),
    }
}

fn algorithm(oid: ObjectIdentifier, parameters: Option<Any>) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned { oid, parameters }
}

// ---------------------------------------------------------------------------------------
// Certificates
// ---------------------------------------------------------------------------------------

#[derive(Sequence)]
struct TbsCertificate {
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT")]
    version: u8,
    serial: Uint,
    signature: AlgorithmIdentifierOwned,
    issuer: Name,
    validity: Validity,
    subject: Name,
    subject_public_key_info: SubjectPublicKeyInfoOwned,
}

#[derive(Sequence)]
struct Validity {
    not_before: Time,
    not_after: Time,
}

#[derive(Sequence)]
struct RawCertificate {
    tbs: Any,
    signature_algorithm: AlgorithmIdentifierOwned,
    signature: BitString,
}

fn utc(year: u16) -> Time {
    Time::UtcTime(
        UtcTime::from_date_time(der::DateTime::new(year, 1, 1, 0, 0, 0).unwrap()).unwrap(),
    )
}

/// A self-signed TEST ONLY certificate for `subject_key`, signed by `issuer_key`.
pub fn certificate(
    common_name: &str,
    serial: u8,
    subject_key: &Key,
    issuer_key: &Key,
) -> Certificate {
    let name = Name::from_str(&format!("CN={common_name},O=TEST ONLY,C=TR")).unwrap();
    let tbs = TbsCertificate {
        version: 2,
        serial: Uint::new(&[serial]).unwrap(),
        signature: issuer_key.certificate_algorithm(),
        issuer: name.clone(),
        validity: Validity {
            not_before: utc(2026),
            not_after: utc(2036),
        },
        subject: name,
        subject_public_key_info: subject_key.spki(),
    };
    let tbs = tbs.to_der().unwrap();
    let signature = issuer_key.sign_certificate(&tbs);
    let raw = RawCertificate {
        tbs: Any::from_der(&tbs).unwrap(),
        signature_algorithm: issuer_key.certificate_algorithm(),
        signature: BitString::from_bytes(&signature).unwrap(),
    }
    .to_der()
    .unwrap();
    Certificate::from_der(&raw).unwrap()
}

// ---------------------------------------------------------------------------------------
// CMS
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn oid(self) -> ObjectIdentifier {
        match self {
            Hash::Sha1 => ID_SHA1,
            Hash::Sha256 => ID_SHA256,
            Hash::Sha384 => ID_SHA384,
            Hash::Sha512 => ID_SHA512,
        }
    }

    pub fn digest(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            // SHA-1 is not computed anywhere (§20); a fixed stand-in of the right length.
            Hash::Sha1 => vec![0x5a; 20],
            Hash::Sha256 => sha2::Sha256::digest(bytes).to_vec(),
            Hash::Sha384 => sha2::Sha384::digest(bytes).to_vec(),
            Hash::Sha512 => sha2::Sha512::digest(bytes).to_vec(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    Pkcs1v15,
    Pss { salt: usize },
}

#[derive(Clone)]
pub struct CmsSpec {
    pub key: Key,
    pub certificate: Certificate,
    /// Certificates embedded in the CMS; defaults to `[certificate]`.
    pub embedded: Option<Vec<Certificate>>,
    pub hash: Hash,
    pub scheme: Scheme,
    /// Signature algorithm OID; defaults per key.
    pub signature_oid: Option<ObjectIdentifier>,
    pub signed_attributes: bool,
    pub content_type: bool,
    pub signing_certificate_v2: bool,
    pub message_digest_override: Option<Vec<u8>>,
    /// Signs with this key instead (the certificate stays `certificate`).
    pub signing_key_override: Option<Key>,
    pub corrupt_signature: bool,
    pub signature_timestamp_attribute: bool,
}

impl CmsSpec {
    pub fn new(key: Key, certificate: Certificate) -> Self {
        Self {
            key,
            certificate,
            embedded: None,
            hash: Hash::Sha256,
            scheme: Scheme::Pkcs1v15,
            signature_oid: None,
            signed_attributes: true,
            content_type: true,
            signing_certificate_v2: true,
            message_digest_override: None,
            signing_key_override: None,
            corrupt_signature: false,
            signature_timestamp_attribute: false,
        }
    }
}

fn attribute(oid: ObjectIdentifier, value: Any) -> Attribute {
    Attribute {
        oid,
        values: SetOfVec::try_from(vec![value]).unwrap(),
    }
}

#[derive(Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
}

/// `hashAlgorithm` omitted: SHA-256 by default.
#[derive(Sequence)]
struct EssCertIdV2 {
    cert_hash: OctetString,
}

/// Detached CMS over `covered` (the ByteRange bytes).
pub fn build_cms(spec: &CmsSpec, covered: &[u8]) -> Vec<u8> {
    let document_digest = spec.hash.digest(covered);
    let signature_oid = spec
        .signature_oid
        .unwrap_or(match (&spec.key, spec.scheme, spec.hash) {
            (Key::Rsa(_), Scheme::Pss { .. }, _) => RSASSA_PSS,
            (Key::Rsa(_), _, Hash::Sha1) => SHA1_WITH_RSA,
            (Key::Rsa(_), _, _) => RSA_ENCRYPTION,
            (_, _, Hash::Sha384) => ECDSA_WITH_SHA384,
            _ => ECDSA_WITH_SHA256,
        });
    let signature_parameters = match spec.scheme {
        Scheme::Pss { salt } => {
            let params = match spec.hash {
                Hash::Sha384 => rsa::pkcs1::RsaPssParamsRef::new::<sha2::Sha384>(salt as u8),
                _ => rsa::pkcs1::RsaPssParamsRef::new::<sha2::Sha256>(salt as u8),
            };
            Some(Any::encode_from(&params).unwrap())
        }
        Scheme::Pkcs1v15 if matches!(spec.key, Key::Rsa(_)) => Some(Any::null()),
        Scheme::Pkcs1v15 => None,
    };
    let signed_attrs = spec.signed_attributes.then(|| {
        let mut attributes = Vec::new();
        if spec.content_type {
            attributes.push(attribute(
                ID_CONTENT_TYPE,
                Any::encode_from(&ID_DATA).unwrap(),
            ));
        }
        let digest = spec
            .message_digest_override
            .clone()
            .unwrap_or_else(|| document_digest.clone());
        attributes.push(attribute(
            ID_MESSAGE_DIGEST,
            Any::encode_from(&OctetString::new(digest).unwrap()).unwrap(),
        ));
        attributes.push(attribute(
            ID_SIGNING_TIME,
            Any::encode_from(&utc(2026)).unwrap(),
        ));
        if spec.signing_certificate_v2 {
            let certificate = spec.certificate.to_der().unwrap();
            let value = SigningCertificateV2 {
                certs: vec![EssCertIdV2 {
                    cert_hash: OctetString::new(sha2::Sha256::digest(&certificate).to_vec())
                        .unwrap(),
                }],
            };
            attributes.push(attribute(
                ID_SIGNING_CERTIFICATE_V2,
                Any::encode_from(&value).unwrap(),
            ));
        }
        SetOfVec::try_from(attributes).unwrap()
    });
    let prehash = match &signed_attrs {
        Some(attributes) => spec.hash.digest(&attributes.to_der().unwrap()),
        None => document_digest,
    };
    let key = spec.signing_key_override.as_ref().unwrap_or(&spec.key);
    let mut signature = key.sign_prehash(spec.scheme, spec.hash, &prehash);
    if spec.corrupt_signature {
        let middle = signature.len() / 2;
        signature[middle] ^= 0x01;
    }
    let unsigned_attrs = spec.signature_timestamp_attribute.then(|| {
        // Presence marker only (G5 verifies tokens): an empty SEQUENCE as the token.
        SetOfVec::try_from(vec![attribute(
            ID_SIGNATURE_TIMESTAMP_TOKEN,
            Any::from_der(&[0x30, 0x00]).unwrap(),
        )])
        .unwrap()
    });
    let tbs = spec.certificate.tbs_certificate();
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: tbs.issuer().clone(),
            serial_number: tbs.serial_number().clone(),
        }),
        digest_alg: algorithm(spec.hash.oid(), None),
        signed_attrs,
        signature_algorithm: algorithm(signature_oid, signature_parameters),
        signature: OctetString::new(signature).unwrap(),
        unsigned_attrs,
    };
    let embedded = spec
        .embedded
        .clone()
        .unwrap_or_else(|| vec![spec.certificate.clone()]);
    let certificates = (!embedded.is_empty()).then(|| {
        CertificateSet(
            SetOfVec::try_from(
                embedded
                    .into_iter()
                    .map(CertificateChoices::Certificate)
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        )
    });
    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![algorithm(spec.hash.oid(), None)]).unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: ID_DATA,
            econtent: None,
        },
        certificates,
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer]).unwrap()),
    };
    ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: Any::encode_from(&signed_data).unwrap(),
    }
    .to_der()
    .unwrap()
}

// ---------------------------------------------------------------------------------------
// PDF writer
// ---------------------------------------------------------------------------------------

/// `/Contents` placeholder size in bytes (hex digits = 2×).
pub const CONTENTS_BYTES: usize = 4096;
const RANGE_WIDTH: usize = 10;

/// A PDF under construction: whole revisions with classic xref tables.
pub struct Pdf {
    pub bytes: Vec<u8>,
    last_xref: Option<usize>,
    size: u32,
}

impl Pdf {
    pub fn new() -> Self {
        Self {
            bytes: b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            last_xref: None,
            size: 1,
        }
    }

    /// Appends a revision with `objects` (number, body) and its xref/trailer.
    pub fn revision(&mut self, objects: &[(u32, String)], root: u32) {
        let mut entries = Vec::new();
        for (number, body) in objects {
            entries.push((*number, self.bytes.len()));
            self.bytes
                .extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
            self.size = self.size.max(number + 1);
        }
        let xref = self.bytes.len();
        let mut table = String::from("xref\n");
        if self.last_xref.is_none() {
            table.push_str("0 1\n0000000000 65535 f \n");
        }
        entries.sort();
        for (number, offset) in &entries {
            table.push_str(&format!("{number} 1\n{offset:010} 00000 n \n"));
        }
        let prev = self
            .last_xref
            .map(|offset| format!(" /Prev {offset}"))
            .unwrap_or_default();
        table.push_str(&format!(
            "trailer\n<< /Size {} /Root {root} 0 R{prev} >>\nstartxref\n{xref}\n%%EOF\n",
            self.size
        ));
        self.bytes.extend_from_slice(table.as_bytes());
        self.last_xref = Some(xref);
    }
}

/// The signature dictionary body with ByteRange and `/Contents` placeholders.
pub fn signature_dictionary(
    kind: &str,
    sub_filter: &str,
    extra: &str,
    slots: usize,
    contents_bytes: usize,
) -> String {
    let range = (0..slots)
        .map(|index| {
            if index == 0 {
                "0".repeat(RANGE_WIDTH)
            } else {
                "#".repeat(RANGE_WIDTH)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<< /Type /{kind} /Filter /Adobe.PPKLite /SubFilter /{sub_filter} /ByteRange [{range}] /Contents <{}> /M (D:20260101000000Z){extra} >>",
        "0".repeat(contents_bytes * 2)
    )
}

/// Fills the last ByteRange/Contents placeholders of `bytes`: the ByteRange covers the whole
/// file except the `/Contents` hex string; `sign` receives the covered bytes.
pub fn sign_last(bytes: &mut [u8], sign: impl FnOnce(&[u8]) -> Vec<u8>) {
    let range_at =
        rfind(bytes, b"/ByteRange [").expect("ByteRange placeholder") + b"/ByteRange [".len();
    let contents_at = range_at
        + find(&bytes[range_at..], b"/Contents <").expect("Contents")
        + b"/Contents ".len();
    let contents_end = contents_at + find(&bytes[contents_at..], b">").unwrap() + 1;
    let length = bytes.len();
    let values = [0, contents_at, contents_end, length - contents_end];
    write_range(bytes, range_at, &values.map(|value| value as i64));
    let mut covered = bytes[..contents_at].to_vec();
    covered.extend_from_slice(&bytes[contents_end..]);
    let cms = sign(&covered);
    let capacity = (contents_end - contents_at - 2) / 2;
    assert!(
        cms.len() <= capacity,
        "CMS {} > placeholder {capacity}",
        cms.len()
    );
    let hex: String = cms.iter().map(|byte| format!("{byte:02X}")).collect();
    bytes[contents_at + 1..contents_at + 1 + hex.len()].copy_from_slice(hex.as_bytes());
}

/// Rewrites the last ByteRange with `values` (fixed width, so offsets do not move).
pub fn rewrite_last_range(bytes: &mut [u8], values: &[i64]) {
    let range_at = rfind(bytes, b"/ByteRange [").unwrap() + b"/ByteRange [".len();
    write_range(bytes, range_at, values);
}

fn write_range(bytes: &mut [u8], at: usize, values: &[i64]) {
    let mut text = String::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        if *value < 0 {
            text.push_str(&format!("-{:0width$}", -value, width = RANGE_WIDTH - 1));
        } else {
            text.push_str(&format!("{value:0RANGE_WIDTH$}"));
        }
    }
    let end = at + find(&bytes[at..], b"]").unwrap();
    assert_eq!(text.len(), end - at, "ByteRange placeholder width");
    bytes[at..end].copy_from_slice(text.as_bytes());
}

pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// Field layout of the first revision.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One signed field `Signature1`.
    Signed,
    /// One `/FT /Sig` field without `/V`.
    EmptyField,
    /// No AcroForm signature at all.
    Unsigned,
    /// A `/FT /Sig` parent without `/V` and a signed kid `Kid`.
    SignedKid,
    /// The signed dictionary is referenced only from the catalog, not by a field.
    Unreferenced,
}

pub struct Base {
    pub layout: Layout,
    pub kind: &'static str,
    pub sub_filter: &'static str,
    pub signature_extra: String,
    pub catalog_extra: String,
    pub slots: usize,
    pub contents_bytes: usize,
}

impl Base {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            kind: "Sig",
            sub_filter: "ETSI.CAdES.detached",
            signature_extra: String::new(),
            catalog_extra: String::new(),
            slots: 4,
            contents_bytes: CONTENTS_BYTES,
        }
    }

    /// First revision (objects 1–7); unsigned until [`sign_last`] runs.
    pub fn build(&self) -> Pdf {
        let mut pdf = Pdf::new();
        let content = "q 0 0 1 rg 20 20 160 160 re f Q";
        let signature = signature_dictionary(
            self.kind,
            self.sub_filter,
            &self.signature_extra,
            self.slots,
            self.contents_bytes,
        );
        let mut objects = vec![
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (
                6,
                format!(
                    "<< /Length {} >>\nstream\n{content}\nendstream",
                    content.len()
                ),
            ),
        ];
        let widget = "/Type /Annot /Subtype /Widget /Rect [0 0 0 0] /F 132 /P 3 0 R";
        let (fields, annots) = match self.layout {
            Layout::Signed => {
                objects.push((
                    4,
                    format!("<< /FT /Sig /T (Signature1) /V 5 0 R {widget} >>"),
                ));
                objects.push((5, signature));
                ("[4 0 R]", "[4 0 R]")
            }
            Layout::EmptyField => {
                objects.push((4, format!("<< /FT /Sig /T (Empty1) {widget} >>")));
                ("[4 0 R]", "[4 0 R]")
            }
            Layout::Unsigned => ("[]", "[]"),
            Layout::SignedKid => {
                objects.push((4, "<< /FT /Sig /T (Parent) /Kids [7 0 R] >>".to_string()));
                objects.push((7, format!("<< /Parent 4 0 R /T (Kid) /V 5 0 R {widget} >>")));
                objects.push((5, signature));
                ("[4 0 R]", "[7 0 R]")
            }
            Layout::Unreferenced => {
                objects.push((5, signature));
                ("[]", "[]")
            }
        };
        let orphan = if self.layout == Layout::Unreferenced {
            " /TestOnlyOrphan 5 0 R"
        } else {
            ""
        };
        objects.push((
            1,
            format!(
                "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields {fields} /SigFlags 3 >>{orphan}{} >>",
                self.catalog_extra
            ),
        ));
        objects.push((
            3,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 6 0 R /Annots {annots} >>"),
        ));
        pdf.revision(&objects, 1);
        pdf
    }
}

/// Appends an incremental revision with a second signed field `Signature2` (objects 8–9).
pub fn add_second_signature(pdf: &mut Pdf, extra: &str) {
    let widget = "/Type /Annot /Subtype /Widget /Rect [0 0 0 0] /F 132 /P 3 0 R";
    pdf.revision(
        &[
            (8, format!("<< /FT /Sig /T (Signature2) /V 9 0 R {widget} >>")),
            (9, signature_dictionary("Sig", "ETSI.CAdES.detached", extra, 4, CONTENTS_BYTES)),
            (1, "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 8 0 R] /SigFlags 3 >> >>".to_string()),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 6 0 R /Annots [4 0 R 8 0 R] >>".to_string()),
        ],
        1,
    );
}

/// Appends an unsigned incremental revision (one new object).
pub fn add_unsigned_revision(pdf: &mut Pdf) {
    pdf.revision(
        &[(
            10,
            "<< /Title (TEST ONLY changed after signing) >>".to_string(),
        )],
        1,
    );
}

// ---------------------------------------------------------------------------------------
// The G2 fixture set
// ---------------------------------------------------------------------------------------

pub struct Signers {
    pub rsa: Key,
    pub rsa_certificate: Certificate,
    pub other_rsa: Key,
    pub other_rsa_certificate: Certificate,
    pub weak_rsa: Key,
    pub weak_rsa_certificate: Certificate,
    pub p256: Key,
    pub p256_certificate: Certificate,
    pub p384: Key,
    pub p384_certificate: Certificate,
}

pub fn signers() -> Signers {
    let rsa = rsa_key("rsa2048-signer");
    let other_rsa = rsa_key("rsa2048-other");
    let weak_rsa = rsa_key("rsa1024-weak");
    let p256 = p256_key("signer");
    let p384 = p384_key("signer");
    Signers {
        rsa_certificate: certificate("TEST ONLY RSA Signer", 1, &rsa, &rsa),
        other_rsa_certificate: certificate("TEST ONLY RSA Other", 2, &other_rsa, &other_rsa),
        weak_rsa_certificate: certificate("TEST ONLY RSA 1024", 3, &weak_rsa, &weak_rsa),
        p256_certificate: certificate("TEST ONLY P-256 Signer", 4, &p256, &p256),
        p384_certificate: certificate("TEST ONLY P-384 Signer", 5, &p384, &p384),
        rsa,
        other_rsa,
        weak_rsa,
        p256,
        p384,
    }
}

fn signed(base: &Base, spec: &CmsSpec) -> Pdf {
    let mut pdf = base.build();
    sign_last(&mut pdf.bytes, |covered| build_cms(spec, covered));
    pdf
}

fn rsa_spec(signers: &Signers) -> CmsSpec {
    CmsSpec::new(signers.rsa.clone(), signers.rsa_certificate.clone())
}

/// `(file name, bytes)` of every G2 fixture, in a fixed order.
pub fn g2_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let s = signers();
    let standard = Base::new(Layout::Signed);
    let mut out: Vec<(&'static str, Vec<u8>)> = Vec::new();

    out.push((
        "sig-01-unsigned.pdf",
        Base::new(Layout::Unsigned).build().bytes,
    ));
    out.push((
        "sig-03-empty-signature-field.pdf",
        Base::new(Layout::EmptyField).build().bytes,
    ));

    let valid = signed(&standard, &rsa_spec(&s));
    out.push(("sig-09-rsa-pkcs1-sha256.pdf", valid.bytes.clone()));

    let mut pss = CmsSpec::new(s.rsa.clone(), s.rsa_certificate.clone());
    pss.hash = Hash::Sha384;
    pss.scheme = Scheme::Pss { salt: 48 };
    out.push(("sig-10-rsa-pss-sha384.pdf", signed(&standard, &pss).bytes));

    out.push((
        "sig-11-ecdsa-p256-sha256.pdf",
        signed(
            &standard,
            &CmsSpec::new(s.p256.clone(), s.p256_certificate.clone()),
        )
        .bytes,
    ));
    let mut p384 = CmsSpec::new(s.p384.clone(), s.p384_certificate.clone());
    p384.hash = Hash::Sha384;
    out.push((
        "sig-12-ecdsa-p384-sha384.pdf",
        signed(&standard, &p384).bytes,
    ));

    let mut adbe = Base::new(Layout::Signed);
    adbe.sub_filter = "adbe.pkcs7.detached";
    let mut no_attributes = rsa_spec(&s);
    no_attributes.signed_attributes = false;
    out.push((
        "sig-09b-adbe-pkcs7-no-signed-attributes.pdf",
        signed(&adbe, &no_attributes).bytes,
    ));

    let mut two = signed(&standard, &rsa_spec(&s));
    add_second_signature(&mut two, "");
    let mut second = CmsSpec::new(s.p256.clone(), s.p256_certificate.clone());
    second.signing_certificate_v2 = true;
    sign_last(&mut two.bytes, |covered| build_cms(&second, covered));
    out.push(("sig-16-two-signatures.pdf", two.bytes));

    let mut modified = valid.bytes.clone();
    let at = find(&modified, b"0 0 1 rg").unwrap();
    modified[at] = b'1';
    out.push(("sig-19-modified-after-signing.pdf", modified));

    let mut corrupt = rsa_spec(&s);
    corrupt.corrupt_signature = true;
    out.push((
        "sig-21-bad-signature-value.pdf",
        signed(&standard, &corrupt).bytes,
    ));

    let mut digest = rsa_spec(&s);
    digest.message_digest_override = Some(vec![0x11; 32]);
    out.push((
        "sig-22-message-digest-mismatch.pdf",
        signed(&standard, &digest).bytes,
    ));

    let mut no_content_type = rsa_spec(&s);
    no_content_type.content_type = false;
    out.push((
        "sig-24-content-type-missing.pdf",
        signed(&standard, &no_content_type).bytes,
    ));

    let mut sha1 = rsa_spec(&s);
    sha1.hash = Hash::Sha1;
    out.push(("sig-27-sha1-digest.pdf", signed(&standard, &sha1).bytes));

    let mut certification = Base::new(Layout::Signed);
    certification.signature_extra =
        " /Reference [<< /Type /SigRef /TransformMethod /DocMDP /TransformParams << /Type /TransformParams /P 1 /V /1.2 >> >>]".into();
    certification.catalog_extra = " /Perms << /DocMDP 5 0 R >>".into();
    let mut certified = signed(&certification, &rsa_spec(&s));
    out.push(("sig-39-docmdp-p1.pdf", certified.bytes.clone()));
    add_unsigned_revision(&mut certified);
    out.push(("sig-39b-docmdp-p1-later-revision.pdf", certified.bytes));

    let mut later = valid.clone_pdf();
    add_unsigned_revision(&mut later);
    out.push(("sig-unsigned-revision-after-signing.pdf", later.bytes));

    let mut trailing = valid.bytes.clone();
    trailing.extend_from_slice(b"TEST ONLY trailing bytes\n");
    out.push(("sig-trailing-bytes.pdf", trailing));

    let mut timestamp = Base::new(Layout::Signed);
    timestamp.kind = "DocTimeStamp";
    timestamp.sub_filter = "ETSI.RFC3161";
    let mut stamped = timestamp.build();
    sign_last(&mut stamped.bytes, |_| vec![0x30, 0x03, 0x02, 0x01, 0x03]);
    out.push(("sig-doc-timestamp.pdf", stamped.bytes));

    out.push((
        "sig-signed-kid-field.pdf",
        signed(&Base::new(Layout::SignedKid), &rsa_spec(&s)).bytes,
    ));
    out.push((
        "sig-unreferenced-signature-dictionary.pdf",
        signed(&Base::new(Layout::Unreferenced), &rsa_spec(&s)).bytes,
    ));

    let mut missing = rsa_spec(&s);
    missing.embedded = Some(Vec::new());
    out.push((
        "sig-signer-certificate-missing.pdf",
        signed(&standard, &missing).bytes,
    ));

    let mut ambiguous = rsa_spec(&s);
    let impostor = certificate("TEST ONLY RSA Signer", 1, &s.other_rsa, &s.other_rsa);
    ambiguous.embedded = Some(vec![s.rsa_certificate.clone(), impostor]);
    out.push((
        "sig-signer-certificate-ambiguous.pdf",
        signed(&standard, &ambiguous).bytes,
    ));

    let mut wrong_key = rsa_spec(&s);
    wrong_key.signing_key_override = Some(s.other_rsa.clone());
    out.push(("sig-wrong-key.pdf", signed(&standard, &wrong_key).bytes));

    out.push((
        "sig-rsa-1024-weak-key.pdf",
        signed(
            &standard,
            &CmsSpec::new(s.weak_rsa.clone(), s.weak_rsa_certificate.clone()),
        )
        .bytes,
    ));

    let mut timestamped = rsa_spec(&s);
    timestamped.signature_timestamp_attribute = true;
    out.push((
        "sig-signature-timestamp-attribute.pdf",
        signed(&standard, &timestamped).bytes,
    ));

    // Category 6: hostile ByteRange and /Contents (classes 49–55).
    let ranges = |bytes: &[u8]| -> [i64; 4] {
        let at = rfind(bytes, b"/ByteRange [").unwrap() + 12;
        let end = at + find(&bytes[at..], b"]").unwrap();
        let text = std::str::from_utf8(&bytes[at..end]).unwrap();
        let values: Vec<i64> = text
            .split(' ')
            .map(|value| value.parse().unwrap())
            .collect();
        [values[0], values[1], values[2], values[3]]
    };
    let [a, b, c, d] = ranges(&valid.bytes);
    let mut overlapping = valid.bytes.clone();
    rewrite_last_range(&mut overlapping, &[a, c + 1, c, d]);
    out.push(("sig-49-byterange-overlapping.pdf", overlapping));
    let mut out_of_bounds = valid.bytes.clone();
    rewrite_last_range(&mut out_of_bounds, &[a, b, c, d + 100]);
    out.push(("sig-50-byterange-out-of-bounds.pdf", out_of_bounds));
    let mut negative = valid.bytes.clone();
    rewrite_last_range(&mut negative, &[a, b, -c, d]);
    out.push(("sig-51-byterange-negative.pdf", negative));
    let mut gap = valid.bytes.clone();
    rewrite_last_range(&mut gap, &[a, b - 10, c, d]);
    out.push(("sig-52-byterange-uncovered-gap.pdf", gap));

    let mut six = Base::new(Layout::Signed);
    six.slots = 6;
    let mut multiple = six.build();
    sign_last_six(&mut multiple.bytes, |covered| {
        build_cms(&rsa_spec(&s), covered)
    });
    out.push(("sig-53-byterange-two-exclusions.pdf", multiple.bytes));

    let mut empty = Base::new(Layout::Signed);
    empty.contents_bytes = 0;
    let mut empty_contents = empty.build();
    sign_last(&mut empty_contents.bytes, |_| Vec::new());
    out.push(("sig-54-empty-contents.pdf", empty_contents.bytes));

    let mut malformed = standard.build();
    sign_last(&mut malformed.bytes, |_| {
        vec![0x30, 0x82, 0x01, 0x00, 0xde, 0xad, 0xbe, 0xef]
    });
    out.push(("sig-55-malformed-der.pdf", malformed.bytes));

    out
}

/// Six-slot variant of [`sign_last`]: `[0 b c d e f]` with a second, empty exclusion.
fn sign_last_six(bytes: &mut [u8], sign: impl FnOnce(&[u8]) -> Vec<u8>) {
    let range_at = rfind(bytes, b"/ByteRange [").unwrap() + b"/ByteRange [".len();
    let contents_at =
        range_at + find(&bytes[range_at..], b"/Contents <").unwrap() + b"/Contents ".len();
    let contents_end = contents_at + find(&bytes[contents_at..], b">").unwrap() + 1;
    let length = bytes.len();
    let tail = length - contents_end;
    let values = [
        0,
        contents_at as i64,
        contents_end as i64,
        (tail / 2) as i64,
        (contents_end + tail / 2 + 1) as i64,
        (tail - tail / 2 - 1) as i64,
    ];
    write_range(bytes, range_at, &values);
    let mut covered = bytes[..contents_at].to_vec();
    covered.extend_from_slice(&bytes[contents_end..]);
    let cms = sign(&covered);
    let hex: String = cms.iter().map(|byte| format!("{byte:02X}")).collect();
    bytes[contents_at + 1..contents_at + 1 + hex.len()].copy_from_slice(hex.as_bytes());
}

impl Pdf {
    pub fn clone_pdf(&self) -> Pdf {
        Pdf {
            bytes: self.bytes.clone(),
            last_xref: self.last_xref,
            size: self.size,
        }
    }
}
