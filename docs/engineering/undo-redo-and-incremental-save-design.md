# Undo/Redo ve Artımlı Kaydetme Tasarımı (Faz 0)

> [!NOTE]
> **Yalnız tasarım.** Bu dokümanla birlikte hiçbir production kodu değişmedi. Buradaki
> kurallar uygulanırken zorunlu olacak sözleşmedir; "Doğrulanmamış varsayımlar" bölümündeki
> maddeler spike ile doğrulanmadan uygulamaya geçilmez.

## 1. Amaç ve kapsam

pdfcraft-0.9.0'dan özellik taşıma kararı (seçenek B: yeni bağımlılık yok, PDFium + lopdf 0.34)
birçok düzenleme özelliğini getirecek. Bunlardan önce iki altyapı eksiği kapanmalı:

1. **Artımlı mutasyon:** Her içerik mutasyonu mümkün olduğunda belgeyi baştan yazmak yerine
   sona bir revizyon ekler. Böylece imzalı baytlar korunur ve undo ucuzlar.
2. **Undo/redo:** Oturumdaki her doğrulanmış içerik mutasyonu geri alınabilir ve yinelenebilir.

Kapsam dışı (bu faz):

- Şifreleme ekleme/kaldırma, PAdES imzalama, kriptografik imza doğrulama (B kısıtı altında
  engelli; yeni kripto bağımlılığı gerektirir).
- Uygulama yeniden başlatıldıktan sonra undo geçmişini sürdürmek. Mevcut kural korunur:
  süreç içi durum kalıcı değildir (`document-session.md` §Job snapshots).
- Belgeyi sıkıştıran/çöp toplayan "optimize ederek kaydet" (Faz 5, optimize).

## 2. Mevcut durum (kanıtlarla)

### 2.1 Oturum ve kaydetme modeli

- Her açık PDF'in backend'e ait bir `working.pdf` kopyası vardır; frontend yalnız
  `(sessionId, contentGeneration)` görür (`document-session.md` §Identity).
- Mutasyonlar `DocumentSessionStore::mutate` / `mutate_isolated` üzerinden exclusive worker
  admission ve oturum write lease'i altında çalışır
  ([document_session.rs:4228](../../src-tauri/src/document_session.rs),
  [document_session.rs:4287](../../src-tauri/src/document_session.rs)).
- Başarılı commit `working_digest`'i günceller, `dirty = true` yapar, `content_generation` ve
  `state_revision`'ı artırır (`apply_committed_mutation`,
  [document_session.rs:7685](../../src-tauri/src/document_session.rs)).
- Save, `working.pdf`'i hedefe atomik olarak kopyalar ve açılış/son kayıt SHA-256 baseline'ını
  ön koşul olarak kullanır (`document-session.md` §Atomic Save mapping).
- Save sonrası `dirty = content_generation != persisted_content_generation`
  ([document_session.rs:4904-4905](../../src-tauri/src/document_session.rs)).
- Tek dosyalık atomik değiştirme Unix'te düz `fs::rename`'dir ve eski dosyanın yedeğini
  tutmaz (`recovery_backup: None`,
  [document_io.rs:2197-2220](../../src-tauri/src/document_io.rs)). Yedek yalnız Windows
  yolunda (`ReplaceFileW`) ve çoklu dosya çıktısında hard link olarak oluşur
  ([document_io.rs:936-942](../../src-tauri/src/document_io.rs)); commit doğrulanınca
  silinir (`CommittedBackupDisposition::Remove`,
  [document_io.rs:1157-1178](../../src-tauri/src/document_io.rs)).
- Birden fazla hard link'i olan hedef reddedilir: ön kontrolde
  ([document_io.rs:531](../../src-tauri/src/document_io.rs)) ve hedef kilidi altında tekrar
  ([document_io.rs:2082](../../src-tauri/src/document_io.rs)).

**Sonuç:** Save zaten bayt-bayt kopyadır. Yani kaydedilen dosyanın artımlı olup olmaması
tamamen mutasyonların `working.pdf`'i nasıl yazdığına bağlıdır. Save tarafında değişiklik
gerekmez.

### 2.2 Mutasyon başına yazım modu

`execute_mutation` ([job_worker.rs:1362](../../src-tauri/src/job_worker.rs)) ve ilgili modüller:

| Recipe / işlem | Bugünkü yazıcı | Yazım modu |
|---|---|---|
| `RotatePage`, `DeletePages`, `InsertBlankPage` | PDFium `save_to_file` ([manipulation.rs](../../src-tauri/src/manipulation.rs)) | **Tam yeniden yazım** |
| `ReorderPages`, `ExtractPages` | PDFium, yeni belgeye sayfa aktarımı ([manipulation.rs:285](../../src-tauri/src/manipulation.rs)) | **Tam yeniden yazım** |
| `AddAnnotation`, `DeleteAnnotation` | PDFium `save_to_file` ([annotations.rs:672](../../src-tauri/src/annotations.rs), [annotations.rs:776](../../src-tauri/src/annotations.rs)) | **Tam yeniden yazım** |
| Form doldurma | PDFium ham `FPDF_SaveAsCopy(FPDF_INCREMENTAL)` ([forms/fill.rs:239-269](../../src-tauri/src/forms/fill.rs)) | **Artımlı** |
| Form tasarımı | `lopdf::IncrementalDocument` ([forms/author.rs:6](../../src-tauri/src/forms/author.rs), [forms/author.rs:385](../../src-tauri/src/forms/author.rs)) | **Artımlı** (şifreli belge reddedilir, [forms/author.rs:380](../../src-tauri/src/forms/author.rs)) |

Frontend, tam yeniden yazım yapan mutasyonlardan önce imzalı belgede onay ister
([signatureLossGuard.js](../../src/services/signatureLossGuard.js)).

### 2.3 Projede zaten öğrenilmiş dersler

- **PDFium artımlı xref stream hatası:** Kaynak dosya xref stream kullanıyorsa PDFium artımlı
  güncellemenin xref stream nesnesini `/Type /XRef` ve `endobj` olmadan yazar. PDFium bunu
  okur, CoreGraphics (Preview, PDFKit) reddeder. `repair_incremental_xref_stream` bunu offset
  kaydırmadan onarır ([forms/fill.rs:271-279](../../src-tauri/src/forms/fill.rs)).
- **İmzalı belgede sessiz geri dönüş yok:** Artımlı kayıt başarısız olursa imzalı belge hata
  verir; yalnız imzasız belge tam yeniden yazıma düşer
  ([forms/fill.rs:246-253](../../src-tauri/src/forms/fill.rs)).
- **Prefix değişmezliği:** İmzalı belgede çıktının girdi baytlarıyla başlaması doğrulanır
  ([forms/fill.rs:259-262](../../src-tauri/src/forms/fill.rs)).

### 2.4 Undo

Kod tabanında undo/redo yoktur.

### 2.5 lopdf 0.34 `IncrementalDocument` davranışı

Kaynak: `lopdf-0.34.0/src/writer.rs` (`IncrementalDocument::save_internal`) ve
`src/document.rs` (`Document::new_from_prev`).

- Önceki baytları aynen yazar, sonra yeni nesneleri ve önceki dosyayla **aynı türde** xref
  bölümünü (tablo ya da stream) ekler. `/Prev` önceki `xref_start`'a ayarlanır.
- Artımlı bölümün başına `%PDF-1.4` satırı yazar (`new_from_prev` sürümü sabit `"1.4"`
  koyar). Bu bir yorum satırıdır, okuyucular yok sayar; yine de standart dışıdır (bkz. §8 S3).
- Trailer önceki trailer'ın kopyasıdır; xref stream'li girdilerde stream'e özgü anahtarların
  doğru yeniden yazıldığı doğrulanmalıdır (§8 S3).
- Nesne silmeyi (free entry) ifade etmez; silinen nesne yalnız referanssız kalır.
- Yeni nesneleri şifreleyemez. Şifreli belgede lopdf ile artımlı yazım yapılamaz.

## 3. Referans: pdfcraft modeli

Yalnız tasarım fikri alınmıştır; Faz 0'da pdfcraft'tan kod kopyalanmaz.

- Undo yığını bellek içi copy-on-write belge kopyalarıdır; en fazla 100 adım
  (`engine/src/lib.rs:210`, `MAX_UNDO`). Yeni düzenleme redo'yu temizler.
- Undo/redo da yeni bir generation üretir; `dirty`, "çalışma durumu son açılan/kaydedilen
  baytlar mı" sorusuyla hesaplanır (`sync_dirty`), sayaçla değil.
- Undo kayıttan sonra da çalışır. Kaydedilmiş durumdan geriye gidip tekrar kaydetmek,
  kaydedilen dosyanın **üzerine** eski nesneleri geri koyan bir revizyon ekler
  (`save_bytes`, `overlay_changes_from`), kaydedilen dosyayı kısaltmaz.
- Redaction, şifreleme değişikliği ve onarılmış (xref'i yeniden kurulmuş) dosya her zaman tam
  yeniden yazılır; imzalı belgede bunlar reddedilir.
- Kaydetmeden önce üretilen baytlar yeniden açılıp sayfa sayısıyla doğrulanır (`check_save`).

Bizim modelimiz farklıdır: durum bellekte değil disktedir (`working.pdf`), her mutasyon
ayrı bir worker işlemidir ve atomik commit/kurtarma makinesi vardır. Bu yüzden undo'yu
**dosya geçmişi** olarak tasarlıyoruz (§5).

## 4. Tasarım A: Artımlı mutasyon

### 4.1 Hedef yazım modu

| Recipe / işlem | Hedef | Motor | Not |
|---|---|---|---|
| `RotatePage` | Artımlı | lopdf `IncrementalDocument`; şifreli belgede PDFium | Etkin (miras alınmış) `/Rotate` hesaplanıp sayfa sözlüğüne yazılır. PDFium yerine lopdf seçildi: PDFium dokunduğu sayfanın içerik ve görsel nesnelerini de yeniden yazıyor (S6) |
| `DeletePages` | Artımlı | lopdf `IncrementalDocument`; şifreli belgede PDFium | `ReorderPages` ile aynı düzleştirme (S3b). Silinen sayfanın içeriği önceki revizyonda kalır (§7.3) |
| `InsertBlankPage` | Artımlı | PDFium `FPDF_INCREMENTAL` | Büyüme sabit ve küçük (~1,6 KB, S6) |
| `AddAnnotation`, `DeleteAnnotation` | Artımlı | PDFium `FPDF_INCREMENTAL` | PDFium appearance stream üretir; ancak sayfanın görselleri de yeniden yazılır (S6, karar K5) |
| `ReorderPages` | Artımlı | lopdf `IncrementalDocument` | Sayfa ağacı tek bir `/Kids` dizisine yeniden yazılır; miras alınan `Resources`, `MediaBox`, `CropBox`, `Rotate` sayfalara indirilir (S3b ile doğrulandı) |
| `ExtractPages` | Tam yeniden yazım (değişmez) | PDFium | Anlamı "yeni belge"dir; artımlı olması alt kümeyi küçültmez |
| Form doldurma, form tasarımı | Değişmez | | Zaten artımlı |

### 4.2 Ortak artımlı kayıt yardımcıları

- `forms/fill.rs` içindeki `save_copy`, `FPDF_FILEWRITE` adaptörü,
  `save_preserving_signatures` ve `repair_incremental_xref_stream` tek bir paylaşılan modüle
  taşınır: [`incremental_save`](../../src-tauri/src/incremental_save.rs). `document_io` altına
  konmadı, çünkü `tests/document_io_fs.rs` o dosyayı PDFium'suz ayrı bir binary olarak derliyor.
  Davranış değişmez, yalnız yer değişir; mevcut
  testleri de onlarla birlikte taşınır.
- Tüm PDFium tabanlı recipe'ler bu yardımcıyı kullanır.

### 4.3 Append-only doğrulaması

Artımlı yazım iddia eden her çıktı commit'ten önce şunları sağlamalıdır:

1. `output.len() > input.len()`.
2. `sha256(output[..input.len()]) == before_digest` (girdinin baytları aynen korunmuş).
3. Son `startxref` girdi uzunluğunun ötesini gösterir; artımlı bölümün `/Prev` değeri
   girdinin son `startxref` değerine eşittir.
4. Mevcut PDFium yeniden açma ve recipe'ye özgü doğrulama (`atomic_write_pdf` validate) geçer.

1–3 sağlanmazsa çıktı "artımlı" sayılmaz ve §4.4 politikası uygulanır.

### 4.4 Geri dönüş politikası

Hiçbir zaman sessiz değildir:

| Durum | Davranış |
|---|---|
| Artımlı yazım başarılı ve §4.3 geçti | `writeMode = appended` |
| Artımlı yazım mümkün değil, belge imzasız | Tam yeniden yazım, `writeMode = rewritten` |
| Artımlı yazım mümkün değil, belge imzalı | Typed hata `INCREMENTAL_WRITE_UNAVAILABLE`; frontend imza kaybı onayı alırsa aynı istek `allowRewrite: true` ile yeniden gönderilir |
| Şifreli belge + lopdf recipe | Önce PDFium yolu denenir; yoksa yukarıdaki imza kuralı |
| Girdi xref'i onarılarak açıldı | Artımlı yazım güvenli değildir (zincir yok); tam yeniden yazım + imza kuralı |

`writeMode` mutasyon sonucuna ve geçmiş kaydına yazılır (§5.2). Frontend imza uyarısının
doğruluk kaynağı backend olur: `signatureLossGuard.js` sabit bir liste yerine backend'in
recipe için bildirdiği yazım moduna bakar.

### 4.5 Dosya büyümesi

Her artımlı mutasyon dosyayı büyütür (ör. aynı sayfayı 20 kez döndürmek 20 revizyon).
Bu bilinçli bir takastır. Kalıcı çözüm Faz 5'teki "optimize ederek farklı kaydet"tir.

## 5. Tasarım B: Dosya geçmişine dayalı undo/redo

### 5.1 Neden dosya geçmişi

- Ters işlem (inverse command) yaklaşımı reddedildi: her recipe için ayrı ve hataya açık
  geri alma mantığı gerekir, PDFium'un yaptığı yan değişiklikleri (AP üretimi vb.) bilemez.
- Bellek içi belge kopyası (pdfcraft modeli) bizde yok; belge worker sürecindedir.
- Dosya geçmişi her yazıcıyla çalışır, mevcut atomik commit ve digest makinesini aynen
  kullanır ve geri yüklenen durumun bayt-bayt doğruluğu digest ile kanıtlanır.

### 5.2 Veri modeli

`DocumentSession`'a eklenecek, frontend'e path içermeden yansıtılacak alanlar:

```text
HistoryEntry {
  label: HistoryLabel,          // ör. RotatePage, AddAnnotation (i18n frontend'de)
  digest: Sha256,               // bu durumun working.pdf digest'i
  len: u64,                     // bu durumun bayt uzunluğu
  source: BlobRef,              // Working | Retained(BlobId)
  write_mode: Appended | Rewritten,
}

history: {
  undo: Vec<HistoryEntry>,      // en eski → en yeni
  redo: Vec<HistoryEntry>,
  retained_bytes: u64,
}
```

- `BlobRef::Working`: durum, **güncel** `working.pdf`'in ilk `len` baytıdır (append-only
  zincirde ek dosya gerekmez).
- `BlobRef::Retained(id)`: `sessions/session-<id>/history/<id>.pdf` altında değişmez dosya.
- Snapshot'a eklenecek alanlar: `canUndo`, `canRedo`, `undoLabel`, `redoLabel`. Path, digest
  ve blob kimliği snapshot'a girmez (`document-session.md` §Identity kuralı).

### 5.3 Mutasyon commit'inde geçmişe yazma

Mutasyon exclusive lease altında commit edildiğinde:

1. Önceki durum için bir `HistoryEntry` oluşur (`digest = before_digest`).
2. Yeni çıktı append-only ise (§4.3), giriş `BlobRef::Working` olur; ek disk maliyeti sıfırdır.
3. Değilse, eski `working.pdf` kopyalanmadan `history/` altında tutulur. Bunun için
   `document_io`'ya açık bir seçenek eklenir (öneri: `retain_replaced_target_as(path)`):
   - **Unix:** `atomic_replace` içinde, hedef kilidi altındaki link sayısı kontrolünden
     ([document_io.rs:2082](../../src-tauri/src/document_io.rs)) **sonra** ve `rename`'den
     hemen önce `fs::hard_link(target, retain_path)`. Rename sonrası yeni `working.pdf`'in
     link sayısı yine 1'dir; eski inode yalnız geçmişte yaşar.
   - **Windows:** `ReplaceFileW`'nin zaten oluşturduğu yedek silinmek yerine geçmişe taşınır
     (yeni `CommittedBackupDisposition::RetainAsHistory`).
   - Link'i bu sıradan önce oluşturmak, çok-link reddi nedeniyle bir sonraki atomik yazımı
     kırar; link'i desteklemeyen dosya sistemlerinde kopyaya düşülür.

   Bu anda `BlobRef::Working` gösteren tüm eski girdiler de bu dosyaya yeniden bağlanır,
   çünkü artık güncel `working.pdf`'in prefix'i değildirler.
4. Redo listesi ve ona ait blob'lar silinir.
5. Geçmişe yazma başarısız olursa **mutasyon geri alınmaz**: mutasyon zaten commit edilmiştir.
   Bunun yerine geçmiş temizlenir ve `HISTORY_TRUNCATED` uyarısı `health_warning` ile
   bildirilir.

### 5.4 Undo akışı

`undo_session_mutation(sessionId, expectedContentGeneration)`:

1. Mutasyonla aynı kapı: exclusive worker admission, oturum write lease'i,
   `ensure_open_for_mutation(expected_generation)`, eski generation için worker
   belgesinin geçersiz kılınması.
2. Hedef girdi = `undo.last()`.
3. Geri yüklenecek baytları hazırla:
   - `Working` → güncel `working.pdf`'in ilk `len` baytı temp'e akıtılır.
   - `Retained` → blob temp'e kopyalanır (blob redo için gerekmediğinden doğrudan rename de
     olabilir; §8 S5).
4. `sha256(temp) == entry.digest` doğrulanır. Uyuşmazlık geçmiş bozulmasıdır: işlem
   reddedilir, geçmiş temizlenir, `working.pdf`'e dokunulmaz.
5. Mevcut `atomic_write_pdf` doğrulaması (PDFium yeniden açma) çalışır.
6. Atomik değiştirme; eski `working.pdf` redo girdisi olur (append-only ilişki varsa
   `Working`, yoksa `RetainAsHistory`).
7. Durum güncellemesi: `working_digest = entry.digest`, `content_generation` **yeni bir
   değere** artar (generation asla geri gitmez; render ve stale-result korumaları aynen
   çalışır), `state_revision` artar, `dirty` yeniden hesaplanır (§5.6).
8. Commit state eşlemesi mutasyonla aynıdır (`document-session.md` §Atomic mutation
   mapping): `RecoveryRequired` / `Indeterminate` aynı yaşam döngüsü geçişlerini yapar.

`redo_session_mutation` simetriktir.

### 5.5 Geçmişin sınırları

Değerler geçicidir, ölçülmeden kesinleşmez (bkz. §8 S6):

- En fazla 50 undo adımı.
- Saklanan blob'ların toplam boyutu için bir bütçe (öneri: 1 GiB veya belge boyutunun
  8 katı, hangisi küçükse).
- Bütçe aşılırsa en eski girdiler silinir. Bir girdinin blob'u silinirse o girdi ve ondan
  daha eskiler düşer.

### 5.6 `dirty` hesabı

Bugünkü sayaç karşılaştırması (`content_generation != persisted_content_generation`) undo
ile yanlış sonuç verir: kaydedilmiş duruma undo ile dönmek yeni bir generation üretir ama
içerik kaydedilmiş içerikle aynıdır. Yeni kural:

```text
dirty = working_digest != Some(persisted_working_digest)
```

`persisted_working_digest`, açılışta ve her başarılı Save'de `working_digest`'ten alınır.
Save bayt-bayt kopya olduğu için bu, baseline digest'iyle aynı değerdir (§8 S7'de doğrulanır).

### 5.7 Kayıttan sonra undo

İki seçenek var (karar gerektirir, §9 K1):

- **K1-a (öneri, v1):** Undo kaydı aşabilir. Eski bir duruma dönüp kaydetmek o durumun
  baytlarını yazar. Zincir append-only ise bu, kaydedilmiş dosyanın bir prefix'idir:
  imzalar korunur, yalnız kullanıcının geri aldığı revizyonlar düşer. Basit, bayt-bayt
  doğrulanabilir.
- **K1-b (pdfcraft modeli):** Eski duruma dönüp kaydetmek, kaydedilmiş dosyanın üzerine eski
  nesneleri geri koyan bir revizyon ekler; dosya hiç kısalmaz, revizyon zinciri denetim izi
  olarak kalır. İki durum arasında nesne düzeyinde fark hesabı gerektirir (lopdf); daha
  karmaşık.

### 5.8 Geçmişin temizlendiği olaylar

- Oturum kapanışı ve discard: geçmiş dizini silinir.
- `recover_document_session`: `working.pdf`'in geçmişle ilişkisi bilinmediği için undo ve
  redo temizlenir.
- **Güvenlik bariyeri:** İçeriği kalıcı olarak kaldırmayı vaat eden bir işlem (secure
  redaction, sanitization) ileride bir oturum mutasyonu olursa: (1) her zaman tam yeniden
  yazım yapar, (2) commit'te tüm geçmişi ve blob'ları siler. Bugün ikisi de `working.pdf`'e
  değil kullanıcının seçtiği harici hedefe yazdığı için etkilenmez
  ([sanitization_publication.rs:65-76](../../src-tauri/src/sanitization_publication.rs)).

### 5.9 Diğer bileşenlerle etkileşim

| Bileşen | Etki |
|---|---|
| Job snapshot'ları | Undo yeni generation üretir; eski generation'dan alınmış job çıktısı mevcut korumalarla zaten uygulanmaz |
| Render ve thumbnail cache | `(sessionId, contentGeneration)` anahtarlı; ek iş yok. Sayfa sayısı değişebileceği için frontend sayfa boyutlarını yeniden çekmelidir (mevcut mutasyon yolunun bunu nasıl yaptığı uygulamada doğrulanır) |
| VectorStore | Generation değişince eski indeks zaten budanır (`document-session.md` §VectorStore) |
| Transition (switch/close) | Değişiklik yok; discard geçmişi siler |
| Signing/LTV | Oturumu değiştirmez; geçmişe girmez |
| İmzalar | Artımlı bir revizyonu geri almak prefix'e dönmektir, imzalar etkilenmez. Tam yeniden yazımı geri almak orijinal imzalı baytları geri getirir |

### 5.10 IPC ve frontend

- Yeni komutlar: `undo_session_mutation`, `redo_session_mutation`. Payload:
  `sessionId`, `expectedContentGeneration`. Dönüş: `DocumentSessionSnapshot`.
- Frontend komut paleti ve ⌘Z / ⇧⌘Z kısayolları; etiket `undoLabel` kodundan i18n ile.
- Undo/redo, Save/Save As ile aynı single-flight kilidini paylaşır.

## 6. Güvenlik değerlendirmesi

- `history/` dizini oturum dizininin altındadır ve mevcut temp sahipliği kurallarını
  devralır: Unix `0700`/`0600`, Windows'ta yalnız mevcut kullanıcıya açık DACL
  (`document-session.md` §Temp ownership).
- Şifreli belgenin blob'ları da şifreli baytlardır (`working.pdf` ile aynı). Credential
  geçmişe yazılmaz.
- Geri yükleme her zaman digest ile doğrulanır; temp dizinindeki bir değişiklik sessizce
  belgeye giremez.
- Yeni lopdf kullanımı (`ReorderPages`) `AGENTS.md`'deki şartları sağlamalıdır: yalnız job
  worker'da, snapshot kopyası üzerinde, `atomic_write_pdf` içinde, şifreli girdi kuralıyla
  ve doğrudan testle.

## 7. Bilinen takaslar

1. **Dosya büyümesi:** §4.5.
2. **Disk kullanımı:** Tam yeniden yazım adımları tam boy blob tutar; bütçe §5.5.
3. **Silinen içerik dosyada kalır:** Artımlı silme (sayfa, annotation) eski revizyonda
   içeriği bırakır. Bu, pdfcraft ve Acrobat'ın artımlı kaydıyla aynı davranıştır. Kalıcı
   kaldırma için sanitization kullanılır. Kullanıcıya bunun nasıl anlatılacağı karar
   gerektirir (§9 K3).

## 8. Doğrulanmamış varsayımlar (spike listesi)

Uygulamadan önce her biri test fixture'larıyla doğrulanmalıdır:

| # | Varsayım | Nasıl doğrulanır |
|---|---|---|
| S1 | PDFium `FPDF_INCREMENTAL`, rotate/delete/insert/annotation için de append-only ve geçerli çıktı üretir | Her recipe için §4.3 kontrolleri + Preview/PDFKit ile açma; xref tablosu, xref stream, object stream ve hybrid `/XRefStm` girdileri |
| S2 | PDFium artımlı kaydı şifreli belgede şifreyi koruyarak çalışır | Şifreli fixture'da kayıt, tekrar parolayla açma, yeni nesnelerin şifreli olduğunu doğrulama |
| S3 | lopdf 0.34 `IncrementalDocument`, xref stream'li girdide doğru trailer/xref stream yazar; ara `%PDF-1.4` satırı CoreGraphics ve PDFium'da sorun çıkarmaz | `ReorderPages` prototipi + Preview/PDFKit + PDFium açma |
| S4 | Girdinin xref onarımıyla açıldığı tespit edilebilir | PDFium'un public API'si bunu bildirmiyorsa: `startxref` → xref → `/Prev` zincirini bayt düzeyinde doğrulayan küçük bir kontrol |
| S5 | (a) Unix'te kilit altındaki link kontrolünden sonra alınan hard link, rename ve kurtarma (`OriginalRecovered`, `RecoveryRequired`) garantilerini bozmaz; (b) Windows `ReplaceFileW` yedeği geçmişe taşınabilir; (c) geri yüklenecek blob, redo için gerekmediğinde rename ile `working.pdf` yerine geçebilir | `document_io` testleri, Windows dahil; link desteklemeyen dosya sisteminde kopya yolu |
| S6 | §5.5 limitleri makul | 1 MB / 50 MB / 500 MB fixture'larla disk ve süre ölçümü (`scripts/generate-perf-fixtures.py`) |
| S7 | Açılışta `working.pdf`'in digest'i orijinal dosyanınkiyle aynıdır | Oturum açma kodu ve testi |

### 8.1 Spike bulguları (2026-10-18, macOS, debug build)

Harness: [incremental_undo_spike.rs](../../src-tauri/tests/incremental_undo_spike.rs)
(`#[ignore]`, production modüllerini çağırmaz). İşlemler PDFium ham API'siyle yapıldı
(production'daki pdfium-render yolu değil; PDFium düzeyinde eşdeğer). Bağımsız doğrulama:
CoreGraphics + PDFKit (Swift) ve pypdf 6.20 `strict=True`.

| # | Sonuç | Kanıt / not |
|---|---|---|
| S1 | **Doğrulandı, bir koşulla.** Rotate, DeleteLast, InsertBlank, AddAnnot, RemoveAnnot: 12 fixture (xref tablosu, artımlı tablo, xref stream + object stream ×2, xref stream zinciri, hybrid, 2 imzalı) üzerinde tüm durumlarda çıktı girdinin bayt-bayt devamı, `/Prev` doğru, değişiklik yeniden açınca görünüyor, imza sayısı korunuyor. Tablo ve hybrid çıktılar CG/PDFKit/pypdf'de geçerli. **Koşul:** xref stream girdilerinde PDFium kusuru (§2.3) her durumda tekrarlandı; ham çıktıyı CG, PDFKit ve pypdf reddediyor, `repair_incremental_xref_stream` kopyasıyla hepsi geçiyor. Onarım tüm artımlı PDFium yazımlarında zorunludur. | Ek gözlemler: PDFium artımlı bölüme değişmemiş ama yüklenmiş nesneleri de yazıyor (büyüme 400–1.300 bayt/işlem). Alt bölümü 0'dan başlamayan xref (ör. `2 6`) standarda uygun; pypdf bunun için yalnız "not zero-indexed" notu düşüyor, offset'ler doğrulandı. |
| S2 | **Doğrulandı.** RC4-128, AES-128, AES-256 (pypdf ile üretilmiş girdiler) üzerinde tüm işlemler: append-only, yeni nesneler şifreli (işaret metni artımlı bölümde düz metin olarak yok), parolayla açınca içerik doğru, parolasız açılmıyor; CG/PDFKit/pypdf geçerli. | Not: şifreli belgeler bugün tamamen salt okunur (`editable: !encrypted`, [document_session.rs:4196](../../src-tauri/src/document_session.rs)); bu sonuç ileride açılabilmeleri için ön koşuldur. |
| S3 | **Doğrulandı.** lopdf `IncrementalDocument` ile `/Kids` ters çevirme: 12 şifresiz girdinin hepsinde append-only, `/Prev` doğru, sıra doğru, xref türü korunuyor (stream girdilerinde `/Type /XRef` dahil doğru yazıyor), `/ID` ve `/Info` korunuyor; CG/PDFKit/pypdf geçerli. Ara `%PDF-1.4` satırı tüm çıktılarda var ve hiçbir okuyucu sorun çıkarmadı. Şifreli girdi atlandı (lopdf 0.34 şifreleyemez). | Uygulamada ara satır aynı uzunlukta bir yorumla değiştirilebilir (offset'ler değişmez); zorunlu değil. **S3b (iç içe ağaç ve miras):** Kök (`MediaBox` 600×800, `Rotate 90`, `Resources/F1`) → ara düğüm (`MediaBox` 300×400) → P1, P2 (`Rotate 0` override); P3 kendi `MediaBox`'ıyla. Düzleştirip ters çevirince her sayfanın metni (miras `Resources`), boyutu ve dönüşü PDFium'da beklenenle birebir; append-only, `/Prev` doğru, xref zinciri tutarlı; CG/PDFKit/pypdf geçerli. Fixture test içinde lopdf ile üretilir (xref stream'li). Uyarı: lopdf 0.34'te `Dictionary::extend` sözlüğü genişletmez, değiştirir; anahtarlar tek tek `set` edilmeli. |
| S4 | **Doğrulandı, kritik bulguyla.** Dedektör (lopdf yüklenir + her `Normal` xref girdisi `N G obj` başlığını gösterir) 12 iyi girdinin hepsini tutarlı, 5 bozuk girdinin (xref'siz, kaymış `startxref`, yanlış girdi offset'i, baştaki çöp, kesik son) hepsini tutarsız buldu. **Kritik:** PDFium bozuk girdilerin 4'ünü açıp "artımlı" kaydediyor; 3'ünde `/Prev`'siz, tüm nesneleri kapsayan yeni bir xref yazıyor (zinciri sessizce koparıyor), baştaki çöplü girdide ise prefix'i korumuyor. | Sonuç: §4.3 tek başına yetmez; dedektör girdi üzerinde **yazmadan önce** çalışmalı, tutarsız girdide §4.4'teki onarılmış-dosya kuralı uygulanmalı. |
| S5 | **Unix'te doğrulandı (a, c); Windows (b) test edilmedi.** Link sonrası `working.pdf` nlink=2 olur (kontrol bu anda çalışsa reddederdi), rename sonrası yeni dosya nlink=1, saklanan dosya eski inode ve içerikle nlink=1. Blob rename ile geri konabiliyor. | **Yeni gereksinim:** rename başarısız olursa link geride kalır ve `working.pdf` nlink=2 ile kalır; bir sonraki atomik yazım reddedilir. Hata yolunda link mutlaka silinmeli. |
| S6 | **Ölçüldü** (release: `opt-level = "z"` + LTO; test harness için yalnız bu çalıştırmada `--config 'profile.release.panic="unwind"'`). Girdiler: `scripts/generate-perf-fixtures.py` çıktıları (2,1 MB / 6,4 MB / 75 MB / 161 MB) ve 392 MB'lık 620 sayfa görsel. Tek döndürme için PDFium artımlı kayıt 0–66 ms, tam yeniden yazım 18–146 ms. **Büyüme:** PDFium döndürmesi 3,5 KB (metin) → 86 KB (161 MB) → 665 KB (392 MB); aynı döndürme lopdf ile 332–375 bayt (S6b). 392 MB'ta PDFium'un yazdıkları: katalog, sayfa ağacı düğümleri, font, sayfa, içerik stream'i ve 661 KB'lık görsel; değişen tek şey `/Rotate`'dı. Aynı etki AddAnnot'ta (161 MB: 86 KB) ve DeleteLast'te (1.000 sayfalık metin: 172 KB) görüldü; InsertBlank ~1,6 KB'ta sabit. **Undo maliyeti:** prefix kopya + fsync + SHA-256: 25 ms (2 MB), 665 ms (75 MB), 1,4 s (161 MB), 3,7 s (392 MB); sürenin ~%94'ü SHA-256 (~115 MB/s). | Sonuç: (1) PDFium artımlı kaydı "yalnız değişeni yaz" değildir, yüklenen nesneleri de yazar; yapısal düzenlemeler lopdf'e alındı (§4.1). (2) Undo büyük belgelerde saniyeler sürer; mutasyonlar gibi job olarak ve ilerleme göstergesiyle çalışmalıdır. (3) §5.5'teki 1 GiB bütçe 392 MB'lık belgede yalnız ~2 tam yeniden yazım adımı tutar; artımlı (Working-prefix) girdiler bütçe harcamaz. |
| S7 | **Kodla doğrulandı (statik).** Açılışta `working.pdf` kaynağın bayt-bayt kopyasıdır; baseline ve working digest'i aynı kopya digest'inden gelir ve kaynak ikinci kez hashlenerek tutarlılık doğrulanır (`copy_source_consistent`, [document_session.rs:11606](../../src-tauri/src/document_session.rs)); Save sonrası baseline commit digest'iyle güncellenir ([document_session.rs:4902](../../src-tauri/src/document_session.rs)). | Runtime testi yazılmadı. |

Tasarıma etkisi:

- §4.2: Onarım (`repair_incremental_xref_stream`) tüm PDFium artımlı yazımlarının zorunlu adımıdır.
- §4.3: Yazımdan **önce** girdiye S4 dedektörü uygulanır; yazımdan sonra 1–3 kontrolleri korunur.
- §5.3: `retain_replaced_target_as` hata yolunda oluşturduğu link'i silmelidir (S5).
- §4.1: Rotate ve Delete PDFium'dan lopdf'e alındı (S6, S6b); şifreli belgede lopdf yazamadığı için PDFium'a düşülür (S2) ve büyüme kabul edilir.
- §5.4: Undo/redo büyük belgede saniyeler sürdüğü için (S6) job altyapısıyla, iptal noktası digest hesabından önce olacak şekilde çalışır.

## 9. Açık kararlar

Kararlar K1–K4 için öneriler kullanıcı tarafından 2026-10-18'de kabul edildi. K5 için B
seçeneği aynı gün kabul edildi.

| # | Karar | Seçenekler | Öneri |
|---|---|---|---|
| K1 | Kayıttan sonra undo davranışı | K1-a prefix'e dön / K1-b geri koyan revizyon ekle (§5.7) | K1-a |
| K2 | İmzasız belgede artımlı yazım başarısız olursa | Sessizce tam yeniden yazım / kullanıcıya sor | Sessiz tam yeniden yazım, `writeMode` raporlanır (form doldurmadaki mevcut davranış) |
| K3 | Silinen içeriğin dosyada kalması | Bilgilendirme yok / ilk silmede bir kez uyarı / ayarlardan "her zaman tam yeniden yaz" | İlk silmede bir kez uyarı + sanitization'a yönlendirme |
| K4 | Undo adım limiti | 50 / 100 (pdfcraft) | 50, S6 ölçümüne göre güncellenir |
| K5 (yeni, S6) | Görselli sayfaya annotation eklerken PDFium'un sayfa görsellerini de yeniden yazması | **A** kabul et / **B** PDFium düzenlesin, lopdf yalnız değişen nesneleri artımlı yazsın / **C** annotation'ı ve görünüm akışını tamamen lopdf ile yaz | **Kabul edildi: B** (2026-10-18), §9.1'deki korumalar ve geçici boyut eşiğiyle |

### 9.1 K5 karşılaştırması

Deney: `k5_pdfium_edit_lopdf_minimal_write`
([incremental_undo_spike.rs](../../src-tauri/tests/incremental_undo_spike.rs)), 10 sayfa
taranmış görselli girdi (6,3 MB). Highlight, production'daki gibi sayfa render edilerek
PDFium'a ürettirilen görünüm akışıyla eklendi ([annotations.rs:352](../../src-tauri/src/annotations.rs)).
B için PDFium çıktısındaki her nesne girdideki sürümüyle lopdf üzerinden anlamsal olarak
karşılaştırıldı (sözlük eşitliği anahtar sırasından bağımsız; stream'lerde sözlük + ham içerik)
ve yalnız yeni/değişen nesneler `IncrementalDocument` ile yazıldı.

| İşlem | PDFium artımlı büyüme | B ile büyüme | Değişen / toplam nesne | Doğrulama |
|---|---|---|---|---|
| Highlight (+AP) | 665.139 bayt | 857 bayt | 3 / 38 (sayfa, annotation, AP stream) | Append-only, `/Prev` doğru, xref zinciri tutarlı, sayfa render'ı PDFium çıktısıyla piksel piksel aynı; CG/PDFKit/pypdf geçerli |
| Rotate | 664.614 bayt | 354 bayt | 1 / 36 | Aynı |

Bayt düzeyinde karşılaştırma işe yaramaz: PDFium yeniden yazdığı her nesnede anahtar sırasını
değiştiriyor ve sayıları yeniden biçimliyor (`841.89` → `841.89001`); stream verileri ise
birebir aynı kalıyor.

| | A: Kabul et | B: PDFium düzenler, lopdf fark yazar | C: lopdf ile annotation + AP |
|---|---|---|---|
| Büyüme | Sayfanın yüklediği tüm nesneler (taranmış sayfada görsel boyutu) | Yalnız değişen nesneler | Yalnız değişen nesneler |
| Kapsam | — | Tüm PDFium mutasyonları (annotation, boş sayfa, ileride filigran/page object düzenlemeleri) | Yalnız annotation |
| Görünüm akışı | PDFium | PDFium (kanonik kalır) | Bizim (pdfcraft `annot` portu) |
| Şifreli belge | Çalışır | Ham PDFium çıktısına düşer (lopdf şifreleyemez) | Ham PDFium'a düşer |
| Ana risk | Dosya şişmesi; 50 annotation × 2 MB tarama ≈ +100 MB | Anlamsal fark hatası bir düzenlemeyi kaybedebilir; büyük belgede lopdf'in tüm belgeyi iki kez yüklemesinin maliyeti ölçülmedi | AP doğruluğu bizim sorumluluğumuz; iş yükü en büyük |
| İş | Yok | Orta (fark + doğrulama + geri dönüş) | Büyük |

**Karar: B** (kabul edildi), şu korumalarla:

1. Bir nesne yalnız lopdf düzeyinde tam eşitse atlanır; şüphede yazılır. Yanlış yönde hata
   yalnız fazla bayt demektir.
2. Kısaltılmış çıktı commit'ten önce: §4.3 kontrolleri + S4 zincir dedektörü + PDFium yeniden
   açma + recipe durumu + etkilenen sayfaların render hash'inin ham PDFium çıktısıyla aynı olması.
3. Herhangi bir kontrol başarısızsa ham PDFium çıktısı kullanılır (büyüme kabul edilir,
   düzenleme asla kaybolmaz) ve `writeMode = appended-unminimized` raporlanır.
4. Şifreli belge ve lopdf'in açamadığı girdi doğrudan ham PDFium yoluna gider.

B benimsenirse §4.1'de Rotate ve Delete için ayrıca lopdf kodu yazmaya gerek kalmaz: düzenlemeyi
PDFium yapar, lopdf yalnız farkı yazar. `ReorderPages` lopdf'te kalır. C, PDFium'un görünüm akışı
üretemediği annotation türleri için (FreeText, ok uçlu Line, Stamp vb.) Faz 1'de ayrıca
değerlendirilir.

#### B'nin büyük belgelerdeki maliyeti (ölçüldü)

Test: `k5b_minimize_cost`. Release (`opt-level = "z"` + LTO, test için
`--config 'profile.release.panic="unwind"'`), her dosya ayrı süreçte, tepe bellek
`/usr/bin/time -l` "peak memory footprint" (macOS'ta RSS değil footprint esas alınır).
İşlem: ilk sayfayı döndür → PDFium artımlı kayıt → B ile fark yazımı → render doğrulaması.
Makine: 8 GiB RAM.

| Girdi | Boyut | Nesne | lopdf yükleme (girdi / PDFium çıktısı) | Fark + yazım | Render doğrulama | Tepe bellek: yalnız PDFium → B | Büyüme: PDFium → B |
|---|---|---|---|---|---|---|---|
| text-1000 | 2,1 MB | 2.015 | 5 / 5 ms | 12 ms | 25 ms | 10 → 29 MB | 3.553 → 337 B |
| text-3000 | 6,4 MB | 6.035 | 25 / 26 ms | 51 ms | 31 ms | 19 → 59 MB | 3.723 → 337 B |
| mixed-1000 | 75 MB | 2.091 | 8 / 9 ms | 39 ms | 23 ms | 149 → 462 MB | 3.484 → 332 B |
| heavy-400 | 161 MB | 1.028 | 9 / 9 ms | 59 ms | 132 ms | 314 → 946 MB | 86.045 → 375 B |
| image-620 | 392 MB | 1.872 | 26 / 31 ms | 648 ms | 54 ms | 793 → 2.438 MB | 665.472 → 364 B |

Tüm satırlarda render eşit ve değişen nesne sayısı 1 (yalnız sayfa sözlüğü).

Sonuç:

- **Süre sorun değil.** 392 MB'ta bile toplam ek süre ~0,7 s.
- **Bellek sorun.** Spike'taki basit uygulama tepe belleği belge boyutunun ~4 katı artırıyor
  (+313 MB / 75 MB, +632 MB / 161 MB, +1.645 MB / 392 MB). Sebep: girdi ve PDFium çıktısı
  ayrı ayrı tam olarak lopdf'e yükleniyor ve stream içerikleri her yüklemede kopyalanıyor.
  macOS'ta worker belleğine üst sınır uygulanmadığı da bilinen bir sorun.
- **Production için zorunlu iyileştirme:** Önceki revizyon yalnız bir kez yüklenir. PDFium
  çıktısının yalnız **artımlı bölümündeki** nesneler ayrıştırılıp karşılaştırılır (bunlar
  PDFium'un son xref bölümünden bulunur). Ham çıktının tamamı lopdf'e yüklenmez. Beklenen ek
  bellek ≈ 1× belge + artımlı bölüm; uygulamada ölçülerek doğrulanmalı.
- **İyileştirme ölçülene kadar geçici kural:** B yalnız ≤ 64 MiB girdilerde uygulanır (ölçülen
  ek bellek ≈ +250 MB'a kadar). Daha büyük girdiler ham PDFium artımlı çıktısını kullanır ve
  `writeMode = appended-unminimized` raporlanır. Eşik, iyileştirilmiş uygulamanın ölçümüyle
  güncellenir.

Not: PDFium ürettiği AP'yi `/PDFIUM_HasGeneratedAP true` özel anahtarıyla işaretliyor; bu bugünkü
production çıktısında da var ve B'den bağımsızdır.

## 10. Uygulama sırası

Her adım ayrı commit'tir ve kendi testleriyle gelir:

1. Spike'lar S1–S7 (production kodu değişmez; bulgular bu dokümana işlenir).
2. Artımlı kayıt yardımcılarını `forms/fill.rs`'den paylaşılan modüle taşı (davranış aynı). **Yapıldı.**
3. Append-only doğrulayıcısı (§4.3), xref tutarlılık dedektörü (S4) ve `WriteMode` türü.
   **Yapıldı** (saf fonksiyonlar ve testleri; recipe'lere bağlanmadı). `writeMode`'un mutasyon
   sonucunda raporlanması adım 4'e kaldı.
4. PDFium recipe'lerini artımlı yaz (rotate, delete, insert, annotation); imza uyarısını
   backend'in bildirdiği moda bağla.
5. `persisted_working_digest` ile `dirty` hesabı (§5.6).
6. Geçmiş modeli, commit'te geçmişe yazma ve sınırlar (§5.2–5.5).
7. Undo/redo komutları ve frontend (§5.4, §5.10).
8. `ReorderPages` için lopdf artımlı yolu (S3 sonucuna bağlı).
9. `document-session.md` ve `feature-matrix.md` güncellemesi.

## 11. Test planı

- **Birim:** Append-only doğrulayıcısı (prefix uyuşmazlığı, `/Prev` hatası, kısa çıktı);
  `BlobRef::Working` yeniden bağlama; limit ve eviction; `dirty` hesabı (kaydet → düzenle →
  undo ⇒ temiz).
- **Entegrasyon (gerçek PDFium):** Her recipe için artımlı çıktı + PDFium yeniden açma;
  imzalı fixture'da `/ByteRange` baytlarının değişmediği; undo sonrası digest'in tam olarak
  önceki durumla eşleşmesi; undo → yeni mutasyon ⇒ redo temizlenir.
- **Hata yolları:** Geçmiş dizinine yazılamazsa mutasyon başarılı + `HISTORY_TRUNCATED`;
  bozulmuş blob ⇒ undo reddedilir ve `working.pdf` değişmez; undo sırasında
  `RecoveryRequired`/`Indeterminate` eşlemesi.
- **Eşzamanlılık:** Undo, devam eden snapshot job'ı ve Save ile aynı lease kuralına uyar;
  eski generation job çıktısı uygulanmaz.
- **Platform:** Windows'ta hard link yedeğinin geçmişe taşınması ve DACL; macOS
  Preview/PDFKit ile artımlı çıktıların açılması.
