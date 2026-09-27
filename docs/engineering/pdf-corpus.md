# Canonical PDF Fixture Corpus Architecture & Migration Design

## 1. Executive Summary

This document specifies the canonical PDF fixture corpus architecture, schema definition, generator reproducibility classification, and security integrity gating for the Antigravity PDF Reader application (Prompt 11I-A4 & Prompt 11I-A5.1).

The repository maintains an active corpus of **56 unique committed PDF fixture files** (56 distinct SHA-256 digests) distributed across unit tests, integration tests, fuzzing targets, smoke tests, and security verifiers.

The architecture enforces a centralized, strictly-typed canonical manifest schema (`corpus-manifest.json`), an automated Rust integrity and invariant verification suite (`fixture_manifest_integrity.rs`), a pure-Python deterministic generator (`generate_release_smoke.py`), a canonical Feature → Fixture → Test coverage matrix (`docs/engineering/pdf-corpus-coverage.json`), and dedicated dual validators (Rust test suite and Node ESM validator `scripts/validate-pdf-corpus-coverage.mjs`).

---

## 2. Repository Corpus Inventory Summary

| Subdirectory | Actual PDF Count | Unique SHA-256 | Source Type Distribution | Provenance Status | License Status |
|---|---|---|---|---|---|
| `src-tauri/tests/fixtures/pdf/` | 13 | 13 | 7 `deterministically_generated`, 1 `generated`, 3 `project_authored`, 2 `unknown` | 11 `complete`, 2 `partial` | 11 `repository_license_reference`, 2 `missing` |
| `src-tauri/tests/fixtures/redaction/` | 26 | 26 | 26 `deterministically_generated` | 26 `complete` | 26 `missing` |
| `src-tauri/tests/fixtures/sanitization/` | 16 | 16 | 16 `deterministically_generated` | 16 `complete` | 16 `missing` |
| `src-tauri/fuzz/corpus/` | 1 | 1 | 1 `fuzz_seed` | 1 `missing` | 1 `missing` |
| **TOTAL** | **56** | **56** | **49 `det_gen`, 1 `gen`, 3 `proj_auth`, 1 `fuzz_seed`, 2 `unknown`** | **53 `complete`, 2 `partial`, 1 `missing`** | **11 `repo_ref`, 45 `missing`** |

* **Traceability Invariant:** Fixture consumption is governed strictly by verifiable source code inspection edges in `traceabilityEdges`. Fixtures with current active test consumers are linked directly to features; fixtures without current test consumers remain strictly in `unconsumedFixtureIds` without fabricated links.
* **AppleDouble Artifacts (`._*.pdf`):** macOS filesystem metadata sidecar files (`._*.pdf`) are classified as non-corpus filesystem artifacts. They are deterministically skipped by the corpus filesystem scanner without panicking, have zero impact on cardinality and parity, and are strictly rejected if referenced in canonical manifest paths.

---

## 3. Canonical Manifest Schema Specification

The canonical manifest file is located at `src-tauri/tests/fixtures/corpus-manifest.json`.

### 3.1 JSON Wire Representation

```json
{
  "schemaVersion": "1.0.0",
  "corpusVersion": "1.0.0",
  "description": "Canonical PDF Fixture Corpus Manifest for Antigravity PDF Reader",
  "generators": {
    "py-reorder-generator": {
      "kind": "committed_script",
      "path": "src-tauri/tests/fixtures/pdf/generate_reorder_fixtures.py",
      "language": "python3",
      "dependencies": [
        "reportlab",
        "pypdf"
      ],
      "networkRequirement": "none",
      "reproducibilityStatus": "toolchain_unpinned",
      "notes": "Deterministik canvas amaci (invariant=1) kaynak kod tarafindan desteklenmektedir, ancak paket surumleri repo lockfile ile sabitlenmemistir."
    },
    "py-redaction-generator": {
      "kind": "committed_script",
      "path": "src-tauri/tests/fixtures/redaction/generate_redaction_fixtures.py",
      "language": "python3",
      "dependencies": [],
      "networkRequirement": "none",
      "reproducibilityStatus": "deterministic_by_construction",
      "notes": "Yalnizca Python standart kutuphanesini kullanir. Sabit bayt ofsetleri ve tuzlari ile deterministiktir."
    },
    "py-sanitization-generator": {
      "kind": "committed_script",
      "path": "src-tauri/tests/fixtures/sanitization/generate_sanitization_fixtures.py",
      "language": "python3",
      "dependencies": [],
      "networkRequirement": "none",
      "reproducibilityStatus": "deterministic_by_construction",
      "notes": "Yalnizca Python standart kutuphanesini kullanir. Sabit FIXED_ID ile deterministiktir."
    },
    "py-release-smoke-generator": {
      "kind": "committed_script",
      "path": "src-tauri/tests/fixtures/pdf/generate_release_smoke.py",
      "language": "python3",
      "dependencies": [],
      "networkRequirement": "none",
      "reproducibilityStatus": "deterministic_by_construction",
      "notes": "Pure Python standard library generator for release-smoke.pdf and large-page-dimensions.pdf fixtures."
    },
    "pypdf-historical-method": {
      "kind": "historical_documented_method",
      "path": null,
      "language": "python3 / pypdf 6.10.0",
      "dependencies": [
        "pypdf 6.10.0"
      ],
      "networkRequirement": "none",
      "reproducibilityStatus": "reproducibility_unverified",
      "notes": "minimal.pdf uzerinden pypdf 6.10.0 ile uretilmistir; bayt-bayt determinizmi dogrulanmamistir."
    }
  },
  "fixtures": []
}
```

### 3.2 Rust Serde Type Model

All schema structures enforce `#[serde(rename_all = "camelCase", deny_unknown_fields)]` and enums enforce `#[serde(rename_all = "snake_case")]`:

```rust
pub struct PdfFixtureManifest {
    pub schema_version: String,
    pub corpus_version: String,
    pub description: String,
    pub generators: HashMap<String, GeneratorRecord>,
    pub fixtures: Vec<PdfFixtureRecord>,
}

pub struct GeneratorRecord {
    pub kind: GeneratorKind,
    pub path: Option<String>,
    pub language: String,
    pub dependencies: Vec<String>,
    pub network_requirement: NetworkRequirement,
    pub reproducibility_status: ReproducibilityStatus,
    pub notes: Option<String>,
}

pub enum GeneratorKind {
    CommittedScript,
    HistoricalDocumentedMethod,
}

pub enum NetworkRequirement {
    None,
    BuildTimeOnly,
    RuntimeOptional,
}

pub enum ReproducibilityStatus {
    DeterministicByConstruction,
    ReproducibilityVerified,
    ToolchainUnpinned,
    ReproducibilityUnverified,
}

pub struct PdfFixtureRecord {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub byte_length: u64,
    pub pdf_version: String,
    pub license: Option<String>,
    pub license_status: LicenseStatus,
    pub copyright: Option<String>,
    pub source_type: SourceType,
    pub source: Option<String>,
    pub provenance_status: ProvenanceStatus,
    pub generator_id: Option<String>,
    pub generated_from: Option<String>,
    pub encrypted: bool,
    pub password_reference: Option<String>,
    pub categories: Vec<FixtureCategory>,
    pub expected_capabilities: Vec<String>,
    pub expected_findings: Vec<String>,
    pub allowed_tests: Vec<TestKind>,
    pub feature_manifest: Option<String>,
    pub notes: Option<String>,
    pub allow_empty: bool,
}

pub enum SourceType {
    ProjectAuthored,
    Generated,
    DeterministicallyGenerated,
    ThirdPartyRedistributable,
    ExternalTestOnly,
    FuzzSeed,
    FuzzRegression,
    Unknown,
}

pub enum ProvenanceStatus {
    Complete,
    Partial,
    Missing,
    NotApplicable,
}

pub enum LicenseStatus {
    Known,
    RepositoryLicenseReference,
    Missing,
    NotApplicable,
}

pub enum FixtureCategory {
    Core,
    Baseline,
    DocumentStructure,
    Mutation,
    Security,
    Accessibility,
    Print,
    Malformed,
    Scanned,
    Encrypted,
    ReleaseSmoke,
    Redaction,
    Sanitization,
    Prepress,
    Ai,
    Fuzz,
    Minimal,
    Reorder,
    Annotations,
}

pub enum TestKind {
    Unit,
    Integration,
    Acceptance,
    Fuzz,
    Smoke,
}
```

---

## 4. Generator Classification & Reproducibility Analysis

1. **`py-reorder-generator`:**
   - **Kind:** `committed_script`
   - **Path:** `"src-tauri/tests/fixtures/pdf/generate_reorder_fixtures.py"`
   - **Dependencies:** `reportlab`, `pypdf`
   - **Reproducibility Status:** `toolchain_unpinned`. The script passes `invariant=1` to ReportLab canvas to produce deterministic PDF timestamps and metadata, but external pip dependencies are not pinned via a repository lockfile.
2. **`py-redaction-generator`:**
   - **Kind:** `committed_script`
   - **Path:** `"src-tauri/tests/fixtures/redaction/generate_redaction_fixtures.py"`
   - **Dependencies:** None (Python 3 standard library only).
   - **Reproducibility Status:** `deterministic_by_construction`. Calculates exact byte `/Length`, fixed RC4 salts, and fixed PDF trailer IDs (`FIXED_FILE_ID_16`).
3. **`py-sanitization-generator`:**
   - **Kind:** `committed_script`
   - **Path:** `"src-tauri/tests/fixtures/sanitization/generate_sanitization_fixtures.py"`
   - **Dependencies:** None (Python 3 standard library only).
   - **Reproducibility Status:** `deterministic_by_construction`. Emits fixed `FIXED_ID = b"SANITIZE-FIX-ID"` and deterministic object graphs.
4. **`py-release-smoke-generator`:**
   - **Kind:** `committed_script`
   - **Path:** `"src-tauri/tests/fixtures/pdf/generate_release_smoke.py"`
   - **Dependencies:** None (Python 3 standard library only).
   - **Reproducibility Status:** `deterministic_by_construction`. Constructs pure Python byte-level PDF 1.4 objects, Type 3 Turkish Unicode font CharProcs, and exact xref tables for `release-smoke.pdf` and `large-page-dimensions.pdf`. Provides `--verify-determinism` mode.
5. **`pypdf-historical-method`:**
   - **Kind:** `historical_documented_method`
   - **Path:** `null` (No committed generator script).
   - **Dependencies:** `pypdf 6.10.0`
   - **Reproducibility Status:** `reproducibility_unverified`. Historical one-off creation method for `pdf/encrypted.pdf` documented in `pdf/README.md`.

---

## 5. Cross-Field Validation Invariants & Security Rules

The integrity test suite (`src-tauri/tests/fixture_manifest_integrity.rs`) enforces strict fail-closed validation rules:

### 5.1 Exact Supported Schema Version
- `validate_manifest()` strictly enforces `schemaVersion == "1.0.0"`.
- Any unknown, future, or empty schema version (e.g. `"0.9.0"`, `"1.0.1"`, `"2.0.0"`, `"future"`) results in a validation failure.

### 5.2 Fixture and Generator ID Format (`validate_kebab_case_id`)
- All fixture IDs, generator IDs in `manifest.generators`, and fixture `generatorId` references must strictly match:
  ```regex
  ^[a-z0-9]+(?:-[a-z0-9]+)*$
  ```
- Rejects uppercase, underscores, spaces, leading/trailing hyphens, consecutive hyphens, and path traversal strings.
- Duplicate fixture IDs and duplicate fixture paths are rejected.
- A non-null fixture `generatorId` must both satisfy the syntax and reference a defined key in `manifest.generators`.

### 5.3 Canonical SHA-256 Syntax (`validate_sha256_format`)
- Every fixture `sha256` must be exactly 64 lowercase hexadecimal characters matching:
  ```regex
  ^[a-f0-9]{64}$
  ```
- This syntax validation runs unconditionally even when `check_disk == false`.
- Disk hash comparison remains an additional invariant when `check_disk == true`.

### 5.4 Common Safe Repo-Relative Path Validator (`validate_safe_repo_relative_path` & `validate_lexical_path`)
- **Phase A (Lexical Validation):**
  - `/` is the **canonical and only permitted path separator**.
  - Backslash `\` is **strictly rejected** as a non-canonical path separator.
  - Leading `/` (absolute path) is rejected.
  - Empty segments and double slashes `//` are rejected.
  - Current directory `.` and parent traversal `..` segments are rejected.
  - AppleDouble sidecar paths (`._*.pdf`) are strictly rejected.
  - Windows drive letters (`C:\`, `C:/`, `d:/`) and UNC paths (`\\server\share`, `//server/share`) are rejected.
  - URI schemes (`file://`, `https://`) are rejected.
- **Phase B (Filesystem Containment):**
  - Resolves against `repo_root`.
  - Verifies canonicalized target path remains strictly within canonical repository root, rejecting any symlink escape.

### 5.5 Disk Verification & Empty Malformed Policy
- When `check_disk == true`, every referenced fixture file must physically exist on disk. A missing file is always a hard validation failure.
- `allowEmpty == true` permits an existing physical file to have 0 bytes only under the explicit `Malformed` category exception:
  ```text
  byteLength == 0 OR allowEmpty == true
      -> FixtureCategory::Malformed must be present
      -> expectedFindings must contain an explicit finding code (e.g. malformed_empty_file)
  ```
- Non-malformed fixtures must never set `allowEmpty == true` and must have `byteLength > 0`.
- Orthogonal categories (e.g., `Core`, `Security`) co-existing with `Malformed` are fully permitted.

### 5.6 Provenance & SourceType Invariants
- `sourceType == DeterministicallyGenerated`: Must specify `generatorId` referencing a generator in `manifest.generators` whose `kind` is `CommittedScript`.
- `sourceType == Generated`: Must specify at least one of `generatorId` or `source`.
- `sourceType == ProjectAuthored`: Must have `provenanceStatus == Complete` and a non-empty `source` description.
- `sourceType == Unknown`: `provenanceStatus` cannot be `Complete` (must be `Partial` or `Missing`).
- `provenanceStatus == Complete`: `source` description is required.

### 5.7 License Invariants
- `licenseStatus == Known`: `license` string (e.g. SPDX identifier) is mandatory.
- `licenseStatus == RepositoryLicenseReference`: `license` must be `None` (documents association with repo license without fabricated SPDX).
- `licenseStatus == Missing` / `NotApplicable`: `license` must be `None` (fabrication of synthetic license strings is strictly forbidden).

### 5.8 Expected Capabilities Bounded Allowlist & Uniqueness
- All strings in `expectedCapabilities` must match `^[a-z0-9]+(?:_[a-z0-9]+)*$` and belong to the canonical bounded allowlist:
  ```rust
  pub const CANONICAL_EXPECTED_CAPABILITIES: &[&str] = &[
      "has_text",
      "has_unicode_text",
      "has_vector_content",
      "multi_page",
      "large_page_dimensions",
  ];
  ```
- Any token outside this closed vocabulary is a hard validation failure.
- Duplicate tokens within `expectedCapabilities` are rejected.
- Duplicate tokens within `expectedFindings`, duplicate categories within `categories`, and duplicate entries within `allowedTests` are strictly rejected.

### 5.9 Password Reference Security
- `passwordReference` must strictly match `^test-credential:[a-z0-9-]+$` or be `null`.
- Raw plaintext passwords (e.g. `"password123"`, `"secret"`) are strictly forbidden.
- If `encrypted == true`, `FixtureCategory::Encrypted` must be present in `categories`. If `encrypted == false`, `passwordReference` must be `None`.

### 5.10 Canonical Invariants for `core-release-smoke` and `core-large-page-dimensions`
- **`core-release-smoke`:**
  - Path: `src-tauri/tests/fixtures/pdf/release-smoke.pdf`
  - Size: 4520 bytes, SHA-256: `727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611`
  - Generator: `py-release-smoke-generator` (`deterministically_generated`)
  - Categories: `core`, `release_smoke`
  - Capabilities: `has_text`, `has_unicode_text`, `has_vector_content`, `multi_page`
- **`core-large-page-dimensions`:**
  - Path: `src-tauri/tests/fixtures/pdf/large-page-dimensions.pdf`
  - Size: 680 bytes, SHA-256: `93cb5f67e9587658f60a1db12a5f29aecf29d6b8ec47403400d9a8f48406fa55`
  - Generator: `py-release-smoke-generator` (`deterministically_generated`)
  - Categories: `core`, `document_structure`
  - Capabilities: `large_page_dimensions`

---

## 6. Model B Manifest Coexistence & Migration Design

1. **Phase A1 (Manifest & Engine Foundation):**
   - Central manifest schema and automated Rust integrity verifier.
2. **Phase A2 (Corpus Registration & Parity Bridge):**
   - Full 54-fixture registration, bidirectional parity with feature manifests (`redaction/manifest.json`, `sanitization/manifest.json`).
3. **Phase A3 (Coverage & Determinism Engine):**
   - Introduction of pure-Python deterministic generator `generate_release_smoke.py`, canonical `release-smoke.pdf` and `large-page-dimensions.pdf` fixtures (expanding corpus to 56 fixtures).
4. **Phase A4 (Corpus Integrity & Security Gate):**
   - Hardened lexical path validation (`/` canonical, `\` rejected, AppleDouble sidecar exclusion).
   - Bounded allowlist for `expectedCapabilities`.
   - Comprehensive adversarial test matrix.
5. **Phase A5.1 (Canonical Coverage Model Correctness & Traceability Matrix):**
   - Single source of truth: [`docs/engineering/pdf-corpus-coverage.json`](file:///Users/umut/antigravity-proje/docs/engineering/pdf-corpus-coverage.json).
   - Direct `Feature → Fixture → Test` canonical traceability graph (`traceabilityEdges`).
   - Exact-set equality enforcement for `unconsumedFixtureIds` and feature-level summary sets.
   - Authoritative immediate roadmap owner invariants (`plannedPrompt`).

---

## 7. 56-Fixture Complete Catalog & Consumer Graph

| # | Relative Path (`path`) | Bytes | SHA-256 | Encrypted | Source Type | Generator ID | Provenance | License | Categories | Consumer Graph (Modules / Tests / Docs) |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `src-tauri/fuzz/corpus/minimal.pdf` | 328 | `9d8c1d4382b8d77da0504aeca4a3ff39bdf8641186b1d3ff2d5d87013a00d484` | No | `fuzz_seed` | `-` | `missing` | `missing` | `fuzz, minimal` | `fuzz/fuzz_targets/pdf_parser.rs` |
| 2 | `src-tauri/tests/fixtures/pdf/ai-text-rich.pdf` | 2793 | `9f858642202b10dd6dbb5fd6a642ddb7762667f043aa16c635cd46615cd3893d` | No | `unknown` | `-` | `partial` | `missing` | `ai, core` | `src/ai/security_acceptance_tests.rs` |
| 3 | `src-tauri/tests/fixtures/pdf/annotations.pdf` | 470 | `43e4cbd366712cee2290f1de48c1cb1a476bed120e18bd074b62f418e9b5deda` | No | `project_authored` | `-` | `complete` | `repository_license_reference` | `core, annotations` | `src/annotations.rs`, `tests/fixtures/pdf/README.md` |
| 4 | `src-tauri/tests/fixtures/pdf/encrypted.pdf` | 807 | `4a07c1b65793257cf412dc5f478ea84525f07e94705bbbb084bb427cd241f28d` | Yes | `generated` | `pypdf-historical-method` | `complete` | `repository_license_reference` | `core, security, encrypted` | `session_worker_integration.rs`, `worker_pdfium_integration.rs`, `documentOpenCoordinator.test.js`, `documentOpenController.integration.test.js`, `tests/fixtures/pdf/README.md` |
| 5 | `src-tauri/tests/fixtures/pdf/large-page-dimensions.pdf` | 680 | `93cb5f67e9587658f60a1db12a5f29aecf29d6b8ec47403400d9a8f48406fa55` | No | `deterministically_generated` | `py-release-smoke-generator` | `complete` | `repository_license_reference` | `core, document_structure` | `fixture_manifest_integrity.rs`, `generate_release_smoke.py` |
| 6 | `src-tauri/tests/fixtures/pdf/malformed.pdf` | 85 | `8f61651fa71aa15107abeda98ffdc8724e878bb1ca0475bc23042e3ef5cef27e` | No | `unknown` | `-` | `partial` | `missing` | `core, malformed` | `crash_tests.rs`, `worker_pdfium_integration.rs` |
| 7 | `src-tauri/tests/fixtures/pdf/minimal.pdf` | 329 | `0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d` | No | `project_authored` | `-` | `complete` | `repository_license_reference` | `core, baseline` | `src/a11y.rs`, `src/document_session.rs`, `src/manipulation.rs`, `src/redaction.rs`, `src/sanitization_*.rs`, `crash_tests.rs`, `job_worker_process.rs`, `session_worker_integration.rs`, `worker_pdfium_integration.rs`, `docs/engineering/baseline.md` |
| 8 | `src-tauri/tests/fixtures/pdf/release-smoke.pdf` | 4520 | `727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611` | No | `deterministically_generated` | `py-release-smoke-generator` | `complete` | `repository_license_reference` | `core, release_smoke` | `fixture_manifest_integrity.rs`, `generate_release_smoke.py` |
| 9 | `src-tauri/tests/fixtures/pdf/reorder-rich.pdf` | 1923 | `ec45cb69345f177f77c67073b7a5cb988d96276697ef34a35d0ae68eca5e492c` | No | `deterministically_generated` | `py-reorder-generator` | `complete` | `repository_license_reference` | `core, document_structure, reorder` | `src/manipulation.rs`, `src/redaction.rs`, `document_io_pdfium.rs`, `job_worker_process.rs`, `session_worker_integration.rs`, `worker_pdfium_integration.rs` |
| 10 | `src-tauri/tests/fixtures/pdf/reorder-with-form.pdf` | 4285 | `4f4cb184e4cf4f342b83b3675e8360254ed1f366c0bb8834cc5ff085a8ec58cf` | No | `deterministically_generated` | `py-reorder-generator` | `complete` | `repository_license_reference` | `core, document_structure, reorder` | `src/manipulation.rs`, `document_io_pdfium.rs` |
| 11 | `src-tauri/tests/fixtures/pdf/reorder-with-metadata.pdf` | 2004 | `8825c0d1abecd665c1e89f882ad811237cdbadbf7c07e1a9198600754a28af65` | No | `deterministically_generated` | `py-reorder-generator` | `complete` | `repository_license_reference` | `core, document_structure, reorder` | `src/manipulation.rs`, `document_io_pdfium.rs`, `worker_pdfium_integration.rs` |
| 12 | `src-tauri/tests/fixtures/pdf/reorder-with-named-destination.pdf` | 2152 | `241264be3211c3d50376d77f7136fd3cfa1d9f53791a2611a5475c52e667f325` | No | `deterministically_generated` | `py-reorder-generator` | `complete` | `repository_license_reference` | `core, document_structure, reorder` | `src/manipulation.rs`, `document_io_pdfium.rs` |
| 13 | `src-tauri/tests/fixtures/pdf/reorder-with-outline.pdf` | 2189 | `e2dff7f1786fac45ad65fb70bf38e7842fefda6b5550b128e33eef9c96a5fe96` | No | `deterministically_generated` | `py-reorder-generator` | `complete` | `repository_license_reference` | `core, document_structure, reorder` | `src/manipulation.rs`, `document_io_pdfium.rs`, `worker_pdfium_integration.rs` |
| 14 | `src-tauri/tests/fixtures/pdf/three-distinct-pages.pdf` | 829 | `55f8e4d2aedcfc3603c398c648d1a846e6eafe60f26e5c43c7288984f232914a` | No | `project_authored` | `-` | `complete` | `repository_license_reference` | `core, document_structure` | `src/manipulation.rs`, `tests/fixtures/pdf/README.md` |
| 15 | `src-tauri/tests/fixtures/redaction/acroform-widget.pdf` | 804 | `42cadc768e70985c198104b348b854c5b26c49cb23856a3b76d441f0a0faed51` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `secure_redaction_raster_primitives.rs` |
| 16 | `src-tauri/tests/fixtures/redaction/annotations-fixture.pdf` | 903 | `2829659dfb26dd4a68faf80888d91bc7a5d3d33c3a7b5c11a88d88786eaa9176` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `secure_redaction_raster_primitives.rs` |
| 17 | `src-tauri/tests/fixtures/redaction/cropbox-mediabox-diff.pdf` | 624 | `496ec852744fdacb2235cb3da7b4234de5777e945f9561108b13b8630071b031` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `job_worker_process.rs`, `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `secure_redaction_raster_primitives.rs` |
| 18 | `src-tauri/tests/fixtures/redaction/embedded-files.pdf` | 899 | `26c0874f88dcef688c75c83dc4ba0c1ffa03b309c3c9ad6cb5e2bbff52515470` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `secure_redaction_raster_primitives.rs` |
| 19 | `src-tauri/tests/fixtures/redaction/encrypted-redaction.pdf` | 933 | `2d922b2bdad284ee55bf8b96dded1fe4173cc7da7c368635218a371fbe62d791` | Yes | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security, encrypted` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `secure_redaction_raster_primitives.rs` |
| 20 | `src-tauri/tests/fixtures/redaction/explicit-print-boxes.pdf` | 711 | `80b228c33e9928226932ed5aff7ceb3ab7b41ab63fa9260ab938b1995c67e0b5` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `secure_redaction_raster_primitives.rs` |
| 21 | `src-tauri/tests/fixtures/redaction/image-only.pdf` | 666 | `90cfc9ea441c48fe8ada88a6d22f41b17a1e782a8d354bcdbe348c9686a1ac95` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `whole_object_backend_characterization.rs`, `secure_redaction_planning_worker.rs` |
| 22 | `src-tauri/tests/fixtures/redaction/incremental-revision.pdf` | 1270 | `635f8520ebe3f0617da3634eff066f2dfcfa50a8fc5ccf1861c28c6f8474861f` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs`, `docs/engineering/secure-redaction.md` |
| 23 | `src-tauri/tests/fixtures/redaction/inherited-geometry.pdf` | 627 | `99fe03032c6f17e96d210bf720c4da0d00f50e782b05785e0790f131d84e4a5e` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 24 | `src-tauri/tests/fixtures/redaction/inherited-resources.pdf` | 695 | `23d3d1fe012940a055e325b499edd14ecdad215f71a239729d4b199715b94d6e` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 25 | `src-tauri/tests/fixtures/redaction/js-action.pdf` | 654 | `f8899918bb1e46c2505bb4b996fa47db93a486aee59232dd65c8c8492861bea5` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 26 | `src-tauri/tests/fixtures/redaction/mixed-secure-redaction-pages.pdf` | 1154 | `c3b10284390eb8aba66dc60bfa776ba2f4d0567fe01921050fa7747e2f966d9b` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_execution_acceptance.rs`, `secure_redaction_planning_worker.rs` |
| 27 | `src-tauri/tests/fixtures/redaction/negative-origin.pdf` | 609 | `c689d40fc2b5436a3267ede469682a59f7a869e98837908057b879d9ac8c98cb` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `job_worker_process.rs`, `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 28 | `src-tauri/tests/fixtures/redaction/non-js-openaction.pdf` | 657 | `b69e36bd0bb5d46dbb9d020f92836679470bb7144ea2e20f292bc371d6fb1a01` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 29 | `src-tauri/tests/fixtures/redaction/nonzero-origin.pdf` | 604 | `11bb134408b14960adf5f1d6cce1629ea1e5880cd1cb81f0fdbfd985b1003c0d` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 30 | `src-tauri/tests/fixtures/redaction/object-stream.pdf` | 686 | `68fb1e93556b01fd26376cd0af0955258ce1364f0d7c8bd26b0e56f7107036bb` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 31 | `src-tauri/tests/fixtures/redaction/rotations.pdf` | 1095 | `6665e102d1ad54f8e37bed0149da640930624098794980a10ecf3846fd511628` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `src/secure_redaction.rs`, `job_worker_process.rs`, `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 32 | `src-tauri/tests/fixtures/redaction/same-size-same-rotation-pages.pdf` | 677 | `1383a62e9d5b5e981924b38c82d26fceb33bae18d8ca1a3c01e77c95271dbadc` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 33 | `src-tauri/tests/fixtures/redaction/signature-marked-byterange.pdf` | 921 | `074ae3a0d82c50097fed6d578c31bb5cef7065129a07bf9ae0b229f192eb6b35` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 34 | `src-tauri/tests/fixtures/redaction/simple-text.pdf` | 634 | `2adc560d26fc60706a38061fc486d0e926dce9fa904476e991912d6ea3052af6` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `job_worker_process.rs`, `redaction_fixtures_and_capabilities.rs`, `secure_redaction_execution_acceptance.rs`, `whole_object_backend_characterization.rs` |
| 35 | `src-tauri/tests/fixtures/redaction/stream-literal-prev.pdf` | 599 | `cf17386fdc31be7c9408c745d7c9e81ad499f5e1825ceae2ffaf5954bdc5800e` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 36 | `src-tauri/tests/fixtures/redaction/text-image.pdf` | 836 | `4ae83ed7d9478b51e4c321e6f3523a9612ca114cb68558653a7fef2a8d51f371` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 37 | `src-tauri/tests/fixtures/redaction/unresolved-acroform.pdf` | 618 | `2c4b6922afbc9cc0b9e9cf14f7db71682f4aca30fb7f8529464a38e3b9bdc085` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 38 | `src-tauri/tests/fixtures/redaction/unresolved-resources.pdf` | 581 | `a108798a34b2236b401e027832bed744ab2d993ee98a5dc26400529bfce436c5` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 39 | `src-tauri/tests/fixtures/redaction/whole-object-adversarial.pdf` | 3605 | `9abcb0b23fcf18620f4eb703f59f633258397d7fa20c49837a8023b061cb570d` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `whole_object_backend_characterization.rs`, `secure_redaction_planning_worker.rs` |
| 40 | `src-tauri/tests/fixtures/redaction/xfa-fixture.pdf` | 751 | `504b7e6630e15a663d5933e938237222c33823b06ef4a03c9d85c395ca28fdb3` | No | `deterministically_generated` | `py-redaction-generator` | `complete` | `missing` | `redaction, mutation, security` | `redaction_fixtures_and_capabilities.rs`, `secure_redaction_planning_worker.rs` |
| 41 | `src-tauri/tests/fixtures/sanitization/action-chain-network.pdf` | 1187 | `75dc5c5f923605389cbeac427532840995b00a9acef65c7ca0cc294f23e5f9e0` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 42 | `src-tauri/tests/fixtures/sanitization/clean-minimal.pdf` | 451 | `a7302175f2dfc0e019288c5ab4f536e8f18e4f88dfa59621e198271c43cef555` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 43 | `src-tauri/tests/fixtures/sanitization/combined-names.pdf` | 1086 | `cd9d16018dabadda949137bb4d5a019cc3dbf41e9adca924f0e23f900b600c47` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 44 | `src-tauri/tests/fixtures/sanitization/dangling-critical-references.pdf` | 504 | `f2afe1262bab80d59ca4c0cd38d81acbb03be6de1d0f9b2fa336322eb8059a7b` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 45 | `src-tauri/tests/fixtures/sanitization/embedded-files-af.pdf` | 1502 | `8dfb09ffbe524f78090bf3b72c9d70db1f08b8148f0bd80e0430ef64cd6662e3` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `src/sanitization_verifier.rs`, `sanitization_capabilities.rs` |
| 46 | `src-tauri/tests/fixtures/sanitization/encrypted.pdf` | 780 | `3c959f1f678aeb45459f0d770583c17553fc143e7736281480de0f1d57ae622c` | Yes | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security, encrypted` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs`, `sanitization_inspection_acceptance.rs` |
| 47 | `src-tauri/tests/fixtures/sanitization/hybrid-xref.pdf` | 937 | `e103c998ecd803e07f40a5c52c8e3e9fa8965166e5b4408d25cc022839fea6ff` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_verifier.rs`, `sanitization_capabilities.rs` |
| 48 | `src-tauri/tests/fixtures/sanitization/incremental-revision.pdf` | 880 | `4b2d1b5222d7a9a052f5f32c710076dbaa6935dbbed23e39b64423e30234af9d` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `src/sanitization_verifier.rs`, `sanitization_capabilities.rs` |
| 49 | `src-tauri/tests/fixtures/sanitization/incremental-xref-stream-chain.pdf` | 939 | `1e5e54756b68600c29acb018533e4a11eb2a202f13cd81438226f34278be327c` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_transform.rs`, `src/sanitization_verifier.rs`, `sanitization_capabilities.rs` |
| 50 | `src-tauri/tests/fixtures/sanitization/javascript-actions.pdf` | 1789 | `64adc4db5e4925a0ba231946eed5d7c79a9373fe28b10c46bd4fb689e5ee8c5b` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 51 | `src-tauri/tests/fixtures/sanitization/metadata-paths.pdf` | 1429 | `9de594613615b87830567a8efbc72de3b250c692101b36be1b892922f2806002` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 52 | `src-tauri/tests/fixtures/sanitization/object-xref-stream.pdf` | 597 | `4684a538c93afd5681649cf20a4054f94a051a2dbf079aa97f23cbcf36c027a5` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `src/sanitization_verifier.rs`, `sanitization_capabilities.rs` |
| 53 | `src-tauri/tests/fixtures/sanitization/openaction-destination.pdf` | 475 | `f120326e287539eb30d43fcf7abab2154588b95b546b59c18959da6d87055eb5` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `sanitization_capabilities.rs` |
| 54 | `src-tauri/tests/fixtures/sanitization/signature.pdf` | 938 | `167a04767ed7865b4af4bdfa270fb7394dfa02b00a47352790f447bab088e50b` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `src/sanitization_verifier.rs`, `sanitization_capabilities.rs` |
| 55 | `src-tauri/tests/fixtures/sanitization/unsupported-active-content.pdf` | 2083 | `80c381773d313ce29b0fd65fb10876d53735c80a837eb5296b9421bf2d0f5547` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `src/sanitization.rs`, `src/sanitization_inspection.rs`, `src/sanitization_transform.rs`, `job_worker_process.rs`, `sanitization_capabilities.rs`, `sanitization_inspection_acceptance.rs` |
| 56 | `src-tauri/tests/fixtures/sanitization/xfa-single-stream.pdf` | 585 | `3ebc2988855d0885d5aa2ee150303c6fd559651586ae27f2222645221bb7a57b` | No | `deterministically_generated` | `py-sanitization-generator` | `complete` | `missing` | `sanitization, security` | `sanitization_capabilities.rs` |

---

## 8. Canonical Coverage & Traceability Matrix Model

The application maintains a canonical, machine-readable coverage mapping document at [`docs/engineering/pdf-corpus-coverage.json`](file:///Users/umut/antigravity-proje/docs/engineering/pdf-corpus-coverage.json).

### 8.1 Single Source-of-Truth Principles

1. **Authoritative Representation:** [`docs/engineering/pdf-corpus-coverage.json`](file:///Users/umut/antigravity-proje/docs/engineering/pdf-corpus-coverage.json) is the single canonical source of truth for all coverage assertions, traceability relationships, gap records, and planned test metadata.
2. **Kanonik `Feature → Fixture → Test` Grafı:** The top-level `traceabilityEdges` array defines all active evidence-backed associations. Each edge specifies:
   - `featureId`: The behavioral domain identifier.
   - `fixtureId`: The manifest fixture identifier.
   - `testId`: The unique identifier of a verified test whose `state == "current"`.
   - `relation`: Bounded enum (`direct`, `helper_indirect`, `feature_manifest_indirect`, `generated_lookup`).
   - `evidence`: Structured object with `sourcePath`, `verificationType`, and optional `notes`.
3. **Traceability Edge State Restriction:** Only current, verified tests may participate in `traceabilityEdges`. Planned tests are strictly prohibited from forming edges.
4. **Feature-Level Set Derivation:** For every feature $f$, summary fields are strictly derived from the edge graph:
   $$\text{features}[f].\text{fixtureIds} \equiv \operatorname{unique}(\{ e.\text{fixtureId} \mid e \in \text{traceabilityEdges} \land e.\text{featureId} = f.\text{id} \})$$
   $$\text{features}[f].\text{testIds} \equiv \operatorname{unique}(\{ e.\text{testId} \mid e \in \text{traceabilityEdges} \land e.\text{featureId} = f.\text{id} \})$$
5. **Unconsumed Fixtures Exact-Set Equality:** Unconsumed fixtures are strictly derived via set difference:
   $$\text{coverage}.\text{unconsumedFixtureIds} \equiv \text{manifest}.\text{fixtureIds} \setminus \operatorname{unique}(\{ e.\text{fixtureId} \mid e \in \text{traceabilityEdges} \land \text{test}(e.\text{testId}).\text{state} = \text{"current"} \})$$

### 8.2 7-Dimensional Evaluation Framework

Every behavioral domain is evaluated across 7 orthogonal dimensions using a strictly bounded vocabulary:

* **`positive`**: Verification of expected, valid PDF operations and capability workflows.
* **`negative`**: Rejection and error-boundary verification for unauthorized or boundary-exceeding operations.
* **`malformed`**: Error resilience, parser defense, and crash-free recovery on corrupted or truncated inputs.
* **`encrypted`**: Behavior on password-protected and encrypted PDF streams.
* **`unicode`**: Multilingual character rendering, glyph encoding, search matching, and metadata serialization.
* **`persistence`**: File mutation consistency, generation incrementing, and atomic disk replacement roundtrips.
* **`worker`**: Out-of-process isolation, IPC serialization, and broker recovery boundaries.

#### Permitted Dimension Statuses
* `covered`: Full deterministic coverage validated by current unit, integration, or acceptance tests.
* `partial`: Implemented behavior with partial test coverage or known edge cases.
* `not_covered`: Implemented behavior currently lacking automated test assertions.
* `not_applicable`: Dimension does not apply to this behavioral domain.
* `planned`: Intended future capability mapped to a concrete roadmap prompt owner.

### 8.3 Platform Evidence Levels & Fail-Closed Invariants

Platform evidence is dynamically cross-checked against [`docs/engineering/platform-support.json`](file:///Users/umut/antigravity-proje/docs/engineering/platform-support.json):

* `not_run`: No execution performed for the target platform.
* `source_only`: source/test statik olarak mevcut ve doğrulanmış; compile/runtime execution evidence iddia edilmez.
* `fast_local`: Tests executed offline in host environment (e.g. developer machine / CI fast-local runner).
* `native_runtime`: Tests executed with actual native PDFium dynamic library on host OS.
* `exact_artifact`: Verification executed against packaged release bundle (`.dmg`, `.deb`, `.msi`).

> [!NOTE]
> In accordance with [`docs/engineering/platform-support.json`](file:///Users/umut/antigravity-proje/docs/engineering/platform-support.json), when native integration tests or exact-artifact smoke tests are marked `not_run` on a platform, coverage claims fail closed to `source_only` (for tested features) or `not_run` (for features without active tests).

---

## 9. Authoritative Immediate Roadmap Ownership Table

All gaps and planned tests enforce a single authoritative immediate prompt owner. Slash-separated or composite ownership strings are strictly prohibited and rejected by validators.

| Required Layer / Deficiency Type | Scope / Target Platform | Authoritative `plannedPrompt` | Description |
|---|---|---|---|
| `native_runtime` | macOS native runtime | **`11V-B`** | macOS native PDFium dynamic library verification, Continuity Camera, macOS Share, TTS |
| `native_runtime` | Linux native runtime | **`11V-C`** | Linux native PDFium runtime smoke and integration test execution |
| `native_runtime` | Windows native runtime | **`11V-D`** | Windows native PDFium DLL runtime smoke and integration test execution |
| `exact_artifact` | All platform packages / VeraPDF machine checks | **`11V-E`** | Packaged application bundle smoke tests (.dmg, .deb, .msi) and automated PDF/UA machine validation |
| `integration` | Missing backend/feature behavior | **`11I-B`** | PDF/X export, auto-tagging, reading order, form editing, batch processing, persistence roundtrips |
| `ci` | Missing CI & workflow automation | **`11I-F`** | CI matrix execution, timeout enforcement, permissions configuration |
| `integration` | Missing cryptographic signing | **`11I-G`** | PAdES digital signatures, signature verification, LTV/DSS timestamps, trust store |
| `exact_artifact` | Release certification | **`11R`** | Redaction and sanitization release platform certification |
| Deterministic fixture deficiency | Corpus manifest additions | **`11I-A`** | Fixture corpus additions and manifest registration |

### Bounded `plannedPrompt` Allowlist
```text
11I-A, 11I-B, 11I-C, 11I-E, 11I-F, 11I-G, 11V-B, 11V-C, 11V-D, 11V-E, 11R
```

---

## 10. Contributor Policy & Invariant Gating Contract

### 10.1 Rules for Modifying Coverage & Traceability
1. **No Planned Edges:** Never add a `traceabilityEdge` pointing to a planned test (`state == "planned"`).
2. **Derived Set Consistency:** Never manually edit `features[].fixtureIds`, `features[].testIds`, or `unconsumedFixtureIds` independently; they must always match the derived sets from `traceabilityEdges`.
3. **Single Authoritative Owner:** Every gap and planned test must specify a single `plannedPrompt` from the bounded allowlist. Never use slash (`/`) delimiters.
4. **Deterministic Generators:** Any synthetic fixture must provide a pure-Python standard library generator or pinned script registered in `manifest.generators`.
5. **Credential Security:** Plaintext passwords are strictly forbidden; only `test-credential:<name>` references are permitted.
6. **No AppleDouble / Symlinks:** macOS AppleDouble (`._*.pdf`) and symlink fixtures are strictly rejected.

### 10.2 STRICT_OFFLINE FAST_LOCAL Validation Suite

All pull requests and modifications must pass the complete validation suite:

```bash
# 1. JSON parse verification
python3 -c "import json, pathlib; json.loads(pathlib.Path('src-tauri/tests/fixtures/corpus-manifest.json').read_text('utf-8')); print('corpus manifest JSON parse: PASS')"
python3 -c "import json, pathlib; json.loads(pathlib.Path('docs/engineering/pdf-corpus-coverage.json').read_text('utf-8')); print('coverage JSON parse: PASS')"

# 2. Node Semantic Feature Registry & Traceability Validator (Dependency-Free)
node scripts/validate-pdf-corpus-coverage.mjs

# 3. Release Smoke Non-Destructive Determinizm Doğrulaması
python3 src-tauri/tests/fixtures/pdf/generate_release_smoke.py --verify-determinism

# 4. Redaction Non-Destructive Determinizm Doğrulaması (Tempfile/Verify Modu)
python3 src-tauri/tests/fixtures/redaction/generate_redaction_fixtures.py --verify-determinism

# 5. Sanitization Non-Destructive Determinizm Doğrulaması (Tempfile/Verify Modu)
python3 src-tauri/tests/fixtures/sanitization/generate_sanitization_fixtures.py --verify-determinism

# 6. Reorder Generator Side-Effect-Free Import & Controlled Offline Dependency-Check
PYTHONDONTWRITEBYTECODE=1 python3 -c "import importlib.util; spec = importlib.util.spec_from_file_location('reorder_gen', 'src-tauri/tests/fixtures/pdf/generate_reorder_fixtures.py'); mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod); print('reorder generator side-effect-free import: PASS')"
python3 src-tauri/tests/fixtures/pdf/generate_reorder_fixtures.py --check-dependencies

# 7. Rust Code Formatting Check
(cd src-tauri && CARGO_NET_OFFLINE=true cargo fmt --all -- --check)

# 8. Rust Corpus Coverage Invariant Tests
(cd src-tauri && CARGO_NET_OFFLINE=true cargo test --locked --offline --test pdf_corpus_coverage)

# 9. Rust Corpus Manifest Integrity Validator
(cd src-tauri && CARGO_NET_OFFLINE=true cargo test --locked --offline --test fixture_manifest_integrity)
```

---

## 11. Corpus Hygiene, Deterministic Regeneration & Fixture Lifecycle Policy

This section specifies the canonical developer policy for adding, regenerating, updating, versioning, and retiring PDF fixtures in the Antigravity PDF Reader repository (Prompt 11I-A6).

### 11.1 Adding Fixtures
Before introducing any new fixture to the repository:
1. **Reuse First:** Audit existing 56 fixtures in `src-tauri/tests/fixtures/corpus-manifest.json` to verify whether the target property or edge case can be tested against an already registered fixture.
2. **Provenance & Source:** Every new fixture must have a clearly documented source and license:
   - For synthetically generated fixtures, a pure-Python standard library generator script must be committed or updated in `src-tauri/tests/fixtures/`.
   - For project-authored fixtures, complete provenance documentation and explicit license reference (`repository_license_reference`) must be recorded.
   - For third-party fixtures, exact origin, license status, and redistribution permissions must be verified.
3. **Deterministic Expectations:** Synthetic generators must use fixed IDs, fixed timestamps (e.g. `D:20000101000000Z`), deterministic object streams, and provide a non-destructive `--verify-determinism` mode.
4. **Manifest & Coverage Synchronization:** The new fixture record must be added to `corpus-manifest.json`, linked in `docs/engineering/pdf-corpus-coverage.json` (under `traceabilityEdges` or `unconsumedFixtureIds`), and validated via `fixture_manifest_integrity.rs` and `validate-pdf-corpus-coverage.mjs`.

### 11.2 Regenerating Fixtures
Fixture regeneration is an explicit, intentional developer action and must never occur implicitly during normal test execution.
1. **STRICT_OFFLINE Execution:** Generators must execute without network connectivity. If external dependencies are required but unavailable locally, the check must fail closed with `TOOL_UNAVAILABLE_OFFLINE` rather than attempting package downloads.
2. **Output Containment:** Non-destructive determinism verification must always generate into an isolated temporary directory (`tempfile.TemporaryDirectory()`) or in-memory buffer. Normal tests and verifiers must never overwrite committed fixture files.
3. **Exact-Match Verification:** Generated artifact SHA-256 and byteLength must be compared against committed bytes and central manifest records.
4. **Intentional Update Separation:** Updating committed fixture files on disk is a separate developer operation requiring deliberate review and manifest update.

### 11.3 Updating Fixtures
If a fixture must be updated due to a specification change or bug fix:
1. **Explicit Rationale:** Document why the fixture bytes changed.
2. **Hash & Size Audit:** Record the old SHA-256 / byteLength and new SHA-256 / byteLength.
3. **Consumer & Manifest Impact:** Update `corpus-manifest.json`, relevant feature manifests (`redaction/manifest.json`, `sanitization/manifest.json`), and verify all consumer unit and integration tests.
4. **Coverage Integrity:** Re-run `validate-pdf-corpus-coverage.mjs` and `fixture_manifest_integrity.rs` to verify that no invariant drifts.

### 11.4 Generator Versioning & Reproducibility Classification
1. **`deterministic_by_construction` / `byte_deterministic`:** Pure standard library generators where exact byte reproduction has been demonstrated under the canonical/current supported generator environment and deterministic construction rules (e.g., `py-release-smoke-generator`, `py-redaction-generator`, `py-sanitization-generator`). This classification indicates that the generator follows deterministic construction algorithms (fixed IDs, static dates, deterministic object offsets, sorted mappings), but does **not** assert universal bit-for-bit identity across arbitrary unverified Python interpreter versions, zlib implementations, host operating systems, or external toolchains unless separately verified.
2. **`toolchain_unpinned`:** Generators depending on external packages (`reportlab`, `pypdf`) whose versions are not pinned in a repository lockfile (e.g., `py-reorder-generator`). Exact byte matches on a specific machine must be treated as environment-specific evidence and not claimed as universal cross-platform byte determinism.
3. **`historical_documented_method`:** Historical one-off creation procedures with documented provenance but no committed executable script (e.g., `pypdf-historical-method`).
4. **Controlled Offline Guarding:** Generator scripts must ensure module imports are completely side-effect-free and unhandled dependency exceptions are avoided.

### 11.5 Fixture Deletion Policy
A fixture may only be removed under strict conditions:
1. **Zero Active Consumers:** Perform complete codebase scan verifying no unit test, integration test, or fuzz target consumes the fixture.
2. **Central & Feature Manifest Cleanup:** Remove or retire the fixture entry in `corpus-manifest.json` and any associated feature manifest.
3. **Coverage Matrix Adjustment:** Remove all references in `docs/engineering/pdf-corpus-coverage.json` and re-run validators.
4. **Documented Rationale:** Explicitly record the reason for removal in commit history and documentation. (Prompt 11I-A6 does not delete any fixtures).

