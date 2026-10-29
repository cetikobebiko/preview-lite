//! Belgedeki gömülü görselleri dosyaya çıkarma (Acrobat: "Export all images").
//!
//! Job worker'da PDFium ile çalışır: seçilen sayfaların görsel nesneleri, form XObject'lerinin
//! içindekiler de dahil, kendi çözünürlüklerinde (`FPDFImageObj_GetBitmap`) PNG olarak yazılır.
//! Birden çok yerde kullanılan aynı görsel bir kez yazılır. Maske ve dönüşümler uygulanmaz:
//! dosya, görselin belgede saklandığı halidir.

use pdfium_render::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Bir işte en fazla yazılan görsel.
pub const MAX_EXTRACTED_IMAGES: usize = 2000;
/// Yazılan PNG'lerin toplam sınırı.
const MAX_TOTAL_BYTES: usize = 1024 * 1024 * 1024;
/// Bundan büyük görsel (piksel) atlanır.
const MAX_IMAGE_PIXELS: u64 = 64_000_000;
/// Form XObject iç içeliği sınırı.
const MAX_FORM_DEPTH: usize = 8;

/// Worker'ın iş dizininde yazdığı ad: `image-<sayfa>-<sıra>.png` (sayfa 1 tabanlı).
pub(crate) fn worker_file_name(page_index: u32, ordinal: usize) -> String {
    format!("image-{:05}-{:03}.png", u64::from(page_index) + 1, ordinal)
}

/// Kullanıcının klasöründeki ad; worker adından türetilir, böylece yayın aynı sırayı izler.
pub(crate) fn target_file_name(stem: &str, worker_name: &str) -> Option<String> {
    let rest = worker_name.strip_prefix("image-")?.strip_suffix(".png")?;
    let (page, ordinal) = rest.split_once('-')?;
    let page: u64 = page.parse().ok()?;
    let ordinal: u64 = ordinal.parse().ok()?;
    Some(format!("{stem}_sayfa_{page}_gorsel_{ordinal}.png"))
}

struct Budget {
    images: usize,
    bytes: usize,
    seen: HashSet<[u8; 32]>,
}

/// `pages`'teki (boşsa tümü) görselleri `output_dir`'e yazar; yazılan dosyaları sırayla döndürür.
pub(crate) fn extract_images(
    document: &PdfDocument<'_>,
    pages: &[u32],
    output_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let count = u32::from(document.pages().len());
    let pages: Vec<u32> = if pages.is_empty() {
        (0..count).collect()
    } else {
        pages.to_vec()
    };
    if let Some(bad) = pages.iter().find(|page| **page >= count) {
        return Err(format!("Sayfa aralık dışında: {} > {count}", bad + 1));
    }
    let mut budget = Budget {
        images: 0,
        bytes: 0,
        seen: HashSet::new(),
    };
    let mut written = Vec::new();
    let mut done = HashSet::new();
    for page_index in pages {
        if !done.insert(page_index) {
            continue;
        }
        let page = document
            .pages()
            .get(page_index as u16)
            .map_err(|error| format!("Sayfa {} açılamadı: {error}", page_index + 1))?;
        let mut ordinal = 0;
        for object in page.objects().iter() {
            visit(&object, 0, &mut |image| {
                if let Some(png) = encode(image, &mut budget)? {
                    ordinal += 1;
                    let path = output_dir.join(worker_file_name(page_index, ordinal));
                    std::fs::write(&path, png)
                        .map_err(|error| format!("Görsel yazılamadı: {error}"))?;
                    written.push(path);
                }
                Ok(())
            })?;
            if budget.images >= MAX_EXTRACTED_IMAGES {
                return Ok(written);
            }
        }
    }
    Ok(written)
}

fn visit(
    object: &PdfPageObject<'_>,
    depth: usize,
    found: &mut dyn FnMut(&PdfPageImageObject<'_>) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(image) = object.as_image_object() {
        return found(image);
    }
    if depth >= MAX_FORM_DEPTH {
        return Ok(());
    }
    if let Some(form) = object.as_x_object_form_object() {
        for index in 0..form.len() {
            if let Ok(child) = form.get(index) {
                visit(&child, depth + 1, found)?;
            }
        }
    }
    Ok(())
}

/// Görselin PNG'si; çok büyük, okunamayan, daha önce yazılmış ya da bütçeyi aşan görselde `None`.
fn encode(image: &PdfPageImageObject<'_>, budget: &mut Budget) -> Result<Option<Vec<u8>>, String> {
    if budget.images >= MAX_EXTRACTED_IMAGES {
        return Ok(None);
    }
    let (Ok(width), Ok(height)) = (image.width(), image.height()) else {
        return Ok(None);
    };
    let pixels = u64::from(width.unsigned_abs()) * u64::from(height.unsigned_abs());
    if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
        return Ok(None);
    }
    let Ok(decoded) = image.get_raw_image() else {
        return Ok(None);
    };
    let digest: [u8; 32] = Sha256::new()
        .chain_update(decoded.width().to_le_bytes())
        .chain_update(decoded.height().to_le_bytes())
        .chain_update(decoded.as_bytes())
        .finalize()
        .into();
    if !budget.seen.insert(digest) {
        return Ok(None);
    }
    let mut png = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| format!("Görsel PNG'ye kodlanamadı: {error}"))?;
    let png = png.into_inner();
    if budget.bytes + png.len() > MAX_TOTAL_BYTES {
        return Err(
            "Çıkarılan görsellerin toplamı 1 GiB sınırını aşıyor; daha az sayfa seçin".into(),
        );
    }
    budget.images += 1;
    budget.bytes += png.len();
    Ok(Some(png))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF with the same image drawn twice on page 1 and a different one on page 2.
    fn two_page_pdf_with_images() -> Vec<u8> {
        let directory = tempfile::tempdir().unwrap();
        let red = directory.path().join("red.png");
        let blue = directory.path().join("blue.png");
        image::RgbImage::from_pixel(8, 4, image::Rgb([255, 0, 0]))
            .save(&red)
            .unwrap();
        image::RgbImage::from_pixel(3, 5, image::Rgb([0, 0, 255]))
            .save(&blue)
            .unwrap();
        let paths = [red.clone(), red, blue].map(|path| path.to_string_lossy().into_owned());
        let options = crate::images_to_pdf::ImagesToPdfOptions {
            page_size: crate::images_to_pdf::PageSizing::Image,
            margin: 0.0,
        };
        crate::images_to_pdf::build_pdf(&paths, &options).unwrap()
    }

    #[test]
    fn writes_each_distinct_image_once_in_page_order() {
        let bytes = two_page_pdf_with_images();
        let output = tempfile::tempdir().unwrap();
        let files = crate::pdf_engine::with_pdfium(|pdfium| {
            let document = pdfium.load_pdf_from_byte_slice(&bytes, None).unwrap();
            extract_images(&document, &[], output.path())
        })
        .unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Page 2 repeats page 1's image, so only pages 1 and 3 write one.
        assert_eq!(names, ["image-00001-001.png", "image-00003-001.png"]);
        let first = image::open(&files[0]).unwrap();
        assert_eq!((first.width(), first.height()), (8, 4));
        let second = image::open(&files[1]).unwrap();
        assert_eq!((second.width(), second.height()), (3, 5));
    }

    #[test]
    fn rejects_pages_out_of_range() {
        let bytes = two_page_pdf_with_images();
        let output = tempfile::tempdir().unwrap();
        let error = crate::pdf_engine::with_pdfium(|pdfium| {
            let document = pdfium.load_pdf_from_byte_slice(&bytes, None).unwrap();
            extract_images(&document, &[3], output.path())
        })
        .unwrap_err();
        assert!(error.contains("aralık dışında"), "{error}");
    }

    #[test]
    fn target_names_follow_worker_names() {
        assert_eq!(
            target_file_name("rapor", &worker_file_name(4, 2)).as_deref(),
            Some("rapor_sayfa_5_gorsel_2.png")
        );
        assert_eq!(target_file_name("rapor", "other.png"), None);
    }
}
