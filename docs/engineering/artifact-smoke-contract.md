# Exact Artifact Smoke Contract — 11I-E1

**Status**: `VALID` (Gate 1 approved)  
**Schema Version**: `1.0.0`  
**Generated From**: Source audit of `tauri.conf.json`, `Cargo.toml`, `package.json`, `platform-support.json`, `test-execution-registry.json`, `pdf-corpus.md`, `release-baseline.md`

---

## 1. Exact Artifact Definition

> **EXACT_ARTIFACT** = The release pipeline's output artifact (or its installed result) that is intended for user distribution.

**Exact artifacts are:**
- `MACOS_DMG` — `PreviewLite_<version>_aarch64.dmg`
- `MACOS_APP_BUNDLE` — `PreviewLite.app` (inside DMG or standalone)
- `LINUX_DEB` — `preview-lite_<version>_amd64.deb`
- `WINDOWS_MSI` — `PreviewLite_<version>_x64_en-US.msi`
- `WINDOWS_NSIS` — `PreviewLite_<version>_x64-setup.exe`

**Exact artifacts are NOT:**
- `cargo run` / `tauri dev` / `target/debug` binaries
- Unpackaged worker binary
- Raw repository runtime directory (`src-tauri/lib/`)
- Mock installer layout
- Any source-tree execution path

---

## 2. Artifact Classes (Bounded Vocabulary)

```json
[
  "MACOS_DMG",
  "MACOS_APP_BUNDLE",
  "LINUX_DEB",
  "WINDOWS_MSI",
  "WINDOWS_NSIS"
]
```

Only classes actually produced by the configured build pipeline are valid.  
Current `tauri.conf.json` → `targets: ["dmg", "app", "deb", "msi", "nsis"]`.

---

## 3. Smoke Test Purpose (Minimum Viable)

The smoke test is **not** a regression suite. It proves:

```
artifact structurally exists
  → package/install/extract succeeds
  → executable launches
  → packaged runtime resolves (PDFium via TAURI_RESOURCE_DIR/lib/)
  → canonical release-smoke PDF opens
  → one or more known PDFium-backed operations succeed
  → clean shutdown
```

Platform-specific steps may differ (DMG mount vs DEB install vs MSI install).

---

## 4. Canonical Smoke Input

**Primary**: `core-release-smoke` fixture  
**Path**: `src-tauri/tests/fixtures/pdf/release-smoke.pdf`  
**SHA-256**: `727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611`  
**Size**: 4520 bytes  
**Pages**: 2 (A4, 595×842 pt)  
**Content markers**:
- `RELEASE_SMOKE_ALPHA_TOKEN` (page 1)
- `RELEASE_SMOKE_BETA_TOKEN` (page 2)
- `CANONICAL VERIFICATION TARGET` (page 2)
- `PAGE_MARKER_PAGE_1` / `PAGE_MARKER_PAGE_2`
- Turkish Unicode Type 3 font (İstanbul, Ankara, İzmir, Muğla, etc.)

**Generator**: `py-release-smoke-generator` (pure Python stdlib, `deterministic_by_construction`)  
**Verification**: `python3 generate_release_smoke.py --verify-determinism`

---

## 5. Minimum Assertions (from fixture contract)

| Assertion | Source | Required |
|-----------|--------|----------|
| Document opens | `release-smoke.pdf` is valid PDF 1.4 | REQUIRED |
| Expected page count = 2 | Fixture manifest `categories: ["multi_page"]` | REQUIRED |
| Known text token available | `RELEASE_SMOKE_ALPHA_TOKEN`, `RELEASE_SMOKE_BETA_TOKEN`, `CANONICAL VERIFICATION TARGET` | REQUIRED |
| Known page dimension | 595×842 pt (A4) | REQUIRED |
| Bounded render succeeds | Small thumbnail or page 1 render | REQUIRED |
| Clean close | Worker/document cleanup | REQUIRED |

**Not required**: Full text extraction, search, annotations, form filling, large-page render, OCR, export.

---

## 6. Source Test ≠ Artifact Test

The following **do not substitute** exact artifact smoke:

| Evidence | Class | Why Insufficient |
|----------|-------|------------------|
| `cargo test` PASS | NATIVE_PLATFORM / FAST_LOCAL | Runs source-tree binaries, dev runtime |
| Unpackaged binary PASS | NATIVE_PLATFORM | No package layout, no installer logic |
| Loader unit test PASS | FAST_LOCAL | Mocked runtime resolution |
| Package config validator PASS | FAST_LOCAL | Static config only, no execution |

**Artifact evidence only comes from real package/install/layout execution.**

---

## 7. Native Test ≠ Exact Artifact Test

| Verification Gate | Class | Scope |
|-------------------|-------|-------|
| 11V-B | NATIVE_PLATFORM | macOS arm64 source/native build + runtime |
| 11V-C | NATIVE_PLATFORM | Linux x64 source/native build + runtime |
| 11V-D | NATIVE_PLATFORM | Windows x64 source/native build + runtime |
| **11V-E** | **EXACT_ARTIFACT** | **Packaged artifact (.dmg/.deb/.msi) installed + smoke** |

**E1/E2/E3/E4 build the harness for 11V-E. They do not execute it.**

---

## 8. Application Executable Layout (Packaged)

Single binary with CLI entry points:
```
preview-alternative                    # Main GUI app
preview-alternative --run-worker       # PDFium render/text/search worker
preview-alternative --run-job-worker   # Mutation/persistence worker
preview-alternative --run-plugin-worker # Plugin (Rhai) worker
```

All workers are **the same executable**, spawned via `--run-*` args.  
Production smoke **must** exercise real worker spawn via `--run-worker` (not fake helper).

---

## 9. PDFium Runtime Resolution (Release)

Resolver priority (fail-closed, no fallback):

```
1. TAURI_RESOURCE_DIR/lib/<runtime>     ← PACKAGED (release)
2. PDFIUM_PATH (absolute, validated)    ← DEV OVERRIDE ONLY
3. Staged dev path (CARGO_MANIFEST_DIR) ← DEV ONLY
4. FAIL CLOSED — no CWD, PATH, system search
```

**Runtime filenames by platform:**
- macOS: `libpdfium.dylib`
- Linux: `libpdfium.so`
- Windows: `pdfium.dll`

**Smoke must verify**: Runtime resolves via path #1 (packaged), not #2/#3.

---

## 10. Worker Smoke Requirement

If production architecture is worker-based (it is), smoke **must** use real production path:

```
installed app
  → real worker spawn (--run-worker)
  → open release-smoke.pdf
  → command (render/thumbnail/text/search)
  → cleanup
```

**Not acceptable**: Direct `pdfium-render` test binary, fake worker helper, mocked IPC.

---

## 11. GUI Automation Not Required

Artifact smoke **does not require** mouse/visual GUI automation.  
If the application exposes an internal CLI/test hook that exercises the **exact same production backend path** (same binary, same worker spawn, same runtime resolution), that is preferred for determinism.

**Constraint**: Test hook must not bypass production runtime path.

---

## 12. Test Hook Security (If Used)

If a smoke hook is added, it must satisfy:

- Explicit `--smoke` / `--test-mode` flag (not hidden)
- Bounded command set (open, render, text, search, close, shutdown)
- No arbitrary shell execution
- No arbitrary filesystem mutation
- No debug backdoor exposure
- Release binary security surface not unnecessarily expanded

**Prefer**: Existing CLI surface (`--run-worker` IPC) over new hooks.

---

## 13. Artifact Identity (Per Execution)

Every smoke execution must capture:

```json
{
  "platform": "macos|linux|windows",
  "architecture": "arm64|x86_64",
  "artifactType": "MACOS_DMG|MACOS_APP_BUNDLE|LINUX_DEB|WINDOWS_MSI|WINDOWS_NSIS",
  "applicationVersion": "0.1.0",
  "artifactFileHash": "sha256:...",          // if computable
  "buildProvenance": "ci-run-id|local-build"  // or "unknown"
}
```

**Artifact hash ≠ PASS** — hash is identity, behavior is evidence.

---

## 14. Evidence Model

### 14.1 Status Vocabulary (Bounded, Extends B5)

```
PASS
FAIL
NOT_RUN
NOT_APPLICABLE
TOOL_UNAVAILABLE
RUNTIME_UNAVAILABLE
ARTIFACT_UNAVAILABLE
TOOL_UNAVAILABLE_OFFLINE
```

**`SKIP` alone is forbidden** — use explicit status above.

### 14.2 ArtifactSmokeEvidence Schema

```json
{
  "schemaVersion": "1.0.0",
  "platform": "macos",
  "architecture": "arm64",
  "artifactType": "MACOS_DMG",
  "artifactIdentity": {
    "filePath": "/path/to/PreviewLite_0.1.0_aarch64.dmg",
    "sha256": "c2f68f87...",
    "byteLength": 12345678,
    "buildProvenance": "local-build-20260829"
  },
  "installResult": {
    "status": "PASS|FAIL|NOT_APPLICABLE",
    "method": "dmg_mount_copy|deb_install|msi_install|nsis_install|app_bundle_direct",
    "installPath": "/Applications/PreviewLite.app",
    "durationMs": 1234,
    "error": null
  },
  "launchResult": {
    "status": "PASS|FAIL",
    "executablePath": "/Applications/PreviewLite.app/Contents/MacOS/preview-alternative",
    "pid": 12345,
    "startupDurationMs": 567,
    "error": null
  },
  "runtimeResolutionResult": {
    "status": "PASS|FAIL",
    "resolvedPath": "/Applications/PreviewLite.app/Contents/Resources/lib/libpdfium.dylib",
    "source": "packaged|dev_override|staged_dev",
    "sha256": "c2f68f87...",
    "error": null
  },
  "smokeDocumentResult": {
    "status": "PASS|FAIL",
    "fixtureId": "core-release-smoke",
    "fixturePath": "/tmp/artifact-smoke-abc123/release-smoke.pdf",
    "pageCount": 2,
    "textTokensFound": ["RELEASE_SMOKE_ALPHA_TOKEN", "RELEASE_SMOKE_BETA_TOKEN"],
    "renderTest": {
      "status": "PASS|FAIL",
      "operation": "thumbnail|page_render",
      "dimensions": "256x256",
      "durationMs": 89
    },
    "workerUsed": true,
    "workerSpawnMethod": "production_ipc",
    "error": null
  },
  "shutdownResult": {
    "status": "PASS|FAIL",
    "cleanExit": true,
    "workerCleanup": true,
    "durationMs": 45,
    "error": null
  },
  "externalValidationResults": [],
  "overallStatus": "PASS|FAIL",
  "timestamp": "2026-08-29T12:34:56Z",
  "durationTotalMs": 2345
}
```

**Do not dump full console logs as JSON** — structured evidence only.

---

## 15. Required vs Optional Checks

| Check | Class | Rationale |
|-------|-------|-----------|
| Artifact file exists | REQUIRED | Prerequisite |
| Package/install/extract succeeds | REQUIRED | Validates installer |
| Executable launches | REQUIRED | Validates bundle layout |
| Packaged runtime resolves | REQUIRED | **Critical** — proves self-containment |
| Smoke document opens | REQUIRED | Validates PDFium integration |
| Known text token found | REQUIRED | Validates text extraction |
| Bounded render succeeds | REQUIRED | Validates render pipeline |
| Clean shutdown | REQUIRED | Validates resource cleanup |
| Worker spawns via production IPC | REQUIRED | Validates worker architecture |
| External validator (VeraPDF) | OPTIONAL_DIAGNOSTIC | Roadmap-dependent, tool may be absent |
| Full text extraction | OPTIONAL_DIAGNOSTIC | Beyond smoke scope |
| Search/annotations/forms | OPTIONAL_DIAGNOSTIC | Beyond smoke scope |

**Optional tool absence → `TOOL_UNAVAILABLE` (not FAIL)**  
**Required step failure → `FAIL` (overall FAIL)**

---

## 16. False-Green Rules (Fail-Closed)

The following **must produce FAIL**, never PASS:

| False-Green Pattern | Required Response |
|---------------------|-------------------|
| Artifact missing → script exits 0 | `ARTIFACT_UNAVAILABLE` + non-zero exit |
| App launch skipped → overall PASS | `FAIL` (launchResult=FAIL) |
| Runtime missing → warning only | `FAIL` (runtimeResolutionResult=FAIL) |
| Release-smoke not found → skip | `FAIL` (smokeDocumentResult=FAIL) |
| 0 assertions executed → PASS | `FAIL` (overallStatus=FAIL) |
| Required external validator absent → silent ignore | `TOOL_UNAVAILABLE` in externalValidationResults, overall unaffected IF optional |
| Dev runtime (PDFIUM_PATH) used in release context | `FAIL` (runtimeResolutionResult.source != "packaged") |
| Source-tree binary executed instead of packaged | `FAIL` (launchResult.executablePath not under install prefix) |
| Fake worker used instead of --run-worker | `FAIL` (smokeDocumentResult.workerSpawnMethod != "production_ipc") |

---

## 17. Install vs Portable/Extract Model

| Artifact | Semantics | Harness Action |
|----------|-----------|----------------|
| MACOS_DMG | Mount → copy .app to temp Applications | `hdiutil attach`, `cp -R`, `hdiutil detach` |
| MACOS_APP_BUNDLE | Direct copy to temp | `cp -R` |
| LINUX_DEB | `dpkg -x` to temp root | `dpkg -x`, `PATH` adjustment |
| WINDOWS_MSI | `msiexec /qn /i` to temp prefix | `msiexec`, temp install dir |
| WINDOWS_NSIS | Silent install to temp | `installer.exe /S /D=tempdir` |

**Common conceptual lifecycle** (per platform harness):
```
prepare (temp dirs, fixture copy)
  → deploy (mount/install/extract)
  → launch (spawn executable)
  → smoke (IPC commands to worker)
  → cleanup (uninstall/eject, remove temp)
```

No universal "install" function — platform-specific but lifecycle-unified.

---

## 18. Clean Environment Requirement

Smoke **must not** depend on developer machine global state:

- ❌ Global PDFium (`/usr/lib/libpdfium.so`, system `pdfium.dll`)
- ❌ Dev runtime override (`PDFIUM_PATH` pointing to source tree)
- ❌ Source-tree worker binary (`target/release/preview-alternative`)
- ❌ Source-tree assets (`src-tauri/tests/fixtures/...` directly)

**Smoke must prove**: Artifact contains its own runtime, workers, and resources.

---

## 19. Artifact Self-Containment Verification

The smoke **specifically validates**:

> Does the artifact actually contain the application's production runtime dependencies as intended?

For PDFium: `runtimeResolutionResult.source == "packaged"` and `resolvedPath` under install prefix.

---

## 20. Release Runtime Source Proof

Evidence must show runtime originated from **packaged application-owned location**:

```json
"runtimeResolutionResult": {
  "source": "packaged",
  "resolvedPath": "/Applications/PreviewLite.app/Contents/Resources/lib/libpdfium.dylib"
}
```

If `source == "development_override"` or `source == "staged_development"` → **FAIL** (invalid artifact smoke).

---

## 21. Fixture Ownership Model

`core-release-smoke` is a **committed repository fixture**.

Two valid models for artifact smoke:

| Model | Description | Trade-off |
|-------|-------------|-----------|
| **Packaged test resource** | Fixture bundled inside artifact (e.g., `Resources/test-fixtures/release-smoke.pdf`) | Self-contained; increases artifact size; requires build-time packaging |
| **Controlled external input** | Fixture copied from known-good source to temp workspace at smoke time | Smaller artifact; requires canonical hash verification at smoke time |

**E1 Decision**: **Controlled external input** (simpler, no build changes, hash-verified).  
Harness copies `release-smoke.pdf` from repo (verified via manifest SHA-256) to temp workspace.

---

## 22. Controlled External Input Rules

If fixture is external to artifact:

- Harness **verifies fixture SHA-256** against `corpus-manifest.json` before use
- **No path injection** — fixture path is harness-controlled temp location
- Arbitrary PDF paths **not permitted** — only canonical smoke fixture

---

## 23. No Artifact Mutation

Smoke test **must not**:
- Modify the signed/built artifact file (`.dmg`, `.deb`, `.msi`)
- Mutate packaged PDFium binary
- Overwrite release resources
- Modify installed application bundle post-install

Temp test data only in owned temp workspace.

---

## 24. Output Isolation

All smoke output **must** reside in:
- Platform-appropriate temp directory (`$TMPDIR`, `%TEMP%`, `/tmp`)
- Harness-owned test workspace (e.g., `/tmp/artifact-smoke-<random>/`)

**Never write to**: User Documents, Desktop, Downloads, home directory, system paths.

---

## 25. Cleanup Rules

| Resource | Cleanup Policy |
|----------|----------------|
| Temp workspace | Best-effort removal; explicit failure reported |
| DMG mount | `hdiutil detach` (best-effort) |
| DEB extract | Remove temp root |
| MSI/NSIS install | Bounded uninstall if supported; else temp dir removal |
| Worker processes | Must terminate on main exit; verify cleanup |

**Unrelated system state must not be touched.**

---

## 26. Timeouts (Bounded, No Infinite Wait)

| Operation | Suggested Timeout |
|-----------|-------------------|
| DMG mount / DEB extract / MSI install | 60s |
| App launch (to IPC ready) | 30s |
| Worker spawn | 10s |
| Document open | 15s |
| Render/thumbnail/text command | 15s |
| Clean shutdown | 10s |

**Timeout → FAIL** (not hang). Each external process has bounded timeout.

---

## 27. Exit Codes

| Condition | Exit Code |
|-----------|-----------|
| All REQUIRED checks PASS | `0` |
| Any REQUIRED check FAIL / NOT_RUN | `1` (non-zero) |
| Artifact unavailable | `2` |
| Tool unavailable (optional) | `0` (with `TOOL_UNAVAILABLE` in evidence) |

**Evidence file and process exit code must agree** — never `exit 0` with `overallStatus: FAIL`.

---

## 28. No Silent Fallback

**Forbidden fallbacks:**
- Artifact launch fails → source-tree binary
- Packaged runtime missing → `PDFIUM_PATH` dev override
- Worker spawn fails → direct PDFium call
- Install fails → run from source tree

**Exact artifact test meaning is voided by any fallback.**

---

## 29. Platform-Specific Handoff

| Phase | Target | Artifact Classes |
|-------|--------|------------------|
| **11I-E2** | macOS | `MACOS_DMG`, `MACOS_APP_BUNDLE` |
| **11I-E3** | Linux | `LINUX_DEB` |
| **11I-E4** | Windows | `WINDOWS_MSI`, `WINDOWS_NSIS` |

Each harness implements this contract using platform-native tools.

---

## 30. 11V-E Boundary

| Phase | Responsibility |
|-------|----------------|
| **11I-E1..E4** | Harness infrastructure (contract, common lib, platform stubs) |
| **11V-E** | Actual execution against real release artifacts |

E1-E4 **do not require** real artifacts to exist locally.  
Harnesses must handle `ARTIFACT_UNAVAILABLE` gracefully (evidence + non-zero exit).

---

## 31. CI Relationship (11I-F)

Harnesses are designed for future CI integration:
- `11I-F` will wire them into GitHub Actions matrix
- Evidence JSON collected as workflow artifact
- Exit codes gate release promotion

**E1-E4 do not write CI workflows.**

---

## 32. E1 Acceptance Criteria (Met)

- ✅ Exact artifact concept separated from source/native tests
- ✅ Configured artifact classes derived from source (`tauri.conf.json`)
- ✅ Release-smoke input contract explicit (fixture ID, hash, content markers)
- ✅ Required smoke lifecycle defined (prepare→deploy→launch→smoke→cleanup)
- ✅ Worker/runtime behavior represented in smoke (production IPC path)
- ✅ Artifact identity/evidence model bounded and machine-readable
- ✅ Required/Optional checks classified
- ✅ False-green semantics fail-closed
- ✅ No source-tree fallback policy explicit
- ✅ Cleanup/timeouts explicit
- ✅ E2/E3/E4 handoff prepared
- ✅ CI/verification scope separated
- ✅ Zero network access in E1

---

## 33. Implementation Milestones

- ✅ **11I-E1** — Common Exact Artifact Smoke Contract & Shared Harness Library (`scripts/artifact-smoke-common.mjs`)
- ✅ **11I-E2** — macOS Artifact Smoke Harness (`scripts/artifact-smoke-macos.mjs`, `scripts/test-macos-smoke-harness.mjs`, `src-tauri/tests/artifact_smoke_mac.rs`)
- ✅ **11I-E3** — Linux Artifact Smoke Harness (`scripts/artifact-smoke-linux.mjs`, `scripts/test-linux-smoke-harness.mjs`, `src-tauri/tests/artifact_smoke_linux.rs`)
- ✅ **11I-E4** — Windows Artifact Smoke Harness (`scripts/artifact-smoke-windows.mjs`, `scripts/test-windows-smoke-harness.mjs`, `src-tauri/tests/artifact_smoke_windows.rs`)

---

## 34. Next Stage

**11I-E5** — Cross-Platform Smoke Matrix & Handoff Preparation