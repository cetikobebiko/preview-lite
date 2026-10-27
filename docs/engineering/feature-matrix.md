# Feature Matrix

Bu matris, yalnız kaynak kod, IPC bağlantıları ve çalıştırılabilen baseline
kontrollerinden çıkarılan kanıtlara dayanır. UI'da buton, modal veya command adı
bulunması tek başına özelliğin çalıştığı anlamına gelmez.

## Statü metodolojisi

| Statü | Kullanım |
|---|---|
| `working` | Normal uygulama yolundan erişilebilir, gerçek implementasyona sahip ve ilgili test veya runtime doğrulaması başarılı. |
| `partial` | Temel davranışın bir bölümü gerçek; kapsam, güvenilirlik, güvenlik, platform veya test doğrulaması eksik. |
| `mock` | Hard-coded/sahte sonuç, görsel taklit, copy-only davranış veya gerçek işlemi yapmadan başarı döndüren yol. |
| `scaffold` | UI/modül/command iskeleti var, ancak temel işlem uygulanmamış veya açıkça unsupported hata döndürüyor. |
| `unreachable` | Kod var, ancak handler kaydı, payload, frontend bağlantısı, platform veya yapılandırma nedeniyle normal uygulama yolundan ulaşılamıyor. |
| `unknown` | Statik analiz ve mevcut doğrulamalar güvenilir karar vermeye yetmiyor. |

Test sayılarının güncel kaydı aşağıdaki "Güncel doğrulama" bölümündedir. Prompt 5.1 kaydı tarihseldir (o tarihte 272 Vitest ve 85 Rust testi geçmişti).

Document Session mimarisi, Guard API entegrasyonu, single-flight save kilitleri ve PDFium-only motor kuralları runtime & unit test seviyesinde `working` ve `verified` statüsündedir. Windows temp DACL doğrulaması bir istisnadır: main'de paralel `DocumentSessionStore::initialize` çağrıları aralıklı olarak `TEMP_SECURITY_ACL_UNAVAILABLE` ile düşüyordu (Build Windows CI). Düzeltme `feat/undo-redo-incremental-save` dalındadır (PR #1) ve o dalın Windows CI'ında geçmiştir; merge edilene kadar main için bu garanti `partial` sayılmalıdır.

### Implementation status ve validation state ayrımı

`Status` yalnız implementasyonun kaynakta görülen olgunluğunu sınıflandırır.
`partial`, implementasyonun gerçek fakat eksik/riskli olduğunu; `mock`,
`scaffold` ve `unreachable` ise gerçekten eksik veya bağlı olmayan davranışı
ifade eder. Bir özelliğin yalnız test/runtime çalışmadığı için doğrulanamaması,
tek başına onu `mock` veya `scaffold` yapmaz.

Bu baseline'daki bütün satırlar için kaynak kanıtı **statically verified**'dır.
Hiçbir satır **runtime verified** değildir. `Tests` sütunu ayrıca satırın
validation state'ini aşağıdaki terimlerle gösterir:

- **statically verified / unrun / mock-only test**: test dosyası vardır,
  çalıştırılmamıştır ve Tauri IPC/backend davranışını mock eder.
- **statically verified / unrun / unit**: birim testi vardır, dependency
  çözümlemesi nedeniyle çalışmamıştır.
- **statically verified / unrun / no test**: kaynak davranışı incelenmiştir,
  fakat ilgili test yoktur.
- **statically verified / fuzz unrun**: fuzz hedefi vardır, çalıştırılmamıştır.

Mevcut kısa `unrun / mock IPC`, `unrun / unit` ve `test yok` ifadeleri bu
validation state terimlerinin kısaltmasıdır; hiçbirisi runtime başarı iddiası
değildir.

## Viewer / render

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| PDF açma, sayfa sayısı ve parola cache'i | `src/App.svelte:860`; parola akışı `src/pdfState.js:5,146` | `get_page_count`, `check_pdf_encrypted`, `authenticate_pdf_password`; `pdf_engine/mod.rs:57-112` | `partial` | PDFium load ve `PasswordCache` gerçek implementasyondur. | `check_pdf_encrypted` herhangi bir load hatasını şifreli PDF gibi yorumlayabilir; runtime doğrulanmadı. | `src-tauri/src/lib.rs` eksik-PDF testi unrun; doğrudan parola testi yok. |
| Ana sayfa render'ı | `src/components/PdfPage.svelte:354` | `render_page`; `pdf_engine/mod.rs:114-190` | `partial` | PDFium page render sonucu RGBA olarak frontend canvas'a gönderilir; ağır iş `spawn_blocking` yolundadır. | PDFium dylib yalnız macOS arm64 doğrulandı; build/runtime çalışmadı. | `PdfPage.test.js` unrun / mock IPC; Rust entegrasyon testi yok. |
| Gelişmiş render/cache command'ı | Statik consumer yok | `render_pdf_page`; `pdf_engine/mod.rs:191-359` | `unreachable` | Render cache, cancellation, semaphore ve generation kontrolü vardır; handler'a kayıtlıdır. | Frontend call-site yok; normal UI akışında kullanılmıyor. | Test yok. |
| PDFium subprocess worker izolasyonu | Normal frontend entry point veya Tauri command bağlantısı yok | `SubprocessManager` (`pdf_engine/subprocess_manager.rs:23-79`), `WorkerCommand/WorkerResponse` (`worker/ipc.rs`), worker loop (`worker/pdfium.rs`), `main.rs:9-12 --run-worker` | `unreachable` | Binary `--run-worker` ile stdin/stdout bincode worker moduna girebilir; manager child process başlatıp command gönderir. Kaynak genelinde `SubprocessManager` kullanımı yalnız kendi modülü ve `tests/crash_tests.rs` içindedir. Normal viewer/render/search/text command'ları worker manager'ı kullanmaz. | İzolasyon normal uygulama yoluna bağlı değildir. Ana viewer/render/search/text yolları PDFium'u ana process içinde doğrudan kullanır; native PDFium crash'i uygulama process'ini düşürebilir. Worker yaşam döngüsü, protocol framing ve restart davranışı runtime doğrulanmadı. | `tests/crash_tests.rs` yalnız `fuzz/artifacts/pdfium_lifecycle` varsa assertion/command üretir; artefakt dizini yoksa mesaj basıp `return` ile skip eder. Cargo testi baseline'da tamamlanmadı; statically verified / unrun. |
| Thumbnail render | `PageOrganizer.svelte:94`, `Thumbnail.svelte:34` | `render_thumbnail`, `render_pdf_thumbnail`; `pdf_engine/mod.rs:360-506` | `partial` | PDFium thumbnail render ve cache yolları gerçek. | İki ayrı thumbnail command sözleşmesi bulunuyor; native/runtime doğrulanmadı. | `PageOrganizer.test.js` unrun / mock IPC; thumbnail Rust testi yok. |
| Outline/sidebar | `App.svelte:871`, `Sidebar.svelte:24` | `get_pdf_outline`; `pdf_engine/mod.rs:507-532` | `partial` | PDFium bookmark ağacı çıkarılıyor. | Bozuk/şifreli outline senaryoları test edilmedi. | Test yok. |
| Image açma/dönüştürme ve PDF sayfasını image export | `App.svelte:979,1127,1917` | `read_image_file`, `convert_and_save_image` (`lib.rs`); PDF sayfa export'u `document_session::export_session_page_to_image` (job worker). Kayıtsız `export_pdf_page_to_image` ve yalnız onu besleyen `pdf_engine::render_page_rgba` 2026-10-16'da kaldırıldı. | `partial` | Gerçek image decode/encode ve PDFium render yolları var. | Format/renk profili/EXIF davranışı doğrulanmadı; görsel girdileri 16384 px kenar / 256 MiB decode ve RGBA sınırıyla açılır. | `read_image_file`/`convert_and_save_image`: `lib.rs` birim testleri (async çıktı eşitliği, bozuk dosya, runtime'ı bloklamama, boyut sınırı) ve `client.test.js` binary decode. |

## Search / text layer

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Belge çapında arama | `src/pdfState.js:161`; `SearchBar.svelte` | `search_pdf_text`; `pdf_engine/mod.rs:543-747` | `partial` | PDFium text tarama, NFKC normalizasyonu, geometry, cache ve cancellation gerçek implementasyondur. | Uçtan uca test çalışmadı; büyük/şifreli belgelerde doğrulanmadı. | `SearchBar.test.js`, `searchResultUtils.test.js`, `searchCoordinator.test.js` unrun; UI testleri backend'i doğrulamaz. |
| Sayfa text layout/DOM text layer | `PdfPage.svelte:278`; text-layer utils | `get_page_text_layout`; `pdf_engine/mod.rs:748+` | `partial` | PDFium karakter/kelime geometrisi frontend DOM katmanına taşınır. | Koordinat, rotation, zoom ve stale-result davranışı runtime doğrulanmadı. | `textLayerUtils.test.js`, `canvasUtils.test.js`, `PdfPage.test.js` unrun; PdfPage IPC mock. |
| Search cache temizleme | Consumer yok | `clear_search_cache`; `pdf_engine/mod.rs:533` | `unreachable` | Handler kaydı var. | Frontend çağrısı yok; cache yaşam döngüsü UI'dan tetiklenmiyor. | Test yok. |
| Text export | `ExportModal.svelte` → `exportSessionTextToFile` | `document_session::export_session_text_to_file`: ephemeral job worker çıkarır, Rust hedefe `document_io::write_atomically` ile yazar; hedef yalnız `.txt`/`.md`, session temp root reddedilir. Webview artık dosya yazmaz. | `partial` | PDFium text extraction, atomic hedef yazımı ve hedef doğrulaması gerçek. | Markdown biçim doğruluğu ve encoding kenar durumları test edilmedi; çalışan uygulamada elle doğrulanmadı. | Rust: `text_export_to_file_*` (çıkarılan metinle birebir içerik, mevcut hedefin tamamen değişmesi, geçersiz/özel hedef reddi); Vitest: `ExportModal.test.js` text export, `wrappers.test.js`. |

## Annotation

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Annotation listeleme/ekleme/silme/kaydetme | `AnnotationLayer.svelte`; `src/pdfState.js` (`addAnnotation`, `deleteAnnotation`) | `document_session::{get_session_page_annotations,add_session_annotation,delete_session_annotation}` → `MutationRecipe::{AddAnnotation,DeleteAnnotation}` → job worker `annotations.rs`; kayıtsız path-native `annotations::{add_annotation,delete_annotation,save_pdf_with_annotations}` kaldırıldı | `partial` | Highlight, Underline ve Text oluşturulur, silinir ve okunur. Okumalar pdfium-render'da; oluşturma ve silme ham PDFium'da yapılır, `incremental_save::save_mutation` ile artımlı yazılır. İmzalı belgede imzalı baytlar korunur; Highlight/Underline görünüm akışı PDFium'a render ile ürettirilir. Her ekleme/silme geri alınabilir. | Yalnız üç tür; frontend'deki `Ink` geri dönüşü backend'de desteklenmiyor. Görselli sayfada AP üretimi sayfa görsellerini de artımlı bölüme yazar (K5; lopdf fark yazımı henüz uygulanmadı). Yerel Tauri penceresinde uçtan uca denenmedi. | `annotations::tests` (21; artımlı yazım, imzalı belge, tarih biçimi dahil), `pdfState.signatureLoss.test.js`; frontend akışı mock IPC. |

> 2026-10-16: Annotation burn-in export kaldırıldı. Tauri handler'a hiç kaydedilmemiş `burn_annotations_to_pdf` komutu (lopdf ile yazım, hedefe atomik olmayan `doc.save`, Resources'ta tanımsız `/F1` fontu, ölçeksiz canvas→PDF koordinatları), `annotation_burn_export` feature kaydı, `burn-in-pdf` command palette girdisi, HUD butonu, `burnAnnotationsToPdf` sözleşmesi/wrapper'ı ve ilgili testler silindi.

## Page manipulation

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Rotate/delete/reorder | Page organizer; `src/pdfState.js` (`rotatePage`, `deletePages`, `reorderPages`) | `document_session::{rotate_session_page,delete_session_pages,reorder_session_pages}` → job worker `manipulation.rs`; kayıtsız path-native `manipulation::{rotate_page,delete_pages,reorder_pages}` kaldırıldı | `partial` | Döndürme ve silme ham PDFium ile yapılıp `save_mutation` ile artımlı yazılır. Sıralama yalnız sayfa ağacını lopdf ile artımlı yazar (`reorder_incrementally`), bu yüzden imza, outline, named destination, AcroForm, ek ve metadata içeren belgeler de sıralanabilir. İmzalı belgede imzalı baytlar korunur; yazılamazsa imzasız belge eski tam yeniden yazıma düşer, imzalı belge reddedilir. `rotate_page` mevcut `/Rotate` değerine göreli döndürür. Her adım geri alınabilir. | PDFium artımlı kaydı, dokunulan sayfanın yüklediği nesneleri (görseller dahil) de yeniden yazar; dosya büyür (S6, K5). Yerel Tauri penceresinde uçtan uca denenmedi. | `manipulation::tests` (14), `incremental_save::tests` (11), `tests/document_io_pdfium.rs`, `PageOrganizer.test.js` (mock IPC), `pdfState.signatureLoss.test.js`. |
| Extract/split | `App.svelte:1428`; merge/split modal via `pdfState.js:47` | `extract_pages`, `manipulation::split_pdf` | `partial` | Sayfa aralığı kopyalama ve dosyaya save implementasyonu var. | `extract_pages` `lopdf` kullanır; şifreli ve bozuk PDF senaryoları doğrulanmadı. | Test yok. |
| Merge | `App.svelte:1810`; merge/split modal via `pdfState.js:43` | `document_session::merge_session_pdfs`; `manipulation::merge_pdfs` kayıtsızdır, yalnız atomic I/O testleri için tutulur | `partial` | PDFium tabanlı merge implementasyonu var ve wrapper payload'ı eşleşir. | `App.svelte:1810` doğrudan yolu `{pdfPaths,targetPath}` göndererek sözleşmeyi bozar; hangi UI yolunun baskın olduğu runtime doğrulanmadı. | Test yok. |
| Blank page insert | `src/pdfState.js` (`insertBlankPage`) | `document_session::insert_session_blank_page` → `MutationRecipe::InsertBlankPage` → job worker `manipulation::insert_blank_page_report`; kayıtsız path-native `manipulation::insert_blank_page` kaldırıldı | `unreachable` | Backend gerçek implementasyondur: ham PDFium `FPDFPage_New`, `save_mutation` ile artımlı yazım, MediaBox doğrulaması. | Frontend `page_insert_blank` özelliği kayıtta `disabled` olduğu için normal UI yolundan ulaşılamaz. | `manipulation::tests` (MediaBox ve artımlı yazım testleri). |
| Form field read/update | `FormLayer.svelte`; `stores/formDraftState.svelte.js`; `stores/formCommitController.svelte.js`; `pdfState.{getPageFormFields,getFormInfo,fillFormFields}` | Okuma: `get_session_page_form_fields` → worker → `forms.rs`. Yazma: `fill_session_form_fields` → `MutationRecipe::FillFormFields` → job worker `forms::fill` (ham PDFium FORM API, `atomic_write_pdf`) | `experimental` | Alanlar okunur, sayfa üzerinde düzenlenir ve odak kaybı/değişimde tek mutasyonla belgeye yazılır. İsim-indeks, tür, read-only ve MaxLen job worker'da denetlenir; imzalı belgede onay istenir. Acrobat AF betikleri (biçim, keystroke, aralık doğrulaması, `AFSimple_Calculate` ve basit alan gösterimi) JavaScript motoru olmadan çalışır: geçersiz değer reddedilir, biçimli alanın görünümü biçimli çizilir ve hesaplanan alanlar her commit'te yeniden yazılır (`forms::af`, PdfCraft'tan; bkz. form-filling-plan.md). | Form yazımı artımlıdır ve imzalı baytları korur (`incremental_save::save_mutation`; xref tutarsızsa imzalı belge reddedilir). Diğer oturum mutasyonları da artık artımlıdır; imzalı belgede bunlardan önce bilgilendirme gösterilir (`signatureLossGuard.js`). Linux/Windows runtime'larında Türkçe glif kaybı ve tespit yok. Yerel Tauri penceresinde uçtan uca doğrulanmadı. | `forms::tests` (7), `forms::fill::tests` (27), `forms::af` (31), `job_worker` form dispatch testi, `worker_reads_form_info_and_page_form_fields`, `FormLayer.test.js` (8), `formCommitController.test.js` (5), `formDraftState.test.js` (3), `client.test.js`, `architecture.test.js`. |
| Form field authoring | `MarkupToolbar.svelte` form araçları; `FormDesignLayer.svelte`; `FormFieldProperties.svelte`; `stores/formDesignState.svelte.js`; `pdfState.authorFormFields` | `author_session_form_fields` → `MutationRecipe::AuthorFormFields` → job worker `forms::author` (`lopdf` artımlı yazım, PDFium geometri ve doğrulama) | `experimental` | Alan ekleme (metin, checkbox, radio, açılır liste, liste kutusu; sürükleyerek ya da tıklayarak), seçme, taşıma, boyutlandırma, silme, özellik değiştirme; artımlı ve imzalı baytları koruyor, imzalı belgede uyarı. | Yerel Tauri penceresinde gerçek PDF ile uçtan uca denenmedi; taşınan değerli metin alanında görünüm ölçeklenir; her alan değişikliği undo/redo ile geri alınabilir. | `forms::author::tests` (11), `job_worker` author dispatch testi, `formDesignState.test.js`, `FormDesignLayer.test.js`, `FormFieldProperties.test.js`, `client.test.js`, `architecture.test.js`. |
| Metadata read/write | `MetadataModal.svelte`; session service | `get_session_pdf_metadata`, `update_session_metadata` | `partial` | Read session-native ve PDFium metadata API’sini kullanır. | Write PDFium binding’inde güvenli destek olmadığı için readonly/typed unsupported’dır; legacy path bypass yoktur. | Contract/architecture ve Rust session testleri. |
| Undo / redo | ⌘Z, ⇧⌘Z / ⌘Y (`keyboardController.js`); komut paleti `undo-mutation` / `redo-mutation`; `services/app/mutationHistoryController.js` | `document_session::{undo_session_mutation,redo_session_mutation}` → `mutate_isolated_restore` → aynı izole işlem hattı (`MutationTransactionKind::Restore`) | `experimental` | Her oturum mutasyonu, işlem öncesi snapshot'ı `session_dir/history/` altına taşıyarak bir giriş bırakır (ek kopya yok). Geri yükleme digest ile doğrulanır; uyuşmazlık reddedilip geçmiş temizlenir. Generation hep ilerler; `dirty` içerikten hesaplanır. En fazla 50 adım ve 1 GiB. | Her giriş tam bir dosyadır; büyük belgede bütçe birkaç adımda dolar. Geçmiş uygulama kapanınca silinir. Metin alanındayken tarayıcı undo'su çalışır. Yerel Tauri penceresinde uçtan uca denenmedi. Ayrıntı: `undo-redo-and-incremental-save-design.md` §5.11. | `document_session::tests` (undo/redo bayt eşitliği, redo'nun silinmesi, boş geçmiş, bozuk dosya, kayıttan sonra undo, sınırlar), `mutationHistoryController.test.js`, `keyboardController.test.js`, `architecture.test.js`. |

## OCR

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Scanned-page tespiti | `OcrLayer.svelte` via `ocrDocumentState.js` | `ocr::detect_scanned_pages` | `partial` | Belge başına tek session job sayfa başına bool vektörü döndürür; frontend sonucu `(sessionId, contentGeneration)` başına cache'ler (hata 60 sn cache'lenir). Session/generation identity ve stale-result guard kullanır; PDFium heuristic (`trim().len() < 20`) korunur. | Heuristic doğruluğu ve şifreli PDF davranışı ortam bağımlıdır; büyük belgede ilk sonuç tüm sayfaların metin çıkarımını bekler. | OcrLayer çağrı sayacı/stale, `ocrDocumentState` ve Rust detection testleri. |
| Tesseract OCR | `OcrLayer.svelte` via `pdfState.js` / `ocrDocumentState.js` | `ocr::{get_available_ocr_languages,run_ocr_on_page}` | `partial` | Aktif PDF session-native read sınırındadır. Dil listesi async komutla `spawn_blocking` içinde, 10 sn zaman aşımıyla bir kez alınır; başarı süreç ömrü boyunca, hata 60 sn cache'lenir. | Kurulu Tesseract/dil verisine bağlıdır; searchable output desteklenmez. Uygulama açıkken kurulan dil paketi yeniden başlatma gerektirir. | Contract/wrapper, OcrLayer ve `ocr::tests` testleri. |
| Searchable PDF export | OCR UI via `pdfState.js:85` | `ocr::export_as_searchable_pdf` | `scaffold` | Command ve wrapper vardır. | Backend açıkça unsupported hata döndürür; `_` payload anahtarları belirsizdir. | Test yok. |
| macOS Vision OCR | `App.svelte` | `perform_ocr_on_page` | `partial` | PDF page OCR session snapshot kullanır; temp PNG RAII ile temizlenir. | Bağımsız image OCR unsupported; gerçek Vision runtime doğrulanmadı. | Session/stale ve temp başarı+hata cleanup testleri. |

## AI / RAG

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| AI Endpoint & Consent | `AiSettingsModal.svelte` | `ai::{preview_ai_endpoint,configure_ai_endpoint,get_ai_endpoint_status,clear_ai_endpoint_config}` | `working` | Presentation-only debounced preview, authoritative backend validation, DNS pinning, 4-tier timeout, IP classification, dynamic consent reset. | Uzak HTTPS sunucu erişilebilirliğine bağlıdır. | Comprehensive security acceptance suite (`security_acceptance_tests.rs`, `endpoint_tests`, `AiSettingsModal.test.js`). |
| Semantic index ve search | `AiSidebar.svelte` | `ai::{build_semantic_index,semantic_search}` | `working` | PDFium text extraction, Ollama embedding, atomic vector store staging & publication, canonical dimension check, lifecycle pruning. | Model boyutu ve bellek sınırına tabidir (100 MB per session, 500 MB global). | Vector store staging, dimension mismatch, lifecycle pruning testleri. |
| RAG chat streaming | `AiSidebar.svelte` | `ai::{query_document_rag,cancel_ai_request}`; `ai_stream_event` | `working` | Connect/send race cancellation, fail-closed EOF check, single terminal delivery state machine, session generation guard. | Model yanıt üretme hızına bağlıdır. | Tokio select cancel race, single terminal delivery & stream sequence testleri. |
| Selection/page summary streaming | `AiContextMenu.svelte` | `ai::{summarize_selection_or_page,cancel_ai_request}`; `ai_stream_event` | `working` | Selection-scoped attempt ID, connect/send race cancellation, fail-closed EOF check, single terminal delivery state machine. | Model yanıt üretme hızına bağlıdır. | Attempt ID validation, cancel race & terminal delivery testleri. |

## Redaction

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Sensitive-data scan & review highlight | `src/components/SensitiveDataScanPanel.svelte` | `redaction::scan_sensitive_data`; `redaction.rs` | `working` | Svelte 5 state store, controller, pure rotation selector, amber review highlight katmanı, async job cancel guard'ları. | OCR yapılmamış görsellerdeki metinleri bulmayabilir. | `src/stores/sensitiveDataScanState.test.js`, `src/services/app/sensitiveDataScanController.test.js`, `src/components/SensitiveDataScanPanel.test.js`, `src/components/PdfPage.test.js`. |
| Secure Redaction (Kalıcı Silme) | `SecureRedactionPanel.svelte`; `secureRedactionState.svelte.js`; `secureRedactionController.js`; `PdfPage.svelte` review-selection | `redaction::plan_secure_redaction`; `redaction::execute_secure_redaction`; legacy `apply_redactions` fail-closed | `disabled` | 8B secure rasterized redaction pipeline ve kullanıcı workflow'u tamamlanmıştır: explicit pending-region → authoritative plan review → ayrı loss acknowledgements → re-plan → Save dialog → cancellable independent verification → digest-bound atomic publication. macOS arm64 runtime acceptance PASS'tir. | **Activation deferred:** advertised all-desktop platform kümesi için macOS x86_64, Windows ve Linux runtime certification çalıştırılmamıştır. Registry `enabled: false`, `status: disabled` kalır. Optional whole-object strategy NO-GO'dur ve etkin değildir; bu raster pipeline durumunu düşürmez. | Store/controller/component/rotation/selection/wiring/architecture testleri, backend contract/worker/atomic I/O testleri, feature-gated `secure_redaction_execution_acceptance` ve `whole_object_backend_characterization`. |

## Sanitization

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| PDF Sanitization (Metadüzeltme/Temizlik) | `SanitizationPanel.svelte`; `sanitizationState.svelte.js`; `sanitizationController.js` | `inspect_document_sanitization`; canonical `execute_document_sanitization`; private `SanitizePdf` + independent `VerifySanitizedPdf` workers; legacy `sanitize_pdf_document` fail-closed | `disabled` | Inspection **implemented**; supported-control transform **implemented**; independent verification **implemented**; digest-bound atomic publication **implemented**; 8C.6 frontend workflow **implemented**. 8C-7 security audit PASS; macOS arm64 full execution acceptance PASS. **8C COMPLETE — ACTIVATION DEFERRED PENDING PLATFORM CERTIFICATION — PROMPT 9 READY**. | **Activation deferred pending platform certification.** XFA, RichMedia/rendition, optional content ve encryption blocker'dır; signature explicit invalidation acknowledgement ister. macOS x86_64/Windows/Linux runtime publication certification çalıştırılmadığından all-desktop activation ertelenmiştir. Registry `enabled: false`, `status: disabled` kalır. | State/controller/panel/schema/featureRuntime/architecture testleri; Rust transform/verifier/adversarial/full-rewrite testleri; real-process inspection→transform→verify→publish acceptance; artifact/staged/target TOCTOU; path/symlink/hardlink; atomic fault/recovery/durability; cancellation/session-neutrality. |


Terminoloji sınırı: **8A** sensitive-data detection ve geometry foundation'dır; **8B** yalnız kullanıcının açıkça seçtiği dikdörtgen bölgeler için destructive secure-redaction export'udur; **8C** document-wide sanitization, hidden payload ve metadata cleaning kapsamıdır.

8C.1 yalnız capability characterization'dır. Candidate status production support
anlamına gelmez; verified-limited assurance ancak bağımsız transform/verifier ve
atomic publication kabulünden sonra mümkündür.

## Signature / PAdES

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Sistem sertifikalarını listeleme | Yok (Bileşen/action kaldırıldı) | Yok (Handler silindi) | `disabled` | Mock handler ve UI tamamen silindi; `featureRegistry.js` metadata kaydı disabled. | Yok (Üretim erişimi kapatıldı). | `signatureSurface.test.js`, `signature_surface.rs` |
| PAdES imzalama | Yok (Bileşen/action kaldırıldı) | Yok (Handler silindi) | `disabled` | Sahte imzalama UI ve komut zinciri kaldırıldı; mimari tasarım `pades-architecture-design.md`'de tanımlandı. | Yok (Üretim erişimi kapatıldı). | `signatureSurface.test.js`, `signature_surface.rs` |
| İmza doğrulama | Yok (Bileşen/action kaldırıldı) | Yok (Handler silindi) | `disabled` | Sahte doğrulama paneli ve `is_valid: true` mockları kaldırıldı. | Yok (Üretim erişimi kapatıldı). | `signatureSurface.test.js`, `signature_surface.rs` |
| LTV enablement | Yok (Bileşen/action kaldırıldı) | Yok (Handler silindi) | `disabled` | LTV butonu ve komutu kaldırıldı; geleceğin B-LT/B-LTA mimarisi belgelendi. | Yok (Üretim erişimi kapatıldı). | `signatureSurface.test.js`, `signature_surface.rs` |
| Signature settings / TSA / trust store | Yok (Bileşen kaldırıldı) | Yok | `disabled` | Sahte ayar modalı ve hardcoded TSA girdileri üretimden kaldırıldı. | Yok. | `signatureSurface.test.js` |

## Accessibility / PDF-UA

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Basic accessibility audit | `A11yPanel.svelte` | `audit_session_accessibility` → worker `AuditAccessibility` | `experimental` | Schema v2 structured checks/evidence/coverage/human-review; MarkInfo dictionary/entry/value ayrıdır; score ve conformance claim yoktur. | Tam PDF/UA veya WCAG doğrulaması değildir. | Adversarial `a11y` Rust tests, `A11yPanel.test.js`, schema/worker lifecycle tests. |
| Salt-okunur structure inspection | `A11yPanel.svelte`, `TagTreeExplorer.svelte` | `get_session_structure_tree` → worker cached accessibility document | `partial` | Mixed `/K`, cycle/dangling ref, RoleMap ve temel ParentTree bounded incelenir. | Tam MCID content sequence ve PDF/UA-2 namespace semantiği incomplete. | Rust mixed/cycle/RoleMap/ParentTree tests. |
| PDF/UA machine validation | Production UI yok | Entegrasyon yok | `disabled` | Ayrı gelecek capability olarak kayıtlıdır. | Validator/runtime/license seçimi yapılmamıştır. | Registry tests. |
| Auto-tag ve accessibility editing | Production UI/IPC yok | Yok | `disabled` | Alt-text/raw-path/reorder komutları ve mock UI kaldırıldı. | Remediation gelecek ayrı fazdır. | Architecture/source tests. |

## Prepress / PDF-X

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Print resource inspection | `PrepressPanel.svelte` | `inspect_session_print_resources` → worker `InspectPrintResources` | `experimental` | Bounded/cycle-safe resource graph, basic ICC header, OutputIntent, spots, transparency/overprint, fonts and effective page boxes; `conformanceClaim: false`. | Definition observation is not content usage, print outcome, or PDF/X conformance. Encrypted structural inspection is explicitly unsupported. | Rust scanner tests, schema tests, panel lifecycle tests, architecture tests. |
| RGB-derived coverage preview | `PrepressPanel.svelte` | `generate_session_rgb_derived_coverage_preview` → worker | `experimental` | PDFium display RGB pixels produce a deterministic threshold heatmap and channel-sum statistics with explicit approximate metadata. | Not real TAC, ink consumption, color-managed output, or RIP output. | Formula/input/metadata/schema/panel tests. |
| RGB-derived process preview | `PrepressPanel.svelte` | `generate_session_rgb_derived_process_preview` → worker | `experimental` | Four approximate C/M/Y/K display images; visible legend and permanent warning. | Not printing plates, real separations, spot preservation, overprint simulation, or ICC transform. | Formula/input/metadata/schema/panel tests. |
| Page-box editing | Production UI/IPC yok | Yok | `disabled` | Effective boxes are read-only inspection observations. | Editing/remediation is a separate future feature. | Negative architecture and panel tests. |
| PDF/X export | Production UI/IPC yok | Yok | `disabled` | Registry explicitly states conversion/validation is not implemented. | Requires target policy, ICC assets/CMM, independent validator, rewrite and cross-platform packaging. | Negative source tests; future design document. |

## Plugins

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Isolated Plugin System (V1) | `PluginManagerModal.svelte`; `pluginStore.js` | `plugins::{inspect_plugin_package, install_plugin_package, grant_plugin_consent, uninstall_plugin, set_plugin_enabled, list_installed_plugins, start_plugin_execution, cancel_plugin_execution, get_plugin_execution_status}` | `experimental` (disabled) | Subprocess worker (`--run-plugin-worker`), Rhai 1.25.1 sandbox, 17-step revalidation, single consent policy version, fail-closed feature gate. | Üretimde feature gate kapalıdır (`feature_enabled = false`); yalnızca izin verilen host API'lar (`plugin_log`, `ui_notify`) çağrılabilir. | `cargo test plugins::` (44 unit/security testi); `PluginManagerModal.test.js`, `pluginStore.test.js`. |

## Batch processing

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Batch job yürütme | Yalnız `pluginStore.js` wrapper; consumer yok, `batch_processing` feature gate kapalı | `batch::add_batch_job`; `batch.rs` | `disabled` | Komut kayıtlıdır ancak argümanları okumadan fail-closed `FEATURE_UNAVAILABLE` döndürür. Sahte (500 ms bekleyip tamamlandı bildiren) worker ve kuyruk kaldırılmıştır. | Gerçek batch yürütme uygulanmamıştır. | `cargo test batch::tests::add_batch_job_fails_closed`; `wrappers.test.js` payload sözleşmesi. |
| Batch status UI | Listener bulunmadı | Yok (`batch_job_status` event'i kaldırıldı) | `disabled` | Backend artık batch status event'i emit etmez. | Yok. | Yok. |

## Platform-specific features

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Continuity Camera scan | `App.svelte` disabled flow | `trigger_continuity_camera_scan` handler retained | `disabled` | Frontend aktif-PDF path contract/action kaldırılmıştır. | Session-safe page insertion tasarlanmadan çalıştırılmaz. | Architecture legacy-surface testleri. |
| Text-to-speech/stop | `App.svelte:1182,1198,1468` | `speak_text`, `stop_speaking`; `lib.rs:315-369` | `partial` | macOS `say`, diğer platformlarda PowerShell/fallback dalları vardır. | Linux unsupported; non-macOS voice parametresi/stop davranışı ve process hata kontrolü zayıf. | Test yok; tek platform runtime çalışmadı. |
| macOS Share Sheet | `App.svelte` | `share_file_macos`, `share_external_file_macos` | `platform_limited` | Aktif PDF session snapshot ile; bağımsız image/file explicit external path ile paylaşılır. Process bounded timeout kullanır. | Gerçek picker UI bu testlerde açılmaz; non-macOS unsupported. | Payload ayrımı, timeout ve snapshot cleanup testleri. |
| Sistem yazdırma | Export UI via `pdfState.js:192` | `document_session::print_session_pdf` → `export::submit_print_file`; path-native `export::print_pdf` kaldırıldı | `unknown` | Platforma göre `lpr`/`lp`/PowerShell command yolları var. | Yazıcı bulunabilirliği, sayfa aralığı ve shell/process sonuçları doğrulanmadı. | Test yok. |
| Cross-platform paketleme | CI workflow'ları | Tauri bundle config ve `lib/*` resource | `unknown` | Linux/macOS/Windows CI matrisi tanımlı. | Yerel binary yalnız macOS arm64; CI çalıştırılmadı, Windows workflow test çalıştırmıyor. | CI sonucu yok. |

## Active frontend-only features

UI-only olması envanter dışında bırakılma nedeni değildir. Aşağıdaki özelliklerin
çekirdek davranışı frontend state/DOM/CSS veya Tauri fs plugin yolu üzerindedir;
backend command bulunmaması açıkça belirtilmiştir.

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Book, split, single-page ve smart-dark viewer modları | `App.svelte:68-73,1441-1493,1646-1662,2179-2212,2355-2360`; `PdfPage.svelte` | Özel backend command yok; mevcut `render_page` sonucu frontend layout/CSS/state ile sunulur | `partial` | Toggle state, layout class'ları, page-mode props ve local smart-dark setting persistence kaynakta bağlıdır. | Uçtan uca render/navigation testi çalışmadı; smart-dark renk doğruluğu ve split/book stale-render davranışı doğrulanmadı. | `PdfPage.test.js` statically verified / unrun / mock-only test; App integration testi yok. |
| Bionic Reading | `readingPresentationController.js`; `pageTextSource.js`; `BionicReadingModal.svelte` | Yalnız geçerli fiziksel sayfa: önce `export_session_text_content` (PDFium, worker job), metin katmanı boşsa aynı sayfanın önbellekteki OCR metni veya `perform_ocr_on_page` | `partial` | Frontend metni kelimelere ayırıp başlangıç karakterlerini `<b>` ile biçimler; modal "Yalnız sayfa N · PDF metin katmanı/OCR" etiketini gösterir. Belge değişince metin sıfırlanır ve modal kapanır. | Tüm belge kapsamı yok; erişilebilirlik runtime doğrulanmadı. | Vitest: `readingPresentationController.test.js`, `BionicReadingModal.test.js`; runtime unrun. |
| Reading Ruler | `App.svelte:75,1454-1457,1974-1975,2362`; `ReadingRuler.svelte` | Backend command yok | `partial` | Frontend toggle ve pointer/focus overlay component'i bağlıdır. | Farklı zoom, DPI ve klavye erişilebilirliği runtime doğrulanmadı. | Statically verified / unrun / no test. |
| Session analytics | `App.svelte:215-218,650-654,1992-2000`; `AnalyticsModal.svelte` | Backend command yok | `partial` | Açık belge varken saniye sayacı ilerler ve modal süre/sayfa/zoom özetini gösterir. | Sayaç yalnız mevcut process session'ındadır; background/idle ayrımı ve persistence yoktur. | Statically verified / unrun / no test. |
| Document-state ve recent-file persistence | `configManager.js`; `documentRuntime.js`, `documentPositionMemory.js` | `app_config::read_app_config` / `write_app_config`: AppData `app_config.json`, `document_io::write_atomically`, en fazla 1 MiB JSON nesnesi; okunamayan dosya `app_config.corrupt-*.json` olarak kenara alınır | `partial` | Recent list (20), belge başına sayfa/zoom durumu (en fazla 200, en uzun süredir güncellenmeyen düşer) ve son dosyayı auto-load yolları bağlıdır; yazımlar frontend'de sıraya girer. | Dosya taşınması/silinmesi ve path gizliliği (yollar düz metin saklanır) ele alınmadı; çalışan uygulamada elle doğrulanmadı. | Rust: `app_config::tests`; Vitest: `configManager.test.js`. |
| Command Palette ve keyboard shortcuts | `App.svelte:507-600,1441-1510,2008-2014,2089`; `CommandPalette.svelte`; `ShortcutsModal.svelte` | Backend command yok; seçilen bazı aksiyonlar mevcut IPC handler'larını çağırabilir | `partial` | Global key handler, palette command listesi ve dispatch switch'i kaynakta bağlıdır. | Liste ile gerçek shortcut/dispatch kapsamının senkron kalması ve input focus conflict'leri test edilmedi. | Statically verified / unrun / no test. |
| Markdown notes export | `App.svelte:1342-1388`; Command Palette `export-notes-md` | Backend command veya fs write çağrısı yok | `mock` | Markdown string hazırlanır ve save dialog açılır; save path seçilince başarı toast'ı gösterilir. | Hazırlanan `mdContent` hiçbir dosyaya yazılmaz; kullanıcıya sahte başarılı export bildirimi verilir. | Statically verified / unrun / no test. |

## Güncel doğrulama (23 Ekim 2026, macOS arm64)

`chore/audit-remediation` dalında (main ve audit düzeltmeleri) yerel olarak çalıştırıldı:

```text
137 Vitest dosyası / 1409 test
1121 Rust testi, 31 test hedefinde (788 library testi), 10 ignored
Svelte check 0 error
Build başarılı (chunk boyutu ve dynamic import uyarıları duruyor)
Clippy --all-targets --all-features -D warnings başarılı
```

İzlenen: `worker_pdfium_integration` içindeki `chunked_search_is_identical_to_whole_document_search` bir kez düştü (mesajı kaydedilmedi); ardından yaklaşık 120 lokal ve 40 CI koşusunda tekrarlanmadı. Test artık düşen adımı ve ilk farklı eşleşmeyi raporluyor. Komut bazında ayrıntı `pdfium-worker-isolation.md` "Local verification" tablosundadır.

## Prompt 8A-5 Doğrulaması (5 Ağustos 2026, tarihsel)

O tarihte tüm doğrulamalar yerel ortamda %100 oranında çalıştırılmış ve başarıyla geçmişti:

```text
75 Vitest dosyası / 540 test
297 Rust test
Svelte check 0 error
Build başarılı
Clippy -D warnings başarılı
```

O tarihteki mimari ve güvenlik gerçekleri:
- Production UI'da `apply_redactions` veya `sanitize_pdf_document` callsite'ı bulunmamaktadır.
- `apply_redactions` ve `sanitize_pdf_document` backend action wrapper'ları fail-closed `FEATURE_UNAVAILABLE` döndürür; PDFium veya lopdf parser'ı tetiklemez.
- `scan_sensitive_data` iptal edilebilir session-job kontratı (Job Worker Protocol v3) ve amber inceleme vurgu katmanı (`.review-highlight-layer`) ile güvenli şekilde çalışmaktadır.

## Tarihsel İlk Audit (Prompt 0 / Başlangıç Durumu)

> **Not:** Bu bölüm projenin ilk aşamasındaki (Prompt 0) tarihsel durumu belgeler. Yukarıdaki güncel bölümde belirtildiği üzere günümüzde tüm test ve build adımları yeşildir.

### İlk Test Kapsamı Özeti (Tarihsel)

İlk aşamada frontend'de bulunan 9 test dosyası:

```text
src/components/PdfPage.test.js
src/components/SearchBar.test.js
src/components/PageOrganizer.test.js
src/components/PrepressPanel.test.js
src/components/PdfXWizard.test.js
src/utils/searchResultUtils.test.js
src/utils/canvasUtils.test.js
src/utils/searchCoordinator.test.js
src/utils/textLayerUtils.test.js
```

### İlk Yüksek Riskli Bulgular (Tarihsel)

1. Görsel maskeleme redaksiyon yerine kullanılmaz (Prompt 8A'da `.review-highlight-layer` salt-görsel inceleme vurgusu olarak ayrıştırılmıştır).
2. PAdES, signature verification ve LTV yolları mock/scaffold'dur.
3. PDF/X export gerçek profil embed/convert/validation yapmaz.
4. Tarihsel PDF/UA validation adı kaldırılmıştır; üretim yüzeyi yalnız structured Basic Accessibility Audit olarak sunulur.
5. `apply_redactions` ve `sanitize_pdf_document` fail-closed `FEATURE_UNAVAILABLE` döndürür.
6. Plugin permission enforcement kanıtlanmamıştır.
7. Batch engine uygulanmamıştır; `add_batch_job` fail-closed `FEATURE_UNAVAILABLE` döndürür.
8. Viewer PDFium yolları uzun ömürlü viewer worker’a; OCR/AI extraction/export/print preparation/prepress/sensitive scan, sanitization inspection ve snapshot validation yolları ayrı ephemeral job worker’a bağlıdır. Sanitization transform (`sanitize_pdf_document`) ile diğer mock/disabled signature, redaction, searchable-PDF ve PDF/X handler’ları parse öncesi `FEATURE_UNAVAILABLE` döndürür.
9. Signature Settings ve Markdown notes export UI'ları gerçek persistence/write yapmadan başarı bildirir.
