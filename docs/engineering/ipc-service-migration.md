# Prompt 2 — IPC Service Migration

## Aşama 2 active-PDF identity update — 2026-07-28

The Prompt 2 matrices below describe their historical migration baseline. The
current public frontend boundary no longer retains a path-native contract merely
because an internal Rust helper/handler exists.

- Render requests use `sessionId` and `contentGeneration`; render-session state
  contains no path or `documentId` alias.
- Read, mutation, OCR, signature, prepress, export, and PDF-share adapters use
  session identity with the appropriate expected generation/revision.
- PrepressPanel, the search coordinator, and annotation burn-in stale guards use
  captured session/generation identity rather than display paths.
- Session-preflight inputs, external operands, image paths, and output targets
  remain path-native.
- Active-PDF share and independent external-file share are separate commands.
- Metadata/alt-text write bypasses and unused legacy frontend facades were
  removed; LTV remains session-native and mock-disabled.
- `client.js` remains the only frontend Tauri `invoke` import.

Date: 2026-07-25

This document records the post-Prompt-2 state. The historical Prompt 0 evidence
in `docs/engineering/ipc-inventory.md` is intentionally unchanged.

## Baseline

- Branch: `main`
- Commit: `8a05265c121faab0aba05a76a21be4abc6191b8e`
- Working tree before Prompt 2: 2 staged, 84 unstaged, 25 untracked entries.
- Production files importing `invoke` before Prompt 2: 17.
- Baseline build: passed.
- Baseline `svelte-check`: 0 errors, 65 warnings.
- Baseline full suite: 5 failed files, 12 passed files, 1 skipped file;
  24 failed tests, 103 passed tests, 6 skipped tests.

The baseline failures were:

- `src/components/PageOrganizer.test.js`: suite initialization failed at the
  existing Svelte 5 / `svelte/internal` compatibility boundary.
- `src/components/SearchBar.test.js`: 4 tests failed.
- `src/components/PrepressPanel.test.js`: 6 tests failed.
- `src/components/PdfXWizard.test.js`: 5 tests failed.
- `src/components/PdfPage.test.js`: 9 tests failed.

The exact 24 failing test names were captured before editing and compared again
after the migration. The post-migration suite contains the same failing files
and test names, no formerly passing test became failing, and no new test was
skipped.

## Architecture

Command IPC now follows this boundary:

```text
UI/component
  -> feature-aware public adapter/action
  -> domain backend service
  -> validated client
  -> Tauri invoke
```

`src/services/backend/client.js` is the only file under `src/` that imports
`invoke` from `@tauri-apps/api/core`. Event IPC remains separate; `listen()`,
AI token streams, batch events, and drag/drop events were not moved into command
contracts.

At the Prompt 2 baseline, `contracts.js` contained the 66 commands listed in the
historical table below. Later session-native feature work adds versioned commands
only with a canonical backend service and inventory entry; the current authority is
`contracts.js` plus `ipc-inventory.md`.

| Module | Wrappers | Rust commands |
|---|---:|---|
| `viewer.js` | 9 | `check_pdf_encrypted`, `authenticate_pdf_password`, `get_page_count`, `render_page`, `render_thumbnail`, `render_pdf_thumbnail`, `get_pdf_outline`, `search_pdf_text`, `get_page_text_layout` |
| `document.js` | 11 | `export_pdf_page_to_image`, `get_pdf_metadata`, `set_pdf_metadata`, `extract_pages`, `read_image_file`, `convert_and_save_image`, `reorder_pdf_pages`, `burn_annotations_to_pdf`, `print_pdf`, `export_pages_as_images`, `export_text_content` |
| `manipulation.js` | 8 | `rotate_page`, `delete_pages`, `insert_blank_page`, `merge_pdfs`, `split_pdf`, `get_form_fields`, `update_form_field`, `reorder_pages` |
| `annotations.js` | 4 | `get_page_annotations`, `add_annotation`, `delete_annotation`, `save_pdf_with_annotations` |
| `ocr.js` | 4 | `detect_scanned_page`, `get_available_ocr_languages`, `run_ocr_on_page`, `export_as_searchable_pdf` |
| `ai.js` | 5 | `ask_local_ai`, `build_semantic_index`, `semantic_search`, `query_document_rag`, `summarize_selection_or_page` |
| `plugins.js` | 6 | `get_active_plugins`, `load_plugin`, `unload_plugin`, `trigger_plugin_hook`, `run_script_direct`, `add_batch_job` |
| `signatures.js` | 4 | `list_system_certificates`, `sign_pdf_pades`, `verify_pdf_signatures`, `enable_ltv` |
| `redaction.js` | 3 | `apply_redactions`, `sanitize_pdf_document`, `scan_sensitive_data` |
| `accessibility.js` | 3 | `get_structure_tree`, `update_alt_text`, `validate_pdf_ua` |
| `prepress.js` | 4 | `analyze_color_spaces`, `calculate_ink_coverage`, `render_separation_channels`, `export_pdf_x` |
| `platform.js` | 5 | `trigger_continuity_camera_scan`, `speak_text`, `stop_speaking`, `share_file_macos`, `perform_ocr_on_page` |
| **Total** | **66** | **66 static frontend commands** |

8C.2 adds `sanitization.js` with the single
`inspect_document_sanitization` session-job command. It accepts only job/session/
content-generation/policy-version identity, strictly normalizes the typed report and
is reachable from `sanitizationInspectionController.js` only under the disabled
sanitization feature guard. It does not replace the dormant transform wrapper and
does not accept an input/output/target path.

`index.js` is deliberately a narrow, safe barrel. Protected raw wrappers are
not re-exported. The following paths remain reachable only through existing
feature-aware compatibility adapters (`pdfState.js`, `pluginStore.js`,
`ttsActions.js`) or `services/actions.js`:

- annotation CRUD and annotation burn/export
- blank-page insertion
- form read/update
- searchable PDF export
- redaction application and sanitization
- sensitive-data scan
- PAdES certificate listing/signing, signature verification, and LTV
- PDF/X export
- plugin and batch commands
- platform-limited continuity, speech, share, and Vision OCR commands

Prompt 1 feature statuses and guards were not changed. Correcting a payload does
not make a disabled or mock feature runnable.

## Contract policy

- Required Rust non-`Option<T>` values are rejected when absent or `undefined`.
- Optional `undefined` values are omitted from the payload.
- `null` is accepted only for fields backed by Rust `Option<T>`.
- Unknown top-level keys and keys in validated nested structs are rejected in
  both development and production.
- Primitive, array, tuple, and nested object shapes are checked before Tauri.
- No schema dependency or generated binding system was added.

Nested field names were taken from the actual Serde structs, not inferred from
the top-level Tauri conversion. This includes `PdfMetadata`, `RectData`,
`RedactionArea`, `SanitizationOptions`, `ChatMessage`, `SignatureOptions`, and
`VisualSignatureOptions`. The local Tauri macro source also confirms that its
default command argument policy applies lower camel case conversion; leading
underscores in unused Rust argument identifiers are separators in that
conversion.

`BackendCommandError` is created only in the client/transport layer and keeps:

```js
{
  command,
  message,
  cause
}
```

The original backend value is retained as `cause`; payloads, PINs, prompts,
annotation JSON, and document text are not attached to errors.

## Corrected mismatches

| Command | Before | Canonical payload |
|---|---|---|
| `burn_annotations_to_pdf` | `{ filePath }` | `{ sourcePath, annotationsJson, targetPath }` |
| `get_pdf_metadata` compatibility wrapper | `{ filePath }` | `{ path }` |
| `merge_pdfs` direct App path | `{ pdfPaths, targetPath }` | `{ filePaths, outputPath }` |
| `apply_redactions` | `{ filePath, redactionRects, targetPath }` | `{ jobId, sessionId, expectedContentGeneration, outputPath, redactions }` *(parse/snapshot öncesi `FEATURE_UNAVAILABLE`)* |
| `sanitize_pdf_document` | `{ filePath, options, targetPath }` | `{ jobId, sessionId, expectedContentGeneration, outputPath, options }` *(parse/snapshot öncesi `FEATURE_UNAVAILABLE`)* |

The legacy annotation burn adapter does not synthesize annotations or a target
path. Its Prompt 1 feature remains disabled.

The named suspicious paths `reorder_pdf_pages`, `reorder_pages`, `split_pdf`,
`export_pdf_x`, accessibility updates, OCR, and annotation commands were also
checked against their Rust signatures and retained with their canonical keys.

## Dynamic profiler

`ipc_profiler.js` delegates to the normal validated client. It accepts only
command names registered in `contracts.js`; unknown commands fail closed before
Tauri. It has no architecture-test exception.

## Commands intentionally not exposed

Handler-registered but without a static frontend consumer:

- `render_pdf_page`
- `clear_search_cache`

Defined in Rust but not registered in `generate_handler!`:

- `update_reading_order`
- `set_page_boxes`

There is no frontend command missing from the handler list. Event names are not
commands and are not included in these counts.

## Tests and final verification

- Canonical wrapper matrix: all 66 wrappers assert exact command, payload keys,
  and payload values.
- Client tests: missing fields, unknown fields, primitive/tuple/nested shape,
  optional omission, allowed/disallowed nulls, and normalized string, `Error`,
  object, and already-normalized failures.
- Feature-aware action tests: disabled/mock paths cannot reach raw services;
  enabled and platform-limited paths preserve arguments.
- Architecture tests: only `client.js` imports Tauri `invoke`; protected raw
  wrappers are absent from `index.js` and cannot be imported by components.
- Compatibility tests mock adapters/services instead of Tauri `invoke`.

Final results:

| Check | Result |
|---|---|
| Targeted local Vitest command | 6 files passed; 110 tests passed |
| `./node_modules/.bin/svelte-check --tsconfig ./jsconfig.json` | 0 errors; 65 warnings |
| `npm run build` | passed |
| Full `npm test` | same 5 failed files and same 24 failed tests; 16 passed files, 1 skipped file; 200 passed tests, 6 skipped tests |

The higher pass totals come from the new Prompt 2 tests. Failure and skip
identities are unchanged from baseline.

## Prompt 2 file scope

No Rust, dependency/config, feature registry/status, Prompt 0 inventory, or
event IPC file was changed by Prompt 2.

The migration added `src/services/backend/*`, `src/services/actions.js`,
`src/services/actions.test.js`, and this document. It minimally edited
`App.svelte`, the existing IPC-consuming components, the compatibility adapters,
the profiler, and their affected tests to delegate through the centralized
service boundary.

## Prompt 3 — canonical RGBA render pipeline (2026-07-25)

Prompt 3 replaces the two public viewer render paths with one canonical raw
RGBA service. `render_page` is the only public full-page render command used by
the frontend. The PNG-returning `render_pdf_page` and
`render_pdf_thumbnail` commands were removed from the Tauri handler registry;
their old implementations are compiled out pending a later physical source
cleanup. The internal `render_page_rgba` helper remains because export/OCR
backend paths still use it.

The canonical page response is:

```js
{
  width,
  height,
  data,
  pageIndex,
  generation,
  requestId
}
```

`data` is raw RGBA and is painted directly to canvas. No PNG encode/decode
transport was added.

### Public render contracts

`viewer.js` now has ten wrappers. Its render/lifecycle commands are:

| Contract | Rust command | Purpose |
|---|---|---|
| `renderPage` | `render_page` | main-page RGBA render |
| `renderThumbnail` | `render_thumbnail` | sidebar/organizer RGBA render |
| `setActiveRenderDocument` | `set_active_render_document` | atomic document-generation activation/deactivation |
| `cancelRenderRequest` | `cancel_render_request` | exact request cancellation |

Removing `renderPdfThumbnail` and adding the two lifecycle commands changes the
Prompt 2 frontend command matrix from 66 to 67 contracts. Wrapper, contract,
handler-boundary, and architecture tests use the recalculated count.

The validator now enforces numeric maxima, string length limits, string enums,
and object-level cross-field rules. Render generations are non-negative safe
integers, render/document request identifiers are non-empty and at most 128
characters, paths are non-empty and at most 4096 characters, scale is in
`(0, 16]`, target width is in `[1, 16384]`, and scope is one of
`main_page`, `sidebar_thumbnail`, or `organizer_thumbnail`. Page rotation is
limited to `0/90/180/270`. Activation accepts a string `documentId` and path
together, or two nulls for deactivation. Rust independently validates the same
critical boundaries.

### Session and cancellation identity

`App.svelte` creates one stable render-session object during component
initialization and calls `setContext()` once. Document changes replace the
immutable snapshot inside that object only after backend activation resolves.
`PdfPage`, `Thumbnail`, `ThumbnailItem`, and `PageOrganizer` take a fresh
snapshot at the start of each request.

The backend active slot is:

```text
(document_id, generation, page_index, scope)
```

and the slot value contains the opaque `requestId` plus its cancellation token.
Superseding or cancelling a request checks the exact request ID. A delayed
cancel or cleanup cannot remove a replacement request. `main_page`,
`sidebar_thumbnail`, and `organizer_thumbnail` have isolated cancellation
slots, so the same page can render for different consumers concurrently
without cross-cancellation.

Timeout invalidates the frontend request immediately, sends exact backend
cancellation, and keeps observing the original invoke promise through the
coordinator race. A late result is still rejected by the frontend request,
generation, page, and metadata checks. Cancellation/supersede is not presented
as a PDFium render failure.

Snapshot jobs use a separate identity contract. Every snapshot-job payload
contains `jobId`, `sessionId`, and `expectedContentGeneration`.
`runSessionJob()` creates the UUID and returns `{ jobId, promise, cancel }`;
existing consumers continue to await `promise`. `cancel_session_job` receives
`sessionId + jobId` and returns one of `cancel_requested`,
`already_requested`, `cancel_pending`, or `already_terminal`. No global job
manager, progress panel, unmount cancellation, or session-switch cancellation
policy is introduced.

Snapshot-job IPC success is the generic tagged `SessionJobOutcome<T>`.
Operation `failed` and `cancelled` are terminal success-channel envelopes;
pre-reservation validation/admission failures remain IPC errors. Cleanup
warnings are carried beside the operation result and cannot overwrite a
completed value.

### Cache and PDFium scheduling

The RGBA cache key is:

```text
document_id
+ document generation
+ page index
+ actual output pixel width
+ actual output pixel height
+ normalized rotation
```

Thumbnail keys therefore use the dimensions resulting from the actual
call-site target width. Existing targets were preserved: `ThumbnailItem` uses
150 px, `PageOrganizer` uses 180 px, and `Thumbnail.svelte` continues to derive
its target from measured CSS width and device pixel ratio.

The LRU is constrained by both a 128 MiB total byte budget and a 32 MiB
per-entry limit. Oversized results are returned but not cached. Activation
cancels prior-generation tasks and clears both dimension hints and RGBA cache
entries.

The former permit gate is centralized in a priority-aware PDFium render
scheduler. Main-page requests take priority over queued thumbnails, and stale
thumbnail requests can leave the queue before executing. The service
invariants are:

- no synchronous mutex is held across `.await`;
- cache/request/scheduler locks are released before PDFium work;
- service state and scheduler locks are never acquired together;
- the global PDFium mutex covers document/page access and rendering;
- cancellation is checked before queueing, after permit acquisition, before
  PDFium render, and before caching/returning;
- cleanup removes only its own request ID.

Render profiling remains on the typed `callBackendProfiled()` path without
logging paths or payloads. Backend telemetry separates cache lookup, scheduler
wait, PDFium render, RGBA conversion, and total elapsed time. No benchmark was
run, so no performance improvement is claimed.

### Final inventory and verification

The final frontend contract matrix contains 67 commands. The Tauri
`generate_handler!` registry contains 68 handlers; its viewer render surface is
limited to `render_page`, `render_thumbnail`, `set_active_render_document`, and
`cancel_render_request`. Only `render_page` is the public full-page render
command.

Verification on 2026-07-25:

- the focused Prompt 3 frontend suite passed: 10 files, 122 tests;
- `cargo check --locked` passed;
- `cargo test --locked` passed: 15 library tests and the crash integration
  test;
- `npm run build` passed;
- `svelte-check` completed with 0 errors and the existing 65 warnings;
- the full `npm test` baseline remains red only in the same three out-of-scope
  component files (`SearchBar`, `PrepressPanel`, and `PdfXWizard`): 15 failed,
  240 passed, and 6 skipped. Prompt 3's `PdfPage`, thumbnail, organizer,
  session, coordinator, contract, and architecture tests all pass;
- repository-wide `cargo fmt --all -- --check` remains red only because the
  pre-existing, out-of-scope `tests/crash_tests.rs` is not rustfmt-clean. All
  Rust files changed by Prompt 3 are rustfmt-clean;
- repository-wide Clippy remains red on 18 pre-existing findings outside the
  new render-service code. No Prompt 3 compile, unit-test, or Clippy failure was
  introduced.

PDFium rendering remains serialized by the process-wide mutex. Cancellation is
cooperative around the FFI call, so an already-running
`FPDF_RenderPageBitmap` cannot be interrupted; its result is discarded instead.
Other legacy PDFium entry points must continue to be audited for use of that
same global mutex. The raw RGBA cache also temporarily duplicates a buffer when
returning a cached response, although retained entries remain constrained by
the byte budgets above.

### Prompt 3 close-out corrections (2026-07-26)

`check_pdf_encrypted` and `authenticate_pdf_password` now acquire the same
process-wide `PDFIUM_MUTEX` used by rendering. A scan of `pdf_engine/mod.rs`
confirmed that every concrete PDFium load, page, render, text, and bookmark
operation in that module is inside the mutex. `get_pdfium` and
`open_pdfium_doc` remain caller-locked helpers; they do not recursively acquire
the non-reentrant mutex.

The scheduler queue now owns each pending ticket through an RAII waiter guard.
Dropping or aborting an `acquire()` future removes its ticket and wakes the next
waiter; acquiring the permit disarms the guard. Tests cover aborted main-page
and thumbnail waiters.

RGBA cache accounting now consumes the entry returned by `LruCache::push()` so
capacity-driven eviction decrements the retained byte count. Oversized rendered
buffers are checked before cloning and are returned without a cache copy. Tests
cover byte-budget and entry-capacity eviction independently.

The disabled legacy implementations of `render_pdf_page`,
`render_pdf_thumbnail`, and their associated legacy render block were
physically removed. The architecture test now scans production Rust sources for
those old public function declarations in addition to checking the handler
registry.

The viewer orchestration accepts an internal injected renderer for tests while
the production command still supplies the PDFium renderer. PDFium-independent
tests cover generation changes during rendering, exact cancellation before
renderer return, cancelled/inactive results not entering cache, current request
metadata on cache hits, and preservation of real renderer errors when the
request remains active. Exact cancellation also verifies the active document
path, so a request with a mismatched path is an idempotent no-op.

The repository-wide audit also found direct PDFium users outside
`pdf_engine/mod.rs` in OCR, annotation, export, redaction, prepress,
manipulation, AI, worker, and startup/test paths. Those modules are outside the
Prompt 3 allowlist and were not changed in this correction round; they remain a
separate global-mutex audit item.
## Candidate transition additions

Document opening is coordinated through the canonical backend client. The
candidate lifecycle adds exact contracts for active-session snapshot lookup,
candidate-scoped render probing, prepared Save/Save As decisions, activation
commit envelopes, and PDF-to-image deactivation commit. No component invokes
Tauri commands directly.

`commit_candidate_activation` dispatch is a frontend linearization fence.
Transport failure is reconciled from backend active-session authority rather
than by replaying the activation token.
