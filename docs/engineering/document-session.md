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
  to be committed).
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
| `NewCommitted` | dirty; generation and revision advance; candidate digest becomes working digest |
| `NewCommittedDurabilityUnconfirmed` | same as committed, plus durability warning |
| `Prepared`, `OriginalPreserved` | mutation not applied; generation/revision unchanged |
| `OriginalRecovered` | working file is re-hashed; old digest means not applied, candidate digest means applied, otherwise indeterminate |
| `RecoveryRequired` | dirty, digest unknown, lifecycle becomes recovery-required |
| `Indeterminate` | dirty, digest unknown, lifecycle becomes commit-indeterminate |

## Atomic Save mapping

| CommitState | Session result |
| --- | --- |
| `NewCommitted` | baseline/target update; dirty clears; revision advances |
| `NewCommittedDurabilityUnconfirmed` | same as committed, plus durability warning |
| `Prepared`, `OriginalPreserved`, `OriginalRecovered` | dirty and working copy remain available; typed error returned |
| `RecoveryRequired` | dirty remains; lifecycle blocks normal work pending recovery |
| `Indeterminate` | dirty remains; lifecycle blocks normal work pending recovery |

Normal Save supplies the opening/last-save SHA-256 baseline as an atomic commit
precondition. A missing or changed target is a typed conflict and never overwrites
external content. Save As retargets normal Save after a verified commit.

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

The active-PDF macOS Share Sheet, Apple Vision page OCR, signature verification,
and other long-running exports use this snapshot boundary. External image/file
sharing is intentionally separate as `share_external_file_macos(path)` and must
never identify the active PDF. The process runner used by the macOS share bridge
has a bounded timeout and kills/reaps a timed-out child.

### Snapshot-job cancellation boundaries

| Command | Target class | Checkpoint |
|---|---|---|
| `detect_scanned_page` | `boundary-only` | before PDFium text extraction |
| `run_ocr_on_page` | `boundary-only` | before PDFium render/Tesseract |
| `export_as_searchable_pdf` | `boundary-only` | unsupported scaffold entry only |
| `sign_pdf_pades` | `cooperative` | each scaffold copy chunk; no real PAdES implementation is added |
| `verify_pdf_signatures` | `boundary-only` | mock verification entry only |
| `apply_redactions` | `unavailable` | `FEATURE_UNAVAILABLE` before snapshot/parser |
| `sanitize_pdf_document` | `unavailable` | `FEATURE_UNAVAILABLE` before snapshot/parser |
| `scan_sensitive_data` | `cooperative` | cooperative ephemeral job-worker scan |
| `share_file_macos` | `child-process-killable` | tracked Swift child |
| `perform_ocr_on_page` | `child-process-killable` | render boundaries plus tracked Vision child |
| `burn_annotations_to_pdf` | `cooperative` | page/annotation loops and before save |
| `export_session_page_to_image` | `boundary-only` | single PDFium render/encode |
| `export_session_pages_as_images` | `cooperative` | between pages; each page render remains a boundary |
| `export_session_pages_as_pdf_x` | `boundary-only` | existing prepress export boundary |
| `split_session_pdf` | `cooperative` | ranges, pages, staging and publish boundaries |
| `merge_session_pdfs` | `cooperative` | input document and page-copy loops |
| `export_session_text_content` | `cooperative` | between pages; each text extraction remains a boundary |
| `print_session_pdf` | `child-process-killable` | page preparation plus tracked spooler child |
| `analyze_session_color_spaces` | `boundary-only` | existing prepress analysis boundary |
| `calculate_session_ink_coverage` | `cooperative` | render boundaries and each bitmap scanline |
| `render_session_separation_channels` | `boundary-only` | existing native separation render boundary |
| `extract_session_pages` | `boundary-only` | existing atomic document-IO pipeline |
| `build_semantic_index` | `cooperative` | page/chunk/request boundaries |

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
