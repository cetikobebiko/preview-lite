//! Belge meta verisi düzenleme: `/Info`'nun Title, Author, Subject ve Keywords girdileri.
//!
//! Katalog bir XMP paketi (`/Root /Metadata`) taşıyorsa değişen her girdi pakette de güncellenir
//! (bkz. [`super::xmp`]); yoksa paket eklenmez. Creator ve Producer belgeyi üreten uygulamayı
//! anlatır ve burada değiştirilmez. Sonuç PDFium'un `FPDF_GetMetaText` okumasıyla doğrulanır.

use super::{text_of, xmp, StructureWriter};
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
    DocumentIoStage,
};
use lopdf::{Dictionary, Object, Stream};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Tek bir meta veri değeri için karakter sınırı.
pub const MAX_METADATA_VALUE_CHARS: usize = 32 * 1024;
/// Okunup düzenlenecek XMP paketi için çözülmüş bayt sınırı (paketler birkaç kB olur).
const MAX_XMP_BYTES: usize = 16 * 1024 * 1024;

const OPERATION: &str = "set_document_metadata";
const WHAT: &str = "meta veri düzenleme";

/// İstenen meta veri. Boş (ya da yalnız boşluk) değer girdiyi kaldırır.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct DocumentMetadataEdit {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
}

impl DocumentMetadataEdit {
    /// `/Info` anahtarları, PDFium etiketleri ve istenen (kırpılmış) değerleri.
    fn entries(&self) -> [(&'static str, PdfDocumentMetadataTagType, &str); 4] {
        [
            (
                "Title",
                PdfDocumentMetadataTagType::Title,
                self.title.trim(),
            ),
            (
                "Author",
                PdfDocumentMetadataTagType::Author,
                self.author.trim(),
            ),
            (
                "Subject",
                PdfDocumentMetadataTagType::Subject,
                self.subject.trim(),
            ),
            (
                "Keywords",
                PdfDocumentMetadataTagType::Keywords,
                self.keywords.trim(),
            ),
        ]
    }
}

/// Belgeden bağımsız istek denetimi.
pub(crate) fn validate_request(edit: &DocumentMetadataEdit) -> Result<(), String> {
    for (key, _, value) in edit.entries() {
        if value.chars().count() > MAX_METADATA_VALUE_CHARS {
            return Err(format!(
                "{key} değeri {MAX_METADATA_VALUE_CHARS} karakter sınırını aşıyor"
            ));
        }
        if value.contains('\0') {
            return Err(format!("{key} değeri NUL karakteri içeremez"));
        }
    }
    Ok(())
}

/// `file_path`'teki PDF'in meta verisini yerinde, atomik ve artımlı olarak değiştirir.
pub(crate) fn set_metadata_report(
    file_path: String,
    edit: DocumentMetadataEdit,
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
    validate_request(&edit).map_err(preflight)?;
    if password.is_some() {
        return Err(preflight(format!(
            "Şifreli PDF'lerde {WHAT} desteklenmiyor"
        )));
    }
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    let expected = edit.clone();
    atomic_write_pdf(
        OPERATION,
        std::slice::from_ref(&path),
        &path,
        move |_, temp| write_metadata(&source, &edit, temp),
        move |document, ()| verify_saved(document, &expected),
    )
}

fn write_metadata(
    source: &Path,
    edit: &DocumentMetadataEdit,
    temp: &Path,
) -> Result<(), DocumentIoFailure> {
    let mut writer = StructureWriter::load(source, WHAT)?;
    let (info_id, mut info) = match writer.trailer().get(b"Info").ok().cloned() {
        Some(Object::Reference(id)) => (Some(id), writer.dict(id)?.clone()),
        Some(Object::Dictionary(dict)) => (None, dict),
        _ => (None, Dictionary::new()),
    };
    let mut changed = Vec::new();
    for (key, _, value) in edit.entries() {
        let current = info
            .get(key.as_bytes())
            .ok()
            .and_then(|object| writer.resolve(object).ok())
            .and_then(text_of)
            .unwrap_or_default();
        if current == value {
            continue;
        }
        if value.is_empty() {
            info.remove(key.as_bytes());
        } else {
            info.set(key, lopdf::text_string(value));
        }
        changed.push((key, value));
    }
    if changed.is_empty() {
        return Err(DocumentIoFailure::preflight("Meta veride değişiklik yok"));
    }
    match info_id {
        Some(id) => writer.set(id, Object::Dictionary(info)),
        None => {
            let id = writer.add(Object::Dictionary(info));
            writer.trailer_mut().set("Info", Object::Reference(id));
        }
    }
    sync_xmp(&mut writer, &changed)?;
    writer.finish(temp)
}

/// Kataloğun XMP paketini değişen girdilerle uyumlu hale getirir. Paket yoksa, dolaylı bir
/// stream değilse ya da güvenle düzenlenemiyorsa (UTF-8 değil, çözülemiyor, çok büyük) olduğu
/// gibi kalır.
fn sync_xmp(
    writer: &mut StructureWriter,
    changed: &[(&str, &str)],
) -> Result<(), DocumentIoFailure> {
    let catalog = writer.catalog_id()?;
    let Ok(Object::Reference(id)) = writer.dict(catalog)?.get(b"Metadata").cloned() else {
        return Ok(());
    };
    let Ok(Object::Stream(stream)) = writer.object(id).cloned() else {
        return Ok(());
    };
    let Ok(bytes) = stream.get_plain_content_with_limit(MAX_XMP_BYTES) else {
        return Ok(());
    };
    let Ok(mut packet) = String::from_utf8(bytes) else {
        return Ok(());
    };
    let mut updated = false;
    for (key, value) in changed {
        if let Some(next) = xmp::sync_info(&packet, key, value) {
            packet = next;
            updated = true;
        }
    }
    if !updated {
        return Ok(());
    }
    // Written unfiltered, as PDF/A asks of metadata streams, so other tools can read it.
    let mut dict = stream.dict.clone();
    for key in [&b"Filter"[..], b"DecodeParms", b"DL", b"Length"] {
        dict.remove(key);
    }
    writer.set(
        id,
        Stream::new(dict, packet.into_bytes()).with_compression(false),
    );
    Ok(())
}

fn verify_saved(
    document: &PdfDocument<'_>,
    expected: &DocumentMetadataEdit,
) -> Result<(), DocumentIoFailure> {
    let metadata = document.metadata();
    for (key, tag, value) in expected.entries() {
        let actual = metadata
            .get(tag)
            .map(|tag| tag.value().to_string())
            .unwrap_or_default();
        if actual.trim() != value {
            return Err(DocumentIoFailure::invariant(format!(
                "Kaydedilen {key} değeri beklenen değil: '{actual}'"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{pdf_with, write_temp};
    use super::*;
    use lopdf::dictionary;

    const PACKET: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\
<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Eski başlık</rdf:li></rdf:Alt></dc:title>\
<dc:creator><rdf:Seq><rdf:li>Eski Yazar</rdf:li></rdf:Seq></dc:creator>\
</rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>";

    fn edit(title: &str, author: &str, subject: &str, keywords: &str) -> DocumentMetadataEdit {
        DocumentMetadataEdit {
            title: title.into(),
            author: author.into(),
            subject: subject.into(),
            keywords: keywords.into(),
        }
    }

    fn set(path: &Path, edit: DocumentMetadataEdit) -> Result<AtomicWriteReport, DocumentIoError> {
        set_metadata_report(path.to_string_lossy().to_string(), edit, None)
    }

    fn info(path: &Path) -> Vec<(String, String)> {
        let doc = lopdf::Document::load(path).unwrap();
        let info = doc.trailer.get(b"Info").unwrap().as_reference().unwrap();
        let dict = doc.get_dictionary(info).unwrap();
        let mut out: Vec<_> = dict
            .iter()
            .map(|(key, value)| {
                (
                    String::from_utf8_lossy(key).into_owned(),
                    text_of(value).unwrap_or_default(),
                )
            })
            .collect();
        out.sort();
        out
    }

    fn packet(path: &Path) -> String {
        let doc = lopdf::Document::load(path).unwrap();
        let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let id = doc
            .get_dictionary(catalog)
            .unwrap()
            .get(b"Metadata")
            .unwrap()
            .as_reference()
            .unwrap();
        let stream = doc.get_object(id).unwrap().as_stream().unwrap();
        String::from_utf8(stream.content.clone()).unwrap()
    }

    #[test]
    fn info_is_created_and_appended_incrementally() {
        let bytes = pdf_with(1, |_, _, _| {});
        let (_dir, path) = write_temp(&bytes);
        set(
            &path,
            edit("Çeyrek Raporu", "Şule Ağaoğlu", "", "bütçe, İzmir"),
        )
        .unwrap();
        let output = std::fs::read(&path).unwrap();
        assert!(
            output.starts_with(&bytes),
            "original bytes must stay in place"
        );
        assert_eq!(
            info(&path),
            vec![
                ("Author".to_string(), "Şule Ağaoğlu".to_string()),
                ("Keywords".to_string(), "bütçe, İzmir".to_string()),
                ("Title".to_string(), "Çeyrek Raporu".to_string()),
            ]
        );
    }

    #[test]
    fn existing_entries_are_kept_replaced_or_removed() {
        let bytes = pdf_with(1, |doc, _, _| {
            let info = doc.add_object(dictionary! {
                "Title" => Object::string_literal("Old"),
                "Subject" => Object::string_literal("Gone"),
                "Producer" => Object::string_literal("Writer"),
            });
            doc.trailer.set("Info", info);
        });
        let (_dir, path) = write_temp(&bytes);
        set(&path, edit("New", "", "  ", "")).unwrap();
        assert_eq!(
            info(&path),
            vec![
                ("Producer".to_string(), "Writer".to_string()),
                ("Title".to_string(), "New".to_string()),
            ]
        );
    }

    #[test]
    fn xmp_packet_follows_the_changed_entries() {
        let bytes = pdf_with(1, |doc, catalog, _| {
            let xmp = doc.add_object(Stream::new(
                dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
                PACKET.as_bytes().to_vec(),
            ));
            doc.get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Metadata", xmp);
            let info =
                doc.add_object(dictionary! { "Author" => Object::string_literal("Eski Yazar") });
            doc.trailer.set("Info", info);
        });
        let (_dir, path) = write_temp(&bytes);
        // Clearing the author removes it from both places; the new keywords get a description.
        set(&path, edit("Yeni & <başlık>", "", "", "anahtar")).unwrap();
        let packet = packet(&path);
        assert!(
            packet.contains("<rdf:li xml:lang=\"x-default\">Yeni &amp; &lt;başlık&gt;</rdf:li>"),
            "{packet}"
        );
        assert!(!packet.contains("dc:creator"), "{packet}");
        assert!(
            packet.contains("<pdf:Keywords>anahtar</pdf:Keywords>"),
            "{packet}"
        );
        assert!(packet.ends_with("<?xpacket end=\"w\"?>"), "{packet}");
    }

    #[test]
    fn an_unchanged_request_is_rejected_without_touching_the_file() {
        let bytes = pdf_with(1, |doc, _, _| {
            let info = doc.add_object(dictionary! { "Title" => Object::string_literal("Same") });
            doc.trailer.set("Info", info);
        });
        let (_dir, path) = write_temp(&bytes);
        let error = set(&path, edit("Same", "", "", "")).unwrap_err();
        assert!(error.detail.contains("değişiklik yok"), "{}", error.detail);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn oversized_and_nul_values_are_rejected() {
        let long = "a".repeat(MAX_METADATA_VALUE_CHARS + 1);
        assert!(validate_request(&edit(&long, "", "", "")).is_err());
        assert!(validate_request(&edit("", "a\0b", "", "")).is_err());
        assert!(validate_request(&edit("ok", "", "", "")).is_ok());
    }
}
