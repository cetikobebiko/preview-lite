use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PdfFixtureManifest {
    pub schema_version: String,
    pub corpus_version: String,
    pub description: String,
    pub generators: HashMap<String, GeneratorRecord>,
    pub fixtures: Vec<PdfFixtureRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorKind {
    CommittedScript,
    HistoricalDocumentedMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratorRecord {
    pub kind: GeneratorKind,
    pub path: Option<String>,
    pub language: String,
    pub dependencies: Vec<String>,
    pub network_requirement: NetworkRequirement,
    pub reproducibility_status: ReproducibilityStatus,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkRequirement {
    None,
    BuildTimeOnly,
    RuntimeOptional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproducibilityStatus {
    DeterministicByConstruction,
    ReproducibilityVerified,
    ToolchainUnpinned,
    ReproducibilityUnverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    #[serde(default)]
    pub allow_empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceStatus {
    Complete,
    Partial,
    Missing,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseStatus {
    Known,
    RepositoryLicenseReference,
    Missing,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TestKind {
    Unit,
    Integration,
    Acceptance,
    Fuzz,
    Smoke,
}

pub const CANONICAL_EXPECTED_CAPABILITIES: &[&str] = &[
    "has_text",
    "has_unicode_text",
    "has_vector_content",
    "multi_page",
    "large_page_dimensions",
];

fn get_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Resolve repo root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

pub fn validate_kebab_case_id(id: &str) -> bool {
    let re = Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").unwrap();
    re.is_match(id)
}

pub fn validate_sha256_format(sha: &str) -> bool {
    let re = Regex::new(r"^[a-f0-9]{64}$").unwrap();
    re.is_match(sha)
}

pub fn validate_token_format(token: &str) -> bool {
    let re = Regex::new(r"^[a-z0-9]+(?:_[a-z0-9]+)*$").unwrap();
    re.is_match(token)
}

pub fn validate_password_reference(reference: &str) -> bool {
    let re = Regex::new(r"^test-credential:[a-z0-9-]+$").unwrap();
    re.is_match(reference)
}

pub fn validate_lexical_path(raw_path: &str) -> Result<PathBuf, String> {
    if raw_path.trim().is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    if raw_path.contains("://") {
        return Err(format!("URI scheme is forbidden in path: '{raw_path}'"));
    }

    let bytes = raw_path.as_bytes();
    if raw_path.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(format!(
            "Windows drive letters are forbidden in relative path: '{raw_path}'"
        ));
    }

    if raw_path.starts_with(r"\\") || raw_path.starts_with("//") {
        return Err(format!("UNC paths are forbidden in path: '{raw_path}'"));
    }

    if raw_path.starts_with('/') || raw_path.starts_with('\\') {
        return Err(format!(
            "Absolute paths are forbidden in relative path: '{raw_path}'"
        ));
    }

    if raw_path.contains('\\') {
        return Err(format!(
            "Non-canonical path separator '\\' forbidden in path: '{raw_path}'"
        ));
    }

    let segments: Vec<&str> = raw_path.split('/').collect();

    let mut rel = PathBuf::new();
    for segment in segments {
        if segment.is_empty() {
            return Err(format!(
                "Empty segment or consecutive slashes forbidden in path: '{raw_path}'"
            ));
        }
        if segment == "." {
            return Err(format!(
                "Current directory '.' segment forbidden in path: '{raw_path}'"
            ));
        }
        if segment == ".." {
            return Err(format!(
                "Parent directory '..' traversal forbidden in path: '{raw_path}'"
            ));
        }
        if segment.starts_with("._") {
            return Err(format!(
                "AppleDouble sidecar path segment forbidden in path: '{raw_path}'"
            ));
        }
        rel.push(segment);
    }

    Ok(rel)
}

pub fn validate_safe_repo_relative_path(
    raw_path: &str,
    repo_root: &Path,
) -> Result<PathBuf, String> {
    let rel_path = validate_lexical_path(raw_path)?;
    let resolved = repo_root.join(&rel_path);

    if let Ok(metadata) = resolved.symlink_metadata() {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Symlink fixture is forbidden: '{raw_path}' is a symlink"
            ));
        }

        let canonical_root = repo_root
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize repository root '{repo_root:?}': {e}"))?;
        let canonical_resolved = resolved
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize path '{raw_path}': {e}"))?;
        if !canonical_resolved.starts_with(&canonical_root) {
            return Err(format!(
                "Symlink escapes repo root: '{raw_path}' resolves outside repository"
            ));
        }
    }

    Ok(resolved)
}

pub fn validate_manifest(
    manifest: &PdfFixtureManifest,
    repo_root: &Path,
    check_disk: bool,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // 0. Exact schema version enforcement
    if manifest.schema_version != "1.0.0" {
        errors.push(format!(
            "Unsupported schemaVersion '{}' (only '1.0.0' is supported)",
            manifest.schema_version
        ));
    }
    if manifest.corpus_version.trim().is_empty() {
        errors.push("corpusVersion cannot be empty".to_string());
    }
    if manifest.description.trim().is_empty() {
        errors.push("description cannot be empty".to_string());
    }

    // 1. Generator Invariants
    for (gen_id, gen_record) in &manifest.generators {
        if !validate_kebab_case_id(gen_id) {
            errors.push(format!(
                "Generator ID '{gen_id}' is invalid (must match ^[a-z0-9]+(?:-[a-z0-9]+)*$)"
            ));
        }

        match gen_record.kind {
            GeneratorKind::CommittedScript => match &gen_record.path {
                Some(path_str) => match validate_safe_repo_relative_path(path_str, repo_root) {
                    Ok(resolved_path) => {
                        if check_disk && !resolved_path.is_file() {
                            errors.push(format!(
                                "Committed script for generator '{gen_id}' not found on disk at '{path_str}'"
                            ));
                        }
                    }
                    Err(e) => {
                        errors.push(format!("Invalid path for generator '{gen_id}': {e}"));
                    }
                },
                None => {
                    errors.push(format!(
                        "Generator '{gen_id}' with kind 'committed_script' must have a non-null path"
                    ));
                }
            },
            GeneratorKind::HistoricalDocumentedMethod => {
                if gen_record.path.is_some() {
                    errors.push(format!(
                        "Generator '{gen_id}' with kind 'historical_documented_method' must have path = null"
                    ));
                }
            }
        }
    }

    // 2. Fixture Invariants
    let mut seen_ids = HashSet::new();
    let mut seen_normalized_paths = HashSet::new();
    let mut seen_shas: HashMap<String, (String, String)> = HashMap::new();
    let mut release_smoke_count = 0;

    for fixture in &manifest.fixtures {
        // ID uniqueness and format
        if !validate_kebab_case_id(&fixture.id) {
            errors.push(format!(
                "Fixture ID '{}' is invalid (must match ^[a-z0-9]+(?:-[a-z0-9]+)*$)",
                fixture.id
            ));
        } else if !seen_ids.insert(&fixture.id) {
            errors.push(format!("Duplicate fixture ID: '{}'", fixture.id));
        }

        // Safe repo-relative path validation and normalized duplicate-path detection
        let resolved_file_path = match validate_lexical_path(&fixture.path) {
            Ok(normalized_rel_path) => {
                if !seen_normalized_paths.insert(normalized_rel_path) {
                    errors.push(format!("Duplicate fixture path: '{}'", fixture.path));
                }
                match validate_safe_repo_relative_path(&fixture.path, repo_root) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        errors.push(format!("Fixture '{}' path error: {e}", fixture.id));
                        None
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Fixture '{}' path error: {e}", fixture.id));
                None
            }
        };

        // Canonical SHA-256 syntax and duplicate SHA validation (always runs, independent of check_disk)
        if !validate_sha256_format(&fixture.sha256) {
            errors.push(format!(
                "Fixture '{}' has invalid sha256 '{}' (must be exactly 64 lowercase hexadecimal characters)",
                fixture.id, fixture.sha256
            ));
        } else if let Some((first_id, first_path)) = seen_shas.get(&fixture.sha256) {
            errors.push(format!(
                "Duplicate SHA-256 '{}' detected between fixture '{}' ('{}') and fixture '{}' ('{}')",
                fixture.sha256, first_id, first_path, fixture.id, fixture.path
            ));
        } else {
            seen_shas.insert(
                fixture.sha256.clone(),
                (fixture.id.clone(), fixture.path.clone()),
            );
        }

        // PDF Version
        if fixture.pdf_version.trim().is_empty() {
            errors.push(format!(
                "Fixture '{}' pdfVersion cannot be empty",
                fixture.id
            ));
        }

        // Release smoke category invariant: only canonical record permitted
        if fixture.categories.contains(&FixtureCategory::ReleaseSmoke) {
            release_smoke_count += 1;
            if fixture.id != "core-release-smoke" {
                errors.push(format!(
                    "Fixture '{}' carries category 'release_smoke', but only canonical 'core-release-smoke' may carry this category",
                    fixture.id
                ));
            }
            if fixture.path != "src-tauri/tests/fixtures/pdf/release-smoke.pdf" {
                errors.push(format!(
                    "Fixture '{}' carries category 'release_smoke', but only canonical path 'src-tauri/tests/fixtures/pdf/release-smoke.pdf' may carry this category",
                    fixture.id
                ));
            }
        }

        // Generator ID Reference Validation
        if let Some(gid) = &fixture.generator_id {
            if !validate_kebab_case_id(gid) {
                errors.push(format!(
                    "Fixture '{}' has invalid generatorId '{gid}' (must match ^[a-z0-9]+(?:-[a-z0-9]+)*$)",
                    fixture.id
                ));
            }
            if !manifest.generators.contains_key(gid) {
                errors.push(format!(
                    "Fixture '{}' references unknown generatorId '{gid}'",
                    fixture.id
                ));
            }
        }

        // Provenance & SourceType Invariants
        match fixture.source_type {
            SourceType::DeterministicallyGenerated => match &fixture.generator_id {
                Some(gid) => {
                    if let Some(gen_record) = manifest.generators.get(gid) {
                        if gen_record.kind != GeneratorKind::CommittedScript {
                            errors.push(format!(
                                "Fixture '{}' is deterministically_generated but generator '{gid}' is not a committed_script",
                                fixture.id
                            ));
                        }
                    }
                }
                None => {
                    errors.push(format!(
                        "Fixture '{}' has sourceType 'deterministically_generated' but missing generatorId",
                        fixture.id
                    ));
                }
            },
            SourceType::Generated => {
                let has_gen = fixture.generator_id.is_some();
                let has_source = fixture
                    .source
                    .as_ref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
                if !has_gen && !has_source {
                    errors.push(format!(
                        "Fixture '{}' has sourceType 'generated' but lacks both generatorId and source",
                        fixture.id
                    ));
                }
            }
            SourceType::ProjectAuthored => {
                if fixture.provenance_status != ProvenanceStatus::Complete {
                    errors.push(format!(
                        "Fixture '{}' has sourceType 'project_authored' but provenanceStatus is not 'complete'",
                        fixture.id
                    ));
                }
                if fixture
                    .source
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    errors.push(format!(
                        "Fixture '{}' has sourceType 'project_authored' but missing source description",
                        fixture.id
                    ));
                }
            }
            SourceType::Unknown if fixture.provenance_status == ProvenanceStatus::Complete => {
                errors.push(format!(
                    "Fixture '{}' has sourceType 'unknown' but provenanceStatus is 'complete'",
                    fixture.id
                ));
            }
            _ => {}
        }

        if fixture.provenance_status == ProvenanceStatus::Complete
            && fixture
                .source
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
        {
            errors.push(format!(
                "Fixture '{}' has provenanceStatus 'complete' but missing source description",
                fixture.id
            ));
        }

        // License Invariants
        match fixture.license_status {
            LicenseStatus::Known => {
                if fixture
                    .license
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    errors.push(format!(
                        "Fixture '{}' has licenseStatus 'known' but missing license SPDX/identifier",
                        fixture.id
                    ));
                }
            }
            LicenseStatus::RepositoryLicenseReference => {
                if fixture.license.is_some() {
                    errors.push(format!(
                        "Fixture '{}' has licenseStatus 'repository_license_reference' but specified explicit license identifier",
                        fixture.id
                    ));
                }
            }
            LicenseStatus::Missing => {
                if fixture.license.is_some() {
                    errors.push(format!(
                        "Fixture '{}' has licenseStatus 'missing' but specified fabricated license identifier",
                        fixture.id
                    ));
                }
            }
            LicenseStatus::NotApplicable => {
                if fixture.license.is_some() {
                    errors.push(format!(
                        "Fixture '{}' has licenseStatus 'not_applicable' but specified license identifier",
                        fixture.id
                    ));
                }
            }
        }

        // AllowEmpty & Malformed Invariant
        let is_malformed = fixture.categories.contains(&FixtureCategory::Malformed);
        let is_empty = fixture.byte_length == 0 || fixture.allow_empty;

        if is_empty {
            if !is_malformed {
                errors.push(format!(
                    "Fixture '{}' has byteLength == 0 or allowEmpty == true but lacks category 'malformed'",
                    fixture.id
                ));
            }
            if fixture.expected_findings.is_empty() {
                errors.push(format!(
                    "Fixture '{}' is empty/malformed but has no expectedFindings token",
                    fixture.id
                ));
            }
        } else if !is_malformed && fixture.allow_empty {
            errors.push(format!(
                "Fixture '{}' has allowEmpty == true without malformed category",
                fixture.id
            ));
        }

        // Category Uniqueness
        let mut seen_categories = HashSet::new();
        for cat in &fixture.categories {
            if !seen_categories.insert(cat) {
                errors.push(format!(
                    "Fixture '{}' has duplicate category '{:?}'",
                    fixture.id, cat
                ));
            }
        }

        // Capability Token Format, Uniqueness, and Bounded Vocabulary
        let mut seen_caps = HashSet::new();
        for cap in &fixture.expected_capabilities {
            if !validate_token_format(cap) {
                errors.push(format!(
                    "Fixture '{}' has invalid expectedCapabilities token '{cap}' (must match ^[a-z0-9]+(?:_[a-z0-9]+)*$)",
                    fixture.id
                ));
            } else if !CANONICAL_EXPECTED_CAPABILITIES.contains(&cap.as_str()) {
                errors.push(format!(
                    "Fixture '{}' has non-canonical expectedCapabilities token '{cap}' (must be one of {:?})",
                    fixture.id, CANONICAL_EXPECTED_CAPABILITIES
                ));
            }
            if !seen_caps.insert(cap) {
                errors.push(format!(
                    "Fixture '{}' has duplicate expectedCapabilities token '{cap}'",
                    fixture.id
                ));
            }
        }

        // Finding Token Format and Uniqueness
        let mut seen_findings = HashSet::new();
        for finding in &fixture.expected_findings {
            if !validate_token_format(finding) {
                errors.push(format!(
                    "Fixture '{}' has invalid expectedFindings token '{finding}' (must match ^[a-z0-9]+(?:_[a-z0-9]+)*$)",
                    fixture.id
                ));
            }
            if !seen_findings.insert(finding) {
                errors.push(format!(
                    "Fixture '{}' has duplicate expectedFindings token '{finding}'",
                    fixture.id
                ));
            }
        }

        // Allowed Tests Uniqueness
        let mut seen_allowed_tests = HashSet::new();
        for test_kind in &fixture.allowed_tests {
            if !seen_allowed_tests.insert(test_kind) {
                errors.push(format!(
                    "Fixture '{}' has duplicate allowedTests entry '{:?}'",
                    fixture.id, test_kind
                ));
            }
        }

        // Feature Manifest Invariant
        if let Some(fm_path) = &fixture.feature_manifest {
            match validate_safe_repo_relative_path(fm_path, repo_root) {
                Ok(resolved_fm) => {
                    if check_disk && !resolved_fm.is_file() {
                        errors.push(format!(
                            "Fixture '{}' references non-existent featureManifest at '{fm_path}'",
                            fixture.id
                        ));
                    }
                }
                Err(e) => {
                    errors.push(format!(
                        "Fixture '{}' invalid featureManifest path '{fm_path}': {e}",
                        fixture.id
                    ));
                }
            }
        }

        // Password & Encryption Invariant
        if let Some(pass_ref) = &fixture.password_reference {
            if !validate_password_reference(pass_ref) {
                errors.push(format!(
                    "Fixture '{}' has invalid passwordReference '{pass_ref}' (must match ^test-credential:[a-z0-9-]+$ and never raw passwords)",
                    fixture.id
                ));
            }
            if !fixture.encrypted {
                errors.push(format!(
                    "Fixture '{}' specifies passwordReference but encrypted == false",
                    fixture.id
                ));
            }
        }

        if fixture.encrypted && !fixture.categories.contains(&FixtureCategory::Encrypted) {
            errors.push(format!(
                "Fixture '{}' has encrypted == true but lacks category 'encrypted'",
                fixture.id
            ));
        }

        // Disk checks (if active)
        if check_disk {
            if let Some(file_path) = &resolved_file_path {
                if !file_path.is_file() {
                    errors.push(format!(
                        "Fixture '{}' file not found on disk at '{}'",
                        fixture.id, fixture.path
                    ));
                } else {
                    match fs::read(file_path) {
                        Ok(bytes) => {
                            if bytes.len() as u64 != fixture.byte_length {
                                errors.push(format!(
                                    "Fixture '{}' byteLength mismatch: declared {}, disk has {}",
                                    fixture.id,
                                    fixture.byte_length,
                                    bytes.len()
                                ));
                            }
                            let digest = format!("{:x}", Sha256::digest(&bytes));
                            if digest != fixture.sha256 {
                                errors.push(format!(
                                    "Fixture '{}' sha256 mismatch: declared {}, disk has {}",
                                    fixture.id, fixture.sha256, digest
                                ));
                            }
                        }
                        Err(e) => {
                            errors.push(format!(
                                "Fixture '{}' failed to read file on disk at '{}': {e}",
                                fixture.id, fixture.path
                            ));
                        }
                    }
                }
            }
        }
    }

    if release_smoke_count > 1 {
        errors.push(format!(
            "Manifest contains {} fixtures with category 'release_smoke' (maximum 1 allowed)",
            release_smoke_count
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// -----------------------------------------------------------------------------
// TESTS
// -----------------------------------------------------------------------------

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("ag_corpus_test_{prefix}_{pid}_{count}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("Create temporary test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_canonical_corpus_manifest_parses_and_validates() {
    let repo_root = get_repo_root();
    let manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");
    assert!(
        manifest_path.is_file(),
        "Canonical corpus-manifest.json must exist"
    );

    let raw_bytes = fs::read(&manifest_path).expect("Read canonical manifest");
    let manifest: PdfFixtureManifest =
        serde_json::from_slice(&raw_bytes).expect("Deserialize canonical manifest strictly");

    assert_eq!(manifest.schema_version, "1.0.0");
    assert_eq!(manifest.corpus_version, "1.0.0");
    assert!(!manifest.description.is_empty());

    // Prompt 11I-A3 baseline: 5 canonical generators, plus the heavy-page
    // generator added for the worker page-quarantine policy.
    assert_eq!(manifest.generators.len(), 6);
    assert!(manifest.generators.contains_key("py-heavy-page-generator"));
    assert!(manifest.generators.contains_key("py-reorder-generator"));
    assert!(manifest.generators.contains_key("py-redaction-generator"));
    assert!(manifest
        .generators
        .contains_key("py-sanitization-generator"));
    assert!(manifest
        .generators
        .contains_key("py-release-smoke-generator"));
    assert!(manifest.generators.contains_key("pypdf-historical-method"));

    // Prompt 11I-A3: 56 fixtures, plus core-heavy-page (page quarantine).
    assert_eq!(
        manifest.fixtures.len(),
        57,
        "Canonical corpus-manifest.json must contain exactly 57 fixtures"
    );

    // Exactly one fixture carrying release_smoke category
    let release_smoke_matches: Vec<&PdfFixtureRecord> = manifest
        .fixtures
        .iter()
        .filter(|f| f.categories.contains(&FixtureCategory::ReleaseSmoke))
        .collect();
    assert_eq!(
        release_smoke_matches.len(),
        1,
        "Canonical corpus manifest must contain exactly one fixture with category 'release_smoke'"
    );
    assert_eq!(release_smoke_matches[0].id, "core-release-smoke");
    assert_eq!(
        release_smoke_matches[0].path,
        "src-tauri/tests/fixtures/pdf/release-smoke.pdf"
    );

    // Validate entire manifest against repository root
    let validation_result = validate_manifest(&manifest, &repo_root, true);
    assert!(
        validation_result.is_ok(),
        "Validation failed: {:?}",
        validation_result.err()
    );
}

#[test]
fn test_canonical_generator_paths_exist_on_disk() {
    let repo_root = get_repo_root();
    let manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");
    let raw_bytes = fs::read(&manifest_path).expect("Read canonical manifest");
    let manifest: PdfFixtureManifest = serde_json::from_slice(&raw_bytes).unwrap();

    let reorder_gen = manifest.generators.get("py-reorder-generator").unwrap();
    assert_eq!(reorder_gen.kind, GeneratorKind::CommittedScript);
    assert_eq!(
        reorder_gen.reproducibility_status,
        ReproducibilityStatus::ToolchainUnpinned
    );
    assert_eq!(reorder_gen.network_requirement, NetworkRequirement::None);
    let reorder_path = repo_root.join(reorder_gen.path.as_ref().unwrap());
    assert!(
        reorder_path.is_file(),
        "Reorder generator script must exist at {:?}",
        reorder_path
    );

    let redaction_gen = manifest.generators.get("py-redaction-generator").unwrap();
    assert_eq!(redaction_gen.kind, GeneratorKind::CommittedScript);
    assert_eq!(
        redaction_gen.reproducibility_status,
        ReproducibilityStatus::DeterministicByConstruction
    );
    let redaction_path = repo_root.join(redaction_gen.path.as_ref().unwrap());
    assert!(
        redaction_path.is_file(),
        "Redaction generator script must exist at {:?}",
        redaction_path
    );

    let sanit_gen = manifest
        .generators
        .get("py-sanitization-generator")
        .unwrap();
    assert_eq!(sanit_gen.kind, GeneratorKind::CommittedScript);
    assert_eq!(
        sanit_gen.reproducibility_status,
        ReproducibilityStatus::DeterministicByConstruction
    );
    let sanit_path = repo_root.join(sanit_gen.path.as_ref().unwrap());
    assert!(
        sanit_path.is_file(),
        "Sanitization generator script must exist at {:?}",
        sanit_path
    );

    let release_smoke_gen = manifest
        .generators
        .get("py-release-smoke-generator")
        .unwrap();
    assert_eq!(release_smoke_gen.kind, GeneratorKind::CommittedScript);
    assert_eq!(
        release_smoke_gen.reproducibility_status,
        ReproducibilityStatus::DeterministicByConstruction
    );
    assert_eq!(
        release_smoke_gen.network_requirement,
        NetworkRequirement::None
    );
    let release_smoke_path = repo_root.join(release_smoke_gen.path.as_ref().unwrap());
    assert!(
        release_smoke_path.is_file(),
        "Release smoke generator script must exist at {:?}",
        release_smoke_path
    );

    let historical_gen = manifest.generators.get("pypdf-historical-method").unwrap();
    assert_eq!(
        historical_gen.kind,
        GeneratorKind::HistoricalDocumentedMethod
    );
    assert_eq!(
        historical_gen.reproducibility_status,
        ReproducibilityStatus::ReproducibilityUnverified
    );
    assert!(
        historical_gen.path.is_none(),
        "Historical generator path must be None"
    );
}

#[test]
fn test_schema_version_validation() {
    let repo_root = get_repo_root();

    let mut manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Schema version test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![],
    };

    // 1. Supported version "1.0.0" -> ACCEPTED
    assert!(validate_manifest(&manifest, &repo_root, false).is_ok());

    // 2. Unsupported versions -> REJECTED
    let unsupported_versions = vec!["0.9.0", "1.0.1", "2.0.0", "future", "", "1.0", "v1.0.0"];
    for version in unsupported_versions {
        manifest.schema_version = version.to_string();
        let res = validate_manifest(&manifest, &repo_root, false);
        assert!(
            res.is_err(),
            "schemaVersion '{version}' should be rejected by validate_manifest"
        );
        let errs = res.unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("Unsupported schemaVersion")),
            "Expected 'Unsupported schemaVersion' error for version '{version}', got: {errs:?}"
        );
    }
}

#[test]
fn test_fixture_and_generator_id_syntax() {
    let repo_root = get_repo_root();

    // Valid IDs
    let valid_ids = vec![
        "core-minimal",
        "py-reorder-generator",
        "a",
        "pdf-123",
        "complex-kebab-case-identifier-99",
        "123",
    ];
    for id in valid_ids {
        assert!(
            validate_kebab_case_id(id),
            "ID '{id}' should be valid kebab-case"
        );
    }

    // Invalid IDs
    let invalid_ids = vec![
        "",
        "Core-Minimal",
        "core_minimal",
        "core minimal",
        "-core-minimal",
        "core-minimal-",
        "core--minimal",
        "../core",
        "core/minimal",
        "core.minimal",
        "core@minimal",
    ];
    for id in invalid_ids {
        assert!(
            !validate_kebab_case_id(id),
            "ID '{id}' should be invalid kebab-case"
        );
    }

    // Generator ID validation in manifest
    let mut manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "ID test".to_string(),
        generators: HashMap::from([(
            "Invalid_Gen_ID".to_string(),
            GeneratorRecord {
                kind: GeneratorKind::HistoricalDocumentedMethod,
                path: None,
                language: "python3".to_string(),
                dependencies: vec![],
                network_requirement: NetworkRequirement::None,
                reproducibility_status: ReproducibilityStatus::ReproducibilityUnverified,
                notes: None,
            },
        )]),
        fixtures: vec![],
    };
    assert!(
        validate_manifest(&manifest, &repo_root, false).is_err(),
        "Generator ID with underscore must be rejected"
    );

    // Fixture generatorId syntax and presence in generators map
    manifest.generators.clear();
    manifest.generators.insert(
        "valid-gen-id".to_string(),
        GeneratorRecord {
            kind: GeneratorKind::HistoricalDocumentedMethod,
            path: None,
            language: "python3".to_string(),
            dependencies: vec![],
            network_requirement: NetworkRequirement::None,
            reproducibility_status: ReproducibilityStatus::ReproducibilityUnverified,
            notes: None,
        },
    );

    let mut fixture = PdfFixtureRecord {
        id: "valid-fixture-id".to_string(),
        path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
        sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
        byte_length: 329,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::Generated,
        source: Some("Source description".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: Some("valid-gen-id".to_string()),
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };
    manifest.fixtures = vec![fixture.clone()];
    assert!(validate_manifest(&manifest, &repo_root, false).is_ok());

    // Malformed generatorId on fixture
    fixture.generator_id = Some("Invalid_Gid".to_string());
    manifest.fixtures = vec![fixture.clone()];
    assert!(validate_manifest(&manifest, &repo_root, false).is_err());

    // Dangling generatorId on fixture
    fixture.generator_id = Some("non-existent-gen".to_string());
    manifest.fixtures = vec![fixture.clone()];
    assert!(validate_manifest(&manifest, &repo_root, false).is_err());

    // Malformed fixture ID
    fixture.id = "Malformed_Fixture_ID".to_string();
    fixture.generator_id = Some("valid-gen-id".to_string());
    manifest.fixtures = vec![fixture];
    assert!(validate_manifest(&manifest, &repo_root, false).is_err());
}

#[test]
fn test_sha256_syntax_validation() {
    let repo_root = get_repo_root();

    let valid_sha = "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d";
    assert!(validate_sha256_format(valid_sha));

    let invalid_shas = vec![
        "",                                                                  // empty
        "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00",   // 63 chars
        "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00da", // 65 chars
        "0D509281E91D6D7CBFBCB4D16F5A205578E65765A11C59187B07B585D555B00D",  // uppercase
        "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00g",  // non-hex 'g'
        "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00z",  // non-hex 'z'
        "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00!",  // non-hex '!'
        " 0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d", // leading space
    ];

    for sha in invalid_shas {
        assert!(
            !validate_sha256_format(sha),
            "SHA '{sha}' should be invalid"
        );
    }

    // Manifest validation with check_disk = false must still reject malformed SHA
    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "SHA syntax test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![PdfFixtureRecord {
            id: "test-bad-sha".to_string(),
            path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
            sha256: "0D509281E91D6D7CBFBCB4D16F5A205578E65765A11C59187B07B585D555B00D".to_string(),
            byte_length: 329,
            pdf_version: "1.4".to_string(),
            license: None,
            license_status: LicenseStatus::RepositoryLicenseReference,
            copyright: None,
            source_type: SourceType::ProjectAuthored,
            source: Some("Authored".to_string()),
            provenance_status: ProvenanceStatus::Complete,
            generator_id: None,
            generated_from: None,
            encrypted: false,
            password_reference: None,
            categories: vec![FixtureCategory::Core],
            expected_capabilities: vec!["has_text".to_string()],
            expected_findings: vec![],
            allowed_tests: vec![TestKind::Integration],
            feature_manifest: None,
            notes: None,
            allow_empty: false,
        }],
    };

    let res = validate_manifest(&manifest, &repo_root, false);
    assert!(
        res.is_err(),
        "Uppercase SHA-256 must be rejected even when check_disk == false"
    );
}

#[test]
fn test_safe_path_validator_rejection_rules() {
    let repo_root = get_repo_root();

    let invalid_paths = vec![
        "",
        "   ",
        "../outside.pdf",
        "foo/../../bar.pdf",
        r"..\windows_traversal.pdf",
        r"foo\..\..\bar.pdf",
        "/absolute/unix/path.pdf",
        r"\absolute\windows\path.pdf",
        "C:\\windows\\system32.pdf",
        "c:/windows/system32.pdf",
        "D:\\data.pdf",
        r"\\server\share\file.pdf",
        "//server/share/file.pdf",
        "file:///path/to/file.pdf",
        "http://example.com/file.pdf",
        "https://example.com/file.pdf",
        "foo//bar.pdf",
        "foo/./bar.pdf",
        r"foo\\bar.pdf",
        r"foo\bar.pdf",
        "src-tauri/tests/fixtures/pdf/._minimal.pdf",
    ];

    for path in invalid_paths {
        let res = validate_safe_repo_relative_path(path, &repo_root);
        assert!(
            res.is_err(),
            "Path '{path}' should have been rejected by validate_safe_repo_relative_path, but got Ok({:?})",
            res.ok()
        );
    }
}

#[test]
fn test_safe_path_validator_valid_paths() {
    let repo_root = get_repo_root();

    let valid_paths = vec![
        "src-tauri/tests/fixtures/pdf/minimal.pdf",
        "src-tauri/tests/fixtures/redaction/simple-text.pdf",
        "src-tauri/tests/fixtures/sanitization/clean-minimal.pdf",
        "src-tauri/fuzz/corpus/minimal.pdf",
        "src-tauri/tests/fixtures/contracts/secure-redaction-contract-v1.json",
    ];

    for path in valid_paths {
        let res = validate_safe_repo_relative_path(path, &repo_root);
        assert!(
            res.is_ok(),
            "Valid path '{path}' was rejected: {:?}",
            res.err()
        );
        let resolved = res.unwrap();
        assert!(
            resolved.is_file(),
            "Resolved path '{:?}' must exist on disk",
            resolved
        );
    }
}

#[test]
fn test_disk_validation_sha_and_byte_length() {
    let temp_dir = TempTestDir::new("disk_validation");
    let repo_root = temp_dir.path();

    // Create a real physical file in temp repo
    let fixture_rel_path = "fixtures/test-sample.pdf";
    let fixture_abs_path = repo_root.join(fixture_rel_path);
    fs::create_dir_all(fixture_abs_path.parent().unwrap()).unwrap();
    let content = b"%PDF-1.4 sample content for hash testing\n";
    fs::write(&fixture_abs_path, content).unwrap();

    let true_len = content.len() as u64;
    let true_sha = format!("{:x}", Sha256::digest(content));

    let base_record = PdfFixtureRecord {
        id: "disk-test-fixture".to_string(),
        path: fixture_rel_path.to_string(),
        sha256: true_sha.clone(),
        byte_length: true_len,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::ProjectAuthored,
        source: Some("Authored".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: None,
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Disk validation test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![base_record.clone()],
    };

    // 1. Correct SHA + correct byteLength + existing file -> PASS
    assert!(validate_manifest(&manifest, repo_root, true).is_ok());

    // 2. Wrong SHA on disk -> FAIL
    let mut bad_sha = manifest.clone();
    bad_sha.fixtures[0].sha256 =
        "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    let res = validate_manifest(&bad_sha, repo_root, true);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("sha256 mismatch")));

    // 3. Wrong byteLength on disk -> FAIL
    let mut bad_len = manifest.clone();
    bad_len.fixtures[0].byte_length = true_len + 10;
    let res = validate_manifest(&bad_len, repo_root, true);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("byteLength mismatch")));

    // 4. Missing file on disk -> FAIL
    let mut missing_file = manifest.clone();
    missing_file.fixtures[0].path = "fixtures/non-existent.pdf".to_string();
    let res = validate_manifest(&missing_file, repo_root, true);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("file not found on disk")));
}

#[test]
fn test_disk_validation_missing_vs_empty_malformed_fixture() {
    let temp_dir = TempTestDir::new("empty_vs_missing");
    let repo_root = temp_dir.path();

    // Create an existing 0-byte physical file
    let empty_rel_path = "fixtures/empty-malformed.pdf";
    let empty_abs_path = repo_root.join(empty_rel_path);
    fs::create_dir_all(empty_abs_path.parent().unwrap()).unwrap();
    fs::write(&empty_abs_path, b"").unwrap();

    let empty_sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string();

    let valid_empty_fixture = PdfFixtureRecord {
        id: "empty-malformed".to_string(),
        path: empty_rel_path.to_string(),
        sha256: empty_sha.clone(),
        byte_length: 0,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::Missing,
        copyright: None,
        source_type: SourceType::Unknown,
        source: Some("Empty malformed test file".to_string()),
        provenance_status: ProvenanceStatus::Partial,
        generator_id: None,
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Malformed, FixtureCategory::Core],
        expected_capabilities: vec![],
        expected_findings: vec!["malformed_empty_file".to_string()],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: true,
    };

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Empty vs Missing test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![valid_empty_fixture.clone()],
    };

    // 1. Existing zero-byte malformed fixture with explicit finding exception -> ACCEPTED
    assert!(validate_manifest(&manifest, repo_root, true).is_ok());

    // 2. Nonexistent malformed fixture with byteLength=0 and allowEmpty=true -> REJECTED (Missing file hard error)
    let mut missing_empty = manifest.clone();
    missing_empty.fixtures[0].path = "fixtures/nonexistent-empty.pdf".to_string();
    let res = validate_manifest(&missing_empty, repo_root, true);
    assert!(
        res.is_err(),
        "Missing empty file must fail disk validation even if allowEmpty == true"
    );
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("file not found on disk")));

    // 3. Nonexistent normal fixture -> REJECTED
    let mut missing_normal = manifest.clone();
    missing_normal.fixtures[0].path = "fixtures/nonexistent-normal.pdf".to_string();
    missing_normal.fixtures[0].byte_length = 500;
    missing_normal.fixtures[0].allow_empty = false;
    missing_normal.fixtures[0].categories = vec![FixtureCategory::Core];
    let res = validate_manifest(&missing_normal, repo_root, true);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("file not found on disk")));

    // 4. Normal fixture with allowEmpty == true -> REJECTED by policy
    let mut normal_allow_empty = manifest.clone();
    normal_allow_empty.fixtures[0].categories = vec![FixtureCategory::Core];
    normal_allow_empty.fixtures[0].allow_empty = true;
    let res = validate_manifest(&normal_allow_empty, repo_root, false);
    assert!(
        res.is_err(),
        "Normal fixture with allowEmpty == true must be rejected"
    );
}

#[test]
#[cfg(unix)]
fn test_symlink_internal_target_rejection() {
    let repo_temp = TempTestDir::new("symlink_internal_repo");
    let real_file = repo_temp.path().join("fixtures/real.pdf");
    fs::create_dir_all(real_file.parent().unwrap()).unwrap();
    fs::write(&real_file, b"%PDF-1.4 internal regular file\n").unwrap();

    let symlink_path = repo_temp.path().join("fixtures/link_internal.pdf");
    std::os::unix::fs::symlink(&real_file, &symlink_path)
        .expect("Create symlink pointing inside repo");

    let res = validate_safe_repo_relative_path("fixtures/link_internal.pdf", repo_temp.path());
    assert!(
        res.is_err(),
        "Symlink pointing to file inside repo root must be rejected"
    );
    let err = res.unwrap_err();
    assert!(
        err.contains("is a symlink"),
        "Expected symlink rejection error, got: {err}"
    );
}

#[test]
#[cfg(unix)]
fn test_symlink_external_target_rejection() {
    let repo_temp = TempTestDir::new("symlink_external_repo");
    let outside_temp = TempTestDir::new("symlink_external_outside");
    let external_file = outside_temp.path().join("external.pdf");
    fs::write(&external_file, b"%PDF-1.4 external file\n").unwrap();

    let symlink_path = repo_temp.path().join("fixtures/link_external.pdf");
    fs::create_dir_all(symlink_path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&external_file, &symlink_path)
        .expect("Create symlink pointing outside repo");

    let res = validate_safe_repo_relative_path("fixtures/link_external.pdf", repo_temp.path());
    assert!(
        res.is_err(),
        "Symlink pointing to file outside repo root must be rejected"
    );
    let err = res.unwrap_err();
    assert!(
        err.contains("is a symlink"),
        "Expected symlink rejection error, got: {err}"
    );
}

#[test]
#[cfg(unix)]
fn test_symlink_parent_directory_escape_rejection() {
    let repo_temp = TempTestDir::new("symlink_parent_repo");
    let outside_temp = TempTestDir::new("symlink_parent_outside");
    let external_file = outside_temp.path().join("secret.pdf");
    fs::write(&external_file, b"%PDF-1.4 external secret\n").unwrap();

    let symlink_dir = repo_temp.path().join("fixtures/symlink_dir");
    fs::create_dir_all(symlink_dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(outside_temp.path(), &symlink_dir)
        .expect("Create directory symlink pointing outside repo");

    let res = validate_safe_repo_relative_path("fixtures/symlink_dir/secret.pdf", repo_temp.path());
    assert!(
        res.is_err(),
        "Parent directory symlink escaping repo root must be rejected"
    );
    let err = res.unwrap_err();
    assert!(
        err.contains("Symlink escapes repo root"),
        "Expected symlink escape error, got: {err}"
    );
}

#[test]
fn test_synthetic_provenance_and_sourcetype_matrix() {
    let repo_root = get_repo_root();

    let base_fixture = PdfFixtureRecord {
        id: "test-fixture".to_string(),
        path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
        sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
        byte_length: 329,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::DeterministicallyGenerated,
        source: Some("Committed generator script".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: Some("py-redaction-generator".to_string()),
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core, FixtureCategory::Baseline],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Synthetic test".to_string(),
        generators: HashMap::from([
            (
                "py-redaction-generator".to_string(),
                GeneratorRecord {
                    kind: GeneratorKind::CommittedScript,
                    path: Some(
                        "src-tauri/tests/fixtures/redaction/generate_redaction_fixtures.py"
                            .to_string(),
                    ),
                    language: "python3".to_string(),
                    dependencies: vec![],
                    network_requirement: NetworkRequirement::None,
                    reproducibility_status: ReproducibilityStatus::DeterministicByConstruction,
                    notes: None,
                },
            ),
            (
                "pypdf-historical-method".to_string(),
                GeneratorRecord {
                    kind: GeneratorKind::HistoricalDocumentedMethod,
                    path: None,
                    language: "python3".to_string(),
                    dependencies: vec!["pypdf".to_string()],
                    network_requirement: NetworkRequirement::None,
                    reproducibility_status: ReproducibilityStatus::ReproducibilityUnverified,
                    notes: None,
                },
            ),
        ]),
        fixtures: vec![base_fixture.clone()],
    };

    // 1. Valid baseline should pass
    assert!(validate_manifest(&manifest, &repo_root, false).is_ok());

    // 2. DeterministicallyGenerated missing generator_id -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].generator_id = None;
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 3. DeterministicallyGenerated referencing unknown generator -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].generator_id = Some("unknown-gen".to_string());
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 4. DeterministicallyGenerated referencing historical method -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].generator_id = Some("pypdf-historical-method".to_string());
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 5. Generated with neither generator nor source -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].source_type = SourceType::Generated;
    bad.fixtures[0].generator_id = None;
    bad.fixtures[0].source = None;
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 6. ProjectAuthored with partial provenance -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].source_type = SourceType::ProjectAuthored;
    bad.fixtures[0].provenance_status = ProvenanceStatus::Partial;
    bad.fixtures[0].source = Some("Authored by project".to_string());
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 7. ProjectAuthored with None source -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].source_type = SourceType::ProjectAuthored;
    bad.fixtures[0].provenance_status = ProvenanceStatus::Complete;
    bad.fixtures[0].source = None;
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 8. Unknown source_type with Complete provenance -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].source_type = SourceType::Unknown;
    bad.fixtures[0].provenance_status = ProvenanceStatus::Complete;
    bad.fixtures[0].source = Some("Unknown".to_string());
    assert!(validate_manifest(&bad, &repo_root, false).is_err());
}

#[test]
fn test_synthetic_license_invariants_matrix() {
    let repo_root = get_repo_root();

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Synthetic test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![PdfFixtureRecord {
            id: "test-license".to_string(),
            path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
            sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
            byte_length: 329,
            pdf_version: "1.4".to_string(),
            license: None,
            license_status: LicenseStatus::RepositoryLicenseReference,
            copyright: None,
            source_type: SourceType::ProjectAuthored,
            source: Some("Authored by team".to_string()),
            provenance_status: ProvenanceStatus::Complete,
            generator_id: None,
            generated_from: None,
            encrypted: false,
            password_reference: None,
            categories: vec![FixtureCategory::Core],
            expected_capabilities: vec!["has_text".to_string()],
            expected_findings: vec![],
            allowed_tests: vec![TestKind::Integration],
            feature_manifest: None,
            notes: None,
            allow_empty: false,
        }],
    };

    // 1. RepositoryLicenseReference with license = None -> PASS
    assert!(validate_manifest(&manifest, &repo_root, false).is_ok());

    // 2. RepositoryLicenseReference with license = Some("MIT") -> FAIL (must be None)
    let mut bad = manifest.clone();
    bad.fixtures[0].license = Some("MIT".to_string());
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 3. Known with license = Some("Apache-2.0") -> PASS
    let mut known = manifest.clone();
    known.fixtures[0].license_status = LicenseStatus::Known;
    known.fixtures[0].license = Some("Apache-2.0".to_string());
    assert!(validate_manifest(&known, &repo_root, false).is_ok());

    // 4. Known with license = None -> FAIL
    let mut bad_known = manifest.clone();
    bad_known.fixtures[0].license_status = LicenseStatus::Known;
    bad_known.fixtures[0].license = None;
    assert!(validate_manifest(&bad_known, &repo_root, false).is_err());

    // 5. Missing with license = Some("MIT") -> FAIL (no fabricated license strings)
    let mut bad_missing = manifest.clone();
    bad_missing.fixtures[0].license_status = LicenseStatus::Missing;
    bad_missing.fixtures[0].license = Some("MIT".to_string());
    assert!(validate_manifest(&bad_missing, &repo_root, false).is_err());

    // 6. Missing with license = None -> PASS
    let mut good_missing = manifest.clone();
    good_missing.fixtures[0].license_status = LicenseStatus::Missing;
    good_missing.fixtures[0].license = None;
    assert!(validate_manifest(&good_missing, &repo_root, false).is_ok());

    // 7. NotApplicable with license = Some("CC0-1.0") -> FAIL
    let mut bad_na = manifest.clone();
    bad_na.fixtures[0].license_status = LicenseStatus::NotApplicable;
    bad_na.fixtures[0].license = Some("CC0-1.0".to_string());
    assert!(validate_manifest(&bad_na, &repo_root, false).is_err());
}

#[test]
fn test_synthetic_allow_empty_and_malformed_matrix() {
    let repo_root = get_repo_root();

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Synthetic test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![PdfFixtureRecord {
            id: "test-empty".to_string(),
            path: "src-tauri/tests/fixtures/pdf/malformed.pdf".to_string(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            byte_length: 0,
            pdf_version: "1.4".to_string(),
            license: None,
            license_status: LicenseStatus::Missing,
            copyright: None,
            source_type: SourceType::Unknown,
            source: Some("Empty malformed test artifact".to_string()),
            provenance_status: ProvenanceStatus::Partial,
            generator_id: None,
            generated_from: None,
            encrypted: false,
            password_reference: None,
            categories: vec![FixtureCategory::Malformed, FixtureCategory::Core],
            expected_capabilities: vec![],
            expected_findings: vec!["malformed_empty_file".to_string()],
            allowed_tests: vec![TestKind::Integration],
            feature_manifest: None,
            notes: None,
            allow_empty: true,
        }],
    };

    // 1. Valid empty malformed fixture with category Malformed and finding token -> PASS
    assert!(validate_manifest(&manifest, &repo_root, false).is_ok());

    // 2. byte_length == 0 but missing Malformed category -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].categories = vec![FixtureCategory::Core];
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 3. byte_length == 0, Malformed present, but expected_findings is empty -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].expected_findings = vec![];
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 4. allow_empty == true on non-malformed fixture -> FAIL
    let mut normal = manifest.clone();
    normal.fixtures[0].byte_length = 100;
    normal.fixtures[0].allow_empty = true;
    normal.fixtures[0].categories = vec![FixtureCategory::Core];
    assert!(validate_manifest(&normal, &repo_root, false).is_err());

    // 5. Normal non-malformed file with byte_length > 0 and allow_empty == false -> PASS
    let mut normal_pass = manifest.clone();
    normal_pass.fixtures[0].byte_length = 500;
    normal_pass.fixtures[0].allow_empty = false;
    normal_pass.fixtures[0].categories = vec![FixtureCategory::Core, FixtureCategory::Baseline];
    normal_pass.fixtures[0].expected_findings = vec![];
    assert!(validate_manifest(&normal_pass, &repo_root, false).is_ok());
}

#[test]
fn test_synthetic_token_format_validation() {
    let valid_tokens = vec![
        "render_page",
        "text_extraction",
        "malformed_empty_file",
        "stream_xref",
        "page_1",
        "p0",
        "a",
        "complex_capability_token_123",
    ];

    for token in valid_tokens {
        assert!(
            validate_token_format(token),
            "Token '{token}' should be valid"
        );
    }

    let invalid_tokens = vec![
        "",
        "RenderPage",
        "render-page",
        "render page",
        "render_page!",
        "render.page",
        "_leading_underscore",
        "trailing_underscore_",
        "double__underscore",
    ];

    for token in invalid_tokens {
        assert!(
            !validate_token_format(token),
            "Token '{token}' should be invalid"
        );
    }
}

#[test]
fn test_synthetic_password_security_and_encryption_invariants() {
    let repo_root = get_repo_root();

    let valid_creds = vec![
        "test-credential:user-pass",
        "test-credential:owner-key-1",
        "test-credential:test-password-123",
    ];

    for cred in valid_creds {
        assert!(
            validate_password_reference(cred),
            "Credential reference '{cred}' should be valid"
        );
    }

    let invalid_creds = vec![
        "",
        "fixture-password",
        "secret123",
        "test-credential:",
        "test-credential:UPPERCASE",
        "test-credential:has_underscore",
        "test-credential:has space",
    ];

    for cred in invalid_creds {
        assert!(
            !validate_password_reference(cred),
            "Credential reference '{cred}' should be invalid (never raw passwords)"
        );
    }

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Synthetic test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![PdfFixtureRecord {
            id: "test-enc".to_string(),
            path: "src-tauri/tests/fixtures/pdf/encrypted.pdf".to_string(),
            sha256: "4a07c1b65793257cf412dc5f478ea84525f07e94705bbbb084bb427cd241f28d".to_string(),
            byte_length: 807,
            pdf_version: "1.4".to_string(),
            license: None,
            license_status: LicenseStatus::RepositoryLicenseReference,
            copyright: None,
            source_type: SourceType::ProjectAuthored,
            source: Some("Encrypted fixture".to_string()),
            provenance_status: ProvenanceStatus::Complete,
            generator_id: None,
            generated_from: None,
            encrypted: true,
            password_reference: Some("test-credential:user-pass".to_string()),
            categories: vec![
                FixtureCategory::Core,
                FixtureCategory::Security,
                FixtureCategory::Encrypted,
            ],
            expected_capabilities: vec!["has_text".to_string()],
            expected_findings: vec![],
            allowed_tests: vec![TestKind::Integration],
            feature_manifest: None,
            notes: None,
            allow_empty: false,
        }],
    };

    // 1. Valid encrypted fixture -> PASS
    assert!(validate_manifest(&manifest, &repo_root, false).is_ok());

    // 2. Encrypted == true but missing Encrypted category -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].categories = vec![FixtureCategory::Core, FixtureCategory::Security];
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 3. Encrypted == false but password_reference is Some -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].encrypted = false;
    assert!(validate_manifest(&bad, &repo_root, false).is_err());

    // 4. Raw password string in password_reference -> FAIL
    let mut bad = manifest.clone();
    bad.fixtures[0].password_reference = Some("my-raw-password".to_string());
    assert!(validate_manifest(&bad, &repo_root, false).is_err());
}

fn get_valid_synthetic_manifest_value() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": "1.0.0",
        "corpusVersion": "1.0.0",
        "description": "Synthetic valid manifest for Serde testing",
        "generators": {
            "test-generator": {
                "kind": "committed_script",
                "path": "src-tauri/tests/fixtures/pdf/generate_reorder_fixtures.py",
                "language": "python3",
                "dependencies": ["pypdf"],
                "networkRequirement": "none",
                "reproducibilityStatus": "deterministic_by_construction",
                "notes": null
            }
        },
        "fixtures": [
            {
                "id": "synthetic-fixture",
                "path": "src-tauri/tests/fixtures/pdf/minimal.pdf",
                "sha256": "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d",
                "byteLength": 329,
                "pdfVersion": "1.4",
                "license": null,
                "licenseStatus": "repository_license_reference",
                "copyright": null,
                "sourceType": "project_authored",
                "source": "Synthetic fixture test description",
                "provenanceStatus": "complete",
                "generatorId": null,
                "generatedFrom": null,
                "encrypted": false,
                "passwordReference": null,
                "categories": ["core", "baseline"],
                "expectedCapabilities": ["has_text"],
                "expectedFindings": [],
                "allowedTests": ["integration", "unit"],
                "featureManifest": null,
                "notes": null,
                "allowEmpty": false
            }
        ]
    })
}

#[test]
fn test_serde_roundtrip_and_unknown_fields_rejection() {
    let repo_root = get_repo_root();
    let manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");
    let raw_bytes = fs::read(&manifest_path).expect("Read canonical manifest");

    // 1. Canonical manifest roundtrip
    let manifest: PdfFixtureManifest = serde_json::from_slice(&raw_bytes).unwrap();
    let reserialized = serde_json::to_string_pretty(&manifest).unwrap();
    let roundtripped: PdfFixtureManifest = serde_json::from_str(&reserialized).unwrap();
    assert_eq!(manifest, roundtripped);

    // 2. Exact valid synthetic manifest with full fixture deserialization & roundtrip
    let synthetic_val = get_valid_synthetic_manifest_value();
    let synthetic_manifest: PdfFixtureManifest = serde_json::from_value(synthetic_val.clone())
        .expect("Valid synthetic manifest with fixture record must deserialize strictly");
    assert_eq!(synthetic_manifest.fixtures.len(), 1);
    assert_eq!(synthetic_manifest.fixtures[0].id, "synthetic-fixture");
    let synthetic_str = serde_json::to_string_pretty(&synthetic_manifest).unwrap();
    let synthetic_roundtrip: PdfFixtureManifest = serde_json::from_str(&synthetic_str).unwrap();
    assert_eq!(synthetic_manifest, synthetic_roundtrip);

    // 3. Rejection of unknown fields at root
    let mut unknown_root = synthetic_val.clone();
    unknown_root["unknownRootField"] = serde_json::Value::String("forbidden".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(unknown_root).is_err(),
        "Must reject unknown field at root level"
    );

    // 4. Rejection of unknown fields inside generator
    let mut unknown_gen = synthetic_val.clone();
    unknown_gen["generators"]["test-generator"]["unknownGenField"] = serde_json::Value::Bool(true);
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(unknown_gen).is_err(),
        "Must reject unknown field inside generator record"
    );

    // 5. Rejection of unknown fields inside fixture record (PdfFixtureRecord deny_unknown_fields)
    let mut unknown_fixture = synthetic_val;
    unknown_fixture["fixtures"][0]["unknownFixtureField"] = serde_json::Value::Bool(true);
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(unknown_fixture).is_err(),
        "Must reject unknown field inside fixture record"
    );
}

#[test]
fn test_serde_unknown_enum_values_rejection() {
    let valid_base = get_valid_synthetic_manifest_value();

    // 1. Unknown sourceType in fixture
    let mut bad_source_type = valid_base.clone();
    bad_source_type["fixtures"][0]["sourceType"] =
        serde_json::Value::String("future_source".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_source_type).is_err(),
        "Unknown sourceType 'future_source' must fail deserialization"
    );

    // 2. Unknown licenseStatus in fixture
    let mut bad_license_status = valid_base.clone();
    bad_license_status["fixtures"][0]["licenseStatus"] =
        serde_json::Value::String("mystery_license".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_license_status).is_err(),
        "Unknown licenseStatus 'mystery_license' must fail deserialization"
    );

    // 3. Unknown provenanceStatus in fixture
    let mut bad_provenance = valid_base.clone();
    bad_provenance["fixtures"][0]["provenanceStatus"] =
        serde_json::Value::String("verified_somehow".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_provenance).is_err(),
        "Unknown provenanceStatus 'verified_somehow' must fail deserialization"
    );

    // 4. Unknown category in fixture categories array
    let mut bad_categories = valid_base.clone();
    bad_categories["fixtures"][0]["categories"] = serde_json::json!(["core", "future_category"]);
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_categories).is_err(),
        "Unknown category 'future_category' must fail deserialization"
    );

    // 5. Unknown allowedTests value in fixture allowedTests array
    let mut bad_allowed_tests = valid_base.clone();
    bad_allowed_tests["fixtures"][0]["allowedTests"] =
        serde_json::json!(["integration", "future_test"]);
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_allowed_tests).is_err(),
        "Unknown test kind 'future_test' must fail deserialization"
    );

    // 6. Unknown generator kind
    let mut bad_gen_kind = valid_base.clone();
    bad_gen_kind["generators"]["test-generator"]["kind"] =
        serde_json::Value::String("external_binary".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_gen_kind).is_err(),
        "Unknown generator kind 'external_binary' must fail deserialization"
    );

    // 7. Unknown generator reproducibilityStatus
    let mut bad_repro = valid_base.clone();
    bad_repro["generators"]["test-generator"]["reproducibilityStatus"] =
        serde_json::Value::String("magic".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_repro).is_err(),
        "Unknown reproducibilityStatus 'magic' must fail deserialization"
    );

    // 8. Unknown generator networkRequirement
    let mut bad_net = valid_base;
    bad_net["generators"]["test-generator"]["networkRequirement"] =
        serde_json::Value::String("future_network".to_string());
    assert!(
        serde_json::from_value::<PdfFixtureManifest>(bad_net).is_err(),
        "Unknown networkRequirement 'future_network' must fail deserialization"
    );
}

#[test]
fn test_duplicate_detection() {
    let repo_root = get_repo_root();

    let fixture1 = PdfFixtureRecord {
        id: "fixture-1".to_string(),
        path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
        sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
        byte_length: 329,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::ProjectAuthored,
        source: Some("Authored".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: None,
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };

    // 1. Duplicate ID -> REJECTED
    let mut duplicate_id = fixture1.clone();
    duplicate_id.id = "fixture-1".to_string();
    duplicate_id.path = "src-tauri/tests/fixtures/pdf/annotations.pdf".to_string();

    let manifest_dup_id = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Duplicate ID test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![fixture1.clone(), duplicate_id],
    };
    let res = validate_manifest(&manifest_dup_id, &repo_root, false);
    assert!(res.is_err(), "Duplicate fixture ID must be rejected");
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("Duplicate fixture ID")));

    // 2. Exact Duplicate Path -> REJECTED
    let mut duplicate_exact_path = fixture1.clone();
    duplicate_exact_path.id = "fixture-2".to_string();
    duplicate_exact_path.path = "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string();

    let manifest_dup_exact_path = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Exact duplicate path test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![fixture1.clone(), duplicate_exact_path],
    };
    let res = validate_manifest(&manifest_dup_exact_path, &repo_root, false);
    assert!(res.is_err(), "Exact duplicate path must be rejected");
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("Duplicate fixture path")));

    // 3. Non-canonical backslash separator path -> REJECTED directly
    let mut duplicate_slash_path = fixture1.clone();
    duplicate_slash_path.id = "fixture-3".to_string();
    duplicate_slash_path.path = r"src-tauri\tests\fixtures\pdf\minimal.pdf".to_string();

    let manifest_dup_slash_path = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Non-canonical slash rejection test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![fixture1.clone(), duplicate_slash_path],
    };
    let res = validate_manifest(&manifest_dup_slash_path, &repo_root, false);
    assert!(
        res.is_err(),
        "Path with '\\' must be rejected as non-canonical path separator"
    );
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("Non-canonical path separator")));

    // 4. Genuinely different normalized paths -> ACCEPTED
    let mut diff_path = fixture1.clone();
    diff_path.id = "fixture-diff".to_string();
    diff_path.path = "src-tauri/tests/fixtures/pdf/annotations.pdf".to_string();
    diff_path.sha256 =
        "1111111111111111111111111111111111111111111111111111111111111111".to_string();

    let manifest_diff_paths = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Different normalized paths test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![fixture1.clone(), diff_path],
    };
    assert!(
        validate_manifest(&manifest_diff_paths, &repo_root, false).is_ok(),
        "Genuinely different paths must be accepted"
    );

    // 5. Invalid traversal path -> REJECTED by safe-path validation
    let mut traversal_fixture = fixture1.clone();
    traversal_fixture.id = "fixture-traversal".to_string();
    traversal_fixture.path = "src-tauri/tests/fixtures/../../outside.pdf".to_string();
    traversal_fixture.sha256 =
        "2222222222222222222222222222222222222222222222222222222222222222".to_string();

    let manifest_traversal = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Traversal path test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![fixture1, traversal_fixture],
    };
    let res = validate_manifest(&manifest_traversal, &repo_root, false);
    assert!(
        res.is_err(),
        "Traversal path must be rejected by safe-path validation"
    );
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("traversal forbidden")));
}

#[test]
fn test_duplicate_sha256_detection() {
    let repo_root = get_repo_root();

    let fixture1 = PdfFixtureRecord {
        id: "fixture-a".to_string(),
        path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
        sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
        byte_length: 329,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::ProjectAuthored,
        source: Some("Authored".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: None,
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };

    let mut fixture2 = fixture1.clone();
    fixture2.id = "fixture-b".to_string();
    fixture2.path = "src-tauri/tests/fixtures/pdf/annotations.pdf".to_string();

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Duplicate SHA test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![fixture1, fixture2],
    };

    let res = validate_manifest(&manifest, &repo_root, false);
    assert!(res.is_err(), "Duplicate SHA-256 must be rejected");
    let errs = res.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("Duplicate SHA-256")
            && e.contains("fixture-a")
            && e.contains("fixture-b")
            && e.contains("src-tauri/tests/fixtures/pdf/minimal.pdf")
            && e.contains("src-tauri/tests/fixtures/pdf/annotations.pdf")
            && e.contains("0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d")),
        "Error message should contain diagnostic with IDs, paths, and shared SHA: {errs:?}"
    );
}

#[test]
fn test_adversarial_second_release_smoke_rejected() {
    let repo_root = get_repo_root();

    let smoke1 = PdfFixtureRecord {
        id: "core-release-smoke".to_string(),
        path: "src-tauri/tests/fixtures/pdf/release-smoke.pdf".to_string(),
        sha256: "727488fe32189a68a176ec32de5a3194d2b1e370a3fac2d5930871f45ba20611".to_string(),
        byte_length: 4520,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::DeterministicallyGenerated,
        source: Some("Release smoke fixture".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: Some("py-release-smoke-generator".to_string()),
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core, FixtureCategory::ReleaseSmoke],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Smoke, TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };

    let mut smoke2 = smoke1.clone();
    smoke2.id = "secondary-release-smoke".to_string();
    smoke2.path = "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string();
    smoke2.sha256 = "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string();
    smoke2.byte_length = 329;

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Multiple release-smoke test".to_string(),
        generators: HashMap::from([(
            "py-release-smoke-generator".to_string(),
            GeneratorRecord {
                kind: GeneratorKind::CommittedScript,
                path: Some("src-tauri/tests/fixtures/pdf/generate_release_smoke.py".to_string()),
                language: "python3".to_string(),
                dependencies: vec![],
                network_requirement: NetworkRequirement::None,
                reproducibility_status: ReproducibilityStatus::DeterministicByConstruction,
                notes: None,
            },
        )]),
        fixtures: vec![smoke1, smoke2],
    };

    let res = validate_manifest(&manifest, &repo_root, false);
    assert!(
        res.is_err(),
        "Manifest with second fixture having category release_smoke must be rejected"
    );
    let errs = res.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("release_smoke")),
        "Expected release_smoke rejection error, got: {errs:?}"
    );
}

#[test]
fn test_corpus_roots_enumeration_and_no_unmanifested_pdf() {
    let repo_root = get_repo_root();
    let manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");
    let raw_bytes = fs::read(&manifest_path).expect("Read canonical manifest");
    let manifest: PdfFixtureManifest = serde_json::from_slice(&raw_bytes).unwrap();

    let mut declared_paths: Vec<String> = manifest
        .fixtures
        .iter()
        .map(|f| f.path.replace('\\', "/"))
        .collect();
    declared_paths.sort();
    declared_paths.dedup();

    assert_eq!(
        declared_paths.len(),
        57,
        "Expected 57 unique declared fixture paths in canonical manifest"
    );

    let corpus_roots = [
        repo_root.join("src-tauri/tests/fixtures"),
        repo_root.join("src-tauri/fuzz/corpus"),
    ];

    let mut disk_paths: Vec<String> = Vec::new();

    fn walk_dir_recursively(dir: &Path, repo_root: &Path, disk_paths: &mut Vec<String>) {
        if !dir.is_dir() {
            return;
        }
        let entries = fs::read_dir(dir).expect("Read dir entries");
        for entry in entries {
            let entry = entry.expect("Valid entry");
            let path = entry.path();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();

            // macOS AppleDouble / hidden file sidecar exclusion (skip deterministically, do not panic)
            if file_name.starts_with("._") && file_name.ends_with(".pdf") {
                continue;
            }

            if path.is_dir() {
                walk_dir_recursively(&path, repo_root, disk_paths);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("pdf") {
                        let rel = path
                            .strip_prefix(repo_root)
                            .expect("Strip repo root prefix")
                            .to_string_lossy()
                            .replace('\\', "/");
                        disk_paths.push(rel);
                    }
                }
            }
        }
    }

    for root in &corpus_roots {
        walk_dir_recursively(root, &repo_root, &mut disk_paths);
    }

    disk_paths.sort();

    assert_eq!(
        disk_paths.len(),
        57,
        "Found {} PDF files on disk across corpus roots, expected 57",
        disk_paths.len()
    );

    assert_eq!(
        declared_paths, disk_paths,
        "Declared manifest paths must exactly match disk PDF files (0 unmanifested PDF, 0 missing)"
    );
}

#[test]
fn test_model_b_feature_manifest_parity_bridge() {
    let repo_root = get_repo_root();
    let central_manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");
    let central_bytes = fs::read(&central_manifest_path).expect("Read canonical manifest");
    let central_manifest: PdfFixtureManifest = serde_json::from_slice(&central_bytes).unwrap();

    // 1. Redaction Feature Manifest Parity
    let redaction_manifest_path =
        repo_root.join("src-tauri/tests/fixtures/redaction/manifest.json");
    assert!(
        redaction_manifest_path.is_file(),
        "Redaction manifest must exist"
    );

    #[derive(Deserialize)]
    struct RedactionManifest {
        fixtures: Vec<RedactionFixtureEntry>,
    }

    #[derive(Deserialize)]
    struct RedactionFixtureEntry {
        filename: String,
        sha256: String,
        byte_size: u64,
    }

    let redaction_bytes = fs::read(&redaction_manifest_path).expect("Read redaction manifest");
    let redaction_manifest: RedactionManifest =
        serde_json::from_slice(&redaction_bytes).expect("Deserialize redaction manifest");

    assert_eq!(
        redaction_manifest.fixtures.len(),
        26,
        "Redaction manifest must have exactly 26 fixtures"
    );

    for r_entry in &redaction_manifest.fixtures {
        let matching_central = central_manifest.fixtures.iter().find(|f| {
            f.path
                .replace('\\', "/")
                .starts_with("src-tauri/tests/fixtures/redaction/")
                && Path::new(&f.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(&r_entry.filename)
        });

        assert!(
            matching_central.is_some(),
            "Redaction fixture '{}' not found in central manifest",
            r_entry.filename
        );

        let central_rec = matching_central.unwrap();
        assert_eq!(
            central_rec.feature_manifest,
            Some("src-tauri/tests/fixtures/redaction/manifest.json".to_string()),
            "Fixture '{}' featureManifest mismatch",
            r_entry.filename
        );
        assert_eq!(
            central_rec.sha256, r_entry.sha256,
            "Fixture '{}' SHA-256 mismatch between central and redaction manifest",
            r_entry.filename
        );
        assert_eq!(
            central_rec.byte_length, r_entry.byte_size,
            "Fixture '{}' byte length mismatch between central and redaction manifest",
            r_entry.filename
        );
    }

    // 2. Sanitization Feature Manifest Parity
    let sanitization_manifest_path =
        repo_root.join("src-tauri/tests/fixtures/sanitization/manifest.json");
    assert!(
        sanitization_manifest_path.is_file(),
        "Sanitization manifest must exist"
    );

    #[derive(Deserialize)]
    struct SanitizationManifest {
        fixtures: Vec<SanitizationFixtureEntry>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SanitizationFixtureEntry {
        name: String,
        sha256: String,
        byte_size: u64,
    }

    let sanitization_bytes =
        fs::read(&sanitization_manifest_path).expect("Read sanitization manifest");
    let sanitization_manifest: SanitizationManifest =
        serde_json::from_slice(&sanitization_bytes).expect("Deserialize sanitization manifest");

    assert_eq!(
        sanitization_manifest.fixtures.len(),
        16,
        "Sanitization manifest must have exactly 16 fixtures"
    );

    for s_entry in &sanitization_manifest.fixtures {
        let matching_central = central_manifest.fixtures.iter().find(|f| {
            f.path
                .replace('\\', "/")
                .starts_with("src-tauri/tests/fixtures/sanitization/")
                && Path::new(&f.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(&s_entry.name)
        });

        assert!(
            matching_central.is_some(),
            "Sanitization fixture '{}' not found in central manifest",
            s_entry.name
        );

        let central_rec = matching_central.unwrap();
        assert_eq!(
            central_rec.feature_manifest,
            Some("src-tauri/tests/fixtures/sanitization/manifest.json".to_string()),
            "Fixture '{}' featureManifest mismatch",
            s_entry.name
        );
        assert_eq!(
            central_rec.sha256, s_entry.sha256,
            "Fixture '{}' SHA-256 mismatch between central and sanitization manifest",
            s_entry.name
        );
        assert_eq!(
            central_rec.byte_length, s_entry.byte_size,
            "Fixture '{}' byte length mismatch between central and sanitization manifest",
            s_entry.name
        );
    }
}

#[test]
fn test_release_smoke_and_large_dimensions_fixtures_integrity() {
    let repo_root = get_repo_root();
    let central_manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");
    let central_bytes = fs::read(&central_manifest_path).expect("Read canonical manifest");
    let central_manifest: PdfFixtureManifest = serde_json::from_slice(&central_bytes).unwrap();

    // 1. Release smoke fixture assertions: exactly one canonical fixture carrying release_smoke category
    let release_smoke_matches: Vec<&PdfFixtureRecord> = central_manifest
        .fixtures
        .iter()
        .filter(|f| f.categories.contains(&FixtureCategory::ReleaseSmoke))
        .collect();
    assert_eq!(
        release_smoke_matches.len(),
        1,
        "Manifest must contain exactly one fixture carrying category 'release_smoke', found {}",
        release_smoke_matches.len()
    );

    let smoke_fixture = release_smoke_matches[0];
    assert_eq!(smoke_fixture.id, "core-release-smoke");
    assert_eq!(
        smoke_fixture.path,
        "src-tauri/tests/fixtures/pdf/release-smoke.pdf"
    );
    assert_eq!(
        smoke_fixture.source_type,
        SourceType::DeterministicallyGenerated
    );
    assert_eq!(
        smoke_fixture.generator_id,
        Some("py-release-smoke-generator".to_string())
    );
    assert!(smoke_fixture.categories.contains(&FixtureCategory::Core));
    assert!(smoke_fixture
        .categories
        .contains(&FixtureCategory::ReleaseSmoke));
    assert_eq!(
        smoke_fixture.expected_capabilities,
        vec![
            "has_text".to_string(),
            "has_unicode_text".to_string(),
            "has_vector_content".to_string(),
            "multi_page".to_string(),
        ]
    );
    assert!(!smoke_fixture.allow_empty);
    assert!(smoke_fixture.allowed_tests.contains(&TestKind::Smoke));

    let smoke_disk_bytes =
        fs::read(repo_root.join(&smoke_fixture.path)).expect("Read release-smoke.pdf from disk");
    assert_eq!(smoke_disk_bytes.len() as u64, smoke_fixture.byte_length);
    assert_eq!(
        format!("{:x}", Sha256::digest(&smoke_disk_bytes)),
        smoke_fixture.sha256
    );

    // 2. Large page dimensions fixture assertions
    let large_fixture = central_manifest
        .fixtures
        .iter()
        .find(|f| f.id == "core-large-page-dimensions")
        .expect("core-large-page-dimensions must exist in manifest");

    assert_eq!(
        large_fixture.path,
        "src-tauri/tests/fixtures/pdf/large-page-dimensions.pdf"
    );
    assert_eq!(
        large_fixture.source_type,
        SourceType::DeterministicallyGenerated
    );
    assert_eq!(
        large_fixture.generator_id,
        Some("py-release-smoke-generator".to_string())
    );
    assert!(large_fixture.categories.contains(&FixtureCategory::Core));
    assert!(large_fixture
        .categories
        .contains(&FixtureCategory::DocumentStructure));
    assert_eq!(
        large_fixture.expected_capabilities,
        vec!["large_page_dimensions".to_string()]
    );
    assert!(!large_fixture.allow_empty);

    let large_disk_bytes = fs::read(repo_root.join(&large_fixture.path))
        .expect("Read large-page-dimensions.pdf from disk");
    assert_eq!(large_disk_bytes.len() as u64, large_fixture.byte_length);
    assert_eq!(
        format!("{:x}", Sha256::digest(&large_disk_bytes)),
        large_fixture.sha256
    );
}

#[test]
fn test_expected_capabilities_bounded_allowlist_and_uniqueness() {
    let repo_root = get_repo_root();

    let base_record = PdfFixtureRecord {
        id: "cap-test-fixture".to_string(),
        path: "src-tauri/tests/fixtures/pdf/minimal.pdf".to_string(),
        sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
        byte_length: 329,
        pdf_version: "1.4".to_string(),
        license: None,
        license_status: LicenseStatus::RepositoryLicenseReference,
        copyright: None,
        source_type: SourceType::ProjectAuthored,
        source: Some("Authored".to_string()),
        provenance_status: ProvenanceStatus::Complete,
        generator_id: None,
        generated_from: None,
        encrypted: false,
        password_reference: None,
        categories: vec![FixtureCategory::Core],
        expected_capabilities: vec!["has_text".to_string()],
        expected_findings: vec![],
        allowed_tests: vec![TestKind::Integration],
        feature_manifest: None,
        notes: None,
        allow_empty: false,
    };

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "Capability test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![base_record.clone()],
    };

    // 1. All valid canonical tokens pass individually
    for &cap in CANONICAL_EXPECTED_CAPABILITIES {
        let mut valid_manifest = manifest.clone();
        valid_manifest.fixtures[0].expected_capabilities = vec![cap.to_string()];
        assert!(
            validate_manifest(&valid_manifest, &repo_root, false).is_ok(),
            "Canonical capability '{cap}' should be accepted"
        );
    }

    // 2. Non-canonical capability token -> REJECTED
    let invalid_caps = vec![
        "has_text2",
        "render_page",
        "arbitrary_capability",
        "decrypt_rc4",
        "",
    ];
    for bad_cap in invalid_caps {
        let mut bad_manifest = manifest.clone();
        bad_manifest.fixtures[0].expected_capabilities = vec![bad_cap.to_string()];
        let res = validate_manifest(&bad_manifest, &repo_root, false);
        assert!(
            res.is_err(),
            "Non-canonical capability '{bad_cap}' should be rejected"
        );
    }

    // 3. Duplicate capability tokens -> REJECTED
    let mut dup_cap_manifest = manifest.clone();
    dup_cap_manifest.fixtures[0].expected_capabilities =
        vec!["has_text".to_string(), "has_text".to_string()];
    let res = validate_manifest(&dup_cap_manifest, &repo_root, false);
    assert!(res.is_err(), "Duplicate capability token must be rejected");
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("duplicate expectedCapabilities")));

    // 4. Duplicate categories -> REJECTED
    let mut dup_cat_manifest = manifest.clone();
    dup_cat_manifest.fixtures[0].categories = vec![FixtureCategory::Core, FixtureCategory::Core];
    let res = validate_manifest(&dup_cat_manifest, &repo_root, false);
    assert!(res.is_err(), "Duplicate category must be rejected");
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("duplicate category")));

    // 5. Duplicate findings -> REJECTED
    let mut dup_finding_manifest = manifest.clone();
    dup_finding_manifest.fixtures[0].categories =
        vec![FixtureCategory::Malformed, FixtureCategory::Core];
    dup_finding_manifest.fixtures[0].allow_empty = true;
    dup_finding_manifest.fixtures[0].byte_length = 0;
    dup_finding_manifest.fixtures[0].expected_findings = vec![
        "malformed_empty_file".to_string(),
        "malformed_empty_file".to_string(),
    ];
    let res = validate_manifest(&dup_finding_manifest, &repo_root, false);
    assert!(res.is_err(), "Duplicate finding token must be rejected");
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("duplicate expectedFindings")));

    // 6. Duplicate allowedTests -> REJECTED
    let mut dup_tests_manifest = manifest.clone();
    dup_tests_manifest.fixtures[0].allowed_tests =
        vec![TestKind::Integration, TestKind::Integration];
    let res = validate_manifest(&dup_tests_manifest, &repo_root, false);
    assert!(res.is_err(), "Duplicate allowedTests must be rejected");
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("duplicate allowedTests")));
}

#[test]
fn test_appledouble_manifest_rejection_and_scanner_skip() {
    let temp_dir = TempTestDir::new("appledouble_test");
    let repo_root = temp_dir.path();

    // Create a real PDF fixture and an AppleDouble sidecar
    let fixtures_dir = repo_root.join("src-tauri/tests/fixtures/pdf");
    fs::create_dir_all(&fixtures_dir).unwrap();

    let real_pdf = fixtures_dir.join("minimal.pdf");
    let sidecar_pdf = fixtures_dir.join("._minimal.pdf");

    let content = b"%PDF-1.4 sample content\n";
    fs::write(&real_pdf, content).unwrap();
    fs::write(&sidecar_pdf, b"Mac OS X resource fork metadata").unwrap();

    // 1. Filesystem scanner must skip ._minimal.pdf and include only minimal.pdf
    let mut disk_paths: Vec<String> = Vec::new();

    fn walk_dir_recursively(dir: &Path, repo_root: &Path, disk_paths: &mut Vec<String>) {
        if !dir.is_dir() {
            return;
        }
        let entries = fs::read_dir(dir).expect("Read dir entries");
        for entry in entries {
            let entry = entry.expect("Valid entry");
            let path = entry.path();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();

            // macOS AppleDouble / hidden file sidecar exclusion (skip deterministically, do not panic)
            if file_name.starts_with("._") && file_name.ends_with(".pdf") {
                continue;
            }

            if path.is_dir() {
                walk_dir_recursively(&path, repo_root, disk_paths);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("pdf") {
                        let rel = path
                            .strip_prefix(repo_root)
                            .expect("Strip repo root prefix")
                            .to_string_lossy()
                            .replace('\\', "/");
                        disk_paths.push(rel);
                    }
                }
            }
        }
    }

    walk_dir_recursively(
        &repo_root.join("src-tauri/tests/fixtures"),
        repo_root,
        &mut disk_paths,
    );
    disk_paths.sort();

    assert_eq!(disk_paths.len(), 1);
    assert_eq!(disk_paths[0], "src-tauri/tests/fixtures/pdf/minimal.pdf");

    // 2. Manifest referencing AppleDouble sidecar path must be hard-rejected
    let bad_appledouble_path = "src-tauri/tests/fixtures/pdf/._minimal.pdf";
    assert!(
        validate_lexical_path(bad_appledouble_path).is_err(),
        "AppleDouble manifest path must be rejected by validate_lexical_path"
    );

    let manifest = PdfFixtureManifest {
        schema_version: "1.0.0".to_string(),
        corpus_version: "1.0.0".to_string(),
        description: "AppleDouble manifest rejection test".to_string(),
        generators: HashMap::new(),
        fixtures: vec![PdfFixtureRecord {
            id: "bad-appledouble-fixture".to_string(),
            path: bad_appledouble_path.to_string(),
            sha256: "0d509281e91d6d7cbfbcb4d16f5a205578e65765a11c59187b07b585d555b00d".to_string(),
            byte_length: 32,
            pdf_version: "1.4".to_string(),
            license: None,
            license_status: LicenseStatus::RepositoryLicenseReference,
            copyright: None,
            source_type: SourceType::ProjectAuthored,
            source: Some("Authored".to_string()),
            provenance_status: ProvenanceStatus::Complete,
            generator_id: None,
            generated_from: None,
            encrypted: false,
            password_reference: None,
            categories: vec![FixtureCategory::Core],
            expected_capabilities: vec!["has_text".to_string()],
            expected_findings: vec![],
            allowed_tests: vec![TestKind::Integration],
            feature_manifest: None,
            notes: None,
            allow_empty: false,
        }],
    };

    let res = validate_manifest(&manifest, repo_root, false);
    assert!(
        res.is_err(),
        "Manifest referencing AppleDouble path must fail validation"
    );
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("AppleDouble sidecar")));
}

#[test]
fn test_lexical_path_non_canonical_backslash_rejection() {
    let bad_paths = vec![
        r"src-tauri\tests\fixtures\pdf\minimal.pdf",
        r"fixtures\a.pdf",
        r"foo\bar\baz.pdf",
        r"\root\file.pdf",
        r"dir\file.pdf",
    ];

    for path in bad_paths {
        let res = validate_lexical_path(path);
        assert!(
            res.is_err(),
            "Path '{path}' containing backslash must be rejected by validate_lexical_path"
        );
        let err = res.unwrap_err();
        assert!(
            err.contains("Non-canonical path separator") || err.contains("Absolute paths"),
            "Expected backslash or absolute path error for '{path}', got: {err}"
        );
    }
}
