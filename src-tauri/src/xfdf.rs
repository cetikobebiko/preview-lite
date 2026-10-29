//! XFDF (ISO 19444-1) ile açıklama ve form verisi alışverişi.
//!
//! Bu modülün çekirdeği saftır: [`parse`] XFDF metnini, [`write`] veriyi XFDF'e çevirir;
//! PDF açmaz. Belgeyle ilgili kısımlar job worker'da çalışır: [`export_snapshot`] PDFium ile
//! açıklamaları ve alanları okur, [`import_report`] alanları `forms::fill` ile doldurur ve
//! açıklamaları `annotations::import_annotations_report` ile ekler (ikisi de artımlı).
//!
//! Desteklenen açıklamalar uygulamanın ekleyebildikleridir: not (text), vurgu, altı çizili,
//! üstü çizili, dalgalı, kare, daire ve mürekkep. Diğerleri dışa aktarılmaz, içe aktarılırken
//! sayılıp atlanır. Sayfa numaraları XFDF'teki gibi 0 tabanlıdır; koordinatlar PDF user
//! space'tedir.

use crate::document_io::{AtomicWriteReport, CommitState, DocumentIoError, DocumentIoStage};
use crate::forms::fill::{FormFieldEdit, FormFieldValue};
use crate::forms::FormFieldKind;
use crate::incremental_save::{RawDocument, RawPage};
use pdfium_render::prelude::*;
use quick_xml::events::{BytesStart, Event};
use quick_xml::XmlVersion;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::raw::c_ulong;
use std::path::Path;

/// İçe aktarılan XFDF dosyasının sınırı.
pub const MAX_XFDF_BYTES: usize = 16 * 1024 * 1024;
/// En fazla açıklama ve en fazla alan.
const MAX_ITEMS: usize = 10_000;
/// Bir metnin (içerik, değer, ad) en fazla uzunluğu.
const MAX_TEXT_CHARS: usize = 64 * 1024;
/// Bir alanın en fazla değeri (çok seçimli liste).
const MAX_FIELD_VALUES: usize = 1_000;
/// Koordinatların mutlak sınırı (pt).
const MAX_COORDINATE: f32 = 1.0e6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotKind {
    Text,
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Square,
    Circle,
    Ink,
}

impl AnnotKind {
    const ALL: [AnnotKind; 8] = [
        Self::Text,
        Self::Highlight,
        Self::Underline,
        Self::StrikeOut,
        Self::Squiggly,
        Self::Square,
        Self::Circle,
        Self::Ink,
    ];

    pub fn element(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Highlight => "highlight",
            Self::Underline => "underline",
            Self::StrikeOut => "strikeout",
            Self::Squiggly => "squiggly",
            Self::Square => "square",
            Self::Circle => "circle",
            Self::Ink => "ink",
        }
    }

    fn from_element(name: &[u8]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.element().as_bytes() == name)
    }

    /// PDF `/Subtype` adı.
    pub fn subtype(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Highlight => "Highlight",
            Self::Underline => "Underline",
            Self::StrikeOut => "StrikeOut",
            Self::Squiggly => "Squiggly",
            Self::Square => "Square",
            Self::Circle => "Circle",
            Self::Ink => "Ink",
        }
    }

    /// PDFium `FPDF_ANNOTATION_SUBTYPE` değeri.
    pub fn pdfium_subtype(self) -> FPDF_ANNOTATION_SUBTYPE {
        match self {
            Self::Text => 1,
            Self::Square => 5,
            Self::Circle => 6,
            Self::Highlight => 9,
            Self::Underline => 10,
            Self::Squiggly => 11,
            Self::StrikeOut => 12,
            Self::Ink => 15,
        }
    }

    fn from_pdfium(subtype: FPDF_ANNOTATION_SUBTYPE) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.pdfium_subtype() == subtype)
    }

    pub fn is_text_markup(self) -> bool {
        matches!(
            self,
            Self::Highlight | Self::Underline | Self::StrikeOut | Self::Squiggly
        )
    }
}

/// Bir açıklama; geometri PDF user space'tedir.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XfdfAnnotation {
    pub kind: AnnotKind,
    pub page: u32,
    /// sol, alt, sağ, üst.
    pub rect: [f32; 4],
    pub color: Option<[u8; 3]>,
    pub contents: Option<String>,
    /// Yazar (`/T`).
    pub title: Option<String>,
    /// Benzersiz ad (`/NM`).
    pub name: Option<String>,
    /// Değiştirilme tarihi (`/M`, PDF tarih biçimi).
    pub date: Option<String>,
    /// Metin işaretlemesi: dörtgen başına 8 sayı (`/QuadPoints`).
    pub coords: Vec<f32>,
    /// Mürekkep çizgileri.
    pub ink: Vec<Vec<[f32; 2]>>,
    /// Kenarlık kalınlığı (pt).
    pub width: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfdfField {
    /// Tam nitelikli ad (`parent.child`).
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XfdfData {
    pub fields: Vec<XfdfField>,
    pub annotations: Vec<XfdfAnnotation>,
    /// İçe aktarmada tanınmayan ya da desteklenmeyen açıklamalar.
    pub skipped_annotations: usize,
}

/// İçe aktarma isteğinin özeti; komut, XFDF'i belgeye uygulamadan önce döndürür.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XfdfSummary {
    pub fields: usize,
    pub annotations: usize,
    pub skipped_annotations: usize,
}

impl XfdfData {
    pub fn summary(&self) -> XfdfSummary {
        XfdfSummary {
            fields: self.fields.len(),
            annotations: self.annotations.len(),
            skipped_annotations: self.skipped_annotations,
        }
    }
}

// ---------------------------------------------------------------------------
// Yazma
// ---------------------------------------------------------------------------

fn escape(text: &str) -> String {
    quick_xml::escape::escape(text).into_owned()
}

fn number(value: f32) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    let text = format!("{rounded:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

fn numbers(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| number(*value))
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Default)]
struct FieldNode {
    name: String,
    values: Option<Vec<String>>,
    children: Vec<FieldNode>,
}

impl FieldNode {
    fn insert(&mut self, path: &[&str], values: &[String]) {
        let Some((first, rest)) = path.split_first() else {
            self.values = Some(values.to_vec());
            return;
        };
        let index = match self.children.iter().position(|child| child.name == *first) {
            Some(index) => index,
            None => {
                self.children.push(FieldNode {
                    name: (*first).to_string(),
                    ..FieldNode::default()
                });
                self.children.len() - 1
            }
        };
        self.children[index].insert(rest, values);
    }

    fn write(&self, out: &mut String) {
        out.push_str(&format!("<field name=\"{}\">", escape(&self.name)));
        for value in self.values.iter().flatten() {
            out.push_str(&format!("<value>{}</value>", escape(value)));
        }
        for child in &self.children {
            child.write(out);
        }
        out.push_str("</field>\n");
    }
}

/// `data`'yı XFDF metnine çevirir; `file_name` `<f href>` olarak yazılır.
pub fn write(data: &XfdfData, file_name: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n",
    );
    out.push_str(&format!("<f href=\"{}\"/>\n", escape(file_name)));
    if !data.fields.is_empty() {
        let mut root = FieldNode::default();
        for field in &data.fields {
            let path: Vec<&str> = field.name.split('.').collect();
            root.insert(&path, &field.values);
        }
        out.push_str("<fields>\n");
        for child in &root.children {
            child.write(&mut out);
        }
        out.push_str("</fields>\n");
    }
    if !data.annotations.is_empty() {
        out.push_str("<annots>\n");
        for annotation in &data.annotations {
            write_annotation(annotation, &mut out);
        }
        out.push_str("</annots>\n");
    }
    out.push_str("</xfdf>\n");
    out
}

fn write_annotation(annotation: &XfdfAnnotation, out: &mut String) {
    let element = annotation.kind.element();
    out.push_str(&format!(
        "<{element} page=\"{}\" rect=\"{}\"",
        annotation.page,
        numbers(&annotation.rect)
    ));
    if let Some([r, g, b]) = annotation.color {
        out.push_str(&format!(" color=\"#{r:02X}{g:02X}{b:02X}\""));
    }
    for (key, value) in [
        ("name", &annotation.name),
        ("title", &annotation.title),
        ("date", &annotation.date),
    ] {
        if let Some(value) = value {
            out.push_str(&format!(" {key}=\"{}\"", escape(value)));
        }
    }
    if let Some(width) = annotation.width {
        out.push_str(&format!(" width=\"{}\"", number(width)));
    }
    if !annotation.coords.is_empty() {
        out.push_str(&format!(" coords=\"{}\"", numbers(&annotation.coords)));
    }
    out.push('>');
    if let Some(contents) = &annotation.contents {
        out.push_str(&format!("<contents>{}</contents>", escape(contents)));
    }
    if !annotation.ink.is_empty() {
        out.push_str("<inklist>");
        for stroke in &annotation.ink {
            let points: Vec<String> = stroke
                .iter()
                .map(|[x, y]| format!("{},{}", number(*x), number(*y)))
                .collect();
            out.push_str(&format!("<gesture>{}</gesture>", points.join(";")));
        }
        out.push_str("</inklist>");
    }
    out.push_str(&format!("</{element}>\n"));
}

// ---------------------------------------------------------------------------
// Okuma
// ---------------------------------------------------------------------------

fn parse_numbers(text: &str, what: &str) -> Result<Vec<f32>, String> {
    text.split([',', ' ', ';', '\n', '\r', '\t'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<f32>()
                .ok()
                .filter(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
                .ok_or_else(|| format!("XFDF {what} sayısı geçersiz: {part}"))
        })
        .collect()
}

fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn bounded(text: String, what: &str) -> Result<String, String> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("XFDF {what} çok uzun"));
    }
    Ok(text)
}

fn annotation_from(kind: AnnotKind, start: &BytesStart<'_>) -> Result<XfdfAnnotation, String> {
    let mut annotation = XfdfAnnotation {
        kind,
        page: 0,
        rect: [0.0; 4],
        color: None,
        contents: None,
        title: None,
        name: None,
        date: None,
        coords: Vec::new(),
        ink: Vec::new(),
        width: None,
    };
    let mut has_page = false;
    let mut has_rect = false;
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| format!("XFDF özniteliği okunamadı: {error}"))?;
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|error| format!("XFDF özniteliği çözülemedi: {error}"))?
            .into_owned();
        match attribute.key.local_name().as_ref() {
            b"page" => {
                annotation.page = value
                    .trim()
                    .parse()
                    .map_err(|_| format!("XFDF sayfa numarası geçersiz: {value}"))?;
                has_page = true;
            }
            b"rect" => {
                let rect = parse_numbers(&value, "rect")?;
                let [a, b, c, d] = rect[..] else {
                    return Err("XFDF rect dört sayı olmalı".into());
                };
                annotation.rect = [a.min(c), b.min(d), a.max(c), b.max(d)];
                has_rect = true;
            }
            b"color" => annotation.color = parse_color(&value),
            b"title" => annotation.title = Some(bounded(value, "yazar adı")?),
            b"name" => annotation.name = Some(bounded(value, "açıklama adı")?),
            b"date" => annotation.date = Some(bounded(value, "tarih")?),
            b"width" => {
                annotation.width = value
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|width| width.is_finite() && (0.0..=100.0).contains(width));
            }
            b"coords" => {
                let coords = parse_numbers(&value, "coords")?;
                if coords.len() % 8 != 0 || coords.len() > 8 * 4096 {
                    return Err("XFDF coords dörtgen başına 8 sayı olmalı".into());
                }
                annotation.coords = coords;
            }
            _ => {}
        }
    }
    if !has_page || !has_rect {
        return Err(format!("XFDF <{}> page ve rect taşımalı", kind.element()));
    }
    Ok(annotation)
}

fn parse_gesture(text: &str) -> Result<Vec<[f32; 2]>, String> {
    let values = parse_numbers(text, "gesture")?;
    if values.len() % 2 != 0 || values.len() / 2 > crate::annotations::MAX_INK_STROKE_POINTS {
        return Err("XFDF gesture nokta çiftlerinden oluşmalı".into());
    }
    Ok(values.chunks(2).map(|pair| [pair[0], pair[1]]).collect())
}

/// Ayrıştırılan XFDF'in neresinde olunduğu.
enum Context {
    Fields,
    /// `<field>` iç içeliği; `<value>` içinde metin toplanır.
    Field {
        in_value: bool,
    },
    Annots,
    Annotation,
    Contents,
    Gesture,
    /// Atlanan bir alt ağaç (desteklenmeyen açıklama, zengin metin…).
    Skip,
    Other,
}

/// XFDF metnini ayrıştırır.
pub fn parse(text: &str) -> Result<XfdfData, String> {
    if text.len() > MAX_XFDF_BYTES {
        return Err("XFDF dosyası 16 MiB sınırını aşıyor".into());
    }
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut data = XfdfData::default();
    let mut stack: Vec<Context> = Vec::new();
    let mut field_path: Vec<String> = Vec::new();
    // Tam nitelikli ada göre toplanan değerler, belgedeki sırayla.
    let mut field_values: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<XfdfAnnotation> = None;
    let mut buffer = String::new();
    let mut seen_root = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("XFDF ayrıştırılamadı: {error}"))?;
        let is_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                let name = start.local_name();
                let name = name.as_ref();
                let parent = stack.last();
                let context = match (parent, name) {
                    (None, b"xfdf") => {
                        seen_root = true;
                        Context::Other
                    }
                    (None, _) => return Err("XFDF kök öğesi <xfdf> olmalı".into()),
                    (Some(Context::Other), b"fields") if stack.len() == 1 => Context::Fields,
                    (Some(Context::Other), b"annots") if stack.len() == 1 => Context::Annots,
                    (Some(Context::Fields | Context::Field { .. }), b"field") => {
                        let field_name = start
                            .attributes()
                            .filter_map(Result::ok)
                            .find(|attribute| attribute.key.local_name().as_ref() == b"name")
                            .and_then(|attribute| {
                                attribute.normalized_value(XmlVersion::Implicit1_0).ok()
                            })
                            .map(|value| value.into_owned())
                            .ok_or("XFDF <field> name taşımalı")?;
                        field_path.push(bounded(field_name, "alan adı")?);
                        Context::Field { in_value: false }
                    }
                    (Some(Context::Field { .. }), b"value") => {
                        buffer.clear();
                        Context::Field { in_value: true }
                    }
                    (Some(Context::Annots), _) => match AnnotKind::from_element(name) {
                        Some(kind) => {
                            if data.annotations.len() >= MAX_ITEMS {
                                return Err("XFDF'te çok fazla açıklama var".into());
                            }
                            current = Some(annotation_from(kind, &start)?);
                            Context::Annotation
                        }
                        None => {
                            data.skipped_annotations += 1;
                            Context::Skip
                        }
                    },
                    (Some(Context::Annotation), b"contents") => {
                        buffer.clear();
                        Context::Contents
                    }
                    (Some(Context::Annotation), b"inklist") => Context::Other,
                    (Some(Context::Other), b"gesture") if current.is_some() => {
                        buffer.clear();
                        Context::Gesture
                    }
                    _ => Context::Skip,
                };
                if is_empty {
                    close(
                        context,
                        &mut data,
                        &mut field_path,
                        &mut field_values,
                        &mut current,
                        &mut buffer,
                    )?;
                } else {
                    stack.push(context);
                }
            }
            Event::End(_) => {
                let context = stack.pop().ok_or("XFDF öğeleri dengesiz")?;
                close(
                    context,
                    &mut data,
                    &mut field_path,
                    &mut field_values,
                    &mut current,
                    &mut buffer,
                )?;
            }
            Event::Text(text) => {
                if matches!(
                    stack.last(),
                    Some(Context::Field { in_value: true } | Context::Contents | Context::Gesture)
                ) {
                    let decoded = text
                        .xml_content(XmlVersion::Implicit1_0)
                        .map_err(|error| format!("XFDF metni çözülemedi: {error}"))?;
                    buffer.push_str(&decoded);
                }
            }
            Event::GeneralRef(reference) => {
                if matches!(
                    stack.last(),
                    Some(Context::Field { in_value: true } | Context::Contents | Context::Gesture)
                ) {
                    let name = reference
                        .decode()
                        .map_err(|error| format!("XFDF metni çözülemedi: {error}"))?;
                    let entity = format!("&{name};");
                    let resolved = quick_xml::escape::unescape(&entity)
                        .map_err(|error| format!("XFDF metni çözülemedi: {error}"))?;
                    buffer.push_str(&resolved);
                }
            }
            Event::CData(cdata) => {
                if matches!(
                    stack.last(),
                    Some(Context::Field { in_value: true } | Context::Contents | Context::Gesture)
                ) {
                    buffer.push_str(&String::from_utf8_lossy(&cdata));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        if buffer.len() > MAX_TEXT_CHARS * 4 {
            return Err("XFDF metni çok uzun".into());
        }
    }
    if !seen_root {
        return Err("XFDF kök öğesi <xfdf> bulunamadı".into());
    }
    data.fields = field_values
        .into_iter()
        .map(|(name, values)| XfdfField { name, values })
        .collect();
    Ok(data)
}

fn close(
    context: Context,
    data: &mut XfdfData,
    field_path: &mut Vec<String>,
    field_values: &mut Vec<(String, Vec<String>)>,
    current: &mut Option<XfdfAnnotation>,
    buffer: &mut String,
) -> Result<(), String> {
    match context {
        Context::Field { in_value: true } => {
            let name = field_path.join(".");
            let value = bounded(std::mem::take(buffer), "alan değeri")?;
            match field_values
                .iter_mut()
                .find(|(existing, _)| *existing == name)
            {
                Some((_, values)) if values.len() < MAX_FIELD_VALUES => values.push(value),
                Some(_) => return Err(format!("XFDF alanı {name} için çok fazla değer")),
                None => {
                    if field_values.len() >= MAX_ITEMS {
                        return Err("XFDF'te çok fazla alan var".into());
                    }
                    field_values.push((name, vec![value]));
                }
            }
        }
        Context::Field { in_value: false } => {
            field_path.pop();
        }
        Context::Contents => {
            if let Some(annotation) = current.as_mut() {
                annotation.contents = Some(bounded(std::mem::take(buffer), "açıklama metni")?);
            }
        }
        Context::Gesture => {
            if let Some(annotation) = current.as_mut() {
                if annotation.ink.len() >= crate::annotations::MAX_INK_STROKES {
                    return Err("XFDF mürekkep açıklamasında çok fazla çizgi var".into());
                }
                let stroke = parse_gesture(buffer)?;
                if !stroke.is_empty() {
                    annotation.ink.push(stroke);
                }
            }
            buffer.clear();
        }
        Context::Annotation => {
            if let Some(annotation) = current.take() {
                // Çizgisiz mürekkep çizilemez.
                if annotation.kind == AnnotKind::Ink && annotation.ink.is_empty() {
                    data.skipped_annotations += 1;
                } else {
                    data.annotations.push(annotation);
                }
            }
        }
        Context::Fields | Context::Annots | Context::Skip | Context::Other => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Belgeden okuma (dışa aktarma)
// ---------------------------------------------------------------------------

const COLORTYPE_COLOR: FPDFANNOT_COLORTYPE = 0;
const COLORTYPE_INTERIOR_COLOR: FPDFANNOT_COLORTYPE = 1;

fn read_string(
    bindings: &dyn PdfiumLibraryBindings,
    annotation: FPDF_ANNOTATION,
    key: &str,
) -> Option<String> {
    let length = bindings.FPDFAnnot_GetStringValue(annotation, key, std::ptr::null_mut(), 0);
    // UTF-16LE, sonlandırıcı dahil; yalnız sonlandırıcı boş metindir.
    if length <= 2 || length > (MAX_TEXT_CHARS as c_ulong) * 4 {
        return None;
    }
    let mut buffer = vec![0u16; (length / 2) as usize];
    let written = bindings.FPDFAnnot_GetStringValue(annotation, key, buffer.as_mut_ptr(), length);
    if written != length {
        return None;
    }
    buffer.pop();
    Some(String::from_utf16_lossy(&buffer)).filter(|text| !text.is_empty())
}

/// `/C` (yoksa `/IC`). PDFium görünüm akışı olan açıklamada rengi vermez; o zaman görünümün
/// ilk yolunun rengi alınır (`annotations::annotation_display_color` gibi).
fn read_color(
    bindings: &dyn PdfiumLibraryBindings,
    annotation: FPDF_ANNOTATION,
    appearance: Option<&PdfPageAnnotation<'_>>,
) -> Option<[u8; 3]> {
    for color_type in [COLORTYPE_COLOR, COLORTYPE_INTERIOR_COLOR] {
        let (mut r, mut g, mut b, mut a) = (0, 0, 0, 0);
        if bindings.is_true(
            bindings.FPDFAnnot_GetColor(annotation, color_type, &mut r, &mut g, &mut b, &mut a),
        ) {
            return Some([r as u8, g as u8, b as u8]);
        }
    }
    let path = appearance?
        .objects()
        .iter()
        .next()?
        .as_path_object()
        .map(|path| match path.fill_mode() {
            Ok(PdfPathFillMode::None) => path.stroke_color().ok(),
            Ok(_) => path.fill_color().ok(),
            Err(_) => None,
        })??;
    Some([path.red(), path.green(), path.blue()])
}

fn read_annotation(
    bindings: &dyn PdfiumLibraryBindings,
    annotation: FPDF_ANNOTATION,
    appearance: Option<&PdfPageAnnotation<'_>>,
    kind: AnnotKind,
    page: u32,
) -> Option<XfdfAnnotation> {
    let mut rect = FS_RECTF {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    if !bindings.is_true(bindings.FPDFAnnot_GetRect(annotation, &mut rect)) {
        return None;
    }
    let mut coords = Vec::new();
    if kind.is_text_markup() {
        let count = bindings.FPDFAnnot_CountAttachmentPoints(annotation);
        for index in 0..count.min(4096) {
            let mut quad = FS_QUADPOINTSF {
                x1: 0.0,
                y1: 0.0,
                x2: 0.0,
                y2: 0.0,
                x3: 0.0,
                y3: 0.0,
                x4: 0.0,
                y4: 0.0,
            };
            if bindings
                .is_true(bindings.FPDFAnnot_GetAttachmentPoints(annotation, index, &mut quad))
            {
                coords.extend([
                    quad.x1, quad.y1, quad.x2, quad.y2, quad.x3, quad.y3, quad.x4, quad.y4,
                ]);
            }
        }
    }
    let mut ink = Vec::new();
    if kind == AnnotKind::Ink {
        let strokes = bindings
            .FPDFAnnot_GetInkListCount(annotation)
            .min(crate::annotations::MAX_INK_STROKES as c_ulong);
        for stroke in 0..strokes {
            let points =
                bindings.FPDFAnnot_GetInkListPath(annotation, stroke, std::ptr::null_mut(), 0);
            let points = points.min(crate::annotations::MAX_INK_STROKE_POINTS as c_ulong);
            let mut buffer = vec![FS_POINTF { x: 0.0, y: 0.0 }; points as usize];
            let read =
                bindings.FPDFAnnot_GetInkListPath(annotation, stroke, buffer.as_mut_ptr(), points);
            buffer.truncate(read.min(points) as usize);
            if !buffer.is_empty() {
                ink.push(buffer.iter().map(|point| [point.x, point.y]).collect());
            }
        }
        if ink.is_empty() {
            return None;
        }
    }
    let (mut horizontal, mut vertical, mut width) = (0.0, 0.0, 0.0);
    let width = (bindings.is_true(bindings.FPDFAnnot_GetBorder(
        annotation,
        &mut horizontal,
        &mut vertical,
        &mut width,
    )) && matches!(kind, AnnotKind::Square | AnnotKind::Circle | AnnotKind::Ink))
    .then_some(width);
    Some(XfdfAnnotation {
        kind,
        page,
        rect: [
            rect.left.min(rect.right),
            rect.bottom.min(rect.top),
            rect.left.max(rect.right),
            rect.bottom.max(rect.top),
        ],
        color: read_color(bindings, annotation, appearance),
        contents: read_string(bindings, annotation, "Contents"),
        title: read_string(bindings, annotation, "T"),
        name: read_string(bindings, annotation, "NM"),
        date: read_string(bindings, annotation, "M"),
        coords,
        ink,
        width,
    })
}

/// Belgedeki desteklenen açıklamalar, sayfa sırasıyla.
fn read_annotations(
    bindings: &dyn PdfiumLibraryBindings,
    reader: &PdfDocument<'_>,
    path: &Path,
    password: Option<&str>,
) -> Result<Vec<XfdfAnnotation>, String> {
    let path_str = path.to_str().ok_or("PDF yolu UTF-8 değil")?;
    let document = RawDocument::open(bindings, path_str, password).ok_or_else(|| {
        format!(
            "PDF açılamadı (PDFium hata kodu {})",
            bindings.FPDF_GetLastError()
        )
    })?;
    let page_count = document.page_count().map_err(|failure| failure.detail)?;
    let mut annotations = Vec::new();
    for page_index in 0..u32::from(page_count) {
        let page = RawPage::load(&document, page_index).map_err(|failure| failure.detail)?;
        let reader_page = reader.pages().get(page_index as u16).ok();
        let reader_annotations = reader_page.as_ref().map(|page| page.annotations());
        let count = bindings.FPDFPage_GetAnnotCount(page.handle);
        for index in 0..count {
            let handle = bindings.FPDFPage_GetAnnot(page.handle, index);
            if handle.is_null() {
                continue;
            }
            let kind = AnnotKind::from_pdfium(bindings.FPDFAnnot_GetSubtype(handle));
            let appearance = reader_annotations
                .as_ref()
                .and_then(|annotations| annotations.get(index as PdfPageAnnotationIndex).ok());
            let annotation = kind.and_then(|kind| {
                read_annotation(bindings, handle, appearance.as_ref(), kind, page_index)
            });
            bindings.FPDFPage_CloseAnnot(handle);
            if let Some(annotation) = annotation {
                if annotations.len() >= MAX_ITEMS {
                    return Err("Belgede dışa aktarılamayacak kadar çok açıklama var".into());
                }
                annotations.push(annotation);
            }
        }
    }
    Ok(annotations)
}

/// Belgedeki form alanlarının değerleri, ilk görüldükleri sırayla.
fn read_fields(document: &PdfDocument<'_>) -> Result<Vec<XfdfField>, String> {
    let mut fields: Vec<XfdfField> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    for page_index in 0..u32::from(document.pages().len()) {
        let widgets = crate::forms::read_page_form_fields(document, page_index)
            .map_err(|error| format!("Form alanları okunamadı: {error:?}"))?;
        for widget in widgets {
            let values = match widget.kind {
                FormFieldKind::Text | FormFieldKind::ComboBox => {
                    widget.value.clone().into_iter().collect()
                }
                FormFieldKind::ListBox => widget
                    .options
                    .iter()
                    .filter(|option| option.selected)
                    .map(|option| option.label.clone())
                    .collect(),
                FormFieldKind::Checkbox | FormFieldKind::RadioButton => {
                    vec![widget
                        .value
                        .clone()
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "Off".into())]
                }
                FormFieldKind::PushButton | FormFieldKind::Signature | FormFieldKind::Unknown => {
                    continue
                }
            };
            if index_of.contains_key(&widget.field_name) {
                continue;
            }
            if fields.len() >= MAX_ITEMS {
                return Err("Belgede dışa aktarılamayacak kadar çok alan var".into());
            }
            index_of.insert(widget.field_name.clone(), fields.len());
            fields.push(XfdfField {
                name: widget.field_name,
                values,
            });
        }
    }
    Ok(fields)
}

/// Snapshot'ın açıklamalarını ve/veya alan değerlerini XFDF olarak döndürür.
pub(crate) fn export_snapshot(
    snapshot: &Path,
    credential: Option<&str>,
    include_comments: bool,
    include_fields: bool,
    file_name: &str,
) -> Result<String, String> {
    if !include_comments && !include_fields {
        return Err("Açıklamaları ya da form alanlarını seçin".into());
    }
    let pdfium = crate::pdf_engine::get_pdfium()?;
    let mut data = XfdfData::default();
    let document =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot.to_string_lossy(), credential)?;
    if include_fields {
        data.fields = read_fields(&document)?;
    }
    if include_comments {
        data.annotations = read_annotations(pdfium.bindings(), &document, snapshot, credential)?;
    }
    if data.fields.is_empty() && data.annotations.is_empty() {
        return Err("Belgede dışa aktarılacak açıklama ya da form alanı yok".into());
    }
    Ok(write(&data, file_name))
}

// ---------------------------------------------------------------------------
// Belgeye uygulama (içe aktarma)
// ---------------------------------------------------------------------------

/// XFDF alan değerlerini belgedeki widget'lara düzenleme olarak eşler. Belgede olmayan,
/// salt okunur ya da değeri uymayan alanlar atlanır.
fn field_edits(
    document: &PdfDocument<'_>,
    fields: &[XfdfField],
    radio_exports: &HashMap<(u32, u32), String>,
) -> Result<Vec<FormFieldEdit>, String> {
    let wanted: HashMap<&str, &[String]> = fields
        .iter()
        .map(|field| (field.name.as_str(), field.values.as_slice()))
        .collect();
    let mut edits = Vec::new();
    let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();
    for page_index in 0..u32::from(document.pages().len()) {
        let widgets = crate::forms::read_page_form_fields(document, page_index)
            .map_err(|error| format!("Form alanları okunamadı: {error:?}"))?;
        for widget in widgets {
            let Some(values) = wanted.get(widget.field_name.as_str()) else {
                continue;
            };
            if widget.read_only || done.contains(&widget.field_name) {
                continue;
            }
            let first = values.first().map(String::as_str).unwrap_or("");
            let value = match widget.kind {
                FormFieldKind::Text => Some(FormFieldValue::Text {
                    text: first.to_string(),
                }),
                FormFieldKind::Checkbox => Some(FormFieldValue::Checkbox {
                    checked: !first.is_empty() && first != "Off",
                }),
                FormFieldKind::RadioButton => radio_exports
                    .get(&(page_index, widget.annot_index))
                    .filter(|export| export.as_str() == first)
                    .map(|_| FormFieldValue::Radio),
                FormFieldKind::ComboBox => widget
                    .options
                    .iter()
                    .position(|option| option.label == first)
                    .map(|index| FormFieldValue::Choice {
                        selected: vec![index as u32],
                    }),
                FormFieldKind::ListBox => {
                    let selected: Vec<u32> = widget
                        .options
                        .iter()
                        .enumerate()
                        .filter(|(_, option)| values.contains(&option.label))
                        .map(|(index, _)| index as u32)
                        .collect();
                    (!selected.is_empty() || values.is_empty())
                        .then_some(FormFieldValue::Choice { selected })
                }
                FormFieldKind::PushButton | FormFieldKind::Signature | FormFieldKind::Unknown => {
                    None
                }
            };
            // Bir radyo grubunda değeri tutan düğme bulunana kadar diğer düğmelere bakılır.
            if let Some(value) = value {
                done.insert(widget.field_name.clone());
                edits.push(FormFieldEdit {
                    page_index,
                    annot_index: widget.annot_index,
                    field_name: widget.field_name,
                    value,
                });
            }
        }
    }
    Ok(edits)
}

fn import_failure(stage: DocumentIoStage, detail: impl Into<String>) -> DocumentIoError {
    DocumentIoError::new("import_xfdf", stage, CommitState::Prepared, detail.into())
}

/// `xfdf`'i `file_path`'e uygular: önce alanlar doldurulur, sonra açıklamalar eklenir. İkisi de
/// artımlıdır. Belgede uygulanacak hiçbir şey yoksa yazmadan hata döner.
pub(crate) fn import_report(
    file_path: String,
    xfdf: String,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let data = parse(&xfdf).map_err(|detail| import_failure(DocumentIoStage::Preflight, detail))?;
    let path = std::path::PathBuf::from(&file_path);
    let (edits, annotations) = crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
        let document = crate::pdf_engine::open_pdfium_doc(pdfium, &file_path, password.as_deref())?;
        let page_count = u32::from(document.pages().len());
        let edits = if data.fields.is_empty() {
            Vec::new()
        } else {
            let radio_exports = crate::forms::fill::widget_export_values(
                pdfium.bindings(),
                &path,
                password.as_deref(),
            )
            .map_err(|failure| failure.detail)?;
            field_edits(&document, &data.fields, &radio_exports)?
        };
        let annotations: Vec<XfdfAnnotation> = data
            .annotations
            .iter()
            .filter(|annotation| annotation.page < page_count)
            .cloned()
            .collect();
        Ok((edits, annotations))
    })
    .map_err(|detail| import_failure(DocumentIoStage::Preflight, detail))?;
    if edits.is_empty() && annotations.is_empty() {
        return Err(import_failure(
            DocumentIoStage::Preflight,
            "XFDF'te bu belgeye uygulanabilecek açıklama ya da form alanı yok",
        ));
    }
    let mut report = None;
    if !edits.is_empty() {
        report = Some(crate::forms::fill::fill_form_fields_report(
            file_path.clone(),
            edits,
            password.clone(),
        )?);
    }
    if !annotations.is_empty() {
        report = Some(crate::annotations::import_annotations_report(
            file_path,
            annotations,
            password,
        )?);
    }
    Ok(report.expect("at least one write ran"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> XfdfData {
        XfdfData {
            fields: vec![
                XfdfField {
                    name: "person.name".into(),
                    values: vec!["Ada & <Lovelace>".into()],
                },
                XfdfField {
                    name: "person.age".into(),
                    values: vec!["36".into()],
                },
                XfdfField {
                    name: "langs".into(),
                    values: vec!["English".into(), "Japanese".into()],
                },
            ],
            annotations: vec![
                XfdfAnnotation {
                    kind: AnnotKind::Highlight,
                    page: 0,
                    rect: [10.0, 20.0, 110.0, 40.0],
                    color: Some([255, 204, 0]),
                    contents: Some("Önemli: \"çğış\"".into()),
                    title: Some("Umut".into()),
                    name: Some("a1".into()),
                    date: Some("D:20261029120000Z".into()),
                    coords: vec![10.0, 40.0, 110.0, 40.0, 10.0, 20.0, 110.0, 20.0],
                    ink: vec![],
                    width: None,
                },
                XfdfAnnotation {
                    kind: AnnotKind::Ink,
                    page: 1,
                    rect: [0.0, 0.0, 50.0, 50.0],
                    color: Some([0, 0, 255]),
                    contents: None,
                    title: None,
                    name: None,
                    date: None,
                    coords: vec![],
                    ink: vec![
                        vec![[1.0, 2.0], [3.5, 4.25]],
                        vec![[10.0, 10.0], [20.0, 20.0]],
                    ],
                    width: Some(2.0),
                },
            ],
            skipped_annotations: 0,
        }
    }

    #[test]
    fn write_then_parse_round_trips() {
        let data = sample();
        let text = write(&data, "rapor.pdf");
        assert!(
            text.contains("<field name=\"person\"><field name=\"name\">"),
            "{text}"
        );
        assert!(text.contains("Ada &amp; &lt;Lovelace&gt;"));
        assert_eq!(parse(&text).unwrap(), data);
    }

    #[test]
    fn unsupported_annotations_are_counted_and_skipped() {
        let text = r##"<?xml version="1.0"?>
<xfdf xmlns="http://ns.adobe.com/xfdf/">
  <annots>
    <freetext page="0" rect="0,0,10,10"><contents-richtext><body>x</body></contents-richtext></freetext>
    <square page="2" rect="10,10,0,0" color="#ff0000" width="3"/>
    <ink page="0" rect="0,0,1,1"><inklist></inklist></ink>
  </annots>
</xfdf>"##;
        let data = parse(text).unwrap();
        assert_eq!(data.skipped_annotations, 2);
        assert_eq!(data.annotations.len(), 1);
        let square = &data.annotations[0];
        assert_eq!(square.rect, [0.0, 0.0, 10.0, 10.0]);
        assert_eq!(square.color, Some([255, 0, 0]));
        assert_eq!(square.width, Some(3.0));
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(parse("<fdf/>").is_err());
        assert!(
            parse("<xfdf><annots><text rect=\"0,0,1,1\"/></annots></xfdf>")
                .unwrap_err()
                .contains("page")
        );
        assert!(
            parse("<xfdf><annots><text page=\"0\" rect=\"0,0,NaN,1\"/></annots></xfdf>").is_err()
        );
        assert!(parse("<xfdf><annots><highlight page=\"0\" rect=\"0,0,1,1\" coords=\"1,2,3\"/></annots></xfdf>").is_err());
        assert!(parse("<xfdf><fields><field><value>x</value></field></fields></xfdf>").is_err());
        let huge = "a".repeat(MAX_XFDF_BYTES + 1);
        assert!(parse(&huge).unwrap_err().contains("16 MiB"));
    }

    #[test]
    fn entity_and_cdata_text_is_decoded() {
        let text = "<xfdf><fields><field name=\"a\"><value>x &amp; <![CDATA[<y>]]></value></field></fields></xfdf>";
        assert_eq!(
            parse(text).unwrap().fields[0].values,
            vec!["x & <y>".to_string()]
        );
    }

    fn standard_form(directory: &Path) -> std::path::PathBuf {
        let path = directory.join("form.pdf");
        crate::forms::test_support::standard_fixture(&path);
        path
    }

    #[test]
    fn exports_form_values_and_imports_them_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = standard_form(directory.path());
        let exported = export_snapshot(&path, None, false, true, "form.pdf").unwrap();
        let data = parse(&exported).unwrap();
        let value = |name: &str| {
            data.fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| field.values.clone())
        };
        assert_eq!(value("name"), Some(vec!["Ada".to_string()]));
        assert_eq!(value("color"), Some(vec!["Blue".to_string()]));
        assert_eq!(
            value("langs"),
            Some(vec!["English".to_string(), "Japanese".to_string()])
        );
        assert_eq!(value("newsletter"), Some(vec!["Off".to_string()]));

        let changed = r#"<xfdf><fields>
            <field name="name"><value>Grace</value></field>
            <field name="color"><value>Red</value></field>
            <field name="newsletter"><value>On</value></field>
            <field name="country"><value>TR</value></field>
            <field name="locked"><value>changed</value></field>
            <field name="missing"><value>ignored</value></field>
        </fields></xfdf>"#;
        import_report(
            path.to_string_lossy().into_owned(),
            changed.to_string(),
            None,
        )
        .unwrap();
        let data = parse(&export_snapshot(&path, None, false, true, "form.pdf").unwrap()).unwrap();
        let value = |name: &str| {
            data.fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| field.values.clone())
        };
        assert_eq!(value("name"), Some(vec!["Grace".to_string()]));
        assert_eq!(value("color"), Some(vec!["Red".to_string()]));
        assert_eq!(value("newsletter"), Some(vec!["On".to_string()]));
        assert_eq!(value("country"), Some(vec!["TR".to_string()]));
        // Read-only fields keep their value.
        assert_eq!(value("locked"), Some(vec!["fixed".to_string()]));
    }

    #[test]
    fn imports_annotations_and_exports_them_with_their_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doc.pdf");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/pdf/three-distinct-pages.pdf"),
            &path,
        )
        .unwrap();
        let mut data = sample();
        data.fields.clear();
        // Page 7 does not exist; it is dropped.
        let mut stray = data.annotations[0].clone();
        stray.page = 7;
        data.annotations.push(stray);
        let before = std::fs::read(&path).unwrap();
        import_report(
            path.to_string_lossy().into_owned(),
            write(&data, "doc.pdf"),
            None,
        )
        .unwrap();
        assert!(
            std::fs::read(&path).unwrap().starts_with(&before),
            "appended incrementally"
        );

        let exported =
            parse(&export_snapshot(&path, None, true, false, "doc.pdf").unwrap()).unwrap();
        assert_eq!(exported.annotations.len(), 2);
        let highlight = &exported.annotations[0];
        assert_eq!(highlight.kind, AnnotKind::Highlight);
        assert_eq!(highlight.contents.as_deref(), Some("Önemli: \"çğış\""));
        assert_eq!(highlight.title.as_deref(), Some("Umut"));
        assert_eq!(highlight.name.as_deref(), Some("a1"));
        assert_eq!(highlight.color, Some([255, 204, 0]));
        assert_eq!(highlight.coords, data.annotations[0].coords);
        let ink = &exported.annotations[1];
        assert_eq!((ink.kind, ink.page), (AnnotKind::Ink, 1));
        assert_eq!(ink.ink, data.annotations[1].ink);
    }

    #[test]
    fn import_with_nothing_applicable_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doc.pdf");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/minimal.pdf"),
            &path,
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let text = "<xfdf><fields><field name=\"nope\"><value>x</value></field></fields></xfdf>";
        let error =
            import_report(path.to_string_lossy().into_owned(), text.to_string(), None).unwrap_err();
        assert!(error.to_string().contains("uygulanabilecek"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
