# PdfPage.svelte — Svelte 5 runes dönüşüm planı (Prompt 14, aşama 3)

Durum: **uygulandı (2026-10-14).** Çapraz doğrulama: Sonnet 5.5, "APPROVE WITH CHANGES"; düzeltmeler
işlendi; kullanıcı onayladı. Bölüm 6 testleri `PdfPage.test.js` → "PdfPage effect ordering"; eski (legacy) PdfPage
üzerinde de geçiyor, E5 post-effect ve E3/E2 sıra mutasyonlarını yakalıyor.
Kaynak: çalışma ağacındaki `src/components/PdfPage.svelte` (commit edilmemiş `isZooming` WIP'i dahil).

## 0. Ön koşul

`PdfPage.svelte`, `PdfPage.test.js` ve `ViewerShell.svelte` commit edilmemiş zoom-bellek WIP'i (`isZooming` prop'u,
render bloğundaki `!isZooming` koşulu) içeriyor. Dönüşüm bu satırlara dokunmak zorunda; WIP ayrı commit edilmeden
aşama 3 ayrı bir commit olarak izole edilemez.

## 1. Envanter (doğrulandı)

- `export let`: **23** (audit 22 diyor; fark `isZooming` WIP'i).
- `$:` bloğu: **21** (audit 18 diyor). Liste aşağıda.
- `createEventDispatcher`: 1; olaylar `searchHighlightReady` (ViewerShell dinliyor) ve `sessionSnapshot`
  (aşama 2'de AnnotationLayer'dan yeniden dispatch ediliyor).
- `on:` direktifleri: `pointerdown/move/up/cancel` (redaksiyon katmanı), `mouseup` (textLayer), `click` (retry).
- `<script context="module">` (runes modunda deprecated → `<script module>`).
- Hiçbir prop bileşen içinde yeniden atanmıyor (grep ile doğrulandı) → `$bindable` gerekmez.
- `isBookMode`: ViewerShell geçiriyor, PdfPage tanımlamıyor ve kullanmıyor (doğrulandı).
- `fetchTextLayout` (satır ~397) `getPageTextLayout(sessionId, …)` çağırıyor; snapshot'taki `session` değil
  (doğrulandı). Bugün zararsız: çağrı senkron olarak `queueTextLayoutRequest` içinden yapılıyor ve argüman ilk
  `await`'ten önce değerlendiriliyor, yani o anda `sessionId === session`. Yine de kırılgan; `session` kullanılacak.

## 2. Reaktif blokların sınıflandırılması

### 2a. `$derived` (yan etkisiz, yalnız değer) — 15 blok

| # | Satır | Değişken | Bağımlılıklar |
|---|---|---|---|
| D1 | 152 | `normalizedRotation` | `rotation` |
| D2 | 153 | `unscaledWidth` | `lastRenderedScale`, `baseImgWidth` |
| D3 | 154 | `unscaledHeight` | `lastRenderedScale`, `baseImgHeight` |
| D4 | 155 | `desiredRenderKey` | `sessionId`, `scale`, `documentGenerationId`, `pageIndex`, D1, `colorMode`, `generationId` |
| D5 | 158 | `desiredTextLayoutKey` | `sessionId`, `documentGenerationId`, `pageIndex` |
| D6 | 161 | `documentKey` | `sessionId`, `documentGenerationId` |
| D7 | 178 | `shouldRender` | `isSinglePageMode`, `pageIndex`, `currentPage`, `isBufferIntersecting` |
| D8 | 203 | `textLayoutItems` | `paintedRenderKey`, `paintedSessionId`, `sessionId`, `paintedPageIndex`, `pageIndex`, `loadedTextLayoutKey`, D5, `renderedCssWidth/Height`, `textLayouts`, `paintedPageRotation` |
| D9 | 218 | `positionedSearchMatches` | `paintedRenderKey`, `renderedCssWidth/Height`, `searchMatches`, `paintedPageRotation` |
| D10 | 241 | `positionedPendingRedactions` | `pendingRedactionRegions`, D1 |
| D11 | 250 | `redactionPreviewRect` | `redactionDragStart`, `redactionDragCurrent` |
| D12 | 275 | `isTextLayerInteractive` | `activeTool` |
| D13 | 285 | `estimatedPageCssSize` | `pageSizePt`, D6, `scale`, D1 (+ reaktif olmayan `pageSizeFallback`, `devicePixelRatio` — legacy ile aynı) |
| D14 | 291 | `cachedHeight` | `pageSizePt`, D13, `renderedCssHeight`, `lastRenderedScale`, `paintedDocumentKey`, D6, `paintedPageRotation`, D1, `scale` |
| D15 | 705 | `pageHighlights` | `$annotationsStore`, `sessionId`, `pageIndex` |

Not: D8/D9/D14 için `let x = …` başlangıç değerleri ölü (her zaman `$:` ile üzerine yazılıyor); `$derived` ile kalkar.
`cachedHeight` başka yerde yazılmıyor, yalnız `fetchPageImage` içinde okunuyor (`heightBeforePaint`) — derived okunabilir.

### 2b. Yan etki — 6 blok → `$effect.pre`

| # | Satır | İş | İzlenecek bağımlılıklar | Yazdıkları |
|---|---|---|---|---|
| E1 | 175 | `requestPageSize(...)` | D6, `sessionId`, `documentGenerationId`, `pageIndex` | `pageSizePt` (+ düz `pageSizeRequestKey`) |
| E2 | 182 | metin katmanı reset | D5 | `pendingTextLayoutKey`, `loadedTextLayoutKey`, `failedTextLayoutKey`, `textLayouts` |
| E3 | 187 | metin katmanı isteği | D7, D5, `$pageFailureRegistry`, kimlik, `loaded/pending/failedTextLayoutKey` | `pendingTextLayoutKey`, `failedTextLayoutKey` |
| E4 | 259 | highlight-ready bildirimi | D9, `paintedRenderKey`, `searchRequestId`, `pageIndex`, D7 | düz `lastHighlightReadyKey`, `highlightReadySequence` |
| E5 | 278 | `cleanupMemory()` | D7 | `painted*`, `pendingRenderKey`, `failedRenderKey`, metin anahtarları, canvas boyutu |
| E6 | 304 | sayfa render kuyruğu | D7, `isZooming`, D4, `$pageFailureRegistry`, kimlik, `painted/pending/failedRenderKey` | `cssScale`, `pendingRenderKey`, `failedRenderKey`, `renderErrorMsg` |

**Neden `$effect.pre`, `$effect` değil:** legacy `$:` DOM güncellemesinden önce çalışır. E5 özellikle önemli:
`shouldRender` false olunca `{#if shouldRender}` canvas'ı kaldırmadan önce `canvasRef.width = height = 1` ile
GPU/bitmap belleği hemen bırakılıyor. Post-effect'te canvas DOM'dan çıkmış ve `bind:this` null'lanmış olur; bitmap
GC'ye kalır (WebContent bellek araştırmasıyla doğrudan ilgili).

**Kalıp (legacy semantiğiyle birebir):** legacy `$:` derlemesi bağımlılıkları izler, gövdeyi `untrack` içinde çalıştırır
(`legacy_pre_effect`). Aynısı:

```js
$effect.pre(() => {
  if (/* koşul: yalnız izlenmesi gereken değerleri okur */) {
    untrack(() => queuePageRender(/* ... */));
  }
});
```

Koşul ifadeleri yan etkisiz (yalnız `pageFailure()` registry'den okur). Kısa devre `&&` yüzünden runes yalnız fiilen
okunan değerleri izler; koşulun değeri ancak okunan bir değer değişirse değişebileceği için sonuç aynı.
Gövdeler (`queuePageRender`, `queueTextLayoutRequest`, `cleanupMemory`, `resetTextLayoutForDocument`,
`requestPageSize`, `notifySearchHighlightReady`) mutlaka `untrack` içinde — aksi halde fonksiyon içinde okunan her
state (ör. `canvasRef`, `zoomDebounceTimer`) bağımlılığa dönüşür. Neyin dışarıda/içeride olduğu:

- E1: dört argüman (`documentKey`, `sessionId`, `documentGenerationId`, `pageIndex`) `untrack` **dışında** okunur,
  sonra içeri geçirilir. `untrack(() => requestPageSize(documentKey, …))` yazılırsa argümanlar izlenmez ve E1 sayfa/belge
  değişiminde hiç yeniden çalışmaz.
- E4: `activeMatch` / `readyKey` hesabı ve koşul izlenir; yalnız düz değişken yazımları ve
  `void notifySearchHighlightReady(...)` `untrack` içinde.
- E6: `if` koşulu dışarıda; `shouldDebounce`, `cssScale = …` ve `queuePageRender` tamamı `untrack` içinde
  (`lastRenderedScale`/`scale` okumaları ek izleme yaratmasın).

Mekanizma notu: legacy `legacy_pre_effect` gövdeyi `active_effect` ebeveyne ayarlıyken çalıştırır; runes'ta etki
kendisi aktif kalır, bu yüzden izlenen bir değere kendi yazımı (`untracked_writes` /
`schedule_possible_effect_self_invalidation`) **bir** ek no-op tur üretir. Sonuç aynı. Legacy ayrıca ifadedeki tüm
sözcüksel referansları kısa devre olmadan izler; koşul saf olduğu için eşdeğerlik etkilenmez.

**Yerleşim (TDZ):** `$effect.pre` oluşturulduğu anda senkron çalışır; E1 ve E5 ilk turda gerçek iş yapar (E5
sürekli modda init'te `cleanupMemory()` çağırır). Legacy tüm `$:`'leri script gövdesinden sonra yayınlıyordu. Bu
yüzden tüm `$props()`, `$state`, `$derived`, `renderCoordinator` ve düz `let`'ler (`visibilityObserver`,
`bufferObserver` dahil — şu an satır 628-629'da) ilk `$effect.pre`'den **önce** bildirilecek; altı etki tek blokta,
E1…E6 sırasıyla, ardından `onMount`/`onDestroy`. Fonksiyon bildirimleri hoist edildiği için yerinde kalabilir.

**AGENTS.md notu:** kural listesi `$effect` diyor; burada `$effect.pre` bilinçli seçim (canvas'ın DOM'dan kalkmadan
küçültülmesi için pre-DOM zamanlama gerekli).

### 2c. State sınıflandırması

- `canvasRef` ve `searchHighlightLayerRef` **`$state` olmak zorunda**: `{#if}` içinde `bind:this` ile bağlanan düz
  `let`, `non_reactive_update` uyarısı üretir (+2 svelte-check uyarısı). Reaktif olarak okunmuyorlar; yalnız `untrack`
  içinde veya async kodda okunacaklar. `pageRef` en üst div'de, düz `let` olabilir (uyarı yok).
- `$state`: template/derived/izlenen koşullarda okunanlar —
  `lastRenderedScale`, `renderErrorMsg`, `paintedRenderKey`, `paintedSessionId`, `paintedPageIndex`,
  `paintedPageRotation`, `pendingRenderKey`, `failedRenderKey`, `baseImgWidth/Height`, `renderedCssWidth/Height`,
  `cssScale`, `loadedTextLayoutKey`, `pendingTextLayoutKey`, `failedTextLayoutKey`, `isBufferIntersecting`,
  `paintedDocumentKey`, `placeholderHeightDelta`.
- `$state.raw` (derin proxy istenmiyor; dizi/nesne her seferinde yeniden atanıyor): `textLayouts` (büyük layout
  dizisi — proxy maliyeti ve kimlik değişimi riski), `pageSizePt`, `redactionDragStart`, `redactionDragCurrent`.
- Düz `let` (yalnız defter tutma, hiçbir template/derived/izlenen koşulda okunmuyor): `renderRequestId`,
  `textLayoutRequestId`, `zoomDebounceTimer`, `isDestroyed`, `retryPending`, `observedTextLayoutKey`,
  `lastHighlightReadyKey`, `highlightReadySequence`, `pageSizeRequestKey`, `redactionPointerId`,
  `visibilityObserver`, `bufferObserver`, `isVisible`, `isRendering`, `observer` (son üçü yazılıyor ama okunmuyor;
  davranış değişikliği olmasın diye silinmiyor).

## 3. Render anahtarı (desired / painted / pending / failed) — sıralama ve döngü analizi

Koşul (E6): `shouldRender && !isZooming && desired && !failure && desired !== painted && desired !== pending && desired !== failed`.

| Olay | Yazılan | E6 yeniden çalışır mı | Sonuç |
|---|---|---|---|
| E6 kuyruğa alır | `pending = desired`, `failed = ""`, `cssScale` | Evet (izlediği `pending` değişti) | `desired === pending` → no-op. 1 ek tur, döngü yok. |
| Başarılı paint | `painted = key`, finally `pending = ""` | Evet | `desired === painted` → no-op. |
| Sonuç bayat (desired değişti) | finally `pending` yalnız `=== key` ise temizlenir | Yeni anahtar zaten pending | no-op. |
| Hata | `failed = key`, registry kaydı | Evet (`failed`, `$pageFailureRegistry`) | `desired === failed` / failure → no-op; retry `failed=""` ve registry'yi temizleyince tek render. |
| `cleanupMemory` (E5) | `painted = pending = failed = ""` | Evet | `shouldRender` false → no-op. |
| `isZooming` true→false | — | Evet | Commit edilen ölçekle tek render. |
| `!canvasRef` ile erken dönüş | finally `pending = ""` | Evet | **Yeniden kuyruğa alır.** Bu legacy'de de aynı (pending legacy'de de bağımlılık); async IPC üzerinden döner, senkron döngü değil. Davranış korunur, düzeltilmez (kapsam dışı) — açık risk olarak raporlanır. |

Senkron döngü üst sınırı: her etki en fazla 2 tur (yazdığı izlenen değeri ikinci turda eşit görür). Svelte'in
`effect_update_depth_exceeded` eşiğine (≈1000) yaklaşılmaz.

**Etkiler arası sıra.** Runes'ta aynı flush'taki `$effect.pre`'ler bildirim sırasıyla çalışır; legacy derleyici `$:`
bloklarını atama→bağımlılık ilişkisine göre topolojik sıralar. Etkiler arası yazma/okuma ilişkileri:

- E2 → E3 (`pending/loaded/failedTextLayoutKey`): E2 önce bildirilmeli. Ters sırada sayfa/oturum değişiminde E3 istek
  atar, E2 onu iptal eder, E3 tekrar atar → **çift IPC**. Kaynak sırası zaten E2, E3; korunacak ve test edilecek.
- E5 → E3, E6 (anahtarları temizler): E5 E6'dan önce. E3 E5'ten önce bildirilmiş ama E5 yalnız `shouldRender` false
  iken çalışır ve o durumda E3 de no-op; legacy'deki sıra da aynı (E5 sözcüksel atama yapmıyor, topolojik sıralama
  değişmiyor).
- E1 → D13/D14 (`pageSizePt`): derived'lar pull tabanlı, sıra sorunu yok.

**Bayatlık kontrolü farkı.** `fetchPageImage`/`fetchTextLayout` içindeki `key === desiredRenderKey` gibi kontroller
runes'ta `$derived`'ı okur; prop değişip flush olmadan önce bile güncel değeri görür (legacy bir önceki flush'ın
değerini görürdü). Sonuç: bayat yanıtlar en kötü ihtimalle daha erken reddedilir; yanlış kabul olasılığı artmaz.

## 4. Olaylar ve prop API değişimi

- `dispatch("searchHighlightReady", detail)` → `onsearchHighlightReady?.(detail)`. ViewerShell:
  `onsearchHighlightReady={(detail) => onSearchHighlightReady({ detail })}` (`featureRuntime.handleSearchHighlight`
  `event.detail` bekliyor; App/service katmanı değiştirilmiyor — aşama 2'deki WorkspacePanels kalıbıyla aynı).
- `sessionSnapshot` → `onsessionSnapshot` prop'u AnnotationLayer'a doğrudan geçirilir. ViewerShell:
  `onsessionSnapshot={onSessionSnapshot}` (zaten `event.detail` açılıyordu).
- `PdfPage.test.js`: `$$events: { searchHighlightReady: ready }` → `onsearchHighlightReady: ready`; `ready`
  çağrısındaki argüman artık `event.detail` değil doğrudan detail (ilgili assertion'lar buna göre).
- `on:pointer*`, `on:mouseup`, `on:click` → `onpointer*`, `onmouseup`, `onclick`. `pointerdown/move/up`, `mouseup`,
  `click` kökte delegate edilir; `pointercancel` delegate edilmez (doğrudan dinleyici, eskisi gibi). Risk incelendi:
  eleman ile `#app` arasında delegate edilmeyen dinleyici yok; yalnız `document` düzeyinde
  `click` (main.js) ve `mouseup/mousedown` (AiContextMenu) var. Delegasyon kökü `#app` olduğundan `document`
  dinleyicileri yine sonra çalışır; `stopPropagation` hâlâ `document`'a ulaşmayı keser. `event.currentTarget`
  delegasyonda Svelte tarafından doğru ayarlanır.
- textLayer üzerindeki `<!-- svelte-ignore a11y-no-static-element-interactions -->` (satır 875) runes modunda
  tanınmaz (`legacy_code` + bastırılmamış a11y uyarısı = +2). `a11y_no_static_element_interactions` olarak yazılacak.
- Prop fallback farkı (doğrulandı, gözlemlenebilir etkisi yok): runes'ta tanımlıyken `undefined` olan prop varsayılana
  döner, legacy'de `undefined` kalırdı. Yalnız `secureRedactionSelectionMode` ve `onRedactionRegionSelected` bunu
  yaşayabilir; iki durumda da sonuç falsy/fonksiyon-değil.
- `isBookMode`: ViewerShell'den **kaldırılacak** (PdfPage'te hiç kullanılmıyor; kitap modu düzeni ViewerShell'in
  `book-spread-row` sınıfında). Kullanmaya başlamak davranış değişikliği olurdu.

## 5. Uygulama adımları (onaydan sonra)

1. `<script context="module">` → `<script module>`.
2. 23 `export let` → tek `$props()` destructuring (+ `onsearchHighlightReady`, `onsessionSnapshot`).
3. State'ler 2c'ye göre; 15 derived; 6 `$effect.pre` (bildirim sırası E1…E6 korunur), gövdeler `untrack`.
4. `fetchTextLayout`: `getPageTextLayout(session, …)` — saf refactor (bugün iki değer aynı tick'te eşit), test yok.
5. Olay/direktif dönüşümleri, ViewerShell güncellemesi, `isBookMode` kaldırma, `PdfPage.test.js` güncellemesi.
6. Doğrulama: vitest tam paket; `svelte-check` 0 hata / 13 uyarı tabanı ve **PdfPage.svelte'de yeni uyarı yok**;
   PdfPage kaynak testleri.

## 6. Eklenecek doğrudan testler (davranış kilitleme)

- Sayfa/oturum değişiminde metin katmanı için tek `getPageTextLayout` çağrısı (E2→E3 sırası, çift IPC yok).
- Kararlı durumda `renderPage` tek çağrı; aynı props ile rerender yeni çağrı üretmez (E6 döngü yok).
- `shouldRender` false olunca canvas DOM'dan kalkmadan önce 1×1'e küçültülür (E5 pre-timing): spy'lanmış
  `HTMLCanvasElement.prototype.width` setter'ında `canvas.isConnected` kaydedilir, 1'e set edilirken `true` olmalı.
- `isZooming` true iken render yok; false olunca tek render.
- Çift render yolunda `console.warn` ve ikinci `renderPage` çağrısı yok.
- Aynı aktif eşleşme için `onsearchHighlightReady` bir kez çağrılır (mevcut `$$events` testi
  `onsearchHighlightReady: ready` + `ready.mock.calls[0][0]` olarak güncellenir).

## 7. Açık riskler

- `!canvasRef` erken dönüşündeki yeniden kuyruğa alma (legacy'den devralınan; değiştirilmiyor). Gözden geçiren:
  pratikte erişilemez, çünkü canvas bloğu kalktığında `requestIsCurrent()` zaten false.
- E2→E3 sırasını kaynak sırasından başka bir şey zorlamıyor; etki bloğuna bunu belirten yorum eklenecek.
- `isVisible`, `isRendering`, `observer` yazılıp okunmuyor (ölü kod) — ayrı iş olarak önerilir.
- `$state.raw` seçilen alanlarda yerinde mutasyon yapılırsa reaktivite kaybolur — mevcut kod hep yeniden atıyor
  (grep ile doğrulanacak).
- Testler jsdom'da; gerçek WebKit'te pinch-zoom/bellek davranışı ayrıca elle doğrulanmalı (zoom stress build).
