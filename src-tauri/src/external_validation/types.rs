//! Shared types for external validation harness.
//!
//! This module defines the machine-readable contract between the application
//! and external validators (VeraPDF, etc.). No external tool is invoked here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Unique identifier for a validator requirement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ValidatorId(pub String);

/// Unique identifier for a validation profile (e.g., "PDF/A-2b", "PDF/UA-1").
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub String);

/// The feature or operation that produces output requiring validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationFeature {
    /// PDF/UA machine validation (disabled feature)
    PdfUaMachineValidation,
    /// PDF/X export (disabled feature)
    PdfXExport,
    /// Sanitization output (supplementary)
    Sanitization,
    /// Secure redaction output (supplementary)
    Redaction,
    /// Searchable PDF export (disabled feature)
    SearchablePdfExport,
    /// PAdES signing (disabled feature)
    PadesSigning,
    /// Signature verification (disabled feature)
    SignatureVerification,
    /// LTV upgrade (disabled feature)
    Ltv,
    /// PDF auto-tagging (disabled feature)
    PdfAutoTag,
    /// Reading order editing (disabled feature)
    ReadingOrderEditing,
    /// Custom/other
    Other(String),
}

impl ValidationFeature {
    pub fn as_str(&self) -> &str {
        match self {
            ValidationFeature::PdfUaMachineValidation => "pdf_ua_machine_validation",
            ValidationFeature::PdfXExport => "pdf_x_export",
            ValidationFeature::Sanitization => "sanitization",
            ValidationFeature::Redaction => "redaction",
            ValidationFeature::SearchablePdfExport => "searchable_pdf_export",
            ValidationFeature::PadesSigning => "pades_signing",
            ValidationFeature::SignatureVerification => "signature_verification",
            ValidationFeature::Ltv => "ltv",
            ValidationFeature::PdfAutoTag => "pdf_auto_tag",
            ValidationFeature::ReadingOrderEditing => "reading_order_editing",
            ValidationFeature::Other(s) => s,
        }
    }
}

/// Whether a validator check is mandatory for a compliance claim or optional diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requiredness {
    /// Validator must pass for the compliance claim to be valid.
    /// Missing tool → FAIL (not SKIPPED).
    RequiredForClaim,
    /// Validator provides supplementary evidence; failure does not block claim.
    OptionalDiagnostic,
}

/// Execution layer for the validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLayer {
    /// Runs in isolated worker process (ephemeral job directory).
    WorkerProcess,
    /// Runs in main process (not recommended for untrusted input).
    MainProcess,
    /// Runs as part of artifact smoke test (installed artifact → production operation → output).
    ArtifactSmoke,
}

/// Tool identity information for evidence traceability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIdentity {
    /// Tool name (e.g., "verapdf", "pac", "dss").
    pub name: String,
    /// Exact tool version (e.g., "1.30.2").
    pub version: String,
    /// Profile/rule-set version used (e.g., "PDF/A-2b", "ISO-15930-7:2008").
    pub profile_version: Option<String>,
    /// Invocation class/mode (e.g., "CLI", "library", "CLI --format xml").
    pub invocation_class: String,
    /// How the tool was resolved (e.g., "configured_path", "PATH", "bundled").
    pub resolution_method: ToolResolutionMethod,
}

/// How the validator executable was located.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResolutionMethod {
    /// Explicit configured local executable path.
    ConfiguredPath,
    /// Found via PATH lookup (policy-permitted).
    PathLookup,
    /// Bundled with application.
    Bundled,
    /// Not resolved (tool unavailable).
    Unresolved,
}

/// Bounded availability states for external validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAvailability {
    /// Tool executable found and version compatible.
    Available,
    /// Tool executable not found or not executable.
    ToolUnavailable,
    /// Tool executable not found in strict offline environment (zero network / no package install).
    ToolUnavailableOffline,
    /// Tool found but version unsupported.
    VersionUnsupported,
    /// Required profile not available in tool.
    ProfileUnavailable,
    /// Execution failed (non-zero exit, crash, signal).
    ExecutionFailed,
    /// Execution timed out.
    Timeout,
    /// Validator reported PASS.
    Pass,
    /// Validator reported FAIL.
    Fail,
}

impl ToolAvailability {
    /// Returns true if the validator completed and reported PASS.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// Returns true if the validator executable could not be found or launched.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::ToolUnavailable | Self::ToolUnavailableOffline)
    }

    /// String representation matching bounded vocabulary.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::ToolUnavailable => "tool_unavailable",
            Self::ToolUnavailableOffline => "tool_unavailable_offline",
            Self::VersionUnsupported => "version_unsupported",
            Self::ProfileUnavailable => "profile_unavailable",
            Self::ExecutionFailed => "execution_failed",
            Self::Timeout => "timeout",
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

/// Result of a single validator execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationOutcome {
    /// Validator identifier.
    pub validator_id: ValidatorId,
    /// Tool identity (populated if Available).
    pub tool_identity: Option<ToolIdentity>,
    /// Availability/execution status.
    pub status: ToolAvailability,
    /// Profile that was validated against.
    pub profile: ProfileId,
    /// Machine-readable result (if parser succeeded).
    pub machine_result: Option<MachineValidationResult>,
    /// Raw stdout (bounded capture).
    pub stdout_truncated: Option<String>,
    /// Raw stderr (bounded capture).
    pub stderr_truncated: Option<String>,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Error details if status is not Pass.
    pub error: Option<String>,
    /// Whether this was a mock validator run (test only).
    pub is_mock: bool,
}

/// Machine-readable validation result (structured output from validator).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineValidationResult {
    /// Overall validator verdict.
    pub overall_status: ValidatorVerdict,
    /// Per-rule findings (if available).
    pub rules: Vec<ValidationRuleFinding>,
    /// Report digest for tamper evidence.
    pub report_digest: Option<String>,
    /// Validator-specific metadata.
    pub metadata: BTreeMap<String, serde_json::Value>,
}

/// Validator's overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidatorVerdict {
    Pass,
    Fail,
    /// Parser could not determine verdict from output.
    Unparseable,
}

/// Individual rule finding from validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRuleFinding {
    /// Rule identifier (validator-specific).
    pub rule_id: String,
    /// Rule specification reference (e.g., "ISO 19005-2:2011 6.2.11.4").
    pub spec_reference: Option<String>,
    /// Finding status.
    pub status: RuleFindingStatus,
    /// Human-readable message (bounded).
    pub message: Option<String>,
}

/// Status of a single validation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleFindingStatus {
    Pass,
    Fail,
    Warning,
    NotApplicable,
    NotChecked,
}

/// Complete external validation evidence for an output artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalValidationEvidence {
    /// Operation that produced the output.
    pub operation_type: String,
    /// Source fixture identifier.
    pub source_fixture_id: String,
    /// Output file SHA-256.
    pub output_sha256: String,
    /// Output file byte length.
    pub output_byte_length: u64,
    /// Validator outcomes.
    pub outcomes: Vec<ValidationOutcome>,
    /// Overall classification: all RequiredForClaim must be Pass.
    pub overall_status: OverallValidationStatus,
    /// Timestamp of validation.
    pub timestamp: String,
}

/// Overall validation status for the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallValidationStatus {
    /// All RequiredForClaim validators passed.
    Compliant,
    /// At least one RequiredForClaim validator failed or unavailable.
    NonCompliant,
    /// No RequiredForClaim validators defined (diagnostic only).
    DiagnosticOnly,
    /// Validation not executed (e.g., tool unavailable in CI).
    NotExecuted,
}

impl OverallValidationStatus {
    /// Computes overall validation status from configured requirements and actual outcomes.
    ///
    /// Fail-closed semantics:
    /// - If any `RequiredForClaim` requirement is not in `outcomes` or its status is not `Pass`
    ///   (including `ToolUnavailable`, `ToolUnavailableOffline`, `VersionUnsupported`, `Timeout`, `Fail`),
    ///   the overall status is `NonCompliant`.
    /// - If there is at least one `RequiredForClaim` requirement and all pass, returns `Compliant`.
    /// - If no `RequiredForClaim` requirements exist: returns `DiagnosticOnly` if outcomes exist,
    ///   or `NotExecuted` if outcomes is empty.
    pub fn compute(requirements: &[ValidatorRequirement], outcomes: &[ValidationOutcome]) -> Self {
        let required_reqs: Vec<&ValidatorRequirement> = requirements
            .iter()
            .filter(|r| r.requiredness == Requiredness::RequiredForClaim)
            .collect();

        if required_reqs.is_empty() {
            return if outcomes.is_empty() {
                Self::NotExecuted
            } else {
                Self::DiagnosticOnly
            };
        }

        let outcome_map: BTreeMap<&ValidatorId, &ValidationOutcome> = outcomes
            .iter()
            .map(|o| (&o.validator_id, o))
            .collect();

        for req in required_reqs {
            match outcome_map.get(&req.id) {
                Some(outcome) if outcome.status == ToolAvailability::Pass => continue,
                _ => return Self::NonCompliant,
            }
        }

        Self::Compliant
    }
}

impl ExternalValidationEvidence {
    /// Constructs a complete evidence record with evaluated overall status and timestamp.
    pub fn new(
        operation_type: impl Into<String>,
        source_fixture_id: impl Into<String>,
        output_sha256: impl Into<String>,
        output_byte_length: u64,
        requirements: &[ValidatorRequirement],
        outcomes: Vec<ValidationOutcome>,
        timestamp: Option<String>,
    ) -> Self {
        let overall_status = OverallValidationStatus::compute(requirements, &outcomes);
        let ts = timestamp.unwrap_or_else(|| {
            match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => format!("unix:{}", d.as_secs()),
                Err(_) => "unknown".to_string(),
            }
        });
        Self {
            operation_type: operation_type.into(),
            source_fixture_id: source_fixture_id.into(),
            output_sha256: output_sha256.into(),
            output_byte_length,
            outcomes,
            overall_status,
            timestamp: ts,
        }
    }
}

/// Requirement for a specific external validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorRequirement {
    /// Unique requirement ID.
    pub id: ValidatorId,
    /// Feature that requires this validator.
    pub feature: ValidationFeature,
    /// Validator tool name.
    pub validator: String,
    /// Profile to validate against.
    pub profile: ProfileId,
    /// Whether this check is required for compliance claim.
    pub requiredness: Requiredness,
    /// Execution layer.
    pub execution_layer: ExecutionLayer,
    /// Minimum supported tool version (if enforced).
    pub min_version: Option<String>,
    /// Maximum supported tool version (if known).
    pub max_version: Option<String>,
    /// Timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum output capture size (bytes).
    pub max_output_bytes: usize,
    /// Additional validator-specific arguments.
    pub extra_args: Vec<String>,
}

/// Configuration for the external validation harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalValidationConfig {
    /// Validator requirements registry.
    pub requirements: Vec<ValidatorRequirement>,
    /// Tool resolution configuration.
    pub tool_resolution: ToolResolutionConfig,
    /// Default timeout for validators without explicit timeout.
    pub default_timeout_secs: u64,
    /// Default max output capture size.
    pub default_max_output_bytes: usize,
}

/// Tool resolution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResolutionConfig {
    /// Explicit tool paths by validator name.
    pub explicit_paths: BTreeMap<String, PathBuf>,
    /// Whether PATH lookup is permitted.
    pub allow_path_lookup: bool,
    /// Whether bundled tools are used.
    pub use_bundled: bool,
}

impl Default for ExternalValidationConfig {
    fn default() -> Self {
        Self {
            requirements: Vec::new(),
            tool_resolution: ToolResolutionConfig {
                explicit_paths: BTreeMap::new(),
                allow_path_lookup: false,
                use_bundled: false,
            },
            default_timeout_secs: 300,
            default_max_output_bytes: 10_485_760, // 10 MB
        }
    }
}

impl ExternalValidationConfig {
    /// Documented external validation requirements seeded from the design
    /// documents (`pdf-ua-validation-strategy.md`). VeraPDF is a research
    /// candidate and the machine-validation capabilities are disabled, so every
    /// entry is `OptionalDiagnostic`; none is `RequiredForClaim`.
    pub fn documented_requirements() -> Vec<ValidatorRequirement> {
        vec![
            ValidatorRequirement {
                id: ValidatorId("verapdf-pdf-ua-1".to_string()),
                feature: ValidationFeature::PdfUaMachineValidation,
                validator: "verapdf".to_string(),
                profile: ProfileId("PDF/UA-1".to_string()),
                requiredness: Requiredness::OptionalDiagnostic,
                execution_layer: ExecutionLayer::ArtifactSmoke,
                min_version: Some("1.30.2".to_string()),
                max_version: None,
                timeout_secs: 300,
                max_output_bytes: 10_485_760,
                extra_args: Vec::new(),
            },
            ValidatorRequirement {
                id: ValidatorId("verapdf-pdf-ua-2".to_string()),
                feature: ValidationFeature::PdfUaMachineValidation,
                validator: "verapdf".to_string(),
                profile: ProfileId("PDF/UA-2".to_string()),
                requiredness: Requiredness::OptionalDiagnostic,
                execution_layer: ExecutionLayer::ArtifactSmoke,
                min_version: Some("1.30.2".to_string()),
                max_version: None,
                timeout_secs: 300,
                max_output_bytes: 10_485_760,
                extra_args: Vec::new(),
            },
        ]
    }
}

/// Errors from the external validation harness.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExternalValidationError {
    #[error("Validator requirement not found: {0}")]
    RequirementNotFound(String),
    #[error("Tool unavailable: {0}")]
    ToolUnavailable(String),
    #[error("Tool version unsupported: {0}")]
    VersionUnsupported(String),
    #[error("Profile unavailable: {0}")]
    ProfileUnavailable(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Execution timed out after {0}s")]
    Timeout(u64),
    #[error("Output too large: {0} bytes")]
    OutputTooLarge(usize),
    #[error("Result unparseable: {0}")]
    ResultUnparseable(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Command injection attempt blocked")]
    CommandInjectionBlocked,
    #[error("Path safety violation: {0}")]
    PathSafetyViolation(String),
}

/// Result type for harness operations.
pub type Result<T> = std::result::Result<T, ExternalValidationError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_no_requirements_and_no_lookup() {
        let config = ExternalValidationConfig::default();
        assert!(config.requirements.is_empty());
        assert!(!config.tool_resolution.allow_path_lookup);
        assert!(!config.tool_resolution.use_bundled);
    }

    #[test]
    fn documented_requirements_are_optional_and_artifact_scoped() {
        let requirements = ExternalValidationConfig::documented_requirements();
        assert!(!requirements.is_empty());
        for requirement in &requirements {
            assert_eq!(requirement.requiredness, Requiredness::OptionalDiagnostic);
            assert_eq!(requirement.execution_layer, ExecutionLayer::ArtifactSmoke);
            assert!(requirement.min_version.is_some());
        }
    }

    #[test]
    fn validation_outcome_round_trips_through_serde() {
        let outcome = ValidationOutcome {
            validator_id: ValidatorId("verapdf-pdf-ua-1".to_string()),
            tool_identity: None,
            status: ToolAvailability::ToolUnavailableOffline,
            profile: ProfileId("PDF/UA-1".to_string()),
            machine_result: None,
            stdout_truncated: None,
            stderr_truncated: Some("offline: tool not installed".to_string()),
            duration_ms: 12,
            error: Some("offline: tool not installed".to_string()),
            is_mock: false,
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let back: ValidationOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back.validator_id, outcome.validator_id);
        assert_eq!(back.status, ToolAvailability::ToolUnavailableOffline);
        assert!(back.status.is_unavailable());
        assert!(!back.status.is_pass());
        assert_eq!(back.status.as_str(), "tool_unavailable_offline");
        assert_eq!(back.profile, outcome.profile);
    }

    #[test]
    fn overall_status_compute_fail_closed_on_required_checks() {
        let req1 = ValidatorRequirement {
            id: ValidatorId("req-1".to_string()),
            feature: ValidationFeature::PdfUaMachineValidation,
            validator: "verapdf".to_string(),
            profile: ProfileId("PDF/UA-1".to_string()),
            requiredness: Requiredness::RequiredForClaim,
            execution_layer: ExecutionLayer::ArtifactSmoke,
            min_version: None,
            max_version: None,
            timeout_secs: 30,
            max_output_bytes: 1024,
            extra_args: Vec::new(),
        };
        let requirements = vec![req1];

        // 1. Missing outcome -> NonCompliant
        assert_eq!(
            OverallValidationStatus::compute(&requirements, &[]),
            OverallValidationStatus::NonCompliant
        );

        // 2. ToolUnavailableOffline -> NonCompliant (fail-closed for required check)
        let outcome_offline = ValidationOutcome {
            validator_id: ValidatorId("req-1".to_string()),
            tool_identity: None,
            status: ToolAvailability::ToolUnavailableOffline,
            profile: ProfileId("PDF/UA-1".to_string()),
            machine_result: None,
            stdout_truncated: None,
            stderr_truncated: None,
            duration_ms: 5,
            error: Some("Offline".to_string()),
            is_mock: false,
        };
        assert_eq!(
            OverallValidationStatus::compute(&requirements, &[outcome_offline]),
            OverallValidationStatus::NonCompliant
        );

        // 3. Timeout -> NonCompliant
        let outcome_timeout = ValidationOutcome {
            validator_id: ValidatorId("req-1".to_string()),
            tool_identity: None,
            status: ToolAvailability::Timeout,
            profile: ProfileId("PDF/UA-1".to_string()),
            machine_result: None,
            stdout_truncated: None,
            stderr_truncated: None,
            duration_ms: 30000,
            error: Some("Timeout".to_string()),
            is_mock: false,
        };
        assert_eq!(
            OverallValidationStatus::compute(&requirements, &[outcome_timeout]),
            OverallValidationStatus::NonCompliant
        );

        // 4. Pass -> Compliant
        let outcome_pass = ValidationOutcome {
            validator_id: ValidatorId("req-1".to_string()),
            tool_identity: None,
            status: ToolAvailability::Pass,
            profile: ProfileId("PDF/UA-1".to_string()),
            machine_result: None,
            stdout_truncated: None,
            stderr_truncated: None,
            duration_ms: 100,
            error: None,
            is_mock: false,
        };
        assert_eq!(
            OverallValidationStatus::compute(&requirements, &[outcome_pass]),
            OverallValidationStatus::Compliant
        );
    }

    #[test]
    fn overall_status_compute_handles_optional_diagnostics() {
        let req_opt = ValidatorRequirement {
            id: ValidatorId("opt-1".to_string()),
            feature: ValidationFeature::PdfUaMachineValidation,
            validator: "verapdf".to_string(),
            profile: ProfileId("PDF/UA-1".to_string()),
            requiredness: Requiredness::OptionalDiagnostic,
            execution_layer: ExecutionLayer::ArtifactSmoke,
            min_version: None,
            max_version: None,
            timeout_secs: 30,
            max_output_bytes: 1024,
            extra_args: Vec::new(),
        };
        let requirements = vec![req_opt];

        // Zero outcomes -> NotExecuted
        assert_eq!(
            OverallValidationStatus::compute(&requirements, &[]),
            OverallValidationStatus::NotExecuted
        );

        // Optional tool unavailable offline -> DiagnosticOnly (never blocks claim)
        let outcome_offline = ValidationOutcome {
            validator_id: ValidatorId("opt-1".to_string()),
            tool_identity: None,
            status: ToolAvailability::ToolUnavailableOffline,
            profile: ProfileId("PDF/UA-1".to_string()),
            machine_result: None,
            stdout_truncated: None,
            stderr_truncated: None,
            duration_ms: 0,
            error: Some("tool unavailable offline".to_string()),
            is_mock: false,
        };
        assert_eq!(
            OverallValidationStatus::compute(&requirements, &[outcome_offline]),
            OverallValidationStatus::DiagnosticOnly
        );
    }

    #[test]
    fn external_validation_evidence_new_populates_fields() {
        let evidence = ExternalValidationEvidence::new(
            "smoke_test",
            "core-release-smoke",
            "dummy-sha256",
            1234,
            &[],
            Vec::new(),
            Some("2026-09-04T12:00:00Z".to_string()),
        );
        assert_eq!(evidence.operation_type, "smoke_test");
        assert_eq!(evidence.source_fixture_id, "core-release-smoke");
        assert_eq!(evidence.output_sha256, "dummy-sha256");
        assert_eq!(evidence.output_byte_length, 1234);
        assert_eq!(evidence.overall_status, OverallValidationStatus::NotExecuted);
        assert_eq!(evidence.timestamp, "2026-09-04T12:00:00Z");
    }
}
