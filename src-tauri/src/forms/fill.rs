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
//!
//! Acrobat'ın Format, Validate ve Calculate sekmelerinin yazdığı AF betikleri ([`super::af`])
//! JavaScript motoru olmadan çalışır. Betikler PDFium'dan (`FPDFAnnot_GetFormAdditionalAction
//! JavaScript`) okunur:
//! - Metin edit'i önce Keystroke'tan geçer (biçime uymayan değer reddedilir, sayılar
//!   normalleşir), sonra Validate'ten (aralık dışı değer reddedilir).
//! - Biçimli alanın görünüm akışı biçimli metni çizer (`$1,234.50`), `/V` ham değeri tutar
//!   (`1234.5`); Acrobat da böyle saklar. Bu yalnız widget'ı alanın kendisi olan (`/T` taşıyan)
//!   widget'larda yapılır; diğerlerinde görünüm ham değeri çizer.
//! - Edit'lerden sonra hesaplanan metin alanları yeniden hesaplanır ve değişenler yazılır.
//!   Sıra `/CO` değil bağımlılık sırasıdır: her alan okuduğu hesaplanan alanlardan sonra
//!   çalışır (döngüdekiler belge sırasıyla bir kez). Acrobat `/CO`'yu izler; `/CO` yanlış
//!   sıralıysa Acrobat eski değeri kullanır, biz kullanmayız.
//!
//! AF dışındaki özel betikler çalıştırılmaz; o alanlar biçimsiz/hesaplanmamış kalır.

use super::af;
use super::{FormFieldKind, MAX_FORM_OPTIONS_PER_FIELD, MAX_FORM_TEXT_CHARS};
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
};
use crate::incremental_save::{self, RawDocument};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
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
const FPDF_ANNOT_AACTION_KEY_STROKE: c_int = 12;
const FPDF_ANNOT_AACTION_FORMAT: c_int = 13;
const FPDF_ANNOT_AACTION_VALIDATE: c_int = 14;
const FPDF_ANNOT_AACTION_CALCULATE: c_int = 15;

/// Bir alan betiğinin okunacak en fazla uzunluğu (UTF-16LE bayt, sonlandırıcı dahil). Daha
/// uzun betik okunmaz; alan o betik yokmuş gibi davranır.
const MAX_FIELD_SCRIPT_BYTES: c_ulong = 64 * 1024;
/// Yeniden hesaplama için taranacak en fazla widget sayısı.
const MAX_CALCULATION_WIDGETS: usize = 100_000;
/// En fazla hesaplanan alan sayısı; sıralama bu sayının karesiyle büyür.
const MAX_CALCULATED_FIELDS: usize = 2_000;

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
    let document = RawDocument::open(bindings, source, password)
        .ok_or_else(|| DocumentIoFailure::preflight("PDF form yazımı için açılamadı"))?;
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
    // Edit'lerden sonra alanların `/V` değerleri; hesaplamalar bunları okur.
    let mut edited = BTreeMap::new();
    for (page_index, page_edits) in by_page {
        let page = RawPage::load(bindings, &document, &form, page_index)?;
        for edit in page_edits {
            let (stored, field_value) = apply_edit(bindings, &form, &page, edit)?;
            edited.insert(edit.field_name.clone(), field_value);
            expected.push(ExpectedWidget {
                page_index,
                annot_index: edit.annot_index,
                field_name: edit.field_name.clone(),
                value: stored,
            });
        }
    }

    let fields = scan_fields(bindings, source, password)?;
    let calculated = recalculate(&fields, &edited)?;
    let mut writes_by_page: BTreeMap<u32, Vec<&CalculatedValue>> = BTreeMap::new();
    for write in &calculated {
        writes_by_page
            .entry(write.page_index)
            .or_default()
            .push(write);
    }
    for (page_index, writes) in writes_by_page {
        let page = RawPage::load(bindings, &document, &form, page_index)?;
        for write in writes {
            let annot_index = c_int::try_from(write.annot_index)
                .map_err(|_| DocumentIoFailure::write("Annotation indeksi sınır dışı"))?;
            let annot = RawAnnotation::get(bindings, &page, annot_index)?;
            if field_name(bindings, &form, &annot) != write.field_name {
                return Err(DocumentIoFailure::write(format!(
                    "Hesaplanan '{}' alanı belgede yerinde değil",
                    write.field_name
                )));
            }
            write_text(
                bindings,
                &form,
                &page,
                &annot,
                &write.field_name,
                &write.value,
                &write.format,
            )?;
            // Kullanıcı hesaplanan alanı da düzenlediyse hesaplanan değer kazanır.
            expected.retain(|item| item.field_name != write.field_name);
            expected.push(ExpectedWidget {
                page_index,
                annot_index: write.annot_index,
                field_name: write.field_name.clone(),
                value: FormFieldValue::Text {
                    text: write.value.clone(),
                },
            });
        }
    }

    let mode = incremental_save::save_mutation(&document, Path::new(source), temp)?;
    tracing::info!(
        operation = OPERATION,
        write_mode = ?mode,
        calculated_fields = calculated.len(),
        "Form değerleri yazıldı"
    );
    Ok(expected)
}

/// Bir edit'i yazar. Kayıttan sonra beklenen değeri (metin Keystroke'la normalleşmiş olabilir)
/// ve alanın hesaplamaların okuyacağı `/V` değerini döndürür.
fn apply_edit(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    edit: &FormFieldEdit,
) -> Result<(FormFieldValue, String), DocumentIoFailure> {
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
    let name = field_name(bindings, form, &annot);
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

    let stored = match &edit.value {
        FormFieldValue::Text { text } => {
            let actions = field_actions(bindings, form, &annot);
            let raw = af::keystroke(&actions.format, &edit.field_name, text)
                .map_err(DocumentIoFailure::preflight)?;
            af::validate(&actions.validate, &raw).map_err(DocumentIoFailure::preflight)?;
            write_text(
                bindings,
                form,
                page,
                &annot,
                &edit.field_name,
                &raw,
                &actions.format,
            )?;
            FormFieldValue::Text { text: raw }
        }
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
            edit.value.clone()
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
            edit.value.clone()
        }
        FormFieldValue::Choice { selected } => {
            write_choice(
                bindings, form, page, &annot, edit, selected, field_type, flags,
            )?;
            edit.value.clone()
        }
    };
    Ok((stored, field_value(bindings, form, &annot)))
}

/// Metin alanına ham değeri yazar. Alanın biçimi varsa görünüm akışı biçimli metni çizer ve
/// `/V` ham değeri tutar (Acrobat'ın saklama biçimi); bu ancak widget sözlüğü alanın kendisiyse
/// (`/T` taşıyorsa) güvenlidir, yoksa `/V` widget'a yazılırdı. Biçimli metin yazılamazsa (ör.
/// MaxLen onu keserse) görünüm ham değeri çizer.
fn write_text(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    annot: &RawAnnotation<'_>,
    field_name: &str,
    raw: &str,
    format: &af::Format,
) -> Result<(), DocumentIoFailure> {
    let display = af::format_value(format, raw);
    if display != raw && bindings.is_true(bindings.FPDFAnnot_HasKey(annot.handle, "T")) {
        replace_text(bindings, form, page, annot, field_name, &display)?;
        let raw_value = wide(raw);
        if same_text(&field_value(bindings, form, annot), &display)
            && bindings.is_true(bindings.FPDFAnnot_SetStringValue(
                annot.handle,
                "V",
                raw_value.as_ptr(),
            ))
            && same_text(&field_value(bindings, form, annot), raw)
        {
            return Ok(());
        }
    }
    replace_text(bindings, form, page, annot, field_name, raw)?;

    let written = field_value(bindings, form, annot);
    if same_text(&written, raw) {
        return Ok(());
    }
    let requested = normalize_newlines(raw);
    let stored = normalize_newlines(&written);
    if requested.starts_with(&stored) {
        return Err(DocumentIoFailure::preflight(format!(
            "'{field_name}' alanı en fazla {} karakter kabul ediyor (MaxLen)",
            stored.chars().count()
        )));
    }
    Err(DocumentIoFailure::write(format!(
        "'{field_name}' alanına yazılan değer PDFium'dan farklı geri okundu"
    )))
}

fn replace_text(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    page: &RawPage<'_>,
    annot: &RawAnnotation<'_>,
    field_name: &str,
    text: &str,
) -> Result<(), DocumentIoFailure> {
    focus(bindings, form, annot, field_name)?;
    bindings.FORM_SelectAllText(form.handle, page.handle);
    let wide = wide(text);
    bindings.FORM_ReplaceSelection(form.handle, page.handle, wide.as_ptr());
    bindings.FORM_ForceToKillFocus(form.handle);
    Ok(())
}

fn same_text(left: &str, right: &str) -> bool {
    normalize_newlines(left) == normalize_newlines(right)
}

fn field_name(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    annot: &RawAnnotation<'_>,
) -> String {
    wide_string(|buffer, length| {
        bindings.FPDFAnnot_GetFormFieldName(form.handle, annot.handle, buffer, length)
    })
}

fn field_value(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    annot: &RawAnnotation<'_>,
) -> String {
    wide_string(|buffer, length| {
        bindings.FPDFAnnot_GetFormFieldValue(form.handle, annot.handle, buffer, length)
    })
}

/// Alanın Format, Keystroke, Validate ve Calculate betikleri. PDFium betiği alan sözlüğünden
/// (kalıtımla) okur.
fn field_actions(
    bindings: &dyn PdfiumLibraryBindings,
    form: &FormEnvironment<'_>,
    annot: &RawAnnotation<'_>,
) -> af::Actions {
    let script = |event: c_int| {
        let fetch = |buffer: *mut FPDF_WCHAR, length: c_ulong| {
            bindings.FPDFAnnot_GetFormAdditionalActionJavaScript(
                form.handle,
                annot.handle,
                event,
                buffer,
                length,
            )
        };
        let length = fetch(std::ptr::null_mut(), 0);
        if length > MAX_FIELD_SCRIPT_BYTES {
            tracing::warn!(
                operation = OPERATION,
                event,
                length,
                "Alan betiği sınırdan uzun; okunmadı"
            );
            return None;
        }
        Some(wide_string(fetch)).filter(|script| !script.is_empty())
    };
    af::actions_from_scripts(
        script(FPDF_ANNOT_AACTION_FORMAT).as_deref(),
        script(FPDF_ANNOT_AACTION_KEY_STROKE).as_deref(),
        script(FPDF_ANNOT_AACTION_VALIDATE).as_deref(),
        script(FPDF_ANNOT_AACTION_CALCULATE).as_deref(),
    )
}

// ---------------------------------------------------------------------------
// Yeniden hesaplama
// ---------------------------------------------------------------------------

/// Bir alanın hesaplama için okunan durumu (belge sırasındaki ilk widget'ından).
struct ScannedField {
    name: String,
    page_index: u32,
    annot_index: u32,
    field_type: c_int,
    value: String,
    actions: af::Actions,
}

/// Bir hesaplamanın yazılacak sonucu.
#[derive(Debug, Clone, PartialEq)]
struct CalculatedValue {
    page_index: u32,
    annot_index: u32,
    field_name: String,
    value: String,
    format: af::Format,
}

/// Belgedeki tüm alanları ayrı, salt-okunur bir PDFium belgesinde tarar. Yazılan belgede
/// yalnız edit'lerin ve değişen hesaplanan alanların sayfaları yüklenir: bir sayfanın form
/// görünümünü kurmak `/NeedAppearances` belgelerde o sayfanın widget'larına görünüm akışı
/// üretir ve bu, dokunulmamış sayfaları da kayda katardı.
fn scan_fields(
    bindings: &dyn PdfiumLibraryBindings,
    source: &str,
    password: Option<&str>,
) -> Result<Vec<ScannedField>, DocumentIoFailure> {
    let document = RawDocument::open(bindings, source, password)
        .ok_or_else(|| DocumentIoFailure::preflight("PDF form hesaplaması için açılamadı"))?;
    let form = FormEnvironment::init(bindings, &document)?;
    let page_count = u32::try_from(bindings.FPDF_GetPageCount(document.handle)).unwrap_or(0);
    let mut names = HashSet::new();
    let mut fields = Vec::new();
    let mut widgets = 0usize;
    for page_index in 0..page_count {
        let page = RawPage::load(bindings, &document, &form, page_index)?;
        for index in 0..bindings.FPDFPage_GetAnnotCount(page.handle) {
            let annot = RawAnnotation::get(bindings, &page, index)?;
            if bindings.FPDFAnnot_GetSubtype(annot.handle) != FPDF_ANNOT_WIDGET {
                continue;
            }
            widgets += 1;
            if widgets > MAX_CALCULATION_WIDGETS {
                return Err(DocumentIoFailure::preflight(format!(
                    "Belgede form hesaplaması için {MAX_CALCULATION_WIDGETS}'den fazla widget var"
                )));
            }
            let name = field_name(bindings, &form, &annot);
            if name.is_empty() || !names.insert(name.clone()) {
                continue;
            }
            fields.push(ScannedField {
                page_index,
                annot_index: u32::try_from(index).unwrap_or(u32::MAX),
                field_type: bindings.FPDFAnnot_GetFormFieldType(form.handle, annot.handle),
                value: field_value(bindings, &form, &annot),
                actions: field_actions(bindings, &form, &annot),
                name,
            });
        }
    }
    Ok(fields)
}

/// Hesaplanan metin alanlarını bağımlılık sırasıyla çalıştırır; değeri değişenleri döndürür.
/// `edited`, bu istekte yazılan alanların yeni değerleridir.
fn recalculate(
    fields: &[ScannedField],
    edited: &BTreeMap<String, String>,
) -> Result<Vec<CalculatedValue>, DocumentIoFailure> {
    let mut values: BTreeMap<String, String> = fields
        .iter()
        .map(|field| (field.name.clone(), field.value.clone()))
        .collect();
    values.extend(
        edited
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    let mut writes = Vec::new();
    for index in calculation_order(fields)? {
        let field = &fields[index];
        if field.field_type != FPDF_FORMFIELD_TEXTFIELD {
            continue;
        }
        let lookup = |name: &str| values_under(&values, name);
        let Some(value) = af::calculate(&field.actions.calculate, &lookup) else {
            continue;
        };
        if values.get(&field.name) == Some(&value) {
            continue;
        }
        values.insert(field.name.clone(), value.clone());
        writes.push(CalculatedValue {
            page_index: field.page_index,
            annot_index: field.annot_index,
            field_name: field.name.clone(),
            value,
            format: field.actions.format.clone(),
        });
    }
    Ok(writes)
}

/// `name` alanının ve altındaki alanların (`name.0`, `name.1`) değerleri; AF'nin grup okuması.
fn values_under(values: &BTreeMap<String, String>, name: &str) -> Vec<String> {
    values
        .range(name.to_string()..)
        .take_while(|(key, _)| key.starts_with(name))
        .filter(|(key, _)| key.len() == name.len() || key[name.len()..].starts_with('.'))
        .map(|(_, value)| value.clone())
        .collect()
}

/// Hesaplanan alanların (`fields` indeksleri) çalışma sırası: her alan okuduğu hesaplanan
/// alanlardan sonra, bunun dışında belge sırasıyla. Bir döngüdeki alanlar sona, belge
/// sırasıyla eklenir; döngü için doğru sıra yoktur.
fn calculation_order(fields: &[ScannedField]) -> Result<Vec<usize>, DocumentIoFailure> {
    let calculated: Vec<usize> = fields
        .iter()
        .enumerate()
        .filter(|(_, field)| field.actions.calculate != af::Calculate::None)
        .map(|(index, _)| index)
        .collect();
    if calculated.len() > MAX_CALCULATED_FIELDS {
        return Err(DocumentIoFailure::preflight(format!(
            "Belgede {MAX_CALCULATED_FIELDS}'den fazla hesaplanan alan var"
        )));
    }
    let reads = |field: &str, read: &str| {
        field
            .strip_prefix(read)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    };
    let count = calculated.len();
    let mut dependents = vec![Vec::new(); count];
    let mut waiting = vec![0usize; count];
    for (k, &index) in calculated.iter().enumerate() {
        let names = fields[index].actions.calculate.reads();
        for (j, &other) in calculated.iter().enumerate() {
            if j != k && names.iter().any(|read| reads(&fields[other].name, read)) {
                dependents[j].push(k);
                waiting[k] += 1;
            }
        }
    }
    let mut ready: BTreeSet<usize> = (0..count).filter(|&k| waiting[k] == 0).collect();
    let mut order = Vec::with_capacity(count);
    while let Some(k) = ready.pop_first() {
        order.push(k);
        for &dependent in &dependents[k] {
            waiting[dependent] -= 1;
            if waiting[dependent] == 0 {
                ready.insert(dependent);
            }
        }
    }
    if order.len() < count {
        let placed: HashSet<usize> = order.iter().copied().collect();
        order.extend((0..count).filter(|k| !placed.contains(k)));
    }
    Ok(order.into_iter().map(|k| calculated[k]).collect())
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

    fn last_xref_object(bytes: &[u8]) -> (usize, Vec<u8>) {
        let startxref = bytes
            .windows(b"startxref".len())
            .rposition(|window| window == b"startxref")
            .unwrap();
        let digits: String = String::from_utf8_lossy(&bytes[startxref + 9..])
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let offset: usize = digits.parse().unwrap();
        (offset, bytes[offset..startxref].to_vec())
    }

    #[test]
    fn incremental_fill_of_an_xref_stream_pdf_writes_a_well_formed_xref_stream() {
        // lopdf writes the fixture with a cross-reference stream, as most current PDFs are.
        let (_directory, path) = fixture();
        let original = std::fs::read(&path).unwrap();
        assert!(String::from_utf8_lossy(&last_xref_object(&original).1).contains("/Type/XRef"));
        fill(&path, vec![edit(0, 0, "name", text("Grace"))]).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&original));
        let (offset, object) = last_xref_object(&saved);
        assert!(offset >= original.len());
        let text = String::from_utf8_lossy(&object);
        assert!(text.contains("/Type/XRef"), "{text}");
        assert!(text.trim_end().ends_with("endobj"), "{text}");
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

    // --- AF format, validate and calculate --------------------------------------------------

    const MONEY_FORMAT: &str = "AFNumber_Format(2, 0, 0, 0, \"$\", true);";
    const MONEY_KEYSTROKE: &str = "AFNumber_Keystroke(2, 0, 0, 0, \"$\", true);";

    fn text_field(name: &str, events: &[(&str, &str)]) -> lopdf::Dictionary {
        let mut field = dictionary! { "FT" => "Tx", "T" => string(name) };
        if !events.is_empty() {
            field.set("AA", scripts(events));
        }
        field
    }

    /// Text fields stacked down page 0 in the order given; field `i` is annotation `i`.
    fn text_fields(path: &Path, fields: Vec<lopdf::Dictionary>) {
        let mut f = FormFixture::new();
        for (i, field) in fields.into_iter().enumerate() {
            let top = 760.0 - 30.0 * i as f32;
            f.widget(field, [50.0, top - 20.0, 300.0, top]);
        }
        f.save(path, true);
    }

    /// The content of the normal appearance stream of annotation `annot_index` on `page`.
    fn normal_appearance(path: &Path, page: u32, annot_index: usize) -> String {
        let document = lopdf::Document::load(path).unwrap();
        let page_id = *document.get_pages().get(&(page + 1)).unwrap();
        let page = document.get_dictionary(page_id).unwrap();
        let annots = document
            .dereference(page.get(b"Annots").unwrap())
            .unwrap()
            .1
            .as_array()
            .unwrap();
        let widget = document
            .dereference(&annots[annot_index])
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
        let lopdf::Object::Stream(stream) = document.dereference(ap.get(b"N").unwrap()).unwrap().1
        else {
            panic!("/AP /N is not a stream");
        };
        let content = stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone());
        String::from_utf8_lossy(&content).into_owned()
    }

    fn value_of(path: &Path, page: u32, annot_index: u32) -> Option<String> {
        let widgets = crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            super::super::read_page_form_fields(&document, page)
                .map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        widget(&widgets, annot_index).value.clone()
    }

    #[test]
    fn a_formatted_number_is_stored_raw_and_drawn_formatted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("money.pdf");
        text_fields(
            &path,
            vec![text_field(
                "total",
                &[("F", MONEY_FORMAT), ("K", MONEY_KEYSTROKE)],
            )],
        );
        fill(&path, vec![edit(0, 0, "total", text("$1,234.5"))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("1234.5"));
        let appearance = normal_appearance(&path, 0, 0);
        assert!(appearance.contains("($1,234.50) Tj"), "{appearance}");
    }

    #[test]
    fn non_ascii_currency_from_an_escaped_script_is_drawn() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("euro.pdf");
        text_fields(
            &path,
            vec![text_field(
                "price",
                &[("F", "AFNumber_Format(2, 2, 0, 0, \" \\u20AC\", false);")],
            )],
        );
        fill(&path, vec![edit(0, 0, "price", text("1234,5"))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("1234.5"));
        // WinAnsi encodes the euro sign as 0x80.
        let appearance = normal_appearance(&path, 0, 0);
        assert!(appearance.contains("1.234,50 \u{fffd}"), "{appearance}");
    }

    #[test]
    fn a_special_format_keeps_the_typed_value_and_draws_the_mask() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("phone.pdf");
        text_fields(
            &path,
            vec![text_field(
                "phone",
                &[
                    ("F", "AFSpecial_Format(2);"),
                    ("K", "AFSpecial_Keystroke(2);"),
                ],
            )],
        );
        fill(&path, vec![edit(0, 0, "phone", text("5551234567"))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("5551234567"));
        let appearance = normal_appearance(&path, 0, 0);
        assert!(
            appearance.contains("(\\(555\\) 123-4567) Tj"),
            "{appearance}"
        );
    }

    #[test]
    fn text_that_does_not_fit_the_format_is_rejected_without_touching_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("formats.pdf");
        text_fields(
            &path,
            vec![
                text_field("total", &[("F", MONEY_FORMAT), ("K", MONEY_KEYSTROKE)]),
                text_field("due", &[("F", "AFDate_FormatEx(\"mm/dd/yyyy\");")]),
                text_field("zip", &[("K", "AFSpecial_KeystrokeEx(\"99999\");")]),
            ],
        );
        let error = rejected(&path, vec![edit(0, 0, "total", text("ten"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(
            error.detail.contains("[ total ] alanının biçimine uymuyor"),
            "{}",
            error.detail
        );
        let error = rejected(&path, vec![edit(0, 1, "due", text("2/30/2024"))]);
        assert!(error.detail.contains("mm/dd/yyyy"), "{}", error.detail);
        let error = rejected(&path, vec![edit(0, 2, "zip", text("12a45"))]);
        assert!(error.detail.contains("[ zip ]"), "{}", error.detail);
        // A blank value clears a formatted field.
        fill(&path, vec![edit(0, 0, "total", text("  "))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref().unwrap_or(""), "");
    }

    #[test]
    fn a_value_outside_the_validated_range_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("range.pdf");
        text_fields(
            &path,
            vec![text_field(
                "percent",
                &[("V", "AFRange_Validate(true, 0, true, 100);")],
            )],
        );
        let error = rejected(&path, vec![edit(0, 0, "percent", text("120"))]);
        assert_eq!(error.stage, DocumentIoStage::Preflight);
        assert!(
            error
                .detail
                .contains("0 veya daha büyük ve 100 veya daha küçük"),
            "{}",
            error.detail
        );
        fill(&path, vec![edit(0, 0, "percent", text("100"))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("100"));
    }

    #[test]
    fn calculated_fields_follow_the_fields_they_read() {
        // `total` comes first in the document but reads `subtotal`, which reads the edited
        // fields: the dependency order, not document order, decides. Both are read-only, as
        // calculated fields usually are.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invoice.pdf");
        let mut total = text_field(
            "total",
            &[
                (
                    "C",
                    "AFSimple_Calculate(\"SUM\", new Array (\"subtotal\", \"shipping\"));",
                ),
                ("F", MONEY_FORMAT),
            ],
        );
        total.set("Ff", 1);
        let mut subtotal = text_field(
            "subtotal",
            &[(
                "C",
                "/** BVCALC unit\\ price * qty EVCALC **/ event.value = AFMakeNumber(getField(\"unit price\").value) * AFMakeNumber(getField(\"qty\").value);",
            )],
        );
        subtotal.set("Ff", 1);
        let mut shipping = text_field("shipping", &[]);
        shipping.set("V", string("5"));
        text_fields(
            &path,
            vec![
                total,
                subtotal,
                text_field("unit price", &[]),
                text_field("qty", &[]),
                shipping,
            ],
        );
        let original = std::fs::read(&path).unwrap();
        fill(
            &path,
            vec![
                edit(0, 2, "unit price", text("2.5")),
                edit(0, 3, "qty", text("4")),
            ],
        )
        .unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(&original));
        assert_eq!(value_of(&path, 0, 1).as_deref(), Some("10"));
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("15"));
        let appearance = normal_appearance(&path, 0, 0);
        assert!(appearance.contains("($15.00) Tj"), "{appearance}");

        // A later edit recalculates from the stored values.
        fill(&path, vec![edit(0, 4, "shipping", text("0.5"))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("10.5"));
    }

    #[test]
    fn a_calculation_reads_fields_on_other_pages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pages.pdf");
        let mut f = FormFixture::new();
        f.widget(text_field("qty", &[]), [50.0, 700.0, 300.0, 720.0]);
        f.next_page();
        f.widget(
            text_field("double", &[("C", "/** BVCALC qty * 2 EVCALC **/")]),
            [50.0, 700.0, 300.0, 720.0],
        );
        f.save(&path, true);
        fill(&path, vec![edit(0, 0, "qty", text("21"))]).unwrap();
        assert_eq!(value_of(&path, 1, 0).as_deref(), Some("42"));
    }

    #[test]
    fn a_calculated_field_the_user_typed_into_takes_the_calculated_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("override.pdf");
        text_fields(
            &path,
            vec![
                text_field("a", &[]),
                text_field("sum", &[("C", "AFSimple_Calculate(\"SUM\", \"a\");")]),
            ],
        );
        fill(
            &path,
            vec![edit(0, 0, "a", text("3")), edit(0, 1, "sum", text("99"))],
        )
        .unwrap();
        assert_eq!(value_of(&path, 0, 1).as_deref(), Some("3"));
    }

    #[test]
    fn checkbox_export_values_feed_calculations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("checkbox.pdf");
        let mut f = FormFixture::new();
        f.checkbox("extra", [50.0, 700.0, 66.0, 716.0], "10", "Off");
        f.widget(
            text_field(
                "cost",
                &[("C", "AFSimple_Calculate(\"SUM\", new Array(\"extra\"));")],
            ),
            [100.0, 700.0, 300.0, 720.0],
        );
        f.save(&path, true);
        fill(
            &path,
            vec![edit(
                0,
                0,
                "extra",
                FormFieldValue::Checkbox { checked: true },
            )],
        )
        .unwrap();
        assert_eq!(value_of(&path, 0, 1).as_deref(), Some("10"));
    }

    #[test]
    fn a_widget_that_is_not_the_field_draws_the_raw_value() {
        // `/V` belongs to the parent field; writing it to the kid widget would leave the
        // field and the widget disagreeing, so the appearance shows the raw value instead.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("kid.pdf");
        let mut f = FormFixture::new();
        f.field_with_kid_widget(
            text_field("total", &[("F", MONEY_FORMAT)]),
            [50.0, 700.0, 300.0, 720.0],
        );
        f.save(&path, true);
        fill(&path, vec![edit(0, 0, "total", text("7"))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("7"));
        let appearance = normal_appearance(&path, 0, 0);
        assert!(appearance.contains("(7) Tj"), "{appearance}");
        let document = lopdf::Document::load(&path).unwrap();
        let kids_with_v = document
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .filter(|dict| dict.has(b"Parent") && dict.has(b"V"))
            .count();
        assert_eq!(kids_with_v, 0, "a kid widget got its own /V");
    }

    #[test]
    fn fields_without_scripts_are_written_as_before() {
        let (_directory, path) = fixture();
        fill(&path, vec![edit(0, 0, "name", text("  spaced  "))]).unwrap();
        assert_eq!(value_of(&path, 0, 0).as_deref(), Some("  spaced  "));
    }

    #[test]
    fn scanning_for_calculations_leaves_untouched_pages_as_they_were() {
        // With /NeedAppearances, loading a page's form view generates appearances for its
        // widgets. The scan loads every page, so it runs in its own document: a fill on page 1
        // must not give page 2's widget an appearance stream in the saved file.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("untouched.pdf");
        let mut f = FormFixture::new();
        f.widget(text_field("first", &[]), [50.0, 700.0, 300.0, 720.0]);
        f.next_page();
        let mut second = text_field("second", &[]);
        second.set("V", string("keep"));
        f.widget(second, [50.0, 700.0, 300.0, 720.0]);
        f.save(&path, true);
        let mut document = lopdf::Document::load(&path).unwrap();
        let acroform = document
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .unwrap()
            .as_reference()
            .unwrap();
        document
            .get_dictionary_mut(acroform)
            .unwrap()
            .set("NeedAppearances", true);
        document.save(&path).unwrap();

        fill(&path, vec![edit(0, 0, "first", text("x"))]).unwrap();

        let saved = lopdf::Document::load(&path).unwrap();
        let second = saved
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find(|dict| dict.get(b"T").ok() == Some(&string("second")))
            .unwrap();
        assert!(!second.has(b"AP"), "page 2's widget got an appearance");
    }

    fn scanned(name: &str, calculate: &str) -> ScannedField {
        ScannedField {
            name: name.to_string(),
            page_index: 0,
            annot_index: 0,
            field_type: FPDF_FORMFIELD_TEXTFIELD,
            value: String::new(),
            actions: af::actions_from_scripts(
                None,
                None,
                None,
                Some(calculate).filter(|c| !c.is_empty()),
            ),
        }
    }

    #[test]
    fn calculation_order_puts_each_field_after_what_it_reads() {
        let fields = vec![
            scanned("total", "AFSimple_Calculate(\"SUM\", \"row\");"),
            scanned("row.0", "/** BVCALC a * 2 EVCALC **/"),
            scanned("a", ""),
            scanned("row.1", "/** BVCALC row.0 + 1 EVCALC **/"),
            scanned("rowing", "/** BVCALC 1 EVCALC **/"),
        ];
        // `total` reads the group `row` (row.0 and row.1) but not `rowing`.
        assert_eq!(calculation_order(&fields).unwrap(), vec![1, 3, 0, 4]);
        // A loop has no right order: its fields run once, in document order, after the rest.
        let looped = vec![
            scanned("x", "/** BVCALC y + 1 EVCALC **/"),
            scanned("y", "/** BVCALC x + 1 EVCALC **/"),
            scanned("z", "/** BVCALC 3 EVCALC **/"),
        ];
        assert_eq!(calculation_order(&looped).unwrap(), vec![2, 0, 1]);
    }

    #[test]
    fn group_reads_take_the_field_and_its_children_only() {
        let values: BTreeMap<String, String> = [
            ("row", "1"),
            ("row.0", "2"),
            ("row.1", "3"),
            ("rowing", "9"),
            ("rox", "8"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(values_under(&values, "row"), ["1", "2", "3"]);
        assert_eq!(values_under(&values, "row.1"), ["3"]);
        assert!(values_under(&values, "missing").is_empty());
    }

    #[test]
    fn too_many_calculated_fields_are_refused() {
        let fields: Vec<_> = (0..=MAX_CALCULATED_FIELDS)
            .map(|i| scanned(&format!("f{i}"), "/** BVCALC 1 EVCALC **/"))
            .collect();
        assert!(calculation_order(&fields).is_err());
    }
}
