# Linux Runtime Contract — 11I-C1, 11I-C2, 11I-C3, 11I-C4 & 11I-C5

**Scope:** Production PDFium runtime foundation for Linux — deterministic, provenance-aware, architecture-bound, fail-closed resolution, and packaging runtime wiring.  
**Gate:** `11I-C1 COMPLETE` → `11I-C2 COMPLETE` → `11I-C3 COMPLETE` → `11I-C4 COMPLETE` → `11I-C5 IMPLEMENTATION: COMPLETE` (final closure audit, parity verification, 11V-C / 11I-F handoff). No binary download, strict offline.  
**Strict offline:** No network resolution attempted. Missing runtime binary reported as `RUNTIME_BYTES_UNAVAILABLE_OFFLINE`. Network attempts strictly forbidden (§0: curl, wget, GitHub API, Cargo online resolution, apt, npm all denied).  
**Canonical sources:**
- `docs/engineering/platform-support.json` (machine-readable, `schemaVersion: "1.0.0"`, `canonicalRuntimeManifest` = self)
- `scripts/validate-pdfium-runtime-manifest.mjs` (manifest validator, node stdlib only)
- `scripts/validate-linux-packaging-contract.mjs` (packaging & resource layout contract validator)
- `src-tauri/src/pdf_engine/mod.rs` (canonical loader & platform resolvers)
- `src-tauri/src/pdf_engine/runtime_metadata.rs` (build-time generated constants)
- `src-tauri/src/worker/pdfium.rs` (isolated worker process loader)
- `src-tauri/src/pdf_engine/subprocess_manager.rs` (worker launcher & runtime path propagation)
- `src-tauri/build.rs` (build-time manifest parser, code generator, and pre-package verification gate)
- `src-tauri/tauri.linux.conf.json` & `src-tauri/tauri.conf.json` (packaging resource configs)

**Status:** `IMPLEMENTATION COMPLETE & HANDOFF READY` (11I-C1, 11I-C2, 11I-C3, 11I-C4 & 11I-C5 COMPLETE) — Linux target specified (`deb` only), canonical runtime identity, deterministic packaged layout (`resourceDir/lib/libpdfium.so`), fail-closed pre-package gates, offline validators, and 11V-C / 11I-F handoffs established.

---

## 1. Supported Linux Targets (Source-Backed)

| Target Triple | OS | Arch | Status | Evidence |
|---|---|---|---|---|
| `x86_64-unknown-linux-gnu` | `linux` | `x86_64` | `not_evaluated` (contract defined, runtime absent in repo, metadata incomplete) | `platform-support.json:131`, `build.rs:25`, `.github/workflows/release.yml:14` (`ubuntu-22.04`), `pdf_engine/mod.rs:410` (`EM_X86_64`) |

### Strict Target Boundary
- **Single Supported Architecture:** Only `x86_64-unknown-linux-gnu` is targeted.
- **Excluded / Not Supported:**
  - `aarch64-unknown-linux-gnu` — Not present in any repository source, configuration, or CI workflow.
  - `x86_64-unknown-linux-musl` — Not supported; glibc dynamic runtime expected.
  - `i686-unknown-linux-gnu` (32-bit) — Explicitly rejected by `verify_linux_architecture()` (`ei_class != 2` check).
- **Target Invariant:** No additional Linux architecture may be introduced without an explicit architectural decision, upstream artifact provenance, SHA-256 pin, `platform-support.json` entry, and native runner evidence in `11V-C`.

---

## 2. Expected Runtime Filename and Placement

| Item | Value | Source / Evidence |
|---|---|---|
| **Filename** | `libpdfium.so` | `platform-support.json:141`, `runtime_metadata.rs:14` (`LINUX_RUNTIME_FILENAME`), `pdf_engine/mod.rs:296` |
| **Suffix Policy** | Bare name only (no version suffix like `libpdfium.so.152`) | `pdfium-render` crate binds to un-suffixed platform name; loader strictly checks `file_name() == RUNTIME_FILENAME`. |
| **Repository Staging** | `src-tauri/lib/libpdfium.so` | `platform-support.json:163` (`expectedStagedPath`), `build.rs:57`, `runtime_metadata.rs:18` (`LINUX_EXPECTED_STAGED_PATH`). |
| **Packaged Location** | `resourceDir/lib/libpdfium.so` | `platform-support.json:164` (`expectedPackagedLocation`), `build.rs:61`, `runtime_metadata.rs:20` (`LINUX_EXPECTED_PACKAGED_LOCATION`). |
| **Resolved Form** | Absolute, canonicalized, normalized, validated under allowlist root | Required by path safety contract; `is_path_under_allowlist` & `is_valid_runtime_file`. |

---

## 3. Runtime Ownership

| Context | Owner | Authoritative Path | Ownership Policy |
|---|---|---|---|
| **Packaged Release** | Application Bundle (Tauri Resource Directory) | `resourceDir/lib/libpdfium.so` | Release artifact must carry its own verified runtime bytes. Must NOT depend on host global libraries, `LD_LIBRARY_PATH`, or developer machine installs. |
| **Development (Local)** | Repository Staging OR Allowlisted Developer Override | `src-tauri/lib/libpdfium.so` OR `PDFIUM_PATH` | Allowed only in development builds (`IS_DEV_BUILD`). Must be absolute and under allowlisted development roots. |
| **System** | **Not an owner** | — | `bind_to_system_library()` is disallowed. System PDFium cannot be verified for release determinism. |

---

## 4. Current PDFium Loading Inventory (Source-Extracted)

The codebase contains the following PDFium loading and binding components:

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

### C. `resolve_linux_runtime_path()` (`src-tauri/src/pdf_engine/mod.rs:94-148`)
- Linux-specific path resolver (`#[cfg(target_os = "linux")]`).
- Targets `TAURI_RESOURCE_DIR/lib/libpdfium.so` for packaged builds.
- Inspects `PDFIUM_PATH` and `get_expected_staged_path()` under `IS_DEV_BUILD`.
- Returns `AppError::RuntimeUnavailable` when runtime is not found.

### D. `verify_linux_architecture()` (`src-tauri/src/pdf_engine/mod.rs:366-426`)
- ELF header verification for Linux binaries:
  - Magic byte check: `\x7fELF`.
  - Class check: `ei_class == 2` (64-bit ELFCLASS64).
  - Endianness handling: parses `ei_data`.
  - Machine check: `e_machine == 0x003e` (`EM_X86_64`).
  - Fails with `AppError::RuntimeArchitectureMismatch` on any non-x86_64 ELF file.

### E. `run_worker()` (`src-tauri/src/worker/pdfium.rs:31-43`)
- Entry point for `--run-worker` background process.
- Reads `PDFIUM_RESOLVED_PATH` from environment.
- Calls `get_pdfium_with_resolved_path(resolved_runtime_path)`.
- If unavailable, immediately returns `WorkerFailure(WorkerFailureKind::PdfiumUnavailable, ...)`.

### F. `PdfiumWorkerBroker` (`src-tauri/src/pdf_engine/subprocess_manager.rs:88, 100, 1026-1043`)
- Pre-resolves runtime path at broker creation via `resolve_runtime_path().ok()`.
- Injects `PDFIUM_RESOLVED_PATH` into worker child process environment.
- Ensures worker never performs independent, uncoordinated lookups.

### G. `build.rs` (`src-tauri/build.rs:1-307, 309-486`)
- Parses `docs/engineering/platform-support.json` at compile time.
- Emits `OUT_DIR/runtime_metadata.rs` with Linux constants (`LINUX_EXPECTED_SHA256`, `LINUX_EXPECTED_BYTE_LENGTH`, `LINUX_RUNTIME_FILENAME`, `LINUX_EXPECTED_ARCH`, `IS_DEV_BUILD`).
- Implements Windows pre-package verification (`verify_windows_runtime_pre_package`); Linux equivalent identified as needed for C3/build.rs.

### H. `fuzz_targets/pdfium_lifecycle.rs:13-14` (Development Harness)
- Standalone fuzz target using legacy `bind_to_system_library().or_else(...)`.
- Isolated from production runtime; not part of application binary.

---

## 5. Security Threat Evaluation & Identified Loader Defects

| Threat / Defect | Current Code Status | Risk Level | Action for 11I-C3 |
|---|---|---|---|
| **Unvalidated `PDFIUM_PATH` branch** | In `resolve_linux_runtime_path()` (lines 101-106), `PDFIUM_PATH` is checked outside `IS_DEV_BUILD` without canonicalization or allowlist check if `TAURI_RESOURCE_DIR` is unset. | **BLOCKER** | Remove lines 101-106. Ensure `PDFIUM_PATH` is strictly gated under `if IS_DEV_BUILD` with `is_path_under_allowlist` and `canonicalize()`. |
| **Unverified Runtime Binary (Linux)** | `LINUX_EXPECTED_SHA256` and `LINUX_EXPECTED_BYTE_LENGTH` are `None` in metadata. `verify_runtime()` skips hash check when `None`. | **BLOCKER** | C2 pins metadata; C3 enforces non-optional verification for release builds. |
| **Missing Linux Pre-Package Gate** | `build.rs` has `verify_windows_runtime_pre_package()` for Windows release builds, but lacks an equivalent `verify_linux_runtime_pre_package()`. | **BLOCKER** | Add `verify_linux_runtime_pre_package()` to `build.rs` in C3. |
| **Tauri Resource Configuration Mismatch** | `tauri.linux.conf.json:4` declares `"resources": ["libpdfium.so"]` (places at `resourceDir/libpdfium.so`), while `resolve_linux_runtime_path()` and `build.rs` expect `resourceDir/lib/libpdfium.so`. | **BLOCKER** | Align `tauri.linux.conf.json` resource declaration with `lib/libpdfium.so` staging structure. |
| **Arbitrary CWD Probing** (`./`, `./lib`) | Production `get_pdfium()` has eliminated CWD probes. | **Mitigated** | Maintain strict rejection of relative/CWD paths. |
| **Uncontrolled `LD_LIBRARY_PATH`** | `bind_to_system_library()` eliminated from production loader. Absolute path passed to `Library::new()`. | **Mitigated** | Retain absolute path loading; prohibit system fallback. |
| **Architecture Mismatch** | `verify_linux_architecture()` validates ELF header against `EM_X86_64` (0x3e) before loading. | **Mitigated** | Retain ELF check; verify endianness and 64-bit class. |
| **Symlink Escape** | `is_symlink_outside_root()` validates symlinks when `TAURI_RESOURCE_DIR` is set. | **Partial** | Ensure canonicalization checks apply uniformly across all paths. |
| **Silent System Fallback** | Removed from production loader; returns typed error. | **Mitigated** | System fallback remains `DISALLOWED`. |

---

## 6. Deterministic Lookup Precedence Contract

The application must resolve the Linux PDFium runtime deterministically according to the execution mode:

### Mode 1: Packaged Release (`!IS_DEV_BUILD`)
```text
1. Packaged Resource:
   Path: TAURI_RESOURCE_DIR/lib/libpdfium.so
   Requirements:
     - File exists and is a regular file.
     - Filename exactly matches "libpdfium.so".
     - Not a symlink escaping resourceDir.
     - Byte length matches EXPECTED_BYTE_LENGTH.
     - SHA-256 matches EXPECTED_SHA256.
     - ELF header matches EM_X86_64 (64-bit, little-endian).
2. Otherwise: FAIL-CLOSED -> AppError::RuntimeUnavailable / PDFIUM_RUNTIME_UNAVAILABLE.
   NO fallback to PDFIUM_PATH, NO fallback to repo staging, NO fallback to system, NO network download.
```

### Mode 2: Local Development (`IS_DEV_BUILD`)
```text
1. Explicit Developer Override (PDFIUM_PATH):
   Requirements:
     - Path must be absolute and non-empty.
     - Canonicalized path must start with an allowlisted development root (CARGO_MANIFEST_DIR or current_exe parent).
     - Filename exactly matches "libpdfium.so".
     - Valid ELF x86_64 architecture.
2. Staged Repository Path:
   Path: CARGO_MANIFEST_DIR/../src-tauri/lib/libpdfium.so
   Requirements:
     - File exists and is a regular file.
     - Valid ELF x86_64 architecture.
3. Packaged Resource (if testing packaged layout in dev build):
   Path: TAURI_RESOURCE_DIR/lib/libpdfium.so
4. Otherwise: FAIL-CLOSED -> AppError::RuntimeUnavailable.
```

---

## 7. System PDFium Fallback Policy

```text
SYSTEM PDFIUM FALLBACK: DISALLOWED
```

### Rationale
- **ABI Instability:** System PDFium libraries (e.g. from distribution packages) may be compiled with differing build flags, ABI revisions, or patch levels incompatible with `pdfium-render 0.8.37`.
- **Supply-Chain Indeterminism:** Relying on host-installed libraries breaks release reproducibility and verification evidence boundaries.
- **Security:** Global search paths (`LD_LIBRARY_PATH`, `/usr/lib`, `/lib`) expose the application to unintended library substitution or injection.
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

All resolved runtime paths must satisfy:
1. **Absolute After Resolution:** Relative paths are rejected before dynamic linking.
2. **Canonicalized & Normalized:** `..` and `.` segments resolved via `std::fs::canonicalize()`.
3. **Allowlist Containment:**
   - Packaged paths must reside under `TAURI_RESOURCE_DIR`.
   - Development paths must reside under `CARGO_MANIFEST_DIR` or `current_exe` parent.
4. **Symlink Protection:** Symlinks pointing outside the designated root are rejected.
5. **Bare Filename Equality:** `path.file_name() == Some("libpdfium.so")`. No command interpretation or shell expansion is permitted on path variables.

---

## 10. Supply-Chain Identity & Metadata Model (C2 Foundation)

The conceptual metadata model for `platform-support.json` runtime records:

```text
PdfiumRuntimeRecord {
    id: "pdfium-linux-x86_64-libpdfium.so-UNKNOWN"
    platform: "linux"
    architecture: "x86_64"
    runtimeFileName: "libpdfium.so"
    versionOrRevision: null -> EXTERNAL_RUNTIME_METADATA_REQUIRED
    sha256: null            -> EXTERNAL_RUNTIME_METADATA_REQUIRED
    byteLength: null        -> EXTERNAL_RUNTIME_METADATA_REQUIRED
    provenance: {
        class: "unknown",
        upstreamUrl: null,
        sourceArtifactName: null,
        sourceArtifactChecksum: null,
        sourceCommit: null,
        builder: null,
        buildDate: null
    }
    distributionClass: "bundled_with_artifact"
    expectedStagedPath: "src-tauri/lib/libpdfium.so"
    expectedPackagedLocation: "resourceDir/lib/libpdfium.so"
    supplyChainStatus: "INCOMPLETE"
}
```

### Current Completeness
```text
PDFIUM_RUNTIME_IDENTITY: INCOMPLETE
```
No checksum, version, or upstream provenance is invented in C1. All missing external information is classified as `EXTERNAL_RUNTIME_METADATA_REQUIRED` and handed off to 11I-C2.

---

## 11. Package Boundary (11I-C4 COMPLETE)

| Package Property | Contract / Status | Source / Evidence |
|---|---|---|
| **Supported Package Targets** | Strictly `deb` only. AppImage and RPM are not configured (`SUPPORTED PACKAGE TARGETS = deb`). | `src-tauri/tauri.conf.json:28`, `src-tauri/tauri.linux.conf.json:5` |
| **Artifact Contains Runtime?** | YES (Packaged `.deb` must bundle `libpdfium.so`). Binary absent in local repo (`RUNTIME_BYTES_UNAVAILABLE_OFFLINE`). | `platform-support.json:162` (`bundled_with_artifact`) |
| **Staged Path in Repository** | `src-tauri/lib/libpdfium.so` | `platform-support.json:163`, `build.rs:57` |
| **Runtime Location Inside Package** | `resourceDir/lib/libpdfium.so` (`tauri.linux.conf.json` declares `"resources": ["lib/libpdfium.so"]`). | `tauri.linux.conf.json:8`, `platform-support.json:164` |
| **Loader Expected Relative Path** | Resolved via Tauri `resource_dir().join("lib").join("libpdfium.so")` (`TAURI_RESOURCE_DIR`). | `pdf_engine/mod.rs:97-107` |
| **Loader ↔ Package Path Parity** | `PACKAGE WRITES HERE` (`resourceDir/lib/libpdfium.so`) == `LOADER READS HERE` (`TAURI_RESOURCE_DIR/lib/libpdfium.so`). **PASS**. | `scripts/validate-linux-packaging-contract.mjs` |
| **Worker Resolution Parity** | Main process resolves path once and injects `PDFIUM_RESOLVED_PATH`. Worker binds directly without CWD/system probe. **PASS**. | `subprocess_manager.rs:1033-1036`, `worker/pdfium.rs:35-42` |
| **File Permissions** | `0755` (`-rwxr-xr-x`, standard ELF dynamic shared object). No post-install `chmod` script permitted. | Linux filesystem standard DSO policy |
| **Architecture Isolation** | Strictly `x86_64` (ELF class 64, `EM_X86_64` 0x003e). Multi-arch mixing strictly forbidden. | `pdf_engine/mod.rs:410`, `build.rs:608-622` |
| **Pre-Package Gate Semantics** | Fail-closed: `build.rs` requires `supplyChainStatus: COMPLETE` (`RELEASE_ELIGIBILITY_GATE_FAILED`) and staged file existence (`MISSING_RUNTIME_ARTIFACT`). | `build.rs:506-576` |
| **Dynamic Library Dependencies** | **UNKNOWN** — Requires `ldd` and `readelf -d` audit once binary is provisioned in CI/11V-C. Glibc minimum version floor handoff to 11V-C. | Deferred to 11V-C / 11V-E |

---

## 12. Fail-Closed Philosophy

When the Linux runtime binary is absent, corrupted, architecture-mismatched, or fails integrity verification:
- The application will **NEVER** fall back to another PDF engine (MuPDF, PDF.js, etc.).
- `lopdf` will **NEVER** be substituted as a render engine (strictly maintained as structural/inspection helper).
- The application will **NEVER** attempt an online network download.
- The application will **NEVER** scan arbitrary system paths.
- The application will **NEVER** degrade to silent partial functionality without reporting.
- Any operation requiring PDFium will fail closed with:
  ```text
  AppError::RuntimeUnavailable / PDFIUM_RUNTIME_UNAVAILABLE
  ```
  or corresponding worker protocol error `WorkerFailureKind::PdfiumUnavailable`.

---

## 13. Verification Boundary & Handoffs

- **11I-C1:** COMPLETE — Linux runtime contract, loader inventory, security audit, and platform boundary established.
- **11I-C2:** COMPLETE — Single canonical runtime identity owner (`docs/engineering/platform-support.json`), provenance model, SHA-256 contract (fail-closed, explicit UNKNOWN/INCOMPLETE), offline validator (`scripts/validate-pdfium-runtime-manifest.mjs`), and C3 handoff established.
- **11I-C3:** COMPLETE — Production loader/runtime resolution wiring (`resolve_linux_runtime_path()`), worker path propagation (`PDFIUM_RESOLVED_PATH`), pre-package verification gate in `build.rs`, and mandatory hash verification on release.
- **11I-C4:** COMPLETE — Linux packaging wiring, resource layout (`resourceDir/lib/libpdfium.so`), single package target (`deb`), manifest/package/loader parity, offline validator (`scripts/validate-linux-packaging-contract.mjs`), and strict offline boundary maintained.
- **11I-C5:** COMPLETE — Linux runtime final closure audit, loader/manifest/package parity verification, error taxonomy closure, security closure, and 11V-C / 11I-F handoff established.
- **11V-C:** Native Linux platform verification on `x86_64-unknown-linux-gnu` runner (owns real Linux PDFium test execution and dynamic library audit).
- **11V-E:** Exact packaged artifact smoke tests (`.deb` install, launch, worker IPC, release-smoke verification).
- **Next Implementation Stage:** 11I-D1 (Windows Runtime Implementation).

---

## 14. References

- `src-tauri/src/pdf_engine/mod.rs` — Canonical runtime resolution and loading
- `src-tauri/src/pdf_engine/runtime_metadata.rs` — Generated platform constants
- `src-tauri/src/worker/pdfium.rs` — Worker runtime initialization
- `src-tauri/src/pdf_engine/subprocess_manager.rs` — Worker process launcher
- `src-tauri/build.rs` — Build-time code generation and release gates
- `src-tauri/tauri.linux.conf.json` — Linux packaging and resource configuration
- `docs/engineering/platform-support.json` — Single machine-readable platform manifest
- `scripts/validate-pdfium-runtime-manifest.mjs` — Offline manifest validator
- `scripts/validate-linux-packaging-contract.mjs` — Packaging & resource layout contract validator
- `AGENTS.md` — Project rules, PDFium canonical engine authority, and `STRICT_OFFLINE` policy
