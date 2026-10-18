use crate::document_io::{
    atomic_write_pdf, ensure_unprotected, AtomicWriteReport, CommitState, DocumentIoError,
    DocumentIoFailure, DocumentIoStage,
};
use crate::incremental_save::{save_mutation, RawDocument, RawPage};
use pdfium_render::prelude::*;
use std::collections::HashSet;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};

const MAX_PAGE_DIMENSION_POINTS: f32 = 14_400.0;

fn degrees_from_rotation(rotation: PdfPageRenderRotation) -> i32 {
    match rotation {
        PdfPageRenderRotation::Degrees90 => 90,
        PdfPageRenderRotation::Degrees180 => 180,
        PdfPageRenderRotation::Degrees270 => 270,
        PdfPageRenderRotation::None => 0,
    }
}

fn rotation_from_degrees(degrees: i32) -> Result<PdfPageRenderRotation, String> {
    if degrees % 90 != 0 {
        return Err(format!(
            "DOCUMENT_IO_ERROR operation=rotate_page stage=preflight state=prepared: \
             Rotasyon 90 derecenin katı olmalı: {degrees}"
        ));
    }
    Ok(match degrees.rem_euclid(360) {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    })
}

fn save_logged(
    operation: &str,
    document: &RawDocument<'_>,
    source: &Path,
    temp: &Path,
) -> Result<(), DocumentIoFailure> {
    let mode = save_mutation(document, source, temp)?;
    tracing::info!(operation, write_mode = ?mode, "Sayfa mutasyonu kaydedildi");
    Ok(())
}

#[derive(Clone, Debug)]
struct PageSnapshot {
    width: f32,
    height: f32,
    rotation: String,
    text: String,
    annotations: Vec<(String, [f32; 4], Option<String>)>,
}

fn page_snapshots(document: &PdfDocument<'_>) -> Result<Vec<PageSnapshot>, DocumentIoFailure> {
    document
        .pages()
        .as_range()
        .map(|index| {
            let page = document.pages().get(index).map_err(|error| {
                DocumentIoFailure::preflight(format!("Sayfa {index} okunamadı: {error}"))
            })?;
            let rotation = page.rotation().map_err(|error| {
                DocumentIoFailure::preflight(format!("Sayfa {index} rotasyonu okunamadı: {error}"))
            })?;
            let text = page.text().map_err(|error| {
                DocumentIoFailure::preflight(format!("Sayfa {index} metni okunamadı: {error}"))
            })?;
            let annotations = page.annotations();
            let mut annotation_snapshots = Vec::with_capacity(annotations.len());
            for annotation_index in 0..annotations.len() {
                let annotation = annotations.get(annotation_index).map_err(|error| {
                    DocumentIoFailure::preflight(format!(
                        "Sayfa {index} annotation {annotation_index} okunamadı: {error}"
                    ))
                })?;
                let bounds = annotation.bounds().map_err(|error| {
                    DocumentIoFailure::preflight(format!(
                        "Sayfa {index} annotation {annotation_index} bounds okunamadı: {error}"
                    ))
                })?;
                annotation_snapshots.push((
                    format!("{:?}", annotation.annotation_type()),
                    [
                        bounds.left().value,
                        bounds.bottom().value,
                        bounds.right().value,
                        bounds.top().value,
                    ],
                    annotation
                        .contents()
                        .filter(|contents| !contents.is_empty()),
                ));
            }
            Ok(PageSnapshot {
                width: page.width().value,
                height: page.height().value,
                rotation: format!("{rotation:?}"),
                text: text.all(),
                annotations: annotation_snapshots,
            })
        })
        .collect()
}

fn validate_page_snapshots(
    document: &PdfDocument<'_>,
    expected: &[PageSnapshot],
) -> Result<(), DocumentIoFailure> {
    let actual =
        page_snapshots(document).map_err(|error| DocumentIoFailure::invariant(error.detail))?;
    if actual.len() != expected.len() {
        return Err(DocumentIoFailure::invariant(format!(
            "Reorder sayfa sayısı invariant'ı bozuldu: beklenen {}, bulunan {}",
            expected.len(),
            actual.len()
        )));
    }
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let dimensions_match = (actual.width - expected.width).abs() <= 0.01
            && (actual.height - expected.height).abs() <= 0.01;
        let annotations_match = actual.annotations.len() == expected.annotations.len()
            && actual
                .annotations
                .iter()
                .zip(&expected.annotations)
                .all(|(left, right)| {
                    left.0 == right.0
                        && left.2 == right.2
                        && left
                            .1
                            .iter()
                            .zip(right.1)
                            .all(|(left, right)| (*left - right).abs() <= 0.01)
                });
        if !dimensions_match
            || actual.rotation != expected.rotation
            || actual.text != expected.text
            || !annotations_match
        {
            return Err(DocumentIoFailure::invariant(format!(
                "Reorder sayfa {index} içerik/rotasyon/annotation invariant'ı bozuldu"
            )));
        }
    }
    Ok(())
}

fn reject_unsupported_reorder_features(
    pdfium: &Pdfium,
    document: &PdfDocument<'_>,
    source: &std::path::Path,
    password: Option<&str>,
) -> Result<(), DocumentIoFailure> {
    let unsupported = if !document.metadata().is_empty() {
        Some("metadata")
    } else if document.bookmarks().root().is_some() {
        Some("bookmark/outline")
    } else if document.form().is_some() {
        Some("AcroForm")
    } else if !document.attachments().is_empty() {
        Some("attachment")
    } else if !document.signatures().is_empty() {
        Some("digital signature")
    } else {
        None
    };
    if let Some(feature) = unsupported {
        return Err(DocumentIoFailure::preflight(format!(
            "Reorder bu belge özelliğini güvenli biçimde koruyamıyor: {feature}"
        )));
    }

    let source = source.to_str().ok_or_else(|| {
        DocumentIoFailure::preflight("Named-destination kontrolü için kaynak yolu UTF-8 olmalı")
    })?;
    let raw = pdfium.bindings().FPDF_LoadDocument(source, password);
    if raw.is_null() {
        return Err(DocumentIoFailure::preflight(
            "Named-destination kontrolü için kaynak PDF açılamadı",
        ));
    }
    let named_destination_count = pdfium.bindings().FPDF_CountNamedDests(raw);
    pdfium.bindings().FPDF_CloseDocument(raw);
    if named_destination_count > 0 {
        return Err(DocumentIoFailure::preflight(
            "Reorder named destination içeren belgeleri güvenli biçimde koruyamıyor",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum ReorderMode {
    FullPermutation,
    OrderedSubset,
}

fn validate_reorder(
    order: &[u32],
    page_count: u16,
    mode: ReorderMode,
) -> Result<(), DocumentIoFailure> {
    if order.is_empty() {
        return Err(DocumentIoFailure::preflight("Sayfa sırası boş bırakılamaz"));
    }
    if matches!(mode, ReorderMode::FullPermutation) && order.len() != page_count as usize {
        return Err(DocumentIoFailure::preflight(format!(
            "Yerinde reorder tam permütasyon olmalı: beklenen {page_count}, verilen {}",
            order.len()
        )));
    }
    let mut seen = HashSet::with_capacity(order.len());
    for &index in order {
        if index >= page_count as u32 {
            return Err(DocumentIoFailure::preflight(format!(
                "Sayfa indeksi aralık dışında: {index} >= {page_count}"
            )));
        }
        if !seen.insert(index) {
            return Err(DocumentIoFailure::preflight(format!(
                "Tekrarlanan sayfa indeksi: {index}"
            )));
        }
    }
    if matches!(mode, ReorderMode::FullPermutation) && seen.len() != page_count as usize {
        return Err(DocumentIoFailure::preflight(
            "Yerinde reorder bütün sayfaları tam bir kez içermeli",
        ));
    }
    Ok(())
}

pub fn reorder_pdf_to_target(
    source_path: String,
    page_order: Vec<u32>,
    target_path: String,
    password: Option<String>,
    mode: ReorderMode,
) -> Result<(), String> {
    reorder_pdf_to_target_report(source_path, page_order, target_path, password, mode)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn reorder_pdf_to_target_report(
    source_path: String,
    page_order: Vec<u32>,
    target_path: String,
    password: Option<String>,
    mode: ReorderMode,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let source = PathBuf::from(&source_path);
    let target = PathBuf::from(&target_path);
    let source_for_write = source.clone();
    atomic_write_pdf(
        match mode {
            ReorderMode::FullPermutation => "reorder_pages",
            ReorderMode::OrderedSubset => "reorder_pdf_pages",
        },
        std::slice::from_ref(&source),
        &target,
        move |pdfium, temp| {
            let source_document = pdfium
                .load_pdf_from_file(&source_for_write, password.as_deref())
                .map_err(|error| {
                    DocumentIoFailure::preflight(format!("Kaynak PDF açılamadı: {error}"))
                })?;
            ensure_unprotected(&source_document)?;
            reject_unsupported_reorder_features(
                pdfium,
                &source_document,
                &source_for_write,
                password.as_deref(),
            )?;
            let source_count = source_document.pages().len();
            validate_reorder(&page_order, source_count, mode)?;
            let source_pages = page_snapshots(&source_document)?;
            let expected_pages = page_order
                .iter()
                .map(|index| source_pages[*index as usize].clone())
                .collect::<Vec<_>>();

            let mut output = pdfium.create_new_pdf().map_err(|error| {
                DocumentIoFailure::write(format!("Yeni PDF oluşturulamadı: {error}"))
            })?;
            for &index in &page_order {
                let destination = output.pages().len();
                output
                    .pages_mut()
                    .copy_page_from_document(&source_document, index as u16, destination)
                    .map_err(|error| {
                        DocumentIoFailure::write(format!("Sayfa {index} kopyalanamadı: {error}"))
                    })?;
            }
            output.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            Ok(expected_pages)
        },
        |document, expected_pages| validate_page_snapshots(document, expected_pages),
    )
}

/// Rotates a page clockwise by `degrees` relative to its current /Rotate value.
pub(crate) fn rotate_page_report(
    file_path: String,
    page_index: u32,
    degrees: i32,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    rotation_from_degrees(degrees).map_err(|detail| {
        DocumentIoError::new(
            "rotate_page",
            DocumentIoStage::Preflight,
            CommitState::Prepared,
            detail,
        )
    })?;
    let path = PathBuf::from(&file_path);
    let source = path.clone();

    atomic_write_pdf(
        "rotate_page",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let bindings = pdfium.bindings();
            let document = RawDocument::open_for_mutation(bindings, &source, password.as_deref())?;
            let page_count = document.page_count()?;
            if page_index >= u32::from(page_count) {
                return Err(DocumentIoFailure::preflight(format!(
                    "Sayfa indeksi aralık dışında: {page_index} >= {page_count}"
                )));
            }
            let rotation = {
                let page = RawPage::load(&document, page_index)?;
                // FPDFPage_{Get,Set}Rotation çeyrek dönüş sayısıdır (0–3).
                let rotation = rotation_from_degrees(
                    bindings.FPDFPage_GetRotation(page.handle) * 90 + degrees,
                )
                .map_err(DocumentIoFailure::preflight)?;
                bindings.FPDFPage_SetRotation(page.handle, degrees_from_rotation(rotation) / 90);
                rotation
            };
            save_logged("rotate_page", &document, &source, temp)?;
            Ok((page_count, format!("{rotation:?}")))
        },
        move |document, (expected_count, expected_rotation)| {
            if document.pages().len() != *expected_count {
                return Err(DocumentIoFailure::invariant(format!(
                    "Rotate sayfa sayısını değiştirdi: beklenen {expected_count}, bulunan {}",
                    document.pages().len()
                )));
            }
            let page = document.pages().get(page_index as u16).map_err(|error| {
                DocumentIoFailure::invariant(format!("Döndürülen sayfa doğrulanamadı: {error}"))
            })?;
            let actual = page.rotation().map_err(|error| {
                DocumentIoFailure::invariant(format!("Sayfa rotasyonu okunamadı: {error}"))
            })?;
            if format!("{actual:?}") != *expected_rotation {
                return Err(DocumentIoFailure::invariant(format!(
                    "Rotasyon invariant'ı bozuldu: beklenen {expected_rotation}, bulunan {actual:?}"
                )));
            }
            Ok(())
        },
    )
}

pub(crate) fn delete_pages_report(
    file_path: String,
    page_indices: Vec<u32>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        "delete_pages",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let bindings = pdfium.bindings();
            let document = RawDocument::open_for_mutation(bindings, &source, password.as_deref())?;
            let page_count = document.page_count()?;
            if page_indices.is_empty() {
                return Err(DocumentIoFailure::preflight("Silinecek sayfa listesi boş"));
            }
            let mut unique = HashSet::with_capacity(page_indices.len());
            for &index in &page_indices {
                if index >= page_count as u32 {
                    return Err(DocumentIoFailure::preflight(format!(
                        "Sayfa indeksi aralık dışında: {index} >= {page_count}"
                    )));
                }
                if !unique.insert(index) {
                    return Err(DocumentIoFailure::preflight(format!(
                        "Tekrarlanan sayfa indeksi: {index}"
                    )));
                }
            }
            if unique.len() >= page_count as usize {
                return Err(DocumentIoFailure::preflight(
                    "Belgedeki bütün sayfalar silinemez",
                ));
            }
            let mut sorted = unique.into_iter().collect::<Vec<_>>();
            sorted.sort_unstable_by(|left, right| right.cmp(left));
            for index in sorted {
                bindings.FPDFPage_Delete(document.handle, index as c_int);
            }
            let expected_count = page_count - page_indices.len() as u16;
            if document.page_count()? != expected_count {
                return Err(DocumentIoFailure::write(format!(
                    "Sayfalar silinemedi: beklenen {expected_count} sayfa"
                )));
            }
            save_logged("delete_pages", &document, &source, temp)?;
            Ok(expected_count)
        },
        |document, expected_count| {
            if document.pages().len() != *expected_count {
                return Err(DocumentIoFailure::invariant(format!(
                    "Delete sayfa sayısı invariant'ı bozuldu: beklenen {expected_count}, bulunan {}",
                    document.pages().len()
                )));
            }
            Ok(())
        },
    )
}

pub(crate) fn insert_blank_page_report(
    file_path: String,
    at_index: u32,
    width: f32,
    height: f32,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > MAX_PAGE_DIMENSION_POINTS
        || height > MAX_PAGE_DIMENSION_POINTS
    {
        return Err(DocumentIoError::new(
            "insert_blank_page",
            DocumentIoStage::Preflight,
            CommitState::Prepared,
            format!(
                "Sayfa boyutları PDF point olarak 0 ile {MAX_PAGE_DIMENSION_POINTS} arasında olmalı"
            ),
        ));
    }

    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        "insert_blank_page",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let bindings = pdfium.bindings();
            let document = RawDocument::open_for_mutation(bindings, &source, password.as_deref())?;
            let page_count = document.page_count()?;
            if at_index > page_count as u32 {
                return Err(DocumentIoFailure::preflight(format!(
                    "Ekleme indeksi aralık dışında: {at_index} > {page_count}"
                )));
            }
            let page = bindings.FPDFPage_New(
                document.handle,
                at_index as c_int,
                f64::from(width),
                f64::from(height),
            );
            if page.is_null() {
                return Err(DocumentIoFailure::write("Boş sayfa oluşturulamadı"));
            }
            bindings.FPDF_ClosePage(page);
            save_logged("insert_blank_page", &document, &source, temp)?;
            Ok((page_count + 1, width, height))
        },
        move |document, (expected_count, expected_width, expected_height)| {
            if document.pages().len() != *expected_count {
                return Err(DocumentIoFailure::invariant(format!(
                    "Blank-page sayfa sayısı invariant'ı bozuldu: beklenen {expected_count}, bulunan {}",
                    document.pages().len()
                )));
            }
            let size = document
                .pages()
                .page_size(at_index as u16)
                .map_err(|error| {
                    DocumentIoFailure::invariant(format!("Yeni sayfa MediaBox okunamadı: {error}"))
                })?;
            if (size.width().value - expected_width).abs() > 0.01
                || (size.height().value - expected_height).abs() > 0.01
            {
                return Err(DocumentIoFailure::invariant(format!(
                    "Yeni sayfa MediaBox invariant'ı bozuldu: beklenen {}x{}, bulunan {}x{}",
                    expected_width,
                    expected_height,
                    size.width().value,
                    size.height().value
                )));
            }
            Ok(())
        },
    )
}

pub fn merge_pdfs(file_paths: Vec<String>, output_path: String) -> Result<(), String> {
    if file_paths.is_empty() {
        return Err(
            "DOCUMENT_IO_ERROR operation=merge_pdfs stage=preflight state=prepared: \
             Birleştirilecek PDF listesi boş"
                .to_string(),
        );
    }
    let sources = file_paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    let sources_for_write = sources.clone();
    let target = PathBuf::from(output_path);
    atomic_write_pdf(
        "merge_pdfs",
        &sources,
        &target,
        move |pdfium, temp| {
            let mut documents = Vec::with_capacity(sources_for_write.len());
            let mut expected_count: u32 = 0;
            for source in &sources_for_write {
                let document = pdfium.load_pdf_from_file(source, None).map_err(|error| {
                    DocumentIoFailure::preflight(format!(
                        "{} yüklenemedi: {error}",
                        source.display()
                    ))
                })?;
                ensure_unprotected(&document)?;
                expected_count = expected_count
                    .checked_add(document.pages().len() as u32)
                    .ok_or_else(|| {
                        DocumentIoFailure::preflight(
                            "Birleştirme sayfa sayısı desteklenen aralığı aşıyor",
                        )
                    })?;
                documents.push(document);
            }
            if expected_count == 0 || expected_count > u16::MAX as u32 {
                return Err(DocumentIoFailure::preflight(format!(
                    "Birleştirme çıktı sayfa sayısı geçersiz: {expected_count}"
                )));
            }
            let mut output = pdfium.create_new_pdf().map_err(|error| {
                DocumentIoFailure::write(format!("Yeni PDF oluşturulamadı: {error}"))
            })?;
            for document in &documents {
                output.pages_mut().append(document).map_err(|error| {
                    DocumentIoFailure::write(format!("PDF birleştirilemedi: {error}"))
                })?;
            }
            output.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            Ok(expected_count as u16)
        },
        |document, expected_count| {
            if document.pages().len() != *expected_count {
                return Err(DocumentIoFailure::invariant(format!(
                    "Merge sayfa sayısı invariant'ı bozuldu: beklenen {expected_count}, bulunan {}",
                    document.pages().len()
                )));
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
    use std::path::Path;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pdf")
            .join(name)
    }

    fn copied_fixture(name: &str, directory: &Path) -> PathBuf {
        let target = directory.join(name);
        fs::copy(fixture(name), &target).unwrap();
        target
    }

    fn rotate_page_with_password(
        file_path: String,
        page_index: u32,
        degrees: i32,
        password: Option<String>,
    ) -> Result<(), String> {
        rotate_page_report(file_path, page_index, degrees, password)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn delete_pages_with_password(
        file_path: String,
        page_indices: Vec<u32>,
        password: Option<String>,
    ) -> Result<(), String> {
        delete_pages_report(file_path, page_indices, password)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn insert_blank_page_with_password(
        file_path: String,
        at_index: u32,
        width: f32,
        height: f32,
        password: Option<String>,
    ) -> Result<(), String> {
        insert_blank_page_report(file_path, at_index, width, height, password)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn assert_page_sizes(path: &Path, expected: &[(f32, f32)]) {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            assert_eq!(document.pages().len() as usize, expected.len());
            for (index, (width, height)) in expected.iter().enumerate() {
                let size = document
                    .pages()
                    .page_size(index as u16)
                    .map_err(|error| error.to_string())?;
                assert!((size.width().value - width).abs() <= 0.01);
                assert!((size.height().value - height).abs() <= 0.01);
            }
            Ok(())
        })
        .unwrap();
    }

    fn assert_reordered_pages(source: &Path, output: &Path, order: &[usize]) {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let source_document = pdfium
                .load_pdf_from_file(source, None)
                .map_err(|error| error.to_string())?;
            let output_document = pdfium
                .load_pdf_from_file(output, None)
                .map_err(|error| error.to_string())?;
            let source_pages = page_snapshots(&source_document).map_err(|error| error.detail)?;
            let expected = order
                .iter()
                .map(|index| source_pages[*index].clone())
                .collect::<Vec<_>>();
            validate_page_snapshots(&output_document, &expected).map_err(|error| error.detail)
        })
        .unwrap();
    }

    #[test]
    fn document_io_rotate_page_commits_valid_pdf() {
        let directory = tempfile::tempdir().unwrap();
        let path = copied_fixture("minimal.pdf", directory.path());
        let original = fs::read(&path).unwrap();
        let error =
            rotate_page_with_password(path.to_string_lossy().to_string(), 0, 45, None).unwrap_err();
        assert!(error.contains("90 derecenin katı"));
        assert_eq!(fs::read(&path).unwrap(), original);

        rotate_page_with_password(path.to_string_lossy().to_string(), 0, 90, None).unwrap();

        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let document = pdfium
                .load_pdf_from_file(&path, None)
                .map_err(|error| error.to_string())?;
            let rotation = document
                .pages()
                .get(0)
                .map_err(|error| error.to_string())?
                .rotation()
                .map_err(|error| error.to_string())?;
            assert_eq!(format!("{rotation:?}"), "Degrees90");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn document_io_rotate_page_is_relative_to_current_rotation() {
        let directory = tempfile::tempdir().unwrap();
        let path = copied_fixture("minimal.pdf", directory.path());
        let file_path = path.to_string_lossy().to_string();
        let page_rotation = || {
            crate::pdf_engine::with_pdfium(|pdfium| -> Result<String, String> {
                let document = pdfium
                    .load_pdf_from_file(&path, None)
                    .map_err(|error| error.to_string())?;
                let rotation = document
                    .pages()
                    .get(0)
                    .map_err(|error| error.to_string())?
                    .rotation()
                    .map_err(|error| error.to_string())?;
                Ok(format!("{rotation:?}"))
            })
            .unwrap()
        };

        rotate_page_with_password(file_path.clone(), 0, 90, None).unwrap();
        rotate_page_with_password(file_path.clone(), 0, 90, None).unwrap();
        assert_eq!(page_rotation(), "Degrees180");
        rotate_page_with_password(file_path.clone(), 0, -90, None).unwrap();
        assert_eq!(page_rotation(), "Degrees90");
        rotate_page_with_password(file_path.clone(), 0, -180, None).unwrap();
        assert_eq!(page_rotation(), "Degrees270");
        rotate_page_with_password(file_path, 0, 90, None).unwrap();
        assert_eq!(page_rotation(), "None");
    }

    #[test]
    fn document_io_rejects_encrypted_mutation_without_changing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = copied_fixture("encrypted.pdf", directory.path());
        let original = fs::read(&path).unwrap();

        let error = rotate_page_with_password(
            path.to_string_lossy().to_string(),
            0,
            90,
            Some("fixture-password".to_string()),
        )
        .unwrap_err();

        assert!(error.contains("Şifreli PDF mutasyonları henüz desteklenmiyor"));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn document_io_delete_pages_preflights_and_preserves_invalid_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = copied_fixture("three-distinct-pages.pdf", directory.path());
        let original = fs::read(&path).unwrap();

        let error =
            delete_pages_with_password(path.to_string_lossy().to_string(), vec![1, 1], None)
                .unwrap_err();
        assert!(error.contains("Tekrarlanan"));
        assert_eq!(fs::read(&path).unwrap(), original);

        let error = delete_pages_with_password(path.to_string_lossy().to_string(), vec![99], None)
            .unwrap_err();
        assert!(error.contains("aralık dışında"));
        assert_eq!(fs::read(&path).unwrap(), original);

        delete_pages_with_password(path.to_string_lossy().to_string(), vec![1], None).unwrap();
        assert_page_sizes(&path, &[(300.0, 400.0), (500.0, 600.0)]);
    }

    #[test]
    fn document_io_reorder_modes_preserve_full_and_subset_contracts() {
        let directory = tempfile::tempdir().unwrap();
        let source = copied_fixture("reorder-rich.pdf", directory.path());
        let subset = directory.path().join("subset.pdf");
        let original = fs::read(&source).unwrap();

        for (invalid_order, expected_error) in [
            (vec![2, 0], "tam permütasyon"),
            (vec![2, 2, 0], "Tekrarlanan"),
            (vec![0, 1, 99], "aralık dışında"),
            (vec![], "boş bırakılamaz"),
        ] {
            let error = reorder_pdf_to_target(
                source.to_string_lossy().to_string(),
                invalid_order,
                source.to_string_lossy().to_string(),
                None,
                ReorderMode::FullPermutation,
            )
            .unwrap_err();
            assert!(error.contains(expected_error), "{error}");
            assert_eq!(fs::read(&source).unwrap(), original);
        }

        reorder_pdf_to_target(
            source.to_string_lossy().to_string(),
            vec![2, 0],
            subset.to_string_lossy().to_string(),
            None,
            ReorderMode::OrderedSubset,
        )
        .unwrap();
        assert_reordered_pages(&source, &subset, &[2, 0]);

        let before_full_reorder = directory.path().join("before-full-reorder.pdf");
        fs::copy(&source, &before_full_reorder).unwrap();
        reorder_pdf_to_target(
            source.to_string_lossy().to_string(),
            vec![2, 0, 1],
            source.to_string_lossy().to_string(),
            None,
            ReorderMode::FullPermutation,
        )
        .unwrap();
        assert_reordered_pages(&before_full_reorder, &source, &[2, 0, 1]);
    }

    #[test]
    fn document_io_reorder_rejects_unpreserved_document_features() {
        for fixture_name in [
            "reorder-with-metadata.pdf",
            "reorder-with-outline.pdf",
            "reorder-with-named-destination.pdf",
            "reorder-with-form.pdf",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let source = copied_fixture(fixture_name, directory.path());
            let original = fs::read(&source).unwrap();

            let error = reorder_pdf_to_target(
                source.to_string_lossy().to_string(),
                vec![2, 0, 1],
                source.to_string_lossy().to_string(),
                None,
                ReorderMode::FullPermutation,
            )
            .unwrap_err();

            assert!(
                error.contains("güvenli biçimde koruyamıyor"),
                "{fixture_name}: {error}"
            );
            assert_eq!(fs::read(source).unwrap(), original, "{fixture_name}");
        }
    }

    #[test]
    fn document_io_insert_blank_page_validates_media_box() {
        let directory = tempfile::tempdir().unwrap();
        let path = copied_fixture("minimal.pdf", directory.path());
        insert_blank_page_with_password(path.to_string_lossy().to_string(), 1, 200.0, 300.0, None)
            .unwrap();
        assert_page_sizes(&path, &[(612.0, 792.0), (200.0, 300.0)]);
    }

    fn copied_from(relative: &str, directory: &Path) -> PathBuf {
        let target = directory.join(Path::new(relative).file_name().unwrap());
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(relative),
            &target,
        )
        .unwrap();
        target
    }

    fn signature_bytes(path: &Path) -> Vec<Vec<u8>> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            Ok(document
                .signatures()
                .iter()
                .map(|signature| signature.bytes())
                .collect())
        })
        .unwrap()
    }

    fn last_xref_section(bytes: &[u8]) -> (usize, String) {
        let keyword = bytes
            .windows(b"startxref".len())
            .rposition(|window| window == b"startxref")
            .unwrap();
        let offset: usize = String::from_utf8_lossy(&bytes[keyword + 9..])
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap();
        (
            offset,
            String::from_utf8_lossy(&bytes[offset..keyword]).into_owned(),
        )
    }

    type Mutation = fn(String) -> Result<(), String>;

    const PAGE_MUTATIONS: [(&str, Mutation); 3] = [
        ("rotate", |path| {
            rotate_page_with_password(path, 0, 90, None)
        }),
        ("delete", |path| {
            delete_pages_with_password(path, vec![1], None)
        }),
        ("insert", |path| {
            insert_blank_page_with_password(path, 0, 200.0, 300.0, None)
        }),
    ];

    #[test]
    fn document_io_page_mutations_append_an_incremental_update() {
        for (name, mutate) in PAGE_MUTATIONS {
            let directory = tempfile::tempdir().unwrap();
            let path = copied_fixture("three-distinct-pages.pdf", directory.path());
            let original = fs::read(&path).unwrap();
            mutate(path.to_string_lossy().to_string()).unwrap();
            let saved = fs::read(&path).unwrap();
            assert!(saved.len() > original.len(), "{name}");
            assert!(
                saved.starts_with(&original),
                "{name}: original bytes were rewritten"
            );
        }
    }

    #[test]
    fn document_io_page_mutations_keep_signed_bytes_and_signatures() {
        // signature.pdf tek sayfalıdır; silme tüm sayfaları sileceği için burada denenmez.
        for (name, mutate) in PAGE_MUTATIONS
            .into_iter()
            .filter(|(name, _)| *name != "delete")
        {
            let directory = tempfile::tempdir().unwrap();
            let path = copied_from("sanitization/signature.pdf", directory.path());
            let original = fs::read(&path).unwrap();
            let before = signature_bytes(&path);
            assert_eq!(before.len(), 1);
            mutate(path.to_string_lossy().to_string()).unwrap();
            assert!(
                fs::read(&path).unwrap().starts_with(&original),
                "{name}: signed bytes were rewritten"
            );
            assert_eq!(signature_bytes(&path), before, "{name}");
        }
    }

    #[test]
    fn document_io_rotate_of_an_xref_stream_pdf_writes_a_well_formed_xref_stream() {
        let directory = tempfile::tempdir().unwrap();
        let path = copied_from("sanitization/object-xref-stream.pdf", directory.path());
        let original = fs::read(&path).unwrap();
        rotate_page_with_password(path.to_string_lossy().to_string(), 0, 90, None).unwrap();
        let saved = fs::read(&path).unwrap();
        assert!(saved.starts_with(&original));
        let (offset, section) = last_xref_section(&saved);
        assert!(offset >= original.len());
        assert!(section.contains("/Type/XRef"), "{section}");
        assert!(section.trim_end().ends_with("endobj"), "{section}");
    }

    #[test]
    fn document_io_merge_validates_total_page_count() {
        let directory = tempfile::tempdir().unwrap();
        let first = copied_fixture("minimal.pdf", directory.path());
        let second_source = fixture("three-distinct-pages.pdf");
        let output = directory.path().join("merged.pdf");
        merge_pdfs(
            vec![
                first.to_string_lossy().to_string(),
                second_source.to_string_lossy().to_string(),
            ],
            output.to_string_lossy().to_string(),
        )
        .unwrap();
        assert_page_sizes(
            &output,
            &[
                (612.0, 792.0),
                (300.0, 400.0),
                (400.0, 500.0),
                (500.0, 600.0),
            ],
        );
    }
}
