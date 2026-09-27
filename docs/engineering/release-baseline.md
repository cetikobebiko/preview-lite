# Prompt 11 Release and Verification Baseline

Captured on 20 August 2026 for Prompt 11.0. This is a source and repository
inventory, not release verification evidence. No platform build, full test
suite, package smoke, signing check, or native runtime verification was run to
create this baseline.

## Baseline identity and freeze state

| Field | Value |
|---|---|
| Baseline HEAD commit | `7b82b0319a4e12c58556567f2890a2fa310961e9` |
| Branch | `main` |
| Upstream state | `origin/main`, ahead by 1 commit |
| Working tree at capture | Dirty: 153 porcelain entries (104 modified, 11 deleted, 38 untracked) |
| Implementation freeze | **NOT ESTABLISHED** |
| Freeze commit | None |

The baseline HEAD identifies the commit underneath the dirty working-tree
snapshot. It is not a Prompt 11 implementation-freeze SHA. A freeze can only
be established after 11I-A through 11I-G are complete; native verification
evidence must then identify that exact freeze commit.

All pre-existing working-tree changes are user-owned. Prompt 11.0 does not
reset, restore, reformat, stage, or otherwise adopt them.

## Host and toolchain inventory

| Field | Observed value | Release interpretation |
|---|---|---|
| Host | macOS `26.6.1` (`25G76`), arm64 | Development host only; not a cross-platform build farm |
| Repository package manager | `npm@10.9.2` | Exact policy from `packageManager` |
| Repository Node policy | `^20.19.0 || ^22.12.0 || >=24.0.0` | Recorded in `package.json` and lockfile root |
| Repository npm engine | `10.9.x` | Local npm 11 is not canonical |
| Local Node | `v26.5.0` | Satisfies repository Node range |
| Local global npm | `11.17.0` | Does not satisfy repository npm engine; not changed by 11.0 |
| `package-lock.json` | lockfile v3, SHA-256 `580072079b351b9fa93935971339984f420d3c84a47caa4c94ca31826d589022` | Existing reproducibility baseline; not regenerated |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` | Homebrew toolchain |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` | Homebrew toolchain |
| rustup | Not installed | Active rustup toolchain cannot be reported |
| `rust-toolchain.toml` | Absent | Release toolchain is currently floating in workflows |
| `src-tauri/Cargo.lock` | SHA-256 `07fd56015eee2455572d49cdee0906e83b8e9c6dde1f2eb6cb85f2cf6d73c437` | Existing lockfile; not changed by 11.0 |

The package policy is internally consistent: `packageManager` is
`npm@10.9.2`, both root engine records require npm `10.9.x`, and the
cross-platform architecture test asserts those values. The lockfile contains
the optional native packages for supported npm dependency architectures,
including `@tauri-apps/cli-darwin-arm64@2.11.4`. The current `node_modules`
tree is missing that native binding, so the local Tauri CLI cannot currently
start. This is a local installed-state observation, not a lockfile defect and
not a platform verification result.

## Framework and native dependency versions

| Component | Resolved version or state |
|---|---|
| Svelte | `5.56.9` |
| Vite | `5.4.21` |
| Vitest | `4.1.10` |
| svelte-check | `4.7.6` |
| Tailwind CSS | `4.3.3` |
| shadcn-svelte | `1.5.0` |
| Tauri JavaScript CLI | `2.11.4` in `package-lock.json` |
| `tauri` crate | `2.11.5` in `src-tauri/Cargo.lock` |
| `tauri-build` crate | `2.6.3` in `src-tauri/Cargo.lock` |
| `pdfium-render` | `0.8.37` in `src-tauri/Cargo.lock` |
| Tauri plugins | dialog, fs, shell, log registered; updater dependency present but not registered |

`package.json`, `Cargo.toml`, and `tauri.conf.json` report application version
`0.1.0`. No tag/updater/changelog version consistency claim is made here.

## PDFium runtime inventory

The repository contains only a macOS arm64 runtime. Both tracked paths below
are byte-identical copies.

| Path | Architecture | Bytes | SHA-256 |
|---|---|---:|---|
| `src-tauri/libpdfium.dylib` | Mach-O 64-bit arm64 | 7,151,568 | `c2f68f87106da4f0cb6d6a90a7aad9644b3c1cb92a3463dbdc8f39449aec3869` |
| `src-tauri/lib/libpdfium.dylib` | Mach-O 64-bit arm64 | 7,151,568 | `c2f68f87106da4f0cb6d6a90a7aad9644b3c1cb92a3463dbdc8f39449aec3869` |

The adjacent `src-tauri/VERSION` labels the repository runtime as
`152.0.7961.0`. Mach-O metadata declares macOS 13.0 as the minimum OS and SDK
26.0 as the build SDK. Direct dependencies are AppKit, CoreGraphics,
CoreFoundation, Foundation, and `libSystem`.

Current supply-chain gaps:

- There is no central PDFium runtime manifest.
- The exact upstream source URL, source artifact checksum, source commit,
  builder, and build date are not recorded with the binary.
- `libpdfium.so` for `x86_64-unknown-linux-gnu` is absent.
- `pdfium.dll` for `x86_64-pc-windows-msvc` is absent.
- `src-tauri/licenses/pdfium.txt` exists, but no complete root
  `THIRD_PARTY_NOTICES.md` exists.
- No Linux or Windows runtime is inferred from source portability or workflow
  configuration.

## Runtime loader baseline

The application setup resolves Tauri's resource directory and assigns its
`lib` child to `PDFIUM_PATH`. The common PDFium loader then tries, in order:

1. `PDFIUM_PATH` when present;
2. `./src-tauri/lib`;
3. `./lib`;
4. the current directory;
5. the system PDFium library.

The worker launcher propagates `PDFIUM_PATH`. It does not currently receive a
parent-verified target/version/hash fingerprint. Consequently the current
loader is not a fail-closed release loader and cannot provide exact-artifact
evidence. Prompt 11.0 records this behavior without changing it; remediation
belongs to 11I-C.

## Bundle, signing, and updater baseline

`src-tauri/tauri.conf.json` has bundling active with `targets: "all"` and a
generic `lib/*` resource glob. There are no platform-specific Tauri config
files and no deterministic PDFium staging directory or script.

No macOS code-signing identity, entitlements, hardened-runtime policy,
notarization configuration, Windows Authenticode configuration, Linux package
signature configuration, or package-layout manifest exists. Relevant local
signing environment variables were observed as unset; no secret values were
read or recorded.

`tauri-plugin-updater` is a Cargo dependency, but it is not registered in the
Tauri builder and no updater endpoint/public-key configuration is present.
The release workflow references Tauri updater-signing secret names. This does
not constitute macOS code signing, Apple notarization, Windows Authenticode,
Linux signing, or a verified updater configuration.

## Workflow inventory

| Workflow | Trigger and current responsibility | Baseline issue |
|---|---|---|
| `.github/workflows/atomic-document-io.yml` | PR/main matrix plus macOS arm64 PDFium job | Mixes fast and native suites; full Rust suite runs where verified runtimes are absent |
| `.github/workflows/build-windows.yml` | Main push/manual Windows checks and package build | No verified `pdfium.dll`; build cannot be treated as runtime support evidence |
| `.github/workflows/release.yml` | Tag matrix tests, build, updater signing variables, and draft release | Validation, build, signing, and publication are combined |

All action references currently use floating tags such as `@v4`, `@v0`, or
`@stable`. Workflows do not declare default `permissions: contents: read`, job
timeouts, or concurrency policy. Rust is installed from floating `stable`.
No `pull_request_target` or `continue-on-error` use was found.

These are 11I-F/11I-G implementation findings, not changes made by 11.0.

## Test inventory and primary execution class

Source counts are inventory counts, not executed-test counts:

- 99 frontend `*.test.js`/`*.spec.js` files with 638 declared `it(...)` or
  `test(...)` calls.
- 476 Rust `#[test]`/`#[tokio::test]` attributes under `src-tauri/src`.
- 18 Rust integration targets with 247 test attributes under
  `src-tauri/tests`.
- One fuzz target: `src-tauri/fuzz/fuzz_targets/pdfium_lifecycle.rs`.

Primary classes have the following meaning:

- `FAST_LOCAL`: no verified PDFium binary or packaged application required.
- `NATIVE_PLATFORM`: real runtime, native worker, or mutation/persistence
  coverage; must run only where the target runtime is verified.
- `EXACT_ARTIFACT`: installed or extracted release artifact coverage.

| Suite or target | Primary class | Notes |
|---|---|---|
| `npm run test` | `FAST_LOCAL` | Frontend Vitest suite; backend calls are mocked/bounded |
| `npm run test:cross-platform` | `FAST_LOCAL` | Package policy, lockfile architecture, shell portability, and path checks |
| Rust library unit-test aggregate | `NATIVE_PLATFORM` | Currently mixes PDFium-independent and runtime-touching modules; 11I-B must split targeted fast gates |
| `document_io_fs` | `FAST_LOCAL` | Filesystem, identity, lock, ACL, and atomic publication behavior |
| `plugin_worker_process` | `FAST_LOCAL` | Test-only bounded plugin process protocol |
| `plugin_worker_supervisor` | `FAST_LOCAL` | Test-only worker lifecycle and supervision |
| `secure_redaction_raster_pure` | `FAST_LOCAL` | Pure geometry/merge/buffer primitives without PDFium (B2 split, 28 tests) |
| `secure_redaction_planning_worker` | `NATIVE_PLATFORM` | Real PDFium inspection & planning worker (B2 reclassified: 28 tests require libpdfium) |
| `secure_redaction_raster_primitives` | `NATIVE_PLATFORM` | PDFium mapping/render/raster-payload/plan-immutability with runtime (B2 retained 40 tests) |
| `signature_surface` | `FAST_LOCAL` | Disabled feature fails closed |
| `worker_broker_process` | `FAST_LOCAL` | Fake worker/broker lifecycle behavior |
| `crash_tests` | `NATIVE_PLATFORM` | Production worker crash/restart flow with PDF fixture |
| `document_io_pdfium` | `NATIVE_PLATFORM` | Real PDFium mutation/save/reopen |
| `job_worker_process` | `NATIVE_PLATFORM` | Real-process mutation and verification aggregate |
| `redaction_fixtures_and_capabilities` | `NATIVE_PLATFORM` | Aggregate includes explicit real PDFium initialization |
| `sanitization_capabilities` | `NATIVE_PLATFORM` | Aggregate includes real PDFium reopen/render checks |
| `sanitization_inspection_acceptance` | `NATIVE_PLATFORM` | Native macOS arm64 session/worker acceptance target |
| `sanitization_publication_acceptance` | `NATIVE_PLATFORM` | Mutation/publication persistence acceptance |
| `secure_redaction_execution_acceptance` | `NATIVE_PLATFORM` | Session-native execution/publication acceptance |
| `session_worker_integration` | `NATIVE_PLATFORM` | Production session-to-real-worker behavior |
| `whole_object_backend_characterization` | `NATIVE_PLATFORM` | Explicit real PDFium object mutation/reopen characterization |
| `worker_pdfium_integration` | `NATIVE_PLATFORM` | Real PDFium render, text, search, metadata, encryption, and worker lifecycle |
| Packaged artifact smoke | `EXACT_ARTIFACT` | No implementation exists yet |
| Fuzz target | Separate fuzz track | Not silently counted as one of the three release gates |

The current Rust library aggregate is deliberately classified conservatively
as `NATIVE_PLATFORM`; running a filtered PDFium-independent module does not
reclassify the aggregate. Repository-level suite separation and zero-test
protection belong to 11I-B.

## Fixture inventory

| Root | PDF | JSON | Generator | Documentation |
|---|---:|---:|---:|---:|
| `src-tauri/tests/fixtures/pdf` | 11 | 0 | 1 Python | 1 README |
| `src-tauri/tests/fixtures/redaction` | 26 | 1 manifest | 1 Python | 0 |
| `src-tauri/tests/fixtures/sanitization` | 16 | 1 manifest | 1 Python | 0 |
| `src-tauri/tests/fixtures/contracts` | 0 | 2 | 0 | 0 |
| **Total** | **53** | **4** | **3** | **1** |

No fixture symlinks were found. Existing feature-specific redaction and
sanitization manifests must not be duplicated or broken. There is currently no
central corpus manifest, no corpus coverage document, and no qualifying
two-page `release-smoke.pdf`; those belong to 11I-A.

## Current platform support

The canonical machine-readable record is
[`platform-support.json`](platform-support.json), with a human-readable view in
[`platform-support.md`](platform-support.md). All current-baseline platform
statuses are `not_evaluated` because this dirty snapshot has not passed Prompt
11 native or exact-artifact verification.

Earlier macOS and Linux results supplied in the Prompt 11 task context are
retained only as `historicalEvidence`. They are not current-baseline evidence,
cannot establish a freeze, and cannot authorize publication.

## Verification boundary

Prompt 11.0 performs only documentation/static integrity gates. Deferred work:

- 11V-A: clean-checkout fast repository verification;
- 11V-B: macOS arm64 native PDFium, worker, mutation, and cleanup evidence;
- 11V-C: native Linux x64 evidence after an approved runtime exists;
- 11V-D: native Windows x64 evidence after an approved runtime exists;
- 11V-E: exact packaged artifact smoke for every candidate platform.

No `PASS`, `verified`, signed-release, or release-ready claim follows from this
baseline.

## Prompt 11.0 fast local gate results

| Gate | Result | Detail |
|---|---|---|
| Canonical npm selection | PASS | Existing cached npm `10.9.2` was invoked directly; global npm remained `11.17.0` before and after |
| `npm run test:cross-platform` via npm `10.9.2` | **NOT RUN / RUNNER_ENVIRONMENT_FAILURE** | Vitest stopped during startup because the existing `node_modules` tree lacks a Rolldown native binding; 0 test cases executed |
| Platform-support JSON parse/invariants | PASS | All three current statuses are `not_evaluated`; baseline HEAD and `not_established` freeze invariants match |
| PDFium static architecture/hash/length | PASS | Both arm64 dylib copies match the recorded byte length and SHA-256 |
| Inventory paths/counts | PASS | 99 frontend test files, 18 Rust integration targets, and 53 PDF fixtures found |
| New-file whitespace check | PASS | `git diff --no-index --check` reported no errors for the three Prompt 11.0 files |

The Vitest startup failure is not a test assertion failure and is not a PASS.
Prompt 11.0 did not repair installed dependencies because `npm ci` was
explicitly outside the approved gate. No full frontend suite, Cargo suite,
Tauri build, native runtime verification, or exact-artifact smoke was run.
