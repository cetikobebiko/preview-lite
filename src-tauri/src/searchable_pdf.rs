//! Aranabilir PDF kopyası: taranmış sayfalara görünmez bir OCR metin katmanı eklenir.
//!
//! Yalnız job worker'da, oturum snapshot'ı üzerinde çalışır. Her kelime ayrı bir metin nesnesi
//! olarak `FPDF_TEXTRENDERMODE_INVISIBLE` ile yazılır; sayfanın var olan içerik akışlarına
//! dokunulmaz, PDFium yeni nesneleri ayrı bir akışa yazar. Belge PDFium ile tam yeniden yazılan
//! bir kopya olarak kaydedilir (artımlı kayıt dokunulan her sayfa için bütün dosyayı yeniden
//! ekliyor, karar K5) ve kullanıcı hedefine yayımlanmadan önce worker'da doğrulanır: sayfa sayısı
//! ve boyutları aynı, işlenen her sayfanın render'ı girdiyle piksel piksel aynı, çıkarılan metin
//! eklenen kelimeleri sırasıyla içeriyor.
//!
//! Font: Tesseract'ın glyphless TrueType fontu (`GLYPHLESS_FONT`, Apache-2.0; lisans
//! `src-tauri/licenses/tesseract-glyphless-font.txt`). Tek glifi (GID 1) boştur ve 0.5 em
//! avanslıdır. `FPDFText_LoadCidType2Font` ile CID = Unicode BMP kod noktası olarak yüklenir:
//! CIDToGIDMap her CID'yi GID 1'e, ToUnicode her CID'yi kendisine eşler. Font belge başına bir
//! kez yüklenir ve bütün sayfalarda paylaşılır.

use crate::incremental_save::{save_copy, RawDocument, RawPage, FPDF_FULL_REWRITE};
pub(crate) use crate::page_geometry::PageFrame;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::os::raw::{c_int, c_ulong};
use std::path::Path;

/// `RenderOcrPage` ölçeği; OCR katmanıyla (`OcrLayer`) aynı.
pub(crate) const OCR_RENDER_SCALE: f32 = 2.0;
/// Boşluk olmayan karakter sayısı bunun altındaysa sayfanın metin katmanı yok sayılır
/// (`detect_scanned_pages` ile aynı eşik).
pub(crate) const SCANNED_PAGE_MAX_TEXT_CHARS: usize = 20;
pub(crate) const MAX_SEARCHABLE_PAGES: usize = 2_000;
pub(crate) const MAX_SEARCHABLE_WORDS: usize = 2_000_000;
/// Bir kelimede en fazla karakter; daha uzunu OCR gürültüsüdür ve atlanır.
pub(crate) const MAX_WORD_CHARS: usize = 256;

const FPDF_PAGEOBJ_IMAGE: c_int = 3;
const FPDF_PAGEOBJ_FORM: c_int = 5;
const FPDF_TEXTRENDERMODE_INVISIBLE: c_int = 3;
const MAX_FORM_DEPTH: usize = 8;
/// Bitmap boyutu ile sayfa boyutu arasındaki izinli fark (piksel; yuvarlama).
const BITMAP_SIZE_TOLERANCE: f64 = 1.5;
/// Kapalı formül ile `FPDF_DeviceToPage` arasındaki izinli fark (pt).
const GEOMETRY_TOLERANCE_PT: f64 = 1.0;

/// Tesseract 5.5.2 `tessdata/pdf.ttf` (glyphless font), SHA-256
/// c7845420925a23d88ed830a63957b8af85a66a8daf8d9fc90e843673b2ef1a59.
pub(crate) const GLYPHLESS_FONT: [u8; 572] = [
    0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x80, 0x00, 0x03, 0x00, 0x20, 0x4f, 0x53, 0x2f, 0x32,
    0x56, 0xde, 0xc8, 0x94, 0x00, 0x00, 0x01, 0x28, 0x00, 0x00, 0x00, 0x60, 0x63, 0x6d, 0x61, 0x70,
    0x00, 0x0a, 0x00, 0x34, 0x00, 0x00, 0x01, 0x90, 0x00, 0x00, 0x00, 0x1e, 0x67, 0x6c, 0x79, 0x66,
    0x15, 0x22, 0x41, 0x24, 0x00, 0x00, 0x01, 0xb8, 0x00, 0x00, 0x00, 0x18, 0x68, 0x65, 0x61, 0x64,
    0x0b, 0x78, 0xf1, 0x65, 0x00, 0x00, 0x00, 0xac, 0x00, 0x00, 0x00, 0x36, 0x68, 0x68, 0x65, 0x61,
    0x0c, 0x02, 0x04, 0x02, 0x00, 0x00, 0x00, 0xe4, 0x00, 0x00, 0x00, 0x24, 0x68, 0x6d, 0x74, 0x78,
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x88, 0x00, 0x00, 0x00, 0x08, 0x6c, 0x6f, 0x63, 0x61,
    0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x01, 0xb0, 0x00, 0x00, 0x00, 0x06, 0x6d, 0x61, 0x78, 0x70,
    0x00, 0x04, 0x00, 0x05, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x20, 0x6e, 0x61, 0x6d, 0x65,
    0xf2, 0xeb, 0x16, 0xda, 0x00, 0x00, 0x01, 0xd0, 0x00, 0x00, 0x00, 0x4b, 0x70, 0x6f, 0x73, 0x74,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x1c, 0x00, 0x00, 0x00, 0x20, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0xb0, 0x94, 0x71, 0x10, 0x5f, 0x0f, 0x3c, 0xf5, 0x04, 0x07, 0x08, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xcf, 0x9a, 0xfc, 0x6e, 0x00, 0x00, 0x00, 0x00, 0xd4, 0xc3, 0xa7, 0xf2,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x02, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x08, 0x00, 0xff, 0xff, 0x00, 0x00, 0x04, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x01, 0x90, 0x00, 0x05,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x47, 0x4f, 0x4f, 0x47, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xff, 0xff,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x03, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x14, 0x00, 0x06, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00,
    0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x31, 0x21, 0x11, 0x21, 0x04, 0x00, 0xfc, 0x00, 0x08, 0x00,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x05, 0x00, 0x16,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x0b, 0x00, 0x16, 0x00, 0x03,
    0x00, 0x01, 0x04, 0x09, 0x00, 0x05, 0x00, 0x16, 0x00, 0x00, 0x00, 0x56, 0x00, 0x65, 0x00, 0x72,
    0x00, 0x73, 0x00, 0x69, 0x00, 0x6f, 0x00, 0x6e, 0x00, 0x20, 0x00, 0x31, 0x00, 0x2e, 0x00, 0x30,
    0x56, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x31, 0x2e, 0x30, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchableFailure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl SearchableFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn write(message: impl Into<String>) -> Self {
        Self::new("SEARCHABLE_PDF_WRITE_FAILED", message)
    }

    fn verify(message: impl Into<String>) -> Self {
        Self::new("SEARCHABLE_PDF_VERIFY_FAILED", message)
    }
}

/// Bir sayfanın neden işlenmediği.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Sayfada zaten metin var; ikinci bir metin katmanı eklenmez.
    HasText,
    /// Metin de görüntü de yok (boş ya da yalnız vektör sayfa).
    NoImage,
    /// Render ya da Tesseract bu sayfada başarısız oldu.
    OcrFailed,
    /// Tesseract sayfada kelime bulmadı.
    NoWords,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PagePlan {
    pub page_index: u32,
    /// `None`: sayfa OCR'lanır.
    pub skip: Option<SkipReason>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchablePlan {
    pub page_count: u32,
    /// İstenen sayfalar, istenen sırayla (istek yoksa bütün sayfalar).
    pub pages: Vec<PagePlan>,
    pub signature_count: u32,
}

/// Tesseract'ın bir kelimesi, OCR bitmap'inin piksel koordinatlarında (sol üst köşe başlangıç).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutWord {
    pub(crate) text: String,
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageWords {
    pub(crate) page_index: u32,
    pub(crate) bitmap_width: u32,
    pub(crate) bitmap_height: u32,
    pub(crate) words: Vec<LayoutWord>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchableCopyReport {
    pub pages_written: u32,
    pub words_written: u64,
    /// Boş, sınır dışı ya da metni yazılamayan kelimeler.
    pub words_skipped: u64,
    pub signatures_invalidated: bool,
}

/// İstenen sayfaları sınıflandırır. Şifreli belge reddedilir (mevcut mutasyon ve kopya
/// davranışıyla aynı). Metin tespiti PDFium karakterleriyle yapılır; `PdfPageText::all()`
/// döndürülmüş ya da kırpılmış sayfalarda metin kaçırdığı için kullanılmaz.
pub(crate) fn plan(
    bindings: &dyn PdfiumLibraryBindings,
    snapshot: &Path,
    credential: Option<&str>,
    pages: Option<&[u32]>,
) -> Result<SearchablePlan, SearchableFailure> {
    let document = open_unprotected(bindings, snapshot, credential)?;
    let page_count = u32::try_from(bindings.FPDF_GetPageCount(document.handle)).map_err(|_| {
        SearchableFailure::new("SESSION_JOB_PDF_OPEN_FAILED", "Sayfa sayısı okunamadı")
    })?;
    let requested = requested_pages(pages, page_count)?;
    let mut plans = Vec::with_capacity(requested.len());
    for page_index in requested {
        let page = RawPage::load(&document, page_index).map_err(|failure| {
            SearchableFailure::new("SEARCHABLE_PDF_INVALID_PAGES", failure.detail)
        })?;
        let skip = if text_char_count(bindings, page.handle) >= SCANNED_PAGE_MAX_TEXT_CHARS {
            Some(SkipReason::HasText)
        } else if !has_image(bindings, page.handle) {
            Some(SkipReason::NoImage)
        } else {
            None
        };
        plans.push(PagePlan { page_index, skip });
    }
    // CAP-01 canonical discovery (Prompt 11 G2); too many signatures still warns.
    let signature_count =
        crate::signature_verification::discovery::discover_at_path(bindings, snapshot, credential)
            .map_or(u32::MAX, |discovery| discovery.signature_count());
    Ok(SearchablePlan {
        page_count,
        pages: plans,
        signature_count,
    })
}

fn open_unprotected<'b>(
    bindings: &'b dyn PdfiumLibraryBindings,
    snapshot: &Path,
    credential: Option<&str>,
) -> Result<RawDocument<'b>, SearchableFailure> {
    let path = snapshot.to_str().ok_or_else(|| {
        SearchableFailure::new("SESSION_JOB_PDF_OPEN_FAILED", "PDF yolu UTF-8 değil")
    })?;
    let document = RawDocument::open(bindings, path, credential).ok_or_else(|| {
        SearchableFailure::new(
            "SESSION_JOB_PDF_OPEN_FAILED",
            format!(
                "PDF açılamadı (PDFium hata kodu {})",
                bindings.FPDF_GetLastError()
            ),
        )
    })?;
    if bindings.FPDF_GetSecurityHandlerRevision(document.handle) != -1 {
        // Düz kopya korumayı düşürür; yeniden korumak sahibin ayarlarını gerektirir.
        return Err(SearchableFailure::new(
            "SEARCHABLE_PDF_ENCRYPTED",
            "Parola korumalı belgelerin aranabilir kopyası kaydedilemez; önce Güvenlik ile korumasız bir kopya kaydedip onu kullanın",
        ));
    }
    Ok(document)
}

/// İstek yoksa bütün sayfalar. Boş, yinelenen ya da sınır dışı sayfa reddedilir.
pub(crate) fn requested_pages(
    pages: Option<&[u32]>,
    page_count: u32,
) -> Result<Vec<u32>, SearchableFailure> {
    let invalid = |message: String| SearchableFailure::new("SEARCHABLE_PDF_INVALID_PAGES", message);
    let requested: Vec<u32> = match pages {
        None => (0..page_count).collect(),
        Some(pages) => {
            if pages.is_empty() {
                return Err(invalid("Sayfa seçimi boş".into()));
            }
            let mut seen = std::collections::HashSet::with_capacity(pages.len());
            for &page in pages {
                if page >= page_count {
                    return Err(invalid(format!(
                        "Sayfa {} belgede yok (sayfa sayısı {page_count})",
                        u64::from(page) + 1
                    )));
                }
                if !seen.insert(page) {
                    return Err(invalid(format!(
                        "Sayfa {} birden fazla kez seçildi",
                        u64::from(page) + 1
                    )));
                }
            }
            pages.to_vec()
        }
    };
    if requested.len() > crate::job_worker::MAX_JOB_PAGES {
        return Err(invalid(format!(
            "En fazla {} sayfa işlenebilir",
            crate::job_worker::MAX_JOB_PAGES
        )));
    }
    Ok(requested)
}

/// Sayfadaki boşluk olmayan karakterler (PDFium'un ürettiği boşluk ve satır sonları dahil değil).
fn text_char_count(bindings: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> usize {
    let text = bindings.FPDFText_LoadPage(page);
    if text.is_null() {
        return 0;
    }
    let count = bindings.FPDFText_CountChars(text).max(0);
    let visible = (0..count)
        .filter(|&index| {
            char::from_u32(bindings.FPDFText_GetUnicode(text, index))
                .is_some_and(|c| c != '\0' && !c.is_whitespace())
        })
        .count();
    bindings.FPDFText_ClosePage(text);
    visible
}

/// Sayfada (form XObject'ler içinde de) en az bir görüntü nesnesi var mı.
fn has_image(bindings: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> bool {
    let count = bindings.FPDFPage_CountObjects(page).max(0);
    (0..count).any(|index| object_has_image(bindings, bindings.FPDFPage_GetObject(page, index), 0))
}

fn object_has_image(
    bindings: &dyn PdfiumLibraryBindings,
    object: FPDF_PAGEOBJECT,
    depth: usize,
) -> bool {
    if object.is_null() {
        return false;
    }
    match bindings.FPDFPageObj_GetType(object) {
        FPDF_PAGEOBJ_IMAGE => true,
        FPDF_PAGEOBJ_FORM if depth < MAX_FORM_DEPTH => {
            let count = bindings.FPDFFormObj_CountObjects(object).max(0);
            (0..count).any(|index| {
                object_has_image(
                    bindings,
                    bindings.FPDFFormObj_GetObject(object, index as c_ulong),
                    depth + 1,
                )
            })
        }
        _ => false,
    }
}

/// Font metrikleri, font boyutu 1 için (em).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FontMetrics {
    pub(crate) ascent: f64,
    pub(crate) descent: f64,
    /// Her karakterin avansı (bütün CID'ler aynı glife eşlendiği için sabit).
    pub(crate) advance: f64,
}

/// Bir kelimenin metin nesnesi matrisi `[a, b, c, d, e, f]` (font boyutu 1 ile oluşturulan
/// nesneye uygulanır). Kelimenin loose kutusu (ascent/descent ve avans) OCR kutusuna oturur:
/// taban çizgisi kutunun altından `-descent · fs` yukarıda, yatay ölçek kelime genişliğine göre.
/// `chars` kelimenin sondaki boşluk hariç karakter sayısıdır. Kutu boşsa ya da bitmap dışına
/// taşıyorsa `None`.
pub(crate) fn word_matrix(
    frame: &PageFrame,
    bitmap: (u32, u32),
    word: &LayoutWord,
    metrics: &FontMetrics,
    chars: usize,
) -> Option<[f64; 6]> {
    let (bitmap_width, bitmap_height) = (f64::from(bitmap.0), f64::from(bitmap.1));
    let (left, top) = (f64::from(word.left), f64::from(word.top));
    let (right, bottom) = (left + f64::from(word.width), top + f64::from(word.height));
    if word.width <= 0
        || word.height <= 0
        || chars == 0
        || left < 0.0
        || top < 0.0
        || right > bitmap_width
        || bottom > bitmap_height
    {
        return None;
    }
    let (display_width, display_height) = frame.display_size();
    let (sx, sy) = (display_width / bitmap_width, display_height / bitmap_height);
    let (l, r, t, b) = (left * sx, right * sx, top * sy, bottom * sy);
    let width = r - l;
    let height = b - t;
    let origin = frame.to_user(l, b);
    let end = frame.to_user(r, b);
    let up = frame.to_user(l, t);
    let along = ((end.0 - origin.0) / width, (end.1 - origin.1) / width);
    let normal = ((up.0 - origin.0) / height, (up.1 - origin.1) / height);
    let font_size = height / (metrics.ascent - metrics.descent);
    let natural = metrics.advance * chars as f64 * font_size;
    let scale = width / natural;
    let lift = -metrics.descent * font_size;
    let matrix = [
        along.0 * scale * font_size,
        along.1 * scale * font_size,
        normal.0 * font_size,
        normal.1 * font_size,
        origin.0 + normal.0 * lift,
        origin.1 + normal.1 * lift,
    ];
    matrix
        .iter()
        .all(|value| value.is_finite())
        .then_some(matrix)
}

/// Kelimeden yazılacak metin: kontrol ve BMP dışı karakterler atılır (font yalnız BMP'yi
/// eşler). Boş ya da `MAX_WORD_CHARS`'tan uzunsa `None`.
pub(crate) fn writable_text(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace() && u32::from(*c) <= 0xFFFF)
        .filter(|c| !(0xD800..=0xDFFF).contains(&u32::from(*c)))
        .collect();
    let count = cleaned.chars().count();
    (count > 0 && count <= MAX_WORD_CHARS).then_some(cleaned)
}

/// ToUnicode CMap: CID = Unicode BMP kod noktası. PDFium bir `bfrange`'i yalnız son baytı
/// değişen aralık olarak kabul ettiği için (tek `<0000> <FFFF>` aralığında yalnız Latin-1
/// eşlenir) aralık 256 parçaya, blok başına en fazla 100 girişle bölünür.
pub(crate) fn to_unicode_cmap() -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let highs: Vec<u32> = (0..=0xFF).collect();
    for block in highs.chunks(100) {
        cmap.push_str(&format!("{} beginbfrange\n", block.len()));
        for high in block {
            cmap.push_str(&format!("<{high:02X}00> <{high:02X}FF> <{high:02X}00>\n"));
        }
        cmap.push_str("endbfrange\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

/// CIDToGIDMap: her BMP CID'si glyphless fontun boş glifine (GID 1).
fn cid_to_gid_map() -> Vec<u8> {
    std::iter::repeat_n([0u8, 1u8], 0x1_0000)
        .flatten()
        .collect()
}

/// Belgeye yüklenmiş glyphless font; düşerken kapanır.
struct LoadedFont<'a> {
    bindings: &'a dyn PdfiumLibraryBindings,
    handle: FPDF_FONT,
    metrics: FontMetrics,
}

impl<'a> LoadedFont<'a> {
    fn load(document: &RawDocument<'a>) -> Result<Self, SearchableFailure> {
        let bindings = document.bindings;
        let cmap = to_unicode_cmap();
        let gid_map = cid_to_gid_map();
        let handle = bindings.FPDFText_LoadCidType2Font(
            document.handle,
            GLYPHLESS_FONT.as_ptr(),
            GLYPHLESS_FONT.len() as u32,
            &cmap,
            gid_map.as_ptr(),
            gid_map.len() as u32,
        );
        if handle.is_null() {
            return Err(SearchableFailure::write("Metin katmanı fontu yüklenemedi"));
        }
        let mut font = Self {
            bindings,
            handle,
            metrics: FontMetrics {
                ascent: 0.0,
                descent: 0.0,
                advance: 0.0,
            },
        };
        let (mut ascent, mut descent, mut advance) = (0f32, 0f32, 0f32);
        let ok = bindings.is_true(bindings.FPDFFont_GetAscent(handle, 1.0, &mut ascent))
            && bindings.is_true(bindings.FPDFFont_GetDescent(handle, 1.0, &mut descent))
            && bindings.is_true(bindings.FPDFFont_GetGlyphWidth(handle, 1, 1.0, &mut advance));
        if !ok || advance <= 0.0 || ascent <= descent {
            return Err(SearchableFailure::write(
                "Metin katmanı fontunun metrikleri okunamadı",
            ));
        }
        font.metrics = FontMetrics {
            ascent: f64::from(ascent),
            descent: f64::from(descent),
            advance: f64::from(advance),
        };
        Ok(font)
    }
}

impl Drop for LoadedFont<'_> {
    fn drop(&mut self) {
        self.bindings.FPDFFont_Close(self.handle);
    }
}

/// Sayfanın PDFium'un render ettiği çerçevesi. Kapalı formül, PDFium'un sayfa boyutuyla ve
/// `FPDF_DeviceToPage` ile çapraz denetlenir; uyuşmazsa sayfaya yazılmaz.
fn page_frame(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    bitmap: (u32, u32),
) -> Result<PageFrame, SearchableFailure> {
    let mismatch = || {
        SearchableFailure::new(
            "SEARCHABLE_PDF_GEOMETRY_MISMATCH",
            "Sayfa geometrisi PDFium'un render geometrisiyle uyuşmuyor",
        )
    };
    let frame = PageFrame::of_page(bindings, page).ok_or_else(mismatch)?;
    let (display_width, display_height) = frame.display_size();
    let (page_width, page_height) = (display_width, display_height);
    let (bitmap_width, bitmap_height) = (f64::from(bitmap.0), f64::from(bitmap.1));
    let expected_width = (page_width * f64::from(OCR_RENDER_SCALE)).round();
    if (bitmap_width - expected_width).abs() > 0.0
        || (bitmap_height - bitmap_width * page_height / page_width).abs() > BITMAP_SIZE_TOLERANCE
    {
        return Err(SearchableFailure::new(
            "SEARCHABLE_PDF_GEOMETRY_MISMATCH",
            "OCR görüntüsünün boyutu sayfanın render boyutuyla uyuşmuyor",
        ));
    }
    for (x, y) in [
        (0, 0),
        (bitmap.0 as c_int - 1, bitmap.1 as c_int - 1),
        (bitmap.0 as c_int / 3, bitmap.1 as c_int / 2),
    ] {
        let (mut page_x, mut page_y) = (0f64, 0f64);
        if !bindings.is_true(bindings.FPDF_DeviceToPage(
            page,
            0,
            0,
            bitmap.0 as c_int,
            bitmap.1 as c_int,
            0,
            x,
            y,
            &mut page_x,
            &mut page_y,
        )) {
            return Err(mismatch());
        }
        let (u, v) = (
            f64::from(x) * display_width / bitmap_width,
            f64::from(y) * display_height / bitmap_height,
        );
        let (user_x, user_y) = frame.to_user(u, v);
        if (user_x - page_x).abs() > GEOMETRY_TOLERANCE_PT
            || (user_y - page_y).abs() > GEOMETRY_TOLERANCE_PT
        {
            return Err(mismatch());
        }
    }
    Ok(frame)
}

/// Kelimeleri yazar, kopyayı `output`'a kaydeder ve doğrular. Doğrulama geçmezse `output`
/// silinir ve hata döner.
pub(crate) fn write_searchable_copy(
    pdfium: &Pdfium,
    snapshot: &Path,
    credential: Option<&str>,
    pages: &[PageWords],
    output: &Path,
) -> Result<SearchableCopyReport, SearchableFailure> {
    let result = write_unverified(pdfium.bindings(), snapshot, credential, pages, output).and_then(
        |(report, expected)| {
            verify_copy(pdfium, snapshot, credential, output, &expected).map(|()| report)
        },
    );
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

/// Doğrulama için: işlenen sayfa ve o sayfaya eklenen metin (boşluksuz).
struct ExpectedPage {
    page_index: u32,
    added: String,
}

fn write_unverified(
    bindings: &dyn PdfiumLibraryBindings,
    snapshot: &Path,
    credential: Option<&str>,
    pages: &[PageWords],
    output: &Path,
) -> Result<(SearchableCopyReport, Vec<ExpectedPage>), SearchableFailure> {
    let document = open_unprotected(bindings, snapshot, credential)?;
    let font = LoadedFont::load(&document)?;
    let mut report = SearchableCopyReport {
        signatures_invalidated: crate::signature_verification::discovery::copy_loses_signatures(
            bindings, snapshot, credential,
        ),
        ..SearchableCopyReport::default()
    };
    let mut expected = Vec::with_capacity(pages.len());
    for page_words in pages {
        let page = RawPage::load(&document, page_words.page_index)
            .map_err(|failure| SearchableFailure::write(failure.detail))?;
        let bitmap = (page_words.bitmap_width, page_words.bitmap_height);
        let frame = page_frame(bindings, page.handle, bitmap)?;
        let mut added = String::new();
        for word in &page_words.words {
            let Some(text) = writable_text(&word.text) else {
                report.words_skipped += 1;
                continue;
            };
            let Some(matrix) =
                word_matrix(&frame, bitmap, word, &font.metrics, text.chars().count())
            else {
                report.words_skipped += 1;
                continue;
            };
            insert_word(&document, &page, &font, &format!("{text} "), matrix)?;
            added.push_str(&text);
            report.words_written += 1;
        }
        if !bindings.is_true(bindings.FPDFPage_GenerateContent(page.handle)) {
            return Err(SearchableFailure::write(format!(
                "Sayfa {} içeriği yazılamadı",
                u64::from(page_words.page_index) + 1
            )));
        }
        report.pages_written += 1;
        expected.push(ExpectedPage {
            page_index: page_words.page_index,
            added,
        });
    }
    save_copy(bindings, document.handle, output, FPDF_FULL_REWRITE)
        .map_err(|failure| SearchableFailure::write(failure.detail))?;
    drop(font);
    Ok((report, expected))
}

fn insert_word(
    document: &RawDocument<'_>,
    page: &RawPage<'_>,
    font: &LoadedFont<'_>,
    text: &str,
    [a, b, c, d, e, f]: [f64; 6],
) -> Result<(), SearchableFailure> {
    let bindings = document.bindings;
    let object = bindings.FPDFPageObj_CreateTextObj(document.handle, font.handle, 1.0);
    if object.is_null() {
        return Err(SearchableFailure::write("Metin nesnesi oluşturulamadı"));
    }
    if !bindings.is_true(bindings.FPDFText_SetText_str(object, text))
        || !bindings.is_true(
            bindings.FPDFTextObj_SetTextRenderMode(object, FPDF_TEXTRENDERMODE_INVISIBLE as _),
        )
    {
        bindings.FPDFPageObj_Destroy(object);
        return Err(SearchableFailure::write("Metin nesnesine metin yazılamadı"));
    }
    bindings.FPDFPageObj_Transform(object, a, b, c, d, e, f);
    // Nesnenin sahipliği sayfaya geçer.
    bindings.FPDFPage_InsertObject(page.handle, object);
    Ok(())
}

/// Sayfanın PDFium karakterleri, boşluklar atılarak.
fn squashed_text(page: &PdfPage<'_>) -> String {
    page.text()
        .map(|text| {
            text.chars()
                .iter()
                .filter_map(|c| c.unicode_char())
                .filter(|c| *c != '\0' && !c.is_whitespace())
                .collect()
        })
        .unwrap_or_default()
}

fn verify_copy(
    pdfium: &Pdfium,
    snapshot: &Path,
    credential: Option<&str>,
    output: &Path,
    expected: &[ExpectedPage],
) -> Result<(), SearchableFailure> {
    let reopen_failed =
        |message: String| SearchableFailure::verify(format!("Belge yeniden açılamadı: {message}"));
    let source =
        crate::pdf_engine::open_pdfium_doc(pdfium, &snapshot.to_string_lossy(), credential)
            .map_err(reopen_failed)?;
    // Kopya şifresizdir: girdi şifreli olsaydı `open_unprotected` reddederdi.
    let copy = crate::pdf_engine::open_pdfium_doc(pdfium, &output.to_string_lossy(), None)
        .map_err(reopen_failed)?;
    if source.pages().len() != copy.pages().len() {
        return Err(SearchableFailure::verify(
            "Kopyanın sayfa sayısı girdiyle aynı değil",
        ));
    }
    for (index, (source_page, copy_page)) in
        source.pages().iter().zip(copy.pages().iter()).enumerate()
    {
        if source_page.width() != copy_page.width() || source_page.height() != copy_page.height() {
            return Err(SearchableFailure::verify(format!(
                "Sayfa {} boyutu değişti",
                index + 1
            )));
        }
    }
    for page in expected {
        let label = u64::from(page.page_index) + 1;
        let index = u16::try_from(page.page_index)
            .map_err(|_| SearchableFailure::verify("Sayfa numarası sınır dışında"))?;
        let source_page = source
            .pages()
            .get(index)
            .map_err(|_| SearchableFailure::verify("Sayfa bulunamadı"))?;
        let copy_page = copy
            .pages()
            .get(index)
            .map_err(|_| SearchableFailure::verify("Sayfa bulunamadı"))?;
        let config = PdfRenderConfig::new()
            .set_target_width(source_page.width().value.round().max(1.0) as i32);
        let render = |page: &PdfPage<'_>| {
            page.render_with_config(&config)
                .map(|bitmap| bitmap.as_rgba_bytes())
                .map_err(|_| SearchableFailure::verify(format!("Sayfa {label} render edilemedi")))
        };
        if render(&source_page)? != render(&copy_page)? {
            return Err(SearchableFailure::verify(format!(
                "Sayfa {label} görsel olarak değişti"
            )));
        }
        let wanted = format!("{}{}", squashed_text(&source_page), page.added);
        if squashed_text(&copy_page) != wanted {
            return Err(SearchableFailure::verify(format!(
                "Sayfa {label} metin katmanı eklenen kelimelerle uyuşmuyor"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    const SCAN_PAGES: [u32; 5] = [0, 1, 2, 3, 4];

    pub(crate) fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name)
    }

    /// A fixture word and its box in display points `[left, top, right, bottom]`.
    pub(crate) type FixtureWord = (String, [f64; 4]);

    /// `scanned-turkish.words.json`: per scanned page, words with display-point boxes.
    pub(crate) fn fixture_words() -> Vec<(u32, Vec<FixtureWord>)> {
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture("scanned-turkish.words.json")).unwrap())
                .unwrap();
        json["scannedPages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|page| {
                let words = page["words"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|word| {
                        let b: Vec<f64> = word["box"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .collect();
                        (
                            word["text"].as_str().unwrap().to_string(),
                            [b[0], b[1], b[2], b[3]],
                        )
                    })
                    .collect();
                (page["pageIndex"].as_u64().unwrap() as u32, words)
            })
            .collect()
    }

    /// Bitmap size `RenderOcrPage` produces for the page at `OCR_RENDER_SCALE`.
    fn ocr_bitmap_size(page: &PdfPage<'_>) -> (u32, u32) {
        let width = (page.width().value * OCR_RENDER_SCALE).round() as i32;
        let bitmap = page
            .render_with_config(&PdfRenderConfig::new().set_target_width(width))
            .unwrap();
        (bitmap.width() as u32, bitmap.height() as u32)
    }

    /// The fixture's words as OCR output for `page`: display points → bitmap pixels.
    fn page_words(page_index: u32, bitmap: (u32, u32), display: (f64, f64)) -> PageWords {
        let words = fixture_words()
            .into_iter()
            .find(|(index, _)| *index == page_index)
            .unwrap()
            .1;
        let (sx, sy) = (
            f64::from(bitmap.0) / display.0,
            f64::from(bitmap.1) / display.1,
        );
        PageWords {
            page_index,
            bitmap_width: bitmap.0,
            bitmap_height: bitmap.1,
            words: words
                .into_iter()
                .map(|(text, [l, t, r, b])| LayoutWord {
                    text,
                    left: (l * sx).floor() as i32,
                    top: (t * sy).floor() as i32,
                    width: ((r - l) * sx).ceil() as i32,
                    height: ((b - t) * sy).ceil() as i32,
                })
                .collect(),
        }
    }

    pub(crate) fn fixture_page_words(pdfium: &Pdfium) -> Vec<PageWords> {
        let document = pdfium
            .load_pdf_from_file(&fixture("scanned-turkish.pdf"), None)
            .unwrap();
        SCAN_PAGES
            .iter()
            .map(|&index| {
                let page = document.pages().get(index as u16).unwrap();
                let display = (
                    f64::from(page.width().value),
                    f64::from(page.height().value),
                );
                page_words(index, ocr_bitmap_size(&page), display)
            })
            .collect()
    }

    fn squash(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    const METRICS: FontMetrics = FontMetrics {
        ascent: 1.0,
        descent: 0.0,
        advance: 0.5,
    };

    fn apply(m: [f64; 6], x: f64, y: f64) -> (f64, f64) {
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn glyphless_font_is_tesseracts_pdf_ttf() {
        assert_eq!(
            crate::hex_lower(Sha256::digest(GLYPHLESS_FONT)),
            "c7845420925a23d88ed830a63957b8af85a66a8daf8d9fc90e843673b2ef1a59"
        );
    }

    #[test]
    fn frame_maps_display_corners_for_every_rotation_and_crop_offset() {
        // bbox 100 x 200 pt with an origin offset (crop box not at 0,0).
        let frame = |rotation| PageFrame {
            rotation,
            bbox: [10.0, 20.0, 110.0, 220.0],
        };
        assert_eq!(frame(0).display_size(), (100.0, 200.0));
        assert_eq!(frame(1).display_size(), (200.0, 100.0));
        assert_eq!(frame(2).display_size(), (100.0, 200.0));
        assert_eq!(frame(3).display_size(), (200.0, 100.0));
        // Display top-left, top-right and bottom-left corners.
        let cases = [
            (0, [(10.0, 220.0), (110.0, 220.0), (10.0, 20.0)]),
            (1, [(10.0, 20.0), (10.0, 220.0), (110.0, 20.0)]),
            (2, [(110.0, 20.0), (10.0, 20.0), (110.0, 220.0)]),
            (3, [(110.0, 220.0), (110.0, 20.0), (10.0, 220.0)]),
        ];
        for (rotation, [top_left, top_right, bottom_left]) in cases {
            let frame = frame(rotation);
            let (w, h) = frame.display_size();
            assert_eq!(frame.to_user(0.0, 0.0), top_left, "rotation {rotation}");
            assert_eq!(frame.to_user(w, 0.0), top_right, "rotation {rotation}");
            assert_eq!(frame.to_user(0.0, h), bottom_left, "rotation {rotation}");
        }
    }

    #[test]
    fn word_matrix_fits_the_loose_box_to_the_ocr_box() {
        // 200 x 400 px bitmap of a 100 x 200 pt page: 2 px per point.
        let word = LayoutWord {
            text: "abcd".into(),
            left: 20,
            top: 40,
            width: 80,
            height: 20,
        };
        for rotation in 0..4u8 {
            let frame = PageFrame {
                rotation,
                bbox: [10.0, 20.0, 110.0, 220.0],
            };
            let (w, h) = frame.display_size();
            let bitmap = ((w * 2.0) as u32, (h * 2.0) as u32);
            let m = word_matrix(&frame, bitmap, &word, &METRICS, 4).unwrap();
            // Display box: left 10, top 20, right 50, bottom 30 (pt).
            let natural = METRICS.advance * 4.0;
            assert!(
                close(apply(m, 0.0, 0.0), frame.to_user(10.0, 30.0)),
                "baseline origin r{rotation}"
            );
            assert!(
                close(apply(m, natural, 0.0), frame.to_user(50.0, 30.0)),
                "baseline end r{rotation}"
            );
            assert!(
                close(apply(m, 0.0, METRICS.ascent), frame.to_user(10.0, 20.0)),
                "ascent r{rotation}"
            );
        }
    }

    #[test]
    fn word_matrix_lifts_the_baseline_by_the_descent() {
        let metrics = FontMetrics {
            ascent: 0.8,
            descent: -0.2,
            advance: 0.5,
        };
        let frame = PageFrame {
            rotation: 0,
            bbox: [0.0, 0.0, 100.0, 100.0],
        };
        let word = LayoutWord {
            text: "ab".into(),
            left: 0,
            top: 0,
            width: 10,
            height: 10,
        };
        let m = word_matrix(&frame, (100, 100), &word, &metrics, 2).unwrap();
        // Box bottom y = 90; font size 10 → baseline 2 pt above the bottom.
        assert!(close(apply(m, 0.0, 0.0), (0.0, 92.0)));
        assert!(close(apply(m, 0.0, -0.2), (0.0, 90.0)));
        assert!(close(apply(m, 0.0, 0.8), (0.0, 100.0)));
    }

    #[test]
    fn word_matrix_rejects_empty_and_out_of_bitmap_boxes() {
        let frame = PageFrame {
            rotation: 0,
            bbox: [0.0, 0.0, 100.0, 100.0],
        };
        let word = |left, top, width, height| LayoutWord {
            text: "a".into(),
            left,
            top,
            width,
            height,
        };
        for bad in [
            word(0, 0, 0, 5),
            word(0, 0, 5, 0),
            word(-1, 0, 5, 5),
            word(0, -1, 5, 5),
            word(96, 0, 5, 5),
            word(0, 96, 5, 5),
        ] {
            assert_eq!(
                word_matrix(&frame, (100, 100), &bad, &METRICS, 1),
                None,
                "{bad:?}"
            );
        }
        assert_eq!(
            word_matrix(&frame, (100, 100), &word(0, 0, 5, 5), &METRICS, 0),
            None
        );
        assert!(word_matrix(&frame, (100, 100), &word(95, 95, 5, 5), &METRICS, 1).is_some());
    }

    #[test]
    fn writable_text_keeps_turkish_and_drops_controls_and_non_bmp() {
        assert_eq!(writable_text("çğıİöşü").as_deref(), Some("çğıİöşü"));
        assert_eq!(writable_text("a\u{0}b\tc").as_deref(), Some("abc"));
        assert_eq!(writable_text("x\u{1F600}y").as_deref(), Some("xy"));
        assert_eq!(writable_text(" \u{7} "), None);
        assert!(writable_text(&"a".repeat(MAX_WORD_CHARS)).is_some());
        assert_eq!(writable_text(&"a".repeat(MAX_WORD_CHARS + 1)), None);
    }

    #[test]
    fn to_unicode_cmap_maps_the_bmp_in_last_byte_ranges() {
        let cmap = to_unicode_cmap();
        let ranges: Vec<&str> = cmap
            .lines()
            .filter(|line| line.matches('<').count() == 3)
            .collect();
        assert_eq!(ranges.len(), 256);
        assert!(ranges.contains(&"<0100> <01FF> <0100>"), "ğ ı İ ş");
        assert!(ranges.contains(&"<FF00> <FFFF> <FF00>"));
        assert!(
            !cmap.contains("<0000> <FFFF> <0000>"),
            "a range must not cross the high byte"
        );
        let blocks: Vec<usize> = cmap
            .lines()
            .filter_map(|line| line.strip_suffix(" beginbfrange"))
            .map(|count| count.parse().unwrap())
            .collect();
        assert_eq!(blocks, vec![100, 100, 56]);
        assert_eq!(cid_to_gid_map().len(), 0x2_0000);
        assert!(cid_to_gid_map().chunks(2).all(|gid| gid == [0, 1]));
    }

    #[test]
    fn requested_pages_validates_the_selection() {
        assert_eq!(requested_pages(None, 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(requested_pages(Some(&[2, 0]), 3).unwrap(), vec![2, 0]);
        for bad in [&[][..], &[3][..], &[1, 1][..]] {
            assert_eq!(
                requested_pages(Some(bad), 3).unwrap_err().code,
                "SEARCHABLE_PDF_INVALID_PAGES",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn plan_classifies_scanned_text_and_blank_pages() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let all = plan(
                pdfium.bindings(),
                &fixture("scanned-turkish.pdf"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(all.page_count, 7);
            assert_eq!(all.signature_count, 0);
            let skips: Vec<Option<SkipReason>> = all.pages.iter().map(|page| page.skip).collect();
            assert_eq!(
                skips,
                vec![
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(SkipReason::HasText),
                    Some(SkipReason::NoImage)
                ]
            );
            let some = plan(
                pdfium.bindings(),
                &fixture("scanned-turkish.pdf"),
                None,
                Some(&[6, 3]),
            )
            .unwrap();
            assert_eq!(
                some.pages
                    .iter()
                    .map(|page| page.page_index)
                    .collect::<Vec<_>>(),
                vec![6, 3]
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn plan_and_write_reject_encrypted_documents() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let encrypted = fixture("encrypted.pdf");
            let failure = plan(
                pdfium.bindings(),
                &encrypted,
                Some("fixture-password"),
                None,
            )
            .unwrap_err();
            assert_eq!(failure.code, "SEARCHABLE_PDF_ENCRYPTED");
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("out.pdf");
            let failure =
                write_searchable_copy(pdfium, &encrypted, Some("fixture-password"), &[], &output)
                    .unwrap_err();
            assert_eq!(failure.code, "SEARCHABLE_PDF_ENCRYPTED");
            assert!(!output.exists());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn page_frame_matches_pdfium_on_every_fixture_page() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let bindings = pdfium.bindings();
            let document = pdfium
                .load_pdf_from_file(&fixture("scanned-turkish.pdf"), None)
                .unwrap();
            let raw = RawDocument::open(
                bindings,
                fixture("scanned-turkish.pdf").to_str().unwrap(),
                None,
            )
            .unwrap();
            let expected_rotation = [0u8, 1, 2, 3, 0, 0, 0];
            for (index, rotation) in expected_rotation.into_iter().enumerate() {
                let bitmap = ocr_bitmap_size(&document.pages().get(index as u16).unwrap());
                let page = RawPage::load(&raw, index as u32).unwrap();
                let frame = page_frame(bindings, page.handle, bitmap).unwrap();
                assert_eq!(frame.rotation, rotation, "page {index}");
                if index == 4 {
                    assert_eq!(frame.bbox, [36.0, 30.0, 561.0, 788.0]);
                }
                // A bitmap of another size is refused.
                let wrong =
                    page_frame(bindings, page.handle, (bitmap.0 + 3, bitmap.1)).unwrap_err();
                assert_eq!(wrong.code, "SEARCHABLE_PDF_GEOMETRY_MISMATCH");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn copy_has_invisible_turkish_text_where_the_scanned_words_are() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("searchable.pdf");
            let source = fixture("scanned-turkish.pdf");
            let pages = fixture_page_words(pdfium);
            let report = write_searchable_copy(pdfium, &source, None, &pages, &output).unwrap();
            let total: usize = pages.iter().map(|page| page.words.len()).sum();
            assert_eq!(report.pages_written, 5);
            assert_eq!(report.words_written, total as u64);
            assert_eq!(report.words_skipped, 0);
            assert!(!report.signatures_invalidated);

            let original = pdfium.load_pdf_from_file(&source, None).unwrap();
            let copy = pdfium.load_pdf_from_file(&output, None).unwrap();
            assert_eq!(copy.pages().len(), 7);
            let words = fixture_words();
            for (page_index, page_words) in &words {
                let label = format!("page {page_index}");
                let source_page = original.pages().get(*page_index as u16).unwrap();
                let page = copy.pages().get(*page_index as u16).unwrap();
                assert_eq!(
                    (page.width(), page.height()),
                    (source_page.width(), source_page.height()),
                    "{label}"
                );

                // Text: every word, Turkish characters included, in order.
                let text = page.text().unwrap();
                let extracted: String = text
                    .chars()
                    .iter()
                    .filter_map(|c| c.unicode_char())
                    .collect();
                let expected: String = page_words.iter().map(|(word, _)| word.as_str()).collect();
                assert_eq!(squash(&extracted), squash(&expected), "{label}");
                assert!(
                    extracted.contains("Çalışma")
                        && extracted.contains("Muğla")
                        && extracted.contains("öğüt"),
                    "{label}"
                );

                // Pixels: the text layer is invisible.
                let scale_width = (page.width().value * OCR_RENDER_SCALE).round() as i32;
                let config = PdfRenderConfig::new().set_target_width(scale_width);
                let rendered = page.render_with_config(&config).unwrap();
                assert_eq!(
                    rendered.as_rgba_bytes(),
                    source_page
                        .render_with_config(&config)
                        .unwrap()
                        .as_rgba_bytes(),
                    "{label}"
                );

                // Geometry: each word's character boxes, mapped to the rendered bitmap,
                // cover the word in the image (also on rotated and cropped pages).
                let image = rendered.as_image().to_luma8();
                let (sx, sy) = (
                    f64::from(rendered.width()) / f64::from(page.width().value),
                    f64::from(rendered.height()) / f64::from(page.height().value),
                );
                let chars = text.chars();
                let mut glyphs = chars
                    .iter()
                    .filter(|c| c.unicode_char().is_some_and(|c| !c.is_whitespace()));
                for (word, [l, t, r, b]) in page_words {
                    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for _ in word.chars() {
                        let bounds = glyphs.next().unwrap().loose_bounds().unwrap();
                        for (x, y) in [
                            (bounds.left(), bounds.top()),
                            (bounds.right(), bounds.bottom()),
                        ] {
                            let (px, py) = page.points_to_pixels(x, y, &config).unwrap();
                            x0 = x0.min(f64::from(px));
                            x1 = x1.max(f64::from(px));
                            y0 = y0.min(f64::from(py));
                            y1 = y1.max(f64::from(py));
                        }
                    }
                    let expected = [l * sx, t * sy, r * sx, b * sy];
                    let ix = (x1.min(expected[2]) - x0.max(expected[0])).max(0.0);
                    let iy = (y1.min(expected[3]) - y0.max(expected[1])).max(0.0);
                    let union = (x1 - x0) * (y1 - y0)
                        + (expected[2] - expected[0]) * (expected[3] - expected[1])
                        - ix * iy;
                    let iou = ix * iy / union;
                    assert!(iou > 0.85, "{label} {word}: IoU {iou:.3}");
                    let ink = (x0.max(0.0) as u32..(x1 as u32).min(image.width()))
                        .flat_map(|x| {
                            (y0.max(0.0) as u32..(y1 as u32).min(image.height()))
                                .map(move |y| (x, y))
                        })
                        .filter(|&(x, y)| image.get_pixel(x, y).0[0] < 128)
                        .count();
                    assert!(ink > 0, "{label} {word}: no ink under the text layer box");
                }
            }
            // The text and blank pages are copied untouched.
            for index in [5u16, 6] {
                let text = |document: &PdfDocument<'_>| {
                    squash(&document.pages().get(index).unwrap().text().unwrap().all())
                };
                assert_eq!(text(&copy), text(&original));
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn copy_keeps_the_original_content_stream_and_adds_its_own() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("searchable.pdf");
            let source = fixture("scanned-turkish.pdf");
            let pages = fixture_page_words(pdfium);
            write_searchable_copy(pdfium, &source, None, &pages[..1], &output).unwrap();
            // Test-only structural inspection: PDFium does not expose raw content streams.
            let contents = |path: &Path| -> Vec<Vec<u8>> {
                let document = lopdf::Document::load(path).unwrap();
                let page = *document.get_pages().get(&1).unwrap();
                document
                    .get_page_contents(page)
                    .into_iter()
                    .map(|id| {
                        document
                            .get_object(id)
                            .unwrap()
                            .as_stream()
                            .unwrap()
                            .decompressed_content()
                            .unwrap_or_else(|_| {
                                document
                                    .get_object(id)
                                    .unwrap()
                                    .as_stream()
                                    .unwrap()
                                    .content
                                    .clone()
                            })
                    })
                    .collect()
            };
            let before = contents(&source);
            let after = contents(&output);
            assert_eq!(before.len(), 1);
            assert!(after.len() >= 2, "text layer goes into its own stream");
            assert_eq!(
                after[0], before[0],
                "the scanned page's own stream is unchanged"
            );
            assert!(
                String::from_utf8_lossy(after.last().unwrap()).contains("3 Tr"),
                "invisible render mode"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn failed_write_leaves_no_output() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("searchable.pdf");
            let mut pages = fixture_page_words(pdfium);
            pages[1].bitmap_width += 7;
            let failure = write_searchable_copy(
                pdfium,
                &fixture("scanned-turkish.pdf"),
                None,
                &pages,
                &output,
            )
            .unwrap_err();
            assert_eq!(failure.code, "SEARCHABLE_PDF_GEOMETRY_MISMATCH");
            assert!(!output.exists());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unwritable_words_are_counted_not_written() {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("searchable.pdf");
            let mut pages = fixture_page_words(pdfium);
            pages.truncate(1);
            let width = pages[0].bitmap_width as i32;
            pages[0].words.push(LayoutWord {
                text: "\u{1F600}".into(),
                left: 1,
                top: 1,
                width: 5,
                height: 5,
            });
            pages[0].words.push(LayoutWord {
                text: "taşan".into(),
                left: width - 2,
                top: 1,
                width: 5,
                height: 5,
            });
            let report = write_searchable_copy(
                pdfium,
                &fixture("scanned-turkish.pdf"),
                None,
                &pages,
                &output,
            )
            .unwrap();
            assert_eq!(report.words_skipped, 2);
            assert_eq!(report.words_written as usize, pages[0].words.len() - 2);
            Ok(())
        })
        .unwrap();
    }
}
