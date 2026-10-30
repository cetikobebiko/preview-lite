//! Parola ile koruma ve güvenliği kaldırma. Oturum belgesinin şifreli ya da şifresiz bir
//! kopyası üretilir; oturumdaki dosya değişmez.
//!
//! PDFium belgeyi (gerekiyorsa oturumun parolasıyla) açar ve `FPDF_REMOVE_SECURITY` ile
//! şifresiz bir kopya yazar. lopdf yalnız bu kopyayı şifreleyip serileştirir (AGENTS.md:
//! serialization/writing); kullanıcının PDF'ini kendisi ayrıştırmaz. Çıktı PDFium ile yeniden
//! açılır: sayfa sayısı, parolalar ve izinler istenenle karşılaştırılır.
//!
//! Kopya tam yeniden yazımdır; varsa dijital imzalar kopyada geçersiz olur.

use crate::incremental_save::{save_copy, RawDocument};
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter};
use lopdf::{EncryptionState, EncryptionVersion, Object, Permissions, StringFormat};
use pdfium_render::prelude::PdfiumLibraryBindings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// `FPDF_SaveAsCopy`: şifrelemeyi kaldırarak tam yeniden yazım.
const FPDF_REMOVE_SECURITY: pdfium_render::prelude::FPDF_DWORD = 3;
/// Eski yöntemlerin (RC4, AES-128) parola sınırı; standart güvenlik işleyicisi 32 bayt kullanır.
const MAX_LEGACY_PASSWORD_BYTES: usize = 32;
/// AES-256 (R6) parolaları UTF-8 olarak en fazla 127 bayttır.
const MAX_AES256_PASSWORD_BYTES: usize = 127;
/// Güvenlik ayarlarını değiştirmek için açılan belgede bulunması gereken izinler: yazdırma,
/// değiştirme, kopyalama, açıklama, form doldurma, erişilebilirlik, birleştirme, yüksek
/// kaliteli yazdırma. Sahip parolasıyla açılan belgede PDFium hepsini verir.
const OWNER_EQUIVALENT_PERMISSIONS: u32 = 0xF3C;

/// PDFium izin değeri (`FPDF_DWORD`: Windows'ta `u32`, diğerlerinde `u64`) ya da lopdf bitleri;
/// izin bitleri alttaki 32 bittedir.
fn permission_bits(value: impl Into<u64>) -> u32 {
    (value.into() & 0xFFFF_FFFF) as u32
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionMethod {
    /// AES-256 (PDF 2.0, R6). Acrobat X ve sonrası.
    Aes256,
    /// AES-128 (PDF 1.6, R4). Acrobat 7 ve sonrası.
    Aes128,
    /// RC4 128 bit (PDF 1.4, R3). Eski okuyucular için.
    Rc4_128,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrintPermission {
    None,
    LowResolution,
    HighResolution,
}

/// Parolasız (kullanıcı parolasıyla ya da parolasız) açan kişiye verilen izinler.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionSet {
    pub print: PrintPermission,
    pub modify: bool,
    pub copy: bool,
    pub annotate: bool,
    pub fill_forms: bool,
    pub assemble: bool,
}

impl PermissionSet {
    fn is_unrestricted(&self) -> bool {
        self.print == PrintPermission::HighResolution
            && self.modify
            && self.copy
            && self.annotate
            && self.fill_forms
            && self.assemble
    }

    fn to_lopdf(self) -> Permissions {
        // PDF 2.0'da erişilebilirlik için kopyalama her zaman açık olmalıdır.
        let mut permissions = Permissions::COPYABLE_FOR_ACCESSIBILITY;
        match self.print {
            PrintPermission::None => {}
            PrintPermission::LowResolution => permissions |= Permissions::PRINTABLE,
            PrintPermission::HighResolution => {
                permissions |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY
            }
        }
        for (granted, flag) in [
            (self.modify, Permissions::MODIFIABLE),
            (self.copy, Permissions::COPYABLE),
            (self.annotate, Permissions::ANNOTABLE),
            (self.fill_forms, Permissions::FILLABLE),
            (self.assemble, Permissions::ASSEMBLABLE),
        ] {
            if granted {
                permissions |= flag;
            }
        }
        permissions
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtectionSettings {
    /// Belgeyi açma parolası; boşsa belge parolasız açılır.
    pub user_password: String,
    /// İzin (sahip) parolası; boşsa ve izinler kısıtlı değilse açma parolasıyla aynı olur.
    pub owner_password: String,
    pub method: EncryptionMethod,
    pub permissions: PermissionSet,
    /// Yalnız AES: false ise XMP meta verisi arama motorları için şifresiz kalır.
    pub encrypt_metadata: bool,
}

impl std::fmt::Debug for ProtectionSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redact = |password: &str| {
            if password.is_empty() {
                ""
            } else {
                "[REDACTED]"
            }
        };
        f.debug_struct("ProtectionSettings")
            .field("user_password", &redact(&self.user_password))
            .field("owner_password", &redact(&self.owner_password))
            .field("method", &self.method)
            .field("permissions", &self.permissions)
            .field("encrypt_metadata", &self.encrypt_metadata)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecurityOp {
    /// Kopyayı verilen parolalar ve izinlerle şifreler.
    Protect { settings: ProtectionSettings },
    /// Şifresiz bir kopya yazar.
    Remove,
}

/// İsteğin belgeden bağımsız kuralları; komut ve job worker ikisi de çağırır.
pub fn validate_request(op: &SecurityOp) -> Result<(), String> {
    let SecurityOp::Protect { settings } = op else {
        return Ok(());
    };
    let user = settings.user_password.as_str();
    let owner = settings.owner_password.as_str();
    if user.is_empty() && owner.is_empty() {
        return Err("Açma parolası ya da izin parolası girin".into());
    }
    if !settings.permissions.is_unrestricted() {
        if owner.is_empty() {
            return Err("İzinleri kısıtlamak için bir izin parolası girin".into());
        }
        if owner == user {
            return Err("İzin parolası açma parolasından farklı olmalı".into());
        }
    }
    for password in [user, owner] {
        if password.chars().any(char::is_control) {
            return Err("Parola denetim karakteri içeremez".into());
        }
        match settings.method {
            EncryptionMethod::Aes256 => {
                if password.len() > MAX_AES256_PASSWORD_BYTES {
                    return Err("AES-256 parolası en fazla 127 bayt olabilir".into());
                }
            }
            EncryptionMethod::Aes128 | EncryptionMethod::Rc4_128 => {
                // Eski güvenlik işleyicileri parolayı PDFDocEncoding baytları olarak alır;
                // ASCII dışı karakterler okuyucudan okuyucuya farklı kodlanır.
                if !password.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                    return Err(
                        "Bu şifreleme yönteminde parola yalnız ASCII karakterler içerebilir; Türkçe karakterler için AES-256 seçin".into(),
                    );
                }
                if password.len() > MAX_LEGACY_PASSWORD_BYTES {
                    return Err(
                        "Bu şifreleme yönteminde parola en fazla 32 karakter olabilir".into(),
                    );
                }
            }
        }
    }
    if settings.method == EncryptionMethod::Rc4_128 && !settings.encrypt_metadata {
        return Err("RC4 şifrelemede meta veri de şifrelenir".into());
    }
    Ok(())
}

/// `snapshot`'ın `op`'a göre korunmuş ya da şifresiz kopyasını `output`'a yazar.
/// `credential` oturumun belgeyi açtığı paroladır.
pub(crate) fn write_security_copy(
    bindings: &dyn PdfiumLibraryBindings,
    snapshot: &Path,
    credential: Option<&str>,
    op: &SecurityOp,
    output: &Path,
) -> Result<(), String> {
    validate_request(op)?;
    let path = snapshot.to_str().ok_or("PDF yolu UTF-8 değil")?;
    let source = RawDocument::open(bindings, path, credential).ok_or_else(|| {
        format!(
            "PDF açılamadı (PDFium hata kodu {})",
            bindings.FPDF_GetLastError()
        )
    })?;
    let page_count = source.page_count().map_err(|failure| failure.detail)?;
    if bindings.FPDF_GetSecurityHandlerRevision(source.handle) != -1 {
        // Kısıtlı izinlerle (sahip parolası olmadan) açılan belgenin korumasını kaldırmak
        // izinleri atlatmak olur.
        let granted = permission_bits(bindings.FPDF_GetDocPermissions(source.handle));
        if granted & OWNER_EQUIVALENT_PERMISSIONS != OWNER_EQUIVALENT_PERMISSIONS {
            return Err(
                "Belgenin güvenlik ayarları yalnız izin (sahip) parolasıyla açıldığında değiştirilebilir".into(),
            );
        }
    }
    let plain = output.with_file_name(format!(
        "{}.plain",
        output
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Çıktı adı geçersiz")?
    ));
    let written = save_copy(bindings, source.handle, &plain, FPDF_REMOVE_SECURITY)
        .map_err(|failure| failure.detail);
    drop(source);
    let result = written.and_then(|()| write_output(&plain, op, output));
    let _ = std::fs::remove_file(&plain);
    result?;
    verify_copy(bindings, output, op, page_count)
}

/// PDFium'un şifresiz kopyasını `op`'a göre (şifreleyerek ya da olduğu gibi) `output`'a yazar.
fn write_output(plain: &Path, op: &SecurityOp, output: &Path) -> Result<(), String> {
    let size = std::fs::metadata(plain)
        .map_err(|error| format!("Şifresiz kopya okunamadı: {error}"))?
        .len();
    if size > crate::document_structure::MAX_STRUCTURE_INPUT_BYTES as u64 {
        return Err("Belge güvenlik kopyası için çok büyük (256 MiB sınırı)".into());
    }
    let mut document = lopdf::Document::load(plain)
        .map_err(|error| format!("Şifresiz kopya ayrıştırılamadı: {error}"))?;
    if crate::lopdf_compat::was_encrypted(&document) {
        return Err("PDFium şifresiz kopya yazamadı".into());
    }
    // PDFium eski `/Encrypt` sözlüğünü referanssız da olsa kopyada bırakır; içindeki O/U
    // değerleri eski parolanın çevrimdışı denenmesine izin verir.
    document.prune_objects();
    if let SecurityOp::Protect { settings } = op {
        encrypt_document(&mut document, settings)?;
    }
    document
        .save(output)
        .map_err(|error| format!("Kopya yazılamadı: {error}"))?;
    Ok(())
}

fn encrypt_document(
    document: &mut lopdf::Document,
    settings: &ProtectionSettings,
) -> Result<(), String> {
    ensure_file_identifier(document);
    let user = settings.user_password.as_str();
    let owner = if settings.owner_password.is_empty() {
        user
    } else {
        settings.owner_password.as_str()
    };
    let permissions = settings.permissions.to_lopdf();
    let filters = |filter: Arc<dyn CryptFilter>| BTreeMap::from([(b"StdCF".to_vec(), filter)]);
    let key = random_file_key();
    let version = match settings.method {
        EncryptionMethod::Aes256 => EncryptionVersion::V5 {
            encrypt_metadata: settings.encrypt_metadata,
            crypt_filters: filters(Arc::new(Aes256CryptFilter)),
            file_encryption_key: &key,
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: owner,
            user_password: user,
            permissions,
        },
        EncryptionMethod::Aes128 => EncryptionVersion::V4 {
            document,
            encrypt_metadata: settings.encrypt_metadata,
            crypt_filters: filters(Arc::new(Aes128CryptFilter)),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: owner,
            user_password: user,
            permissions,
        },
        EncryptionMethod::Rc4_128 => EncryptionVersion::V2 {
            document,
            owner_password: owner,
            user_password: user,
            key_length: 128,
            permissions,
        },
    };
    let state = EncryptionState::try_from(version)
        .map_err(|error| format!("Şifreleme hazırlanamadı: {error}"))?;
    document
        .encrypt(&state)
        .map_err(|error| format!("Belge şifrelenemedi: {error}"))
}

/// RC4 ve AES-128 anahtarı dosya kimliğinden türetilir; kimliği olmayan belgeye bir tane verilir.
fn ensure_file_identifier(document: &mut lopdf::Document) {
    let has_id = matches!(
        document.trailer.get(b"ID"),
        Ok(Object::Array(parts)) if parts.len() == 2 && parts.iter().all(|part| part.as_str().is_ok())
    );
    if !has_id {
        let id = Object::String(
            uuid::Uuid::new_v4().as_bytes().to_vec(),
            StringFormat::Hexadecimal,
        );
        document
            .trailer
            .set("ID", Object::Array(vec![id.clone(), id]));
    }
}

/// AES-256 dosya anahtarı: üç rastgele (v4) UUID'nin SHA-256 özeti. Her UUID 122 bit
/// işletim sistemi rastgeleliği taşır; özet 256 bitin tamamını doldurur.
fn random_file_key() -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for _ in 0..3 {
        hasher.update(uuid::Uuid::new_v4().as_bytes());
    }
    hasher.finalize().into()
}

fn verify_copy(
    bindings: &dyn PdfiumLibraryBindings,
    output: &Path,
    op: &SecurityOp,
    page_count: u16,
) -> Result<(), String> {
    let path = output.to_str().ok_or("Çıktı yolu UTF-8 değil")?;
    let open = |password: Option<&str>| RawDocument::open(bindings, path, password);
    let check_pages = |document: &RawDocument| -> Result<(), String> {
        let pages = document.page_count().map_err(|failure| failure.detail)?;
        if pages != page_count {
            return Err(format!(
                "Kopyanın sayfa sayısı değişti ({page_count} → {pages})"
            ));
        }
        Ok(())
    };
    let SecurityOp::Protect { settings } = op else {
        let document = open(None).ok_or("Şifresiz kopya PDFium ile açılamadı")?;
        check_pages(&document)?;
        if bindings.FPDF_GetSecurityHandlerRevision(document.handle) != -1 {
            return Err("Kopya hâlâ şifreli".into());
        }
        return Ok(());
    };
    let user = settings.user_password.as_str();
    if !user.is_empty() && open(None).is_some() {
        return Err("Korunan kopya parolasız açılabiliyor".into());
    }
    let document = open((!user.is_empty()).then_some(user))
        .ok_or("Korunan kopya açma parolasıyla açılamadı")?;
    check_pages(&document)?;
    let expected =
        permission_bits(settings.permissions.to_lopdf().bits()) & OWNER_EQUIVALENT_PERMISSIONS;
    let actual = permission_bits(bindings.FPDF_GetDocUserPermissions(document.handle))
        & OWNER_EQUIVALENT_PERMISSIONS;
    if actual != expected {
        return Err(format!(
            "Kopyanın izinleri istenenle uyuşmuyor ({actual:#x} ≠ {expected:#x})"
        ));
    }
    drop(document);
    if !settings.owner_password.is_empty() {
        let owner = open(Some(&settings.owner_password))
            .ok_or("Korunan kopya izin parolasıyla açılamadı")?;
        let granted = permission_bits(bindings.FPDF_GetDocPermissions(owner.handle));
        if granted & OWNER_EQUIVALENT_PERMISSIONS != OWNER_EQUIVALENT_PERMISSIONS {
            return Err("İzin parolası tam yetki vermiyor".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(relative)
    }

    fn all_permissions() -> PermissionSet {
        PermissionSet {
            print: PrintPermission::HighResolution,
            modify: true,
            copy: true,
            annotate: true,
            fill_forms: true,
            assemble: true,
        }
    }

    fn protect(
        user: &str,
        owner: &str,
        method: EncryptionMethod,
        permissions: PermissionSet,
    ) -> SecurityOp {
        SecurityOp::Protect {
            settings: ProtectionSettings {
                user_password: user.into(),
                owner_password: owner.into(),
                method,
                permissions,
                encrypt_metadata: true,
            },
        }
    }

    fn run(
        snapshot: &Path,
        credential: Option<&str>,
        op: &SecurityOp,
        output: &Path,
    ) -> Result<(), String> {
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        write_security_copy(pdfium.bindings(), snapshot, credential, op, output)
    }

    #[test]
    fn validation_requires_a_password_and_an_owner_password_for_restrictions() {
        let none = protect("", "", EncryptionMethod::Aes256, all_permissions());
        assert!(validate_request(&none).is_err());
        let restricted = PermissionSet {
            copy: false,
            ..all_permissions()
        };
        assert!(
            validate_request(&protect("açık", "", EncryptionMethod::Aes256, restricted)).is_err()
        );
        assert!(validate_request(&protect(
            "aynı",
            "aynı",
            EncryptionMethod::Aes256,
            restricted
        ))
        .is_err());
        assert!(
            validate_request(&protect("", "sahip", EncryptionMethod::Aes256, restricted)).is_ok()
        );
        assert!(validate_request(&protect(
            "şifre",
            "",
            EncryptionMethod::Aes256,
            all_permissions()
        ))
        .is_ok());
        assert!(validate_request(&SecurityOp::Remove).is_ok());
    }

    #[test]
    fn legacy_methods_accept_only_short_ascii_passwords() {
        assert!(validate_request(&protect(
            "şifre",
            "",
            EncryptionMethod::Aes128,
            all_permissions()
        ))
        .is_err());
        assert!(validate_request(&protect(
            &"a".repeat(33),
            "",
            EncryptionMethod::Rc4_128,
            all_permissions()
        ))
        .is_err());
        assert!(validate_request(&protect(
            "pass word",
            "",
            EncryptionMethod::Rc4_128,
            all_permissions()
        ))
        .is_ok());
        assert!(validate_request(&protect(
            &"ş".repeat(64),
            "",
            EncryptionMethod::Aes256,
            all_permissions()
        ))
        .is_err());
    }

    #[test]
    fn debug_output_redacts_passwords() {
        let op = protect(
            "gizli-açma",
            "gizli-sahip",
            EncryptionMethod::Aes256,
            all_permissions(),
        );
        let debug = format!("{op:?}");
        assert!(!debug.contains("gizli"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn every_method_protects_a_copy_that_opens_only_with_its_password() {
        let directory = tempfile::tempdir().unwrap();
        let restricted = PermissionSet {
            print: PrintPermission::LowResolution,
            copy: false,
            modify: false,
            ..all_permissions()
        };
        for (index, method) in [
            EncryptionMethod::Aes256,
            EncryptionMethod::Aes128,
            EncryptionMethod::Rc4_128,
        ]
        .into_iter()
        .enumerate()
        {
            let output = directory.path().join(format!("protected-{index}.pdf"));
            let op = protect("acma", "sahip", method, restricted);
            run(&fixture("minimal.pdf"), None, &op, &output)
                .unwrap_or_else(|error| panic!("{method:?}: {error}"));
            assert!(std::fs::read(&output)
                .unwrap()
                .windows(8)
                .any(|w| w == b"/Encrypt"));
            assert!(!directory
                .path()
                .join(format!("protected-{index}.pdf.plain"))
                .exists());
        }
    }

    #[test]
    fn unicode_open_password_works_with_aes256() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("unicode.pdf");
        let op = protect(
            "Şifre-ığüöç",
            "",
            EncryptionMethod::Aes256,
            all_permissions(),
        );
        run(&fixture("minimal.pdf"), None, &op, &output).unwrap();
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let path = output.to_str().unwrap();
        assert!(RawDocument::open(pdfium.bindings(), path, Some("Şifre-ığüöç")).is_some());
        assert!(RawDocument::open(pdfium.bindings(), path, Some("Sifre-iguoc")).is_none());
    }

    #[test]
    fn owner_only_protection_opens_without_a_password_but_restricts() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("owner-only.pdf");
        let restricted = PermissionSet {
            print: PrintPermission::None,
            ..all_permissions()
        };
        run(
            &fixture("minimal.pdf"),
            None,
            &protect("", "sahip", EncryptionMethod::Aes256, restricted),
            &output,
        )
        .unwrap();
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let bindings = pdfium.bindings();
        let document = RawDocument::open(bindings, output.to_str().unwrap(), None).unwrap();
        assert_eq!(
            permission_bits(bindings.FPDF_GetDocPermissions(document.handle)) & 0b100,
            0
        );
    }

    #[test]
    fn removing_security_needs_the_owner_password() {
        let directory = tempfile::tempdir().unwrap();
        let protected = directory.path().join("protected.pdf");
        let restricted = PermissionSet {
            modify: false,
            ..all_permissions()
        };
        run(
            &fixture("minimal.pdf"),
            None,
            &protect("acma", "sahip", EncryptionMethod::Aes128, restricted),
            &protected,
        )
        .unwrap();

        let plain = directory.path().join("plain.pdf");
        let refused = run(&protected, Some("acma"), &SecurityOp::Remove, &plain).unwrap_err();
        assert!(refused.contains("izin"), "{refused}");
        assert!(!plain.exists());

        run(&protected, Some("sahip"), &SecurityOp::Remove, &plain).unwrap();
        // Not even the unreferenced old `/Encrypt` dictionary (its O/U verifiers) survives.
        let bytes = std::fs::read(&plain).unwrap();
        assert!(!bytes.windows(8).any(|w| w == b"/Encrypt"));
        assert!(!bytes.windows(9).any(|w| w == b"/Standard"));
    }

    #[test]
    fn re_protecting_replaces_the_previous_passwords() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.pdf");
        run(
            &fixture("minimal.pdf"),
            None,
            &protect("eski", "", EncryptionMethod::Rc4_128, all_permissions()),
            &first,
        )
        .unwrap();
        let second = directory.path().join("second.pdf");
        run(
            &first,
            Some("eski"),
            &protect("yeni", "", EncryptionMethod::Aes256, all_permissions()),
            &second,
        )
        .unwrap();
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let path = second.to_str().unwrap();
        assert!(RawDocument::open(pdfium.bindings(), path, Some("eski")).is_none());
        assert!(RawDocument::open(pdfium.bindings(), path, Some("yeni")).is_some());
        let bytes = std::fs::read(&second).unwrap();
        let handlers = bytes.windows(9).filter(|w| *w == b"/Standard").count();
        assert_eq!(
            handlers, 1,
            "the old security handler dictionary must not be carried over"
        );
    }
}
