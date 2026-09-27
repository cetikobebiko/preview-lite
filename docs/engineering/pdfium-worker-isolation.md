# PDFium Worker Isolation

## Scope and guarantee

The production session-native paths below execute user-controlled PDF parsing and read-only
PDFium work in the `--run-worker` process:

- `open_document_session`
- `authenticate_document_session`
- `probe_candidate_render`
- `get_session_page_count`
- `get_session_pdf_outline`
- `get_session_pdf_metadata`
- `get_session_page_annotations`
- `get_session_structure_tree`
- `audit_session_accessibility`
- `render_session_page`
- `render_session_thumbnail`
- `get_session_page_text_layout`
- `search_session_pdf_text`

These paths do not fall back to main-process PDFium when the worker is unavailable, crashes,
times out, violates the protocol, or is quarantined. The frontend continues to use the existing
session ID, content generation, page index, render request ID, scale/rotation, and thumbnail
contracts. Worker document IDs are never returned to the frontend.

Heavy snapshot jobs do not use this long-lived viewer worker. They execute in a separate,
single-command `--run-job-worker` process described below. PDF mutation/atomic-output code that
does not use PDFium remains outside the viewer worker; disabled or mock PDF operations are either
unregistered or fail with `FEATURE_UNAVAILABLE` before a snapshot is created or parsed.

## Ephemeral snapshot job worker

Every production session job first creates `jobs/job-<uuid>/snapshot.pdf`. Snapshot validation
itself then executes in an ephemeral job worker, so malformed input cannot crash the Tauri
process during the admission check.

The active job-worker commands cover:

- OCR scanned-page detection and OCR/Apple Vision render preparation;
- AI/RAG semantic-index text extraction;
- text export;
- page/image export;
- print-range PDF preparation;
- prepress color-space analysis, ink-coverage render, and separation render;
- recovery snapshot validation and normal session-job snapshot validation;
- sensitive-data scanning.

Sanitization remains feature-disabled, but its read-only
`inspect_document_sanitization` command now uses an immutable session snapshot and the ephemeral
`InspectDocumentSanitization` job worker. The Tauri process validates/routs the typed report and
does not parse the PDF. JavaScript stream payloads that require decoding are accounted against
per-stream and total decoded-byte budgets after lopdf's whole-buffer decode returns; the parser
allocation itself is bounded by process isolation, address-space limits, and the parent timeout,
not by a cooperative bounded-decode API. Sanitization cancellation is parent-enforced process
kill + wait/reap; there is no production worker-side cooperative cancellation IPC. Legacy
8C.3 additionally provides a private `SanitizePdf` ephemeral operation. It receives only the
immutable snapshot, versioned policy/digest authority and a fixed internal output name; lopdf
targeted mutation, pruning, fingerprints and full serialization remain inside the worker. The
main process only validates the typed report, its exact requested policy/profile/control-set
correlation, and private artifact size/digest/containment. Transform fingerprinting reuses the
sanitization per-stream and total decoded-byte limits after lopdf's whole-buffer decode returns;
this is post-decode enforcement, not a streaming pre-allocation claim. The artifact is valid
only inside the active snapshot-job closure. 8C.4 now starts a distinct `VerifySanitizedPdf`
ephemeral process after a parent-side pre-verification digest gate. That process derives only the
fixed private artifact name, performs a fresh lopdf/object-graph/fingerprint/full-rewrite scan,
reopens and smoke-renders with PDFium, checks page/document preservation facts, and recomputes the
artifact digest without mutation. The parent revalidates the strict report and transform/verifier/
actual digest equality before accepting an internal verified candidate. Neither worker operation
is registered as a public Tauri command or receives a target. 8C.5's internal main-process core
may publish only that verified artifact through the shared adjacent-temp/atomic-replace
`document_io` boundary before terminal cleanup; it performs no lopdf/PDFium reopen. The legacy
`sanitize_pdf_document`, searchable-PDF export, and PDF/X export return `FEATURE_UNAVAILABLE`
before their transformation paths run, while legacy mock signature/PAdES handlers have been completely
removed from Tauri command registration. The obsolete raw
annotation-burn and raw merge handlers are not registered.

Verifier closure: the fresh 8C.2 scanner's registry-derived blocking findings are mandatory
verification evidence, including unsupported XFA, optional content, rich-media/rendition and
encryption categories. Decoded payload absence is authoritative: an undecodable scanned stream
fails the mandatory payload check and raw/encoded bytes cannot turn it into a pass. The verifier
reuses the 16 MiB per-stream and 128 MiB total decoded-byte limits after lopdf's whole-buffer
decode, and separately bounds actual parsed/PDFium output page count to 10,000 before page
invariants or render sampling proceed. These private transform→verify steps remain in one active
snapshot-job lifecycle; publication is still absent and must be inserted before terminal cleanup.

The job flow is:

```text
immutable jobs/ snapshot
  -> private, create-new bounded request file
    -> --run-job-worker child owned by SessionJobChildProcessOwner
      -> one bounded command
        -> typed response or private job-directory output file
      -> worker exit
    -> timeout/cancel/shutdown: kill + wait
  -> main process validates output is inside the job directory
    -> atomic staging/persist to an external export target, or typed result acceptance
```

The request limit is 1 MiB and the typed response limit is 256 MiB. Text output is limited to
32 Mi characters and page-list operations to 10,000 pages. The worker applies the same Unix
address-space resource ceiling as the viewer worker. Request and response files are create-new,
mode `0600` on Unix, and live only in the private session-job directory. Credentials are never
placed in process arguments or logs.

Each invocation gets an independent process, timeout, parent-enforced cancellation, resource limit,
and child ownership record. A long OCR or prepress operation therefore cannot block viewer
rendering, and a job crash does not invalidate open viewer-worker document handles.

## Previous call graph

```text
frontend backend service
  -> Tauri document_session command
    -> SessionEntry operation lease
      -> working path / credential snapshot
        -> pdf_engine main-process helper
          -> spawn_blocking
            -> PDFIUM_MUTEX
              -> get_pdfium()
                -> load_pdf_from_file() / render / text / search
```

`SubprocessManager` was used only by `tests/crash_tests.rs`. Its protocol had no version,
request correlation, handshake, framing limit, restart epoch, or session ownership.

## Current call graph

Non-render reads use:

```text
broker admission
  -> session read lease
    -> revalidate lifecycle + expected content generation
      -> short state snapshot (working path + credential)
        -> ensure worker document for (session ID, content generation, worker epoch)
          -> worker command
```

Render and thumbnail requests first pass the existing render coordinator and cache. The renderer
closure then acquires broker admission, acquires the session read lease, revalidates generation,
and dispatches the worker command. A queued render therefore holds neither a session read lease
nor the broker's active slot.

Exclusive mutation/close admission has priority over queued reads. Starting an exclusive action
blocks and cancels queued reads for that session. A backlog for Session A cannot indefinitely
starve an exclusive close or mutation for Session B.

The active worker command retains both broker admission and the session read lease until the
response arrives or cancellation/timeout kills and reaps the worker.

## Protocol version and envelopes

Protocol version: `2`.

```text
WorkerRequest {
  protocol_version: u16,
  request_id: u64,
  command: WorkerCommand,
}

WorkerResponse {
  protocol_version: u16,
  request_id: u64,
  result: Result<WorkerResult, WorkerFailure>,
}
```

Every spawn completes `Hello` before normal commands. The response declares readiness,
capabilities, and resource-limit diagnostics. Protocol version, capability set, and request ID
must match. IPC request IDs are monotonically increasing `u64` values owned by the broker; they
are distinct from frontend render request IDs.

Commands are `Hello`, `OpenDocument`, `CloseDocument`, `CloseAllDocuments`, `GetPageCount`,
`RenderPage`, `RenderThumbnail`, `GetPageTextLayout`, `SearchText`, `GetOutline`, `GetMetadata`,
`GetPageAnnotations`, `GetStructureTree`, `AuditAccessibility`, `Ping`, and `Shutdown`.

Worker protocol version 3 returns `AccessibilityStructure` and `AccessibilityAudit`.
Both read the cached worker-owned accessibility document; no active PDF is reopened
in the main Tauri process. Audit traversal adds bounded structure, RoleMap, ParentTree,
XMP, evidence, report-size, visited-object, and cooperative-time limits. Malformed
structure remains a successful bounded audit report; source/load, timeout, crash,
protocol and response-size failures remain typed worker-operation errors.

## Framing and message limits

Frames are a four-byte little-endian length followed by fixed-integer bincode data.
Deserialization uses a bincode byte limit and rejects trailing bytes.

| Limit | Value | Basis |
|---|---:|---|
| Request frame | 64 KiB | 4096-character path, 4096-character credential, 8192-character query, and envelope overhead |
| Response frame | 256 MiB + 64 KiB | Existing `MAX_RENDER_BYTES` plus bounded envelope overhead |
| Render dimensions | 16,384 x 16,384 maximum dimension | Existing render-service contract |
| Render RGBA | 256 MiB | Existing `MAX_RENDER_BYTES` |
| Text layout items | 250,000 | Prevent unbounded item vectors |
| Search results returned | 10,000 | Prevent unbounded result/geometry responses |
| Cached accessibility source | 64 MiB | Bounds the document-owned structure-tree/PDF/UA parser view |
| Password-protected in-memory PDF | 256 MiB | Credential-safe owned-byte load within the worker resource envelope |

The Unix address-space ceiling is 2 GiB. Its explicit peak model reserves one 256 MiB PDFium
bitmap, one 256 MiB RGBA result, one 256 MiB serialized response copy, an optional 256 MiB
password-protected document buffer, 512 MiB PDFium/runtime headroom, and another 512 MiB safety
margin. A unit invariant prevents the bounded peak model from exceeding the address-space limit.

The frame length is checked before allocation; `u32`/`usize` and serialized length conversions
are checked. Partial header, partial payload, malformed bincode, trailing bytes, oversized
messages, wrong protocol version, and wrong request ID are errors rather than panics.

Worker stdout is binary protocol only. Worker diagnostics go to stderr. The parent drains stderr
continuously and keeps only a bounded diagnostic buffer for process tests; command payloads,
paths, and credentials are not logged.

## Worker ownership and epochs

`PdfiumWorkerBroker` owns:

- child process, stdin, stdout, and stderr drain task;
- next IPC request ID and next worker epoch;
- document registry;
- admission queue;
- global restart history;
- per-document categorized failure history and quarantine state.

There is one worker and one active command. Each successful spawn gets a new `worker_epoch`.
Every admission waiter is owned by an RAII registration from queue insertion through permit
construction. The active slot carries the exact ticket, session, and read/exclusive mode. Task
abort or cancellation removes either the queued registration or delivered handoff and dispatches
the next waiter. Exclusive session blocking is reference-counted, so concurrent exclusive waiters
cannot clear each other's block.

Permits also contain the controller identity. Broker operations validate broker, session, active
ticket, and mode. Read permits authorize only read/open commands; document/session invalidation
requires an exclusive permit. Reservation abort uses a narrower key-and-document-reference-bound
internal primitive.

```text
WorkerDocumentKey = (session_id, content_generation)
WorkerDocumentRef = (worker_epoch, worker_document_id)
```

A restart clears all document references. It does not change session content generation, dirty
state, state revision, transition state, or snapshots. The next read lazily reopens the current
working copy. A numeric worker document ID from an older epoch is never reused as an identity.

## Transactional open and credentials

Candidate open and authentication use a `WorkerDocumentReservation`. A successful worker open is
armed until session/candidate state is connected. Error, early return, cancellation, or unwind
drops transfer cleanup ownership to the broker queue; explicit abort waits for close and falls
back to worker kill/reap if close fails. Drop is not fire-and-forget: it transfers the exact
key/reference to a broker-owned cleanup queue that is drained before the next operation and during
shutdown. Ordinary fallible flows still call explicit async abort. Commit disarms the reservation.

Password-required and wrong-password loads never enter the worker document registry. Credentials
use a redacted wrapper whose `Debug` output contains no secret. `Box::leak` is not used.

For unencrypted files PDFium uses its file-backed loader. For password-protected files the worker
loads an owned byte vector; this avoids tying `PdfDocument` to borrowed password storage while
keeping both PDF data and credential owned by the worker document record. Document close,
restart, or shutdown drops the credential and document.

Outline, metadata, and page annotations are read directly from that open `PdfDocument`. PDFium
does not expose its structure-tree wrappers through the pinned `pdfium-render` public API, so the
worker document record also builds one cached, read-only accessibility view during
`OpenDocument`. Structure-tree and PDF/UA commands query that owned view by `document_id`; they
never receive a path or reopen the file per request. Mutation invalidation drops both views and
the next generation builds a fresh worker document record.

## Render, text, and search behavior

Render uses the existing scale/target-width, rotation, dimension, pixel, cache, and stale-result
contracts. PDFium BGRA bytes are converted to RGBA8 in the worker. Both worker and broker-side
render flow validate `width * height * 4` with checked arithmetic.

Text layout retains the production word grouping behavior:

- NBSP normalization and soft-hyphen/zero-width filtering;
- finite positive character bounds;
- whitespace, horizontal-gap, and line-change boundaries;
- normalized top-left coordinates;
- bounded item count.

Search retains lowercase plus NFKC normalization, normalized-to-original character mapping,
page order, snippets, line-grouped geometry, and global match indices. Normalization operates on
base-plus-combining-mark clusters, equates composed/decomposed forms, normalizes NBSP, removes soft
hyphen/zero-width discretionary marks, and handles dotted capital Turkish İ consistently with
lowercase i. Empty queries return an empty result before worker dispatch. Query length and
returned-result count are bounded.

Render cache keys continue to include session identity, content generation, page index, sizing,
rotation, and render scope. A worker restart alone does not change content generation or require
cache invalidation.

## Timeout and cancellation

| Command class | Timeout |
|---|---:|
| Hello | 5 seconds |
| Ping, Close, Shutdown | 2 seconds |
| Page count | 5 seconds |
| Thumbnail | 10 seconds |
| Open document, text layout | 15 seconds |
| Main render | 20 seconds |
| Search | 30 seconds |

These values replace the former universal three-second watchdog. Small committed fixtures are
measured by integration tests; the values remain conservative for slower CI. A single timeout
fails only that request, kills and reaps the worker, and invalidates the epoch. The active command
is not replayed.

Queued cancellation removes the request before it holds a session lease. Cancelling an active
native command kills and reaps the worker because PDFium may not return to read a cancel command.
Wrong-session cancellation only removes matching session queue entries.

## Restart and quarantine

Global process restarts and document failures have separate budgets.

- Global budget: five spawn attempts in a rolling 60-second window.
- Document failure window: 60 seconds per `(session_id, content_generation)`.
- Quarantine: three failures of the same category within the window.
- Categories: crash, timeout, and protocol violation are counted separately.

One timeout does not quarantine a document. Generation change, session invalidation/close, or
candidate discard clears the relevant document failure and quarantine records. Quarantine changes
neither dirty state nor content generation and prevents automatic rerender loops for that
generation.

## Mutation invalidation

Working-copy mutations use:

```text
mark session exclusive and cancel queued reads
  -> acquire broker exclusive admission
    -> acquire session write lease
      -> revalidate expected generation and snapshot mutation inputs
        -> CloseDocument for generation N
          -> on close failure: kill + wait
            -> only after confirmed process cleanup, create immutable mutation snapshot
              -> bounded ephemeral mutation worker writes a private output
                -> separate bounded worker reopens and validates the output
                  -> main process verifies the output path and SHA-256
                    -> existing atomic replace commits filesystem bytes
                      -> success: apply_committed_mutation increments generation
                      -> failure: original working file and generation remain unchanged
```

This shared path covers rotate, delete, reorder, blank-page insertion, and annotation add/delete.
Unsupported metadata and alt-text writes remain unsupported. Save and Save As copy the working
file to an external target and do not replace the working copy, so they do not close a worker
document unnecessarily; their PDFium reopen validation nevertheless runs in an ephemeral worker.

## Mutation and atomic validation isolation

The Prompt 4 transaction core is unchanged. It still owns path locks, target preconditions,
same-directory temp creation, permission/ACL preservation, fsync, atomic rename/replace, recovery
classification, and commit-state reporting. The isolation layer only moves PDFium work ahead of
that filesystem commit:

1. The main process holds the session write lease and invalidates the long-lived viewer handle.
2. It byte-copies the current working file into a private immutable mutation exchange directory.
3. A typed `MutationRecipe` is executed by a bounded `--run-job-worker` process. PDFium load,
   mutation, save, and operation-specific invariant checks all occur there.
4. A new worker process reopens the private output and returns `MutationValidated`.
5. The main process accepts only a canonical file inside the owned exchange directory, binds it
   to a SHA-256 digest, and calls `atomic_commit_prevalidated_pdf`.
6. The existing atomic core copies those already validated bytes to its same-directory temp,
   rechecks the digest and source identities, then performs the existing atomic commit.

A mutation or validation crash therefore happens before replacement begins. The original and
working copy remain intact and the session generation/dirty state is updated only from the
existing typed commit report.

Session split, merge, and page-extraction outputs use the same ephemeral worker plus separate
reopen-validation rule. Additional merge inputs are byte-copied into the private job directory
before PDFium sees them. Output publication remains the existing job-output staging operation.

Candidate discard, activation rollback, prior-session activation cleanup, deactivation, direct
session close, and close-all invalidate worker state before deleting working directories.

## Shutdown

Tauri `ExitRequested` is prevented once and starts an asynchronous, bounded coordinator. It does
not call `block_on` on the event loop.

1. Global shutdown token is cancelled and further broker admission stops.
2. Admission `closed` is committed under the queue lock; queued waiters receive a typed shutdown
   error and can never receive a later permit.
3. Worker shutdown/reap and existing session-job shutdown execute concurrently under their
   independent single-owner registries.
4. Active commands, render/search/text tokens, and session-job child processes are cancelled,
   killed if required, and waited.
5. After `ShutdownComplete`, the broker performs bounded `child.wait`; timeout triggers kill and a
   second bounded wait before stderr drain and registry cleanup.
6. Worker stderr drain and existing job process drains are joined or boundedly aborted.
7. Only after both ownership systems finish are session write leases acquired and working
   directories removed. Lease or filesystem failure becomes a structured deferred-cleanup warning
   naming the session; it is not silently skipped.
8. The coordinator exits the app; repeated exit requests await the same bounded result.

No child is registered in both ownership systems.
The former outer 12-second timeout was removed because dropping the whole cleanup future could
strand half-finished ownership transitions. Admission drain, command/ACK, child wait/kill,
stderr join, session-job quiescence, and session lease cleanup now carry their own bounded
deadlines.

## Typed failures

Stable lifecycle codes include:

- `PDFIUM_WORKER_UNAVAILABLE`
- `PDFIUM_WORKER_SPAWN_FAILED`
- `PDFIUM_WORKER_PROTOCOL_MISMATCH`
- `PDFIUM_WORKER_PROTOCOL_ERROR`
- `PDFIUM_WORKER_MESSAGE_TOO_LARGE`
- `PDFIUM_WORKER_TIMEOUT`
- `PDFIUM_WORKER_CRASHED`
- `PDFIUM_WORKER_RESTART_LIMIT`
- `PDFIUM_WORKER_CANCELLED`
- `PDFIUM_WORKER_CLEANUP_FAILED`
- `PDFIUM_WORKER_INTERNAL`
- `PDFIUM_WORKER_LOAD_FAILED`
- `PDFIUM_WORKER_RENDER_FAILED`
- `PDFIUM_WORKER_ADMISSION_INVALID`
- `PDFIUM_DOCUMENT_QUARANTINED`

Document/password/render categories are mapped separately. `DocumentSessionError` serializes the
code and message. The central frontend backend client copies the code onto `BackendCommandError`
while preserving the original cause and message. It does not copy request payloads or credentials.

## Platform and CI status

PDFium-independent protocol, fake-process, queue/admission, timeout, kill/reap, epoch, restart
budget, quarantine, shutdown, session ownership, architecture, and frontend error tests are
configured for Linux, macOS, and Windows.

The repository currently proves a real PDFium runtime only for macOS arm64. That job runs worker
load/render/rotation/thumbnail/text/search, encrypted/malformed PDF, forced kill, restart, and lazy
reopen tests. Windows uses fake-worker lifecycle and handle-order tests; this is not a claim that
real Windows PDFium integration has been verified. Linux real PDFium integration is likewise not
claimed.

The Unix worker attempts a 2 GiB address-space limit. Failure is reported in handshake
diagnostics rather than silently ignored. macOS may report this limit as unavailable. Windows Job
Object containment is not implemented and is explicitly reported as unavailable; process
kill/reap isolation still applies.

## Local verification (2026-07-29, macOS arm64)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo check --locked --all-targets` | Passed; existing dead-code warnings remain |
| `cargo test --locked --all-targets -- --test-threads=1` | Passed: 147 library tests plus all integration suites; mutation/reopen and split/merge process tests included |
| `cargo test --locked --test worker_broker_process -- --test-threads=1` | Passed: 16 |
| `cargo test --locked --test worker_pdfium_integration -- --test-threads=1` | Passed: 4, including worker-owned interactive reads, Unicode parity, and a >240 MiB render |
| `cargo test --locked --test crash_tests -- --test-threads=1` | Passed: 2 |
| `cargo test --locked --test session_worker_integration -- --test-threads=1` | Passed: 1 |
| `npm run test` | Passed: 338 |
| `npm run build` | Passed; existing Svelte/accessibility and chunking warnings remain |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed; enforced as a CI gate. See `strict-clippy.md` for the category-by-category cleanup report. |

## Remaining native crash surface

Registered session-native viewer reads, active snapshot jobs, mutations, split/merge/page
extraction, recovery validation, and production atomic-output validation do not load user PDFs
with PDFium in the Tauri process. Legacy raw-path mutation helpers still contain the PDFium
implementation because ephemeral workers call those pure helpers, but those raw handlers are not
registered. Test-only validation branches may execute PDFium in-process so filesystem transaction
tests can compile as standalone modules; production builds select the worker branch.

## Plugin Subsystem Demarcation (Prompt 9B)

The isolated Rhai plugin worker (`--run-plugin-worker`) is completely distinct and isolated from the PDFium session worker (`--run-worker`) and job worker (`--run-job-worker`):
- **No PDFium Linkage**: Plugin subprocesses never initialize or invoke PDFium, nor do they access PDFium dynamic library symbols.
- **No IPC Cross-Talk**: There is no IPC channel between plugin workers and PDFium workers. All plugin coordination is handled exclusively by the Tauri host supervisor under strict capability-gated boundaries.
- **Immutable Boundaries**: PDF text extraction and rendering remain the exclusive authority of PDFium workers; plugins cannot intercept, modify, or inject PDFium calls.

## Signature Architecture Worker Demarcation

> [!NOTE]
> **Design-Only / Future Architecture**: The signature worker processes described in this section (`SignatureInspectionWorker`, `SignaturePrepWorker`, `SignatureFinalizationWorker`, `SignatureVerificationWorker`, `RevocationAndTimestampWorker`) and the `SigningBroker` component **do not exist in the current production codebase**. The signature feature surface has been completely removed (`status: disabled`). This section documents the normative future PAdES implementation architecture. No production `--run-job-worker` process currently receives a signature command, and no platform key store is currently accessed via the signing path.

When the PAdES signing feature is implemented, dedicated ephemeral `--run-job-worker` processes should be used for signature operations, completely separate from the long-lived viewer worker (`--run-worker`) and plugin worker (`--run-plugin-worker`). Signature workers must not be registered as Tauri commands and must not receive user-facing session handles.

### Signature Worker Responsibilities (Future Design)

| Worker | Input | Responsibilities | Security Boundary |
|---|---|---|---|
| **`SignatureInspectionWorker`** | Immutable snapshot PDF path | Signature discovery, ByteRange validation, CMS parsing, offline cryptographic verification | Network: forbidden; no file writes; no private key access |
| **`SignaturePrepWorker`** | Immutable snapshot PDF path + signature configuration | Incremental update allocation; ByteRange & `/Contents` placeholder creation; SignedAttributes DER construction; digest calculation | Network: forbidden; no private key access; writes only to private job staging directory |
| **`SignatureFinalizationWorker`** | Staged PDF path + DER CMS SignedData | Inject CMS bytes into placeholder; finalize incremental revision EOF | Network: forbidden; no private key access |
| **`SignatureVerificationWorker`** | Staged/output PDF path | Independent fresh reopen; ByteRange gap verification; CMS cryptographic check; pre-publication gate | Network: forbidden; read-only; fails publication if any check fails |
| **`RevocationAndTimestampWorker`** | SHA-256 imprint hash or X.509 certificate identity only | RFC 3161 TSA requests; OCSP/CRL revocation evidence collection | Document content and filename: never exposed; only Prompt 9 network policy HTTP/HTTPS allowed; max 100 KB response body |

### Native Signing Broker (Main Process) — Future Design

The `SigningBroker` is intended to run in the Tauri main process and will be the **only** component with platform key store access when implemented:

- Must receive `{ jobId, requestNonce, signingDigest, byteRange, expiresAt }` from `SignaturePrepWorker`.
- Must validate `requestNonce`, `jobId`, and `expiresAt` (replay and TTL protection).
- Must call `sign_digest(signingDigest, algorithm)` through the `SigningKeyHandle` trait (platform API: `SecKeyCreateSignature` on macOS / NCrypt on Windows).
- Must return the signed value for CMS `SignedData` construction.
- Must **never** receive raw PDF bytes, file paths, or document content.
- Must **never** pass private key material to any worker.

### Worker Boundary Invariants (Must Be Enforced When Implemented)

- No signature worker must be registered as a public Tauri command or return worker document IDs to the frontend.
- Private key material must never cross the main process → worker boundary in either direction.
- All signature worker I/O must use private job directories (`jobs/job-<uuid>/`); output must be published only after `SignatureVerificationWorker` passes.
- Session `content_generation` and `state_revision` must never be modified by any signature worker. See `document-session.md § Signing and LTV Session Non-Mutation Invariant` and `pades-architecture-design.md §6.6`.
