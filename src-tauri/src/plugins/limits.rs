//! Bounded limits and constants for plugin system security.

/// Maximum allowable size for `manifest.json` (64 KB).
pub const MAX_MANIFEST_SIZE_BYTES: u64 = 64 * 1024;

/// Maximum allowable size for a single plugin script (1 MB).
pub const MAX_SCRIPT_SIZE_BYTES: u64 = 1024 * 1024;

/// Maximum allowable total size for an uncompressed plugin package directory (10 MB).
pub const MAX_PACKAGE_SIZE_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum allowable payload size sent over IPC / worker channels (256 KB).
pub const MAX_IPC_PAYLOAD_BYTES: usize = 256 * 1024;

/// Maximum allowable IPC execute request size (1 MB).
pub const MAX_IPC_REQUEST_BYTES: usize = 1024 * 1024;

/// Maximum allowable control channel message size (4 KB).
pub const MAX_IPC_CONTROL_BYTES: usize = 4096;
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 4096;

/// Maximum allowable execution result size returned from a plugin (1 MB).
pub const MAX_IPC_RESULT_BYTES: usize = 1024 * 1024;

/// Maximum allowable Rhai instruction operations per execution (500,000 ops).
pub const MAX_RHAI_OPERATIONS: u64 = 500_000;

/// Maximum allowable variables in Rhai scope (1,000).
pub const MAX_RHAI_VARIABLES: usize = 1_000;

/// Maximum allowable scripted functions in Rhai script (128).
pub const MAX_RHAI_FUNCTIONS: usize = 128;

/// Maximum allowable modules in Rhai (0 - module loading disabled).
pub const MAX_RHAI_MODULES: usize = 0;

/// Maximum expression depth for Rhai AST parser / evaluator (32).
pub const MAX_RHAI_EXPR_DEPTH: usize = 32;

/// Maximum function call stack depth for Rhai (32).
pub const MAX_RHAI_CALL_STACK_DEPTH: usize = 32;

/// Maximum string length allocated in Rhai sandbox (256 KB).
pub const MAX_RHAI_STRING_SIZE: usize = 256 * 1024;

/// Maximum array element count in Rhai sandbox (10,000).
pub const MAX_RHAI_ARRAY_SIZE: usize = 10_000;

/// Maximum map element count in Rhai sandbox (1,000).
pub const MAX_RHAI_MAP_SIZE: usize = 1_000;

/// Maximum total accumulated log bytes across all `plugin_log` calls (64 KB).
pub const MAX_HOST_API_LOG_BYTES: usize = 64 * 1024;

/// Maximum number of notifications across execution (32).
pub const MAX_HOST_API_NOTIFICATIONS: usize = 32;

/// Maximum string bytes for a single notification (4 KB).
pub const MAX_HOST_API_NOTIFICATION_STRING_BYTES: usize = 4096;

/// Maximum allowable hook payload bytes (256 KB).
pub const MAX_PLUGIN_HOOK_PAYLOAD_BYTES: usize = 256 * 1024;

/// Maximum allowed JSON nesting depth for hook payloads (8).
pub const MAX_PLUGIN_HOOK_JSON_DEPTH: usize = 8;

/// Maximum allowed string length inside hook JSON payload (64 KB).
pub const MAX_PLUGIN_HOOK_STRING_BYTES: usize = 64 * 1024;

/// Default overall execution timeout (10 seconds).
pub const DEFAULT_EXECUTION_TIMEOUT_SECS: u64 = 10;

/// Cooperative cancellation grace window before hard-kill fallback (250 ms).
pub const COOPERATIVE_CANCEL_GRACE_MS: u64 = 250;

/// Post-terminal supervisor exit grace window for child drain and exit (500 ms).
pub const PLUGIN_WORKER_EXIT_GRACE_MS: u64 = 500;

/// Maximum stderr drain capture buffer size (64 KB).
pub const MAX_STDERR_DRAIN_BYTES: usize = 64 * 1024;

/// Current supported plugin versions
pub const CURRENT_PLUGIN_CONSENT_POLICY_VERSION: u32 = 1;
pub const CURRENT_PLUGIN_MANIFEST_VERSION: u32 = 1;
pub const CURRENT_PLUGIN_PROTOCOL_VERSION: u32 = 1;
pub const CURRENT_PLUGIN_RESOURCE_PROFILE_VERSION: u32 = 1;
pub const CURRENT_PLUGIN_REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const CURRENT_PLUGIN_HOOK_SCHEMA_VERSION: u32 = 1;
pub const CURRENT_PLUGIN_RESULT_SCHEMA_VERSION: u32 = 1;
