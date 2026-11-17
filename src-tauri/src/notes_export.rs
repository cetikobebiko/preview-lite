//! Not dosyası: belgedeki PDF açıklamalarından Markdown.
//!
//! Job worker'da çalışır ([`export_snapshot`]). Açıklamalar, renkleri ve text markup
//! açıklamalarının (Highlight, Underline, StrikeOut, Squiggly) kapladığı metin PDFium ile okunur;
//! metin önce `/QuadPoints`'ten, yoksa `/Rect`'ten çıkarılır. Sayfa etiketleri mevcut
//! `/PageLabels` okuma yolundan ([`crate::document_structure::labels`]) gelir. Markdown burada,
//! girdilerin saf bir fonksiyonu olarak üretilir ([`render_markdown`]): aynı belge ve aynı
//! oluşturma zamanı aynı baytları verir.
//!
//! Not taşımayan açıklama türleri ([`SKIPPED_ANNOTATION_TYPES`]) atlanır. Atlandıktan sonra hiç
//! not kalmazsa [`NOTES_EXPORT_EMPTY`] döner ve dosya yazılmaz.

use crate::document_export::markdown::{block_start, md_text};
use crate::job_worker::JobFailure;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Worker'ın iş dizinine yazdığı dosyanın adı.
pub const NOTES_OUTPUT_NAME: &str = "notes-output.md";
/// Çıktının üst sınırı; ana process yayımlamadan önce aynı sınırı yeniden denetler.
pub const MAX_NOTES_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
/// En fazla not sayısı; daha fazlası [`NOTES_EXPORT_TOO_LARGE`] ile reddedilir.
pub const MAX_NOTES: usize = 50_000;
/// Tek bir alıntı veya not metninin en fazla karakteri; fazlası `…` ile kesilir.
pub const MAX_NOTE_FIELD_CHARS: usize = 20_000;
/// Bir text markup açıklamasından okunan en fazla dörtgen.
const MAX_MARKUP_QUADS: usize = 512;

/// Belgede (atlanan türler dışında) hiç açıklama yok; dosya yazılmadı.
pub const NOTES_EXPORT_EMPTY: &str = "NOTES_EXPORT_EMPTY";
/// Not sayısı ya da çıktı boyutu sınırı aşıldı.
pub const NOTES_EXPORT_TOO_LARGE: &str = "NOTES_EXPORT_TOO_LARGE";
/// İstek seçenekleri geçersiz.
pub const NOTES_EXPORT_INVALID: &str = "NOTES_EXPORT_INVALID";

/// Not taşımayan, bu yüzden not dosyasına girmeyen açıklama türleri: açılır pencereler (notun
/// kendisi üst açıklamadadır), bağlantılar, form alanları, ortam/3B oynatıcıları ve baskı
/// işaretleri.
pub const SKIPPED_ANNOTATION_TYPES: &[PdfPageAnnotationType] = &[
    PdfPageAnnotationType::Popup,
    PdfPageAnnotationType::Link,
    PdfPageAnnotationType::Widget,
    PdfPageAnnotationType::XfaWidget,
    PdfPageAnnotationType::Screen,
    PdfPageAnnotationType::Movie,
    PdfPageAnnotationType::RichMedia,
    PdfPageAnnotationType::ThreeD,
    PdfPageAnnotationType::PrinterMark,
    PdfPageAnnotationType::TrapNet,
    PdfPageAnnotationType::Watermark,
];

/// Frontend'in gönderdiği seçenekler. Serbest metin taşımaz.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotesExportOptions {
    /// Oluşturma tarihinin yazılacağı saat diliminin UTC'den farkı (dakika); yoksa UTC.
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
}

impl NotesExportOptions {
    /// Gerçek saat dilimleri UTC−12:00 ile UTC+14:00 arasındadır.
    pub fn validate(&self) -> Result<(), String> {
        match self.utc_offset_minutes {
            Some(offset) if !(-12 * 60..=14 * 60).contains(&offset) => Err(format!(
                "Saat dilimi farkı geçersiz: {offset} dakika (−720 … 840)"
            )),
            _ => Ok(()),
        }
    }
}

/// Not dosyasının başlık bilgisi: belge adı (yalnız dosya adı) ve oluşturma zamanı.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct NotesHeader {
    pub title: String,
    pub created_at_unix_ms: i64,
    pub utc_offset_minutes: i32,
}

/// Not dosyasına giren açıklama türleri.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Text,
    FreeText,
    Line,
    Square,
    Circle,
    Polygon,
    Polyline,
    Highlight,
    Underline,
    Squiggly,
    StrikeOut,
    Stamp,
    Caret,
    Ink,
    FileAttachment,
    Sound,
    Redact,
    Other,
}

impl NoteKind {
    /// PDFium türünün not türü; [`SKIPPED_ANNOTATION_TYPES`] için `None`.
    pub fn from_annotation_type(annotation_type: PdfPageAnnotationType) -> Option<Self> {
        use PdfPageAnnotationType as T;
        if SKIPPED_ANNOTATION_TYPES.contains(&annotation_type) {
            return None;
        }
        Some(match annotation_type {
            T::Text => Self::Text,
            T::FreeText => Self::FreeText,
            T::Line => Self::Line,
            T::Square => Self::Square,
            T::Circle => Self::Circle,
            T::Polygon => Self::Polygon,
            T::Polyline => Self::Polyline,
            T::Highlight => Self::Highlight,
            T::Underline => Self::Underline,
            T::Squiggly => Self::Squiggly,
            T::Strikeout => Self::StrikeOut,
            T::Stamp => Self::Stamp,
            T::Caret => Self::Caret,
            T::Ink => Self::Ink,
            T::FileAttachment => Self::FileAttachment,
            T::Sound => Self::Sound,
            T::Redacted => Self::Redact,
            _ => Self::Other,
        })
    }

    /// Türün Türkçe adı.
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "Metin notu",
            Self::FreeText => "Serbest metin",
            Self::Line => "Çizgi",
            Self::Square => "Dikdörtgen",
            Self::Circle => "Elips",
            Self::Polygon => "Çokgen",
            Self::Polyline => "Çoklu çizgi",
            Self::Highlight => "Vurgu",
            Self::Underline => "Altı çizili",
            Self::Squiggly => "Dalgalı altı çizili",
            Self::StrikeOut => "Üstü çizili",
            Self::Stamp => "Damga",
            Self::Caret => "Ekleme işareti",
            Self::Ink => "Serbest çizim",
            Self::FileAttachment => "Dosya eki",
            Self::Sound => "Ses",
            Self::Redact => "Karartma işareti",
            Self::Other => "Diğer açıklama",
        }
    }

    /// Kapladığı metin alıntılanan türler.
    pub fn is_text_markup(self) -> bool {
        matches!(
            self,
            Self::Highlight | Self::Underline | Self::Squiggly | Self::StrikeOut
        )
    }
}

/// Tek bir açıklamanın not dosyasındaki karşılığı.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub kind: NoteKind,
    /// `#RRGGBB`; başka biçimdeyse yazılmaz.
    pub color: String,
    /// Text markup açıklamasının kapladığı metin.
    pub quoted_text: Option<String>,
    /// Açıklamanın `/Contents`'i (not metni).
    pub contents: Option<String>,
}

/// Bir sayfanın notları.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotesPage {
    /// 0 tabanlı fiziksel sayfa.
    pub page_index: u32,
    /// Sayfa etiketi (`/PageLabels`); yoksa ya da boşsa `None`.
    pub label: Option<String>,
    pub notes: Vec<Note>,
}

/// Belgeyi job worker'da PDFium ile okur ve not dosyasını döndürür. `check_cancelled` her
/// sayfadan önce çağrılır.
pub fn export_snapshot(
    snapshot_path: &Path,
    credential: Option<&str>,
    header: &NotesHeader,
    mut check_cancelled: impl FnMut() -> Result<(), JobFailure>,
) -> Result<String, JobFailure> {
    check_cancelled()?;
    // Etiketler yalnız başlık içindir: okunamazlarsa sayfalar numarayla yazılır.
    let label_ranges =
        crate::document_structure::labels::inspect_snapshot(snapshot_path, credential)
            .unwrap_or_default();
    let pdfium = crate::pdf_engine::get_pdfium()
        .map_err(|message| JobFailure::new("SESSION_JOB_PDF_ENGINE_UNAVAILABLE", message))?;
    let document =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
            .map_err(|message| JobFailure::new("SESSION_JOB_PDF_OPEN_FAILED", message))?;
    let mut pages = Vec::new();
    let mut total = 0usize;
    for (index, page) in document.pages().iter().enumerate() {
        check_cancelled()?;
        let page_index = u32::try_from(index)
            .map_err(|_| JobFailure::new(NOTES_EXPORT_TOO_LARGE, "Sayfa sayısı sınırı aşıldı"))?;
        let notes = read_page_notes(&page);
        if notes.is_empty() {
            continue;
        }
        total = total.saturating_add(notes.len());
        if total > MAX_NOTES {
            return Err(JobFailure::new(
                NOTES_EXPORT_TOO_LARGE,
                format!("Belgede {MAX_NOTES}'den fazla açıklama var"),
            ));
        }
        let label = crate::document_structure::labels::label(&label_ranges, page_index);
        pages.push(NotesPage {
            page_index,
            label,
            notes,
        });
    }
    check_cancelled()?;
    render_markdown(header, &pages)
}

/// Sayfadaki not taşıyan açıklamalar, sayfadaki sıralarıyla.
pub fn read_page_notes(page: &PdfPage<'_>) -> Vec<Note> {
    let mut text: Option<PdfPageText<'_>> = None;
    let mut notes = Vec::new();
    for annotation in page.annotations().iter() {
        let Some(kind) = NoteKind::from_annotation_type(annotation.annotation_type()) else {
            continue;
        };
        let quoted_text = if kind.is_text_markup() {
            if text.is_none() {
                text = page.text().ok();
            }
            text.as_ref()
                .and_then(|text| markup_text(text, &markup_rects(&annotation)))
        } else {
            None
        };
        let color = crate::annotations::annotation_display_color(&annotation);
        notes.push(Note {
            kind,
            color: crate::annotations::rgb_to_hex(color.red(), color.green(), color.blue()),
            quoted_text,
            contents: annotation.contents(),
        });
    }
    notes
}

/// Text markup açıklamasının kapladığı alanlar: `/QuadPoints`'in her dörtgeninin sınır kutusu,
/// dörtgen yoksa `/Rect`.
fn markup_rects(annotation: &PdfPageAnnotation<'_>) -> Vec<PdfRect> {
    let quads: Vec<PdfRect> = annotation
        .attachment_points()
        .iter()
        .take(MAX_MARKUP_QUADS)
        .map(|quad| quad.to_rect())
        .collect();
    if quads.is_empty() {
        annotation.bounds().ok().into_iter().collect()
    } else {
        quads
    }
}

/// Alanlardaki metin (PDFium `FPDFText_GetBoundedText`), boşlukları tek boşluğa indirilmiş;
/// metin yoksa `None`.
fn markup_text(text: &PdfPageText<'_>, rects: &[PdfRect]) -> Option<String> {
    let joined = rects
        .iter()
        .map(|rect| collapse_whitespace(&text.inside_rect(*rect)))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!joined.is_empty()).then_some(joined)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `value`'nun ilk [`MAX_NOTE_FIELD_CHARS`] karakteri; kesildiyse sonuna `…` eklenir.
fn bounded(value: &str) -> String {
    match value.char_indices().nth(MAX_NOTE_FIELD_CHARS) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.to_string(),
    }
}

/// Satır sonları `\n`'e indirilmiş metin.
fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

/// Bir metin satırı Markdown olarak: işaretler kaçırılır, satır başında liste, başlık ya da
/// setext çizgisi (`===`) okunacak bir şey kalmaz.
fn escape_line(line: &str) -> String {
    let escaped = block_start(md_text(line.trim()));
    if escaped.starts_with('=') {
        format!("\\{escaped}")
    } else {
        escaped
    }
}

/// `#RRGGBB` biçimindeki renk; başka bir şeyse `None`.
fn valid_color(color: &str) -> Option<&str> {
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(color)
}

/// Sayfa başlığı: etiketi sayfa numarasından farklıysa `Sayfa iv (5)`, değilse `Sayfa 5`.
pub fn page_heading(page_index: u32, label: Option<&str>) -> String {
    let number = u64::from(page_index) + 1;
    match label
        .map(str::trim)
        .filter(|label| !label.is_empty() && *label != number.to_string())
    {
        Some(label) => format!("Sayfa {} ({number})", md_text(label)),
        None => format!("Sayfa {number}"),
    }
}

/// `2026-11-10 14:03 (UTC+03:00)`. Tarih mevcut takvim yardımcısıyla
/// ([`crate::annotations::civil_from_days`]) hesaplanır; `utc_offset_minutes`
/// [`NotesExportOptions::validate`]'in sınırları içinde beklenir.
pub fn format_created_at(unix_ms: i64, utc_offset_minutes: i32) -> String {
    let offset_minutes = i64::from(utc_offset_minutes.clamp(-12 * 60, 14 * 60));
    let local_minutes = unix_ms.div_euclid(60_000) + offset_minutes;
    let (year, month, day) = crate::annotations::civil_from_days(local_minutes.div_euclid(1_440));
    let minute_of_day = local_minutes.rem_euclid(1_440);
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let offset = offset_minutes.unsigned_abs();
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} (UTC{sign}{:02}:{:02})",
        minute_of_day / 60,
        minute_of_day % 60,
        offset / 60,
        offset % 60
    )
}

fn push_note(out: &mut String, number: usize, note: &Note) {
    out.push_str(&format!("**{number}. {}**", note.kind.label()));
    if let Some(color) = valid_color(&note.color) {
        out.push_str(&format!(" · Renk: {color}"));
    }
    out.push_str("\n\n");
    if let Some(quote) = note
        .quoted_text
        .as_deref()
        .map(collapse_whitespace)
        .filter(|quote| !quote.is_empty())
    {
        out.push_str("> ");
        out.push_str(&escape_line(&bounded(&quote)));
        out.push_str("\n\n");
    }
    if let Some(contents) = note.contents.as_deref() {
        // Boş satırlar paragrafları ayırır; paragraf içindeki satırlar katı satır sonuyla kalır.
        let contents = normalize_line_endings(&bounded(contents));
        let mut paragraph: Vec<String> = Vec::new();
        let flush = |paragraph: &mut Vec<String>, out: &mut String| {
            if !paragraph.is_empty() {
                out.push_str(&paragraph.join("  \n"));
                out.push_str("\n\n");
                paragraph.clear();
            }
        };
        for line in contents.split('\n') {
            let line = escape_line(line);
            if line.is_empty() {
                flush(&mut paragraph, out);
            } else {
                paragraph.push(line);
            }
        }
        flush(&mut paragraph, out);
    }
}

/// Not dosyası. Hiç not yoksa [`NOTES_EXPORT_EMPTY`], çıktı [`MAX_NOTES_OUTPUT_BYTES`]'ı
/// aşarsa [`NOTES_EXPORT_TOO_LARGE`].
pub fn render_markdown(header: &NotesHeader, pages: &[NotesPage]) -> Result<String, JobFailure> {
    let note_count: usize = pages.iter().map(|page| page.notes.len()).sum();
    let page_count = pages.iter().filter(|page| !page.notes.is_empty()).count();
    if note_count == 0 {
        return Err(JobFailure::new(
            NOTES_EXPORT_EMPTY,
            "Belgede dışa aktarılacak açıklama yok",
        ));
    }
    let title = md_text(header.title.trim());
    let mut out = String::new();
    out.push_str(&format!(
        "# Notlar: {}\n\n",
        if title.is_empty() { "belge" } else { &title }
    ));
    out.push_str(&format!(
        "*Oluşturulma tarihi: {}*\n\n",
        format_created_at(header.created_at_unix_ms, header.utc_offset_minutes)
    ));
    out.push_str(&format!(
        "{note_count} açıklama, {page_count} sayfa. Notlar belgedeki PDF açıklamalarından \
         üretildi; kaydedilmemiş değişiklikler dahildir.\n"
    ));
    for page in pages.iter().filter(|page| !page.notes.is_empty()) {
        out.push_str(&format!(
            "\n## {}\n\n",
            page_heading(page.page_index, page.label.as_deref())
        ));
        for (index, note) in page.notes.iter().enumerate() {
            push_note(&mut out, index + 1, note);
        }
        // Son notun ardından tek boş satır kalsın.
        while out.ends_with("\n\n") {
            out.pop();
        }
    }
    if out.len() as u64 > MAX_NOTES_OUTPUT_BYTES {
        return Err(JobFailure::new(
            NOTES_EXPORT_TOO_LARGE,
            "Not dosyası 16 MiB sınırını aşıyor",
        ));
    }
    Ok(out)
}

/// Test belgesi (lopdf ile üretilir): iki sayfa, `/PageLabels` (i, 1) ve şu açıklamalar:
/// sayfa 1'de `/QuadPoints`'li bir Highlight (1. satır), `/QuadPoints`'siz bir Highlight
/// (2. satır, `/Rect`'ten okunur), `/Rect`'i üç satırı kapsayan ama iki dörtgenli (iki satır)
/// bir StrikeOut, atlanması gereken bir
/// Popup ve bir Link; sayfa 2'de iki satırlık bir Text notu.
#[cfg(test)]
pub(crate) fn notes_fixture_pdf() -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream, StringFormat};

    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let font = document.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let line = |y: i64, value: &str| {
        vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
            Operation::new("Td", vec![72.into(), y.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(
                    value.as_bytes().to_vec(),
                    StringFormat::Literal,
                )],
            ),
            Operation::new("ET", vec![]),
        ]
    };
    let literal = |value: &str| Object::string_literal(value);
    let numbers = |values: &[f32]| -> Object {
        Object::Array(values.iter().map(|value| Object::Real(*value)).collect())
    };
    let mut page = |lines: Vec<(i64, &str)>, annotations: Vec<lopdf::Dictionary>| {
        let operations: Vec<Operation> = lines
            .into_iter()
            .flat_map(|(y, value)| line(y, value))
            .collect();
        let content = document.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let annots: Vec<Object> = annotations
            .into_iter()
            .map(|annotation| document.add_object(annotation).into())
            .collect();
        document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            "Contents" => content,
            "Annots" => annots,
        })
    };
    // Helvetica 12 pt: taban çizgileri y = 700 ve y = 650.
    let first_line = [70.0, 695.0, 250.0, 713.0];
    let second_line = [70.0, 645.0, 250.0, 663.0];
    let quad = |[left, bottom, right, top]: [f32; 4]| {
        vec![left, top, right, top, left, bottom, right, bottom]
    };
    let markup =
        |subtype: &str, rect: [f32; 4], quads: Option<Vec<f32>>, contents: Option<&str>| {
            let mut annotation = dictionary! {
                "Type" => "Annot",
                "Subtype" => subtype.to_string(),
                "Rect" => numbers(&rect),
                "C" => numbers(&[1.0, 1.0, 0.0]),
            };
            if let Some(quads) = quads {
                annotation.set("QuadPoints", numbers(&quads));
            }
            if let Some(contents) = contents {
                annotation.set("Contents", literal(contents));
            }
            annotation
        };
    let mut both_lines = quad(first_line);
    both_lines.extend(quad(second_line));
    let first = page(
        vec![
            (700, "Highlighted passage"),
            (650, "Second line text"),
            (600, "Outside the quads"),
        ],
        vec![
            markup(
                "Highlight",
                first_line,
                Some(quad(first_line)),
                Some("Ilk not"),
            ),
            markup("Highlight", second_line, None, None),
            markup(
                "StrikeOut",
                // `/Rect` üçüncü satırı da kapsar; dörtgenler yalnız ilk ikisini.
                [70.0, 595.0, 250.0, 713.0],
                Some(both_lines),
                Some("iki satir"),
            ),
            dictionary! {
                "Type" => "Annot", "Subtype" => "Popup",
                "Rect" => numbers(&[300.0, 600.0, 400.0, 700.0]),
                "Contents" => literal("popup kopyasi"),
            },
            dictionary! {
                "Type" => "Annot", "Subtype" => "Link",
                "Rect" => numbers(&[70.0, 695.0, 250.0, 713.0]),
                "Contents" => literal("baglanti"),
            },
        ],
    );
    let second = page(
        vec![(700, "Page two body")],
        vec![dictionary! {
            "Type" => "Annot", "Subtype" => "Text",
            "Rect" => numbers(&[100.0, 100.0, 120.0, 120.0]),
            "C" => numbers(&[1.0, 0.0, 0.0]),
            "Contents" => literal("Sayfa iki notu\nikinci satir"),
        }],
    );
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![first.into(), second.into()],
            "Count" => 2,
        }),
    );
    let catalog = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "PageLabels" => dictionary! {
            "Nums" => vec![
                0.into(), dictionary! { "S" => "r" }.into(),
                1.into(), dictionary! { "S" => "D", "St" => 1 }.into(),
            ],
        },
    });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> NotesHeader {
        NotesHeader {
            title: "rapor.pdf".to_string(),
            // 2026-11-10 11:03:00 UTC
            created_at_unix_ms: 1_794_308_580_000,
            utc_offset_minutes: 180,
        }
    }

    fn note(kind: NoteKind, quoted: Option<&str>, contents: Option<&str>) -> Note {
        Note {
            kind,
            color: "#FFFF00".to_string(),
            quoted_text: quoted.map(str::to_string),
            contents: contents.map(str::to_string),
        }
    }

    fn fixture_path(directory: &tempfile::TempDir) -> std::path::PathBuf {
        let path = directory.path().join("snapshot.pdf");
        std::fs::write(&path, notes_fixture_pdf()).unwrap();
        path
    }

    #[test]
    fn reads_highlighted_text_from_quad_points_and_falls_back_to_the_rect() {
        let directory = tempfile::tempdir().unwrap();
        let path = fixture_path(&directory);
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let document = pdfium.load_pdf_from_file(&path, None).unwrap();
        let notes = read_page_notes(&document.pages().get(0).unwrap());
        // Popup ve Link atlanır.
        let summary: Vec<_> = notes
            .iter()
            .map(|note| {
                (
                    note.kind,
                    note.quoted_text.as_deref(),
                    note.contents.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    NoteKind::Highlight,
                    Some("Highlighted passage"),
                    Some("Ilk not")
                ),
                (NoteKind::Highlight, Some("Second line text"), None),
                (
                    NoteKind::StrikeOut,
                    Some("Highlighted passage Second line text"),
                    Some("iki satir")
                ),
            ]
        );
        assert!(notes.iter().all(|note| valid_color(&note.color).is_some()));
        let second = read_page_notes(&document.pages().get(1).unwrap());
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].kind, NoteKind::Text);
        assert_eq!(second[0].quoted_text, None);
        assert_eq!(
            second[0].contents.as_deref(),
            Some("Sayfa iki notu\nikinci satir")
        );
    }

    #[test]
    fn exports_the_snapshot_with_page_labels_and_checks_cancellation_per_page() {
        let directory = tempfile::tempdir().unwrap();
        let path = fixture_path(&directory);
        let mut checks = 0;
        let markdown = export_snapshot(&path, None, &header(), || {
            checks += 1;
            Ok(())
        })
        .unwrap();
        // Başta, her sayfadan önce ve sonda.
        assert_eq!(checks, 4);
        assert!(markdown.contains("4 açıklama, 2 sayfa."));
        assert!(markdown.contains("\n## Sayfa i (1)\n"));
        assert!(markdown.contains("\n## Sayfa 1 (2)\n"));
        assert!(markdown.contains("> Highlighted passage\n\nIlk not\n"));
        assert!(markdown.contains("**1. Metin notu** · Renk: "));
        assert!(markdown.contains("Sayfa iki notu  \nikinci satir\n"));
        assert!(!markdown.contains("popup kopyasi"));
        assert!(!markdown.contains("baglanti"));
        assert!(!markdown.contains("Outside the quads"));

        // İkinci sayfadan önce iptal: sonuç yok.
        let mut checks = 0;
        let cancelled = export_snapshot(&path, None, &header(), || {
            checks += 1;
            if checks == 3 {
                Err(JobFailure::new("SESSION_JOB_WORKER_CANCELLED", "iptal"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(cancelled.code, "SESSION_JOB_WORKER_CANCELLED");
    }

    #[test]
    fn renders_pages_with_labels_kinds_quotes_and_contents() {
        let pages = vec![
            NotesPage {
                page_index: 4,
                label: Some("iv".to_string()),
                notes: vec![
                    note(
                        NoteKind::Highlight,
                        Some("Vurgulanan  metin\n"),
                        Some("Önemli"),
                    ),
                    note(
                        NoteKind::Text,
                        None,
                        Some("satır 1\r\nsatır 2\r\rparagraf 2"),
                    ),
                ],
            },
            NotesPage {
                page_index: 5,
                label: Some("6".to_string()),
                notes: vec![note(NoteKind::Ink, None, None)],
            },
            NotesPage {
                page_index: 7,
                label: None,
                notes: vec![],
            },
        ];
        let markdown = render_markdown(&header(), &pages).unwrap();
        assert_eq!(
            markdown,
            "# Notlar: rapor.pdf\n\n\
             *Oluşturulma tarihi: 2026-11-10 14:03 (UTC+03:00)*\n\n\
             3 açıklama, 2 sayfa. Notlar belgedeki PDF açıklamalarından üretildi; \
             kaydedilmemiş değişiklikler dahildir.\n\
             \n## Sayfa iv (5)\n\n\
             **1. Vurgu** · Renk: #FFFF00\n\n\
             > Vurgulanan metin\n\n\
             Önemli\n\n\
             **2. Metin notu** · Renk: #FFFF00\n\n\
             satır 1  \nsatır 2\n\n\
             paragraf 2\n\
             \n## Sayfa 6\n\n\
             **1. Serbest çizim** · Renk: #FFFF00\n"
        );
        // Deterministik: aynı girdi aynı baytları verir.
        assert_eq!(render_markdown(&header(), &pages).unwrap(), markdown);
    }

    #[test]
    fn escapes_markdown_in_title_quotes_and_contents() {
        let mut header = header();
        header.title = "# [rapor]_*v2*.pdf".to_string();
        let pages = vec![NotesPage {
            page_index: 0,
            label: Some("*A*".to_string()),
            notes: vec![note(
                NoteKind::Underline,
                Some("<b>kalın</b> & `kod`"),
                Some("# başlık değil\n- liste değil\n1. sıra değil\n===\n> alıntı değil\n|tablo|"),
            )],
        }];
        let markdown = render_markdown(&header, &pages).unwrap();
        assert!(markdown.starts_with("# Notlar: \\# \\[rapor\\]\\_\\*v2\\*.pdf\n"));
        assert!(markdown.contains("\n## Sayfa \\*A\\* (1)\n"));
        assert!(markdown.contains("\n> \\<b\\>kalın\\</b\\> \\& \\`kod\\`\n"));
        assert!(markdown.contains(
            "\\# başlık değil  \n\\- liste değil  \n1\\. sıra değil  \n\\===  \n\\> alıntı değil  \n\\|tablo\\|\n"
        ));
        // Not metninden gelen hiçbir satır blok işaretiyle başlamaz.
        let body = markdown.split("**1. Altı çizili**").nth(1).unwrap();
        for line in body.lines().filter(|line| !line.starts_with("> ")) {
            assert!(
                !line.starts_with(['#', '-', '=', '>', '|', '*']),
                "kaçırılmamış satır: {line:?}"
            );
        }
    }

    #[test]
    fn empty_documents_are_rejected_and_invalid_colors_are_omitted() {
        let empty = render_markdown(
            &header(),
            &[NotesPage {
                page_index: 0,
                label: None,
                notes: vec![],
            }],
        )
        .unwrap_err();
        assert_eq!(empty.code, NOTES_EXPORT_EMPTY);
        assert_eq!(
            render_markdown(&header(), &[]).unwrap_err().code,
            NOTES_EXPORT_EMPTY
        );

        let mut odd = note(NoteKind::Text, None, Some("x"));
        odd.color = "red](javascript:x)".to_string();
        let markdown = render_markdown(
            &header(),
            &[NotesPage {
                page_index: 0,
                label: None,
                notes: vec![odd],
            }],
        )
        .unwrap();
        assert!(markdown.contains("**1. Metin notu**\n\nx\n"));
        assert!(!markdown.contains("javascript"));
    }

    #[test]
    fn long_fields_are_bounded() {
        let long = "a".repeat(MAX_NOTE_FIELD_CHARS + 10);
        let markdown = render_markdown(
            &header(),
            &[NotesPage {
                page_index: 0,
                label: None,
                notes: vec![note(NoteKind::Text, None, Some(&long))],
            }],
        )
        .unwrap();
        assert!(markdown.contains(&format!("{}…\n", "a".repeat(MAX_NOTE_FIELD_CHARS))));
        assert!(!markdown.contains(&"a".repeat(MAX_NOTE_FIELD_CHARS + 1)));
    }

    #[test]
    fn page_headings_show_labels_that_differ_from_the_number() {
        assert_eq!(page_heading(4, Some("iv")), "Sayfa iv (5)");
        assert_eq!(page_heading(4, Some("5")), "Sayfa 5");
        assert_eq!(page_heading(4, Some("  ")), "Sayfa 5");
        assert_eq!(page_heading(4, None), "Sayfa 5");
        assert_eq!(page_heading(0, Some("A-1")), "Sayfa A-1 (1)");
    }

    #[test]
    fn created_at_uses_the_requested_offset() {
        assert_eq!(
            format_created_at(1_794_308_580_000, 0),
            "2026-11-10 11:03 (UTC+00:00)"
        );
        assert_eq!(
            format_created_at(1_794_308_580_000, -330),
            "2026-11-10 05:33 (UTC-05:30)"
        );
    }

    #[test]
    fn options_reject_impossible_offsets_and_unknown_fields() {
        let options = |value| serde_json::from_value::<NotesExportOptions>(value);
        assert_eq!(
            options(serde_json::json!({})).unwrap(),
            NotesExportOptions::default()
        );
        let valid = options(serde_json::json!({ "utcOffsetMinutes": 180 })).unwrap();
        assert!(valid.validate().is_ok());
        for offset in [-721, 841] {
            let invalid = options(serde_json::json!({ "utcOffsetMinutes": offset })).unwrap();
            assert!(invalid.validate().is_err());
        }
        assert!(options(serde_json::json!({ "markdown": "# x" })).is_err());
        assert!(options(serde_json::json!({ "drawings": [] })).is_err());
    }

    #[test]
    fn kinds_map_to_turkish_labels_and_skip_non_note_types() {
        use PdfPageAnnotationType as T;
        let cases = [
            (T::Text, "Metin notu"),
            (T::FreeText, "Serbest metin"),
            (T::Highlight, "Vurgu"),
            (T::Underline, "Altı çizili"),
            (T::Strikeout, "Üstü çizili"),
            (T::Squiggly, "Dalgalı altı çizili"),
            (T::Square, "Dikdörtgen"),
            (T::Circle, "Elips"),
            (T::Ink, "Serbest çizim"),
            (T::Stamp, "Damga"),
            (T::Unknown, "Diğer açıklama"),
        ];
        for (annotation_type, label) in cases {
            assert_eq!(
                NoteKind::from_annotation_type(annotation_type).map(NoteKind::label),
                Some(label),
                "{annotation_type:?}"
            );
        }
        for annotation_type in [T::Popup, T::Link, T::Widget, T::XfaWidget] {
            assert!(SKIPPED_ANNOTATION_TYPES.contains(&annotation_type));
        }
        for annotation_type in SKIPPED_ANNOTATION_TYPES {
            assert_eq!(NoteKind::from_annotation_type(*annotation_type), None);
        }
        let markup: Vec<_> = [
            T::Highlight,
            T::Underline,
            T::Strikeout,
            T::Squiggly,
            T::Text,
        ]
        .into_iter()
        .filter_map(NoteKind::from_annotation_type)
        .filter(|kind| kind.is_text_markup())
        .collect();
        assert_eq!(markup.len(), 4);
    }
}
