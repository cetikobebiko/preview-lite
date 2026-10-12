# Canonical Test Execution Architecture — 11I-B1 / B2 / B5

Source-of-truth for repository test layering: **FAST_LOCAL / NATIVE_PLATFORM / EXACT_ARTIFACT**.

**B2 amendment (11I-B2):** `secure_redaction_planning_worker` FAST→NATIVE reclassified (`usesPdfium=true`, `pdfium_dynamic_lib`, `supports macos`) — `inspect_and_plan_secure_redaction` `src-tauri/src/secure_redaction_inspection.rs:51` `get_pdfium()` runtime gereksinimi nedeniyle. `secure_redaction_raster_primitives` 68-test mixed aggregate B2’de ikiye bölündü: `secure_redaction_raster_pure.rs` (FAST_LOCAL 28 pure geometry/merge/buffer, `usesPdfium=false`) + `secure_redaction_raster_primitives.rs` (NATIVE 40 pdfium mapping/render/payload). `document_io_fs` test sayısı derlenen izole modül testlerini (25 birim + 1 tip kontrolü) tam yansıtacak şekilde `expectedTestCount: 26` olarak eşitlendi. `test_execution_registry_integrity` (6 saf Rust testi) registry'ye tescil edildi. Registry `totalTargets 27`, `FAST_LOCAL 10`, `NATIVE_PLATFORM 14`, `EXACT_ARTIFACT 3`, `pdfiumRuntimeRequired 17`, `realWorkerRequired 10`.

**B5 amendment (11I-B5):** Zero-test / skip / false-green hardening — `Exit 0 ≠ PASS`. Bounded taxonomy `ZERO_TESTS, FILTER_NO_MATCH, CFG_ELIDED, IGNORED_TEST, RUNTIME_UNAVAILABLE, TOOL_UNAVAILABLE, DEPENDENCY_UNAVAILABLE, MOCK_REALITY_MISMATCH, AGGREGATE_MIXING, EXPECTED_EVIDENCE_MISSING` eklendi. `crash_tests` `exact 3 → conditional default:0 on_macos_aarch64:3` (B2 sonrası tek kalan `exact` cfg masquerade düzeltmesi). Fail-closed evidence validator `scripts/verify-test-evidence.mjs` + vitest `passWithNoTests:false, allowOnly:false` + registry/coverage mock-real guards + Rust `test_b5_*` eklendi. `summary pdfium/realWorker` counts artık otomatik doğrulanıyor. Registry `totalTargets 27` (FAST_LOCAL 10, NATIVE_PLATFORM 14, EXACT_ARTIFACT 3) korunuyor, `B5` yeni target eklemez.
This document is the single canonical engineering reference for test execution ownership required by 11I-B1.

## 1. Execution Classes (Kanonik Sınır)

### FAST_LOCAL

`FAST_LOCAL` **yalnız** aşağıdaki şartların tümünü sağladığında geçerlidir:

- gerçek PDFium dynamic runtime (`libpdfium.dylib / libpdfium.so / pdfium.dll`) gerektirmez,
- native OS API (macOS Security, Windows Win32 filesystem ACL, Linux-specific) gerektirmez,
- package artifact (`.app / .dmg / .deb / .msi`) gerektirmez,
- network / harici service gerektirmez,
- deterministik local repository state (`cargo test --offline`, `vitest run` jsdom) ile çalışır.

`FAST_LOCAL` *"macOS'ta şu an çalışıyor"* demek değildir; platform-independent local architecture anlamına gelir.
Sınıflandırma kanıtı: kaynakta `get_pdfium`/`with_pdfium` çağrısı **yok**, `WorkerLauncher::for_test(... "--run-worker")` **yok**, `CARGO_BIN_EXE` **yok** (veya yalnız fake helper).

Validator invariant: `executionClass == FAST_LOCAL` ise `usesPdfium == false`, `artifactRequirement == null`, `nativeRequirements` içinde `pdfium_dynamic_lib` / `pdfium_worker_executable` / `aarch64` / `job_worker_executable` bulunamaz. Bulunursa fail-closed.

### NATIVE_PLATFORM

Aşağıdakilerden biri gerekiyorsa **NATIVE_PLATFORM**'tur:

- gerçek PDFium runtime (`pdf_engine::get_pdfium()` `src-tauri/src/pdf_engine/mod.rs:27` -> `Pdfium::bind_to_library`),
- gerçek worker + PDFium (`preview-alternative --run-worker`, `--run-job-worker`),
- platform-specific filesystem/runtime davranışı (`#[cfg(unix)]` symlink, macOS code-sign).

Test kodunun cross-platform olması onu FAST_LOCAL yapmaz. Ör: `document_io_pdfium.rs` kaynakta portable görünür ama `get_pdfium().unwrap()` nedeniyle NATIVE'dir.

### EXACT_ARTIFACT

Yalnız source tree/build output değil **gerçek release artifact** gerektiriyorsa `EXACT_ARTIFACT` olur. Ör: installed `.app/.dmg`, `.deb`, `.msi`, packaged runtime resolution, post-install launch. Şu anda repository'de **0** EXACT_ARTIFACT target vardır (`test-execution-registry.json: summary.exactArtifactRequired = 0`); gelecek `11V-E` için `artifactRequirement: { kind: "dmg"|"deb"|"msi" }` taşır. Validator `EXACT_ARTIFACT` için `artifactRequirement != null` ister.

## 2. Requirement Flags ExecutionClass Yerine Geçmez (§4)

Ayrı metadata boyutları:

```
usesPdfium: bool               // gerçek PDFium dynamic runtime gereksinimi (import değil)
realWorker: bool               // gerçek production worker executable + gerçek backend
supportedPlatforms: ["any"|"macos"|"linux"|"windows"]
nativeRequirements: string[]   // bounded: pdfium_dynamic_lib, pdfium_worker_executable, aarch64...
artifactRequirement: null|{ kind }
```

Örnek geçerli kombinasyon:

```
executionClass = NATIVE_PLATFORM
usesPdfium = true
realWorker = false    // ör: redaction_fixtures_and_capabilities (PDFium var, worker yok)
```

```
executionClass = FAST_LOCAL
usesPdfium = false
realWorker = false    // worker_broker_process fake helper, FAST_LOCAL kalır
```

## 3. Fake Worker / Real Worker Ayrımı (§5)

**Kanonik kural:**

- Mock/fake subprocess testleri: `realWorker = false`. Ör: `worker_broker_process.rs:11` `CARGO_BIN_EXE_pdfium-worker-test-helper` (mode `wrong-id-hello`, `hang-hello` vb), `plugin_worker_supervisor.rs:23` `CARGO_BIN_EXE_plugin-worker-test-helper` adversarial helper. Bunlar **worker integration fully covered** evidence olarak gösterilemez.
- Gerçek production worker executable + gerçek backend kullanan testler: `realWorker = true`. Ör: `crash_tests.rs:12` `WorkerLauncher::for_test(... "--run-worker")`, `session_worker_integration.rs:31`, `job_worker_process.rs:44` `preview-alternative --run-job-worker`, `worker_pdfium_integration.rs:14`.

Plugin worker (`--run-plugin-worker`) Rhai sandbox'tur; registry'de PDFium worker ayrımını korumak için `realWorker=false` olarak sınıflanmıştır (PDFium worker değil). Detay `test-execution-registry.json` notes alanlarında.

## 4. PDFium Gereksinimini Doğru Sınıflandır (§6)

`PDFium source symbols referenced` ≠ `real PDFium dynamic runtime required`.

- Source sembol: `use pdfium_render::prelude::*`, `use pdfium_render::prelude::Pdfium` tipi. Ör: `document_io_fs.rs:8` import var ama `mod pdf_engine::with_pdfium` stub `Err` döndürür — runtime yok, sembol var → `usesPdfium = false`.
- Runtime: `get_pdfium().unwrap()`, `with_pdfium(|pdfium| ...)`, `pdfium.load_pdf_from_file`. Ör: `document_io_pdfium.rs:29`, `redaction_fixtures_and_capabilities.rs:14`, `secure_redaction_raster_primitives.rs:21`, `secure_redaction_planning_worker.rs:2` `inspect_and_plan_secure_redaction` → `usesPdfium = true` (B2 reclassified).

Registry `usesPdfium` yalnız runtime'ı işaretler; import yalnız kanıt değildir. Tahmin yasak.

## 5. Platform Modeli Bounded (§7)

Vocabulary kesin sınırlı: `any`, `macos`, `linux`, `windows`. Distro varyantı yok. Arch requirement gerekiyorsa `nativeRequirements` içinde taşınır (ör: `aarch64` için `worker_pdfium_integration`, `crash_tests`). `supportedPlatforms` hangi OS'te koşması beklendiğini söyler; `nativeRequirements` içindeki `aarch64` ek kısıttır. Mevcut `platform-support.json` `not_evaluated` olduğundan `macos` arm64 dışındaki NATIVE iddiaları fail-closed kalır.

## 6. Canonical Fixture IDs (§8)

Registry `fixtureIds` yalnız `src-tauri/tests/fixtures/corpus-manifest.json` içindeki kanonik `id` değerlerini taşır. Feature manifest local basename'leri, temp/synthetic file'lar canonical listeye girmez; synthetic için ayrı `syntheticInputs` alanı kullanılır (ör: `synthetic_tempdir`, `synthetic_lopdf_document`, `synthetic_type3_unicode_lopdf`). Validator her `fixtureId` için `corpus-manifest.json` içinde varlık kontrolü yapar; yoksa fail.

## 7. Registry Validation (§9) — B5 Genişletmesi

Fail-closed validatorlar:

- Rust: `src-tauri/tests/test_execution_registry_integrity.rs` (`cargo test --test test_execution_registry_integrity`)
- Node: `scripts/validate-test-execution-registry.mjs` (`node scripts/validate-test-execution-registry.mjs`, stdlib only)
- B5: `scripts/verify-test-evidence.mjs` (`node scripts/verify-test-evidence.mjs`, stdlib only, execution-class aware)
- B5: `scripts/validate-pdf-corpus-coverage.mjs` (mock/real mismatch guard)

Invariantlar (B1/B2 + B5):

- duplicate test `id` yok
- duplicate `target` identity ambiguity yok
- unknown `executionClass` yok
- unknown `platform` yok
- canonical olmayan `fixtureId` yok
- nonexistent `target`'a referans yok
- invalid repo-relative path (lexical + symlink escape) yok
- traversal yok
- impossible `FAST_LOCAL` + native-only (`usesPdfium`, `pdfium_dynamic_lib`, `aarch64`...) kombinasyon yok
- `EXACT_ARTIFACT` `artifactRequirement` taşır
- `FAST_LOCAL` native-only requirement taşımaz
- `summary` counts gerçek `targets` uzunluğu ve `usesPdfium`/`realWorker` sayılarıyla eşleşir
- (B5) `conditional` presence `default` profil içermeli, tüm profiller `0` olamaz (`ZERO_TESTS`/`CFG_ELIDED` hardening)
- (B5) `realWorker=true` ⇒ `NATIVE_PLATFORM` veya `EXACT_ARTIFACT` (`MOCK_REALITY_MISMATCH` hardening; artifact smoke paketlenmiş binary üzerinden production `--run-worker` kullanır)
- (B5) coverage `worker` dimension yalnızca `realWorker=true` evidence ile tatmin edilebilir (fake/mocked terfisi fail-closed)
- (B5) `NATIVE_PLATFORM` cfg-zero (`default:0`) `NOT_APPLICABLE`/`RUNTIME_UNAVAILABLE`, asla `PASS` değil
- (B5) `IGNORED_TEST` / `describe.skip` / `#[ignore]` `PASS` sayılmaz — ayrı `IGNORED_TEST` evidence sayılır
- (B5) `FILTER_NO_MATCH` (cargo `running 0 tests` / vitest `No test files`) → `FILTER_NO_MATCH` fail-closed, Exit 0 değil
- (B5) `SKIP` tek başına belirsiz — `PASS|FAIL|NOT_RUN|NOT_APPLICABLE|RUNTIME_UNAVAILABLE|TOOL_UNAVAILABLE|DEPENDENCY_UNAVAILABLE` kullanılır

Yeni dependency eklenmez; mevcut Rust/Node infrastructure kullanılır.

## 8. Coverage Modeliyle Drift Önleme (§10)

| Belge | Owner | Tanım |
|---|---|---|
| `docs/engineering/test-execution-registry.json` | **Testin nasıl/nerede/hangi runtime requirement ile çalıştığı** | executionClass, usesPdfium, realWorker, supportedPlatforms, nativeRequirements |
| `docs/engineering/pdf-corpus-coverage.json` | **Feature → fixture → test evidence ilişkisi** | traceabilityEdges, features[], gaps[] |

Aynı metadata iki dosyada tekrar etmez. Çakışan tek alan `fixtureId` ve `testId` stable reference'dır; registry owner execution attachment, coverage owner feature graph. Birinden diğerine stable ID reference kullanılır.

## 9. Zero-Test Riski (§12) — B5 Enforcement

Her registry entry `expectedTestCount` ve `expectedTestPresence` taşır:

- `expectedTestCount`: uygun platform/feature profilinde gözlemlenmesi gereken `#[test]` sayısı (örn `crash_tests:3` conditional, `worker_broker_process:19`)
- `expectedTestPresence`: `exact` (mutlaka bu sayı), `at_least_one`, `conditional` (profil bazlı `expectedTestCountByProfile` ile birlikte — ör: `plugin_worker_supervisor` default 1 vs `--features plugin-test-support` 13, `sanitization_inspection_acceptance` default 0 vs `macos_aarch64+feature` 10, `crash_tests` default 0 vs `on_macos_aarch64:3` — **B5 düzeltmesi**)

Amaç: `cargo test --test foo` typo/filter/cfg nedeniyle **0 test çalıştırıp exit 0** dönerse B5'te platform gate tarafından yakalanabilmesi. B1 bunun metadata temelini kurar; tam enforcement B5'te.

**B5 enforcement:**
- `cargo test --test foo` → `running 0 tests` + `expected>0` ⇒ `ZERO_TESTS` fail-closed ( `verify-test-evidence.mjs` + wrapper script contract). Profile-aware: `default` 0 olan `NATIVE_PLATFORM` target off-host `NOT_APPLICABLE` sayılır, current host `PASS` değil.
- `cargo test foo -- <filter>` / `vitest run <pattern>` → filter hiçbir test bulamazsa `FILTER_NO_MATCH` fail-closed (B5 wrapper `verify-test-evidence` `--cargo-json` / vitest `passWithNoTests:false`).
- `#![cfg(feature="...")]` / `#[cfg(target_os="macos")]` ile compile edilmeyen testler `CFG_ELIDED` — single `PASS` değil, `NOT_APPLICABLE_ON_THIS_HOST` veya registry profile karşılığı.
- `#[ignore]` / `describe.skip` / `it.skip` (örn `CommandPalette.component.test.js:8` `hasClientMount ? describe : describe.skip`) → `IGNORED_TEST` — `PASS` sayılmaz, `reason/owner/executionClass/requiredStage` ile raporlanır, validator `skippedCount` evidence döner.

## 10. Registry Ownership & CI İlişkisi

- Owner: `docs/engineering/test-execution-registry.json` (machine-readable canonical).
- Human-readable view: bu belge.
- Validatorlar CI `11I-F` fazına bağlanır; henüz GitHub Actions implement edilmez (B1 boundary).
- `11V-A` fast local gate: `FAST_LOCAL` target'ları temiz checkout'ta offline koşar.
- `11V-B/C/D` native gates: `NATIVE_PLATFORM` target'ları ilgili runner'da (macos arm64 / linux x64 / windows x64) koşar, registry `supportedPlatforms` ile filtrelenir.
- `11V-E` exact artifact gate: `EXACT_ARTIFACT` target'ları `artifactRequirement` ile eşleşen pakete karşı koşar.

## 11. STRICT_OFFLINE Doğrulama (B1 Boundary §14-15) — B2/B5 genişletmesi

B1 scoped checks + B2 FAST_LOCAL izolasyon smoke + B5 fail-closed validation:

```bash
node scripts/validate-test-execution-registry.mjs
node scripts/validate-pdf-corpus-coverage.mjs
node scripts/verify-test-evidence.mjs
python3 -c "import json; json.load(open('docs/engineering/test-execution-registry.json'))"
CARGO_NET_OFFLINE=true cargo test --locked --offline --test test_execution_registry_integrity
CARGO_NET_OFFLINE=true cargo test --locked --offline --test fixture_manifest_integrity
CARGO_NET_OFFLINE=true cargo test --locked --offline --test pdf_corpus_coverage
CARGO_NET_OFFLINE=true cargo test --locked --offline --test document_io_fs
CARGO_NET_OFFLINE=true cargo test --locked --offline --test signature_surface
CARGO_NET_OFFLINE=true cargo test --locked --offline --test worker_broker_process
CARGO_NET_OFFLINE=true cargo test --locked --offline --test secure_redaction_raster_pure
```

Full `cargo test`, real PDFium integration suite, Tauri build, Linux/Windows matrix, package build, artifact smoke, GitHub Actions B1/B2/B5'de çalıştırılmaz. B2’de `secure_redaction_planning_worker` ve `secure_redaction_raster_primitives` (40) NATIVE’e taşındığı için FAST smoke’a dahil değildir; `libpdfium.dylib` yoksa FAIL beklenir (skip false-green değil).

**B5 static evidence (offline, no native runtime):**

```bash
node scripts/verify-test-evidence.mjs
# optional runtime evidence (when cargo/vitest json captured):
# node scripts/verify-test-evidence.mjs --cargo-json /tmp/cargo-evidence.json --vitest-json /tmp/vitest.json
```

B5 `evidence statuses` vocabulary: `PASS | FAIL | NOT_RUN | NOT_APPLICABLE | RUNTIME_UNAVAILABLE | TOOL_UNAVAILABLE | DEPENDENCY_UNAVAILABLE` ( `SKIP` tek başına yasak §13). `TOOL_UNAVAILABLE_OFFLINE` (reportlab/pypdf install yok) ve `DEPENDENCY_UNAVAILABLE_OFFLINE` (cargo cache miss) ayrı status, uygulama regression değil, ama `PASS` de değil. `RUNTIME_UNAVAILABLE` (pdfium yok) `PASS` değil; off-host `crash_tests` `NOT_APPLICABLE`.

## 14. B2 FAST_LOCAL İzolasyonu — Reclassification & Split Detayı

- **Reclassified to NATIVE_PLATFORM:** `secure_redaction_planning_worker` (28 tests) — `src-tauri/tests/secure_redaction_planning_worker.rs:1-755` `inspect_and_plan_secure_redaction` → `src-tauri/src/secure_redaction_inspection.rs:51` `get_pdfium()` zinciri. `registry.json` `usesPdfium false→true`, `nativeRequirements []→["pdfium_dynamic_lib"]`, `supportedPlatforms ["any"]→["macos"]`, `expectedTestPresence exact→at_least_one`. Pure `build_secure_redaction_plan` `src-tauri/src/secure_redaction.rs:4043` zaten lib unit-testlerinde FAST; inspection wrapper doğal NATIVE.
- **Split:** `secure_redaction_raster_primitives` 68 → `secure_redaction_raster_pure.rs` FAST 28 (`calculate_raster_dimensions` `test_01-11`, `merge_device_pixel_rects` `test_26-31`, `apply_opaque_black_overwrite` `test_32-37,42,44`, `LosslessRasterPayload` `test_40`, `merge`/`stride` `test_57-58`) `usesPdfium=false` + `secure_redaction_raster_primitives.rs` NATIVE 40 (`with_test_pdfium`, `render_affected_page_raster`, `map_pdf_rect_to_device_pixel_rect`, `build_lossless_raster_payload_for_page`). Helper `make_mapping` ve `sample_geometry` yalnızca pure’da, `with_test_pdfium`/`fixture_path` yalnız native’de. Coverage `pdf-corpus-coverage.json` native edge’ler `secure_redaction_raster_primitives`’te kalır (pure’un edge’i yok).
- **Korunanlar:** `document_io_fs` stub izolasyonu `src-tauri/tests/document_io_fs.rs:7-20` (derlenen izole `document_io` modülü 25 birim test + 1 tip kontrolü ile 26 test olarak senkronize edildi), `test_execution_registry_integrity` (6 test) FAST_LOCAL tescili yapıldı, `worker_broker_process` fake helper `src-tauri/src/bin/pdfium-worker-test-helper.rs`, `plugin_worker_supervisor` feature-gate `src-tauri/tests/plugin_worker_supervisor.rs:1-8` sıfır-test riski kapatılmış, frontend `vitest` jsdom izolasyonu — tümü B1’deki gibi korunuyor.
- **Production değişmedi:** PDFium canonical engine, `lopdf` helper-only, ikinci renderer yok, `Cargo.toml` yeni crate/dependency yok.

## 12. Kısıtlar (§16)

Yeni crate/npm package eklenmez, PDF engine değiştirilmez, fixture bytes değiştirilmez, test silinmez, assertion zayıflatılmaz, runtime absent PASS'a dönüştürülmez, CI workflow değiştirilmez.

## 15. B5 Fail-Closed Taxonomy & Evidence Model (11I-B5 Kanonik) — §2, §13, §20

**Bounded false-green vocabulary (semantik ayrım korunur):**

```
ZERO_TESTS, FILTER_NO_MATCH, CFG_ELIDED, IGNORED_TEST,
RUNTIME_UNAVAILABLE, TOOL_UNAVAILABLE, DEPENDENCY_UNAVAILABLE,
MOCK_REALITY_MISMATCH, AGGREGATE_MIXING, EXPECTED_EVIDENCE_MISSING
```

**Evidence record modeli (machine-readable, §13):**

```
PASS | FAIL | NOT_RUN | NOT_APPLICABLE | RUNTIME_UNAVAILABLE | TOOL_UNAVAILABLE | DEPENDENCY_UNAVAILABLE
```

`SKIP` tek başına yasak. `NOT_APPLICABLE` off-host cfg-elided (örn `crash_tests` default 0, `sanitization_*` default 0) için; `RUNTIME_UNAVAILABLE` pdfium yokken NATIVE test koşulamazsa; `TOOL_UNAVAILABLE_OFFLINE` reportlab/pypdf install eksik; `DEPENDENCY_UNAVAILABLE_OFFLINE` cargo cache eksik. Hepsi fail-closed: hiçbiri `PASS` değil, current evidence değil.

**Invariantlar (otomatik §20, `verify-test-evidence.mjs` + registry validators):**

- registered target exists
- expected test-bearing target zero-test değil (`running 0 tests` → `ZERO_TESTS` fail-closed)
- FAST_LOCAL target native runtime skip ile green olmaz (`FAST_LOCAL` + `usesPdfium`/`pdfium_dynamic_lib` impossible)
- NATIVE_PLATFORM missing runtime current PASS değil (`RUNTIME_UNAVAILABLE` / `NOT_APPLICABLE`)
- fake worker target real-worker evidence üretemez (`worker_broker_process` fake → real `worker_pdfium_integration` yerine geçemez)
- mocked frontend (`vi.mock("@tauri-apps/api/core")`) real backend integration evidence değil
- ignored test current PASS evidence değil (`describe.skip` hasClientMount tek başına 1 skip, `PASS` sayılmaz)
- unknown execution result yok
- coverage current PASS valid evidence source'a dayanır (historical `pass` → `not_evaluated` → `verified` geçiş without `currentBaselineEvidence` yasak)
- aggregate suite (`cargo test --lib`, `npm run test` 115) Exit 0 hangi layer geçtiğini gizlemeyecek — per-target evidence + execution-class aware reporting zorunlu
- historical PASS current PASS’a taşınmaz (`platform-support.json` `historicalEvidence.currentBaselineEvidence=false` invariant)

**Test invocation wrappers (§19):** Mevcut script/helper güçlendirildi (`validate-*`, `verify-test-evidence`); yeni wrapper `verify-test-evidence.mjs` küçük, dependency-free, deterministic, execution-class aware. “Test runner içinde test runner” framework yok.

**Node/Python/Vitest fail-closed (§16-18):** `scripts/*.mjs` `catch { console.error }` sonrası implicit Exit 0 yasak → `process.exit(1)`. Python `sys.exit(0 if ok else 1)` korunuyor, yeni generator bu contract’ı korumalı. Vitest `passWithNoTests:false` + `allowOnly:false` → `No test files matched` / `.only` kalıntısı Exit 0 değil.

**Filter no-match (§5):** `cargo test <filter>` veya Vitest pattern hiçbir test bulmadan Exit 0 dönerse `FILTER_NO_MATCH` wrapper/validator tarafından fail-closed. Test framework’ü yeniden yazılmaz, command-specific evidence assertion yapılır.

**Cargo target existence vs execution (§4):** `compiled / launched / discovered / executed / passed` ayrı; `Finished test profile` + `Running tests/foo.rs` + `running 0 tests` + `test result: ok` overall PASS değil eğer target test çalıştırması bekleniyorsa.

**Native test davranışı (§22):** Native target local runtime olmadan çalışmıyorsa beklendik. B5’in işi: `çalışmadı ama PASS göründü` engellemek, her host’ta çalıştırmak değil.

## 13. Kabul Kriterleri Referansı

B1 COMPLETE ancak: envanter kaynak üzerinden yeniden doğrulandı, sınıflar kanonik, PDFium ayrımı modellendi, fake/real worker ayrık, fixture linkage doğru (`corpus-manifest.json`), registry validate ediliyor, coverage ile drift yok, zero-test foundation mevcut, STRICT_OFFLINE korundu.

Final rapor formatı §17'ye uygun `11I-B1 IMPLEMENTATION: ...` şeklinde `node`+`cargo` doğrulamaları sonrası üretilir.
