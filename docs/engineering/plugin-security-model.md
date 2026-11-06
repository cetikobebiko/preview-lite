# Plugin Security Model & Worker Isolation Specification (V1)

## 1. Overview & Architecture Demarcation
Antigravity provides an isolated plugin architecture designed for local extensibility while preserving strict fail-closed security invariants. Plugins execute in dedicated subprocess workers (`--run-plugin-worker`) running a stripped-down, unprivileged Rhai 1.25.1 sandbox.

### Core Architectural Invariants
1. **DocumentSession Demarcation**: The Plugin subsystem and Rhai workers have **zero access** to `DocumentSession`, `SessionWorker`, PDFium engine handles, PDF credentials, file paths, or raw document content. Plugins cannot inspect or mutate document state.
2. **Build-Profile Gate (5a)**: The backend gate's only source is the Rust build profile. `plugins::plugin_feature_gate_for_build(cfg!(debug_assertions))` → `PLUGIN_FEATURE_ENABLED_IN_THIS_BUILD`; Tauri `setup` builds the state with `PluginState::for_app(...)`, which stores that value before the state is managed. Development builds (`tauri dev`, `debug_assertions` on) open it; production builds (`tauri build`) keep it closed. There is **no** webview-callable command that changes the gate; `set_feature_enabled` is a Rust-only API used by tests. The webview can only *read* it through `get_plugin_feature_status` → `{ enabled, build_profile }`.
   - Frontend mirror: `plugin_system` in `src/featureRegistry.js` is `enabled: true, buildProfiles: ["development"]`, so it is runnable only when `getRuntimeFeatureContext().build === "development"` (Vite `import.meta.env.DEV`) and hidden in production builds. `buildProfiles` is an optional registry field added for this; the existing `build` context only affected visibility, never runnability.
   - Parity and mismatch: `src/pluginGateParity.test.js` checks that both sides decide the same per profile. If they still disagree at runtime (e.g. `tauri dev --release`: Vite dev server, release backend), `PluginManagerModal` queries `get_plugin_feature_status`, shows a mismatch warning and disables every privilege-increasing action; disable/uninstall stay available. A failed status query counts as closed. The opposite mismatch (`tauri build --debug`: production frontend, debug backend) hides the UI.
   - Any attempt to inspect, install, grant consent, enable or execute while the backend gate is closed returns `PLUGIN_FEATURE_DISABLED`.
3. **Model A Feature Linearization**: All state modifications acquire `feature_gate_lock` before reading or updating `feature_enabled`. Privilege-reducing or lifecycle-terminating commands (`uninstall_plugin`, `set_plugin_enabled(false)`, `cancel_plugin_execution`, `list_installed_plugins`, `get_plugin_execution_status`) remain available even when the feature is globally disabled to guarantee safe cleanup and observation.

---

## 2. Package V1 Format & Path Security

### Package Layout
A Plugin Package V1 is a local directory containing **strictly exactly two files**:
- `manifest.json`: Plugin identity and permission manifest (max 64 KB).
- `main.rhai`: Rhai 1.25.1 execution script (max 1 MB).

Any directory containing extra files, hidden files (e.g. `.DS_Store`), nested subdirectories, symlinks, or hardlinks (`nlink > 1` on Unix) is rejected immediately with `PluginPackageStructureInvalid` or `PluginHardlinkRejected`.

### Path Security & Directory Permissions
- **App Data Directory (5a)**: `<app_data_dir>` is Tauri's `app.path().app_data_dir()` for identifier `com.preview.lite` (macOS `~/Library/Application Support/com.preview.lite`, Windows `%APPDATA%\com.preview.lite`, Linux `~/.local/share/com.preview.lite`), resolved once in `setup`. If it cannot be resolved the state has no root and every write-path command fails closed with `PLUGIN_PATH_INVALID`. The previous `HOME`/`APPDATA` environment-variable resolver (`default_plugin_app_data_dir`, `…/antigravity-preview`) and `PluginState::new()`/`Default` were removed; no production code path builds a plugin root from environment variables any more. **Migration:** none. Production builds never had an open gate, so no production install could exist under the old path; development installs there (if any) are not carried over and must be reinstalled.
- **Private Plugin Root**: All installed plugins reside under `<app_data_dir>/plugins/installed/<plugin_id>/<version>/`.
- **Creation-Time Directory Permissions**: Directory paths are created with `0700` (`rwx------`) and files with `0600` (`rw-------`) on Unix platforms.
- **Parent-Controlled Root**: The worker process discovers its root strictly via the parent-controlled environment variable `ANTIGRAVITY_PLUGIN_ROOT`, which the supervisor sets to `<app_data_dir>/plugins` (the Tauri path above). This env channel is parent→child only; it is not a resolver.
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
- On startup, before enabling, before consent and before every execution, the host re-hashes the installed `manifest.json` + `main.rhai`. If the package digest no longer matches the consented one (or the files cannot be read), the plugin is logically quarantined: `enabled = false`, `consent_granted = false`, `consent_policy_version = 0` (preserving physical files in place without uncontainment risk). The quarantine is persisted; re-consent to the changed files is refused (`PLUGIN_PACKAGE_DIGEST_MISMATCH`), so the only way back is uninstall + reinstall.
- **Post-worker re-check (5a)**: after any failed worker run the host re-hashes again and quarantines if the files changed after the pre-check (the worker's own step 14–16 revalidation catches that window and refuses to run).
- **Error codes from the worker** are preserved end-to-end (previously every worker `Failed` response was rewritten to `PLUGIN_EXECUTION_NOT_FOUND`). An unknown or missing code is `PLUGIN_WORKER_PROTOCOL_ERROR`. Rhai script runtime errors (`throw`, type errors) use the new `PLUGIN_SCRIPT_RUNTIME_ERROR`.
- UI: `pluginLifecycleState()` in `src/pluginErrors.js` shows a plugin as *Karantinada* when consent is revoked after having been given (`reviewed_package_digest` set), and every `PLUGIN_*` code has a Turkish message (`PLUGIN_ERROR_MESSAGES`, completeness checked against `error.rs`).

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

---

## 6. Development-Build Usage, Example Package & Proof (5a)

### Scope
- Open only in development builds. Hooks are **not** wired to application events yet (that is 5b); the Plugin Manager runs a declared hook manually with a fixed, schema-valid sample payload (`on_custom_command` → `{"command_name":"manual_run","parameters":{}}`, `on_document_open` → `{"page_count":1}`, `on_export_complete` → `{"export_format":"pdf","success":true,"page_count":1}`). No payload carries document content, paths or credentials.
- Host APIs remain exactly `plugin_log` and `ui_notify`. Execution results (text, log lines, notifications) are shown in the Plugin Manager's execution box.

### Example package
`docs/engineering/examples/plugins/hello-log/` (`manifest.json` + `main.rhai`) uses both permissions and declares all three hooks. `on_custom_command` writes one log line (`hello-log: on_custom_command <payload>`), one notification (`Merhaba! hello-log eklentisi çalıştı.`) and returns `hello-log tamam`. A `.DS_Store` created by Finder inside that folder makes the package invalid (exactly-two-files rule); delete it before installing.

### Permission proof (scenario → test)
| Scenario | Tests |
|---|---|
| Ungranted host API call → `PermissionDenied` | `plugins::tests::test_host_api_permission_enforcement` (plugin_log), `plugins::host_api::tests::test_host_api_ui_notify_requires_notification_permission` (ui_notify), `plugin_hello_log_example::ungranted_host_api_call_ends_in_permission_denied_status` (real worker → status `FAILED` / `PLUGIN_PERMISSION_DENIED`) |
| Script or manifest changed after consent → quarantine | `plugins::tests::test_script_changed_after_consent_quarantines_plugin`, `plugins::tests::test_manifest_changed_after_consent_quarantines_plugin`, `plugins::tests::test_change_after_precheck_is_quarantined_after_worker_failure`, `plugins::tests::test_modified_script_on_disk_blocks_execution`, `plugins::tests::test_consent_reset_on_package_digest_change`, `plugin_hello_log_example::hello_log_disabled_then_tampered_is_rejected_and_quarantined` |
| Undeclared hook rejected | `plugins::tests::test_undeclared_hook_rejected` (host, before any worker); worker step 12 in `worker::revalidate_worker_target_directory` |
| Notification / log limits → error status | `plugins::host_api::tests::test_host_api_log_byte_flood_rejected`, `plugins::host_api::tests::test_host_api_notification_count_and_string_limit`, `plugin_hello_log_example::log_byte_limit_ends_in_resource_limit_status`, `plugin_hello_log_example::notification_limits_end_in_resource_limit_status` (status `FAILED` / `PLUGIN_RESOURCE_LIMIT_EXCEEDED`) |
| Unconsented plugin cannot be enabled or run | `plugins::tests::test_unconsented_plugin_cannot_be_enabled_or_executed` (incl. hand-edited `enabled=true` registry and stale policy version), `plugins::tests::test_plugin_installation_consent_and_persistence_lifecycle` |
| Gate closed → only restrictive commands | `plugins::tests::test_gate_closed_after_install_allows_only_restrictive_commands` (real installed plugin), `plugins::tests::test_disabled_*` (11 tests) |
| Gate per build profile | `plugins::tests::test_gate_decision_closed_for_production_profile`, `plugins::tests::test_gate_constant_follows_debug_assertions`, `plugins::tests::test_debug_build_gate_is_open` / `plugins::tests::test_release_build_gate_is_closed` (cfg branches), `src/pluginGateParity.test.js` |
| Example end-to-end | `plugin_hello_log_example::hello_log_example_runs_on_custom_command_through_real_worker` |
