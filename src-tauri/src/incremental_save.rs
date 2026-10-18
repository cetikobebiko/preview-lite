//! Artımlı (append-only) PDF kaydı ve doğrulaması
//! (docs/engineering/undo-redo-and-incremental-save-design.md §4.2–§4.4).
//!
//! PDFium'un `FPDF_SaveAsCopy(FPDF_INCREMENTAL)` yolu, onun xref stream kusurunun onarımı ve
//! artımlı çıktının commit'ten önce denetlenmesi burada toplanır. Yalnız job worker'da, session
//! snapshot'ının kopyası üzerinde, `atomic_write_pdf` içinde çağrılır.
//!
//! `find_xref_inconsistency` kullanıcı PDF'ini lopdf ile ayrıştırır. AGENTS.md'nin lopdf
//! sınırları içindedir: yalnız yapısal denetimdir, render veya geometri üretmez, belgeyi
//! değiştirmez ve aynı worker sınırında çalışır. Gerekçe: PDFium onarılmış (tutarsız) bir xref
//! zinciri üzerine artımlı kayıtta `/Prev`'i sessizce düşürür ve zinciri koparır (tasarım §8.1,
//! S4); PDFium bunu bildiren bir API sunmaz.

use crate::document_io::DocumentIoFailure;
use pdfium_render::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::io::Write;
use std::os::raw::{c_int, c_ulong};
use std::path::Path;

pub(crate) const FPDF_INCREMENTAL: FPDF_DWORD = 1;
pub(crate) const FPDF_FULL_REWRITE: FPDF_DWORD = 0;

/// Bir mutasyonun working PDF'i nasıl yazdığı (tasarım §4.4).
#[allow(dead_code)] // Recipe'lere tasarım §10 adım 4'te bağlanır.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WriteMode {
    /// Girdinin baytları aynen korundu, değişiklikler sona eklendi; §4.3 denetimleri geçti.
    Appended,
    /// Artımlı, ancak değişmemiş nesneler de yazıldı (ham PDFium çıktısı, karar K5).
    AppendedUnminimized,
    /// Belge baştan yazıldı; mevcut imzalar geçerliliğini yitirir.
    Rewritten,
}

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

/// Artımlı olduğu iddia edilen çıktının commit'ten önceki denetimi (tasarım §4.3, 1–3):
/// çıktı girdiden uzundur, ilk `input.len()` baytının SHA-256'sı `before_digest`'tir (girdi
/// baytları aynen korunmuş), son `startxref` girdinin ötesindeki yeni bölümü gösterir ve o
/// bölümün `/Prev`'i girdinin son `startxref` değeridir. PDFium ile yeniden açma ve recipe'ye
/// özgü denetim (§4.3, 4) çağıranın işidir.
#[allow(dead_code)] // Recipe'lere tasarım §10 adım 4'te bağlanır.
pub(crate) fn validate_append_only(
    input: &[u8],
    before_digest: &str,
    output: &[u8],
) -> Result<(), DocumentIoFailure> {
    if output.len() <= input.len() {
        return Err(DocumentIoFailure::invariant(format!(
            "Artımlı çıktı ({} bayt) girdiden ({} bayt) uzun değil",
            output.len(),
            input.len()
        )));
    }
    let prefix_digest = format!("{:x}", Sha256::digest(&output[..input.len()]));
    if prefix_digest != before_digest {
        return Err(DocumentIoFailure::invariant(
            "Artımlı çıktı girdinin baytlarıyla başlamıyor",
        ));
    }
    let (_, input_xref) = last_startxref(input)
        .ok_or_else(|| DocumentIoFailure::invariant("Girdide okunabilir startxref yok"))?;
    let (keyword, output_xref) = last_startxref(output)
        .ok_or_else(|| DocumentIoFailure::invariant("Artımlı çıktıda okunabilir startxref yok"))?;
    if output_xref < input.len() || output_xref >= keyword {
        return Err(DocumentIoFailure::invariant(format!(
            "Son startxref ({output_xref}) girdiden sonra eklenen bölümü göstermiyor"
        )));
    }
    let section = &output[output_xref..keyword];
    let prev = section_prev(section)
        .ok_or_else(|| DocumentIoFailure::invariant("Artımlı bölümün trailer'ında /Prev yok"))?;
    if prev != input_xref {
        return Err(DocumentIoFailure::invariant(format!(
            "Artımlı bölümün /Prev değeri ({prev}) girdinin son startxref değeri ({input_xref}) değil"
        )));
    }
    Ok(())
}

/// Belgenin xref zinciri dosyayla tutarlı mı, yani belge onarım gerektirmeden açılabiliyor mu
/// (tasarım §4.4, S4). Tutarlıysa `None`, değilse nedeni. Tutarsız girdi üzerine artımlı kayıt
/// yapılmaz: eklenecek sağlam bir zincir yoktur.
///
/// Denetim: lopdf belgeyi yükleyebilir; son `startxref` bir xref tablosunu (`xref`) ya da bir
/// nesne başlığını gösterir; çözülen her `Normal` xref girdisi kendi `N G obj` başlığını
/// gösterir. Şifreli belgeler parola gerektirmeden denetlenir (lopdf 0.34 yüklerken çözmez).
/// lopdf belgenin tüm nesnelerini belleğe ayrıştırır (yaklaşık bir belge boyutu, tasarım S6).
#[allow(dead_code)] // Recipe'lere tasarım §10 adım 4'te bağlanır.
pub(crate) fn find_xref_inconsistency(bytes: &[u8]) -> Option<String> {
    let document = match lopdf::Document::load_mem(bytes) {
        Ok(document) => document,
        Err(error) => return Some(format!("lopdf xref zincirini okuyamadı: {error}")),
    };
    let Some((_, xref)) = last_startxref(bytes) else {
        return Some("Okunabilir startxref yok".to_string());
    };
    match bytes.get(xref..) {
        Some(rest) if rest.starts_with(b"xref") || rest.first().is_some_and(u8::is_ascii_digit) => {
        }
        _ => return Some(format!("startxref ({xref}) bir xref bölümünü göstermiyor")),
    }
    for (&number, entry) in &document.reference_table.entries {
        if let lopdf::xref::XrefEntry::Normal { offset, generation } = *entry {
            if !object_header_at(bytes, offset as usize, number, u32::from(generation)) {
                return Some(format!(
                    "Nesne {number} {generation} için xref offset'i ({offset}) nesne başlığını göstermiyor"
                ));
            }
        }
    }
    None
}

/// Son `startxref` anahtar kelimesinin konumu ve gösterdiği offset.
fn last_startxref(bytes: &[u8]) -> Option<(usize, usize)> {
    let keyword = find_last(bytes, b"startxref")?;
    let offset = leading_uint(skip_whitespace(&bytes[keyword + b"startxref".len()..]))?.0;
    Some((keyword, usize::try_from(offset).ok()?))
}

/// Bir xref bölümünün (tablo + `trailer` ya da xref stream nesnesi) trailer sözlüğündeki `/Prev`.
fn section_prev(section: &[u8]) -> Option<usize> {
    let dict = if section.starts_with(b"xref") {
        &section[find_last(section, b"trailer")?..]
    } else {
        &section[..find_first(section, b"stream").unwrap_or(section.len())]
    };
    let mut from = 0;
    while let Some(at) = find_first(&dict[from..], b"/Prev") {
        let after = &dict[from + at + b"/Prev".len()..];
        // `/PrevX` gibi daha uzun bir ad değil, tam `/Prev` anahtarı olmalı.
        if after
            .first()
            .is_some_and(|byte| is_pdf_whitespace(*byte) || byte.is_ascii_digit())
        {
            return usize::try_from(leading_uint(skip_whitespace(after))?.0).ok();
        }
        from += at + 1;
    }
    None
}

/// `offset`'te `number generation obj` başlığı var mı (aralarda PDF boşluğu).
fn object_header_at(bytes: &[u8], offset: usize, number: u32, generation: u32) -> bool {
    let Some(rest) = bytes.get(offset..) else {
        return false;
    };
    let Some((found_number, rest)) = leading_uint(rest) else {
        return false;
    };
    let after_number = skip_whitespace(rest);
    if found_number != u64::from(number) || after_number.len() == rest.len() {
        return false;
    }
    let Some((found_generation, rest)) = leading_uint(after_number) else {
        return false;
    };
    let after_generation = skip_whitespace(rest);
    found_generation == u64::from(generation)
        && after_generation.len() < rest.len()
        && after_generation.starts_with(b"obj")
}

fn leading_uint(bytes: &[u8]) -> Option<(u64, &[u8])> {
    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || digits > 19 {
        return None;
    }
    let value = std::str::from_utf8(&bytes[..digits]).ok()?.parse().ok()?;
    Some((value, &bytes[digits..]))
}

fn skip_whitespace(bytes: &[u8]) -> &[u8] {
    let skipped = bytes
        .iter()
        .take_while(|byte| is_pdf_whitespace(**byte))
        .count();
    &bytes[skipped..]
}

/// ISO 32000-1 Tablo 1: NUL, TAB, LF, FF, CR, SP.
fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
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
    use crate::document_io::DocumentIoStage;
    use std::path::PathBuf;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
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
    fn pdfium_incremental_saves_pass_the_append_only_check() {
        for (relative, password) in [
            ("pdf/three-distinct-pages.pdf", None),
            ("sanitization/incremental-revision.pdf", None),
            ("sanitization/object-xref-stream.pdf", None),
            ("sanitization/incremental-xref-stream-chain.pdf", None),
            ("sanitization/hybrid-xref.pdf", None),
            ("sanitization/signature.pdf", None),
            ("pdf/encrypted.pdf", Some("fixture-password")),
        ] {
            let (input, output) = rotate_first_page(relative, password);
            validate_append_only(&input, &digest(&input), &output)
                .unwrap_or_else(|failure| panic!("{relative}: {}", failure.detail));
            assert_eq!(find_xref_inconsistency(&output), None, "{relative}");
        }
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

    fn invariant_detail(input: &[u8], output: &[u8]) -> String {
        let failure = validate_append_only(input, &digest(input), output).unwrap_err();
        assert_eq!(failure.stage, DocumentIoStage::ValidateInvariant);
        failure.detail
    }

    #[test]
    fn append_only_check_rejects_each_broken_invariant() {
        let (input, output) = rotate_first_page("pdf/three-distinct-pages.pdf", None);

        // 1. Çıktı girdiden uzun değil.
        assert!(invariant_detail(&input, &input).contains("uzun değil"));

        // 2. Girdinin bir baytı değişmiş.
        let mut changed = output.clone();
        changed[20] ^= 0x01;
        assert!(invariant_detail(&input, &changed).contains("baytlarıyla başlamıyor"));
        // Yanlış beklenen digest de aynı sonucu verir.
        let failure = validate_append_only(&input, &digest(b"other"), &output).unwrap_err();
        assert!(failure.detail.contains("baytlarıyla başlamıyor"));

        // 3a. Sona eklenen bölüm yeni bir startxref içermiyor.
        let mut no_section = input.clone();
        no_section.extend_from_slice(b"\n% only a comment\n");
        assert!(invariant_detail(&input, &no_section).contains("startxref"));

        // 3b. /Prev girdinin son startxref değeri değil.
        let (_, input_xref) = last_startxref(&input).unwrap();
        let prev = format!("/Prev {input_xref}");
        let at = find_last(&output, prev.as_bytes()).expect("PDFium /Prev yazmalı");
        let mut wrong_prev = output.clone();
        let digit = at + "/Prev ".len();
        wrong_prev[digit] = if wrong_prev[digit] == b'1' {
            b'2'
        } else {
            b'1'
        };
        assert!(invariant_detail(&input, &wrong_prev).contains("/Prev"));

        // 3c. /Prev hiç yok.
        let mut no_prev = output.clone();
        no_prev[at + 1..at + 5].copy_from_slice(b"Xrev");
        assert!(invariant_detail(&input, &no_prev).contains("/Prev yok"));
    }

    #[test]
    fn section_prev_reads_tables_and_streams_and_ignores_longer_names() {
        assert_eq!(
            section_prev(b"xref\n0 1\n0000000000 65535 f \ntrailer\n<</Size 1/Prev 186>>\n"),
            Some(186)
        );
        assert_eq!(
            section_prev(b"7 0 obj\n<</Type/XRef/Prev\n42/W[1 2 1]>>stream\n/Prev 9\nendstream"),
            Some(42)
        );
        assert_eq!(
            section_prev(b"xref\n0 1\ntrailer\n<</PrevX 5/Prev 7>>"),
            Some(7)
        );
        assert_eq!(section_prev(b"xref\n0 1\ntrailer\n<</Size 1>>"), None);
    }

    #[test]
    fn consistent_fixtures_have_no_xref_inconsistency() {
        for relative in [
            "pdf/minimal.pdf",
            "pdf/three-distinct-pages.pdf",
            "pdf/encrypted.pdf",
            "sanitization/incremental-revision.pdf",
            "sanitization/object-xref-stream.pdf",
            "sanitization/incremental-xref-stream-chain.pdf",
            "sanitization/hybrid-xref.pdf",
            "sanitization/signature.pdf",
            "redaction/object-stream.pdf",
        ] {
            let bytes = std::fs::read(fixture(relative)).unwrap();
            assert_eq!(find_xref_inconsistency(&bytes), None, "{relative}");
        }
    }

    #[test]
    fn files_that_need_xref_repair_are_reported() {
        let base = std::fs::read(fixture("pdf/three-distinct-pages.pdf")).unwrap();
        let (keyword, xref) = last_startxref(&base).unwrap();

        let mut shifted = base[..keyword].to_vec();
        shifted.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref + 7).as_bytes());

        let mut wrong_entry = base.clone();
        let entry = find_first(&wrong_entry[xref..], b" 00000 n").unwrap() + xref - 10;
        wrong_entry[entry..entry + 10].copy_from_slice(b"0000000003");

        let mut leading_garbage = b"XYZ".to_vec();
        leading_garbage.extend_from_slice(&base);

        for (name, bytes) in [
            (
                "malformed",
                std::fs::read(fixture("pdf/malformed.pdf")).unwrap(),
            ),
            ("startxref-shifted", shifted),
            ("entry-offset-wrong", wrong_entry),
            ("leading-garbage", leading_garbage),
            ("truncated-tail", base[..keyword].to_vec()),
        ] {
            assert!(find_xref_inconsistency(&bytes).is_some(), "{name}");
        }
    }

    #[test]
    fn object_headers_accept_pdf_whitespace_only_between_tokens() {
        let bytes = b"xx1 0 obj<<>>12  3\r\nobj 4 0obj";
        assert!(object_header_at(bytes, 2, 1, 0));
        assert!(object_header_at(bytes, 13, 12, 3));
        assert!(!object_header_at(bytes, 13, 12, 0));
        assert!(!object_header_at(bytes, 2, 2, 0));
        // `obj` tokenine boşluk olmadan bitişik numara geçerli bir başlık değil.
        assert!(!object_header_at(bytes, 24, 4, 0));
        assert!(!object_header_at(bytes, 999, 1, 0));
    }

    #[test]
    fn write_modes_serialize_to_the_design_names() {
        assert_eq!(
            serde_json::to_string(&[
                WriteMode::Appended,
                WriteMode::AppendedUnminimized,
                WriteMode::Rewritten,
            ])
            .unwrap(),
            r#"["appended","appended-unminimized","rewritten"]"#
        );
    }
}
