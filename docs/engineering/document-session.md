# Document Session Model

Prompt 5 uses Model B: every opened PDF has a backend-owned working copy. The
frontend only receives a session identifier and `DocumentSessionSnapshot`;
`working.pdf`, credentials, digests, and recovery artifacts are never serialized.

## Identity and revision rules

- Frontend active-PDF identity is exactly `(sessionId, contentGeneration)`.
  Render payloads use `sessionId` + `contentGeneration`; read/job payloads use
  `sessionId` + `expectedContentGeneration`.
- `working_path`, the source/open path, credentials, digests, job paths, and temp
  ownership never appear in `DocumentSessionSnapshot`. `displayPath` and
  `fileName` are presentation/save-target data and must not be used as IPC
  identity or stale-result guards.
- Paths remain public only for pre-session open/authentication, external
  operands (including images and merge inputs), and user-selected output
  targets.
- `content_generation` starts at 1 and advances only after a verified content
  mutation (including a durability-unconfirmed commit whose replacement is known
  to be committed). Undo and redo are content mutations too: they advance the
  generation and never move it back, even when they restore earlier bytes.
- `dirty` is derived, not counted: `working_digest != Some(baseline.digest)`
  (`refresh_dirty`). `baseline.digest` is the digest of the bytes last opened or
  saved, because open copies the source byte for byte and Save copies the
  working PDF byte for byte. An unknown working digest counts as dirty. Undoing
  back to the saved bytes is therefore clean although the generation advanced.
- `DocumentSessionSnapshot.undoOperation` / `redoOperation` name the operation
  (for example `rotate_page`) that undo or redo would reverse or reapply; they
  carry no path, digest or file name.
- `state_revision` advances when dirty, save target, baseline, lifecycle, warning, or other backend session state changes.
- `transition_epoch` is frontend-only and advances only when the active session changes. Mutation, Save, Save As, warning, or recovery changes do not advance it.
- Save/Save As operations enforce single-flight concurrency per session ID on the frontend and validate `expected_state_revision` after acquiring backend write lease.
- Frontend snapshot acceptance (`documentSession.applySnapshot()`) strictly enforces monotonic `state_revision` checking (rejecting stale or equal revision snapshots) and NEVER increments `state_revision` locally.
- Render/read work is guarded by `(session_id, content_generation)`.

## Session lifecycle

```text
CandidateAuthPending -> CandidateReady -> Open
CandidateReady --------------------------> Closed
Open -> DecisionPending -> Open
Open -> DecisionPending -> Closing -> Closed
Open -> RecoveryRequired -> Open | Closing -> Closed
Open -> CommitIndeterminate -> Open | Closing -> Closed
```

`RecoveryRequired` and `CommitIndeterminate` reject normal mutation and Save.
`recover_document_session` must reopen and validate the working PDF, hash it, and
publish a new generation before normal operation resumes.

## Atomic mutation mapping

| CommitState | Session result |
| --- | --- |
| `NewCommitted` | generation and revision advance; candidate digest becomes working digest; dirty is recomputed; the pre-mutation state enters the undo history (see Undo/redo history) |
| `NewCommittedDurabilityUnconfirmed` | same as committed, plus durability warning |
| `Prepared`, `OriginalPreserved` | mutation not applied; generation/revision unchanged |
| `OriginalRecovered` | working file is re-hashed; old digest means not applied, candidate digest means applied, otherwise indeterminate |
| `RecoveryRequired` | dirty, digest unknown, lifecycle becomes recovery-required |
| `Indeterminate` | dirty, digest unknown, lifecycle becomes commit-indeterminate |

## Atomic Save mapping

| CommitState | Session result |
| --- | --- |
| `NewCommitted` | baseline/target update; dirty is recomputed (clean, because the target now holds the working bytes); revision advances; the undo history is kept |
| `NewCommittedDurabilityUnconfirmed` | same as committed, plus durability warning |
| `Prepared`, `OriginalPreserved`, `OriginalRecovered` | dirty and working copy remain available; typed error returned |
| `RecoveryRequired` | dirty remains; lifecycle blocks normal work pending recovery |
| `Indeterminate` | dirty remains; lifecycle blocks normal work pending recovery |

Normal Save supplies the opening/last-save SHA-256 baseline as an atomic commit
precondition. A missing or changed target is a typed conflict and never overwrites
external content. Save As retargets normal Save after a verified commit.

## Incremental writes

Session mutations append to the working PDF instead of rewriting it, so signed
bytes stay intact (`incremental_save::save_mutation`; ReorderPages through
`manipulation::reorder_incrementally`). Before writing, the input's xref chain
must be consistent, because PDFium appends to a repaired chain by silently
dropping `/Prev`. After writing, the output must start with the input bytes
and link its new section to the input's last `startxref`
(`validate_append_only`). If either check fails, a signed document returns an
error and an unsigned one is rewritten as before. ExtractPages produces a new
document and stays a full rewrite. Design and spike evidence:
`undo-redo-and-incremental-save-design.md` §4 and §8.1.

## Undo/redo history

Each session owns `session_dir/history/`. Every committed mutation leaves the
state before it there. No new copy is written: the transaction's own
pre-mutation snapshot is renamed into the directory after the commit, provided
its digest matches the pre-mutation digest. The history holds at most 50 undo
steps and 1 GiB in total. Oldest undo entries go first; with no undo entries
left, the farthest redo entries go. A new mutation discards the redo stack.

`undo_session_mutation` and `redo_session_mutation` (`sessionId`,
`expectedContentGeneration`) restore the top entry through the same isolated
transaction as recipe mutations (`MutationTransactionKind::Restore`). Admission,
the write lease, worker-document invalidation, the commit-state mapping above,
cancellation and recovery are therefore identical. A restore runs no worker and
no PDFium validation: the entry's bytes must hash to the digest recorded when
it was created. A mismatch is `HISTORY_ENTRY_CORRUPTED`; the working PDF is
untouched and the history is cleared. The restore transaction's own snapshot
becomes the entry in the opposite direction. An empty stack is `HISTORY_EMPTY`.

If a mutation commits but its history entry cannot be kept, the history is
cleared and `health_warning` becomes `HISTORY_TRUNCATED`; the mutation itself
stands. `recover_document_session` clears the history, because the recovered
content's relation to it is unknown. Closing the session removes `session_dir`
and with it the history. Nothing persists across process exit.

## Transition protocol

There is no general `finalize_transition` API.

```text
begin_session_transition
  -> DecisionPending
  -> save_transition
       -> failure: DecisionPending (same token is retryable)
       -> switch/image-close: PreparedSave
       -> close: Closing -> Closed
  -> discard_transition
       -> switch/image-close: PreparedDiscard
       -> close: Closing -> Closed
  -> cancel_transition -> Open
```

Tokens are session-bound opaque identifiers with kind, creation time, expected
revision, and decision status. Replay after consumption is rejected. A second
transition is rejected. Expired tokens cannot Save/Discard; Cancel or a new begin
restores the safe default `Open` state. Candidate activation uses separate
`prepare_candidate_activation`, `commit_candidate_activation`, and
`rollback_candidate_activation` commands.

`DocumentSessionStore.active_session_id` is authoritative. Registry size and
candidate presence are never used to infer the active session. Prepare receives
an explicit `(previousSessionId, previousTransitionToken)` pair. Both are null
only when backend active authority is null; otherwise both must identify the
exact active session and a prepared document-switch transition.

The frontend coordinator treats `commit_candidate_activation` dispatch as the
point of no return. During this commit fence it retains one latest pending
PDF/image request, not a FIFO queue. Superseded pending requests do not own
dialogs or publish UI state.

Candidate validation uses a candidate-scoped first-page PDFium probe that checks
page count, dimensions, and RGBA byte length without changing global backend or
local frontend render identity. Render identity activation occurs only after
authoritative logical commit.

Candidate commit returns `{ snapshot, cleanupWarnings }`. Candidate `Open`,
previous registry removal, and `active_session_id` replacement occur inside one
lifecycle-gate boundary. A later physical private-directory cleanup failure is
reported as a path-free warning and does not turn logical commit into an error.

After a lost IPC response the frontend reconciles using
`get_active_document_session_snapshot` and candidate lifecycle. An `Open`
candidate is committed; a pre-commit candidate is rolled back. Exhausting the
bounded retry budget enters `CommitOutcomeUnknown` and keeps later open requests
in the latest pending slot.

Activation ownership uses token-to-activation and candidate-to-token indexes.
Lazy activation expiry removes both indexes and returns the candidate to
`CandidateReady` while preserving its credential. Lazy transition expiry also
rolls back a linked activation and returns the old session to `Open`. Candidate
discard, activation rollback, and session close remove password-cache entries.

PDF-to-image is candidate-free but transactional: decode and buffer validation
run first, the old PDF decision is prepared second, and
`commit_session_deactivation` changes backend active authority to null before
the image becomes authoritative UI state.

## Job snapshots

Long work uses an immutable physical snapshot:

```text
instance-<nonce>/
  sessions/session-<id>/working.pdf
  jobs/job-<id>/snapshot.pdf
```

The session shared lease is held while `working.pdf` is streamed to `snapshot.pdf`,
hashed, fsynced, validated, and all source/output handles are closed. The job then
owns its directory and memory-only credential context independently. Normal
session close stops new admission but does not cancel a job that already owns a
snapshot. The owner session ID remains on the job record until terminal cleanup.
Late job output does not update a closed session.

Every snapshot command receives a frontend-generated UUID. Rust parses it with
`Uuid::parse_str()`, uses only the canonical UUID for registry and directory
identity, and atomically reserves it. Duplicate active IDs return
`SESSION_JOB_ALREADY_EXISTS`; cross-session reuse returns
`SESSION_JOB_OWNER_MISMATCH`.

All terminal operations return `Ok(SessionJobOutcome<T>)`, tagged as
`completed`, `failed`, or `cancelled`. Each variant carries `jobId`, `sessionId`,
the captured `contentGeneration`, and an independent cleanup result. Completed
also carries `cancelRequested` and optional `cancellationReason`. Operation
failure and cleanup warning can therefore coexist, while cleanup failure never
turns a completed operation into a failed IPC call. Payload, UUID, ownership,
generation, lifecycle, and admission failures remain Tauri errors.

Operation state is:

```text
CreatingSnapshot -> Ready -> Running
CreatingSnapshot | Ready | Running -> CancelRequested
Running -> Completed | Failed | Cancelled
CancelRequested -> Completed | Failed | Cancelled
```

`Completed` is retained when an irreversible side effect finishes after a cancel
request; the request remains visible through `cancelRequested` and
`cancellationReason`. Internal closures return a typed operation error, so
cooperative cancellation is never represented as an ordinary string failure.

Cleanup state is independent:

```text
Pending -> InProgress -> Cleaned
                      -> RetryScheduled
                      -> Deferred
```

Credential context is cleared at terminal operation state. Cleanup remains
single-flight per job. After five failed attempts a record moves from the active
registry to the bounded deferred-cleanup map only if that insertion succeeds.
When deferred capacity is full, the active terminal record and cleanup path are
retained and new job admission receives `SESSION_JOB_CLEANUP_BACKPRESSURE`.

One background reaper starts during application setup. It prunes bounded
cancel-before-registration entries, retries due cleanup records, and owns no
session credential. Pending cancellations use `(sessionId, canonicalJobId)`, a
60-second TTL, and a 256-entry limit. App shutdown stops the reaper, requests
`shutdown` cancellation for all active jobs, performs a bounded quiesce, and
runs one last best-effort pass. In-memory state is not persistent across process
exit; any remaining instance directory is left to the existing startup orphan
defence. Startup orphan cleanup itself is unchanged.

The active-PDF macOS Share Sheet, Apple Vision page OCR, prepress operations,
and other long-running exports use this snapshot boundary. External image/file
sharing is intentionally separate as `share_external_file_macos(path)` and must
never identify the active PDF. The process runner used by the macOS share bridge
has a bounded timeout and kills/reaps a timed-out child.

### Snapshot-job cancellation boundaries

| Command | Target class | Checkpoint |
|---|---|---|
| `detect_scanned_pages` | `boundary-only` | before PDFium text extraction (one document-wide job) |
| `run_ocr_on_page` | `boundary-only` | before PDFium render/Tesseract |
| `export_as_searchable_pdf` | `boundary-only` | unsupported scaffold entry only |
| `apply_redactions` | `unavailable` | `FEATURE_UNAVAILABLE` before snapshot/parser |
| `sanitize_pdf_document` | `unavailable` | `FEATURE_UNAVAILABLE` before snapshot/parser |
| internal `execute_document_sanitization_core` | `cooperative → atomic disk truth` | inspection/transform/verifier and pre-publication checkpoints; no cancellation once shared verified-artifact replace begins |
| `scan_sensitive_data` | `cooperative` | cooperative ephemeral job-worker scan |
| `share_file_macos` | `child-process-killable` | tracked Swift child |
| `perform_ocr_on_page` | `child-process-killable` | render boundaries plus tracked Vision child |
| `export_session_page_to_image` | `boundary-only` | single PDFium render/encode |
| `export_session_pages_as_images` | `cooperative` | between pages; each page render remains a boundary |
| `split_session_pdf` | `cooperative` | ranges, pages, staging and publish boundaries |
| `merge_session_pdfs` | `cooperative` | input document and page-copy loops |
| `export_session_text_content` | `cooperative` | between pages; each text extraction remains a boundary |
| `export_session_text_to_file` | `cooperative` | same extraction boundaries; checked again before the atomic write, which is not interrupted once started |
| `export_session_document` | `child-process-killable` | the job-worker child reads and writes the whole file; publication of the finished file is not interrupted |
| `print_session_pdf` | `child-process-killable` | page preparation plus tracked spooler child |
| `inspect_session_print_resources` | `cooperative` | bounded resource traversal and private worker cancellation marker |
| `generate_session_rgb_derived_coverage_preview` | `cooperative` | PDFium render boundaries, bounded scanline groups, PNG encode and serialization checkpoints |
| `generate_session_rgb_derived_process_preview` | `cooperative` | PDFium render boundaries, bounded scanline groups, between-PNG and serialization checkpoints |
| `extract_session_pages` | `boundary-only` | existing atomic document-IO pipeline |
| `build_semantic_index` | `cooperative` | page/chunk/request boundaries |

### Multi-file output publication

`export_session_pages_as_images` and `split_session_pdf` write several files
into a user-chosen folder. Both take a required `overwrite: bool` and publish
through `document_io::publish_output_set`:

- **Planned names.** The worker writes private names inside the job directory
  (`sayfa_{n}.{png|jpg}`, `split_{rangeIndex}_{start}.pdf`) and must return
  exactly the planned files, in order, or the job fails with
  `SESSION_JOB_WORKER_PROTOCOL_ERROR`. The main process publishes each one under
  a document-based name (`output_names.rs`): `{stem}_sayfa_{n}.{png|jpg}` for
  images and `{stem}_sayfa_{start}-{end}.pdf` (`{stem}_sayfa_{n}.pdf` for a
  single page) for split ranges, all 1-based. `stem` is the session's current
  save-target file name without `.pdf`, with `< > : " / \ | ? *`, control and
  bidi characters replaced by `_`, edge dots/spaces trimmed, at most 120 bytes,
  Windows device names (`CON`, `NUL`, `COM1`…) suffixed with `_`, and `belge`
  when nothing is left. The same split range given twice is rejected with
  `SESSION_JOB_INVALID_SPLIT_RANGES` before any work.
- **Conflict policy.** Before the worker runs, and again at publish time, every
  planned target is inspected. With `overwrite: false` any existing entry fails
  the job with `SESSION_JOB_OUTPUT_CONFLICT` and `error.conflictingNames`
  (file names) and nothing is written. The frontend
  (`services/app/outputConflict.js`) asks the user and, on confirmation,
  repeats the call once with `overwrite: true`. Existing entries that are not
  regular files (directory, symlink, reparse point) fail with
  `SESSION_JOB_OUTPUT_TARGET_UNSUPPORTED` in both modes and are never replaced.
- **All-or-nothing.** Every output is first copied to a hidden, fsynced
  `.antigravity-write-*` temp beside its target. Only after all are staged are
  they renamed into place. Targets that did not exist are created with a
  no-clobber rename (`persist_noclobber`), so a file that appears after
  inspection is reported as a conflict instead of being replaced; on
  filesystems without exclusive rename or hard links this degrades to an
  existence check plus rename. Targets being overwritten are first kept as
  hidden `.antigravity-backup-*` hard links (a copy where hard links are
  unsupported). If staging, a rename, or cancellation fails mid-way, files
  already created by this job are removed and overwritten targets are restored
  from their backups; the job reports `SESSION_JOB_OUTPUT_WRITE_FAILED`,
  `SESSION_JOB_OUTPUT_CONFLICT`, or `Cancelled`. If the rollback itself fails,
  the job reports `SESSION_JOB_OUTPUT_ROLLBACK_FAILED` naming each leftover file
  and any backup kept for a target that could not be restored.
- **Limits.** This is not a PDF document transaction: there is no path lock and
  no post-commit verification. A process abort or machine crash during the
  rename phase can leave a partial set, temps, or backups; they are not
  scavenged by name.

## Temp ownership and platform security

Each process owns a sibling lock and instance directory:

```text
app-temp-root/
  instance-<nonce>.lock
  instance-<nonce>/
```

Cleanup obtains the sibling lock nonblocking, verifies paths, renames the instance
to a quarantine name, deletes quarantine, closes the lock handle, then removes the
lock file. Unix directories/files use `0700`/`0600`. Windows applies and verifies a
protected current-user-only DACL using `SetEntriesInAclW`,
`SetNamedSecurityInfoW`, and `GetNamedSecurityInfoW`. Failure is
`TEMP_SECURITY_ACL_UNAVAILABLE`; no temp-backed session or job is created.

Atomic output locks use canonical path plus file identity for existing targets.
For missing targets the key is canonical parent plus normalized filename (Windows
namespace comparison is case-insensitive). Target existence and the baseline
precondition are checked again while the target lock is held.

## VectorStore Derived State ve Kilit Sıralaması (Prompt 9A.9)

- **Session Yaşam Döngüsü**: `VectorStore` indeksleri oturum ID (`sessionId`) ve `expectedContentGeneration` ile eşleştirilir.
- **Budama ve Temizlik**:
  - `clear_session_derived_state()`: Oturum kapandığında veya sıfırlandığında `vector_store.remove_session(session_id)` çağrılır.
  - `close_all_document_sessions()`: Tüm oturumlar sonlandırıldığında `vector_store.clear_all()` çağrılır.
  - Generasyon değişiminde veya model/endpoint uyumsuzluğunda eski indeksler atomik staging ve pruning mekanizmalarıyla temizlenir.
- **Kilit Hiyerarşisi (Lock Ordering Invariant)**: `DocumentSession` durum kilitleri hiçbir koşulda `VectorStore` RwLock'u, HTTP ağ çağrıları veya IPC worker operasyonları sırasında tutulamaz.

## Plugin Subsystem & Worker Demarcation (Prompt 9B)

- **Worker Ayrımı ve İzolasyon**: Eklenti alt sistemi (`PluginState`, `PluginExecutionRegistry`) ve subprocess worker süreçleri (`--run-plugin-worker`) `DocumentSession` yapısından, `SessionWorker` havuzundan ve PDFium motorundan tamamen izoledir.
- **Sıfır Oturum / Belge Erişimi**: Plugin worker süreçlerine hiçbir oturum tanıtıcısı (`sessionId`), `generation`, dosya yolu, PDFium instance handle'ı, parola veya ham PDF baytı iletilmez.
- **Durum Dokunulmazlığı**: Plugin worker'lar `DocumentSession` state'ini okuyamaz veya mutate edemez. Belge açılış kancalarında (`on_document_open`) yalnızca anonimleştirilmiş ve denetimli meta veriler (örn. sayfa sayısı) iletilir.

## Signing and LTV Session Non-Mutation Invariant

> [!NOTE]
> **Design-Only / Future PAdES Implementation Invariant**: The signing and LTV worker pipeline described here is not yet implemented in production. The constraints below are the normative architectural invariants that **must be enforced when implemented**. No production code currently creates `SignaturePrepWorker`, `SignatureFinalizationWorker`, `SignatureVerificationWorker`, or `RevocationAndTimestampWorker` processes, nor enforces `SIGNATURE_TARGET_PROTECTED_PATH` at `target_preflight`. The signature feature surface has been removed from production (`status: disabled`); this section records the future-implementation contract.

Signing (digital signature) and LTV (Long-Term Validation upgrade) operations **must never mutate the active `DocumentSession` state** when implemented. This is a normative constraint that must be enforced at the architecture level.

- Signing must use an immutable snapshot (`jobs/job-<uuid>/snapshot.pdf`), not the session's `working.pdf`.
- The signed output must be written to a user-selected external target; it must never replace `working.pdf`, `original_path`, or any session/job directory path (`SIGNATURE_TARGET_PROTECTED_PATH` must be enforced at `target_preflight` when implemented).
- On signing completion (success or failure): `content_generation`, `state_revision`, `dirty`, and session lifecycle state must remain unchanged.
- LTV upgrade must operate on an immutable snapshot of the signed PDF (typically an external file), not the active session working copy. No boolean toggle (`is_ltv_enabled`) must represent LTV state; each LTV upgrade must produce an independent output PDF at a separate external target.
- If the session is closed or `content_generation` changes after a signing job starts, the job output must not be applied back; the job must reach `Cancelled` or `Failed` terminal state without publishing.

For the full normative specification of these invariants, including the Worker/Job/Atomic-Publication lifecycle diagram, see `pades-architecture-design.md §6.6`.

# Accessibility audit read boundary

`audit_session_accessibility` and `get_session_structure_tree` are session-native,
generation-checked document reads. They acquire the existing worker document and use
its cached accessibility source. They do not mutate the session snapshot, dirty flag,
state revision, content generation, working PDF, or output filesystem. See
`basic-accessibility-audit.md` for result semantics and limits.
