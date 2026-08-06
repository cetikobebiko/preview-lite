# Redaksiyon Yetenek Sınırı ve Hassas Veri Tespiti Mimarisi

## 1. Genel Bakış ve Kavramsal Sınırlar

Antigravity PDF Reader belgelerdeki hassas içeriklerin yönetimi için katı güvenlik ve yetenek sınırları uygular. Karışıklığı ve güvenlik ihlallerini önlemek adına aşağıdaki kavramlar birbirinden kesin çizgilerle ayrılmıştır:

| Kavram | Tanım | Yetki / Sorumlu Katman | PDF Değişikliği (Mutation) |
|---|---|---|---|
| **Detection (Tespit)** | PDF metin içeriğinden regex kalıpları ile hassas veri alanlarının tespiti ve raporlanması. | Rust / PDFium backend & session-job izolasyonu. | **HAYIR** (Salt okuma) |
| **Review Highlight (İnceleme Vurgusu)** | Taranan `resolved` ögeler için viewer arayüzünde gösterilen geçici, salt-görsel amber renkli vurgulama katmanı. | Svelte 5 frontend state & pure rotation selector. | **HAYIR** (Geçici UI) |
| **Visual Mask (Görsel Maske)** | Siyah dikdörtgen veya kapatıcı çizimlerle görsel gizleme. *(Mevcut sürümde kanonik özellik değildir; kalıcı güvenlik sağlamaz).* | UI gösterimi. | **HAYIR** (İçeriği silmez) |
| **Secure Redaction (Güvenli Redaksiyon)** | PDF nesne ağacından ve metin akışlarından hedeflenen verilerin atomik biçimde kazınması/silinmesi. | Rust / PDFium atomic document I/O. | **EVET** (Kalıcı silme) |
| **Sanitization (Dezenfeksiyon)** | Belgedeki tüm gizli metadata, gömülü dosyalar, ek açıklamalar veya biçimlendirme artıklarının temizlenmesi. | Rust / PDFium atomic document I/O. | **EVET** (Kalıcı temizlik) |

> [!WARNING]
> Eski siyah maske uygulamaları güvenli redaksiyon sağlamaz. Antigravity mimarisinde görsel maskeleme redaksiyon yerine KULLANILMAZ. Hassas veri tespiti sonuçları belge nesne ağacını değiştirmeyen salt-görsel inceleme vurgularıdır (`review-highlight-layer`).

---

## 2. State Ownership & Controller Lifecycle

Prompt 7 mimarisine uygun olarak, hassas veri tespiti yaşam döngüsü aşağıdaki sınırlara sadıktır:

### 2.1 Sorumluluk Dağılımı

- **Store (`sensitiveDataScanState.svelte.js`)**: Factory tabanlıdır (`createSensitiveDataScanState`). Yalnızca UI sunum state'ini (`isScanning`, `report`, `error`, `capturedPatternLabels`, `logicalHighlightsByPage`) taşır. İstek ID, job ID, cancel handle veya oturum sahipliği barındırmaz.
- **Controller (`sensitiveDataScanController.js`)**: `createSensitiveDataScanController({ state, startScan, getContext })`. Asenkron iş sahipliğini, private `requestToken` sayacını, aktif `cancelHandle` referansını ve `capturedContext` bilgilerini yönetir.
- **Actions Katmanı (`actions.js`)**: Senkron `assertFeatureRunnable` ile özellik erişimini doğrular ve `startSensitiveDataScanAction` üzerinden handle'ı (`{ jobId, promise, cancel }`) dönüştürmeden iletir (`scanSensitiveDataAction` tek bir policy zinciridir).
- **Façade Katmanı (`featureRuntime.js`)**: Raw controller'ı dışarı açmaz; `scanSensitiveData`, `reconcileSensitiveDataContext` ve `getSensitiveDataHighlightsForPage` façade metotlarını sunar.

### 2.2 Reconcile ve Cancellation Guard'ları

Asenkron tarama işleri aşağıdaki durumlarda otomatik ve best-effort biçimde iptal edilir (stale request token increment + cancel handle execution):
1. Yeni bir tarama başlatıldığında.
2. Hassas veri paneli kapatıldığında.
3. Aktif belge oturumu (`sessionId`) değiştiğinde.
4. Belge içerik nesli (`contentGeneration`) güncellendiğinde.
5. Belge PDF'den görsele (image) geçtiğinde.
6. Bileşen / Controller `destroy()` edildiğinde.

---

## 3. Geometri Dönüşüm Zinciri ve Saf Selector Yapısı

Hassas veri tespiti görsel vurguları persistent search worker önbelleğini kullanmaz. Bağımsız kanonikometri zinciri izlenir:

```text
sourceRectsPdfPoints (Raw PDF point rects)
  ↓ (effectiveBox ile pdfPointRectToNormalizedTopLeft)
Logical Highlight Rect (Rotasyondan bağımsız normalized_page koordinatı [x, y, w, h] + intrinsicRotation)
  ↓ (Store: logicalHighlightsByPage)
Viewer Rotation Change Event
  ↓ (Pure Selector: selectViewportHighlights)
Viewport-Normalized Review Rect (Intrinsic + Viewer rotation birleşik rotasyon matrisinden geçmiş [x, y, w, h])
  ↓ (ViewerShell → PdfPage prop aktarımı)
<div class="review-highlight-layer"><div class="review-highlight-rect"></div></div>
```

- Raw PDF puanları ve `PageGeometry` nesneleri `PdfPage` veya `ViewerShell` bileşenlerine geçirilmez.
- `.review-highlight-layer` doğrudan CSS rotasyonu uygulamaz; koordinatlar selector seviyesinde döndürülmüştür.
- Katman `aria-hidden="true"` ve `pointer-events: none` özellikleriyle etkileşimsiz render edilir.

---

## 4. Bilinen Teknik Borç (Known Technical Debt)

> [!NOTE]
> Intrinsic rotation içeren sayfaların persistent search worker kutularındaki rotasyon uyumsuzluğu bilinen bir teknik borçtur ve ileriki sürümlerde ele alınacaktır. Prompt 8A-5 kapsamında hassas veri tespiti (sensitive-data scan) bu sorundan etkilenmez; çünkü arama worker geometri önbelleğini kullanmak yerine saf `buildLogicalHighlightsByPage` ve `selectViewportHighlights` dönüşüm yolunu kullanır.
