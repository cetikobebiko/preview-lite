//! Faz 0 spike (docs/engineering/form-filling-plan.md): PDFium form-fill ortamı
//! üzerinden alan değerleri yazıldığında `/V`, `/AS` ve `/AP /N` birlikte
//! güncelleniyor mu? Atılabilir ölçüm kodu; production yolu değildir.
//!
//! Çalıştırma (src-tauri içinden):
//! `FORM_SPIKE_OUT=/tmp/x TAURI_RESOURCE_DIR=$PWD cargo test --test form_fill_spike -- --ignored --nocapture`

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use pdfium_render::prelude::*;
use preview_alternative_lib::get_pdfium;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::os::raw::{c_int, c_ulong};
use std::path::{Path, PathBuf};

const FPDF_ANNOT_WIDGET: i32 = 20;
const FIELD_TYPES: [&str; 9] = [
    "unknown",
    "pushbutton",
    "checkbox",
    "radio",
    "combo",
    "list",
    "text",
    "signature",
    "xfa",
];

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    doc: Document,
    dr: Dictionary,
    page_id: ObjectId,
    annots: Vec<Object>,
    fields: Vec<Object>,
}

fn pdf_string(text: &str) -> Object {
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

fn name(text: &str) -> Object {
    Object::Name(text.as_bytes().to_vec())
}

fn rect(r: [f32; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
}

impl Fixture {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let dr = dictionary! { "Font" => dictionary! { "Helv" => font } };
        let page_id = doc.new_object_id();
        Self {
            doc,
            dr,
            page_id,
            annots: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn appearance(&mut self, r: [f32; 4], content: &str) -> Object {
        let stream = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => rect([0.0, 0.0, r[2] - r[0], r[3] - r[1]]),
                "Resources" => self.dr.clone(),
            },
            content.as_bytes().to_vec(),
        );
        Object::Reference(self.doc.add_object(stream))
    }

    fn text_ap(&mut self, r: [f32; 4], value: &str) -> Object {
        let content = format!("/Tx BMC q BT /Helv 10 Tf 0 g 2 5 Td ({value}) Tj ET Q EMC");
        let n = self.appearance(r, &content);
        Object::Dictionary(dictionary! { "N" => n })
    }

    fn widget_base(&self, r: [f32; 4]) -> Dictionary {
        dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect(r), "F" => 4,
            "P" => self.page_id,
        }
    }

    /// Terminal field that is also its own widget.
    fn field(&mut self, mut dict: Dictionary, listed: bool) -> ObjectId {
        let id = self.doc.add_object(dict.clone());
        dict.set("Type", "Annot");
        self.annots.push(Object::Reference(id));
        if listed {
            self.fields.push(Object::Reference(id));
        }
        id
    }

    fn text(&mut self, t: &str, r: [f32; 4], flags: i64, value: Option<&str>, extra: Dictionary) {
        let mut d = self.widget_base(r);
        d.set("FT", "Tx");
        d.set("T", pdf_string(t));
        d.set("Ff", flags);
        d.set("DA", pdf_string("/Helv 10 Tf 0 g"));
        if let Some(v) = value {
            d.set("V", pdf_string(v));
            let ap = self.text_ap(r, v);
            d.set("AP", ap);
        }
        for (k, v) in extra.into_iter() {
            d.set(k, v);
        }
        self.field(d, t != "orphan");
    }

    fn on_off_ap(&mut self, r: [f32; 4], on: &str) -> Object {
        let w = r[2] - r[0];
        let on_ap = self.appearance(
            r,
            &format!("q 0 g 1 w 2 2 m {0} {0} l S 2 {0} m {0} 2 l S Q", w - 2.0),
        );
        let off_ap = self.appearance(r, "q 0.6 g 0 0 m Q");
        Object::Dictionary(dictionary! {
            "N" => dictionary! { on => on_ap, "Off" => off_ap },
        })
    }

    fn checkbox(&mut self, t: &str, r: [f32; 4], on: &str, opt: Option<&str>) {
        let mut d = self.widget_base(r);
        d.set("FT", "Btn");
        d.set("T", pdf_string(t));
        d.set("Ff", 0);
        d.set("V", name("Off"));
        d.set("AS", name("Off"));
        d.set("DA", pdf_string("/ZaDb 0 Tf 0 g"));
        let ap = self.on_off_ap(r, on);
        d.set("AP", ap);
        if let Some(export) = opt {
            d.set("Opt", Object::Array(vec![pdf_string(export)]));
        }
        self.field(d, true);
    }

    fn radio(&mut self, t: &str, kids: &[(&str, [f32; 4])], selected: &str) {
        let parent = self.doc.new_object_id();
        let mut kid_refs = Vec::new();
        for (state, r) in kids {
            let mut d = self.widget_base(*r);
            d.set("Parent", parent);
            d.set("AS", name(if *state == selected { state } else { "Off" }));
            let ap = self.on_off_ap(*r, state);
            d.set("AP", ap);
            let id = self.doc.add_object(d);
            self.annots.push(Object::Reference(id));
            kid_refs.push(Object::Reference(id));
        }
        let flags = (1 << 15) | (1 << 14); // Radio | NoToggleToOff
        self.doc.objects.insert(
            parent,
            Object::Dictionary(dictionary! {
                "FT" => "Btn", "T" => pdf_string(t), "Ff" => flags, "V" => name(selected),
                "Kids" => kid_refs, "DA" => pdf_string("/ZaDb 0 Tf 0 g"),
            }),
        );
        self.fields.push(Object::Reference(parent));
    }

    fn choice(&mut self, t: &str, r: [f32; 4], flags: i64, options: &[&str], value: Option<&str>) {
        let mut d = self.widget_base(r);
        d.set("FT", "Ch");
        d.set("T", pdf_string(t));
        d.set("Ff", flags);
        d.set("DA", pdf_string("/Helv 10 Tf 0 g"));
        d.set(
            "Opt",
            Object::Array(options.iter().map(|o| pdf_string(o)).collect()),
        );
        if let Some(v) = value {
            d.set("V", pdf_string(v));
            let ap = self.text_ap(r, v);
            d.set("AP", ap);
        }
        self.field(d, true);
    }

    fn finish(mut self, path: &Path) {
        let content = self.doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 9 Tf 50 740 Td (Form fill spike fixture) Tj ET".to_vec(),
        ));
        let pages_id = self.doc.new_object_id();
        let font_ref = self
            .dr
            .get(b"Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Helv")
            .unwrap()
            .clone();
        self.doc.objects.insert(
            self.page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => rect([0.0, 0.0, 612.0, 792.0]),
                "Contents" => content,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_ref } },
                "Annots" => self.annots.clone(),
            }),
        );
        self.doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![Object::Reference(self.page_id)], "Count" => 1,
            }),
        );
        let mut dr = self.dr.clone();
        let zadb = self.doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ZapfDingbats",
        });
        dr.get_mut(b"Font")
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("ZaDb", zadb);
        let acroform = self.doc.add_object(dictionary! {
            "Fields" => self.fields.clone(), "DR" => dr, "DA" => pdf_string("/Helv 0 Tf 0 g"),
        });
        let catalog = self.doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id, "AcroForm" => acroform,
        });
        self.doc.trailer.set("Root", catalog);
        self.doc.compress();
        self.doc.save(path).unwrap();
    }
}

fn build_fixture(path: &Path) {
    let mut f = Fixture::new();
    f.text(
        "name",
        [50.0, 700.0, 300.0, 720.0],
        0,
        Some("old"),
        dictionary! {},
    );
    f.text(
        "notes",
        [50.0, 620.0, 300.0, 690.0],
        1 << 12,
        None,
        dictionary! {},
    );
    f.text(
        "code",
        [50.0, 590.0, 200.0, 610.0],
        1 << 24,
        None,
        dictionary! { "MaxLen" => 6 },
    );
    f.text(
        "locked",
        [320.0, 700.0, 560.0, 720.0],
        1,
        Some("fixed"),
        dictionary! {},
    );
    f.text(
        "turkish",
        [320.0, 650.0, 560.0, 670.0],
        0,
        None,
        dictionary! {},
    );
    f.text(
        "rot",
        [320.0, 450.0, 345.0, 640.0],
        0,
        None,
        dictionary! { "MK" => dictionary! { "R" => 90 } },
    );
    f.text(
        "orphan",
        [320.0, 600.0, 560.0, 620.0],
        0,
        None,
        dictionary! {},
    );
    f.checkbox("agree", [50.0, 550.0, 66.0, 566.0], "On", None);
    f.checkbox(
        "optbox",
        [100.0, 550.0, 116.0, 566.0],
        "0",
        Some("Export-A"),
    );
    f.radio(
        "color",
        &[
            ("Red", [50.0, 510.0, 66.0, 526.0]),
            ("Green", [100.0, 510.0, 116.0, 526.0]),
            ("Blue", [150.0, 510.0, 166.0, 526.0]),
        ],
        "Red",
    );
    f.choice(
        "country",
        [50.0, 470.0, 200.0, 490.0],
        1 << 17,
        &["TR", "DE", "JP"],
        Some("TR"),
    );
    f.choice(
        "langs",
        [50.0, 380.0, 200.0, 460.0],
        1 << 21,
        &["English", "Deutsch", "Japanese"],
        None,
    );
    f.finish(path);
}

// ---------------------------------------------------------------------------
// Raw PDFium helpers
// ---------------------------------------------------------------------------

fn form_fill_info() -> Box<FPDF_FORMFILLINFO> {
    Box::new(FPDF_FORMFILLINFO {
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
    })
}

fn wide_string(fetch: impl Fn(*mut FPDF_WCHAR, c_ulong) -> c_ulong) -> String {
    let len = fetch(std::ptr::null_mut(), 0);
    if len <= 2 {
        return String::new();
    }
    let mut buffer = vec![0u16; (len as usize).div_ceil(2)];
    fetch(buffer.as_mut_ptr(), len);
    let end = buffer.iter().position(|u| *u == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[repr(C)]
struct VecWriter {
    base: FPDF_FILEWRITE,
    out: Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    // SAFETY: `this` is the first field of a live `VecWriter` (repr(C)) and
    // PDFium passes a readable buffer of `size` bytes.
    let writer = unsafe { &mut *(this as *mut VecWriter) };
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
    writer.out.extend_from_slice(bytes);
    1
}

struct FormDoc<'a> {
    b: &'a dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    form: FPDF_FORMHANDLE,
    _info: Box<FPDF_FORMFILLINFO>,
}

impl<'a> FormDoc<'a> {
    fn open(b: &'a dyn PdfiumLibraryBindings, path: &Path) -> Self {
        let doc = b.FPDF_LoadDocument(path.to_str().unwrap(), None);
        assert!(!doc.is_null(), "FPDF_LoadDocument failed");
        let mut info = form_fill_info();
        let form = b.FPDFDOC_InitFormFillEnvironment(doc, &mut *info);
        assert!(!form.is_null(), "FPDFDOC_InitFormFillEnvironment failed");
        Self {
            b,
            doc,
            form,
            _info: info,
        }
    }

    fn save(&self, flags: u32) -> Vec<u8> {
        let mut writer = Box::new(VecWriter {
            base: FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            out: Vec::new(),
        });
        let ok = self
            .b
            .FPDF_SaveAsCopy(self.doc, &mut writer.base, flags as FPDF_DWORD);
        assert!(self.b.is_true(ok), "FPDF_SaveAsCopy failed");
        std::mem::take(&mut writer.out)
    }
}

impl Drop for FormDoc<'_> {
    fn drop(&mut self) {
        self.b.FPDFDOC_ExitFormFillEnvironment(self.form);
        self.b.FPDF_CloseDocument(self.doc);
    }
}

#[derive(Debug, Clone)]
struct WidgetInfo {
    index: i32,
    name: String,
    kind: &'static str,
    value: String,
    export: String,
    checked: bool,
    flags: i32,
    rect: [f32; 4],
    selected: Vec<i32>,
}

fn widgets(fd: &FormDoc, page: FPDF_PAGE) -> Vec<WidgetInfo> {
    let b = fd.b;
    let mut out = Vec::new();
    for index in 0..b.FPDFPage_GetAnnotCount(page) {
        let annot = b.FPDFPage_GetAnnot(page, index);
        if b.FPDFAnnot_GetSubtype(annot) == FPDF_ANNOT_WIDGET {
            let mut r = FS_RECTF {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            };
            b.FPDFAnnot_GetRect(annot, &mut r);
            let kind = b.FPDFAnnot_GetFormFieldType(fd.form, annot);
            let option_count = b.FPDFAnnot_GetOptionCount(fd.form, annot);
            out.push(WidgetInfo {
                index,
                name: wide_string(|p, l| b.FPDFAnnot_GetFormFieldName(fd.form, annot, p, l)),
                kind: FIELD_TYPES.get(kind as usize).copied().unwrap_or("?"),
                value: wide_string(|p, l| b.FPDFAnnot_GetFormFieldValue(fd.form, annot, p, l)),
                export: wide_string(|p, l| {
                    b.FPDFAnnot_GetFormFieldExportValue(fd.form, annot, p, l)
                }),
                checked: b.is_true(b.FPDFAnnot_IsChecked(fd.form, annot)),
                flags: b.FPDFAnnot_GetFormFieldFlags(fd.form, annot),
                rect: [r.left, r.bottom, r.right, r.top],
                selected: (0..option_count.max(0))
                    .filter(|i| b.is_true(b.FPDFAnnot_IsOptionSelected(fd.form, annot, *i)))
                    .collect(),
            });
        }
        b.FPDFPage_CloseAnnot(annot);
    }
    out
}

enum Edit {
    Text(&'static str),
    Toggle,
    SelectIndices(&'static [i32]),
}

fn apply(fd: &FormDoc, page: FPDF_PAGE, w: &WidgetInfo, edit: &Edit) -> String {
    let b = fd.b;
    let annot = b.FPDFPage_GetAnnot(page, w.index);
    let focused = b.is_true(b.FORM_SetFocusedAnnot(fd.form, annot));
    let mut note = format!("focus={focused}");
    match edit {
        Edit::Text(text) => {
            let all = b.is_true(b.FORM_SelectAllText(fd.form, page));
            let wtext = wide(text);
            b.FORM_ReplaceSelection(fd.form, page, wtext.as_ptr());
            note.push_str(&format!(" selectAll={all}"));
        }
        Edit::Toggle => {
            let before = b.is_true(b.FPDFAnnot_IsChecked(fd.form, annot));
            let ch = b.is_true(b.FORM_OnChar(fd.form, page, ' ' as c_int, 0));
            let after_char = b.is_true(b.FPDFAnnot_IsChecked(fd.form, annot));
            note.push_str(&format!(
                " onChar(space)={ch} checked {before}->{after_char}"
            ));
            if before == after_char {
                let x = f64::from(w.rect[0] + w.rect[2]) / 2.0;
                let y = f64::from(w.rect[1] + w.rect[3]) / 2.0;
                let down = b.is_true(b.FORM_OnLButtonDown(fd.form, page, 0, x, y));
                let up = b.is_true(b.FORM_OnLButtonUp(fd.form, page, 0, x, y));
                let after_click = b.is_true(b.FPDFAnnot_IsChecked(fd.form, annot));
                note.push_str(&format!(
                    " | click down={down} up={up} checked->{after_click}"
                ));
            }
        }
        Edit::SelectIndices(indices) => {
            for i in *indices {
                let ok = b.is_true(b.FORM_SetIndexSelected(fd.form, page, *i, 1));
                note.push_str(&format!(" setIndex({i})={ok}"));
            }
        }
    }
    let killed = b.is_true(b.FORM_ForceToKillFocus(fd.form));
    note.push_str(&format!(" kill={killed}"));
    b.FPDFPage_CloseAnnot(annot);
    note
}

// ---------------------------------------------------------------------------
// Independent inspection with lopdf
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct RawWidget {
    field: String,
    v: String,
    a_s: String,
    ap_n_digest: String,
    ap_n_preview: String,
    need_appearances: bool,
}

fn obj_text(o: &Object) -> String {
    match o {
        Object::Name(n) => format!("/{}", String::from_utf8_lossy(n)),
        Object::String(s, _) if s.starts_with(&[0xFE, 0xFF]) => {
            let units: Vec<u16> = s[2..]
                .chunks(2)
                .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        Object::String(s, _) => String::from_utf8_lossy(s).into_owned(),
        Object::Array(items) => format!(
            "[{}]",
            items.iter().map(obj_text).collect::<Vec<_>>().join(", ")
        ),
        other => format!("{other:?}"),
    }
}

fn inherited<'d>(doc: &'d Document, mut dict: &'d Dictionary, key: &[u8]) -> Option<&'d Object> {
    for _ in 0..32 {
        if let Ok(v) = dict.get(key) {
            return Some(doc.dereference(v).ok()?.1);
        }
        dict = doc
            .dereference(dict.get(b"Parent").ok()?)
            .ok()?
            .1
            .as_dict()
            .ok()?;
    }
    None
}

fn qualified(doc: &Document, dict: &Dictionary) -> String {
    let mut parts = Vec::new();
    let mut current = Some(dict);
    while let Some(d) = current {
        if let Ok(t) = d.get(b"T") {
            parts.push(obj_text(t));
        }
        current = d
            .get(b"Parent")
            .ok()
            .and_then(|p| doc.dereference(p).ok())
            .and_then(|(_, o)| o.as_dict().ok());
    }
    parts.reverse();
    parts.join(".")
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(bytes);
    hash[..6].iter().map(|b| format!("{b:02x}")).collect()
}

fn raw_widgets(path: &Path) -> Vec<RawWidget> {
    let doc = Document::load(path).unwrap();
    let catalog = doc.catalog().unwrap();
    let acroform = doc
        .dereference(catalog.get(b"AcroForm").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let need = acroform
        .get(b"NeedAppearances")
        .map(|o| matches!(o, Object::Boolean(true)))
        .unwrap_or(false);
    let page_id = *doc.get_pages().get(&1).unwrap();
    let page = doc.get_dictionary(page_id).unwrap();
    let annots = doc
        .dereference(page.get(b"Annots").unwrap())
        .unwrap()
        .1
        .as_array()
        .unwrap();
    annots
        .iter()
        .map(|a| {
            let d = doc.dereference(a).unwrap().1.as_dict().unwrap();
            let (ap_n_digest, ap_n_preview) = d
                .get(b"AP")
                .ok()
                .and_then(|ap| doc.dereference(ap).ok()?.1.as_dict().ok())
                .and_then(|ap| doc.dereference(ap.get(b"N").ok()?).ok())
                .map(|(_, n)| match n {
                    Object::Stream(s) => {
                        let content = s
                            .decompressed_content()
                            .unwrap_or_else(|_| s.content.clone());
                        let preview: String = String::from_utf8_lossy(&content)
                            .replace('\n', " ")
                            .chars()
                            .take(160)
                            .collect();
                        (digest(&content), preview)
                    }
                    Object::Dictionary(states) => {
                        let mut out = Vec::new();
                        for (k, v) in states.iter() {
                            let bytes = match doc.dereference(v).map(|(_, o)| o) {
                                Ok(Object::Stream(s)) => s
                                    .decompressed_content()
                                    .unwrap_or_else(|_| s.content.clone()),
                                _ => Vec::new(),
                            };
                            out.push(format!("{}:{}", String::from_utf8_lossy(k), digest(&bytes)));
                        }
                        (out.join(" "), String::from("(state dict)"))
                    }
                    _ => (String::from("?"), String::new()),
                })
                .unwrap_or_else(|| (String::from("-"), String::new()));
            RawWidget {
                field: qualified(&doc, d),
                v: inherited(&doc, d, b"V").map(obj_text).unwrap_or_default(),
                a_s: d.get(b"AS").map(obj_text).unwrap_or_default(),
                ap_n_digest,
                ap_n_preview,
                need_appearances: need,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The spike
// ---------------------------------------------------------------------------

fn render_png(pdfium: &Pdfium, pdf: &Path, png: &Path, form_data: bool) {
    let document = pdfium.load_pdf_from_file(pdf, None).unwrap();
    let page = document.pages().get(0).unwrap();
    let config = PdfRenderConfig::new()
        .set_target_width(1224)
        .render_form_data(form_data);
    page.render_with_config(&config)
        .unwrap()
        .as_image()
        .save(png)
        .unwrap();
}

fn out_dir() -> (PathBuf, Option<tempfile::TempDir>) {
    match std::env::var_os("FORM_SPIKE_OUT") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            (dir, None)
        }
        None => {
            let temp = tempfile::tempdir().unwrap();
            (temp.path().to_path_buf(), Some(temp))
        }
    }
}

#[test]
#[ignore = "Faz 0 form-fill spike; manual measurement"]
fn form_fill_spike() {
    let (dir, _guard) = out_dir();
    let input = dir.join("fixture.pdf");
    build_fixture(&input);

    let pdfium = get_pdfium().unwrap();
    let b = pdfium.bindings();

    let edits: BTreeMap<&str, Edit> = BTreeMap::from([
        ("name", Edit::Text("Ada Lovelace")),
        ("notes", Edit::Text("line one\rline two")),
        ("code", Edit::Text("ABCDEFGH")),
        ("locked", Edit::Text("hacked")),
        ("turkish", Edit::Text("Çağrı Şişli ığüö")),
        ("rot", Edit::Text("rotated")),
        ("orphan", Edit::Text("orphan value")),
        ("agree", Edit::Toggle),
        ("optbox", Edit::Toggle),
        ("country", Edit::SelectIndices(&[2])),
        ("langs", Edit::SelectIndices(&[0, 2])),
    ]);

    let before_raw = raw_widgets(&input);
    let (full, incremental, before, after_in_session, notes) = {
        let fd = FormDoc::open(b, &input);
        println!("form_type = {}", b.FPDF_GetFormType(fd.doc));
        let page = b.FPDF_LoadPage(fd.doc, 0);
        b.FORM_OnAfterLoadPage(page, fd.form);
        let before = widgets(&fd, page);
        let mut notes = Vec::new();
        for w in &before {
            if let Some(edit) = edits.get(w.name.as_str()) {
                notes.push((w.name.clone(), apply(&fd, page, w, edit)));
            }
        }
        // Radio: select the "Blue" widget (third kid).
        if let Some(blue) = before.iter().filter(|w| w.name == "color").nth(2) {
            notes.push(("color[Blue]".into(), apply(&fd, page, blue, &Edit::Toggle)));
        }
        let after = widgets(&fd, page);
        b.FORM_OnBeforeClosePage(page, fd.form);
        b.FPDF_ClosePage(page);
        (fd.save(0), fd.save(1), before, after, notes)
    };

    let full_path = dir.join("filled-full.pdf");
    let inc_path = dir.join("filled-incremental.pdf");
    std::fs::write(&full_path, &full).unwrap();
    std::fs::write(&inc_path, &incremental).unwrap();
    let original = std::fs::read(&input).unwrap();
    println!(
        "sizes: original={} full={} incremental={} incremental_preserves_prefix={}",
        original.len(),
        full.len(),
        incremental.len(),
        incremental.starts_with(&original)
    );

    println!("\n== edit notes (in-session) ==");
    for (n, note) in &notes {
        println!("{n:<12} {note}");
    }

    println!("\n== PDFium in-session: before -> after ==");
    for (bw, aw) in before.iter().zip(&after_in_session) {
        println!(
            "{:<8} {:<9} flags={:<8} value {:?} -> {:?} | checked {} -> {} | export {:?} | sel {:?} -> {:?}",
            bw.name, bw.kind, bw.flags, bw.value, aw.value, bw.checked, aw.checked, bw.export, bw.selected, aw.selected
        );
    }

    for (label, path) in [("full", &full_path), ("incremental", &inc_path)] {
        println!("\n== reopened ({label}): fresh PDFium ==");
        let fd = FormDoc::open(b, path);
        let page = b.FPDF_LoadPage(fd.doc, 0);
        b.FORM_OnAfterLoadPage(page, fd.form);
        for w in widgets(&fd, page) {
            println!(
                "{:<8} value={:?} checked={} sel={:?}",
                w.name, w.value, w.checked, w.selected
            );
        }
        b.FORM_OnBeforeClosePage(page, fd.form);
        b.FPDF_ClosePage(page);

        println!("\n== reopened ({label}): lopdf /V /AS /AP ==");
        let after_raw = raw_widgets(path);
        for (br, ar) in before_raw.iter().zip(&after_raw) {
            let ap_changed = br.ap_n_digest != ar.ap_n_digest;
            println!(
                "{:<8} V {:?} -> {:?} | AS {:?} -> {:?} | AP changed={} [{} -> {}] NeedAppearances={}",
                ar.field, br.v, ar.v, br.a_s, ar.a_s, ap_changed, br.ap_n_digest, ar.ap_n_digest, ar.need_appearances
            );
            if ap_changed && !ar.ap_n_preview.is_empty() && ar.ap_n_preview != "(state dict)" {
                println!("         AP: {}", ar.ap_n_preview);
            }
        }
    }

    render_png(&pdfium, &input, &dir.join("before-ap-only.png"), false);
    render_png(
        &pdfium,
        &full_path,
        &dir.join("after-full-ap-only.png"),
        false,
    );
    render_png(
        &pdfium,
        &full_path,
        &dir.join("after-full-ffldraw.png"),
        true,
    );
    println!("\nartifacts: {}", dir.display());
}
