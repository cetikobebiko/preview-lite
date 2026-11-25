//! lopdf structural walk for signatures (new lopdf use, Gate 1 §NEW LOPDF USES 1).
//!
//! Architectural reason: PDFium does not show signature fields below `/Kids`, signature
//! dictionaries outside the AcroForm tree, FieldMDP parameters, `/Perms /DocMDP` or `/DSS`
//! (G2 spike, 2026-11-24). Security: runs only in the job worker, on the immutable snapshot's
//! bytes, read-only, bounded by [`super::limits::MAX_STRUCTURAL_INPUT_BYTES`]; the encryption
//! decision uses `lopdf_compat` (never `trailer.has(Prev)` / `is_encrypted()`).

use super::limits::{self, MAX_SIGNATURES};
use super::report::FieldMdpReport;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashSet;

const MAX_FIELD_DEPTH: usize = 32;
const MAX_FIELDS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StructuralSignature {
    pub field_name: Option<String>,
    pub byte_range: Option<Vec<i64>>,
    pub contents: Option<Vec<u8>>,
    pub sub_filter: Option<String>,
    pub is_document_timestamp: bool,
    pub pdf_m: Option<String>,
    pub reason: Option<String>,
    /// `/Reference` DocMDP transform `/P` (spec default 2 when `/P` is missing).
    pub docmdp_p: Option<i64>,
    pub field_mdp: Option<FieldMdpReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Structure {
    /// Signed signature fields reachable from `/AcroForm /Fields`, in tree order.
    pub signed: Vec<StructuralSignature>,
    pub empty_fields: u32,
    /// Signature dictionaries no AcroForm field points at.
    pub unreferenced: Vec<StructuralSignature>,
    pub dss_present: bool,
    /// ByteRange of the signature `/Perms /DocMDP` points at.
    pub certification_byte_range: Option<Vec<i64>>,
    /// The field tree hit a cycle, depth or size limit; some fields may be missing.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureError {
    TooLarge,
    Unparsable,
    CredentialRequired,
    CredentialRejected,
    TooManySignatures,
}

pub fn read_structure(bytes: &[u8], credential: Option<&str>) -> Result<Structure, StructureError> {
    if bytes.len() as u64 > limits::MAX_STRUCTURAL_INPUT_BYTES {
        return Err(StructureError::TooLarge);
    }
    let document =
        crate::lopdf_compat::load_mem_with_credential(bytes, credential).map_err(|error| {
            match error {
                crate::lopdf_compat::CredentialedLoadError::Parse => StructureError::Unparsable,
                crate::lopdf_compat::CredentialedLoadError::CredentialRequired => {
                    StructureError::CredentialRequired
                }
                crate::lopdf_compat::CredentialedLoadError::CredentialRejected => {
                    StructureError::CredentialRejected
                }
            }
        })?;
    walk(&document)
}

fn walk(document: &Document) -> Result<Structure, StructureError> {
    let mut structure = Structure::default();
    let catalog = document.catalog().ok();
    let mut referenced: HashSet<ObjectId> = HashSet::new();
    if let Some(catalog) = catalog {
        structure.dss_present = catalog.has(b"DSS");
        structure.certification_byte_range = catalog
            .get(b"Perms")
            .ok()
            .and_then(|perms| resolve_dict(document, perms))
            .and_then(|perms| perms.get(b"DocMDP").ok())
            .and_then(|signature| resolve_dict(document, signature))
            .and_then(|signature| integer_array(document, signature.get(b"ByteRange").ok()?));
        let fields = catalog
            .get(b"AcroForm")
            .ok()
            .and_then(|acroform| resolve_dict(document, acroform))
            .and_then(|acroform| acroform.get(b"Fields").ok())
            .and_then(|fields| resolve(document, fields).as_array().ok().cloned())
            .unwrap_or_default();
        let mut walker = FieldWalker {
            document,
            visited: HashSet::new(),
            referenced: &mut referenced,
            structure: &mut structure,
            fields_seen: 0,
        };
        for field in &fields {
            walker.field(field, None, None, 0)?;
        }
    }
    for (id, object) in &document.objects {
        if referenced.contains(id) {
            continue;
        }
        if let Object::Dictionary(dictionary) = object {
            if is_signature_dictionary(dictionary) {
                structure
                    .unreferenced
                    .push(signature_from(document, None, dictionary));
                if structure.unreferenced.len() + structure.signed.len() > MAX_SIGNATURES {
                    return Err(StructureError::TooManySignatures);
                }
            }
        }
    }
    Ok(structure)
}

struct FieldWalker<'a> {
    document: &'a Document,
    visited: HashSet<ObjectId>,
    referenced: &'a mut HashSet<ObjectId>,
    structure: &'a mut Structure,
    fields_seen: usize,
}

impl FieldWalker<'_> {
    fn field(
        &mut self,
        object: &Object,
        parent_name: Option<&str>,
        inherited_type: Option<&[u8]>,
        depth: usize,
    ) -> Result<(), StructureError> {
        self.fields_seen += 1;
        if depth > MAX_FIELD_DEPTH || self.fields_seen > MAX_FIELDS {
            self.structure.truncated = true;
            return Ok(());
        }
        if let Object::Reference(id) = object {
            if !self.visited.insert(*id) {
                self.structure.truncated = true;
                return Ok(());
            }
        }
        let Some(dictionary) = resolve_dict(self.document, object) else {
            return Ok(());
        };
        let partial = dictionary
            .get(b"T")
            .ok()
            .and_then(|value| text(self.document, value));
        let name = match (parent_name, partial.as_deref()) {
            (Some(parent), Some(own)) => Some(format!("{parent}.{own}")),
            (None, Some(own)) => Some(own.to_string()),
            (parent, None) => parent.map(str::to_string),
        };
        let field_type = dictionary
            .get(b"FT")
            .ok()
            .and_then(|value| resolve(self.document, value).as_name().ok())
            .or(inherited_type);
        let kids: Vec<Object> = dictionary
            .get(b"Kids")
            .ok()
            .and_then(|kids| resolve(self.document, kids).as_array().ok().cloned())
            .unwrap_or_default();
        let named_kids: Vec<&Object> = kids
            .iter()
            .filter(|kid| {
                resolve_dict(self.document, kid)
                    .is_some_and(|kid| kid.has(b"T") || kid.has(b"Kids"))
            })
            .collect();
        if field_type == Some(b"Sig") {
            if let Ok(value) = dictionary.get(b"V") {
                if let Some(signature) = resolve_dict(self.document, value) {
                    if let Object::Reference(id) = value {
                        self.referenced.insert(*id);
                    }
                    let name = name
                        .as_deref()
                        .map(|value| limits::trim(value, limits::MAX_FIELD_NAME_CHARS));
                    self.structure
                        .signed
                        .push(signature_from(self.document, name, signature));
                    if self.structure.signed.len() > MAX_SIGNATURES {
                        return Err(StructureError::TooManySignatures);
                    }
                    return Ok(());
                }
            }
            if named_kids.is_empty() {
                self.structure.empty_fields = self.structure.empty_fields.saturating_add(1);
                return Ok(());
            }
        }
        for kid in named_kids {
            self.field(kid, name.as_deref(), field_type, depth + 1)?;
        }
        Ok(())
    }
}

fn is_signature_dictionary(dictionary: &Dictionary) -> bool {
    matches!(dictionary.get(b"Type"), Ok(Object::Name(name)) if name == b"Sig" || name == b"DocTimeStamp")
        || (dictionary.has(b"ByteRange") && dictionary.has(b"Contents"))
}

fn signature_from(
    document: &Document,
    field_name: Option<String>,
    dictionary: &Dictionary,
) -> StructuralSignature {
    let sub_filter = dictionary
        .get(b"SubFilter")
        .ok()
        .and_then(|value| resolve(document, value).as_name().ok())
        .map(|name| {
            limits::trim(
                &String::from_utf8_lossy(name),
                limits::MAX_NAME_COMPONENT_CHARS,
            )
        });
    let is_document_timestamp = matches!(dictionary.get(b"Type"), Ok(Object::Name(name)) if name == b"DocTimeStamp")
        || sub_filter.as_deref() == Some("ETSI.RFC3161");
    let mut signature = StructuralSignature {
        field_name,
        byte_range: dictionary
            .get(b"ByteRange")
            .ok()
            .and_then(|value| integer_array(document, value)),
        contents: dictionary.get(b"Contents").ok().and_then(|value| {
            match resolve(document, value) {
                Object::String(bytes, _) => Some(bytes.clone()),
                _ => None,
            }
        }),
        sub_filter,
        is_document_timestamp,
        pdf_m: dictionary
            .get(b"M")
            .ok()
            .and_then(|value| text(document, value))
            .map(|value| limits::trim(&value, limits::MAX_NAME_COMPONENT_CHARS)),
        reason: dictionary
            .get(b"Reason")
            .ok()
            .and_then(|value| text(document, value))
            .map(|value| limits::trim(&value, limits::MAX_REASON_CHARS)),
        docmdp_p: None,
        field_mdp: None,
    };
    let references = dictionary
        .get(b"Reference")
        .ok()
        .and_then(|value| resolve(document, value).as_array().ok().cloned())
        .unwrap_or_default();
    for reference in references.iter().take(16) {
        let Some(reference) = resolve_dict(document, reference) else {
            continue;
        };
        let method = reference
            .get(b"TransformMethod")
            .ok()
            .and_then(|value| resolve(document, value).as_name().ok());
        let params = reference
            .get(b"TransformParams")
            .ok()
            .and_then(|value| resolve_dict(document, value));
        match method {
            Some(b"DocMDP") => {
                signature.docmdp_p = Some(
                    params
                        .and_then(|params| params.get(b"P").ok())
                        .and_then(|value| resolve(document, value).as_i64().ok())
                        .unwrap_or(2),
                );
            }
            Some(b"FieldMDP") => {
                let action = params
                    .and_then(|params| params.get(b"Action").ok())
                    .and_then(|value| resolve(document, value).as_name().ok())
                    .map(|name| limits::trim(&String::from_utf8_lossy(name), 32))
                    .unwrap_or_default();
                let field_count = params
                    .and_then(|params| params.get(b"Fields").ok())
                    .and_then(|value| resolve(document, value).as_array().ok())
                    .map_or(0, |fields| u32::try_from(fields.len()).unwrap_or(u32::MAX));
                signature.field_mdp = Some(FieldMdpReport {
                    action,
                    field_count,
                });
            }
            _ => {}
        }
    }
    signature
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> &'a Object {
    let mut current = object;
    for _ in 0..8 {
        match current {
            Object::Reference(id) => match document.get_object(*id) {
                Ok(next) => current = next,
                Err(_) => return current,
            },
            _ => return current,
        }
    }
    current
}

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    resolve(document, object).as_dict().ok()
}

fn integer_array(document: &Document, object: &Object) -> Option<Vec<i64>> {
    let array = resolve(document, object).as_array().ok()?;
    if array.len() > 64 {
        return None;
    }
    array
        .iter()
        .map(|value| resolve(document, value).as_i64().ok())
        .collect()
}

/// A PDF text string: UTF-16BE with a BOM, else PDFDocEncoding (read as Latin-1, which
/// matches it for the printable range).
fn text(document: &Document, object: &Object) -> Option<String> {
    let Object::String(bytes, _) = resolve(document, object) else {
        return None;
    };
    Some(decode_text_string(bytes))
}

pub fn decode_text_string(bytes: &[u8]) -> String {
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = utf16
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_be_bytes(pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    bytes.iter().map(|byte| char::from(*byte)).collect()
}
