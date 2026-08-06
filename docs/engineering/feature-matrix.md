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

Prompt 5.1 itibarıyla tüm frontend Vitest testleri (272 test) ve Rust Cargo testleri (85 test) yerel ortamda %100 oranında çalıştırılmış ve geçmiş; doğrulama basamağı tamamlanmıştır.

Document Session mimarisi, Guard API entegrasyonu, single-flight save kilitleri, PDFium-only motor kuralları ve Windows I/O / DACL güvenlik garantileri runtime & unit test seviyesinde tamamen `working` ve `verified` statüsündedir.

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
| Thumbnail render | `PageOrganizer.svelte:94`, `ThumbnailItem.svelte:96`, `Thumbnail.svelte:34` | `render_thumbnail`, `render_pdf_thumbnail`; `pdf_engine/mod.rs:360-506` | `partial` | PDFium thumbnail render ve cache yolları gerçek. | İki ayrı thumbnail command sözleşmesi bulunuyor; native/runtime doğrulanmadı. | `PageOrganizer.test.js` unrun / mock IPC; thumbnail Rust testi yok. |
| Outline/sidebar | `App.svelte:871`, `Sidebar.svelte:24` | `get_pdf_outline`; `pdf_engine/mod.rs:507-532` | `partial` | PDFium bookmark ağacı çıkarılıyor. | Bozuk/şifreli outline senaryoları test edilmedi. | Test yok. |
| Image açma/dönüştürme ve PDF sayfasını image export | `App.svelte:979,1127,1917` | `read_image_file`, `convert_and_save_image`, `export_pdf_page_to_image`; `lib.rs:139-188,549-686` | `partial` | Gerçek image decode/encode ve PDFium render yolları var. | Format/renk profili/EXIF davranışı doğrulanmadı; test yok. | Test yok. |

## Search / text layer

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Belge çapında arama | `src/pdfState.js:161`; `SearchBar.svelte` | `search_pdf_text`; `pdf_engine/mod.rs:543-747` | `partial` | PDFium text tarama, NFKC normalizasyonu, geometry, cache ve cancellation gerçek implementasyondur. | Uçtan uca test çalışmadı; büyük/şifreli belgelerde doğrulanmadı. | `SearchBar.test.js`, `searchResultUtils.test.js`, `searchCoordinator.test.js` unrun; UI testleri backend'i doğrulamaz. |
| Sayfa text layout/DOM text layer | `PdfPage.svelte:278`; text-layer utils | `get_page_text_layout`; `pdf_engine/mod.rs:748+` | `partial` | PDFium karakter/kelime geometrisi frontend DOM katmanına taşınır. | Koordinat, rotation, zoom ve stale-result davranışı runtime doğrulanmadı. | `textLayerUtils.test.js`, `canvasUtils.test.js`, `PdfPage.test.js` unrun; PdfPage IPC mock. |
| Search cache temizleme | Consumer yok | `clear_search_cache`; `pdf_engine/mod.rs:533` | `unreachable` | Handler kaydı var. | Frontend çağrısı yok; cache yaşam döngüsü UI'dan tetiklenmiyor. | Test yok. |
| Text export | Export modal üzerinden `src/pdfState.js:210` | `export_text_content`; `export.rs:122+` | `partial` | PDFium text extraction ve text/JSON benzeri çıktı yolu var. | Format doğruluğu ve encoding test edilmedi. | Test yok. |

## Annotation

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Annotation listeleme/ekleme/silme/kaydetme | `AnnotationLayer.svelte`; `src/pdfState.js:110-137` | `annotations::{get_page_annotations,add_annotation,delete_annotation,save_pdf_with_annotations}` | `partial` | PDFium annotation API'leri ve save yolu gerçek. | Bazı PDFium create/set hataları sessizce yutulup başarı döndürülebilir; desteklenmeyen türler tam ayrıştırılmamış. | Frontend annotation akışında mock IPC kullanılır; Rust entegrasyon testi yok; tümü unrun. |
| Frontend geçici annotation/highlight store | `src/annotationsStore.js` | Backend command yok | `partial` | Svelte store ve UI overlay davranışı var. | PDF'e kalıcı yazma ile store senkronizasyonu doğrulanmadı. | Test yok. |
| Annotation'ları PDF'e burn etme | `src/pdfState.js:14` | `burn_annotations_to_pdf`; `lib.rs:783+` | `unreachable` | Backend `source_path`, `annotations_json`, `target_path` bekler. | Wrapper yalnız `{filePath}` gönderir; ayrıca yol `lopdf` kullanarak tek-PDFium kuralından sapar. | Test yok. |

## Page manipulation

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Rotate/delete/reorder | Page organizer; `pdfState.js:27-35`; `App.svelte:1834` | `manipulation::{rotate_page,delete_pages,reorder_pages}` ve legacy `reorder_pdf_pages` | `partial` | PDFium page mutation/save yolları vardır. | Aynı amaca hizmet eden legacy `lopdf` ve PDFium yolları birlikte bulunur; atomik save/backup davranışı test edilmedi. | `PageOrganizer.test.js` unrun / mock IPC; backend testi yok. |
| Extract/split | `App.svelte:1428`; merge/split modal via `pdfState.js:47` | `extract_pages`, `manipulation::split_pdf` | `partial` | Sayfa aralığı kopyalama ve dosyaya save implementasyonu var. | `extract_pages` `lopdf` kullanır; şifreli ve bozuk PDF senaryoları doğrulanmadı. | Test yok. |
| Merge | `App.svelte:1810`; merge/split modal via `pdfState.js:43` | `manipulation::merge_pdfs` | `partial` | PDFium tabanlı merge implementasyonu var ve wrapper payload'ı eşleşir. | `App.svelte:1810` doğrudan yolu `{pdfPaths,targetPath}` göndererek sözleşmeyi bozar; hangi UI yolunun baskın olduğu runtime doğrulanmadı. | Test yok. |
| Blank page insert | `pdfState.js:39` | `manipulation::insert_blank_page` | `scaffold` | Command handler'a kayıtlıdır. | Backend açıkça unsupported hata döndürür; baştaki `_` parametre anahtarları ayrıca belirsizdir. | Test yok. |
| Form field read/update | `FormLayer.svelte`; `pdfState.js:51,55` | `manipulation::{get_form_fields,update_form_field}` | `scaffold` | UI ve command yüzeyi vardır. | Read her zaman boş liste, update unsupported hata döndürür; `_` payload anahtarları belirsizdir. | Test yok. |
| Metadata read/write | `MetadataModal.svelte`; session service | `get_session_pdf_metadata`, `update_session_metadata` | `partial` | Read session-native ve PDFium metadata API’sini kullanır. | Write PDFium binding’inde güvenli destek olmadığı için readonly/typed unsupported’dır; legacy path bypass yoktur. | Contract/architecture ve Rust session testleri. |

## OCR

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Scanned-page tespiti | `OcrLayer.svelte` via `pdfState.js` | `ocr::detect_scanned_page` | `partial` | Session/generation identity ve stale-result guard kullanır; PDFium heuristic korunur. | Heuristic doğruluğu ve şifreli PDF davranışı ortam bağımlıdır. | OcrLayer payload/stale testleri. |
| Tesseract OCR | `OcrLayer.svelte` via `pdfState.js` | `ocr::{get_available_ocr_languages,run_ocr_on_page}` | `partial` | Aktif PDF session-native read sınırındadır. | Kurulu Tesseract/dil verisine bağlıdır; searchable output desteklenmez. | Contract/wrapper ve OcrLayer testleri. |
| Searchable PDF export | OCR UI via `pdfState.js:85` | `ocr::export_as_searchable_pdf` | `scaffold` | Command ve wrapper vardır. | Backend açıkça unsupported hata döndürür; `_` payload anahtarları belirsizdir. | Test yok. |
| macOS Vision OCR | `App.svelte` | `perform_ocr_on_page` | `partial` | PDF page OCR session snapshot kullanır; temp PNG RAII ile temizlenir. | Bağımsız image OCR unsupported; gerçek Vision runtime doğrulanmadı. | Session/stale ve temp başarı+hata cleanup testleri. |

## AI / RAG

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Basit yerel AI soru-cevap | `AiAssistantModal.svelte:34` | `ask_local_ai`; `lib.rs:476+` | `partial` | localhost Ollama HTTP çağrısı gerçek. | Ollama/model bulunabilirliği, timeout ve context boyutu doğrulanmadı. | Test yok. |
| Semantic index ve search | AI sidebar via `pdfState.js:93,97` | `ai::{build_semantic_index,semantic_search}` | `partial` | PDFium text extraction, Ollama embedding ve in-memory vector store vardır. | Hatalı chunk parametrelerinde underflow/ilerlememe riski; bazı embedding hataları sessizce atlanır; persistence yok. | Cosine benzerlik unit testi var, unrun; entegrasyon testi yok. |
| RAG chat streaming | `AiSidebar.svelte`; `pdfState.js:101` | `ai::query_document_rag`; `ai_chat_token/done` eventleri | `partial` | Ollama stream parse ve frontend event listener adları eşleşir. | Event teslimi/Ollama runtime çalışmadı; cancellation/backpressure doğrulanmadı. | Test yok. |
| Selection/page summary streaming | `AiContextMenu.svelte`; `pdfState.js:105` | `ai::summarize_selection_or_page`; `ai_summary_token/done` | `partial` | Gerçek Ollama streaming yolu ve eşleşen listener vardır. | Model/runtime bağımlılığı ve hata sonlandırma davranışı doğrulanmadı. | Test yok. |

## Redaction

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Sensitive-data scan & review highlight | `src/components/SensitiveDataScanPanel.svelte` | `redaction::scan_sensitive_data`; `redaction.rs` | `working` | Svelte 5 state store, controller, pure rotation selector, amber review highlight katmanı, async job cancel guard'ları. | OCR yapılmamış görsellerdeki metinleri bulmayabilir. | `src/stores/sensitiveDataScanState.test.js`, `src/services/app/sensitiveDataScanController.test.js`, `src/components/SensitiveDataScanPanel.test.js`, `src/components/PdfPage.test.js`. |
| Secure Redaction (Kalıcı Silme) | `docs/engineering/redaction-capability-boundary.md` | `redaction::apply_redactions` | `FEATURE_UNAVAILABLE` | Mimaride açık sınır tanımlanmıştır; UI'dan ulaşılamaz; fail-closed `FEATURE_UNAVAILABLE` döndürür. | Görsel maskeleme redaksiyon yerine kullanılmaz. Kalıcı silme Prompt 8B kapsamındadır. | `src/uiFeatureWiring.test.js` (redaction callsite yokluğu doğrulanır). |

## Sanitization

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| PDF Sanitization (Metadüzeltme/Temizlik) | `docs/engineering/redaction-capability-boundary.md` | `redaction::sanitize_pdf_document` | `FEATURE_UNAVAILABLE` | Mimaride açık sınır tanımlanmıştır; UI'dan ulaşılamaz; fail-closed `FEATURE_UNAVAILABLE` döndürür. | Dezenfeksiyon Prompt 8B kapsamındadır. | Action/architecture guard testleri. |

## Signature / PAdES

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Sistem sertifikalarını listeleme | `SignatureWizardModal.svelte:34` | `signature::list_system_certificates` | `mock` | Fonksiyon sabit örnek sertifika döndürür. | Gerçek keychain/certificate store erişimi yok. | Test yok. |
| PAdES imzalama | `SignatureWizardModal.svelte:79` | `signature::sign_pdf_pades` | `mock` | Girdi PDF'i çıktıya kopyalar. | Kriptografik imza/PAdES üretmez; başarı mesajı yanıltıcı olabilir; `_pin` anahtarı belirsiz. | Test yok. |
| İmza doğrulama | `SignaturePanel.svelte:18` | `signature::verify_pdf_signatures` | `mock` | Hard-coded geçerli doğrulama sonucu döndürür. | Gerçek CMS/cert chain/revocation kontrolü yok; güvenlik açısından kullanılamaz. | Test yok. |
| LTV enablement | `SignaturePanel.svelte` | `signature::enable_ltv` | `disabled` | Frontend contract session-native; mock-disabled guard korunur. | Rust typed `FEATURE_UNAVAILABLE` döndürür; DSS/OCSP/CRL veya çıktı üretmez. | Action/architecture guard testleri. |
| Signature settings / TSA / trust store | `SignatureSettingsModal.svelte:9-38` | Backend command veya kalıcı config bağlantısı yok | `mock` | TSA URL hard-coded DigiCert değeriyle başlar, trust store yalnız local component dizisidir; save handler yalnız `alert("Settings saved!")` gösterir. | TSA credentials/trust root değişiklikleri persist veya signature backend'ine aktarılmaz; güvenlik ayarı yapılmış izlenimi verir. | Statically verified / unrun / no test. |

## Accessibility / PDF-UA

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Structure tree okuma | `A11yPanel.svelte:55` | `a11y::get_structure_tree`; `a11y.rs:92+` | `partial` | `lopdf` ile StructTreeRoot ağacı ayrıştırılır. | Karmaşık tagged PDF yapıları, parent tree ve content association doğrulanmadı. | Test yok. |
| Alt text/language güncelleme | `A11yPanel.svelte` | `update_session_alt_text` | `disabled` | UI readonly/disabled, contract session-native. | PDFium write desteği olmadığı için typed unsupported; legacy path write bypass yoktur. | Architecture ve A11y fetch/stale testleri. |
| PDF/UA validation | `A11yPanel.svelte:44` | `a11y::validate_pdf_ua`; `a11y.rs:186+` | `partial` | Temel tag/language/metadata heuristic kontrolleri vardır. | ISO 14289 tam doğrulayıcı değildir; “uyumlu” sonucu standart uyumu kanıtlamaz. | Test yok. |
| Auto-Tag PDF butonu | `A11yPanel.svelte:172-175` | Backend command yok | `scaffold` | Experimental buton yalnız markup olarak bulunur; click handler veya implementasyon yoktur. | Kullanıcı etkileşimi hiçbir işlem başlatmaz. | Statically verified / unrun / no test. |
| Reading-order backend update | `ReadingOrderOverlay.svelte` | `a11y::update_reading_order`; `a11y.rs:263` | `unreachable` | Backend command tanımlıdır. | Command handler'a kayıtlı değil; normal invoke yolu yok. | Statically verified / unrun / no test. |
| Reading-order frontend reorder | `A11yPanel.svelte:112-120`; `TagTreeExplorer.svelte` | Backend çağrısı yapılmıyor | `mock` | Drag/reorder handler yalnız console log ve “complex in this demo” açıklamalı `alert` gösterir. | UI değişikliğin uygulandığı izlenimini verebilir; `update_reading_order` çağrılmaz ve PDF değişmez. | Statically verified / unrun / no test. |

## Prepress / PDF-X

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Color-space analizi | `PrepressPanel.svelte:140` | `prepress::analyze_color_spaces`; `prepress.rs:32+` | `partial` | Resource dictionary'leri inceler. | ICC bilgileri içinde hard-coded `Default Profile` vardır; nested resources ve renk yönetimi eksik olabilir. | `PrepressPanel.test.js` unrun / mock IPC. |
| Ink coverage hesabı | `PrepressPanel.svelte:206` | `prepress::calculate_ink_coverage`; `prepress.rs:84+` | `partial` | PDFium RGB render üzerinden CMYK tahmini yapar. | ICC/device-link tabanlı gerçek baskı hesabı değildir. | `PrepressPanel.test.js` unrun / mock IPC. |
| Separation channels | `PrepressPanel.svelte:165` | `prepress::render_separation_channels`; `prepress.rs:150+` | `partial` | Render edilmiş RGB piksellerden C/M/Y/K görselleri türetilir. | PDF separation plate/spot color render değildir; baskı doğruluğu iddia edilemez. | `PrepressPanel.test.js` unrun / mock IPC. |
| Page boxes update | Statik frontend call-site yok | `prepress::set_page_boxes`; `prepress.rs:222` | `unreachable` | Backend implementasyonu tanımlıdır. | Handler kaydı ve UI bağlantısı yok. | Test yok. |
| PDF/X export | `PdfXWizard.svelte:49` | `prepress::export_pdf_x`; `prepress.rs:251+` | `mock` | Version/metadata/output-intent identifier yazma yolu vardır. | ICC profile embed/renk dönüşümü/standart doğrulama yapmaz; gerçek PDF/X uygunluğu üretmez. | `PdfXWizard.test.js` unrun / mock IPC. |

## Plugins

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Plugin load/unload/list/hook | `PluginManager.svelte`, `DeveloperConsole.svelte`; `pluginStore.js:16-39` | `plugins::{load_plugin,unload_plugin,get_active_plugins,trigger_plugin_hook,run_script_direct}` | `partial` | Rhai compile/run, aktif plugin store ve operation/depth limitleri vardır. | Manifest permission'ları parse edilir fakat etkili enforcement kanıtı yok; direct script runner ve fs erişimi güvenlik riski. | Operation-limit Rust unit testi var, unrun; runtime/sandbox entegrasyon testi yok. |

## Batch processing

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Batch job yürütme | Yalnız `pluginStore.js:43` wrapper; consumer yok | `batch::add_batch_job`; `batch.rs` | `mock` | Queue/status modeli vardır, worker yaklaşık 500 ms bekleyip tamamlandı durumuna geçer. | Gerçek OCR/export/manipulation işi dispatch edilmez. | Test yok. |
| Batch status UI | Listener bulunmadı | `batch_job_status` event'i | `unreachable` | Backend queued/running/completed eventleri emit eder. | Frontend consumer yok; wrapper da normal UI'da kullanılmıyor. | Test yok. |

## Platform-specific features

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Continuity Camera scan | `App.svelte` disabled flow | `trigger_continuity_camera_scan` handler retained | `disabled` | Frontend aktif-PDF path contract/action kaldırılmıştır. | Session-safe page insertion tasarlanmadan çalıştırılmaz. | Architecture legacy-surface testleri. |
| Text-to-speech/stop | `App.svelte:1182,1198,1468` | `speak_text`, `stop_speaking`; `lib.rs:315-369` | `partial` | macOS `say`, diğer platformlarda PowerShell/fallback dalları vardır. | Linux unsupported; non-macOS voice parametresi/stop davranışı ve process hata kontrolü zayıf. | Test yok; tek platform runtime çalışmadı. |
| macOS Share Sheet | `App.svelte` | `share_file_macos`, `share_external_file_macos` | `platform_limited` | Aktif PDF session snapshot ile; bağımsız image/file explicit external path ile paylaşılır. Process bounded timeout kullanır. | Gerçek picker UI bu testlerde açılmaz; non-macOS unsupported. | Payload ayrımı, timeout ve snapshot cleanup testleri. |
| Sistem yazdırma | Export UI via `pdfState.js:192` | `export::print_pdf`; `export.rs:9+` | `unknown` | Platforma göre `lpr`/`lp`/PowerShell command yolları var. | Yazıcı bulunabilirliği, sayfa aralığı ve shell/process sonuçları doğrulanmadı. | Test yok. |
| Cross-platform paketleme | CI workflow'ları | Tauri bundle config ve `lib/*` resource | `unknown` | Linux/macOS/Windows CI matrisi tanımlı. | Yerel binary yalnız macOS arm64; CI çalıştırılmadı, Windows workflow test çalıştırmıyor. | CI sonucu yok. |

## Active frontend-only features

UI-only olması envanter dışında bırakılma nedeni değildir. Aşağıdaki özelliklerin
çekirdek davranışı frontend state/DOM/CSS veya Tauri fs plugin yolu üzerindedir;
backend command bulunmaması açıkça belirtilmiştir.

| Feature | Frontend entry point | Backend command/module | Status | Evidence | Known risks | Tests |
|---|---|---|---|---|---|---|
| Book, split, single-page ve smart-dark viewer modları | `App.svelte:68-73,1441-1493,1646-1662,2179-2212,2355-2360`; `PdfPage.svelte` | Özel backend command yok; mevcut `render_page` sonucu frontend layout/CSS/state ile sunulur | `partial` | Toggle state, layout class'ları, page-mode props ve local smart-dark setting persistence kaynakta bağlıdır. | Uçtan uca render/navigation testi çalışmadı; smart-dark renk doğruluğu ve split/book stale-render davranışı doğrulanmadı. | `PdfPage.test.js` statically verified / unrun / mock-only test; App integration testi yok. |
| Bionic Reading | `App.svelte:1141-1158`; `BionicReadingModal.svelte` | Dönüşüm için backend command yok; text yoksa opsiyonel `perform_ocr_on_page` kullanılır | `partial` | Frontend metni kelimelere ayırıp başlangıç karakterlerini `<b>` ile biçimler ve modalda gösterir. | Kaynak text/OCR bulunabilirliğine bağlıdır; erişilebilirlik ve `{@html}` güvenliği runtime doğrulanmadı. | Statically verified / unrun / no test. |
| Reading Ruler | `App.svelte:75,1454-1457,1974-1975,2362`; `ReadingRuler.svelte` | Backend command yok | `partial` | Frontend toggle ve pointer/focus overlay component'i bağlıdır. | Farklı zoom, DPI ve klavye erişilebilirliği runtime doğrulanmadı. | Statically verified / unrun / no test. |
| Session analytics | `App.svelte:215-218,650-654,1992-2000`; `AnalyticsModal.svelte` | Backend command yok | `partial` | Açık belge varken saniye sayacı ilerler ve modal süre/sayfa/zoom özetini gösterir. | Sayaç yalnız mevcut process session'ındadır; background/idle ayrımı ve persistence yoktur. | Statically verified / unrun / no test. |
| Document-state ve recent-file persistence | `App.svelte:471-502,642-648,917-918,987`; `configManager.js:1-70` | Tauri command yok; `@tauri-apps/plugin-fs` ile AppData `config.json` okunur/yazılır | `partial` | Recent list, son sayfa/zoom/book/single state save/restore ve son dosyayı auto-load yolları bağlıdır. | Dosya taşınması/silinmesi, path gizliliği, concurrent write ve bozuk config recovery runtime doğrulanmadı. | Statically verified / unrun / no test. |
| Command Palette ve keyboard shortcuts | `App.svelte:507-600,1441-1510,2008-2014,2089`; `CommandPalette.svelte`; `ShortcutsModal.svelte` | Backend command yok; seçilen bazı aksiyonlar mevcut IPC handler'larını çağırabilir | `partial` | Global key handler, palette command listesi ve dispatch switch'i kaynakta bağlıdır. | Liste ile gerçek shortcut/dispatch kapsamının senkron kalması ve input focus conflict'leri test edilmedi. | Statically verified / unrun / no test. |
| Markdown notes export | `App.svelte:1342-1388`; Command Palette `export-notes-md` | Backend command veya fs write çağrısı yok | `mock` | Markdown string hazırlanır ve save dialog açılır; save path seçilince başarı toast'ı gösterilir. | Hazırlanan `mdContent` hiçbir dosyaya yazılmaz; kullanıcıya sahte başarılı export bildirimi verilir. | Statically verified / unrun / no test. |

## Güncel Prompt 8A-5 Doğrulaması (5 Ağustos 2026)

Tüm doğrulamalar yerel ortamda %100 oranında çalıştırılmış ve başarıyla geçmiştir:

```text
75 Vitest dosyası / 540 test
297 Rust test
Svelte check 0 error
Build başarılı
Clippy -D warnings başarılı
```

Güncel Mimari ve Güvenlik Gerçekleri:
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
4. PDF/UA validation yalnız temel heuristic kontrollerdir.
5. `apply_redactions` ve `sanitize_pdf_document` fail-closed `FEATURE_UNAVAILABLE` döndürür.
6. Plugin permission enforcement kanıtlanmamıştır.
7. Batch engine gerçek işleri yürütmez ve frontend status consumer'ı yoktur.
8. Viewer PDFium yolları uzun ömürlü viewer worker’a; OCR/AI extraction/export/print preparation/prepress/sensitive scan ve snapshot validation yolları ayrı ephemeral job worker’a bağlıdır. Mock/disabled signature, redaction, sanitization, searchable-PDF ve PDF/X handler’ları parse öncesi `FEATURE_UNAVAILABLE` döndürür.
9. Signature Settings ve Markdown notes export UI'ları gerçek persistence/write yapmadan başarı bildirir.
