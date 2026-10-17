//! Form taslaklarını belgeye yazma (docs/engineering/form-filling-plan.md, Faz 3).
//!
//! Yalnız job worker'da, session snapshot'ının kopyası üzerinde `atomic_write_pdf` içinde
//! çalışır. Değerler PDFium'un form-fill ortamından yazılır (`FORM_*`): PDFium `/V`, `/AS`
//! ve `/AP`'yi birlikte günceller (bkz. plan, Faz 0). pdfium-render'ın `PdfDocument`'i belge
//! ve form handle'larını açmadığı ve `FORM_OnAfterLoadPage`'i çağırmadığı için ham
//! binding'ler kullanılır; pdfium-render'ın form setter'ları kullanılmaz.
//!
//! Doğrulama iki katmanlıdır. Yazmadan önce her edit belgeye karşı denetlenir: alan adı ile
//! annotation indeksi eşleşmeli, alan türü değer türüne uymalı, alan read-only olmamalı
//! (PDFium read-only alana yazmayı engellemiyor). Yazdıktan sonra değer PDFium'dan geri
//! okunur: PDFium MaxLen'i aşan metni sessizce kırptığı için farklı geri gelen değer
//! reddedilir. Kayıttan sonra `atomic_write_pdf`'in doğrulama adımı geçici dosyayı yeniden
//! açıp Faz 1 okuyucusuyla aynı değerleri bekler.
//!
//! Kayıt artımlıdır (`FPDF_INCREMENTAL`): orijinal baytlar korunur ve değişiklikler sona
//! eklenir, böylece mevcut dijital imzalar kendi revizyonları için geçerli kalır.

use super::{FormFieldKind, MAX_FORM_OPTIONS_PER_FIELD, MAX_FORM_TEXT_CHARS};
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::ffi::c_void;
use std::io::Write;
use std::os::raw::{c_int, c_ulong};
use std::path::{Path, PathBuf};

/// Tek istekte en fazla edit sayısı.
pub const MAX_FORM_EDITS: usize = 500;
/// Tek istekteki tüm metin değerlerinin toplam karakter sınırı (job isteği 1 MiB ile sınırlı).
pub const MAX_FORM_EDIT_TEXT_TOTAL_CHARS: usize = 256 * 1024;

const OPERATION: &str = "fill_form_fields";
const FPDF_ANNOT_WIDGET: i32 = 20;
const FORMTYPE_ACRO_FORM: c_int = 1;
const FIELD_FLAG_READ_ONLY: c_int = 1;
const CHOICE_FLAG_MULTI_SELECT: c_int = 1 << 21;
const FPDF_FORMFIELD_CHECKBOX: c_int = 2;
const FPDF_FORMFIELD_RADIOBUTTON: c_int = 3;
const FPDF_FORMFIELD_COMBOBOX: c_int = 4;
const FPDF_FORMFIELD_LISTBOX: c_int = 5;
const FPDF_FORMFIELD_TEXTFIELD: c_int = 6;
const FPDF_INCREMENTAL: FPDF_DWORD = 1;
const FPDF_FULL_REWRITE: FPDF_DWORD = 0;

/// Bir widget'a yazılacak değer. `field_name` ve `annot_index` birlikte doğrulanır:
/// annotation indeksi okumadan bu yana kaydıysa istek reddedilir.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldEdit {
    pub page_index: u32,
    pub annot_index: u32,
    pub field_name: String,
    #[serde(flatten)]
    pub value: FormFieldValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FormFieldValue {
    Text {
        text: String,
    },
    Checkbox {
        checked: bool,
    },
    /// Bu widget'ı grubunda seçili yap.
    Radio,
    /// Seçili seçenek indeksleri (combo için tam bir tane).
    Choice {
        selected: Vec<u32>,
    },
}

/// İsteğin belgeden bağımsız denetimi; session preflight'ı ve job worker aynı kuralı uygular.
pub fn validate_request(edits: &[FormFieldEdit]) -> Result<(), String> {
    if edits.is_empty() {
        return Err("Form yazma isteği boş".to_string());
    }
    if edits.len() > MAX_FORM_EDITS {
        return Err(format!(
            "Form yazma isteği {MAX_FORM_EDITS} edit sınırını aşıyor"
        ));
    }
    let mut names = HashSet::new();
    let mut total_chars = 0usize;
    for edit in edits {
        let name_chars = edit.field_name.chars().count();
        if name_chars == 0 || name_chars > MAX_FORM_TEXT_CHARS {
            return Err("Form alanı adı boş veya çok uzun".to_string());
        }
        if !names.insert(edit.field_name.as_str()) {
            return Err(format!(
                "'{}' alanı istekte birden fazla kez yer alıyor",
                edit.field_name
            ));
        }
        match &edit.value {
            FormFieldValue::Text { text } => {
                let chars = text.chars().count();
                if chars > MAX_FORM_TEXT_CHARS {
                    return Err(format!(
                        "'{}' değeri {MAX_FORM_TEXT_CHARS} karakter sınırını aşıyor",
                        edit.field_name
                    ));
                }
                if text.contains('\0') {
                    return Err(format!(
                        "'{}' değeri NUL karakteri içeriyor",
                        edit.field_name
                    ));
                }
                total_chars += chars;
            }
            FormFieldValue::Choice { selected } => {
                if selected.len() > MAX_FORM_OPTIONS_PER_FIELD {
                    return Err(format!("'{}' seçimi çok uzun", edit.field_name));
                }
                let unique: HashSet<_> = selected.iter().collect();
                if unique.len() != selected.len() {
                    return Err(format!(
                        "'{}' seçimi tekrar eden indeks içeriyor",
                        edit.field_name
                    ));
                }
            }
            FormFieldValue::Checkbox { .. } | FormFieldValue::Radio => {}
        }
    }
    if total_chars > MAX_FORM_EDIT_TEXT_TOTAL_CHARS {
        return Err(format!(
            "Form yazma isteğindeki toplam metin {MAX_FORM_EDIT_TEXT_TOTAL_CHARS} karakter sınırını aşıyor"
        ));
    }
    Ok(())
}

/// `file_path`'teki PDF'in form alanlarını yerinde, atomik olarak doldurur.
pub(crate) fn fill_form_fields_report(
    file_path: String,
    edits: Vec<FormFieldEdit>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    validate_request(&edits).map_err(|detail| {
        DocumentIoError::new(
            OPERATION,
            crate::document_io::DocumentIoStage::Preflight,
            CommitState::Prepared,
            detail,
        )
    })?;
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    atomic_write_pdf(
        OPERATION,
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| {
            fill_and_save(
                pdfium.bindings(),
                &source,
                password.as_deref(),
                &edits,
                temp,
            )
        },
        |document, expected| verify_saved(document, expected),
    )
}

/// Yazılan bir widget'ın kayıttan sonra okunması beklenen durumu.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExpectedWidget {
    page_index: u32,
    annot_index: u32,
    field_name: String,
    value: FormFieldValue,
}

fn fill_and_save(
    bindings: &dyn PdfiumLibraryBindings,
    source: &Path,
    password: Option<&str>,
    edits: &[FormFieldEdit],
    temp: &Path,
) -> Result<Vec<ExpectedWidget>, DocumentIoFailure> {
    let source = source
        .to_str()
        .ok_or_else(|| DocumentIoFailure::preflight("PDF yolu UTF-8 değil"))?;
    let document = RawDocument::open(bindings, source, password)?;
    if bindings.FPDF_GetSecurityHandlerRevision(document.handle) != -1 {
        return Err(DocumentIoFailure::preflight(
            "Şifreli PDF mutasyonları henüz desteklenmiyor",
        ));
    }
    if bindings.FPDF_GetFormType(document.handle) != FORMTYPE_ACRO_FORM {
        return Err(DocumentIoFailure::preflight(
            "Belge AcroForm içermiyor (XFA formları desteklenmiyor)",
        ));
    }
    let form = FormEnvironment::init(bindings, &document)?;
    let page_count = u32::try_from(bindings.FPDF_GetPageCount(document.handle)).unwrap_or(0);

    let mut by_page: BTreeMap<u32, Vec<&FormFieldEdit>> = BTreeMap::new();
    for edit in edits {
        if edit.page_index >= page_count {
            return Err(DocumentIoFailure::preflight(format!(
                "'{}' için sayfa {} yok (sayfa sayısı {page_count})",
                edit.field_name, edit.page_index
            )));
        }
        by_page.entry(edit.page_index).or_default().push(edit);
    }

    let mut expected = Vec::with_capacity(edits.len());
    for (page_index, page_edits) in by_page {
        let page = RawPage::load(bindings, &document, &form, page_index)?;
        for edit in page_edits {
            apply_edit(bindings, &form, &page, edit)?;
            expected.push(ExpectedWidget {
                page_index,
                annot_index: edit.annot_index,
                field_name: edit.field_name.clone(),
                value: edit.value.clone(),
            });
        }
    }
    save_preserving_signatures(bindings, &document, source, temp)?;
    Ok(expected)
}

/// Saves as an incremental update: the original bytes stay byte-for-byte at the start of
/// the file and the changes are appended, so the byte ranges existing signatures cover are
/// untouched and the signatures stay valid for their revision. Only an unsigned document
/// falls back to a full rewrite when PDFium cannot save incrementally.
fn save_preserving_signatures(
    bindings: &dyn PdfiumLibraryBindings,
    document: &RawDocument<'_>,
    source: &str,
    temp: &Path,
) -> Result<(), DocumentIoFailure> {
    let signature_count = bindings.FPDF_GetSignatureCount(document.handle);
    if let Err(error) = save_copy(bindings, document, temp, FPDF_INCREMENTAL) {
        if signature_count > 0 {
            return Err(DocumentIoFailure::write(format!(
                "İmzalı belge artımlı (incremental) kaydedilemedi; imzaları korumak için tam yeniden yazıma geçilmedi: {}",
                error.detail
            )));
        }
        return save_copy(bindings, document, temp, FPDF_FULL_REWRITE);
    }
    if signature_count > 0 {
        let original = std::fs::read(source)
            .map_err(|error| DocumentIoFailure::write(format!("Kaynak PDF okunamadı: {error}")))?;
        let saved = std::fs::read(temp)
            .map_err(|error| DocumentIoFailure::write(format!("Temp PDF okunamadı: {error}")))?;
        if !saved.starts_with(&original) {
            return Err(DocumentIoFailure::invariant(
                "Artımlı kayıt imzalı baytları korumadı",
            ));
        }
    }
    Ok(())
}

fn apply_edit(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    edit: &FormFieldEdit,
) -> Result<(), DocumentIoFailure> {
    let annot_index = c_int::try_from(edit.annot_index)
        .map_err(|_| DocumentIoFailure::preflight("Annotation indeksi sınır dışı"))?;
    if annot_index >= bindings.FPDFPage_GetAnnotCount(page.handle) {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' için annotation {} sayfada yok",
            edit.field_name, edit.annot_index
        )));
    }
    let annot = RawAnnotation::get(bindings, page, annot_index)?;
    if bindings.FPDFAnnot_GetSubtype(annot.handle) != FPDF_ANNOT_WIDGET {
        return Err(DocumentIoFailure::preflight(format!(
            "Annotation {} bir form widget'ı değil",
            edit.annot_index
        )));
    }
    let name = wide_string(|buffer, length| {
        bindings.FPDFAnnot_GetFormFieldName(form.handle, annot.handle, buffer, length)
    });
    if name != edit.field_name {
        return Err(DocumentIoFailure::preflight(format!(
            "Annotation {} '{}' alanına ait, istek '{}' bekliyordu; belge okumadan bu yana değişmiş",
            edit.annot_index, name, edit.field_name
        )));
    }
    let field_type = bindings.FPDFAnnot_GetFormFieldType(form.handle, annot.handle);
    let expected_kind = match &edit.value {
        FormFieldValue::Text { .. } => field_type == FPDF_FORMFIELD_TEXTFIELD,
        FormFieldValue::Checkbox { .. } => field_type == FPDF_FORMFIELD_CHECKBOX,
        FormFieldValue::Radio => field_type == FPDF_FORMFIELD_RADIOBUTTON,
        FormFieldValue::Choice { .. } => {
            field_type == FPDF_FORMFIELD_COMBOBOX || field_type == FPDF_FORMFIELD_LISTBOX
        }
    };
    if !expected_kind {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' alanının türü istenen değerle uyuşmuyor",
            edit.field_name
        )));
    }
    let flags = bindings.FPDFAnnot_GetFormFieldFlags(form.handle, annot.handle);
    if flags & FIELD_FLAG_READ_ONLY != 0 {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' alanı read-only",
            edit.field_name
        )));
    }

    match &edit.value {
        FormFieldValue::Text { text } => write_text(bindings, form, page, &annot, edit, text),
        FormFieldValue::Checkbox { checked } => {
            if is_checked(bindings, form, &annot) != *checked {
                press_space(bindings, form, page, &annot)?;
            }
            if is_checked(bindings, form, &annot) != *checked {
                return Err(DocumentIoFailure::write(format!(
                    "'{}' checkbox durumu değiştirilemedi",
                    edit.field_name
                )));
            }
            Ok(())
        }
        FormFieldValue::Radio => {
            if !is_checked(bindings, form, &annot) {
                press_space(bindings, form, page, &annot)?;
            }
            if !is_checked(bindings, form, &annot) {
                return Err(DocumentIoFailure::write(format!(
                    "'{}' radio düğmesi seçilemedi",
                    edit.field_name
                )));
            }
            Ok(())
        }
        FormFieldValue::Choice { selected } => write_choice(
            bindings, form, page, &annot, edit, selected, field_type, flags,
        ),
    }
}

fn write_text(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    annot: &RawAnnotation<'_>,
    edit: &FormFieldEdit,
    text: &str,
) -> Result<(), DocumentIoFailure> {
    focus(bindings, form, annot, &edit.field_name)?;
    bindings.FORM_SelectAllText(form.handle, page.handle);
    let wide = wide(text);
    bindings.FORM_ReplaceSelection(form.handle, page.handle, wide.as_ptr());
    bindings.FORM_ForceToKillFocus(form.handle);

    let written = wide_string(|buffer, length| {
        bindings.FPDFAnnot_GetFormFieldValue(form.handle, annot.handle, buffer, length)
    });
    if normalize_newlines(&written) == normalize_newlines(text) {
        return Ok(());
    }
    let requested = normalize_newlines(text);
    let stored = normalize_newlines(&written);
    if requested.starts_with(&stored) {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' alanı en fazla {} karakter kabul ediyor (MaxLen)",
            edit.field_name,
            stored.chars().count()
        )));
    }
    Err(DocumentIoFailure::write(format!(
        "'{}' alanına yazılan değer PDFium'dan farklı geri okundu",
        edit.field_name
    )))
}

#[allow(clippy::too_many_arguments)]
fn write_choice(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    annot: &RawAnnotation<'_>,
    edit: &FormFieldEdit,
    selected: &[u32],
    field_type: c_int,
    flags: c_int,
) -> Result<(), DocumentIoFailure> {
    let option_count = bindings
        .FPDFAnnot_GetOptionCount(form.handle, annot.handle)
        .max(0);
    if selected
        .iter()
        .any(|index| i64::from(*index) >= i64::from(option_count))
    {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' seçimi seçenek sayısının ({option_count}) dışında",
            edit.field_name
        )));
    }
    let multi = field_type == FPDF_FORMFIELD_LISTBOX && flags & CHOICE_FLAG_MULTI_SELECT != 0;
    if field_type == FPDF_FORMFIELD_COMBOBOX && selected.len() != 1 {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' combo kutusunda tam bir seçenek seçilmeli",
            edit.field_name
        )));
    }
    if !multi && selected.len() > 1 {
        return Err(DocumentIoFailure::preflight(format!(
            "'{}' birden fazla seçime izin vermiyor",
            edit.field_name
        )));
    }

    focus(bindings, form, annot, &edit.field_name)?;
    for index in 0..option_count {
        let want = selected
            .iter()
            .any(|value| i64::from(*value) == i64::from(index));
        let has =
            bindings.is_true(bindings.FPDFAnnot_IsOptionSelected(form.handle, annot.handle, index));
        if want != has {
            bindings.FORM_SetIndexSelected(
                form.handle,
                page.handle,
                index,
                if want { 1 } else { 0 },
            );
        }
    }
    bindings.FORM_ForceToKillFocus(form.handle);

    let actual: Vec<u32> = (0..option_count)
        .filter(|index| {
            bindings.is_true(bindings.FPDFAnnot_IsOptionSelected(form.handle, annot.handle, *index))
        })
        .map(|index| index as u32)
        .collect();
    let mut wanted = selected.to_vec();
    wanted.sort_unstable();
    if actual != wanted {
        return Err(DocumentIoFailure::write(format!(
            "'{}' seçimi uygulanamadı",
            edit.field_name
        )));
    }
    Ok(())
}

fn focus(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    annot: &RawAnnotation<'_>,
    field_name: &str,
) -> Result<(), DocumentIoFailure> {
    if bindings.is_true(bindings.FORM_SetFocusedAnnot(form.handle, annot.handle)) {
        Ok(())
    } else {
        Err(DocumentIoFailure::write(format!(
            "'{field_name}' alanına odaklanılamadı"
        )))
    }
}

fn press_space(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    annot: &RawAnnotation<'_>,
) -> Result<(), DocumentIoFailure> {
    focus(bindings, form, annot, "button")?;
    bindings.FORM_OnChar(form.handle, page.handle, c_int::from(b' '), 0);
    bindings.FORM_ForceToKillFocus(form.handle);
    Ok(())
}

fn is_checked(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    annot: &RawAnnotation<'_>,
) -> bool {
    bindings.is_true(bindings.FPDFAnnot_IsChecked(form.handle, annot.handle))
}

/// PDFium çok satırlı alanda satır sonunu `\r\n` olarak saklar.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// PDFium'un "uzunluğu sor, sonra doldur" UTF-16LE string API'leri için.
fn wide_string(fetch: impl Fn(*mut FPDF_WCHAR, c_ulong) -> c_ulong) -> String {
    let length = fetch(std::ptr::null_mut(), 0);
    if length <= 2 {
        return String::new();
    }
    let mut buffer = vec![0u16; (length as usize).div_ceil(2)];
    fetch(buffer.as_mut_ptr(), length);
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn verify_saved(
    document: &PdfDocument<'_>,
    expected: &[ExpectedWidget],
) -> Result<(), DocumentIoFailure> {
    let mut pages: BTreeMap<u32, Vec<super::FormWidgetData>> = BTreeMap::new();
    for item in expected {
        let widgets = match pages.entry(item.page_index) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(
                super::read_page_form_fields(document, item.page_index).map_err(|error| {
                    DocumentIoFailure::invariant(format!("Kaydedilen form okunamadı: {error:?}"))
                })?,
            ),
        };
        let widget = widgets
            .iter()
            .find(|widget| widget.annot_index == item.annot_index)
            .ok_or_else(|| {
                DocumentIoFailure::invariant(format!(
                    "Kaydedilen belgede '{}' widget'ı yok",
                    item.field_name
                ))
            })?;
        if widget.field_name != item.field_name {
            return Err(DocumentIoFailure::invariant(format!(
                "Kaydedilen widget '{}' yerine '{}' adını taşıyor",
                item.field_name, widget.field_name
            )));
        }
        let matches = match &item.value {
            FormFieldValue::Text { text } => {
                widget.kind == FormFieldKind::Text
                    && normalize_newlines(widget.value.as_deref().unwrap_or(""))
                        == normalize_newlines(text)
            }
            FormFieldValue::Checkbox { checked } => widget.checked == Some(*checked),
            FormFieldValue::Radio => widget.checked == Some(true),
            FormFieldValue::Choice { selected } => {
                let mut wanted = selected.clone();
                wanted.sort_unstable();
                widget
                    .options
                    .iter()
                    .enumerate()
                    .filter(|(_, option)| option.selected)
                    .map(|(index, _)| index as u32)
                    .collect::<Vec<_>>()
                    == wanted
            }
        };
        if !matches {
            return Err(DocumentIoFailure::invariant(format!(
                "Kaydedilen belgede '{}' değeri doğrulanamadı",
                item.field_name
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Ham PDFium kaynakları. Bırakma sırası önemlidir: annotation → sayfa → form ortamı →
// belge. Sarmalayıcılar ödünç alma ilişkisiyle bu sırayı derleyiciye zorlatır.
// ---------------------------------------------------------------------------

struct RawDocument<'b> {
    bindings: &'b dyn PdfiumLibraryBindings,
    handle: FPDF_DOCUMENT,
}

impl<'b> RawDocument<'b> {
    fn open(
        bindings: &'b dyn PdfiumLibraryBindings,
        path: &str,
        password: Option<&str>,
    ) -> Result<Self, DocumentIoFailure> {
        let handle = bindings.FPDF_LoadDocument(path, password);
        if handle.is_null() {
            return Err(DocumentIoFailure::preflight(
                "PDF form yazımı için açılamadı",
            ));
        }
        Ok(Self { bindings, handle })
    }
}

impl Drop for RawDocument<'_> {
    fn drop(&mut self) {
        self.bindings.FPDF_CloseDocument(self.handle);
    }
}

struct FormEnvironment<'d> {
    bindings: &'d dyn PdfiumLibraryBindings,
    handle: FPDF_FORMHANDLE,
    // PDFium bu yapının adresini ortam kapanana kadar tutar.
    _info: Box<FPDF_FORMFILLINFO>,
    _document: std::marker::PhantomData<&'d RawDocument<'d>>,
}

impl<'d> FormEnvironment<'d> {
    fn init(
        bindings: &'d dyn PdfiumLibraryBindings,
        document: &'d RawDocument<'_>,
    ) -> Result<Self, DocumentIoFailure> {
        let mut info = Box::new(form_fill_info());
        let handle = bindings.FPDFDOC_InitFormFillEnvironment(document.handle, &mut *info);
        if handle.is_null() {
            return Err(DocumentIoFailure::preflight("PDF form ortamı kurulamadı"));
        }
        Ok(Self {
            bindings,
            handle,
            _info: info,
            _document: std::marker::PhantomData,
        })
    }
}

impl Drop for FormEnvironment<'_> {
    fn drop(&mut self) {
        self.bindings.FPDFDOC_ExitFormFillEnvironment(self.handle);
    }
}

struct RawPage<'f> {
    bindings: &'f dyn PdfiumLibraryBindings,
    form: FPDF_FORMHANDLE,
    handle: FPDF_PAGE,
    _form: std::marker::PhantomData<&'f FormEnvironment<'f>>,
}

impl<'f> RawPage<'f> {
    fn load(
        bindings: &'f dyn PdfiumLibraryBindings,
        document: &'f RawDocument<'_>,
        form: &'f FormEnvironment<'_>,
        page_index: u32,
    ) -> Result<Self, DocumentIoFailure> {
        let index = c_int::try_from(page_index)
            .map_err(|_| DocumentIoFailure::preflight("Sayfa indeksi sınır dışı"))?;
        let handle = bindings.FPDF_LoadPage(document.handle, index);
        if handle.is_null() {
            return Err(DocumentIoFailure::preflight(format!(
                "Sayfa {page_index} yüklenemedi"
            )));
        }
        // Sayfanın form görünümünü kurar; `/Fields` dışı widget'lar da burada alana döner.
        bindings.FORM_OnAfterLoadPage(handle, form.handle);
        Ok(Self {
            bindings,
            form: form.handle,
            handle,
            _form: std::marker::PhantomData,
        })
    }
}

impl Drop for RawPage<'_> {
    fn drop(&mut self) {
        self.bindings.FORM_OnBeforeClosePage(self.handle, self.form);
        self.bindings.FPDF_ClosePage(self.handle);
    }
}

struct RawAnnotation<'p> {
    bindings: &'p dyn PdfiumLibraryBindings,
    handle: FPDF_ANNOTATION,
    _page: std::marker::PhantomData<&'p RawPage<'p>>,
}

impl<'p> RawAnnotation<'p> {
    fn get(
        bindings: &'p dyn PdfiumLibraryBindings,
        page: &'p RawPage<'_>,
        index: c_int,
    ) -> Result<Self, DocumentIoFailure> {
        let handle = bindings.FPDFPage_GetAnnot(page.handle, index);
        if handle.is_null() {
            return Err(DocumentIoFailure::preflight(format!(
                "Annotation {index} okunamadı"
            )));
        }
        Ok(Self {
            bindings,
            handle,
            _page: std::marker::PhantomData,
        })
    }
}

impl Drop for RawAnnotation<'_> {
    fn drop(&mut self) {
        self.bindings.FPDFPage_CloseAnnot(self.handle);
    }
}

fn form_fill_info() -> FPDF_FORMFILLINFO {
    FPDF_FORMFILLINFO {
        version: 2,
        Release: None,
        FFI_Invalidate: None,
        FFI_OutputSelectedRect: None,
        FFI_SetCursor: None,
        FFI_SetTimer: None,
        FFI_KillTimer: None,
        FFI_GetLocalTime: None,
        FFI_OnChange: None,
        FFI_GetPage: None,
        FFI_GetCurrentPage: None,
        FFI_GetRotation: None,
        FFI_ExecuteNamedAction: None,
        FFI_SetTextFieldFocus: None,
        FFI_DoURIAction: None,
        FFI_DoGoToAction: None,
        m_pJsPlatform: std::ptr::null_mut(),
        xfa_disabled: 1,
        FFI_DisplayCaret: None,
        FFI_GetCurrentPageIndex: None,
        FFI_SetCurrentPage: None,
        FFI_GotoURL: None,
        FFI_GetPageViewRect: None,
        FFI_PageEvent: None,
        FFI_PopupMenu: None,
        FFI_OpenFile: None,
        FFI_EmailTo: None,
        FFI_UploadTo: None,
        FFI_GetPlatform: None,
        FFI_GetLanguage: None,
        FFI_DownloadFromURL: None,
        FFI_PostRequestURL: None,
        FFI_PutRequestURL: None,
        FFI_OnFocusChange: None,
        FFI_DoURIActionWithKeyboardModifier: None,
    }
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

fn save_copy(
    bindings: &dyn PdfiumLibraryBindings,
    document: &RawDocument<'_>,
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
    let saved = bindings.FPDF_SaveAsCopy(document.handle, &mut writer.base, flags);
    if !bindings.is_true(saved) || writer.failed {
        return Err(DocumentIoFailure::write("Temp PDF kaydedilemedi"));
    }
    writer.file.sync_all().map_err(|error| {
        DocumentIoFailure::write(format!("Temp PDF senkronize edilemedi: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::document_io::DocumentIoStage;
    use lopdf::dictionary;

    // standard_fixture annotation order: name 0, notes 1, code 2 (comb, MaxLen 6), locked 3
    // (read-only), agree 4, newsletter 5, color Red 6 / Blue 7, country 8, langs 9, secret 10.

    fn edit(
        page_index: u32,
        annot_index: u32,
        field_name: &str,
        value: FormFieldValue,
    ) -> FormFieldEdit {
        FormFieldEdit {
            page_index,
            annot_index,
            field_name: field_name.to_string(),
            value,
        }
    }

    fn text(value: &str) -> FormFieldValue {
        FormFieldValue::Text {
            text: value.to_string(),
        }
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("form.pdf");
        standard_fixture(&path);
        (directory, path)
    }

    fn fill(path: &Path, edits: Vec<FormFieldEdit>) -> Result<AtomicWriteReport, DocumentIoError> {
        fill_form_fields_report(path.to_string_lossy().to_string(), edits, None)
    }

    fn read(path: &Path) -> Vec<super::super::FormWidgetData> {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            super::super::read_page_form_fields(&document, 0).map_err(|error| format!("{error:?}"))
        })
        .unwrap()
    }

    fn widget(
        widgets: &[super::super::FormWidgetData],
        annot_index: u32,
    ) -> &super::super::FormWidgetData {
        widgets
            .iter()
            .find(|w| w.annot_index == annot_index)
            .unwrap()
    }

    fn rejected(path: &Path, edits: Vec<FormFieldEdit>) -> DocumentIoError {
        let before = std::fs::read(path).unwrap();
        let error = fill(path, edits).unwrap_err();
        assert_eq!(
            std::fs::read(path).unwrap(),
            before,
            "rejected fill changed the file"
        );
        error
    }

    #[test]
    fn writes_every_kind_and_the_saved_file_reads_back() {
        let (_directory, path) = fixture();
        fill(
            &path,
            vec![
                edit(0, 0, "name", text("Çağrı Yılmaz")),
                edit(0, 1, "notes", text("Satır bir\nSatır iki")),
                edit(0, 2, "code", text("AB12")),
                edit(0, 4, "agree", FormFieldValue::Checkbox { checked: false }),
                edit(
                    0,
                    5,
                    "newsletter",
                    FormFieldValue::Checkbox { checked: true },
                ),
                edit(0, 6, "color", FormFieldValue::Radio),
                edit(
                    0,
                    8,
                    "country",
                    FormFieldValue::Choice { selected: vec![0] },
                ),
                edit(0, 9, "langs", FormFieldValue::Choice { selected: vec![1] }),
            ],
        )
        .unwrap();

        let widgets = read(&path);
        assert_eq!(widget(&widgets, 0).value.as_deref(), Some("Çağrı Yılmaz"));
        assert_eq!(
            normalize_newlines(widget(&widgets, 1).value.as_deref().unwrap()),
            "Satır bir\nSatır iki"
        );
        assert_eq!(widget(&widgets, 2).value.as_deref(), Some("AB12"));
        assert_eq!(widget(&widgets, 3).value.as_deref(), Some("fixed"));
        assert_eq!(widget(&widgets, 4).checked, Some(false));
        assert_eq!(widget(&widgets, 5).checked, Some(true));
        assert_eq!(widget(&widgets, 6).checked, Some(true));
        assert_eq!(widget(&widgets, 7).checked, Some(false));
        assert_eq!(
            widget(&widgets, 8)
                .options
                .iter()
                .map(|o| o.selected)
                .collect::<Vec<_>>(),
            vec![true, false]
        );
        assert_eq!(
            widget(&widgets, 9)
                .options
                .iter()
                .map(|o| o.selected)
                .collect::<Vec<_>>(),
            vec![false, true, false]
        );
    }

    #[test]
    fn written_text_gets_a_regenerated_normal_appearance() {
        // The fixture's text widgets have no /AP; PDFium's form-fill path must create one
        // that draws the new value (pdfium-render's set_value only writes /V).
        let (_directory, path) = fixture();
        fill(&path, vec![edit(0, 0, "name", text("Lovelace"))]).unwrap();
        let document = lopdf::Document::load(&path).unwrap();
        let page_id = *document.get_pages().get(&1).unwrap();
        let page = document.get_dictionary(page_id).unwrap();
        let annots = document
            .dereference(page.get(b"Annots").unwrap())
            .unwrap()
            .1
            .as_array()
            .unwrap();
        let widget = document
            .dereference(&annots[0])
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        let ap = document
            .dereference(widget.get(b"AP").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        let normal = document.dereference(ap.get(b"N").unwrap()).unwrap().1;
        let lopdf::Object::Stream(stream) = normal else {
            panic!("/AP /N is not a stream: {normal:?}");
        };
        let content = stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone());
        assert!(
            String::from_utf8_lossy(&content).contains("(Lovelace) Tj"),
            "{}",
            String::from_utf8_lossy(&content)
        );
    }

    #[test]
    fn unsigned_fill_appends_an_incremental_update() {
        let (_directory, path) = fixture();
        let original = std::fs::read(&path).unwrap();
        fill(&path, vec![edit(0, 0, "name", text("Grace"))]).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.len() > original.len());
        assert!(
            saved.starts_with(&original),
            "original bytes were rewritten"
        );
    }

    #[test]
    fn signed_fill_keeps_the_signed_bytes_and_the_signature() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("signed.pdf");
        let mut f = FormFixture::new();
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("name"), "V" => string("Ada") },
            [50.0, 700.0, 300.0, 720.0],
        );
        f.signature_field(Some(2));
        f.save(&path, true);
        let original = std::fs::read(&path).unwrap();
        let signatures = |path: &Path| {
            crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
                let document = pdfium
                    .load_pdf_from_file(path, None)
                    .map_err(|error| error.to_string())?;
                Ok(document
                    .signatures()
                    .iter()
                    .map(|signature| signature.bytes())
                    .collect::<Vec<_>>())
            })
            .unwrap()
        };
        let before = signatures(&path);
        assert_eq!(before.len(), 1);

        fill(&path, vec![edit(0, 0, "name", text("Grace"))]).unwrap();

        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&original), "signed bytes were rewritten");
        assert_eq!(signatures(&path), before);
        assert_eq!(widget(&read(&path), 0).value.as_deref(), Some("Grace"));
    }

    #[test]
    fn read_only_field_is_rejected_without_touching_the_file() {
        let (_directory, path) = fixture();
        let error = rejected(&path, vec![edit(0, 3, "locked", text("hacked"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.detail.contains("read-only"), "{}", error.detail);
    }

    #[test]
    fn name_and_index_must_match() {
        let (_directory, path) = fixture();
        let error = rejected(&path, vec![edit(0, 0, "notes", text("x"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(error.detail.contains("'name'"), "{}", error.detail);
        let error = rejected(&path, vec![edit(0, 40, "name", text("x"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
        let error = rejected(&path, vec![edit(3, 0, "name", text("x"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
    }

    #[test]
    fn text_over_max_len_is_rejected_instead_of_truncated() {
        let (_directory, path) = fixture();
        let error = rejected(&path, vec![edit(0, 2, "code", text("ABCDEFGH"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(
            error.detail.contains("MaxLen") && error.detail.contains('6'),
            "{}",
            error.detail
        );
    }

    #[test]
    fn value_kind_must_match_the_field() {
        let (_directory, path) = fixture();
        let error = rejected(
            &path,
            vec![edit(
                0,
                0,
                "name",
                FormFieldValue::Checkbox { checked: true },
            )],
        );
        assert!(error.detail.contains("türü"), "{}", error.detail);
        let error = rejected(
            &path,
            vec![edit(
                0,
                8,
                "country",
                FormFieldValue::Choice {
                    selected: vec![0, 1],
                },
            )],
        );
        assert!(error.detail.contains("combo"), "{}", error.detail);
        let error = rejected(
            &path,
            vec![edit(
                0,
                9,
                "langs",
                FormFieldValue::Choice { selected: vec![7] },
            )],
        );
        assert!(error.detail.contains("seçenek"), "{}", error.detail);
    }

    #[test]
    fn one_bad_edit_rejects_the_whole_batch() {
        let (_directory, path) = fixture();
        rejected(
            &path,
            vec![
                edit(0, 0, "name", text("ok")),
                edit(0, 3, "locked", text("no")),
            ],
        );
    }

    #[test]
    fn writes_widgets_outside_acroform_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("orphan.pdf");
        let mut f = FormFixture::new();
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("listed"), "V" => string("a") },
            [50.0, 700.0, 300.0, 720.0],
        );
        f.orphan_widget(
            dictionary! { "FT" => "Tx", "T" => string("orphan"), "V" => string("b") },
            [50.0, 650.0, 300.0, 670.0],
        );
        f.save(&path, true);
        fill(&path, vec![edit(0, 1, "orphan", text("written"))]).unwrap();
        assert_eq!(widget(&read(&path), 1).value.as_deref(), Some("written"));
    }

    #[test]
    fn request_validation_is_document_independent() {
        assert!(validate_request(&[]).is_err());
        let duplicate = vec![edit(0, 0, "a", text("1")), edit(0, 1, "a", text("2"))];
        assert!(validate_request(&duplicate)
            .unwrap_err()
            .contains("birden fazla"));
        assert!(validate_request(&[edit(0, 0, "a", text("x\0y"))]).is_err());
        assert!(validate_request(&[edit(0, 0, "", text("x"))]).is_err());
        let long = "x".repeat(MAX_FORM_TEXT_CHARS + 1);
        assert!(validate_request(&[edit(0, 0, "a", text(&long))]).is_err());
        let repeated = FormFieldValue::Choice {
            selected: vec![1, 1],
        };
        assert!(validate_request(&[edit(0, 0, "a", repeated)]).is_err());
        let many: Vec<_> = (0..=MAX_FORM_EDITS as u32)
            .map(|i| edit(0, i, &format!("f{i}"), text("x")))
            .collect();
        assert!(validate_request(&many).is_err());
        let chunk = "x".repeat(MAX_FORM_TEXT_CHARS);
        let big: Vec<_> = (0..(MAX_FORM_EDIT_TEXT_TOTAL_CHARS / MAX_FORM_TEXT_CHARS + 1) as u32)
            .map(|i| edit(0, i, &format!("f{i}"), text(&chunk)))
            .collect();
        assert!(validate_request(&big).unwrap_err().contains("toplam"));
    }

    #[test]
    fn edit_json_shape_matches_the_frontend_contract() {
        let parsed: Vec<FormFieldEdit> = serde_json::from_str(
            r#"[
                {"pageIndex":0,"annotIndex":1,"fieldName":"a","kind":"text","text":"x"},
                {"pageIndex":0,"annotIndex":2,"fieldName":"b","kind":"checkbox","checked":true},
                {"pageIndex":1,"annotIndex":3,"fieldName":"c","kind":"radio"},
                {"pageIndex":1,"annotIndex":4,"fieldName":"d","kind":"choice","selected":[0,2]}
            ]"#,
        )
        .unwrap();
        assert_eq!(parsed[0].value, text("x"));
        assert_eq!(parsed[1].value, FormFieldValue::Checkbox { checked: true });
        assert_eq!(parsed[2].value, FormFieldValue::Radio);
        assert_eq!(
            parsed[3].value,
            FormFieldValue::Choice {
                selected: vec![0, 2]
            }
        );
        assert_eq!(parsed[3].page_index, 1);
    }
}
