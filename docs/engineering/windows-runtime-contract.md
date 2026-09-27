# Windows Runtime Contract — 11I-D1, 11I-D2, 11I-D3, 11I-D4 & 11I-D5

**Scope:** Production PDFium runtime foundation for Windows — deterministic, provenance-aware, architecture-bound, fail-closed resolution, supply-chain identity, loader security, and integrity contract.  
**Gate:** `11I-D1 COMPLETE` → `11I-D2 COMPLETE` → `11I-D3 COMPLETE` → `11I-D4 COMPLETE` → `11I-D5 IMPLEMENTATION: COMPLETE` (Windows Runtime Implementation Final Closure ve 11V-D / 11I-F Handoff). No binary download, strict offline.  
**Strict offline:** No network resolution attempted. Missing runtime binary reported as `RUNTIME_BYTES_UNAVAILABLE_OFFLINE`. Network attempts strictly forbidden (§0: curl, wget, GitHub API, Cargo online resolution, npm, winget, PowerShell web requests all denied).  
**Canonical sources:**
- `docs/engineering/platform-support.json` (machine-readable, `schemaVersion: "1.0.0"`, `canonicalRuntimeManifest` = self)
- `scripts/validate-pdfium-runtime-manifest.mjs` (manifest validator, node stdlib only)
- `scripts/validate-windows-packaging-contract.mjs` (packaging contract validator, node stdlib only)
- `src-tauri/src/pdf_engine/mod.rs` (canonical loader & platform resolvers)
- `src-tauri/src/pdf_engine/runtime_metadata.rs` (build-time generated constants)
- `src-tauri/src/worker/pdfium.rs` (isolated worker process loader)
- `src-tauri/src/pdf_engine/subprocess_manager.rs` (worker launcher & runtime path propagation)
- `src-tauri/build.rs` (build-time manifest parser, code generator, and pre-package verification gate)
- `src-tauri/tauri.windows.conf.json` & `src-tauri/tauri.conf.json` (packaging resource configs)

**Status:** `IMPLEMENTATION COMPLETE & HANDOFF READY` (11I-D1 COMPLETE, 11I-D2 COMPLETE, 11I-D3 COMPLETE, 11I-D4 COMPLETE, 11I-D5 COMPLETE) — Windows target specified (`x86_64-pc-windows-msvc` only), canonical runtime identity, deterministic packaged layout (`resourceDir/lib/pdfium.dll`), stdlib-based PE architecture validation with pure testable helper, fail-closed pre-package and runtime gates, offline packaging contract validators, DLL search-order risk elimination, direct canonical path binding, and 11V-D / 11V-E / 11I-F handoffs established.

---

## 1. Supported Windows Targets (Source-Backed)

| Target Triple | OS | Arch | Status | Evidence |
|---|---|---|---|---|
| `x86_64-pc-windows-msvc` | `windows` | `x86_64` | `not_evaluated` (contract defined, runtime absent in repo, metadata incomplete) | `platform-support.json:206`, `build.rs:33`, `.github/workflows/build-windows.yml:10` (`windows-latest`), `pdf_engine/mod.rs:513` (`IMAGE_FILE_MACHINE_AMD64`) |

### Strict Target Boundary
- **Single Supported Architecture:** Only `x86_64-pc-windows-msvc` is targeted.
- **Excluded / Not Supported:**
  - `aarch64-pc-windows-msvc` (ARM64 Windows) — Not present in repository source, configuration, or CI workflows; no product support.
  - `i686-pc-windows-msvc` (32-bit Windows x86) — Explicitly rejected by `verify_windows_architecture()` (`IMAGE_FILE_32BIT_MACHINE` check and `IMAGE_FILE_MACHINE_I386` rejection).
  - GNU ABI (`x86_64-pc-windows-gnu`) — Disallowed; MSVC runtime dynamic linking expected.
- **Target Invariant:** No additional Windows architecture may be introduced without an explicit architectural decision, upstream artifact provenance, SHA-256 pin, `platform-support.json` entry, and native runner evidence in `11V-D`.

---

## 2. Expected Runtime Filename and Placement

| Item | Value | Source / Evidence |
|---|---|---|
| **Filename** | `pdfium.dll` | `platform-support.json:216`, `runtime_metadata.rs:31` (`WINDOWS_RUNTIME_FILENAME`), `pdf_engine/mod.rs:300` |
| **Suffix Policy** | Bare name only (no version suffix like `pdfium.152.dll`) | `pdfium-render` crate binds to un-suffixed platform name; loader strictly checks `file_name() == RUNTIME_FILENAME`. |
| **Repository Staging** | `src-tauri/lib/pdfium.dll` | `platform-support.json:238` (`expectedStagedPath`), `build.rs:98`, `runtime_metadata.rs:35` (`WINDOWS_EXPECTED_STAGED_PATH`). |
| **Packaged Location** | `resourceDir/lib/pdfium.dll` | `platform-support.json:239` (`expectedPackagedLocation`), `build.rs:102`, `runtime_metadata.rs:37` (`WINDOWS_EXPECTED_PACKAGED_LOCATION`). |
| **Resolved Form** | Absolute, canonicalized, normalized, validated under allowlist root | Required by path safety contract; `is_path_under_allowlist` & `is_valid_runtime_file`. |

---

## 3. Runtime Ownership

| Context | Owner | Authoritative Path | Ownership Policy |
|---|---|---|---|
| **Packaged Release** | Application Bundle (Tauri Resource Directory) | `resourceDir/lib/pdfium.dll` | Release artifact must carry its own verified runtime bytes. Must NOT depend on host global libraries, system directories, CWD, or developer machine installs. |
| **Development (Local)** | Repository Staging OR Allowlisted Developer Override | `src-tauri/lib/pdfium.dll` OR `PDFIUM_PATH` | Allowed only in development builds (`IS_DEV_BUILD`). Must be absolute and under allowlisted development roots. |
| **System** | **Not an owner** | — | `bind_to_system_library()` is disallowed. System PDFium cannot be verified for release determinism. |

---

## 4. Current PDFium Loading Inventory (Source-Extracted)

The codebase contains the following PDFium loading and binding components for Windows:

### A. `get_pdfium()` (`src-tauri/src/pdf_engine/mod.rs:29-37`)
- Coordinates runtime path resolution via `resolve_runtime_path()`.
- Runs integrity and architecture validation via `verify_runtime(&runtime_path)`.
- Binds library via `Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(...))`.
- Returns `Result<Pdfium, String>` fail-closed.

### B. `get_pdfium_with_resolved_path()` (`src-tauri/src/pdf_engine/mod.rs:42-70`)
- Accepts `resolved_path: Option<PathBuf>`.
- When `Some(path)` is provided: verifies `path.is_file()`, checks `file_name() == RUNTIME_FILENAME`, calls `verify_runtime(&path)`, and binds.
- When `None`: delegates to `resolve_runtime_path()`.
- Guarantees main process and worker process load the exact same binary without divergence.

### C. `resolve_windows_runtime_path()` (`src-tauri/src/pdf_engine/mod.rs:155-214`)
- Windows-specific path resolver (`#[cfg(target_os = "windows")]`).
- Targets `TAURI_RESOURCE_DIR/lib/pdfium.dll` for packaged builds.
- Inspects `PDFIUM_PATH` and `get_expected_staged_path()` under `IS_DEV_BUILD`.
- Returns `AppError::RuntimeUnavailable` when runtime is not found.

### D. `verify_windows_architecture()` (`src-tauri/src/pdf_engine/mod.rs:461-545`)
- PE COFF header verification using pure Rust stdlib (no external parser crates):
  - DOS header check: `&dos_header[0..2] == b"MZ"`.
  - PE header offset: uint32 at offset `0x3C`.
  - PE signature check: `&pe_sig == b"PE\0\0"`.
  - Machine check: `machine == IMAGE_FILE_MACHINE_AMD64` (`0x8664`).
  - 32-bit flag rejection: `characteristics & IMAGE_FILE_32BIT_MACHINE == 0` (bit `0x0020` must NOT be set).
  - Fails with `AppError::RuntimeArchitectureMismatch` on any non-x86_64 PE file.

### E. `run_worker()` (`src-tauri/src/worker/pdfium.rs:31-43`)
- Entry point for `--run-worker` background process.
- Reads `PDFIUM_RESOLVED_PATH` from environment.
- Calls `get_pdfium_with_resolved_path(resolved_runtime_path)`.
- If unavailable, immediately returns `WorkerFailure(WorkerFailureKind::PdfiumUnavailable, ...)`.

### F. `PdfiumWorkerBroker` (`src-tauri/src/pdf_engine/subprocess_manager.rs:88, 100, 1026-1043`)
- Pre-resolves runtime path at broker creation via `resolve_runtime_path().ok()`.
- Injects `PDFIUM_RESOLVED_PATH` into worker child process environment.
- Ensures worker never performs independent, uncoordinated lookups.

### G. `build.rs` (`src-tauri/build.rs:70-132, 142-150, 320-399, 425-497`)
- Parses `docs/engineering/platform-support.json` at compile time.
- Emits `OUT_DIR/runtime_metadata.rs` with Windows constants (`WINDOWS_EXPECTED_SHA256`, `WINDOWS_EXPECTED_BYTE_LENGTH`, `WINDOWS_RUNTIME_FILENAME`, `WINDOWS_EXPECTED_ARCH`, `IS_DEV_BUILD`).
- Implements `verify_windows_runtime_pre_package()`:
  - Fails if `supplyChainStatus != "COMPLETE"` on release Windows builds.
  - Cleans stale staged DLL and verifies existence at staged path.
  - Validates filename `pdfium.dll`.
  - Verifies byte length and SHA-256.
  - Verifies PE architecture (`IMAGE_FILE_MACHINE_AMD64`, rejecting 32-bit).

---

## 5. Security Threat Evaluation & DLL Search-Order Risk

| Threat / Risk | Current Code Status | Risk Level | Action / Handoff |
|---|---|---|---|
| **PATH / CWD DLL Search-Order Hijacking** | Default Win32 `LoadLibrary` behavior searches CWD, application dir, and system PATH. `resolve_windows_runtime_path()` resolves absolute canonical path, mitigating direct application search. However, underlying `libloading` dynamic binding and indirect dependency resolution remain subject to search order rules. | **HIGH** | **Handoff to D3:** Loader verification must enforce strict absolute path passing, reject relative paths, and ensure no ambient CWD/PATH search order hijacking occurs for `pdfium.dll` or its dependencies. |
| **Unverified Runtime Binary (Windows)** | `WINDOWS_EXPECTED_SHA256` and `WINDOWS_EXPECTED_BYTE_LENGTH` are `None` in metadata (`SHA256 STATUS: MISSING`, `SUPPLY-CHAIN STATUS: INCOMPLETE`). `verify_runtime()` warns in dev mode but release builds fail closed in `build.rs` (`RELEASE_ELIGIBILITY_GATE_FAILED`). | **BLOCKER (Release)** | D2 records contract as fail-closed; release builds require complete hash pin and provenance before promotion. |
| **Tauri Packaging Config Alignment** | `src-tauri/tauri.windows.conf.json` declares `"targets": ["msi", "nsis"]` and `"resources": ["lib/pdfium.dll"]`. Aligned with `resolve_windows_runtime_path()`, `build.rs`, and `platform-support.json` which expect `resourceDir/lib/pdfium.dll`. | **Mitigated** | Resolved in 11I-D4: Full path parity verified offline via `scripts/validate-windows-packaging-contract.mjs`. |
| **Architecture Mismatch (Contamination)** | `verify_windows_architecture()` validates PE header against `IMAGE_FILE_MACHINE_AMD64` (0x8664) and rejects 32-bit before loading. | **Mitigated** | Retain stdlib PE check in `mod.rs` and `build.rs`; fail closed on mismatch. |
| **Symlink / Reparse Point Escape** | `is_symlink_outside_root()` validates symlinks when `TAURI_RESOURCE_DIR` is set. | **Partial** | Retain canonicalization checks; Windows reparse point traversal rejected. |
| **Silent System Fallback** | Removed from production loader; returns typed error. | **Mitigated** | System fallback remains `DISALLOWED`. |

---

## 6. Deterministic Lookup Precedence Contract

The application must resolve the Windows PDFium runtime deterministically according to the execution mode:

### Mode 1: Packaged Release (`!IS_DEV_BUILD`)
```text
1. Packaged Resource:
   Path: TAURI_RESOURCE_DIR\lib\pdfium.dll
   Requirements:
     - File exists and is a regular file.
     - Filename exactly matches "pdfium.dll".
     - Not a symlink/reparse point escaping resourceDir.
     - Byte length matches EXPECTED_BYTE_LENGTH.
     - SHA-256 matches EXPECTED_SHA256.
     - PE header matches IMAGE_FILE_MACHINE_AMD64 (0x8664, 64-bit, non-32-bit).
2. Otherwise: FAIL-CLOSED -> AppError::RuntimeUnavailable / PDFIUM_RUNTIME_UNAVAILABLE.
   NO fallback to PDFIUM_PATH, NO fallback to repo staging, NO fallback to CWD, NO fallback to PATH, NO system search, NO network download.
```

### Mode 2: Local Development (`IS_DEV_BUILD`)
```text
1. Explicit Developer Override (PDFIUM_PATH):
   Requirements:
     - Path must be absolute and non-empty.
     - Canonicalized path must start with an allowlisted development root (CARGO_MANIFEST_DIR or current_exe parent).
     - Filename exactly matches "pdfium.dll".
     - Valid PE x86_64 architecture.
2. Staged Repository Path:
   Path: CARGO_MANIFEST_DIR\..\src-tauri\lib\pdfium.dll
   Requirements:
     - File exists and is a regular file.
     - Valid PE x86_64 architecture.
3. Packaged Resource (if testing packaged layout in dev build):
   Path: TAURI_RESOURCE_DIR\lib\pdfium.dll
4. Otherwise: FAIL-CLOSED -> AppError::RuntimeUnavailable.
   NO CWD probing, NO PATH scanning.
```

---

## 7. System PDFium Fallback Policy

```text
SYSTEM PDFIUM FALLBACK: DISALLOWED
```

### Rationale
- **ABI Instability:** System PDFium binaries (e.g. from third-party tools or developer PATH) may have differing build configurations, ABI revisions, or patch levels incompatible with `pdfium-render 0.8.37`.
- **Supply-Chain Indeterminism:** Relying on host-installed libraries breaks release reproducibility and verification evidence boundaries.
- **Security / DLL Hijacking:** Ambient search paths (`CWD`, `PATH`, system folders) expose the application to unintended DLL substitution or pre-loading attacks.
- **Uniformity:** Both packaged release and local development disallow unpinned system PDFium loading.

---

## 8. Worker Runtime Ownership & Parity

1. **Single Identity:** The main Tauri process and all worker processes (`--run-worker`) must bind to the exact same physical runtime binary.
2. **Path Forwarding:**
   - The main process resolves and verifies the runtime path once.
   - The verified absolute path is injected into child process environments via `PDFIUM_RESOLVED_PATH`.
   - The worker process receives `PDFIUM_RESOLVED_PATH` and invokes `get_pdfium_with_resolved_path(Some(path))`.
3. **No Independent Worker Search:** The worker process must never perform an independent CWD, PATH, or system probe. If `PDFIUM_RESOLVED_PATH` is invalid or verification fails, the worker exits immediately with `WorkerFailureKind::PdfiumUnavailable`.

---

## 9. Path Safety Contract

All resolved runtime paths on Windows must satisfy:
1. **Absolute After Resolution:** Relative paths are rejected before dynamic linking.
2. **Canonicalized & Normalized:** `..` and `.` segments resolved via `std::fs::canonicalize()`.
3. **Allowlist Containment:**
   - Packaged paths must reside under `TAURI_RESOURCE_DIR`.
   - Development paths must reside under `CARGO_MANIFEST_DIR` or `current_exe` parent.
4. **Reparse Point / Symlink Protection:** Reparse points pointing outside the designated root are rejected.
5. **Bare Filename Equality:** `path.file_name() == Some("pdfium.dll")`. No path command interpretation or shell expansion is permitted.

---

## 10. Supply-Chain Identity & Metadata Model (D2 Foundation)

The canonical metadata model in `platform-support.json`:

```json
{
  "id": "pdfium-windows-x86_64-pdfium.dll-UNKNOWN",
  "platform": "windows",
  "architecture": "x86_64",
  "runtimeFileName": "pdfium.dll",
  "filename": "pdfium.dll",
  "versionOrRevision": null,
  "repositoryVersionLabel": null,
  "present": false,
  "verifiedForRelease": false,
  "paths": [],
  "sha256": null,
  "byteLength": null,
  "binaryArchitecture": null,
  "minimumOs": null,
  "licenseReference": "src-tauri/licenses/pdfium.txt",
  "provenance": {
    "class": "unknown",
    "upstreamUrl": null,
    "sourceArtifactName": null,
    "sourceArtifactChecksum": null,
    "sourceCommit": null,
    "builder": null,
    "buildDate": null
  },
  "distributionClass": "bundled_with_artifact",
  "expectedStagedPath": "src-tauri/lib/pdfium.dll",
  "expectedPackagedLocation": "resourceDir/lib/pdfium.dll",
  "supplyChainStatus": "INCOMPLETE",
  "sourceProvenanceComplete": false
}
```

### Current Status
```text
SHA256 STATUS: MISSING
SUPPLY-CHAIN STATUS: INCOMPLETE
PROVENANCE STATUS: INCOMPLETE (class: unknown)
```
No checksum, version, or upstream provenance is invented in D2. All missing external information is classified as `EXTERNAL_RUNTIME_METADATA_REQUIRED`.

---

## 11. PE Architecture Identity Contract

PE architecture validation is performed via Rust stdlib (zero new crate dependencies):
- **DOS Header Signature:** `b"MZ"` (offset `0x00`).
- **PE Header Offset:** Read uint32 at offset `0x3C`.
- **PE Signature:** `b"PE\0\0"`.
- **COFF Header Machine:**
  - Expected: `IMAGE_FILE_MACHINE_AMD64` (`0x8664`).
  - Reject: `IMAGE_FILE_MACHINE_I386` (`0x014c`), `IMAGE_FILE_MACHINE_ARM64` (`0xaa64`), or any other machine type.
- **Characteristics Flag:**
  - Must NOT contain `IMAGE_FILE_32BIT_MACHINE` (`0x0020`).
- **Failure Mode:** Fail-closed with `AppError::RuntimeArchitectureMismatch { expected, found }`.

---

## 12. Authenticode Verification Policy

```text
AUTHENTICODE VERIFICATION: NOT IMPLEMENTED
```
- The repository does not currently implement Windows Authenticode signature checking for third-party runtime binaries.
- SHA-256 cryptographic digest is the canonical integrity identity.
- Authenticode signature validation remains an open engineering gap and is not claimed to be active.

---

## 13. PDFium Revision & ABI Compatibility

```text
PDFIUM ABI COMPATIBILITY: UNVERIFIED
```
- `pdfium-render = "0.8.37"` in `Cargo.lock`.
- No empirical ABI compatibility matrix is proven in repo documentation for Windows runtime revisions.
- Pinning will be hash-based, not version-string-based.

---

## 14. License & Third-Party Notices

```text
RUNTIME LICENSE METADATA GAP
```
- `src-tauri/licenses/pdfium.txt` exists and references PDFium (Apache-2.0 / BSD-3-Clause).
- Root-level `THIRD_PARTY_NOTICES.md` is absent.
- Identified as `RUNTIME_LICENSE_METADATA_GAP`.

---

## 15. Future Runtime Acquisition Contract

If a future pipeline or provisioning stage (11I-F) provisions the Windows DLL:
1. **Expected Source Identity:** Verified upstream source URL and build commit.
2. **Expected Artifact Name:** Specific upstream release archive name.
3. **Platform & Arch:** `windows` and `x86_64` only.
4. **Expected SHA-256:** Strict comparison against canonical pin.
5. **Verify Before Staging:** Hash verified in isolated temp location prior to staging.
6. **Fail-Closed:** Any hash, architecture, or naming mismatch terminates provisioning immediately.
7. **No Fallback:** Never fall back to unverified or ambient downloads.

---

## 16. Future Archive Handling Security Contract

If the Windows runtime is extracted from a zip/archive:
1. **Path Traversal Rejection:** Reject any entry with `..` or leading slashes.
2. **Absolute Path Rejection:** Reject entries with drive prefixes or leading `/` / `\`.
3. **Symlink / Reparse Point Rejection:** Prohibit symbolic links and NTFS reparse points in archive.
4. **Exact File Selection:** Extract only the exact expected `pdfium.dll` binary.
5. **Unexpected File Rejection:** Reject archives containing unexpected executables or duplicate DLLs.
6. **Isolated Temporary Extraction:** Extract to temporary directory with unique prefix.
7. **Post-Extraction Hash Verification:** Compute SHA-256 on extracted bytes before staging.
8. **Atomic Staging:** Move to `src-tauri/lib/pdfium.dll` atomically.

---

## 17. Package Boundary & Resource Wiring Contract (11I-D4)

| Package Property | Contract / Status | Source / Evidence |
|---|---|---|
| **Supported Package Targets** | Strictly `msi` and `nsis`. | `src-tauri/tauri.conf.json:28`, `src-tauri/tauri.windows.conf.json:4` |
| **Artifact Contains Runtime?** | YES (Packaged installer must bundle `pdfium.dll`). Binary currently absent in repo (`RUNTIME_BYTES_UNAVAILABLE_OFFLINE`). | `platform-support.json:237` (`bundled_with_artifact`) |
| **Staged Path in Repository** | `src-tauri/lib/pdfium.dll` | `platform-support.json:238`, `build.rs:98` |
| **Runtime Location Inside Package** | `resourceDir/lib/pdfium.dll`. | `platform-support.json:239`, `build.rs:102`, `tauri.windows.conf.json:9` |
| **Packaging Config Alignment** | `src-tauri/tauri.windows.conf.json` declares `"resources": ["lib/pdfium.dll"]`. 100% path parity with D3 loader. | `src-tauri/tauri.windows.conf.json:9`, `validate-windows-packaging-contract.mjs` |
| **Pre-Package Gate Semantics** | Fail-closed: `build.rs` requires `supplyChainStatus: COMPLETE` (`RELEASE_ELIGIBILITY_GATE_FAILED`) and staged file existence (`MISSING_RUNTIME_ARTIFACT`), correct filename, byte length, SHA-256, and PE architecture. | `build.rs:320-406` |

---

## 18. D4 Implementation & Packaging Wiring (Completed)

The following contract terms were implemented and locked in `11I-D4`:
- **Tauri Windows Config Alignment:** Updated `src-tauri/tauri.windows.conf.json` to lock `"targets": ["msi", "nsis"]` and declare `"resources": ["lib/pdfium.dll"]`, eliminating the previous packaging discrepancy and guaranteeing that Tauri places `pdfium.dll` under `resourceDir/lib/pdfium.dll`.
- **Pre-Package Gate Verification:** Hardened `verify_windows_runtime_pre_package()` in `src-tauri/build.rs` by eliminating the erroneous self-deletion of staged DLLs while preserving strict fail-closed release gates (`RELEASE_ELIGIBILITY_GATE_FAILED`, `MISSING_RUNTIME_ARTIFACT`, `INVALID_RUNTIME_FILENAME`, `INTEGRITY_MISMATCH`, `ARCHITECTURE_MISMATCH`).
- **Path Parity Unit Test:** Added `windows_packaging_path_parity` unit test in `src-tauri/src/pdf_engine/mod.rs` verifying that `WINDOWS_EXPECTED_PACKAGED_LOCATION`, staged path subpath, and D3 loader composition (`resourceDir.join("lib").join(RUNTIME_FILENAME)`) have 100% path parity.
- **Offline Packaging Validator:** Implemented `scripts/validate-windows-packaging-contract.mjs` (dependency-free Node stdlib, zero network) verifying the 9 contract invariants: package targets, resource layout, manifest parity, loader parity, worker propagation, PE architecture isolation, fail-closed pre-package gate, path security, and 11V-E handoff.
- **Worker & Single Ownership Parity:** Main process passes the verified application-owned runtime path via `PDFIUM_RESOLVED_PATH`. Worker process reads this variable and binds to the exact same DLL without duplicate runtime copies.

### 11I-D5 Final Closure & Handoffs (Completed)
- **11I-D5 (Final Closure):** Independent audit of D1–D4 completed; loader, supply-chain, and packaging contracts reconciled; zero network violations; minimal corrections verified.
- **11V-D (Native Runtime Verification Handoff):** Real Windows runner test matrix established (application-owned DLL load, `core-release-smoke` document verification, worker lifecycle, Windows I/O semantics, failure paths). Deferred to dedicated Windows runner.
- **11V-E (Exact Artifact Verification Handoff):** Exact artifact smoke contract established in `docs/engineering/11v-e-handoff.json` and `scripts/artifact-smoke-windows.mjs` for `WINDOWS_MSI` and `WINDOWS_NSIS`. Deferred to Windows CI packaging.
- **11I-F (CI Handoff):** CI inputs defined for runner (`windows-2022`), build command (`npm run tauri build -- --target x86_64-pc-windows-msvc`), and runtime staging (`src-tauri/lib/pdfium.dll`).

---

## 19. Fail-Closed Philosophy

When the Windows runtime DLL is absent, corrupted, architecture-mismatched, or fails integrity verification:
- The application will **NEVER** fall back to another PDF engine (MuPDF, PDF.js, etc.).
- `lopdf` will **NEVER** be substituted as a render engine (strictly maintained as structural/inspection helper).
- The application will **NEVER** attempt an online network download.
- The application will **NEVER** scan CWD, PATH, or arbitrary system directories.
- The application will **NEVER** degrade to silent partial functionality without reporting.
- Any operation requiring PDFium will fail closed with:
  ```text
  AppError::RuntimeUnavailable / PDFIUM_RUNTIME_UNAVAILABLE
  ```
  or corresponding worker protocol error `WorkerFailureKind::PdfiumUnavailable`.

---

## 20. References

- `src-tauri/src/pdf_engine/mod.rs` — Canonical runtime resolution and loading
- `src-tauri/src/pdf_engine/runtime_metadata.rs` — Generated platform constants
- `src-tauri/src/worker/pdfium.rs` — Worker runtime initialization
- `src-tauri/src/pdf_engine/subprocess_manager.rs` — Worker process launcher
- `src-tauri/build.rs` — Build-time code generation and release gates
- `src-tauri/tauri.windows.conf.json` — Windows packaging and resource configuration
- `docs/engineering/platform-support.json` — Single machine-readable platform manifest
- `scripts/validate-pdfium-runtime-manifest.mjs` — Offline manifest validator
- `scripts/artifact-smoke-windows.mjs` — Windows artifact smoke harness
- `AGENTS.md` — Project rules, PDFium canonical engine authority, and `STRICT_OFFLINE` policy
