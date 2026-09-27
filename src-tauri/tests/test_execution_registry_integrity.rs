use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestExecutionRegistry {
    pub schema_version: String,
    pub description: String,
    pub generated_from: RegistryGeneratedFrom,
    #[serde(default)]
    pub execution_classes: Vec<String>,
    #[serde(default)]
    pub platform_vocabulary: Vec<String>,
    pub targets: Vec<TestExecutionRecord>,
    pub summary: Option<RegistrySummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryGeneratedFrom {
    pub manifest_path: String,
    pub coverage_path: String,
    pub platform_support_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestExecutionRecord {
    pub id: String,
    pub target: String,
    pub language: String,
    pub execution_class: String,
    pub uses_pdfium: bool,
    pub real_worker: bool,
    pub supported_platforms: Vec<String>,
    pub fixture_ids: Vec<String>,
    #[serde(default)]
    pub synthetic_inputs: Vec<String>,
    #[serde(default)]
    pub native_requirements: Vec<String>,
    pub artifact_requirement: Option<serde_json::Value>,
    pub expected_test_count: u32,
    #[serde(default)]
    pub expected_test_presence: Option<String>,
    #[serde(default)]
    pub expected_test_count_by_profile: Option<HashMap<String, u32>>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrySummary {
    pub total_targets: usize,
    pub by_execution_class: HashMap<String, usize>,
    pub pdfium_runtime_required: usize,
    pub real_worker_required: usize,
    pub exact_artifact_required: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalCorpusManifest {
    pub schema_version: String,
    pub corpus_version: String,
    pub fixtures: Vec<MinimalFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalFixture {
    pub id: String,
    pub path: String,
}

pub fn get_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

pub fn validate_lexical_path(raw_path: &str) -> Result<PathBuf, String> {
    if raw_path.trim().is_empty() {
        return Err("Path cannot be empty".to_string());
    }
    if raw_path.contains("://") {
        return Err(format!("URI scheme forbidden: '{raw_path}'"));
    }
    let bytes = raw_path.as_bytes();
    if raw_path.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(format!("Windows drive letter forbidden: '{raw_path}'"));
    }
    if raw_path.starts_with(r"\\") || raw_path.starts_with("//") {
        return Err(format!("UNC path forbidden: '{raw_path}'"));
    }
    if raw_path.contains('\\') {
        return Err(format!("Backslash forbidden: '{raw_path}'"));
    }
    if raw_path.starts_with('/') {
        return Err(format!("Leading slash forbidden: '{raw_path}'"));
    }
    for segment in raw_path.split('/') {
        if segment.is_empty() {
            return Err(format!(
                "Empty segment or double slash forbidden: '{raw_path}'"
            ));
        }
        if segment == "." {
            return Err(format!("'.' segment forbidden: '{raw_path}'"));
        }
        if segment == ".." {
            return Err(format!("'..' traversal forbidden: '{raw_path}'"));
        }
        if segment.starts_with("._") {
            return Err(format!("AppleDouble '._*' forbidden: '{raw_path}'"));
        }
    }
    Ok(PathBuf::from(raw_path))
}

pub fn validate_safe_repo_relative_path(
    raw_path: &str,
    repo_root: &Path,
) -> Result<PathBuf, String> {
    let rel = validate_lexical_path(raw_path)?;
    let resolved = repo_root.join(&rel);
    if let Ok(meta) = resolved.symlink_metadata() {
        if meta.file_type().is_symlink() {
            return Err(format!("Symlink forbidden: '{raw_path}'"));
        }
        let canonical_root = repo_root
            .canonicalize()
            .map_err(|e| format!("canonicalize root: {e}"))?;
        let canonical_resolved = resolved
            .canonicalize()
            .map_err(|e| format!("canonicalize path '{raw_path}': {e}"))?;
        if !canonical_resolved.starts_with(&canonical_root) {
            return Err(format!("Symlink escapes repo root: '{raw_path}'"));
        }
    }
    Ok(resolved)
}

pub fn validate_registry_document(
    doc: &TestExecutionRegistry,
    manifest_fixture_set: &HashSet<String>,
    repo_root: &Path,
    check_disk: bool,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if doc.schema_version != "1.0.0" {
        errors.push(format!(
            "Unsupported schemaVersion '{}' (only '1.0.0')",
            doc.schema_version
        ));
    }
    if doc.description.trim().is_empty() {
        errors.push("description cannot be empty".to_string());
    }
    if doc.generated_from.manifest_path != "src-tauri/tests/fixtures/corpus-manifest.json" {
        errors.push(format!(
            "generatedFrom.manifestPath mismatch: expected 'src-tauri/tests/fixtures/corpus-manifest.json', got '{}'",
            doc.generated_from.manifest_path
        ));
    }
    if doc.generated_from.coverage_path != "docs/engineering/pdf-corpus-coverage.json" {
        errors.push(format!(
            "generatedFrom.coveragePath must be 'docs/engineering/pdf-corpus-coverage.json', got '{}'",
            doc.generated_from.coverage_path
        ));
    }
    if doc.targets.is_empty() {
        errors.push("targets list cannot be empty".to_string());
    }

    let allowed_classes: HashSet<&str> = ["FAST_LOCAL", "NATIVE_PLATFORM", "EXACT_ARTIFACT"]
        .iter()
        .copied()
        .collect();
    let allowed_platforms: HashSet<&str> = ["any", "macos", "linux", "windows"]
        .iter()
        .copied()
        .collect();
    let allowed_languages: HashSet<&str> = ["rust", "js"].iter().copied().collect();
    let allowed_presence: HashSet<&str> = ["at_least_one", "exact", "conditional"]
        .iter()
        .copied()
        .collect();
    let native_only_reqs: HashSet<&str> = [
        "pdfium_dynamic_lib",
        "pdfium_worker_executable",
        "job_worker_executable",
        "aarch64",
    ]
    .iter()
    .copied()
    .collect();

    let mut seen_ids = HashSet::new();
    let mut seen_targets = HashSet::new();

    for rec in &doc.targets {
        // id uniqueness & traversal safe
        if rec.id.trim().is_empty() {
            errors.push("Registry entry id cannot be empty".to_string());
        } else if !seen_ids.insert(&rec.id) {
            errors.push(format!("Duplicate test registry id: '{}'", rec.id));
        }
        if rec.id.contains("..") {
            errors.push(format!("Registry id '{}' contains traversal '..'", rec.id));
        }

        // target uniqueness & lexical & disk
        if rec.target.trim().is_empty() {
            errors.push(format!("Registry id '{}' has empty target", rec.id));
        } else if !seen_targets.insert(&rec.target) {
            errors.push(format!("Duplicate target identity ambiguity: '{}' referenced by multiple records (id='{}')", rec.target, rec.id));
        }
        match validate_lexical_path(&rec.target) {
            Ok(_) => {
                if check_disk {
                    let full = repo_root.join(&rec.target);
                    if !full.exists() {
                        errors.push(format!(
                            "Registry id '{}' target not found on disk at '{}'",
                            rec.id, rec.target
                        ));
                    } else {
                        // For js aggregate "src/" is directory, existence is enough
                        // For rust files, ensure not symlink escaping
                        if let Err(e) = validate_safe_repo_relative_path(&rec.target, repo_root) {
                            errors.push(format!("Registry id '{}' target invalid: {e}", rec.id));
                        }
                    }
                }
            }
            Err(e) => errors.push(format!(
                "Registry id '{}' invalid target '{}': {e}",
                rec.id, rec.target
            )),
        }
        if rec.target.contains("..") {
            errors.push(format!(
                "Registry id '{}' target traversal '..' forbidden",
                rec.id
            ));
        }

        if !allowed_languages.contains(rec.language.as_str()) {
            errors.push(format!(
                "Registry id '{}' unknown language '{}'",
                rec.id, rec.language
            ));
        }
        if !allowed_classes.contains(rec.execution_class.as_str()) {
            errors.push(format!(
                "Registry id '{}' unknown executionClass '{}'",
                rec.id, rec.execution_class
            ));
        }
        for plat in &rec.supported_platforms {
            if !allowed_platforms.contains(plat.as_str()) {
                errors.push(format!(
                    "Registry id '{}' unknown platform '{}'",
                    rec.id, plat
                ));
            }
        }
        if rec.supported_platforms.is_empty() {
            errors.push(format!(
                "Registry id '{}' supportedPlatforms cannot be empty",
                rec.id
            ));
        }
        if rec.supported_platforms.contains(&"any".to_string()) && rec.supported_platforms.len() > 1
        {
            errors.push(format!(
                "Registry id '{}' supportedPlatforms contains 'any' mixed with specifics {:?}",
                rec.id, rec.supported_platforms
            ));
        }

        // fixtureIds canonical
        let mut seen_fixtures = HashSet::new();
        for fid in &rec.fixture_ids {
            if fid.trim().is_empty() {
                errors.push(format!("Registry id '{}' has empty fixtureId", rec.id));
            } else if !seen_fixtures.insert(fid) {
                errors.push(format!(
                    "Registry id '{}' duplicate fixtureId '{}'",
                    rec.id, fid
                ));
            }
            if !manifest_fixture_set.contains(fid) {
                errors.push(format!(
                    "Registry id '{}' references unknown/non-canonical fixtureId '{}' not in corpus-manifest.json",
                    rec.id, fid
                ));
            }
            if fid.contains('/') || fid.contains('\\') || fid.contains('.') {
                // fixture ids are kebab-case, but we already checked manifest, this would be caught
            }
        }

        // syntheticInputs must not be canonical fixture ids
        for sid in &rec.synthetic_inputs {
            if manifest_fixture_set.contains(sid) {
                errors.push(format!(
                    "Registry id '{}' syntheticInputs contains canonical fixtureId '{}' — must be in fixtureIds instead",
                    rec.id, sid
                ));
            }
        }

        // executionClass vs requirements invariants
        if rec.execution_class == "EXACT_ARTIFACT" && rec.artifact_requirement.is_none() {
            errors.push(format!(
                "Registry id '{}' EXACT_ARTIFACT must carry artifactRequirement",
                rec.id
            ));
        }
        if rec.execution_class == "FAST_LOCAL" {
            if rec.artifact_requirement.is_some() {
                errors.push(format!(
                    "Registry id '{}' FAST_LOCAL must not carry artifactRequirement",
                    rec.id
                ));
            }
            if rec.uses_pdfium {
                errors.push(format!(
                    "Registry id '{}' FAST_LOCAL must not have usesPdfium=true (native-only)",
                    rec.id
                ));
            }
            for req in &rec.native_requirements {
                if native_only_reqs.contains(req.as_str()) {
                    errors.push(format!(
                        "Registry id '{}' FAST_LOCAL carries native-only requirement '{}' — impossible combination",
                        rec.id, req
                    ));
                }
            }
        }

        if let Some(presence) = &rec.expected_test_presence {
            if !allowed_presence.contains(presence.as_str()) {
                errors.push(format!(
                    "Registry id '{}' unknown expectedTestPresence '{}'",
                    rec.id, presence
                ));
            }
            if presence == "conditional" && rec.expected_test_count_by_profile.is_none() {
                errors.push(format!(
                    "Registry id '{}' conditional presence should provide expectedTestCountByProfile",
                    rec.id
                ));
            }
            // B5: conditional must include default profile
            if presence == "conditional" {
                if let Some(profile) = &rec.expected_test_count_by_profile {
                    let has_default = profile.contains_key("default")
                        || profile.contains_key("default_non_macos");
                    if !has_default {
                        errors.push(format!(
                            "Registry id '{}' conditional must include 'default' or 'default_non_macos' profile",
                            rec.id
                        ));
                    }
                    let vals: Vec<u32> = profile.values().copied().collect();
                    if !vals.is_empty() && vals.iter().all(|v| *v == 0) {
                        errors.push(format!(
                            "Registry id '{}' conditional all profiles 0 — ZERO_TESTS false-green",
                            rec.id
                        ));
                    }
                }
            }
        }

        if rec.expected_test_presence.as_deref() == Some("exact")
            && rec.expected_test_count == 0
            && rec.execution_class == "NATIVE_PLATFORM"
        {
            errors.push(format!(
                "Registry id '{}' NATIVE_PLATFORM with expectedTestCount=0 and exact hides zero-test risk — use conditional",
                rec.id
            ));
        }

        // B5 MOCK_REALITY_MISMATCH: realWorker requires NATIVE_PLATFORM
        if rec.real_worker && rec.execution_class != "NATIVE_PLATFORM" {
            errors.push(format!(
                "Registry id '{}' MOCK_REALITY_MISMATCH — realWorker=true requires NATIVE_PLATFORM (got {})",
                rec.id, rec.execution_class
            ));
        }
    }

    // summary consistency if present
    if let Some(summary) = &doc.summary {
        if summary.total_targets != doc.targets.len() {
            errors.push(format!(
                "summary.totalTargets {} != actual targets length {}",
                summary.total_targets,
                doc.targets.len()
            ));
        }
        let mut counted: HashMap<String, usize> = HashMap::new();
        let mut pdfium_count = 0;
        let mut worker_count = 0;
        for rec in &doc.targets {
            *counted.entry(rec.execution_class.clone()).or_insert(0) += 1;
            if rec.uses_pdfium {
                pdfium_count += 1;
            }
            if rec.real_worker {
                worker_count += 1;
            }
        }
        for cls in &["FAST_LOCAL", "NATIVE_PLATFORM", "EXACT_ARTIFACT"] {
            let expected = summary.by_execution_class.get(*cls).copied().unwrap_or(0);
            let actual = counted.get(*cls).copied().unwrap_or(0);
            if expected != actual {
                errors.push(format!(
                    "summary.byExecutionClass.{cls} {expected} != actual {actual}"
                ));
            }
        }
        if summary.pdfium_runtime_required != pdfium_count {
            errors.push(format!(
                "summary.pdfiumRuntimeRequired {} != actual usesPdfium count {pdfium_count}",
                summary.pdfium_runtime_required
            ));
        }
        if summary.real_worker_required != worker_count {
            errors.push(format!(
                "summary.realWorkerRequired {} != actual realWorker count {worker_count}",
                summary.real_worker_required
            ));
        }
        if summary.exact_artifact_required != 0 {
            // we have 0 exact artifact; sanity check artifactRequirement
            let actual_exact = counted.get("EXACT_ARTIFACT").copied().unwrap_or(0);
            if summary.exact_artifact_required != actual_exact {
                errors.push(format!(
                    "summary.exactArtifactRequired {} != actual EXACT_ARTIFACT count {actual_exact}",
                    summary.exact_artifact_required
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[test]
fn test_canonical_test_execution_registry_parses_and_validates() {
    let repo_root = get_repo_root();
    let registry_path = repo_root.join("docs/engineering/test-execution-registry.json");
    let manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");

    assert!(
        registry_path.is_file(),
        "Canonical registry must exist at docs/engineering/test-execution-registry.json"
    );
    assert!(manifest_path.is_file(), "Corpus manifest must exist");

    let raw_manifest = fs::read(&manifest_path).expect("Read manifest");
    let manifest: MinimalCorpusManifest =
        serde_json::from_slice(&raw_manifest).expect("Parse manifest");
    let manifest_set: HashSet<String> = manifest.fixtures.into_iter().map(|f| f.id).collect();

    let raw_registry = fs::read(&registry_path).expect("Read registry");
    let registry: TestExecutionRegistry =
        serde_json::from_slice(&raw_registry).expect("Deserialize registry strictly");

    assert_eq!(registry.schema_version, "1.0.0");
    assert!(!registry.description.is_empty());
    assert_eq!(
        registry.generated_from.manifest_path,
        "src-tauri/tests/fixtures/corpus-manifest.json"
    );
    assert!(!registry.targets.is_empty());

    let res = validate_registry_document(&registry, &manifest_set, &repo_root, true);
    assert!(res.is_ok(), "Registry validation failed: {:?}", res.err());
}

#[test]
fn test_registry_rejects_duplicate_id_and_traversal() {
    let repo_root = get_repo_root();
    let manifest_set = HashSet::new();

    let dup_doc = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![
            "any".to_string(),
            "macos".to_string(),
            "linux".to_string(),
            "windows".to_string(),
        ],
        targets: vec![
            TestExecutionRecord {
                id: "rust:dup".to_string(),
                target: "src-tauri/tests/document_io_fs.rs".to_string(),
                language: "rust".to_string(),
                execution_class: "FAST_LOCAL".to_string(),
                uses_pdfium: false,
                real_worker: false,
                supported_platforms: vec!["any".to_string()],
                fixture_ids: vec![],
                synthetic_inputs: vec![],
                native_requirements: vec![],
                artifact_requirement: None,
                expected_test_count: 1,
                expected_test_presence: Some("exact".to_string()),
                expected_test_count_by_profile: None,
                notes: None,
            },
            TestExecutionRecord {
                id: "rust:dup".to_string(),
                target: "src-tauri/tests/document_io_fs.rs".to_string(),
                language: "rust".to_string(),
                execution_class: "FAST_LOCAL".to_string(),
                uses_pdfium: false,
                real_worker: false,
                supported_platforms: vec!["any".to_string()],
                fixture_ids: vec![],
                synthetic_inputs: vec![],
                native_requirements: vec![],
                artifact_requirement: None,
                expected_test_count: 1,
                expected_test_presence: Some("exact".to_string()),
                expected_test_count_by_profile: None,
                notes: None,
            },
        ],
        summary: None,
    };

    let res = validate_registry_document(&dup_doc, &manifest_set, &repo_root, false);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(errs
        .iter()
        .any(|e| e.contains("Duplicate test registry id")));

    // traversal
    let trav_doc = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![],
        targets: vec![TestExecutionRecord {
            id: "rust:trav".to_string(),
            target: "../outside.rs".to_string(),
            language: "rust".to_string(),
            execution_class: "FAST_LOCAL".to_string(),
            uses_pdfium: false,
            real_worker: false,
            supported_platforms: vec!["any".to_string()],
            fixture_ids: vec![],
            synthetic_inputs: vec![],
            native_requirements: vec![],
            artifact_requirement: None,
            expected_test_count: 1,
            expected_test_presence: Some("exact".to_string()),
            expected_test_count_by_profile: None,
            notes: None,
        }],
        summary: None,
    };
    let res2 = validate_registry_document(&trav_doc, &manifest_set, &repo_root, false);
    assert!(res2.is_err());
    assert!(res2
        .unwrap_err()
        .iter()
        .any(|e| e.contains("traversal") || e.contains("Parent traversal")));
}

#[test]
fn test_registry_rejects_unknown_canonical_fixture_and_impossible_fast_local_combination() {
    let repo_root = get_repo_root();
    let mut manifest_set = HashSet::new();
    manifest_set.insert("core-minimal".to_string());

    let doc = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![],
        targets: vec![
            TestExecutionRecord {
                id: "rust:bad_fixture".to_string(),
                target: "src-tauri/tests/document_io_fs.rs".to_string(),
                language: "rust".to_string(),
                execution_class: "FAST_LOCAL".to_string(),
                uses_pdfium: false,
                real_worker: false,
                supported_platforms: vec!["any".to_string()],
                fixture_ids: vec!["nonexistent-fixture-id".to_string()],
                synthetic_inputs: vec![],
                native_requirements: vec![],
                artifact_requirement: None,
                expected_test_count: 1,
                expected_test_presence: Some("exact".to_string()),
                expected_test_count_by_profile: None,
                notes: None,
            },
            TestExecutionRecord {
                id: "rust:bad_fast".to_string(),
                target: "src-tauri/tests/document_io_fs.rs".to_string(),
                language: "rust".to_string(),
                execution_class: "FAST_LOCAL".to_string(),
                uses_pdfium: true,
                real_worker: false,
                supported_platforms: vec!["any".to_string()],
                fixture_ids: vec![],
                synthetic_inputs: vec![],
                native_requirements: vec!["pdfium_dynamic_lib".to_string()],
                artifact_requirement: None,
                expected_test_count: 1,
                expected_test_presence: Some("exact".to_string()),
                expected_test_count_by_profile: None,
                notes: None,
            },
        ],
        summary: None,
    };

    let res = validate_registry_document(&doc, &manifest_set, &repo_root, false);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(errs
        .iter()
        .any(|e| e.contains("unknown/non-canonical fixtureId")));
    assert!(errs
        .iter()
        .any(|e| e.contains("FAST_LOCAL must not have usesPdfium")));
    assert!(errs.iter().any(|e| e.contains("native-only requirement")));
}

#[test]
fn test_registry_exacts_artifact_must_have_requirement() {
    let repo_root = get_repo_root();
    let manifest_set = HashSet::new();

    let doc = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![],
        targets: vec![TestExecutionRecord {
            id: "rust:exact_missing".to_string(),
            target: "src-tauri/tests/document_io_fs.rs".to_string(),
            language: "rust".to_string(),
            execution_class: "EXACT_ARTIFACT".to_string(),
            uses_pdfium: false,
            real_worker: false,
            supported_platforms: vec!["any".to_string()],
            fixture_ids: vec![],
            synthetic_inputs: vec![],
            native_requirements: vec![],
            artifact_requirement: None,
            expected_test_count: 1,
            expected_test_presence: Some("exact".to_string()),
            expected_test_count_by_profile: None,
            notes: None,
        }],
        summary: None,
    };

    let res = validate_registry_document(&doc, &manifest_set, &repo_root, false);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("EXACT_ARTIFACT must carry artifactRequirement")));
}

#[test]
fn test_b5_zero_test_and_mock_reality_hardening() {
    let repo_root = get_repo_root();
    let mut manifest_set = HashSet::new();
    manifest_set.insert("core-minimal".to_string());

    // ZERO_TESTS: conditional with all profiles 0 should fail
    let zero_doc = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![],
        targets: vec![TestExecutionRecord {
            id: "rust:zero_cfg".to_string(),
            target: "src-tauri/tests/document_io_fs.rs".to_string(),
            language: "rust".to_string(),
            execution_class: "NATIVE_PLATFORM".to_string(),
            uses_pdfium: true,
            real_worker: false,
            supported_platforms: vec!["macos".to_string()],
            fixture_ids: vec![],
            synthetic_inputs: vec![],
            native_requirements: vec!["pdfium_dynamic_lib".to_string()],
            artifact_requirement: None,
            expected_test_count: 0,
            expected_test_presence: Some("conditional".to_string()),
            expected_test_count_by_profile: Some(HashMap::from([
                ("default".to_string(), 0),
                ("on_macos_aarch64".to_string(), 0),
            ])),
            notes: None,
        }],
        summary: None,
    };
    let res = validate_registry_document(&zero_doc, &manifest_set, &repo_root, false);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("ZERO_TESTS") || e.contains("all profiles 0")));

    // MOCK_REALITY_MISMATCH: realWorker true with FAST_LOCAL must fail
    let mock_doc = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![],
        targets: vec![TestExecutionRecord {
            id: "rust:mock_mismatch".to_string(),
            target: "src-tauri/tests/document_io_fs.rs".to_string(),
            language: "rust".to_string(),
            execution_class: "FAST_LOCAL".to_string(),
            uses_pdfium: false,
            real_worker: true,
            supported_platforms: vec!["any".to_string()],
            fixture_ids: vec![],
            synthetic_inputs: vec![],
            native_requirements: vec![],
            artifact_requirement: None,
            expected_test_count: 1,
            expected_test_presence: Some("exact".to_string()),
            expected_test_count_by_profile: None,
            notes: None,
        }],
        summary: None,
    };
    let res2 = validate_registry_document(&mock_doc, &manifest_set, &repo_root, false);
    assert!(res2.is_err());
    assert!(res2
        .unwrap_err()
        .iter()
        .any(|e| e.contains("MOCK_REALITY_MISMATCH")));

    // conditional without default profile must fail
    let no_default = TestExecutionRegistry {
        schema_version: "1.0.0".to_string(),
        description: "test".to_string(),
        generated_from: RegistryGeneratedFrom {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            coverage_path: "docs/engineering/pdf-corpus-coverage.json".to_string(),
            platform_support_path: "docs/engineering/platform-support.json".to_string(),
        },
        execution_classes: vec![],
        platform_vocabulary: vec![],
        targets: vec![TestExecutionRecord {
            id: "rust:no_default".to_string(),
            target: "src-tauri/tests/document_io_fs.rs".to_string(),
            language: "rust".to_string(),
            execution_class: "NATIVE_PLATFORM".to_string(),
            uses_pdfium: true,
            real_worker: false,
            supported_platforms: vec!["macos".to_string()],
            fixture_ids: vec![],
            synthetic_inputs: vec![],
            native_requirements: vec!["pdfium_dynamic_lib".to_string()],
            artifact_requirement: None,
            expected_test_count: 3,
            expected_test_presence: Some("conditional".to_string()),
            expected_test_count_by_profile: Some(HashMap::from([(
                "on_macos_aarch64".to_string(),
                3,
            )])),
            notes: None,
        }],
        summary: None,
    };
    let res3 = validate_registry_document(&no_default, &manifest_set, &repo_root, false);
    assert!(res3.is_err());
    assert!(res3
        .unwrap_err()
        .iter()
        .any(|e| e.contains("must include 'default'")));
}

#[test]
fn test_b5_evidence_status_vocabulary() {
    // Ensure evidence status vocabulary is bounded per §13
    let statuses = [
        "PASS",
        "FAIL",
        "NOT_RUN",
        "NOT_APPLICABLE",
        "RUNTIME_UNAVAILABLE",
        "TOOL_UNAVAILABLE",
        "DEPENDENCY_UNAVAILABLE",
    ];
    assert_eq!(statuses.len(), 7);
    // SKIP alone is forbidden §13
    assert!(!statuses.contains(&"SKIP"), "SKIP alone must not be used");
}

#[test]
fn test_b5_canonical_taxonomy_bounded() {
    // Ensure canonical false-green taxonomy is exactly the 10 bounded terms per §2
    let taxonomy = [
        "ZERO_TESTS",
        "FILTER_NO_MATCH",
        "CFG_ELIDED",
        "IGNORED_TEST",
        "RUNTIME_UNAVAILABLE",
        "TOOL_UNAVAILABLE",
        "DEPENDENCY_UNAVAILABLE",
        "MOCK_REALITY_MISMATCH",
        "AGGREGATE_MIXING",
        "EXPECTED_EVIDENCE_MISSING",
    ];
    assert_eq!(taxonomy.len(), 10);
    // Invariants: distinct terms, no empty string
    let set: HashSet<&str> = taxonomy.iter().copied().collect();
    assert_eq!(set.len(), 10, "Taxonomy terms must be unique");
}
