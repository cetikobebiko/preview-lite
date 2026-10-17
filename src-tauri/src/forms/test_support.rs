//! Form test fixture'ları: lopdf ile test içinde üretilen tek sayfalık AcroForm PDF'leri.

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::path::Path;

pub(crate) fn string(text: &str) -> Object {
    Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
}

pub(crate) fn name(text: &str) -> Object {
    Object::Name(text.as_bytes().to_vec())
}

pub(crate) fn rect(r: [f32; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
}

/// Tek sayfalık (612×792) AcroForm fixture'ı. `widgets` her biri kendi widget'ı
/// olan terminal alanlardır; radio grubu ayrıca eklenir.
pub(crate) struct FormFixture {
    doc: Document,
    page_id: ObjectId,
    font: ObjectId,
    annots: Vec<Object>,
    fields: Vec<Object>,
}

impl FormFixture {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn widget(&mut self, dict: Dictionary, r: [f32; 4]) {
        self.add_widget(dict, r, true);
    }

    /// A widget that only the page's `/Annots` reaches, not the form's `/Fields`.
    pub(crate) fn orphan_widget(&mut self, dict: Dictionary, r: [f32; 4]) {
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

    pub(crate) fn checkbox(&mut self, t: &str, r: [f32; 4], on: &str, state: &str) {
        let ap = self.on_off(on);
        self.widget(
            dictionary! { "FT" => "Btn", "T" => string(t), "V" => name(state), "AS" => name(state), "AP" => ap },
            r,
        );
    }

    pub(crate) fn radio(&mut self, t: &str, kids: &[(&str, [f32; 4])], selected: &str) {
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

    pub(crate) fn save(mut self, path: &Path, with_acroform: bool) {
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

pub(crate) fn standard_fixture(path: &Path) {
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
