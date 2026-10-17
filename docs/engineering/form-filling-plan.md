# Form Doldurma (AcroForm) — Uygulama Planı

Referans: PdfCraft 0.8.0 `crates/forms` (MIT OR Apache-2.0). Kod taşınmayacak;
alan modeli, kenar durumları ve test listesi referans alınacak. Doğrudan kod
uyarlanırsa MIT/Apache bildirimi korunur.

Yeni npm paketi, crate veya plugin **gerekmiyor**. Her şey mevcut
`pdfium-render 0.8.37` içindeki ham `FORM_*` / `FPDFAnnot_*` binding'leriyle yapılır.

---

## 1. Mevcut durum

| Katman | Durum |
|---|---|
| Feature gate | `src/featureRegistry.js:266` `form_editing` → `disabled`, `enabled: false` |
| Frontend | `src/components/FormLayer.svelte`: boş bir `div`. `PdfPage.svelte:926` yalnız `formPresentation.runnable` olduğunda mount ediyor |
| Backend | Yok. Eski `manipulation::{get_form_fields,update_form_field}` stub'ları kaldırılmış (`feature-matrix.md:83`, durum `scaffold`) |
| Render | `worker/pdfium.rs:547` `PdfRenderConfig` varsayılanı `render_form_data=true` → widget'lar `FPDF_FFLDraw` ile bitmap'e statik çiziliyor |
| Okuma worker'ı | `worker/ipc.rs` `WorkerCommand`: salt-okunur, stdout = IPC frame kanalı |
| Yazma | `job_worker.rs` `MutationRecipe` + `mutate_isolated` → ayrı süreç, snapshot üzerinde PDFium, yeni generation, `ValidateMutationOutput` |
| Kayıt | `pdfium-render` `save_to_file` → `FPDF_SaveAsCopy(flags=0)`: **tam yeniden yazım, incremental değil** |

### pdfium-render'ın yüksek seviye form API'si neden kullanılmayacak

`pdfium-render-0.8.37/src/pdf/document/page/field/`:

- `PdfFormTextField::set_value` yalnız `/V`'yi `FPDFAnnot_SetStringValue` ile yazıyor ve
  **`/AP` görünüm akışını yeniden üretmiyor**. Kaydedilen dosya diğer
  görüntüleyicilerde eski değeri gösterir.
- `PdfFormCheckboxField::set_checked` on-state'i `"/Yes"` olarak sabit kodluyor. On-state'i
  `/On`, `/1`, `/Choice1` ya da Shift-JIS bir isim olan checkbox'lar bozulur
  (PdfCraft `Widget::on_state`, `name_text` bu durumları ayrıca ele alıyor).
- `PdfFormRadioButtonField::set_checked` içinde **iki adet `println!`** var. Bu çağrı render
  worker'ında çalışırsa stdout IPC frame'lerini bozar. Kardeş widget'ların `/AS`'ini de
  güncellemiyor.

**Karar:** Değerler PDFium'un form-fill ortamı üzerinden yazılır: `FORM_SetFocusedAnnot`,
ardından `FORM_SelectAllText` + `FORM_ReplaceSelection`, `FORM_SetIndexSelected`,
buton tıklaması/boşluk tuşu ve en sonda `FORM_ForceToKillFocus`. Bu yol PDFium'un
`CPDF_InteractiveForm` katmanını çalıştırır: `/V`, `/AS` ve `/AP` birlikte güncellenir,
radio grupları ve kardeş widget'lar PDFium tarafından tutarlı tutulur.

---

## 2. Etkilenecek dosyalar

**Rust (src-tauri/src)**
- `forms.rs` *(yeni)*: alan DTO'ları, PDFium okuma ve yazma yardımcıları, limitler, birim testleri
- `worker/ipc.rs`: `WorkerCommand::{GetFormInfo, GetPageFormFields}`, ilgili `WorkerCapability` ve `WorkerResult`
- `worker/pdfium.rs`: iki yeni komutun handler'ı
- `job_worker.rs`: `MutationRecipe::FillFormFields` (+ `operation_name`)
- `document_session.rs`: `get_session_form_info`, `get_session_page_form_fields`, `fill_session_form_fields` komutları
- `lib.rs`: `generate_handler!` kaydı, `mod forms`
- `annotations.rs`: `PageDisplayTransform` ve normalize dikdörtgen yardımcıları yeniden kullanılır (gerekirse `pub(crate)`)

**Frontend (src)**
- `components/FormLayer.svelte`: Svelte 5 runes ile baştan yazılır
- `components/PdfPage.svelte`: FormLayer'a sayfa, session, generation ve boyut prop'ları
- `features/forms/formState.svelte.js` *(yeni)*: taslak değerler, dirty takibi, commit kuyruğu
- `services/backend/contracts.js`, `session.js` (veya yeni `forms.js`): 3 komut sözleşmesi
- `featureRegistry.js`: `form_editing` → `experimental` (son aşamada)

**Dokümantasyon**
- `docs/engineering/feature-matrix.md` satır 83, `ipc-inventory.md`

---

## 3. Değişiklik adımları

### Faz 0 — Spike: PDFium FORM API doğrulaması (kodu atılabilir)

Amaç, yazma yolunun gerçekten `/AP` ürettiğini **ölçerek** kanıtlamak. Bu faz başarısız olursa plan değişir.

1. Test fixture'ları: tek satır, multiline, comb + MaxLen, password, read-only, required;
   on-state'i `Yes` olmayan checkbox; `/Opt`'lu checkbox (pdf-lib stili `/0`, `/1`);
   3'lü radio grubu; combo (editable ve değil); multi-select list; aynı isimli iki widget'lı
   alan; `/Fields`'te listelenmemiş widget; `/MK /R 90` döndürülmüş alan; Türkçe/Japonca
   değer. Fixture'lar test içinde ham PDF baytı olarak üretilir (PdfCraft `tests.rs`
   yaklaşımı; yeni bağımlılık yok).
2. job_worker'da: sayfayı yükle, `FORM_OnAfterLoadPage`, annot'u bul, focus, değer yaz,
   `FORM_ForceToKillFocus`, `FORM_OnBeforeClosePage`, `FPDF_SaveAsCopy`.
3. Çıktıyı yeni bir PDFium örneğinde aç ve `/V`, `/AS`, `/AP /N` akışının değiştiğini doğrula.
   Mevcutsa `external_validation` (qpdf/poppler) ile bağımsız kontrol yap.
4. Ölçümler: checkbox/radio için `FORM_OnChar(' ')` mı `FORM_OnLButtonDown/Up` mu daha
   güvenilir? `pdfium-render` form handle'ı her sayfa için `FORM_OnAfterLoadPage` çağırıyor
   mu? Kullandığımız PDFium build'inde (V8 yok) değer değişince format/calculate
   script'leri sessizce atlanıyor mu?

**Çıkış kriteri:** Fixture'ların tamamında `/AP` yenileniyor ve yeniden açılınca değer doğru render ediliyor. Sorunlu türler yazılı olarak kayda geçiriliyor.

#### Faz 0 sonuçları (2026-10-17, macOS arm64, PDFium 152.0.7961.0, `pdf_enable_v8=false`, `pdf_enable_xfa=false`)

Spike: `src-tauri/tests/form_fill_spike.rs` (`#[ignore]`, manuel). Fixture lopdf ile test
içinde üretilir. Doğrulama üç yoldan yapıldı: (a) lopdf ile `/V`, `/AS`, `/AP /N`
karşılaştırması, (b) taze bir PDFium örneğinde yeniden okuma, (c) macOS PDFKit
(bağımsız motor) ile değer okuma ve render. qpdf/poppler makinede kurulu değil;
çalıştırılmadı.

| Alan | Yöntem | `/V` | `/AS` | `/AP` | Not |
|---|---|---|---|---|---|
| Tek satır metin | `FORM_SelectAllText` + `FORM_ReplaceSelection` | ✅ | — | ✅ yeniden üretildi | |
| Multiline | aynı (`\r` ile satır) | ✅ `line one\r\nline two` | — | ✅ iki satır | |
| Comb + MaxLen 6 | aynı, 8 karakter gönderildi | ✅ `ABCDEF` | — | ✅ hücrelere dağıtılmış | PDFium **sessizce kırpıyor** |
| `/MK /R 90` | aynı | ✅ | — | ✅ döndürülmüş çiziliyor | |
| `/Fields` dışı widget | aynı | ✅ | — | ✅ | PDFium page-only widget'ı alan olarak kabul ediyor |
| Checkbox, on-state `/On` | `FORM_OnChar(' ')` | ✅ `/On` | ✅ `/On` | mevcut durum akışı | Sabit `/Yes` sorunu yok |
| Checkbox, `/Opt` + on-state `/0` | `FORM_OnChar(' ')` | ✅ `/0` | ✅ `/0` | mevcut | Export value `Export-A` doğru raporlanıyor |
| Radio (3 kardeş) | `FORM_OnChar(' ')` | ✅ `/Blue` | ✅ kardeşler `/Off` | mevcut | |
| Combo | `FORM_SetIndexSelected` | ✅ `JP` | — | ✅ | |
| Multi-select list | `FORM_SetIndexSelected` ×2 | ✅ `[English, Japanese]` | — | ✅ seçim vurgulu | |
| **Read-only metin** | aynı | ❌ **`hacked` yazıldı** | — | ❌ | **PDFium read-only'yi engellemiyor** |
| **Türkçe metin** | aynı | ✅ `Çağrı Şişli ığüö` | — | ⚠️ **`Çar Şişli üö`** | **`ğ` ve `ı` AP'den düşüyor** |

Diğer bulgular:

1. **Read-only uygulama sorumluluğu bizde.** `FORM_SetFocusedAnnot` ve
   `FORM_ReplaceSelection`, `Ff` bit 1 olan alana yazıyor. Backend doğrulaması (Faz 3
   adım 1) opsiyonel değil, güvenlik gereği.
2. **Türkçe glif kaybı (blocker).** PDFium, `/DR`'de bulamadığı karakterler için
   gömülmemiş `Arial` TrueType ekliyor (`/Arial_EE`, CP1250). Ş ve ş bulunuyor; CP1254'e
   özgü `ğ` ve `ı` hiçbir fonta eşlenmiyor ve AP'den **sessizce siliniyor** (`?` bile
   konmuyor). `/V` doğru kaldığı için veri kaybı yok, ancak AP kullanan görüntüleyiciler
   (Acrobat, Chrome, bizim render'ımız) eksik metin gösterir. Ek olarak eklenen Arial
   gömülü değil; Arial'i olmayan sistemlerde ikame fontla çizilir. → **Faz 0b**.
3. **MaxLen kırpması sessiz.** Faz 3'te `max_len` aşımı backend'de reddedilmeli ya da
   kullanıcıya bildirilmeli.
4. **Incremental save çalışıyor.** `FPDF_SaveAsCopy(FPDF_INCREMENTAL)` orijinal baytları
   önek olarak koruyor (`incremental_preserves_prefix=true`). İmzalı belgelerde imzaları
   korumanın teknik yolu mevcut; Faz 4'teki incremental save maddesi öne çekilebilir.
5. **pdfium-render `PdfDocument` form düzenleme için kullanılamaz.** Belge ve form
   handle'ları `pub(crate)`. `FORM_OnAfterLoadPage` hiç çağrılmıyor. `save_to_file` her
   zaman `flags=0` kullanıyor. Form yazma yolu ham `FPDF_LoadDocument`, kendi
   `FPDF_FORMFILLINFO`'muz ve ham `FPDF_SaveAsCopy` (`FPDF_FILEWRITE` callback'i, `unsafe`)
   ile kurulmalı.
6. **Render.** `render_form_data(false)` ile pdfium-render widget'ları hiç çizmiyor.
   Mevcut varsayılan (`true`, `FPDF_FFLDraw`) korunmalı.
7. **PDFKit** metin alanlarını `/AP` yerine `/V`'den yeniden çiziyor (Türkçe doğru,
   döndürme ve liste vurgusu yok). Değer birlikte çalışabilirliği doğrulandı; PDFKit
   bağımsız bir `/AP` kanıtı sayılmaz.
8. Checkbox ve radio için `FORM_OnChar(' ')` yeterli oldu; tıklama yedeğine gerek kalmadı.

Doğrulanmayanlar: format/calculate JavaScript (build'de V8 yok, script'ler çalışmaz),
XFA (build'de kapalı), şifreli belgeler, gerçek dünya formları, Windows ve Linux PDFium
build'leri (font eşleme platforma göre değişir; Türkçe davranışı orada farklı olabilir),
qpdf/poppler doğrulaması.

### Faz 0b — Türkçe ve Unicode görünüm akışı (Faz 3'ten önce zorunlu)

Amaç: `ğ ı İ` ve genel Unicode değerlerin AP'de doğru çizilmesi ya da kaybın kesin
biçimde tespit edilip kullanıcıya bildirilmesi.

Çıkış kriteri: Türkçe değer üç platformda da ya doğru çiziliyor ya da kayıp tespit
edilip bildiriliyor. Sessiz kayıp kabul edilmez.

#### Faz 0b sonuçları (2026-10-17, macOS arm64, aynı PDFium build'i)

Spike: `form_unicode_spike` (`tests/form_fill_spike.rs`, `#[ignore]`). Her değer
`FORM_ReplaceSelection` ile yazıldı ve tam kayıt yapıldı. Kaydedilen kopyada sayfanın
kendi içeriği silindi, `FPDFPage_Flatten` uygulandı ve her widget dikdörtgeninde
`FPDFText_GetBoundedText` ile çizilen metin çıkarıldı.

| Alan | `/DA` fontu | Değer | AP'de çizilen | Kayıp | PDFium'un eklediği font |
|---|---|---|---|---|---|
| Türkçe küçük | Helv (WinAnsi) | `ğışçöü` | `şçöü` | **ğ ı** | `Arial_EE` (CP1250) |
| Türkçe büyük | Helv | `ĞİŞÇÖÜ` | `ŞÇÖÜ` | **Ğ İ** | `Arial_EE` |
| Türkçe ad | Helv | `Çağrı Yılmaz İstanbul` | `Çar Ylmaz stanbul` | **ğ ı ı İ** | — |
| Lehçe | Helv | `ąęłńśźż` | aynı | yok | `Arial_EE` |
| Fransızca/€ | Helv | `œŒ€é` | aynı | yok | `ArialUnicodeMS` |
| Kiril | Helv | `Жжё` | aynı | yok | `Arial_CC` (CP1251) |
| Yunanca | Helv | `Ωωλ` | aynı | yok | `ArialUnicodeMS` (Greek) |
| CJK | Helv | `日本語` | aynı | yok | `_86` (GB) |
| Türkçe ×3 | **TrHv** (WinAnsi + CP1254 Differences) | aynı üç değer | aynı kayıplar | **aynı** | `Arial_EE` |

Yorum:

1. **Kayıp yalnız `ğ Ğ ı İ` ile sınırlı.** Bu dört karakter CP1254'te bulunuyor, CP1250'de
   bulunmuyor. Ş/ş ise her iki kod sayfasında var. PDFium, Latin Extended-A
   karakterleri için CP1250 (`_EE`) fontunu seçiyor ve Türkçe charset'e (162)
   düşmüyor. Gözlenen davranış bununla birebir uyumlu. PDFium kaynak kodu bu çalışmada
   okunmadı; neden bir **çıkarım**, doğrulanmış bir gerçek değil.
2. **Belgenin `/DR` fontu kurtarmıyor.** CP1254 Differences taşıyan `/DA` fontu ğ ve ı için
   seçilmedi. Seçenek 2 (belgenin kendi fontu) en azından basit fontlar için çalışmıyor.
   Gömülü Unicode CID fontlu fixture denenmedi.
3. **Font mapper değil, charset seçimi.** Kiril, Yunanca ve CJK için platform fontları
   bulundu. Sorun font bulunamaması değil, Türkçe için yanlış charset'in seçilmesi.
   Bu nedenle Windows ve Linux'ta da aynı davranış **beklenir**, ancak ölçülmedi.
4. **Tespit yolu çalışıyor ve yalnız PDFium kullanıyor.** Kaydedilen kopyayı
   aç → sayfa nesnelerini sil → `FPDFPage_Flatten` → widget dikdörtgeninde
   `FPDFText_GetBoundedText` → değerdeki karakterlerden çizilmeyenleri bul. Eksik küme
   tüm alanlarda birebir yakalandı. Sayfa içeriği silinmeden yapılan ilk denemede
   başlık metni widget alanına taşıp sonucu kirletti; içerik silme adımı zorunlu.
5. **`/NeedAppearances` PDFium public API'siyle yazılamıyor.** AcroForm sözlüğüne anahtar
   ekleyen bir FPDF fonksiyonu yok. Bu bayrak için `lopdf` yazımı gerekir ve AGENTS.md
   gereği ayrı mimari gerekçe ile onay ister.

#### Düzeltme seçenekleri (karar bekliyor)

| Seçenek | Ne yapar | Maliyet | Sonuç |
|---|---|---|---|
| **A. Tespit + uyarı + overlay** | Faz 3 yazımından sonra yukarıdaki tespit çalışır. Kayıp varsa kullanıcı uyarılır, edit sonucu `undrawn_chars` döner ve FormLayer o alanın değerini DOM'da çizer | Düşük, yeni bağımlılık yok | Veri doğru (`/V`). Uygulama içinde doğru görünür. Diğer AP kullanan görüntüleyicilerde eksik görünür; bu kullanıcıya açıkça söylenir |
| A + `/NeedAppearances` | A'ya ek olarak bayrak `lopdf` ile yazılır | `lopdf` onayı + test | Acrobat ve pdf.js görünümü kendi fontuyla yeniden üretir. Chrome (PDFium) aynı hatayı tekrarlar. PDF/A'da yasak |
| B. Yamalı PDFium build'i | Türkçe karakterler için CP1254 charset seçimi | Yüksek: altı hedef için kendi build'imiz, SHA-pinned provenance zinciri yeniden kurulur | Kökten çözüm. Upstream'e hata raporu ve yama gönderilmeli |
| C. Kendi AP üreticimiz | Etkilenen alanlar için görünüm akışını gömülü Unicode fontla biz yazarız | Yüksek: font gömme/subsetting, muhtemelen yeni crate; PDFium'un AP otoritesiyle çakışır | Önerilmez |

**Öneri:** Faz 3'ü **A** ile ilerlet (sessiz kayıp yok, yeni bağımlılık yok). PDFium'a
upstream hata raporu aç. B'yi rapora gelen yanıta göre ayrıca değerlendir.
`/NeedAppearances` ayrı bir `lopdf` kararı olarak ele alınır.

#### Seçenek B — PDFium yaması: kök neden ve maliyet (2026-10-17)

**Kök neden kaynakta doğrulandı** (`chromium/7961` dalından indirilip okundu; `main`
dalında da değişmemiş):

- `core/fxge/cfx_font.cpp` `CFX_Font::GetCharSetFromUnicode` U+0100–U+024F
  aralığının tamamını `FX_Charset::kMSWin_EasternEuropean` olarak döndürüyor. Türkçe dalı yok.
- `core/fpdfdoc/cpdf_bafontmap.cpp` `CPDF_BAFontMap::GetWordFontIndex` sırasıyla şunları
  dener: (1) `/DA` fontu, yalnız charset'i eşleşirse (TrHv denemesinin neden başarısız
  olduğunu açıklar); (2) o charset'in yerel fontu (`Arial_EE`, CP1250); (3) `"Arial Unicode MS"`.
  Üçüncü font sistemin ANSI kod sayfasıyla, yani WinAnsi ile ekleniyor. Üçü de `ğ ı`'yı
  kodlayamıyor, fonksiyon `-1` döndürüyor ve karakter düşüyor.
- `kDefaultTTFMap` içinde `kMSWin_Turkish` girdisi yok. PDFium'da CP1254 tablosu
  (`kFX_MSWinTurkishUnicodes`, `core/fxcrt/fx_codepage.cpp`) zaten var.
- `CPWL_EditImpl` metni `FX_Charset::kDefault` ile ekliyor. Bu yüzden "önceki charset'i
  takip et" kuralı devreye girmiyor ve her karakter bağımsız eşleniyor.

**Yama taslağı** (22 satırlık diff; aşağıdaki yerel build ile doğrulandı):

```diff
--- a/core/fxge/cfx_font.cpp
+++ b/core/fxge/cfx_font.cpp
@@ -44,6 +44,7 @@
     {static_cast<int>(FX_Charset::kMSWin_Arabic), "Arial"},
+    {static_cast<int>(FX_Charset::kMSWin_Turkish), "Arial"},
@@ -127,6 +128,11 @@
+  // Turkish letters that Windows-1250 lacks but Windows-1254 has.
+  if (word == 0x011E || word == 0x011F || word == 0x0130 || word == 0x0131) {
+    return FX_Charset::kMSWin_Turkish;
+  }
+
   if (word >= 0x0100 && word <= 0x024F) {
     return FX_Charset::kMSWin_EasternEuropean;
```

Yamanın açık soruları:
- ~~`AddNativeTrueTypeFontToPDF`'in `kMSWin_Turkish` için doğru CP1254 kodlaması
  ürettiği varsayılıyor.~~ macOS'ta doğrulandı: `Arial_A2` ekleniyor ve `ğ Ğ ı İ` çiziliyor.
- `kDefaultTTFMap`, kullanımdan kaldırılmış public `FPDF_GetDefaultTTFMap*` API'siyle
  dışarı açık. Upstream kuralına göre public API'ye dokunan değişiklik için bug ve mailing
  list tartışması gerekir. Upstream'de bu satır olmadan, yalnız charset dalıyla
  başlanabilir.
- Linux'ta Arial genelde kurulu değil. `FindNativeTrueTypeFont("Arial")` başarısız
  olunca ne olduğu (bugün Lehçe için de geçerli) ölçülmedi. Bu risk Seçenek A için de geçerli.
- Romence `ș ț` (U+0219/U+021B; CP1250'de yalnız cedilla biçimleri var) ve Azerice `ə`
  muhtemelen aynı şekilde düşüyor; ölçülmedi. Genel bir çözüm, yerel font kelimeyi
  bilmediğinde karakteri içeren diğer kod sayfalarını denemek olurdu. Bu daha büyük bir
  yamadır ve daha uzun review gerektirir.

**Yol 1 — Upstream CL** (önerilen)

| Kalem | Tahmin |
|---|---|
| CLA (bireysel, online), depot_tools, Gerrit kurulumu | ~0,5 gün |
| Yama + `cfx_font_unittest` + embedder testi (form doldur, düzleştir, metni karşılaştır; spike'ın C++ karşılığı) | ~1 gün |
| Review | **Bilinmiyor.** İlk CL'lerde try-bot'ları bir committer tetikler. Yalnız charset dalı küçük ve izole bir bug fix'tir |
| Yayına ulaşma | `main`'e girdikten sonra bir sonraki chromium dalı, ardından bblanchon sürümü, ardından bizim runtime pin'imizin 7961'den yükseltilmesi. Haftalar sürer; tarih verilemez |
| Bizim sürekli maliyetimiz | Sıfır. Pin yükseltmesi zaten yapılacak ayrı bir iş |

**Yol 2 — pdfium-binaries fork'u + yama** (kendi build'imiz)

bblanchon düzeni buna uygun: `patches/*.patch` dosyaları `steps/03-patch.sh` içinde
`patch -p1` ile uygulanıyor. `build-one.yml` fork'ta Actions sekmesinden `branch`,
`target_os` ve `target_cpu` girdileriyle elle tetiklenebiliyor.

| Kalem | Ölçüm / tahmin |
|---|---|
| Fork + `patches/turkish_charset.patch` + `03-patch.sh`'ye tek satır | ~0,5 gün |
| Build süresi (bblanchon `Build all`, 2026-10-12, ölçüldü) | macOS arm64 8,1 dk; macOS x64 6,7 dk; Windows x64 21,7 dk; Linux x64 8,0 dk. Hedeflerimiz için **~45 runner-dakika/sürüm** |
| CI ücreti | Public fork'ta ücretsiz. Private repoda GitHub çarpanları (macOS ×10, Windows ×2) ile ~200 faturalanan dakika, yani sürüm başına birkaç dolar |
| Tedarik zinciri | `platform-support.json` provenance sınıfı `upstream_prebuilt_verified`'dan yeni bir self-built sınıfına geçer. `scripts/provision-pdfium-runtime.mjs` `ALLOWED_UPSTREAM_PREFIX` bizim fork'umuzu kabul etmeli ve testleri güncellenmeli. Artifact attestation eklenmeli. ~1–2 gün |
| Yan kazanç | macOS runtime'ın provenance'ı bugün `class: unknown`, `sourceProvenanceComplete: false`. Kendi pipeline'ımız macOS'u da tam provenance'a taşır |
| Sürekli maliyet | Her PDFium yükseltmesinde yamayı rebase etmek (dosya küçük ve az değişiyor), upstream fork'u merge etmek, dört hedefi yeniden build etmek, hash'leri yeniden pinlemek ve PDFium'a bağlı acceptance testlerini koşmak: ~0,5–1 gün/yükseltme |
| Risk | Chrome ve diğer PDFium kullanıcılarından ayrışan davranış (küçük). Fork CI'ı bizim sorumluluğumuza geçer |

#### Yama doğrulaması — yerel macOS arm64 build'i (2026-10-17)

**Sonuç: yama çalışıyor.** `ğ Ğ ı İ` artık AP'de çiziliyor. Diğer diller ve diğer alan
türleri değişmedi.

Build ortamı:
- Kaynak: PDFium `chromium/7961`, commit `dd3583292697`. bblanchon/pdfium-binaries
  `chromium/7961` etiketi (`c6529b58791d`), yamaları `steps/03-patch.sh` ile uygulandı.
- `args.gn`, gönderilen runtime'ın `src-tauri/args.gn` dosyasıyla birebir aynı.
- **Sapma:** Makinede tam Xcode yok, yalnız Command Line Tools var (SDK 26.1). Chromium'un
  `build/config/apple/sdk_info.py` ve `build/mac/find_sdk.py` betikleri, bu
  throwaway ağaçta CLT'ye düşen yedek dallarla yerel olarak yamalandı. bblanchon CI'ı
  Xcode 26.0 kullanıyor. Bu nedenle yerel dylib gönderilen dylib ile bayt düzeyinde aynı değil
  (7 199 216 / 7 151 568 bayt).
- Süre (8 çekirdek, 8 GB RAM): `gclient sync` ~3,3 GB indirme. Tam build 272 sn
  (1093 adım). Yama sonrası artımlı build 2,3 sn. Disk kullanımı ~3,5 GB.

Karşılaştırma (`form_unicode_spike`, `FORM_SPIKE_PDFIUM` ile; bu değişken yalnız
`#[ignore]` spike'ta runtime SHA-256 pin'ini atlar, production yolu değişmedi):

| Runtime | Türkçe 6 alan | Lehçe, Fransızca/€, Kiril, Yunanca, CJK |
|---|---|---|
| Gönderilen (`src-tauri/lib`, bblanchon) | `ğ Ğ ı İ` kayıp | doğru |
| Yerel, yamasız (kontrol) | `ğ Ğ ı İ` kayıp (gönderilenle **aynı**) | doğru |
| Yerel, yamalı | **tümü doğru** (`Arial_A2` = CP1254 fontu eklendi) | doğru, AP fontları değişmedi |

`form_fill_spike` (metin, multiline, comb, read-only, checkbox, radio, combo, list,
incremental save) gönderilen runtime ile yamalı runtime arasında karşılaştırıldı. Tek fark
Türkçe alanın AP'si ve eklenen font nedeniyle dosya boyutu (+2,3 KB). Read-only atlatma,
MaxLen kırpma ve incremental save davranışı aynı kaldı.

Yeni bulgular:
1. **Eklenen `Arial_A2` gömülü değil**, `Arial_EE` gibi. Arial'i olmayan sistemlerde
   (çoğu Linux) diğer görüntüleyiciler ikame font kullanır. Yamanın Linux ve Windows'taki
   davranışı ölçülmedi.
2. **Seçenek A'nın tespit yolunun bir sınırı var.** CJK satırında `語` (U+8A9E) her üç
   runtime'da da render'da görünmüyor, oysa flatten + metin çıkarma `日本語` döndürüyor.
   Karakter kodu AP'de bulunuyor, ama ikame fontta glif yok. Metin çıkarma yalnız
   *kodlanamayan* karakterleri yakalar; *kodlanmış ama glifi olmayan* karakterleri
   yakalamaz. Türkçe kaybı birinci türden olduğu için tespit edilir. Bu sınır Faz 3'te
   belgelenmeli. Glif düzeyinde kontrol (ör. `FPDFText_GetCharBox` ile sıfır alan
   arama ya da render tabanlı kontrol) ayrıca değerlendirilebilir.

Yama dosyası: `docs/engineering/pdfium-patches/turkish-charset.patch`.

**Öneri (güncel):** Yama doğrulandığı için **Yol 1 (upstream CL)** artık güçlü bir aday:
küçük, izole ve ölçülmüş bir düzeltme. Upstream'e ilk etapta yalnız charset dalı
(`GetCharSetFromUnicode`) ve bir birim testi gönderilmeli; `kDefaultTTFMap` satırı public
API'ye dokunduğu için ayrı tartışılmalı. Ürün tarafında Faz 3 **Seçenek A** ile ilerlemeli;
yama upstream'e girip bblanchon sürümüne ulaştığında tespit uyarısı Türkçe için kendiliğinden
susar. Yol 2 (fork) hâlâ şu iki koşula bağlı: upstream review takılıyor **ve** diğer
görüntüleyicilerde Türkçe görünüm ürün gereksinimi.

#### Karar: yamalı runtime uygulamada (2026-10-17)

Uygulama yalnız kişisel kullanım için olduğundan upstream gönderimi yapılmadı.
`src-tauri/lib/libpdfium.dylib` yamalı yerel build ile değiştirildi (SHA-256 `cb167bb4…57ab`,
provenance `locally_built_verified`; ayrıntı `pdfium-patches/README.md`). Uygulamanın
normal runtime çözümlemesiyle `form_unicode_spike` Türkçe alanların tamamında eşleşiyor.
`cargo test --locked`: 1084 geçti, 0 başarısız, 10 ignored. Runtime manifest
doğrulayıcısı, macOS paketleme sözleşmesi ve macOS smoke harness (34/34) geçti.

Bu değişiklik yalnız macOS arm64 runtime'ını kapsar. Linux ve Windows runtime'ları
yamasız bblanchon build'leridir ve Türkçe kaybı orada sürer. Seçenek A'nın tespit +
uyarı katmanı bu nedenle ve glif düzeyindeki sınırlar için Faz 3'te güvenlik ağı olarak
kalır.

### Faz 1 — Okuma: alan modeli (salt-okunur)

`forms.rs` DTO'su (PdfCraft `Field`/`Widget` modelinden sadeleştirilmiş):

```rust
pub struct FormInfo {
    pub form_type: FormType,          // None | AcroForm | XfaFull | XfaForeground (FPDF_GetFormType)
    pub field_count: u32,
    pub has_signature_fields: bool,   // Faz 3 kayıt uyarısı için
    pub need_appearances: bool,
}
pub struct FormWidget {
    pub field_name: String,           // tam nitelikli (FPDFAnnot_GetFormFieldName)
    pub annot_index: u32,             // sayfa içi; yazmada kimlik doğrulaması için isimle birlikte
    pub control_index: i32,           // FPDFAnnot_GetFormControlIndex
    pub kind: FieldKind,              // Text | CheckBox | Radio | PushButton | Combo | List | Signature
    pub flags: u32,                   // ReadOnly, Required, Multiline, Password, Comb, MultiSelect, Edit...
    pub value: Option<String>,
    pub export_value: Option<String>, // checkbox/radio (FPDFAnnot_GetFormFieldExportValue)
    pub checked: Option<bool>,
    pub options: Vec<FormOption>,     // label + selected (FPDFAnnot_GetOptionLabel / IsOptionSelected)
    pub max_len: Option<u32>,
    pub rect: NormalizedPageRect,     // annotations.rs ile aynı görüntü uzayı
    pub hidden: bool,
    pub tab_order: u32,
}
```

Adımlar:
1. `WorkerCommand::GetFormInfo { document_id }` ve
   `GetPageFormFields { document_id, page_index }` için `WorkerCapability` ekle, handler'ı `worker/pdfium.rs` içine yaz.
2. Limitler: sayfa başı en fazla N widget, değer/etiket için uzunluk sınırı, seçenek sayısı
   sınırı. `MessageTooLarge` ve `InvalidInput` hatalarına eşlenir. Okuma yolunda
   `FPDFAnnot_*` getter'ları dışında hiçbir form-fill mutasyonu yapılmaz.
3. `document_session.rs` içinde `get_session_form_info` ve `get_session_page_form_fields`
   komutlarını `acquire_worker_read(..., expected_content_generation)` ile yaz (mevcut
   `get_session_page_annotations` deseni).
4. XFA belgelerinde (`XfaFull`) alanlar dönülmez; `FormInfo` frontend'e "desteklenmiyor" bildirimi göstermesini söyler.

#### Faz 1 sonuçları (2026-10-17)

Uygulandı: `src-tauri/src/forms.rs`; worker `GetFormInfo` / `GetPageFormFields`
(protokol 6); `get_session_form_info` / `get_session_page_form_fields`; frontend sözleşmesi
`getSessionFormInfo` / `getSessionPageFormFields`. Frontend bağlantısı yok (Faz 2).

Plandan sapmalar:
- **Okuma ham `FPDFAnnot_*` yerine pdfium-render getter'larıyla yapılıyor.** Worker
  belgeleri `PdfDocument` olarak tutuyor ve form handle'ı `pub(crate)`. Getter'lar
  `FPDFAnnot_GetFormField*` çağırdığı için güvenli; değer yazan setter'lar kullanılmıyor.
  Worker'a yeni `unsafe` kod eklenmedi.
- **Checkbox durumu `/AS`'den türetiliyor.** pdfium-render `is_checked()` yalnız `/Yes`
  on-state'ini tanıyor (on-state'i `/On` olan checkbox için yanlış sonuç). Radio için
  `FPDFAnnot_IsChecked` kullanan `is_checked()` doğru çalışıyor.
- **Comb, DoNotScroll, RichText, FileSelect ve Sort bayrakları modelde yok.** pdfium-render
  0.8.37'nin kendi tanımladığı bu bayraklar ISO 32000 bitlerinin bir üstünü okuyor (ör.
  Comb `1 << 25`, doğrusu `1 << 24`). Ham `/Ff` public değil. PDFium sabitlerinden gelen
  Multiline, Password, Combo, Edit ve MultiSelect doğru.
- **MaxLen, export value ve control index okunmuyor** (pdfium-render'da public getter yok).
  Faz 3'teki ham yazma yolu bunları doğrulama için job worker'da okuyacak.
- `FormInfo`'da alan sayısı yok; belgedeki tüm sayfaları dolaşmak gerekeceği için
  çıkarıldı. `signature_count` (`FPDF_GetSignatureCount`) eklendi.
- Tab sırası `/Annots` sırası (`annot_index`); `/Tabs` henüz uygulanmıyor.

Doğrulama: `forms::tests` (5 birim testi: türler/değerler/bayraklar/seçenekler, `/On`
checkbox, normalize rect, AcroForm'suz belge, limit hatası),
`worker_pdfium_integration::worker_reads_form_info_and_page_form_fields` (gerçek worker
süreci), `architecture.test.js` (komutlar worker broker sınırında).

### Faz 2 — Frontend overlay (taslak, henüz yazma yok)

1. `formState.svelte.js`: `$state` ile anahtarı `(sessionId, contentGeneration, page)` olan
   alan listesi ve `fieldName → taslak değer` haritası. Generation değişince alan listesi
   geçersiz sayılır ve yeniden istenir (stale-result koruması). Taslaklar isim üzerinden
   taşınır.
2. `FormLayer.svelte`: her görünür widget için mutlak konumlu DOM kontrolü (`input`,
   `textarea`, `select`, checkbox, radio). Read-only alanlar `disabled`, required alanlar
   işaretli olur. `maxlength` uygulanır. Tab sırası `tab_order` değerinden gelir.
   Kontrollerin bir `aria-label`'ı olur (alan adı veya tooltip).
3. Çift çizim sorunu: bitmap alanın eski değerini zaten çizer. Kirli (dirty) veya odaktaki
   kontrol opak arka planla çizilir. Commit sonrası yeni generation render'ı yeni `/AP`'yi
   gösterir ve kontrol yeniden şeffaf olur.
4. Lazy davranış: alanlar yalnız sayfa render edildiğinde istenir.

#### Faz 2 sonuçları (2026-10-17)

Uygulandı:
- `src/stores/formDraftState.svelte.js`: `SvelteMap` tabanlı taslak store'u. Anahtar alan
  adı (belgeden gelen adlar prototype ile çakışmasın diye düz nesne değil). Oturum
  değişince taslaklar silinir, generation değişince ada göre korunur.
  `App.svelte` `FORM_DRAFTS_CONTEXT` ile sağlar, böylece sayfalar arası paylaşılır.
- `src/components/FormLayer.svelte` (Svelte 5 runes): text/textarea/password, checkbox,
  radio, combo ve list kontrolleri; read-only `disabled`, `aria-label`/`aria-required`;
  yanıt isteğin `(sessionId, generation, page)` üçlüsüyle eşleşmezse yok sayılır.
  Değişmemiş kontrol şeffaftır (görünümü PDFium bitmap'i çizer), taslaklı alan opak beyaz
  ve turuncu çerçevelidir. Font boyutu `cqh` ile alan yüksekliğinden türetilir.
- `pdfState.getPageFormFields` `form_editing` kapısıyla korunuyor. Kapı `experimental`
  ve açık; uyarı metni değişikliklerin kaydedilmediğini söylüyor.

Doğrulama:
- Vitest: `FormLayer.test.js` (6), `formDraftState.test.js` (3); tüm paket 1369/1369.
- Gerçek tarayıcı: `tmp/form-harness` (gitignore'da) gerçek `FormLayer`, `pdfState`,
  feature guard ve sözleşme doğrulamasını Vite'ta çalıştırdı. Yalnız Tauri `invoke` köprüsü
  taklit edildi ve **gerçek worker'dan** alınmış `FormWidgetData` ile PDFium render'ı
  kullanıldı. Alan hizası, şeffaflık, Türkçe metin, checkbox, radio, combo ve multiline
  taslakları görsel olarak doğrulandı; konsolda hata yok.
- **Yerel Tauri penceresinde doğrulanmadı.** Bu oturumda masaüstü penceresini görüntüleyip
  sürebilecek bir araç yoktu.

Tarayıcı doğrulamasında bulunup düzeltilenler:
- Değişmemiş liste kutusu kendi seçeneklerini ve gri seçim vurgusunu bitmap'in üstüne
  çiziyordu. WebKit'te `color` ile gizlenemiyor; değişmemiş liste kutusu artık
  `opacity: 0` (etkileşim sürüyor).
- Yuvarlak radio taslağında bitmap'teki eski işaretin köşeleri sızıyordu. Taslaklı alanın
  kapsayıcısı artık tüm dikdörtgeni opak örtüyor.

Bilinen eksikler:
- ~~**`/Fields` dışında kalan widget'lar görünmüyor.**~~ Çözüldü, aşağıdaki bölüme bakın.
- Editable combo'da serbest metin girişi, `/Tabs` sırası, imza uyarısı ve XFA bildirimi
  yok. `FormInfo` henüz frontend'de kullanılmıyor.
- Aynı adlı birden çok checkbox widget'ı tek taslağı paylaşıyor; export value okunmadığı
  için ayırt edilemiyorlar.

#### `/Fields` dışı widget'lar (2026-10-17)

**Kök neden (PDFium 7961 kaynağında doğrulandı):** PDFium, yalnız sayfanın `/Annots`
dizisinden erişilen widget'ları `CPDF_InteractiveForm::FixPageFields` ile alana çeviriyor.
Bu fonksiyon yalnızca `CPDFSDK_PageView` kurucusunda çağrılıyor. Sayfa görünümünü
`FormHandleToPageView` → `GetOrCreatePageView` oluşturuyor; bunu tetikleyen public
fonksiyonlar `FORM_*` ailesi ve `FPDF_FFLDraw`. pdfium-render bunlardan yalnız
`FPDF_FFLDraw`'ı, `render_form_data(true)` ile render sırasında çağırıyor ve
`FORM_OnAfterLoadPage`'i hiç çağırmıyor.

Değerlendirilen seçenekler:

| Seçenek | Artı | Eksi |
|---|---|---|
| **A. Okumadan önce 1×1 bitmap'e form verisiyle render** (seçildi) | Yeni `unsafe` yok, değişiklik `forms.rs` içinde birkaç satır, deterministik | Bir yan etki üzerinden çalışıyor (yorumla belgelendi); MaxLen/export value/Comb sorununu çözmüyor |
| B. Worker'da ham ikinci `FPDF_DOCUMENT` + `FPDF_FORMFILLINFO` + `FORM_OnAfterLoadPage` | Ham `FPDFAnnot_*` ile MaxLen, export value, doğru `/Ff` bayrakları ve control index de okunur | FFI `unsafe`, form içeren her belge için ikinci belge örneği; "açıldıktan sonra kaynak yolu yeniden açma" kuralı nedeniyle `OpenDocument` anında açılmalı; şifreli belgede bayt kopyası tutulmalı |

Ölçüm (debug profil, macOS arm64): süreçteki ilk okuma **1781 ms**, aynı belgede sonraki
okumalar 0,3 ms, yeni belgede ilk okuma 1,4 ms. Ağır sayfada 1×1 render ~0,1 ms.
İlk okumadaki maliyet süreç başına bir kezdir: PDFium görünüm akışı olmayan alanlar için
ilk görünümü üretirken sistem fontlarını tarıyor. Worker'ın normal sayfa render'ı da
`FFLDraw` ile çalıştığından uygulamada bu maliyet ilk render'da zaten ödeniyor.

Doğrulama: `forms::tests::widgets_outside_acroform_fields_are_read` düzeltmeden önce
başarısız oldu (yalnız `listed` döndü), sonra geçti.
`worker_reads_form_info_and_page_form_fields` gerçek worker'da orphan widget'ı da
doğruluyor.

B, Faz 3'ün UI'da MaxLen veya comb göstermesi gerekirse yeniden değerlendirilir.

### Faz 3 — Yazma: `FillFormFields` mutasyonu

```rust
MutationRecipe::FillFormFields { edits: Vec<FormFieldEdit> }
pub struct FormFieldEdit {
    pub page_index: u32,
    pub field_name: String,
    pub annot_index: u32,            // isimle eşleşmezse reddet (stale koruması)
    pub value: FormFieldValue,       // Text(String) | Checked(bool) | RadioSelect | ChoiceIndices(Vec<u32>)
}
```

1. job_worker'da Faz 0'da doğrulanan yol kullanılır. Her edit için önce doğrulama yapılır:
   alan var mı, türü uyuyor mu, read-only mi, MaxLen aşılıyor mu, seçenek indeksi sınır
   içinde mi, multi-select değilse tek seçim mi. Tek bir geçersiz edit tüm batch'i reddeder
   (atomik).
   Yazma sonrası aynı job içinde görünüm doğrulaması yapılır (Faz 0b tespit yolu,
   seçenek A). Kayıp karakterler edit sonucuna `undrawn_chars` olarak eklenir.
2. `fill_session_form_fields(session_id, expected_content_generation, edits)` komutu
   `mutate_isolated` üzerinden çalışır. Generation, digest, recovery ve atomic I/O mevcut
   yoldan gelir. Ana process kullanıcı PDF'ini açmaz.
3. Commit sıklığı: blur veya Enter olduğunda kuyruğa ekle, kısa debounce ile tek batch
   halinde gönder. Uçuştaki commit sırasında yeni taslaklar sonraki batch'e kalır. Generation
   uyuşmazlığında taslaklar korunur, alanlar yeniden okunur ve kullanıcıya bildirilir.
4. İmzalı belgeler: `has_signature_fields` ise ilk düzenlemede uyarı göster. Kayıt tam
   yeniden yazım olduğu için mevcut imzalar geçersiz olur (sanitization'daki "explicit
   invalidation acknowledgement" deseni).
5. Feature gate `form_editing` → `experimental`. Feature matrix ve IPC envanteri güncellenir.

#### Faz 3 sonuçları (2026-10-17)

Uygulandı:
- `src-tauri/src/forms/fill.rs`: `FormFieldEdit` (`{ pageIndex, annotIndex, fieldName, kind,
  text? | checked? | selected? }`), `validate_request` ve `fill_form_fields_report`. Yazma
  `atomic_write_pdf` içinde ham PDFium binding'leriyle yapılıyor: kendi
  `FPDF_FORMFILLINFO`'muz, `FORM_OnAfterLoadPage`, metin için
  `FORM_SelectAllText` + `FORM_ReplaceSelection`, checkbox/radio için `FORM_OnChar(' ')`,
  seçim için `FORM_SetIndexSelected`, ardından `FORM_ForceToKillFocus` ve
  `FPDF_SaveAsCopy` (flags 0). Belge, form ortamı, sayfa ve annotation RAII ile
  kapatılıyor; `unsafe` yalnız `FPDF_FILEWRITE` callback'inde.
- Doğrulama (belgeye karşı, yazmadan önce): sayfa ve annotation var mı, widget mı,
  `FPDFAnnot_GetFormFieldName` istekteki adla aynı mı (**isim-indeks**), alan türü değer
  türüne uyuyor mu, `/Ff` bit 1 (**read-only**) ve combo için tek seçim, liste için
  multi-select. Yazdıktan sonra değer PDFium'dan geri okunuyor; kırpılmış metin
  **MaxLen** hatası olarak reddediliyor ("en fazla N karakter"). Tek bir hata tüm batch'i
  reddediyor ve dosya değişmiyor. Kayıttan sonra `atomic_write_pdf`'in doğrulama adımı
  geçici dosyayı Faz 1 okuyucusuyla yeniden okuyup aynı değerleri bekliyor.
- `MutationRecipe::FillFormFields` → `fill_session_form_fields` (`mutate_isolated`;
  istek önce `validate_request` ile, belgeden bağımsız denetleniyor) → frontend
  `fillSessionFormFields`. Sözleşme `kind`'a göre tam olarak doğru alanı istiyor.
- Frontend: `formCommitController.svelte.js` taslak store'u başına tek denetleyici.
  Metin alanı odaktan çıkınca ya da Enter'da, düğme/seçim değişince 350 ms gecikmeyle tüm
  taslakları tek mutasyonla yazıyor. Uçuştaki yazma sırasında gelen istek, yazma bitince
  yanıttaki generation ile yeniden çalışıyor. Yalnız gönderildiğinden beri değişmemiş
  taslaklar siliniyor. Belgede imza varsa (`FormInfo.signature_count`) ilk yazmadan önce
  onay isteniyor ve oturum boyunca hatırlanıyor. Hata durumunda taslaklar korunuyor;
  alanlar kırmızı çerçeveyle işaretleniyor ve hata `title` olarak gösteriliyor.
  Yazma sırasında çerçeve kesikli.

Doğrulama:
- Rust: `forms::fill::tests` (10): tüm türlerin yazılıp geri okunması, regenere `/AP /N`,
  read-only, isim-indeks, MaxLen, tür uyuşmazlığı, batch atomikliği, `/Fields` dışı widget,
  istek doğrulaması, JSON biçimi. `job_worker::tests::fill_form_fields_mutation_round_trips_through_job_worker_dispatch`
  (JSON üzerinden job komutu, gerçek dağıtım, read-only reddi).
- Vitest: `formCommitController.test.js` (5), `FormLayer.test.js` (8), `client.test.js`
  sözleşme testi, `architecture.test.js` (komut `mutate_isolated` kullanıyor, dosya yolu
  almıyor).
- Gerçek tarayıcı: gerçek `FormLayer`, `pdfState`, denetleyici ve sözleşme doğrulaması
  yalnız Tauri `invoke` taklit edilerek çalıştırıldı. Başarı yolu (Enter → yazma → yeni
  generation → yeniden okuma, taslak silinir) ve hata yolu (MaxLen → kırmızı çerçeve,
  taslak korunur, mesaj `title`'da) doğrulandı. Bu sırada imzasız belgede her commit'te
  `FormInfo`'nun yeniden istendiği görüldü; artık oturum başına bir kez isteniyor.
- Uygulama içinde uçtan uca (yerel Tauri penceresi) **doğrulanmadı**.

Testlerin yakaladığı hata: uçuştaki yazma sırasında kuyruğa giren istek, yazmanın döndürdüğü
yeni generation'ı öğrenmiyor ve sonraki yazmayı eski generation ile gönderiyordu.
Düzeltildi.

Kapsam dışı kalanlar:
- Seçenek A'nın görünüm tespiti (yazma sonrası flatten + metin çıkarma ile
  `undrawn_chars`): macOS runtime yamalı olduğu için Türkçe için gerekli değil; Linux ve
  Windows runtime'ları için ayrıca eklenmeli.
- Incremental save (imzaları korumak), editable combo'ya serbest metin, formu temizleme,
  XFA bildirimi.

### Faz 4 — Sonraki adımlar (bu planın kapsamı dışında, ayrı karar gerektirir)

- **Formu temizle (Reset)**: `/DV`'ye dönüş. PDFium'da doğrudan API yok, `/DV` okunup Faz 3 yoluyla yazılır.
- **Düzleştirme (Flatten)**: `FPDFPage_Flatten(FLAT_NORMALDISPLAY)`, ucuz bir kazanım.
- **Format/Calculate script'leri**: PDFium build'imizde JS yok. PdfCraft `af.rs` (AFNumber_Format, AFSimple_Calculate...) Rust'ta yeniden yazılabilir. Sandbox ve güvenlik değerlendirmesi gerektirir.
- **FDF/XFDF içe/dışa aktarma**: PdfCraft `crates/xfdf` referans.
- **Incremental save**: imzaları korumak için `FPDF_INCREMENTAL`. Atomic I/O sözleşmesi ve `pades-architecture-design.md` ile birlikte ele alınmalı.
- **Alan oluşturma (authoring)**: PdfCraft `author.rs`.

---

## 4. Mimari riskler

| Risk | Etki | Önlem |
|---|---|---|
| PDFium form-fill yolu bazı alan türlerinde `/AP` üretmiyor | Kaydedilen dosya başka görüntüleyicilerde yanlış görünür | Faz 0 çıkış kriteri. Gerekirse o tür için `NeedAppearances` + belgeli sınırlama |
| `pdfium-render` yüksek seviye setter'larının (println!, sabit `/Yes`) yanlışlıkla kullanılması | Worker IPC bozulur, checkbox state'i bozulur | `forms.rs` yalnız ham binding kullanır. Bunu bir mimari test ya da grep testiyle zorla |
| Render worker'ında form-fill ortamının mutasyon yapması | Salt-okunur worker durum biriktirir | Okuma handler'ları yalnız getter çağırır. Yazma sadece job_worker'da |
| Her commit = alt süreç + yeni generation | Gecikme, render yeniden tetiklenmesi | Batch commit + debounce. Faz 0'da gecikme ölçülür |
| Annot indeksinin generation'lar arasında kayması | Yanlış alana yazma | İsim + indeks çift doğrulaması, `expected_content_generation` |
| Tam yeniden yazım kaydı | Dijital imzalar geçersiz olur | Faz 3 uyarısı. Incremental save Faz 4'te |
| Türkçe `ğ Ğ ı İ` glif kaybı | **Doğrulandı (Faz 0b):** PDFium bu dört karakteri AP'den sessizce siliyor. `/DR` fontu kurtarmıyor. Kiril, Yunanca ve CJK etkilenmiyor | Seçenek A: flatten + metin çıkarma ile tespit, uyarı ve DOM overlay. Upstream rapor. B (yamalı PDFium) ayrı karar |
| PDFium read-only'yi uygulamıyor | Read-only alan değiştirilebilir | **Doğrulandı.** Backend her edit'te `Ff` bit 1'i kontrol eder ve reddeder. Bunun için test zorunlu |
| MaxLen sessiz kırpma | Kullanıcının yazdığı kaybolur | Backend aşımı reddeder ya da bildirir |

## 5. Regresyon riskleri

- `PdfPage.svelte` overlay sırası: FormLayer, AnnotationLayer, metin seçimi ve OCR katmanıyla pointer-event çakışması yaratabilir. Mevcut `PdfPage.test.js` ve `AnnotationLayer.test.js` korunmalı.
- `WorkerCapability` ve `generate_handler!` değişikliği: `ipc-inventory`, `architecture.test.js` ve `contracts.js` sözleşme testleri güncellenmeli.
- `MutationRecipe` eşleşmeleri: exhaustive `match`'ler (`operation_name`, recovery yolları) derleme zamanında yakalanır.
- `render_form_data` varsayılanı değiştirilmemeli. Mevcut thumbnail ve render çıktıları aynı kalır.

## 6. Gerekli testler

**Rust**
- `forms.rs` birim testleri: Faz 0 fixture'larının hepsi için okuma DTO'su (tür, bayrak, seçenek, export value, rect, tab sırası).
- job_worker: her alan türü için yazma, ardından yeniden açma ile `/V` + `/AS` + `/AP` doğrulaması. Read-only, MaxLen, geçersiz indeks, isim-indeks uyuşmazlığı reddi. Batch atomikliği (bir hata varsa hiçbir değişiklik yazılmaz).
- Worker limitleri: büyük alan sayısı, uzun değer.
- XFA belgesi: `FormInfo` doğru, alan dönmüyor.
- `forms.rs` içinde `PdfForm*Field::set_*` kullanılmadığını doğrulayan mimari test.

**Frontend (Vitest)**
- `FormLayer.test.js`: render, read-only, maxlength, tab sırası, aria-label.
- `formState` testleri: stale generation sonucunu yok sayma, taslak koruma, batch/debounce, uçuştaki commit.
- `contracts.js` sözleşme testi.

**Uçtan uca (manuel, kanıtlı)**
- Gerçek bir resmi form (ör. vergi formu) doldurulur, kaydedilir, Preview.app ve qpdf/poppler ile açılır, ekran görüntüsü alınır.

## 7. Kabul kriterleri

1. AcroForm içeren bir belgede text, checkbox, radio, combo ve list alanları overlay olarak görünür ve düzenlenebilir.
2. Kaydedilen dosya, uygulamamız dışında (en az bir bağımsız araçla) aynı değerleri gösterir.
3. Read-only alan düzenlenemez. Backend de reddeder.
4. Stale generation ile gelen yazma isteği reddedilir ve kullanıcının taslakları kaybolmaz.
5. XFA belgelerinde açık bir "desteklenmiyor" bildirimi gösterilir.
6. İmza alanı içeren belgede ilk düzenlemeden önce uyarı gösterilir.
7. Ana Tauri process'i kullanıcı PDF'ini açmaz. Tüm PDFium işleri worker'larda yapılır.
8. Yeni bağımlılık eklenmez. `cargo test`, `cargo clippy` ve `npm test` geçer. Çalıştırılamayan doğrulama "çalıştırılmadı" olarak raporlanır.

## 8. Kapsam dışı

- JavaScript (format, keystroke, validate, calculate) ve push button eylemleri
- XFA formları
- Alan oluşturma ve düzenleme (authoring), alan özellikleri
- Formu temizle, düzleştirme, FDF/XFDF
- Incremental save ve imzaların korunması, dijital imza atma
- Rich text (`/RV`) değerleri
- Form gönderme (submit)
