use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// -----------------------------------------------------------------------------
// Data Structures for PDF Corpus Coverage Document
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PdfCorpusCoverageDocument {
    pub schema_version: String,
    pub generated_from: GeneratedFromMetadata,
    pub description: String,
    pub unconsumed_fixture_ids: Vec<String>,
    pub traceability_edges: Vec<TraceabilityEdge>,
    pub features: Vec<FeatureCorpusCoverage>,
    pub tests: Vec<TestRecord>,
    pub gaps: Vec<GapRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedFromMetadata {
    #[serde(alias = "manifestPath", alias = "corpusManifest")]
    pub manifest_path: String,
    pub corpus_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureCorpusCoverage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_observation: Option<FeatureStatusObservation>,
    pub fixture_ids: Vec<String>,
    pub test_ids: Vec<String>,
    pub dimensions: CoverageDimensions,
    pub platform_evidence: PlatformEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatusObservation {
    Stable,
    Experimental,
    Mock,
    Disabled,
    PlatformLimited,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageDimensions {
    pub positive: CoverageDimensionStatus,
    pub negative: CoverageDimensionStatus,
    pub malformed: CoverageDimensionStatus,
    pub encrypted: CoverageDimensionStatus,
    pub unicode: CoverageDimensionStatus,
    pub persistence: CoverageDimensionStatus,
    pub worker: CoverageDimensionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDimensionStatus {
    Covered,
    Partial,
    NotCovered,
    NotApplicable,
    Planned,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformEvidence {
    pub macos: PlatformEvidenceLevel,
    pub linux: PlatformEvidenceLevel,
    pub windows: PlatformEvidenceLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PlatformEvidenceLevel {
    NotRun,
    SourceOnly,
    FastLocal,
    NativeRuntime,
    ExactArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceabilityEdge {
    pub feature_id: String,
    pub fixture_id: String,
    pub test_id: String,
    pub relation: TraceabilityRelation,
    pub dimensions: Vec<CoverageDimensionName>,
    pub evidence: TraceabilityEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TraceabilityRelation {
    Direct,
    HelperIndirect,
    FeatureManifestIndirect,
    GeneratedLookup,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceabilityEvidence {
    pub source_path: String,
    pub verification_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestRecord {
    pub id: String,
    pub kind: TestRecordKind,
    pub state: TestState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TestRecordKind {
    UnitTest,
    IntegrationTest,
    AcceptanceTest,
    E2eTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TestState {
    Current,
    Planned,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GapRecord {
    pub id: String,
    pub feature_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_type: Option<GapType>,
    pub dimension: CoverageDimensionName,
    pub severity: GapSeverity,
    pub required_layer: RequiredLayer,
    pub planned_prompt: String,
    pub release_blocking: bool,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GapType {
    TestGap,
    PlatformGap,
    ConformanceGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDimensionName {
    Positive,
    Negative,
    Malformed,
    Encrypted,
    Unicode,
    Persistence,
    Worker,
    Platform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GapSeverity {
    Informational,
    Low,
    Medium,
    High,
    Critical,
    ReleaseBlocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RequiredLayer {
    Integration,
    NativeRuntime,
    ExactArtifact,
    Ci,
}

// -----------------------------------------------------------------------------
// Minimal Manifest & Platform Support Types for Cross-Verification
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalCorpusManifest {
    pub schema_version: String,
    pub corpus_version: String,
    pub fixtures: Vec<MinimalFixtureRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalFixtureRecord {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalPlatformSupportDocument {
    pub platforms: Vec<MinimalPlatformEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalPlatformEntry {
    pub operating_system: String,
    pub status: String,
    pub current_baseline_evidence: MinimalCurrentBaselineEvidence,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimalCurrentBaselineEvidence {
    pub native_integration_tests: String,
    pub worker_runtime_tests: String,
    pub packaged_artifact: String,
    pub exact_artifact_smoke: String,
}

// -----------------------------------------------------------------------------
// Authoritative Bounded plannedPrompt Allowlist
// -----------------------------------------------------------------------------

pub const ALLOWED_PLANNED_PROMPTS: &[&str] = &[
    "11I-A", "11I-B", "11I-C", "11I-E", "11I-F", "11I-G", "11V-B", "11V-C", "11V-D", "11V-E", "11R",
];

// -----------------------------------------------------------------------------
// Validation Utilities & Invariants
// -----------------------------------------------------------------------------

pub fn get_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Resolve repo root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

pub fn validate_kebab_case_id(id: &str) -> bool {
    let re = Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").unwrap();
    re.is_match(id)
}

pub fn validate_domain_id(id: &str) -> bool {
    let re = Regex::new(r"^[a-z0-9_]+(?:-[a-z0-9_]+)*$").unwrap();
    re.is_match(id)
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

    if raw_path.contains('\\') {
        return Err(format!(
            "Backslash '\\' is forbidden; '/' is canonical path separator: '{raw_path}'"
        ));
    }

    if raw_path.starts_with('/') {
        return Err(format!(
            "Leading slash (absolute path) is forbidden: '{raw_path}'"
        ));
    }

    for segment in raw_path.split('/') {
        if segment.is_empty() {
            return Err(format!(
                "Empty segment or double slash is forbidden: '{raw_path}'"
            ));
        }
        if segment == "." {
            return Err(format!(
                "Current directory '.' segment is forbidden: '{raw_path}'"
            ));
        }
        if segment == ".." {
            return Err(format!(
                "Parent directory '..' traversal segment is forbidden: '{raw_path}'"
            ));
        }
        if segment.starts_with("._") {
            return Err(format!(
                "AppleDouble sidecar '._*' artifact is forbidden in path: '{raw_path}'"
            ));
        }
    }

    Ok(PathBuf::from(raw_path))
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
                "Symlink file is forbidden: '{raw_path}' is a symlink"
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

pub fn validate_coverage_document(
    doc: &PdfCorpusCoverageDocument,
    manifest_fixtures: &HashSet<String>,
    repo_root: &Path,
    check_disk: bool,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // 0. Schema version & metadata invariants
    if doc.schema_version != "1.0.0" {
        errors.push(format!(
            "Unsupported schemaVersion '{}' (only '1.0.0' is supported)",
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
    if doc.features.is_empty() {
        errors.push("features list cannot be empty".to_string());
    }
    if doc.tests.is_empty() {
        errors.push("tests list cannot be empty".to_string());
    }

    let allowed_prompts_set: HashSet<&str> = ALLOWED_PLANNED_PROMPTS.iter().copied().collect();

    // 1. Validate Tests and build test lookup
    let mut seen_test_ids = HashSet::new();
    let mut test_map = HashMap::new();

    for test in &doc.tests {
        if test.id.trim().is_empty() {
            errors.push("Test ID cannot be empty".to_string());
        } else if !seen_test_ids.insert(&test.id) {
            errors.push(format!("Duplicate test ID: '{}'", test.id));
        }
        test_map.insert(&test.id, test);

        match test.state {
            TestState::Current => {
                if test.planned_prompt.is_some() {
                    errors.push(format!(
                        "Current test '{}' must have plannedPrompt = null, found {:?}",
                        test.id, test.planned_prompt
                    ));
                }
                match &test.source_path {
                    Some(spath) => match validate_lexical_path(spath) {
                        Ok(_) => {
                            if check_disk {
                                match validate_safe_repo_relative_path(spath, repo_root) {
                                    Ok(resolved) => {
                                        if !resolved.is_file() {
                                            errors.push(format!(
                                                "Current test '{}' sourcePath not found on disk at '{}'",
                                                test.id, spath
                                            ));
                                        }
                                    }
                                    Err(e) => {
                                        errors.push(format!(
                                            "Current test '{}' sourcePath invalid: {e}",
                                            test.id
                                        ));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            errors.push(format!(
                                "Current test '{}' invalid sourcePath '{spath}': {e}",
                                test.id
                            ));
                        }
                    },
                    None => {
                        errors.push(format!(
                            "Current test '{}' must have a non-null sourcePath",
                            test.id
                        ));
                    }
                }
            }
            TestState::Planned => {
                match &test.planned_prompt {
                    Some(prompt) => {
                        if prompt.contains('/') {
                            errors.push(format!(
                                "Planned test '{}' contains forbidden slash in plannedPrompt '{}'",
                                test.id, prompt
                            ));
                        }
                        if !allowed_prompts_set.contains(prompt.as_str()) {
                            errors.push(format!(
                                "Planned test '{}' specifies invalid plannedPrompt '{}' (not in allowlist)",
                                test.id, prompt
                            ));
                        }
                    }
                    None => {
                        errors.push(format!(
                            "Planned test '{}' must specify plannedPrompt",
                            test.id
                        ));
                    }
                }
                if let Some(spath) = &test.source_path {
                    if let Err(e) = validate_lexical_path(spath) {
                        errors.push(format!(
                            "Planned test '{}' invalid sourcePath '{spath}': {e}",
                            test.id
                        ));
                    }
                }
            }
        }
    }

    // 2. Validate Gaps, build gap lookup, and validate TEST_GAP linkage
    let mut seen_gap_ids = HashSet::new();
    let mut gap_map = HashMap::new();
    let mut test_gap_fixtures = HashSet::new();

    for gap in &doc.gaps {
        if !validate_kebab_case_id(&gap.id) {
            errors.push(format!(
                "Gap ID '{}' is invalid (must match ^[a-z0-9]+(?:-[a-z0-9]+)*$)",
                gap.id
            ));
        } else if !seen_gap_ids.insert(&gap.id) {
            errors.push(format!("Duplicate gap ID: '{}'", gap.id));
        }
        gap_map.insert(&gap.id, gap);

        if gap.description.trim().is_empty() {
            errors.push(format!("Gap '{}' has empty description", gap.id));
        }
        if gap.planned_prompt.contains('/') {
            errors.push(format!(
                "Gap '{}' contains forbidden slash in plannedPrompt '{}'",
                gap.id, gap.planned_prompt
            ));
        }
        if !allowed_prompts_set.contains(gap.planned_prompt.as_str()) {
            errors.push(format!(
                "Gap '{}' specifies invalid plannedPrompt '{}' (not in allowlist)",
                gap.id, gap.planned_prompt
            ));
        }

        if gap.gap_type == Some(GapType::TestGap) || gap.fixture_id.is_some() {
            match &gap.fixture_id {
                Some(fid) => {
                    if !manifest_fixtures.contains(fid) {
                        errors.push(format!(
                            "Gap '{}' references unknown fixtureId '{}'",
                            gap.id, fid
                        ));
                    }
                    if !test_gap_fixtures.insert(fid.clone()) {
                        errors.push(format!(
                            "Multiple TEST_GAP records reference the same fixtureId '{}'",
                            fid
                        ));
                    }
                }
                None => {
                    errors.push(format!(
                        "Gap '{}' specifies gapType=test_gap but missing fixtureId",
                        gap.id
                    ));
                }
            }
        }
    }

    // 3. Validate Features lookup
    let mut seen_feature_ids = HashSet::new();
    let mut feature_map = HashMap::new();
    for feature in &doc.features {
        if !validate_domain_id(&feature.id) {
            errors.push(format!(
                "Feature ID '{}' is invalid domain identifier",
                feature.id
            ));
        } else if !seen_feature_ids.insert(&feature.id) {
            errors.push(format!("Duplicate feature ID: '{}'", feature.id));
        }
        feature_map.insert(&feature.id, feature);

        if feature.name.trim().is_empty() {
            errors.push(format!("Feature '{}' has empty name", feature.id));
        }
    }

    // 4. Invariant: Validate Traceability Edges and Dimension Tracking
    let mut seen_edge_keys = HashSet::new();
    let mut consumed_fixtures = HashSet::new();
    let mut feature_edge_fixtures: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut feature_edge_tests: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut feature_proven_dimensions: HashMap<&str, HashSet<CoverageDimensionName>> =
        HashMap::new();
    let mut feature_current_edge_counts: HashMap<&str, usize> = HashMap::new();

    for edge in &doc.traceability_edges {
        let edge_key = (
            &edge.feature_id,
            &edge.fixture_id,
            &edge.test_id,
            edge.relation,
        );
        if !seen_edge_keys.insert(edge_key) {
            errors.push(format!(
                "Duplicate traceability edge: feature='{}', fixture='{}', test='{}', relation={:?}",
                edge.feature_id, edge.fixture_id, edge.test_id, edge.relation
            ));
        }

        if !feature_map.contains_key(&edge.feature_id) {
            errors.push(format!(
                "Traceability edge references unknown featureId '{}'",
                edge.feature_id
            ));
        }

        if !manifest_fixtures.contains(&edge.fixture_id) {
            errors.push(format!(
                "Traceability edge references unknown fixtureId '{}' not present in corpus manifest",
                edge.fixture_id
            ));
        } else {
            consumed_fixtures.insert(&edge.fixture_id);
        }

        if edge.dimensions.is_empty() {
            errors.push(format!(
                "Traceability edge (feature='{}', fixture='{}', test='{}') has empty dimensions array",
                edge.feature_id, edge.fixture_id, edge.test_id
            ));
        }

        match test_map.get(&edge.test_id) {
            Some(test_rec) => {
                if test_rec.state == TestState::Planned {
                    errors.push(format!(
                        "HARD INVARIANT VIOLATION: Planned test '{}' forbidden in traceabilityEdges (feature='{}', fixture='{}')",
                        edge.test_id, edge.feature_id, edge.fixture_id
                    ));
                } else if test_rec.state == TestState::Current {
                    *feature_current_edge_counts
                        .entry(edge.feature_id.as_str())
                        .or_default() += 1;
                    let proven = feature_proven_dimensions
                        .entry(edge.feature_id.as_str())
                        .or_default();
                    for d in &edge.dimensions {
                        proven.insert(*d);
                    }
                }
            }
            None => {
                errors.push(format!(
                    "Traceability edge references unknown testId '{}' not defined in tests list",
                    edge.test_id
                ));
            }
        }

        if edge.evidence.verification_type.trim().is_empty() {
            errors.push(format!(
                "Traceability edge (feature='{}', fixture='{}', test='{}') has empty verificationType",
                edge.feature_id, edge.fixture_id, edge.test_id
            ));
        }

        match validate_lexical_path(&edge.evidence.source_path) {
            Ok(_) => {
                if check_disk {
                    if let Err(e) =
                        validate_safe_repo_relative_path(&edge.evidence.source_path, repo_root)
                    {
                        errors.push(format!(
                            "Traceability edge evidence sourcePath invalid: {e}"
                        ));
                    }
                }
            }
            Err(e) => {
                errors.push(format!(
                    "Traceability edge invalid evidence sourcePath '{}': {e}",
                    edge.evidence.source_path
                ));
            }
        }

        feature_edge_fixtures
            .entry(edge.feature_id.as_str())
            .or_default()
            .insert(edge.fixture_id.as_str());

        feature_edge_tests
            .entry(edge.feature_id.as_str())
            .or_default()
            .insert(edge.test_id.as_str());
    }

    // 5. Invariant: Validate unconsumedFixtureIds exact set equality with consumed edges and TEST_GAPs
    let expected_unconsumed: HashSet<&String> = manifest_fixtures
        .iter()
        .filter(|fid| !consumed_fixtures.contains(*fid))
        .collect();

    let mut declared_unconsumed = HashSet::new();
    for fid in &doc.unconsumed_fixture_ids {
        if !declared_unconsumed.insert(fid) {
            errors.push(format!(
                "Duplicate fixtureId '{}' in unconsumedFixtureIds",
                fid
            ));
        }
        if !manifest_fixtures.contains(fid) {
            errors.push(format!(
                "unconsumedFixtureIds contains unknown fixtureId '{}' not in corpus manifest",
                fid
            ));
        }
    }

    for fid in &expected_unconsumed {
        if !declared_unconsumed.contains(*fid) {
            errors.push(format!(
                "unconsumedFixtureIds missing truly unconsumed fixture '{}'",
                fid
            ));
        }
        if !test_gap_fixtures.contains(*fid) {
            errors.push(format!(
                "Unconsumed fixture '{}' has no corresponding TEST_GAP record in gaps array",
                fid
            ));
        }
    }
    for fid in &declared_unconsumed {
        if !expected_unconsumed.contains(fid) {
            errors.push(format!(
                "unconsumedFixtureIds declared fixture '{}' which is actually consumed by a traceabilityEdge",
                fid
            ));
        }
    }
    for tg_fid in &test_gap_fixtures {
        if !declared_unconsumed.contains(tg_fid) {
            errors.push(format!(
                "TEST_GAP references fixture '{}' which is not in unconsumedFixtureIds",
                tg_fid
            ));
        }
    }

    // 6. Invariant: Feature-level summary sets exact equality and Dimension <-> Evidence Invariant
    for feature in &doc.features {
        let expected_f_fixtures: HashSet<&str> = feature_edge_fixtures
            .get(feature.id.as_str())
            .cloned()
            .unwrap_or_default();
        let declared_f_fixtures: HashSet<&str> =
            feature.fixture_ids.iter().map(|s| s.as_str()).collect();

        for fix in &expected_f_fixtures {
            if !declared_f_fixtures.contains(fix) {
                errors.push(format!(
                    "Feature '{}' fixtureIds missing edge fixture '{}'",
                    feature.id, fix
                ));
            }
        }
        for fix in &declared_f_fixtures {
            if !expected_f_fixtures.contains(fix) {
                errors.push(format!(
                    "Feature '{}' fixtureIds contains fixture '{}' with no corresponding traceabilityEdge",
                    feature.id, fix
                ));
            }
        }

        let expected_f_tests: HashSet<&str> = feature_edge_tests
            .get(feature.id.as_str())
            .cloned()
            .unwrap_or_default();
        let declared_f_tests: HashSet<&str> = feature.test_ids.iter().map(|s| s.as_str()).collect();

        for t in &expected_f_tests {
            if !declared_f_tests.contains(t) {
                errors.push(format!(
                    "Feature '{}' testIds missing edge test '{}'",
                    feature.id, t
                ));
            }
        }
        for t in &declared_f_tests {
            if !expected_f_tests.contains(t) {
                errors.push(format!(
                    "Feature '{}' testIds contains test '{}' with no corresponding traceabilityEdge",
                    feature.id, t
                ));
            }
        }

        // Invariant: Feature Dimension Consistency with Open Gaps
        let mut open_non_informational_gaps: HashMap<(&str, CoverageDimensionName), Vec<&str>> =
            HashMap::new();
        for gap in &doc.gaps {
            if gap.severity != GapSeverity::Informational {
                open_non_informational_gaps
                    .entry((gap.feature_id.as_str(), gap.dimension))
                    .or_default()
                    .push(gap.id.as_str());
            }
        }

        // Dimension <-> Evidence Invariant
        let proven = feature_proven_dimensions
            .get(feature.id.as_str())
            .cloned()
            .unwrap_or_default();
        let edge_count = feature_current_edge_counts
            .get(feature.id.as_str())
            .copied()
            .unwrap_or(0);

        let mut check_dim = |name: CoverageDimensionName, status: CoverageDimensionStatus| {
            if status == CoverageDimensionStatus::Covered {
                if let Some(conflicting_gaps) =
                    open_non_informational_gaps.get(&(feature.id.as_str(), name))
                {
                    errors.push(format!(
                        "COVERAGE-STATUS CONSISTENCY VIOLATION: Feature '{}' claims dimension '{:?}' as 'covered', but open non-informational gap(s) exist: {:?}",
                        feature.id, name, conflicting_gaps
                    ));
                }
            }

            if status == CoverageDimensionStatus::Covered
                || status == CoverageDimensionStatus::Partial
            {
                if edge_count == 0 {
                    errors.push(format!(
                        "DIMENSION EVIDENCE INVARIANT VIOLATION: Feature '{}' has 0 current edges but claims dimension '{:?}' as '{:?}'",
                        feature.id, name, status
                    ));
                } else if !proven.contains(&name) {
                    errors.push(format!(
                        "DIMENSION EVIDENCE INVARIANT VIOLATION: Feature '{}' claims dimension '{:?}' as '{:?}', but no current traceabilityEdge provides evidence for it",
                        feature.id, name, status
                    ));
                }
            }
        };

        check_dim(CoverageDimensionName::Positive, feature.dimensions.positive);
        check_dim(CoverageDimensionName::Negative, feature.dimensions.negative);
        check_dim(
            CoverageDimensionName::Malformed,
            feature.dimensions.malformed,
        );
        check_dim(
            CoverageDimensionName::Encrypted,
            feature.dimensions.encrypted,
        );
        check_dim(CoverageDimensionName::Unicode, feature.dimensions.unicode);
        check_dim(
            CoverageDimensionName::Persistence,
            feature.dimensions.persistence,
        );
        check_dim(CoverageDimensionName::Worker, feature.dimensions.worker);
    }

    // 7. Dynamic Platform Support Evaluation & Fail-Closed Check
    if check_disk {
        let platform_support_path = repo_root.join("docs/engineering/platform-support.json");
        if !platform_support_path.is_file() {
            errors.push(format!(
                "Required platform support document not found at {platform_support_path:?}"
            ));
        } else {
            match fs::read(&platform_support_path) {
                Ok(raw_ps) => {
                    match serde_json::from_slice::<MinimalPlatformSupportDocument>(&raw_ps) {
                        Ok(ps_doc) => {
                            let mut platform_policies: HashMap<String, (bool, bool)> =
                                HashMap::new();
                            for plat in ps_doc.platforms {
                                let allow_native = plat.status == "verified"
                                    && plat.current_baseline_evidence.native_integration_tests
                                        != "not_run"
                                    && plat.current_baseline_evidence.worker_runtime_tests
                                        != "not_run";
                                let allow_exact = plat.status == "verified"
                                    && plat.current_baseline_evidence.packaged_artifact
                                        != "not_run"
                                    && plat.current_baseline_evidence.exact_artifact_smoke
                                        != "not_run";
                                platform_policies
                                    .insert(plat.operating_system, (allow_native, allow_exact));
                            }

                            for feature in &doc.features {
                                let pe = &feature.platform_evidence;
                                let check_os =
                                    |os: &str,
                                     level: PlatformEvidenceLevel,
                                     errors: &mut Vec<String>| {
                                        let (allow_native, allow_exact) = platform_policies
                                            .get(os)
                                            .copied()
                                            .unwrap_or((false, false));
                                        if level == PlatformEvidenceLevel::NativeRuntime
                                            && !allow_native
                                        {
                                            errors.push(format!(
                                            "Feature '{}' claims {} native_runtime before baseline verification",
                                            feature.id, os
                                        ));
                                        }
                                        if level == PlatformEvidenceLevel::ExactArtifact
                                            && !allow_exact
                                        {
                                            errors.push(format!(
                                            "Feature '{}' claims {} exact_artifact before baseline verification",
                                            feature.id, os
                                        ));
                                        }
                                    };

                                check_os("macos", pe.macos, &mut errors);
                                check_os("linux", pe.linux, &mut errors);
                                check_os("windows", pe.windows, &mut errors);
                            }
                        }
                        Err(e) => {
                            errors.push(format!("Failed to parse platform-support.json: {e}"));
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!("Failed to read platform-support.json: {e}"));
                }
            }
        }
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

#[test]
pub fn test_canonical_pdf_corpus_coverage_parses_and_validates() {
    let repo_root = get_repo_root();
    let coverage_path = repo_root.join("docs/engineering/pdf-corpus-coverage.json");
    let manifest_path = repo_root.join("src-tauri/tests/fixtures/corpus-manifest.json");

    assert!(
        coverage_path.is_file(),
        "Canonical docs/engineering/pdf-corpus-coverage.json must exist"
    );
    assert!(
        manifest_path.is_file(),
        "Canonical corpus-manifest.json must exist"
    );

    let raw_manifest = fs::read(&manifest_path).expect("Read manifest");
    let manifest: MinimalCorpusManifest =
        serde_json::from_slice(&raw_manifest).expect("Parse minimal manifest");
    let manifest_fixture_set: HashSet<String> =
        manifest.fixtures.into_iter().map(|f| f.id).collect();

    let raw_coverage = fs::read(&coverage_path).expect("Read coverage document");
    let coverage_doc: PdfCorpusCoverageDocument = serde_json::from_slice(&raw_coverage)
        .expect("Deserialize canonical coverage document strictly");

    assert_eq!(coverage_doc.schema_version, "1.0.0");
    assert_eq!(
        coverage_doc.generated_from.corpus_version, manifest.corpus_version,
        "generatedFrom.corpusVersion must dynamically match manifest.corpusVersion"
    );
    assert!(!coverage_doc.description.is_empty());
    assert!(!coverage_doc.features.is_empty());

    let validation_result =
        validate_coverage_document(&coverage_doc, &manifest_fixture_set, &repo_root, true);
    assert!(
        validation_result.is_ok(),
        "Canonical coverage document validation failed: {:?}",
        validation_result.err()
    );
}

#[test]
pub fn test_schema_version_validation() {
    let repo_root = get_repo_root();
    let manifest_fixtures = HashSet::new();

    let mut doc = PdfCorpusCoverageDocument {
        schema_version: "1.0.0".to_string(),
        description: "Test".to_string(),
        generated_from: GeneratedFromMetadata {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            corpus_version: "1.0.0".to_string(),
        },
        features: vec![],
        traceability_edges: vec![],
        unconsumed_fixture_ids: vec![],
        tests: vec![],
        gaps: vec![],
    };

    // Unsupported versions must be rejected
    let unsupported_versions = vec!["0.9.0", "1.0.1", "2.0.0", "future", "", "1.0", "v1.0.0"];
    for version in unsupported_versions {
        doc.schema_version = version.to_string();
        let res = validate_coverage_document(&doc, &manifest_fixtures, &repo_root, false);
        assert!(res.is_err(), "schemaVersion '{version}' should be rejected");
        let errs = res.unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("Unsupported schemaVersion")),
            "Expected 'Unsupported schemaVersion' error for version '{version}', got: {errs:?}"
        );
    }
}

#[test]
pub fn test_test_record_state_invariants() {
    let repo_root = get_repo_root();
    let manifest_fixtures = HashSet::new();

    // 1. Current test missing sourcePath -> REJECTED
    let doc_bad_current = PdfCorpusCoverageDocument {
        schema_version: "1.0.0".to_string(),
        description: "Test".to_string(),
        generated_from: GeneratedFromMetadata {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            corpus_version: "1.0.0".to_string(),
        },
        features: vec![],
        traceability_edges: vec![],
        unconsumed_fixture_ids: vec![],
        tests: vec![TestRecord {
            id: "test:bad_current".to_string(),
            kind: TestRecordKind::UnitTest,
            state: TestState::Current,
            source_path: None,
            planned_prompt: None,
        }],
        gaps: vec![],
    };
    let res = validate_coverage_document(&doc_bad_current, &manifest_fixtures, &repo_root, false);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("must have a non-null sourcePath")));

    // 2. Current test with plannedPrompt -> REJECTED
    let doc_bad_current_prompt = PdfCorpusCoverageDocument {
        schema_version: "1.0.0".to_string(),
        description: "Test".to_string(),
        generated_from: GeneratedFromMetadata {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            corpus_version: "1.0.0".to_string(),
        },
        features: vec![],
        traceability_edges: vec![],
        unconsumed_fixture_ids: vec![],
        tests: vec![TestRecord {
            id: "test:bad_current_prompt".to_string(),
            kind: TestRecordKind::UnitTest,
            state: TestState::Current,
            source_path: Some("src-tauri/src/lib.rs".to_string()),
            planned_prompt: Some("11I-B".to_string()),
        }],
        gaps: vec![],
    };
    let res = validate_coverage_document(
        &doc_bad_current_prompt,
        &manifest_fixtures,
        &repo_root,
        false,
    );
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("must have plannedPrompt = null")));

    // 3. Planned test missing plannedPrompt -> REJECTED
    let doc_bad_planned = PdfCorpusCoverageDocument {
        schema_version: "1.0.0".to_string(),
        description: "Test".to_string(),
        generated_from: GeneratedFromMetadata {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            corpus_version: "1.0.0".to_string(),
        },
        features: vec![],
        traceability_edges: vec![],
        unconsumed_fixture_ids: vec![],
        tests: vec![TestRecord {
            id: "test:bad_planned".to_string(),
            kind: TestRecordKind::IntegrationTest,
            state: TestState::Planned,
            source_path: Some("src-tauri/tests/future.rs".to_string()),
            planned_prompt: None,
        }],
        gaps: vec![],
    };
    let res = validate_coverage_document(&doc_bad_planned, &manifest_fixtures, &repo_root, false);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("must specify plannedPrompt")));
}

#[test]
pub fn test_planned_test_in_traceability_edges_is_rejected() {
    let repo_root = get_repo_root();
    let mut manifest_fixtures = HashSet::new();
    manifest_fixtures.insert("core-minimal".to_string());

    let doc = PdfCorpusCoverageDocument {
        schema_version: "1.0.0".to_string(),
        description: "Test".to_string(),
        generated_from: GeneratedFromMetadata {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            corpus_version: "1.0.0".to_string(),
        },
        features: vec![FeatureCorpusCoverage {
            id: "pdf_viewer".to_string(),
            name: "PDF Viewer".to_string(),
            status_observation: None,
            fixture_ids: vec!["core-minimal".to_string()],
            test_ids: vec!["rust:planned_test".to_string()],
            dimensions: CoverageDimensions {
                positive: CoverageDimensionStatus::Covered,
                negative: CoverageDimensionStatus::Covered,
                malformed: CoverageDimensionStatus::NotApplicable,
                encrypted: CoverageDimensionStatus::NotApplicable,
                unicode: CoverageDimensionStatus::NotApplicable,
                persistence: CoverageDimensionStatus::NotApplicable,
                worker: CoverageDimensionStatus::Covered,
            },
            platform_evidence: PlatformEvidence {
                macos: PlatformEvidenceLevel::SourceOnly,
                linux: PlatformEvidenceLevel::SourceOnly,
                windows: PlatformEvidenceLevel::SourceOnly,
            },
        }],
        traceability_edges: vec![TraceabilityEdge {
            feature_id: "pdf_viewer".to_string(),
            fixture_id: "core-minimal".to_string(),
            test_id: "rust:planned_test".to_string(),
            relation: TraceabilityRelation::Direct,
            dimensions: vec![
                CoverageDimensionName::Positive,
                CoverageDimensionName::Worker,
            ],
            evidence: TraceabilityEvidence {
                source_path: "src-tauri/tests/worker_pdfium_integration.rs".to_string(),
                verification_type: "direct_fixture_path_load".to_string(),
                notes: None,
            },
        }],
        unconsumed_fixture_ids: vec![],
        tests: vec![TestRecord {
            id: "rust:planned_test".to_string(),
            kind: TestRecordKind::IntegrationTest,
            state: TestState::Planned,
            source_path: Some("src-tauri/tests/future.rs".to_string()),
            planned_prompt: Some("11I-B".to_string()),
        }],
        gaps: vec![],
    };

    let res = validate_coverage_document(&doc, &manifest_fixtures, &repo_root, false);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("HARD INVARIANT VIOLATION: Planned test")));
}

#[test]
pub fn test_gap_record_invariants_and_planned_prompt_allowlist() {
    let repo_root = get_repo_root();
    let manifest_fixtures = HashSet::new();

    // 1. Invalid plannedPrompt (multi-owner with slash) -> REJECTED
    let doc_slash = PdfCorpusCoverageDocument {
        schema_version: "1.0.0".to_string(),
        description: "Test".to_string(),
        generated_from: GeneratedFromMetadata {
            manifest_path: "src-tauri/tests/fixtures/corpus-manifest.json".to_string(),
            corpus_version: "1.0.0".to_string(),
        },
        features: vec![FeatureCorpusCoverage {
            id: "pdf_viewer".to_string(),
            name: "Test".to_string(),
            status_observation: None,
            fixture_ids: vec![],
            test_ids: vec![],
            dimensions: CoverageDimensions {
                positive: CoverageDimensionStatus::NotCovered,
                negative: CoverageDimensionStatus::NotCovered,
                malformed: CoverageDimensionStatus::NotApplicable,
                encrypted: CoverageDimensionStatus::NotApplicable,
                unicode: CoverageDimensionStatus::NotApplicable,
                persistence: CoverageDimensionStatus::NotApplicable,
                worker: CoverageDimensionStatus::NotApplicable,
            },
            platform_evidence: PlatformEvidence {
                macos: PlatformEvidenceLevel::SourceOnly,
                linux: PlatformEvidenceLevel::SourceOnly,
                windows: PlatformEvidenceLevel::SourceOnly,
            },
        }],
        traceability_edges: vec![],
        unconsumed_fixture_ids: vec![],
        tests: vec![],
        gaps: vec![GapRecord {
            id: "gap-bad-slash".to_string(),
            feature_id: "pdf_viewer".to_string(),
            fixture_id: None,
            gap_type: None,
            dimension: CoverageDimensionName::Positive,
            description: "Test gap".to_string(),
            severity: GapSeverity::Medium,
            required_layer: RequiredLayer::Integration,
            planned_prompt: "11I-C / 11V-C".to_string(),
            release_blocking: false,
        }],
    };
    let res = validate_coverage_document(&doc_slash, &manifest_fixtures, &repo_root, false);
    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .iter()
        .any(|e| e.contains("forbidden slash in plannedPrompt")));
}

#[test]
pub fn test_safe_path_validator_rejection_rules() {
    let repo_root = get_repo_root();

    let invalid_paths = vec![
        "",
        "   ",
        "../outside.rs",
        "foo/../../bar.rs",
        r"..\windows_traversal.rs",
        r"foo\..\..\bar.rs",
        "/absolute/unix/path.rs",
        r"\absolute\windows\path.rs",
        "C:\\windows\\system32.rs",
        "c:/windows/system32.rs",
        "D:\\data.rs",
        r"\\server\share\file.rs",
        "//server/share/file.rs",
        "file:///path/to/file.rs",
        "http://example.com/file.rs",
        "https://example.com/file.rs",
        "foo//bar.rs",
        "foo/./bar.rs",
        r"foo\\bar.rs",
        r"foo\bar.rs",
        "src-tauri/src/._lib.rs",
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
