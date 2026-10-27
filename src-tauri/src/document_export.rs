//! Belgeyi Word (.docx), PowerPoint (.pptx), HTML ya da RTF olarak dışa aktarma.
//!
//! Job worker'da çalışır. PDFium her sayfayı PdfCraft yazıcılarının modeline indirir
//! ([`writers::Page`]): biçimi (punto, kalın, italik, renk, açı) aynı kalan metin parçaları
//! paragraflara birleştirilir; sayfadaki görüntüler PNG olarak alınır. Yazıcılar
//! ([`writers`], [`pptx`]) PdfCraft 0.14.0'dan taşınmıştır: başlıkları punto farkından, tabloları
//! hizalanmış kısa bloklardan bulurlar.
//!
//! Sınırlar: form XObject içindeki görüntüler ve sayfa arka plan rengi alınmaz; metin
//! katmanı olmayan (taranmış) sayfalar yalnız görüntüleriyle çıkar. Office paketleri
//! sıkıştırılmadan (stored) yazılır.

mod pptx;
mod writers;
mod zip;

use crate::job_worker::JobFailure;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use writers::{Block, Image, Page};

/// Bir belgeden alınacak en fazla görüntü ve kodlanmış toplam boyutları. Yazıcılar Word
/// paketinde de 1000 görüntü ve 512 MiB ile sınırlıdır.
const MAX_IMAGES: usize = 1000;
const MAX_IMAGE_BYTES: usize = 256 << 20;
/// Bundan büyük bir görüntü (piksel) alınmaz: PNG'ye kodlamak worker belleğini zorlar.
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentExportFormat {
    Docx,
    Pptx,
    Html,
    Rtf,
}

impl DocumentExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Html => "html",
            Self::Rtf => "rtf",
        }
    }
}

/// Belgenin `pages` sayfalarını (fiziksel, bu sırayla) `format`'ta yazar; `title` belgenin
/// başlığı olur.
pub(crate) fn export_document(
    pdfium: &Pdfium,
    document: &PdfDocument<'_>,
    pages: &[u32],
    format: DocumentExportFormat,
    title: &str,
) -> Result<Vec<u8>, JobFailure> {
    let pages = read_pages(pdfium, document, pages)?;
    Ok(write(&pages, format, title))
}

fn write(pages: &[Page], format: DocumentExportFormat, title: &str) -> Vec<u8> {
    match format {
        DocumentExportFormat::Docx => writers::docx(pages, title),
        DocumentExportFormat::Pptx => pptx::pptx(pages, title),
        DocumentExportFormat::Html => writers::html(pages, title).into_bytes(),
        DocumentExportFormat::Rtf => writers::rtf(pages).into_bytes(),
    }
}

/// Sayfanın görüntülendiği hâline (`/Rotate` uygulanmış, sol alt köken) dönüşüm.
#[derive(Clone, Copy, Debug)]
struct Frame {
    left: f64,
    bottom: f64,
    width: f64,
    height: f64,
    rotation: u16,
}

impl Frame {
    fn displayed_size(self) -> (f64, f64) {
        if self.rotation % 180 == 90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    fn point(self, x: f64, y: f64) -> (f64, f64) {
        let (u, v) = (x - self.left, y - self.bottom);
        match self.rotation {
            90 => (v, self.width - u),
            180 => (self.width - u, self.height - v),
            270 => (self.height - v, u),
            _ => (u, v),
        }
    }

    /// `[x0, y0, x1, y1]` in page space → the displayed page's axis-aligned box.
    fn rect(self, [x0, y0, x1, y1]: [f64; 4]) -> [f64; 4] {
        let (a, b) = (self.point(x0, y0), self.point(x1, y1));
        [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
    }
}

fn frame(pdfium: &Pdfium, page: &PdfPage<'_>) -> Frame {
    if let Some(geometry) = crate::redaction::extract_page_geometry(pdfium, page) {
        let b = geometry.effective_box;
        return Frame {
            left: b.left.into(),
            bottom: b.bottom.into(),
            width: b.width.into(),
            height: b.height.into(),
            rotation: geometry.intrinsic_rotation.as_u16(),
        };
    }
    Frame {
        left: 0.0,
        bottom: 0.0,
        width: page.width().value.into(),
        height: page.height().value.into(),
        rotation: 0,
    }
}

fn read_pages(
    pdfium: &Pdfium,
    document: &PdfDocument<'_>,
    selection: &[u32],
) -> Result<Vec<Page>, JobFailure> {
    if selection.is_empty() || selection.len() > crate::job_worker::MAX_JOB_PAGES {
        return Err(JobFailure::new(
            "SESSION_JOB_INVALID_PAGE_SELECTION",
            "Dışa aktarılacak sayfa seçimi boş ya da sınırı aşıyor",
        ));
    }
    let mut budget = Budget::default();
    let mut pages = Vec::with_capacity(selection.len());
    for &index in selection {
        let page = u16::try_from(index)
            .ok()
            .and_then(|index| document.pages().get(index).ok())
            .ok_or_else(|| {
                JobFailure::new(
                    "SESSION_JOB_INVALID_PAGE_SELECTION",
                    format!("Belgede {}. sayfa yok", u64::from(index) + 1),
                )
            })?;
        let frame = frame(pdfium, &page);
        let text = page.text().map_err(|error| {
            JobFailure::new(
                "SESSION_JOB_EXPORT_TEXT_FAILED",
                format!(
                    "{}. sayfanın metni okunamadı: {error}",
                    u64::from(index) + 1
                ),
            )
        })?;
        let (width, height) = frame.displayed_size();
        pages.push(Page {
            width,
            height,
            blocks: paragraphs(page_segments(&text, frame, &mut budget)?),
            images: page_images(document, &page, frame, &mut budget),
            background: None,
        });
    }
    Ok(pages)
}

#[derive(Default)]
struct Budget {
    chars: usize,
    images: usize,
    image_bytes: usize,
}

/// What makes two pieces of text one block: the same look and direction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Style {
    size: f64,
    bold: bool,
    italic: bool,
    color: [f64; 3],
    rotation: f64,
}

/// A run of same-styled text on one line.
#[derive(Clone, Debug, PartialEq)]
struct Segment {
    text: String,
    rect: [f64; 4],
    style: Style,
    starts_line: bool,
    ends_line: bool,
}

fn char_style(character: &PdfPageTextChar<'_>, page_rotation: u16) -> Style {
    let name = character.font_name().to_ascii_lowercase();
    let weight = match character.font_weight() {
        Some(PdfFontWeight::Weight600)
        | Some(PdfFontWeight::Weight700Bold)
        | Some(PdfFontWeight::Weight800)
        | Some(PdfFontWeight::Weight900) => true,
        Some(PdfFontWeight::Custom(weight)) => weight >= 600,
        _ => false,
    };
    let bold = weight
        || ["bold", "black", "heavy", "semibold", "demi"]
            .iter()
            .any(|part| name.contains(part));
    let italic = character.font_is_italic() || name.contains("italic") || name.contains("oblique");
    let color = character.fill_color().map_or([0.0; 3], |color| {
        [color.red(), color.green(), color.blue()].map(|c| f64::from(c) / 255.0)
    });
    let angle = f64::from(character.angle_degrees().unwrap_or(0.0)) - f64::from(page_rotation);
    let mut rotation = angle.rem_euclid(360.0);
    if rotation > 180.0 {
        rotation -= 360.0;
    }
    if rotation.abs() < 0.5 || (360.0 - rotation.abs()) < 0.5 {
        rotation = 0.0;
    }
    Style {
        size: (f64::from(character.scaled_font_size().value) * 2.0).round() / 2.0,
        bold,
        italic,
        color,
        rotation,
    }
}

/// The page's text as same-styled runs per line, in PDFium's reading order.
fn page_segments(
    text: &PdfPageText<'_>,
    frame: Frame,
    budget: &mut Budget,
) -> Result<Vec<Segment>, JobFailure> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut current: Option<Segment> = None;
    let mut line_break = true;
    let mut space = false;
    let mut last: Option<[f64; 4]> = None;
    for character in text.chars().iter() {
        let value = match character.unicode_char() {
            Some('\u{00A0}') => ' ',
            Some('\u{00AD}' | '\u{200B}' | '\0') | None => continue,
            Some(value) => value,
        };
        if value == '\r' || value == '\n' {
            line_break = true;
            continue;
        }
        if value.is_whitespace() {
            space = true;
            continue;
        }
        let Ok(bounds) = character.loose_bounds() else {
            continue;
        };
        let raw = [
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        ]
        .map(f64::from);
        if raw.iter().any(|v| !v.is_finite()) || raw[2] < raw[0] || raw[3] < raw[1] {
            continue;
        }
        budget.chars += 1;
        if budget.chars > crate::job_worker::MAX_JOB_TEXT_CHARS {
            return Err(JobFailure::new(
                "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
                "Belgenin metni dışa aktarma sınırını aşıyor",
            ));
        }
        let rect = frame.rect(raw);
        let style = char_style(&character, frame.rotation);
        let size = style.size.max(1.0);
        // A line ends where PDFium says so, or (for upright text) where the next character
        // no longer shares the line's height or jumps back to the left.
        if let (Some(prev), false) = (last, line_break) {
            if style.rotation == 0.0 {
                let overlap = prev[3].min(rect[3]) - prev[1].max(rect[1]);
                let shorter = (prev[3] - prev[1]).min(rect[3] - rect[1]).max(0.1);
                if overlap < 0.4 * shorter || rect[0] < prev[0] - size {
                    line_break = true;
                }
            }
            if !line_break && rect[0] - prev[2] > 0.25 * size {
                space = true;
            }
        }
        let continues = current
            .as_ref()
            .is_some_and(|segment| !line_break && segment.style == style);
        if continues {
            if let Some(segment) = current.as_mut() {
                if space {
                    segment.text.push(' ');
                }
                segment.text.push(value);
                segment.rect = union(segment.rect, rect);
            }
        } else {
            if let Some(mut done) = current.take() {
                done.ends_line = line_break;
                segments.push(done);
            }
            current = Some(Segment {
                text: value.to_string(),
                rect,
                style,
                starts_line: line_break,
                ends_line: true,
            });
        }
        line_break = false;
        space = false;
        last = Some(rect);
    }
    if let Some(done) = current {
        segments.push(done);
    }
    Ok(segments)
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Blocks from segments: whole upright lines in the same style, one under the other with
/// ordinary line spacing and overlapping horizontally, become one paragraph.
fn paragraphs(segments: Vec<Segment>) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    // Whether the last block is made of whole lines, so a following line may join it.
    let mut open = false;
    for segment in segments {
        let whole_line = segment.starts_line && segment.ends_line;
        let joins = open
            && whole_line
            && segment.style.rotation == 0.0
            && blocks.last().is_some_and(|block| {
                let size = segment.style.size.max(1.0);
                let same = block.size == segment.style.size
                    && block.bold == segment.style.bold
                    && block.italic == segment.style.italic
                    && block.color == segment.style.color
                    && block.rotation == 0.0;
                let gap = block.rect[1] - segment.rect[3];
                let overlap =
                    block.rect[2].min(segment.rect[2]) - block.rect[0].max(segment.rect[0]);
                same && (-0.2 * size..=0.9 * size).contains(&gap) && overlap > 0.0
            });
        if joins {
            if let Some(block) = blocks.last_mut() {
                let hyphenated = block.text.ends_with('-')
                    && block
                        .text
                        .chars()
                        .rev()
                        .nth(1)
                        .is_some_and(char::is_alphabetic);
                if hyphenated {
                    block.text.pop();
                } else {
                    block.text.push(' ');
                }
                block.text.push_str(&segment.text);
                block.rect = union(block.rect, segment.rect);
            }
            continue;
        }
        open = whole_line && segment.style.rotation == 0.0;
        blocks.push(Block {
            text: segment.text,
            rect: segment.rect,
            size: segment.style.size,
            bold: segment.style.bold,
            italic: segment.style.italic,
            color: segment.style.color,
            rotation: segment.style.rotation,
        });
    }
    blocks
}

/// The page's own image objects as PNG; images past the budget or too large are left out.
fn page_images(
    document: &PdfDocument<'_>,
    page: &PdfPage<'_>,
    frame: Frame,
    budget: &mut Budget,
) -> Vec<Image> {
    let mut images = Vec::new();
    for object in page.objects().iter() {
        let Some(image) = object.as_image_object() else {
            continue;
        };
        if budget.images >= MAX_IMAGES {
            break;
        }
        let (Ok(width), Ok(height)) = (image.width(), image.height()) else {
            continue;
        };
        let pixels = u64::from(width.unsigned_abs()) * u64::from(height.unsigned_abs());
        if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
            continue;
        }
        let (Ok(bounds), Ok(matrix)) = (object.bounds(), image.matrix()) else {
            continue;
        };
        let Ok(bitmap) = image.get_processed_image(document) else {
            continue;
        };
        let mut png = std::io::Cursor::new(Vec::new());
        if bitmap.write_to(&mut png, image::ImageFormat::Png).is_err() {
            continue;
        }
        let png = png.into_inner();
        if budget.image_bytes + png.len() > MAX_IMAGE_BYTES {
            continue;
        }
        budget.images += 1;
        budget.image_bytes += png.len();
        let rect = bounds.to_rect();
        let (a, b, c, d) = (
            f64::from(matrix.a()),
            f64::from(matrix.b()),
            f64::from(matrix.c()),
            f64::from(matrix.d()),
        );
        let rotation = (b.atan2(a).to_degrees() - f64::from(frame.rotation)).rem_euclid(360.0);
        images.push(Image {
            ext: "png",
            bytes: Arc::from(png),
            rect: frame.rect(
                [
                    rect.left().value,
                    rect.bottom().value,
                    rect.right().value,
                    rect.top().value,
                ]
                .map(f64::from),
            ),
            size: [a.hypot(b), c.hypot(d)],
            rotation: if rotation > 180.0 {
                rotation - 360.0
            } else {
                rotation
            },
        });
    }
    images
}

/// Test PDF'i: kalın 20 pt bir başlık, iki satırlık 11 pt bir paragraf ve 2×2 piksellik bir
/// görüntü (sayfada 100×50 pt, (72, 500)'de).
#[cfg(test)]
pub(crate) fn sample_pdf(rotate: i64) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream, StringFormat};

    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let font = |name: &str| dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => name.to_string() };
    let regular = document.add_object(font("Helvetica"));
    let bold = document.add_object(font("Helvetica-Bold"));
    let image = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 2,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
        },
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
    ));
    let text = |font: &str, size: i64, y: i64, value: &str| {
        vec![
            Operation::new("BT", vec![]),
            Operation::new(
                "Tf",
                vec![Object::Name(font.as_bytes().to_vec()), size.into()],
            ),
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
    let mut operations = Vec::new();
    operations.extend(text("F2", 20, 720, "Quarterly Report"));
    operations.extend(text("F1", 11, 690, "This is the first line and"));
    operations.extend(text("F1", 11, 677, "this is the second line."));
    operations.extend([
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                100.into(),
                0.into(),
                0.into(),
                50.into(),
                72.into(),
                500.into(),
            ],
        ),
        Operation::new("Do", vec![Object::Name(b"Im1".to_vec())]),
        Operation::new("Q", vec![]),
    ]);
    let content = document.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode().unwrap(),
    ));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Rotate" => rotate,
        "Resources" => dictionary! {
            "Font" => dictionary! { "F1" => regular, "F2" => bold },
            "XObject" => dictionary! { "Im1" => image },
        },
        "Contents" => content,
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPRIGHT: Style = Style {
        size: 12.0,
        bold: false,
        italic: false,
        color: [0.0; 3],
        rotation: 0.0,
    };

    fn line(text: &str, y: f64, style: Style) -> Segment {
        Segment {
            text: text.into(),
            rect: [72.0, y, 72.0 + 6.0 * text.len() as f64, y + 12.0],
            style,
            starts_line: true,
            ends_line: true,
        }
    }

    fn sample_pages(rotate: i64) -> Vec<Page> {
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let bytes = sample_pdf(rotate);
        let document = pdfium.load_pdf_from_byte_slice(&bytes, None).unwrap();
        read_pages(&pdfium, &document, &[0]).unwrap()
    }

    #[test]
    fn pdfium_reads_headings_paragraphs_and_images() {
        let pages = sample_pages(0);
        let page = &pages[0];
        assert_eq!((page.width, page.height), (612.0, 792.0));
        let got: Vec<_> = page
            .blocks
            .iter()
            .map(|b| (b.text.as_str(), b.size, b.bold))
            .collect();
        assert_eq!(
            got,
            [
                ("Quarterly Report", 20.0, true),
                (
                    "This is the first line and this is the second line.",
                    11.0,
                    false
                ),
            ]
        );
        assert_eq!(page.images.len(), 1);
        let image = &page.images[0];
        assert_eq!(image.rect, [72.0, 500.0, 172.0, 550.0]);
        assert_eq!(image.size, [100.0, 50.0]);
        assert!(image.bytes.starts_with(b"\x89PNG"));
    }

    #[test]
    fn exports_are_word_powerpoint_html_and_rtf() {
        let pages = sample_pages(0);
        let docx = write(&pages, DocumentExportFormat::Docx, "Rapor");
        let document = writers::tests::part(&docx, "word/document.xml");
        assert!(
            document.contains("<w:pStyle w:val=\"Heading1\"/>"),
            "{document}"
        );
        assert!(document.contains("This is the first line and this is the second line."));
        assert!(docx
            .windows(b"word/media/image1.png".len())
            .any(|w| w == b"word/media/image1.png"));
        let slide = writers::tests::part(
            &write(&pages, DocumentExportFormat::Pptx, "Rapor"),
            "ppt/slides/slide1.xml",
        );
        assert!(slide.contains("Quarterly Report"), "{slide}");
        let html = String::from_utf8(write(&pages, DocumentExportFormat::Html, "Rapor")).unwrap();
        assert!(
            html.contains("<title>Rapor</title>") && html.contains("<h1>Quarterly Report</h1>"),
            "{html}"
        );
        assert!(html.contains("data:image/png;base64,"));
        let rtf = String::from_utf8(write(&pages, DocumentExportFormat::Rtf, "Rapor")).unwrap();
        assert!(
            rtf.starts_with("{\\rtf1") && rtf.contains("\\b Quarterly Report"),
            "{rtf}"
        );
    }

    #[test]
    fn rotated_pages_are_exported_as_displayed() {
        let pages = sample_pages(90);
        let page = &pages[0];
        assert_eq!((page.width, page.height), (792.0, 612.0));
        assert_eq!(page.images[0].rect, [500.0, 440.0, 550.0, 540.0]);
        // The text reads top to bottom on the displayed page, so it is rotated there.
        assert!(
            page.blocks.iter().all(|b| b.rotation == -90.0),
            "{:?}",
            page.blocks
        );
    }

    #[test]
    fn rotated_frames_map_page_space_to_the_displayed_page() {
        let frame = |rotation| Frame {
            left: 10.0,
            bottom: 20.0,
            width: 100.0,
            height: 200.0,
            rotation,
        };
        // The page's top-left corner, (10, 220).
        assert_eq!(frame(0).point(10.0, 220.0), (0.0, 200.0));
        assert_eq!(frame(90).point(10.0, 220.0), (200.0, 100.0));
        assert_eq!(frame(180).point(10.0, 220.0), (100.0, 0.0));
        assert_eq!(frame(270).point(10.0, 220.0), (0.0, 0.0));
        assert_eq!(frame(90).displayed_size(), (200.0, 100.0));
        assert_eq!(
            frame(90).rect([10.0, 20.0, 20.0, 40.0]),
            [0.0, 90.0, 20.0, 100.0]
        );
    }

    #[test]
    fn whole_lines_in_one_style_join_into_paragraphs() {
        let heading = Style {
            size: 20.0,
            bold: true,
            ..UPRIGHT
        };
        let blocks = paragraphs(vec![
            Segment {
                rect: [72.0, 700.0, 300.0, 720.0],
                ..line("Başlık", 700.0, heading)
            },
            line("The first line of a para-", 680.0, UPRIGHT),
            line("graph goes on here.", 666.0, UPRIGHT),
            line("Far below: a new one.", 600.0, UPRIGHT),
        ]);
        let got: Vec<_> = blocks.iter().map(|b| (b.text.as_str(), b.bold)).collect();
        assert_eq!(
            got,
            [
                ("Başlık", true),
                ("The first line of a paragraph goes on here.", false),
                ("Far below: a new one.", false),
            ]
        );
        assert_eq!(blocks[1].rect[1], 666.0);
    }

    #[test]
    fn a_style_change_inside_a_line_is_a_block_of_its_own() {
        let bold = Style {
            bold: true,
            ..UPRIGHT
        };
        let mut start = line("Read the", 700.0, UPRIGHT);
        start.ends_line = false;
        let mut word = line("manual", 700.0, bold);
        word.starts_line = false;
        word.ends_line = false;
        let mut end = line("today.", 700.0, UPRIGHT);
        end.starts_line = false;
        let next = line("Next line.", 686.0, UPRIGHT);
        let blocks = paragraphs(vec![start, word, end, next]);
        let got: Vec<_> = blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(got, ["Read the", "manual", "today.", "Next line."]);
    }

    #[test]
    fn rotated_text_never_joins() {
        let turned = Style {
            rotation: 90.0,
            ..UPRIGHT
        };
        let blocks = paragraphs(vec![line("one", 700.0, turned), line("two", 686.0, turned)]);
        assert_eq!(blocks.len(), 2);
    }
}
