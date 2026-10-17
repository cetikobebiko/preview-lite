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

Denenecek seçenekler (sırasıyla):
1. `FPDF_SetSystemFontInfo` veya varsayılan font mapper'ın Türkçe charset'i (162)
   raporlayıp raporlamadığını incele. Platform bazında (macOS, Windows, Linux) ölç.
2. Belgenin `/DR`'sinde Türkçe kapsayan bir font varsa PDFium onu kullanıyor mu?
   (gömülü TrueType veya Unicode CMap'li CID font içeren fixture)
3. Hiçbiri çalışmazsa: yazma sonrası `/V` ile AP'de çizilen karakterleri karşılaştırarak
   kaybı tespit et, `/NeedAppearances true` ayarla (PdfCraft `undrawn_fields` yaklaşımı;
   PDF/A belgelerinde yasak) ve kullanıcıyı uyar.

Çıkış kriteri: Türkçe değer üç platformda da ya doğru çiziliyor ya da kayıp tespit
edilip bildiriliyor. Sessiz kayıp kabul edilmez.

**Çıkış kriteri:** Fixture'ların tamamında `/AP` yenileniyor ve yeniden açılınca değer doğru render ediliyor. Sorunlu türler yazılı olarak kayda geçiriliyor.

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
| Font kapsamı (Türkçe `ğ ı`, CJK) | **Doğrulandı:** PDFium `ğ ı` karakterlerini AP'den sessizce siliyor | Faz 0b (zorunlu): platform font eşlemesi, `/DR` fontu ya da tespit + `NeedAppearances` + uyarı |
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
