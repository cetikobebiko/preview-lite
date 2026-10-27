//! Yazdırma tabakalarını PDFium ile kurar: [`crate::print_layout::layout`] her kaynak sayfanın
//! hangi tabakaya hangi matrisle gideceğini söyler, bu modül o tabakaları yeni bir PDF olarak
//! yazar. Job worker'da çalışır; anlık görüntü dosyası yalnız okunur.
//!
//! Her kaynak sayfa tabakaya `FPDF_NewXObjectFromPage` ile form XObject olarak konur. PDFium o
//! XObject'i zaten görüntülenen sayfa koordinatlarında kurar (BBox = crop box, `/Matrix`
//! `/Rotate`'i uygular ve sol alt köşeyi sıfıra taşır); yerleşim matrisleri de PdfCraft'taki
//! gibi bu koordinatlara uyar ve crop box dışı BBox ile kesilir. Annotation'lar önce yazdırma
//! bayrağına göre düzleştirilir (`FLAT_PRINT`), yoksa form XObject onları taşımaz. Sayfanın
//! yalnız bir kısmını gösteren yerleşim (poster parçası) önce o pencere kadar bir ara sayfaya
//! konur; onun BBox'ı da pencere dışını keser.

use crate::incremental_save::RawDocument;
use crate::job_worker::JobFailure;
use crate::print_layout::{
    booklet_duplex, cups_media, layout, Binding, BookletSubset, Duplex, Layout, Matrix,
    Orientation, PageOrder, Settings, Sheet, SizeMode, PAPERS,
};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::{Entry, HashMap};
use std::path::Path;

/// Yazdır penceresinin sayfa düzeni seçimi, IPC biçimiyle.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrintLayoutRequest {
    /// `print_layout::PAPERS` adlarından biri ("A4", "US Letter", …).
    pub paper: String,
    pub orientation: PrintOrientation,
    pub mode: PrintMode,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrintOrientation {
    Auto,
    Portrait,
    Landscape,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrintPageOrder {
    Horizontal,
    HorizontalReversed,
    Vertical,
    VerticalReversed,
    CutStack,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrintBookletSides {
    Both,
    Front,
    Back,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PrintMode {
    Fit,
    Actual,
    Shrink,
    Scale {
        percent: f64,
    },
    Multiple {
        per_sheet: u32,
        order: PrintPageOrder,
        border: bool,
    },
    Booklet {
        sides: PrintBookletSides,
        right_binding: bool,
    },
    Poster {
        percent: f64,
        overlap_points: f64,
        cut_marks: bool,
    },
}

/// What the spooler is told besides the file: the paper to feed and two-sided printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolOptions {
    pub media: Option<&'static str>,
    pub duplex: Duplex,
}

impl PrintLayoutRequest {
    fn paper_index(&self) -> Result<usize, JobFailure> {
        PAPERS
            .iter()
            .position(|(name, _)| *name == self.paper)
            .ok_or_else(|| {
                JobFailure::new(
                    "SESSION_JOB_PRINT_LAYOUT_INVALID",
                    format!("Bilinmeyen kağıt boyutu: {}", self.paper),
                )
            })
    }

    /// The layout settings for `pages` (physical, in print order).
    pub(crate) fn settings(&self, pages: Vec<usize>) -> Result<Settings, JobFailure> {
        let layout = match self.mode {
            PrintMode::Fit => Layout::Size(SizeMode::Fit),
            PrintMode::Actual => Layout::Size(SizeMode::Actual),
            PrintMode::Shrink => Layout::Size(SizeMode::Shrink),
            PrintMode::Scale { percent } => Layout::Size(SizeMode::Custom(percent)),
            PrintMode::Multiple {
                per_sheet,
                order,
                border,
            } => match Layout::multiple(per_sheet as usize) {
                Layout::Multiple {
                    cols,
                    rows,
                    per_sheet,
                    ..
                } => Layout::Multiple {
                    cols,
                    rows,
                    per_sheet,
                    order: match order {
                        PrintPageOrder::Horizontal => PageOrder::Horizontal,
                        PrintPageOrder::HorizontalReversed => PageOrder::HorizontalReversed,
                        PrintPageOrder::Vertical => PageOrder::Vertical,
                        PrintPageOrder::VerticalReversed => PageOrder::VerticalReversed,
                        PrintPageOrder::CutStack => PageOrder::CutStack,
                    },
                    border,
                    auto_rotate: true,
                },
                other => other,
            },
            PrintMode::Booklet {
                sides,
                right_binding,
            } => Layout::Booklet {
                subset: booklet_subset(sides),
                binding: if right_binding {
                    Binding::Right
                } else {
                    Binding::Left
                },
            },
            PrintMode::Poster {
                percent,
                overlap_points,
                cut_marks,
            } => Layout::Poster {
                scale: percent,
                overlap: overlap_points,
                cut_marks,
            },
        };
        Ok(Settings {
            pages,
            paper: PAPERS[self.paper_index()?].1,
            orientation: match self.orientation {
                PrintOrientation::Auto => Orientation::Auto,
                PrintOrientation::Portrait => Orientation::Portrait,
                PrintOrientation::Landscape => Orientation::Landscape,
            },
            layout,
        })
    }

    /// The paper by name and, for a two-sided booklet, short-edge duplex (as Acrobat does).
    pub fn spool_options(&self) -> SpoolOptions {
        SpoolOptions {
            media: self.paper_index().ok().and_then(cups_media),
            duplex: match self.mode {
                PrintMode::Booklet { sides, .. } => booklet_duplex(booklet_subset(sides)),
                _ => Duplex::Off,
            },
        }
    }
}

fn booklet_subset(sides: PrintBookletSides) -> BookletSubset {
    match sides {
        PrintBookletSides::Both => BookletSubset::BothSides,
        PrintBookletSides::Front => BookletSubset::FrontOnly,
        PrintBookletSides::Back => BookletSubset::BackOnly,
    }
}

/// `FPDFPage_Flatten`'in yazdırılacak görünümü seçen bayrağı.
const FLAT_PRINT: i32 = 1;
const FLATTEN_FAIL: i32 = 0;
/// Kenarlık ve kesim işareti çizgisinin kalınlığı (pt).
const STROKE_WIDTH: f32 = 0.5;

fn failure(message: impl Into<String>) -> JobFailure {
    JobFailure::new("SESSION_JOB_PRINT_PREPARATION_FAILED", message)
}

/// The page's size as it displays: its crop box (media box without one), turned by `/Rotate`.
fn display_size(bindings: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> (f64, f64) {
    let (mut l, mut b, mut r, mut t) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let has_crop =
        bindings.is_true(bindings.FPDFPage_GetCropBox(page, &mut l, &mut b, &mut r, &mut t));
    if !has_crop || r <= l || t <= b {
        bindings.FPDFPage_GetMediaBox(page, &mut l, &mut b, &mut r, &mut t);
    }
    let (w, h) = (f64::from((r - l).abs()), f64::from((t - b).abs()));
    // FPDFPage_GetRotation: 0–3 quarter turns clockwise.
    if bindings.FPDFPage_GetRotation(page) % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    }
}

/// An owned PDFium document created here; closed on drop.
fn new_document<'b>(
    bindings: &'b dyn PdfiumLibraryBindings,
) -> Result<RawDocument<'b>, JobFailure> {
    let handle = bindings.FPDF_CreateNewDocument();
    if handle.is_null() {
        return Err(failure("Yazdırma belgesi oluşturulamadı"));
    }
    Ok(RawDocument { bindings, handle })
}

struct OwnedXObject<'b> {
    bindings: &'b dyn PdfiumLibraryBindings,
    handle: FPDF_XOBJECT,
}

impl Drop for OwnedXObject<'_> {
    fn drop(&mut self) {
        self.bindings.FPDF_CloseXObject(self.handle);
    }
}

fn xobject<'b>(
    bindings: &'b dyn PdfiumLibraryBindings,
    destination: FPDF_DOCUMENT,
    source: FPDF_DOCUMENT,
    page: usize,
) -> Result<OwnedXObject<'b>, JobFailure> {
    let index = i32::try_from(page).map_err(|_| failure("Sayfa numarası geçersiz"))?;
    let handle = bindings.FPDF_NewXObjectFromPage(destination, source, index);
    if handle.is_null() {
        return Err(failure(format!(
            "{}. sayfa yazdırma için alınamadı",
            page + 1
        )));
    }
    Ok(OwnedXObject { bindings, handle })
}

fn place(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    xobject: &OwnedXObject<'_>,
    matrix: Matrix,
) -> Result<(), JobFailure> {
    let object = bindings.FPDF_NewFormObjectFromXObject(xobject.handle);
    if object.is_null() {
        return Err(failure("Sayfa tabakaya yerleştirilemedi"));
    }
    let [a, b, c, d, e, f] = matrix.0;
    bindings.FPDFPageObj_Transform(object, a, b, c, d, e, f);
    bindings.FPDFPage_InsertObject(page, object);
    Ok(())
}

fn stroke(bindings: &dyn PdfiumLibraryBindings, page: FPDF_PAGE, path: FPDF_PAGEOBJECT) {
    if path.is_null() {
        return;
    }
    bindings.FPDFPath_SetDrawMode(path, 0, 1);
    bindings.FPDFPageObj_SetStrokeColor(path, 0, 0, 0, 255);
    bindings.FPDFPageObj_SetStrokeWidth(path, STROKE_WIDTH);
    bindings.FPDFPage_InsertObject(page, path);
}

struct NewPage<'b> {
    bindings: &'b dyn PdfiumLibraryBindings,
    handle: FPDF_PAGE,
}

impl<'b> NewPage<'b> {
    fn create(
        document: &RawDocument<'b>,
        index: usize,
        (w, h): (f64, f64),
    ) -> Result<Self, JobFailure> {
        let index = i32::try_from(index).map_err(|_| failure("Tabaka sayısı sınırı aşıldı"))?;
        let handle = document.bindings.FPDFPage_New(document.handle, index, w, h);
        if handle.is_null() {
            return Err(failure("Yazdırma tabakası oluşturulamadı"));
        }
        Ok(Self {
            bindings: document.bindings,
            handle,
        })
    }

    fn finish(self) -> Result<(), JobFailure> {
        if self
            .bindings
            .is_true(self.bindings.FPDFPage_GenerateContent(self.handle))
        {
            Ok(())
        } else {
            Err(failure("Yazdırma tabakasının içeriği yazılamadı"))
        }
    }
}

impl Drop for NewPage<'_> {
    fn drop(&mut self) {
        self.bindings.FPDF_ClosePage(self.handle);
    }
}

/// `source`'u `settings`'e göre tabakalara dizip `output`'a yazar ve tabakaları döndürür.
pub(crate) fn impose(
    bindings: &dyn PdfiumLibraryBindings,
    source: &Path,
    credential: Option<&str>,
    settings: &Settings,
    output: &Path,
) -> Result<Vec<Sheet>, JobFailure> {
    let path = source
        .to_str()
        .ok_or_else(|| failure("Anlık görüntü yolu UTF-8 değil"))?;
    let source = RawDocument::open(bindings, path, credential).ok_or_else(|| {
        JobFailure::new(
            "SESSION_JOB_PDF_OPEN_FAILED",
            format!(
                "PDF yüklenemedi (PDFium hata kodu {})",
                bindings.FPDF_GetLastError()
            ),
        )
    })?;
    let page_count = usize::try_from(bindings.FPDF_GetPageCount(source.handle)).unwrap_or(0);
    if settings.pages.is_empty() || settings.pages.len() > crate::job_worker::MAX_JOB_PAGES {
        return Err(JobFailure::new(
            "SESSION_JOB_INVALID_PAGE_SELECTION",
            "Yazdırılacak sayfa seçimi boş ya da sınırı aşıyor",
        ));
    }
    if let Some(&missing) = settings.pages.iter().find(|&&page| page >= page_count) {
        return Err(JobFailure::new(
            "SESSION_JOB_INVALID_PAGE_SELECTION",
            format!("Belgede {}. sayfa yok", missing + 1),
        ));
    }

    // Each selected page once: flattened for print, measured as it displays.
    let mut sizes = vec![(0.0, 0.0); page_count];
    let mut flattened = vec![false; page_count];
    for &page in &settings.pages {
        if flattened[page] {
            continue;
        }
        let index = i32::try_from(page).map_err(|_| failure("Sayfa numarası geçersiz"))?;
        let handle = bindings.FPDF_LoadPage(source.handle, index);
        if handle.is_null() {
            return Err(failure(format!("{}. sayfa açılamadı", page + 1)));
        }
        let result = bindings.FPDFPage_Flatten(handle, FLAT_PRINT);
        sizes[page] = display_size(bindings, handle);
        bindings.FPDF_ClosePage(handle);
        if result == FLATTEN_FAIL {
            return Err(failure(format!(
                "{}. sayfanın açıklamaları düzleştirilemedi",
                page + 1
            )));
        }
        flattened[page] = true;
    }

    let sheets = layout(&sizes, settings)
        .map_err(|error| JobFailure::new("SESSION_JOB_PRINT_LAYOUT_INVALID", error.to_string()))?;
    if sheets.len() > crate::job_worker::MAX_JOB_PAGES {
        return Err(JobFailure::new(
            "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
            "Yazdırma tabaka sayısı sınırı aşıldı",
        ));
    }

    let out = new_document(bindings)?;
    // Windows onto pages (poster tiles), each its own page here.
    let windows = new_document(bindings)?;
    let mut window_sources: HashMap<usize, OwnedXObject<'_>> = HashMap::new();
    let mut xobjects: HashMap<usize, OwnedXObject<'_>> = HashMap::new();
    let mut window_count = 0usize;
    for (index, sheet) in sheets.iter().enumerate() {
        let page = NewPage::create(&out, index, sheet.size)?;
        for placement in &sheet.placed {
            let (w, h) = sizes[placement.page];
            let [x0, y0, x1, y1] = placement.clip;
            let whole = x0 <= 0.0 && y0 <= 0.0 && x1 >= w && y1 >= h;
            if whole {
                let content = match xobjects.entry(placement.page) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => entry.insert(xobject(
                        bindings,
                        out.handle,
                        source.handle,
                        placement.page,
                    )?),
                };
                place(bindings, page.handle, content, placement.matrix)?;
                continue;
            }
            // The window's own page: the page moved so the window's corner is at the origin.
            let window = NewPage::create(&windows, window_count, (x1 - x0, y1 - y0))?;
            let content = match window_sources.entry(placement.page) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(xobject(
                    bindings,
                    windows.handle,
                    source.handle,
                    placement.page,
                )?),
            };
            let shift = Matrix([1.0, 0.0, 0.0, 1.0, -x0, -y0]);
            place(bindings, window.handle, content, shift)?;
            window.finish()?;
            let tile = xobject(bindings, out.handle, windows.handle, window_count)?;
            window_count += 1;
            let back = Matrix([1.0, 0.0, 0.0, 1.0, x0, y0]).then(&placement.matrix);
            place(bindings, page.handle, &tile, back)?;
        }
        for &[x0, y0, x1, y1] in &sheet.borders {
            let rect = bindings.FPDFPageObj_CreateNewRect(
                x0 as f32,
                y0 as f32,
                (x1 - x0) as f32,
                (y1 - y0) as f32,
            );
            stroke(bindings, page.handle, rect);
        }
        for &[x0, y0, x1, y1] in &sheet.lines {
            let line = bindings.FPDFPageObj_CreateNewPath(x0 as f32, y0 as f32);
            if !line.is_null() {
                bindings.FPDFPath_LineTo(line, x1 as f32, y1 as f32);
            }
            stroke(bindings, page.handle, line);
        }
        page.finish()?;
    }
    drop(window_sources);
    drop(xobjects);
    crate::incremental_save::save_copy(bindings, out.handle, output, 0).map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Yazdırma PDF'i yazılamadı: {}", error.detail),
        )
    })?;
    Ok(sheets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Dictionary, Document, Object, Stream};

    /// A source page: its media box, optional crop box and `/Rotate`, filled rectangles
    /// `(rgb, [x, y, w, h])`, and annotations `(print flag, rgb, rect)` drawn by their `/AP`.
    struct SourcePage {
        media: [i64; 4],
        crop: Option<[i64; 4]>,
        rotate: i64,
        fills: Vec<([f32; 3], [i64; 4])>,
        annots: Vec<(bool, [f32; 3], [i64; 4])>,
    }

    fn page(fills: Vec<([f32; 3], [i64; 4])>) -> SourcePage {
        SourcePage {
            media: [0, 0, 200, 300],
            crop: None,
            rotate: 0,
            fills,
            annots: Vec::new(),
        }
    }

    fn fill_ops(rgb: [f32; 3], [x, y, w, h]: [i64; 4]) -> Vec<Operation> {
        vec![
            Operation::new("rg", rgb.iter().map(|&c| c.into()).collect()),
            Operation::new("re", vec![x.into(), y.into(), w.into(), h.into()]),
            Operation::new("f", vec![]),
        ]
    }

    fn source_pdf(pages: Vec<SourcePage>) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut kids = Vec::new();
        for source in pages {
            let operations: Vec<Operation> = source
                .fills
                .iter()
                .flat_map(|&(rgb, rect)| fill_ops(rgb, rect))
                .collect();
            let content = document.add_object(Stream::new(
                dictionary! {},
                Content { operations }.encode().unwrap(),
            ));
            let mut annots = Vec::new();
            for (print, rgb, [x0, y0, x1, y1]) in source.annots {
                let appearance = document.add_object(Stream::new(
                    dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), (x1 - x0).into(), (y1 - y0).into()] },
                    Content { operations: fill_ops(rgb, [0, 0, x1 - x0, y1 - y0]) }.encode().unwrap(),
                ));
                annots.push(Object::Reference(document.add_object(dictionary! {
                    "Type" => "Annot",
                    "Subtype" => "Square",
                    "F" => if print { 4 } else { 0 },
                    "Rect" => vec![x0.into(), y0.into(), x1.into(), y1.into()],
                    "AP" => dictionary! { "N" => appearance },
                })));
            }
            let mut dict: Dictionary = dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => source.media.iter().map(|&v| v.into()).collect::<Vec<Object>>(),
                "Rotate" => source.rotate,
                "Contents" => content,
                "Resources" => dictionary! {},
            };
            if let Some(crop) = source.crop {
                dict.set(
                    "CropBox",
                    crop.iter().map(|&v| v.into()).collect::<Vec<Object>>(),
                );
            }
            if !annots.is_empty() {
                dict.set("Annots", annots);
            }
            kids.push(Object::Reference(document.add_object(dict)));
        }
        let count = kids.len() as i64;
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
        );
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn request(paper: &str, mode: PrintMode) -> PrintLayoutRequest {
        PrintLayoutRequest {
            paper: paper.into(),
            orientation: PrintOrientation::Auto,
            mode,
        }
    }

    /// Imposes `pages` of `source` and renders every sheet at 72 dpi (one pixel per point).
    fn impose_and_render(
        source: Vec<SourcePage>,
        pages: Vec<usize>,
        layout: PrintLayoutRequest,
    ) -> (Vec<Sheet>, Vec<image::RgbaImage>) {
        let directory = tempfile::tempdir().unwrap();
        let (input, output) = (
            directory.path().join("in.pdf"),
            directory.path().join("out.pdf"),
        );
        std::fs::write(&input, source_pdf(source)).unwrap();
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let sheets = impose(
            pdfium.bindings(),
            &input,
            None,
            &layout.settings(pages).unwrap(),
            &output,
        )
        .unwrap();
        let document = pdfium.load_pdf_from_file(&output, None).unwrap();
        assert_eq!(usize::from(document.pages().len()), sheets.len());
        let images = document
            .pages()
            .iter()
            .map(|page| {
                let width = page.width().value.round() as i32;
                let config = PdfRenderConfig::new().set_target_width(width);
                page.render_with_config(&config)
                    .unwrap()
                    .as_image()
                    .to_rgba8()
            })
            .collect();
        (sheets, images)
    }

    fn count(image: &image::RgbaImage, rgb: [u8; 3]) -> usize {
        image
            .pixels()
            .filter(|p| p.0[..3].iter().zip(rgb).all(|(a, b)| a.abs_diff(b) < 40))
            .count()
    }

    const RED: [f32; 3] = [1.0, 0.0, 0.0];
    const GREEN: [f32; 3] = [0.0, 1.0, 0.0];
    const BLUE: [f32; 3] = [0.0, 0.0, 1.0];
    const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

    #[test]
    fn two_up_sheets_show_each_page_inside_its_crop_box() {
        let cropped = SourcePage {
            crop: Some([20, 20, 180, 280]),
            ..page(vec![(RED, [0, 0, 200, 300]), (GREEN, [20, 20, 160, 260])])
        };
        let mode = PrintMode::Multiple {
            per_sheet: 2,
            order: PrintPageOrder::Horizontal,
            border: true,
        };
        let (sheets, images) = impose_and_render(
            vec![cropped, page(vec![(BLUE, [0, 0, 200, 300])])],
            vec![0, 1],
            request("US Letter", mode),
        );
        assert_eq!(sheets.len(), 1);
        let sheet = &images[0];
        assert_eq!(sheet.dimensions(), (792, 612));
        assert_eq!(
            count(sheet, [255, 0, 0]),
            0,
            "nothing outside the crop box prints"
        );
        let half = |x: u32| if x < 396 { 0 } else { 1 };
        let mut green = [0, 0];
        let mut blue = [0, 0];
        for (x, _, p) in sheet.enumerate_pixels() {
            match p.0[..3] {
                [0..=40, 215..=255, 0..=40] => green[half(x)] += 1,
                [0..=40, 0..=40, 215..=255] => blue[half(x)] += 1,
                _ => {}
            }
        }
        assert!(green[0] > 50_000 && green[1] == 0, "{green:?}");
        assert!(blue[1] > 50_000 && blue[0] == 0, "{blue:?}");
    }

    #[test]
    fn rotated_pages_print_as_they_display() {
        // A black mark at the page's top-left; /Rotate 90 shows it at the top-right.
        let turned = SourcePage {
            rotate: 90,
            ..page(vec![(BLACK, [0, 260, 40, 40])])
        };
        let (sheets, images) =
            impose_and_render(vec![turned], vec![0], request("US Letter", PrintMode::Fit));
        // The page displays 300 × 200, so Auto picks a landscape sheet; fit = 2.52, at (18, 54).
        assert_eq!(sheets[0].size, (792.0, 612.0));
        let dark = |x: u32, y: u32| images[0].get_pixel(x, y).0[0] < 60;
        assert!(dark(720, 100), "top-right");
        assert!(
            !dark(60, 100) && !dark(720, 500),
            "not top-left or bottom-right"
        );
    }

    #[test]
    fn annotations_print_by_their_print_flag() {
        let annotated = SourcePage {
            annots: vec![
                (true, BLUE, [20, 20, 80, 80]),
                (false, RED, [120, 20, 180, 80]),
            ],
            ..page(Vec::new())
        };
        let (_, images) =
            impose_and_render(vec![annotated], vec![0], request("A4", PrintMode::Actual));
        assert!(
            count(&images[0], [0, 0, 255]) > 3000,
            "a printable comment prints"
        );
        assert_eq!(count(&images[0], [255, 0, 0]), 0, "a hidden one does not");
    }

    #[test]
    fn booklets_and_posters_make_their_sheets() {
        let pages = || {
            (0..6)
                .map(|_| page(vec![(BLUE, [0, 0, 200, 300])]))
                .collect::<Vec<_>>()
        };
        let booklet = PrintMode::Booklet {
            sides: PrintBookletSides::Both,
            right_binding: false,
        };
        let (sheets, images) = impose_and_render(pages(), (0..6).collect(), request("A4", booklet));
        assert_eq!(sheets.len(), 4, "6 pages pad to 8: two sheets, both sides");
        assert!(
            images.iter().all(|image| image.width() > image.height()),
            "landscape spreads"
        );
        let poster = PrintMode::Poster {
            percent: 400.0,
            overlap_points: 36.0,
            cut_marks: true,
        };
        let (sheets, images) = impose_and_render(
            vec![page(vec![(BLUE, [0, 0, 200, 300])])],
            vec![0],
            request("US Letter", poster),
        );
        assert_eq!(sheets.len(), 4);
        // The tile is clipped to the printable area: the 18 pt margin stays white but for the
        // cut marks.
        assert!(images
            .iter()
            .all(|image| image.get_pixel(5, 300).0[..3] == [255, 255, 255]));
        assert!(images
            .iter()
            .all(|image| count(image, [0, 0, 255]) > 100_000));
    }

    #[test]
    fn invalid_selections_and_papers_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("in.pdf");
        std::fs::write(&input, source_pdf(vec![page(Vec::new())])).unwrap();
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let fit = request("A4", PrintMode::Fit);
        let run = |pages: Vec<usize>| {
            impose(
                pdfium.bindings(),
                &input,
                None,
                &fit.settings(pages).unwrap(),
                &directory.path().join("out.pdf"),
            )
            .unwrap_err()
            .code
        };
        assert_eq!(run(vec![]), "SESSION_JOB_INVALID_PAGE_SELECTION");
        assert_eq!(run(vec![1]), "SESSION_JOB_INVALID_PAGE_SELECTION");
        assert_eq!(
            request("B7", PrintMode::Fit)
                .settings(vec![0])
                .unwrap_err()
                .code,
            "SESSION_JOB_PRINT_LAYOUT_INVALID"
        );
        let zero = request("A4", PrintMode::Scale { percent: 0.0 });
        let error = impose(
            pdfium.bindings(),
            &input,
            None,
            &zero.settings(vec![0]).unwrap(),
            &directory.path().join("out.pdf"),
        )
        .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            (
                "SESSION_JOB_PRINT_LAYOUT_INVALID",
                "Ölçek %0'dan büyük olmalı"
            )
        );
    }

    #[test]
    fn the_request_is_camel_case_and_tells_the_spooler_paper_and_duplex() {
        let booklet: PrintLayoutRequest = serde_json::from_value(serde_json::json!({
            "paper": "A4",
            "orientation": "auto",
            "mode": { "kind": "booklet", "sides": "both", "rightBinding": false },
        }))
        .unwrap();
        assert_eq!(
            booklet.spool_options(),
            SpoolOptions {
                media: Some("A4"),
                duplex: Duplex::ShortEdge
            }
        );
        let multiple: PrintLayoutRequest = serde_json::from_value(serde_json::json!({
            "paper": "US Letter",
            "orientation": "landscape",
            "mode": { "kind": "multiple", "perSheet": 4, "order": "cut_stack", "border": true },
        }))
        .unwrap();
        assert_eq!(
            multiple.spool_options(),
            SpoolOptions {
                media: Some("Letter"),
                duplex: Duplex::Off
            }
        );
        let settings = multiple.settings(vec![0, 1]).unwrap();
        assert_eq!(settings.orientation, Orientation::Landscape);
        assert!(matches!(
            settings.layout,
            Layout::Multiple {
                cols: 2,
                rows: 2,
                per_sheet: 4,
                order: PageOrder::CutStack,
                border: true,
                auto_rotate: true
            }
        ));
        let front = PrintLayoutRequest {
            mode: PrintMode::Booklet {
                sides: PrintBookletSides::Front,
                right_binding: true,
            },
            ..booklet
        };
        assert_eq!(front.spool_options().duplex, Duplex::Off);
    }
}
