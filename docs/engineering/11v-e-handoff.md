# 11V-E Handoff — Exact Artifact Smoke Matrix (§20)

**Machine-readable**: `docs/engineering/11v-e-handoff.json`

This handoff is the release-verification matrix consumed by 11V-E. It does **not**
execute anything; it defines, per platform/artifact, the deployment tool, install
method, executable/worker/runtime expectations, required smoke assertions, timeouts
and cleanup policy. All values are derived from source:

- `docs/engineering/artifact-smoke-contract.md`
- `docs/engineering/platform-support.json`
- `docs/engineering/test-execution-registry.json`
- `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml`
- `scripts/artifact-smoke-{common,macos,linux,windows}.mjs`

## Key invariants

| Item | Value |
|------|-------|
| Smoke binary | `preview-alternative` (same binary for `--run-worker`) |
| Smoke hook | `--smoke` interactive IPC |
| Worker spawn | `production_ipc` (never fake helper / direct pdfium) |
| Runtime source | `packaged` only, resolved under install prefix |
| Runtime files | macOS `libpdfium.dylib`, Linux `libpdfium.so`, Windows `pdfium.dll` |
| Canonical fixture | `core-release-smoke` (SHA-256 pinned in contract) |

## Required vs optional

- **Required external validators**: none. VeraPDF (PDF/UA-1, PDF/UA-2) is
  `OptionalDiagnostic`; the machine-validation feature is disabled
  (`src-tauri/src/external_validation/types.rs`). Absence → `TOOL_UNAVAILABLE`,
  never `FAIL`, never release-blocking.

## Trigger pattern

Each row maps 1:1 to a registry target and its evidence output name:

| Artifact | Env var | Rust test (ignored) | Evidence output |
|----------|---------|---------------------|-----------------|
| MACOS_DMG / MACOS_APP_BUNDLE | `ARTIFACT_SMOKE_MACOS_PATH` | `rust:artifact_smoke_mac` | `macos-smoke-evidence-<type>.json` |
| LINUX_DEB | `ARTIFACT_SMOKE_LINUX_PATH` | `rust:artifact_smoke_linux` | `linux-smoke-evidence-linux_deb.json` |
| WINDOWS_MSI / WINDOWS_NSIS | `ARTIFACT_SMOKE_WINDOWS_PATH` | `rust:artifact_smoke_windows` | `windows-smoke-evidence-<type>.json` |

`ARTIFACT_SMOKE_OUTPUT_DIR` routes evidence to a specific output directory.

## Execution boundary

Real artifact smoke (`11V-E`) is the **only** valid exact-artifact evidence.
`cargo test`, `tauri dev`, unpackaged binaries and source-tree paths are not
substitutes (contract §6/§7). Real artifact verification is **deferred** — no
`.dmg`/`.deb`/`.msi` currently exists in the repository, and the harnesses correctly
report `ARTIFACT_UNAVAILABLE` / non-zero exit when an artifact is absent.