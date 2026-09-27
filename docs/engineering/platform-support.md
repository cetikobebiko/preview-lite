# Platform Support

This document describes the Prompt 11.0 support baseline captured on 20 August
2026, with 11I-C2 Linux and 11I-D2 Windows supply-chain contract amendments (2026-09-03). It is a human-readable view of
[`platform-support.json`](platform-support.json); the JSON file is canonical (`schemaVersion: "1.0.0"`).

**Canonical runtime manifest:** `docs/engineering/platform-support.json` — single owner for id/platform/architecture/runtimeFileName/versionOrRevision/sha256/provenance/distributionClass (§2). Validated offline by `scripts/validate-pdfium-runtime-manifest.mjs` (§15, dependency-free).

## Current status (C2 / D2)

| Platform | Status | Runtime in repository | SHA-256 | Provenance | Supply-chain | Publication |
|---|---|---|---|---|---|---|
| macOS arm64 (`aarch64-apple-darwin`) | `not_evaluated` | arm64 `libpdfium.dylib` present (2 copies, byte-identical, 7,151,568 B) | `c2f68f87106da4f0cb6d6a90a7aad9644b3c1cb92a3463dbdc8f39449aec3869` known | `unknown` (upstreamUrl/builder/date missing) | PARTIAL (hash known, provenance INCOMPLETE) | Not allowed |
| Linux x64 (`x86_64-unknown-linux-gnu`) | `not_evaluated` | `libpdfium.so` absent — `src-tauri/lib/libpdfium.so` expected staging | `SHA256: UNKNOWN` | `unknown` — `EXTERNAL_RUNTIME_METADATA_REQUIRED` | **INCOMPLETE — RUNTIME_METADATA_INCOMPLETE** | Not allowed |
| Windows x64 (`x86_64-pc-windows-msvc`) | `not_evaluated` | `pdfium.dll` absent — `src-tauri/lib/pdfium.dll` expected staging | `SHA256: UNKNOWN` | `unknown` — `EXTERNAL_RUNTIME_METADATA_REQUIRED` | **INCOMPLETE — RUNTIME_METADATA_INCOMPLETE** | Not allowed |

- Linux runtime record ID: `pdfium-linux-x86_64-libpdfium.so-UNKNOWN` — platform `linux`, arch `x86_64` (canonical), file `libpdfium.so` (no suffix), staged `src-tauri/lib/libpdfium.so` → packaged `resourceDir/lib/libpdfium.so`.
- Windows runtime record ID: `pdfium-windows-x86_64-pdfium.dll-UNKNOWN` — platform `windows`, arch `x86_64` (canonical), file `pdfium.dll` (no suffix), staged `src-tauri/lib/pdfium.dll` → packaged `resourceDir/lib/pdfium.dll`. Hash required before use/package fail-closed, no fallback to unverified or ambient search. Detailed in [`windows-runtime-contract.md`](windows-runtime-contract.md).

The baseline HEAD commit is
`7b82b0319a4e12c58556567f2890a2fa310961e9`. Because the working tree was
dirty, this is only the commit beneath the captured snapshot. Prompt 11's
implementation freeze is **NOT ESTABLISHED** and has no commit SHA.

## Status meanings

- `verified`: verified runtime, native integration and worker tests, packaged
  artifact, exact-artifact smoke, and required signing verification all exist
  for the same freeze commit.
- `experimental`: runtime and package smoke work, but at least one stable
  release criterion is missing.
- `build_only`: compile/build evidence exists, but runtime/package evidence
  does not.
- `unsupported`: the required runtime or architecture support is unavailable.
- `not_evaluated`: the platform has not yet been evaluated against the current
  Prompt 11 freeze and evidence policy.

Compile success does not imply support. A missing runtime, skipped runtime
suite, development binary, or successful workflow from another commit cannot
produce `verified` status.

## Evidence interpretation

The current worktree contained 153 pre-existing change entries before the
three Prompt 11.0 documentation files were created. No platform build, native
test, exact package smoke, or signing verification was run on that snapshot.
Therefore all platforms remain `not_evaluated`.

The Prompt 11 task context reports earlier macOS arm64 native success and
earlier Linux x64 source/build portability success. Those records are retained
as `historicalEvidence` in the JSON document. They are explicitly:

- not current-baseline evidence;
- not bound to a Prompt 11 implementation-freeze commit;
- not exact-artifact evidence;
- not sufficient to change a platform status or authorize publication.

The reported local ARM-to-Linux-amd64 emulation `rustc SIGKILL` is recorded as
a runner-environment failure, not a compiler or test-assertion failure and not
canonical Linux evidence.

## Runtime facts (C2)

The only runtime currently committed is a macOS arm64
`libpdfium.dylib`. Its two repository copies are byte-identical, each
7,151,568 bytes with SHA-256
`c2f68f87106da4f0cb6d6a90a7aad9644b3c1cb92a3463dbdc8f39449aec3869`.
The repository version label is `152.0.7961.0`, but complete upstream artifact
and build provenance is not yet recorded → `sourceProvenanceComplete:false`, provenance class `unknown`.

**11I-C2 & 11I-D2 supply-chain contract:**

- Canonical manifest `platform-support.json: schemaVersion "1.0.0"` + validator `scripts/validate-pdfium-runtime-manifest.mjs` (offline, §15 invariants: unique ID, unique platform+arch, known vocab, lowercase SHA-256, safe filename, no traversal/absolute/URL executor, unknown provenance explicit).
- Every supported Linux runtime has explicit record (§3: `id/platform/architecture/runtimeFileName/versionOrRevision/sha256/provenance/distributionClass` plus `byteLength/licenseReference/expectedStagedPath/expectedPackagedLocation`). Linux record `pdfium-linux-x86_64-libpdfium.so-UNKNOWN` has `sha256:null` → `SHA256: UNKNOWN` + `SUPPLY_CHAIN STATUS: INCOMPLETE` (§6) — no invented hash.
- Every supported Windows runtime has explicit record (11I-D2 §3: `id/platform/architecture/runtimeFileName/versionOrRevision/sha256/provenance/distributionClass` plus `byteLength/licenseReference/expectedStagedPath/expectedPackagedLocation`). Windows record `pdfium-windows-x86_64-pdfium.dll-UNKNOWN` has `sha256:null` → `SHA256: UNKNOWN` + `SUPPLY_CHAIN STATUS: INCOMPLETE` (11I-D2 §6, §7) — no invented hash.
- Provenance bounded classes `project_bundled_verified | upstream_prebuilt_verified | locally_built_verified | development_external | unknown` (§7) — currently all `unknown`; answers `Where are these bytes intended to come from?` as unknown explicit.
- Distribution `bundled_with_artifact` (§8): repo staging `src-tauri/lib/pdfium.dll` → packaged `resourceDir/lib/pdfium.dll` (Tauri `resources`, worker propagation via `subprocess_manager.rs`).
- PE Architecture Identity (11I-D2 §9): Standard library MZ/PE COFF header inspection (`IMAGE_FILE_MACHINE_AMD64` 0x8664, 32-bit flag rejection) fail-closed with `AppError::RuntimeArchitectureMismatch`.
- DLL Search-Order Threat (11I-D2 §14): Win32 default DLL search order (CWD, PATH) mitigated via absolute canonical path resolution; loader strictness handed off to D3.
- Acquisition contract future (§9, 11I-D2 §15): expected source/platform/arch/filename/SHA-256, verify before use/package, fail closed, no fallback to unverified — no download script in C2/D2, no network.
- Archive extraction contract future (§10, 11I-D2 §16): traversal/absolute/symlink/extra file reject, exact selection, temp+atomic+hash.
- Architecture mismatch fail-closed, alias normalization bounded `x86_64↔amd64`, `aarch64↔arm64`, canonical `x86_64`/`arm64`.
- Compatibility `pdfium-render 0.8.37` vs runtime → `COMPATIBILITY ASSUMPTION: UNVERIFIED` (no table invented).
- License `src-tauri/licenses/pdfium.txt` exists, but `THIRD_PARTY_NOTICES.md` absent → `RUNTIME_LICENSE_METADATA_GAP`.
- Authenticode signature validation policy absent → `AUTHENTICODE VERIFICATION: NOT IMPLEMENTED`.

Linux and Windows runtimes must not be guessed or downloaded without explicit
approval, an exact source/version, checksums, architecture evidence, license,
and provenance. Network attempts strictly forbidden in C2 and D2 (§0).

## Validation (C2 offline)

```bash
node scripts/validate-pdfium-runtime-manifest.mjs
python3 -c "import json,pathlib; json.load(open('docs/engineering/platform-support.json'))"
# optional offline Rust smoke (no native pdfium required):
# CARGO_NET_OFFLINE=true cargo test --locked --offline --test document_io_fs
```

Validator enforces §15: schema valid, unique ID, unique platform+arch, known vocab, lowercase SHA-256 or explicit UNKNOWN+INCOMPLETE, safe filename, no absolute/traversal/URL executor, supported target has exactly one record, unknown provenance explicit, malformed reject; §16: manifest data only (no shell/download/exec/postInstall).

## Promotion gates

Status promotion must be evidence-driven and freeze-bound:

1. Complete 11I-A through 11I-G and establish an implementation-freeze commit.
2. Run 11V-A on a canonical clean checkout.
3. Run the relevant native verification on the actual target runner:
   11V-B for macOS arm64, 11V-C for Linux x64, or 11V-D for Windows x64.
4. Run 11V-E against the exact packaged artifact.
5. Complete the platform's required signing/notarization verification.
6. Update both platform-support documents with reports tied to that same
   commit before publication.

Stable assets may be published only for platforms whose final status is
`verified`.

## C3 handoff (C2)

```text
canonical runtime manifest path: docs/engineering/platform-support.json
runtime record ID(s): pdfium-linux-x86_64-libpdfium.so-UNKNOWN (linux), pdfium-macos-..., pdfium-windows-...
supported Linux target(s): x86_64-unknown-linux-gnu (only)
expected runtime filename: libpdfium.so
expected packaged location contract: resourceDir/lib/libpdfium.so (absolute after resource_dir, canonicalize, parent==resourceDir/lib, symlink/traversal rejected; staging src-tauri/lib/libpdfium.so)
verification function required?: YES — C3 must wire hash+byteLength+arch gate before dlopen, fail-closed
hash status: PARTIAL — 1 known (macOS) / 2 unknown (linux, windows)
provenance status: INCOMPLETE — all unknown, EXTERNAL_RUNTIME_METADATA_REQUIRED (upstreamUrl/sourceCommit/builder/date)
unresolved blockers: libpdfium.so absent, sha unknown, provenance null, THIRD_PARTY_NOTICES.md missing, loader still CWD/system fallback (C3), hash/arch gates absent (C3), ldd/glibc unknown (C4), compatibility UNVERIFIED
```
