# 11I-F Handoff — Artifact Production → Smoke CI (SHA-256 Identity Contract)

**Owner**: 11I-F (CI wiring) ← produced for, not executed by, 11I-E
**Boundary**: This handoff defines the release-verification pipeline for 11V-E.
It embeds the workflow as a deliverable (F handoff-only); it does **not** commit
`.github/workflows/*` files and does **not** run in this stage.

---

## 1. Purpose

Prove, without trusting the transfer channel, that:

> **the artifact that is smoke-tested is byte-identical to the artifact that was produced.**

This is enforced via a producer→consumer SHA-256 identity contract, independent of
GitHub's storage layer.

## 2. SHA-256 Identity Contract (required)

```
producer:  build artifact  → compute SHA-256 (lowercase hex) → emit {file, sha256}
transfer:  upload artifact + upload/handshake the hash sidecar
consumer:  download artifact → re-hash → compare to producer hash → MUST match
           └─ mismatch → FAIL (evidence retained, NO release promotion)
```

- Hash is computed with the repository's harness helper (`hashSha256`, stdlib
  `node:crypto`), not a third-party checksum tool.
- Match is **required**; any mismatch is fail-closed (`overallStatus: FAIL`, exit 1).
- The harness already records `artifactIdentity.sha256`; the consumer compares the
  live re-hash against the producer's recorded value before executing smoke.

## 3. Producer Job

| Field | Value |
|-------|-------|
| Job | `produce` |
| Runs on | matrix per platform (macos-arm64 → `macos-14` arm64 runner, linux-x64 → `ubuntu-22.04`, windows-x64 → `windows-2022`) |
| Build | `npm ci && npm run tauri build -- --target <triple>` (runs `tauri build`, bundling `dmg`/`app`/`deb`/`msi`/`nsis` per `tauri.conf.json`) |
| Output | exact artifacts per `docs/engineering/11v-e-handoff.json` matrix rows |
| Identity | compute SHA-256 of each produced artifact; write `artifact-manifest.json` (filename → sha256 → byteLength) |
| Signing | **out of scope** (11I-G/11R). No codesign/notarization step here. |

Producer does **not** run smoke; separation prevents the producer host's environment
from masking a packaging defect.

## 4. Consumer Smoke Jobs

| Field | Value |
|-------|-------|
| Job | `smoke` (per-artifact matrix) |
| Runs on | same-OS native runner for the artifact class |
| Harness | `node scripts/artifact-smoke-<platform>.mjs --artifact <downloaded> --artifact-type <TYPE> --output <evidence>.json` |
| Fixture | `core-release-smoke` copied from repo, SHA-256 verified against `corpus-manifest.json` before use |
| Gate | harness exit code gates promotion; `ACTIONS_RUN_ATTEMPT`/exit 0 alone is **not** PASS |

Map:

| Artifact | Harness | Env-equivalent |
|----------|---------|----------------|
| `MACOS_DMG` / `MACOS_APP_BUNDLE` | `artifact-smoke-macos.mjs` | `ARTIFACT_SMOKE_MACOS_PATH` |
| `LINUX_DEB` | `artifact-smoke-linux.mjs` | `ARTIFACT_SMOKE_LINUX_PATH` |
| `WINDOWS_MSI` / `WINDOWS_NSIS` | `artifact-smoke-windows.mjs` | `ARTIFACT_SMOKE_WINDOWS_PATH` |

## 5. Transfer Mechanism

- `actions/upload-artifact` (producer) → `actions/download-artifact` (consumer).
- The SHA-256 sidecar (`artifact-manifest.json`) travels as an additional artifact.
- The identity contract does **not** rely on `actions/upload-artifact` integrity
  guarantees; consumer re-hashes and compares unconditionally.

## 6. Runner + Prerequisites

| Platform | Runner | Native tools required | Node |
|----------|--------|----------------------|------|
| macOS arm64 | `macos-14` (arm64) | `hdiutil` (built-in) | `^20.19 \|\| ^22.12 \|\| >=24` |
| Linux x64 | `ubuntu-22.04` | `dpkg`, `dpkg-deb` (built-in) | same |
| Windows x64 | `windows-2022` | `msiexec`, `powershell` (built-in) | same |

No network fetch of PDFium runtime: the runtime is bundled into the artifact
(`bundled_with_artifact`, `platform-support.json`). Offline tooling only.

## 7. Evidence Upload

- Each consumer uploads `*-smoke-evidence-*.json` (ArtifactSmokeEvidence schema 1.0.0)
  keyed by artifact class and run attempt.
- Evidence must satisfy `verify-test-evidence.mjs` invariants (B5 fail-closed):
  exit 0 without evidence is rejected.

## 8. Failure Retention

Retain:
- `*-smoke-evidence-*.json` (always — this is the evidence of pass/fail)
- `artifact-manifest.json` + harness stderr/stdout

Do **not** retain:
- Secrets, codesign identity, signing certificates, notarization credentials
- Real user PDFs (smoke uses only the committed canonical fixture, never user data)
- Temporary workspaces / mounted DMGs / extracted install roots

## 9. Security

- No secrets are written into evidence JSON (evidence schema has no credential field;
  `validate-pdfium-runtime-manifest.mjs` forbids executable/downloader config).
- Fixture is path-pinned and hash-verified; no arbitrary PDF path accepted
  (contract §22).
- Worker spawn is `--run-worker` `production_ipc`; no shell execution, no debug hook
  beyond the bounded `--smoke` command set (contract §11/§12).

## 10. Workflow (11I-F handoff only — DESIGN, not committed)

```yaml
# docs/engineering/11i-f-handoff.md — reference workflow, NOT committed (.github/workflows)
name: 11V-E exact-artifact smoke
on:
  workflow_dispatch:

permissions:
  contents: read

jobs:
  produce:
    strategy:
      matrix:
        include:
          - { os: macos-14,        triple: aarch64-apple-darwin,      node: 22 }
          - { os: ubuntu-22.04,    triple: x86_64-unknown-linux-gnu,  node: 22 }
          - { os: windows-2022,    triple: x86_64-pc-windows-msvc,    node: 22 }
    runs-on: ${{ matrix.os }}
    outputs:
      manifest: ${{ steps.manifest.outputs.json }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: ${{ matrix.node }} }
      - run: npm ci
      - run: npm run tauri build -- --target ${{ matrix.triple }}
      - id: manifest
        run: node scripts/compute-artifact-manifest.mjs > artifact-manifest.json
      - uses: actions/upload-artifact@v4
        with:
          name: artifacts-${{ matrix.triple }}
          path: |
            src-tauri/target/${{ matrix.triple }}/release/bundle/**/*
            artifact-manifest.json

  smoke:
    needs: produce
    strategy:
      matrix:
        include:
          - { os: macos-14,       platform: macos,   type: MACOS_DMG }
          - { os: windows-2022,   platform: windows, type: WINDOWS_MSI }
          - { os: windows-2022,   platform: windows, type: WINDOWS_NSIS }
          - { os: ubuntu-22.04,   platform: linux,   type: LINUX_DEB }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - uses: actions/download-artifact@v4
        with:
          name: artifacts-${{ matrix.platform }}
          path: ./dist
      - name: verify identity contract (re-hash == producer hash)
        run: node scripts/verify-artifact-identity.mjs ./dist
      - name: exact-artifact smoke
        run: |
          node scripts/artifact-smoke-${{ matrix.platform }}.mjs \
            --artifact "$ARTIFACT_PATH" \
            --artifact-type ${{ matrix.type }} \
            --output ${{ matrix.type }}.json
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: evidence-${{ matrix.type }}
          path: ${{ matrix.type }}.json
```

> Notes: `scripts/compute-artifact-manifest.mjs` and
> `scripts/verify-artifact-identity.mjs` are 11I-F wiring seams **not yet
> implemented** — outlined here as the required producer/consumer contract, not
> claimed as existing tooling. The existing harnesses (`artifact-smoke-*.mjs`),
> the evidence schema and the registry are already implemented.