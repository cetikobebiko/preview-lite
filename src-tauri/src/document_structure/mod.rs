//! Belge düzeyindeki yapı düzenlemeleri: meta veri (Info + XMP), sayfa etiketleri ve yer imleri.
//!
//! PDFium'un public API'si bunları yalnız okur; yapı bu yüzden `lopdf` ile yazılır. Kullanım
//! AGENTS.md'nin sınırları içindedir: yalnız job worker'da, session snapshot'ının kopyası
//! üzerinde, `atomic_write_pdf` içinde çalışır; şifreli belgeler reddedilir. Yazım artımlıdır
//! (`lopdf::IncrementalDocument`): orijinal baytlar aynen kalır, değişen ve yeni nesneler sona
//! eklenir, böylece imzalı baytlar korunur. Kayıttan sonra çıktı PDFium ile yeniden açılır ve
//! sonuç PDFium'un okuduğuyla doğrulanır.

pub(crate) mod labels;
pub(crate) mod metadata;
pub(crate) mod outline;
mod xmp;

use crate::document_io::DocumentIoFailure;
use crate::incremental_save::{
    find_xref_inconsistency, read_appended_section, validate_appended_section, InputFingerprint,
};
use lopdf::{Dictionary, IncrementalDocument, Object, ObjectId};
use std::path::Path;

/// `lopdf` tüm dosyayı belleğe alır; bu boyutun üstündeki belgeler reddedilir.
pub const MAX_STRUCTURE_INPUT_BYTES: usize = 256 * 1024 * 1024;
/// Referans zincirlerinde en fazla adım.
const MAX_REFERENCE_DEPTH: usize = 32;

/// Bir yapı düzenlemesinin artımlı yazıcısı: önceki revizyonu okur, değişen nesneleri yeni
/// revizyona kopyalayıp düzenler ve [`StructureWriter::finish`] ile `temp`'e yazar.
pub(crate) struct StructureWriter {
    incremental: IncrementalDocument,
    input: InputFingerprint,
}

impl StructureWriter {
    /// `source`'u artımlı düzenleme için açar. `what` hata iletilerinde işlemin adıdır.
    pub(crate) fn load(source: &Path, what: &str) -> Result<Self, DocumentIoFailure> {
        let bytes = std::fs::read(source)
            .map_err(|error| DocumentIoFailure::preflight(format!("PDF okunamadı: {error}")))?;
        if bytes.len() > MAX_STRUCTURE_INPUT_BYTES {
            return Err(DocumentIoFailure::preflight(format!(
                "PDF {what} için çok büyük (256 MiB sınırı)"
            )));
        }
        if let Some(reason) = find_xref_inconsistency(&bytes) {
            return Err(DocumentIoFailure::preflight(format!(
                "Belgenin xref zinciri onarım gerektiriyor: {reason}"
            )));
        }
        let previous = lopdf::Document::load_mem(&bytes).map_err(|error| {
            DocumentIoFailure::preflight(format!("PDF yapısı ayrıştırılamadı: {error}"))
        })?;
        // lopdf decrypts empty-user-password files on load; the bytes on disk are still
        // encrypted, so plain incremental objects would corrupt them.
        if crate::lopdf_compat::was_encrypted(&previous) {
            return Err(DocumentIoFailure::preflight(format!(
                "Şifreli PDF'lerde {what} desteklenmiyor"
            )));
        }
        let version = previous.version.clone();
        let input = InputFingerprint::of(&bytes);
        let mut incremental = IncrementalDocument::create_from(bytes, previous);
        // Artımlı bölümün başına yazılan `%PDF-x.y` satırı yalnız bir yorumdur; sürümü koru.
        incremental.new_document.version = version;
        Ok(Self { incremental, input })
    }

    /// Önceki revizyonun sayfa nesneleri, belge sırasıyla.
    pub(crate) fn page_ids(&self) -> Vec<ObjectId> {
        self.incremental
            .get_prev_documents()
            .get_pages()
            .into_values()
            .collect()
    }

    /// Nesnenin güncel hali: yeni revizyonda değiştirildiyse o, değilse önceki.
    pub(crate) fn object(&self, id: ObjectId) -> Result<&Object, DocumentIoFailure> {
        self.incremental
            .new_document
            .get_object(id)
            .or_else(|_| self.incremental.get_prev_documents().get_object(id))
            .map_err(|_| {
                DocumentIoFailure::preflight(format!("Nesne {} {} bulunamadı", id.0, id.1))
            })
    }

    /// Referansları (sınırlı) izleyerek değeri çözer.
    pub(crate) fn resolve<'a>(
        &'a self,
        mut object: &'a Object,
    ) -> Result<&'a Object, DocumentIoFailure> {
        for _ in 0..MAX_REFERENCE_DEPTH {
            match object {
                Object::Reference(id) => object = self.object(*id)?,
                other => return Ok(other),
            }
        }
        Err(DocumentIoFailure::preflight("Referans zinciri çok derin"))
    }

    pub(crate) fn dict(&self, id: ObjectId) -> Result<&Dictionary, DocumentIoFailure> {
        self.object(id)?.as_dict().map_err(|_| {
            DocumentIoFailure::preflight(format!("Nesne {} {} sözlük değil", id.0, id.1))
        })
    }

    /// Belge kataloğu (`/Root`).
    pub(crate) fn catalog_id(&self) -> Result<ObjectId, DocumentIoFailure> {
        self.incremental
            .new_document
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| DocumentIoFailure::preflight("Belge kataloğu bulunamadı"))
    }

    /// Yeni revizyonun trailer'ı (önceki trailer'ın kopyası; `/Prev` lopdf'indir).
    pub(crate) fn trailer_mut(&mut self) -> &mut Dictionary {
        &mut self.incremental.new_document.trailer
    }

    pub(crate) fn trailer(&self) -> &Dictionary {
        &self.incremental.new_document.trailer
    }

    pub(crate) fn add(&mut self, object: impl Into<Object>) -> ObjectId {
        self.incremental.new_document.add_object(object)
    }

    /// Nesneyi yeni revizyonda `object` ile değiştirir.
    pub(crate) fn set(&mut self, id: ObjectId, object: impl Into<Object>) {
        self.incremental.new_document.set_object(id, object);
    }

    /// Nesneyi yeni revizyona kopyalayıp sözlük olarak düzenler.
    pub(crate) fn edit<R>(
        &mut self,
        id: ObjectId,
        f: impl FnOnce(&mut Dictionary) -> R,
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
        Ok(f(dict))
    }

    /// Artımlı güncellemeyi `temp`'e yazar ve orijinal baytların korunduğunu doğrular.
    pub(crate) fn finish(mut self, temp: &Path) -> Result<(), DocumentIoFailure> {
        {
            let file = std::fs::File::create(temp).map_err(|error| {
                DocumentIoFailure::write(format!("Temp PDF açılamadı: {error}"))
            })?;
            let mut output = std::io::BufWriter::new(file);
            self.incremental.save_to(&mut output).map_err(|error| {
                DocumentIoFailure::write(format!("Artımlı güncelleme yazılamadı: {error}"))
            })?;
            output
                .into_inner()
                .map_err(|error| error.into_error())
                .and_then(|file| file.sync_all())
                .map_err(|error| {
                    DocumentIoFailure::write(format!("Temp PDF yazılamadı: {error}"))
                })?;
        }
        let input = self.input;
        drop(self.incremental);
        let appended = read_appended_section(temp, &input)
            .map_err(|error| DocumentIoFailure::write(format!("Temp PDF okunamadı: {error}")))?
            .map_err(|_| {
                DocumentIoFailure::invariant("Artımlı güncelleme orijinal baytları korumadı")
            })?;
        validate_appended_section(&input, &appended)
    }
}

/// PDFium ile kaynak belgenin sayfa sayısı; lopdf'in sayfa ağacıyla aynı olmalıdır.
pub(crate) fn check_page_count(
    pdfium: &pdfium_render::prelude::Pdfium,
    source: &Path,
    writer: &StructureWriter,
) -> Result<usize, DocumentIoFailure> {
    let document = pdfium
        .load_pdf_from_file(source, None)
        .map_err(|error| DocumentIoFailure::preflight(format!("PDF açılamadı: {error}")))?;
    let pdfium_count = usize::from(document.pages().len());
    let lopdf_count = writer.page_ids().len();
    if pdfium_count != lopdf_count {
        return Err(DocumentIoFailure::preflight(format!(
            "Sayfa ağacında {lopdf_count} sayfa var, PDFium {pdfium_count} sayfa görüyor"
        )));
    }
    Ok(pdfium_count)
}

/// `value` bir metin dizesi olarak; çözülemezse `None`.
pub(crate) fn text_of(value: &Object) -> Option<String> {
    lopdf::decode_text_string(value).ok()
}

#[cfg(test)]
pub(crate) mod test_support {
    use lopdf::{dictionary, Document, Object, Stream};

    /// `page_count` boş sayfalı, xref tablolu, `extra` ile kataloğu genişletilmiş bir PDF.
    pub(crate) fn pdf_with(
        page_count: usize,
        extra: impl FnOnce(&mut Document, lopdf::ObjectId, &[lopdf::ObjectId]),
    ) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let content = doc.add_object(Stream::new(dictionary! {}, b"".to_vec()));
        let kids: Vec<_> = (0..page_count)
            .map(|_| {
                doc.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
                    "Contents" => content,
                })
            })
            .collect();
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(),
                "Count" => page_count as i64,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        extra(&mut doc, catalog, &kids);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    /// `bytes`'ı geçici dizinde bir dosyaya yazar.
    pub(crate) fn write_temp(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.pdf");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }
}
