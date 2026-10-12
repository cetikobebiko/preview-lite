# Tauri IPC Inventory

> Current-state note — 2026-07-28, Aşama 2: The detailed Prompt 0 inventory below
> is retained as historical evidence and its line numbers/counts are not the
> current public frontend matrix. The canonical active-PDF surface now uses
> `sessionId` plus `contentGeneration` (render) or
> `expectedContentGeneration` (read/job). Legacy path-native active-PDF
> contracts/wrappers/facades are removed from `contracts.js`, backend barrels,
> and `pdfState.js`.

Current path exceptions are deliberate: session-open/encryption preflight,
external operands such as merge inputs/images/batch inputs, and user-selected
output targets. Active PDF sharing remains
`share_file_macos(sessionId, expectedContentGeneration)`; independent files use
`share_external_file_macos(path)`. PDF page OCR remains
`perform_ocr_on_page(sessionId, expectedContentGeneration, pageIndex)`;
independent image OCR is unsupported. The public session snapshot omits source
and working-copy paths. Metadata reads use PDFium; metadata and alt-text writes
return `UNSUPPORTED_PDFIUM_CAPABILITY` without mutation.

Bu envanter `src-tauri/src/**/*.rs` altındaki bütün `#[tauri::command]`
tanımlarını, `src-tauri/src/lib.rs:970-1037` içindeki
`tauri::generate_handler![...]` kaydını ve `src/**/*.{svelte,js,ts}` altındaki
`invoke()` call-site'larını statik olarak karşılaştırır.

## Özet

| Ölçüm | Sayı |
|---|---:|
| Total Registered Commands in `generate_handler!` | 65 |
| Session PDF Operation Commands | 28 |
| Legacy Unsessioned Render Commands Removed | 4 (`render_page`, `render_thumbnail`, `set_active_render_document`, `cancel_render_request`) |
| Invalid / Broken IPC Call-sites | 0 |
| Contract Schema Coverage | 100% |

Beş fiziksel çift tanım `cfg(target_os = "macos")` ve
`cfg(not(target_os = "macos"))` dallarındaki
`trigger_continuity_camera_scan`, `speak_text`, `stop_speaking`,
`share_file_macos` ve `perform_ocr_on_page` command'larıdır.

Tauri'nin varsayılan frontend anahtar dönüşümünde Rust `snake_case`
parametreleri JavaScript `camelCase` olarak gönderilir. Aşağıdaki karşılaştırma
bu dönüşümü hesaba katar. Rust `State` ve `AppHandle` parametreleri Tauri
tarafından enjekte edildiği için istemci payload sütunundan çıkarılmıştır.

## Rust command ve handler envanteri

| Command | Rust istemci parametreleri | Tanım / koşul | Handler | Statik frontend call-site |
|---|---|---|:---:|---|
| `get_session_structure_tree` | `session_id: String; expected_content_generation: u64` | `src-tauri/src/document_session.rs`, pub async | evet | `src/components/A11yPanel.svelte` through `session.js` |
| `audit_session_accessibility` | `session_id: String; expected_content_generation: u64` | `src-tauri/src/document_session.rs`, pub async | evet | `src/components/A11yPanel.svelte` through `session.js` |
| `get_session_page_sizes` | `session_id: String; expected_content_generation: u64` | `src-tauri/src/document_session.rs`, pub async; worker `GetPageSizes` | evet | `src/services/pageSizeStore.js` through `viewer.js` (`getPageSizes`) — PdfPage placeholder heights |
| `build_semantic_index` | `job_id: String; session_id: String; expected_content_generation: u64; chunk_size: usize; chunk_overlap: usize; embed_model: String; request_id: String` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:39` |
| `semantic_search` | `session_id: String; expected_content_generation: u64; query: String; top_k: usize; embed_model: String; expected_chunk_size: usize; expected_chunk_overlap: usize; request_id: String` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:60` |
| `query_document_rag` | `session_id: String; expected_content_generation: u64; prompt: String; history: Vec<ChatMessage>; llm_model: String; embed_model: String; expected_chunk_size: usize; expected_chunk_overlap: usize; request_id: String` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:82` |
| `summarize_selection_or_page` | `text_content: String; target_length: String; llm_model: String; request_id: String; caller_attempt_id: String` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:101` |
| `configure_ai_endpoint` | `url: String; remote_approved: bool; private_network_approved: bool` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:7` |
| `preview_ai_endpoint` | `url: String` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:14` |
| `get_ai_endpoint_status` | — | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:19` |
| `clear_ai_endpoint_config` | — | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:21` |
| `cancel_ai_request` | `request_id: String; session_id: Option<String>; expected_content_generation: Option<u64>; caller_attempt_id: Option<String>` | `src-tauri/src/ai.rs`, pub async | evet | `src/services/backend/ai.js:23` |
| `add_annotation` | `file_path: String; page_index: u32; annot_type: String; rect: RectData; color: String; contents: Option<String>` | `src-tauri/src/annotations.rs:104`, pub sync | evet | `src/pdfState.js:119` |
| `delete_annotation` | `file_path: String; page_index: u32; annot_id: String` | `src-tauri/src/annotations.rs:167`, pub sync | evet | `src/pdfState.js:128` |
| `save_pdf_with_annotations` | `file_path: String; output_path: String` | `src-tauri/src/annotations.rs:199`, pub sync | evet | `src/pdfState.js:137` |
| `add_batch_job` | `job_type: String; file_path: String; parameters: String` | `src-tauri/src/batch.rs:71`, pub async | evet | `src/pluginStore.js:43` |
| `print_pdf` | `file_path: String; page_range: Option<Vec<u32>>` | `src-tauri/src/export.rs:9`, pub sync | evet | `src/pdfState.js:192` |
| `export_pages_as_images` | `file_path: String; pages: Vec<u32>; output_dir: String; format: String; dpi: f32` | `src-tauri/src/export.rs:78`, pub sync | evet | `src/pdfState.js:201` |
| `export_text_content` | `file_path: String; pages: Vec<u32>; format: String` | `src-tauri/src/export.rs:122`, pub sync | evet | `src/pdfState.js:210` |
| `export_pdf_page_to_image` | `path: String; page_index: u32; target_path: String; format: String` | `src-tauri/src/lib.rs:139`, private async | evet | `src/App.svelte:1127` |
| `get_pdf_metadata` | `path: String` | `src-tauri/src/lib.rs:189`, private sync | evet | `src/App.svelte:883`; `src/pdfState.js:154` |
| `set_pdf_metadata` | `source_path: String; metadata: PdfMetadata; target_path: String` | `src-tauri/src/lib.rs:216`, private sync | evet | `src/App.svelte:1097` |
| `trigger_continuity_camera_scan` | macOS `source_pdf_path: String`; diğer platform `_source_pdf_path: String` | `src-tauri/src/lib.rs:245/308`, private sync, platform `cfg` | evet | `src/App.svelte:1168` |
| `speak_text` | macOS `text: String; voice: Option<String>`; diğer platform `text: String; _voice: Option<String>` | `src-tauri/src/lib.rs:315/336`, private sync, platform `cfg` | evet | `src/App.svelte:1198` |
| `stop_speaking` | — | `src-tauri/src/lib.rs:353/360`, private sync, platform `cfg` | evet | `src/App.svelte:1182,1468` |
| `share_file_macos` | macOS `file_path: String`; diğer platform `_file_path: String` | `src-tauri/src/lib.rs:371/400`, private sync, platform `cfg` | evet | `src/App.svelte:1209` |
| `perform_ocr_on_page` | macOS `path: String; page_index: u32`; diğer platform `_path: String; _page_index: u32` | `src-tauri/src/lib.rs:407/463`, private async/sync, platform `cfg` | evet | `src/App.svelte:1147,1193,1220` |
| `extract_pages` | `source_path: String; page_indices: Vec<u32>; target_path: String` | `src-tauri/src/lib.rs:513`, private sync | evet | `src/App.svelte:1428` |
| `read_image_file` | `path: String; rotation: i32; flip_h: bool; flip_v: bool` | `src-tauri/src/lib.rs:567`, private async (`spawn_blocking`), `render-frame/1` binary yanıt | evet | `src/services/backend/document.js:4` |
| `convert_and_save_image` | `source_path: String; target_path: String; format: String; rotation: i32; flip_h: bool; flip_v: bool` | `src-tauri/src/lib.rs:618`, private async (`spawn_blocking`) | evet | `src/services/backend/document.js:7` |
| `reorder_pdf_pages` | `source_path: String; page_order: Vec<u32>; target_path: String` | `src-tauri/src/lib.rs:687`, private sync | evet | `src/App.svelte:1834` |
| `burn_annotations_to_pdf` | `source_path: String; annotations_json: String; target_path: String` | `src-tauri/src/lib.rs:783`, private sync | evet | `src/pdfState.js:14` |
| `rotate_page` | `file_path: String; page_index: u32; degrees: i32` | `src-tauri/src/manipulation.rs:14`, pub sync | evet | `src/pdfState.js:27` |
| `delete_pages` | `file_path: String; page_indices: Vec<u32>` | `src-tauri/src/manipulation.rs:37`, pub sync | evet | `src/pdfState.js:31` |
| `insert_blank_page` | `_file_path: String; _at_index: u32; _width: f32; _height: f32` | `src-tauri/src/manipulation.rs:64`, pub sync | evet | `src/pdfState.js:39` |
| `merge_pdfs` | `file_paths: Vec<String>; output_path: String` | `src-tauri/src/manipulation.rs:69`, pub sync | evet | `src/App.svelte:1810`; `src/pdfState.js:43` |
| `split_pdf` | `file_path: String; ranges: Vec<(u32,u32)>; output_dir: String` | `src-tauri/src/manipulation.rs:90`, pub sync | evet | `src/pdfState.js:47` |
| `get_form_fields` | `_file_path: String` | `src-tauri/src/manipulation.rs:130`, pub sync | evet | `src/pdfState.js:51` |
| `update_form_field` | `_file_path: String; _field_id: String; _value: String` | `src-tauri/src/manipulation.rs:135`, pub sync | evet | `src/pdfState.js:55` |
| `reorder_pages` | `file_path: String; new_order: Vec<u32>` | `src-tauri/src/manipulation.rs:140`, pub sync | evet | `src/pdfState.js:35` |
| `detect_scanned_page` | `file_path: String; page_index: u32` | `src-tauri/src/ocr.rs:20`, pub sync | evet | `src/pdfState.js:64` |
| `get_available_ocr_languages` | — | `src-tauri/src/ocr.rs:40`, pub sync | evet | `src/pdfState.js:73` |
| `run_ocr_on_page` | `file_path: String; page_index: u32; lang: String` | `src-tauri/src/ocr.rs:46`, pub sync | evet | `src/pdfState.js:81` |
| `export_as_searchable_pdf` | `_file_path: String; _output_path: String; _lang: String` | `src-tauri/src/ocr.rs:84`, pub sync | evet | `src/pdfState.js:85` |
| `check_pdf_encrypted` | `file_path: String` | `src-tauri/src/pdf_engine/mod.rs:57`, pub async | evet | `src/pdfState.js:146` |
| `authenticate_pdf_password` | `file_path: String; password: String` | `src-tauri/src/pdf_engine/mod.rs:71`, pub async | evet | `src/pdfState.js:5` |
| `get_page_count` | `path: String` | `src-tauri/src/pdf_engine/mod.rs:100`, pub async | evet | `src/App.svelte:860` |
| `render_page` | `path: String; page_index: u32; scale: f64; rotation: i32` | `src-tauri/src/pdf_engine/mod.rs:114`, pub async | evet | `src/components/PdfPage.svelte:354` |
| `render_pdf_page` | `file_path: String; page_index: u32; scale: f32; color_mode: Option<String>` | `src-tauri/src/pdf_engine/mod.rs:191`, pub async | evet | — |
| `render_thumbnail` | `path: String; page_index: u32; target_width: u32` | `src-tauri/src/pdf_engine/mod.rs:360`, pub async | evet | `src/components/PageOrganizer.svelte:94`; `src/components/ThumbnailItem.svelte:96` |
| `render_pdf_thumbnail` | `file_path: String; page_index: u32` | `src-tauri/src/pdf_engine/mod.rs:472`, pub async | evet | `src/components/Thumbnail.svelte:34` |
| `get_pdf_outline` | `file_path: String` | `src-tauri/src/pdf_engine/mod.rs:507`, pub async | evet | `src/App.svelte:871`; `src/components/Sidebar.svelte:24` |
| `clear_search_cache` | `_path: Option<String>` | `src-tauri/src/pdf_engine/mod.rs:533`, pub sync | evet | — |
| `search_pdf_text` | `path: String; query: String` | `src-tauri/src/pdf_engine/mod.rs:543`, pub async | evet | `src/pdfState.js:161` |
| `inspect_plugin_package` | `package_path: String` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `install_plugin_package` | `package_path: String; expected_digest: String` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `grant_plugin_consent` | `plugin_id: String; expected_package_digest: String; expected_permissions: Vec<String>` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `uninstall_plugin` | `plugin_id: String` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `set_plugin_enabled` | `plugin_id: String; enabled: bool` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `list_installed_plugins` | — | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `start_plugin_execution` | `plugin_id: String; hook_name: String; payload_json: String; execution_id: String` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `cancel_plugin_execution` | `execution_id: String` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `get_plugin_execution_status` | `execution_id: String` | `src-tauri/src/plugins/mod.rs`, pub async | evet | `src/pluginStore.js` |
| `inspect_session_print_resources` | `job_id; session_id; expected_content_generation; physical_page_index; request_id` | `document_session.rs` → isolated worker | evet | `PrepressPanel.svelte` |
| `generate_session_rgb_derived_coverage_preview` | identity + `threshold_percent; scale` | `document_session.rs` → isolated worker | evet | `PrepressPanel.svelte` |
| `generate_session_rgb_derived_process_preview` | identity + `scale` | `document_session.rs` → isolated worker | evet | `PrepressPanel.svelte` |
| `apply_redactions` | `job_id: String; session_id: String; expected_content_generation: u64; output_path: String; redactions: Vec<RedactionArea>` | `src-tauri/src/redaction.rs`, session-job | **hayır** | Legacy command parse/snapshot öncesi fail-closed `FEATURE_UNAVAILABLE`; secure pipeline'a alias değildir. |
| `plan_secure_redaction` | `job_id; session_id; expected_content_generation; policy; acknowledgements; regions` | `src-tauri/src/redaction.rs`, session-job + isolated planner worker | `src/services/app/secureRedactionController.js` | Canonical 8B planning operation; feature disabled olduğu için production UI runnable değildir. |
| `execute_secure_redaction` | `job_id; session_id; expected_content_generation; policy; acknowledgements; regions; expected_plan_digest; target_path` | `src-tauri/src/redaction.rs`, session-job + isolated transform/verifier workers | `src/services/app/secureRedactionController.js` | Canonical 8B secure export operation; feature disabled olduğu için production UI runnable değildir. |
| `sanitize_pdf_document` | `job_id: String; session_id: String; expected_content_generation: u64; output_path: String; options: SanitizationOptions` | `src-tauri/src/redaction.rs`, dormant legacy contract | **hayır** | 8C.3 sonrasında da parse/snapshot/worker/I/O öncesi fail-closed `FEATURE_UNAVAILABLE`. Broad boolean options ve `output_path` versioned policy authority'si değildir. |
| `inspect_document_sanitization` | `job_id: String; session_id: String; expected_content_generation: u64; policy_schema_version: String` | `src-tauri/src/sanitization.rs`, immutable session snapshot + ephemeral `InspectDocumentSanitization` worker | Feature-disabled canonical controller | Read-only typed `SessionJobOutcome<SanitizationInspectionReport>`; input digest + registry version bağlıdır; raw input/working/snapshot path, credential, output ve target kabul etmez. |
| `execute_document_sanitization` | `job_id: String; session_id: String; expected_content_generation: u64; policy: SanitizationPolicy; target_path: String` | `src-tauri/src/sanitization_publication.rs` | Feature-disabled canonical controller only | Versioned supported-control policy ve explicit target alır; raw input/working/snapshot path veya legacy boolean option kabul etmez. Actual immutable snapshot üzerinde inspection/policy revalidation → private transform worker → independent verifier worker → mandatory digest gate → target TOCTOU preflight → atomic publication uygular. Typed commit/recovery metadata `SessionJobOutcome` failure surface'ında korunur. |
| `scan_sensitive_data` | `job_id: String; session_id: String; expected_content_generation: u64; patterns: Vec<String>` | `src-tauri/src/redaction.rs`, session-job (Job-Worker Protocol v3) | evet | `src/services/backend/redaction.js` (`startSensitiveDataScanAction` → cancellable handle → job worker protocol v3) |

## Eşleşme sınıfları

### Tanımlı, handler'a kayıtlı ve frontend'den statik çağrılan

66 benzersiz command bu sınıftadır. Yukarıdaki tabloda `Handler = evet` ve
call-site sütunu dolu olan satırlardır. Her statik frontend command adı handler
listesinde bulunur.

### Handler'da kayıtlı fakat statik frontend çağrısı olmayan

- `render_pdf_page`
- `clear_search_cache`

`src/ipc_profiler.js` herhangi bir command adını dinamik olarak alabilir; ancak
repository taramasında bu wrapper'ın bir consumer'ı bulunmadı. Bu nedenle iki
command dinamik wrapper'a varsayımsal olarak atanmadı.

### Handler'da olup tanımı çözülemeyen

Yok.

### Frontend'de statik çağrılıp handler'da bulunmayan

Yok. Bununla birlikte command adının eşleşmesi payload sözleşmesinin doğru olduğu
anlamına gelmez; kesin uyuşmazlıklar aşağıdadır.

## Frontend call-site ve payload envanteri

Tablodaki payload'lar kaynakta gönderilen nesne ifadeleridir. Tauri'nin beklenen
camelCase dönüşümü uygulanınca eşleşen anahtarlar ayrıca işaretlenmemiştir.

| Call-site | Command | Payload |
|---|---|---|
| `src/App.svelte:860` | `get_page_count` | `{ path: filePath }` |
| `src/App.svelte:871` | `get_pdf_outline` | `{ filePath }` |
| `src/App.svelte:883` | `get_pdf_metadata` | `{ path: filePath }` |
| `src/App.svelte:979` | `read_image_file` | `{ path, rotation, flipH, flipV }` |
| `src/App.svelte:1097` | `set_pdf_metadata` | `{ sourcePath, metadata, targetPath }` |
| `src/App.svelte:1127` | `export_pdf_page_to_image` | `{ path, pageIndex, targetPath, format }` |
| `src/App.svelte:1147` | `perform_ocr_on_page` | `{ path, pageIndex }` |
| `src/App.svelte:1168` | `trigger_continuity_camera_scan` | `{ sourcePdfPath }` |
| `src/App.svelte:1182` | `stop_speaking` | — |
| `src/App.svelte:1193` | `perform_ocr_on_page` | `{ path, pageIndex }` |
| `src/App.svelte:1198` | `speak_text` | `{ text, voice }` |
| `src/App.svelte:1209` | `share_file_macos` | `{ filePath }` |
| `src/App.svelte:1220` | `perform_ocr_on_page` | `{ path, pageIndex }` |
| `src/App.svelte:1428` | `extract_pages` | `{ sourcePath, pageIndices, targetPath }` |
| `src/App.svelte:1468` | `stop_speaking` | — |
| `src/App.svelte:1810` | `merge_pdfs` | `{ pdfPaths, targetPath }` |
| `src/App.svelte:1834` | `reorder_pdf_pages` | `{ sourcePath, pageOrder, targetPath }` |
| `src/App.svelte:1917` | `convert_and_save_image` | `{ sourcePath, targetPath, format, rotation, flipH, flipV }` |
| `src/components/A11yPanel.svelte` | `audit_session_accessibility` | `{ sessionId, expectedContentGeneration }` |
| `src/components/A11yPanel.svelte` | `get_session_structure_tree` | `{ sessionId, expectedContentGeneration }` |
| `src/services/pageSizeStore.js` | `get_session_page_sizes` | `{ sessionId, expectedContentGeneration }` |
| `src/components/PageOrganizer.svelte:94` | `render_thumbnail` | `{ path, pageIndex, targetWidth }` |
| `src/components/PdfPage.svelte:278` | `get_page_text_layout` | `{ filePath, pageIndex }` |
| `src/components/PdfPage.svelte:354` | `render_page` | `{ path, pageIndex, scale, rotation }` |
| `src/services/backend/prepress.js` | `inspect_session_print_resources` | `{ jobId, sessionId, expectedContentGeneration, physicalPageIndex, requestId }` |
| `src/services/backend/prepress.js` | `generate_session_rgb_derived_process_preview` | `{ jobId, sessionId, expectedContentGeneration, physicalPageIndex, requestId, scale }` |
| `src/services/backend/prepress.js` | `generate_session_rgb_derived_coverage_preview` | `{ jobId, sessionId, expectedContentGeneration, physicalPageIndex, requestId, thresholdPercent, scale }` |
| `src/components/Sidebar.svelte:24` | `get_pdf_outline` | `{ filePath }` |
| `src/components/Thumbnail.svelte:34` | `render_pdf_thumbnail` | `{ filePath, pageIndex }` |
| `src/components/ThumbnailItem.svelte:96` | `render_thumbnail` | `{ path, pageIndex, targetWidth }` |
| `src/ipc_profiler.js:12` | dinamik `cmd` | `args` |
| `src/pdfState.js:5` | `authenticate_pdf_password` | `{ filePath, password }` |
| `src/pdfState.js:14` | `burn_annotations_to_pdf` | `{ filePath }` |
| `src/pdfState.js:27` | `rotate_page` | `{ filePath, pageIndex, degrees }` |
| `src/pdfState.js:31` | `delete_pages` | `{ filePath, pageIndices }` |
| `src/pdfState.js:35` | `reorder_pages` | `{ filePath, newOrder }` |
| `src/pdfState.js:39` | `insert_blank_page` | `{ filePath, atIndex, width, height }` |
| `src/pdfState.js:43` | `merge_pdfs` | `{ filePaths, outputPath }` |
| `src/pdfState.js:47` | `split_pdf` | `{ filePath, ranges, outputDir }` |
| `src/pdfState.js:51` | `get_form_fields` | `{ filePath }` |
| `src/pdfState.js:55` | `update_form_field` | `{ filePath, fieldId, value }` |
| `src/pdfState.js:64` | `detect_scanned_page` | `{ filePath, pageIndex }` |
| `src/pdfState.js:73` | `get_available_ocr_languages` | — |
| `src/pdfState.js:81` | `run_ocr_on_page` | `{ filePath, pageIndex, lang }` |
| `src/services/backend/ai.js:7` | `configure_ai_endpoint` | `{ url, remoteApproved, privateNetworkApproved }` |
| `src/services/backend/ai.js:14` | `preview_ai_endpoint` | `{ url }` |
| `src/services/backend/ai.js:19` | `get_ai_endpoint_status` | — |
| `src/services/backend/ai.js:21` | `clear_ai_endpoint_config` | — |
| `src/services/backend/ai.js:23` | `cancel_ai_request` | `{ requestId, sessionId, expectedContentGeneration, callerAttemptId }` |
| `src/services/backend/ai.js:39` | `build_semantic_index` | `{ jobId, sessionId, expectedContentGeneration, chunkSize, chunkOverlap, embedModel, requestId }` |
| `src/services/backend/ai.js:60` | `semantic_search` | `{ sessionId, expectedContentGeneration, query, topK, embedModel, expectedChunkSize, expectedChunkOverlap, requestId }` |
| `src/services/backend/ai.js:82` | `query_document_rag` | `{ sessionId, expectedContentGeneration, prompt, history, llmModel, embedModel, expectedChunkSize, expectedChunkOverlap, requestId }` |
| `src/services/backend/ai.js:101` | `summarize_selection_or_page` | `{ textContent, targetLength, llmModel, requestId, callerAttemptId }` |
| `src/pdfState.js:119` | `add_annotation` | `{ filePath, pageIndex, annotType, rect, color, contents }` |
| `src/pdfState.js:128` | `delete_annotation` | `{ filePath, pageIndex, annotId }` |
| `src/pdfState.js:137` | `save_pdf_with_annotations` | `{ filePath, outputPath }` |
| `src/pdfState.js:146` | `check_pdf_encrypted` | `{ filePath }` |
| `src/pdfState.js:154` | `get_pdf_metadata` | `{ filePath }` |
| `src/pdfState.js:161` | `search_pdf_text` | `{ path, query }` |
| `src/pdfState.js:192` | `print_pdf` | `{ filePath, pageRange }` |
| `src/pdfState.js:201` | `export_pages_as_images` | `{ filePath, pages, outputDir, format, dpi }` |
| `src/pdfState.js:210` | `export_text_content` | `{ filePath, pages, format }` |
| `src/pluginStore.js` | `inspect_plugin_package` | `{ packagePath }` |
| `src/pluginStore.js` | `install_plugin_package` | `{ packagePath, expectedDigest }` |
| `src/pluginStore.js` | `grant_plugin_consent` | `{ pluginId, expectedPackageDigest, expectedPermissions }` |
| `src/pluginStore.js` | `uninstall_plugin` | `{ pluginId }` |
| `src/pluginStore.js` | `set_plugin_enabled` | `{ pluginId, enabled }` |
| `src/pluginStore.js` | `list_installed_plugins` | — |
| `src/pluginStore.js` | `start_plugin_execution` | `{ pluginId, hookName, payloadJson, executionId }` |
| `src/pluginStore.js` | `cancel_plugin_execution` | `{ executionId }` |
| `src/pluginStore.js` | `get_plugin_execution_status` | `{ executionId }` |
| `src/pluginStore.js` | `add_batch_job` | `{ jobType, filePath, parameters }` |

`src/configManager.js` Tauri fs pluginini doğrudan kullanır, command invoke etmez.
`src/annotationsStore.js` yalnızca frontend store'dur. `src/ipc_profiler.js`
dinamik `invoke(cmd, args)` sağlar; ancak repository içinde import/consumer
bulunmadığı için üretebileceği command kümesi **unknown** kabul edildi.

## Payload uyuşmazlıkları

### Kesin uyuşmazlıklar

| Call-site | Gönderilen | Rust'ın beklediği | Etki |
|---|---|---|---|
| `src/pdfState.js:14` `burn_annotations_to_pdf` | yalnız `{ filePath }` | `sourcePath`, `annotationsJson`, `targetPath` | Wrapper çağrısı deserialize edilemez; annotation burn yolu erişilemez. |
| `src/pdfState.js:154` `get_pdf_metadata` | `{ filePath }` | `{ path }` | Wrapper hatalıdır. `src/App.svelte:883` aynı command'ı doğru `{ path }` ile çağırır. |
| `src/App.svelte:1810` `merge_pdfs` | `{ pdfPaths, targetPath }` | `{ filePaths, outputPath }` | Bu doğrudan UI yolu deserialize edilemez. `src/pdfState.js:43` wrapper payload'ı doğrudur. |

Güncel durum:
- `plan_secure_redaction` ve `execute_secure_redaction` production frontend'de yalnız canonical `secureRedactionController` üzerinden çağrılır; disabled feature gate nedeniyle kullanıcıya runnable kontrol sunulmaz.
- `apply_redactions` ve `sanitize_pdf_document` için production UI callsite bulunmamaktadır.
- `apply_redactions` ve `sanitize_pdf_document` parse/snapshot öncesi fail-closed `FEATURE_UNAVAILABLE` döndürmektedir.
- `scan_sensitive_data` iptal edilebilir session-job kontratı ve Job Worker Protocol v3 kullanmaktadır.

Bu bulgular statik payload anahtarlarının doğrudan karşılaştırılmasıyla kesindir;
build veya runtime gerekmez. Prompt 0 kapsamında düzeltilmediler.

### Macro çıktısı olmadan kesinleştirilemeyen anahtarlar

Aşağıdaki Rust parametrelerinin başında `_` vardır; frontend ise normal camelCase
anahtarı gönderir:

- `insert_blank_page`: `_file_path`, `_at_index`, `_width`, `_height`
- `get_form_fields`: `_file_path`
- `update_form_field`: `_file_path`, `_field_id`, `_value`
- `export_as_searchable_pdf`: `_file_path`, `_output_path`, `_lang`
- non-macOS platform stub'larındaki `_source_pdf_path`, `_voice`, `_file_path`,
  `_path`, `_page_index`

Tauri macro'sunun bu tanımlarda baştaki `_` karakterini IPC anahtarına dahil edip
etmediği compile/macro expansion olmadan doğrulanamadı. Bunlar kesin hata yerine
**potential mismatch / unknown** olarak tutuldu.

## Wrapper ve UI erişilebilirliği

| Wrapper | Bilinen consumer'lar | Not |
|---|---|---|
| `src/pdfState.js` | `App.svelte`, AI sidebar/context menu, OCR layer, export modal, page organizer, merge/split modal, annotation/form katmanları ve testler | Ana PDF/AI/OCR/export wrapper'ı; iki kesin payload hatası içerir. |
| `src/pluginStore.js` | `App.svelte`, `DeveloperConsole.svelte`, `PluginManager.svelte` | Plugin command'ları bağlıdır; `addBatchJob` için UI consumer bulunmadı. |
| `src/ipc_profiler.js` | Consumer bulunmadı | Command adı dinamik; statik olarak çözülemez ve normal UI'dan erişilebilirliği kanıtlanmadı. |

Bir command'ın handler'da bulunması runtime erişilebilirliği tek başına kanıtlamaz.
Payload sözleşmesi, wrapper consumer'ı, platform `cfg`, capability/plugin kaydı ve
native runtime gereksinimleri birlikte değerlendirilmiştir.

## Event IPC

Event'ler command sayılarına dahil edilmemiştir.

| Event | Producer | Payload | Consumer | Eşleşme | Test |
|---|---|---|---|---|---|
| `ai_stream_event` | `src-tauri/src/ai/stream_parser.rs` | `AiStreamEvent` (schema v1) | `src/services/app/aiController.js` | Tam uyumlu (Canonical) | Security Acceptance suite (`security_acceptance_tests.rs`, `aiController.test.js`) |
| `batch_job_status` | `src-tauri/src/batch.rs:40,49` | `JobStatus` object | Consumer bulunmadı | **backend-only** | Test yok / runtime çalışmadı |
| `tauri://drag-drop` | Tauri sistem event'i | frontend `.paths` okur | `src/App.svelte:658` | Sistem producer'ı bekleniyor | Runtime test edilmedi |

### AI Streaming Event Sözleşmesi (`ai_stream_event`)
Prompt 9A kapsamında eski global eventler (`ai_chat_token`, `ai_chat_done`, `ai_summary_token`, `ai_summary_done`) tamamen kaldırılmış ve yerini request-scoped, yapılandırılmış tekil event kanalı almıştır:

* **Sözleşme Alanları (Schema v1)**:
  - `schemaVersion: number` (1)
  - `requestId: string` (`req_<uuid_v4>`)
  - `operation: string` (`"chat"` | `"summary"`)
  - `sessionId?: string` (Session-bound RAG işlemleri için)
  - `contentGeneration?: number` (Belge nesil koruması)
  - `endpointFingerprint: string` (Onaylanmış endpoint bağıntısı)
  - `sequence: number` (İstek başına kesin monoton artan sayaç, `1..5000`)
  - `kind: "started" | "delta" | "completed" | "failed" | "cancelled"`
  - `delta?: string` (Metin parçası)
  - `result?: any` (Tamamlanma sonucu)
  - `error?: { code: string, message: string }`
  - `terminal: boolean` (Sonlandırıcı event bayrağı)

* **Garanti Edilen Semantik & Güvenlik İlkeleri**:
  - **Kesin Monoton Sıralama**: `sequence` düşüş veya tekrar durumlarında frontend `AI_STREAM_SEQUENCE_ERROR` ile fail-closed olur.
  - **Tekil Terminal Teslimat**: `TerminalDeliveryState` durum makinesi (`Pending -> Delivering -> Delivered`) ile yalnızca tek bir terminal event iletilir; terminal sonrası yeni event üretilmez/kabul edilmez.
  - **Cross-Request İzolasyonu**: Frontend ve backend istekleri `requestId` ile ayrıştırır; eski veya farklı isteğe ait parçalar hedefe karışamaz.
  - **Session/Generation Stale Koruması**: Oturum veya belge jenerasyonu değiştiğinde eski isteğe ait stream verileri frontend controller seviyesinde elenir.
  - **İptal & Dinleyici Temizliği**: `cancel_ai_request` çağrıldığında backend ağ future'ını sonlandırıp `cancelled` terminal eventi iletir; `aiController.dispose()` listener aboneliğini temizler.

## Yapılandırma ve platform erişilebilirliği

- Tauri builder dialog, fs, shell ve log pluginlerini kaydeder.
- Capability dosyaları dialog open/save, shell open ve geniş fs read/write
  izinleri içerir.
- Updater dependency'si görülür, fakat updater plugin kaydı görülmez.
- Continuity Camera, TTS, share ve macOS OCR command'larının macOS ve non-macOS
  dalları farklı davranır. Bu envanter macOS arm64 makinede çıkarılmış, runtime
  çalıştırılmamıştır.
- Bundled PDFium dylib arm64 macOS'tur. Diğer target'lardaki command
  erişilebilirliği native library bulunabilirliğine bağlıdır.

## Sonuç

İsim düzeyinde frontend–handler kapsamı yüksektir: 66 statik frontend command'ının
tamamı handler'a kayıtlıdır. Buna karşılık beş kesin payload sözleşmesi hatası,
iki kayıt dışı backend command, iki frontend consumer'ı olmayan handler ve bir
çözülemeyen dinamik wrapper vardır. Bu nedenle yalnız command adını görmek bir
özelliği `working` saymak için yeterli değildir.
## Candidate/session transition lifecycle commands

| Command | Payload | Result |
|---|---|---|
| `get_active_document_session_snapshot` | none | authoritative active snapshot or `null` |
| `probe_candidate_render` | `sessionId`, `expectedContentGeneration` | verified page count and first-page RGBA dimensions/byte length |
| `save_transition` | `token`, `targetPath: string \| null` | prepared old-session snapshot |
| `prepare_candidate_activation` | `candidateSessionId`, `previousSessionId`, `previousTransitionToken` | activation token and candidate snapshot |
| `commit_candidate_activation` | `activationToken` | `{ snapshot, cleanupWarnings }` |
| `commit_session_deactivation` | `transitionToken` | `{ previousSnapshot, cleanupWarnings }` |

The previous-session ID/token pair is either both null or both non-null. Backend
`active_session_id` is the authority for this validation.

### Snapshot-job command envelope

All commands routed through `run_snapshot_job()` additionally require a
frontend-generated `jobId`. Their terminal response is
`SessionJobOutcome<T> = completed | failed | cancelled`; all three variants
contain `jobId`, `sessionId`, captured `contentGeneration`, and cleanup status.
`cancel_session_job` requires `{ sessionId, jobId }`. Cleanup retry/deferred
warnings are path-free and do not replace the operation result.

## Page quarantine retry

| Command | Payload | Result |
|---|---|---|
| `retry_session_page` | `sessionId`, `expectedContentGeneration`, `pageIndex` | `boolean`: whether a page quarantine was lifted |

Explicit user action only (`viewer.js` `retryPage`, PdfPage "Yeniden dene"). It validates the
session lifecycle and generation (`STALE_GENERATION`), and the page index against the open worker
document (`PDFIUM_WORKER_INVALID_PAGE_INDEX`). It does not open the document or lift a document
quarantine (`PDFIUM_DOCUMENT_QUARANTINED`). Policy: `pdfium-worker-isolation.md`, "Restart and
quarantine".

## Render binary response (`render-frame/1`, Prompt 3)

`render_session_page` and `render_session_thumbnail` return their successful
result as a raw `tauri::ipc::Response` body (Tauri 2.11.5,
`InvokeResponseBody::Raw`). The webview receives an `ArrayBuffer`; there is no
JSON pixel array. Failures are unchanged: the command rejects with the JSON
`DocumentSessionError` (`{ code, message }`), including `render_cancelled`,
`render_superseded` and `render_inactive_document` messages.

| Command | Payload (unchanged) | Success body | Error body |
|---|---|---|---|
| `render_session_page` | `sessionId`, `pageIndex`, `scale`, `rotation`, `contentGeneration`, `requestId` | `render-frame/1` bytes | JSON `DocumentSessionError` |
| `render_session_thumbnail` | `sessionId`, `pageIndex`, `targetWidth`, `contentGeneration`, `requestId`, `scope` | `render-frame/1` bytes | JSON `DocumentSessionError` |

Wire format v1, all integers little-endian. Encoder:
`src-tauri/src/pdf_engine/render_wire.rs`; decoder:
`src/services/backend/renderFrame.js`. A golden frame is asserted on both sides.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic `AGRF` (`41 47 52 46`) |
| 4 | 2 | version `u16` = 1 |
| 6 | 4 | width `u32` (> 0) |
| 10 | 4 | height `u32` (> 0) |
| 14 | 4 | pageIndex `u32` |
| 18 | 8 | generation `u64` (<= `Number.MAX_SAFE_INTEGER`) |
| 26 | 2 | requestId byte length `u16` (1..=512; <= 128 characters) |
| 28 | n | requestId, UTF-8 |
| 28 + n | width × height × 4 | RGBA pixels |

The decoder rejects short buffers, wrong magic/version, invalid metadata and
any total length other than `28 + n + width × height × 4` with
`render_invalid_rgba_payload` (normalized to `BackendCommandError` by
`callBackend`). The decoded object keeps the previous shape
`{ width, height, data, pageIndex, generation, requestId }`, so
`validateRenderResponse` stale-result checks (requestId / generation /
pageIndex) and cancellation handling are unchanged. `data` is a zero-copy
`Uint8ClampedArray` view over the IPC buffer; `transformRgbaBuffer` paints it
without copying in `normal` mode and makes exactly one copy for color modes.

Contract schema: both contracts in `contracts.js` carry
`response: { type: "binary", format: "render-frame/1", decode }`; payload
schemas are unchanged, so contract schema coverage stays 100%.

Transport note: on Tauri's postMessage fallback (used only when the custom
`ipc://` protocol fails) a raw body is delivered as a number array; the
decoder accepts it, but that path does not get the size benefit.

## Görsel komutları (Prompt 6)

`read_image_file` ve `convert_and_save_image` artık `async`; görsel
decode/transform/encode işi `run_image_job` üzerinden
`tokio::task::spawn_blocking` içinde çalışır, Tauri async runtime thread'leri
ve ana thread bloklanmaz. Payload'lar değişmedi. Hatalar eskisi gibi düz
`String` döner (`Görsel dosyası bulunamadı…`, `Görsel okunamadı…`,
`Görsel açılamadı…`, `Desteklenmeyen format…`); blocking görev panic ederse
`Görsel işlemi tamamlanamadı…` döner.

| Command | Success body | Error body |
|---|---|---|
| `read_image_file` | `render-frame/1` bytes (`tauri::ipc::Response`) | JSON string |
| `convert_and_save_image` | `null` (`()`) | JSON string |

`read_image_file` yeni format icat etmez; `render_wire::encode_render_frame`
ile yukarıdaki `render-frame/1` çerçevesini üretir. Görsel bir oturum
olmadığından `pageIndex = 0`, `generation = 0`, `requestId = "read_image_file"`
sabit yer tutuculardır. `contracts.js` içindeki `readImageFile` kontratı
`response: { type: "binary", format: "render-frame/1", decode }` taşır;
decoder `decodeRenderFrame` çıktısından yalnız `{ width, height, data }`
döndürür, böylece çağıranlar (`documentOpenController.prepareImage`,
`viewerController.renderImage` → `paintBufferToCanvas`) önceki şekli görür;
`data` artık sayı dizisi değil, IPC buffer'ı üzerinde kopyasız
`Uint8ClampedArray`'dır.

Bellek sınırı: her iki komut da görseli `open_bounded_image` ile açar ve PDF
render sınırlarını paylaşır (`render_service::MAX_PIXEL_DIMENSION`,
`MAX_RENDER_BYTES`): kenar başına en fazla 16384 px, decoder tamponu
(`total_bytes`) ve RGBA çıktısı (`width × height × 4`) için en fazla 256 MiB
(≈ 67 MP). Boyutlar yalnız başlıktan okunup piksel tamponu ayrılmadan
denetlenir; aşımda `Görsel çok büyük: en fazla 16384×16384 piksel ve 256 MiB
çözümlenmiş veri desteklenir.` hatası döner (`image::ImageError::Limits` de
aynı mesaja eşlenir). Decode sonrası rotate/flip, `to_rgba8` ve çerçeve
kodlaması sınır içindeki görsel için ek kopyalar üretir.

## AI Subsystem IPC Envanteri (Prompt 9A.11)

| Command | Payload | Result | Açıklama |
|---|---|---|---|
| `preview_ai_endpoint` | `{ url: string }` | `AiEndpointProfile` | Presentation-only endpoint URL doğrulama ve sınıflandırma |
| `configure_ai_endpoint` | `{ url: string, remoteApproved: boolean, privateNetworkApproved: boolean }` | `AiEndpointProfile` | Authoritative endpoint doğrulama, DNS sabitleme, yetkilendirme ve kaydetme |
| `get_ai_endpoint_status` | none | `AiEndpointProfile` | Aktif endpoint profilini döndürür |
| `clear_ai_endpoint_config` | none | `AiEndpointProfile` | Yapılandırmayı temizler ve loopback varsayılanına döner |
| `cancel_ai_request` | `{ requestId: string, sessionId?: string, expectedContentGeneration?: number, callerAttemptId?: string }` | `()` | AI isteğini atomik olarak iptal eder |
| `build_semantic_index` | `{ jobId: string, sessionId: string, expectedContentGeneration: number, chunkSize: number, chunkOverlap: number, embedModel: string, requestId: string }` | `SessionJobOutcome<()>` | Belge metnini parçalayıp embedding vektörlerini staging üzerinden atomik derler |
| `semantic_search` | `{ sessionId: string, expectedContentGeneration: number, query: string, topK: number, embedModel: string, expectedChunkSize: number, expectedChunkOverlap: number, requestId: string }` | `Vec<SearchResultChunk>` | Belge vektör indeksinde kosinüs benzerliği araması |
| `query_document_rag` | `{ sessionId: string, expectedContentGeneration: number, prompt: string, history: Vec<ChatMessage>, llmModel: string, embedModel: string, expectedChunkSize: number, expectedChunkOverlap: number, requestId: string }` | `()` (Stream) | Belge bağlamlı akışlı sohbet yanıtı üretir |
| `summarize_selection_or_page` | `{ textContent: string, targetLength: string, llmModel: string, requestId: string, callerAttemptId: string }` | `()` (Stream) | Metin seçimi veya sayfa özetini akışlı üretir |
