# CI Architecture — 11I-F

Source-of-truth for the GitHub Actions CI taxonomy, job DAG, execution-class
mapping, and evidence contract. The machine-readable execution class registry is
`docs/engineering/test-execution-registry.json`; this document records how those
layers map onto CI jobs.

## 1. Execution classes (canonical, from 11I-B)

```
FAST_LOCAL        — no PDFium runtime, no native OS API, no artifact, no network
NATIVE_PLATFORM   — real PDFium runtime / real worker / native OS behavior
EXACT_ARTIFACT    — packaged release artifact (.dmg/.app/.deb/.msi/.nsis)
```

CI does not invent new, conflicting test classifications.

## 2. Job taxonomy

| Job class | Execution class | Runner | Status |
|---|---|---|---|
| `FAST_LOCAL` | FAST_LOCAL | `ubuntu-22.04` (single, platform-independent) | implemented (F2) |
| `MACOS_NATIVE` | NATIVE_PLATFORM | `macos-14` (arm64) | runtime committed; needs rewrite |
| `LINUX_NATIVE` | NATIVE_PLATFORM | `ubuntu-22.04` (x64) | BLOCKED — runtime absent |
| `WINDOWS_NATIVE` | NATIVE_PLATFORM | `windows-latest` (x64) | BLOCKED — runtime absent |
| `MACOS_ARTIFACT_BUILD` | EXACT_ARTIFACT | `macos-14` | F3+ |
| `LINUX_ARTIFACT_BUILD` | EXACT_ARTIFACT | `ubuntu-22.04` | F3+ |
| `WINDOWS_ARTIFACT_BUILD` | EXACT_ARTIFACT | `windows-latest` | F3+ |
| `MACOS_ARTIFACT_SMOKE` | EXACT_ARTIFACT | `macos-14` | F3+ |
| `LINUX_ARTIFACT_SMOKE` | EXACT_ARTIFACT | `ubuntu-22.04` | F3+ |
| `WINDOWS_ARTIFACT_SMOKE` | EXACT_ARTIFACT | `windows-latest` | F3+ |
| `EXTERNAL_VALIDATION` | optional diagnostic | — | not required (disabled) |
| `DEPENDENCY_AUDIT` | supply-chain check (no test class) | `ubuntu-22.04` | implemented: `dependency-audit.yml`, `cargo audit` on PRs, `main` and weekly |
| `RELEASE_GATE` | signing/publish | — | 11I-G/11R scope |

## 3. CI DAG

```
FAST_LOCAL
   │
   ├───────────────┐
   ↓               ↓
MACOS_NATIVE   LINUX_NATIVE / WINDOWS_NATIVE   (parallel)
   │               │
   ↓               ↓
MACOS_ARTIFACT_BUILD  LINUX/WINDOWS_ARTIFACT_BUILD
        ...               ...
   ↓               ↓
MACOS_ARTIFACT_SMOKE  LINUX/WINDOWS_ARTIFACT_SMOKE
        ...               ...
        (EXTERNAL_VALIDATION — optional, never release-blocking)
   ↓
RELEASE_GATE
```

`FAST_LOCAL` is an independent preflight with no PDFium, no artifact, and no
platform runtime provisioning dependency (§6).

## 4. F2 deliverables

- `scripts/compute-artifact-manifest.mjs` — producer SHA-256 identity manifest.
- `scripts/verify-artifact-identity.mjs` — consumer byte-identity re-hash (fail-closed on mismatch/missing/extra).
- `scripts/collect-test-evidence.mjs` — runs FAST_LOCAL registry targets, captures
  per-target observed test count + exit code + vitest aggregate counts, and emits
  evidence JSON consumed by `verify-test-evidence.mjs`.
- `.github/workflows/fast-local.yml` — the FAST_LOCAL job class (least-privilege,
  bounded timeout, concurrency, validator + evidence wiring).

## 5. Artifact identity contract

Producer builds once; consumer re-hashes and compares unconditionally. SHA-256
match is required; filename alone is never trusted (11i-f-handoff §2/§9).

```
producer: build → compute-artifact-manifest.mjs (sha256 + byteLength) → upload artifact + manifest
consumer: download → verify-artifact-identity.mjs (re-hash == producer hash) → smoke exact bytes
```

## 6. Permissions (default deny / least privilege)

- `FAST_LOCAL` and all test jobs: `contents: read` only.
- Artifact build/smoke: `contents: read` (+ `actions: read` where cross-job download is used).
- `RELEASE_GATE`: `contents: write` explicit, signing secrets, trusted tag push only.
- No top-level `write-all`.

## 7. Untrusted PR boundary

- No `pull_request_target` anywhere (audited F1). Untrusted PR code runs with
  default `contents: read` in `fast-local.yml`.
- Signing/release credentials are never exposed to `pull_request` events.
- `release.yml` (tag push) currently injects signing secrets into `tauri-action`;
  F3+ restructures this under `RELEASE_GATE` with explicit permissions.

## 8. False-green contract (B5)

Command exit 0 alone is never PASS. Every job verifies expected target count
(via `collect-test-evidence.mjs` + `verify-test-evidence.mjs`) and expected
evidence files. Vitest runs with `passWithNoTests: false` and `allowOnly: false`.

## 9. Network model (per job)

| Job | Network |
|---|---|
| FAST_LOCAL | `DEPENDENCY_PROVISIONING_ONLY` (npm ci, cargo --locked fetch; no PDFium, no artifact) |
| NATIVE (macOS) | `NONE_REQUIRED` for runtime (repo-committed dylib) + dependency provisioning |
| NATIVE (linux/windows) | `RUNTIME_PROVISIONING` — BLOCKED until canonical metadata is pinned |
| ARTIFACT_BUILD | `DEPENDENCY_PROVISIONING_ONLY` |
| ARTIFACT_SMOKE | `ARTIFACT_TRANSFER` (download-artifact; runtime bundled in artifact) |
| RELEASE_GATE | `EXTERNAL_SERVICE_REQUIRED` (trusted publish) |
| DEPENDENCY_AUDIT | `ADVISORY_DB_FETCH` (crates.io for the pinned cargo-audit, RustSec advisory-db) |

## 10. Known gaps (unchanged by F2)

```
CI_ACTION_PIN_REQUIRED            — all actions floating (@v4/@stable/@v0); no SHA fabricated
RUNTIME_PROVISIONING_INPUT_INCOMPLETE — Linux libpdfium.so / Windows pdfium.dll absent
BRANCH_PROTECTION_REQUIREMENTS: UNKNOWN — not visible from codebase
CI EXECUTION: NOT RUN               — local agent is strict-offline; no workflow dispatch
```

## 11. Dependency audit

`dependency-audit.yml` runs `cargo audit` (cargo-audit 0.22.2, Rust 1.99.1) in
`src-tauri/` on every pull request, on `main` and every Monday. A RustSec
vulnerability fails the job; unmaintained/unsound warnings are reported only. It is
kept out of FAST_LOCAL because fetching the advisory database is outside that job's
network model. Ignored advisories live in `src-tauri/.cargo/audit.toml`, each with
its containment and exit condition. The list is empty: RUSTSEC-2026-0187 (lopdf 0.34)
was removed with the lopdf 0.45 upgrade.
