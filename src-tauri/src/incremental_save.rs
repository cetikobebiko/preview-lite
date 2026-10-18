//! Artımlı (append-only) PDF kaydı
//! (docs/engineering/undo-redo-and-incremental-save-design.md §4.2).
//!
//! PDFium'un `FPDF_SaveAsCopy(FPDF_INCREMENTAL)` yolu ve onun xref stream kusurunun onarımı
//! burada toplanır. Yalnız job worker'da, session snapshot'ının kopyası üzerinde,
//! `atomic_write_pdf` içinde çağrılır.

use crate::document_io::DocumentIoFailure;
use pdfium_render::prelude::*;
use std::ffi::c_void;
use std::io::Write;
use std::os::raw::{c_int, c_ulong};
use std::path::Path;

pub(crate) const FPDF_INCREMENTAL: FPDF_DWORD = 1;
pub(crate) const FPDF_FULL_REWRITE: FPDF_DWORD = 0;

/// Saves as an incremental update: the original bytes stay byte-for-byte at the start of
/// the file and the changes are appended, so the byte ranges existing signatures cover are
/// untouched and the signatures stay valid for their revision. Only an unsigned document
/// falls back to a full rewrite when PDFium cannot save incrementally.
pub(crate) fn save_preserving_signatures(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    source: &Path,
    temp: &Path,
) -> Result<(), DocumentIoFailure> {
    let signature_count = bindings.FPDF_GetSignatureCount(document);
    if let Err(error) = save_copy(bindings, document, temp, FPDF_INCREMENTAL) {
        if signature_count > 0 {
            return Err(DocumentIoFailure::write(format!(
                "İmzalı belge artımlı (incremental) kaydedilemedi; imzaları korumak için tam yeniden yazıma geçilmedi: {}",
                error.detail
            )));
        }
        return save_copy(bindings, document, temp, FPDF_FULL_REWRITE);
    }
    let original = std::fs::read(source)
        .map_err(|error| DocumentIoFailure::write(format!("Kaynak PDF okunamadı: {error}")))?;
    let mut saved = std::fs::read(temp)
        .map_err(|error| DocumentIoFailure::write(format!("Temp PDF okunamadı: {error}")))?;
    if signature_count > 0 && !saved.starts_with(&original) {
        return Err(DocumentIoFailure::invariant(
            "Artımlı kayıt imzalı baytları korumadı",
        ));
    }
    if saved.starts_with(&original) && repair_incremental_xref_stream(&mut saved, original.len()) {
        std::fs::write(temp, &saved)
            .map_err(|error| DocumentIoFailure::write(format!("Temp PDF yazılamadı: {error}")))?;
    }
    Ok(())
}

/// PDFium (7961; `main`'de de aynı) kaynak dosya cross-reference stream kullanıyorsa artımlı
/// güncellemenin xref stream nesnesini `/Type /XRef` olmadan ve `endobj` olmadan yazar
/// (`CPDF_Creator::WriteDoc_Stage4`: trailer'dan `Type`'ı atlar, `endstream`'den sonra
/// `startxref`'e geçer). PDFium bunu okur ama CoreGraphics (Preview, PDFKit) reddeder.
///
/// Onarım yalnız PDFium'un yazdığı kalıp birebir eşleşirse yapılır: `startxref` güncellemenin
/// içinde `N 0 obj <<` ile başlayan bir nesneyi göstermeli. Eklemeler o nesnenin başlangıcından
/// sonradır; `startxref`, xref girdileri ve stream `/Length` değişmez. Onarım yapıldıysa true.
pub(crate) fn repair_incremental_xref_stream(output: &mut Vec<u8>, original_len: usize) -> bool {
    let Some(startxref) = find_last(output, b"startxref") else {
        return false;
    };
    let digits: Vec<u8> = output[startxref + b"startxref".len()..]
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect();
    let Some(xref_offset) = std::str::from_utf8(&digits)
        .ok()
        .and_then(|text| text.parse::<usize>().ok())
    else {
        return false;
    };
    if xref_offset < original_len || xref_offset >= startxref {
        return false;
    }
    let object = &output[xref_offset..startxref];
    let header_len = object
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if header_len == 0 || !object[header_len..].starts_with(b" 0 obj <<") {
        return false;
    }
    let dict_start = xref_offset + header_len + b" 0 obj <<".len();
    let Some(stream_keyword) = find_first(&output[dict_start..startxref], b">>stream") else {
        return false;
    };
    let dict = &output[dict_start..dict_start + stream_keyword];
    if find_first(dict, b"/W[").is_none() || find_first(dict, b"/Index[").is_none() {
        return false;
    }
    let has_type = find_first(dict, b"/Type").is_some();
    let mut repaired = false;
    let mut insert_endobj = None;
    if let Some(endstream) = find_last(&output[dict_start..startxref], b"endstream") {
        let after = dict_start + endstream + b"endstream".len();
        let rest = &output[after..startxref];
        if find_first(rest, b"endobj").is_none() {
            insert_endobj = Some(after);
        }
    } else {
        return false;
    }
    // Sondan başa ekle ki önceki konumlar geçerli kalsın.
    if let Some(at) = insert_endobj {
        output.splice(at..at, b"\r\nendobj".iter().copied());
        repaired = true;
    }
    if !has_type {
        output.splice(dict_start..dict_start, b"/Type/XRef".iter().copied());
        repaired = true;
    }
    repaired
}

fn find_first(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// `FPDF_SaveAsCopy` için dosyaya yazan `FPDF_FILEWRITE`. `base` ilk alan olmalıdır:
/// PDFium callback'e bu alanın adresini geri verir.
#[repr(C)]
struct FileWriter {
    base: FPDF_FILEWRITE,
    file: std::fs::File,
    failed: bool,
}

unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    // SAFETY: `this` points at the `base` field of the live, pinned `FileWriter` passed to
    // `FPDF_SaveAsCopy` below (`#[repr(C)]`, `base` first), and PDFium passes a readable
    // buffer of `size` bytes that is valid for the duration of this call.
    let writer = unsafe { &mut *this.cast::<FileWriter>() };
    let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) };
    if writer.file.write_all(bytes).is_err() {
        writer.failed = true;
        return 0;
    }
    1
}

pub(crate) fn save_copy(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    temp: &Path,
    flags: FPDF_DWORD,
) -> Result<(), DocumentIoFailure> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(temp)
        .map_err(|error| DocumentIoFailure::write(format!("Temp PDF açılamadı: {error}")))?;
    let mut writer = Box::new(FileWriter {
        base: FPDF_FILEWRITE {
            version: 1,
            WriteBlock: Some(write_block),
        },
        file,
        failed: false,
    });
    let saved = bindings.FPDF_SaveAsCopy(document, &mut writer.base, flags);
    if !bindings.is_true(saved) || writer.failed {
        return Err(DocumentIoFailure::write("Temp PDF kaydedilemedi"));
    }
    writer.file.sync_all().map_err(|error| {
        DocumentIoFailure::write(format!("Temp PDF senkronize edilemedi: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    fn last_xref_object(bytes: &[u8]) -> (usize, Vec<u8>) {
        let startxref = find_last(bytes, b"startxref").unwrap();
        let digits: String = String::from_utf8_lossy(&bytes[startxref + 9..])
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let offset: usize = digits.parse().unwrap();
        (offset, bytes[offset..startxref].to_vec())
    }

    /// Fixture'ın ilk sayfasını PDFium ile 90° döndürüp `save_preserving_signatures` ile kaydeder.
    fn rotate_first_page(relative: &str, password: Option<&str>) -> (Vec<u8>, Vec<u8>) {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.pdf");
        let temp = directory.path().join("temp.pdf");
        std::fs::copy(fixture(relative), &source).unwrap();
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let bindings = pdfium.bindings();
            let document = bindings.FPDF_LoadDocument(source.to_str().unwrap(), password);
            assert!(!document.is_null(), "{relative} açılamadı");
            let page = bindings.FPDF_LoadPage(document, 0);
            bindings.FPDFPage_SetRotation(page, 1);
            bindings.FPDF_ClosePage(page);
            let result = save_preserving_signatures(bindings, document, &source, &temp);
            bindings.FPDF_CloseDocument(document);
            result.map_err(|failure| failure.detail)
        })
        .unwrap();
        (
            std::fs::read(&source).unwrap(),
            std::fs::read(&temp).unwrap(),
        )
    }

    #[test]
    fn repairs_pdfium_incremental_xref_streams_without_moving_offsets() {
        let original = b"%PDF-1.7\n1 0 obj <<>> endobj\nstartxref\n9\n%%EOF\n".to_vec();
        let object_at = original.len();
        let mut output = original.clone();
        output.extend_from_slice(
            b"12 0 obj <</Root 1 0 R /Size 13/Prev 9/W[0 4 1]/Index[1 1]/Length 5>>stream\r\nABCDE\r\nendstream",
        );
        output.extend_from_slice(format!("\r\nstartxref\r\n{object_at}\r\n%%EOF\r\n").as_bytes());

        assert!(repair_incremental_xref_stream(&mut output, original.len()));
        assert!(output.starts_with(&original));
        let (offset, object) = last_xref_object(&output);
        assert_eq!(
            offset, object_at,
            "startxref must still point at the object"
        );
        let text = String::from_utf8_lossy(&object);
        assert!(
            text.starts_with("12 0 obj <</Type/XRef/Root 1 0 R"),
            "{text}"
        );
        assert!(
            text.contains(">>stream\r\nABCDE\r\nendstream\r\nendobj"),
            "{text}"
        );
        // Idempotent, and a classic trailer is left alone.
        assert!(!repair_incremental_xref_stream(&mut output, original.len()));
        let mut classic = original.clone();
        classic.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<</Size 1>>\nstartxref\n{object_at}\n%%EOF\n").as_bytes());
        let before = classic.clone();
        assert!(!repair_incremental_xref_stream(
            &mut classic,
            original.len()
        ));
        assert_eq!(classic, before);
    }

    #[test]
    fn xref_stream_saves_are_repaired_into_well_formed_objects() {
        let (input, output) = rotate_first_page("sanitization/object-xref-stream.pdf", None);
        let (offset, object) = last_xref_object(&output);
        assert!(offset >= input.len());
        let text = String::from_utf8_lossy(&object);
        assert!(text.contains("/Type/XRef"), "{text}");
        assert!(text.trim_end().ends_with("endobj"), "{text}");
    }
}
