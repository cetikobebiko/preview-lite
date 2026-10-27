//! Belge karşılaştırma: açık belgenin metni ile seçilen bir PDF'in metni, kelime düzeyinde.
//!
//! Job worker'da çalışır. İki belge de PDFium ile açılır; kelimeler metin katmanıyla aynı
//! kurala göre ayrılır ([`crate::worker::pdfium::for_each_word`]) ve [`diff::compare`] ile
//! karşılaştırılır. Değişikliklerin dikdörtgenleri sayfanın etkin kutusuna göre normalize
//! edilir (sol üst köken, `/Rotate` uygulanmadan), böylece arayüz onları hassas veri
//! vurgularıyla aynı yoldan sayfaya çizer.
//!
//! Yalnız metin karşılaştırılır: görsel farklar ([`diff::visual_regions`]) henüz arayüze
//! bağlı değildir. Metin katmanı olmayan (taranmış) sayfalar kelime vermez.

pub mod diff;

use crate::annotations::NormalizedPageRect;
use crate::job_worker::JobFailure;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

/// Bir belgeden okunacak en fazla kelime. Kelime başına ~100 bayt ile iki belge birlikte
/// ~100 MB eder; diff'in kendisi bundan küçüktür.
pub(crate) const MAX_COMPARE_WORDS: usize = 500_000;
/// Arayüze gönderilen en fazla değişiklik; fazlası `totalChanges` ile sayılır.
pub(crate) const MAX_COMPARE_CHANGES: usize = 5_000;
const MAX_SIDE_TEXT_CHARS: usize = 1_000;
const MAX_SIDE_RECTS: usize = 200;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Inserted,
    Deleted,
    Replaced,
}

/// Bir değişikliğin bir belgedeki tarafı.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComparedSide {
    /// Bu taraftaki kelimeler; ekleme ya da silmenin karşı tarafında boştur.
    pub text: String,
    pub text_truncated: bool,
    /// 0 tabanlı fiziksel sayfa. Boş tarafta, kelimelerin olacağı yerin sayfası.
    pub page_index: u32,
    /// Sayfanın `/Rotate` değeri; arayüz dikdörtgeni görünüme göre döndürürken kullanır.
    pub intrinsic_rotation: u16,
    /// Satır başına birleştirilmiş kelime kutuları, sayfanın etkin kutusuna göre normalize.
    pub rects: Vec<NormalizedPageRect>,
    /// Boş tarafta: kelimelerin olacağı yerin yanındaki kelimenin kutusu.
    pub near: Option<NormalizedPageRect>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComparedChange {
    pub kind: ChangeKind,
    pub old: ComparedSide,
    pub new: ComparedSide,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentComparison {
    /// Açık belge eski sürüm mü (`true`), yeni sürüm mü.
    pub current_is_old: bool,
    pub changes: Vec<ComparedChange>,
    /// Bulunan tüm değişiklikler; `changes` en fazla [`MAX_COMPARE_CHANGES`] tanesini taşır.
    pub total_changes: usize,
    pub old_words: usize,
    pub new_words: usize,
}

/// Bir sayfanın etkin kutusu (`left, bottom, width, height`, point) ve `/Rotate` değeri.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PageFrame {
    effective_box: [f64; 4],
    rotation: u16,
}

#[derive(Debug, Default)]
struct DocumentWords {
    words: Vec<diff::Word>,
    /// Sayfa başına; geometrisi okunamayan sayfa `None` (kelimeleri dikdörtgensiz kalır).
    pages: Vec<Option<PageFrame>>,
}

/// Açık belgeyi (`current`) seçilen belgeyle (`other`) karşılaştırır.
pub(crate) fn compare_documents(
    pdfium: &Pdfium,
    current: &PdfDocument<'_>,
    other: &PdfDocument<'_>,
    current_is_old: bool,
) -> Result<DocumentComparison, JobFailure> {
    let current_words = read_words(pdfium, current, "Açık belgenin")?;
    let other_words = read_words(pdfium, other, "Seçilen belgenin")?;
    let (old, new) = if current_is_old {
        (current_words, other_words)
    } else {
        (other_words, current_words)
    };
    Ok(comparison_of(&old, &new, current_is_old))
}

fn comparison_of(
    old: &DocumentWords,
    new: &DocumentWords,
    current_is_old: bool,
) -> DocumentComparison {
    let found = diff::compare(&old.words, &new.words);
    let total_changes = found.changes.len();
    let changes = found
        .changes
        .iter()
        .take(MAX_COMPARE_CHANGES)
        .map(|change| ComparedChange {
            kind: match change.kind {
                diff::Kind::Inserted => ChangeKind::Inserted,
                diff::Kind::Deleted => ChangeKind::Deleted,
                diff::Kind::Replaced => ChangeKind::Replaced,
            },
            old: side(&change.old, &old.pages),
            new: side(&change.new, &new.pages),
        })
        .collect();
    DocumentComparison {
        current_is_old,
        changes,
        total_changes,
        old_words: old.words.len(),
        new_words: new.words.len(),
    }
}

fn side(side: &diff::Side, pages: &[Option<PageFrame>]) -> ComparedSide {
    let frame = pages.get(side.page).copied().flatten();
    let mut text: String = side.text.chars().take(MAX_SIDE_TEXT_CHARS).collect();
    let text_truncated = text.len() < side.text.len();
    if text_truncated {
        text.push('…');
    }
    let normalize = |rect: &[f64; 4]| frame.and_then(|frame| normalized(rect, frame));
    ComparedSide {
        text,
        text_truncated,
        page_index: u32::try_from(side.page).unwrap_or(u32::MAX),
        intrinsic_rotation: frame.map_or(0, |frame| frame.rotation),
        rects: side
            .rects
            .iter()
            .take(MAX_SIDE_RECTS)
            .filter_map(normalize)
            .collect(),
        near: side.near.as_ref().and_then(normalize),
    }
}

/// `[x0, y0, x1, y1]` (point, sol alt köken) → etkin kutuya göre `[0, 1]`, sol üst köken.
fn normalized(rect: &[f64; 4], frame: PageFrame) -> Option<NormalizedPageRect> {
    let [left, bottom, width, height] = frame.effective_box;
    let top = bottom + height;
    let out = NormalizedPageRect {
        x: ((rect[0] - left) / width) as f32,
        y: ((top - rect[3]) / height) as f32,
        width: ((rect[2] - rect[0]) / width) as f32,
        height: ((rect[3] - rect[1]) / height) as f32,
    };
    let finite = [out.x, out.y, out.width, out.height]
        .iter()
        .all(|value| value.is_finite());
    (finite && out.width > 0.0 && out.height > 0.0).then_some(out)
}

fn read_words(
    pdfium: &Pdfium,
    document: &PdfDocument<'_>,
    label: &str,
) -> Result<DocumentWords, JobFailure> {
    let page_count = usize::from(document.pages().len());
    if page_count > crate::job_worker::MAX_JOB_PAGES {
        return Err(JobFailure::new(
            "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
            format!("{label} sayfa sayısı karşılaştırma sınırını aşıyor"),
        ));
    }
    let mut out = DocumentWords {
        words: Vec::new(),
        pages: Vec::with_capacity(page_count),
    };
    for (page_index, page) in document.pages().iter().enumerate() {
        out.pages.push(page_frame(pdfium, &page));
        let text = page.text().map_err(|error| {
            JobFailure::new(
                "SESSION_JOB_COMPARE_TEXT_FAILED",
                format!(
                    "{label} {}. sayfasının metni okunamadı: {error}",
                    page_index + 1
                ),
            )
        })?;
        crate::worker::pdfium::for_each_word(&text, |word, [x0, y0, x1, y1]| {
            if word.is_empty() || x0 >= x1 || y0 >= y1 {
                return Ok(());
            }
            if out.words.len() >= MAX_COMPARE_WORDS {
                return Err(JobFailure::new(
                    "SESSION_JOB_COMPARE_WORD_LIMIT_EXCEEDED",
                    format!(
                        "{label} kelime sayısı karşılaştırma sınırını ({MAX_COMPARE_WORDS}) aşıyor"
                    ),
                ));
            }
            out.words.push(diff::Word {
                text: word.to_string(),
                page: page_index,
                rect: [x0.into(), y0.into(), x1.into(), y1.into()],
            });
            Ok(())
        })?;
    }
    Ok(out)
}

fn page_frame(pdfium: &Pdfium, page: &PdfPage<'_>) -> Option<PageFrame> {
    let geometry = crate::redaction::extract_page_geometry(pdfium, page)?;
    let effective = geometry.effective_box;
    Some(PageFrame {
        effective_box: [
            effective.left.into(),
            effective.bottom.into(),
            effective.width.into(),
            effective.height.into(),
        ],
        rotation: geometry.intrinsic_rotation.as_u16(),
    })
}

/// Test PDF'i: her sayfa satır satır Helvetica 12 pt metin (ilk satır taban çizgisi y=720,
/// x=72), isteğe bağlı CropBox ve `/Rotate` ile.
#[cfg(test)]
pub(crate) struct TestPage<'a> {
    pub lines: &'a [&'a str],
    pub crop_box: Option<[i64; 4]>,
    pub rotate: i64,
}

#[cfg(test)]
pub(crate) fn text_pdf(pages: &[TestPage<'_>]) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream, StringFormat};

    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let mut kids = Vec::new();
    for page in pages {
        let mut operations = Vec::new();
        for (line, text) in page.lines.iter().enumerate() {
            operations.extend([
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Td", vec![72.into(), (720 - 20 * line as i64).into()]),
                Operation::new(
                    "Tj",
                    vec![Object::String(
                        text.as_bytes().to_vec(),
                        StringFormat::Literal,
                    )],
                ),
                Operation::new("ET", vec![]),
            ]);
        }
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let mut dict = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            "Contents" => content_id,
            "Rotate" => page.rotate,
        };
        if let Some(crop) = page.crop_box {
            dict.set(
                "CropBox",
                crop.iter().map(|&v| v.into()).collect::<Vec<Object>>(),
            );
        }
        kids.push(Object::Reference(document.add_object(dict)));
    }
    let count = kids.len() as i64;
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: PageFrame = PageFrame {
        effective_box: [0.0, 0.0, 600.0, 800.0],
        rotation: 0,
    };

    fn document(pages: &[&str]) -> DocumentWords {
        let mut out = DocumentWords::default();
        for (page, text) in pages.iter().enumerate() {
            out.pages.push(Some(FRAME));
            for (i, word) in text.split_whitespace().enumerate() {
                let x = 60.0 + 50.0 * i as f64;
                out.words.push(diff::Word {
                    text: word.into(),
                    page,
                    rect: [x, 700.0, x + 40.0, 720.0],
                });
            }
        }
        out
    }

    fn page<'a>(lines: &'a [&'a str]) -> TestPage<'a> {
        TestPage {
            lines,
            crop_box: None,
            rotate: 0,
        }
    }

    fn compare_pdfs(current: &[u8], other: &[u8], current_is_old: bool) -> DocumentComparison {
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let current = pdfium.load_pdf_from_byte_slice(current, None).unwrap();
        let other = pdfium.load_pdf_from_byte_slice(other, None).unwrap();
        compare_documents(&pdfium, &current, &other, current_is_old).unwrap()
    }

    #[test]
    fn pdfium_words_are_compared_and_located_on_their_pages() {
        let v1 = text_pdf(&[
            page(&["The quick brown fox", "jumps over the dog"]),
            page(&["Second page stays"]),
        ]);
        let v2 = text_pdf(&[
            page(&["The quick red fox", "jumps over the dog"]),
            page(&["Second page stays here"]),
        ]);
        let comparison = compare_pdfs(&v2, &v1, false);
        assert_eq!((comparison.old_words, comparison.new_words), (11, 12));
        let got: Vec<_> = comparison
            .changes
            .iter()
            .map(|c| {
                (
                    c.kind,
                    c.old.text.as_str(),
                    c.new.text.as_str(),
                    c.new.page_index,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (ChangeKind::Replaced, "brown", "red", 0),
                (ChangeKind::Inserted, "", "here", 1),
            ]
        );
        // "red" sits on the first line: baseline 720 of 792, after "The quick " at x=72.
        let rect = &comparison.changes[0].new.rects[0];
        assert!((0.15..0.35).contains(&rect.x), "{rect:?}");
        assert!((0.07..0.10).contains(&rect.y), "{rect:?}");
        assert!(rect.width > 0.0 && rect.width < 0.1, "{rect:?}");
        // The inserted word has no box in the old document, only its neighbour's.
        let inserted = &comparison.changes[1];
        assert!(inserted.old.rects.is_empty() && inserted.old.near.is_some());
        assert!(inserted.new.rects[0].x > 0.25, "{:?}", inserted.new.rects);
    }

    #[test]
    fn rects_are_relative_to_the_crop_box_and_carry_the_page_rotation() {
        let cropped = |lines| TestPage {
            lines,
            crop_box: Some([36, 396, 576, 756]),
            rotate: 90,
        };
        let v1 = text_pdf(&[cropped(&["alpha beta"])]);
        let v2 = text_pdf(&[cropped(&["alpha gamma"])]);
        let comparison = compare_pdfs(&v1, &v2, true);
        let change = &comparison.changes[0];
        assert_eq!(change.old.intrinsic_rotation, 90);
        let rect = &change.old.rects[0];
        // Baseline 720 is 36 pt under the crop box's top (756) of its 360 pt height. A
        // 12 pt line is ~0.035 of that height; against the 792 pt media box it would be
        // under 0.02.
        assert!((0.05..0.12).contains(&rect.y), "{rect:?}");
        assert!((0.025..0.045).contains(&rect.height), "{rect:?}");
        assert!(rect.x > (72.0 - 36.0) / 540.0, "{rect:?}");
    }

    #[test]
    fn identical_documents_have_no_changes() {
        let v1 = text_pdf(&[page(&["same words on the page"])]);
        let comparison = compare_pdfs(&v1, &v1, true);
        assert!(comparison.changes.is_empty());
        assert_eq!(comparison.total_changes, 0);
        assert_eq!(comparison.old_words, 5);
    }

    #[test]
    fn rects_are_normalized_to_the_effective_box_with_a_top_left_origin() {
        let frame = PageFrame {
            effective_box: [100.0, 50.0, 400.0, 200.0],
            rotation: 90,
        };
        let rect = normalized(&[150.0, 100.0, 250.0, 150.0], frame).unwrap();
        assert_eq!(
            rect,
            NormalizedPageRect {
                x: 0.125,
                y: 0.5,
                width: 0.25,
                height: 0.25
            }
        );
        assert!(normalized(&[150.0, 100.0, 150.0, 150.0], frame).is_none());
    }

    #[test]
    fn the_current_document_can_be_either_side() {
        let v1 = document(&["the quick brown fox"]);
        let v2 = document(&["the quick red fox"]);
        let as_old = comparison_of(&v1, &v2, true);
        assert_eq!(as_old.changes.len(), 1);
        let change = &as_old.changes[0];
        assert_eq!(change.kind, ChangeKind::Replaced);
        assert_eq!(
            (change.old.text.as_str(), change.new.text.as_str()),
            ("brown", "red")
        );
        assert_eq!(
            change.new.rects,
            [NormalizedPageRect {
                x: 0.266_666_68,
                y: 0.1,
                width: 0.066_666_67,
                height: 0.025
            }]
        );
        assert_eq!((as_old.old_words, as_old.new_words), (4, 4));

        let as_new = comparison_of(&v2, &v1, false);
        assert!(!as_new.current_is_old);
        assert_eq!(as_new.changes[0].old.text, "red");
    }

    #[test]
    fn a_one_sided_change_points_at_its_neighbour_in_the_other_document() {
        let old = document(&["alpha beta", "gamma delta"]);
        let new = document(&["alpha beta", "gamma"]);
        let comparison = comparison_of(&old, &new, true);
        let change = &comparison.changes[0];
        assert_eq!(change.kind, ChangeKind::Deleted);
        assert_eq!((change.old.page_index, change.new.page_index), (1, 1));
        assert!(change.new.rects.is_empty());
        assert!(change.new.near.is_some());
    }

    #[test]
    fn pages_without_geometry_keep_their_text_but_no_rects() {
        let mut old = document(&["one two"]);
        old.pages[0] = None;
        let new = document(&["one three"]);
        let comparison = comparison_of(&old, &new, true);
        let change = &comparison.changes[0];
        assert_eq!(change.old.text, "two");
        assert!(change.old.rects.is_empty());
        assert_eq!(change.new.rects.len(), 1);
    }

    #[test]
    fn long_sides_and_many_changes_are_capped() {
        let long = vec!["x"; MAX_SIDE_TEXT_CHARS].join(" ");
        let comparison = comparison_of(&document(&[""]), &document(&[&long]), true);
        let inserted = &comparison.changes[0].new;
        assert!(inserted.text_truncated);
        assert_eq!(inserted.text.chars().count(), MAX_SIDE_TEXT_CHARS + 1);
        assert!(inserted.rects.len() <= MAX_SIDE_RECTS);

        let old: Vec<String> = (0..MAX_COMPARE_CHANGES + 10)
            .map(|i| format!("a{i} same{i}"))
            .collect();
        let new: Vec<String> = (0..MAX_COMPARE_CHANGES + 10)
            .map(|i| format!("b{i} same{i}"))
            .collect();
        let comparison = comparison_of(
            &document(&[&old.join(" ")]),
            &document(&[&new.join(" ")]),
            true,
        );
        assert_eq!(comparison.total_changes, MAX_COMPARE_CHANGES + 10);
        assert_eq!(comparison.changes.len(), MAX_COMPARE_CHANGES);
    }

    #[test]
    fn the_wire_shape_is_camel_case() {
        let comparison = comparison_of(&document(&["a b"]), &document(&["a c"]), true);
        let json = serde_json::to_value(&comparison).unwrap();
        assert_eq!(json["currentIsOld"], true);
        assert_eq!(json["totalChanges"], 1);
        assert_eq!(json["changes"][0]["kind"], "replaced");
        assert_eq!(json["changes"][0]["new"]["pageIndex"], 0);
        assert_eq!(json["changes"][0]["new"]["intrinsicRotation"], 0);
        assert!(json["changes"][0]["old"]["near"].is_null());
    }
}
