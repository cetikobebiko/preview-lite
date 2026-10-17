//! Form alanı oluşturma ve değiştirme (docs/engineering/form-authoring-plan.md, Faz A).
//!
//! PDFium'un public API'si widget oluşturamaz ve `/AcroForm /Fields`'i değiştiremez; bu yüzden
//! yapı `lopdf` ile yazılır. Kullanım AGENTS.md'nin sınırları içindedir: yalnız job worker'da,
//! session snapshot'ının kopyası üzerinde, `atomic_write_pdf` içinde çalışır; şifreli belgeler
//! reddedilir. Yazım artımlıdır (`lopdf::IncrementalDocument`): orijinal baytlar aynen kalır,
//! değişen ve yeni nesneler sona eklenir, böylece imzalı baytlar korunur.
//!
//! PDFium otoritesini korur: normalize görüntü uzayı ↔ PDF user space dönüşümü PDFium'un
//! `PageDisplayTransform`'u ile yapılır, `annot_index` PDFium'un annotation sırasıdır ve kayıttan
//! sonra çıktı PDFium ile yeniden açılıp Faz 1 okuyucusuyla doğrulanır. Değer ve değer görünümü
//! bu modülün işi değildir; onlar `forms::fill`'in PDFium FORM yoludur. Burada üretilen görünüm
//! akışları yalnız sabit kenar/zemin ve checkbox/radio işaretleridir; kullanıcı metni görünüm
//! akışına yazılmaz.

use super::{FormFieldKind, FormWidgetData, MAX_FORM_OPTIONS_PER_FIELD};
use crate::annotations::{validate_normalized_rect, NormalizedPageRect, PageDisplayTransform};
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
    DocumentIoStage,
};
use lopdf::{dictionary, Dictionary, IncrementalDocument, Object, ObjectId, Stream, StringFormat};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Tek istekte en fazla işlem sayısı.
pub const MAX_AUTHOR_OPS: usize = 50;
/// Alan adı (kısmi ad) ve radio export değeri için karakter sınırı.
pub const MAX_FIELD_NAME_CHARS: usize = 255;
/// İpucu ve seçenek etiketi için karakter sınırı.
pub const MAX_FIELD_LABEL_CHARS: usize = 1024;
/// `lopdf` tüm dosyayı belleğe alır; bu boyutun üstündeki belgeler reddedilir.
pub const MAX_AUTHOR_INPUT_BYTES: usize = 256 * 1024 * 1024;
/// Alan ağacı yürüyüşünde en fazla düğüm ve derinlik.
const MAX_FIELD_NODES: usize = 100_000;
const MAX_FIELD_DEPTH: usize = 64;

const OPERATION: &str = "author_form_fields";

// ISO 32000-2 Tablo 226, 228, 229, 231 (bit n → 1 << (n - 1)).
const FF_READ_ONLY: i64 = 1;
const FF_REQUIRED: i64 = 1 << 1;
const FF_MULTILINE: i64 = 1 << 12;
const FF_NO_TOGGLE_TO_OFF: i64 = 1 << 14;
const FF_RADIO: i64 = 1 << 15;
const FF_COMBO: i64 = 1 << 17;
const FF_EDIT: i64 = 1 << 18;
const FF_MULTI_SELECT: i64 = 1 << 21;
/// Annotation flag Print.
const ANNOT_PRINT: i64 = 4;

const BORDER_GRAY: f32 = 0.5;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FormAuthorOp {
    /// Sayfaya yeni alan ekler. `name` yoksa Acrobat kuralıyla üretilir (`Text1` …).
    Add {
        page_index: u32,
        normalized_rect: NormalizedPageRect,
        field: NewField,
        name: Option<String>,
    },
    /// Widget'ı sayfadan, alanı formdan siler; alanın son widget'ıysa üst alan da kalkar.
    Delete {
        page_index: u32,
        annot_index: u32,
        field_name: String,
    },
    /// Widget'ı taşır/boyutlandırır. Görünüm akışı PDF kuralıyla yeni `/Rect`'e ölçeklenir.
    Move {
        page_index: u32,
        annot_index: u32,
        field_name: String,
        normalized_rect: NormalizedPageRect,
    },
    /// Widget'ın ait olduğu alanın özelliklerini değiştirir.
    SetProps {
        page_index: u32,
        annot_index: u32,
        field_name: String,
        props: FieldPropsPatch,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NewField {
    Text {
        multiline: bool,
    },
    Checkbox,
    /// `group` adında radio grubu varsa ona katılır, yoksa grup oluşturulur.
    Radio {
        group: String,
        export: String,
    },
    Combo {
        options: Vec<String>,
        editable: bool,
    },
    List {
        options: Vec<String>,
        multi: bool,
    },
}

/// `None` alanlar değişmez. `tooltip: ""` ipucunu, `max_len: 0` sınırı kaldırır.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FieldPropsPatch {
    pub name: Option<String>,
    pub tooltip: Option<String>,
    pub required: Option<bool>,
    pub read_only: Option<bool>,
    pub multiline: Option<bool>,
    pub max_len: Option<u32>,
    pub options: Option<Vec<String>>,
}

impl FormAuthorOp {
    fn page_index(&self) -> u32 {
        match self {
            Self::Add { page_index, .. }
            | Self::Delete { page_index, .. }
            | Self::Move { page_index, .. }
            | Self::SetProps { page_index, .. } => *page_index,
        }
    }
}

// ---------------------------------------------------------------------------
// İstek doğrulaması (belgeden bağımsız)
// ---------------------------------------------------------------------------

pub fn validate_request(ops: &[FormAuthorOp]) -> Result<(), String> {
    if ops.is_empty() {
        return Err("Form tasarım isteği boş".to_string());
    }
    if ops.len() > MAX_AUTHOR_OPS {
        return Err(format!(
            "Form tasarım isteği {MAX_AUTHOR_OPS} işlem sınırını aşıyor"
        ));
    }
    let mut targets = HashSet::new();
    for op in ops {
        match op {
            FormAuthorOp::Add {
                normalized_rect,
                field,
                name,
                ..
            } => {
                validate_rect(normalized_rect)?;
                if let Some(name) = name {
                    validate_partial_name(name)?;
                }
                match field {
                    NewField::Radio { group, export } => {
                        validate_partial_name(group)?;
                        validate_export(export)?;
                    }
                    NewField::Combo { options, .. } | NewField::List { options, .. } => {
                        validate_options(options)?;
                    }
                    NewField::Text { .. } | NewField::Checkbox => {}
                }
            }
            FormAuthorOp::Delete {
                page_index,
                annot_index,
                field_name,
            }
            | FormAuthorOp::Move {
                page_index,
                annot_index,
                field_name,
                ..
            }
            | FormAuthorOp::SetProps {
                page_index,
                annot_index,
                field_name,
                ..
            } => {
                if field_name.is_empty() || field_name.chars().count() > MAX_FIELD_LABEL_CHARS {
                    return Err("Hedef alan adı boş veya çok uzun".to_string());
                }
                if !targets.insert((*page_index, *annot_index)) {
                    return Err(format!(
                        "Sayfa {page_index} annotation {annot_index} istekte birden fazla kez hedefleniyor"
                    ));
                }
                if let FormAuthorOp::Move {
                    normalized_rect, ..
                } = op
                {
                    validate_rect(normalized_rect)?;
                }
                if let FormAuthorOp::SetProps { props, .. } = op {
                    if let Some(name) = &props.name {
                        validate_partial_name(name)?;
                    }
                    if let Some(tooltip) = &props.tooltip {
                        if tooltip.chars().count() > MAX_FIELD_LABEL_CHARS {
                            return Err("İpucu çok uzun".to_string());
                        }
                    }
                    if let Some(options) = &props.options {
                        validate_options(options)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_rect(rect: &NormalizedPageRect) -> Result<(), String> {
    validate_normalized_rect(rect).map_err(|failure| failure.detail)
}

fn validate_partial_name(name: &str) -> Result<(), String> {
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_FIELD_NAME_CHARS {
        return Err("Alan adı boş olamaz ve en fazla 255 karakter olabilir".to_string());
    }
    if name.contains('.') {
        return Err("Alan adı nokta içeremez (nokta alan hiyerarşisini ayırır)".to_string());
    }
    if name.chars().any(char::is_control) {
        return Err("Alan adı kontrol karakteri içeremez".to_string());
    }
    Ok(())
}

fn validate_export(export: &str) -> Result<(), String> {
    let chars = export.chars().count();
    if chars == 0 || chars > MAX_FIELD_NAME_CHARS || export == "Off" {
        return Err("Radio export değeri boş, çok uzun ya da 'Off' olamaz".to_string());
    }
    if export
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '/')
    {
        return Err("Radio export değeri boşluk, '/' veya kontrol karakteri içeremez".to_string());
    }
    Ok(())
}

fn validate_options(options: &[String]) -> Result<(), String> {
    if options.is_empty() || options.len() > MAX_FORM_OPTIONS_PER_FIELD {
        return Err("Seçenek listesi boş olamaz ve sınırı aşamaz".to_string());
    }
    if options
        .iter()
        .any(|option| option.is_empty() || option.chars().count() > MAX_FIELD_LABEL_CHARS)
    {
        return Err("Seçenekler boş olamaz ve en fazla 1024 karakter olabilir".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Giriş noktası
// ---------------------------------------------------------------------------

/// `file_path`'teki PDF'te form yapısını yerinde, atomik ve artımlı olarak değiştirir.
pub(crate) fn author_form_fields_report(
    file_path: String,
    ops: Vec<FormAuthorOp>,
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
    validate_request(&ops).map_err(preflight)?;
    if password.is_some() {
        return Err(preflight(
            "Şifreli PDF'lerde form tasarımı desteklenmiyor".to_string(),
        ));
    }
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        OPERATION,
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| author_and_save(pdfium, &source, &ops, temp),
        verify_saved,
    )
}

/// Kayıttan sonra PDFium ile doğrulanacak sonuç.
#[derive(Debug)]
pub(crate) struct Expected {
    /// Sayfa → beklenen form widget sayısı.
    widget_counts: BTreeMap<u32, usize>,
    checks: Vec<ExpectedWidget>,
}

#[derive(Debug)]
struct ExpectedWidget {
    page_index: u32,
    field_name: String,
    kind: FormFieldKind,
    normalized_rect: Option<NormalizedPageRect>,
    required: Option<bool>,
    read_only: Option<bool>,
    multiline: Option<bool>,
    options: Option<Vec<String>>,
}

fn author_and_save(
    pdfium: &Pdfium,
    source: &Path,
    ops: &[FormAuthorOp],
    temp: &Path,
) -> Result<Expected, DocumentIoFailure> {
    let bytes = std::fs::read(source)
        .map_err(|error| DocumentIoFailure::preflight(format!("PDF okunamadı: {error}")))?;
    if bytes.len() > MAX_AUTHOR_INPUT_BYTES {
        return Err(DocumentIoFailure::preflight(
            "PDF form tasarımı için çok büyük (256 MiB sınırı)",
        ));
    }

    // PDFium: geometri ve mevcut widget sayıları.
    let geometry = pdfium
        .load_pdf_from_file(source, None)
        .map_err(|error| DocumentIoFailure::preflight(format!("PDF açılamadı: {error}")))?;
    if !matches!(
        geometry.permissions().security_handler_revision(),
        Ok(PdfSecurityHandlerRevision::Unprotected)
    ) {
        return Err(DocumentIoFailure::preflight(
            "Şifreli PDF'lerde form tasarımı desteklenmiyor",
        ));
    }
    let page_count = u32::from(geometry.pages().len());
    let mut widget_counts = BTreeMap::new();
    let mut transforms = BTreeMap::new();
    for op in ops {
        let page_index = op.page_index();
        if page_index >= page_count {
            return Err(DocumentIoFailure::preflight(format!(
                "Sayfa {page_index} yok (sayfa sayısı {page_count})"
            )));
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = transforms.entry(page_index) {
            let page = geometry.pages().get(page_index as u16).map_err(|error| {
                DocumentIoFailure::preflight(format!("Sayfa açılamadı: {error}"))
            })?;
            entry.insert(
                PageDisplayTransform::for_page(&page).map_err(DocumentIoFailure::preflight)?,
            );
            let widgets = super::read_page_form_fields(&geometry, page_index).map_err(|error| {
                DocumentIoFailure::preflight(format!("Form okunamadı: {error:?}"))
            })?;
            widget_counts.insert(page_index, widgets.len());
        }
    }
    drop(geometry);

    // lopdf: yapı.
    let previous = lopdf::Document::load_mem(&bytes).map_err(|error| {
        DocumentIoFailure::preflight(format!("PDF yapısı ayrıştırılamadı: {error}"))
    })?;
    if previous.is_encrypted() {
        return Err(DocumentIoFailure::preflight(
            "Şifreli PDF'lerde form tasarımı desteklenmiyor",
        ));
    }
    let mut writer = Writer::new(IncrementalDocument::create_from(bytes.clone(), previous));
    let pages = writer.page_ids();

    // Hedefler istekteki tüm işlemlerden önce, orijinal annotation sırasına göre çözülür.
    let mut resolved = Vec::with_capacity(ops.len());
    for op in ops {
        let page_id = *pages.get(&(op.page_index() + 1)).ok_or_else(|| {
            DocumentIoFailure::preflight(format!("Sayfa {} yapıda bulunamadı", op.page_index()))
        })?;
        let target = match op {
            FormAuthorOp::Add { .. } => None,
            FormAuthorOp::Delete {
                annot_index,
                field_name,
                ..
            }
            | FormAuthorOp::Move {
                annot_index,
                field_name,
                ..
            }
            | FormAuthorOp::SetProps {
                annot_index,
                field_name,
                ..
            } => Some(writer.resolve_widget(page_id, *annot_index, field_name)?),
        };
        resolved.push((page_id, target));
    }

    let mut names = writer.field_names()?;
    let mut checks = Vec::new();
    for (op, (page_id, target)) in ops.iter().zip(resolved) {
        let page_index = op.page_index();
        let transform = &transforms[&page_index];
        match op {
            FormAuthorOp::Add {
                normalized_rect,
                field,
                name,
                ..
            } => {
                let rect = user_rect(transform, normalized_rect);
                let full_name =
                    writer.add_field(page_id, rect, field, name.as_deref(), &mut names)?;
                *widget_counts.entry(page_index).or_default() += 1;
                checks.push(ExpectedWidget {
                    page_index,
                    field_name: full_name,
                    kind: new_field_kind(field),
                    normalized_rect: Some(normalized_rect.clone()),
                    required: None,
                    read_only: None,
                    multiline: match field {
                        NewField::Text { multiline } => Some(*multiline),
                        _ => None,
                    },
                    options: match field {
                        NewField::Combo { options, .. } | NewField::List { options, .. } => {
                            Some(options.clone())
                        }
                        _ => None,
                    },
                });
            }
            FormAuthorOp::Delete { .. } => {
                let target = target.expect("resolved");
                writer.delete_widget(page_id, &target)?;
                names.remove(&target.full_name);
                if let Some(count) = widget_counts.get_mut(&page_index) {
                    *count = count.saturating_sub(1);
                }
            }
            FormAuthorOp::Move {
                normalized_rect, ..
            } => {
                let target = target.expect("resolved");
                writer.edit(target.widget, |widget| {
                    widget.set("Rect", rect_object(user_rect(transform, normalized_rect)));
                    Ok(())
                })?;
                checks.push(ExpectedWidget {
                    page_index,
                    field_name: target.full_name.clone(),
                    kind: target.kind,
                    normalized_rect: Some(normalized_rect.clone()),
                    required: None,
                    read_only: None,
                    multiline: None,
                    options: None,
                });
            }
            FormAuthorOp::SetProps { props, .. } => {
                let target = target.expect("resolved");
                let full_name = writer.set_props(&target, props, &mut names)?;
                checks.push(ExpectedWidget {
                    page_index,
                    field_name: full_name,
                    kind: target.kind,
                    normalized_rect: None,
                    required: props.required,
                    read_only: props.read_only,
                    multiline: props.multiline,
                    options: props.options.clone(),
                });
            }
        }
    }

    let mut output = Vec::with_capacity(bytes.len() + 16 * 1024);
    writer.incremental.save_to(&mut output).map_err(|error| {
        DocumentIoFailure::write(format!("Artımlı güncelleme yazılamadı: {error}"))
    })?;
    if !output.starts_with(&bytes) {
        return Err(DocumentIoFailure::invariant(
            "Artımlı güncelleme orijinal baytları korumadı",
        ));
    }
    std::fs::write(temp, &output)
        .map_err(|error| DocumentIoFailure::write(format!("Temp PDF yazılamadı: {error}")))?;
    Ok(Expected {
        widget_counts,
        checks,
    })
}

fn new_field_kind(field: &NewField) -> FormFieldKind {
    match field {
        NewField::Text { .. } => FormFieldKind::Text,
        NewField::Checkbox => FormFieldKind::Checkbox,
        NewField::Radio { .. } => FormFieldKind::RadioButton,
        NewField::Combo { .. } => FormFieldKind::ComboBox,
        NewField::List { .. } => FormFieldKind::ListBox,
    }
}

/// PDF user space dikdörtgeni: [sol, alt, sağ, üst].
fn user_rect(transform: &PageDisplayTransform, rect: &NormalizedPageRect) -> [f32; 4] {
    let user = transform.normalized_to_user_rect(rect);
    [user.x, user.y, user.x + user.width, user.y + user.height]
}

fn rect_object(rect: [f32; 4]) -> Object {
    Object::Array(rect.iter().map(|value| Object::Real(*value)).collect())
}

fn verify_saved(document: &PdfDocument<'_>, expected: &Expected) -> Result<(), DocumentIoFailure> {
    let mut pages: BTreeMap<u32, Vec<FormWidgetData>> = BTreeMap::new();
    for (&page_index, &count) in &expected.widget_counts {
        let widgets = super::read_page_form_fields(document, page_index).map_err(|error| {
            DocumentIoFailure::invariant(format!("Kaydedilen form okunamadı: {error:?}"))
        })?;
        if widgets.len() != count {
            return Err(DocumentIoFailure::invariant(format!(
                "Sayfa {page_index}: beklenen {count} form widget'ı, okunan {}",
                widgets.len()
            )));
        }
        pages.insert(page_index, widgets);
    }
    for check in &expected.checks {
        let found = pages[&check.page_index].iter().any(|widget| {
            widget.field_name == check.field_name
                && widget.kind == check.kind
                && check.normalized_rect.as_ref().is_none_or(|rect| {
                    widget
                        .normalized_rect
                        .as_ref()
                        .is_some_and(|read| close_rect(read, rect))
                })
                && check.required.is_none_or(|value| widget.required == value)
                && check
                    .read_only
                    .is_none_or(|value| widget.read_only == value)
                && check
                    .multiline
                    .is_none_or(|value| widget.multiline == value)
                && check.options.as_ref().is_none_or(|labels| {
                    widget
                        .options
                        .iter()
                        .map(|option| &option.label)
                        .eq(labels.iter())
                })
        });
        if !found {
            return Err(DocumentIoFailure::invariant(format!(
                "Kaydedilen belgede '{}' alanı beklenen biçimde bulunamadı",
                check.field_name
            )));
        }
    }
    Ok(())
}

fn close_rect(a: &NormalizedPageRect, b: &NormalizedPageRect) -> bool {
    const TOLERANCE: f32 = 1e-3;
    (a.x - b.x).abs() < TOLERANCE
        && (a.y - b.y).abs() < TOLERANCE
        && (a.width - b.width).abs() < TOLERANCE
        && (a.height - b.height).abs() < TOLERANCE
}

// ---------------------------------------------------------------------------
// lopdf artımlı yazım
// ---------------------------------------------------------------------------

/// Hedeflenen mevcut widget.
#[derive(Debug, Clone)]
struct Target {
    widget: ObjectId,
    /// `/T` taşıyan alan düğümü (widget kendisi ya da üst alanı).
    field: ObjectId,
    full_name: String,
    kind: FormFieldKind,
}

struct Writer {
    incremental: IncrementalDocument,
}

impl Writer {
    fn new(incremental: IncrementalDocument) -> Self {
        Self { incremental }
    }

    fn page_ids(&self) -> BTreeMap<u32, ObjectId> {
        self.incremental.get_prev_documents().get_pages()
    }

    /// Nesnenin güncel hali: yeni revizyonda değiştirildiyse o, değilse önceki.
    fn object(&self, id: ObjectId) -> Result<&Object, DocumentIoFailure> {
        self.incremental
            .new_document
            .get_object(id)
            .or_else(|_| self.incremental.get_prev_documents().get_object(id))
            .map_err(|_| {
                DocumentIoFailure::preflight(format!("Nesne {} {} bulunamadı", id.0, id.1))
            })
    }

    fn dict(&self, id: ObjectId) -> Result<Dictionary, DocumentIoFailure> {
        self.object(id)?.as_dict().cloned().map_err(|_| {
            DocumentIoFailure::preflight(format!("Nesne {} {} sözlük değil", id.0, id.1))
        })
    }

    /// Referansları (sınırlı) izleyerek değeri çözer.
    fn resolve<'a>(&'a self, mut object: &'a Object) -> Result<&'a Object, DocumentIoFailure> {
        for _ in 0..MAX_FIELD_DEPTH {
            match object {
                Object::Reference(id) => object = self.object(*id)?,
                other => return Ok(other),
            }
        }
        Err(DocumentIoFailure::preflight("Referans zinciri çok derin"))
    }

    fn add(&mut self, object: impl Into<Object>) -> ObjectId {
        self.incremental.new_document.add_object(object)
    }

    /// Nesneyi yeni revizyona kopyalayıp sözlük olarak düzenler.
    fn edit<R>(
        &mut self,
        id: ObjectId,
        f: impl FnOnce(&mut Dictionary) -> Result<R, DocumentIoFailure>,
    ) -> Result<R, DocumentIoFailure> {
        self.incremental
            .opt_clone_object_to_new_document(id)
            .map_err(|error| DocumentIoFailure::write(format!("Nesne kopyalanamadı: {error}")))?;
        let dict = self
            .incremental
            .new_document
            .get_object_mut(id)
            .and_then(Object::as_dict_mut)
            .map_err(|_| {
                DocumentIoFailure::write(format!("Nesne {} {} sözlük değil", id.0, id.1))
            })?;
        f(dict)
    }

    /// `id` nesnesinden başlayıp `path` anahtarlarını izleyen sözlüğü düzenler. Yolda bir
    /// referans varsa düzenleme o nesnede sürer; eksik sözlükler satır içi oluşturulur.
    fn edit_path(
        &mut self,
        id: ObjectId,
        path: &[&[u8]],
        f: &mut dyn FnMut(&mut Dictionary) -> Result<(), DocumentIoFailure>,
    ) -> Result<(), DocumentIoFailure> {
        let root = self.dict(id)?;
        let mut cursor = &root;
        for (index, key) in path.iter().enumerate() {
            match cursor.get(key) {
                Ok(Object::Reference(next)) => {
                    let next = *next;
                    return self.edit_path(next, &path[index + 1..], f);
                }
                Ok(Object::Dictionary(inner)) => cursor = inner,
                Ok(_) => {
                    return Err(DocumentIoFailure::preflight(format!(
                        "'{}' bir sözlük değil",
                        String::from_utf8_lossy(key)
                    )))
                }
                Err(_) => break,
            }
        }
        self.edit(id, |dict| {
            let mut current = dict;
            for key in path {
                if !current.has(key) {
                    current.set(key.to_vec(), Dictionary::new());
                }
                current = current
                    .get_mut(key)
                    .and_then(Object::as_dict_mut)
                    .map_err(|_| DocumentIoFailure::write("Sözlük yolu düzenlenemedi"))?;
            }
            f(current)
        })
    }

    /// `owner` sözlüğündeki (`path` ile) `key` dizisini düzenler; dizi referanssa o nesne
    /// düzenlenir, yoksa oluşturulur.
    fn edit_array(
        &mut self,
        owner: ObjectId,
        path: &[&[u8]],
        key: &[u8],
        f: &mut dyn FnMut(&mut Vec<Object>),
    ) -> Result<(), DocumentIoFailure> {
        let mut holder = owner;
        let mut cursor = self.dict(owner)?;
        let mut remaining = path;
        while let Some((first, rest)) = remaining.split_first() {
            match cursor.get(first).cloned() {
                Ok(Object::Reference(next)) => {
                    holder = next;
                    cursor = self.dict(next)?;
                    remaining = rest;
                    continue;
                }
                Ok(Object::Dictionary(inner)) => {
                    cursor = inner;
                    remaining = rest;
                    continue;
                }
                _ => break,
            }
        }
        if let Ok(Object::Reference(array_id)) = cursor.get(key) {
            let array_id = *array_id;
            self.incremental
                .opt_clone_object_to_new_document(array_id)
                .map_err(|error| {
                    DocumentIoFailure::write(format!("Dizi kopyalanamadı: {error}"))
                })?;
            let array = self
                .incremental
                .new_document
                .get_object_mut(array_id)
                .and_then(Object::as_array_mut)
                .map_err(|_| DocumentIoFailure::write("Dizi nesnesi düzenlenemedi"))?;
            f(array);
            return Ok(());
        }
        let inline_path: Vec<&[u8]> = if holder == owner {
            path.to_vec()
        } else {
            remaining.to_vec()
        };
        self.edit_path(holder, &inline_path, &mut |dict| {
            if !matches!(dict.get(key), Ok(Object::Array(_))) {
                dict.set(key.to_vec(), Object::Array(Vec::new()));
            }
            let array = dict
                .get_mut(key)
                .and_then(Object::as_array_mut)
                .map_err(|_| DocumentIoFailure::write("Dizi düzenlenemedi"))?;
            f(array);
            Ok(())
        })
    }

    fn catalog_id(&self) -> Result<ObjectId, DocumentIoFailure> {
        self.incremental
            .get_prev_documents()
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| DocumentIoFailure::preflight("Belge kataloğu bulunamadı"))
    }

    /// AcroForm sözlüğünün güncel hali (yoksa boş).
    fn acroform(&self) -> Result<Dictionary, DocumentIoFailure> {
        let catalog = self.dict(self.catalog_id()?)?;
        match catalog.get(b"AcroForm") {
            Ok(object) => self
                .resolve(object)?
                .as_dict()
                .cloned()
                .map_err(|_| DocumentIoFailure::preflight("AcroForm bir sözlük değil")),
            Err(_) => Ok(Dictionary::new()),
        }
    }

    /// `/T` zincirinden tam nitelikli ad (PDFium'un `FPDFAnnot_GetFormFieldName`'i ile aynı).
    fn full_name(&self, mut id: ObjectId) -> Result<String, DocumentIoFailure> {
        let mut parts = Vec::new();
        let mut seen = HashSet::new();
        for _ in 0..MAX_FIELD_DEPTH {
            if !seen.insert(id) {
                return Err(DocumentIoFailure::preflight("Alan ağacında döngü var"));
            }
            let dict = self.dict(id)?;
            if let Ok(title) = dict.get(b"T") {
                if let Ok(bytes) = self.resolve(title)?.as_str() {
                    parts.push(decode_text(bytes));
                }
            }
            match dict.get(b"Parent").and_then(Object::as_reference) {
                Ok(parent) => id = parent,
                Err(_) => {
                    parts.reverse();
                    return Ok(parts.join("."));
                }
            }
        }
        Err(DocumentIoFailure::preflight("Alan ağacı çok derin"))
    }

    /// Kalıtımla gelen bir anahtar (`/FT`, `/Ff` …).
    fn inherited(&self, mut id: ObjectId, key: &[u8]) -> Result<Option<Object>, DocumentIoFailure> {
        for _ in 0..MAX_FIELD_DEPTH {
            let dict = self.dict(id)?;
            if let Ok(value) = dict.get(key) {
                return Ok(Some(self.resolve(value)?.clone()));
            }
            match dict.get(b"Parent").and_then(Object::as_reference) {
                Ok(parent) => id = parent,
                Err(_) => return Ok(None),
            }
        }
        Err(DocumentIoFailure::preflight("Alan ağacı çok derin"))
    }

    fn field_kind(&self, widget: ObjectId) -> Result<FormFieldKind, DocumentIoFailure> {
        let field_type = self.inherited(widget, b"FT")?;
        let flags = self
            .inherited(widget, b"Ff")?
            .and_then(|value| value.as_i64().ok())
            .unwrap_or(0);
        Ok(
            match field_type.as_ref().and_then(|value| value.as_name().ok()) {
                Some(b"Tx") => FormFieldKind::Text,
                Some(b"Btn") if flags & (1 << 16) != 0 => FormFieldKind::PushButton,
                Some(b"Btn") if flags & FF_RADIO != 0 => FormFieldKind::RadioButton,
                Some(b"Btn") => FormFieldKind::Checkbox,
                Some(b"Ch") if flags & FF_COMBO != 0 => FormFieldKind::ComboBox,
                Some(b"Ch") => FormFieldKind::ListBox,
                Some(b"Sig") => FormFieldKind::Signature,
                _ => FormFieldKind::Unknown,
            },
        )
    }

    /// Sayfadaki `annot_index`'inci annotation'ı (PDFium sırası) çözer ve adını doğrular.
    fn resolve_widget(
        &self,
        page: ObjectId,
        annot_index: u32,
        field_name: &str,
    ) -> Result<Target, DocumentIoFailure> {
        let page_dict = self.dict(page)?;
        let annots = match page_dict.get(b"Annots") {
            Ok(object) => self
                .resolve(object)?
                .as_array()
                .cloned()
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let entry = annots.get(annot_index as usize).ok_or_else(|| {
            DocumentIoFailure::preflight(format!("Annotation {annot_index} sayfada yok"))
        })?;
        let widget = entry.as_reference().map_err(|_| {
            DocumentIoFailure::preflight("Satır içi annotation sözlükleri düzenlenemiyor")
        })?;
        let dict = self.dict(widget)?;
        if dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Widget".as_slice()) {
            return Err(DocumentIoFailure::preflight(format!(
                "Annotation {annot_index} bir form widget'ı değil"
            )));
        }
        let full_name = self.full_name(widget)?;
        if full_name != field_name {
            return Err(DocumentIoFailure::preflight(format!(
                "Annotation {annot_index} '{full_name}' alanına ait, istek '{field_name}' bekliyordu; belge okumadan bu yana değişmiş"
            )));
        }
        let field = if dict.has(b"T") {
            widget
        } else {
            dict.get(b"Parent")
                .and_then(Object::as_reference)
                .map_err(|_| DocumentIoFailure::preflight("Widget'ın alanı bulunamadı"))?
        };
        Ok(Target {
            widget,
            field,
            full_name,
            kind: self.field_kind(widget)?,
        })
    }

    /// Belgedeki tüm tam nitelikli alan adları: `/Fields` ağacı ve sayfa widget'ları.
    fn field_names(&self) -> Result<HashSet<String>, DocumentIoFailure> {
        let mut names = HashSet::new();
        let mut seen = HashSet::new();
        let mut stack: Vec<(ObjectId, usize)> = Vec::new();
        if let Ok(fields) = self.acroform()?.get(b"Fields") {
            for field in self
                .resolve(fields)?
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                if let Ok(id) = field.as_reference() {
                    stack.push((id, 0));
                }
            }
        }
        for page in self.page_ids().values() {
            if let Ok(annots) = self.dict(*page)?.get(b"Annots") {
                for annot in self
                    .resolve(annots)?
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    if let Ok(id) = annot.as_reference() {
                        stack.push((id, 0));
                    }
                }
            }
        }
        while let Some((id, depth)) = stack.pop() {
            if depth > MAX_FIELD_DEPTH || !seen.insert(id) {
                continue;
            }
            if seen.len() > MAX_FIELD_NODES {
                return Err(DocumentIoFailure::preflight("Form alan ağacı çok büyük"));
            }
            let Ok(dict) = self.dict(id) else { continue };
            if dict.has(b"T") {
                names.insert(self.full_name(id)?);
            }
            if let Ok(kids) = dict.get(b"Kids") {
                for kid in self.resolve(kids)?.as_array().cloned().unwrap_or_default() {
                    if let Ok(kid) = kid.as_reference() {
                        stack.push((kid, depth + 1));
                    }
                }
            }
        }
        Ok(names)
    }

    /// `/DR`'de `/Helv` ve `/ZaDb` fontlarını garanti eder; AcroForm yoksa oluşturur.
    fn ensure_acroform(&mut self) -> Result<(), DocumentIoFailure> {
        let catalog = self.catalog_id()?;
        if self.dict(catalog)?.get(b"AcroForm").is_err() {
            let acroform = self.add(dictionary! { "Fields" => Object::Array(Vec::new()) });
            self.edit(catalog, |dict| {
                dict.set("AcroForm", acroform);
                Ok(())
            })?;
        }
        let fonts = self.acroform()?;
        let existing = fonts
            .get(b"DR")
            .ok()
            .and_then(|dr| self.resolve(dr).ok())
            .and_then(|dr| dr.as_dict().ok())
            .and_then(|dr| dr.get(b"Font").ok())
            .and_then(|font| self.resolve(font).ok())
            .and_then(|font| font.as_dict().ok())
            .cloned()
            .unwrap_or_default();
        let mut missing = Vec::new();
        if !existing.has(b"Helv") {
            missing.push((
                b"Helv".to_vec(),
                self.add(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                    "Encoding" => "WinAnsiEncoding",
                }),
            ));
        }
        if !existing.has(b"ZaDb") {
            missing.push((
                b"ZaDb".to_vec(),
                self.add(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ZapfDingbats" }),
            ));
        }
        if missing.is_empty() {
            return Ok(());
        }
        self.edit_path(catalog, &[b"AcroForm", b"DR", b"Font"], &mut |font| {
            for (name, id) in &missing {
                font.set(name.clone(), *id);
            }
            Ok(())
        })
    }

    fn helv(&self) -> Result<Object, DocumentIoFailure> {
        self.acroform()?
            .get(b"DR")
            .ok()
            .and_then(|dr| self.resolve(dr).ok())
            .and_then(|dr| dr.as_dict().ok())
            .and_then(|dr| dr.get(b"Font").ok())
            .and_then(|font| self.resolve(font).ok())
            .and_then(|font| font.as_dict().ok())
            .and_then(|font| font.get(b"Helv").ok())
            .cloned()
            .ok_or_else(|| DocumentIoFailure::write("/Helv fontu bulunamadı"))
    }

    fn appearance(
        &mut self,
        width: f32,
        height: f32,
        content: String,
    ) -> Result<ObjectId, DocumentIoFailure> {
        let helv = self.helv()?;
        Ok(self.add(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => rect_object([0.0, 0.0, width, height]),
                "Resources" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
            },
            content.into_bytes(),
        )))
    }

    fn add_field(
        &mut self,
        page: ObjectId,
        rect: [f32; 4],
        field: &NewField,
        name: Option<&str>,
        names: &mut HashSet<String>,
    ) -> Result<String, DocumentIoFailure> {
        self.ensure_acroform()?;
        let catalog = self.catalog_id()?;
        let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
        let mut widget = dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect_object(rect),
            "F" => ANNOT_PRINT, "P" => page,
            "MK" => dictionary! {
                "BC" => vec![Object::Real(BORDER_GRAY), Object::Real(BORDER_GRAY), Object::Real(BORDER_GRAY)],
                "BG" => vec![Object::Real(1.0), Object::Real(1.0), Object::Real(1.0)],
            },
            "BS" => dictionary! { "W" => 1, "S" => "S" },
        };

        if let NewField::Radio { group, export } = field {
            let on = self.appearance(width, height, radio_on(width, height))?;
            let off = self.appearance(width, height, radio_off(width, height))?;
            widget.set("AS", "Off");
            widget.set("AP", dictionary! { "N" => dictionary! { export.as_bytes().to_vec() => on, "Off" => off } });
            widget.set("DA", text_string("/ZaDb 0 Tf 0 g"));
            if let Some(mk) = widget
                .get_mut(b"MK")
                .ok()
                .and_then(|mk| mk.as_dict_mut().ok())
            {
                mk.set("CA", text_string("l"));
            }
            let existing = self.find_radio_group(group)?;
            let parent = match existing {
                Some(parent) => parent,
                None => {
                    if names.contains(group) {
                        return Err(DocumentIoFailure::preflight(format!(
                            "'{group}' adında radio olmayan bir alan zaten var"
                        )));
                    }
                    let parent = self.add(dictionary! {
                        "FT" => "Btn", "T" => text_string(group), "Ff" => FF_RADIO | FF_NO_TOGGLE_TO_OFF,
                        "V" => "Off", "Kids" => Object::Array(Vec::new()), "DA" => text_string("/ZaDb 0 Tf 0 g"),
                    });
                    self.edit_array(catalog, &[b"AcroForm"], b"Fields", &mut |fields| {
                        fields.push(Object::Reference(parent));
                    })?;
                    names.insert(group.clone());
                    parent
                }
            };
            widget.set("Parent", parent);
            let widget = self.add(widget);
            self.edit_array(parent, &[], b"Kids", &mut |kids| {
                kids.push(Object::Reference(widget))
            })?;
            self.edit_array(page, &[], b"Annots", &mut |annots| {
                annots.push(Object::Reference(widget))
            })?;
            return Ok(group.clone());
        }

        let full_name = match name {
            Some(name) => {
                if names.contains(name) {
                    return Err(DocumentIoFailure::preflight(format!(
                        "'{name}' adında bir alan zaten var"
                    )));
                }
                name.to_string()
            }
            None => default_name(field, names),
        };
        widget.set("T", text_string(&full_name));
        match field {
            NewField::Text { multiline } => {
                widget.set("FT", "Tx");
                widget.set("DA", text_string("/Helv 12 Tf 0 g"));
                if *multiline {
                    widget.set("Ff", FF_MULTILINE);
                }
                let ap = self.appearance(width, height, box_appearance(width, height, true))?;
                widget.set("AP", dictionary! { "N" => ap });
            }
            NewField::Checkbox => {
                let on = self.appearance(width, height, checkbox_on(width, height))?;
                let off = self.appearance(width, height, box_appearance(width, height, false))?;
                widget.set("FT", "Btn");
                widget.set("V", "Off");
                widget.set("AS", "Off");
                widget.set("DA", text_string("/ZaDb 0 Tf 0 g"));
                if let Some(mk) = widget
                    .get_mut(b"MK")
                    .ok()
                    .and_then(|mk| mk.as_dict_mut().ok())
                {
                    mk.set("CA", text_string("4"));
                }
                widget.set(
                    "AP",
                    dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } },
                );
            }
            NewField::Combo { options, editable } => {
                widget.set("FT", "Ch");
                widget.set("Ff", FF_COMBO | if *editable { FF_EDIT } else { 0 });
                widget.set(
                    "Opt",
                    Object::Array(options.iter().map(|option| text_string(option)).collect()),
                );
                widget.set("DA", text_string("/Helv 12 Tf 0 g"));
                let ap = self.appearance(width, height, box_appearance(width, height, true))?;
                widget.set("AP", dictionary! { "N" => ap });
            }
            NewField::List { options, multi } => {
                widget.set("FT", "Ch");
                if *multi {
                    widget.set("Ff", FF_MULTI_SELECT);
                }
                widget.set(
                    "Opt",
                    Object::Array(options.iter().map(|option| text_string(option)).collect()),
                );
                widget.set("DA", text_string("/Helv 12 Tf 0 g"));
                let ap = self.appearance(width, height, box_appearance(width, height, true))?;
                widget.set("AP", dictionary! { "N" => ap });
            }
            NewField::Radio { .. } => unreachable!("handled above"),
        }
        let widget = self.add(widget);
        self.edit_array(catalog, &[b"AcroForm"], b"Fields", &mut |fields| {
            fields.push(Object::Reference(widget))
        })?;
        self.edit_array(page, &[], b"Annots", &mut |annots| {
            annots.push(Object::Reference(widget))
        })?;
        names.insert(full_name.clone());
        Ok(full_name)
    }

    /// Üst düzeyde `group` adını taşıyan radio alanı.
    fn find_radio_group(&self, group: &str) -> Result<Option<ObjectId>, DocumentIoFailure> {
        let Ok(fields) = self.acroform()?.get(b"Fields").cloned() else {
            return Ok(None);
        };
        for field in self
            .resolve(&fields)?
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let Ok(id) = field.as_reference() else {
                continue;
            };
            let Ok(dict) = self.dict(id) else { continue };
            let is_group = dict.has(b"Kids")
                && dict
                    .get(b"T")
                    .ok()
                    .and_then(|title| title.as_str().ok())
                    .map(decode_text)
                    .as_deref()
                    == Some(group);
            if is_group {
                let flags = dict.get(b"Ff").and_then(Object::as_i64).unwrap_or(0);
                if dict.get(b"FT").and_then(Object::as_name).ok() == Some(b"Btn".as_slice())
                    && flags & FF_RADIO != 0
                {
                    return Ok(Some(id));
                }
                return Err(DocumentIoFailure::preflight(format!(
                    "'{group}' adında radio olmayan bir alan zaten var"
                )));
            }
        }
        Ok(None)
    }

    fn delete_widget(&mut self, page: ObjectId, target: &Target) -> Result<(), DocumentIoFailure> {
        let widget = target.widget;
        let catalog = self.catalog_id()?;
        let remove = |list: &mut Vec<Object>, id: ObjectId| {
            list.retain(|entry| entry.as_reference().ok() != Some(id));
        };
        self.edit_array(page, &[], b"Annots", &mut |annots| remove(annots, widget))?;
        if target.field == widget {
            // Alan ile widget aynı nesne: alanı üstünden ya da /Fields'ten çıkar.
            match self
                .dict(widget)?
                .get(b"Parent")
                .and_then(Object::as_reference)
            {
                Ok(parent) => {
                    self.edit_array(parent, &[], b"Kids", &mut |kids| remove(kids, widget))?
                }
                Err(_) => self.edit_array(catalog, &[b"AcroForm"], b"Fields", &mut |fields| {
                    remove(fields, widget)
                })?,
            }
            return Ok(());
        }
        // Widget, alanın çocuğu: alanın son widget'ıysa alan da kalkar.
        let field = target.field;
        let mut remaining = 0;
        self.edit_array(field, &[], b"Kids", &mut |kids| {
            remove(kids, widget);
            remaining = kids.len();
        })?;
        if remaining == 0 {
            match self
                .dict(field)?
                .get(b"Parent")
                .and_then(Object::as_reference)
            {
                Ok(parent) => {
                    self.edit_array(parent, &[], b"Kids", &mut |kids| remove(kids, field))?
                }
                Err(_) => self.edit_array(catalog, &[b"AcroForm"], b"Fields", &mut |fields| {
                    remove(fields, field)
                })?,
            }
        }
        Ok(())
    }

    fn set_props(
        &mut self,
        target: &Target,
        props: &FieldPropsPatch,
        names: &mut HashSet<String>,
    ) -> Result<String, DocumentIoFailure> {
        let is_text = target.kind == FormFieldKind::Text;
        let is_choice = matches!(
            target.kind,
            FormFieldKind::ComboBox | FormFieldKind::ListBox
        );
        if props.multiline.is_some() && !is_text {
            return Err(DocumentIoFailure::preflight(
                "Çok satırlı yalnız metin alanlarında ayarlanabilir",
            ));
        }
        if props.max_len.is_some() && !is_text {
            return Err(DocumentIoFailure::preflight(
                "En fazla karakter yalnız metin alanlarında ayarlanabilir",
            ));
        }
        if props.options.is_some() && !is_choice {
            return Err(DocumentIoFailure::preflight(
                "Seçenekler yalnız liste ve açılır kutularda ayarlanabilir",
            ));
        }
        let mut full_name = target.full_name.clone();
        if let Some(name) = &props.name {
            let prefix = target
                .full_name
                .rsplit_once('.')
                .map(|(prefix, _)| format!("{prefix}."))
                .unwrap_or_default();
            let renamed = format!("{prefix}{name}");
            if renamed != target.full_name {
                if names.contains(&renamed) {
                    return Err(DocumentIoFailure::preflight(format!(
                        "'{renamed}' adında bir alan zaten var"
                    )));
                }
                names.remove(&target.full_name);
                names.insert(renamed.clone());
                full_name = renamed;
            }
        }
        let current_flags = self
            .inherited(target.field, b"Ff")?
            .and_then(|value| value.as_i64().ok())
            .unwrap_or(0);
        let mut flags = current_flags;
        for (value, bit) in [
            (props.required, FF_REQUIRED),
            (props.read_only, FF_READ_ONLY),
            (props.multiline, FF_MULTILINE),
        ] {
            match value {
                Some(true) => flags |= bit,
                Some(false) => flags &= !bit,
                None => {}
            }
        }
        self.edit(target.field, |field| {
            if let Some(name) = &props.name {
                field.set("T", text_string(name));
            }
            if let Some(tooltip) = &props.tooltip {
                if tooltip.is_empty() {
                    field.remove(b"TU");
                } else {
                    field.set("TU", text_string(tooltip));
                }
            }
            if flags != current_flags {
                field.set("Ff", flags);
            }
            if let Some(max_len) = props.max_len {
                if max_len == 0 {
                    field.remove(b"MaxLen");
                } else {
                    field.set("MaxLen", i64::from(max_len));
                }
            }
            if let Some(options) = &props.options {
                field.set(
                    "Opt",
                    Object::Array(options.iter().map(|option| text_string(option)).collect()),
                );
            }
            Ok(())
        })?;
        Ok(full_name)
    }
}

/// İlk boş Acrobat adı: `Text1`, `Check Box1`, `Dropdown1`, `List Box1`.
fn default_name(field: &NewField, names: &HashSet<String>) -> String {
    let base = match field {
        NewField::Text { .. } => "Text",
        NewField::Checkbox => "Check Box",
        NewField::Combo { .. } => "Dropdown",
        NewField::List { .. } => "List Box",
        NewField::Radio { .. } => "Group",
    };
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|candidate| !names.contains(candidate))
        .expect("unbounded")
}

/// PDF text string: ASCII literal, diğerleri UTF-16BE (BOM).
fn text_string(text: &str) -> Object {
    if text.is_ascii() {
        Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

/// PDF text string çözümü: UTF-16BE (BOM) ya da PDFDocEncoding (Latin-1 ile yaklaşık).
fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

fn fmt(value: f32) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let text = format!("{rounded:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Beyaz zemin ve gri kenar; metin/seçim alanında boş `/Tx` işaretli içerik bloğu.
fn box_appearance(width: f32, height: f32, text_block: bool) -> String {
    let mut content = format!(
        "1 g 0 0 {w} {h} re f {gray} G 1 w 0.5 0.5 {iw} {ih} re S",
        w = fmt(width),
        h = fmt(height),
        gray = fmt(BORDER_GRAY),
        iw = fmt((width - 1.0).max(0.0)),
        ih = fmt((height - 1.0).max(0.0)),
    );
    if text_block {
        content.push_str(" /Tx BMC EMC");
    }
    content
}

/// Kutu ve onay işareti (yollarla; font gerekmez).
fn checkbox_on(width: f32, height: f32) -> String {
    format!(
        "{} 0 g {line} w 1 J 1 j {x1} {y1} m {x2} {y2} l {x3} {y3} l S",
        box_appearance(width, height, false),
        line = fmt((width.min(height) / 10.0).max(1.0)),
        x1 = fmt(width * 0.22),
        y1 = fmt(height * 0.52),
        x2 = fmt(width * 0.42),
        y2 = fmt(height * 0.28),
        x3 = fmt(width * 0.78),
        y3 = fmt(height * 0.74),
    )
}

/// Çember yolu (4 Bezier eğrisi).
fn circle(cx: f32, cy: f32, r: f32) -> String {
    const KAPPA: f32 = 0.552_284_8;
    let k = r * KAPPA;
    format!(
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c",
        fmt(cx + r),
        fmt(cy),
        fmt(cx + r),
        fmt(cy + k),
        fmt(cx + k),
        fmt(cy + r),
        fmt(cx),
        fmt(cy + r),
        fmt(cx - k),
        fmt(cy + r),
        fmt(cx - r),
        fmt(cy + k),
        fmt(cx - r),
        fmt(cy),
        fmt(cx - r),
        fmt(cy - k),
        fmt(cx - k),
        fmt(cy - r),
        fmt(cx),
        fmt(cy - r),
        fmt(cx + k),
        fmt(cy - r),
        fmt(cx + r),
        fmt(cy - k),
        fmt(cx + r),
        fmt(cy),
    )
}

fn radio_off(width: f32, height: f32) -> String {
    let r = (width.min(height) / 2.0 - 0.5).max(0.5);
    let (cx, cy) = (width / 2.0, height / 2.0);
    format!(
        "1 g {} f {} G 1 w {} S",
        circle(cx, cy, r),
        fmt(BORDER_GRAY),
        circle(cx, cy, r)
    )
}

fn radio_on(width: f32, height: f32) -> String {
    let r = (width.min(height) / 2.0 - 0.5).max(0.5);
    let (cx, cy) = (width / 2.0, height / 2.0);
    format!(
        "{} 0 g {} f",
        radio_off(width, height),
        circle(cx, cy, r * 0.45)
    )
}

#[cfg(test)]
mod tests {
    use super::super::fill::{fill_form_fields_report, FormFieldEdit, FormFieldValue};
    use super::super::test_support::*;
    use super::*;

    // standard_fixture annotation order: name 0, notes 1, code 2, locked 3, agree 4,
    // newsletter 5, color Red 6 / Blue 7, country 8, langs 9, secret 10.

    fn rect(x: f32, y: f32, width: f32, height: f32) -> NormalizedPageRect {
        NormalizedPageRect {
            x,
            y,
            width,
            height,
        }
    }

    fn add(field: NewField, name: Option<&str>, at: NormalizedPageRect) -> FormAuthorOp {
        FormAuthorOp::Add {
            page_index: 0,
            normalized_rect: at,
            field,
            name: name.map(str::to_string),
        }
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("form.pdf");
        standard_fixture(&path);
        (directory, path)
    }

    fn author(path: &Path, ops: Vec<FormAuthorOp>) -> Result<AtomicWriteReport, DocumentIoError> {
        author_form_fields_report(path.to_string_lossy().to_string(), ops, None)
    }

    fn rejected(path: &Path, ops: Vec<FormAuthorOp>) -> DocumentIoError {
        let before = std::fs::read(path).unwrap();
        let error = author(path, ops).unwrap_err();
        assert_eq!(
            std::fs::read(path).unwrap(),
            before,
            "rejected request changed the file"
        );
        error
    }

    fn read(path: &Path) -> Vec<FormWidgetData> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            super::super::read_page_form_fields(&document, 0).map_err(|error| format!("{error:?}"))
        })
        .unwrap()
    }

    fn named<'a>(widgets: &'a [FormWidgetData], name: &str) -> Vec<&'a FormWidgetData> {
        widgets
            .iter()
            .filter(|widget| widget.field_name == name)
            .collect()
    }

    #[test]
    fn adds_every_kind_incrementally_and_the_fill_path_fills_them() {
        let (_directory, path) = fixture();
        let original = std::fs::read(&path).unwrap();
        author(
            &path,
            vec![
                add(
                    NewField::Text { multiline: true },
                    None,
                    rect(0.55, 0.30, 0.30, 0.06),
                ),
                add(NewField::Checkbox, None, rect(0.55, 0.40, 0.03, 0.02)),
                add(
                    NewField::Combo {
                        options: vec!["Bir".into(), "İki".into()],
                        editable: false,
                    },
                    None,
                    rect(0.55, 0.45, 0.25, 0.03),
                ),
                add(
                    NewField::List {
                        options: vec!["A".into(), "B".into(), "C".into()],
                        multi: true,
                    },
                    None,
                    rect(0.55, 0.50, 0.25, 0.08),
                ),
                add(
                    NewField::Radio {
                        group: "Answer".into(),
                        export: "Yes".into(),
                    },
                    None,
                    rect(0.55, 0.62, 0.03, 0.02),
                ),
                add(
                    NewField::Radio {
                        group: "Answer".into(),
                        export: "No".into(),
                    },
                    None,
                    rect(0.60, 0.62, 0.03, 0.02),
                ),
            ],
        )
        .unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(
            saved.starts_with(&original),
            "authoring rewrote the original bytes"
        );

        let widgets = read(&path);
        assert_eq!(widgets.len(), 11 + 6);
        let text = named(&widgets, "Text1")[0];
        assert_eq!(text.kind, FormFieldKind::Text);
        assert!(text.multiline);
        assert!(close_rect(
            text.normalized_rect.as_ref().unwrap(),
            &rect(0.55, 0.30, 0.30, 0.06)
        ));
        assert_eq!(named(&widgets, "Check Box1")[0].checked, Some(false));
        let combo = named(&widgets, "Dropdown1")[0];
        assert_eq!(
            combo
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Bir", "İki"]
        );
        assert!(named(&widgets, "List Box1")[0].multi_select);
        let radios = named(&widgets, "Answer");
        assert_eq!(radios.len(), 2);
        assert!(radios
            .iter()
            .all(|radio| radio.kind == FormFieldKind::RadioButton));

        let index = |name: &str, nth: usize| named(&widgets, name)[nth].annot_index;
        fill_form_fields_report(
            path.to_string_lossy().to_string(),
            vec![
                FormFieldEdit {
                    page_index: 0,
                    annot_index: index("Text1", 0),
                    field_name: "Text1".into(),
                    value: FormFieldValue::Text {
                        text: "Satır".into(),
                    },
                },
                FormFieldEdit {
                    page_index: 0,
                    annot_index: index("Check Box1", 0),
                    field_name: "Check Box1".into(),
                    value: FormFieldValue::Checkbox { checked: true },
                },
                FormFieldEdit {
                    page_index: 0,
                    annot_index: index("Dropdown1", 0),
                    field_name: "Dropdown1".into(),
                    value: FormFieldValue::Choice { selected: vec![1] },
                },
                FormFieldEdit {
                    page_index: 0,
                    annot_index: index("Answer", 1),
                    field_name: "Answer".into(),
                    value: FormFieldValue::Radio,
                },
            ],
            None,
        )
        .unwrap();
        let filled = read(&path);
        assert_eq!(named(&filled, "Text1")[0].value.as_deref(), Some("Satır"));
        assert_eq!(named(&filled, "Check Box1")[0].checked, Some(true));
        assert!(named(&filled, "Dropdown1")[0].options[1].selected);
        assert_eq!(
            named(&filled, "Answer")
                .iter()
                .map(|radio| radio.checked)
                .collect::<Vec<_>>(),
            vec![Some(false), Some(true)]
        );
    }

    #[test]
    fn default_names_skip_existing_ones_and_explicit_duplicates_are_rejected() {
        let (_directory, path) = fixture();
        author(
            &path,
            vec![
                add(
                    NewField::Text { multiline: false },
                    None,
                    rect(0.6, 0.3, 0.2, 0.03),
                ),
                add(
                    NewField::Text { multiline: false },
                    None,
                    rect(0.6, 0.35, 0.2, 0.03),
                ),
            ],
        )
        .unwrap();
        let widgets = read(&path);
        assert_eq!(named(&widgets, "Text1").len(), 1);
        assert_eq!(named(&widgets, "Text2").len(), 1);
        let error = rejected(
            &path,
            vec![add(
                NewField::Text { multiline: false },
                Some("name"),
                rect(0.6, 0.4, 0.2, 0.03),
            )],
        );
        assert!(error.detail.contains("zaten var"), "{}", error.detail);
        let error = rejected(
            &path,
            vec![add(
                NewField::Radio {
                    group: "name".into(),
                    export: "A".into(),
                },
                None,
                rect(0.6, 0.4, 0.03, 0.02),
            )],
        );
        assert!(error.detail.contains("radio olmayan"), "{}", error.detail);
    }

    #[test]
    fn delete_removes_widgets_and_empty_parent_fields() {
        let (_directory, path) = fixture();
        author(
            &path,
            vec![
                FormAuthorOp::Delete {
                    page_index: 0,
                    annot_index: 1,
                    field_name: "notes".into(),
                },
                FormAuthorOp::Delete {
                    page_index: 0,
                    annot_index: 6,
                    field_name: "color".into(),
                },
            ],
        )
        .unwrap();
        let widgets = read(&path);
        assert!(named(&widgets, "notes").is_empty());
        assert_eq!(named(&widgets, "color").len(), 1);
        let blue = named(&widgets, "color")[0].annot_index;
        author(
            &path,
            vec![FormAuthorOp::Delete {
                page_index: 0,
                annot_index: blue,
                field_name: "color".into(),
            }],
        )
        .unwrap();
        assert!(named(&read(&path), "color").is_empty());

        // The radio parent left the form's /Fields once its last kid was gone.
        let document = lopdf::Document::load(&path).unwrap();
        let catalog = document.catalog().unwrap();
        let acroform = document
            .dereference(catalog.get(b"AcroForm").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        let fields = document
            .dereference(acroform.get(b"Fields").unwrap())
            .unwrap()
            .1
            .as_array()
            .unwrap();
        let titles: Vec<String> = fields
            .iter()
            .map(|field| {
                let dict = document.dereference(field).unwrap().1.as_dict().unwrap();
                decode_text(dict.get(b"T").unwrap().as_str().unwrap())
            })
            .collect();
        assert!(
            !titles.contains(&"color".to_string()) && !titles.contains(&"notes".to_string()),
            "{titles:?}"
        );
    }

    #[test]
    fn move_updates_the_widget_rect() {
        let (_directory, path) = fixture();
        let target = rect(0.2, 0.6, 0.4, 0.05);
        author(
            &path,
            vec![FormAuthorOp::Move {
                page_index: 0,
                annot_index: 0,
                field_name: "name".into(),
                normalized_rect: target.clone(),
            }],
        )
        .unwrap();
        let moved = read(&path);
        assert!(close_rect(
            named(&moved, "name")[0].normalized_rect.as_ref().unwrap(),
            &target
        ));
        assert_eq!(named(&moved, "name")[0].value.as_deref(), Some("Ada"));
    }

    #[test]
    fn set_props_renames_and_changes_flags_options_and_limits() {
        let (_directory, path) = fixture();
        author(
            &path,
            vec![
                FormAuthorOp::SetProps {
                    page_index: 0,
                    annot_index: 0,
                    field_name: "name".into(),
                    props: FieldPropsPatch {
                        name: Some("fullName".into()),
                        tooltip: Some("Ad soyad".into()),
                        required: Some(true),
                        read_only: Some(true),
                        ..Default::default()
                    },
                },
                FormAuthorOp::SetProps {
                    page_index: 0,
                    annot_index: 8,
                    field_name: "country".into(),
                    props: FieldPropsPatch {
                        options: Some(vec!["TR".into(), "DE".into(), "FR".into()]),
                        ..Default::default()
                    },
                },
                FormAuthorOp::SetProps {
                    page_index: 0,
                    annot_index: 1,
                    field_name: "notes".into(),
                    props: FieldPropsPatch {
                        multiline: Some(false),
                        max_len: Some(4),
                        ..Default::default()
                    },
                },
            ],
        )
        .unwrap();
        let widgets = read(&path);
        let renamed = named(&widgets, "fullName")[0];
        assert!(renamed.required && renamed.read_only);
        assert!(named(&widgets, "name").is_empty());
        assert_eq!(
            named(&widgets, "country")[0]
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["TR", "DE", "FR"]
        );
        assert!(!named(&widgets, "notes")[0].multiline);
        // MaxLen 4 is enforced by the fill path.
        let error = fill_form_fields_report(
            path.to_string_lossy().to_string(),
            vec![FormFieldEdit {
                page_index: 0,
                annot_index: 1,
                field_name: "notes".into(),
                value: FormFieldValue::Text {
                    text: "12345".into(),
                },
            }],
            None,
        )
        .unwrap_err();
        assert!(error.detail.contains("MaxLen"), "{}", error.detail);

        let error = rejected(
            &path,
            vec![FormAuthorOp::SetProps {
                page_index: 0,
                annot_index: 2,
                field_name: "code".into(),
                props: FieldPropsPatch {
                    name: Some("fullName".into()),
                    ..Default::default()
                },
            }],
        );
        assert!(error.detail.contains("zaten var"), "{}", error.detail);
        let error = rejected(
            &path,
            vec![FormAuthorOp::SetProps {
                page_index: 0,
                annot_index: 4,
                field_name: "agree".into(),
                props: FieldPropsPatch {
                    multiline: Some(true),
                    ..Default::default()
                },
            }],
        );
        assert!(error.detail.contains("metin"), "{}", error.detail);
    }

    #[test]
    fn name_and_index_must_match() {
        let (_directory, path) = fixture();
        let error = rejected(
            &path,
            vec![FormAuthorOp::Delete {
                page_index: 0,
                annot_index: 0,
                field_name: "notes".into(),
            }],
        );
        assert!(error.detail.contains("'name'"), "{}", error.detail);
        rejected(
            &path,
            vec![FormAuthorOp::Delete {
                page_index: 0,
                annot_index: 99,
                field_name: "x".into(),
            }],
        );
        rejected(
            &path,
            vec![FormAuthorOp::Delete {
                page_index: 4,
                annot_index: 0,
                field_name: "name".into(),
            }],
        );
    }

    #[test]
    fn adding_to_a_document_without_a_form_creates_the_acroform() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plain.pdf");
        FormFixture::new().save(&path, false);
        author(
            &path,
            vec![add(
                NewField::Checkbox,
                Some("Onay"),
                rect(0.1, 0.1, 0.03, 0.02),
            )],
        )
        .unwrap();
        let widgets = read(&path);
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].field_name, "Onay");
        assert_eq!(widgets[0].kind, FormFieldKind::Checkbox);
    }

    #[test]
    fn encrypted_input_and_cyclic_field_trees_are_rejected() {
        let (_directory, path) = fixture();
        let error = author_form_fields_report(
            path.to_string_lossy().to_string(),
            vec![add(NewField::Checkbox, None, rect(0.1, 0.1, 0.03, 0.02))],
            Some("secret".into()),
        )
        .unwrap_err();
        assert!(error.detail.contains("Şifreli"), "{}", error.detail);

        // Point the "name" widget's /Parent at itself.
        let mut document = lopdf::Document::load(&path).unwrap();
        let page_id = *document.get_pages().get(&1).unwrap();
        let annots = document
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        let widget = annots[0].as_reference().unwrap();
        document
            .get_dictionary_mut(widget)
            .unwrap()
            .set("Parent", widget);
        document.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let result = author(
            &path,
            vec![FormAuthorOp::Delete {
                page_index: 0,
                annot_index: 0,
                field_name: "name".into(),
            }],
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn signed_documents_keep_their_signed_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("signed.pdf");
        let mut f = FormFixture::new();
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("name") },
            [50.0, 700.0, 300.0, 720.0],
        );
        f.signature_field(Some(2));
        f.save(&path, true);
        let original = std::fs::read(&path).unwrap();
        author(
            &path,
            vec![add(
                NewField::Text { multiline: false },
                None,
                rect(0.1, 0.3, 0.3, 0.03),
            )],
        )
        .unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(&original));
        assert_eq!(named(&read(&path), "Text1").len(), 1);
    }

    #[test]
    fn request_validation_is_document_independent() {
        let good = rect(0.1, 0.1, 0.2, 0.05);
        assert!(validate_request(&[]).is_err());
        let many: Vec<_> = (0..=MAX_AUTHOR_OPS)
            .map(|_| add(NewField::Checkbox, None, good.clone()))
            .collect();
        assert!(validate_request(&many).is_err());
        for name in ["", "a.b", "tab\there"] {
            assert!(
                validate_request(&[add(NewField::Checkbox, Some(name), good.clone())]).is_err(),
                "{name:?}"
            );
        }
        for export in ["", "Off", "a b", "a/b"] {
            let radio = NewField::Radio {
                group: "g".into(),
                export: export.into(),
            };
            assert!(
                validate_request(&[add(radio, None, good.clone())]).is_err(),
                "{export:?}"
            );
        }
        let empty_options = NewField::Combo {
            options: vec![],
            editable: false,
        };
        assert!(validate_request(&[add(empty_options, None, good.clone())]).is_err());
        assert!(
            validate_request(&[add(NewField::Checkbox, None, rect(0.9, 0.1, 0.3, 0.1))]).is_err()
        );
        let delete = FormAuthorOp::Delete {
            page_index: 0,
            annot_index: 1,
            field_name: "a".into(),
        };
        assert!(validate_request(&[delete.clone(), delete])
            .unwrap_err()
            .contains("birden fazla"));
    }

    #[test]
    fn op_json_shape_matches_the_frontend_contract() {
        let ops: Vec<FormAuthorOp> = serde_json::from_str(
            r#"[
                {"op":"add","pageIndex":0,"normalizedRect":{"x":0.1,"y":0.1,"width":0.2,"height":0.05},
                 "field":{"kind":"radio","group":"g","export":"Yes"},"name":null},
                {"op":"add","pageIndex":0,"normalizedRect":{"x":0.1,"y":0.2,"width":0.2,"height":0.05},
                 "field":{"kind":"combo","options":["a"],"editable":true}},
                {"op":"delete","pageIndex":1,"annotIndex":2,"fieldName":"x"},
                {"op":"move","pageIndex":1,"annotIndex":3,"fieldName":"y","normalizedRect":{"x":0,"y":0,"width":0.5,"height":0.5}},
                {"op":"set_props","pageIndex":0,"annotIndex":4,"fieldName":"z","props":{"required":true,"maxLen":5}}
            ]"#,
        )
        .unwrap();
        assert!(
            matches!(&ops[0], FormAuthorOp::Add { field: NewField::Radio { group, .. }, name: None, .. } if group == "g")
        );
        assert!(matches!(
            &ops[1],
            FormAuthorOp::Add {
                field: NewField::Combo { editable: true, .. },
                name: None,
                ..
            }
        ));
        assert!(matches!(
            &ops[2],
            FormAuthorOp::Delete { annot_index: 2, .. }
        ));
        assert!(matches!(&ops[3], FormAuthorOp::Move { page_index: 1, .. }));
        assert!(
            matches!(&ops[4], FormAuthorOp::SetProps { props, .. } if props.required == Some(true) && props.max_len == Some(5))
        );
    }
}
