# Secure Redaction Capability Characterization & Architecture Guide

## Independent Verification Architecture

`ApplySecureRedaction` produces only `transformed_unverified`. A distinct,
private `VerifySecureRedaction` worker invocation reopens both immutable
`snapshot.pdf` and the fixed internal `secure-redaction-transform.pdf` artifact:

```text
source snapshot → Plan worker → Transform worker → transformed_unverified artifact
                                                       ↓
                                              new worker process
                                                       ↓
                                           VerifySecureRedaction
                                                       ↓
                                      verified | verification failed
```

The verifier derives the artifact name itself and treats transform-report paths
and boolean evidence only as consistency input, never proof. It re-derives the
plan from the snapshot and observes the serialized artifact. Transform success
does not mean verified; verified does not mean published. The artifact remains
inside the job directory and no user document is changed.

## Verifier Version and Required Checks

`secure_redaction_verifier_v1` requires this exact ordered check set:
source-plan binding, transform-report consistency, artifact path integrity,
artifact digest/size, dual-parser reopen, full-rewrite structure, forbidden
document structures, page count/strategy, page geometry, affected-page
image-only, affected interactive-content absence, redaction pixel coverage,
unaffected-page equivalence, and final digest stability. Unknown, missing,
reordered, failed, or `not_run` checks cannot yield `is_verified: true`.

## Artifact Integrity and Observation Boundary

The artifact must be a regular non-symlink file in a regular non-symlink job
directory, under its fixed name and below `MAX_SECURE_REDACTION_OUTPUT_BYTES`.
SHA-256 and size are matched before observation and SHA-256 is recalculated at
the end to detect replacement during verification. PDFium supplies page
geometry, object/text/annotation and rendered-pixel observations; lopdf
supplies trailer, catalog, and object-graph observations. The verifier rejects
revision chains, encryption, signatures, forms/XFA, names/actions, optional
content, structure trees, collections, attachments, and executable payload
keys. Affected pages must be exactly one image object with no text or
interactive content and opaque-black coverage across every expected redaction
pixel.

## Verifier Completeness Hardening

### FULL_REWRITE_STRUCTURE and object reachability

`FULL_REWRITE_STRUCTURE` is independent from the forbidden-payload check. It
requires a resolvable indirect trailer `/Root` with a `/Catalog`, no trailer
`/Prev`, a resolvable page tree whose unique `/Page` references and count agree
with both PDFium and the regenerated plan, and no unreachable indirect
objects. Reachability is observed by cloning the parsed `lopdf::Document` and
calling `prune_objects()`; every object it would remove fails the check. There
is no orphan-object allowlist. `FORBIDDEN_DOCUMENT_STRUCTURES_ABSENT` instead
owns the distinct `/Encrypt`, signature, AcroForm/XFA, names/actions,
attachments, optional-content, structure-tree, and other interactive or
executable payload rejection policy.

### Affected-page structural and raster observations

Affected pages are checked by both PDFium and lopdf. PDFium must expose exactly
one image object and no extractable text. lopdf must decode the content stream,
allow only the characterized PDFium image-placement/state operators, permit
exactly one `Do` invocation, and resolve it to the single `/XObject` resource
whose subtype is `/Image`; Form, extra, uninvoked, or masked color-key XObjects
fail closed. The current PDFium serialization may use an `/SMask`; its processed
PDFium pixels must still be fully opaque. PDFium also observes the image bounds
against the canonical effective page box. This structural observation has a
deliberately separate precision policy: `FPDFPageObj_GetBounds()` writes its
four edges as `f32`, and `pdfium-render`'s `PdfMatrix` and `PdfPoints` are
f32-backed. The verifier therefore permits exactly one IEEE-754 f32 ULP at the
canonical effective page's coordinate scale (`max(abs(edges), width, height,
1)`), including when an expected edge is zero. This is neither a visual
tolerance nor a percentage comparison, and it is not an arbitrary epsilon. It
only models the bounded PDFium page-object observation surface; each of left,
bottom, right, and top must remain within that bound, so translation and
scaling corruption still fail closed. `GEOMETRY_EPSILON = 1e-6` remains the
canonical planner/page geometry contract and is not used or relaxed here.

Raw image dimensions must equal freshly calculated raster dimensions. The
affected-page coverage check passes only when both independent observations
pass: (1) each canonical PDF region is mapped by the immutable source page's
PDFium `points_to_pixels()` conversion and is exactly opaque black in the
embedded raw raster, and (2) the same canonical PDF region is independently
mapped by the reopened output page's PDFium `points_to_pixels()` conversion,
using the actual deterministic render configuration, and every mapped pixel is
exactly `[0, 0, 0, 255]` in the reopened RGBA render. The source and output
observations do not share a device rectangle. The render observation uses a
fresh artifact reopen because PDFium image extraction performed by structural
and embedded-raster checks can mutate internal image/cache state on the earlier
document handle. Diagnostics identify the failing observation without
including source text. Thus the lower-precision object-bounds observation is
not the redaction-coverage authority: embedded raw-raster and independently
reopened-render coverage remain exact, full-pixel, outward-mapped checks.

### Unaffected-page equivalence

An unaffected imported page passes only when both its PDFium extracted text and
its deterministic PDFium RGBA render (same regenerated geometry, DPI, rotation,
opaque background, and annotation policy) exactly equal the immutable source.

### Verification boolean and cancellation model

The Rust and frontend schema validators enforce `is_verified IFF all required
checks passed AND blockers are empty`; false-negative reports are invalid just
as false-positive reports are. A short-circuited check remains in canonical
order as `not_run` and cannot be verified. Verification remains a separate
private child worker invocation. Parent-side session-job cancellation and
timeout own child termination, wait/reap, cleanup, and terminal cancellation;
the child has no invented cooperative cancellation channel.

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

1. **Parser Reopen (lopdf & PDFium):** Tüm 26 fixture PDFium tarafından başarıyla açılmıştır. Şifresiz fixture'lar `lopdf::Document::load` ile ayrıştırılmıştır.
2. **Semantik Assertions:** Her fixture için özel yapılar (AcroForm, ObjStm, Incremental revision, Encrypted content) nesne/metin seviyesinde assert edilmiştir.
3. **Render Smoke Test:** Bütün 26 fixture için PDFium ile tüm sayfalar üzerinde 100x100 opak bitmap render smoke testi çalıştırılmış ve pozitif boyutlu (`width > 0`, `height > 0`) bitmap üretimi doğrulanmıştır.

#### Güvenlik Sınırı Uyarısı (Security Boundary)
Doğru parola ile `encrypted-redaction.pdf` içeriğinin PDFium tarafından çözülüp okunabilmesi **yalnızca fixture/credential doğrulama testidir**. Bu durum şifreli girdilerin secure-redaction profilini desteklediği anlamına gelmez; `secure_rasterized_pages_v1` profili şifreli girdileri hâlâ fail-closed olarak (`REDACTION_ENCRYPTED_INPUT_UNSUPPORTED`) bloke eder.

| Capability | PDFium C API | pdfium-render safe wrapper | Raw binding | Bu projede doğrulandı mı? | Güvenlik anlamı |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Page object enumeration** | `FPDFPage_CountObjects`, `FPDFPage_GetObject` | `page.objects().iter()` | `FPDFPage_CountObjects` | **Doğrulandı** | Sayfa üzerindeki üst düzey nesnelerin sıralanmasını sağlar. |
| **Object type** | `FPDFPageObj_GetType` | `object.object_type()` | `FPDFPageObj_GetType` | **Doğrulandı** | Nesne türünü (Text, Image, Path, Form) belirler. |
| **Object bounds** | `FPDFPageObj_GetBounds` | `object.bounds()` | `FPDFPageObj_GetBounds` | **Doğrulandı** | Bounding box koordinatlarını hesaplar. |
| **Object removal** | `FPDFPage_RemoveObject` | `objects_mut().remove_object_at_index()` | `FPDFPage_RemoveObject` | **Characterization-only** | Nesne sayfa object ağacından çıkarılır ve content regenerate edilir; standalone text/image full-save payload gözlemleri olumludur. Detached-object destroy gate başarısız olduğundan production eligibility kanıtı değildir. |
| **Removed object destruction** | `FPDFPageObj_Destroy` | Detached object `Drop` | `FPDFPageObj_Destroy` | **NO-GO / başarısız** | `remove → generate → destroy` sırası gerçek vendored macOS PDFium üzerinde üç izole tekrarda SIGSEGV üretmiştir. Production backend eklenmemiştir. |
| **Form XObject child enumeration** | `FPDFFormObj_CountObjects`, `FPDFFormObj_GetObject` | `PdfPageXObjectFormObject::{len,iter}` | `FPDFFormObj_CountObjects`, `FPDFFormObj_GetObject` | **Doğrulandı** | Text child, transformed child bounds ve nested Form child enumeration gerçek adversarial fixture üzerinde doğrulanmıştır; sharing/mutation kanıtı değildir. |
| **Form XObject child removal** | `FPDFFormObj_RemoveObject` | Mevcut feature set'te immutable | `FPDFFormObj_RemoveObject` | **Unavailable / unverified** | `pdfium-render 0.8.37` default build `PageObjectsCollectionIsImmutable` döndürür. Raw symbol/header varlığı ownership, regeneration veya payload safety kanıtı sayılmaz. |
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

## Optional Whole-Object Backend: NO-GO

### Karar ve güvenlik modeli

**`WholeObjectBackendDecision::NoGo` semantik kararı verilmiştir.** Optional
whole-object/vector-preserving production strategy eklenmemiştir.
`secure_rasterized_pages_v1` ve onun `rasterized_pages` stratejisi authoritative
fallback olarak aynen kalır. Bu NO-GO kararı secure raster pipeline'ın
tamamlanmasını veya 8B kapanışını bloke etmez.

Değerlendirilen dar semantik yalnız bir canonical redaction region'ın bütün
görünür etkisini conservative olarak kapsadığı bağımsız biçimde kanıtlanan tek
bir page object'in tamamının kaldırılmasıdır. Glyph/run bölme, image crop,
path-segment mutation, clip/transparency graph rewrite, Form resource mutation
ve shared Form in-place mutation kapsam dışıdır. False negative kabul edilir;
tek bir belirsizlik bütün affected page'i raster fallback'e göndermelidir.

GO gate aşağıdaki mandatory capability'lerin tümünü ister: standalone text ve
image payload removal, orphan resource pruning, conservative effect bounds,
Form ownership/sharing isolation, güvenli detached-object lifetime,
outside-region preservation, tagged-structure update, exact page-geometry
preservation, visual coverage proof ve strategy-aware independent verifier.
Bir alan false veya `not_run` ise karar NO-GO'dur. Bu fail-closed model
`src-tauri/tests/whole_object_backend_characterization.rs` içinde executable
olarak sabitlenmiştir.

### Gerçek sürüm characterization sonucu

Characterization yalnız repository'nin gerçek `pdfium-render 0.8.37`, vendored
PDFium `152.0.7961.0` ve `lopdf 0.34.0` sürümleriyle yapılmıştır.

| Alan | Sonuç | Production eligibility anlamı |
| :--- | :--- | :--- |
| Top-level text removal | `FPDFPage_RemoveObject` ve content regeneration sonrasında full-save/reopen'da object, PDFium text extraction, decoded stream marker ve serialized plaintext marker yoktur. Bu serialization probe detached object'i bilerek destroy etmez; production primitive değildir. | Payload gözlemi olumlu, lifetime gate başarısız olduğu için **ineligible**. |
| Top-level image removal | Placement object, page `/XObject` resource ve deterministic source pixel payload full-save/reopen'da yoktur; `lopdf::prune_objects()` ek orphan bulmaz. Bu probe da detached object'i bilerek destroy etmez. | Payload/pruning gözlemi olumlu, lifetime gate başarısız olduğu için **ineligible**. |
| Required lifetime sequence | `remove → automatic GenerateContent → drop/destroy detached object` gerçek wrapper/API sırası ayrı child process'te üç deterministik tekrarda `SIGSEGV` üretmiştir. Crash normal Rust test process'inden izole edilip exit signal birebir assert edilir. Sanitizer sonucu değildir. | Mandatory ownership proof başarısız: **NO-GO**. |
| Bounds | Plain/rotated/skewed text, positive/negative image transforms, fill/stroke paths ve Form objects için `FPDFPageObj_GetBounds` finite değer döndürür. Bu, stroke, clipping, transparency, pattern/shading veya Form transform altında bounds'un bütün görünür etkiyi conservative kapsadığını kanıtlamaz. | Yalnız finite bounds characterization; conservative-effect proof yok: **ineligible**. |
| Form child enumeration/type | `FPDFFormObj_CountObjects/GetObject` gerçek fixture'da text child ve nested Form child'ı enumerate eder; child bounds okunur ve nested child yeniden enumerate edilir. | Enumeration **verified**, mutation/isolation kanıtı değildir. |
| Form child removal | Repository'nin derlenen `pdfium-render 0.8.37` default feature set'i Form child collection'ı immutable tutar ve `PageObjectsCollectionIsImmutable` döndürür. Raw API header/symbol varlığı production safety proof sayılmamıştır. | **Unverified/unsupported**, Form target her zaman raster fallback. |
| Nested Form | Nested child enumeration çalışır; ownership, regeneration, resource cloning ve payload removal kanıtlanmamıştır. | **Ineligible**. |
| Shared Form/resource | Aynı `/FmShared` resource iki page-object invocation'ında kullanılarak fixture'da gözlenir. Güvenli clone-and-rebind primitive'i yoktur; shared instance in-place mutate edilmez. | **Ineligible**. |
| Clipping | Clip ile ilişkilendirilmiş contributor fixture'da vardır; clip attribution/rewrite kanıtı yoktur. | **Ineligible**. |
| Transparency/soft mask | Blend mode, alpha ve `/SMask`/transparency group birlikte fixture'da vardır; bounds bu graph'ı çözmez. | **Ineligible**. |
| Pattern/shading | Tiling pattern ve axial shading contributor'ları fixture'da mevcuttur. | İlk sürüm için kategorik **ineligible**. |
| Optional content | `/OCProperties`, kapalı OCG ve marked-content association fixture'da mevcuttur. Association/payload removal kanıtı yoktur. | **Ineligible**. |
| Tagged content | `/StructTreeRoot`, `/StructParents` ve MCID association fixture'da mevcuttur. Structure/ParentTree update primitive'i kanıtlanmamıştır. | **Ineligible**. |
| Annotation/widget | Page-object backend kapsamı dışındadır; mevcut conservative document/page policy korunur. | Annotation/widget katkısı whole-object seçemez. |
| Page geometry | Adversarial fixture non-zero/negative MediaBox origin ile Crop/Trim/Bleed/Art boxes taşır. Mutation backend eklenmediğinden preservation garantisi iddia edilmez. | GO olmadan production geometry contract değişmez. |

`FPDFPageObj_GetBounds()` intersection'ı hiçbir zaman eligibility değildir.
Rotated/text-matrix/scaled/skewed text, stroke width, path fill/stroke, image
transform/negative scale ve Form transform yalnız characterization corpus'unda
gözlenmiştir. Clipping, transparency, soft mask, pattern/shading, optional
content ve structure association bulunan object/page doğrudan ineligible'dır.
Path sınıfı için conservative stroked-effect proof olmadığından hiçbir path
whole-object candidate değildir. Text/image için dahi partial intersection,
overlap, mixed eligible+partial contributor veya ambiguous bounds bütün sayfayı
raster fallback'e yollar.

### Fixture, resource reachability ve verifier sonucu

Deterministik `whole-object-adversarial.pdf` fixture'ı plain/rotated/skewed
text; full/partial değerlendirmeye uygun text ve image contributor'ları;
positive/negative image transform; fill/stroked path; clip; transparency;
soft mask; pattern/shading; OCG; MCID/tag; nested Form ve aynı resource'u iki
kez kullanan shared Form içerir. Unique text marker'lar ve deterministic image
pixel payload'ı source-payload observation için kullanılır. Basit standalone
text ve image removal serialization probe'ları full rewrite, PDFium reopen,
lopdf decoded streams, serialized bytes/resource reachability ve `/Prev`
yokluğunu kontrol eder.

Bu gözlemler production verifier değildir. Mevcut
`secure_redaction_verifier_v1` bilinçli biçimde raster-profile authority'sidir
ve affected page için exactly-one-image invariant'ını zorunlu tutar.
Whole-object strategy-aware candidate inventory, removed payload correlation,
outside-region preservation ve visual-region absence check set'i eklenmemiştir.
Mandatory verifier gate bu nedenle de false'tur; hiçbir verifier check bypass
edilmemiştir.

### Fallback ve değişiklik sınırı

Production DTO/enum, plan digest, transform worker, report, frontend
normalization veya selector değişikliği yoktur. Planner whole-object strategy
seçemez; bütün affected pages mevcut raster strategy'yi kullanır. Feature
registry `redaction.enabled == false` / `status == disabled` kalır; legacy
`apply_redactions` hâlâ `FEATURE_UNAVAILABLE`, visual mask hâlâ mock/disabled'dır.
Original/working PDF, session generation/state ve digest-bound independent
verification → atomic publication hattı değişmemiştir.

Sanitizer/Miri **NOT RUN — uygun native PDFium sanitizer environment yok**.
Windows **NOT RUN**, Linux **NOT RUN**; yeni production raw binding veya ABI
yüzeyi eklenmemiştir. Optional backend NO-GO bir activation blocker değildir.
All-desktop activation'ın kalan borcu macOS x86_64, Windows ve Linux runtime
certification'dır.

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

Tüm 26 fixture `src-tauri/tests/fixtures/redaction/` dizininde bulunmakta olup `generate_redaction_fixtures.py` script'i ile deterministik olarak üretilmekte ve `manifest.json` dosyası kanonik formatta oluşturulmaktadır:

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

---

## 11. WebView-Compatible Digest, Semantik Plan Yeniden Doğrulaması ve Coverage-Safe Planner Kapanışı (Prompt 8B-3.5)

### 11.1 WebView Uyumlu Frontend Digest Doğrulaması
* Production frontend modülü (`redactionSchemas.js`), Node-dependent paketler (`node:crypto`, `Buffer`, stream API) olmadan tamamen standart Web Crypto API (`globalThis.crypto.subtle.digest('SHA-256', ...)` ve `TextEncoder`) ile çalışır.
* WebView ve modern tarayıcı ortamlarında 0 paket bağımlılığıyla SHA-256 plan ve geometri doğrulaması yapabilmektedir.
* Digest doğrulaması yapılmamış veya asenkron `verifyPlanDigestAsync` adımından geçmemiş hiçbir plan `digestVerified` / güvenilir sayılmaz.

### 11.2 Semantik Plan Yeniden Doğrulaması (Semantic Revalidation)
* Taze ve geçerli `planDigest` taşıyan fakat içerik veya mantık olarak tahrif edilmiş semantik sahte planlar (ör. metin varken `text_selection_lost = false` gösteren, belge kayıpları ile sayfa kayıpları uyuşmayan, engelleyici varken `executable = true` diyen veya kaynak kimlikleri sıralanmamış planlar) fail-closed ilkesi gereği Rust ve JS doğrulayıcılarında **kesinlikle reddedilir**.
* `validate_canonical_redaction_region_with_context` ile `original_normalized_rect` ile `base_pdf_point_rect` arasındaki matematiksel dönüşüm tutarlılığı sayfa geometrisi üzerinden `GEOMETRY_EPSILON` toleransı ile denetlenir.

### 11.3 Eşitlik ve Parite (Canonical Rule Parity)
* Planner, Rust validator, JS validator, manifest ve dokümantasyon aynı değişmez limit ve birleştirme profili anahtarlarına (`GEOMETRY_COMPARISON_EPSILON` dahil 4 birleştirme alanı, 5 limit alanı) sahiptir.
* `GEOMETRY_COMPARISON_EPSILON = 1e-6` sabitinin serileştirilmesinde tam anahtar seti ve değer eşitliği sağlanmıştır.

### 11.4 Kapsam Güvenliği (Coverage-Safe Resource Limits)
* Kaynak limitleri (`MAX_CANONICAL_REGIONS_PER_PAGE`, `MAX_SOURCE_REGION_IDS`, `MAX_REPORT_SIZE_BYTES`) aşıldığında redaksiyon kapsamı hiçbir koşulda kırpılmaz veya transform edilebilir kısmi kapsama dönüştürülmez.
* Aşım durumunda plan üzerinde `REDACTION_RESOURCE_LIMIT_EXCEEDED` engelleyicisi üretilir; plan `executable = false` ve `readyForExecution = false` işaretlenerek fail-closed güvenliği korunur.

### 11.5 Gerçek Sıfır I/O Yalıtımı (True No-I/O Planner Isolation)
* Planner çalıştırma sırasında disk, geçici dizin, dosya sistemi veya dış süreç bağlantısı kesinlikle kullanılmaz.
* Birim ve entegrasyon testlerinde planner öncesi ve sonrası geçici dizin dosya listeleri karşılaştırılarak sıfır yan etki (zero disk write) doğrulanmıştır.

---

## 12. Canonical Blocker Parity, Requirement Tekilleştirmesi ve Manifest Authority (Prompt 8B-3.6)

### 12.1 Manifest Authority Netliği
* `secure-redaction-contract-v1.json`: Makine tarafından okunabilir kanonik sözleşme ve limit otoritesidir.
* `manifest.json`: Test fixture korpusu ve test senaryoları otoritesidir.
* Biyektif anahtar kümesi doğrulaması ile hem Rust hem JS tarafında manifest `limits` ve `merge_profile` anahtar kümelerinin tam küme eşitliği doğrulanmıştır.

### 12.2 Acknowledgement Requirements Derivation Tekilleştirmesi
* Planner ve Validator katmanlarında `derive_redaction_acknowledgement_requirements` (Rust) ve `deriveAcknowledgementRequirements` (JS) fonksiyonları kanonik tek kaynak olarak kullanılır.
* Fact durumu `Unknown` olan belgelerde türetilen gereksinimlerin `is_present()` varsayımı nedeniyle sapması engellenmiş, blocked plan'ların doğrulayıcıdan eksiksiz geçmesi garanti edilmiştir.

### 12.3 Page ve Top-Level Blocker/Warning Aggregation Paritesi
* `evaluate_page_redaction_blockers` ile sayfa düzeyi blocker'ları sayfa fact ve geometrisinden kanonik olarak yeniden üretilir.
* Üst düzey `plan.blockers` ve `plan.warnings` dizileri document-level blocker'lar ile tüm sayfaların blocker/warning dizilerinin sıralı aggregasyonuna tam olarak denk olmak zorundadır.

### 12.4 Frontend Verified-Plan API ve Digest Verification Güvencesi
* `normalizeSecureRedactionPlanStructure(rawOrPlan)`: Yalnızca DTO ve invariant normalizasyonu yapar; dönüştürdüğü objeye kesinlikle `digestVerified: true` eklemez.
* `normalizeAndVerifySecureRedactionPlan(rawOrPlan)`: Full normalizasyon + Web Crypto SHA-256 digest doğrulamasını tamamlar ve `{ ...plan, digestVerified: true }` döndürür.
* `validateSecureRedactionPlan(rawOrPlan)`: Kamusal `async` API. Full Web Crypto SHA-256 digest doğrulamasını yürütür ve `true` döndürür (veya digest uyumsuzsa `REDACTION_PLAN_DIGEST_MISMATCH` fırlatır).
* `validateSecureRedactionPlanStructure(rawOrPlan)`: Senkron strüktürel doğrulama API'si.

---

## 13. Final Parity Fixes, UTF-8 Aggregation, Strict Manifest Authority ve 8B-3 Kapanışı (Prompt 8B-3.7)

### 13.1 External Oversized Wire Policy vs Internal Overflow
* **External Wire Validation Policy:** Her iki dilde de (Rust & JS) `pagePlan.canonicalRegions.length > MAX_CANONICAL_REGIONS_PER_PAGE` durumu strict `REDACTION_RESOURCE_LIMIT_EXCEEDED` doğrulama hatası olarak reddedilir. Resource blocker varlığı oversized external wire planını meşrulaştırmaz.
* **Internal Planning Overflow Policy:** Geçerli planner girdisi (`regionInputs.length <= 1000`) durumunda 100'den fazla çakışan kaynak bölgenin birleşmesi (`MAX_SOURCE_REGION_IDS = 100`) sonucu dahili kaynak aşımı meydana gelirse, planner fail-closed blocked plan üretir (`executable = false`, `readyForExecution = false`, `REDACTION_RESOURCE_LIMIT_EXCEEDED` blocker). Bu durumda hiçbir kaynak ID ve kapsama alanı silinmez veya kırpılmaz (coverage preservation). Oluşan plan kendi validator'ından başarıyla geçer.

### 13.2 UTF-8 Byte Aggregation Semantiği ve Paritesi
* Blocker ve Warning `details` aggregasyonu limit uygulaması (`MAX_DETAILS_LEN = 4096`) her iki dilde de **UTF-8 byte** sınırlarına göre uygulanır (JS string `.length` veya code point adedi değil).
* JS tarafında `truncateUtf8ToByteLimit` helper'ı ile `TextEncoder`/`TextDecoder` kullanılarak UTF-8 byte sınırında kesme yapılır; geçersiz surrogate veya yarım emoji/code point üretimi engellenir.
* Aggregasyon ve ilk `details` eklemesi (`pushAggregatedBlocker` / `pushAggregatedWarning`) UTF-8 byte bütçesine göre yapılır. ASCII, Türkçe, Latin-1, Emoji ve karmaşık boundary test vektörleriyle Rust ve JS exact parity'dedir.

### 13.3 Contract ve Manifest Authority Ayrımı
* `src-tauri/tests/fixtures/contracts/secure-redaction-contract-v1.json`: DTO/wire formatı, enum/error code ve kontrat limit otoritesidir.
* `src-tauri/tests/fixtures/redaction/manifest.json`: `secure_rasterized_pages_v1` planner kaynak limiti ve birleştirme profili fixture otoritesidir.
* İki dosya arasındaki ortak limitlerin (`MAX_SOURCE_REGION_IDS` vs `maxSourceRegionIdsPerCanonicalRegion`, `MAX_CANONICAL_REGIONS_PER_PAGE` vs `maxCanonicalRegionsPerPage`) eşitliği exact parity testi altındadır.

### 13.4 Arbitrary Warning Policy
* Warning'ler bilgilendirme/danışma amaçlıdır (informational/extensible).
* Warning'ler `executable` veya `ready_for_execution` kararını etkileyemez; blocker yerine kullanılamaz.
* Bilinmeyen warning kodları çalıştırma semantiğini değiştiremez.
* Unique warning kodları ve `MAX_WARNINGS` / UTF-8 byte limitleri kesinlikle denetlenir.

### 13.5 Structural vs Cryptographically Verified Frontend API
* Structural helper `normalizeSecureRedactionPlanStructure` kesinlikle `digestVerified: true` üretmez.
* Doğrulanmış asenkron helper `normalizeAndVerifySecureRedactionPlan` Web Crypto SHA-256 doğrulamasını tamamlayıp `digestVerified: true` döndürür.
* Kamu API'si `validateSecureRedactionPlan(mutant)` digest uyumsuzsa `REDACTION_PLAN_DIGEST_MISMATCH` hatası fırlatır.

---

## 14. Canonical Ordering Parity & Unicode Lexical Contract (Prompt 8B-3.8)

### 14.1 Canonical UTF-8 Lexical String Ordering Rule
* Secure-redaction kontratı genelinde tüm string sıralama (canonical string order) kararları **valid UTF-8 unsigned-byte lexicographical ascending order** semantiğine zorunludur.
* JS default UTF-16 code unit sıralaması (`<`, `.sort()`), `localeCompare()` veya Unicode normalization/NFC/NFD kullanılmaz.
* JS tarafında `compareCanonicalUtf8Strings(a, b)` helper'ı ile `TextEncoder` unsigned byte dizisi üzerinden byte-by-byte karşılaştırma yapılarak Rust std string `.cmp()` semantiği ile tam parity sağlanmıştır.
* Örneğin: `\uE000` (UTF-8 byte `0xEE...`) vs `😀` U+1F600 (UTF-8 byte `0xF0...`) karşılaştırmasında UTF-8 byte sıralaması `\uE000 < 😀` sonucunu verir.

### 14.2 Exact Finite Numeric Ordering (No Epsilon in Sorting)
* Canonical region sıralama semantiğinde `pageIndex`, `appliedRect.bottom`, `appliedRect.left` ve `regionId` kullanılır.
* `appliedRect.bottom` ve `left` karşılaştırmalarında `GEOMETRY_EPSILON` kullanılmaz; exact finite number ordering uygulanır (`100.0 < 100.0000005`). `GEOMETRY_EPSILON` yalnızca geometri çakışma ve kapsama doğrulamasında saklıdır.

### 14.3 Rust Wire Validator Canonical Region Order Check
* Rust `validate_page_redaction_plan` doğrulayıcısı dışarıdan gelen wire planlarının `canonical_regions` dizisini `compare_canonical_redaction_regions` helper'ı ile adjacent-pair olarak denetler; sırasız bölgeleri `REDACTION_INVALID_POLICY` ile reddeder.

---

## 15. Session-Native Secure Redaction Planning Worker, Snapshot Inspection ve IPC Sınırı (Prompt 8B-4)

### 15.1 Planning IPC Boundary
* `plan_secure_redaction` Tauri command'ı main Tauri process'i üzerinde PDF parsing veya snapshot generation yapmaz.
* Command girdisi: `jobId`, `sessionId`, `expectedContentGeneration`, `policy`, `acknowledgements`, `regions`.
* Command çıktısı: `SessionJobOutcome<SecureRedactionPlan>`.
* External file paths (`outputPath`, `targetPath`, `snapshotPath`), frontend-supplied digests veya geometries IPC girdisi olarak kabul edilmez; `unknownKeys: "reject"` ile reddedilir.

### 15.2 Session Snapshot Flow
* `DocumentSessionStore::run_snapshot_job(...)` kullanılarak immutable `snapshot.pdf` üzerinde ephemeral child worker process çalıştırılır.
* `sessionId + expectedContentGeneration` üzerinden stale-generation ve admission kontrolü yürütülür.
* Capturing context `context.captured_content_generation` plan girdisi olarak worker'a aktarılır; frontend'den gelen bağımsız nesnelere güvenilmez.

### 15.3 Worker Inspection Adapter (`secure_redaction_inspection.rs`)
* Ephemeral worker süreci içerisinde PDFium ve `lopdf` kullanılarak gerçek snapshot analizi yapılır.
* PDFium: Kanonik sayfa nesneleri, görsel/metin/vektör tespiti, sayfa boyutları, dönüş açısı (`rotation`) ve açıklama türü otoritesidir.
* `lopdf`: Katalog ve nesne düzeyi yapılar (`/AcroForm`, `/XFA`, `/Signatures`, `/ByteRange`, `/StructTreeRoot`, `/Names`, `/Prev`, `/OCProperties`) için yapısal analiz otoritesidir.

### 15.4 Tri-State Fact Semantics (Present / Absent / Unknown)
* Her belge ve sayfa fact'i `InspectionFactState::Present`, `Absent` veya `Unknown` olarak belirlenir.
* Bir özelliğin varlığı kanıtlandığında `Present`, kapsayıcı şekilde olmadığı kanıtlandığında `Absent`, belirsiz veya çözümlenemeyen durumlarda `Unknown` atanır.
* `Unknown` durumları asla sessizce `Absent` olarak değerlendirilmez; fail-closed blocker politikası korunur.

### 15.5 Snapshot Digest Binding
* `input_digest` değeri frontend'den alınmaz.
* Ephemeral worker, diskteki `snapshot.pdf` ham byte'larının SHA-256 digest'ini kendisi hesaplar (64 küçük harf hex karakter).

### 15.6 Credential Handling
* Session credential'ı süreç parametrelerinde, env variable'larda, log'larda veya `Debug` çıktılarında görünmez.
* Ephemeral request JSON dosyasında güvenli geçici dizin içerisinde tutulur ve işlem sonunda / iptal durumunda temizlenir.

### 15.7 Planning Cancellation Model
* Cancellation kontrolü `SessionJobCancellationCheckpoint` ve ephemeral child process kill/wait altyapısı ile yönetilir.
* Planning aşaması kalıcı hiçbir filesystem artifact'ı (`.pdf` çıktısı veya önbellek) bırakmaz.

### 15.8 Frontend Wire Serialization & Cryptographic Verification
* Frontend camelCase objeleri backend'e iletilmeden önce strict `toSecureRedactionPolicyWire`, `toRedactionAcknowledgementsWire` ve `toRedactionRegionInputWire` helper'ları ile Rust'ın beklediği snake_case wire DTO'larına dönüştürülür.
* Wrapper `startSecureRedactionPlan` backend'den dönen planı `normalizeAndVerifySecureRedactionPlan` üzerinden geçirir. Plan Web Crypto SHA-256 doğrulaması sonrasında yalnız `digestVerified: true` ile teslim edilir.

### 15.9 Known Inspection Limitations & 8B-5 Preconditions
* Şifreli belgeler veya imzalı belgeler inspection katmanında doğru tespit edilerek plan üzerinde uygun blocker'lar (`REDACTION_ENCRYPTED_INPUT_UNSUPPORTED`, `REDACTION_SIGNED_INPUT_UNSUPPORTED`) üretir; `executable = false` kalır.
* Bu aşamada henüz PDF transform, redaksiyon uygulaması, output PDF üretimi, verification veya publication yapılmamaktadır. `apply_redactions` komutu fail-closed `FEATURE_UNAVAILABLE` olarak kalır ve `redaction` feature'ı `disabled` durumdadır.

---

## 15.10 Secure Execution Orchestration ve Digest-Bound Publication

`execute_secure_redaction`, canonical `secureRedactionController` frontend workflow'ünden çağrılan session-native backend export sınırıdır. Tek bir authoritative session job altında zincir şöyledir:

```text
session → immutable snapshot → authoritative plan recomputation → transform worker
→ internal artifact → independent verifier worker → mandatory verification gate
→ digest-bound atomic publication → execution report
```

`expectedPlanDigest` planlama sonucu ile execution'ı bağlar; actual snapshot üzerinde worker tarafından yeniden hesaplanan plan eşleşmezse `REDACTION_PLAN_DIGEST_MISMATCH` ile hiçbir external write yapılmaz. Eski `apply_redactions` command'i bu zinciri kullanmaz ve fail-closed kalır.

### Publication trust boundary

Transform ve verifier worker protocol'leri external target, save-dialog yolu veya output publication path taşımaz; yalnız private job dizinindeki snapshot ve artifact'larla çalışır. Main process PDF semantiği, text veya visual verification üretmez. Main process yalnız target filesystem identity/preflight, SHA-256 binding, path lock ve atomic publication authority'sidir.

### Digest chain ve TOCTOU policy

```text
planDigest == transform.planDigest == verification.planDigest
inputDigest == immutable snapshot digest
transform.outputDigest == verification.transformDigest == publication.publishedDigest
```

Publication, verifier sonrasında artifact'ı tekrar hash'ler, adjacent staged temp'in digest'ini tekrar kontrol eder ve existing artifact bytes değişmişse target'a dokunmaz. Missing target için canonical namespace-bound `Missing`, existing target için canonical namespace + filesystem identity + SHA-256 expectation yakalanır; shared path lock altında atomic replacement'tan hemen önce tekrar doğrulanır. Aynı digest ile farklı file identity de `REDACTION_TARGET_CONFLICT` olur. Symlink/reparse-point, unsupported hard-link, special/directory target, session/job private paths ve original/working PDF aliases reddedilir.

### Atomic replacement, cancellation ve commit truth

Secure export, target preflight'ından publication sonuna kadar session operation read lease'ini taşır; Save/Save As paired write lease'i aldığından current save target, working path veya private roots bu aralıkta değiştirilemez. Publication mevcut `document_io` shared path-lock ve adjacent-temp primitive'ini kullanır: pre-delete yapılmaz, temp fsync edilir, replacement atomiktir, target identity ve digest commit sonrası kontrol edilir ve parent-directory durability mevcut platform modeline göre raporlanır. `publication.commitState` typed olarak `new_committed` veya `new_committed_durability_unconfirmed` değerini taşır. `RecoveryRequired` veya `Indeterminate` hata sonuçları typed `commitState`, `recoveryPath` ve `replacementDigest` metadata'sını terminal job failure'da korur; recovery artifact cleanup tarafından silinmez. Commit öncesi cancellation target'ı değiştirmez. Atomic commit başladıktan sonra cooperative cancellation kritik bölümü kesmez; job sonucu gerçek disk sonucunu gözlemler.

### 15.11 Execution acceptance status (8B-8.3)

**Runtime validated — macOS arm64:** `secure_redaction_execution_acceptance.rs`, gerçek session activation, immutable snapshot, transform worker, bağımsız verifier worker ve production `execute_secure_redaction_core` zincirini çalıştırır. Missing-target publication, existing-target atomic replacement, transform/verifier failure ve controlled deadline, invalid transform authority, mandatory verification gate, verifier-success sonrası artifact TOCTOU, staged-temp corruption, missing/existing target race, pre-transform ve pre-publication cancellation ile post-commit-boundary disk-truth sonucu deterministic phase synchronization ile doğrulanır. Başarılı zincirler gerçek worker process'lerini kullanmaya devam eder; environment-variable veya sleep tabanlı coordination yoktur.

**Boundary validated:** `document_io` testleri same-digest/different-identity target değişimini, symlink/directory/unsupported hard-link reddini, pre-commit `OriginalPreserved`, `NewCommitted`, `NewCommittedDurabilityUnconfirmed`, `RecoveryRequired` ve `Indeterminate` state propagation'ını kapsar. `document_session` testleri generic child kill/reap/timeout, snapshot/job/credential cleanup ve publication failure `commitState`/`recoveryPath`/`replacementDigest` wire metadata köprüsünü doğrular. Worker/verifier corpus testleri full rewrite, reopen ve pixel/placement/stale-digest corruption regresyonlarının otoritesidir.

**Not run:** macOS x86_64 runtime execution acceptance — architecture unavailable. Windows runtime execution acceptance — environment unavailable. Linux runtime execution acceptance — environment unavailable. Platform-specific implementation ve cfg-gated filesystem testleri korunur; gerçek runtime certification feature activation/release öncesinde ayrıca yapılmalıdır. Feature matrix'te redaction `disabled`, eski `apply_redactions` ise `FEATURE_UNAVAILABLE` kalır.

---

## 16. Güvenli Raster Dönüşüm Primitive’leri, PDFium Device Mapping ve Lossless Pixel Pipeline (Prompt 8B-5 – 8B-5.3)

### 16.1 Modül Yapısı ve Mimari Sınırlar
* **Raster Modülü Konumu:** `src-tauri/src/secure_redaction_raster.rs` (`pub mod secure_redaction_raster;` `lib.rs` içerisinde).
* **İzolasyon Kuralı:** Modül Tauri state, external path, session store, frontend state veya viewer render cache'ine erişmez. Disk üzerinde hiçbir `.pdf`, `.png`, `.jpg` veya geçici dosya oluşturmaz (`no-output invariant`).
* **Bağımlılık Kısıtı:** Yeni dependency eklenmemiştir (`pdfium-render v0.8.37`, `lopdf v0.34.0` korunmuştur). İkinci bir render motoru (PDF.js, MuPDF vb.) kesinlikle eklenemez.

### 16.2 Plan-bound Raster API ve Readiness Gate
* `build_lossless_raster_payload_for_page(plan: &SecureRedactionPlan, document: &PdfDocument, page_index: u32, cancellation)` imzası ile rasterizasyon doğrudan tam plan yetkisine bağlanmıştır. Primitive, sayfa handle’ını yalnızca bağlı `document.pages().get(page_index)` çağrısıyla edinir; caller tarafından `PdfPage` injection kabul edilmez.
* Caller tarafından bağımsız `SecureRedactionPolicy` injection kabul edilmez; tüm render parametreleri (DPI, boyutlar) strictly `plan.policy` üzerinden alınır.
* **Geometry identity denetimi:** `PdfPage::width()/height()` raw MediaBox boyutları değildir; CropBox ve intrinsic rotation’dan etkilenen effective display size’dır. Bu nedenle raw MediaBox ile karşılaştırılmaz. Primitive, PDFium’dan `boundaries().media()`, distinct `boundaries().crop()` ve intrinsic `rotation()` okur; MediaBox, CropBox varlığı ve `left`, `bottom`, `width`, `height` değerlerini `GEOMETRY_EPSILON` ile plan geometry’siyle karşılaştırır. Effective box, CropBox varsa CropBox, yoksa MediaBox’tır ve ayrıca doğrulanır. Her okunamayan veya finite olmayan zorunlu geometry değeri `REDACTION_PAGE_GEOMETRY_MISMATCH` ile fail-closed olur.
* `rotation()` için production dalı `Err(_) -> REDACTION_PAGE_GEOMETRY_MISMATCH` invariant’ını taşır. PDFium rotation-read failure üreten bir fixture/adaptör bulunmadığından bu dal fixture ile sertifikalandırılmaz.
* **Fail-closed Preflight Denetimleri:**
  * `plan.executable == true`
  * `plan.ready_for_execution == true`
  * `plan.blockers.is_empty()`
  * `page_plan.executable == true`
  * `page_plan.blockers.is_empty()`
  * `page_plan.strategy == PageRedactionStrategy::RasterizePage` (`ImportUnaffectedPage` strategy ile raster isteği `REDACTION_INVALID_REGION` fırlatır)
  * `!page_plan.canonical_regions.is_empty()`

### 16.3 Plan Immutability ve Digest Koruması
* Planner tarafından üretilen ve cryptographic digest'e bağlanan `SecureRedactionPlan` nesnesi raster primitive katmanı tarafından **mutate edilmez**.
* `planDigest`, `geometryDigest` ve `canonicalRegions` listesi byte/semantik olarak tam eşitliğini korur; `device_pixel_rect` alanları plan içerisinde `None` kalır.
* Pixel mapping ve rasterization sonuçları plan nesnesinden tamamen ayrı olan internal `RasterizedRegionMapping` ve `LosslessRasterPayload` nesnelerinde taşınır.

### 16.4 Shared Render Config Authority ve High-Level Conversion API
* `create_redaction_render_config(pdfium_page, spec)` helper'ı hem sayfa renderizasyonunun hem de coordinate mapping'in tek yapılandırma otoritesidir.
* **Explicit Render Bayrakları:**
  * `.set_fixed_size(viewport_width, viewport_height)`
  * Relative rotation: `(target_deg + 360 - intrinsic_deg) % 360` ile intrinsic sayfa rotasyonu hesaba katılarak uygulanır.
  * `.clear_before_rendering(true)` ve `.set_clear_color(PdfColor::WHITE)`
  * `.render_annotations(false)` ve `.render_form_data(false)` (AcroForm/Widget ve Annotation render'ı kesinlikle kapalıdır)
  * `.set_reverse_byte_order(true)`
  * `.use_lcd_text_rendering(false)` ve `.disable_native_text_rendering(true)` (Platformlar arası deterministik anti-aliasing)
* **Coordinate Conversion API:** Raw C API `FPDF_PageToDevice` yerine, `pdfium-render` high-level `pdfium_page.points_to_pixels(...)` metodu exact `render_config` ile çağrılır.

### 16.5 Preflight, Kaynak Sınırları ve Checked Arithmetic
* `calculate_raster_dimensions` helper'ı `geom.effective_box`, `rotation` ve `plan.policy.raster_dpi` üzerinden Checked Arithmetic ile `(width, height)` hesaplar (`points / 72.0 * DPI`).
* **Aktif Limit Denetimleri (Fail-Closed):**
  * `MIN_RASTER_DPI` (72) <= DPI <= `MAX_RASTER_DPI` (600)
  * `width`, `height` <= `MAX_PIXEL_DIMENSION` (16,384)
  * `total_pixels` <= `MAX_PIXELS_PER_PAGE` (100,000,000)
  * `total_bytes` <= `MAX_RENDER_BYTES` (256 MB)
* Sınır aşımında `REDACTION_RESOURCE_LIMIT_EXCEEDED` veya `REDACTION_INVALID_POLICY` fırlatılır. Float -> integer dönüşümünde checked rounding ve bounds guard uygulanır.

### 16.6 PDFium Coordinate Mapping, Viewport Invariant ve Outward Rounding
* Her `applied_pdf_point_rect` için `pdfium_page.points_to_pixels(...)` çağrısı ile köşe koordinatları cihaz piksellerine dönüştürülür.
* **Viewport Dimension Invariant:** Render tamamlandıktan hemen sonra `bitmap.width == viewport_width && bitmap.height == viewport_height` kontrol edilir; uyumsuzluk durumunda `REDACTION_PAGE_GEOMETRY_MISMATCH` fırlatılır.
* **Outward Rounding:** `left = min_x.floor() as i32`, `top = min_y.floor() as i32`, `right = max_x.ceil() as i32`, `bottom = max_y.ceil() as i32`. Pozitif point alanları en az 1 piksel kapsama alanına yuvarlanır.
* **Bitmap Boundary Intersection:** `[0, bitmap_width)` ve `[0, bitmap_height)` ile kesişim hesaplanır; sınır daraltmaları mapping kanıtında `boundary_clamped: true` olarak raporlanır. Sayfa dışı kalan alanlar reddedilir.

### 16.7 Deterministik Pixel-Space Merge ve Multi-Page Protection
* `merge_device_pixel_rects` fonksiyonu tüm girdi `RasterizedRegionMapping` elemanlarının aynı `page_index` değerine sahip olduğunu doğrular (farklı sayfa index'i `REDACTION_INVALID_REGION` üretir).
* Örtüşen (`overlapping`) veya kenar-temaslı (`edge-touching`) `DevicePixelRect` nesneleri deterministik olarak birleştirilir.
* Birleştirilen bölgelerin `source_region_ids` listeleri birleştirilir, sıralanır ve tekilleştirilir.

### 16.8 Arka Plan, Piksel Formatı, Overwrite ve Cancellation
* **Background Policy:** `opaque_white_v1` (`RGBA8 255, 255, 255, 255`). Output alpha değerleri pixel-level doğrulanır. Transparent source-object composition bağımsız bir fixture ile karakterize edilmemiştir; bu nedenle yalnız opaque white clear/background ve output alpha kanıtı iddia edilir.
* **Canonical Pixel Format:** `RGBA8` (8 bit/kanal, R, G, B, A sıralaması, Stride = `width * 4`). Tightly-packed buffer `bitmap.as_rgba_bytes()` ile elde edilir ve sıkı stride kontrolü (`pixels.len() == stride * height`) yapılır.
* **Opak Siyah Overwrite:** `apply_opaque_black_overwrite` birleştirilmiş alanları strictly `R=0, G=0, B=0, A=255` pikselleriyle ezer. Checked row/x/alpha index aritmetiği kullanılır.
* **Row-Level Cancellation:** 64 satırda bir ve tüm aşamalarda `SessionJobCancellationCheckpoint` kontrolü yapılır.

### 16.9 Test Matrisi ve 8B-6 Readiness Gate
* **68 Adet Unit / Entegrasyon Testi:**
  * Tests 1–11: DPI/boyut/limit denetimleri ve checked arithmetic.
  * Tests 12–16: Primitive-level rotation exact parity (0°, 90°, 180°, 270° sayfalarının her biri için intrinsic rotasyon, viewport boyutları, bilinen canonical PDF point rectangle, `DevicePixelRect.left`, `top`, `width`, `height` exact assertion'ları ve `points_to_pixels` -> `pixels_to_points` round-trip doğrulaması).
  * Test 17: CropBox exact parity (`cropbox-mediabox-diff.pdf` üzerinde `[50,50,550,750]` effective box, `500x700` viewport, köşe haritalamaları `(50,50) -> (0,700)` ve `(550,750) -> (500,0)`, dahili dikdörtgen haritalaması).
  * Test 18: Non-zero origin exact parity (`nonzero-origin.pdf` üzerinde `(50,50)` origin, `600x800` viewport, köşe haritalaması `(50,50) -> (0,800)` & `(650,850) -> (600,0)`).
  * Test 19: Negative origin exact parity (`negative-origin.pdf` üzerinde `(-100,-100)` origin, `600x800` viewport, köşe haritalaması `(-100,-100) -> (0,800)` & `(500,700) -> (600,0)`).
  * Tests 20–25: Outward rounding, 1-pixel minimum coverage ve boundary intersection evidence testleri.
  * Tests 26–31: Deterministik pixel merge ve source ID koruması.
  * Tests 32–40: Opaque black overwrite, RGBA8 kanal sırası, stride validation, white background policy (`[255,255,255,255]`), opaque output alpha (transparent source composition fixture sertifikasyonu değildir) ve lossless payload roundtrip.
  * Tests 41–44: Cancellation checkpoints (render öncesi, paint sırasında, mapping aşamasında).
  * Tests 45–48: Plan immutability, digest regression ve filesystem no-output invariant.
  * Tests 49–53: Full-plan readiness gate (unacknowledged plan refusal, document blocker refusal, strategy validation, empty region guard, policy authority enforcement).
  * Test 54: Annotation render flag real pixel testi (`render_annotations(true)` vs `render_annotations(false)` pikseller arası fark kanıtı ve `DoNotRender` durumunda piksel beyazlığı `[255,255,255,255]`).
  * Test 55: AcroForm widget blocked full plan testi.
  * Test 56–58: Text/image rasterization, multi-page error guard, buffer stride validation.
  * Test 59: Real RGB channel-order color fidelity (2x2 `/DeviceRGB` resmi üzerinde Red `[255,0,0,255]`, Green `[0,255,0,255]`, Blue `[0,0,255,255]`, White `[255,255,255,255]` piksel değerlerinin exact doğrulanması).
  * Tests 60–62: Real blocker fixture regressions (`encrypted-redaction.pdf`, `signature-marked-byterange.pdf`, `xfa-fixture.pdf` için plan üretimi ve `build_lossless_raster_payload_for_page` fail-closed reddi).
  * Test 63: Same-size/same-rotation fakat farklı görünür içerikli iki-sayfalı fixture üzerinde document-bound exact-index identity. Page-1 handle injection kabul eden API yoktur; page-0 payload’ında yalnız page-0’ın kırmızı içeriği pixel-level doğrulanır.
  * Test 64: Lower-level relative rotation render-config davranışının doğru adlandırılmış karakterizasyonu.
  * Test 65: Her dört rotation için gerçek plan-bound payload üretimi; page index, payload boyutu ve canonical region device mapping exact assert edilir.
  * Test 66: `cropbox-mediabox-diff.pdf` için gerçek full pipeline; MediaBox `[0,0,600,800]`, CropBox `[50,50,550,750]`, 72 DPI `500x700` payload ve exact mapping assert edilir.
  * Tests 67–68: Non-zero ve negative MediaBox origin fixture’ları için gerçek full pipeline geometry, payload dimensions ve exact mapping assert edilir.
* **8B-6 regression gate:** Full-pipeline 0/90/180/270 rotation, CropBox != MediaBox, non-zero origin, negative origin, exact device mapping, document-bound page identity, plan/digest immutability ve no-output invariant birlikte zorunludur.

## 17. 8B-9 Frontend Workflow (Implemented, Feature Disabled)

Production kaynak kodunda frontend workflow uygulanmıştır; bu bir activation kararı değildir. Feature registry kaydı `status: disabled` ve `enabled: false` kalır. Kullanıcıya runnable bir kontrol sunulmaz ve legacy `apply_redactions` hâlâ parse öncesi `FEATURE_UNAVAILABLE` döndürür.

### 17.1 Pending region ve manual selection sınırı

* `SecureRedactionState` yalnız versioned `RedactionRegionInput { id, pageIndex, normalizedTopLeftRect, source }` modelini saklar. Her ekleme/değiştirme ve backend gönderimi `validateRedactionRegionInputs` üzerinden geçer; UUID tabanlı region kimlikleri UI sıra index'inden bağımsızdır.
* `PdfPage` review-selection katmanı PDF'yi mutate etmez, annotation store'a yazmaz ve black/opaque mask göstermez. Kesikli amber/kırmızı tint ve `PENDING` etiketi seçimin yalnız aday olduğunu belirtir.
* Pointer koordinatları viewport sınırında clamp edilir; üç pikselden küçük/zero-area seçimler reddedilir. 0/90/180/270 viewer rotation ters dönüşümü yalnız interaction sırasında logical, rotation-independent top-left normalized rectangle üretmek için kullanılır. Stored rectangle viewer rotation değişince mutate edilmez; presentation dönüşümü render-time'dır.
* Region `pageIndex` physical backend page index'idir. Navigation sırasında `viewer.pageOrder.indexOf(pageIndex)` ile display index bulunur.

### 17.2 Sensitive scan promotion

Sensitive-data scan ayrı detection/review state ve controller'ında kalır. Yalnız kullanıcı `Redaksiyon adayı olarak ekle` eylemini açıkça seçtiğinde, `geometryStatus === resolved`, geçerli `pageGeometry` ve non-empty `sourceRectsPdfPoints` bulunan item'lar mevcut `pdfPointRectToNormalizedTopLeft` helper'ıyla pending bölgelere çevrilir. Her source rect yeni unique ID alır; unresolved/missing geometry için fallback rectangle üretilmez. Scan tamamlanması planlama veya execution başlatmaz. Redaction feature gate kapalıyken promotion kontrolü runnable/görünür değildir.

### 17.3 Planning, two-pass review ve digest ownership

State planning ve execution lifecycle'larını ayrı tutar; `idle`, `editing`, `planning`, `review_required`, `ready`, `executing`, `completed`, `failed`, `cancelled` durumları aynı semantik modelden türetilir. `plan != null` readiness değildir. Execution yalnız `digestVerified === true`, `executable === true`, `readyForExecution === true` ve empty blockers koşullarının tamamında açılır.

İlk plan mevcut, başlangıçta tamamı `false` acknowledgement setiyle alınır. UI yalnız backend `acknowledgementRequirements` içinde required olan ayrı kayıp kabullerini gösterir. Region, geometry, policy veya herhangi bir acknowledgement değişikliği planı ve sonucu invalidate eder. Kullanıcının kabulleri değiştirmesinden sonra yeni authoritative plan alınmadan Save veya execution mümkün değildir; checkbox'lar blocker'ları bypass etmez.

Conservative tek policy authority 300 DPI, 2 pt safety margin, `secure_rasterized_pages_v1`/`rasterized_pages`, solid black opacity 1 ve interactive/signature/encryption `block` değerlerini kullanır. UI yalnız contract sınırları içindeki DPI (72–600) ve safety margin (0–36 pt) alanlarını düzenler.

### 17.4 Stale ownership ve context reconciliation

Controller her plan/execution başlangıcında session ID, content generation, canonical job ID, local request token ve workflow revision capture eder. Sonuç ancak bu kimliklerin tamamı güncelse state'e uygulanır. Path stale-result authority değildir. Session/PDF/generation değişimi, PDF kapanması, non-PDF geçişi ve runtime destroy active handle'ları cancel eder ve transient state'i temizler. Panel kapanırken active job iptal edilir; açıkça seçilmiş pending bölgeler aynı belge context'inde korunur, fakat plan/result/selection state'i temizlenir.

### 17.5 Save, execution, cancellation ve publication truth

Save dialog yalnız latest ready plan sonrasında `src/services/platform/dialogs.js` üzerinden PDF filtresi ve `<base>-secure-redacted.pdf` önerisiyle açılır. Dialog cancel edilirse plan review korunur ve execution başlamaz. Seçilen target string yalnız backend'e taşınır; filesystem preflight, source/alias/link ve atomic publication authority Rust katmanındadır.

Execution input'u exact policy, acknowledgement, regions, verified plan digest, session/generation ve workflow revision'ın immutable frontend snapshot'ıdır. Editing execution boyunca kapalıdır. `startSecureRedactionExecution` production callsite'ı yalnız `secureRedactionController` içindedir; component raw `invoke`, `callBackend` veya command string kullanmaz.

Backend live sub-phase/yüzde event'i vermediği için UI sahte yüzde üretmez; transform, bağımsız verification ve publication zincirini high-level wording ile gösterir. Cancel current handle/job ID'ye bağlanır. Cancel isteğinden sonra backend published report döndürürse disk truth başarı olarak gösterilir ve isteğin commit boundary sonrasında ulaştığı açıklanır.

`new_committed` normal verified success'tir. `new_committed_durability_unconfirmed` bytes'ın commit edildiğini inkâr etmeden üst dizin durability warning'i gösterir. Failed job outcome içindeki `commitState`, `recoveryPath` ve `replacementDigest` metadata'sı controller tarafından korunur; `recovery_required`, `indeterminate`, target conflict, verification failure ve stale plan ayrı güvenli kullanıcı mesajlarına map edilir. Successful export active session'ı veya current document'ı değiştirmez; yeniden execution için explicit `Yeni redaksiyon` reset'i gerekir.

### 17.6 Activation ve platform borcu

Frontend ve canonical callsite'ın production source'ta bulunması feature activation anlamına gelmez. `redaction.enabled == false`, `visual_redaction_mask.enabled == false` ve legacy `apply_redactions == FEATURE_UNAVAILABLE` invariant'ları korunur. macOS arm64 execution acceptance doğrulaması geçerlidir. macOS x86_64, Windows ve Linux runtime execution acceptance **NOT RUN** borcu activation certification'a taşınmıştır.

## 18. Prompt 8B Final Audit ve Activation Kararı

### 18.1 Karar

**8B COMPLETE — ACTIVATION DEFERRED.** `secure_rasterized_pages_v1` production
profili ve explicit rectangular secure-redaction export zinciri tamamlanmıştır.
Registry advertised platform kümesi `ALL_DESKTOP_PLATFORMS` olduğu halde yalnız
macOS arm64 runtime acceptance çalıştırılmıştır. Yeni platform/architecture gate
icat edilmemiştir; bu nedenle `redaction.status == "disabled"` ve
`redaction.enabled == false` korunur. Release platform certification tamamlanınca
activation ayrı bir karardır; ilk activation yapılırsa status `experimental`
olmalıdır, `stable` değildir.

| Platform | Certification |
|---|---|
| macOS arm64 | **PASS** — real worker transform, independent verifier ve atomic publication acceptance |
| macOS x86_64 | **NOT RUN** — bu architecture ortamı yok |
| Windows | **NOT RUN** — runtime environment yok |
| Linux | **NOT RUN** — runtime environment yok |

### 18.2 Final blocker matrisi

| Kategori | Sonuç | Kanıt/sınır |
|---|---|---|
| Backend security | **PASS** | immutable snapshot, session/generation authority, private workers, plan recomputation ve target TOCTOU guards |
| Independent verification | **PASS** | transform artifact ayrı worker tarafından yeniden açılır; failed/`not_run` verification publish edilemez |
| Atomic publication | **PASS** | verifier digest binding, target identity/digest precondition ve typed commit states |
| Frontend workflow | **PASS** | pending review semantics, two-pass acknowledgement, ready-only Save dialog, canonical controller ve stale-result guards |
| Whole-object optional strategy | **NO-GO — non-blocking** | detached-object lifetime SIGSEGV ve diğer mandatory proof eksikleri; production strategy eklenmemiştir |
| macOS arm64 runtime | **PASS** | `secure_redaction_execution_acceptance` |
| macOS x86_64 runtime | **NOT RUN** | architecture unavailable |
| Windows runtime | **NOT RUN** | environment unavailable |
| Linux runtime | **NOT RUN** | environment unavailable |
| Documentation parity | **PASS** | feature matrix, IPC inventory ve bu karar aynı disabled/8C sınırını taşır |
| Production test-seam isolation | **PASS** | seam'ler yalnız Cargo `secure-redaction-test-support` opt-in feature'ında derlenir; default/release build, Tauri IPC, environment ve frontend payload ile etkinleşmez |

### 18.3 Production surface ve test seam sınırı

Public canonical operasyonlar `plan_secure_redaction`,
`execute_secure_redaction` ve generic `cancel_session_job` mekanizmasıdır. Worker
transform/verifier command'ları, private snapshot/job path'leri ve ara artifact
frontend'e açılmaz. Main Tauri process PDF planning/transform/verification
parser'ı değildir; yalnız session authority, orchestration, target filesystem
authority ve publication yürütür. Legacy `apply_redactions` fail-closed kalır.

8B-8.3 phase hooks, controlled child failure/timeout, result injection ve
staged-temp corruption seam'leri yalnız explicit
`secure-redaction-test-support` Cargo feature'ı ile integration acceptance
derlemesine alınır. Default feature set boştur. Production command yalnız
`execute_secure_redaction_core` çağırır; environment variable, IPC payload,
frontend input veya runtime configuration test authority üretemez. Whole-object
SIGSEGV probe'u yalnız `tests/whole_object_backend_characterization.rs` child
process'idir ve production module/import/runtime dependency değildir.

### 18.4 Güvenlik iddiası ve 8C sınırı

8B yalnız kullanıcının açıkça seçtiği bölgeleri yeni bir export dosyasında
destructive olarak işler. Sensitive-data scan completeness garantisi vermez;
working/original PDF'yi güvenli veya sanitize edilmiş ilan etmez. Etkilenen
sayfalar rasterize edilir; metin seçimi/arama, erişilebilirlik, annotation,
form ve vektör kalitesi contract'ta bildirildiği ölçüde kaybolabilir.

8A sensitive-data detection/geometry foundation, 8B explicit rectangular
secure-redaction export, 8C ise document-wide sanitization/hidden payload/
metadata cleaning kapsamıdır. `sanitize_pdf_document == FEATURE_UNAVAILABLE`
ve `sanitization.enabled == false` kalır; bu kapanışta 8C implementation yoktur.
