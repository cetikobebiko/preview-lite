use lopdf::{Document, Object};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TagNode {
    pub id: String,
    pub tag_type: String,
    pub alt_text: Option<String>,
    pub language: Option<String>,
    pub children: Vec<TagNode>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccessibilityReport {
    pub is_tagged: bool,
    pub has_document_title: bool,
    pub issues: Vec<String>,
    pub score: u8,
}

fn parse_struct_elem(doc: &Document, obj: &Object) -> Option<TagNode> {
    if let Ok(dict) = obj.as_dict() {
        // Only parse if it's a StructElem
        if let Ok(type_name) = dict.get(b"Type").and_then(|t| t.as_name()) {
            if type_name != b"StructElem" {
                return None;
            }
        }

        let tag_type = dict
            .get(b"S")
            .and_then(|s| s.as_name())
            .map(|n| String::from_utf8_lossy(n).to_string())
            .unwrap_or_else(|_| "Unknown".to_string());

        let alt_text = dict
            .get(b"Alt")
            .and_then(|a| a.as_str())
            .map(|a| String::from_utf8_lossy(a).to_string())
            .or_else(|_| {
                // Also check for string formats
                dict.get(b"Alt")
                    .and_then(|a| a.as_string())
                    .map(|a| a.to_string())
            })
            .ok();

        let language = dict
            .get(b"Lang")
            .and_then(|l| l.as_str())
            .map(|l| String::from_utf8_lossy(l).to_string())
            .or_else(|_| {
                dict.get(b"Lang")
                    .and_then(|l| l.as_string())
                    .map(|l| l.to_string())
            })
            .ok();

        let mut children = Vec::new();
        if let Ok(kids) = dict.get(b"K") {
            match kids {
                Object::Array(arr) => {
                    for kid in arr {
                        if let Object::Reference(id) = kid {
                            if let Ok(kid_obj) = doc.get_object(*id) {
                                if let Some(mut child_node) = parse_struct_elem(doc, kid_obj) {
                                    child_node.id = format!("{}_{}", id.0, id.1);
                                    children.push(child_node);
                                }
                            }
                        }
                    }
                }
                Object::Reference(id) => {
                    if let Ok(kid_obj) = doc.get_object(*id) {
                        if let Some(mut child_node) = parse_struct_elem(doc, kid_obj) {
                            child_node.id = format!("{}_{}", id.0, id.1);
                            children.push(child_node);
                        }
                    }
                }
                Object::Dictionary(_) => {
                    if let Some(child_node) = parse_struct_elem(doc, kids) {
                        children.push(child_node);
                    }
                }
                _ => {}
            }
        }

        return Some(TagNode {
            id: "inline_dict".to_string(), // This will be overwritten if it was a reference
            tag_type,
            alt_text,
            language,
            children,
        });
    }
    None
}

pub(crate) fn get_structure_tree_from_document(doc: &Document) -> Result<TagNode, String> {
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .and_then(|o| o.as_reference())
        .map_err(|_| "Root kataloğu bulunamadı".to_string())?;

    let catalog = doc
        .get_dictionary(catalog_id)
        .map_err(|_| "Katalog sözlüğü okunamadı".to_string())?;

    let struct_tree_root_id = catalog
        .get(b"StructTreeRoot")
        .and_then(|o| o.as_reference())
        .map_err(|_| "Bu belgede Yapı Ağacı (Tagged PDF) bulunmuyor".to_string())?;

    let struct_tree_root = doc
        .get_dictionary(struct_tree_root_id)
        .map_err(|_| "Yapı Ağacı sözlüğü okunamadı".to_string())?;

    let mut root_node = TagNode {
        id: format!("{}_{}", struct_tree_root_id.0, struct_tree_root_id.1),
        tag_type: "DocumentRoot".to_string(),
        alt_text: None,
        language: None,
        children: Vec::new(),
    };

    if let Ok(kids) = struct_tree_root.get(b"K") {
        match kids {
            Object::Array(arr) => {
                for kid in arr {
                    if let Object::Reference(id) = kid {
                        if let Ok(kid_obj) = doc.get_object(*id) {
                            if let Some(mut child_node) = parse_struct_elem(doc, kid_obj) {
                                child_node.id = format!("{}_{}", id.0, id.1);
                                root_node.children.push(child_node);
                            }
                        }
                    }
                }
            }
            Object::Reference(id) => {
                if let Ok(kid_obj) = doc.get_object(*id) {
                    if let Some(mut child_node) = parse_struct_elem(doc, kid_obj) {
                        child_node.id = format!("{}_{}", id.0, id.1);
                        root_node.children.push(child_node);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(root_node)
}

#[tauri::command]
pub fn update_alt_text(
    file_path: String,
    object_id_str: String,
    alt_text: Option<String>,
    language: Option<String>,
    target_path: String,
) -> Result<(), String> {
    let mut doc = Document::load(&file_path).map_err(|e| format!("Belge açılamadı: {}", e))?;

    let parts: Vec<&str> = object_id_str.split('_').collect();
    if parts.len() != 2 {
        return Err("Geçersiz Object ID formatı".to_string());
    }

    let obj_num: u32 = parts[0].parse().unwrap_or(0);
    let gen_num: u16 = parts[1].parse().unwrap_or(0);

    if let Ok(Object::Dictionary(ref mut dict)) = doc.get_object_mut((obj_num, gen_num)) {
        if let Some(alt) = alt_text {
            if alt.is_empty() {
                dict.remove(b"Alt");
            } else {
                dict.set(
                    "Alt",
                    Object::String(alt.into_bytes(), lopdf::StringFormat::Literal),
                );
            }
        }

        if let Some(lang) = language {
            if lang.is_empty() {
                dict.remove(b"Lang");
            } else {
                dict.set(
                    "Lang",
                    Object::String(lang.into_bytes(), lopdf::StringFormat::Literal),
                );
            }
        }
    } else {
        return Err("Etiket nesnesi bulunamadı veya değiştirilemiyor".to_string());
    }

    doc.save(&target_path)
        .map_err(|e| format!("Belge kaydedilemedi: {}", e))?;
    Ok(())
}

pub(crate) fn validate_pdf_ua_document(doc: &Document) -> AccessibilityReport {
    let mut report = AccessibilityReport {
        is_tagged: false,
        has_document_title: false,
        issues: Vec::new(),
        score: 100,
    };

    // 1. Check if tagged
    if let Ok(catalog_id) = doc.trailer.get(b"Root").and_then(|o| o.as_reference()) {
        if let Ok(catalog) = doc.get_dictionary(catalog_id) {
            // Check MarkInfo / Marked true
            if let Ok(Object::Dictionary(mark_info)) = catalog.get(b"MarkInfo") {
                if let Ok(marked) = mark_info.get(b"Marked").and_then(|o| o.as_bool()) {
                    report.is_tagged = marked;
                }
            }

            // Or check if StructTreeRoot exists
            if catalog.has(b"StructTreeRoot") {
                report.is_tagged = true;
            }

            // Check ViewerPreferences / DisplayDocTitle true
            if let Ok(Object::Dictionary(viewer_prefs)) = catalog.get(b"ViewerPreferences") {
                if let Ok(display_doc_title) = viewer_prefs
                    .get(b"DisplayDocTitle")
                    .and_then(|o| o.as_bool())
                {
                    report.has_document_title = display_doc_title;
                }
            }
        }
    }

    if !report.is_tagged {
        report.issues.push(
            "PDF etiketlenmemiş (Untagged PDF). Ekran okuyucular yapıyı anlayamaz.".to_string(),
        );
        report.score = report.score.saturating_sub(40);
    }

    if !report.has_document_title {
        report
            .issues
            .push("Belge başlığı (Document Title) görüntülenmek üzere ayarlanmamış.".to_string());
        report.score = report.score.saturating_sub(10);
    }

    let mut figures_without_alt = 0;

    for object in doc.objects.values() {
        if let Object::Dictionary(dict) = object {
            if let Ok(type_name) = dict.get(b"Type").and_then(|t| t.as_name()) {
                if type_name == b"StructElem" {
                    if let Ok(s) = dict.get(b"S").and_then(|s| s.as_name()) {
                        if s == b"Figure" {
                            let has_alt = dict.has(b"Alt");
                            if !has_alt {
                                figures_without_alt += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if figures_without_alt > 0 {
        report.issues.push(format!(
            "{} adet görselde alternatif metin (Alt-text) eksik.",
            figures_without_alt
        ));
        report.score = report.score.saturating_sub(10 * figures_without_alt as u8);
    }

    if report.issues.is_empty() {
        report
            .issues
            .push("Belge temel PDF/UA standartlarına uygun görünüyor.".to_string());
    }

    report
}

#[tauri::command]
pub fn update_reading_order(
    file_path: String,
    parent_id_str: String,
    ordered_child_ids: Vec<String>,
    target_path: String,
) -> Result<(), String> {
    let mut doc = Document::load(&file_path).map_err(|e| format!("Belge açılamadı: {}", e))?;

    let parts: Vec<&str> = parent_id_str.split('_').collect();
    if parts.len() != 2 {
        return Err("Geçersiz Parent ID formatı".to_string());
    }
    let parent_obj_num: u32 = parts[0].parse().unwrap_or(0);
    let parent_gen_num: u16 = parts[1].parse().unwrap_or(0);

    let mut new_k_array = Vec::new();
    for child_id_str in ordered_child_ids {
        let child_parts: Vec<&str> = child_id_str.split('_').collect();
        if child_parts.len() == 2 {
            let child_obj_num: u32 = child_parts[0].parse().unwrap_or(0);
            let child_gen_num: u16 = child_parts[1].parse().unwrap_or(0);
            new_k_array.push(Object::Reference((child_obj_num, child_gen_num)));
        }
    }

    if let Ok(Object::Dictionary(ref mut dict)) =
        doc.get_object_mut((parent_obj_num, parent_gen_num))
    {
        if !new_k_array.is_empty() {
            if new_k_array.len() == 1 {
                dict.set("K", new_k_array.remove(0));
            } else {
                dict.set("K", Object::Array(new_k_array));
            }
        }
    } else {
        return Err("Üst etiket (Parent) nesnesi bulunamadı".to_string());
    }

    doc.save(&target_path)
        .map_err(|e| format!("Belge kaydedilemedi: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_document() -> Document {
        Document::load_mem(include_bytes!("../tests/fixtures/pdf/minimal.pdf"))
            .expect("minimal PDF fixture")
    }

    #[test]
    fn structure_tree_read_uses_an_owned_document_without_path_io() {
        let error = get_structure_tree_from_document(&minimal_document()).unwrap_err();
        assert!(error.contains("Yapı Ağacı"));
    }

    #[test]
    fn pdf_ua_validation_uses_an_owned_document_without_path_io() {
        let report = validate_pdf_ua_document(&minimal_document());
        assert!(!report.is_tagged);
        assert!(!report.has_document_title);
        assert_eq!(report.score, 50);
        assert_eq!(report.issues.len(), 2);
    }
}
