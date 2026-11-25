//! CAP-01: the canonical signature discovery (design §2.3 note, §18.3). Worker only.
//!
//! PDFium first: its signature objects supply ByteRange, `/Contents`, SubFilter, `/M`,
//! `/Reason` and the DocMDP permission. lopdf ([`super::structure`]) supplies what PDFium
//! does not see. The G2 spike (2026-11-24) measured PDFium 0.8.37's vendored build:
//!
//! | Structure | `FPDF_GetSignatureCount` |
//! |---|---|
//! | `/FT /Sig` field without `/V` (empty) | counted, object has no ByteRange/Contents |
//! | signed field below a `/FT /Sig` parent | counted as the parent, with no ByteRange/Contents |
//! | signed field below a parent without `/FT` | not counted |
//! | `/DocTimeStamp` field value | counted |
//! | `/Sig` dictionary outside the AcroForm tree | not counted |
//!
//! So a PDFium object without ByteRange and Contents is not a signature; lopdf tells whether
//! it was an empty field or the parent of signed kids. Signed fields PDFium misses are added
//! from lopdf with a `SIGNATURE_NOT_VISIBLE_TO_PDFIUM` finding. Disagreements are findings,
//! never silently resolved.

use super::limits::{self, MAX_CONTENTS_BYTES, MAX_SIGNATURES};
use super::report::{FindingSeverity, StructuralStatus, ValidationFinding};
use super::structure::{self, Structure, StructureError};
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_DOCUMENT};
use std::path::Path;

/// One signature object as PDFium reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PdfiumSignature {
    pub byte_range: Vec<i64>,
    /// `None` when larger than [`MAX_CONTENTS_BYTES`].
    pub contents: Option<Vec<u8>>,
    pub contents_length: u64,
    pub sub_filter: Option<String>,
    pub time: Option<String>,
    pub reason: Option<String>,
    /// 1–3, or 0 when the signature has no DocMDP transform.
    pub docmdp: u32,
}

impl PdfiumSignature {
    fn is_empty_field(&self) -> bool {
        self.byte_range.is_empty() && self.contents_length == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureSource {
    Pdfium,
    /// lopdf only: PDFium did not report it.
    Structure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSignature {
    pub source: SignatureSource,
    pub field_name: Option<String>,
    pub byte_range: Vec<i64>,
    pub contents: Option<Vec<u8>>,
    pub sub_filter: Option<String>,
    pub is_document_timestamp: bool,
    pub pdf_m: Option<String>,
    pub reason: Option<String>,
    pub pdfium_docmdp: Option<u32>,
    pub structural_docmdp_p: Option<i64>,
    pub field_mdp: Option<super::report::FieldMdpReport>,
    /// `/Perms /DocMDP` points at this signature.
    pub certification: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureDiscovery {
    pub signatures: Vec<DiscoveredSignature>,
    /// `None`: PDFium could not open the document.
    pub pdfium_object_count: Option<u32>,
    pub empty_signature_fields: u32,
    pub unreferenced_signature_dictionaries: u32,
    pub structural_status: StructuralStatus,
    pub dss_present: bool,
    pub findings: Vec<ValidationFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryError {
    TooManySignatures,
}

impl SignatureDiscovery {
    /// Signed signature fields, document timestamps included.
    pub fn signature_count(&self) -> u32 {
        u32::try_from(self.signatures.len()).unwrap_or(u32::MAX)
    }

    pub fn document_timestamp_count(&self) -> u32 {
        u32::try_from(
            self.signatures
                .iter()
                .filter(|signature| signature.is_document_timestamp)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Whether a full rewrite of this document loses signatures. Fails safe: an unreadable
    /// document, or PDFium objects that lopdf could not classify, count as signed.
    pub fn signatures_invalidated(&self) -> bool {
        !self.signatures.is_empty()
            || self.unreferenced_signature_dictionaries > 0
            || self.pdfium_object_count.is_none()
            || (self.structural_status == StructuralStatus::Unavailable
                && self.pdfium_object_count.is_some_and(|count| count > 0))
    }
}

/// Reads PDFium's signature objects from an open document.
pub fn read_pdfium_signatures(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
) -> Result<Vec<PdfiumSignature>, DiscoveryError> {
    let count = bindings.FPDF_GetSignatureCount(document);
    let count = usize::try_from(count).unwrap_or(0);
    if count > MAX_SIGNATURES {
        return Err(DiscoveryError::TooManySignatures);
    }
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let Ok(index) = i32::try_from(index) else {
            break;
        };
        let handle = bindings.FPDF_GetSignatureObject(document, index);
        if handle.is_null() {
            out.push(PdfiumSignature::default());
            continue;
        }
        let mut signature = PdfiumSignature::default();
        let range_length = bindings.FPDFSignatureObj_GetByteRange(handle, std::ptr::null_mut(), 0);
        if range_length > 0 && range_length <= 64 {
            let mut buffer = vec![0i32; range_length as usize];
            let written =
                bindings.FPDFSignatureObj_GetByteRange(handle, buffer.as_mut_ptr(), range_length);
            if written == range_length {
                signature.byte_range = buffer.into_iter().map(i64::from).collect();
            }
        }
        let contents_length =
            bindings.FPDFSignatureObj_GetContents(handle, std::ptr::null_mut(), 0);
        // `c_ulong` is u32 on Windows and u64 elsewhere.
        #[allow(clippy::useless_conversion)]
        {
            signature.contents_length = u64::from(contents_length);
        }
        if contents_length > 0 && (contents_length as u64) <= MAX_CONTENTS_BYTES as u64 {
            let mut buffer = vec![0u8; contents_length as usize];
            let written = bindings.FPDFSignatureObj_GetContents(
                handle,
                buffer.as_mut_ptr().cast(),
                contents_length,
            );
            if written == contents_length {
                signature.contents = Some(buffer);
            }
        }
        signature.sub_filter = ascii_getter(|buffer, length| {
            bindings.FPDFSignatureObj_GetSubFilter(handle, buffer, length)
        });
        signature.time = ascii_getter(|buffer, length| {
            bindings.FPDFSignatureObj_GetTime(handle, buffer, length)
        });
        signature.reason = utf16_getter(|buffer, length| {
            bindings.FPDFSignatureObj_GetReason(handle, buffer, length)
        })
        .map(|reason| limits::trim(&reason, limits::MAX_REASON_CHARS));
        signature.docmdp = bindings.FPDFSignatureObj_GetDocMDPPermission(handle);
        out.push(signature);
    }
    Ok(out)
}

fn ascii_getter(
    mut call: impl FnMut(*mut std::os::raw::c_char, std::os::raw::c_ulong) -> std::os::raw::c_ulong,
) -> Option<String> {
    let length = call(std::ptr::null_mut(), 0);
    if length == 0 || length > 1024 {
        return None;
    }
    let mut buffer = vec![0 as std::os::raw::c_char; length as usize];
    if call(buffer.as_mut_ptr(), length) != length {
        return None;
    }
    let bytes: Vec<u8> = buffer
        .iter()
        .map(|character| *character as u8)
        .take_while(|byte| *byte != 0)
        .collect();
    Some(limits::trim(
        &String::from_utf8_lossy(&bytes),
        limits::MAX_NAME_COMPONENT_CHARS,
    ))
}

fn utf16_getter(
    mut call: impl FnMut(*mut std::ffi::c_void, std::os::raw::c_ulong) -> std::os::raw::c_ulong,
) -> Option<String> {
    let length = call(std::ptr::null_mut(), 0);
    if !(2..=64 * 1024).contains(&length) {
        return None;
    }
    let mut buffer = vec![0u8; length as usize];
    if call(buffer.as_mut_ptr().cast(), length) != length {
        return None;
    }
    let units: Vec<u16> = buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16_lossy(&units);
    (!text.is_empty()).then_some(text)
}

/// Combines PDFium's objects (`None`: PDFium could not open the document) with the lopdf
/// structure of the same bytes (`Err`: lopdf could not read them).
pub fn combine(
    pdfium: Option<Vec<PdfiumSignature>>,
    structure: Result<Structure, StructureError>,
) -> Result<SignatureDiscovery, DiscoveryError> {
    let mut findings = Vec::new();
    let pdfium_object_count = pdfium
        .as_ref()
        .map(|objects| u32::try_from(objects.len()).unwrap_or(u32::MAX));
    let mut structural_status = StructuralStatus::Complete;
    let structure = match structure {
        Ok(structure) => {
            if structure.truncated {
                structural_status = StructuralStatus::Inconsistent;
                findings.push(ValidationFinding::new(
                    "SIGNATURE_FIELD_TREE_TRUNCATED",
                    FindingSeverity::Warning,
                    "İmza alanı ağacında döngü ya da sınır aşımı var; bazı alanlar okunmamış olabilir",
                ));
            }
            Some(structure)
        }
        Err(StructureError::TooManySignatures) => return Err(DiscoveryError::TooManySignatures),
        Err(error) => {
            structural_status = StructuralStatus::Unavailable;
            findings.push(
                ValidationFinding::new(
                    "SIGNATURE_STRUCTURE_UNAVAILABLE",
                    FindingSeverity::Warning,
                    "Belge yapısı okunamadı; alan adları, DocMDP ve revizyon eşleşmesi eksik kalabilir",
                )
                .with_context(format!("{error:?}")),
            );
            None
        }
    };
    let mut signatures = Vec::new();
    let mut matched = vec![
        false;
        structure
            .as_ref()
            .map_or(0, |structure| structure.signed.len())
    ];
    let mut pdfium_empty = 0u32;
    for object in pdfium.iter().flatten() {
        if object.is_empty_field() {
            pdfium_empty = pdfium_empty.saturating_add(1);
            continue;
        }
        let structural = structure.as_ref().and_then(|structure| {
            let index = structure
                .signed
                .iter()
                .enumerate()
                .position(|(index, candidate)| {
                    !matched[index]
                        && candidate.byte_range.as_deref() == Some(object.byte_range.as_slice())
                })?;
            matched[index] = true;
            Some(&structure.signed[index])
        });
        if structure.is_some() && structural.is_none() {
            structural_status = StructuralStatus::Inconsistent;
            findings.push(
                ValidationFinding::new(
                    "DISCOVERY_SOURCE_MISMATCH",
                    FindingSeverity::Warning,
                    "PDFium'un bildirdiği bir imza belge yapısında bulunamadı",
                )
                .with_context(format!("byte_range={:?}", object.byte_range)),
            );
        }
        let sub_filter = object
            .sub_filter
            .clone()
            .or_else(|| structural.and_then(|signature| signature.sub_filter.clone()));
        signatures.push(DiscoveredSignature {
            source: SignatureSource::Pdfium,
            field_name: structural.and_then(|signature| signature.field_name.clone()),
            byte_range: object.byte_range.clone(),
            contents: object.contents.clone(),
            is_document_timestamp: sub_filter.as_deref() == Some("ETSI.RFC3161")
                || structural.is_some_and(|signature| signature.is_document_timestamp),
            sub_filter,
            pdf_m: object
                .time
                .clone()
                .or_else(|| structural.and_then(|signature| signature.pdf_m.clone())),
            reason: object
                .reason
                .clone()
                .or_else(|| structural.and_then(|signature| signature.reason.clone())),
            pdfium_docmdp: Some(object.docmdp),
            structural_docmdp_p: structural.and_then(|signature| signature.docmdp_p),
            field_mdp: structural.and_then(|signature| signature.field_mdp.clone()),
            certification: false,
        });
        if object.contents.is_none() && object.contents_length > 0 {
            findings.push(
                ValidationFinding::new(
                    limits::SIGNATURE_RESOURCE_LIMIT_EXCEEDED,
                    FindingSeverity::Error,
                    "/Contents boyut sınırını aşıyor",
                )
                .with_context(format!("bytes={}", object.contents_length)),
            );
        }
    }
    let mut empty_fields = pdfium_empty;
    let mut unreferenced = 0;
    let mut dss_present = false;
    if let Some(structure) = &structure {
        empty_fields = structure.empty_fields;
        unreferenced = u32::try_from(structure.unreferenced.len()).unwrap_or(u32::MAX);
        dss_present = structure.dss_present;
        for (index, signature) in structure.signed.iter().enumerate() {
            if matched[index] {
                continue;
            }
            findings.push(
                ValidationFinding::new(
                    "SIGNATURE_NOT_VISIBLE_TO_PDFIUM",
                    FindingSeverity::Warning,
                    "Bir imzalı alan PDFium tarafından listelenmiyor; belge yapısından okundu",
                )
                .with_context(signature.field_name.clone().unwrap_or_default()),
            );
            signatures.push(DiscoveredSignature {
                source: SignatureSource::Structure,
                field_name: signature.field_name.clone(),
                byte_range: signature.byte_range.clone().unwrap_or_default(),
                contents: signature
                    .contents
                    .clone()
                    .filter(|contents| contents.len() <= MAX_CONTENTS_BYTES),
                sub_filter: signature.sub_filter.clone(),
                is_document_timestamp: signature.is_document_timestamp,
                pdf_m: signature.pdf_m.clone(),
                reason: signature.reason.clone(),
                pdfium_docmdp: None,
                structural_docmdp_p: signature.docmdp_p,
                field_mdp: signature.field_mdp.clone(),
                certification: false,
            });
        }
        if unreferenced > 0 {
            findings.push(
                ValidationFinding::new(
                    "SIGNATURE_DICTIONARY_UNREFERENCED",
                    FindingSeverity::Warning,
                    "Hiçbir imza alanının göstermediği imza sözlükleri var; doğrulanmadılar",
                )
                .with_context(format!("count={unreferenced}")),
            );
        }
        if let Some(certification) = &structure.certification_byte_range {
            for signature in &mut signatures {
                signature.certification = &signature.byte_range == certification;
            }
        }
    } else if pdfium_empty > 0 {
        findings.push(ValidationFinding::new(
            "SIGNATURE_FIELDS_UNCLASSIFIED",
            FindingSeverity::Warning,
            "PDFium değeri olmayan imza alanları bildiriyor; yapı okunamadığı için alt alanlarda imza olup olmadığı bilinmiyor",
        ));
    }
    if signatures.len() > MAX_SIGNATURES {
        return Err(DiscoveryError::TooManySignatures);
    }
    Ok(SignatureDiscovery {
        signatures,
        pdfium_object_count,
        empty_signature_fields: empty_fields,
        unreferenced_signature_dictionaries: unreferenced,
        structural_status,
        dss_present,
        findings,
    })
}

/// Discovery of the document at `path` for the copy and rewrite paths. Opens its own PDFium
/// handle with `bindings`; the structural walk reads the file only when it is within the
/// structural limit.
pub fn discover_at_path(
    bindings: &dyn PdfiumLibraryBindings,
    path: &Path,
    credential: Option<&str>,
) -> Result<SignatureDiscovery, DiscoveryError> {
    let pdfium = path.to_str().and_then(|text| {
        let document = crate::incremental_save::RawDocument::open(bindings, text, credential)?;
        Some(read_pdfium_signatures(bindings, document.handle))
    });
    let pdfium = pdfium.transpose()?;
    combine(pdfium, read_structure_at(path, credential))
}

pub fn read_structure_at(
    path: &Path,
    credential: Option<&str>,
) -> Result<Structure, StructureError> {
    let length = std::fs::metadata(path)
        .map_err(|_| StructureError::Unparsable)?
        .len();
    if length > limits::MAX_STRUCTURAL_INPUT_BYTES {
        return Err(StructureError::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| StructureError::Unparsable)?;
    structure::read_structure(&bytes, credential)
}

/// `signatures_invalidated` for a copy path; a discovery failure counts as signed.
pub fn copy_loses_signatures(
    bindings: &dyn PdfiumLibraryBindings,
    path: &Path,
    credential: Option<&str>,
) -> bool {
    discover_at_path(bindings, path, credential)
        .map_or(true, |discovery| discovery.signatures_invalidated())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("signature")
            .join(name)
    }

    /// The G2 spike measurements, kept as regression: what PDFium lists, what the canonical
    /// discovery decides, and whether a full rewrite loses signatures.
    #[test]
    fn canonical_discovery_classifies_the_spike_structures() {
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        // (fixture, signatures, empty fields, unreferenced, PDFium objects, invalidated)
        let cases = [
            ("sig-01-unsigned.pdf", 0, 0, 0, 0, false),
            // PDFium counts the empty field; it is not a signature and nothing is lost.
            ("sig-03-empty-signature-field.pdf", 0, 1, 0, 1, false),
            ("sig-09-rsa-pkcs1-sha256.pdf", 1, 0, 0, 1, true),
            // PDFium lists only the empty parent; the signed kid comes from lopdf.
            ("sig-signed-kid-field.pdf", 1, 0, 0, 1, true),
            // PDFium does not list a dictionary outside the AcroForm tree.
            (
                "sig-unreferenced-signature-dictionary.pdf",
                0,
                0,
                1,
                0,
                true,
            ),
            ("sig-doc-timestamp.pdf", 1, 0, 0, 1, true),
            ("sig-16-two-signatures.pdf", 2, 0, 0, 2, true),
        ];
        for (name, signatures, empty, unreferenced, objects, invalidated) in cases {
            let discovery = discover_at_path(pdfium.bindings(), &fixture(name), None).unwrap();
            assert_eq!(discovery.signature_count(), signatures, "{name}");
            assert_eq!(discovery.empty_signature_fields, empty, "{name}");
            assert_eq!(
                discovery.unreferenced_signature_dictionaries, unreferenced,
                "{name}"
            );
            assert_eq!(discovery.pdfium_object_count, Some(objects), "{name}");
            assert_eq!(discovery.signatures_invalidated(), invalidated, "{name}");
            assert_eq!(
                copy_loses_signatures(pdfium.bindings(), &fixture(name), None),
                invalidated,
                "{name}"
            );
        }
    }

    #[test]
    fn unreadable_documents_count_as_signed() {
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("broken.pdf");
        std::fs::write(&path, b"%PDF-1.7\nnot a pdf").unwrap();
        assert!(copy_loses_signatures(pdfium.bindings(), &path, None));
        let discovery = discover_at_path(pdfium.bindings(), &path, None).unwrap();
        assert_eq!(discovery.pdfium_object_count, None);
        assert_eq!(discovery.structural_status, StructuralStatus::Unavailable);
    }

    #[test]
    fn structure_unavailable_with_pdfium_objects_fails_safe() {
        let empty_field = PdfiumSignature::default();
        let discovery = combine(Some(vec![empty_field]), Err(StructureError::TooLarge)).unwrap();
        assert!(discovery.signatures.is_empty());
        assert!(discovery.signatures_invalidated());
        assert!(discovery
            .findings
            .iter()
            .any(|finding| finding.code == "SIGNATURE_FIELDS_UNCLASSIFIED"));
        let none = combine(Some(Vec::new()), Err(StructureError::TooLarge)).unwrap();
        assert!(!none.signatures_invalidated());
    }

    #[test]
    fn a_pdfium_signature_missing_from_the_structure_is_a_finding() {
        let object = PdfiumSignature {
            byte_range: vec![0, 1, 2, 3],
            contents: Some(vec![0x30]),
            contents_length: 1,
            ..PdfiumSignature::default()
        };
        let discovery = combine(Some(vec![object]), Ok(Structure::default())).unwrap();
        assert_eq!(discovery.structural_status, StructuralStatus::Inconsistent);
        assert_eq!(discovery.signature_count(), 1);
        assert!(discovery
            .findings
            .iter()
            .any(|finding| finding.code == "DISCOVERY_SOURCE_MISMATCH"));
    }
}
