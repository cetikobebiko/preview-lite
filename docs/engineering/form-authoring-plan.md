# Form Alanı Oluşturma (Form Tasarlama) — Uygulama Planı

Önkoşul: `form-filling-plan.md` (Faz 0–3 ve incremental save) tamamlandı. Referans: PdfCraft
0.8.0 `crates/forms/src/author.rs` (MIT OR Apache-2.0); davranış ve kenar durumları referans
alınır, kod taşınmaz.

Hedef: kullanıcı sayfaya yeni alan (metin, çok satırlı metin, checkbox, radio, açılır liste,
liste kutusu) çizebilsin; mevcut alanı taşıyıp boyutlandırabilsin, silebilsin ve
özelliklerini (ad, zorunlu, salt okunur, en fazla karakter, seçenekler, ipucu) değiştirebilsin.
Oluşturulan alanlar mevcut doldurma yoluyla (Faz 1–3) okunur ve doldurulur.

---

## 1. Mevcut durum

| Katman | Durum |
|---|---|
| Okuma/doldurma | Var: `forms.rs` (okuma), `forms/fill.rs` (ham PDFium FORM API, artımlı kayıt), `FormLayer` + taslak/commit denetleyicisi |
| Alan oluşturma | Yok |
| PDFium public API | **Widget oluşturamaz.** `FPDFAnnot_IsSupportedSubtype` (`fpdfsdk/fpdf_annot.cpp`) yalnız Circle, FileAttachment, FreeText, Highlight, Ink, Link, Popup, Square, Squiggly, Stamp, StrikeOut, Text, Underline kabul ediyor. AcroForm'a alan ekleyen fonksiyon yok. |
| PDFium ile değiştirme | Kısmen: `FPDFAnnot_SetRect`, `FPDFAnnot_SetFormFieldFlags`, `FPDFAnnot_SetStringValue`, `FPDFAnnot_SetFontColor`. `FPDFPage_RemoveAnnot` widget'ı sayfadan siler ama `/AcroForm /Fields`'te sarkan referans bırakır. |
| `lopdf` | 0.34, zaten bağımlılık; üretimde sanitization, redaksiyon, a11y ve prepress tarafından job/worker süreçlerinde kullanılıyor. `IncrementalDocument` artımlı yazım destekliyor. |

## 2. Spike sonuçları (2026-10-17, macOS arm64, yamalı PDFium 7961)

Geçici ignored test (kaldırıldı): `lopdf::IncrementalDocument` ile standart fixture'a bir
metin alanı ve bir checkbox eklendi. Her ikisi de kenar ve zemin çizen kendi `/AP`'lerine,
`/DA (/Helv 12 Tf 0 g)` ve `/MK` değerlerine sahipti. Alanlar sayfanın `/Annots`'una ve
`/AcroForm /Fields`'e eklenip artımlı kaydedildi.

| Kontrol | Sonuç |
|---|---|
| Orijinal baytlar önek olarak korundu mu | ✅ (3 293 → 4 775 bayt) |
| Faz 1 okuyucusu (pdfium-render) yeni alanları görüyor mu | ✅ `Text1` (Text, annot 11), `Check Box1` (Checkbox, annot 12, `Off`), normalize rect doğru |
| Faz 3 yazım yolu dolduruyor mu | ✅ `Text1` = "Yeni alan çalışıyor", `Check Box1` = `Yes` |
| Boş alan render'da görünüyor mu | ✅ kendi `/AP`'imizle gri kenar, beyaz zemin |
| Doldurulmuş alan | ✅ PDFium görünümü `/MK` kenar ve zemini koruyarak yeniden üretti; Türkçe doğru |

Sonuç: Yapı `lopdf` ile artımlı yazılır, değer ve görünüm Faz 3'ün PDFium yoluna kalır. İki
araç birbirinin çıktısını sorunsuz okuyor.

## 3. Mimari karar ve `lopdf` gerekçesi (AGENTS.md)

**Neden `lopdf`:** PDFium public API'si widget/alan oluşturmayı ve `/AcroForm /Fields`'i
değiştirmeyi desteklemiyor (bölüm 1). Alternatifler: ikinci bir PDF motoru (AGENTS.md yasaklıyor),
yamalı PDFium'a yeni public API eklemek (yüksek bakım maliyeti, upstream'de karşılığı yok) ya da
ham bayt düzenleme (güvensiz). `lopdf` burada **yalnız yapısal yazım** için kullanılır:
sözlük ekleme/değiştirme ve artımlı serileştirme. Render, metin ve geometri otoritesi PDFium'da
kalır: normalize rect ↔ user space dönüşümü PDFium'un `PageDisplayTransform`'u ile yapılır,
çıktı PDFium ile yeniden açılıp doğrulanır.

**Sınırlar (AGENTS.md ile uyum):**
- Yalnız job worker'da (`--run-job-worker`), session snapshot'ının **kopyası** üzerinde,
  `atomic_write_pdf` içinde çalışır. Ana Tauri sürecinde ya da aktif kullanıcı PDF yolunda
  `lopdf` açılmaz; yeni path-native akış yoktur.
- Şifreli belgeler reddedilir (`lopdf` şifre çözme kullanılmaz; credential job'a iletilmez).
- Mutasyon `MutationRecipe` → `mutate_isolated` yolundan geçer: generation kontrolü, digest,
  recovery ve atomic commit mevcut altyapıdan gelir.

**Güvenlik değerlendirmesi:**
- Güvenilmeyen PDF ayrıştırma, çökmesi ana süreci etkilemeyen, kaynak sınırlı ve tek komutluk
  job worker sürecinde yapılır (sanitization ile aynı model).
- Girdi boyutu sınırlanır (`lopdf` tüm dosyayı belleğe alır): üst sınır, PDFium'un zaten açtığı
  snapshot boyutuyla aynı politika. Sınır aşılırsa preflight hatası.
- Alan ağacı yürüyüşü (`/Parent`, `/Kids`) döngüye karşı ziyaret kümesi ve derinlik sınırıyla
  yapılır. Nesne ve alan sayısı sınırlıdır.
- Kullanıcı girdisi (alan adı, ipucu, seçenekler) yalnız PDF string nesnesi olarak yazılır;
  içerik akışına yalnız kendi ürettiğimiz sabit görünüm operatörleri girer, kullanıcı metni
  görünüm akışına yazılmaz (değer görünümü PDFium'un işi). Alan adında `.` yasaktır (hiyerarşi
  ayırıcısı); uzunluk sınırlıdır.
- Çıktı doğrulaması: PDFium ile yeniden açılır; beklenen alanlar Faz 1 okuyucusuyla, beklenen
  ad, tür, rect ve bayraklarla bulunmalıdır. Önek korunmalıdır (artımlı).
- Doğrudan testler: her işlem için birim testi, bozuk/döngülü alan ağacı, şifreli girdi, sınır
  aşımı ve PDFium geri okuma testleri.

## 4. Etkilenecek dosyalar

**Rust**
- `src-tauri/src/forms/author.rs` *(yeni)*: işlem modeli, istek doğrulaması, `lopdf` artımlı
  yazım, görünüm akışı üretimi, PDFium ile doğrulama.
- `src-tauri/src/forms/test_support.rs`: radio grubu, şifreli ve döngülü ağaç fixture'ları.
- `src-tauri/src/job_worker.rs`: `MutationRecipe::AuthorFormFields { ops }`.
- `src-tauri/src/document_session.rs`, `lib.rs`: `author_session_form_fields` komutu.

**Frontend**
- `src/components/FormDesignLayer.svelte` *(yeni)*: tasarım modunda alan çizme, seçme,
  taşıma/boyutlandırma tutamaçları, silme.
- `src/components/FormFieldProperties.svelte` *(yeni)*: özellik paneli.
- Araç çubuğu: "Form tasarla" modu ve alan türü seçimi; `featureRegistry.js` yeni
  `form_authoring` kapısı (`experimental`).
- `services/backend/contracts.js`, `session.js`, `pdfState.js`: sözleşme ve korunan sarmalayıcı.

**Dokümantasyon**: bu plan, `feature-matrix.md`, `pdfium-worker-isolation.md`.

## 5. Değişiklik adımları

### Faz A — Backend: yapısal işlemler (job worker)

İşlem modeli (bir istek birden çok işlem taşıyabilir; tümü tek artımlı güncelleme):

```rust
enum FormAuthorOp {
    Add { page_index, normalized_rect, kind: NewFieldKind, name: Option<String> },
    Delete { page_index, annot_index, field_name },
    Move { page_index, annot_index, field_name, normalized_rect },
    SetProps { page_index, annot_index, field_name, props: FieldPropsPatch },
}
enum NewFieldKind { Text { multiline }, Checkbox, Radio { group, export }, Combo { options, editable }, List { options, multi } }
struct FieldPropsPatch { name, tooltip, required, read_only, max_len, options, multiline }  // hepsi Option
```

1. **Ekleme:** Acrobat ad kuralı (`Text1`, `Check Box1`, `Group1`, `Dropdown1`, `List Box1`),
   belgede benzersizlik denetimi. AcroForm yoksa oluşturulur (katalog artımlı klonlanır);
   `/DR`'de `/Helv` (ve checkbox için `/ZaDb`) yoksa eklenir. `/DA (/Helv 12 Tf 0 g)`, `/MK`
   (gri kenar, beyaz zemin), radio/checkbox için on/off görünüm akışları (işaret yollarla
   çizilir, font gerekmez), metin/seçim için kenar+zemin akışı. Radio: aynı grup adı varsa
   mevcut alanın `/Kids`'ine eklenir.
2. **Silme:** widget sayfanın `/Annots`'undan, alan `/AcroForm /Fields`'ten (ya da
   üst alanın `/Kids`'inden) çıkarılır; son widget giderse üst alan da kalkar.
3. **Taşıma/boyutlandırma:** `/Rect` güncellenir; kendi ürettiğimiz kenar/zemin görünümü yeni
   boyutla yeniden üretilir. Değeri olan metin alanında görünüm PDFium'a yeniden ürettirilir
   (açık soru, Faz A'da spike: değer aynıyken PDFium'u yeniden çizmeye zorlamanın yolu).
4. **Özellikler:** `/T` (yeniden adlandırma, benzersizlik ve `.` denetimi), `/TU`, `/Ff`
   bitleri (doğru ISO 32000 bitleriyle; pdfium-render'ın kaymış sabitleri kullanılmaz),
   `/MaxLen`, `/Opt`.
5. **İsim-indeks doğrulaması:** Faz 3 ile aynı ilke; `annot_index` ile `field_name`
   eşleşmezse istek reddedilir.
6. **Doğrulama:** `atomic_write_pdf` doğrulama adımı PDFium ile yeniden açar; eklenen,
   taşınan ve değişen alanları Faz 1 okuyucusuyla, silinenlerin yokluğunu denetler. Önek
   korunmalıdır.
7. **İmzalar:** yazım artımlıdır, imzalı baytlar korunur. Ancak alan eklemek ya da silmek
   DocMDP P=2'nin izin verdiği değişikliklerden değildir. Frontend, imzalı belgede ayrı bir
   uyarı gösterir ("imzalar korunur ama alan değişiklikleri imzanın izin verdiği
   değişikliklerden değildir").

#### Faz A sonuçları (2026-10-17)

Uygulandı: `src-tauri/src/forms/author.rs`: `FormAuthorOp` (`add` / `delete` / `move` /
`set_props`, JSON `{ op, pageIndex, … }`), `validate_request`, `author_form_fields_report`
(`atomic_write_pdf`), `MutationRecipe::AuthorFormFields`, `author_session_form_fields`
(`mutate_isolated`). Frontend sözleşmesi Faz B'de eklenecek.

- **Yazım:** `lopdf::IncrementalDocument`. Değişen nesneler (sayfa, AcroForm, üst alan,
  `/Annots`/`/Fields`/`/Kids` dizileri) yeni revizyona kopyalanır. Referanslı ve satır içi
  sözlük/diziler desteklenir. Kayıttan sonra çıktının orijinal baytlarla başladığı
  denetlenir.
- **Ekleme:** metin (çok satırlı), checkbox, radio (aynı grup adına katılır, yoksa üst alan
  oluşturulur), açılır liste (düzenlenebilir), liste kutusu (çoklu seçim). Acrobat varsayılan
  adları (`Text1`, `Check Box1`, `Dropdown1`, `List Box1`), ad çakışması reddi. AcroForm yoksa
  oluşturulur; `/DR`'ye `/Helv` ve `/ZaDb` eklenir. Görünüm akışları: beyaz zemin + gri kenar;
  checkbox ✓ ve radio çember/nokta yollarla çizilir (font gerekmez).
- **Silme:** widget sayfadan, alan `/Fields`'ten ya da üst alanın `/Kids`'inden; son çocuk
  giderse üst alan da kalkar.
- **Taşıma:** yalnız `/Rect`; görünüm akışı PDF kuralıyla (BBox → Rect) yeni dikdörtgene
  ölçeklenir. Değerli metin alanı boyutlandırılırsa metin bir sonraki doldurmaya kadar
  gerilmiş görünebilir (bilinen sınır).
- **Özellikler:** ad (aynı hiyerarşide, çakışma ve `.` reddi), ipucu, zorunlu, salt okunur,
  çok satırlı (yalnız metin), `MaxLen` (yalnız metin, 0 kaldırır), seçenekler (yalnız seçim).
  `/Ff` bitleri ISO 32000'e göre yazılıyor.
- **Hedefleme:** `annot_index` PDFium'un annotation sırasıyla aynı (`/Annots` dizi indeksi);
  tüm hedefler işlemler uygulanmadan önce, orijinal sıraya göre çözülüyor ve tam nitelikli
  adla doğrulanıyor. Satır içi annotation sözlükleri reddediliyor.
- **Güvenlik sınırları:** şifreli girdi (credential ya da `lopdf` `is_encrypted`) reddedilir;
  256 MiB girdi sınırı; alan ağacı yürüyüşünde ziyaret kümesi, 64 derinlik ve 100 000 düğüm
  sınırı; `/Parent` döngüsü hata verir.
- **Doğrulama:** `atomic_write_pdf`'in doğrulama adımı PDFium ile yeniden açar; sayfa başına
  form widget sayısını ve eklenen/taşınan/değişen alanları (ad, tür, rect, bayraklar,
  seçenekler) Faz 1 okuyucusuyla denetler.

Testler: `forms::author::tests` (11): her türün eklenip PDFium ile okunması ve Faz 3 yoluyla
doldurulması, varsayılan adlar ve çakışma, silme (radio grubunun son widget'ı ve `/Fields`
temizliği), taşıma, özellikler (MaxLen'in doldurmada uygulanması dahil), isim-indeks,
AcroForm'suz belge, şifreli girdi ve döngülü ağaç, imzalı belgede önek, istek doğrulaması,
JSON biçimi. `job_worker::tests::author_form_fields_mutation_round_trips_through_job_worker_dispatch`.
Görsel kontrol: PDFKit (CoreGraphics) oluşturulan ve doldurulan alanları doğru çiziyor.

**Bu sırada bulunan kritik hata (form doldurmayı etkiliyordu):** PDFium'un artımlı kaydı,
xref stream kullanan kaynaklarda güncellemenin xref stream nesnesini `/Type /XRef` ve `endobj`
olmadan yazıyor; CoreGraphics (Preview, PDFKit) bu dosyaları açmıyordu. `forms::fill` artık
artımlı kayıttan sonra bu nesneyi onarıyor (ayrı commit; ayrıntı `form-filling-plan.md`).

### Faz B — Frontend: tasarım modu

1. Araç çubuğunda "Form tasarla" modu; açıkken `FormLayer` (doldurma) yerine
   `FormDesignLayer` görünür.
2. Alan türü seç → sayfada sürükleyerek dikdörtgen çiz → `Add` işlemi hemen gönderilir.
   Geometri AnnotationLayer'ın normalize uzay yardımcılarıyla aynıdır.
3. Mevcut alana tıklama onu seçer: taşıma (sürükle), köşe tutamaçlarıyla boyutlandırma,
   Delete tuşu ve silme düğmesi. Bırakınca `Move` gönderilir.
4. Özellik paneli: ad, ipucu, zorunlu, salt okunur, çok satırlı, en fazla karakter, seçenek
   listesi. Değişiklik odak kaybında `SetProps` olarak gönderilir.
5. Her işlem bir mutasyondur (yeni generation); stale-result koruması Faz 2–3 ile aynı.

### Faz C — Sonraki adımlar (ayrı karar)

- Alanı başka sayfalara çoğaltma, sekme sırası (`/Tabs`), hizalama/dağıtma araçları.
- Push button, imza alanı, tarih alanı (JavaScript format eylemi ister).
- Görünüm sekmesi: kenar/zemin rengi, font boyutu, kenar stili.

## 6. Mimari riskler

| Risk | Önlem |
|---|---|
| `lopdf`'in ayrıştıramadığı gerçek dünya PDF'leri (bozuk xref, olağandışı yapılar) | Hata açık mesajla döner, belge değişmez. Spike sınırlı fixture'larla yapıldı; Faz A'da `tests/fixtures/pdf` ve perf fixture'larıyla ayrıştırma taraması |
| Object stream / xref stream içeren belgelerde `IncrementalDocument` | Faz A'da xref-stream'li fixture ile test; çıktı PDFium doğrulamasından geçmezse reddedilir |
| İki yazarın (lopdf yapı, PDFium değer) aynı nesneyi farklı yorumlaması | Her işlem sonrası PDFium geri okuması zorunlu; fill testleri yeni alanlar üzerinde de koşar |
| Taşıma sonrası değerli metin alanında eski boyutlu görünüm | Faz A spike'ı; çözülemezse taşıma sonrası `/AP` kaldırılıp `NeedAppearances` yerine PDFium ile yeniden yazım |
| DocMDP izinleri | Uyarı; karar kullanıcıda |
| Ad çakışması / hiyerarşik adlar | Benzersizlik ve `.` denetimi backend'de |

## 7. Regresyon riskleri

- Faz 3 doldurma yolunun yeni alanlarda da çalışması: ortak fixture'larla test.
- `FormLayer` ile `FormDesignLayer`'ın aynı anda olay yakalaması: mod tek kaynaktan.
- Feature kapısı açılmadan UI görünmemeli.

## 8. Gerekli testler

- Rust: her işlem türü için ekle → PDFium ile oku → doldur; silme sonrası `/Fields` ve
  `/Annots` tutarlılığı; yeniden adlandırma çakışması; `.` ve uzun ad reddi; isim-indeks
  uyuşmazlığı; şifreli girdi reddi; döngülü `/Parent` ağacı; önek korunması; imzalı fixture'da
  imza içeriğinin değişmemesi; xref-stream'li girdi.
- Job worker dağıtım testi (JSON).
- Vitest: tasarım katmanı (çizim → doğru normalize rect ve istek), seçim/taşıma/boyutlandırma,
  özellik paneli, sözleşme doğrulaması.
- Tarayıcı harness'i ile görsel kontrol; yerel Tauri penceresi (kullanıcı).

## 9. Kabul kriterleri

1. Beş alan türü sayfaya çizilerek eklenebilir; eklenen alan hemen görünür (kendi görünüm
   akışıyla) ve doldurma modunda doldurulabilir.
2. Alan taşınabilir, boyutlandırılabilir, silinebilir; özellikleri değiştirilebilir.
3. Tüm yazımlar artımlıdır; imzalı belgede imza içeriği korunur ve kullanıcı uyarılır.
4. `lopdf` yalnız job worker'da, snapshot kopyası üzerinde çalışır; ana süreçte yeni PDF
   ayrıştırma yoktur.
5. Yeni bağımlılık yoktur; `cargo test`, `clippy`, `vitest` geçer.

## 10. Kapsam dışı

- Push button, imza alanı, tarih/sayı format eylemleri (JavaScript), hesaplama.
- Görünüm sekmesi (renk, font, kenar stili), alan çoğaltma, sekme sırası düzenleme.
- XFA formları.
