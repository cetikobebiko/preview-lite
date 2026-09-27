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

Tek `activeModal`, AI settings dahil blocking modal'ların aynı anda açılmasını engeller. AI sidebar ve docked panel'ler modal state'inden bağımsızdır.

## Kalan doğrudan Tauri tüketicileri

Bounded altyapı noktaları:

- `src/services/backend/client.js`: raw `invoke` sınırı.
- `src/services/platform/dialogs.js`, `window.js`, `events.js`: platform adaptörleri.
- `src/configManager.js`: AppData filesystem config sınırı.
- `src/main.js`: shell plugin startup bağlantısı.

Henüz platform adaptörüne taşınmamış legacy bileşenler:

- `AnnotationLayer.svelte`
- `ExportModal.svelte`
- `PageOrganizer.svelte`
- `PluginManagerModal.svelte`
- `MergeSplitModal.svelte`

Bu envanter 7R kapsamında toplu olarak migrate edilmez; App composition sınırına sızmalarına izin verilmez.

## AI Subsystem Frontend Mimarisi (Prompt 9A.9)

Canonical AI akışı `aiState.svelte.js` → `aiController.js` → `AiSettingsModal.svelte` / `AiSidebar.svelte` / `AiContextMenu.svelte` sınırındadır:
- **Svelte 5 Runes**: `aiState.svelte.js` reaktif durumları `$state` ve `$derived` ile yönetir. `activeProfile` başlangıçta `null` olarak başlatılır; `isHydrated` durumu `Boolean(activeProfile?.endpoint_fingerprint)` üzerinden reaktif türetilir.
- **Hydration Otoritesi**: `aiController.ensureHydrated()` çağrısı tüm AI eylemlerinden önce çalışır ve backend endpoint durumunu garanti eder.
- **Authoritative Preview & Debounce**: `AiSettingsModal.svelte` kullanıcı girdi revizyon sayacı (`previewRevision`) ve 250ms debouncing ile `preview_ai_endpoint` çağrısı yapar; eski yanıtlar elenir.
- **Lifecycle & Listener Teardown**: `aiController.dispose()` `applicationRuntime.js` unmount callback'ine bağlanmıştır; tekilleştirilmiş event listener güvenli bir şekilde kapatılır ve dispose sonrası gelen stream event'leri elenir.


## Sanitization frontend workflow (8C.6)

Canonical akış `featureRuntime.sanitization` → `sanitizationController.js` →
`sanitizationState.svelte.js` → `SanitizationPanel.svelte` sınırındadır. `App.svelte`
yalnız composition ve session/generation reconciliation tetiklemesi taşır; component
backend wrapper veya Tauri dialog import etmez. Inspection, policy ve execution
state'leri ayrıdır. Controller session ID, content generation, job ID, inspection
version ve workflow revision ownership uygular; eski session/generation sonuçları
presentation state'e uygulanmaz.

Panel önce immutable-snapshot inspection yapar; findings/support matrix gösterildikten
sonra canonical `verified_limited_v1` veya yalnız supported control içeren
`custom_supported_v1` policy seçilir. Unsupported/unverified controls seçilemez ve
XFA, RichMedia/rendition, optional content veya encryption blocker'ı acknowledgement
ile aşılamaz. Signed input için signature invalidation onayı ayrı ve default `false`
alanıdır. Policy değişikliği önceki execution-ready authority'yi workflow revision ile
invalid eder.

Save dialog yalnız current inspection, geçerli policy, blocker yokluğu ve gerekli açık
onaylar sağlandıktan sonra platform adapter'ı üzerinden açılır. İptal edilen dialog job
başlatmaz. Execution request session/generation/inspection/profile/controls/
acknowledgements/revision immutable snapshot'ıdır. Canonical backend command aynı
snapshot üzerinde inspection ve policy'yi yeniden doğrular; frontend inspection
security authority değildir. Cancellation publication öncesinde cooperative'tir;
commit başladıktan sonra backend'in published disk truth'u kazanır. Success sonucu
limited/custom assurance, applied/not-found controls, verification, output path/digest
ve typed commit state'i ayrı gösterir; recovery/indeterminate/target-conflict/
verification-failure anlamları flatten edilmez.

Legacy geniş boolean UI/wire service yolu ve implicit `_sanitized.pdf`/auto-open akışı
8C.6'da kaldırılmıştır. Rust `sanitize_pdf_document` command'ı yalnız fail-closed legacy
surface olarak kalır; canonical workflow bunu çağırmaz. `remove_hidden_layers`
production control değildir. Registry hâlâ `status: disabled`, `enabled: false`;
aktivasyon 8C.7 kapsamındadır.

## Plugin System Frontend Workflow (Prompt 9B)

Eklenti yönetimi `PluginManagerModal.svelte` ve `pluginStore.js` bileşenleri üzerinden yürütülür:
- **Feature Gate Koruması**: Tüm eklenti eylemleri (`inspectPlugin`, `installPlugin`, `grantConsent`, `togglePluginEnabled`, `startPluginHook`, `cancelPlugin`) `guardFeatureAction('plugin_system', ...)` ile korunur. Feature devre dışıyken (`enabled: false`) tüm eylemler fail-closed olarak `FeatureUnavailableError` fırlatır.
- **İki Aşamalı Onay & Yükleme**: Eklenti paketi seçildiğinde önce statik denetim (`inspectPlugin`) çalışır; manifest bilgileri, imzasız paket uyarısı ve izin bazlı veri etkisi açıklamaları sunulur. Kullanıcı onay kutusunu işaretlemeden (`consentAcknowledged === true`) yükleme butonu aktifleşmez. Onaylandığında atomik staging (`installPlugin`) ve tekil politika versiyonu (`grantConsent`) adımları işletilir.
- **Svelte 5 Runes**: `PluginManagerModal.svelte` tamamen Svelte 5 runes (`$state`, `$derived`, `$props`) ile reaktifliği yönetir; legacy Svelte 3/4 syntax'ı barındırmaz.

## Kabul kapıları

- `App.svelte` hard limit: `<700` satır; script hedefi: `<200` satır.
- Yeni runtime/controller/component hard limit: `<600` satır.
- `npm run test`, `npm run build` ve `npx svelte-check` başarılı olmalıdır.
- Architecture testleri App import sınırlarını, ownership ayrımını ve satır limitlerini doğrular.
- `src-tauri/target` dışındaki `src-tauri/**` manifest'i ve `package-lock.json` SHA-256 finalde baseline ile karşılaştırılır.
- Kalıcı todo/expected-failure bırakılmaz.

Prompt 8 kapsamındaki gerçek sanitization/redaction semantiği bu refactor'un kapsamı değildir.

## Signature Feature Production Surface Removal

Dijital imza özelliğinin production UI, IPC ve backend yüzeyleri bu kodlamanın kapsam sınırında tamamen kaldırılmıştır. Aşağıdaki bileşenler ve artifactlar production kaynak ağacında **artık mevcut değildir**:

### Kaldırılan Frontend Bileşenleri ve Dosyalar

- `SignatureWizardModal.svelte` — Signature wizard modal bileşeni kaldırılmıştır; `AppModalHost` içermez.
- `SignaturePanel.svelte` — Signature panel bileşeni kaldırılmıştır; `AppModalHost` içermez.
- `SignatureSettingsModal.svelte` — Signature settings modal kaldırılmıştır; `AppModalHost` içermez.
- `src/services/backend/signatures.js` — Production backend wrapper kaldırılmıştır.
- `src/services/actions.js` içindeki signature action export'ları kaldırılmıştır.

### Kaldırılan IPC Contracts

Production `contracts.js` içinde imzayla ilgili anahtarlar (`verify_pdf_signatures`, `list_system_certificates`, `sign_pdf_pades`, `enable_ltv`) bulunmamaktadır.

### Kaldırılan Backend Handler'lar

- `src-tauri/src/signature.rs` dosyası mevcut değildir.
- `lib.rs` generate_handler! içinde `signature::verify_pdf_signatures`, `signature::list_system_certificates`, `signature::sign_pdf_pades`, `signature::enable_ltv` kaydı yoktur.

### Kaldırılan Mock / Prototype DTO'lar

Production `src-tauri/src/` altında `SignatureValidationResult`, `SignatureOptions`, `VisualSignatureOptions` veya benzer mock DTO tanımı bulunmamaktadır.

### Kaldırılan Hassas Production Kimlik Bilgileri

- PIN alanı production kaynakta yoktur (yalnızca `PasswordModal` içindeki parola girişi, imza ile ilgisizdir).
- `tsaPassword`, `timestamp.digicert` veya benzeri TSA credential alanı production kaynakta yoktur.
- Hard-coded TSA URL yoktur.
- DigiCert Assured ID Root CA, GlobalSign Root CA gibi sahte trust-root listeleri yoktur.

### Feature Guard Durumu

`featureRegistry.js` içinde signature feature'ları `status: "disabled"`, `enabled: false` olarak kayıtlıdır. Feature guard dışında backend capability sınırı da kapalıdır; backend handler'lar kaldırılmıştır.


> [!NOTE]
> Bu bölüm, acceptance matrix kriterleri #3–#20 ile doğrudan eşleşmektedir. Statik kontrol komutları için bkz. `ipc-inventory.md` ve `pades-architecture-design.md` (signature surface removal bağlamında §6).

# Basic accessibility audit UI boundary

`A11yPanel.svelte` consumes only the session-native basic audit and read-only structure
inspection wrappers. Both responses cross strict schema/enum/size validation before UI
state. Request sequence, session identity, content generation, and mounted state prevent
stale commits. The production panel contains no score, conformance badge, screen-reader
simulation, auto-tag action, accessibility editing form, or reading-order drag/drop.

Audit schema `accessibility-audit/2` exposes `/MarkInfo` dictionary presence, `/Marked`
entry presence, and the boolean `/Marked == true` result separately. Session close,
generation change, a newer audit request, panel unmount, and PDF-to-image UI transitions
all invalidate older results before state commit.
