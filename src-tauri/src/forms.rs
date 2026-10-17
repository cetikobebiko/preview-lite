//! AcroForm alanlarının salt-okunur modeli (docs/engineering/form-filling-plan.md, Faz 1).
//!
//! Okuma yalnız worker process'inde, pdfium-render'ın getter'larıyla yapılır. Değer
//! yazan pdfium-render setter'ları kullanılmaz: görünüm akışı üretmezler ve checkbox
//! on-state'ini `/Yes` varsayarlar (bkz. plan, "pdfium-render'ın yüksek seviye form
//! API'si neden kullanılmayacak"). Aynı nedenle checkbox durumu pdfium-render'ın
//! `is_checked` sezgisinden değil, widget'ın `/AS` değerinden türetilir.

use crate::annotations::{rect_data_from_pdf_rect, NormalizedPageRect, PageDisplayTransform};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

/// Bir sayfada döndürülecek en fazla widget sayısı.
pub const MAX_FORM_WIDGETS_PER_PAGE: usize = 2_000;
/// Bir alan değerinin veya seçenek etiketinin en fazla karakter sayısı.
pub const MAX_FORM_TEXT_CHARS: usize = 32_768;
/// Bir combo/list alanındaki en fazla seçenek sayısı.
pub const MAX_FORM_OPTIONS_PER_FIELD: usize = 4_096;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormType {
    None,
    AcroForm,
    XfaFull,
    XfaForeground,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FormInfo {
    pub form_type: FormType,
    /// Belgedeki dijital imza sayısı. Form kaydı tam yeniden yazım olduğundan imzaları
    /// geçersiz kılar; frontend ilk düzenlemeden önce uyarmalıdır.
    pub signature_count: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFieldKind {
    Text,
    Checkbox,
    RadioButton,
    ComboBox,
    ListBox,
    PushButton,
    Signature,
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FormFieldOption {
    pub label: String,
    pub selected: bool,
}

/// Bir sayfadaki tek bir form widget'ı. Aynı ada sahip birden fazla widget aynı alanın
/// görünümleridir (ör. radio grubunun düğmeleri).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FormWidgetData {
    /// Sayfanın `/Annots` dizisindeki indeks. Yazma isteği alan adıyla birlikte doğrular.
    pub annot_index: u32,
    /// Tam nitelikli alan adı (`parent.child`).
    pub field_name: String,
    pub kind: FormFieldKind,
    pub read_only: bool,
    pub required: bool,
    /// Metin, combo ve list için alanın değeri; checkbox ve radio için grubun değeri.
    pub value: Option<String>,
    /// Checkbox ve radio: bu widget seçili mi.
    pub checked: Option<bool>,
    /// Combo ve list seçenekleri, belgedeki sırayla.
    pub options: Vec<FormFieldOption>,
    pub multiline: bool,
    pub password: bool,
    // Comb, DoNotScroll, RichText, FileSelect ve Sort bilinçli olarak yok: pdfium-render
    // 0.8.37 bu bayrakları ISO 32000 Tablo 228/230'daki bitlerin bir üstünden okuyor
    // (ör. Comb için `1 << 25`) ve ham `/Ff` public değil. Faz 3'ün ham yazma yolu
    // `FPDFAnnot_GetFormFieldFlags` ile doğru bayrakları okur.
    pub multi_select: bool,
    /// Combo: serbest metin girişine izin veriyor mu.
    pub editable: bool,
    /// Hidden veya NoView bayrağı: görüntülenmez, etkileşim almaz.
    pub hidden: bool,
    /// Widget `/Rect`'inin görüntü uzayındaki karşılığı; bounds okunamazsa `None`.
    pub normalized_rect: Option<NormalizedPageRect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormReadError {
    /// PDFium sayfayı veya alanı okuyamadı.
    Pdfium(String),
    /// Sayfa, IPC sınırları içinde döndürülemeyecek kadar büyük form verisi taşıyor.
    LimitExceeded(String),
}

pub(crate) fn read_form_info(document: &PdfDocument<'_>) -> FormInfo {
    let form_type = match document.form().map(|form| form.form_type()) {
        None | Some(PdfFormType::None) => FormType::None,
        Some(PdfFormType::Acrobat) => FormType::AcroForm,
        Some(PdfFormType::XfaFull) => FormType::XfaFull,
        Some(PdfFormType::XfaForeground) => FormType::XfaForeground,
    };
    FormInfo {
        form_type,
        signature_count: u32::from(document.signatures().len()),
    }
}

pub(crate) fn read_page_form_fields(
    document: &PdfDocument<'_>,
    page_index: u32,
) -> Result<Vec<FormWidgetData>, FormReadError> {
    if document.form().is_none() {
        return Ok(Vec::new());
    }
    let page_index = u16::try_from(page_index)
        .map_err(|_| FormReadError::Pdfium("Sayfa indeksi PDFium sınırını aşıyor".to_string()))?;
    let page = document
        .pages()
        .get(page_index)
        .map_err(|error| FormReadError::Pdfium(format!("Sayfa bulunamadı: {error:?}")))?;
    let display = PageDisplayTransform::for_page(&page).map_err(FormReadError::Pdfium)?;

    // PDFium adopts widgets that only the page's `/Annots` reaches (not the form's
    // `/Fields`) when it builds the page's form view: `CPDF_InteractiveForm::FixPageFields`
    // runs in the `CPDFSDK_PageView` constructor. pdfium-render creates that view only
    // through `FPDF_FFLDraw`, so a 1×1 render with form data runs it before the widgets are
    // read. Without it such widgets have no form field type and are skipped.
    page.render_with_config(
        &PdfRenderConfig::new()
            .set_fixed_size(1, 1)
            .render_form_data(true),
    )
    .map_err(|error| FormReadError::Pdfium(format!("Sayfa form görünümü kurulamadı: {error}")))?;

    let annotations = page.annotations();
    let mut widgets = Vec::new();
    for index in 0..annotations.len() {
        let Ok(annotation) = annotations.get(index) else {
            continue;
        };
        let Some(field) = annotation.as_form_field() else {
            continue;
        };
        if widgets.len() == MAX_FORM_WIDGETS_PER_PAGE {
            return Err(FormReadError::LimitExceeded(format!(
                "Sayfada {MAX_FORM_WIDGETS_PER_PAGE}'den fazla form widget'ı var"
            )));
        }
        let normalized_rect = annotation
            .bounds()
            .ok()
            .map(|bounds| display.user_to_normalized_rect(&rect_data_from_pdf_rect(&bounds)));
        let hidden = annotation.is_hidden() || annotation.is_printable_but_not_viewable();
        let annot_index = u32::try_from(index).map_err(|_| {
            FormReadError::LimitExceeded("Annotation indeksi u32 sınırını aşıyor".to_string())
        })?;
        widgets.push(widget_data(annot_index, field, hidden, normalized_rect)?);
    }
    Ok(widgets)
}

fn widget_data(
    annot_index: u32,
    field: &PdfFormField<'_>,
    hidden: bool,
    normalized_rect: Option<NormalizedPageRect>,
) -> Result<FormWidgetData, FormReadError> {
    let mut widget = FormWidgetData {
        annot_index,
        field_name: bounded(field.name().unwrap_or_default())?,
        kind: FormFieldKind::Unknown,
        read_only: field.is_read_only(),
        required: field.is_required(),
        value: None,
        checked: None,
        options: Vec::new(),
        multiline: false,
        password: false,
        multi_select: false,
        editable: false,
        hidden,
        normalized_rect,
    };
    match field {
        PdfFormField::Text(text) => {
            widget.kind = FormFieldKind::Text;
            widget.value = text.value().map(bounded).transpose()?;
            widget.multiline = text.is_multiline();
            widget.password = text.is_password();
        }
        PdfFormField::Checkbox(checkbox) => {
            widget.kind = FormFieldKind::Checkbox;
            widget.value = checkbox.group_value().map(bounded).transpose()?;
            widget.checked = Some(match field.appearance_stream() {
                Some(state) => is_on_state(&state),
                None => checkbox.is_checked().unwrap_or(false),
            });
        }
        PdfFormField::RadioButton(radio) => {
            widget.kind = FormFieldKind::RadioButton;
            widget.value = radio.group_value().map(bounded).transpose()?;
            widget.checked = Some(radio.is_checked().unwrap_or(false));
        }
        PdfFormField::ComboBox(combo) => {
            widget.kind = FormFieldKind::ComboBox;
            widget.value = combo.value().map(bounded).transpose()?;
            widget.options = options(combo.options())?;
            widget.multi_select = combo.is_multiselect();
            widget.editable = combo.has_editable_text_box();
        }
        PdfFormField::ListBox(list) => {
            widget.kind = FormFieldKind::ListBox;
            widget.value = list.value().map(bounded).transpose()?;
            widget.options = options(list.options())?;
            widget.multi_select = list.is_multiselect();
        }
        PdfFormField::PushButton(_) => widget.kind = FormFieldKind::PushButton,
        PdfFormField::Signature(_) => widget.kind = FormFieldKind::Signature,
        PdfFormField::Unknown(_) => {}
    }
    Ok(widget)
}

/// `/AS` değeri bir on-state mi: tanımlı ve `Off` dışında bir ad.
fn is_on_state(state: &str) -> bool {
    let name = state.trim_start_matches('/');
    !name.is_empty() && name != "Off"
}

fn options(options: &PdfFormFieldOptions<'_>) -> Result<Vec<FormFieldOption>, FormReadError> {
    if options.len() > MAX_FORM_OPTIONS_PER_FIELD {
        return Err(FormReadError::LimitExceeded(format!(
            "Form alanında {MAX_FORM_OPTIONS_PER_FIELD}'ten fazla seçenek var"
        )));
    }
    options
        .iter()
        .map(|option| {
            Ok(FormFieldOption {
                label: bounded(option.label().cloned().unwrap_or_default())?,
                selected: option.is_set(),
            })
        })
        .collect()
}

fn bounded(text: String) -> Result<String, FormReadError> {
    if text.chars().count() > MAX_FORM_TEXT_CHARS {
        return Err(FormReadError::LimitExceeded(format!(
            "Form alanı metni {MAX_FORM_TEXT_CHARS} karakter sınırını aşıyor"
        )));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
    use std::path::Path;

    fn string(text: &str) -> Object {
        Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
    }

    fn name(text: &str) -> Object {
        Object::Name(text.as_bytes().to_vec())
    }

    fn rect(r: [f32; 4]) -> Object {
        Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
    }

    /// Tek sayfalık (612×792) AcroForm fixture'ı. `widgets` her biri kendi widget'ı
    /// olan terminal alanlardır; radio grubu ayrıca eklenir.
    struct FormFixture {
        doc: Document,
        page_id: ObjectId,
        font: ObjectId,
        annots: Vec<Object>,
        fields: Vec<Object>,
    }

    impl FormFixture {
        fn new() -> Self {
            let mut doc = Document::with_version("1.7");
            let font = doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                "Encoding" => "WinAnsiEncoding",
            });
            let page_id = doc.new_object_id();
            Self {
                doc,
                page_id,
                font,
                annots: Vec::new(),
                fields: Vec::new(),
            }
        }

        fn on_off(&mut self, on: &str) -> Object {
            let on_ap = self.doc.add_object(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0.0, 0.0, 16.0, 16.0]) },
                b"0 g 2 2 12 12 re f".to_vec(),
            ));
            let off_ap = self.doc.add_object(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0.0, 0.0, 16.0, 16.0]) },
                Vec::new(),
            ));
            Object::Dictionary(dictionary! { "N" => dictionary! { on => on_ap, "Off" => off_ap } })
        }

        fn widget(&mut self, dict: Dictionary, r: [f32; 4]) {
            self.add_widget(dict, r, true);
        }

        /// A widget that only the page's `/Annots` reaches, not the form's `/Fields`.
        fn orphan_widget(&mut self, dict: Dictionary, r: [f32; 4]) {
            self.add_widget(dict, r, false);
        }

        fn add_widget(&mut self, mut dict: Dictionary, r: [f32; 4], listed: bool) {
            dict.set("Type", "Annot");
            dict.set("Subtype", "Widget");
            dict.set("Rect", rect(r));
            if !dict.has(b"F") {
                dict.set("F", 4);
            }
            dict.set("P", self.page_id);
            dict.set("DA", string("/Helv 10 Tf 0 g"));
            let id = self.doc.add_object(dict);
            self.annots.push(Object::Reference(id));
            if listed {
                self.fields.push(Object::Reference(id));
            }
        }

        fn checkbox(&mut self, t: &str, r: [f32; 4], on: &str, state: &str) {
            let ap = self.on_off(on);
            self.widget(
                dictionary! { "FT" => "Btn", "T" => string(t), "V" => name(state), "AS" => name(state), "AP" => ap },
                r,
            );
        }

        fn radio(&mut self, t: &str, kids: &[(&str, [f32; 4])], selected: &str) {
            let parent = self.doc.new_object_id();
            let mut kid_refs = Vec::new();
            for (state, r) in kids {
                let ap = self.on_off(state);
                let id = self.doc.add_object(dictionary! {
                    "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect(*r), "F" => 4,
                    "P" => self.page_id, "Parent" => parent, "AP" => ap,
                    "AS" => name(if *state == selected { state } else { "Off" }),
                });
                self.annots.push(Object::Reference(id));
                kid_refs.push(Object::Reference(id));
            }
            self.doc.objects.insert(
                parent,
                Object::Dictionary(dictionary! {
                    "FT" => "Btn", "T" => string(t), "Ff" => (1 << 15) | (1 << 14),
                    "V" => name(selected), "Kids" => kid_refs,
                }),
            );
            self.fields.push(Object::Reference(parent));
        }

        fn save(mut self, path: &Path, with_acroform: bool) {
            let pages_id = self.doc.new_object_id();
            let content = self.doc.add_object(Stream::new(dictionary! {}, Vec::new()));
            self.doc.objects.insert(
                self.page_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Page", "Parent" => pages_id,
                    "MediaBox" => rect([0.0, 0.0, 612.0, 792.0]), "Contents" => content,
                    "Annots" => self.annots.clone(),
                }),
            );
            self.doc.objects.insert(
                pages_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Pages", "Kids" => vec![Object::Reference(self.page_id)], "Count" => 1,
                }),
            );
            let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
            if with_acroform {
                let acroform = self.doc.add_object(dictionary! {
                    "Fields" => self.fields.clone(),
                    "DR" => dictionary! { "Font" => dictionary! { "Helv" => self.font } },
                    "DA" => string("/Helv 0 Tf 0 g"),
                });
                catalog.set("AcroForm", acroform);
            }
            let catalog = self.doc.add_object(catalog);
            self.doc.trailer.set("Root", catalog);
            self.doc.save(path).unwrap();
        }
    }

    fn standard_fixture(path: &Path) {
        let mut f = FormFixture::new();
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("name"), "V" => string("Ada") },
            [50.0, 700.0, 300.0, 720.0],
        );
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("notes"), "Ff" => 1 << 12 },
            [50.0, 620.0, 300.0, 690.0],
        );
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("code"), "Ff" => 1 << 24, "MaxLen" => 6 },
            [50.0, 590.0, 200.0, 610.0],
        );
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("locked"), "Ff" => 1 | (1 << 1), "V" => string("fixed") },
            [320.0, 700.0, 560.0, 720.0],
        );
        f.checkbox("agree", [50.0, 550.0, 66.0, 566.0], "On", "On");
        f.checkbox("newsletter", [100.0, 550.0, 116.0, 566.0], "On", "Off");
        f.radio(
            "color",
            &[
                ("Red", [50.0, 510.0, 66.0, 526.0]),
                ("Blue", [100.0, 510.0, 116.0, 526.0]),
            ],
            "Blue",
        );
        f.widget(
            dictionary! {
                "FT" => "Ch", "T" => string("country"), "Ff" => (1 << 17) | (1 << 18),
                "Opt" => vec![string("TR"), string("DE")], "V" => string("DE"),
            },
            [50.0, 470.0, 200.0, 490.0],
        );
        f.widget(
            dictionary! {
                "FT" => "Ch", "T" => string("langs"), "Ff" => 1 << 21,
                "Opt" => vec![string("English"), string("Deutsch"), string("Japanese")],
                "V" => vec![string("English"), string("Japanese")],
            },
            [50.0, 380.0, 200.0, 460.0],
        );
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("secret"), "F" => 2 },
            [320.0, 650.0, 560.0, 670.0],
        );
        f.save(path, true);
    }

    fn read(path: &Path) -> (FormInfo, Result<Vec<FormWidgetData>, FormReadError>) {
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(path, None)
                .map_err(|error| error.to_string())?;
            Ok((
                read_form_info(&document),
                read_page_form_fields(&document, 0),
            ))
        })
        .unwrap()
    }

    fn by_name<'a>(widgets: &'a [FormWidgetData], field: &str) -> Vec<&'a FormWidgetData> {
        widgets.iter().filter(|w| w.field_name == field).collect()
    }

    #[test]
    fn reads_kinds_values_flags_and_options() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("form.pdf");
        standard_fixture(&path);
        let (info, widgets) = read(&path);
        let widgets = widgets.unwrap();

        assert_eq!(info.form_type, FormType::AcroForm);
        assert_eq!(info.signature_count, 0);
        assert_eq!(widgets.len(), 11);
        assert!(widgets
            .windows(2)
            .all(|w| w[0].annot_index < w[1].annot_index));

        let name = by_name(&widgets, "name")[0];
        assert_eq!(name.kind, FormFieldKind::Text);
        assert_eq!(name.value.as_deref(), Some("Ada"));
        assert!(!name.read_only && !name.multiline && !name.hidden);
        assert!(by_name(&widgets, "notes")[0].multiline);
        assert_eq!(by_name(&widgets, "code")[0].kind, FormFieldKind::Text);
        let locked = by_name(&widgets, "locked")[0];
        assert!(locked.read_only && locked.required);
        assert!(by_name(&widgets, "secret")[0].hidden);

        let radios = by_name(&widgets, "color");
        assert_eq!(radios.len(), 2);
        assert!(radios.iter().all(|w| w.kind == FormFieldKind::RadioButton));
        assert_eq!(
            radios.iter().map(|w| w.checked).collect::<Vec<_>>(),
            vec![Some(false), Some(true)]
        );

        let country = by_name(&widgets, "country")[0];
        assert_eq!(country.kind, FormFieldKind::ComboBox);
        assert!(country.editable && !country.multi_select);
        assert_eq!(country.value.as_deref(), Some("DE"));
        assert_eq!(
            country.options,
            vec![
                FormFieldOption {
                    label: "TR".into(),
                    selected: false
                },
                FormFieldOption {
                    label: "DE".into(),
                    selected: true
                },
            ]
        );

        let langs = by_name(&widgets, "langs")[0];
        assert_eq!(langs.kind, FormFieldKind::ListBox);
        assert!(langs.multi_select);
        assert_eq!(
            langs.options.iter().map(|o| o.selected).collect::<Vec<_>>(),
            vec![true, false, true]
        );
    }

    #[test]
    fn checkbox_state_comes_from_appearance_state_not_yes() {
        // pdfium-render's is_checked() only accepts a "/Yes" appearance state.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("form.pdf");
        standard_fixture(&path);
        let widgets = read(&path).1.unwrap();
        assert_eq!(by_name(&widgets, "agree")[0].checked, Some(true));
        assert_eq!(by_name(&widgets, "newsletter")[0].checked, Some(false));
        assert!(is_on_state("/On") && is_on_state("Yes") && is_on_state("0"));
        assert!(!is_on_state("Off") && !is_on_state("/Off") && !is_on_state(""));
    }

    #[test]
    fn widget_rect_is_normalized_display_space() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("form.pdf");
        standard_fixture(&path);
        let widgets = read(&path).1.unwrap();
        let rect = by_name(&widgets, "name")[0]
            .normalized_rect
            .clone()
            .unwrap();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(rect.x, 50.0 / 612.0), "{rect:?}");
        assert!(close(rect.y, (792.0 - 720.0) / 792.0), "{rect:?}");
        assert!(close(rect.width, 250.0 / 612.0), "{rect:?}");
        assert!(close(rect.height, 20.0 / 792.0), "{rect:?}");
    }

    #[test]
    fn widgets_outside_acroform_fields_are_read() {
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
        let widgets = read(&path).1.unwrap();
        assert_eq!(
            widgets
                .iter()
                .map(|w| (w.field_name.as_str(), w.value.as_deref()))
                .collect::<Vec<_>>(),
            vec![("listed", Some("a")), ("orphan", Some("b"))]
        );
    }

    #[test]
    fn document_without_acroform_has_no_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plain.pdf");
        FormFixture::new().save(&path, false);
        let (info, widgets) = read(&path);
        assert_eq!(info.form_type, FormType::None);
        assert_eq!(widgets.unwrap(), Vec::new());
    }

    #[test]
    fn oversized_value_is_a_limit_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("big.pdf");
        let mut f = FormFixture::new();
        let long = "x".repeat(MAX_FORM_TEXT_CHARS + 1);
        f.widget(
            dictionary! { "FT" => "Tx", "T" => string("big"), "V" => string(&long) },
            [50.0, 700.0, 300.0, 720.0],
        );
        f.save(&path, true);
        assert!(matches!(
            read(&path).1,
            Err(FormReadError::LimitExceeded(_))
        ));
    }
}
