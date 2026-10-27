//! Form test fixture'ları: lopdf ile test içinde üretilen tek sayfalık AcroForm PDF'leri.

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::path::Path;

pub(crate) fn string(text: &str) -> Object {
    Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
}

pub(crate) fn name(text: &str) -> Object {
    Object::Name(text.as_bytes().to_vec())
}

/// An additional-actions dictionary of JavaScript actions, e.g. `[("F", "AFNumber_Format(…)")]`.
pub(crate) fn scripts(events: &[(&str, &str)]) -> Dictionary {
    let mut aa = Dictionary::new();
    for (event, js) in events {
        aa.set(
            *event,
            dictionary! { "S" => "JavaScript", "JS" => string(js) },
        );
    }
    aa
}

pub(crate) fn rect(r: [f32; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
}

/// Tek sayfalık (612×792) AcroForm fixture'ı. `widgets` her biri kendi widget'ı
/// olan terminal alanlardır; radio grubu ayrıca eklenir.
pub(crate) struct FormFixture {
    /// Signature dictionary of a signed-looking signature field, used as `/Perms /DocMDP`
    /// when it carries a DocMDP transform.
    certification: Option<ObjectId>,
    signed: bool,
    doc: Document,
    /// Earlier pages, closed by [`FormFixture::next_page`], with their annotations.
    pages: Vec<(ObjectId, Vec<Object>)>,
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
            certification: None,
            signed: false,
            doc,
            pages: Vec::new(),
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

    /// Widgets added after this go on a new page.
    pub(crate) fn next_page(&mut self) {
        let page_id = self.doc.new_object_id();
        let previous = std::mem::replace(&mut self.page_id, page_id);
        self.pages
            .push((previous, std::mem::take(&mut self.annots)));
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

    /// A terminal field whose only widget is a separate kid dictionary without `/T`.
    pub(crate) fn field_with_kid_widget(&mut self, mut field: Dictionary, r: [f32; 4]) {
        let parent = self.doc.new_object_id();
        let kid = self.doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect(r), "F" => 4,
            "P" => self.page_id, "Parent" => parent, "DA" => string("/Helv 10 Tf 0 g"),
        });
        field.set("Kids", vec![Object::Reference(kid)]);
        self.doc.objects.insert(parent, Object::Dictionary(field));
        self.annots.push(Object::Reference(kid));
        self.fields.push(Object::Reference(parent));
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

    /// A signature field whose `/V` looks like a detached PKCS#7 signature (the bytes are not
    /// a real signature). `doc_mdp` adds a DocMDP transform with that `/P` and makes it the
    /// certification signature.
    pub(crate) fn signature_field(&mut self, doc_mdp: Option<i64>) {
        let mut signature = dictionary! {
            "Type" => "Sig", "Filter" => "Adobe.PPKLite", "SubFilter" => "adbe.pkcs7.detached",
            "ByteRange" => vec![0.into(), 100.into(), 200.into(), 300.into()],
            "Contents" => Object::String(vec![0x30, 0x82, 0x01, 0x00, 0xAB, 0xCD], StringFormat::Hexadecimal),
            "M" => string("D:20261017120000Z"),
        };
        if let Some(p) = doc_mdp {
            signature.set(
                "Reference",
                vec![Object::Dictionary(dictionary! {
                    "Type" => "SigRef", "TransformMethod" => "DocMDP",
                    "TransformParams" => dictionary! { "Type" => "TransformParams", "P" => p, "V" => name("1.2") },
                })],
            );
        }
        let signature = self.doc.add_object(signature);
        self.signed = true;
        if doc_mdp.is_some() {
            self.certification = Some(signature);
        }
        self.widget(
            dictionary! { "FT" => "Sig", "T" => string("signature"), "V" => signature, "F" => 132 },
            [0.0, 0.0, 0.0, 0.0],
        );
    }

    pub(crate) fn save(mut self, path: &Path, with_acroform: bool) {
        let pages_id = self.doc.new_object_id();
        let mut pages = std::mem::take(&mut self.pages);
        pages.push((self.page_id, std::mem::take(&mut self.annots)));
        let count = pages.len() as i64;
        let mut kids = Vec::with_capacity(pages.len());
        for (page_id, annots) in pages {
            let content = self.doc.add_object(Stream::new(dictionary! {}, Vec::new()));
            self.doc.objects.insert(
                page_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Page", "Parent" => pages_id,
                    "MediaBox" => rect([0.0, 0.0, 612.0, 792.0]), "Contents" => content,
                    "Annots" => annots,
                }),
            );
            kids.push(Object::Reference(page_id));
        }
        self.doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
        );
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if let Some(signature) = self.certification {
            catalog.set("Perms", dictionary! { "DocMDP" => signature });
        }
        if with_acroform {
            let mut acroform = dictionary! {
                "Fields" => self.fields.clone(),
                "DR" => dictionary! { "Font" => dictionary! { "Helv" => self.font } },
                "DA" => string("/Helv 0 Tf 0 g"),
            };
            if self.signed {
                // SigFlags: SignaturesExist | AppendOnly.
                acroform.set("SigFlags", 3);
            }
            let acroform = self.doc.add_object(acroform);
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
