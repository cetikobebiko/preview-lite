//! VeraPDF-specific external validation harness.
//!
//! Provides bounded invocation, structured output parsing, and tool identity tracking
//! for VeraPDF. Does not download or install VeraPDF; requires local availability.

use crate::external_validation::types::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Validates input PDF path for existence, regular file, canonicalization, and injection safety.
pub fn validate_input_pdf_path(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Err(ExternalValidationError::PathSafetyViolation(format!(
            "Input PDF does not exist: {}",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(ExternalValidationError::PathSafetyViolation(format!(
            "Input PDF is not a regular file: {}",
            path.display()
        )));
    }
    let canonical = path.canonicalize().map_err(|e| {
        ExternalValidationError::PathSafetyViolation(format!(
            "Failed to canonicalize path {}: {}",
            path.display(),
            e
        ))
    })?;

    let path_str = canonical.to_string_lossy();
    if path_str.contains('\0') || path_str.contains('\n') || path_str.contains('\r') {
        return Err(ExternalValidationError::CommandInjectionBlocked);
    }

    Ok(canonical)
}

/// VeraPDF profile identifiers (mapped from design documents).
pub mod profiles {
    pub const PDF_A_1B: &str = "PDF/A-1b";
    pub const PDF_A_2B: &str = "PDF/A-2b";
    pub const PDF_A_2U: &str = "PDF/A-2u";
    pub const PDF_A_3B: &str = "PDF/A-3b";
    pub const PDF_A_3U: &str = "PDF/A-3u";
    pub const PDF_UA_1: &str = "PDF/UA-1";
    pub const PDF_UA_2: &str = "PDF/UA-2";
    pub const PDF_X_1A_2001: &str = "PDF/X-1a:2001";
    pub const PDF_X_4: &str = "PDF/X-4";
    pub const PDF_X_4P: &str = "PDF/X-4p";
    pub const PDF_X_5N: &str = "PDF/X-5n";
    pub const PDF_X_5G: &str = "PDF/X-5g";
    pub const PDF_X_6: &str = "PDF/X-6";
    pub const PDF_X_6P: &str = "PDF/X-6p";
    pub const PDF_X_6N: &str = "PDF/X-6n";
}

/// Minimum supported VeraPDF version (from pades-architecture-design.md).
pub const MIN_VERAPDF_VERSION: &str = "1.30.2";

/// VeraPDF CLI invocation builder and parser.
pub struct VerapdfHarness {
    config: VerapdfConfig,
}

/// Configuration for VeraPDF invocation.
#[derive(Debug, Clone)]
pub struct VerapdfConfig {
    /// Explicit path to verapdf executable (or "verapdf" for PATH lookup).
    pub executable: String,
    /// Additional JVM arguments (if using JAR).
    pub jvm_args: Vec<String>,
    /// Whether to use XML output format (machine-readable).
    pub use_xml_output: bool,
    /// Timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum output capture size in bytes.
    pub max_output_bytes: usize,
}

impl Default for VerapdfConfig {
    fn default() -> Self {
        Self {
            executable: "verapdf".to_string(),
            jvm_args: vec!["-Xmx2g".to_string()],
            use_xml_output: true,
            timeout_secs: 300,
            max_output_bytes: 10_485_760, // 10 MB
        }
    }
}

/// Parsed VeraPDF tool identity.
#[derive(Debug, Clone)]
pub struct VerapdfIdentity {
    pub version: String,
    pub profile_version: Option<String>,
    pub invocation_class: String,
    pub resolution_method: ToolResolutionMethod,
}

/// VeraPDF XML report structure (subset for parsing).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerapdfXmlReport {
    #[serde(rename = "@version")]
    version: Option<String>,
    #[serde(rename = "profile")]
    profile: Option<VerapdfProfile>,
    #[serde(rename = "validationResult")]
    validation_result: Option<VerapdfValidationResult>,
    #[serde(rename = "rules")]
    rules: Option<VerapdfRules>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct VerapdfProfile {
    #[serde(rename = "@name")]
    name: Option<String>,
    #[serde(rename = "@version")]
    version: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct VerapdfValidationResult {
    #[serde(rename = "@status")]
    status: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct VerapdfRules {
    #[serde(rename = "rule")]
    rule: Vec<VerapdfRule>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct VerapdfRule {
    #[serde(rename = "@id")]
    id: Option<String>,
    #[serde(rename = "@specification")]
    specification: Option<String>,
    #[serde(rename = "@status")]
    status: Option<String>,
    #[serde(rename = "@message")]
    message: Option<String>,
}

impl VerapdfHarness {
    /// Create a new VeraPDF harness with default configuration.
    pub fn new() -> Self {
        Self {
            config: VerapdfConfig::default(),
        }
    }

    /// Create a new VeraPDF harness with custom configuration.
    pub fn with_config(config: VerapdfConfig) -> Self {
        Self { config }
    }

    /// Detect VeraPDF tool identity (version, profile support).
    pub fn detect_identity(&self) -> Result<VerapdfIdentity> {
        let output = self.run_command(&["--version"])?;
        let version = Self::parse_version(&output)?;

        let profile_version = if self.config.use_xml_output {
            Self::detect_profile_version(&self.config.executable, &self.config.jvm_args)?
        } else {
            None
        };

        Ok(VerapdfIdentity {
            version,
            profile_version,
            invocation_class: self.invocation_class(),
            resolution_method: ToolResolutionMethod::PathLookup,
        })
    }

    /// Validate a PDF file against a profile.
    pub fn validate(&self, pdf_path: &Path, profile: &str) -> Result<ValidationOutcome> {
        let validator_id = ValidatorId(format!("verapdf-{}", profile.replace(['/', ':'], "-")));
        let profile_id = ProfileId(profile.to_string());
        let start = Instant::now();

        // 1. Enforce strict input path safety
        let canonical_pdf_path = match validate_input_pdf_path(pdf_path) {
            Ok(path) => path,
            Err(e) => {
                return Ok(ValidationOutcome {
                    validator_id,
                    tool_identity: None,
                    status: ToolAvailability::ExecutionFailed,
                    profile: profile_id,
                    machine_result: None,
                    stdout_truncated: None,
                    stderr_truncated: Some(e.to_string()),
                    duration_ms: start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                    is_mock: false,
                });
            }
        };

        // 2. Check tool availability (fail-closed offline status)
        let tool_identity = match self.detect_identity() {
            Ok(id) => Some(ToolIdentity {
                name: "verapdf".to_string(),
                version: id.version.clone(),
                profile_version: id.profile_version,
                invocation_class: id.invocation_class,
                resolution_method: id.resolution_method,
            }),
            Err(e) => {
                return Ok(ValidationOutcome {
                    validator_id,
                    tool_identity: None,
                    status: ToolAvailability::ToolUnavailableOffline,
                    profile: profile_id,
                    machine_result: None,
                    stdout_truncated: None,
                    stderr_truncated: Some(e.to_string()),
                    duration_ms: start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                    is_mock: false,
                });
            }
        };

        // 3. Check version compatibility
        if let Some(ref id) = tool_identity {
            if !Self::version_satisfies(&id.version, MIN_VERAPDF_VERSION) {
                return Ok(ValidationOutcome {
                    validator_id,
                    tool_identity: Some(id.clone()),
                    status: ToolAvailability::VersionUnsupported,
                    profile: profile_id,
                    machine_result: None,
                    stdout_truncated: None,
                    stderr_truncated: Some(format!(
                        "VeraPDF version {} < minimum {}",
                        id.version, MIN_VERAPDF_VERSION
                    )),
                    duration_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!(
                        "VeraPDF version {} < minimum {}",
                        id.version, MIN_VERAPDF_VERSION
                    )),
                    is_mock: false,
                });
            }
        }

        // 4. Build validation command with canonical path
        let mut args = Vec::new();
        if self.config.use_xml_output {
            args.push("--format".to_string());
            args.push("xml".to_string());
        }
        args.push("--profile".to_string());
        args.push(profile.to_string());
        args.push(canonical_pdf_path.to_string_lossy().to_string());

        let mut cmd = Command::new(&self.config.executable);
        cmd.args(&args);
        for jvm_arg in &self.config.jvm_args {
            cmd.env("JAVA_TOOL_OPTIONS", jvm_arg);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        // Execute with timeout
        let output = self.run_command_with_timeout(&mut cmd, self.config.timeout_secs)?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Truncate output if needed
        let stdout_truncated = Self::truncate_output(&stdout, self.config.max_output_bytes);
        let stderr_truncated = Self::truncate_output(&stderr, self.config.max_output_bytes);

        if !output.status.success() {
            let status = if output.status.code() == Some(124) || output.status.code() == Some(143) {
                ToolAvailability::Timeout
            } else {
                ToolAvailability::ExecutionFailed
            };
            return Ok(ValidationOutcome {
                validator_id,
                tool_identity,
                status,
                profile: profile_id,
                machine_result: None,
                stdout_truncated: Some(stdout_truncated),
                stderr_truncated: Some(stderr_truncated),
                duration_ms,
                error: Some(format!(
                    "VeraPDF exited with status: {:?}",
                    output.status.code()
                )),
                is_mock: false,
            });
        }

        // Parse machine-readable result
        let machine_result = if self.config.use_xml_output {
            Self::parse_xml_report(&stdout)?
        } else {
            Self::parse_text_report(&stdout)?
        };

        let status = match machine_result.overall_status {
            ValidatorVerdict::Pass => ToolAvailability::Pass,
            ValidatorVerdict::Fail => ToolAvailability::Fail,
            ValidatorVerdict::Unparseable => ToolAvailability::ExecutionFailed,
        };

        Ok(ValidationOutcome {
            validator_id,
            tool_identity,
            status,
            profile: profile_id,
            machine_result: Some(machine_result),
            stdout_truncated: Some(stdout_truncated),
            stderr_truncated: Some(stderr_truncated),
            duration_ms,
            error: None,
            is_mock: false,
        })
    }

    /// Run a command and capture output with timeout.
    fn run_command_with_timeout(
        &self,
        cmd: &mut Command,
        timeout_secs: u64,
    ) -> Result<std::process::Output> {
        let mut child = cmd
            .spawn()
            .map_err(|e| ExternalValidationError::ExecutionFailed(e.to_string()))?;
        let start = Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    let output = child
                        .wait_with_output()
                        .map_err(|e| ExternalValidationError::ExecutionFailed(e.to_string()))?;
                    return Ok(output);
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        return Err(ExternalValidationError::Timeout(timeout_secs));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(ExternalValidationError::ExecutionFailed(e.to_string())),
            }
        }
    }

    /// Run a simple command and return stdout.
    fn run_command(&self, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new(&self.config.executable);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = self.run_command_with_timeout(&mut cmd, 30)?;
        if !output.status.success() {
            return Err(ExternalValidationError::ExecutionFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Parse VeraPDF version from --version output.
    fn parse_version(output: &str) -> Result<String> {
        // VeraPDF --version outputs like "verapdf 1.30.2" or "1.30.2"
        let version = output
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().last())
            .map(|s| s.trim().to_string())
            .ok_or_else(|| {
                ExternalValidationError::ResultUnparseable("No version in output".to_string())
            })?;
        Ok(version)
    }

    /// Detect supported profile versions (simplified).
    fn detect_profile_version(_executable: &str, _jvm_args: &[String]) -> Result<Option<String>> {
        // Could run `verapdf --list-profiles` and parse
        // For now, return None (profile version tracked separately)
        Ok(None)
    }

    /// Check if version satisfies minimum (simple semver comparison).
    fn version_satisfies(current: &str, minimum: &str) -> bool {
        let current_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();
        let min_parts: Vec<u32> = minimum.split('.').filter_map(|s| s.parse().ok()).collect();

        for (c, m) in current_parts.iter().zip(min_parts.iter()) {
            if c > m {
                return true;
            }
            if c < m {
                return false;
            }
        }
        current_parts.len() >= min_parts.len()
    }

    /// Get invocation class string.
    fn invocation_class(&self) -> String {
        if self.config.use_xml_output {
            "CLI --format xml".to_string()
        } else {
            "CLI".to_string()
        }
    }

    /// Truncate output to max bytes.
    fn truncate_output(output: &str, max_bytes: usize) -> String {
        if output.len() <= max_bytes {
            output.to_string()
        } else {
            format!(
                "{}... [TRUNCATED {} bytes]",
                &output[..max_bytes],
                output.len() - max_bytes
            )
        }
    }

    /// Parse VeraPDF XML report into machine-readable result.
    pub fn parse_xml_report(xml: &str) -> Result<MachineValidationResult> {
        let trimmed = xml.trim();
        if trimmed.is_empty() {
            return Err(ExternalValidationError::ResultUnparseable(
                "Empty XML report".to_string(),
            ));
        }

        let report: VerapdfXmlReport = quick_xml::de::from_str(trimmed).map_err(|e| {
            ExternalValidationError::ResultUnparseable(format!("XML parse error: {}", e))
        })?;

        let overall_status = report
            .validation_result
            .as_ref()
            .and_then(|r| r.status.as_ref())
            .map(|s| match s.to_ascii_lowercase().as_str() {
                "passed" => ValidatorVerdict::Pass,
                "failed" => ValidatorVerdict::Fail,
                _ => ValidatorVerdict::Unparseable,
            })
            .unwrap_or(ValidatorVerdict::Unparseable);

        let mut rules = Vec::new();
        if let Some(rules_xml) = report.rules {
            for rule in rules_xml.rule {
                rules.push(ValidationRuleFinding {
                    rule_id: rule.id.unwrap_or_default(),
                    spec_reference: rule.specification,
                    status: match rule.status.as_deref().map(|s| s.to_ascii_lowercase()).as_deref() {
                        Some("passed") => RuleFindingStatus::Pass,
                        Some("failed") => RuleFindingStatus::Fail,
                        Some("warning") => RuleFindingStatus::Warning,
                        Some("notapplicable") => RuleFindingStatus::NotApplicable,
                        _ => RuleFindingStatus::NotChecked,
                    },
                    message: rule.message,
                });
            }
        }

        let mut metadata = BTreeMap::new();
        if let Some(profile) = report.profile {
            if let Some(name) = profile.name {
                metadata.insert("profile_name".to_string(), serde_json::Value::String(name));
            }
            if let Some(version) = profile.version {
                metadata.insert(
                    "profile_version".to_string(),
                    serde_json::Value::String(version),
                );
            }
        }
        if let Some(version) = report.version {
            metadata.insert(
                "report_version".to_string(),
                serde_json::Value::String(version),
            );
        }

        Ok(MachineValidationResult {
            overall_status,
            rules,
            report_digest: None,
            metadata,
        })
    }

    /// Parse VeraPDF text report (fallback).
    pub fn parse_text_report(text: &str) -> Result<MachineValidationResult> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(ExternalValidationError::ResultUnparseable(
                "Empty text report".to_string(),
            ));
        }

        // Strict line-level marker detection per Rule 21 (avoid brittle substring scraping)
        let has_passed = text.lines().any(|l| {
            let line = l.trim();
            line == "Validation result: PASSED" || line.starts_with("Validation result: PASSED")
        });
        let has_failed = text.lines().any(|l| {
            let line = l.trim();
            line == "Validation result: FAILED" || line.starts_with("Validation result: FAILED")
        });

        let overall_status = if has_passed && !has_failed {
            ValidatorVerdict::Pass
        } else if has_failed {
            ValidatorVerdict::Fail
        } else {
            ValidatorVerdict::Unparseable
        };

        Ok(MachineValidationResult {
            overall_status,
            rules: Vec::new(),
            report_digest: None,
            metadata: BTreeMap::new(),
        })
    }
}

impl Default for VerapdfHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate an exact artifact-produced PDF output against configured requirements.
pub fn validate_artifact_output(
    pdf_path: &Path,
    operation_type: &str,
    source_fixture_id: &str,
    config: &ExternalValidationConfig,
    harness: &VerapdfHarness,
) -> Result<ExternalValidationEvidence> {
    let canonical = validate_input_pdf_path(pdf_path)?;
    let bytes = std::fs::read(&canonical)
        .map_err(|e| ExternalValidationError::PathSafetyViolation(e.to_string()))?;
    let byte_length = bytes.len() as u64;

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let output_sha256 = format!("{:x}", hasher.finalize());

    let mut outcomes = Vec::new();
    for req in &config.requirements {
        if req.validator == "verapdf" {
            let outcome = harness.validate(&canonical, &req.profile.0)?;
            outcomes.push(outcome);
        } else {
            outcomes.push(ValidationOutcome {
                validator_id: req.id.clone(),
                tool_identity: None,
                status: ToolAvailability::ToolUnavailableOffline,
                profile: req.profile.clone(),
                machine_result: None,
                stdout_truncated: None,
                stderr_truncated: Some(format!("Validator '{}' unsupported offline", req.validator)),
                duration_ms: 0,
                error: Some(format!("Validator '{}' unsupported offline", req.validator)),
                is_mock: false,
            });
        }
    }

    Ok(ExternalValidationEvidence::new(
        operation_type,
        source_fixture_id,
        output_sha256,
        byte_length,
        &config.requirements,
        outcomes,
        None,
    ))
}

/// Mock VeraPDF harness for FAST_LOCAL tests.
pub struct MockVerapdfHarness {
    pub should_pass: bool,
    pub should_timeout: bool,
    pub should_fail_launch: bool,
    pub should_be_offline: bool,
    pub custom_output: Option<String>,
    pub version: String,
}

impl Default for MockVerapdfHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl MockVerapdfHarness {
    pub fn new() -> Self {
        Self {
            should_pass: true,
            should_timeout: false,
            should_fail_launch: false,
            should_be_offline: false,
            custom_output: None,
            version: MIN_VERAPDF_VERSION.to_string(),
        }
    }

    pub fn validate(&self, _pdf_path: &Path, profile: &str) -> Result<ValidationOutcome> {
        let validator_id =
            ValidatorId(format!("mock-verapdf-{}", profile.replace(['/', ':'], "-")));
        let profile_id = ProfileId(profile.to_string());
        let start = Instant::now();

        if self.should_be_offline {
            return Ok(ValidationOutcome {
                validator_id,
                tool_identity: None,
                status: ToolAvailability::ToolUnavailableOffline,
                profile: profile_id,
                machine_result: None,
                stdout_truncated: None,
                stderr_truncated: Some("Mock: tool unavailable in strict offline environment".to_string()),
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some("Mock: tool unavailable in strict offline environment".to_string()),
                is_mock: true,
            });
        }

        if self.should_fail_launch {
            return Ok(ValidationOutcome {
                validator_id,
                tool_identity: None,
                status: ToolAvailability::ToolUnavailable,
                profile: profile_id,
                machine_result: None,
                stdout_truncated: None,
                stderr_truncated: Some("Mock: executable not found".to_string()),
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some("Mock: executable not found".to_string()),
                is_mock: true,
            });
        }

        let tool_identity = Some(ToolIdentity {
            name: "verapdf".to_string(),
            version: self.version.clone(),
            profile_version: None,
            invocation_class: "MOCK".to_string(),
            resolution_method: ToolResolutionMethod::ConfiguredPath,
        });

        if !Self::version_satisfies(&self.version, MIN_VERAPDF_VERSION) {
            return Ok(ValidationOutcome {
                validator_id,
                tool_identity,
                status: ToolAvailability::VersionUnsupported,
                profile: profile_id,
                machine_result: None,
                stdout_truncated: None,
                stderr_truncated: Some(format!(
                    "Mock: version {} < minimum {}",
                    self.version, MIN_VERAPDF_VERSION
                )),
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some(format!(
                    "Mock: version {} < minimum {}",
                    self.version, MIN_VERAPDF_VERSION
                )),
                is_mock: true,
            });
        }

        if self.should_timeout {
            return Ok(ValidationOutcome {
                validator_id,
                tool_identity,
                status: ToolAvailability::Timeout,
                profile: profile_id,
                machine_result: None,
                stdout_truncated: None,
                stderr_truncated: Some("Mock: timeout".to_string()),
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some("Mock: timeout".to_string()),
                is_mock: true,
            });
        }

        let machine_result = if self.should_pass {
            MachineValidationResult {
                overall_status: ValidatorVerdict::Pass,
                rules: vec![ValidationRuleFinding {
                    rule_id: "MOCK_RULE".to_string(),
                    spec_reference: Some("MOCK".to_string()),
                    status: RuleFindingStatus::Pass,
                    message: Some("Mock validation passed".to_string()),
                }],
                report_digest: Some("mock-digest".to_string()),
                metadata: BTreeMap::new(),
            }
        } else {
            MachineValidationResult {
                overall_status: ValidatorVerdict::Fail,
                rules: vec![ValidationRuleFinding {
                    rule_id: "MOCK_RULE".to_string(),
                    spec_reference: Some("MOCK".to_string()),
                    status: RuleFindingStatus::Fail,
                    message: Some("Mock validation failed".to_string()),
                }],
                report_digest: Some("mock-digest".to_string()),
                metadata: BTreeMap::new(),
            }
        };

        Ok(ValidationOutcome {
            validator_id,
            tool_identity,
            status: if self.should_pass {
                ToolAvailability::Pass
            } else {
                ToolAvailability::Fail
            },
            profile: profile_id,
            machine_result: Some(machine_result),
            stdout_truncated: self.custom_output.clone(),
            stderr_truncated: None,
            duration_ms: start.elapsed().as_millis() as u64,
            error: None,
            is_mock: true,
        })
    }

    fn version_satisfies(current: &str, minimum: &str) -> bool {
        let current_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();
        let min_parts: Vec<u32> = minimum.split('.').filter_map(|s| s.parse().ok()).collect();

        for (c, m) in current_parts.iter().zip(min_parts.iter()) {
            if c > m {
                return true;
            }
            if c < m {
                return false;
            }
        }
        current_parts.len() >= min_parts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn mock_pass_produces_pass_and_mock_marker() {
        let harness = MockVerapdfHarness::new();
        let outcome = harness
            .validate(Path::new("/tmp/fixture.pdf"), "PDF/A-2b")
            .unwrap();
        assert_eq!(outcome.status, ToolAvailability::Pass);
        assert!(outcome.is_mock);
        assert!(outcome.machine_result.is_some());
    }

    #[test]
    fn mock_fail_produces_fail() {
        let mut harness = MockVerapdfHarness::new();
        harness.should_pass = false;
        let outcome = harness
            .validate(Path::new("/tmp/fixture.pdf"), "PDF/A-2b")
            .unwrap();
        assert_eq!(outcome.status, ToolAvailability::Fail);
    }

    #[test]
    fn mock_timeout_and_unavailable_are_distinct_fail_closed_states() {
        let mut timeout = MockVerapdfHarness::new();
        timeout.should_timeout = true;
        assert_eq!(
            timeout.validate(Path::new("x"), "PDF/A-2b").unwrap().status,
            ToolAvailability::Timeout
        );

        let mut unavailable = MockVerapdfHarness::new();
        unavailable.should_fail_launch = true;
        assert_eq!(
            unavailable
                .validate(Path::new("x"), "PDF/A-2b")
                .unwrap()
                .status,
            ToolAvailability::ToolUnavailable
        );
    }

    #[test]
    fn mock_version_gate_is_fail_closed() {
        let mut old = MockVerapdfHarness::new();
        old.version = "1.0.0".to_string();
        assert_eq!(
            old.validate(Path::new("x"), "PDF/A-2b").unwrap().status,
            ToolAvailability::VersionUnsupported
        );
    }

    #[test]
    fn version_satisfies_compares_minimum() {
        assert!(VerapdfHarness::version_satisfies("1.30.2", "1.30.2"));
        assert!(VerapdfHarness::version_satisfies("2.0.0", "1.30.2"));
        assert!(!VerapdfHarness::version_satisfies("1.30.1", "1.30.2"));
    }

    #[test]
    fn text_report_parser_distinguishes_pass_fail_unparseable() {
        assert_eq!(
            VerapdfHarness::parse_text_report("Validation result: PASSED")
                .unwrap()
                .overall_status,
            ValidatorVerdict::Pass
        );
        assert_eq!(
            VerapdfHarness::parse_text_report("Validation result: FAILED")
                .unwrap()
                .overall_status,
            ValidatorVerdict::Fail
        );
        assert_eq!(
            VerapdfHarness::parse_text_report("unexpected output")
                .unwrap()
                .overall_status,
            ValidatorVerdict::Unparseable
        );
    }

    #[test]
    fn xml_report_parser_extracts_verdict_and_rules() {
        let xml = r#"<report version="1.0"><profile name="PDF/A-2b" version="1.0"/><validationResult status="passed"/><rules><rule id="6.2.11.4" specification="ISO 19005-2" status="passed" message="ok"/></rules></report>"#;
        let result = VerapdfHarness::parse_xml_report(xml).unwrap();
        assert_eq!(result.overall_status, ValidatorVerdict::Pass);
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].status, RuleFindingStatus::Pass);
    }

    #[test]
    fn text_report_parser_rejects_empty_and_brittle_substring_masquerade() {
        // Empty text is unparseable error
        assert!(VerapdfHarness::parse_text_report("   ").is_err());

        // File path containing substring PASSED or FAILED is NOT treated as verdict (Rule 21)
        let masquerade = "Processing file /var/tmp/PASSED_doc.pdf...\nDone.";
        let res = VerapdfHarness::parse_text_report(masquerade).unwrap();
        assert_eq!(res.overall_status, ValidatorVerdict::Unparseable);

        // Proper structured line is required
        let valid_line = "Processing /tmp/file.pdf\nValidation result: PASSED\nChecked 120 rules.";
        let valid_res = VerapdfHarness::parse_text_report(valid_line).unwrap();
        assert_eq!(valid_res.overall_status, ValidatorVerdict::Pass);
    }

    #[test]
    fn xml_report_parser_rejects_empty_and_malformed() {
        assert!(VerapdfHarness::parse_xml_report("").is_err());
        assert!(VerapdfHarness::parse_xml_report("<unclosed tag").is_err());
    }

    #[test]
    fn mock_offline_produces_tool_unavailable_offline() {
        let mut harness = MockVerapdfHarness::new();
        harness.should_be_offline = true;
        let outcome = harness.validate(Path::new("dummy.pdf"), "PDF/A-2b").unwrap();
        assert_eq!(outcome.status, ToolAvailability::ToolUnavailableOffline);
        assert!(outcome.status.is_unavailable());
        assert!(!outcome.status.is_pass());
        assert!(outcome.is_mock);
    }

    #[test]
    fn path_safety_validation_rejects_nonexistent_and_directories() {
        // Non-existent path
        let non_existent = Path::new("/definitely/non/existent/file_12345.pdf");
        assert!(validate_input_pdf_path(non_existent).is_err());

        // Directory
        let temp_dir = std::env::temp_dir();
        assert!(validate_input_pdf_path(&temp_dir).is_err());
    }
}
