# Frontend Architecture

Bu belge Svelte 5 frontend shell'inin state, async request ve lifecycle sahipliklerini tanımlar. PDF işlemlerinin tek motoru Rust/PDFium katmanıdır; frontend yalnız UI state, canvas boyama, DOM katmanları, lazy rendering ve stale-result koruması taşır.

## Root composition

`App.svelte` yalnız root store/context oluşturur, bounded runtime'ları compose eder, shell bileşenlerine typed props geçirir ve senkron bir mount cleanup kaydeder.

Bağımlılık yönü:

1. Root store/context'ler oluşturulur.
2. `viewerRuntime` tek `viewerController` çevresinde kurulur.
3. `documentRuntime`, viewer controller'a document open/save/transition işlemlerini bağlar.
4. `featureRuntime`, document ve viewer runtime API'lerini feature action'larına bağlar.
5. `applicationRuntime`, startup, keyboard, window close listener ve mount/unmount sırasını koordine eder.

App doğrudan backend, feature action, config manager, Tauri dialog/window API veya command switch kullanmaz.

## Authoritative reactive state

| Domain | Tek reactive owner | Runtime/controller'ın taşıyabileceği private state |
| --- | --- | --- |
| Viewer presentation | `viewerState` | Render request ID, RAF, debounce timer, resize observer, activation promise queue |
| Search | `searchState` | Debounce coordinator ve request ownership |
| Prepress | `prepressState` | Async request identity |
| Annotation | `annotationState` | Pointer interaction lifecycle |
| Modal, panel, toast ve error | `uiState` | Toast timer ve doğrulanmış presentation forwarding |
| Sensitive Data Scan | `sensitiveDataScanState` | Request token, canonical job ID, active cancel handle, captured context, mounted state |
| Document identity/revision | `documentSession` | Transition/request promise ve recovery attempt bilgisi |

Controller veya runtime içinde metadata, outline, viewer, modal ya da document state'in ikinci mutable reactive kopyası tutulmaz. Store getter'ları `physicalPageIndex`, `spreads`, active modal ve panel state için authoritative kaynaktır.

## Runtime boundaries

### `documentRuntime`

Yalnız document open, dirty transition, save/save-as, recovery, metadata, outline ve session mutation sonrası document refresh işlemlerini koordine eder. Document position persistence da document presentation lifecycle'ına bağlıdır.

Image export, page extraction, conversion, annotation burn-in, Markdown export, OCR, share ve transform feature'larını sahiplenmez.

Metadata ve outline sonuçları uygulanmadan önce hem session ID hem content generation eşleşmelidir.

### `viewerRuntime`

Render, navigation ve annotation canvas wiring'ini compose eder. PDF worker activation/deactivation serialization queue'sunun tek sahibi `viewerController`dır. Runtime ikinci queue veya viewer state oluşturmaz.

Image render başlangıcında image path primitive olarak capture edilir. Path, request ID veya mount durumu değişen geç sonuç canvas'a boyanmaz.

### `featureRuntime`

Export, extraction, image conversion, annotation burn-in, Markdown, OCR, share, TTS, reading presentation ve mevcut transform action'larını koordine eder. Backend export action'ları yalnız bu runtime altında `documentExportController`a bağlanır.

Salt tek bir service fonksiyonuna pas veren yeni controller oluşturulmaz. Gerçek async ownership veya birden fazla servisi koordine etmeyen action dependency doğrudan runtime/router'a enjekte edilir.

### `commandActionRouter`

Command ID'yi registry ile doğrular ve bounded action tablosuna yönlendirir. Reactive state tutmaz, feature guard'ı atlamaz ve bilinmeyen/unregistered command'i çalıştırmaz.

### `presentationController`

Toast timer'ını ve doğrulanmış transient payload'ların `uiState`e yansıtılmasını taşır. Password attempt ID üretmez ve password operation kararı vermez.

## Password attempt ownership

Authoritative attempt ID, candidate path/name, resolver, supersede ve stale-response kararı yalnız `documentLifecycleController` private state'indedir.

Akış:

1. Document open coordinator `{attemptId, path}` ile parola ister.
2. Document controller önceki resolver'ı supersede eder ve `{attemptId, candidateName, error}` payload'ı üretir.
3. Presentation controller bu payload'ı typed password modal state'ine yansıtır.
4. Submit/cancel, modalın attempt ID'siyle document controller'a döner.
5. ID eşleşmiyorsa cevap reddedilir.
6. Authentication error yalnız hâlâ authoritative olan attempt için presentation'a gönderilir.

## Window close transaction

Native close transaction'ın tek sahibi `windowLifecycleController`dır. Aşağıdaki state bu controller'ın private runtime state'inde kalır:

- In-flight close promise ve duplicate guard.
- İlk senkron işlem olarak `preventDefault()`.
- Pending transition.
- Commit reconciliation/precommit recovery sırası.
- Cancel veya hata sonrasında coordinator resume.
- Session close ve `closeAllDocumentSessions` tamamlanmadan window destroy etmeme garantisi.

`documentLifecycleController` save/discard/recovery operasyonlarını sağlar; window transaction sahibi değildir.

## Mount ve unmount sırası

Mount callback senkrondur; Promise döndüren `onMount(async ...)` kullanılmaz.

Unmount sırası:

1. Mounted flag kapatılır.
2. Document open coordinator stop edilir.
3. Search/feature lifecycle yok edilir.
4. Document, application ve window listener lifecycle'ları durdurulur.
5. Pending async prompt iptal edilir.
6. Presentation timer'ları temizlenir.
7. Viewer RAF/timer/observer state'i durdurulur.
8. Coordinator stop sonucu güvenliyse worker document deactivation aynı serialization queue üzerinden yapılır.

Application startup, config initialization sırasında stop gelmesine karşı epoch guard kullanır. Geç native listener registration hemen dispose edilir.

## Shell API

Extracted shell bileşenleri modal işlemlerinde `openModal`, `closeModal`, `isModalOpen`; panel işlemlerinde `togglePanel`, `setPanel` ve `panels` kullanır. Compatibility boolean alias'ları yalnız henüz migrate edilmemiş legacy tüketiciler için store sınırında bulunabilir.

Tek `activeModal`, AI assistant ile AI settings dahil blocking modal'ların aynı anda açılmasını engeller. Docked panel'ler modal state'inden bağımsızdır.

## Kalan doğrudan Tauri tüketicileri

Bounded altyapı noktaları:

- `src/services/backend/client.js`: raw `invoke` sınırı.
- `src/services/platform/dialogs.js`, `window.js`, `events.js`: platform adaptörleri.
- `src/configManager.js`: AppData filesystem config sınırı.
- `src/main.js`: shell plugin startup bağlantısı.

Henüz platform adaptörüne taşınmamış legacy bileşenler:

- `AnnotationLayer.svelte`
- `PdfXWizard.svelte`
- `ExportModal.svelte`
- `PageOrganizer.svelte`
- `PluginManagerModal.svelte`
- `MergeSplitModal.svelte`

Bu envanter 7R kapsamında toplu olarak migrate edilmez; App composition sınırına sızmalarına izin verilmez.

## Kalan `pdfState` tüketicileri

- `AiContextMenu.svelte`
- `AiSidebar.svelte`
- `OcrLayer.svelte`
- `AnnotationLayer.svelte`
- `MergeSplitModal.svelte`
- `PageOrganizer.svelte`

Bu legacy facade tüketicileri bağımsız takip işidir. `App.svelte` doğrudan `pdfState` kullanmaz.

## Sanitization inspection ve legacy sınırı (8C.2)

`SanitizationDialog.svelte` içindeki geniş boolean seçenekler, featureRuntime/App modal
wiring ve `documentTransformController` içindeki implicit `_sanitized.pdf` hedefi
dormant migration debt'tir. Feature gate kapalıdır ve bu zincir future 8C workflow
contract'ı değildir. 8C.2 versioned inspection modeli bu payload'dan bağımsız olarak
`sanitizationSchemas.js` → backend `sanitization.js` →
`sanitizationInspectionController.js` → `sanitizationState.svelte.js` → read-only
`SanitizationInspectionPanel.svelte` hattında kurulmuştur. Controller session ID,
content generation, job ID ve local revision ownership uygular; App yalnız composition
ve context reconciliation taşır. Feature gate kapalı olduğu için panel production'da
runnable değildir. Save/transform/verifier/publication migration'ı 8C.6 kapsamındadır.
`remove_hidden_layers` production control olarak taşınamaz çünkü `/OCProperties`
removal hidden-content removal değildir.

## Kabul kapıları

- `App.svelte` hard limit: `<700` satır; script hedefi: `<200` satır.
- Yeni runtime/controller/component hard limit: `<600` satır.
- `npm run test`, `npm run build` ve `npx svelte-check` başarılı olmalıdır.
- Architecture testleri App import sınırlarını, ownership ayrımını ve satır limitlerini doğrular.
- `src-tauri/target` dışındaki `src-tauri/**` manifest'i ve `package-lock.json` SHA-256 finalde baseline ile karşılaştırılır.
- Kalıcı todo/expected-failure bırakılmaz.

Prompt 8 kapsamındaki gerçek sanitization/redaction semantiği bu refactor'un kapsamı değildir.
