use crate::document_io::{
    atomic_write_pdf, ensure_unprotected, AtomicWriteReport, CommitState, DocumentIoError,
    DocumentIoFailure, DocumentIoStage,
};
use crate::PasswordCache;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RectData {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AnnotationData {
    pub id: String,
    pub annot_type: String,
    pub color: String, // hex: #RRGGBB
    pub rect: Option<RectData>,
    pub contents: Option<String>,
}

fn open_pdfium_doc<'a>(
    pdfium: &'a Pdfium,
    file_path: &str,
    password: Option<&'a str>,
) -> Result<PdfDocument<'a>, String> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err("Dosya bulunamadı".to_string());
    }

    pdfium
        .load_pdf_from_file(path, password)
        .map_err(|e| format!("Pdfium açılamadı: {:?}", e))
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        (r, g, b)
    } else {
        (0, 0, 0)
    }
}

fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

#[derive(Clone, Debug)]
struct AnnotationFingerprint {
    annotation_type: String,
    bounds: [f32; 4],
    contents: Option<String>,
}

impl AnnotationFingerprint {
    fn matches(&self, other: &Self) -> bool {
        self.annotation_type == other.annotation_type
            && self.contents == other.contents
            && self
                .bounds
                .iter()
                .zip(other.bounds)
                .all(|(left, right)| (*left - right).abs() <= 0.01)
    }
}

fn build_annotation_fingerprint<E: std::fmt::Display>(
    annotation_type: String,
    bounds: Result<PdfRect, E>,
    contents: Option<String>,
) -> Result<AnnotationFingerprint, DocumentIoFailure> {
    let bounds = bounds.map_err(|error| {
        DocumentIoFailure::invariant(format!("Annotation bounds okunamadı: {error}"))
    })?;
    Ok(AnnotationFingerprint {
        annotation_type,
        bounds: [
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        ],
        contents: contents.filter(|contents| !contents.is_empty()),
    })
}

fn annotation_fingerprint(
    annotation: &PdfPageAnnotation<'_>,
) -> Result<AnnotationFingerprint, DocumentIoFailure> {
    build_annotation_fingerprint(
        format!("{:?}", annotation.annotation_type()),
        annotation.bounds(),
        annotation.contents(),
    )
}

fn page_annotation_fingerprints(
    document: &PdfDocument<'_>,
    page_index: u16,
) -> Result<Vec<AnnotationFingerprint>, DocumentIoFailure> {
    let page = document.pages().get(page_index).map_err(|error| {
        DocumentIoFailure::invariant(format!(
            "Sayfa {page_index} annotation doğrulaması için açılamadı: {error}"
        ))
    })?;
    let annotations = page.annotations();
    (0..annotations.len())
        .map(|index| {
            annotations
                .get(index)
                .map_err(|error| {
                    DocumentIoFailure::invariant(format!("Annotation {index} okunamadı: {error}"))
                })
                .and_then(|annotation| annotation_fingerprint(&annotation))
        })
        .collect()
}

fn validate_page_index(
    document: &PdfDocument<'_>,
    page_index: u32,
) -> Result<u16, DocumentIoFailure> {
    let page_count = document.pages().len();
    if page_index >= page_count as u32 {
        return Err(DocumentIoFailure::preflight(format!(
            "Sayfa indeksi aralık dışında: {page_index} >= {page_count}"
        )));
    }
    Ok(page_count)
}

fn validate_rect(rect: &RectData) -> Result<(), DocumentIoFailure> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return Err(DocumentIoFailure::preflight(
            "Annotation rect değerleri finite olmalı; width ve height pozitif olmalı",
        ));
    }
    Ok(())
}

fn expected_annotation_type(value: &str) -> Result<&'static str, DocumentIoFailure> {
    match value {
        "Highlight" => Ok("Highlight"),
        "Underline" => Ok("Underline"),
        "Text" => Ok("Text"),
        _ => Err(DocumentIoFailure::preflight(format!(
            "Desteklenmeyen annotation tipi: {value}"
        ))),
    }
}

pub(crate) fn read_page_annotations(
    document: &PdfDocument<'_>,
    page_index: u32,
) -> Result<Vec<AnnotationData>, String> {
    let page_index = u16::try_from(page_index)
        .map_err(|_| "Sayfa indeksi PDFium sınırını aşıyor".to_string())?;
    let page = document
        .pages()
        .get(page_index)
        .map_err(|e| format!("Sayfa bulunamadı: {:?}", e))?;

    let annots = page.annotations();
    let mut result = Vec::new();

    for i in 0..annots.len() {
        if let Ok(annot) = annots.get(i) {
            let color = annot.fill_color().unwrap_or(PdfColor::new(0, 0, 0, 255));
            let bounds = annot
                .bounds()
                .unwrap_or(PdfRect::new_from_values(0.0, 0.0, 0.0, 0.0));

            // Convert bounds to RectData (pdfium uses bottom-left origin usually, but let's just pass what it gives)
            let rect = RectData {
                x: bounds.left().value,
                y: bounds.bottom().value,
                width: bounds.width().value,
                height: bounds.height().value,
            };

            let annot_type = format!("{:?}", annot.annotation_type());
            let contents = annot.contents();

            result.push(AnnotationData {
                id: i.to_string(),
                annot_type,
                color: rgb_to_hex(color.red(), color.green(), color.blue()),
                rect: Some(rect),
                contents,
            });
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn add_annotation(
    file_path: String,
    page_index: u32,
    annot_type: String,
    rect: RectData,
    color: String,
    contents: Option<String>,
    cache: State<'_, PasswordCache>,
) -> Result<String, String> {
    let password = {
        let lock = cache.0.lock();
        lock.get(&file_path).cloned()
    };
    add_annotation_with_password(
        file_path, page_index, annot_type, rect, color, contents, password,
    )
}

fn add_annotation_with_password(
    file_path: String,
    page_index: u32,
    annot_type: String,
    rect: RectData,
    color: String,
    contents: Option<String>,
    password: Option<String>,
) -> Result<String, String> {
    add_annotation_report(
        file_path, page_index, annot_type, rect, color, contents, password,
    )
    .map(|_| "ok".to_string())
    .map_err(|error| error.to_string())
}

pub(crate) fn add_annotation_report(
    file_path: String,
    page_index: u32,
    annot_type: String,
    rect: RectData,
    color: String,
    contents: Option<String>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    let expected_type = expected_annotation_type(&annot_type)
        .map_err(|error| {
            DocumentIoError::new(
                "add_annotation",
                error.stage,
                CommitState::Prepared,
                error.detail,
            )
        })?
        .to_string();
    validate_rect(&rect).map_err(|error| {
        DocumentIoError::new(
            "add_annotation",
            error.stage,
            CommitState::Prepared,
            error.detail,
        )
    })?;

    atomic_write_pdf(
        "add_annotation",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let mut document = open_pdfium_doc(
                pdfium,
                source.to_string_lossy().as_ref(),
                password.as_deref(),
            )
            .map_err(DocumentIoFailure::preflight)?;
            ensure_unprotected(&document)?;
            let page_count = validate_page_index(&document, page_index)?;
            let before = page_annotation_fingerprints(&document, page_index as u16)?;
            let (red, green, blue) = hex_to_rgb(&color);
            let pdf_color = PdfColor::new(red, green, blue, 255);
            let bounds =
                PdfRect::new_from_values(rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
            {
                let mut page = document
                    .pages_mut()
                    .get(page_index as u16)
                    .map_err(|error| {
                        DocumentIoFailure::preflight(format!("Sayfa bulunamadı: {error}"))
                    })?;
                let annotations = page.annotations_mut();
                match annot_type.as_str() {
                    "Highlight" => {
                        let mut annotation =
                            annotations.create_highlight_annotation().map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Highlight annotation oluşturulamadı: {error}"
                                ))
                            })?;
                        annotation.set_fill_color(pdf_color).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation rengi ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation.set_bounds(bounds).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation bounds ayarlanamadı: {error}"
                            ))
                        })?;
                        if let Some(value) = contents.as_deref() {
                            annotation.set_contents(value).map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation içeriği ayarlanamadı: {error}"
                                ))
                            })?;
                        }
                    }
                    "Underline" => {
                        let mut annotation =
                            annotations.create_underline_annotation().map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Underline annotation oluşturulamadı: {error}"
                                ))
                            })?;
                        annotation.set_fill_color(pdf_color).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation rengi ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation.set_bounds(bounds).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation bounds ayarlanamadı: {error}"
                            ))
                        })?;
                        if let Some(value) = contents.as_deref() {
                            annotation.set_contents(value).map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation içeriği ayarlanamadı: {error}"
                                ))
                            })?;
                        }
                    }
                    "Text" => {
                        let mut annotation =
                            annotations.create_text_annotation("").map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Text annotation oluşturulamadı: {error}"
                                ))
                            })?;
                        annotation.set_fill_color(pdf_color).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation rengi ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation.set_bounds(bounds).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation bounds ayarlanamadı: {error}"
                            ))
                        })?;
                        if let Some(value) = contents.as_deref() {
                            annotation.set_contents(value).map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation içeriği ayarlanamadı: {error}"
                                ))
                            })?;
                        }
                    }
                    _ => {
                        return Err(DocumentIoFailure::preflight(format!(
                            "Desteklenmeyen annotation tipi: {annot_type}"
                        )))
                    }
                }
            }
            document.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            let expected = AnnotationFingerprint {
                annotation_type: expected_type.clone(),
                bounds: [
                    bounds.left().value,
                    bounds.bottom().value,
                    bounds.right().value,
                    bounds.top().value,
                ],
                contents: contents.clone().filter(|contents| !contents.is_empty()),
            };
            Ok((page_count, before.len(), expected))
        },
        move |document, (page_count, previous_count, expected)| {
            if document.pages().len() != *page_count {
                return Err(DocumentIoFailure::invariant(
                    "Annotation ekleme sayfa sayısını değiştirdi",
                ));
            }
            let after = page_annotation_fingerprints(document, page_index as u16)?;
            if after.len() != previous_count + 1 {
                return Err(DocumentIoFailure::invariant(format!(
                    "Annotation sayısı +1 olmadı: önce {previous_count}, sonra {}",
                    after.len()
                )));
            }
            if !after
                .last()
                .is_some_and(|fingerprint| fingerprint.matches(expected))
            {
                return Err(DocumentIoFailure::invariant(
                    "Eklenen annotation type/bounds/contents ile doğrulanamadı",
                ));
            }
            Ok(())
        },
    )
}

#[tauri::command]
pub fn delete_annotation(
    file_path: String,
    page_index: u32,
    annot_id: String,
    cache: State<'_, PasswordCache>,
) -> Result<(), String> {
    let password = {
        let lock = cache.0.lock();
        lock.get(&file_path).cloned()
    };
    delete_annotation_with_password(file_path, page_index, annot_id, password)
}

fn delete_annotation_with_password(
    file_path: String,
    page_index: u32,
    annot_id: String,
    password: Option<String>,
) -> Result<(), String> {
    delete_annotation_report(file_path, page_index, annot_id, password)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn delete_annotation_report(
    file_path: String,
    page_index: u32,
    annot_id: String,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let annotation_index = annot_id.parse::<usize>().map_err(|_| {
        DocumentIoError::new(
            "delete_annotation",
            DocumentIoStage::Preflight,
            CommitState::Prepared,
            "Annotation ID geçerli bir indeks değil",
        )
    })?;
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        "delete_annotation",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let mut document = open_pdfium_doc(
                pdfium,
                source.to_string_lossy().as_ref(),
                password.as_deref(),
            )
            .map_err(DocumentIoFailure::preflight)?;
            ensure_unprotected(&document)?;
            let page_count = validate_page_index(&document, page_index)?;
            let before = page_annotation_fingerprints(&document, page_index as u16)?;
            let target_fingerprint = before.get(annotation_index).cloned().ok_or_else(|| {
                DocumentIoFailure::preflight(format!(
                    "Annotation indeksi aralık dışında: {annotation_index} >= {}",
                    before.len()
                ))
            })?;
            let matching_before = before
                .iter()
                .filter(|item| item.matches(&target_fingerprint))
                .count();
            {
                let mut page = document
                    .pages_mut()
                    .get(page_index as u16)
                    .map_err(|error| {
                        DocumentIoFailure::preflight(format!("Sayfa bulunamadı: {error}"))
                    })?;
                let annotations = page.annotations_mut();
                let annotation = annotations.get(annotation_index).map_err(|error| {
                    DocumentIoFailure::preflight(format!("Annotation bulunamadı: {error}"))
                })?;
                annotations.delete_annotation(annotation).map_err(|error| {
                    DocumentIoFailure::write(format!("Annotation silinemedi: {error}"))
                })?;
            }
            document.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            Ok((
                page_count,
                before.len(),
                target_fingerprint,
                matching_before,
            ))
        },
        move |document, (page_count, previous_count, deleted, matching_before)| {
            if document.pages().len() != *page_count {
                return Err(DocumentIoFailure::invariant(
                    "Annotation silme sayfa sayısını değiştirdi",
                ));
            }
            let after = page_annotation_fingerprints(document, page_index as u16)?;
            if after.len() + 1 != *previous_count {
                return Err(DocumentIoFailure::invariant(format!(
                    "Annotation sayısı -1 olmadı: önce {previous_count}, sonra {}",
                    after.len()
                )));
            }
            let matching_after = after.iter().filter(|item| item.matches(deleted)).count();
            if matching_after + 1 != *matching_before {
                return Err(DocumentIoFailure::invariant(
                    "Silinen annotation fingerprint'i çıktıdan kaybolmadı",
                ));
            }
            Ok(())
        },
    )
}

#[tauri::command]
pub fn save_pdf_with_annotations(
    file_path: String,
    output_path: String,
    cache: State<'_, PasswordCache>,
) -> Result<(), String> {
    let password = {
        let lock = cache.0.lock();
        lock.get(&file_path).cloned()
    };
    save_pdf_with_annotations_with_password(file_path, output_path, password)
}

fn save_pdf_with_annotations_with_password(
    file_path: String,
    output_path: String,
    password: Option<String>,
) -> Result<(), String> {
    let source = PathBuf::from(&file_path);
    let source_for_write = source.clone();
    let target = PathBuf::from(&output_path);
    atomic_write_pdf(
        "save_pdf_with_annotations",
        std::slice::from_ref(&source),
        &target,
        move |pdfium, temp| {
            let document = open_pdfium_doc(
                pdfium,
                source_for_write.to_string_lossy().as_ref(),
                password.as_deref(),
            )
            .map_err(DocumentIoFailure::preflight)?;
            ensure_unprotected(&document)?;
            let page_count = document.pages().len();
            let mut annotations = Vec::with_capacity(page_count as usize);
            for page_index in 0..page_count {
                annotations.push(page_annotation_fingerprints(&document, page_index)?);
            }
            document.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            Ok(annotations)
        },
        |document, expected| {
            if document.pages().len() as usize != expected.len() {
                return Err(DocumentIoFailure::invariant(format!(
                    "Annotation save sayfa sayısını değiştirdi: beklenen {}, bulunan {}",
                    expected.len(),
                    document.pages().len()
                )));
            }
            for (page_index, expected_page) in expected.iter().enumerate() {
                let actual = page_annotation_fingerprints(document, page_index as u16)?;
                if actual.len() != expected_page.len()
                    || !actual
                        .iter()
                        .zip(expected_page)
                        .all(|(left, right)| left.matches(right))
                {
                    return Err(DocumentIoFailure::invariant(format!(
                        "Sayfa {page_index} annotation invariant'ı bozuldu"
                    )));
                }
            }
            Ok(())
        },
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pdf")
            .join("annotations.pdf")
    }

    fn annotation_fingerprints(path: &Path) -> Vec<AnnotationFingerprint> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            page_annotation_fingerprints(&document, 0).map_err(|error| error.detail)
        })
        .unwrap()
    }

    #[test]
    fn document_io_annotation_add_delete_and_same_path_save_are_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        assert_eq!(annotation_fingerprints(&path).len(), 1);

        add_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "Highlight".to_string(),
            RectData {
                x: 100.0,
                y: 100.0,
                width: 80.0,
                height: 20.0,
            },
            "#FFCC00".to_string(),
            Some("Atomic highlight".to_string()),
            None,
        )
        .unwrap();
        let after_add = annotation_fingerprints(&path);
        assert_eq!(after_add.len(), 2);
        assert_eq!(
            after_add.last().and_then(|item| item.contents.as_deref()),
            Some("Atomic highlight")
        );

        save_pdf_with_annotations_with_password(
            path.to_string_lossy().to_string(),
            path.to_string_lossy().to_string(),
            None,
        )
        .unwrap();
        assert_eq!(annotation_fingerprints(&path).len(), 2);

        delete_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "0".to_string(),
            None,
        )
        .unwrap();
        let after_delete = annotation_fingerprints(&path);
        assert_eq!(after_delete.len(), 1);
        assert!(!after_delete
            .iter()
            .any(|item| item.contents.as_deref() == Some("Fixture note")));
    }

    #[test]
    fn document_io_invalid_annotation_type_preserves_original_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        let original = fs::read(&path).unwrap();

        let error = add_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "Unknown".to_string(),
            RectData {
                x: 1.0,
                y: 1.0,
                width: 10.0,
                height: 10.0,
            },
            "#FFFFFF".to_string(),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("Desteklenmeyen annotation"));
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn document_io_invalid_annotation_id_preserves_original_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        let original = fs::read(&path).unwrap();

        let error = delete_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "99".to_string(),
            None,
        )
        .unwrap_err();

        assert!(error.contains("aralık dışında"), "{error}");
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn annotation_bounds_read_error_is_not_silently_defaulted() {
        let error =
            build_annotation_fingerprint::<&str>("Text".to_string(), Err("simulated"), None)
                .unwrap_err();

        assert_eq!(
            error.stage,
            crate::document_io::DocumentIoStage::ValidateInvariant
        );
        assert!(error.detail.contains("Annotation bounds okunamadı"));
    }
}
