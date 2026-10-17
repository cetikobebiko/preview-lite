use crate::document_io::{
    atomic_write_pdf, ensure_unprotected, AtomicWriteReport, CommitState, DocumentIoError,
    DocumentIoFailure, DocumentIoStage,
};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Annotation IPC koordinat sözleşmesi: sayfanın görüntü uzayında normalize dikdörtgen.
///
/// Görüntü uzayı, PDFium'un sayfayı ek döndürme olmadan render ettiği bitmap'tir:
/// sayfanın `/Rotate`'i ve CropBox'ı uygulanmış, viewer rotation'ı uygulanmamış.
/// Köken sol-üst köşe, y aşağı doğru; `x`/`width` görüntü genişliğine, `y`/`height`
/// görüntü yüksekliğine bölünmüştür ve dikdörtgen [0, 1]² içinde kalır.
///
/// PDF user space'e (ve tersine) dönüşüm yalnız bu modülde, worker process'inde
/// PDFium'un `FPDF_DeviceToPage` dönüşümüyle yapılır (bkz. [`PageDisplayTransform`]).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct NormalizedPageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// PDF user space dikdörtgeni (point): x = left, y = bottom. IPC'ye çıkmaz.
#[derive(Debug, Clone)]
pub(crate) struct RectData {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AnnotationData {
    pub id: String,
    pub annot_type: String,
    pub color: String, // hex: #RRGGBB
    /// Annotation `/Rect`'inin görüntü uzayındaki karşılığı; bounds okunamazsa `None`.
    pub normalized_rect: Option<NormalizedPageRect>,
    pub contents: Option<String>,
}

/// Normalize sınır toleransı: frontend'in [0, 1]'e kıstığı değerlerdeki float hatası.
const NORMALIZED_RECT_EPSILON: f32 = 1e-4;

/// Görüntü uzayı → PDF user space dönüşümünün örneklendiği sabit cihaz boyutu (px).
/// Yalnız köşeler örneklendiğinden sonuç bu değerden bağımsızdır.
const DISPLAY_SAMPLE_PIXELS: Pixels = 1000;

/// Sayfanın normalize görüntü uzayı ile PDF user space arasındaki afin dönüşüm.
///
/// `/Rotate` ve CropBox formülle türetilmez: PDFium'un kendi `FPDF_DeviceToPage`
/// dönüşümü ek döndürmesiz bir cihaz dikdörtgeninin üç köşesinde örneklenir
/// (sol-üst, sağ-üst, sol-alt). PDFium sayfa görüntüsünü cihaz dikdörtgenine
/// köşeden köşeye eşlediği için köşe örnekleri tam dönüşümü verir; render ve
/// metin geometrisi de aynı matristen geldiğinden annotation render edilen
/// görüntüde frontend'in gördüğü yere düşer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageDisplayTransform {
    /// Görüntü (0, 0) noktasının user space karşılığı.
    origin: [f64; 2],
    /// Görüntü (1, 0) − (0, 0).
    x_axis: [f64; 2],
    /// Görüntü (0, 1) − (0, 0).
    y_axis: [f64; 2],
}

impl PageDisplayTransform {
    pub(crate) fn for_page(page: &PdfPage<'_>) -> Result<Self, String> {
        let config =
            PdfRenderConfig::new().set_fixed_size(DISPLAY_SAMPLE_PIXELS, DISPLAY_SAMPLE_PIXELS);
        let sample = |x: Pixels, y: Pixels| {
            page.pixels_to_points(x, y, &config)
                .map(|(page_x, page_y)| [f64::from(page_x.value), f64::from(page_y.value)])
                .map_err(|error| format!("PDFium görüntü → sayfa dönüşümü başarısız: {error}"))
        };
        let origin = sample(0, 0)?;
        let right = sample(DISPLAY_SAMPLE_PIXELS, 0)?;
        let bottom = sample(0, DISPLAY_SAMPLE_PIXELS)?;
        Self::from_corners(origin, right, bottom)
    }

    fn from_corners(origin: [f64; 2], right: [f64; 2], bottom: [f64; 2]) -> Result<Self, String> {
        let transform = Self {
            origin,
            x_axis: [right[0] - origin[0], right[1] - origin[1]],
            y_axis: [bottom[0] - origin[0], bottom[1] - origin[1]],
        };
        let determinant = transform.determinant();
        if !determinant.is_finite() || determinant.abs() < f64::EPSILON {
            return Err("Sayfa görüntü dönüşümü tekil; sayfa boyutu geçersiz".to_string());
        }
        Ok(transform)
    }

    fn determinant(&self) -> f64 {
        self.x_axis[0] * self.y_axis[1] - self.y_axis[0] * self.x_axis[1]
    }

    fn display_to_user(&self, [u, v]: [f64; 2]) -> [f64; 2] {
        [
            self.origin[0] + u * self.x_axis[0] + v * self.y_axis[0],
            self.origin[1] + u * self.x_axis[1] + v * self.y_axis[1],
        ]
    }

    fn user_to_display(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let (dx, dy) = (x - self.origin[0], y - self.origin[1]);
        let determinant = self.determinant();
        [
            (dx * self.y_axis[1] - dy * self.y_axis[0]) / determinant,
            (self.x_axis[0] * dy - self.x_axis[1] * dx) / determinant,
        ]
    }

    /// Görüntü uzayı dikdörtgeninin user space'teki eksen hizalı karşılığı.
    /// `/Rotate` 90'ın katı olduğundan köşeler yine eksen hizalı bir dikdörtgen oluşturur.
    pub(crate) fn normalized_to_user_rect(&self, rect: &NormalizedPageRect) -> RectData {
        let [left, bottom, right, top] = bounding_box(
            [
                [rect.x, rect.y],
                [rect.x + rect.width, rect.y],
                [rect.x, rect.y + rect.height],
                [rect.x + rect.width, rect.y + rect.height],
            ]
            .map(|[u, v]| self.display_to_user([f64::from(u), f64::from(v)])),
        );
        RectData {
            x: left as f32,
            y: bottom as f32,
            width: (right - left) as f32,
            height: (top - bottom) as f32,
        }
    }

    pub(crate) fn user_to_normalized_rect(&self, rect: &RectData) -> NormalizedPageRect {
        let [left, top, right, bottom] = bounding_box(
            [
                [rect.x, rect.y],
                [rect.x + rect.width, rect.y],
                [rect.x, rect.y + rect.height],
                [rect.x + rect.width, rect.y + rect.height],
            ]
            .map(|[x, y]| self.user_to_display([f64::from(x), f64::from(y)])),
        );
        NormalizedPageRect {
            x: left as f32,
            y: top as f32,
            width: (right - left) as f32,
            height: (bottom - top) as f32,
        }
    }
}

/// Noktaların eksen hizalı sınır kutusu: [min_x, min_y, max_x, max_y].
fn bounding_box(points: [[f64; 2]; 4]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[min_x, min_y, max_x, max_y], [x, y]| {
            [min_x.min(*x), min_y.min(*y), max_x.max(*x), max_y.max(*y)]
        },
    )
}

fn open_pdfium_doc<'a>(
    pdfium: &'a Pdfium,
    file_path: &str,
    password: Option<&'a str>,
) -> Result<PdfDocument<'a>, String> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err("Dosya bulunamadı".to_string());
    }

    pdfium
        .load_pdf_from_file(path, password)
        .map_err(|e| format!("Pdfium açılamadı: {:?}", e))
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        (r, g, b)
    } else {
        (0, 0, 0)
    }
}

fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

#[derive(Clone, Debug)]
struct AnnotationFingerprint {
    annotation_type: String,
    bounds: [f32; 4],
    contents: Option<String>,
}

impl AnnotationFingerprint {
    fn matches(&self, other: &Self) -> bool {
        self.annotation_type == other.annotation_type
            && self.contents == other.contents
            && self
                .bounds
                .iter()
                .zip(other.bounds)
                .all(|(left, right)| (*left - right).abs() <= 0.01)
    }
}

fn build_annotation_fingerprint<E: std::fmt::Display>(
    annotation_type: String,
    bounds: Result<PdfRect, E>,
    contents: Option<String>,
) -> Result<AnnotationFingerprint, DocumentIoFailure> {
    let bounds = bounds.map_err(|error| {
        DocumentIoFailure::invariant(format!("Annotation bounds okunamadı: {error}"))
    })?;
    Ok(AnnotationFingerprint {
        annotation_type,
        bounds: [
            bounds.left().value,
            bounds.bottom().value,
            bounds.right().value,
            bounds.top().value,
        ],
        contents: contents.filter(|contents| !contents.is_empty()),
    })
}

fn annotation_fingerprint(
    annotation: &PdfPageAnnotation<'_>,
) -> Result<AnnotationFingerprint, DocumentIoFailure> {
    build_annotation_fingerprint(
        format!("{:?}", annotation.annotation_type()),
        annotation.bounds(),
        annotation.contents(),
    )
}

fn page_annotation_fingerprints(
    document: &PdfDocument<'_>,
    page_index: u16,
) -> Result<Vec<AnnotationFingerprint>, DocumentIoFailure> {
    let page = document.pages().get(page_index).map_err(|error| {
        DocumentIoFailure::invariant(format!(
            "Sayfa {page_index} annotation doğrulaması için açılamadı: {error}"
        ))
    })?;
    let annotations = page.annotations();
    (0..annotations.len())
        .map(|index| {
            annotations
                .get(index)
                .map_err(|error| {
                    DocumentIoFailure::invariant(format!("Annotation {index} okunamadı: {error}"))
                })
                .and_then(|annotation| annotation_fingerprint(&annotation))
        })
        .collect()
}

fn validate_page_index(
    document: &PdfDocument<'_>,
    page_index: u32,
) -> Result<u16, DocumentIoFailure> {
    let page_count = document.pages().len();
    if page_index >= page_count as u32 {
        return Err(DocumentIoFailure::preflight(format!(
            "Sayfa indeksi aralık dışında: {page_index} >= {page_count}"
        )));
    }
    Ok(page_count)
}

pub(crate) fn validate_normalized_rect(rect: &NormalizedPageRect) -> Result<(), DocumentIoFailure> {
    let values = [rect.x, rect.y, rect.width, rect.height];
    if !values.iter().all(|value| value.is_finite()) || rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(DocumentIoFailure::preflight(
            "Annotation rect değerleri finite olmalı; width ve height pozitif olmalı",
        ));
    }
    let within_page = |start: f32, extent: f32| {
        start >= -NORMALIZED_RECT_EPSILON && start + extent <= 1.0 + NORMALIZED_RECT_EPSILON
    };
    if !within_page(rect.x, rect.width) || !within_page(rect.y, rect.height) {
        return Err(DocumentIoFailure::preflight(
            "Annotation rect normalize görüntü uzayında [0, 1] aralığında olmalı",
        ));
    }
    Ok(())
}

pub(crate) fn rect_data_from_pdf_rect(bounds: &PdfRect) -> RectData {
    RectData {
        x: bounds.left().value,
        y: bounds.bottom().value,
        width: bounds.width().value,
        height: bounds.height().value,
    }
}

/// `RectData` PDF user space'te x = left, y = bottom taşır (bkz. `read_page_annotations`).
/// pdfium-render `PdfRect::new` argüman sırası (bottom, left, top, right)'tır.
fn pdf_rect_from_rect_data(rect: &RectData) -> PdfRect {
    PdfRect::new(
        PdfPoints::new(rect.y),
        PdfPoints::new(rect.x),
        PdfPoints::new(rect.y + rect.height),
        PdfPoints::new(rect.x + rect.width),
    )
}

/// Text markup (Highlight/Underline) için tek dikdörtgenlik `/QuadPoints`.
///
/// Sıra: (x1, y1) üst-sol, (x2, y2) üst-sağ, (x3, y3) alt-sol, (x4, y4) alt-sağ.
/// PDF 1.7 §12.5.6.10 metni saat yönü tersine sıra tarif eder; ancak Acrobat bu
/// "Z" sırasını yazar/bekler ve PDFium da aynı varsayımla okur
/// (`CPDF_Annot::RectFromQuadPoints`: sol-alt = (x3, y3), sağ-üst = (x2, y2)).
/// pdfium-render `PdfQuadPoints::from_rect` alt-soldan başlayan saat yönü tersi
/// sıra üretir; PDFium onu sıfır genişlikli bir dikdörtgen olarak okur ve
/// görünüm akışı boş kalır. Bu yüzden `from_rect` kullanılmaz.
fn text_markup_quad_points(bounds: &PdfRect) -> PdfQuadPoints {
    PdfQuadPoints::new(
        bounds.left(),
        bounds.top(),
        bounds.right(),
        bounds.top(),
        bounds.left(),
        bounds.bottom(),
        bounds.right(),
        bounds.bottom(),
    )
}

/// Kaydedilen dosyada açık `/AP /N` görünüm akışı taşıması gereken tipler.
fn requires_generated_appearance(annotation_type: &str) -> bool {
    matches!(annotation_type, "Highlight" | "Underline")
}

/// Sayfanın son annotation'ı için normal görünüm akışını (`/AP /N`) PDFium'a ürettirir.
///
/// pdfium-render 0.8.37 annotation/page handle'larını public açmadığından
/// `FPDFAnnot_SetAP` mevcut nesne modelinden çağrılamıyor. PDFium'un kendi AP
/// üreticisi (`CPDF_GenerateAP`), `/AP`'si olmayan ve gizli olmayan annotation'lar
/// için sayfa annotation'larla render edilirken çalışır ve akışı annotation
/// sözlüğüne yazar; ardından yapılan kayıt bu akışı dosyaya taşır. Highlight için
/// `/C` renginde `re f` dolgusu ve `/BM /Multiply` ExtGState'i, Underline için
/// dikdörtgenin altında 1 pt `/C` renginde çizgi üretir.
///
/// Sayfadaki diğer annotation'lar render süresince gizlenir; aksi halde onların
/// görünümleri de dosyaya kalıcı olarak yazılırdı. Bayrakları sonra geri yüklenir
/// (`/F` hiç yoksa anlamca eşdeğer `/F 0` olarak yazılır).
fn generate_last_annotation_appearance(page: &mut PdfPage<'_>) -> Result<(), DocumentIoFailure> {
    let annotations = page.annotations_mut();
    let target_index = annotations.len().checked_sub(1).ok_or_else(|| {
        DocumentIoFailure::invariant("Görünüm akışı üretilecek annotation bulunamadı")
    })?;
    let mut hidden_for_render = Vec::new();
    for index in 0..target_index {
        let mut annotation = annotations.get(index).map_err(|error| {
            DocumentIoFailure::write(format!("Annotation {index} okunamadı: {error}"))
        })?;
        if !annotation.is_hidden() {
            annotation.set_is_hidden(true).map_err(|error| {
                DocumentIoFailure::write(format!("Annotation {index} gizlenemedi: {error}"))
            })?;
            hidden_for_render.push(index);
        }
    }

    // Bitmap içeriği kullanılmaz; yalnız PDFium'un annotation listesini kurması gerekir.
    let render_result = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(16)
                .render_annotations(true)
                .render_form_data(false),
        )
        .map(|_| ());

    let annotations = page.annotations_mut();
    for index in hidden_for_render {
        annotations
            .get(index)
            .and_then(|mut annotation| annotation.set_is_hidden(false))
            .map_err(|error| {
                DocumentIoFailure::write(format!(
                    "Annotation {index} görünürlüğü geri yüklenemedi: {error}"
                ))
            })?;
    }
    render_result.map_err(|error| {
        DocumentIoFailure::write(format!("Annotation görünüm akışı üretilemedi: {error}"))
    })
}

fn last_annotation_has_appearance_objects(
    document: &PdfDocument<'_>,
    page_index: u16,
) -> Result<bool, DocumentIoFailure> {
    let page = document.pages().get(page_index).map_err(|error| {
        DocumentIoFailure::invariant(format!(
            "Sayfa {page_index} görünüm akışı doğrulaması için açılamadı: {error}"
        ))
    })?;
    let annotations = page.annotations();
    let last = annotations
        .len()
        .checked_sub(1)
        .ok_or_else(|| DocumentIoFailure::invariant("Doğrulanacak annotation bulunamadı"))
        .and_then(|index| {
            annotations.get(index).map_err(|error| {
                DocumentIoFailure::invariant(format!("Annotation {index} okunamadı: {error}"))
            })
        })?;
    // FPDFAnnot_GetObjectCount AP üretmez; yalnız dosyadaki `/AP /N` akışını sayar.
    Ok(!last.objects().is_empty())
}

/// Annotation'ın UI'da gösterilecek rengi.
///
/// `/AP` taşıyan annotation'larda PDFium `FPDFAnnot_GetColor`'ı başarısız döndürür ve
/// pdfium-render 0.8.37 bu durumda annotation handle'ını page object handle'ı gibi
/// `FPDFPageObj_Get*Color`'a geçirir. O yola girmemek için AP nesnesi varsa renk
/// görünüm akışındaki ilk path nesnesinden okunur.
fn annotation_display_color(annotation: &PdfPageAnnotation<'_>) -> PdfColor {
    const FALLBACK: PdfColor = PdfColor::new(0, 0, 0, 255);
    if let Some(object) = annotation.objects().iter().next() {
        return object
            .as_path_object()
            .and_then(|path| match path.fill_mode() {
                Ok(PdfPathFillMode::None) => path.stroke_color().ok(),
                Ok(_) => path.fill_color().ok(),
                Err(_) => None,
            })
            .unwrap_or(FALLBACK);
    }
    annotation.fill_color().unwrap_or(FALLBACK)
}

fn expected_annotation_type(value: &str) -> Result<&'static str, DocumentIoFailure> {
    match value {
        "Highlight" => Ok("Highlight"),
        "Underline" => Ok("Underline"),
        "Text" => Ok("Text"),
        _ => Err(DocumentIoFailure::preflight(format!(
            "Desteklenmeyen annotation tipi: {value}"
        ))),
    }
}

pub(crate) fn read_page_annotations(
    document: &PdfDocument<'_>,
    page_index: u32,
) -> Result<Vec<AnnotationData>, String> {
    let page_index = u16::try_from(page_index)
        .map_err(|_| "Sayfa indeksi PDFium sınırını aşıyor".to_string())?;
    let page = document
        .pages()
        .get(page_index)
        .map_err(|e| format!("Sayfa bulunamadı: {:?}", e))?;

    let display = PageDisplayTransform::for_page(&page)?;
    let annots = page.annotations();
    let mut result = Vec::new();

    for i in 0..annots.len() {
        if let Ok(annot) = annots.get(i) {
            let color = annotation_display_color(&annot);
            // `/Rect` PDF user space'tedir; frontend'e görüntü uzayında normalize gider.
            let normalized_rect = annot
                .bounds()
                .ok()
                .map(|bounds| display.user_to_normalized_rect(&rect_data_from_pdf_rect(&bounds)));

            let annot_type = format!("{:?}", annot.annotation_type());
            let contents = annot.contents();

            result.push(AnnotationData {
                id: i.to_string(),
                annot_type,
                color: rgb_to_hex(color.red(), color.green(), color.blue()),
                normalized_rect,
                contents,
            });
        }
    }

    Ok(result)
}

pub(crate) fn add_annotation_report(
    file_path: String,
    page_index: u32,
    annot_type: String,
    normalized_rect: NormalizedPageRect,
    color: String,
    contents: Option<String>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    let expected_type = expected_annotation_type(&annot_type)
        .map_err(|error| {
            DocumentIoError::new(
                "add_annotation",
                error.stage,
                CommitState::Prepared,
                error.detail,
            )
        })?
        .to_string();
    validate_normalized_rect(&normalized_rect).map_err(|error| {
        DocumentIoError::new(
            "add_annotation",
            error.stage,
            CommitState::Prepared,
            error.detail,
        )
    })?;

    atomic_write_pdf(
        "add_annotation",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let mut document = open_pdfium_doc(
                pdfium,
                source.to_string_lossy().as_ref(),
                password.as_deref(),
            )
            .map_err(DocumentIoFailure::preflight)?;
            ensure_unprotected(&document)?;
            let page_count = validate_page_index(&document, page_index)?;
            let before = page_annotation_fingerprints(&document, page_index as u16)?;
            let (red, green, blue) = hex_to_rgb(&color);
            let pdf_color = PdfColor::new(red, green, blue, 255);
            let bounds = {
                let page = document.pages().get(page_index as u16).map_err(|error| {
                    DocumentIoFailure::preflight(format!("Sayfa bulunamadı: {error}"))
                })?;
                let display =
                    PageDisplayTransform::for_page(&page).map_err(DocumentIoFailure::preflight)?;
                pdf_rect_from_rect_data(&display.normalized_to_user_rect(&normalized_rect))
            };
            {
                let mut page = document
                    .pages_mut()
                    .get(page_index as u16)
                    .map_err(|error| {
                        DocumentIoFailure::preflight(format!("Sayfa bulunamadı: {error}"))
                    })?;
                let annotations = page.annotations_mut();
                match annot_type.as_str() {
                    "Highlight" => {
                        let mut annotation =
                            annotations.create_highlight_annotation().map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Highlight annotation oluşturulamadı: {error}"
                                ))
                            })?;
                        // Text markup rengi spec gereği `/C`'dir; `/IC` bu tiplerde tanımsızdır.
                        // FPDFAnnot_SetColor AP varken başarısız olduğundan AP'den önce ayarlanır.
                        annotation.set_stroke_color(pdf_color).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation rengi ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation.set_bounds(bounds).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation bounds ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation
                            .attachment_points_mut()
                            .create_attachment_point_at_end(text_markup_quad_points(&bounds))
                            .map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation QuadPoints ayarlanamadı: {error}"
                                ))
                            })?;
                        if let Some(value) = contents.as_deref() {
                            annotation.set_contents(value).map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation içeriği ayarlanamadı: {error}"
                                ))
                            })?;
                        }
                    }
                    "Underline" => {
                        let mut annotation =
                            annotations.create_underline_annotation().map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Underline annotation oluşturulamadı: {error}"
                                ))
                            })?;
                        // Text markup rengi spec gereği `/C`'dir; `/IC` bu tiplerde tanımsızdır.
                        // FPDFAnnot_SetColor AP varken başarısız olduğundan AP'den önce ayarlanır.
                        annotation.set_stroke_color(pdf_color).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation rengi ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation.set_bounds(bounds).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation bounds ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation
                            .attachment_points_mut()
                            .create_attachment_point_at_end(text_markup_quad_points(&bounds))
                            .map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation QuadPoints ayarlanamadı: {error}"
                                ))
                            })?;
                        if let Some(value) = contents.as_deref() {
                            annotation.set_contents(value).map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation içeriği ayarlanamadı: {error}"
                                ))
                            })?;
                        }
                    }
                    "Text" => {
                        let mut annotation =
                            annotations.create_text_annotation("").map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Text annotation oluşturulamadı: {error}"
                                ))
                            })?;
                        annotation.set_fill_color(pdf_color).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation rengi ayarlanamadı: {error}"
                            ))
                        })?;
                        annotation.set_bounds(bounds).map_err(|error| {
                            DocumentIoFailure::write(format!(
                                "Annotation bounds ayarlanamadı: {error}"
                            ))
                        })?;
                        if let Some(value) = contents.as_deref() {
                            annotation.set_contents(value).map_err(|error| {
                                DocumentIoFailure::write(format!(
                                    "Annotation içeriği ayarlanamadı: {error}"
                                ))
                            })?;
                        }
                    }
                    _ => {
                        return Err(DocumentIoFailure::preflight(format!(
                            "Desteklenmeyen annotation tipi: {annot_type}"
                        )))
                    }
                }
                if requires_generated_appearance(&annot_type) {
                    generate_last_annotation_appearance(&mut page)?;
                }
            }
            document.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            let expected = AnnotationFingerprint {
                annotation_type: expected_type.clone(),
                bounds: [
                    bounds.left().value,
                    bounds.bottom().value,
                    bounds.right().value,
                    bounds.top().value,
                ],
                contents: contents.clone().filter(|contents| !contents.is_empty()),
            };
            Ok((page_count, before.len(), expected))
        },
        move |document, (page_count, previous_count, expected)| {
            if document.pages().len() != *page_count {
                return Err(DocumentIoFailure::invariant(
                    "Annotation ekleme sayfa sayısını değiştirdi",
                ));
            }
            let after = page_annotation_fingerprints(document, page_index as u16)?;
            if after.len() != previous_count + 1 {
                return Err(DocumentIoFailure::invariant(format!(
                    "Annotation sayısı +1 olmadı: önce {previous_count}, sonra {}",
                    after.len()
                )));
            }
            if !after
                .last()
                .is_some_and(|fingerprint| fingerprint.matches(expected))
            {
                return Err(DocumentIoFailure::invariant(
                    "Eklenen annotation type/bounds/contents ile doğrulanamadı",
                ));
            }
            if requires_generated_appearance(&expected.annotation_type)
                && !last_annotation_has_appearance_objects(document, page_index as u16)?
            {
                return Err(DocumentIoFailure::invariant(format!(
                    "Eklenen {} annotation'ın normal görünüm akışı (/AP /N) yok veya boş",
                    expected.annotation_type
                )));
            }
            Ok(())
        },
    )
}

pub(crate) fn delete_annotation_report(
    file_path: String,
    page_index: u32,
    annot_id: String,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let annotation_index = annot_id.parse::<usize>().map_err(|_| {
        DocumentIoError::new(
            "delete_annotation",
            DocumentIoStage::Preflight,
            CommitState::Prepared,
            "Annotation ID geçerli bir indeks değil",
        )
    })?;
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        "delete_annotation",
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            let mut document = open_pdfium_doc(
                pdfium,
                source.to_string_lossy().as_ref(),
                password.as_deref(),
            )
            .map_err(DocumentIoFailure::preflight)?;
            ensure_unprotected(&document)?;
            let page_count = validate_page_index(&document, page_index)?;
            let before = page_annotation_fingerprints(&document, page_index as u16)?;
            let target_fingerprint = before.get(annotation_index).cloned().ok_or_else(|| {
                DocumentIoFailure::preflight(format!(
                    "Annotation indeksi aralık dışında: {annotation_index} >= {}",
                    before.len()
                ))
            })?;
            let matching_before = before
                .iter()
                .filter(|item| item.matches(&target_fingerprint))
                .count();
            {
                let mut page = document
                    .pages_mut()
                    .get(page_index as u16)
                    .map_err(|error| {
                        DocumentIoFailure::preflight(format!("Sayfa bulunamadı: {error}"))
                    })?;
                let annotations = page.annotations_mut();
                let annotation = annotations.get(annotation_index).map_err(|error| {
                    DocumentIoFailure::preflight(format!("Annotation bulunamadı: {error}"))
                })?;
                annotations.delete_annotation(annotation).map_err(|error| {
                    DocumentIoFailure::write(format!("Annotation silinemedi: {error}"))
                })?;
            }
            document.save_to_file(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF kaydedilemedi: {error}"))
            })?;
            Ok((
                page_count,
                before.len(),
                target_fingerprint,
                matching_before,
            ))
        },
        move |document, (page_count, previous_count, deleted, matching_before)| {
            if document.pages().len() != *page_count {
                return Err(DocumentIoFailure::invariant(
                    "Annotation silme sayfa sayısını değiştirdi",
                ));
            }
            let after = page_annotation_fingerprints(document, page_index as u16)?;
            if after.len() + 1 != *previous_count {
                return Err(DocumentIoFailure::invariant(format!(
                    "Annotation sayısı -1 olmadı: önce {previous_count}, sonra {}",
                    after.len()
                )));
            }
            let matching_after = after.iter().filter(|item| item.matches(deleted)).count();
            if matching_after + 1 != *matching_before {
                return Err(DocumentIoFailure::invariant(
                    "Silinen annotation fingerprint'i çıktıdan kaybolmadı",
                ));
            }
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pdf")
            .join("annotations.pdf")
    }

    /// Döndürülmemiş, MediaBox kökeni (0, 0) olan bir sayfada user space dikdörtgenini
    /// görüntü uzayına elle çevirir. `PageDisplayTransform`'u kullanmaz; böylece bu
    /// yardımcıyı kullanan testlerin beklenen değerleri dönüşümden bağımsız kalır.
    fn unrotated_page_normalized_rect(
        path: &Path,
        page_index: u32,
        rect: &RectData,
    ) -> NormalizedPageRect {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            let page = document
                .pages()
                .get(page_index as u16)
                .map_err(|error| error.to_string())?;
            assert_eq!(page.rotation().unwrap(), PdfPageRenderRotation::None);
            let media = page.boundaries().media().unwrap().bounds;
            assert_eq!((media.left().value, media.bottom().value), (0.0, 0.0));
            assert!(page.boundaries().crop().is_err(), "CropBox beklenmiyor");
            let (width, height) = (page.width().value, page.height().value);
            Ok(NormalizedPageRect {
                x: rect.x / width,
                y: (height - rect.y - rect.height) / height,
                width: rect.width / width,
                height: rect.height / height,
            })
        })
        .unwrap()
    }

    /// Testlerdeki user space dikdörtgenini (döndürülmemiş sayfa) IPC sözleşmesine çevirip ekler.
    fn add_annotation_with_password(
        file_path: String,
        page_index: u32,
        annot_type: String,
        rect: RectData,
        color: String,
        contents: Option<String>,
        password: Option<String>,
    ) -> Result<String, String> {
        let normalized_rect =
            unrotated_page_normalized_rect(Path::new(&file_path), page_index, &rect);
        add_annotation_report(
            file_path,
            page_index,
            annot_type,
            normalized_rect,
            color,
            contents,
            password,
        )
        .map(|_| "ok".to_string())
        .map_err(|error| error.to_string())
    }

    fn delete_annotation_with_password(
        file_path: String,
        page_index: u32,
        annot_id: String,
        password: Option<String>,
    ) -> Result<(), String> {
        delete_annotation_report(file_path, page_index, annot_id, password)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn annotation_fingerprints(path: &Path) -> Vec<AnnotationFingerprint> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            page_annotation_fingerprints(&document, 0).map_err(|error| error.detail)
        })
        .unwrap()
    }

    #[test]
    fn document_io_annotation_add_and_delete_are_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        assert_eq!(annotation_fingerprints(&path).len(), 1);

        add_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "Highlight".to_string(),
            RectData {
                x: 100.0,
                y: 100.0,
                width: 80.0,
                height: 20.0,
            },
            "#FFCC00".to_string(),
            Some("Atomic highlight".to_string()),
            None,
        )
        .unwrap();
        let after_add = annotation_fingerprints(&path);
        assert_eq!(after_add.len(), 2);
        assert_eq!(
            after_add.last().and_then(|item| item.contents.as_deref()),
            Some("Atomic highlight")
        );

        delete_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "0".to_string(),
            None,
        )
        .unwrap();
        let after_delete = annotation_fingerprints(&path);
        assert_eq!(after_delete.len(), 1);
        assert!(!after_delete
            .iter()
            .any(|item| item.contents.as_deref() == Some("Fixture note")));
    }

    #[test]
    fn document_io_invalid_annotation_type_preserves_original_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        let original = fs::read(&path).unwrap();

        let error = add_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "Unknown".to_string(),
            RectData {
                x: 1.0,
                y: 1.0,
                width: 10.0,
                height: 10.0,
            },
            "#FFFFFF".to_string(),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("Desteklenmeyen annotation"));
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn document_io_invalid_annotation_id_preserves_original_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        let original = fs::read(&path).unwrap();

        let error = delete_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            "99".to_string(),
            None,
        )
        .unwrap_err();

        assert!(error.contains("aralık dışında"), "{error}");
        assert_eq!(fs::read(path).unwrap(), original);
    }

    fn read_annotations(path: &Path) -> Vec<AnnotationData> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            read_page_annotations(&document, 0)
        })
        .unwrap()
    }

    fn assert_normalized_rect_eq(actual: &NormalizedPageRect, expected: &NormalizedPageRect) {
        const TOLERANCE: f32 = 1e-5;
        for (field, read, written) in [
            ("x", actual.x, expected.x),
            ("y", actual.y, expected.y),
            ("width", actual.width, expected.width),
            ("height", actual.height, expected.height),
        ] {
            assert!(
                (read - written).abs() <= TOLERANCE,
                "{field}: okunan {read}, yazılan {written}; okunan {actual:?}, yazılan {expected:?}"
            );
        }
    }

    #[test]
    fn pdf_rect_from_rect_data_maps_x_to_left_and_y_to_bottom() {
        let bounds = pdf_rect_from_rect_data(&RectData {
            x: 100.0,
            y: 500.0,
            width: 50.0,
            height: 20.0,
        });

        assert_eq!(bounds.left().value, 100.0);
        assert_eq!(bounds.bottom().value, 500.0);
        assert_eq!(bounds.right().value, 150.0);
        assert_eq!(bounds.top().value, 520.0);
    }

    #[test]
    fn added_annotation_rect_round_trips_through_production_read_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        // Asimetrik rect: x≠y ve width≠height olmalı ki eksen takası yakalansın.
        let rect = RectData {
            x: 100.0,
            y: 500.0,
            width: 50.0,
            height: 20.0,
        };

        for annot_type in ["Highlight", "Underline", "Text"] {
            add_annotation_with_password(
                path.to_string_lossy().to_string(),
                0,
                annot_type.to_string(),
                rect.clone(),
                "#FFCC00".to_string(),
                Some(format!("{annot_type} round-trip")),
                None,
            )
            .unwrap();

            let annotations = read_annotations(&path);
            let added = annotations.last().unwrap();
            assert_eq!(added.annot_type, annot_type);
            assert_eq!(
                added.contents.as_deref(),
                Some(format!("{annot_type} round-trip").as_str())
            );
            // 612x792 sayfada elle hesaplanan görüntü uzayı karşılığı.
            assert_normalized_rect_eq(
                added.normalized_rect.as_ref().unwrap(),
                &NormalizedPageRect {
                    x: 100.0 / 612.0,
                    y: 272.0 / 792.0,
                    width: 50.0 / 612.0,
                    height: 20.0 / 792.0,
                },
            );
            let bounds = annotation_fingerprints(&path).last().unwrap().bounds;
            for (read, written) in bounds.into_iter().zip([100.0, 500.0, 150.0, 520.0]) {
                assert!(
                    (read - written).abs() <= 0.01,
                    "{annot_type} user space bounds: okunan {bounds:?}"
                );
            }
            assert_eq!(added.color, "#FFCC00", "{annot_type} rengi");
        }
    }

    fn named_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pdf")
            .join(name)
    }

    /// Sayfayı 1 px = 1 pt ölçekle, annotation'larla birlikte beyaz zemine render eder.
    fn render_page_with_annotations(path: &Path, page_index: u16) -> (usize, usize, Vec<u8>) {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            let page = document
                .pages()
                .get(page_index)
                .map_err(|error| error.to_string())?;
            let bitmap = page
                .render_with_config(
                    &PdfRenderConfig::new()
                        .scale_page_by_factor(1.0)
                        .set_clear_color(PdfColor::WHITE)
                        .render_annotations(true),
                )
                .map_err(|error| error.to_string())?;
            Ok((
                bitmap.width() as usize,
                bitmap.height() as usize,
                bitmap.as_rgba_bytes(),
            ))
        })
        .unwrap()
    }

    /// PDF user space dikdörtgeninin bitmap satır/sütun aralığı: (left, top, right, bottom), bitiş hariç.
    fn pixel_bounds(rect: &RectData, page_height: usize) -> (usize, usize, usize, usize) {
        (
            rect.x as usize,
            page_height - (rect.y + rect.height) as usize,
            (rect.x + rect.width) as usize,
            page_height - rect.y as usize,
        )
    }

    fn is_non_white(pixel: &[u8]) -> bool {
        pixel[0] < 250 || pixel[1] < 250 || pixel[2] < 250
    }

    struct PixelCensus {
        inside: usize,
        outside: usize,
        inside_rows: Vec<usize>,
    }

    fn census(width: usize, rgba: &[u8], bounds: (usize, usize, usize, usize)) -> PixelCensus {
        let (left, top, right, bottom) = bounds;
        let mut census = PixelCensus {
            inside: 0,
            outside: 0,
            inside_rows: Vec::new(),
        };
        for (index, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
            if !is_non_white(pixel) {
                continue;
            }
            let (x, y) = (index % width, index / width);
            if (left..right).contains(&x) && (top..bottom).contains(&y) {
                census.inside += 1;
                if !census.inside_rows.contains(&y) {
                    census.inside_rows.push(y);
                }
            } else {
                census.outside += 1;
            }
        }
        census
    }

    fn add_markup(path: &Path, annot_type: &str, rect: &RectData, color: &str) {
        add_annotation_with_password(
            path.to_string_lossy().to_string(),
            0,
            annot_type.to_string(),
            rect.clone(),
            color.to_string(),
            None,
            None,
        )
        .unwrap();
    }

    fn markup_rect() -> RectData {
        // Tamsayı koordinatlar: 1 pt = 1 px ölçekte kenar antialias'ı oluşmaz.
        RectData {
            x: 100.0,
            y: 500.0,
            width: 200.0,
            height: 20.0,
        }
    }

    #[test]
    fn text_markup_quad_points_use_acrobat_corner_order() {
        let quad = text_markup_quad_points(&pdf_rect_from_rect_data(&markup_rect()));

        let corners = [
            (quad.x1.value, quad.y1.value),
            (quad.x2.value, quad.y2.value),
            (quad.x3.value, quad.y3.value),
            (quad.x4.value, quad.y4.value),
        ];
        assert_eq!(
            corners,
            [
                (100.0, 520.0),
                (300.0, 520.0),
                (100.0, 500.0),
                (300.0, 500.0)
            ]
        );
    }

    #[test]
    fn saved_highlight_is_painted_by_pdfium_only_inside_its_rect() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("highlight.pdf");
        fs::copy(named_fixture("minimal.pdf"), &path).unwrap();
        let rect = markup_rect();

        add_markup(&path, "Highlight", &rect, "#FFCC00");

        let bytes = fs::read(&path).unwrap();
        for key in [&b"/AP"[..], b"/QuadPoints", b"/C[", b"/BM/Multiply"] {
            assert!(
                bytes.windows(key.len()).any(|window| window == key),
                "kaydedilen dosyada {} yok",
                String::from_utf8_lossy(key)
            );
        }
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let document = pdfium
                .load_pdf_from_file(&path, None)
                .map_err(|error| error.to_string())?;
            let page = document.pages().get(0).map_err(|error| error.to_string())?;
            let annotation = page
                .annotations()
                .get(0)
                .map_err(|error| error.to_string())?;
            assert!(!annotation.objects().is_empty(), "/AP /N akışı boş");
            assert_eq!(
                annotation.attachment_points().first().unwrap(),
                text_markup_quad_points(&pdf_rect_from_rect_data(&rect))
            );
            Ok(())
        })
        .unwrap();

        let (width, height, rgba) = render_page_with_annotations(&path, 0);
        assert_eq!((width, height), (612, 792));
        let bounds = pixel_bounds(&rect, height);
        let census = census(width, &rgba, bounds);
        eprintln!(
            "highlight: inside={} outside={} area={}",
            census.inside,
            census.outside,
            200 * 20
        );
        assert_eq!(
            census.inside,
            200 * 20,
            "highlight dikdörtgeni tam boyanmalı"
        );
        assert_eq!(census.outside, 0, "highlight dikdörtgen dışına taşmamalı");
        let center = ((bounds.1 + 10) * width + bounds.0 + 100) * 4;
        assert_eq!(&rgba[center..center + 3], &[255, 204, 0]);

        let annotations = read_annotations(&path);
        assert_eq!(annotations[0].color, "#FFCC00");
    }

    #[test]
    fn saved_underline_is_painted_by_pdfium_as_line_at_rect_bottom() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("underline.pdf");
        fs::copy(named_fixture("minimal.pdf"), &path).unwrap();
        let rect = markup_rect();

        add_markup(&path, "Underline", &rect, "#0033CC");

        let (width, height, rgba) = render_page_with_annotations(&path, 0);
        let bounds = pixel_bounds(&rect, height);
        let census = census(width, &rgba, bounds);
        eprintln!(
            "underline: inside={} outside={} rows={:?} rect_rows={}..{}",
            census.inside, census.outside, census.inside_rows, bounds.1, bounds.3
        );
        assert!(census.inside > 0, "underline hiç boyanmadı");
        assert_eq!(census.outside, 0, "underline dikdörtgen dışına taşmamalı");
        assert!(
            census.inside_rows.iter().all(|row| *row >= bounds.3 - 3),
            "underline yalnız dikdörtgenin alt 3 px'inde olmalı: {:?}",
            census.inside_rows
        );

        let annotations = read_annotations(&path);
        assert_eq!(annotations[0].color, "#0033CC");
    }

    #[test]
    fn markup_appearance_generation_does_not_bake_other_annotations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();

        add_markup(&path, "Highlight", &markup_rect(), "#FFCC00");

        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let document = pdfium
                .load_pdf_from_file(&path, None)
                .map_err(|error| error.to_string())?;
            let page = document.pages().get(0).map_err(|error| error.to_string())?;
            let annotations = page.annotations();
            assert_eq!(annotations.len(), 2);
            let note = annotations.get(0).map_err(|error| error.to_string())?;
            assert_eq!(note.contents().as_deref(), Some("Fixture note"));
            assert!(
                note.objects().is_empty(),
                "fixture notunun AP'si yazılmamalı"
            );
            assert!(!note.is_hidden(), "fixture notu görünür kalmalı");
            let highlight = annotations.get(1).map_err(|error| error.to_string())?;
            assert!(!highlight.objects().is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn highlighted_text_stays_extractable_and_visible() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release-smoke.pdf");
        fs::copy(named_fixture("release-smoke.pdf"), &path).unwrap();
        // Helvetica 12 pt `RELEASE_SMOKE_ALPHA_TOKEN` satırı, taban çizgisi y = 720.
        let rect = RectData {
            x: 45.0,
            y: 715.0,
            width: 200.0,
            height: 20.0,
        };
        let dark_pixels = |path: &Path| {
            let (width, height, rgba) = render_page_with_annotations(path, 0);
            let (left, top, right, bottom) = pixel_bounds(&rect, height);
            rgba.as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                .filter(|(index, pixel)| {
                    let (x, y) = (index % width, index / width);
                    (left..right).contains(&x)
                        && (top..bottom).contains(&y)
                        && pixel[..3].iter().all(|channel| *channel < 80)
                })
                .count()
        };
        let dark_before = dark_pixels(&path);

        add_markup(&path, "Highlight", &rect, "#FFFF00");

        let dark_after = dark_pixels(&path);
        eprintln!("highlighted text: dark pixels before={dark_before} after={dark_after}");
        assert!(dark_before > 0);
        assert!(
            dark_after >= dark_before,
            "highlight metni örtmemeli: önce {dark_before}, sonra {dark_after}"
        );
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let document = pdfium
                .load_pdf_from_file(&path, None)
                .map_err(|error| error.to_string())?;
            let page = document.pages().get(0).map_err(|error| error.to_string())?;
            let text = page.text().map_err(|error| error.to_string())?;
            assert_eq!(
                text.inside_rect(pdf_rect_from_rect_data(&rect)).trim(),
                "RELEASE_SMOKE_ALPHA_TOKEN"
            );
            assert!(text.all().contains("RELEASE_SMOKE_ALPHA_TOKEN"));
            Ok(())
        })
        .unwrap();
    }

    fn redaction_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("redaction")
            .join(name)
    }

    /// `cropbox-mediabox-diff.pdf`'ten türetilir: `/Rotate 90` ve asimetrik CropBox
    /// `[30 70 530 720]` (MediaBox `[0 0 600 800]`). İkisi birden yalnız bu sayfada var.
    fn rotated_offset_cropbox_copy(directory: &Path) -> PathBuf {
        let path = directory.join("rotated-offset-cropbox.pdf");
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let mut document = pdfium
                .load_pdf_from_file(&redaction_fixture("cropbox-mediabox-diff.pdf"), None)
                .map_err(|error| error.to_string())?;
            {
                let mut page = document
                    .pages_mut()
                    .get(0)
                    .map_err(|error| error.to_string())?;
                page.set_rotation(PdfPageRenderRotation::Degrees90);
                page.boundaries_mut()
                    .set_crop(PdfRect::new_from_values(70.0, 30.0, 720.0, 530.0))
                    .map_err(|error| error.to_string())?;
            }
            document
                .save_to_file(&path)
                .map_err(|error| error.to_string())
        })
        .unwrap();
        path
    }

    /// Görüntü uzayı geometri vakası. `expected_user` elle türetilmiştir:
    /// `/Rotate` sayfayı görüntülerken saat yönünde döndürür (PDF 32000-1 §7.7.3.3)
    /// ve görüntü CropBox'ı kaplar.
    struct DisplayCase {
        name: &'static str,
        path: PathBuf,
        page_index: u16,
        display_size: (usize, usize),
        normalized: NormalizedPageRect,
        expected_user: RectData,
    }

    fn normalized(x: f32, y: f32, width: f32, height: f32) -> NormalizedPageRect {
        NormalizedPageRect {
            x,
            y,
            width,
            height,
        }
    }

    fn user(x: f32, y: f32, width: f32, height: f32) -> RectData {
        RectData {
            x,
            y,
            width,
            height,
        }
    }

    /// Kare olmayan sayfalar; normalize değerler görüntüde tamsayı piksele denk gelir.
    fn display_cases(directory: &Path) -> Vec<DisplayCase> {
        let rotations = redaction_fixture("rotations.pdf");
        vec![
            // MediaBox [0 0 600 800]: x = 600u, y = 800 − 800v.
            DisplayCase {
                name: "/Rotate 0",
                path: rotations.clone(),
                page_index: 0,
                display_size: (600, 800),
                normalized: normalized(0.1, 0.25, 0.3, 0.05),
                expected_user: user(60.0, 560.0, 180.0, 40.0),
            },
            // Görüntü sol-üst = user (0, 0); sağa gidiş +y, aşağı gidiş +x.
            DisplayCase {
                name: "/Rotate 90",
                path: rotations.clone(),
                page_index: 1,
                display_size: (800, 600),
                normalized: normalized(0.1, 0.25, 0.3, 0.05),
                expected_user: user(150.0, 80.0, 30.0, 240.0),
            },
            // Görüntü sol-üst = user (600, 0); sağa gidiş −x, aşağı gidiş +y.
            DisplayCase {
                name: "/Rotate 180",
                path: rotations.clone(),
                page_index: 2,
                display_size: (600, 800),
                normalized: normalized(0.1, 0.25, 0.3, 0.05),
                expected_user: user(360.0, 200.0, 180.0, 40.0),
            },
            // Görüntü sol-üst = user (600, 800); sağa gidiş −y, aşağı gidiş −x.
            DisplayCase {
                name: "/Rotate 270",
                path: rotations,
                page_index: 3,
                display_size: (800, 600),
                normalized: normalized(0.1, 0.25, 0.3, 0.05),
                expected_user: user(420.0, 480.0, 30.0, 240.0),
            },
            // CropBox [50 50 550 750]: x = 50 + 500u, y = 750 − 700v.
            DisplayCase {
                name: "CropBox origin (50, 50)",
                path: redaction_fixture("cropbox-mediabox-diff.pdf"),
                page_index: 0,
                display_size: (500, 700),
                normalized: normalized(0.1, 0.25, 0.3, 0.05),
                expected_user: user(100.0, 540.0, 150.0, 35.0),
            },
            // CropBox [30 70 530 720] + /Rotate 90: görüntü sol-üst = user (30, 70);
            // x = 30 + 500v, y = 70 + 650u.
            DisplayCase {
                name: "/Rotate 90 + CropBox origin (30, 70)",
                path: rotated_offset_cropbox_copy(directory),
                page_index: 0,
                display_size: (650, 500),
                normalized: normalized(0.2, 0.1, 0.4, 0.06),
                expected_user: user(80.0, 200.0, 30.0, 260.0),
            },
        ]
    }

    fn display_transform(path: &Path, page_index: u16) -> PageDisplayTransform {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            let page = document
                .pages()
                .get(page_index)
                .map_err(|error| error.to_string())?;
            PageDisplayTransform::for_page(&page)
        })
        .unwrap()
    }

    fn assert_user_rect_eq(case: &str, actual: &RectData, expected: &RectData) {
        for (field, read, written) in [
            ("x", actual.x, expected.x),
            ("y", actual.y, expected.y),
            ("width", actual.width, expected.width),
            ("height", actual.height, expected.height),
        ] {
            assert!(
                (read - written).abs() <= 0.01,
                "{case} {field}: hesaplanan {actual:?}, beklenen {expected:?}"
            );
        }
    }

    #[test]
    fn page_display_transform_matches_hand_derived_user_space_rects() {
        let directory = tempfile::tempdir().unwrap();
        for case in display_cases(directory.path()) {
            let transform = display_transform(&case.path, case.page_index);

            let user_rect = transform.normalized_to_user_rect(&case.normalized);
            assert_user_rect_eq(case.name, &user_rect, &case.expected_user);
            assert_normalized_rect_eq(
                &transform.user_to_normalized_rect(&case.expected_user),
                &case.normalized,
            );
        }
    }

    #[test]
    fn page_display_transform_rejects_degenerate_corners() {
        let error =
            PageDisplayTransform::from_corners([0.0, 0.0], [10.0, 0.0], [20.0, 0.0]).unwrap_err();
        assert!(error.contains("tekil"), "{error}");
    }

    #[test]
    fn normalized_rect_outside_unit_square_is_rejected_before_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("annotations.pdf");
        fs::copy(fixture(), &path).unwrap();
        let original = fs::read(&path).unwrap();

        for rect in [
            normalized(-0.01, 0.1, 0.2, 0.2),
            normalized(0.9, 0.1, 0.2, 0.2),
            normalized(0.1, 0.9, 0.2, 0.2),
            normalized(0.1, 0.1, 0.0, 0.2),
            // Eski sözleşmenin point değerleri artık reddedilir.
            normalized(100.0, 500.0, 50.0, 20.0),
        ] {
            let error = add_annotation_report(
                path.to_string_lossy().to_string(),
                0,
                "Highlight".to_string(),
                rect.clone(),
                "#FFCC00".to_string(),
                None,
                None,
            )
            .unwrap_err();
            assert_eq!(error.stage, DocumentIoStage::Preflight, "{rect:?}");
        }
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    fn read_annotations_on(path: &Path, page_index: u32) -> Vec<AnnotationData> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            read_page_annotations(&document, page_index)
        })
        .unwrap()
    }

    #[test]
    fn display_space_rect_lands_on_the_same_pixels_in_the_pdfium_render() {
        let directory = tempfile::tempdir().unwrap();
        for (index, case) in display_cases(directory.path()).into_iter().enumerate() {
            let path = directory.path().join(format!("case-{index}.pdf"));
            fs::copy(&case.path, &path).unwrap();
            let (_, _, before) = render_page_with_annotations(&path, case.page_index);

            add_annotation_report(
                path.to_string_lossy().to_string(),
                u32::from(case.page_index),
                "Highlight".to_string(),
                case.normalized.clone(),
                "#FFCC00".to_string(),
                None,
                None,
            )
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));

            // Render, sayfanın kendi /Rotate'i ve CropBox'ıyla yapılır (viewer rotation yok);
            // frontend'in görüntü uzayı tam olarak bu bitmap'tir.
            let (width, height, after) = render_page_with_annotations(&path, case.page_index);
            assert_eq!((width, height), case.display_size, "{}", case.name);
            let (display_width, display_height) = (width as f32, height as f32);
            let rect = &case.normalized;
            let left = (rect.x * display_width).round() as usize;
            let top = (rect.y * display_height).round() as usize;
            let right = ((rect.x + rect.width) * display_width).round() as usize;
            let bottom = ((rect.y + rect.height) * display_height).round() as usize;

            let (mut inside, mut outside) = (0, 0);
            for (pixel_index, (old, new)) in before
                .as_chunks::<4>()
                .0
                .iter()
                .zip(after.as_chunks::<4>().0)
                .enumerate()
            {
                let changed = old[..3]
                    .iter()
                    .zip(&new[..3])
                    .any(|(a, b)| a.abs_diff(*b) > 2);
                if !changed {
                    continue;
                }
                let (x, y) = (pixel_index % width, pixel_index / width);
                if (left..right).contains(&x) && (top..bottom).contains(&y) {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
            let area = (right - left) * (bottom - top);
            eprintln!(
                "{}: display {width}x{height}, rect px [{left},{top})-[{right},{bottom}), inside={inside}/{area} outside={outside}",
                case.name
            );
            assert_eq!(
                outside, 0,
                "{}: highlight dikdörtgen dışına düştü",
                case.name
            );
            assert!(
                inside * 10 >= area * 9,
                "{}: highlight dikdörtgenin yalnız {inside}/{area} pikselini boyadı",
                case.name
            );

            let annotations = read_annotations_on(&path, u32::from(case.page_index));
            let added = annotations.last().unwrap();
            assert_normalized_rect_eq(added.normalized_rect.as_ref().unwrap(), rect);
        }
    }

    #[test]
    fn every_annotation_type_round_trips_display_rect_on_rotated_offset_cropbox_page() {
        let directory = tempfile::tempdir().unwrap();
        let path = rotated_offset_cropbox_copy(directory.path());
        let rect = normalized(0.2, 0.1, 0.4, 0.06);

        for annot_type in ["Highlight", "Underline", "Text"] {
            add_annotation_report(
                path.to_string_lossy().to_string(),
                0,
                annot_type.to_string(),
                rect.clone(),
                "#0033CC".to_string(),
                Some(format!("{annot_type} rotated")),
                None,
            )
            .unwrap_or_else(|error| panic!("{annot_type}: {error}"));

            let added = read_annotations(&path).pop().unwrap();
            assert_eq!(added.annot_type, annot_type);
            assert_normalized_rect_eq(added.normalized_rect.as_ref().unwrap(), &rect);
            let bounds = annotation_fingerprints(&path).last().unwrap().bounds;
            for (read, written) in bounds.into_iter().zip([80.0, 200.0, 110.0, 460.0]) {
                assert!(
                    (read - written).abs() <= 0.01,
                    "{annot_type} user space bounds: okunan {bounds:?}"
                );
            }
        }
    }

    #[test]
    fn annotation_bounds_read_error_is_not_silently_defaulted() {
        let error =
            build_annotation_fingerprint::<&str>("Text".to_string(), Err("simulated"), None)
                .unwrap_err();

        assert_eq!(
            error.stage,
            crate::document_io::DocumentIoStage::ValidateInvariant
        );
        assert!(error.detail.contains("Annotation bounds okunamadı"));
    }
}
