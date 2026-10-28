// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// The token expansion ([`expand`], [`bates_start`]) and the header/footer and watermark layout are
// ported from PdfCraft 0.14.0, crates/edit/src/lib.rs (MIT OR Apache-2.0, used under MIT).
// Changes from the original: the marks are drawn as PDFium page objects instead of hand-written
// content streams, so text can use an embedded TrueType font (Turkish ğ ş ı İ are not in
// WinAnsi); Turkish tokens (`<<Sayfa 1>>`, `<<Sayfa 1 / n>>`, `<<dd.mm.yyyy>>`) are added; the
// date comes from the request; picture watermarks, underlines and shrink-to-fit are not ported.

//! Sayfa işaretleri: üstbilgi/altbilgi (sayfa numarası, tarih ve Bates belirteçleriyle),
//! filigran ve düz renk arka plan.
//!
//! İşaretler PDFium sayfa nesneleri olarak eklenir ve `/Artifact` işaretli içerik taşır
//! (`Type (Pagination)`, `Subtype (Header|Footer|Watermark|Background)`); böylece metin çıkarma
//! ve erişilebilirlik onları içerik saymaz, sonradan bulunup değiştirilebilir ya da
//! kaldırılabilir. PDFium yeni nesneleri sayfanın sonuna ayrı bir içerik akışı olarak ekler ve var
//! olan akışlara dokunmaz; "arkada" istenen işaretlerde bu akış lopdf ile `/Contents`'in başına
//! taşınır. Yazım artımlıdır; sonuç PDFium ile yeniden açılıp işaret sayılarıyla doğrulanır.
//!
//! Konumlar sayfanın görüntü uzayındadır (`/Rotate` ve CropBox uygulanmış, sol-alt köken,
//! point): döndürülmüş sayfada üstbilgi okuyucunun gördüğü üst kenardadır.
//!
//! Metin yalnız WinAnsi karakterleri taşıyorsa standart Helvetica kullanılır (gömülmez). Türkçe
//! ğ ş ı İ gibi karakterlerde bulunan ilk TrueType sistem fontu tümüyle gömülür; PDFium alt küme
//! çıkarmadığı için bu, dosyaya işlem başına birkaç yüz kB ekler.

use crate::annotations::PageDisplayTransform;
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
    DocumentIoStage,
};
use crate::incremental_save::{save_mutation, RawDocument, RawPage};
use lopdf::Object;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::raw::{c_int, c_ulong};
use std::path::{Path, PathBuf};

/// Bir metin kutusunda en fazla karakter ve satır.
pub const MAX_MARK_TEXT_CHARS: usize = 1024;
pub const MAX_MARK_TEXT_LINES: usize = 10;

const OPERATION: &str = "add_page_marks";
const MARK_NAME: &str = "Artifact";
const FPDF_FONT_TRUETYPE: c_int = 2;
const FPDF_FILLMODE_WINDING: c_int = 2;

/// Türkçe gibi WinAnsi dışı karakterler için gömülecek TrueType fontu adayları, sırayla.
/// Uygulama font taşımaz; bulunan ilk sistem fontu kullanılır.
const SYSTEM_FONT_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/Library/Fonts/Arial.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PageMarkKind {
    HeaderFooter,
    Watermark,
    Background,
}

impl PageMarkKind {
    /// İşaretin `Subtype` değerleri; üstbilgi ve altbilgi aynı türdür.
    fn subtypes(self) -> &'static [&'static str] {
        match self {
            Self::HeaderFooter => &["Header", "Footer"],
            Self::Watermark => &["Watermark"],
            Self::Background => &["Background"],
        }
    }

    fn of_subtype(subtype: &str) -> Option<Self> {
        [Self::HeaderFooter, Self::Watermark, Self::Background]
            .into_iter()
            .find(|kind| kind.subtypes().contains(&subtype))
    }
}

/// Bugünün tarihi (yerel); tarih belirteçleri bundan üretilir.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MarkDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeaderFooterSpec {
    /// Sol, orta, sağ üstbilgi; sonra sol, orta, sağ altbilgi. Belirteçler için bkz. [`expand`].
    pub texts: Vec<String>,
    pub font_size: f32,
    /// `#RRGGBB`.
    pub color: String,
    /// Kenar boşlukları (pt): üst, alt, sol, sağ.
    pub margins: [f32; 4],
    /// Aralığın ilk sayfasında `<<1>>`'in gösterdiği numara.
    pub start_number: u32,
    pub date: MarkDate,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkSpec {
    pub text: String,
    /// 0: sayfa köşegeninin yarısına sığacak boyut.
    pub font_size: f32,
    pub color: String,
    /// 0–1.
    pub opacity: f32,
    /// Saat yönü tersine derece (45: sol-alttan sağ-üste).
    pub rotation: f32,
    /// Merkezin sayfa merkezinden kayması (pt; sağa, yukarı).
    pub offset: [f32; 2],
    /// İçeriğin arkasına çiz.
    pub behind: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSpec {
    pub color: String,
    pub opacity: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PageMarkOp {
    /// Üstbilgi/altbilgi ekler; `replace` aynı türden var olan işaretleri önce kaldırır.
    HeaderFooter {
        spec: HeaderFooterSpec,
        replace: bool,
    },
    Watermark {
        spec: WatermarkSpec,
        replace: bool,
    },
    Background {
        spec: BackgroundSpec,
        replace: bool,
    },
    /// Bu uygulamanın eklediği işaretlerden `kinds` türündekileri kaldırır.
    Remove {
        kinds: Vec<PageMarkKind>,
    },
}

impl PageMarkOp {
    fn kind(&self) -> Option<PageMarkKind> {
        match self {
            Self::HeaderFooter { .. } => Some(PageMarkKind::HeaderFooter),
            Self::Watermark { .. } => Some(PageMarkKind::Watermark),
            Self::Background { .. } => Some(PageMarkKind::Background),
            Self::Remove { .. } => None,
        }
    }

    /// Eklemeden önce kaldırılacak türler.
    fn removes(&self) -> Vec<PageMarkKind> {
        match self {
            Self::HeaderFooter { replace, .. }
            | Self::Watermark { replace, .. }
            | Self::Background { replace, .. } => {
                if *replace {
                    self.kind().into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            Self::Remove { kinds } => kinds.clone(),
        }
    }

    fn behind(&self) -> bool {
        match self {
            Self::Watermark { spec, .. } => spec.behind,
            Self::Background { .. } => true,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Belirteçler
// ---------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Expand `<<…>>` tokens for a page numbered `page_number` of `count` pages:
/// `<<1>>`, `<<n>>`, `<<1 of n>>`, `<<1/n>>`, `<<Page 1>>`, `<<Page 1 of n>>`, `<<Sayfa 1>>`,
/// `<<Sayfa 1 / n>>`, dates (`<<m/d/yyyy>>`, `<<dd.mm.yyyy>>`, `<<yyyy-mm-dd>>`, …) and Bates
/// numbers `<<Bates Number#digits#start#prefix#suffix>>`. Unknown tokens stay as written.
pub(crate) fn expand(
    template: &str,
    page_number: u32,
    count: usize,
    bates: u64,
    date: MarkDate,
) -> String {
    let MarkDate {
        year: y,
        month: m,
        day: d,
    } = date;
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("<<") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find(">>") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = &after[..end];
        let p = page_number;
        let value = match token {
            "1" => p.to_string(),
            "n" => count.to_string(),
            "1 of n" => format!("{p} of {count}"),
            "1/n" => format!("{p}/{count}"),
            "Page 1" => format!("Page {p}"),
            "Page 1 of n" => format!("Page {p} of {count}"),
            "Sayfa 1" => format!("Sayfa {p}"),
            "Sayfa 1 / n" => format!("Sayfa {p} / {count}"),
            "m/d" => format!("{m}/{d}"),
            "m/d/yy" => format!("{m}/{d}/{:02}", y.rem_euclid(100)),
            "m/d/yyyy" => format!("{m}/{d}/{y}"),
            "mm/dd/yy" => format!("{m:02}/{d:02}/{:02}", y.rem_euclid(100)),
            "mm/dd/yyyy" => format!("{m:02}/{d:02}/{y}"),
            "d/m/yy" => format!("{d}/{m}/{:02}", y.rem_euclid(100)),
            "d/m/yyyy" => format!("{d}/{m}/{y}"),
            "dd/mm/yyyy" => format!("{d:02}/{m:02}/{y}"),
            "dd.mm.yyyy" => format!("{d:02}.{m:02}.{y}"),
            "yyyy-mm-dd" => format!("{y}-{m:02}-{d:02}"),
            "yyyy/mm/dd" => format!("{y}/{m:02}/{d:02}"),
            "yyyy/m/d" => format!("{y}/{m}/{d}"),
            "mmmm d, yyyy" => format!("{} {d}, {y}", MONTHS[(m.clamp(1, 12) - 1) as usize]),
            t if t.starts_with("Bates Number") => {
                // Bates Number#digits#start#prefix#suffix (start is applied by the caller).
                let parts: Vec<&str> = t.split('#').collect();
                let digits = parts
                    .get(1)
                    .and_then(|d| d.parse::<usize>().ok())
                    .unwrap_or(6)
                    .clamp(1, 15);
                format!(
                    "{}{:0digits$}{}",
                    parts.get(3).unwrap_or(&""),
                    bates,
                    parts.get(4).unwrap_or(&"")
                )
            }
            _ => format!("<<{token}>>"),
        };
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// The Bates start number in a template (`<<Bates Number#digits#start#…>>`), if any.
fn bates_start(template: &str) -> Option<u64> {
    let i = template.find("<<Bates Number")?;
    let t = &template[i + 2..];
    let t = &t[..t.find(">>")?];
    t.split('#').nth(2).and_then(|s| s.parse().ok()).or(Some(1))
}

// ---------------------------------------------------------------------------
// İstek denetimi
// ---------------------------------------------------------------------------

fn parse_color(value: &str) -> Result<(u8, u8, u8), String> {
    let hex = value
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| format!("Renk #RRGGBB biçiminde olmalı: {value}"))?;
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
    Ok((channel(0), channel(2), channel(4)))
}

fn check_text(text: &str, what: &str) -> Result<(), String> {
    if text.chars().count() > MAX_MARK_TEXT_CHARS || text.lines().count() > MAX_MARK_TEXT_LINES {
        return Err(format!(
            "{what} en fazla {MAX_MARK_TEXT_LINES} satır ve {MAX_MARK_TEXT_CHARS} karakter olabilir"
        ));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        return Err(format!("{what} denetim karakteri içeremez"));
    }
    Ok(())
}

fn check_opacity(opacity: f32) -> Result<(), String> {
    if !(opacity.is_finite() && (0.01..=1.0).contains(&opacity)) {
        return Err("Opaklık 0,01 ile 1 arasında olmalı".to_string());
    }
    Ok(())
}

/// Belgeden bağımsız istek denetimi.
pub(crate) fn validate_request(op: &PageMarkOp, pages: &[u32]) -> Result<(), String> {
    if pages.len() > crate::job_worker::MAX_JOB_PAGES {
        return Err("Sayfa listesi çok uzun".to_string());
    }
    match op {
        PageMarkOp::HeaderFooter { spec, .. } => {
            if spec.texts.len() != 6 {
                return Err("Üstbilgi/altbilgi altı metin kutusu taşımalı".to_string());
            }
            if spec.texts.iter().all(|text| text.trim().is_empty()) {
                return Err("Üstbilgi ya da altbilgi metni girin".to_string());
            }
            for text in &spec.texts {
                check_text(text, "Üstbilgi/altbilgi metni")?;
            }
            if !(spec.font_size.is_finite() && (1.0..=200.0).contains(&spec.font_size)) {
                return Err("Yazı boyutu 1 ile 200 pt arasında olmalı".to_string());
            }
            if !spec
                .margins
                .iter()
                .all(|margin| margin.is_finite() && (0.0..=2000.0).contains(margin))
            {
                return Err("Kenar boşlukları 0 ile 2000 pt arasında olmalı".to_string());
            }
            if !(1..=12).contains(&spec.date.month) || !(1..=31).contains(&spec.date.day) {
                return Err("Tarih geçersiz".to_string());
            }
            parse_color(&spec.color)?;
        }
        PageMarkOp::Watermark { spec, .. } => {
            if spec.text.trim().is_empty() {
                return Err("Filigran metnini girin".to_string());
            }
            check_text(&spec.text, "Filigran metni")?;
            if !(spec.font_size.is_finite()
                && (spec.font_size == 0.0 || (1.0..=500.0).contains(&spec.font_size)))
            {
                return Err("Filigran yazı boyutu 0 (otomatik) ya da 1–500 pt olmalı".to_string());
            }
            if !spec.rotation.is_finite() || !spec.offset.iter().all(|value| value.is_finite()) {
                return Err("Filigran açısı ve kayması sayı olmalı".to_string());
            }
            check_opacity(spec.opacity)?;
            parse_color(&spec.color)?;
        }
        PageMarkOp::Background { spec, .. } => {
            check_opacity(spec.opacity)?;
            parse_color(&spec.color)?;
        }
        PageMarkOp::Remove { kinds } => {
            if kinds.is_empty() {
                return Err("Kaldırılacak işaret türünü seçin".to_string());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Yazım
// ---------------------------------------------------------------------------

/// `file_path`'teki PDF'in `pages` sayfalarına (boşsa tümüne) `op`'u uygular; yerinde, atomik
/// ve artımlı.
pub(crate) fn add_page_marks_report(
    file_path: String,
    op: PageMarkOp,
    pages: Vec<u32>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let preflight = |detail: String| {
        DocumentIoError::new(
            OPERATION,
            DocumentIoStage::Preflight,
            CommitState::Prepared,
            detail,
        )
    };
    validate_request(&op, &pages).map_err(preflight)?;
    if password.is_some() {
        return Err(preflight(
            "Şifreli PDF'lerde sayfa işaretleri desteklenmiyor".to_string(),
        ));
    }
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        OPERATION,
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| write_marks(pdfium, &source, &op, &pages, temp),
        |_, ()| Ok(()),
    )
}

/// Görüntü uzayı (point, sol-alt köken) → user space dönüşümü ve görüntü boyutu.
#[derive(Clone, Copy, Debug)]
struct DisplayFrame {
    width: f64,
    height: f64,
    /// `[a b c d e f]`: görüntü noktasını user space'e götüren afin matris.
    matrix: [f64; 6],
}

impl DisplayFrame {
    fn of(transform: &PageDisplayTransform) -> Self {
        let (origin, x_axis, y_axis) = transform.parts();
        let width = x_axis[0].hypot(x_axis[1]);
        let height = y_axis[0].hypot(y_axis[1]);
        Self {
            width,
            height,
            matrix: [
                x_axis[0] / width,
                x_axis[1] / width,
                -y_axis[0] / height,
                -y_axis[1] / height,
                origin[0] + y_axis[0],
                origin[1] + y_axis[1],
            ],
        }
    }

    /// Önce `local` (görüntü uzayında), sonra bu çerçeve.
    fn compose(&self, local: [f64; 6]) -> [f64; 6] {
        let [a, b, c, d, e, f] = local;
        let [a2, b2, c2, d2, e2, f2] = self.matrix;
        [
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ]
    }
}

/// `text` WinAnsi (Windows-1252) ile yazılabilir mi; değilse TrueType gömülür.
fn win_ansi(text: &str) -> bool {
    const EXTRA: &str = "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ";
    text.chars().all(|c| {
        matches!(c as u32, 0x20..=0x7E | 0xA0..=0xFF)
            || matches!(c, '\n' | '\r' | '\t')
            || EXTRA.contains(c)
    })
}

/// Belgeye yüklenmiş bir font; düşerken bırakılır.
struct LoadedFont<'a> {
    bindings: &'a dyn PdfiumLibraryBindings,
    handle: FPDF_FONT,
}

impl<'a> LoadedFont<'a> {
    fn load(document: &RawDocument<'a>, needs_unicode: bool) -> Result<Self, DocumentIoFailure> {
        let bindings = document.bindings;
        let handle = if needs_unicode {
            let bytes = SYSTEM_FONT_CANDIDATES
                .iter()
                .find_map(|path| std::fs::read(path).ok())
                .ok_or_else(|| {
                    DocumentIoFailure::preflight(
                        "Bu karakterler için gömülecek bir TrueType sistem fontu bulunamadı",
                    )
                })?;
            let size = u32::try_from(bytes.len())
                .map_err(|_| DocumentIoFailure::preflight("Sistem fontu çok büyük"))?;
            bindings.FPDFText_LoadFont(document.handle, bytes.as_ptr(), size, FPDF_FONT_TRUETYPE, 1)
        } else {
            bindings.FPDFText_LoadStandardFont(document.handle, "Helvetica")
        };
        if handle.is_null() {
            return Err(DocumentIoFailure::write("Font yüklenemedi"));
        }
        Ok(Self { bindings, handle })
    }
}

impl Drop for LoadedFont<'_> {
    fn drop(&mut self) {
        self.bindings.FPDFFont_Close(self.handle);
    }
}

/// Sayfaya eklenecek bir nesnenin `/Artifact` işareti.
fn mark(
    document: &RawDocument<'_>,
    object: FPDF_PAGEOBJECT,
    subtype: &str,
) -> Result<(), DocumentIoFailure> {
    let bindings = document.bindings;
    let mark = bindings.FPDFPageObj_AddMark(object, MARK_NAME);
    let set = |key: &str, value: &str| {
        bindings.is_true(bindings.FPDFPageObjMark_SetStringParam(
            document.handle,
            object,
            mark,
            key,
            value,
        ))
    };
    if mark.is_null() || !set("Type", "Pagination") || !set("Subtype", subtype) {
        return Err(DocumentIoFailure::write("Nesne işaretlenemedi"));
    }
    Ok(())
}

/// Tek satırlık metin nesnesi ve dönüşümsüz genişliği. Nesne sayfaya eklenene kadar
/// çağıranındır.
fn text_object(
    document: &RawDocument<'_>,
    font: &LoadedFont<'_>,
    size: f32,
    text: &str,
    (red, green, blue): (u8, u8, u8),
    alpha: u8,
) -> Result<(FPDF_PAGEOBJECT, f64), DocumentIoFailure> {
    let bindings = document.bindings;
    let object = bindings.FPDFPageObj_CreateTextObj(document.handle, font.handle, size);
    if object.is_null() {
        return Err(DocumentIoFailure::write("Metin nesnesi oluşturulamadı"));
    }
    if !bindings.is_true(bindings.FPDFText_SetText_str(object, text))
        || !bindings.is_true(bindings.FPDFPageObj_SetFillColor(
            object,
            u32::from(red),
            u32::from(green),
            u32::from(blue),
            u32::from(alpha),
        ))
    {
        bindings.FPDFPageObj_Destroy(object);
        return Err(DocumentIoFailure::write("Metin nesnesi ayarlanamadı"));
    }
    let (mut left, mut bottom, mut right, mut top) = (0.0, 0.0, 0.0, 0.0);
    let width = if bindings.is_true(bindings.FPDFPageObj_GetBounds(
        object,
        &mut left,
        &mut bottom,
        &mut right,
        &mut top,
    )) {
        f64::from(right - left)
    } else {
        0.0
    };
    Ok((object, width))
}

/// Nesneyi işaretleyip dönüştürerek sayfaya ekler; başarısızlıkta nesneyi yok eder.
fn place(
    document: &RawDocument<'_>,
    page: &RawPage<'_>,
    object: FPDF_PAGEOBJECT,
    matrix: [f64; 6],
    subtype: &str,
) -> Result<(), DocumentIoFailure> {
    let bindings = document.bindings;
    if let Err(error) = mark(document, object, subtype) {
        bindings.FPDFPageObj_Destroy(object);
        return Err(error);
    }
    let [a, b, c, d, e, f] = matrix;
    bindings.FPDFPageObj_Transform(object, a, b, c, d, e, f);
    bindings.FPDFPage_InsertObject(page.handle, object);
    Ok(())
}

/// PDFium'un UTF-16LE döndüren bir okumasını (önce uzunluk, sonra içerik) metne çevirir.
fn read_utf16(
    bindings: &dyn PdfiumLibraryBindings,
    read: impl Fn(*mut FPDF_WCHAR, c_ulong, *mut c_ulong) -> FPDF_BOOL,
) -> Option<String> {
    let mut length: c_ulong = 0;
    if !bindings.is_true(read(std::ptr::null_mut(), 0, &mut length)) || length < 2 {
        return None;
    }
    let mut buffer = vec![0 as FPDF_WCHAR; (length as usize).div_ceil(2)];
    if !bindings.is_true(read(buffer.as_mut_ptr(), length, &mut length)) {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end]).ok()
}

/// Nesne bu uygulamanın işaretlerinden birini taşıyorsa türü.
fn mark_kind(
    bindings: &dyn PdfiumLibraryBindings,
    object: FPDF_PAGEOBJECT,
) -> Option<PageMarkKind> {
    for index in 0..bindings.FPDFPageObj_CountMarks(object).max(0) {
        let mark = bindings.FPDFPageObj_GetMark(object, index as c_ulong);
        if mark.is_null() {
            continue;
        }
        let name = read_utf16(bindings, |buffer, length, out| {
            bindings.FPDFPageObjMark_GetName(mark, buffer, length, out)
        });
        if name.as_deref() != Some(MARK_NAME) {
            continue;
        }
        let param = |key: &str| {
            read_utf16(bindings, |buffer, length, out| {
                bindings.FPDFPageObjMark_GetParamStringValue(mark, key, buffer, length, out)
            })
        };
        if param("Type").as_deref() == Some("Pagination") {
            if let Some(kind) = param("Subtype")
                .as_deref()
                .and_then(PageMarkKind::of_subtype)
            {
                return Some(kind);
            }
        }
    }
    None
}

/// Sayfadaki işaretlerin türlere göre sayısı.
fn count_marks(page: &RawPage<'_>) -> BTreeMap<PageMarkKind, usize> {
    let bindings = page.bindings;
    let mut counts = BTreeMap::new();
    for index in 0..bindings.FPDFPage_CountObjects(page.handle).max(0) {
        let object = bindings.FPDFPage_GetObject(page.handle, index);
        if object.is_null() {
            continue;
        }
        if let Some(kind) = mark_kind(bindings, object) {
            *counts.entry(kind).or_default() += 1;
        }
    }
    counts
}

/// `kinds` türündeki işaretleri sayfadan kaldırır; kaldırılan sayısı.
fn remove_marks(page: &RawPage<'_>, kinds: &[PageMarkKind]) -> Result<usize, DocumentIoFailure> {
    let bindings = page.bindings;
    let mut removed = 0;
    // Backwards, so the indices of the objects still to visit don't shift.
    for index in (0..bindings.FPDFPage_CountObjects(page.handle).max(0)).rev() {
        let object = bindings.FPDFPage_GetObject(page.handle, index);
        if object.is_null()
            || !mark_kind(bindings, object).is_some_and(|kind| kinds.contains(&kind))
        {
            continue;
        }
        if !bindings.is_true(bindings.FPDFPage_RemoveObject(page.handle, object)) {
            return Err(DocumentIoFailure::write("Sayfa işareti kaldırılamadı"));
        }
        bindings.FPDFPageObj_Destroy(object);
        removed += 1;
    }
    Ok(removed)
}

/// Sayfa → işlemden sonra beklenen işaret sayıları.
type Expected = BTreeMap<u32, BTreeMap<PageMarkKind, usize>>;

fn write_marks(
    pdfium: &Pdfium,
    source: &Path,
    op: &PageMarkOp,
    pages: &[u32],
    temp: &Path,
) -> Result<(), DocumentIoFailure> {
    // Okumalar (sayfa sayısı, görüntü dönüşümleri) pdfium-render ile; düzenleme ham PDFium ile.
    let (page_count, frames) = {
        let reader = pdfium
            .load_pdf_from_file(source, None)
            .map_err(|error| DocumentIoFailure::preflight(format!("PDF açılamadı: {error}")))?;
        let page_count = u32::from(reader.pages().len());
        let targets: Vec<u32> = if pages.is_empty() {
            (0..page_count).collect()
        } else {
            pages.to_vec()
        };
        if let Some(page) = targets.iter().find(|&&page| page >= page_count) {
            return Err(DocumentIoFailure::preflight(format!(
                "Sayfa {} belgede yok (sayfa sayısı {page_count})",
                page + 1
            )));
        }
        let mut frames = Vec::with_capacity(targets.len());
        for page_index in targets {
            let page = reader.pages().get(page_index as u16).map_err(|error| {
                DocumentIoFailure::preflight(format!("Sayfa açılamadı: {error}"))
            })?;
            let transform =
                PageDisplayTransform::for_page(&page).map_err(DocumentIoFailure::preflight)?;
            frames.push((page_index, DisplayFrame::of(&transform)));
        }
        (page_count, frames)
    };

    let bindings = pdfium.bindings();
    let document = RawDocument::open_for_mutation(bindings, source, None)?;
    let font = match op {
        PageMarkOp::HeaderFooter { spec, .. } => Some(LoadedFont::load(
            &document,
            spec.texts.iter().any(|text| !win_ansi(text)),
        )?),
        PageMarkOp::Watermark { spec, .. } => {
            Some(LoadedFont::load(&document, !win_ansi(&spec.text))?)
        }
        _ => None,
    };
    let removes = op.removes();
    let bates0 = match op {
        PageMarkOp::HeaderFooter { spec, .. } => spec
            .texts
            .iter()
            .find_map(|text| bates_start(text))
            .unwrap_or(1),
        _ => 1,
    };
    let mut expected: Expected = BTreeMap::new();
    let mut removed_any = false;
    for (position, &(page_index, frame)) in frames.iter().enumerate() {
        let page = RawPage::load(&document, page_index)?;
        let mut counts = count_marks(&page);
        if !removes.is_empty() {
            removed_any |= remove_marks(&page, &removes)? > 0;
            for kind in &removes {
                counts.remove(kind);
            }
        }
        let added = match (op, font.as_ref()) {
            (PageMarkOp::HeaderFooter { spec, .. }, Some(font)) => {
                let number = spec
                    .start_number
                    .checked_add(position as u32)
                    .ok_or_else(|| DocumentIoFailure::preflight("Sayfa numaraları çok büyük"))?;
                let bates = bates0
                    .checked_add(position as u64)
                    .ok_or_else(|| DocumentIoFailure::preflight("Bates numaraları çok büyük"))?;
                let numbers = PageNumbers {
                    number,
                    count: page_count as usize,
                    bates,
                };
                add_header_footer(&document, &page, frame, font, spec, numbers)?
            }
            (PageMarkOp::Watermark { spec, .. }, Some(font)) => {
                add_watermark(&document, &page, frame, font, spec)?
            }
            (PageMarkOp::Background { spec, .. }, _) => {
                add_background(&document, &page, frame, spec)?
            }
            _ => 0,
        };
        if let Some(kind) = op.kind() {
            if added > 0 {
                *counts.entry(kind).or_default() += added;
            }
        }
        if !bindings.is_true(bindings.FPDFPage_GenerateContent(page.handle)) {
            return Err(DocumentIoFailure::write("Sayfa içeriği üretilemedi"));
        }
        expected.insert(page_index, counts);
    }
    if matches!(op, PageMarkOp::Remove { .. }) && !removed_any {
        return Err(DocumentIoFailure::preflight(
            "Seçilen sayfalarda kaldırılacak işaret yok",
        ));
    }
    drop(font);
    save_mutation(&document, source, temp)?;
    drop(document);

    if op.behind() {
        let pages: Vec<u32> = frames.iter().map(|(page, _)| *page).collect();
        let above_backgrounds = !matches!(op, PageMarkOp::Background { .. });
        move_last_stream_behind_content(temp, &pages, above_backgrounds)?;
    }
    verify_marks(pdfium, temp, &expected)
}

/// Bir sayfanın üstbilgi/altbilgi numaraları.
#[derive(Clone, Copy)]
struct PageNumbers {
    number: u32,
    count: usize,
    bates: u64,
}

fn add_header_footer(
    document: &RawDocument<'_>,
    page: &RawPage<'_>,
    frame: DisplayFrame,
    font: &LoadedFont<'_>,
    spec: &HeaderFooterSpec,
    numbers: PageNumbers,
) -> Result<usize, DocumentIoFailure> {
    let color = parse_color(&spec.color).map_err(DocumentIoFailure::preflight)?;
    let size = f64::from(spec.font_size);
    let [top, bottom, left, right] = spec.margins.map(f64::from);
    let line_height = size * 1.2;
    let mut added = 0;
    for (slot, template) in spec.texts.iter().enumerate() {
        if template.trim().is_empty() {
            continue;
        }
        let expanded = expand(
            template,
            numbers.number,
            numbers.count,
            numbers.bates,
            spec.date,
        );
        let lines: Vec<&str> = expanded.lines().collect();
        let header = slot < 3;
        for (line_index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let (object, width) = text_object(document, font, spec.font_size, line, color, 255)?;
            let x = match slot % 3 {
                0 => left,
                1 => (frame.width - width) / 2.0,
                _ => frame.width - right - width,
            };
            // Headers hang below the top margin; footers sit on the bottom margin.
            let y = if header {
                frame.height - top - size * 0.8 - line_index as f64 * line_height
            } else {
                bottom + (lines.len() - 1 - line_index) as f64 * line_height
            };
            let subtype = if header { "Header" } else { "Footer" };
            let matrix = frame.compose([1.0, 0.0, 0.0, 1.0, x, y]);
            place(document, page, object, matrix, subtype)?;
            added += 1;
        }
    }
    Ok(added)
}

fn add_watermark(
    document: &RawDocument<'_>,
    page: &RawPage<'_>,
    frame: DisplayFrame,
    font: &LoadedFont<'_>,
    spec: &WatermarkSpec,
) -> Result<usize, DocumentIoFailure> {
    let color = parse_color(&spec.color).map_err(DocumentIoFailure::preflight)?;
    let alpha = (spec.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let lines: Vec<&str> = spec
        .text
        .trim()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    // Width at 1 pt decides the automatic size: the widest line spans half the page diagonal.
    let size = if spec.font_size > 0.0 {
        f64::from(spec.font_size)
    } else {
        let mut widest: f64 = 0.0;
        for line in &lines {
            let (object, width) = text_object(document, font, 1.0, line, color, alpha)?;
            document.bindings.FPDFPageObj_Destroy(object);
            widest = widest.max(width);
        }
        (frame.width.hypot(frame.height) * 0.5 / widest.max(0.01)).clamp(6.0, 300.0)
    };
    let (sin, cos) = f64::from(spec.rotation).to_radians().sin_cos();
    let center = (
        frame.width / 2.0 + f64::from(spec.offset[0]),
        frame.height / 2.0 + f64::from(spec.offset[1]),
    );
    let line_height = size * 1.15;
    let total = line_height * (lines.len() as f64 - 1.0);
    for (line_index, line) in lines.iter().enumerate() {
        let (object, width) = text_object(document, font, size as f32, line, color, alpha)?;
        // The line in the watermark's own frame (centred), then rotated about the centre.
        let (lx, ly) = (
            -width / 2.0,
            total / 2.0 - line_index as f64 * line_height - size * 0.35,
        );
        let local = [
            cos,
            sin,
            -sin,
            cos,
            center.0 + lx * cos - ly * sin,
            center.1 + lx * sin + ly * cos,
        ];
        place(document, page, object, frame.compose(local), "Watermark")?;
    }
    Ok(lines.len())
}

fn add_background(
    document: &RawDocument<'_>,
    page: &RawPage<'_>,
    frame: DisplayFrame,
    spec: &BackgroundSpec,
) -> Result<usize, DocumentIoFailure> {
    let bindings = document.bindings;
    let (red, green, blue) = parse_color(&spec.color).map_err(DocumentIoFailure::preflight)?;
    let alpha = (spec.opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
    let object =
        bindings.FPDFPageObj_CreateNewRect(0.0, 0.0, frame.width as f32, frame.height as f32);
    if object.is_null() {
        return Err(DocumentIoFailure::write("Arka plan oluşturulamadı"));
    }
    let filled =
        bindings.is_true(bindings.FPDFPageObj_SetFillColor(
            object,
            u32::from(red),
            u32::from(green),
            u32::from(blue),
            alpha,
        )) && bindings.is_true(bindings.FPDFPath_SetDrawMode(object, FPDF_FILLMODE_WINDING, 0));
    if !filled {
        bindings.FPDFPageObj_Destroy(object);
        return Err(DocumentIoFailure::write("Arka plan ayarlanamadı"));
    }
    let matrix = frame.compose([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    place(document, page, object, matrix, "Background")?;
    Ok(1)
}

/// Akışın içeriği (çözülmüş, sınırlı); okunamazsa `None`.
fn stream_content(
    writer: &crate::document_structure::StructureWriter,
    object: &Object,
) -> Option<Vec<u8>> {
    let Object::Reference(id) = object else {
        return None;
    };
    writer
        .object(*id)
        .ok()?
        .as_stream()
        .ok()?
        .get_plain_content_with_limit(64 * 1024 * 1024)
        .ok()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// PDFium yeni nesneleri her sayfanın `/Contents`'inin sonuna ayrı bir akış olarak ekler;
/// "arkada" çizilecek işaretlerde bu akış içeriğin önüne taşınır (ikinci bir artımlı güncelleme
/// olarak). `above_backgrounds` ise baştaki arka plan akışlarının hemen arkasına konur: filigran
/// arka planın üstünde, içeriğin altında kalır.
fn move_last_stream_behind_content(
    temp: &Path,
    pages: &[u32],
    above_backgrounds: bool,
) -> Result<(), DocumentIoFailure> {
    let mut writer = crate::document_structure::StructureWriter::load(temp, "sayfa işareti")?;
    let page_ids = writer.page_ids();
    for &page_index in pages {
        let page_id = *page_ids
            .get(page_index as usize)
            .ok_or_else(|| DocumentIoFailure::invariant("Sayfa yapıda bulunamadı"))?;
        let contents = writer
            .dict(page_id)?
            .get(b"Contents")
            .cloned()
            .map_err(|_| DocumentIoFailure::invariant("Sayfanın içerik akışı yok"))?;
        let (holder, mut streams) = match contents {
            Object::Array(items) => (None, items),
            Object::Reference(id) => match writer.object(id)? {
                Object::Array(items) => (Some(id), items.clone()),
                _ => (None, vec![Object::Reference(id)]),
            },
            _ => {
                return Err(DocumentIoFailure::invariant(
                    "Sayfanın içerik akışı okunamadı",
                ))
            }
        };
        if streams.len() < 2 {
            continue;
        }
        let last = streams.pop().expect("two or more streams");
        // The stream PDFium added is ours only if it holds our marked content.
        if !stream_content(&writer, &last).is_some_and(|content| contains(&content, b"/Artifact")) {
            return Err(DocumentIoFailure::invariant(
                "Eklenen işaret akışı sayfanın sonunda bulunamadı",
            ));
        }
        let at = if above_backgrounds {
            streams
                .iter()
                .take_while(|stream| {
                    stream_content(&writer, stream).is_some_and(|content| {
                        contains(&content, b"/Artifact") && contains(&content, b"(Background)")
                    })
                })
                .count()
        } else {
            0
        };
        streams.insert(at, last);
        match holder {
            Some(id) => writer.set(id, Object::Array(streams)),
            None => writer.edit(page_id, |page| page.set("Contents", Object::Array(streams)))?,
        }
    }
    writer.finish(temp)
}

/// Kaydedilen belgede her sayfanın işaret sayıları beklenenle aynı mı (PDFium ile).
fn verify_marks(
    pdfium: &Pdfium,
    temp: &Path,
    expected: &Expected,
) -> Result<(), DocumentIoFailure> {
    let document = RawDocument::open_for_mutation(pdfium.bindings(), temp, None)?;
    for (&page_index, counts) in expected {
        let page = RawPage::load(&document, page_index)?;
        let actual = count_marks(&page);
        if &actual != counts {
            return Err(DocumentIoFailure::invariant(format!(
                "Sayfa {} işaretleri {actual:?}, beklenen {counts:?}",
                page_index + 1
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::RectData;

    const DATE: MarkDate = MarkDate {
        year: 2026,
        month: 10,
        day: 28,
    };

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name)
    }

    fn copy(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.pdf");
        std::fs::copy(fixture(name), &path).unwrap();
        (dir, path)
    }

    fn apply(
        path: &Path,
        op: PageMarkOp,
        pages: Vec<u32>,
    ) -> Result<AtomicWriteReport, DocumentIoError> {
        add_page_marks_report(path.to_string_lossy().to_string(), op, pages, None)
    }

    fn header(texts: [&str; 6], replace: bool) -> PageMarkOp {
        PageMarkOp::HeaderFooter {
            spec: HeaderFooterSpec {
                texts: texts.iter().map(|text| text.to_string()).collect(),
                font_size: 10.0,
                color: "#000000".to_string(),
                margins: [36.0, 36.0, 72.0, 72.0],
                start_number: 1,
                date: DATE,
            },
            replace,
        }
    }

    fn watermark(text: &str, behind: bool) -> PageMarkOp {
        PageMarkOp::Watermark {
            spec: WatermarkSpec {
                text: text.to_string(),
                font_size: 0.0,
                color: "#999999".to_string(),
                opacity: 0.5,
                rotation: 45.0,
                offset: [0.0, 0.0],
                behind,
            },
            replace: false,
        }
    }

    fn page_text(path: &Path, page: u16) -> String {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<String, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|e| e.to_string())?;
            let text = document.pages().get(page).unwrap().text().unwrap().all();
            Ok(text)
        })
        .unwrap()
    }

    fn first_content_stream_is_ours(path: &Path, page: u32) -> bool {
        content_streams(path, page)[0]
            .windows(9)
            .any(|window| window == b"/Artifact")
    }

    /// The page's content streams, decoded, in drawing order.
    fn content_streams(path: &Path, page: u32) -> Vec<Vec<u8>> {
        let doc = lopdf::Document::load(path).unwrap();
        let page_id = *doc.get_pages().get(&(page + 1)).unwrap();
        let mut contents = doc
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Contents")
            .unwrap();
        if let Object::Reference(id) = contents {
            if let Ok(array @ Object::Array(_)) = doc.get_object(*id) {
                contents = array;
            }
        }
        let ids = match contents {
            Object::Array(items) => items
                .iter()
                .map(|item| item.as_reference().unwrap())
                .collect(),
            Object::Reference(id) => vec![*id],
            other => panic!("{other:?}"),
        };
        ids.into_iter()
            .map(|id| {
                let stream = doc.get_object(id).unwrap().as_stream().unwrap();
                stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone())
            })
            .collect()
    }

    #[test]
    fn tokens_expand_page_numbers_dates_and_bates() {
        let line = "<<Sayfa 1 / n>> · <<1 of n>> · <<dd.mm.yyyy>> · <<yyyy-mm-dd>> · <<Bates Number#6#1#ABC-#>> · <<x>>";
        assert_eq!(
            expand(line, 3, 12, 41, DATE),
            "Sayfa 3 / 12 · 3 of 12 · 28.10.2026 · 2026-10-28 · ABC-000041 · <<x>>"
        );
        assert_eq!(
            expand("<<mmmm d, yyyy>> <<Page 1>>", 7, 9, 1, DATE),
            "October 28, 2026 Page 7"
        );
        assert_eq!(expand("<<1", 1, 1, 1, DATE), "<<1");
        assert_eq!(bates_start("x <<Bates Number#4#250#P#>>"), Some(250));
        assert_eq!(bates_start("<<Bates Number>>"), Some(1));
        assert_eq!(bates_start("<<1>>"), None);
    }

    #[test]
    fn turkish_headers_are_numbered_per_page_and_appended_incrementally() {
        let (_dir, path) = copy("three-distinct-pages.pdf");
        let before = std::fs::read(&path).unwrap();
        apply(
            &path,
            header(
                ["", "Gizli — Ağaç Şube", "", "", "<<Sayfa 1 / n>>", ""],
                false,
            ),
            Vec::new(),
        )
        .unwrap();
        let after = std::fs::read(&path).unwrap();
        assert!(
            after.starts_with(&before),
            "original bytes must stay in place"
        );
        for page in 0..3u16 {
            let text = page_text(&path, page);
            assert!(text.contains("Gizli — Ağaç Şube"), "{text}");
            assert!(text.contains(&format!("Sayfa {} / 3", page + 1)), "{text}");
        }
    }

    #[test]
    fn replacing_keeps_one_set_and_remove_takes_it_away() {
        let (_dir, path) = copy("ai-text-rich.pdf");
        let original = page_text(&path, 0);
        apply(
            &path,
            header(["Taslak", "", "", "", "", ""], false),
            Vec::new(),
        )
        .unwrap();
        // The verification inside the write checks the mark count: one header, not two.
        apply(
            &path,
            header(["Nihai", "", "", "", "", ""], true),
            Vec::new(),
        )
        .unwrap();
        let text = page_text(&path, 0);
        assert!(text.contains("Nihai") && !text.contains("Taslak"), "{text}");
        apply(
            &path,
            PageMarkOp::Remove {
                kinds: vec![PageMarkKind::HeaderFooter],
            },
            Vec::new(),
        )
        .unwrap();
        assert_eq!(page_text(&path, 0), original);
        let error = apply(
            &path,
            PageMarkOp::Remove {
                kinds: vec![PageMarkKind::HeaderFooter],
            },
            Vec::new(),
        )
        .unwrap_err();
        assert!(
            error.detail.contains("kaldırılacak işaret yok"),
            "{}",
            error.detail
        );
    }

    #[test]
    fn behind_marks_are_drawn_before_the_page_content() {
        let (_dir, path) = copy("ai-text-rich.pdf");
        apply(&path, watermark("TASLAK", true), Vec::new()).unwrap();
        assert!(first_content_stream_is_ours(&path, 0));
        assert!(page_text(&path, 0).contains("TASLAK"));
        apply(
            &path,
            PageMarkOp::Background {
                spec: BackgroundSpec {
                    color: "#FFF4D6".to_string(),
                    opacity: 1.0,
                },
                replace: false,
            },
            Vec::new(),
        )
        .unwrap();
        assert!(first_content_stream_is_ours(&path, 0));
        // A later watermark behind the content stays above the background.
        apply(&path, watermark("İKİNCİ", true), Vec::new()).unwrap();
        let streams = content_streams(&path, 0);
        let has = |stream: &[u8], needle: &[u8]| stream.windows(needle.len()).any(|w| w == needle);
        assert!(has(&streams[0], b"(Background)"));
        assert!(has(&streams[1], b"(Watermark)"));

        // On top: the original content keeps the first place.
        let (_dir, path) = copy("ai-text-rich.pdf");
        apply(&path, watermark("ÖRNEK", false), Vec::new()).unwrap();
        assert!(!first_content_stream_is_ours(&path, 0));
        assert!(page_text(&path, 0).contains("ÖRNEK"));
    }

    #[test]
    fn a_header_lands_at_the_visible_top_of_a_rotated_page() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rotated.pdf");
        let mut doc = lopdf::Document::load(fixture("ai-text-rich.pdf")).unwrap();
        let page_id = *doc.get_pages().get(&1).unwrap();
        doc.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Rotate", 90);
        doc.save(&path).unwrap();
        apply(&path, header(["", "ÜST", "", "", "", ""], false), vec![0]).unwrap();
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let document = pdfium
                .load_pdf_from_file(&path, None)
                .map_err(|e| e.to_string())?;
            let page = document.pages().get(0).unwrap();
            let display = PageDisplayTransform::for_page(&page)?;
            let text = page.text().unwrap();
            let search = text.search("ÜST", &PdfSearchOptions::new()).unwrap();
            let segments = search.find_next().expect("header text");
            let bounds = segments.iter().next().unwrap().bounds();
            let rect = display.user_to_normalized_rect(&RectData {
                x: bounds.left().value,
                y: bounds.bottom().value,
                width: bounds.width().value,
                height: bounds.height().value,
            });
            assert!(rect.y < 0.1, "header should be at the top: {rect:?}");
            assert!(
                (rect.x + rect.width / 2.0 - 0.5).abs() < 0.05,
                "header should be centred: {rect:?}"
            );
            assert!(
                rect.width > rect.height,
                "header should read horizontally: {rect:?}"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn requests_are_checked_before_the_document_is_read() {
        assert!(validate_request(&header(["", "", "", "", "", ""], false), &[]).is_err());
        let mut op = header(["x", "", "", "", "", ""], false);
        if let PageMarkOp::HeaderFooter { spec, .. } = &mut op {
            spec.color = "red".to_string();
        }
        assert!(validate_request(&op, &[]).is_err());
        assert!(validate_request(&watermark("  ", false), &[]).is_err());
        assert!(validate_request(&PageMarkOp::Remove { kinds: Vec::new() }, &[]).is_err());
        assert!(validate_request(&watermark("ok", false), &[0, 1]).is_ok());
        let (_dir, path) = copy("minimal.pdf");
        let before = std::fs::read(&path).unwrap();
        let error = apply(&path, watermark("ok", false), vec![5]).unwrap_err();
        assert!(error.detail.contains("belgede yok"), "{}", error.detail);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn win_ansi_text_uses_the_standard_font() {
        assert!(win_ansi("Çok güzel — “Özel” €5"));
        assert!(!win_ansi("ağaç"));
        assert!(!win_ansi("İstanbul"));
        assert!(!win_ansi("ışık"));
    }
}
