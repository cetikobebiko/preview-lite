//! lopdf 0.45 davranış farklarını tek yerde karşılayan yardımcılar.
//!
//! lopdf 0.45 iki şeyi sessizce değiştirdi; ikisi de derleme hatası vermez:
//!
//! - `Document::load*`, `/Prev` zincirini okurken son trailer'dan `/Prev`'i (ve varsa
//!   `/XRefStm`'i) siler. `trailer.has(b"Prev")` her zaman false olur; "önceki revizyon
//!   var mı" ve "çıktı tamamen yeniden yazıldı mı" kontrolleri anlamını yitirir.
//!   [`trailer_history`] aynı bilgiyi ham baytlardan, son xref bölümünün trailer'ından
//!   (ya da xref stream sözlüğünden) okur.
//! - Kullanıcı parolası boş belgeler yüklenirken çözülür ve `/Encrypt` kaldırılır;
//!   `is_encrypted()` false döner. Şifreli belge korumaları ve raporlar
//!   [`was_encrypted`] kullanmalıdır.
//! - Parolası boş olmayan bir belge parolasız yüklenip sonra `decrypt` edilirse şifreli
//!   object stream'lerdeki nesneler (Catalog dahil) hiç yüklenmez. Parola yükleme sırasında
//!   verilmelidir: [`load_mem_with_credential`].

use lopdf::Document;

/// Belge şifreliydi: yüklenirken çözüldüyse de (0.45) hâlâ şifreliyse de true.
pub(crate) fn was_encrypted(document: &Document) -> bool {
    document.was_encrypted() || document.is_encrypted()
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CredentialedLoadError {
    /// lopdf bu baytları parolasız da ayrıştıramadı.
    Parse,
    /// Belge parola istiyor ama kimlik bilgisi verilmedi.
    CredentialRequired,
    /// Verilen parola belgeyi açmadı.
    CredentialRejected,
}

/// `bytes`'ı yükler; parola gerekiyorsa yükleme sırasında verir. Şifresiz ya da boş kullanıcı
/// parolalı (lopdf'in kendisi çözer) belgede kimlik bilgisine bakılmaz.
pub(crate) fn load_mem_with_credential(
    bytes: &[u8],
    credential: Option<&str>,
) -> Result<Document, CredentialedLoadError> {
    let document = Document::load_mem(bytes).map_err(|_| CredentialedLoadError::Parse)?;
    if !document.is_encrypted() {
        return Ok(document);
    }
    let password = credential.ok_or(CredentialedLoadError::CredentialRequired)?;
    // The bytes parsed without a password, so a failure now is the password's.
    Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(password))
        .map_err(|_| CredentialedLoadError::CredentialRejected)
}

/// Son revizyonun trailer'ındaki geçmiş göstergeleri.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TrailerHistory {
    /// Önceki bir xref bölümüne işaret eden `/Prev` var (artımlı güncelleme ya da linearization).
    pub prev: bool,
    /// Hibrit dosyadaki ek xref stream'ine işaret eden `/XRefStm` var.
    pub xref_stm: bool,
}

impl TrailerHistory {
    pub(crate) fn any(self) -> bool {
        self.prev || self.xref_stm
    }
}

/// `bytes` içindeki son xref bölümünün trailer anahtarları. `document`, `bytes`'tan
/// yüklenmiş olmalıdır (lopdf `xref_start`'ı düzeltilmiş haliyle tutar). Trailer
/// sözlüğü bulunamazsa `None`: çağıran güvenli tarafı seçmelidir.
pub fn trailer_history(bytes: &[u8], document: &Document) -> Option<TrailerHistory> {
    trailer_history_at(bytes, document.xref_start)
}

fn trailer_history_at(bytes: &[u8], xref_start: usize) -> Option<TrailerHistory> {
    let section = bytes.get(xref_start..)?;
    let section = &section[leading_whitespace(section)..];
    // Klasik tablo: "xref ... trailer << ... >>". Xref stream: "N G obj << ... >> stream".
    let dictionary = if section.starts_with(b"xref") {
        let trailer = find(section, b"trailer")?;
        let open = trailer + find(&section[trailer..], b"<<")?;
        &section[open..]
    } else {
        let open = find(section, b"<<")?;
        if !section[..open].windows(3).any(|window| window == b"obj") {
            return None;
        }
        &section[open..]
    };
    let keys = top_level_names(dictionary)?;
    Some(TrailerHistory {
        prev: keys.iter().any(|name| name == b"Prev"),
        xref_stm: keys.iter().any(|name| name == b"XRefStm"),
    })
}

fn leading_whitespace(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|byte| is_pdf_whitespace(**byte))
        .count()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Name tokens directly inside the outermost `<< >>` that `dictionary` starts with,
/// skipping nested dictionaries, arrays, strings and comments. Returns `None` when
/// the dictionary is not closed.
fn top_level_names(dictionary: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut names = Vec::new();
    let mut dict_depth = 0usize;
    let mut array_depth = 0usize;
    let mut index = 0usize;
    while index < dictionary.len() {
        let byte = dictionary[index];
        match byte {
            b'<' if dictionary.get(index + 1) == Some(&b'<') => {
                dict_depth += 1;
                index += 2;
            }
            b'>' if dictionary.get(index + 1) == Some(&b'>') => {
                dict_depth = dict_depth.checked_sub(1)?;
                index += 2;
                if dict_depth == 0 {
                    return Some(names);
                }
            }
            b'<' => {
                // Hex string.
                index += 1 + find(&dictionary[index..], b">")?;
            }
            b'(' => index = skip_literal_string(dictionary, index)?,
            b'%' => {
                while index < dictionary.len() && !matches!(dictionary[index], b'\r' | b'\n') {
                    index += 1;
                }
            }
            b'[' => {
                array_depth += 1;
                index += 1;
            }
            b']' => {
                array_depth = array_depth.saturating_sub(1);
                index += 1;
            }
            b'/' => {
                let start = index + 1;
                let mut end = start;
                while end < dictionary.len()
                    && !is_pdf_whitespace(dictionary[end])
                    && !is_delimiter(dictionary[end])
                {
                    end += 1;
                }
                if dict_depth == 1 && array_depth == 0 {
                    names.push(dictionary[start..end].to_vec());
                }
                index = end;
            }
            _ => index += 1,
        }
    }
    None
}

/// Index just past the literal string starting at `start` (an opening parenthesis).
fn skip_literal_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }
    None
}

/// A PDF with an owner password ("owner") and an empty user password, the case lopdf 0.45
/// decrypts while loading. Built with lopdf's own RC4-128 encryption (no qpdf needed).
#[cfg(test)]
pub(crate) fn owner_only_encrypted_pdf(page_count: usize) -> Vec<u8> {
    use lopdf::{dictionary, EncryptionState, EncryptionVersion, Object, Permissions, Stream};
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let kids = (0..page_count)
        .map(|index| {
            let content = format!("BT /F1 12 Tf 20 100 Td (page {index}) Tj ET");
            let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
            Object::Reference(document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
                "Contents" => content_id,
            }))
        })
        .collect::<Vec<_>>();
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => i64::try_from(page_count).unwrap(),
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    document.trailer.set(
        "ID",
        Object::Array(vec![
            Object::string_literal("owner-only-id"),
            Object::string_literal("owner-only-id"),
        ]),
    );
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner",
        user_password: "",
        key_length: 128,
        permissions: Permissions::all(),
    })
    .unwrap();
    document.encrypt(&state).unwrap();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, Stream};

    fn fixture(relative: &str) -> Vec<u8> {
        std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(relative),
        )
        .unwrap()
    }

    fn history(bytes: &[u8]) -> Option<TrailerHistory> {
        let document = Document::load_mem(bytes).unwrap();
        trailer_history(bytes, &document)
    }

    fn single_revision() -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
            "Contents" => content_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn single_revision_has_no_history() {
        let bytes = single_revision();
        assert_eq!(history(&bytes), Some(TrailerHistory::default()));
    }

    #[test]
    fn incremental_update_keeps_its_prev_although_lopdf_strips_it() {
        let base = single_revision();
        let previous = Document::load_mem(&base).unwrap();
        let mut incremental = lopdf::IncrementalDocument::create_from(base.clone(), previous);
        incremental
            .new_document
            .trailer
            .set("Comment", Object::string_literal("x"));
        let mut bytes = Vec::new();
        incremental.save_to(&mut bytes).unwrap();

        let document = Document::load_mem(&bytes).unwrap();
        // The behaviour this module exists for: lopdf 0.45 drops /Prev on load.
        assert!(!document.trailer.has(b"Prev"));
        assert_eq!(
            trailer_history(&bytes, &document),
            Some(TrailerHistory {
                prev: true,
                xref_stm: false
            })
        );
    }

    #[test]
    fn repository_incremental_fixtures_report_prev() {
        for relative in [
            "sanitization/incremental-revision.pdf",
            "redaction/incremental-revision.pdf",
            "sanitization/incremental-xref-stream-chain.pdf",
        ] {
            let bytes = fixture(relative);
            assert!(
                history(&bytes).is_some_and(|history| history.prev),
                "{relative}: {:?}",
                history(&bytes)
            );
        }
    }

    #[test]
    fn names_inside_nested_values_strings_and_comments_are_not_keys() {
        let dictionary = b"<< /Size 5 /Info << /Prev 1 >> /ID [<00> /Prev] \
            /Note (has /Prev \\) inside) % /Prev 9\n /Root 1 0 R >>";
        let names = top_level_names(dictionary).unwrap();
        let names = names
            .iter()
            .map(|name| String::from_utf8_lossy(name).to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Size", "Info", "ID", "Note", "Root"]);
    }

    #[test]
    fn xref_stream_dictionary_is_read() {
        let bytes = b"%PDF-1.7\n9 0 obj\n<< /Type /XRef /Size 10 /Prev 120 /W [1 2 1] >>\nstream\nx\nendstream\nendobj\n";
        let start = find(bytes, b"9 0 obj").unwrap();
        assert_eq!(
            trailer_history_at(bytes, start),
            Some(TrailerHistory {
                prev: true,
                xref_stm: false
            })
        );
    }

    #[test]
    fn unreadable_position_is_none() {
        assert_eq!(trailer_history_at(b"%PDF-1.7\n", 4), None);
        assert_eq!(
            trailer_history_at(b"xref\n0 1\ntrailer << /Size 1 ", 0),
            None
        );
        assert_eq!(trailer_history_at(b"short", 99), None);
    }

    #[test]
    fn owner_only_encryption_is_decrypted_on_load_but_still_reported() {
        let bytes = owner_only_encrypted_pdf(2);
        assert!(bytes
            .windows(b"/Encrypt".len())
            .any(|window| window == b"/Encrypt"));
        let document = Document::load_mem(&bytes).unwrap();
        // The silent change: no error, no /Encrypt, is_encrypted() false.
        assert!(!document.is_encrypted());
        assert!(was_encrypted(&document));
        assert_eq!(document.get_pages().len(), 2);
        let loaded = load_mem_with_credential(&bytes, None).unwrap();
        assert!(was_encrypted(&loaded));
    }

    #[test]
    fn plain_documents_load_without_a_credential() {
        let document = load_mem_with_credential(&single_revision(), None).unwrap();
        assert!(!was_encrypted(&document));
        assert!(document.catalog().is_ok());
    }

    #[test]
    fn encrypted_documents_need_their_password_at_load_time() {
        let bytes = fixture("sanitization/encrypted.pdf");
        // The regression this helper exists for: load, then decrypt, loses the catalog.
        let mut late = Document::load_mem(&bytes).unwrap();
        late.decrypt_raw(b"fixture-password").unwrap();
        assert!(late.catalog().is_err());

        assert_eq!(
            load_mem_with_credential(&bytes, None).unwrap_err(),
            CredentialedLoadError::CredentialRequired
        );
        assert_eq!(
            load_mem_with_credential(&bytes, Some("wrong-password")).unwrap_err(),
            CredentialedLoadError::CredentialRejected
        );
        let document = load_mem_with_credential(&bytes, Some("fixture-password")).unwrap();
        assert!(document.catalog().is_ok());
        assert!(!document.is_encrypted());
        assert!(was_encrypted(&document));
    }
}
