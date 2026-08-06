# Secure Redaction Capability Characterization & Architecture Guide

## 1. Bağımlılıklar ve Sürüm Bilgileri

Bu belgede karakterize edilen tüm yetenekler, repository'de kullanılan aşağıdaki kesin sürümlere dayanmaktadır:

* **`pdfium-render` crate:** `v0.8.37` (`Cargo.lock` doğrulanmıştır)
* **`lopdf` crate:** `v0.34.0` (`Cargo.lock` doğrulanmıştır)
* **Vendored PDFium Binary & C API Header Sürümü:** Chromium/PDFium `152.0.7961.0` (Build `chromium/7961`, `src-tauri/VERSION` ve `src-tauri/include/` doğrulanmıştır)
* **Fixture Üretim Ortamı:** `python3` (Sadece Python `std` kütüphanesi; `hashlib`, `zlib`, `json`, `os`, `shutil`, `pathlib`)
* **Yardımcı Araç Durumu:** `pdfinfo` ve `qpdf` yerel ortamda bulunmadığından çalıştırılmamıştır; bu araçların çıktısı başarılı veya warning-free olarak sayılmamıştır. Doğrulama `lopdf` ve `PDFium` parser matrisi üzerinden yürütülmüş, fixture'lar `lopdf` ve `PDFium` tarafından başarıyla açılmış ve beklenen semantik assertion'ları geçmiştir.

---

## 2. Stream Uzunluk Hesaplama, Raw Validation ve Otomatik Geçerlilik

`generate_redaction_fixtures.py` jeneratöründe tüm PDF stream nesneleri (`text`, `image`, `ObjStm`, `XRef`) için `/Length` değeri `build_stream_object_bytes` helper'ı aracılığıyla ham byte dizisinden (`len(stream_bytes)`) otomatik hesaplanmaktadır.

* Elle sabitleşmiş (`/Length 35`, `/Length 75` vb.) sabit değerler kaldırılmıştır.
* Stream verisi ve `stream\n` ... `\nendstream` sınırları byte düzeyinde kesinleştirilmiştir.
* Raw byte-level stream length doğrulaması `all_generated_fixture_raw_stream_lengths_match_declared_lengths` testi ile ham PDF byte'ları üzerinden (parser decode çıktısına güvenmeden) yapılmaktadır.
* Parser-level stream length doğrulaması `parsed_fixture_stream_lengths_match_decoded_or_loaded_content` entegrasyon testi ile yardımcı kontrol olarak yürütülmektedir.
* Şifreli fixture (`encrypted-redaction.pdf`) için ham stream'lerin `/Encrypt` yapısı ve doğrudan `/Length` değeri ham byte seviyesinde, şifreli içerik çözme (`ENCRYPTED REDACTION FIXTURE SECRET`), parolasız/yanlış parola reddi ve güvenlik revizyonu (`Revision2`, `Unprotected` değil) kimlik doğrulamalı PDFium yaşam döngüsü testi ile doğrulanmıştır. Ham PDF byte'ları içinde beklenen metin marker'ı plaintext olarak bulunmamaktadır.

---

## 3. PDFium Capability Matrisi ve Test İzolasyon Modeli

PDFium testleri `PDFIUM_TEST_MUTEX` adında test-scoped global mutex ile serialize edilir. Testlerde kanıtlanmamış `unsafe impl Send` / `unsafe impl Sync` veya global `OnceLock<Pdfium>` wrapper'ları kullanılmaz. PDFium document ve page nesneleri mutex guard bırakılmadan önce drop edilir. Production `with_pdfium()` görünürlüğü `pub(crate)` olarak korunmuştur.

### 3.1 Parser Reopen, Semantik Assertions ve Render Smoke Test Matrisi

Doğrulama üç ayrı düzeyde yürütülmektedir:

1. **Parser Reopen (lopdf & PDFium):** Tüm 14 fixture PDFium tarafından başarıyla açılmıştır. Şifresiz fixture'lar `lopdf::Document::load` ile ayrıştırılmıştır.
2. **Semantik Assertions:** Her fixture için özel yapılar (AcroForm, ObjStm, Incremental revision, Encrypted content) nesne/metin seviyesinde assert edilmiştir.
3. **Render Smoke Test:** Bütün 14 fixture için PDFium ile tüm sayfalar üzerinde 100x100 opak bitmap render smoke testi çalıştırılmış ve pozitif boyutlu (`width > 0`, `height > 0`) bitmap üretimi doğrulanmıştır.

#### Güvenlik Sınırı Uyarısı (Security Boundary)
Doğru parola ile `encrypted-redaction.pdf` içeriğinin PDFium tarafından çözülüp okunabilmesi **yalnızca fixture/credential doğrulama testidir**. Bu durum şifreli girdilerin secure-redaction profilini desteklediği anlamına gelmez; `secure_rasterized_pages_v1` profili şifreli girdileri hâlâ fail-closed olarak (`REDACTION_ENCRYPTED_INPUT_UNSUPPORTED`) bloke eder.

| Capability | PDFium C API | pdfium-render safe wrapper | Raw binding | Bu projede doğrulandı mı? | Güvenlik anlamı |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Page object enumeration** | `FPDFPage_CountObjects`, `FPDFPage_GetObject` | `page.objects().iter()` | `FPDFPage_CountObjects` | **Doğrulandı** | Sayfa üzerindeki üst düzey nesnelerin sıralanmasını sağlar. |
| **Object type** | `FPDFPageObj_GetType` | `object.object_type()` | `FPDFPageObj_GetType` | **Doğrulandı** | Nesne türünü (Text, Image, Path, Form) belirler. |
| **Object bounds** | `FPDFPageObj_GetBounds` | `object.bounds()` | `FPDFPageObj_GetBounds` | **Doğrulandı** | Bounding box koordinatlarını hesaplar. |
| **Object removal** | `FPDFPage_RemoveObject` | Yok | `FPDFPage_RemoveObject` | **Doğrulandı** | Nesneyi sayfa nesne ağacından çıkarır. Nesne belleğini serbest bırakmak için `FPDFPageObj_Destroy` gereklidir. Font/resource stream'lerini kendiliğinden silmez. |
| **Removed object destruction** | `FPDFPageObj_Destroy` | Yok | `FPDFPageObj_Destroy` | **Doğrulandı** | Sayfadan çıkarılan un-parented nesne belleğini serbest bırakır. |
| **Form XObject child enumeration** | `FPDFFormObj_CountObjects`, `FPDFFormObj_GetObject` | Yok | `FPDFFormObj_GetObject` | **Unverified** | Form XObject içi alt nesnelerin taranmasını sağlar. |
| **Form XObject child removal** | `FPDFFormObj_RemoveObject` | Yok | `FPDFFormObj_RemoveObject` | **Unverified** | Form XObject içindeki alt nesneyi çıkarır. |
| **Page content regeneration** | `FPDFPage_GenerateContent` | Yok | `FPDFPage_GenerateContent` | **Doğrulandı** | Sayfa nesnelerindeki değişikliklerin ardından sayfa content stream'ini yeniden üretir. |
| **Page-to-device conversion** | `FPDF_PageToDevice` | `page.page_to_device_coords(...)` | `FPDF_PageToDevice` | **Doğrulandı** | Sayfa noktasını (PDF point) cihaz piksel koordinatına dönüştürür. |
| **Device-to-page conversion** | `FPDF_DeviceToPage` | `page.device_to_page_coords(...)` | `FPDF_DeviceToPage` | **Doğrulandı** | Cihaz/ekran koordinatını PDF sayfa noktasına dönüştürür. |
| **Page render** | `FPDF_RenderPageBitmap` | `page.render(...)` | `FPDF_RenderPageBitmap` | **Doğrulandı** | Sayfayı yüksek DPI RGBA bitmap olarak rasterize eder. |
| **New document creation** | `FPDF_CreateNewDocument` | `pdfium.create_new_pdf()` | `FPDF_CreateNewDocument` | **Doğrulandı** | Bellekte sıfırdan boş yeni PDF belge nesnesi oluşturur. |
| **New page creation** | `FPDFPage_New` | `document.pages_mut().create_page_at(...)` | `FPDFPage_New` | **Doğrulandı** | Belirtilen genişlik/yükseklik ile boş yeni sayfa ekler. |
| **Image object creation** | `FPDFPageObj_NewImageObj` | `PdfPageImageObject::new(...)` | `FPDFPageObj_NewImageObj` | **Doğrulandı** | Yeni image page object oluşturur. |
| **Bitmap assignment** | `FPDFImageObj_SetBitmap` | `PdfPageImageObject::new(...)` dahili | `FPDFImageObj_SetBitmap` | **Doğrulandı** | RGBA/RGB pikselleri Image nesnesine bağlar. |
| **Page import** | `FPDF_ImportPages` | `document.pages_mut().copy_page_from_document(...)` | `FPDF_ImportPages` | **Doğrulandı** | Sayfayı bir belgeden diğerine kopyalar. |
| **Page box read** | `FPDFPage_GetMediaBox`, `FPDFPage_GetCropBox` | `page.boundaries().media()`, `crop()` | `FPDFPage_GetMediaBox` | **Doğrulandı** | MediaBox ve CropBox sınırlarını okur. |
| **Save flags** | `FPDF_SaveAsCopy`, `FPDF_INCREMENTAL` | `document.save_to_file(...)` | `FPDF_SaveAsCopy` | **Doğrulandı** | Kaydetme modunu yönetir. |
| **Redact annotation subtype** | `FPDF_ANNOT_REDACT` (constant) | Yok | `FPDFPage_CreateAnnot` | **Doğrulandı** | Yalnız görsel/açıklama işaretleyicisidir. **Yıkıcı metin/vektör silme YAPMAZ.** |
| **Native destructive redaction API** | **YOK** | **YOK** | **YOK** | **Doğrulandı** (Yokluğu) | **KRİTİK:** PDFium C API içinde yerleşik yıkıcı (destructive) redaksiyon/burn-in fonksiyonu bulunmamaktadır. |

---

## 4. Incremental Rewrite Karakterizasyon Matrisi

`incremental-revision.pdf` (2 revizyonlu, `/Prev` pointer'lı, `REVISION_ONE_REACHABLE_MARKER`, `REVISION_TWO_REACHABLE_MARKER` ve `UNREACHABLE_OLD_OBJECT_MARKER` barındıran belge) üzerinde yapılan karakterizasyon sonuçları. Tablodaki tüm değerler `test_incremental_rewrite_characterization` entegrasyon testi içindeki `RewriteCharacterization` tip kontrolü ile birebir assert edilmektedir:

| Save Primitive | Reopen (lopdf) | Reopen (PDFium) | Page Count | `/Prev` Pointer | EOF Count | Reachable Rev 1 Marker | Reachable Rev 2 Marker | Unreachable Marker | Append / Prefix | Security Guarantee |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **Original Fixture** | **Ok** | **Ok** | 1 | **Var** | 2 | **Var** | **Var** | **Var** | Baseline | Baseline |
| **`lopdf save`** | **Ok** | **Ok** | 1 | **Yok** | 1 | **Var** | **Var** | **Var** | **Hayır** (Tam serileştirme) | **No — characterization only** |
| **`lopdf prune_objects + save`** | **Ok** | **Ok** | 1 | **Yok** | 1 | **Var** | **Var** | **Yok** (Pruned) | **Hayır** | **No — characterization only** |
| **PDFium `save_to_file`** | **Ok** | **Ok** | 1 | **Yok** | 1 | **Var** | **Var** | **Yok** (Pruned) | **Hayır** | **No — characterization only** |

> [!IMPORTANT]
> `/Prev` yokluğu, tek `%%EOF` bulunması veya nesne temizlenmesi **tek başına güvenli redaksiyon garantisi oluşturmaz**. Güvenli redaksiyon için sıfırdan yeni belge inşası, etkilenen sayfa rasterizasyonu, bağımsız verifier denetimi ve digest-bound yayınlama şarttır. Bütün save primitive'leri için son sütundaki `No — characterization only` kuralı geçerlidir.

---

## 5. Tasarlanan İlk Profil: Planned `secure_rasterized_pages_v1` Contract

### 5.1 Profil ve Strateji Tanımları
* **Expected Profile:** `secure_rasterized_pages_v1`
* **Expected Strategy:** `rasterized_pages`
* **Region Input Tanımı:** `versioned rectangular redaction-region input`

### 5.2 İzin Verilen Politikalar (Allowed)
* Koordinatlı dikdörtgen girdisi.
* Top-left normalized koordinat sözleşmesi.
* Kanonik PDF point dönüşümü.
* Etkilenen (redaksiyon istenen) sayfanın yüksek DPI opak bitmap'e rasterize edilmesi.
* Redaksiyon alanlarının opak siyah (`Solid Black`, RGB 0,0,0) olarak çizilmesi. Opaklık: 1.0.
* Etkilenen sayfanın, sadece redakte edilmiş bitmap taşıyan yeni temiz sayfa olarak yeniden oluşturulması.
* Etkilenen sayfadaki açıklamaların kopyalanmaması (`annotations_not_copied_to_rebuilt_affected_page`).
* Etkilenmeyen sayfaların aktarım davranışı (`unaffected_page_import_behavior_to_be_characterized`).
* Sıfırdan temiz yeni document inşası, bağımsız verifier denetimi ve digest-bound yayınlama.

### 5.3 Normalize Edilmiş Blocker Hata Kodları (Fail-Closed)
* `REDACTION_ENCRYPTED_INPUT_UNSUPPORTED`: Parola korumalı şifreli belgeler.
* `REDACTION_SIGNED_INPUT_UNSUPPORTED`: Dijital imzalı veya `/ByteRange` içeren belgeler.
* `REDACTION_XFA_INPUT_UNSUPPORTED`: Dinamik XFA form yapıları.
* `REDACTION_BLOCKED_DOCUMENT_FEATURE`: Güvenli politikası belirlenemeyen AcroForm / interaktif içerikler.
* `REDACTION_PAGE_GEOMETRY_MISMATCH`: Desteklenmeyen sayfa geometrisi.
* `REDACTION_RESOURCE_LIMIT_EXCEEDED`: Kaynak sınırlarının aşılması.

---

## 6. Fixture Corpus & SHA-256 Manifesti

Tüm 14 fixture `src-tauri/tests/fixtures/redaction/` dizininde bulunmakta olup `generate_redaction_fixtures.py` script'i ile deterministik olarak üretilmekte ve `manifest.json` dosyası kanonik formatta oluşturulmaktadır:

| Fixture Dosyası | Byte Size | SHA-256 Digest | Expected Outcome | Expected Blocker | Semantik Doğrulama Assertions |
| :--- | :---: | :--- | :---: | :---: | :--- |
| `simple-text.pdf` | 634 | `2adc560d26fc60706a38061fc486d0e926dce9fa904476e991912d6ea3052af6` | `ready` | `null` | PDFium 1 sayfa okur, metin `CONFIDENTIAL TC...` içerir, text page object mevcut. |
| `rotations.pdf` | 1095 | `6665e102d1ad54f8e37bed0149da640930624098794980a10ecf3846fd511628` | `ready` | `null` | 4 sayfa okur, oryantasyonlar 0, 90, 180, 270 derece, render smoke test başarılı. |
| `cropbox-mediabox-diff.pdf` | 624 | `496ec852744fdacb2235cb3da7b4234de5777e945f9561108b13b8630071b031` | `ready` | `null` | `media != crop`, `crop.left == 50.0`, `crop.bottom == 50.0`, effective genişlik 500.0, yükseklik 700.0. |
| `nonzero-origin.pdf` | 604 | `11bb134408b14960adf5f1d6cce1629ea1e5880cd1cb81f0fdbfd985b1003c0d` | `ready` | `null` | `media.left == 50.0`, `media.bottom == 50.0`, genişlik 600.0, yükseklik 800.0. |
| `negative-origin.pdf` | 609 | `c689d40fc2b5436a3267ede469682a59f7a869e98837908057b879d9ac8c98cb` | `ready` | `null` | `media.left == -100.0`, `media.bottom == -100.0`, `lopdf` ve PDFium geometrisi örtüşüyor. |
| `image-only.pdf` | 666 | `90cfc9ea441c48fe8ada88a6d22f41b17a1e782a8d354bcdbe348c9686a1ac95` | `ready` | `null` | 1 Image object, 0 Text object, metin çıkarımı boş, render smoke test başarılı. |
| `text-image.pdf` | 828 | `cdda8a688e533c8e48d7052cb61d26f6dc48f5a50e87a0e2f36f59d7ba47f710` | `ready` | `null` | Text object > 0, Image object > 0, metin `TEXT AND IMAGE PAGE` içerir, render smoke test başarılı. |
| `annotations-fixture.pdf` | 743 | `a3c8403190628cba410df6a97cdf3aa81153eb2b0feb7339b91d0a9f39b3f14d` | `ready` | `null` | Annotation sayısı > 0, Subtype `/Text`, sayfa 0 ilişkisi doğru. |
| `acroform-widget.pdf` | 804 | `42cadc768e70985c198104b348b854c5b26c49cb23856a3b76d441f0a0faed51` | `warning` | `null` | Catalog `/AcroForm` taşır, `/Fields` dolu, `/FT /Tx`, Widget `CustomerName` -> `Jane Doe` resolve edilir. |
| `xfa-fixture.pdf` | 751 | `504b7e6630e15a663d5933e938237222c33823b06ef4a03c9d85c395ca28fdb3` | `blocked` | `REDACTION_XFA_INPUT_UNSUPPORTED` | `/AcroForm` içinde `/XFA` varlığı doğrulanır, normal AcroForm'dan ayırt edilir. |
| `encrypted-redaction.pdf` | 933 | `2d922b2bdad284ee55bf8b96dded1fe4173cc7da7c368635218a371fbe62d791` | `blocked` | `REDACTION_ENCRYPTED_INPUT_UNSUPPORTED` | Gerçek Standard Security Handler Rev 2 şifreli içerik; parolasız ve yanlış parola reddedilir; raw PDF byte'larında plaintext marker bulunmaz; doğru parola (`fixture-password`) ile AÇILIR; sayfa metninden exact marker (`ENCRYPTED REDACTION FIXTURE SECRET`) çıkarılır; security handler revision `Revision2` (`Unprotected` değil) ve stream length geçerlidir; render smoke test başarılıdır; expected blocker `REDACTION_ENCRYPTED_INPUT_UNSUPPORTED`. |
| `signature-marked-byterange.pdf` | 921 | `074ae3a0d82c50097fed6d578c31bb5cef7065129a07bf9ae0b229f192eb6b35` | `blocked` | `REDACTION_SIGNED_INPUT_UNSUPPORTED` | Imza sözlüğü `/Type /Sig` ve `/ByteRange` taşır; kriptografik geçerlilik iddiası yapılmaz. |
| `incremental-revision.pdf` | 1270 | `635f8520ebe3f0617da3634eff066f2dfcfa50a8fc5ccf1861c28c6f8474861f` | `ready` | `null` | 2 xref / `startxref`, `/Prev` var, `REVISION_ONE_REACHABLE_MARKER`, `REVISION_TWO_REACHABLE_MARKER`, `UNREACHABLE_OLD_OBJECT_MARKER` ayrılmış. |
| `object-stream.pdf` | 686 | `68fb1e93556b01fd26376cd0af0955258ce1364f0d7c8bd26b0e56f7107036bb` | `ready` | `null` | PDF 1.5, `/Type /ObjStm` ve `/Type /XRef` taşır, nesne 5 uncompressed top-level nesne değildir; `lopdf` `COMPRESSED_OBJ_5_MARKER` resolve eder, PDFium açar ve render eder. |

---

## 7. Prompt 8B-2 Giriş Kriterleri ve Blocker Durumu

* Otomatik stream length hesaplaması, ham byte-level stream doğrulaması, multi-parser matrisi, deterministik manifest ve semantik testlerin tamamı %100 yeşildir.
* Unsafe PDFium test sarmalayıcıları tamamen kaldırılmış, test izolasyonu mutex-scoped lifecycle ile sağlanmıştır.
* **Prompt 8B-2 aşamasına geçiş için herhangi bir engel (blocker) kalmamıştır**.

---

## 8. Wire Format Kararı, Page Plan Sözleşmesi ve Aktif Limitler (Prompt 8B-2.5)

### 8.1 Serileştirme ve Wire Format Sözleşmesi

* **Canonical Backend Wire Format:** `snake_case` (Rust Serde DTO'ları dış dünyaya `snake_case` JSON üretir).
* **Frontend Normalized Domain Format:** `camelCase` (JS normalizer'ları frontend yerel alan adlarında `camelCase` üretir).
* **Alias Compatibility ve Conflict Politikası:** Frontend normalizer `snake_case` backend verisini kabul eder. Geriye uyumluluk için `camelCase` veya `snake_case` takma adları kabul edilir; ancak aynı özellik için hem `camelCase` hem `snake_case` alanı birden var olursa (örneğin `{ page_index: 0, pageIndex: 0 }`), muğlaklık nedeniyle fail-closed olarak `REDACTION_INVALID_POLICY` hatası fırlatılır.

### 8.2 PagePlan Tamamlanan Sözleşmesi

`PageRedactionPlan` nesnesi aşağıdaki alanları taşır ve doğrular:

```text
PageRedactionPlan {
  schemaVersion: "1.0.0",
  pageIndex: u32,
  strategy: "rasterize_page",
  geometry: CanonicalPageGeometry,
  geometryDigest: SHA-256 (64 hex),
  canonicalRegions: CanonicalRedactionRegion[],
  facts: PageRedactionFacts,
  featureLosses: RedactionFeatureLosses,
  blockers: RedactionBlocker[],
  warnings: RedactionWarning[],
  executable: boolean (ZORUNLU)
}
```

#### Page Plan Invariant'ları:
1. `pagePlan.pageIndex == pagePlan.geometry.pageIndex == pagePlan.facts.pageIndex` ve her `region.pageIndex == pagePlan.pageIndex`.
2. `pagePlan.strategy` yalnızca `"rasterize_page"` kabul edilir. Bilinmeyen stratejiler fail-closed reddedilir.
3. `pagePlan.geometryDigest` strictly 64 küçük harf hex SHA-256 string olmak zorundadır.
4. `executable` zorunlu boolean'dır; eksik payload reddedilir (varsayılan `false` yapılmaz).
5. `pagePlan.blockers` dolu ise `executable` `true` olamaz.
6. Üst düzey `plan.executable == true` olması yalnızca tüm sayfa planlarının `executable == true` olması ve hiçbir engelleyici (blocker) bulunmaması şartıyla mümkündür.

### 8.3 Aktif Kontrat Limitleri Envanteri

Aşağıdaki tablo, Rust (`src-tauri/src/secure_redaction.rs`), JavaScript (`src/features/redaction/redactionSchemas.js`), Manifest (`secure-redaction-contract-v1.json`) ve Testlerde aktif kullanılan kesin limitlerdir (Kullanılmayan `MAX_REGIONS_PER_PAGE`, `MAX_DIGEST_LEN`, `MAX_STATUS_LEN` ve `MAX_VERIFICATION_CHECK_LEN` kaldırılmıştır):

| Sabit Adı | Değer | Birim | İlgili DTO / Alanlar | Güvenlik ve Kaynak Gerekçesi |
| :--- | :---: | :---: | :--- | :--- |
| `REDACTION_SCHEMA_VERSION` | `"1.0.0"` | SemVer | Tüm DTO'lar | Sözleşme sürüm uyumsuzluklarını saptamak. |
| `U32_MIN` / `U32_MAX` | `0` / `4294967295` | Integer | `pageIndex`, `totalPages` vb. | Rust `u32` bounds ile parity. |
| `I32_MIN` / `I32_MAX` | `-2147483648` / `2147483647` | Integer | `DevicePixelRect.left`, `top` | Rust `i32` bounds ile parity. |
| `MAX_REGIONS_PER_INPUT` | `1000` | Adet | `RedactionRegionInput` listesi | Worker bellek tükenmesini engellemek. |
| `MAX_REGION_ID_LEN` | `128` | UTF-8 Byte | Region ID, Source Region ID | ID sınırlandırması. |
| `MAX_SOURCE_LEN` | `256` | UTF-8 Byte | Region `source` | Kaynak etiketi sınırı. |
| `MAX_SOURCE_REGION_IDS` | `100` | Adet | `CanonicalRedactionRegion.sourceRegionIds` | Birleşmiş kaynak ID sınırı. |
| `MAX_CANONICAL_REGIONS_PER_PAGE` | `1000` | Adet | `PageRedactionPlan.canonicalRegions` | Sayfa başı kanonik bölge sınırı. |
| `MAX_PAGE_PLANS` | `1000` | Adet | `SecureRedactionPlan.pagePlans` | Maksimum işlenebilir sayfa planı sınırı. |
| `MAX_BLOCKERS` | `100` | Adet | Blocker koleksiyonları | Engelleyici liste boyut sınırı. |
| `MAX_WARNINGS` | `500` | Adet | Warning koleksiyonları | Uyarı liste boyut sınırı. |
| `MAX_VERIFICATION_CHECKS` | `100` | Adet | Verification check listeleri | Kontrol adımı sayı sınırı. |
| `MAX_BLOCKER_CODE_LEN` | `128` | UTF-8 Byte | `RedactionBlocker.code` | Hata kodu string sınırı. |
| `MAX_WARNING_CODE_LEN` | `128` | UTF-8 Byte | `RedactionWarning.code` | Uyarı kodu string sınırı. |
| `MAX_USER_SAFE_MESSAGE_LEN` | `2048` | UTF-8 Byte | Blocker/Warning `message` | Mesaj boyut sınırı. |
| `MAX_DETAILS_LEN` | `4096` | UTF-8 Byte | Blocker/Warning/Check `details` | Detay metni boyut sınırı. |
| `MAX_IDENTITY_LEN` | `128` | UTF-8 Byte | `jobId`, `sessionId`, `publicationId` | Kimlik string sınırı. |
| `MAX_VERIFICATION_CHECK_CODE_LEN` | `256` | UTF-8 Byte | `RedactionVerificationCheck.code` | Kontrol kodu string sınırı. |
| `MAX_VERIFICATION_CHECK_MESSAGE_LEN` | `2048` | UTF-8 Byte | `RedactionVerificationCheck.message` | Kontrol mesajı string sınırı. |
| `MAX_VERIFICATION_CHECK_DETAILS_LEN` | `4096` | UTF-8 Byte | `RedactionVerificationCheck.details` | Kontrol detay metni string sınırı. |
| `MAX_PATH_LEN` | `4096` | UTF-8 Byte | `outputPath`, `publishedPath` | Dosya yolu uzunluk sınırı. |
| `MIN_RASTER_DPI` / `MAX_RASTER_DPI` | `72` / `600` | DPI | `SecureRedactionPolicy.rasterDpi` | Görüntü kalitesi ve RAM dengesi. |
| `MIN_SAFETY_MARGIN_POINTS` / `MAX_SAFETY_MARGIN_POINTS` | `0.0` / `36.0` | PDF Point | `SecureRedactionPolicy.safetyMarginPoints` | Taşma payı sınırları. |

### 8.4 Verification Check Limit Ayrımı

Verification check string limitleri üç ayrı kanonik sabite bölünmüştür:
* `MAX_VERIFICATION_CHECK_CODE_LEN` (256 UTF-8 Byte)
* `MAX_VERIFICATION_CHECK_MESSAGE_LEN` (2048 UTF-8 Byte)
* `MAX_VERIFICATION_CHECK_DETAILS_LEN` (4096 UTF-8 Byte)

Bu sabitler Rust (`validate_verification_check`), JavaScript (`normalizeRedactionVerificationCheck`) ve manifest (`secure-redaction-contract-v1.json`) tarafında birebir eşit olarak uygulanır.

### 8.5 Manifest Otoritesi ve Doğrulama Modeli

`src-tauri/tests/fixtures/contracts/secure-redaction-contract-v1.json` manifest dosyası, Rust ve JS arasındaki kanonik sözleşme beyanıdır:
* Manifest enom kümesini, limitleri, sayısal aralıkları ve wire format kararını tanımlar.
* Rust ve JS testleri bu manifest dosyasını doğrudan okuyup kod parçalarıyla 1:1 doğrular.
* Manifest çalışma zamanı validator'larının yerine geçmez; çalışma zamanı doğrulaması `validate_secure_redaction_plan` ve `normalizeSecureRedactionPlan` tarafından yürütülür.

---

## 9. Saf Redaksiyon Planner Tamamlanması ve Geometri Parity'si (Prompt 8B-3.1)

### 9.1 Plan Sürümü ve Gerçek Zamanlı Digest
* **Plan Version:** `"secure_redaction_plan_v1"` (`SECURE_REDACTION_PLAN_VERSION`).
* **Real-time Plan Digest:** `build_secure_redaction_plan` tarafından kanonik plan içeriği (input digest, content generation, policy, acknowledgements, acknowledgement requirements, facts, page plans, canonical regions, losses, blockers, warnings, executable) üzerinden gerçek zamanlı 64 karakter hex SHA-256 olarak hesaplanır.

### 9.2 JS Geometri Parity'si
* `normalized_top_left_to_pdf_point_rect` fonksiyonunun koordinat dönüşüm hesabı `src/utils/pdfGeometry.js` ile 1:1 eşleştirilmiştir.
* Normalized top-left koordinatlar doğrudan `effectiveBox` (`left`, `bottom`, `width`, `height`) referans alınarak PDF point değerlerine dönüştürülür; normalized koordinatlara intrinsic sayfa rotasyonu uygulanmaz.

### 9.3 Sayfa Redaksiyon Stratejisi Ayrımı
* **Etkilenen Sayfalar ($\ge 1$ kanonik bölge):** Strateji `"rasterize_page"` (`PageRedactionStrategy::RasterizePage`).
* **Etkilenmeyen Sayfalar ($0$ bölge):** Strateji `"import_unaffected_page"` (`PageRedactionStrategy::ImportUnaffectedPage`).

### 9.4 Blocker ve Warning Aggregation
* Planner çalışma zamanında aynı `code` değerine sahip birden fazla uyarı veya engelleyici ürettiğinde, bunlar tek bir kod girdisinde birleştirilir.
* `details` metinleri `"; "` ayracı ile birleştirilir ve `MAX_DETAILS_LEN` (4096 UTF-8 byte) sınırını aşmayacak şekilde kırpılır.

---

## 10. Frontend Digest Verification, Canonical Serialization Parity ve Resource Limits (Prompt 8B-3.4)

### 10.1 Tagged Binary Canonical Serializer Parity
* Rust (`CanonicalSerializer`) ve JavaScript (`CanonicalSerializer`) sınıfları arasında etiketli (tagged) binary serileştirme ve SHA-256 digest üretimi 1:1 eşleştirilmiştir.
* Binary etiket sözleşmesi: `0x01` String, `0x02` Bool, `0x03` U32, `0x04` U64, `0x05` F64, `0x06` Null, `0x07` List, `0x08` Object, `0x09` Field, `0x0A` I32.
* IEEE 754 Big-Endian 64-bit float kodlaması kullanılır. `-0.0` değeri `0.0` olarak normalleştirilir.
* JavaScript tarafında `recomputedGeometryDigest === pagePlan.geometryDigest` ve `recomputedPlanDigest === plan.planDigest` gerçek zamanlı SHA-256 kontrolleri zorunlu kılınmıştır.

### 10.2 Sıfır Toleranslı Komşuluk İzolasyonu (Zero-Gap Isolation Policy)
* `ADJACENCY_TOLERANCE_POINTS = 0.0` olarak sabitleştirilmiştir.
* Pozitif aralık içeren (`gap > 0.0`) ayrı redaksiyon bölgelerinin birleştirilmesi kesinlikle yasaktır (`GEOMETRY_EPSILON` toleransı komşuluk kararında kullanılmaz).
* Yalnızca çakışan veya sıfır mesafeyle kenar kenara dokunan (`gap == 0.0`) bölgeler birleştirilir (`MERGE_EDGE_TOUCHING = true`).

### 10.3 Gerçek Olgulara Dayalı Özellik Kaybı (Fact-Driven Feature Losses)
* Özellik kayıpları (`feature_losses`) doğrudan belgede ve sayfada tespit edilen gerçek olgulara (`facts`) bağlanmıştır:
  - `text` varlığı $\rightarrow$ `text_selection_lost`, `searchability_lost`
  - `structure_association` varlığı $\rightarrow$ `accessibility_tags_lost`, `structure_association_lost`
  - `link_annotations` varlığı $\rightarrow$ `links_lost`
  - `annotations` varlığı $\rightarrow$ `annotations_lost`
  - `widgets` varlığı $\rightarrow$ `forms_lost`
  - `vector_paths` veya `form_xobjects` varlığı $\rightarrow$ `vector_quality_lost`
  - `optional_content` varlığı $\rightarrow$ `layers_lost`
  - `spot_colors` varlığı $\rightarrow$ `spot_colors_lost`
  - `complex_color_spaces`, `transparency` veya `complex_resources` varlığı $\rightarrow$ `color_space_fidelity_risk`
  - `page_actions` varlığı $\rightarrow$ `page_actions_lost`
* Yalnız resim içeren (image-only) etkilenen bir sayfa 12 özellik kaybı alanının tamamında `false` üretir; yanlış genellemeler (ör. resim redaksiyonunun erişilebilirlik etiketlerini kaybettirmesi) engellenmiştir.

### 10.4 Kaynak Limitleri ve Güvenli Kapanma (Fail-Closed Resource Limits)
* Kaynak limitleri Rust, JavaScript, Manifest ve Dokümantasyonda tam eşitlikle tanımlanmıştır:
  - `MAX_PIXELS_PER_PAGE`: `100_000_000` piksel (~10,000x10,000 px)
  - `MAX_TOTAL_ESTIMATED_PIXELS`: `1_000_000_000` piksel
  - `MAX_REPORT_SIZE_BYTES`: `10_485_760` UTF-8 Byte (10 MiB)
* Limit aşımında kanonik bölgelerin kırpılması (truncation) KESİNLİKLE YASAKTIR. Limit aşıldığında `REDACTION_RESOURCE_LIMIT_EXCEEDED` engelleyicisi üretilir (`executable = false`, `readyForExecution = false`).

### 10.5 Kanonik Sıralama ve Deep Equality Kontrolü
* Kanonik bölgeler sayfa ve belge düzeyinde kesin sıralama kurallarına uymalıdır: `(pageIndex, bottom, left, regionId)`.
* JavaScript normalizer'ı üst düzey `canonicalRegions` koleksiyonu ile sayfa düzeyindeki `pagePlans[i].canonicalRegions` elemanları arasında exact deep equality doğrulaması gerçekleştirir.


