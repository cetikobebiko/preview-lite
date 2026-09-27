//! Typed error definitions for the Antigravity plugin system.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginErrorCode {
    PluginFeatureDisabled,
    PluginManifestTooLarge,
    PluginManifestInvalid,
    PluginManifestVersionUnsupported,
    PluginManifestUnknownField,
    PluginIdInvalid,
    PluginVersionInvalid,
    PluginPermissionUnknown,
    PluginHookUnknown,
    PluginPathInvalid,
    PluginPathTraversal,
    PluginSymlinkRejected,
    PluginHardlinkRejected,
    PluginFileNotRegular,
    PluginScriptTooLarge,
    PluginPackageDigestMismatch,
    PluginAlreadyInstalled,
    PluginInstallConflict,
    PluginNotInstalled,
    PluginNotEnabled,
    PluginPermissionDenied,
    PluginConsentRequired,
    PluginConsentPolicyMismatch,
    PluginConsentRevoked,
    PluginRegistryCorrupt,
    PluginHookNotDeclared,
    PluginPayloadTooLarge,
    PluginWorkerStartFailed,
    PluginWorkerProtocolError,
    PluginExecutionTimeout,
    PluginExecutionCancelled,
    PluginExecutionBusy,
    PluginExecutionNotFound,
    PluginResourceLimitExceeded,
    PluginResultTooLarge,
    PluginResultInvalid,
    PluginWorkerCrashed,
    PluginUninstallBusy,
}

impl PluginErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PluginFeatureDisabled => "PLUGIN_FEATURE_DISABLED",
            Self::PluginManifestTooLarge => "PLUGIN_MANIFEST_TOO_LARGE",
            Self::PluginManifestInvalid => "PLUGIN_MANIFEST_INVALID",
            Self::PluginManifestVersionUnsupported => "PLUGIN_MANIFEST_VERSION_UNSUPPORTED",
            Self::PluginManifestUnknownField => "PLUGIN_MANIFEST_UNKNOWN_FIELD",
            Self::PluginIdInvalid => "PLUGIN_ID_INVALID",
            Self::PluginVersionInvalid => "PLUGIN_VERSION_INVALID",
            Self::PluginPermissionUnknown => "PLUGIN_PERMISSION_UNKNOWN",
            Self::PluginHookUnknown => "PLUGIN_HOOK_UNKNOWN",
            Self::PluginPathInvalid => "PLUGIN_PATH_INVALID",
            Self::PluginPathTraversal => "PLUGIN_PATH_TRAVERSAL",
            Self::PluginSymlinkRejected => "PLUGIN_SYMLINK_REJECTED",
            Self::PluginHardlinkRejected => "PLUGIN_HARDLINK_REJECTED",
            Self::PluginFileNotRegular => "PLUGIN_FILE_NOT_REGULAR",
            Self::PluginScriptTooLarge => "PLUGIN_SCRIPT_TOO_LARGE",
            Self::PluginPackageDigestMismatch => "PLUGIN_PACKAGE_DIGEST_MISMATCH",
            Self::PluginAlreadyInstalled => "PLUGIN_ALREADY_INSTALLED",
            Self::PluginInstallConflict => "PLUGIN_INSTALL_CONFLICT",
            Self::PluginNotInstalled => "PLUGIN_NOT_INSTALLED",
            Self::PluginNotEnabled => "PLUGIN_NOT_ENABLED",
            Self::PluginPermissionDenied => "PLUGIN_PERMISSION_DENIED",
            Self::PluginConsentRequired => "PLUGIN_CONSENT_REQUIRED",
            Self::PluginConsentPolicyMismatch => "PLUGIN_CONSENT_POLICY_MISMATCH",
            Self::PluginConsentRevoked => "PLUGIN_CONSENT_REVOKED",
            Self::PluginRegistryCorrupt => "PLUGIN_REGISTRY_CORRUPT",
            Self::PluginHookNotDeclared => "PLUGIN_HOOK_NOT_DECLARED",
            Self::PluginPayloadTooLarge => "PLUGIN_PAYLOAD_TOO_LARGE",
            Self::PluginWorkerStartFailed => "PLUGIN_WORKER_START_FAILED",
            Self::PluginWorkerProtocolError => "PLUGIN_WORKER_PROTOCOL_ERROR",
            Self::PluginExecutionTimeout => "PLUGIN_EXECUTION_TIMEOUT",
            Self::PluginExecutionCancelled => "PLUGIN_EXECUTION_CANCELLED",
            Self::PluginExecutionBusy => "PLUGIN_EXECUTION_BUSY",
            Self::PluginExecutionNotFound => "PLUGIN_EXECUTION_NOT_FOUND",
            Self::PluginResourceLimitExceeded => "PLUGIN_RESOURCE_LIMIT_EXCEEDED",
            Self::PluginResultTooLarge => "PLUGIN_RESULT_TOO_LARGE",
            Self::PluginResultInvalid => "PLUGIN_RESULT_INVALID",
            Self::PluginWorkerCrashed => "PLUGIN_WORKER_CRASHED",
            Self::PluginUninstallBusy => "PLUGIN_UNINSTALL_BUSY",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginError {
    pub code: PluginErrorCode,
    pub message: String,
}

impl PluginError {
    pub fn new(code: PluginErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn feature_disabled() -> Self {
        Self::new(
            PluginErrorCode::PluginFeatureDisabled,
            "Eklenti sistemi bu sürümde özellik bayrağıyla devre dışıdır.",
        )
    }

    pub fn to_wire(&self) -> PluginErrorWire {
        PluginErrorWire {
            code: self.code.as_str().to_string(),
            message: self.message.clone(),
        }
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for PluginError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginErrorWire {
    pub code: String,
    pub message: String,
}

impl From<PluginError> for PluginErrorWire {
    fn from(err: PluginError) -> Self {
        err.to_wire()
    }
}

impl From<PluginError> for String {
    fn from(err: PluginError) -> Self {
        err.to_string()
    }
}
