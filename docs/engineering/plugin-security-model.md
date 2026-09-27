# Plugin Security Model & Worker Isolation Specification (V1)

## 1. Overview & Architecture Demarcation
Antigravity provides an isolated plugin architecture designed for local extensibility while preserving strict fail-closed security invariants. Plugins execute in dedicated subprocess workers (`--run-plugin-worker`) running a stripped-down, unprivileged Rhai 1.25.1 sandbox.

### Core Architectural Invariants
1. **DocumentSession Demarcation**: The Plugin subsystem and Rhai workers have **zero access** to `DocumentSession`, `SessionWorker`, PDFium engine handles, PDF credentials, file paths, or raw document content. Plugins cannot inspect or mutate document state.
2. **Fail-Closed Default**: In production, the plugin system is disabled by default (`feature_enabled = false` in Rust backend, `plugin_system.enabled = false` in frontend registry). Any attempt to inspect, install, grant consent, or execute when disabled is rejected immediately.
3. **Model A Feature Linearization**: All state modifications acquire `feature_gate_lock` before reading or updating `feature_enabled`. Privilege-reducing or lifecycle-terminating commands (`uninstall_plugin`, `set_plugin_enabled(false)`, `cancel_plugin_execution`, `list_installed_plugins`, `get_plugin_execution_status`) remain available even when the feature is globally disabled to guarantee safe cleanup and observation.

---

## 2. Package V1 Format & Path Security

### Package Layout
A Plugin Package V1 is a local directory containing **strictly exactly two files**:
- `manifest.json`: Plugin identity and permission manifest (max 64 KB).
- `main.rhai`: Rhai 1.25.1 execution script (max 1 MB).

Any directory containing extra files, hidden files (e.g. `.DS_Store`), nested subdirectories, symlinks, or hardlinks (`nlink > 1` on Unix) is rejected immediately with `PluginPackageStructureInvalid` or `PluginHardlinkRejected`.

### Path Security & Directory Permissions
- **Private Plugin Root**: All installed plugins reside under `<app_data_dir>/plugins/installed/<plugin_id>/<version>/`.
- **Creation-Time Directory Permissions**: Directory paths are created with `0700` (`rwx------`) and files with `0600` (`rw-------`) on Unix platforms.
- **Parent-Controlled Root**: The worker process discovers its root strictly via the parent-controlled environment variable `ANTIGRAVITY_PLUGIN_ROOT`.
- **Symlink & Traversal Resistance**: All path accesses undergo canonicalization and containment verification ensuring no components escape the plugin root or cross symlinks.

### Domain-Separated Cryptographic Digest
Package integrity is calculated using domain-separated SHA-256 hashing:
```text
digest = SHA-256(
    "ANTIGRAVITY_PLUGIN_V1_PACKAGE\0" ||
    u32_be(len(manifest_bytes)) || manifest_bytes ||
    u32_be(len(script_bytes))   || script_bytes
)
```
This guarantees unambiguous digest calculation immune to delimiter injection or multi-file collision.

---

## 3. Explicit User Consent Model & Registry Persistence

### Single Policy Version Model
- Each installed plugin has a single `consent_policy_version: u32`.
- When installed, `consent_policy_version = 0` (unconsented).
- Explicit user consent (`grant_plugin_consent`) validates matching `expected_package_digest` and exact canonical permission set equality before advancing `consent_policy_version = 1`.
- A plugin cannot be enabled (`set_plugin_enabled(true)`) or executed unless `consent_policy_version == CURRENT_PLUGIN_CONSENT_POLICY_VERSION` (1).

### Atomic Registry Persistence & Logical Quarantine
- Plugin registry is persisted to `<app_data_dir>/plugins/registry.json` using atomic staging files (`registry.json.tmp`) followed by file sync (`fsync`/`sync_all`) and atomic rename (or `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH` on Windows).
- On startup and prior to execution, the supervisor verifies installed plugins. If any script, manifest, or package digest on disk does not match the registered hash, the corrupted plugin is logically quarantined: `enabled = false`, `consent_granted = false`, `consent_policy_version = 0` (preserving physical files in place without uncontainment risk).

---

## 4. Subprocess Worker & Host Supervisor Protocol

### Length-Prefixed IPC Framing
Communication over child `stdin` and `stdout` uses 4-byte big-endian length-prefixed JSON frames:
`[ 4-byte Big-Endian Length: N ] [ N-byte JSON Payload ]`

### 17-Step Mandatory Worker Revalidation
Before building or evaluating any Rhai script, the worker independently executes 17 strict validation steps:
1. Validate protocol version (`1`) and resource profile version (`1`).
2. Verify containment within `ANTIGRAVITY_PLUGIN_ROOT`.
3. Check `target_dir` exists and canonicalize path.
4. Verify Package V1 structure (no symlinks, no hardlinks, exactly two regular files).
5. Read `manifest.json` within bounds (<= 64 KB).
6. Strip UTF-8 BOM and scan for duplicate JSON keys.
7. Validate `manifest_version == 1`.
8. Validate plugin ID format (`[a-zA-Z0-9._-]{3,64}`).
9. Match manifest ID with request ID.
10. Match manifest version with request version.
11. Confirm `main_script == "main.rhai"`.
12. Verify requested hook is explicitly declared in `manifest.hooks`.
13. Verify requested permissions match declared manifest permissions exactly.
14. Compute and verify SHA-256 of `manifest.json`.
15. Compute and verify SHA-256 of `main.rhai`.
16. Compute and verify domain-separated package digest.
17. Decode and validate script UTF-8 content.

### Lifecycle, Cancellation, and Reap
- **Single Active Execution**: Supervisor enforces at most one concurrent active execution per plugin (`PluginExecutionBusy`).
- **Cooperative Cancellation**: Supervisor sends a length-prefixed `WorkerControlMessage { action: Cancel }` over stdin. Worker dedicated stdin listener sets `cancel_flag`. Rhai engine progress hook aborts execution.
- **Grace Period & Hard Kill**: Supervisor grants a 250ms cooperative grace period. If child does not exit, a hard `SIGKILL` is issued.
- **Post-Terminal Exit Grace & Reap**: After receiving the terminal response frame, supervisor awaits child process termination with a 500ms grace period and invokes `child.wait()` to reap zombie processes. Any additional byte received triggers immediate protocol violation error.

---

## 5. Rhai Sandbox Hardening & Resource Bounds

The Rhai engine is instantiated using raw minimal constructor `Engine::new_raw()` with all standard packages (file I/O, networking, eval, blob, process execution, reflection) completely excluded:

| Parameter | Limit | Failure Response |
| :--- | :--- | :--- |
| `max_operations` | 500,000 ops | `PluginResourceLimitExceeded` |
| `max_variables` | 1,000 variables | `PluginResourceLimitExceeded` |
| `max_functions` | 128 functions | `PluginResourceLimitExceeded` |
| `max_modules` | 0 (no imports) | `PluginManifestInvalid` |
| `max_call_levels` | 32 call frames | `PluginResourceLimitExceeded` |
| `max_expr_depth` | 32 levels | `PluginResourceLimitExceeded` |
| `max_string_size` | 256 KB (262,144 bytes) | `PluginResourceLimitExceeded` |
| `max_array_size` | 10,000 elements | `PluginResourceLimitExceeded` |
| `max_map_size` | 1,000 pairs | `PluginResourceLimitExceeded` |
| Execution Timeout | 10 seconds | `PluginExecutionTimeout` |

### Script Return Values & Host APIs
- **Script Return Restriction**: Only `()` (Unit) or `String` (<= 1MB) is permitted. Complex objects, arrays, maps, or function pointers return `PluginResultInvalid`.
- **Stdout Isolation**: `print` and `debug` in Rhai scripts are bound to no-ops to prevent stdout frame corruption.
- **Host APIs**:
  - `plugin_log(msg)`: Requires `plugin:log` permission. Max 64 KB total accumulated across calls.
  - `ui_notify(msg)`: Requires `ui:notification` permission. Max 4 KB per call, max 32 notifications total.
- **Structured Output**: Result is wrapped in canonical `PluginHookResultV1 { schema_version: 1, text, logs, notifications, warnings }`.
