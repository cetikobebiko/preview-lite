use crate::document_session::SessionJobOperationError;
use crate::pdf_engine::{get_pdfium, open_pdfium_doc};
use crate::secure_redaction::{
    build_secure_redaction_plan, validate_secure_redaction_plan, CanonicalPageGeometry,
    DocumentRedactionFacts, GeometryProfile, InspectionFactState, PageRedactionFacts, PageRotation,
    PdfPointRect, RedactionAcknowledgements, RedactionPlannerInput, RedactionRegionInput,
    SecureRedactionPlan, SecureRedactionPolicy, REDACTION_SCHEMA_VERSION,
};
use lopdf::Object;
use pdfium_render::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritedLookup<T> {
    Found(T),
    Absent,
    Unknown,
}

fn pdfium_unavailable(msg: String) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(
        "SESSION_JOB_PDF_ENGINE_UNAVAILABLE",
        format!("PDFium motoru başlatılamadı: {msg}"),
    )
}

fn pdf_open_failed(_msg: String) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(
        "SESSION_JOB_PDF_OPEN_FAILED",
        "PDFium snapshot dosyasını açamadı veya kimlik doğrulaması başarısız oldu",
    )
}

pub fn inspect_and_plan_secure_redaction(
    snapshot_path: &Path,
    credential: Option<&str>,
    content_generation: u64,
    policy: SecureRedactionPolicy,
    acknowledgements: RedactionAcknowledgements,
    region_inputs: Vec<RedactionRegionInput>,
) -> Result<SecureRedactionPlan, SessionJobOperationError> {
    let input_digest = crate::document_io::sha256_file(snapshot_path).map_err(|e| {
        crate::secure_redaction::make_redaction_error(
            crate::secure_redaction::SecureRedactionErrorCode::RedactionPlanDigestMismatch,
            format!("Snapshot SHA-256 digest hesaplanamadı: {e}"),
        )
    })?;

    let pdfium = get_pdfium().map_err(pdfium_unavailable)?;
    let doc = open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
        .map_err(pdf_open_failed)?;

    let lopdf_doc = lopdf::Document::load(snapshot_path).ok();
    let raw_bytes = fs::read(snapshot_path).ok();

    let total_pages = doc.pages().len() as u32;

    let document_facts = inspect_document_facts(&doc, lopdf_doc.as_ref(), raw_bytes.as_deref())?;

    let mut page_geometries = Vec::with_capacity(total_pages as usize);
    let mut page_facts = Vec::with_capacity(total_pages as usize);

    for page_idx in 0..total_pages {
        let pdfium_page = doc.pages().get(page_idx as u16).map_err(|e| {
            crate::secure_redaction::make_redaction_error(
                crate::secure_redaction::SecureRedactionErrorCode::RedactionPageNotFound,
                format!("Sayfa {page_idx} PDFium ile yüklenemedi: {e}"),
            )
        })?;

        let geom = inspect_page_geometry(&pdfium_page, lopdf_doc.as_ref(), page_idx)?;
        let facts = inspect_page_facts(&pdfium_page, lopdf_doc.as_ref(), page_idx, &geom)?;

        page_geometries.push(geom);
        page_facts.push(facts);
    }

    let planner_input = RedactionPlannerInput {
        input_digest,
        content_generation,
        policy,
        acknowledgements,
        region_inputs,
        page_geometries,
        document_facts,
        page_facts,
    };

    let plan = build_secure_redaction_plan(&planner_input)?;
    validate_secure_redaction_plan(&plan)?;

    Ok(plan)
}

fn get_dict<'a>(
    lopdf_doc: &'a lopdf::Document,
    obj: &'a lopdf::Object,
) -> Option<&'a lopdf::Dictionary> {
    match obj {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => lopdf_doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn inspect_document_facts(
    doc: &PdfDocument<'_>,
    lopdf_doc: Option<&lopdf::Document>,
    raw_bytes: Option<&[u8]>,
) -> Result<DocumentRedactionFacts, SessionJobOperationError> {
    let total_pages = doc.pages().len() as u32;

    let encryption = match doc.permissions().security_handler_revision() {
        Ok(PdfSecurityHandlerRevision::Unprotected) => InspectionFactState::Absent,
        Ok(_) => InspectionFactState::Present,
        Err(_) => InspectionFactState::Unknown,
    };

    let (signature_dictionary, signature_field, byte_range) = if let Some(lopdf_doc) = lopdf_doc {
        let mut sig_dict = InspectionFactState::Absent;
        let mut sig_field = InspectionFactState::Absent;
        let mut br = InspectionFactState::Absent;

        let mut sig_dict_unknown = false;
        let mut sig_field_unknown = false;
        let mut br_unknown = false;

        for object in lopdf_doc.objects.values() {
            if let Object::Dictionary(dict) = object {
                if dict.has(b"Type") {
                    if let Ok(type_obj) = dict.get(b"Type") {
                        let deref_type = match type_obj {
                            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                            _ => None,
                        };
                        let target = deref_type.unwrap_or(type_obj);
                        if let Ok(name) = target.as_name() {
                            if name == b"Sig" {
                                sig_dict = InspectionFactState::Present;
                            }
                        } else if type_obj.as_reference().is_ok() && deref_type.is_none() {
                            sig_dict_unknown = true;
                        }
                    } else {
                        sig_dict_unknown = true;
                    }
                }
                if dict.has(b"FT") {
                    if let Ok(ft_obj) = dict.get(b"FT") {
                        let deref_ft = match ft_obj {
                            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                            _ => None,
                        };
                        let target = deref_ft.unwrap_or(ft_obj);
                        if let Ok(name) = target.as_name() {
                            if name == b"Sig" {
                                sig_field = InspectionFactState::Present;
                            }
                        } else if ft_obj.as_reference().is_ok() && deref_ft.is_none() {
                            sig_field_unknown = true;
                        }
                    } else {
                        sig_field_unknown = true;
                    }
                }
                if dict.has(b"ByteRange") {
                    if let Ok(br_obj) = dict.get(b"ByteRange") {
                        let deref_br = match br_obj {
                            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                            _ => None,
                        };
                        let target = deref_br.unwrap_or(br_obj);
                        if target.as_array().is_ok() {
                            br = InspectionFactState::Present;
                        } else if br_obj.as_reference().is_ok() && deref_br.is_none() {
                            br_unknown = true;
                        }
                    } else {
                        br_unknown = true;
                    }
                }
            }
        }

        if !sig_dict.is_present() && sig_dict_unknown {
            sig_dict = InspectionFactState::Unknown;
        }
        if !sig_field.is_present() && sig_field_unknown {
            sig_field = InspectionFactState::Unknown;
        }
        if !br.is_present() && br_unknown {
            br = InspectionFactState::Unknown;
        }

        (sig_dict, sig_field, br)
    } else {
        (
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
        )
    };

    let (acro_form, xfa) = if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"AcroForm") {
                if let Ok(acro_obj) = cat_dict.get(b"AcroForm") {
                    if let Some(acro_dict) = get_dict(lopdf_doc, acro_obj) {
                        let has_xfa = if acro_dict.has(b"XFA") {
                            if let Ok(xfa_obj) = acro_dict.get(b"XFA") {
                                match xfa_obj {
                                    Object::Array(_)
                                    | Object::Dictionary(_)
                                    | Object::Stream(_) => InspectionFactState::Present,
                                    Object::Reference(id) => {
                                        if lopdf_doc.get_object(*id).is_ok() {
                                            InspectionFactState::Present
                                        } else {
                                            InspectionFactState::Unknown
                                        }
                                    }
                                    _ => InspectionFactState::Unknown,
                                }
                            } else {
                                InspectionFactState::Unknown
                            }
                        } else {
                            InspectionFactState::Absent
                        };
                        (InspectionFactState::Present, has_xfa)
                    } else {
                        // AcroForm key exists but dictionary cannot be resolved
                        (InspectionFactState::Present, InspectionFactState::Unknown)
                    }
                } else {
                    (InspectionFactState::Present, InspectionFactState::Unknown)
                }
            } else {
                (InspectionFactState::Absent, InspectionFactState::Absent)
            }
        } else {
            (InspectionFactState::Unknown, InspectionFactState::Unknown)
        }
    } else {
        (InspectionFactState::Unknown, InspectionFactState::Unknown)
    };

    let annotations = {
        let mut has_annots = false;
        for i in 0..total_pages {
            if let Ok(page) = doc.pages().get(i as u16) {
                if !page.annotations().is_empty() {
                    has_annots = true;
                    break;
                }
            }
        }
        if has_annots {
            InspectionFactState::Present
        } else {
            InspectionFactState::Absent
        }
    };

    let optional_content = if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"OCProperties") {
                if let Ok(oc_obj) = cat_dict.get(b"OCProperties") {
                    if get_dict(lopdf_doc, oc_obj).is_some() {
                        InspectionFactState::Present
                    } else {
                        InspectionFactState::Unknown
                    }
                } else {
                    InspectionFactState::Unknown
                }
            } else {
                InspectionFactState::Absent
            }
        } else {
            InspectionFactState::Unknown
        }
    } else {
        InspectionFactState::Unknown
    };

    let embedded_files = if let Some(lopdf_doc) = lopdf_doc {
        inspect_embedded_files(lopdf_doc)
    } else {
        InspectionFactState::Unknown
    };

    let javascript = if let Some(lopdf_doc) = lopdf_doc {
        inspect_javascript(lopdf_doc)
    } else {
        InspectionFactState::Unknown
    };

    let incremental_revisions = inspect_incremental_revisions(lopdf_doc, raw_bytes);

    let structure_tree = if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"StructTreeRoot") {
                if let Ok(st_obj) = cat_dict.get(b"StructTreeRoot") {
                    if get_dict(lopdf_doc, st_obj).is_some() {
                        InspectionFactState::Present
                    } else {
                        InspectionFactState::Unknown
                    }
                } else {
                    InspectionFactState::Unknown
                }
            } else {
                InspectionFactState::Absent
            }
        } else {
            InspectionFactState::Unknown
        }
    } else {
        InspectionFactState::Unknown
    };

    let outlines = if doc.bookmarks().root().is_some() {
        InspectionFactState::Present
    } else if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"Outlines") {
                if let Ok(out_obj) = cat_dict.get(b"Outlines") {
                    if get_dict(lopdf_doc, out_obj).is_some() {
                        InspectionFactState::Present
                    } else {
                        InspectionFactState::Unknown
                    }
                } else {
                    InspectionFactState::Unknown
                }
            } else {
                InspectionFactState::Absent
            }
        } else {
            InspectionFactState::Unknown
        }
    } else {
        InspectionFactState::Unknown
    };

    let names_dictionary = if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"Names") {
                if let Ok(names_obj) = cat_dict.get(b"Names") {
                    if get_dict(lopdf_doc, names_obj).is_some() {
                        InspectionFactState::Present
                    } else {
                        InspectionFactState::Unknown
                    }
                } else {
                    InspectionFactState::Unknown
                }
            } else {
                InspectionFactState::Absent
            }
        } else {
            InspectionFactState::Unknown
        }
    } else {
        InspectionFactState::Unknown
    };

    let document_actions = if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"AA") || cat_dict.has(b"OpenAction") {
                InspectionFactState::Present
            } else {
                InspectionFactState::Absent
            }
        } else {
            InspectionFactState::Unknown
        }
    } else {
        InspectionFactState::Unknown
    };

    let complex_document_resources = if let Some(lopdf_doc) = lopdf_doc {
        if let Ok(cat_dict) = lopdf_doc.catalog() {
            if cat_dict.has(b"Requirements")
                || cat_dict.has(b"Collection")
                || cat_dict.has(b"NeedsRendering")
            {
                InspectionFactState::Present
            } else {
                InspectionFactState::Absent
            }
        } else {
            InspectionFactState::Unknown
        }
    } else {
        InspectionFactState::Unknown
    };

    Ok(DocumentRedactionFacts {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        total_pages,
        encryption,
        signature_dictionary,
        signature_field,
        byte_range,
        acro_form,
        xfa,
        annotations,
        optional_content,
        embedded_files,
        javascript,
        incremental_revisions,
        structure_tree,
        outlines,
        names_dictionary,
        document_actions,
        complex_document_resources,
    })
}

fn inspect_embedded_files(lopdf_doc: &lopdf::Document) -> InspectionFactState {
    inspect_embedded_files_with_budget(lopdf_doc, 10_000)
}

fn inspect_embedded_files_with_budget(
    lopdf_doc: &lopdf::Document,
    limit: usize,
) -> InspectionFactState {
    let cat_dict = match lopdf_doc.catalog() {
        Ok(c) => c,
        Err(_) => return InspectionFactState::Unknown,
    };

    if cat_dict.has(b"EF") || cat_dict.has(b"AF") {
        return InspectionFactState::Present;
    }

    if let Ok(names_obj) = cat_dict.get(b"Names") {
        if let Some(names_dict) = get_dict(lopdf_doc, names_obj) {
            if names_dict.has(b"EmbeddedFiles") {
                return InspectionFactState::Present;
            }
        } else if names_obj.as_reference().is_ok() {
            return InspectionFactState::Unknown;
        }
    }

    let mut unresolvable_candidate = false;
    let mut scanned_count = 0;
    let total_objects = lopdf_doc.objects.len();

    for (_id, object) in lopdf_doc.objects.iter().take(limit) {
        scanned_count += 1;
        if let Object::Dictionary(dict) = object {
            if dict.has(b"EF") || dict.has(b"AF") {
                return InspectionFactState::Present;
            }
            if let Ok(type_obj) = dict.get(b"Type") {
                if type_obj
                    .as_name()
                    .map(|n| n == b"Filespec" || n == b"F")
                    .unwrap_or(false)
                    && dict.has(b"EF")
                {
                    return InspectionFactState::Present;
                }
            }
            if let Ok(subtype_obj) = dict.get(b"Subtype") {
                if subtype_obj
                    .as_name()
                    .map(|n| n == b"FileAttachment")
                    .unwrap_or(false)
                    && (dict.has(b"FS") || dict.has(b"EF"))
                {
                    return InspectionFactState::Present;
                }
            }
        } else if let Object::Reference(id) = object {
            if lopdf_doc.get_object(*id).is_err() {
                unresolvable_candidate = true;
            }
        }
    }

    if scanned_count < total_objects {
        return InspectionFactState::Unknown;
    }

    if unresolvable_candidate {
        InspectionFactState::Unknown
    } else {
        InspectionFactState::Absent
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsSearchResult {
    Found,
    NotFound,
    Indeterminate,
}

fn dict_has_js(
    dict: &lopdf::Dictionary,
    lopdf_doc: &lopdf::Document,
    depth: usize,
    max_depth: usize,
    visited_ids: &mut HashSet<lopdf::ObjectId>,
) -> JsSearchResult {
    if depth > max_depth {
        return JsSearchResult::Indeterminate;
    }
    if dict.has(b"JS") || dict.has(b"JavaScript") {
        return JsSearchResult::Found;
    }
    if let Ok(s_obj) = dict.get(b"S") {
        let deref_s = match s_obj {
            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
            _ => None,
        };
        let s_target = deref_s.as_ref().map_or(s_obj, |v| v);
        if s_target
            .as_name()
            .map(|n| n == b"JavaScript")
            .unwrap_or(false)
        {
            return JsSearchResult::Found;
        }
    }

    let mut indeterminate = false;

    for (_k, v) in dict.iter() {
        let deref_v = match v {
            Object::Reference(id) => {
                if !visited_ids.insert(*id) {
                    continue;
                }
                match lopdf_doc.get_object(*id) {
                    Ok(obj) => Some(obj),
                    Err(_) => {
                        indeterminate = true;
                        None
                    }
                }
            }
            _ => None,
        };
        let target = deref_v.unwrap_or(v);
        match target {
            Object::Dictionary(inner_dict) => {
                match dict_has_js(inner_dict, lopdf_doc, depth + 1, max_depth, visited_ids) {
                    JsSearchResult::Found => return JsSearchResult::Found,
                    JsSearchResult::Indeterminate => indeterminate = true,
                    JsSearchResult::NotFound => {}
                }
            }
            Object::Array(arr) => {
                for item in arr {
                    let deref_item = match item {
                        Object::Reference(id) => {
                            if !visited_ids.insert(*id) {
                                continue;
                            }
                            match lopdf_doc.get_object(*id) {
                                Ok(obj) => Some(obj),
                                Err(_) => {
                                    indeterminate = true;
                                    None
                                }
                            }
                        }
                        _ => None,
                    };
                    let item_target = deref_item.unwrap_or(item);
                    if let Object::Dictionary(inner_dict) = item_target {
                        match dict_has_js(inner_dict, lopdf_doc, depth + 1, max_depth, visited_ids)
                        {
                            JsSearchResult::Found => return JsSearchResult::Found,
                            JsSearchResult::Indeterminate => indeterminate = true,
                            JsSearchResult::NotFound => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if indeterminate {
        JsSearchResult::Indeterminate
    } else {
        JsSearchResult::NotFound
    }
}

fn inspect_javascript(lopdf_doc: &lopdf::Document) -> InspectionFactState {
    inspect_javascript_with_budget(lopdf_doc, 10_000, 16)
}

fn inspect_javascript_with_budget(
    lopdf_doc: &lopdf::Document,
    limit: usize,
    max_depth: usize,
) -> InspectionFactState {
    let cat_dict = match lopdf_doc.catalog() {
        Ok(c) => c,
        Err(_) => return InspectionFactState::Unknown,
    };

    if let Ok(names_obj) = cat_dict.get(b"Names") {
        if let Some(names_dict) = get_dict(lopdf_doc, names_obj) {
            if names_dict.has(b"JavaScript") || names_dict.has(b"JS") {
                return InspectionFactState::Present;
            }
        } else if names_obj.as_reference().is_ok() {
            return InspectionFactState::Unknown;
        }
    }

    let mut scanned_count = 0;
    let total_objects = lopdf_doc.objects.len();
    let mut indeterminate_found = false;

    for (id, object) in lopdf_doc.objects.iter().take(limit) {
        scanned_count += 1;
        if let Object::Dictionary(dict) = object {
            let mut visited_ids = HashSet::new();
            visited_ids.insert(*id);
            match dict_has_js(dict, lopdf_doc, 0, max_depth, &mut visited_ids) {
                JsSearchResult::Found => return InspectionFactState::Present,
                JsSearchResult::Indeterminate => indeterminate_found = true,
                JsSearchResult::NotFound => {}
            }
        }
    }

    if scanned_count < total_objects {
        return InspectionFactState::Unknown;
    }

    if indeterminate_found {
        InspectionFactState::Unknown
    } else {
        InspectionFactState::Absent
    }
}

fn inspect_incremental_revisions(
    lopdf_doc: Option<&lopdf::Document>,
    raw_bytes: Option<&[u8]>,
) -> InspectionFactState {
    // lopdf 0.45 strips /Prev on load, so the trailer is read from the raw bytes.
    if let (Some(lopdf_doc), Some(bytes)) = (lopdf_doc, raw_bytes) {
        if crate::lopdf_compat::trailer_history(bytes, lopdf_doc)
            .is_some_and(|history| history.prev)
        {
            return InspectionFactState::Present;
        }
    }

    if let Some(bytes) = raw_bytes {
        let (count, malformed) = scan_structural_startxref_revisions(bytes);
        if count > 1 {
            return InspectionFactState::Present;
        }
        if malformed {
            return InspectionFactState::Unknown;
        }
    }

    if lopdf_doc.is_some() {
        InspectionFactState::Absent
    } else {
        InspectionFactState::Unknown
    }
}

fn scan_structural_startxref_revisions(bytes: &[u8]) -> (usize, bool) {
    let mut valid_offsets = HashSet::new();
    let mut candidate_malformed = false;
    let len = bytes.len();
    let mut i = 0;

    let mut in_stream = false;
    let mut in_comment = false;
    let mut in_string = false;
    let mut string_depth: usize = 0;
    let mut in_hex_string = false;

    while i < len {
        if in_comment {
            if bytes[i] == b'\n' || bytes[i] == b'\r' {
                in_comment = false;
            }
            i += 1;
            continue;
        }

        if in_string {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'(' {
                string_depth += 1;
            } else if bytes[i] == b')' {
                string_depth -= 1;
                if string_depth == 0 {
                    in_string = false;
                }
            }
            i += 1;
            continue;
        }

        if in_hex_string {
            if bytes[i] == b'>' {
                in_hex_string = false;
            }
            i += 1;
            continue;
        }

        if in_stream {
            if i + 9 <= len && &bytes[i..i + 9] == b"endstream" {
                in_stream = false;
                i += 9;
            } else {
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'%' {
            in_comment = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'(' {
            in_string = true;
            string_depth = 1;
            i += 1;
            continue;
        }
        if i + 2 <= len && &bytes[i..i + 2] == b"<<" {
            i += 2;
            continue;
        }
        if bytes[i] == b'<' {
            in_hex_string = true;
            i += 1;
            continue;
        }
        if i + 6 <= len && &bytes[i..i + 6] == b"stream" {
            in_stream = true;
            i += 6;
            continue;
        }

        if i + 9 <= len && &bytes[i..i + 9] == b"startxref" {
            i += 9;
            while i < len
                && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n')
            {
                i += 1;
            }
            let start_digit = i;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if start_digit == i {
                candidate_malformed = true;
                continue;
            }
            let digit_str = match std::str::from_utf8(&bytes[start_digit..i]) {
                Ok(s) => s,
                Err(_) => {
                    candidate_malformed = true;
                    continue;
                }
            };
            let offset_val: usize = match digit_str.parse() {
                Ok(v) => v,
                Err(_) => {
                    candidate_malformed = true;
                    continue;
                }
            };

            if offset_val < len {
                let sub = &bytes[offset_val..];
                let trimmed_sub = sub
                    .iter()
                    .position(|b| !b.is_ascii_whitespace())
                    .map_or(sub, |pos| &sub[pos..]);
                if is_valid_classic_xref(trimmed_sub) || is_valid_xref_stream_obj(trimmed_sub) {
                    valid_offsets.insert(offset_val);
                } else {
                    candidate_malformed = true;
                }
            } else {
                candidate_malformed = true;
            }
            continue;
        }

        i += 1;
    }

    (valid_offsets.len(), candidate_malformed)
}

fn is_valid_classic_xref(sub: &[u8]) -> bool {
    if !sub.starts_with(b"xref") {
        return false;
    }
    let rest = &sub[4..];
    if rest.is_empty() {
        return false;
    }
    if !rest[0].is_ascii_whitespace() && rest[0] != b'%' {
        return false;
    }
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == b'%' {
            while i < rest.len() && rest[i] != b'\r' && rest[i] != b'\n' {
                i += 1;
            }
        } else if rest[i].is_ascii_whitespace() {
            i += 1;
        } else {
            break;
        }
    }
    if i >= rest.len() || !rest[i].is_ascii_digit() {
        return false;
    }
    true
}

fn is_valid_xref_stream_obj(sub: &[u8]) -> bool {
    let mut i = 0;
    while i < sub.len() && sub[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    while i < sub.len() && (sub[i] == b' ' || sub[i] == b'\t' || sub[i] == b'\r' || sub[i] == b'\n')
    {
        i += 1;
    }
    let gen_start = i;
    while i < sub.len() && sub[i].is_ascii_digit() {
        i += 1;
    }
    if gen_start == i {
        return false;
    }
    while i < sub.len() && (sub[i] == b' ' || sub[i] == b'\t' || sub[i] == b'\r' || sub[i] == b'\n')
    {
        i += 1;
    }
    if i + 3 > sub.len() || &sub[i..i + 3] != b"obj" {
        return false;
    }
    i += 3;
    if i < sub.len() && !sub[i].is_ascii_whitespace() && sub[i] != b'%' && sub[i] != b'<' {
        return false;
    }

    while i < sub.len() {
        if sub[i] == b'%' {
            while i < sub.len() && sub[i] != b'\r' && sub[i] != b'\n' {
                i += 1;
            }
        } else if sub[i].is_ascii_whitespace() {
            i += 1;
        } else {
            break;
        }
    }

    if i + 2 > sub.len() || &sub[i..i + 2] != b"<<" {
        return false;
    }
    i += 2;

    while i < sub.len() {
        if i + 2 <= sub.len() && &sub[i..i + 2] == b">>" {
            break;
        }
        if i + 6 <= sub.len() && &sub[i..i + 6] == b"stream" {
            break;
        }
        if i + 6 <= sub.len() && &sub[i..i + 6] == b"endobj" {
            break;
        }

        if sub[i] == b'/' {
            let name_start = i + 1;
            let mut j = name_start;
            while j < sub.len()
                && !sub[j].is_ascii_whitespace()
                && sub[j] != b'/'
                && sub[j] != b'<'
                && sub[j] != b'>'
                && sub[j] != b'['
                && sub[j] != b']'
                && sub[j] != b'('
                && sub[j] != b')'
                && sub[j] != b'%'
            {
                j += 1;
            }
            let key_name = &sub[name_start..j];
            if key_name == b"Type" {
                let mut k = j;
                while k < sub.len() {
                    if sub[k] == b'%' {
                        while k < sub.len() && sub[k] != b'\r' && sub[k] != b'\n' {
                            k += 1;
                        }
                    } else if sub[k].is_ascii_whitespace() {
                        k += 1;
                    } else {
                        break;
                    }
                }
                if k < sub.len() && sub[k] == b'/' {
                    let val_start = k + 1;
                    let mut val_end = val_start;
                    while val_end < sub.len()
                        && !sub[val_end].is_ascii_whitespace()
                        && sub[val_end] != b'/'
                        && sub[val_end] != b'<'
                        && sub[val_end] != b'>'
                        && sub[val_end] != b'['
                        && sub[val_end] != b']'
                        && sub[val_end] != b'('
                        && sub[val_end] != b')'
                        && sub[val_end] != b'%'
                    {
                        val_end += 1;
                    }
                    let val_name = &sub[val_start..val_end];
                    return val_name == b"XRef";
                } else {
                    return false;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    false
}

pub fn resolve_inherited_box(
    lopdf_doc: &lopdf::Document,
    page_id: lopdf::ObjectId,
    box_name: &[u8],
) -> InheritedLookup<PdfPointRect> {
    let mut current_id = page_id;
    let mut visited = HashSet::new();
    let mut depth = 0;

    while depth < 32 && visited.insert(current_id) {
        depth += 1;
        let dict = match lopdf_doc.get_dictionary(current_id) {
            Ok(d) => d,
            Err(_) => return InheritedLookup::Unknown,
        };

        if dict.has(box_name) {
            let arr_obj = match dict.get(box_name) {
                Ok(obj) => obj,
                Err(_) => return InheritedLookup::Unknown,
            };

            let deref_arr = match arr_obj {
                Object::Array(arr) => Some(arr),
                Object::Reference(ref_id) => match lopdf_doc.get_object(*ref_id) {
                    Ok(Object::Array(arr)) => Some(arr),
                    _ => None,
                },
                _ => None,
            };

            let arr = match deref_arr {
                Some(a) => a,
                None => return InheritedLookup::Unknown,
            };

            if arr.len() != 4 {
                return InheritedLookup::Unknown;
            }

            let mut coords = Vec::with_capacity(4);
            for item in arr {
                let deref_item = match item {
                    Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                    _ => None,
                };
                let target = deref_item.unwrap_or(item);
                let val = match target.as_float() {
                    Ok(f) => f as f64,
                    Err(_) => match target.as_i64() {
                        Ok(i) => i as f64,
                        Err(_) => return InheritedLookup::Unknown,
                    },
                };
                if !val.is_finite() {
                    return InheritedLookup::Unknown;
                }
                coords.push(val);
            }

            let left = coords[0].min(coords[2]);
            let bottom = coords[1].min(coords[3]);
            let width = (coords[2] - coords[0]).abs();
            let height = (coords[3] - coords[1]).abs();

            if width <= 0.0
                || height <= 0.0
                || !left.is_finite()
                || !bottom.is_finite()
                || !(left + width).is_finite()
                || !(bottom + height).is_finite()
            {
                return InheritedLookup::Unknown;
            }

            return InheritedLookup::Found(PdfPointRect {
                left,
                bottom,
                width,
                height,
            });
        }

        if dict.has(b"Parent") {
            match dict.get(b"Parent") {
                Ok(parent_obj) => match parent_obj.as_reference() {
                    Ok(parent_id) => {
                        current_id = parent_id;
                        continue;
                    }
                    Err(_) => return InheritedLookup::Unknown,
                },
                Err(_) => return InheritedLookup::Unknown,
            }
        } else {
            return InheritedLookup::Absent;
        }
    }

    InheritedLookup::Unknown
}

fn inspect_page_geometry(
    pdfium_page: &PdfPage<'_>,
    lopdf_doc: Option<&lopdf::Document>,
    page_idx: u32,
) -> Result<CanonicalPageGeometry, SessionJobOperationError> {
    let rotation = match pdfium_page.rotation() {
        Ok(rot) => {
            let deg = (rot.as_degrees() as i32).rem_euclid(360);
            match deg {
                0 => PageRotation::Deg0,
                90 => PageRotation::Deg90,
                180 => PageRotation::Deg180,
                270 => PageRotation::Deg270,
                _ => {
                    return Err(crate::secure_redaction::make_redaction_error(
                        crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                        format!("Sayfa {page_idx} desteklenmeyen dönüş açısına sahip: {deg}"),
                    ));
                }
            }
        }
        Err(_) => {
            return Err(crate::secure_redaction::make_redaction_error(
                crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Sayfa {page_idx} rotation okunamadı"),
            ));
        }
    };

    let media_box_lookup = if let Some(lopdf_doc) = lopdf_doc {
        let pages = lopdf_doc.get_pages();
        if let Some(&page_id) = pages.get(&(page_idx + 1)) {
            resolve_inherited_box(lopdf_doc, page_id, b"MediaBox")
        } else {
            InheritedLookup::Absent
        }
    } else {
        InheritedLookup::Absent
    };

    let media_box = match media_box_lookup {
        InheritedLookup::Found(mb) => mb,
        InheritedLookup::Unknown => {
            return Err(crate::secure_redaction::make_redaction_error(
                crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Sayfa {page_idx} MediaBox malformed, unresolved or traversal failed"),
            ));
        }
        InheritedLookup::Absent => {
            if let Ok(media_b) = pdfium_page.boundaries().media() {
                let left = media_b.bounds.left().value as f64;
                let bottom = media_b.bounds.bottom().value as f64;
                let right = media_b.bounds.right().value as f64;
                let top = media_b.bounds.top().value as f64;
                let width = (right - left).abs();
                let height = (top - bottom).abs();
                if left.is_finite()
                    && bottom.is_finite()
                    && width > 0.0
                    && height > 0.0
                    && (left + width).is_finite()
                    && (bottom + height).is_finite()
                {
                    PdfPointRect {
                        left,
                        bottom,
                        width,
                        height,
                    }
                } else {
                    return Err(crate::secure_redaction::make_redaction_error(
                        crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                        format!("Sayfa {page_idx} MediaBox bounds from PDFium are invalid"),
                    ));
                }
            } else {
                return Err(crate::secure_redaction::make_redaction_error(
                    crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                    format!("Sayfa {page_idx} MediaBox could not be resolved"),
                ));
            }
        }
    };

    let crop_box_lookup = if let Some(lopdf_doc) = lopdf_doc {
        let pages = lopdf_doc.get_pages();
        if let Some(&page_id) = pages.get(&(page_idx + 1)) {
            resolve_inherited_box(lopdf_doc, page_id, b"CropBox")
        } else {
            InheritedLookup::Absent
        }
    } else {
        InheritedLookup::Absent
    };

    let raw_crop_box = match crop_box_lookup {
        InheritedLookup::Found(cb) => Some(cb),
        InheritedLookup::Unknown => {
            return Err(crate::secure_redaction::make_redaction_error(
                crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Sayfa {page_idx} CropBox is malformed, unresolved or traversal failed"),
            ));
        }
        InheritedLookup::Absent => {
            if let Ok(crop_b) = pdfium_page.boundaries().crop() {
                let left = crop_b.bounds.left().value as f64;
                let bottom = crop_b.bounds.bottom().value as f64;
                let right = crop_b.bounds.right().value as f64;
                let top = crop_b.bounds.top().value as f64;
                let width = (right - left).abs();
                let height = (top - bottom).abs();
                if left.is_finite()
                    && bottom.is_finite()
                    && width > 0.0
                    && height > 0.0
                    && (left + width).is_finite()
                    && (bottom + height).is_finite()
                {
                    Some(PdfPointRect {
                        left,
                        bottom,
                        width,
                        height,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }
    };

    let crop_box = match raw_crop_box {
        Some(cb) => {
            let eps = 1e-6;
            if (cb.left - media_box.left).abs() < eps
                && (cb.bottom - media_box.bottom).abs() < eps
                && (cb.width - media_box.width).abs() < eps
                && (cb.height - media_box.height).abs() < eps
            {
                None
            } else {
                Some(cb)
            }
        }
        None => None,
    };

    // These boxes do not determine the visible viewport, but they are part of
    // the source page's security-relevant geometry. Preserve presence exactly;
    // unlike CropBox, equality with MediaBox does not mean absence.
    let trim_box =
        inspect_optional_page_box(pdfium_page, lopdf_doc, page_idx, b"TrimBox", "TrimBox")?;
    let bleed_box =
        inspect_optional_page_box(pdfium_page, lopdf_doc, page_idx, b"BleedBox", "BleedBox")?;
    let art_box = inspect_optional_page_box(pdfium_page, lopdf_doc, page_idx, b"ArtBox", "ArtBox")?;

    let effective_box = match &crop_box {
        Some(cb) => cb.clone(),
        None => media_box.clone(),
    };

    Ok(CanonicalPageGeometry {
        page_index: page_idx,
        media_box,
        crop_box,
        trim_box,
        bleed_box,
        art_box,
        effective_box,
        rotation,
    })
}

fn boundary_box_to_pdf_point_rect(boundary: PdfPageBoundaryBox) -> Option<PdfPointRect> {
    let left = boundary.bounds.left().value as f64;
    let bottom = boundary.bounds.bottom().value as f64;
    let right = boundary.bounds.right().value as f64;
    let top = boundary.bounds.top().value as f64;
    let width = right - left;
    let height = top - bottom;
    (left.is_finite()
        && bottom.is_finite()
        && width.is_finite()
        && height.is_finite()
        && width > 0.0
        && height > 0.0)
        .then_some(PdfPointRect {
            left,
            bottom,
            width,
            height,
        })
}

fn inspect_optional_page_box(
    pdfium_page: &PdfPage<'_>,
    lopdf_doc: Option<&lopdf::Document>,
    page_idx: u32,
    inherited_name: &[u8],
    label: &str,
) -> Result<Option<PdfPointRect>, SessionJobOperationError> {
    if let Some(lopdf_doc) = lopdf_doc {
        let pages = lopdf_doc.get_pages();
        let Some(&page_id) = pages.get(&(page_idx + 1)) else {
            return Ok(None);
        };
        return match resolve_inherited_box(lopdf_doc, page_id, inherited_name) {
            InheritedLookup::Found(rect) => Ok(Some(rect)),
            InheritedLookup::Absent => Ok(None),
            InheritedLookup::Unknown => Err(crate::secure_redaction::make_redaction_error(
                crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Sayfa {page_idx} {label} malformed, unresolved or traversal failed"),
            )),
        };
    }

    let boundary = match inherited_name {
        b"TrimBox" => pdfium_page.boundaries().trim(),
        b"BleedBox" => pdfium_page.boundaries().bleed(),
        b"ArtBox" => pdfium_page.boundaries().art(),
        _ => unreachable!("only optional output boundary boxes are requested"),
    };
    match boundary {
        Ok(boundary) => boundary_box_to_pdf_point_rect(boundary)
            .map(Some)
            .ok_or_else(|| {
                crate::secure_redaction::make_redaction_error(
                crate::secure_redaction::SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Sayfa {page_idx} {label} bounds from PDFium are invalid"),
            )
            }),
        // Optional boundary getters report an error when the box is absent.
        Err(_) => Ok(None),
    }
}

pub fn resolve_effective_resources(
    lopdf_doc: &lopdf::Document,
    page_id: lopdf::ObjectId,
) -> InheritedLookup<&lopdf::Dictionary> {
    let mut current_id = page_id;
    let mut visited = HashSet::new();
    let mut depth = 0;

    while depth < 32 && visited.insert(current_id) {
        depth += 1;
        let page_or_parent = match lopdf_doc.get_dictionary(current_id) {
            Ok(dict) => dict,
            Err(_) => return InheritedLookup::Unknown,
        };

        if page_or_parent.has(b"Resources") {
            return match page_or_parent.get(b"Resources") {
                Ok(res_obj) => match get_dict(lopdf_doc, res_obj) {
                    Some(res_dict) => InheritedLookup::Found(res_dict),
                    None => InheritedLookup::Unknown,
                },
                Err(_) => InheritedLookup::Unknown,
            };
        }

        if page_or_parent.has(b"Parent") {
            match page_or_parent.get(b"Parent") {
                Ok(parent_obj) => match parent_obj.as_reference() {
                    Ok(parent_id) => {
                        current_id = parent_id;
                        continue;
                    }
                    Err(_) => return InheritedLookup::Unknown,
                },
                Err(_) => return InheritedLookup::Unknown,
            }
        } else {
            return InheritedLookup::Absent;
        }
    }

    InheritedLookup::Unknown
}

fn inspect_color_space_object(
    obj: &lopdf::Object,
    lopdf_doc: &lopdf::Document,
    spot: &mut InspectionFactState,
    comp_cs: &mut InspectionFactState,
) {
    let deref = match obj {
        Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
        _ => None,
    };
    let target = deref.as_ref().map_or(obj, |v| v);

    if let Ok(name) = target.as_name() {
        if name == b"Separation" || name == b"DeviceN" {
            *spot = InspectionFactState::Present;
        } else if name == b"Indexed" || name == b"ICCBased" || name == b"Lab" {
            *comp_cs = InspectionFactState::Present;
        }
    } else if let Ok(arr) = target.as_array() {
        inspect_color_space_array(arr, spot, comp_cs);
    }
}

fn inspect_color_space_array(
    arr: &[lopdf::Object],
    spot: &mut InspectionFactState,
    comp_cs: &mut InspectionFactState,
) {
    if let Some(first) = arr.first().and_then(|o| o.as_name().ok()) {
        if first == b"Separation" || first == b"DeviceN" {
            *spot = InspectionFactState::Present;
        } else if first == b"Indexed" || first == b"ICCBased" || first == b"Lab" {
            *comp_cs = InspectionFactState::Present;
        }
    }
}

fn inspect_page_facts(
    pdfium_page: &PdfPage<'_>,
    lopdf_doc: Option<&lopdf::Document>,
    page_idx: u32,
    geom: &CanonicalPageGeometry,
) -> Result<PageRedactionFacts, SessionJobOperationError> {
    let mut has_text = false;
    let mut has_images = false;
    let mut has_paths = false;
    let mut has_form_xobjects = false;

    for obj in pdfium_page.objects().iter() {
        match obj.object_type() {
            PdfPageObjectType::Text => has_text = true,
            PdfPageObjectType::Image => has_images = true,
            PdfPageObjectType::Path => has_paths = true,
            PdfPageObjectType::XObjectForm => has_form_xobjects = true,
            _ => {}
        }
    }

    if !has_text {
        if let Ok(text_page) = pdfium_page.text() {
            if !text_page.all().trim().is_empty() {
                has_text = true;
            }
        }
    }

    let text = if has_text {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let images = if has_images {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let vector_paths = if has_paths {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let form_xobjects = if has_form_xobjects {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let annots = pdfium_page.annotations();
    let has_annotations = !annots.is_empty();
    let annotations = if has_annotations {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let mut has_link = false;
    let mut has_widget = false;

    for annot in annots.iter() {
        match annot.annotation_type() {
            PdfPageAnnotationType::Link => has_link = true,
            PdfPageAnnotationType::Widget => has_widget = true,
            _ => {}
        }
    }

    let link_annotations = if has_link {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let widgets = if has_widget {
        InspectionFactState::Present
    } else {
        InspectionFactState::Absent
    };

    let (
        transparency,
        optional_content,
        complex_resources,
        structure_association,
        page_actions,
        spot_colors,
        complex_color_spaces,
    ) = if let Some(lopdf_doc) = lopdf_doc {
        let pages = lopdf_doc.get_pages();
        if let Some(&page_id) = pages.get(&(page_idx + 1)) {
            if let Ok(page_dict) = lopdf_doc.get_dictionary(page_id) {
                let mut transp = InspectionFactState::Absent;
                let mut oc = InspectionFactState::Absent;
                let mut comp_res = InspectionFactState::Absent;
                let mut struct_assoc = InspectionFactState::Absent;
                let mut actions = InspectionFactState::Absent;
                let mut spot = InspectionFactState::Absent;
                let mut comp_cs = InspectionFactState::Absent;

                if page_dict.has(b"Group") {
                    transp = InspectionFactState::Present;
                }
                if page_dict.has(b"StructParents") || page_dict.has(b"Tabs") {
                    struct_assoc = InspectionFactState::Present;
                }
                if page_dict.has(b"AA") || page_dict.has(b"Actions") {
                    actions = InspectionFactState::Present;
                }
                if page_dict.has(b"OC") {
                    oc = InspectionFactState::Present;
                }

                let effective_resources = resolve_effective_resources(lopdf_doc, page_id);
                match effective_resources {
                    InheritedLookup::Found(res_dict) => {
                        if res_dict.has(b"Shading") || res_dict.has(b"Pattern") {
                            comp_res = InspectionFactState::Present;
                        }
                        if res_dict.has(b"Properties") && oc == InspectionFactState::Absent {
                            oc = InspectionFactState::Present;
                        }
                        if let Ok(egs_obj) = res_dict.get(b"ExtGState") {
                            if let Some(egs_dict) = get_dict(lopdf_doc, egs_obj) {
                                for (_k, v) in egs_dict.iter() {
                                    if let Some(sub_dict) = get_dict(lopdf_doc, v) {
                                        if sub_dict.has(b"CA")
                                            || sub_dict.has(b"ca")
                                            || sub_dict.has(b"SMask")
                                            || sub_dict.has(b"BM")
                                        {
                                            transp = InspectionFactState::Present;
                                        }
                                    } else if v.as_reference().is_ok()
                                        && transp == InspectionFactState::Absent
                                    {
                                        transp = InspectionFactState::Unknown;
                                    }
                                }
                            } else if egs_obj.as_reference().is_ok()
                                && transp == InspectionFactState::Absent
                            {
                                transp = InspectionFactState::Unknown;
                            }
                        }
                        if let Ok(cs_obj) = res_dict.get(b"ColorSpace") {
                            if let Some(cs_dict) = get_dict(lopdf_doc, cs_obj) {
                                for (_k, v) in cs_dict.iter() {
                                    inspect_color_space_object(
                                        v,
                                        lopdf_doc,
                                        &mut spot,
                                        &mut comp_cs,
                                    );
                                }
                            } else if let Ok(arr) = cs_obj.as_array() {
                                inspect_color_space_array(arr, &mut spot, &mut comp_cs);
                            } else if cs_obj.as_reference().is_ok() {
                                if spot == InspectionFactState::Absent {
                                    spot = InspectionFactState::Unknown;
                                }
                                if comp_cs == InspectionFactState::Absent {
                                    comp_cs = InspectionFactState::Unknown;
                                }
                            }
                        }
                    }
                    InheritedLookup::Unknown => {
                        if transp == InspectionFactState::Absent {
                            transp = InspectionFactState::Unknown;
                        }
                        if oc == InspectionFactState::Absent {
                            oc = InspectionFactState::Unknown;
                        }
                        if comp_res == InspectionFactState::Absent {
                            comp_res = InspectionFactState::Unknown;
                        }
                        if spot == InspectionFactState::Absent {
                            spot = InspectionFactState::Unknown;
                        }
                        if comp_cs == InspectionFactState::Absent {
                            comp_cs = InspectionFactState::Unknown;
                        }
                    }
                    InheritedLookup::Absent => {}
                }

                (transp, oc, comp_res, struct_assoc, actions, spot, comp_cs)
            } else {
                (
                    InspectionFactState::Unknown,
                    InspectionFactState::Unknown,
                    InspectionFactState::Unknown,
                    InspectionFactState::Unknown,
                    InspectionFactState::Unknown,
                    InspectionFactState::Unknown,
                    InspectionFactState::Unknown,
                )
            }
        } else {
            (
                InspectionFactState::Unknown,
                InspectionFactState::Unknown,
                InspectionFactState::Unknown,
                InspectionFactState::Unknown,
                InspectionFactState::Unknown,
                InspectionFactState::Unknown,
                InspectionFactState::Unknown,
            )
        }
    } else {
        (
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
            InspectionFactState::Unknown,
        )
    };

    Ok(PageRedactionFacts {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        page_index: page_idx,
        text,
        images,
        vector_paths,
        form_xobjects,
        annotations,
        link_annotations,
        widgets,
        transparency,
        optional_content,
        complex_resources,
        structure_association,
        page_actions,
        spot_colors,
        complex_color_spaces,
        geometry_profile: GeometryProfile::StandardEffectiveBoxV1,
        rotation: geom.rotation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object};

    #[test]
    fn test_scan_structural_startxref_single_revision() {
        let prefix = b"header\n";
        let xref_offset = prefix.len();
        let pdf_str = format!(
            "header\nxref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\n%%EOF\n",
            xref_offset
        );
        let (count, malformed) = scan_structural_startxref_revisions(pdf_str.as_bytes());
        assert_eq!(count, 1);
        assert!(!malformed);
    }

    #[test]
    fn test_scan_structural_startxref_incremental_revision() {
        let rev1_prefix = b"header\n";
        let offset1 = rev1_prefix.len();
        let rev1_body = format!(
            "header\nxref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\nheader2\n",
            offset1
        );
        let offset2 = rev1_body.len();
        let pdf_str = format!(
            "header\nxref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\nheader2\nxref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\n%%EOF\n",
            offset1, offset2
        );
        let (count, malformed) = scan_structural_startxref_revisions(pdf_str.as_bytes());
        assert!(count >= 2);
        assert!(!malformed);
        let rev = inspect_incremental_revisions(None, Some(pdf_str.as_bytes()));
        assert_eq!(rev, InspectionFactState::Present);
    }

    #[test]
    fn test_scan_structural_startxref_stream_literal() {
        let prefix = b"1 0 obj\n<< /Length 25 >>\nstream\nstartxref\n100\nendstream\nendobj\n";
        let xref_offset = prefix.len();
        let pdf_str = format!(
            "1 0 obj\n<< /Length 25 >>\nstream\nstartxref\n100\nendstream\nendobj\nxref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\n%%EOF\n",
            xref_offset
        );
        let (count, malformed) = scan_structural_startxref_revisions(pdf_str.as_bytes());
        assert_eq!(count, 1);
        assert!(!malformed);
    }

    #[test]
    fn test_scan_structural_startxref_comment_literal() {
        let prefix = b"% startxref 100\n";
        let xref_offset = prefix.len();
        let pdf_str = format!(
            "% startxref 100\nxref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\n%%EOF\n",
            xref_offset
        );
        let (count, malformed) = scan_structural_startxref_revisions(pdf_str.as_bytes());
        assert_eq!(count, 1);
        assert!(!malformed);
    }

    #[test]
    fn test_scan_structural_startxref_string_literal() {
        let prefix = b"(startxref 100) ";
        let xref_offset = prefix.len();
        let pdf_str = format!(
            "(startxref 100) xref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\n%%EOF\n",
            xref_offset
        );
        let (count, malformed) = scan_structural_startxref_revisions(pdf_str.as_bytes());
        assert_eq!(count, 1);
        assert!(!malformed);
    }

    #[test]
    fn test_scan_structural_startxref_hex_string_literal() {
        let prefix = b"<73746172747872656620313030> ";
        let xref_offset = prefix.len();
        let pdf_str = format!(
            "<73746172747872656620313030> xref\n0 1\n0000000000 65535 f\ntrailer << >>\nstartxref\n{}\n%%EOF\n",
            xref_offset
        );
        let (count, malformed) = scan_structural_startxref_revisions(pdf_str.as_bytes());
        assert_eq!(count, 1);
        assert!(!malformed);
    }

    #[test]
    fn test_scan_structural_startxref_arbitrary_object() {
        let pdf = b"1 0 obj\n<< /Type /SomethingElse >>\nendobj\nstartxref\n0\n%%EOF\n";
        let (count, malformed) = scan_structural_startxref_revisions(pdf);
        assert_eq!(count, 0);
        assert!(malformed);
        let rev = inspect_incremental_revisions(None, Some(pdf));
        assert_eq!(rev, InspectionFactState::Unknown);
    }

    #[test]
    fn test_scan_structural_startxref_real_xref_stream() {
        let pdf = b"1 0 obj\n<< /Type /XRef /Size 10 /W [1 1 1] >>\nstream\n12345\nendstream\nendobj\nstartxref\n0\n%%EOF\n";
        let (count, malformed) = scan_structural_startxref_revisions(pdf);
        assert_eq!(count, 1);
        assert!(!malformed);
    }

    #[test]
    fn test_scan_structural_startxref_out_of_range() {
        let pdf = b"startxref\n999999\n%%EOF\n";
        let (count, malformed) = scan_structural_startxref_revisions(pdf);
        assert_eq!(count, 0);
        assert!(malformed);
        let rev = inspect_incremental_revisions(None, Some(pdf));
        assert_eq!(rev, InspectionFactState::Unknown);
    }

    #[test]
    fn test_inspect_embedded_files_budget_exhaustion() {
        let mut doc = lopdf::Document::new();
        let cat_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
        });
        doc.trailer.set("Root", cat_id);

        for _ in 0..10 {
            doc.add_object(dictionary! {
                "Type" => "Page",
            });
        }

        // Entire doc scan with infinite budget -> Absent
        let res_absent = inspect_embedded_files_with_budget(&doc, 100);
        assert_eq!(res_absent, InspectionFactState::Absent);

        // Scan with tight budget (e.g. 2 objects out of 11 total) -> Unknown
        let res_unknown = inspect_embedded_files_with_budget(&doc, 2);
        assert_eq!(res_unknown, InspectionFactState::Unknown);
    }

    #[test]
    fn test_js_search_regression_suite() {
        // 1. Normal graph without JS -> Absent
        let mut doc_normal = lopdf::Document::new();
        let cat1 = doc_normal.add_object(dictionary! {
            "Type" => "Catalog",
        });
        doc_normal.trailer.set("Root", cat1);
        assert_eq!(
            inspect_javascript_with_budget(&doc_normal, 100, 16),
            InspectionFactState::Absent
        );

        // 2. Actual JS action -> Present
        let mut doc_js = lopdf::Document::new();
        let cat2 = doc_js.add_object(dictionary! {
            "Type" => "Catalog",
            "JS" => Object::String(b"console.log('hi')".to_vec(), lopdf::StringFormat::Literal),
        });
        doc_js.trailer.set("Root", cat2);
        assert_eq!(
            inspect_javascript_with_budget(&doc_js, 100, 16),
            InspectionFactState::Present
        );

        // 3. Unresolved reference -> Unknown
        let mut doc_unres = lopdf::Document::new();
        let cat3 = doc_unres.add_object(dictionary! {
            "Type" => "Catalog",
            "OpenAction" => Object::Reference((999, 0)),
        });
        doc_unres.trailer.set("Root", cat3);
        assert_eq!(
            inspect_javascript_with_budget(&doc_unres, 100, 16),
            InspectionFactState::Unknown
        );

        // 4. Depth budget exhaustion -> Unknown
        let mut doc_depth = lopdf::Document::new();
        let child_dict = dictionary! {
            "Type" => "Action",
            "Next" => dictionary! {
                "Type" => "Action",
            },
        };
        let cat4 = doc_depth.add_object(dictionary! {
            "Type" => "Catalog",
            "OpenAction" => child_dict,
        });
        doc_depth.trailer.set("Root", cat4);
        assert_eq!(
            inspect_javascript_with_budget(&doc_depth, 100, 0),
            InspectionFactState::Unknown
        );

        // 5. Object scan budget exhaustion -> Unknown
        let mut doc_scan = lopdf::Document::new();
        let cat5 = doc_scan.add_object(dictionary! {
            "Type" => "Catalog",
        });
        doc_scan.trailer.set("Root", cat5);
        for _ in 0..10 {
            doc_scan.add_object(dictionary! {
                "Type" => "Page",
            });
        }
        assert_eq!(
            inspect_javascript_with_budget(&doc_scan, 2, 16),
            InspectionFactState::Unknown
        );

        // 6. Cycle resistance -> does not recurse forever
        let mut doc_cycle = lopdf::Document::new();
        let obj1_id = (10, 0);
        let obj2_id = (11, 0);
        doc_cycle.objects.insert(
            obj1_id,
            Object::Dictionary(dictionary! {
                "Next" => Object::Reference(obj2_id),
            }),
        );
        doc_cycle.objects.insert(
            obj2_id,
            Object::Dictionary(dictionary! {
                "Next" => Object::Reference(obj1_id),
            }),
        );
        let cat6 = doc_cycle.add_object(dictionary! {
            "Type" => "Catalog",
            "OpenAction" => Object::Reference(obj1_id),
        });
        doc_cycle.trailer.set("Root", cat6);
        assert_eq!(
            inspect_javascript_with_budget(&doc_cycle, 100, 16),
            InspectionFactState::Absent
        );
    }

    #[test]
    fn test_signature_tri_state_regression() {
        // Case 1: Direct /Type /Sig -> Present
        let mut doc1 = lopdf::Document::new();
        doc1.add_object(dictionary! {
            "Type" => "Sig",
        });
        // Test helper on facts extraction logic using dictionary inspection
        let (s_dict, s_field, br) = inspect_document_signatures(&doc1);
        assert_eq!(s_dict, InspectionFactState::Present);
        assert_eq!(s_field, InspectionFactState::Absent);
        assert_eq!(br, InspectionFactState::Absent);

        // Case 2: Indirect reference to /Sig -> Present
        let mut doc2 = lopdf::Document::new();
        let sig_name_id = doc2.add_object(Object::Name(b"Sig".to_vec()));
        doc2.add_object(dictionary! {
            "Type" => Object::Reference(sig_name_id),
            "FT" => Object::Reference(sig_name_id),
        });
        let (s_dict2, s_field2, _) = inspect_document_signatures(&doc2);
        assert_eq!(s_dict2, InspectionFactState::Present);
        assert_eq!(s_field2, InspectionFactState::Present);

        // Case 3: Unresolved reference -> Unknown
        let mut doc3 = lopdf::Document::new();
        doc3.add_object(dictionary! {
            "Type" => Object::Reference((999, 0)),
        });
        let (s_dict3, _, _) = inspect_document_signatures(&doc3);
        assert_eq!(s_dict3, InspectionFactState::Unknown);

        // Case 4: ByteRange direct vs unresolved
        let mut doc4 = lopdf::Document::new();
        doc4.add_object(dictionary! {
            "ByteRange" => Object::Array(vec![Object::Integer(0), Object::Integer(100)]),
        });
        let (_, _, br4) = inspect_document_signatures(&doc4);
        assert_eq!(br4, InspectionFactState::Present);

        let mut doc5 = lopdf::Document::new();
        doc5.add_object(dictionary! {
            "ByteRange" => Object::Reference((999, 0)),
        });
        let (_, _, br5) = inspect_document_signatures(&doc5);
        assert_eq!(br5, InspectionFactState::Unknown);
    }

    fn inspect_document_signatures(
        lopdf_doc: &lopdf::Document,
    ) -> (
        InspectionFactState,
        InspectionFactState,
        InspectionFactState,
    ) {
        let mut sig_dict = InspectionFactState::Absent;
        let mut sig_field = InspectionFactState::Absent;
        let mut br = InspectionFactState::Absent;

        let mut sig_dict_unknown = false;
        let mut sig_field_unknown = false;
        let mut br_unknown = false;

        for object in lopdf_doc.objects.values() {
            if let Object::Dictionary(dict) = object {
                if dict.has(b"Type") {
                    if let Ok(type_obj) = dict.get(b"Type") {
                        let deref_type = match type_obj {
                            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                            _ => None,
                        };
                        let target = deref_type.unwrap_or(type_obj);
                        if let Ok(name) = target.as_name() {
                            if name == b"Sig" {
                                sig_dict = InspectionFactState::Present;
                            }
                        } else if type_obj.as_reference().is_ok() && deref_type.is_none() {
                            sig_dict_unknown = true;
                        }
                    } else {
                        sig_dict_unknown = true;
                    }
                }
                if dict.has(b"FT") {
                    if let Ok(ft_obj) = dict.get(b"FT") {
                        let deref_ft = match ft_obj {
                            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                            _ => None,
                        };
                        let target = deref_ft.unwrap_or(ft_obj);
                        if let Ok(name) = target.as_name() {
                            if name == b"Sig" {
                                sig_field = InspectionFactState::Present;
                            }
                        } else if ft_obj.as_reference().is_ok() && deref_ft.is_none() {
                            sig_field_unknown = true;
                        }
                    } else {
                        sig_field_unknown = true;
                    }
                }
                if dict.has(b"ByteRange") {
                    if let Ok(br_obj) = dict.get(b"ByteRange") {
                        let deref_br = match br_obj {
                            Object::Reference(id) => lopdf_doc.get_object(*id).ok(),
                            _ => None,
                        };
                        let target = deref_br.unwrap_or(br_obj);
                        if target.as_array().is_ok() {
                            br = InspectionFactState::Present;
                        } else if br_obj.as_reference().is_ok() && deref_br.is_none() {
                            br_unknown = true;
                        }
                    } else {
                        br_unknown = true;
                    }
                }
            }
        }

        if !sig_dict.is_present() && sig_dict_unknown {
            sig_dict = InspectionFactState::Unknown;
        }
        if !sig_field.is_present() && sig_field_unknown {
            sig_field = InspectionFactState::Unknown;
        }
        if !br.is_present() && br_unknown {
            br = InspectionFactState::Unknown;
        }

        (sig_dict, sig_field, br)
    }
}
