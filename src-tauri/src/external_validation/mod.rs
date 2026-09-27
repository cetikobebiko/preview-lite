//! External validation harness (11I-E5).
//!
//! Machine-readable contract (`types`) and bounded VeraPDF CLI integration
//! (`verapdf`) for validating application-produced PDF outputs against
//! external standards validators. No external tool is downloaded or bundled
//! here; execution requires local tool availability and reports
//! `ToolUnavailable` / `TOOL_UNAVAILABLE` fail-closed otherwise.

pub mod types;
pub mod verapdf;

pub use types::{
    ExecutionLayer, ExternalValidationConfig, ExternalValidationError, ExternalValidationEvidence,
    MachineValidationResult, OverallValidationStatus, ProfileId, Requiredness, RuleFindingStatus,
    ToolAvailability, ToolIdentity, ToolResolutionMethod, ValidationFeature, ValidationOutcome,
    ValidationRuleFinding, ValidatorId, ValidatorRequirement, ValidatorVerdict,
};
pub use verapdf::{
    validate_artifact_output, validate_input_pdf_path, MockVerapdfHarness, VerapdfConfig,
    VerapdfHarness, MIN_VERAPDF_VERSION,
};
