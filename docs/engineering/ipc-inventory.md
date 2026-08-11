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
| Total Registered Commands in `generate_handler!` | 64 |
| Session PDF Operation Commands | 27 |
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
| `get_structure_tree` | `file_path: String` | `src-tauri/src/a11y.rs:92`, pub sync | evet | `src/components/A11yPanel.svelte:55` |
| `update_alt_text` | `file_path: String; object_id_str: String; alt_text: Option<String>; language: Option<String>; target_path: String` | `src-tauri/src/a11y.rs:144`, pub sync | evet | `src/components/A11yPanel.svelte:98` |
| `validate_pdf_ua` | `file_path: String` | `src-tauri/src/a11y.rs:186`, pub sync | evet | `src/components/A11yPanel.svelte:44` |
| `update_reading_order` | `file_path: String; parent_id_str: String; ordered_child_ids: Vec<String>; target_path: String` | `src-tauri/src/a11y.rs:263`, pub sync | **hayır** | — |
| `build_semantic_index` | `file_path: String; chunk_size: usize; chunk_overlap: usize; ollama_url: String; embed_model: String` | `src-tauri/src/ai.rs:67`, pub async | evet | `src/pdfState.js:93` |
| `semantic_search` | `file_path: String; query: String; top_k: usize; ollama_url: String; embed_model: String` | `src-tauri/src/ai.rs:130`, pub async | evet | `src/pdfState.js:97` |
| `query_document_rag` | `file_path: String; prompt: String; history: Vec<ChatMessage>; ollama_url: String; llm_model: String; embed_model: String` | `src-tauri/src/ai.rs:170`, pub async | evet | `src/pdfState.js:101` |
| `summarize_selection_or_page` | `text_content: String; target_length: String; ollama_url: String; llm_model: String` | `src-tauri/src/ai.rs:245`, pub async | evet | `src/pdfState.js:105` |
| `get_page_annotations` | `file_path: String; page_index: u32` | `src-tauri/src/annotations.rs:54`, pub sync | evet | `src/pdfState.js:110` |
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
| `ask_local_ai` | `model: Option<String>; prompt: String; context: String` | `src-tauri/src/lib.rs:476`, private sync | evet | `src/components/AiAssistantModal.svelte:34` |
| `extract_pages` | `source_path: String; page_indices: Vec<u32>; target_path: String` | `src-tauri/src/lib.rs:513`, private sync | evet | `src/App.svelte:1428` |
| `read_image_file` | `path: String; rotation: i32; flip_h: bool; flip_v: bool` | `src-tauri/src/lib.rs:549`, private sync | evet | `src/App.svelte:979` |
| `convert_and_save_image` | `source_path: String; target_path: String; format: String; rotation: i32; flip_h: bool; flip_v: bool` | `src-tauri/src/lib.rs:572`, private sync | evet | `src/App.svelte:1917` |
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
| `get_page_text_layout` | `file_path: String; page_index: u32` | `src-tauri/src/pdf_engine/mod.rs:748`, pub async | evet | `src/components/PdfPage.svelte:278` |
| `load_plugin` | `manifest_path: String` | `src-tauri/src/plugins.rs:53`, pub sync | evet | `src/pluginStore.js:24` |
| `unload_plugin` | `plugin_id: String` | `src-tauri/src/plugins.rs:89`, pub sync | evet | `src/pluginStore.js:30` |
| `get_active_plugins` | — | `src-tauri/src/plugins.rs:102`, pub sync | evet | `src/pluginStore.js:16` |
| `trigger_plugin_hook` | `plugin_id: String; hook_name: String; payload: String` | `src-tauri/src/plugins.rs:115`, pub sync | evet | `src/pluginStore.js:35` |
| `run_script_direct` | `script_content: String` | `src-tauri/src/plugins.rs:143`, pub sync | evet | `src/pluginStore.js:39` |
| `analyze_color_spaces` | `file_path: String` | `src-tauri/src/prepress.rs:32`, pub sync | evet | `src/components/PrepressPanel.svelte:140` |
| `calculate_ink_coverage` | `file_path: String; page_index: u32; max_limit: f32; scale: f32` | `src-tauri/src/prepress.rs:84`, pub sync | evet | `src/components/PrepressPanel.svelte:206` |
| `render_separation_channels` | `file_path: String; page_index: u32; scale: f32` | `src-tauri/src/prepress.rs:150`, pub async | evet | `src/components/PrepressPanel.svelte:165` |
| `set_page_boxes` | `source_path: String; target_path: String; page_index: u32; boxes: PageBoxes` | `src-tauri/src/prepress.rs:222`, pub sync | **hayır** | — |
| `export_pdf_x` | `source_path: String; target_path: String; standard: String; profile_name: String` | `src-tauri/src/prepress.rs:251`, pub sync | evet | `src/components/PdfXWizard.svelte:49` |
| `apply_redactions` | `job_id: String; session_id: String; expected_content_generation: u64; output_path: String; redactions: Vec<RedactionArea>` | `src-tauri/src/redaction.rs`, session-job | **hayır** | Legacy command parse/snapshot öncesi fail-closed `FEATURE_UNAVAILABLE`; secure pipeline'a alias değildir. |
| `plan_secure_redaction` | `job_id; session_id; expected_content_generation; policy; acknowledgements; regions` | `src-tauri/src/redaction.rs`, session-job + isolated planner worker | `src/services/app/secureRedactionController.js` | Canonical 8B planning operation; feature disabled olduğu için production UI runnable değildir. |
| `execute_secure_redaction` | `job_id; session_id; expected_content_generation; policy; acknowledgements; regions; expected_plan_digest; target_path` | `src-tauri/src/redaction.rs`, session-job + isolated transform/verifier workers | `src/services/app/secureRedactionController.js` | Canonical 8B secure export operation; feature disabled olduğu için production UI runnable değildir. |
| `sanitize_pdf_document` | `job_id: String; session_id: String; expected_content_generation: u64; output_path: String; options: SanitizationOptions` | `src-tauri/src/redaction.rs`, dormant legacy contract | **hayır** | 8C.2 sonrasında da parse/snapshot/worker/I/O öncesi fail-closed `FEATURE_UNAVAILABLE`. Broad boolean options ve `output_path` versioned inspection/policy authority'si değildir. |
| `inspect_document_sanitization` | `job_id: String; session_id: String; expected_content_generation: u64; policy_schema_version: String` | `src-tauri/src/sanitization.rs`, immutable session snapshot + ephemeral `InspectDocumentSanitization` worker | Feature-disabled inspection controller | Read-only typed `SessionJobOutcome<SanitizationInspectionReport>`; raw input/working/snapshot path, credential, output ve target kabul etmez. Transform/output/publication yapmaz. |
| `scan_sensitive_data` | `job_id: String; session_id: String; expected_content_generation: u64; patterns: Vec<String>` | `src-tauri/src/redaction.rs`, session-job (Job-Worker Protocol v3) | evet | `src/services/backend/redaction.js` (`startSensitiveDataScanAction` → cancellable handle → job worker protocol v3) |
| `list_system_certificates` | — | `src-tauri/src/signature.rs:48`, pub async | evet | `src/components/SignatureWizardModal.svelte:34` |
| `sign_pdf_pades` | `input_path: String; output_path: String; cert_id: String; _pin: String; options: SignatureOptions` | `src-tauri/src/signature.rs:71`, pub async | evet | `src/components/SignatureWizardModal.svelte:79` |
| `verify_pdf_signatures` | `file_path: String` | `src-tauri/src/signature.rs:101`, pub async | evet | `src/components/SignaturePanel.svelte:18` |
| `enable_ltv` | `file_path: String; signature_id: String` | `src-tauri/src/signature.rs:135`, pub async | evet | `src/components/SignaturePanel.svelte:30` |

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

### Tanımlı fakat handler'a kayıtlı olmayan

- `a11y::update_reading_order` — `src-tauri/src/a11y.rs:263`
- `prepress::set_page_boxes` — `src-tauri/src/prepress.rs:222`

Bu iki command normal Tauri `invoke` yolu üzerinden erişilemez.

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
| `src/components/A11yPanel.svelte:44` | `validate_pdf_ua` | `{ filePath }` |
| `src/components/A11yPanel.svelte:55` | `get_structure_tree` | `{ filePath }` |
| `src/components/A11yPanel.svelte:98` | `update_alt_text` | `{ filePath, objectIdStr, altText, language, targetPath }` |
| `src/components/AiAssistantModal.svelte:34` | `ask_local_ai` | `{ model, prompt, context }` |
| `src/components/PageOrganizer.svelte:94` | `render_thumbnail` | `{ path, pageIndex, targetWidth }` |
| `src/components/PdfPage.svelte:278` | `get_page_text_layout` | `{ filePath, pageIndex }` |
| `src/components/PdfPage.svelte:354` | `render_page` | `{ path, pageIndex, scale, rotation }` |
| `src/components/PdfXWizard.svelte:49` | `export_pdf_x` | `{ sourcePath, targetPath, standard, profileName }` |
| `src/components/PrepressPanel.svelte:140` | `analyze_color_spaces` | `{ filePath }` |
| `src/components/PrepressPanel.svelte:165` | `render_separation_channels` | `{ filePath, pageIndex, scale }` |
| `src/components/PrepressPanel.svelte:206` | `calculate_ink_coverage` | `{ filePath, pageIndex, maxLimit, scale }` |
| `src/components/Sidebar.svelte:24` | `get_pdf_outline` | `{ filePath }` |
| `src/components/SignaturePanel.svelte:18` | `verify_pdf_signatures` | `{ filePath }` |
| `src/components/SignaturePanel.svelte:30` | `enable_ltv` | `{ filePath, signatureId }` |
| `src/components/SignatureWizardModal.svelte:34` | `list_system_certificates` | — |
| `src/components/SignatureWizardModal.svelte:79` | `sign_pdf_pades` | `{ inputPath, outputPath, certId, pin, options }` |
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
| `src/pdfState.js:85` | `export_as_searchable_pdf` | `{ filePath, outputPath, lang }` |
| `src/pdfState.js:93` | `build_semantic_index` | `{ filePath, chunkSize, chunkOverlap, ollamaUrl, embedModel }` |
| `src/pdfState.js:97` | `semantic_search` | `{ filePath, query, topK, ollamaUrl, embedModel }` |
| `src/pdfState.js:101` | `query_document_rag` | `{ filePath, prompt, history, ollamaUrl, llmModel, embedModel }` |
| `src/pdfState.js:105` | `summarize_selection_or_page` | `{ textContent, targetLength, ollamaUrl, llmModel }` |
| `src/pdfState.js:110` | `get_page_annotations` | `{ filePath, pageIndex }` |
| `src/pdfState.js:119` | `add_annotation` | `{ filePath, pageIndex, annotType, rect, color, contents }` |
| `src/pdfState.js:128` | `delete_annotation` | `{ filePath, pageIndex, annotId }` |
| `src/pdfState.js:137` | `save_pdf_with_annotations` | `{ filePath, outputPath }` |
| `src/pdfState.js:146` | `check_pdf_encrypted` | `{ filePath }` |
| `src/pdfState.js:154` | `get_pdf_metadata` | `{ filePath }` |
| `src/pdfState.js:161` | `search_pdf_text` | `{ path, query }` |
| `src/pdfState.js:192` | `print_pdf` | `{ filePath, pageRange }` |
| `src/pdfState.js:201` | `export_pages_as_images` | `{ filePath, pages, outputDir, format, dpi }` |
| `src/pdfState.js:210` | `export_text_content` | `{ filePath, pages, format }` |
| `src/pluginStore.js:16` | `get_active_plugins` | — |
| `src/pluginStore.js:24` | `load_plugin` | `{ manifestPath }` |
| `src/pluginStore.js:30` | `unload_plugin` | `{ pluginId }` |
| `src/pluginStore.js:35` | `trigger_plugin_hook` | `{ pluginId, hookName, payload }` |
| `src/pluginStore.js:39` | `run_script_direct` | `{ scriptContent }` |
| `src/pluginStore.js:43` | `add_batch_job` | `{ jobType, filePath, parameters }` |

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
- `sign_pdf_pades`: `_pin`
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
| `ai_chat_token` | `src-tauri/src/ai.rs:227` | string token | `src/components/AiSidebar.svelte:30` | eşleşiyor | Runtime test edilmedi |
| `ai_chat_done` | `src-tauri/src/ai.rs:230` | boş string | `src/components/AiSidebar.svelte:41` | eşleşiyor | Runtime test edilmedi |
| `ai_summary_token` | `src-tauri/src/ai.rs:280` | string token | `src/components/AiContextMenu.svelte:51` | eşleşiyor | Runtime test edilmedi |
| `ai_summary_done` | `src-tauri/src/ai.rs:282` | boş string | `src/components/AiContextMenu.svelte:55` | eşleşiyor | Runtime test edilmedi |
| `batch_job_status` | `src-tauri/src/batch.rs:40,49` | `JobStatus` object | Consumer bulunmadı | **backend-only** | Test yok / runtime çalışmadı |
| `tauri://drag-drop` | Tauri sistem event'i | frontend `.paths` okur | `src/App.svelte:658` | Sistem producer'ı bekleniyor | Runtime test edilmedi |

AI streaming producer/consumer adları eşleşir. Ancak Ollama bağlantısı ve Tauri
event teslimi çalıştırılmadığı için uçtan uca başarı kanıtı değildir.
`batch_job_status` için frontend listener yoktur; ayrıca `addBatchJob` wrapper'ının
consumer'ı bulunmadığından batch akışı normal UI yolundan erişilebilir
sayılmamıştır.

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
