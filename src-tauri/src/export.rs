use image::{RgbImage, RgbaImage};
use pdfium_render::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use crate::document_session::{
    SessionJobCancellationCheckpoint, SessionJobChildProcessOwner, SessionJobOperationError,
};
use crate::pdf_engine::render_service::{checked_rgba_len, MAX_PIXEL_DIMENSION, MAX_RENDER_BYTES};
use crate::pdf_engine::{get_pdfium, open_pdfium_doc};

pub(crate) fn submit_print_file(
    print_path: &Path,
    cancellation: &SessionJobCancellationCheckpoint,
    child_processes: &SessionJobChildProcessOwner,
) -> Result<(), SessionJobOperationError> {
    cancellation.check_cancelled()?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("lpr");
        command.arg(print_path);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("lp");
        command.arg(print_path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("powershell");
        command.args([
            "-Command",
            &format!(
                "Start-Process -FilePath \"{}\" -Verb Print",
                print_path.display()
            ),
        ]);
        command
    };
    let status = child_processes.run_with_timeout(
        &mut command,
        std::time::Duration::from_secs(120),
        cancellation,
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(SessionJobOperationError::failed(
            "Yazdırma işlemi başarısız oldu",
        ))
    }
}

/// Görsel dışa aktarımda kabul edilen DPI aralığı. 36 DPI, PDF'in 72 pt/inç
/// biriminin yarısıdır; altında metin okunaksızlaşır. 1200 DPI profesyonel
/// baskı/prepress çıktılarının üst sınırıdır; daha yükseği kartvizit boyutundaki
/// sayfalarda bile piksel/bayt sınırına takılır. Asıl bellek koruması sayfa
/// başına `checked_export_render_size` kontrolüdür; bu aralık yalnız IPC'den
/// gelen anlamsız değerleri (NaN, negatif, 1e30) erken reddeder.
pub(crate) const MIN_EXPORT_DPI: f32 = 36.0;
pub(crate) const MAX_EXPORT_DPI: f32 = 1200.0;

pub(crate) const EXPORT_DPI_INVALID_CODE: &str = "SESSION_JOB_INVALID_EXPORT_DPI";
pub(crate) const EXPORT_RENDER_LIMIT_CODE: &str = "SESSION_JOB_RENDER_LIMIT_EXCEEDED";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExportRenderSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) fn validate_export_dpi(dpi: f32) -> Result<(), SessionJobOperationError> {
    if dpi.is_finite() && (MIN_EXPORT_DPI..=MAX_EXPORT_DPI).contains(&dpi) {
        return Ok(());
    }
    Err(SessionJobOperationError::failed_with_code(
        EXPORT_DPI_INVALID_CODE,
        format!(
            "Çözünürlük {MIN_EXPORT_DPI}–{MAX_EXPORT_DPI} DPI aralığında olmalıdır (istenen: {dpi})"
        ),
    ))
}

/// Canlı render ile aynı sınırlar: kenar başına `MAX_PIXEL_DIMENSION`, RGBA
/// olarak `MAX_RENDER_BYTES`. Boyutlar f64'te hesaplanır ve u32'ye yalnız
/// sınır içindeyse dönüştürülür.
fn export_render_size_at(page_width: f64, page_height: f64, dpi: f64) -> Option<ExportRenderSize> {
    let scale = dpi / 72.0;
    let width = (page_width * scale).round();
    let height = (page_height * scale).round();
    let max_dimension = f64::from(MAX_PIXEL_DIMENSION);
    if !(1.0..=max_dimension).contains(&width) || !(1.0..=max_dimension).contains(&height) {
        return None;
    }
    let size = ExportRenderSize {
        width: width as u32,
        height: height as u32,
    };
    checked_rgba_len(size.width, size.height).ok().map(|_| size)
}

/// Sayfanın sınırlara sığdığı en yüksek tam sayı DPI; `MIN_EXPORT_DPI`
/// altında kalırsa `None`.
fn max_export_dpi_for_page(page_width: f64, page_height: f64) -> Option<f64> {
    let max_dimension = f64::from(MAX_PIXEL_DIMENSION);
    let max_pixels = (MAX_RENDER_BYTES / 4) as f64;
    let scale = (max_dimension / page_width)
        .min(max_dimension / page_height)
        .min((max_pixels / (page_width * page_height)).sqrt());
    let mut dpi = (scale * 72.0).floor().min(f64::from(MAX_EXPORT_DPI));
    // Yuvarlama sınırı birkaç piksel aşabilir; sığana kadar DPI'ı düşür.
    while dpi >= f64::from(MIN_EXPORT_DPI) {
        if export_render_size_at(page_width, page_height, dpi).is_some() {
            return Some(dpi);
        }
        dpi -= 1.0;
    }
    None
}

/// `page_width`/`page_height` PDFium'un /Rotate uygulanmış sayfa boyutudur
/// (`FPDF_GetPageWidthF`/`FPDF_GetPageHeightF`, pt). Sınır aşılırsa render
/// ölçeklenmez; kullanıcı istediği DPI'dan farklı bir çıktıyı fark etmeden
/// almasın diye açık hata ve önerilen en yüksek DPI döner.
pub(crate) fn checked_export_render_size(
    page_number: u32,
    page_width: f32,
    page_height: f32,
    dpi: f32,
) -> Result<ExportRenderSize, SessionJobOperationError> {
    validate_export_dpi(dpi)?;
    let page_width = f64::from(page_width);
    let page_height = f64::from(page_height);
    if !page_width.is_finite()
        || !page_height.is_finite()
        || page_width <= 0.0
        || page_height <= 0.0
    {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_INVALID_RENDER_PARAMETERS",
            format!("Sayfa {page_number} boyutları geçersiz"),
        ));
    }
    let dpi = f64::from(dpi);
    if let Some(size) = export_render_size_at(page_width, page_height, dpi) {
        return Ok(size);
    }
    let scale = dpi / 72.0;
    let width = (page_width * scale).round();
    let height = (page_height * scale).round();
    if width < 1.0 || height < 1.0 {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_INVALID_RENDER_PARAMETERS",
            format!("Sayfa {page_number} {dpi} DPI'da bir pikselden küçük kalıyor"),
        ));
    }
    let limit = format!(
        "Sayfa {page_number}, {dpi} DPI'da {width:.0}×{height:.0} piksel olur; görsel dışa aktarım en fazla {MAX_PIXEL_DIMENSION}×{MAX_PIXEL_DIMENSION} piksel ve {} MiB destekler.",
        MAX_RENDER_BYTES / (1024 * 1024)
    );
    let suggestion = match max_export_dpi_for_page(page_width, page_height) {
        Some(max_dpi) => format!(" Bu sayfa için en fazla {max_dpi:.0} DPI seçin."),
        None => {
            format!(" Bu sayfa en düşük çözünürlükte ({MIN_EXPORT_DPI} DPI) bile sınırı aşıyor.")
        }
    };
    Err(SessionJobOperationError::failed_with_code(
        EXPORT_RENDER_LIMIT_CODE,
        format!("{limit}{suggestion}"),
    ))
}

/// IPC'den gelen u32 indeksi PDFium'un u16 indeksine kayıpsız çevirir;
/// belgede olmayan sayfa sessizce atlanmaz, hata olur.
fn checked_export_page_index(
    page_idx: u32,
    page_count: PdfPageIndex,
) -> Result<PdfPageIndex, SessionJobOperationError> {
    PdfPageIndex::try_from(page_idx)
        .ok()
        .filter(|index| *index < page_count)
        .ok_or_else(|| {
            SessionJobOperationError::failed_with_code(
                "SESSION_JOB_INVALID_PAGE",
                format!(
                    "Sayfa {} belgede yok (sayfa sayısı: {page_count})",
                    u64::from(page_idx) + 1
                ),
            )
        })
}

/// Dışa aktarılan bitmap'i diske yazar ve tepe belleği iki bitmap boyutuyla
/// sınırlar. pdfium-render 0.8 PDFium tamponuna ödünç erişim vermediği için
/// `as_raw_bytes` bir kopya alır; PDFium bitmap'i hemen bırakılır, encoder
/// yalnız bu kopya üzerinde çalışır. JPEG için alfa kanalı yerinde atılır;
/// `image`'in kendi RGBA→RGB dönüşümü üçüncü bir tampon ayırırdı.
fn write_export_bitmap(
    bitmap: PdfBitmap<'_>,
    size: ExportRenderSize,
    format: ExportImageFormat,
    out_path: &Path,
) -> Result<(), SessionJobOperationError> {
    let actual = (bitmap.width(), bitmap.height());
    if actual != (size.width as i32, size.height as i32)
        || bitmap.format().ok() != Some(PdfBitmapFormat::BGRA)
    {
        return Err(SessionJobOperationError::failed(format!(
            "PDFium beklenmeyen bitmap döndürdü: {}×{}",
            actual.0, actual.1
        )));
    }
    let mut pixels = bitmap.as_raw_bytes();
    drop(bitmap);
    let expected = checked_rgba_len(size.width, size.height)
        .map_err(|e| SessionJobOperationError::failed(format!("Bitmap boyutu geçersiz: {e}")))?;
    if pixels.len() != expected {
        return Err(SessionJobOperationError::failed(
            "PDFium geçersiz bitmap tamponu döndürdü",
        ));
    }
    let saved = match format {
        ExportImageFormat::Png => RgbaImage::from_raw(size.width, size.height, pixels)
            .map(|image| image.save_with_format(out_path, image::ImageFormat::Png)),
        ExportImageFormat::Jpeg => {
            rgba_to_rgb_in_place(&mut pixels);
            RgbImage::from_raw(size.width, size.height, pixels)
                .map(|image| image.save_with_format(out_path, image::ImageFormat::Jpeg))
        }
    };
    saved
        .ok_or_else(|| SessionJobOperationError::failed("Görsel tamponu oluşturulamadı"))?
        .map_err(|e| SessionJobOperationError::failed(format!("Görsel kaydedilemedi: {}", e)))
}

/// RGBA pikselleri aynı tampon içinde RGB'ye sıkıştırır; kapasite korunur,
/// yeni ayırma yapılmaz.
fn rgba_to_rgb_in_place(pixels: &mut Vec<u8>) {
    let count = pixels.len() / 4;
    for index in 0..count {
        pixels.copy_within(index * 4..index * 4 + 3, index * 3);
    }
    pixels.truncate(count * 3);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportImageFormat {
    Png,
    Jpeg,
}

impl ExportImageFormat {
    fn from_request(format: &str) -> Self {
        if format.eq_ignore_ascii_case("jpeg") {
            Self::Jpeg
        } else {
            Self::Png
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

fn export_image_file_name(page_idx: u32, format: ExportImageFormat) -> String {
    format!("sayfa_{}.{}", u64::from(page_idx) + 1, format.extension())
}

/// Outputs the image export worker produces for `pages`, in output order, and
/// the names they are published under (`{stem}_sayfa_{n}.{ext}`). A repeated
/// page yields one file, at its first position, as in the worker.
pub(crate) fn planned_export_image_outputs(
    pages: &[u32],
    format: &str,
    stem: &str,
) -> Vec<crate::output_names::PlannedOutput> {
    let format = ExportImageFormat::from_request(format);
    let mut seen_pages = HashSet::with_capacity(pages.len());
    pages
        .iter()
        .filter(|page_idx| seen_pages.insert(**page_idx))
        .map(|&page_idx| crate::output_names::PlannedOutput {
            worker_name: export_image_file_name(page_idx, format),
            target_name: crate::output_names::page_image_output_name(
                stem,
                page_idx,
                format.extension(),
            ),
        })
        .collect()
}

pub(crate) fn export_pages_as_images_with_password(
    file_path: String,
    pages: Vec<u32>,
    output_dir: String,
    format: String,
    dpi: f32,
    password: Option<String>,
    cancellation: Option<&SessionJobCancellationCheckpoint>,
) -> Result<Vec<String>, SessionJobOperationError> {
    validate_export_dpi(dpi)?;
    let format = ExportImageFormat::from_request(&format);
    let pdfium = get_pdfium().map_err(SessionJobOperationError::from)?;
    let document = open_pdfium_doc(&pdfium, &file_path, password.as_deref())
        .map_err(SessionJobOperationError::from)?;

    let doc_pages = document.pages();
    let page_count = doc_pages.len();

    // Tüm sayfalar render'dan önce doğrulanır: sınır aşan bir sayfa hiçbir
    // bitmap ayrılmadan ve kısmi çıktı yazılmadan raporlanır. Tekrarlanan
    // sayfa aynı dosyayı üreteceği için bir kez, ilk geçtiği sırada işlenir.
    let mut seen_pages = HashSet::with_capacity(pages.len());
    let mut planned = Vec::with_capacity(pages.len());
    for page_idx in pages {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if !seen_pages.insert(page_idx) {
            continue;
        }
        let index = checked_export_page_index(page_idx, page_count)?;
        let page = doc_pages.get(index).map_err(|e| {
            SessionJobOperationError::failed(format!("Sayfa {} yüklenemedi: {:?}", page_idx + 1, e))
        })?;
        let size =
            checked_export_render_size(page_idx + 1, page.width().value, page.height().value, dpi)?;
        planned.push((page_idx, index, size));
    }

    let mut generated_files = Vec::with_capacity(planned.len());
    for (page_idx, index, size) in planned {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        let page = doc_pages.get(index).map_err(|e| {
            SessionJobOperationError::failed(format!("Sayfa {} yüklenemedi: {:?}", page_idx + 1, e))
        })?;

        // Doğrulanan boyutlar sınır içinde olduğundan i32'ye sığar.
        // FPDF_REVERSE_BYTE_ORDER ile PDFium doğrudan RGBA yazar.
        let render_config = PdfRenderConfig::new()
            .set_target_size(size.width as i32, size.height as i32)
            .set_reverse_byte_order(true);

        let bitmap = page.render_with_config(&render_config).map_err(|e| {
            SessionJobOperationError::failed(format!("Sayfa render etme hatası: {:?}", e))
        })?;

        let out_path = Path::new(&output_dir).join(export_image_file_name(page_idx, format));
        write_export_bitmap(bitmap, size, format, &out_path)?;

        generated_files.push(out_path.to_string_lossy().to_string());
    }

    Ok(generated_files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure_code(error: SessionJobOperationError) -> String {
        match error {
            SessionJobOperationError::Failed(failure) => failure.code,
            SessionJobOperationError::Cancelled(reason) => panic!("unexpected cancel: {reason:?}"),
        }
    }

    fn failure_message(error: SessionJobOperationError) -> String {
        match error {
            SessionJobOperationError::Failed(failure) => failure.message,
            SessionJobOperationError::Cancelled(reason) => panic!("unexpected cancel: {reason:?}"),
        }
    }

    #[test]
    fn export_dpi_rejects_non_finite_negative_and_out_of_range_values() {
        for dpi in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -300.0,
            0.0,
            35.9,
            1200.1,
            1e30,
            f32::MAX,
        ] {
            let error = validate_export_dpi(dpi).unwrap_err();
            assert_eq!(failure_code(error), EXPORT_DPI_INVALID_CODE, "dpi {dpi}");
            let error = checked_export_render_size(1, 595.0, 842.0, dpi).unwrap_err();
            assert_eq!(failure_code(error), EXPORT_DPI_INVALID_CODE, "dpi {dpi}");
        }
        for dpi in [MIN_EXPORT_DPI, 72.0, 150.0, 300.0, 600.0, MAX_EXPORT_DPI] {
            assert!(validate_export_dpi(dpi).is_ok(), "dpi {dpi}");
        }
    }

    #[test]
    fn a4_at_300_dpi_is_within_limits() {
        let size = checked_export_render_size(1, 595.0, 842.0, 300.0).unwrap();
        assert_eq!(
            size,
            ExportRenderSize {
                width: 2479,
                height: 3508
            }
        );
        // Landscape (/Rotate 90 sonrası) aynı alanı aynı sınırla kabul eder.
        let size = checked_export_render_size(1, 842.0, 595.0, 300.0).unwrap();
        assert_eq!(
            size,
            ExportRenderSize {
                width: 3508,
                height: 2479
            }
        );
    }

    #[test]
    fn a0_at_600_dpi_is_rejected_with_a_lower_dpi_suggestion() {
        // 2384×3370 pt × 600/72 ≈ 19867×28083 px ≈ 2.2 GB RGBA.
        let error = checked_export_render_size(1, 2384.0, 3370.0, 600.0).unwrap_err();
        let SessionJobOperationError::Failed(failure) = error else {
            panic!("expected failure");
        };
        assert_eq!(failure.code, EXPORT_RENDER_LIMIT_CODE);
        assert!(
            failure.message.contains("19867×28083"),
            "{}",
            failure.message
        );
        let max_dpi = max_export_dpi_for_page(2384.0, 3370.0).unwrap();
        assert!(failure
            .message
            .contains(&format!("en fazla {max_dpi:.0} DPI")));
        assert!(export_render_size_at(2384.0, 3370.0, max_dpi).is_some());
        assert!(export_render_size_at(2384.0, 3370.0, max_dpi + 1.0).is_none());
    }

    #[test]
    fn large_page_dimensions_fixture_size_is_rejected_at_common_dpis() {
        // tests/fixtures/pdf/large-page-dimensions.pdf: 7200×7200 pt.
        for dpi in [150.0, 300.0, 600.0, MAX_EXPORT_DPI] {
            let error = checked_export_render_size(1, 7200.0, 7200.0, dpi).unwrap_err();
            assert_eq!(failure_code(error), EXPORT_RENDER_LIMIT_CODE, "dpi {dpi}");
        }
        assert_eq!(max_export_dpi_for_page(7200.0, 7200.0), Some(81.0));
        assert!(checked_export_render_size(1, 7200.0, 7200.0, 81.0).is_ok());
    }

    #[test]
    fn single_side_above_pixel_dimension_is_rejected_even_when_bytes_fit() {
        // 16385×100 px yalnız ~6.5 MB ama kenar sınırını aşar.
        let error = checked_export_render_size(1, 16_385.0, 100.0, 72.0).unwrap_err();
        assert_eq!(failure_code(error), EXPORT_RENDER_LIMIT_CODE);
        assert!(checked_export_render_size(1, 16_384.0, 100.0, 72.0).is_ok());
    }

    #[test]
    fn page_too_large_even_at_minimum_dpi_says_so() {
        let message = failure_message(
            checked_export_render_size(3, 200_000.0, 200_000.0, 300.0).unwrap_err(),
        );
        assert!(message.starts_with("Sayfa 3,"), "{message}");
        assert!(message.contains("en düşük çözünürlükte"), "{message}");
    }

    #[test]
    fn invalid_page_geometry_is_rejected_before_casting() {
        for (width, height) in [
            (f32::NAN, 842.0),
            (595.0, f32::INFINITY),
            (0.0, 842.0),
            (595.0, -1.0),
        ] {
            let error = checked_export_render_size(1, width, height, 300.0).unwrap_err();
            assert_eq!(failure_code(error), "SESSION_JOB_INVALID_RENDER_PARAMETERS");
        }
        let error = checked_export_render_size(1, 0.4, 0.4, 72.0).unwrap_err();
        assert_eq!(failure_code(error), "SESSION_JOB_INVALID_RENDER_PARAMETERS");
    }

    #[test]
    fn page_index_conversion_rejects_out_of_range_and_u16_overflow() {
        assert_eq!(checked_export_page_index(0, 3).unwrap(), 0);
        assert_eq!(checked_export_page_index(2, 3).unwrap(), 2);
        for page_idx in [3, u32::from(u16::MAX), u32::from(u16::MAX) + 1, u32::MAX] {
            let error = checked_export_page_index(page_idx, 3).unwrap_err();
            assert_eq!(
                failure_code(error),
                "SESSION_JOB_INVALID_PAGE",
                "{page_idx}"
            );
        }
        // 65536 → `as u16` ile 0'a sarardı; artık hata.
        let message = failure_message(checked_export_page_index(65_536, u16::MAX).unwrap_err());
        assert!(message.contains("Sayfa 65537"), "{message}");
    }

    #[test]
    fn rgba_to_rgb_in_place_drops_alpha_without_reallocating() {
        let mut pixels = vec![1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 0];
        let capacity = pixels.capacity();
        let buffer = pixels.as_ptr();
        rgba_to_rgb_in_place(&mut pixels);
        assert_eq!(pixels, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(pixels.capacity(), capacity);
        assert_eq!(pixels.as_ptr(), buffer);
    }

    #[test]
    fn export_format_maps_requests_to_extensions() {
        assert_eq!(ExportImageFormat::from_request("jpeg").extension(), "jpg");
        assert_eq!(ExportImageFormat::from_request("JPEG").extension(), "jpg");
        assert_eq!(ExportImageFormat::from_request("png").extension(), "png");
        // Mevcut davranış: bilinmeyen biçim PNG'ye düşer.
        assert_eq!(ExportImageFormat::from_request("webp").extension(), "png");
    }
}
