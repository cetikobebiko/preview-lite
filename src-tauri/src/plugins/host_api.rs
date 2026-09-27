//! Minimal Rhai 1.25.1 Sandbox, Typed Hook Payload Validators, and Permission-Aware Host APIs.

use super::error::{PluginError, PluginErrorCode};
use super::limits::*;
use super::manifest::{PluginHook, PluginPermission};
use super::registry::PluginHookResultV1;
use rhai::packages::Package;
use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicFnPackage, BasicMapPackage, BasicStringPackage,
    LogicPackage,
};
use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, Position, Scope, AST};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Context provided to Rhai host API callbacks during execution.
#[derive(Debug, Clone, Default)]
pub struct HostApiContext {
    pub execution_id: String,
    pub permissions: HashSet<PluginPermission>,
    pub cancel_flag: Arc<AtomicBool>,
    pub logs: Arc<Mutex<Vec<String>>>,
    pub total_log_bytes: Arc<AtomicUsize>,
    pub notifications: Arc<Mutex<Vec<String>>>,
}

/// Builds a hardened, minimal Rhai 1.25.1 engine with all unsafe capabilities stripped.
pub fn build_sandboxed_rhai_engine(context: HostApiContext) -> Engine {
    // 1. Raw minimal engine without standard packages
    let mut engine = Engine::new_raw();

    // 2. Explicitly load safe packages only (Blob, File, System, Eval, Import are omitted)
    ArithmeticPackage::new().register_into_engine(&mut engine);
    LogicPackage::new().register_into_engine(&mut engine);
    BasicStringPackage::new().register_into_engine(&mut engine);
    BasicArrayPackage::new().register_into_engine(&mut engine);
    BasicMapPackage::new().register_into_engine(&mut engine);
    BasicFnPackage::new().register_into_engine(&mut engine);

    // 3. Concrete security boundaries (Rhai 1.25.1 direct limits)
    engine.set_max_operations(MAX_RHAI_OPERATIONS);
    engine.set_max_variables(MAX_RHAI_VARIABLES);
    engine.set_max_functions(MAX_RHAI_FUNCTIONS);
    engine.set_max_modules(MAX_RHAI_MODULES);
    engine.set_max_expr_depths(MAX_RHAI_EXPR_DEPTH, MAX_RHAI_EXPR_DEPTH);
    engine.set_max_call_levels(MAX_RHAI_CALL_STACK_DEPTH);
    engine.set_max_strings_interned(0);
    engine.set_max_string_size(MAX_RHAI_STRING_SIZE);
    engine.set_max_array_size(MAX_RHAI_ARRAY_SIZE);
    engine.set_max_map_size(MAX_RHAI_MAP_SIZE);

    // 4. Stdout and Debug isolation (prevent corrupting length-prefixed stdout framing)
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});

    // 5. Cooperative cancellation hook (None = continue, Some(token) = terminate with ErrorTerminated)
    let cancel_flag = context.cancel_flag.clone();
    engine.on_progress(move |_| {
        if cancel_flag.load(Ordering::SeqCst) {
            Some(Dynamic::from("cancelled"))
        } else {
            None
        }
    });

    // 6. Register host APIs with call-time permission and resource checks
    let logs_clone = context.logs.clone();
    let total_logs = context.total_log_bytes.clone();
    let perms_log = context.permissions.clone();
    engine.register_fn(
        "plugin_log",
        move |msg: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            if !perms_log.contains(&PluginPermission::PluginLog) {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    Dynamic::from(format!(
                        "[{}] 'plugin_log' çağrısı için 'plugin:log' izni gereklidir.",
                        PluginErrorCode::PluginPermissionDenied.as_str()
                    )),
                    Position::NONE,
                )));
            }
            let text = msg.to_string();
            let added_len = text.len();
            let prev = total_logs.fetch_add(added_len, Ordering::SeqCst);
            if prev.saturating_add(added_len) > MAX_HOST_API_LOG_BYTES {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    Dynamic::from(format!(
                        "[{}] Eklenti günlük boyutu üst sınırı aştı (max {} byte).",
                        PluginErrorCode::PluginResourceLimitExceeded.as_str(),
                        MAX_HOST_API_LOG_BYTES
                    )),
                    Position::NONE,
                )));
            }

            if let Ok(mut list) = logs_clone.lock() {
                list.push(text);
            }
            Ok(())
        },
    );

    let perms_ui_notify = context.permissions.clone();
    let notifs_clone = context.notifications.clone();
    engine.register_fn(
        "ui_notify",
        move |msg: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            if !perms_ui_notify.contains(&PluginPermission::UiNotification) {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    Dynamic::from(format!(
                        "[{}] 'ui_notify' çağrısı için 'ui:notification' izni gereklidir.",
                        PluginErrorCode::PluginPermissionDenied.as_str()
                    )),
                    Position::NONE,
                )));
            }
            let text = msg.to_string();
            if text.len() > MAX_HOST_API_NOTIFICATION_STRING_BYTES {
                return Err(Box::new(EvalAltResult::ErrorRuntime(
                    Dynamic::from(format!(
                        "[{}] Bildirim metni izin verilen üst sınırı aştı (max {} byte).",
                        PluginErrorCode::PluginResourceLimitExceeded.as_str(),
                        MAX_HOST_API_NOTIFICATION_STRING_BYTES
                    )),
                    Position::NONE,
                )));
            }

            if let Ok(mut list) = notifs_clone.lock() {
                if list.len() >= MAX_HOST_API_NOTIFICATIONS {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        Dynamic::from(format!(
                            "[{}] Maksimum bildirim sayısı aşıldı (max {} adet).",
                            PluginErrorCode::PluginResourceLimitExceeded.as_str(),
                            MAX_HOST_API_NOTIFICATIONS
                        )),
                        Position::NONE,
                    )));
                }
                list.push(text);
            }
            Ok(())
        },
    );

    engine
}

/// Validates payload JSON structure, limits, and hook-specific schemas.
pub fn validate_hook_payload(
    hook: PluginHook,
    payload: &serde_json::Value,
) -> Result<(), PluginError> {
    let raw_payload_str = payload.to_string();
    if raw_payload_str.len() > MAX_PLUGIN_HOOK_PAYLOAD_BYTES {
        return Err(PluginError::new(
            PluginErrorCode::PluginPayloadTooLarge,
            format!(
                "Kanca yükü {} byte; izin verilen üst sınır {} byte.",
                raw_payload_str.len(),
                MAX_PLUGIN_HOOK_PAYLOAD_BYTES
            ),
        ));
    }

    validate_json_depth_and_strings(payload, 1)?;

    match hook {
        PluginHook::OnDocumentOpen => {
            let obj = payload.as_object().ok_or_else(|| {
                PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnDocumentOpen yükü JSON nesnesi olmalıdır.",
                )
            })?;
            for k in obj.keys() {
                if k != "page_count" {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginManifestInvalid,
                        format!("OnDocumentOpen beklenmeyen alan içeriyor: '{}'", k),
                    ));
                }
            }
            if !obj.contains_key("page_count") || !obj["page_count"].is_u64() {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnDocumentOpen yükü geçerli bir 'page_count' (u32) alanı içermelidir.",
                ));
            }
        }
        PluginHook::OnCustomCommand => {
            let obj = payload.as_object().ok_or_else(|| {
                PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnCustomCommand yükü JSON nesnesi olmalıdır.",
                )
            })?;
            for k in obj.keys() {
                if k != "command_name" && k != "parameters" {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginManifestInvalid,
                        format!("OnCustomCommand beklenmeyen alan içeriyor: '{}'", k),
                    ));
                }
            }
            if !obj.contains_key("command_name") || !obj["command_name"].is_string() {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnCustomCommand geçerli bir 'command_name' dizesi içermelidir.",
                ));
            }
        }
        PluginHook::OnExportComplete => {
            let obj = payload.as_object().ok_or_else(|| {
                PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnExportComplete yükü JSON nesnesi olmalıdır.",
                )
            })?;
            for k in obj.keys() {
                if k != "export_format" && k != "success" && k != "page_count" {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginManifestInvalid,
                        format!("OnExportComplete beklenmeyen alan içeriyor: '{}'", k),
                    ));
                }
            }
            if !obj.contains_key("export_format") || !obj["export_format"].is_string() {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnExportComplete geçerli bir 'export_format' dizesi içermelidir.",
                ));
            }
            if !obj.contains_key("success") || !obj["success"].is_boolean() {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "OnExportComplete geçerli bir 'success' boolean alanı içermelidir.",
                ));
            }
        }
    }

    Ok(())
}

fn validate_json_depth_and_strings(
    val: &serde_json::Value,
    current_depth: usize,
) -> Result<(), PluginError> {
    if current_depth > MAX_PLUGIN_HOOK_JSON_DEPTH {
        return Err(PluginError::new(
            PluginErrorCode::PluginPayloadTooLarge,
            format!(
                "JSON yük derinliği {} sınırını aştı.",
                MAX_PLUGIN_HOOK_JSON_DEPTH
            ),
        ));
    }

    match val {
        serde_json::Value::String(s) => {
            if s.len() > MAX_PLUGIN_HOOK_STRING_BYTES {
                return Err(PluginError::new(
                    PluginErrorCode::PluginPayloadTooLarge,
                    format!(
                        "JSON yükü içindeki dize boyutu {} byte sınırını aştı.",
                        MAX_PLUGIN_HOOK_STRING_BYTES
                    ),
                ));
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                validate_json_depth_and_strings(item, current_depth + 1)?;
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                validate_json_depth_and_strings(item, current_depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Evaluates a plugin script in the minimal sandbox and invokes the specified hook function.
/// Converts output into a strictly structured `PluginHookResultV1`.
pub fn execute_plugin_script(
    script: &str,
    hook: PluginHook,
    payload: &serde_json::Value,
    permissions: &[PluginPermission],
    execution_id: &str,
    cancel_flag: Arc<AtomicBool>,
) -> Result<PluginHookResultV1, PluginError> {
    validate_hook_payload(hook, payload)?;

    let payload_str = payload.to_string();
    let perms_set: HashSet<PluginPermission> = permissions.iter().copied().collect();
    let logs = Arc::new(Mutex::new(Vec::new()));
    let notifications = Arc::new(Mutex::new(Vec::new()));
    let total_log_bytes = Arc::new(AtomicUsize::new(0));

    let context = HostApiContext {
        execution_id: execution_id.to_string(),
        permissions: perms_set,
        cancel_flag,
        logs: logs.clone(),
        total_log_bytes,
        notifications: notifications.clone(),
    };

    let engine = build_sandboxed_rhai_engine(context);

    let ast: AST = engine.compile(script).map_err(|e| {
        let err_msg = e.to_string();
        if err_msg.contains("TooManyFunctions") {
            PluginError::new(
                PluginErrorCode::PluginResourceLimitExceeded,
                format!("Fonksiyon sayısı sınırı aşıldı: {}", err_msg),
            )
        } else {
            PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                format!("Rhai derleme hatası: {}", e),
            )
        }
    })?;

    let mut scope = Scope::new();
    scope.push("payload_json", payload_str.clone());

    let hook_fn_name = hook.as_str();
    let res: Dynamic =
        match engine.call_fn::<Dynamic>(&mut scope, &ast, hook_fn_name, (payload_str,)) {
            Ok(val) => Ok(val),
            Err(err) => {
                if matches!(*err, EvalAltResult::ErrorFunctionNotFound(..)) {
                    engine.call_fn::<Dynamic>(&mut scope, &ast, hook_fn_name, ())
                } else {
                    Err(err)
                }
            }
        }
        .map_err(map_eval_alt_result_to_plugin_error)?;

    // Script Return Restriction:
    // Only () or String (<= 1MB) is allowed. Complex structures, maps, arrays, fn pointers return PluginResultInvalid.
    let text_output = if res.is_unit() {
        None
    } else if res.is_string() {
        let s = res.into_string().unwrap_or_default();
        if s.len() > MAX_IPC_RESULT_BYTES {
            return Err(PluginError::new(
                PluginErrorCode::PluginResultTooLarge,
                format!(
                    "Eklenti sonuç boyutu {} byte; izin verilen üst sınır {} byte.",
                    s.len(),
                    MAX_IPC_RESULT_BYTES
                ),
            ));
        }
        Some(s)
    } else {
        return Err(PluginError::new(
            PluginErrorCode::PluginResultInvalid,
            "Eklenti betiği yalnızca birim değer () veya metin (string) döndürebilir.",
        ));
    };

    let final_logs = logs.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let final_notifications = notifications
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();

    Ok(PluginHookResultV1 {
        schema_version: CURRENT_PLUGIN_RESULT_SCHEMA_VERSION,
        execution_id: execution_id.to_string(),
        text: text_output,
        logs: final_logs,
        notifications: final_notifications,
        warnings: Vec::new(),
    })
}

fn map_eval_alt_result_to_plugin_error(err: Box<EvalAltResult>) -> PluginError {
    let full_str = format!("{:?} {}", err, err);
    if full_str.contains("PLUGIN_PERMISSION_DENIED") {
        return PluginError::new(PluginErrorCode::PluginPermissionDenied, err.to_string());
    }
    if full_str.contains("PLUGIN_RESOURCE_LIMIT_EXCEEDED") {
        return PluginError::new(
            PluginErrorCode::PluginResourceLimitExceeded,
            err.to_string(),
        );
    }
    if full_str.to_lowercase().contains("operation")
        || full_str.to_lowercase().contains("variable")
        || full_str.to_lowercase().contains("function")
        || full_str.to_lowercase().contains("stack")
        || full_str.to_lowercase().contains("data too large")
        || full_str.to_lowercase().contains("depth")
    {
        return PluginError::new(
            PluginErrorCode::PluginResourceLimitExceeded,
            err.to_string(),
        );
    }

    let mut current = &*err;
    loop {
        match current {
            EvalAltResult::ErrorInFunctionCall(_, _, inner, _) => {
                current = inner;
            }
            EvalAltResult::ErrorRuntime(val, _) => {
                let msg = val.to_string();
                if msg.contains("PLUGIN_PERMISSION_DENIED") {
                    return PluginError::new(PluginErrorCode::PluginPermissionDenied, msg);
                }
                if msg.contains("PLUGIN_RESOURCE_LIMIT_EXCEEDED") {
                    return PluginError::new(PluginErrorCode::PluginResourceLimitExceeded, msg);
                }
                return PluginError::new(PluginErrorCode::PluginExecutionNotFound, msg);
            }
            EvalAltResult::ErrorTooManyOperations(_)
            | EvalAltResult::ErrorTooManyVariables(_)
            | EvalAltResult::ErrorStackOverflow(_)
            | EvalAltResult::ErrorDataTooLarge(_, _) => {
                return PluginError::new(
                    PluginErrorCode::PluginResourceLimitExceeded,
                    current.to_string(),
                );
            }
            EvalAltResult::ErrorTerminated(val, _) => {
                let msg = val.to_string();
                if msg.contains("PLUGIN_PERMISSION_DENIED") {
                    return PluginError::new(PluginErrorCode::PluginPermissionDenied, msg);
                }
                return PluginError::new(
                    PluginErrorCode::PluginExecutionCancelled,
                    "Eklenti yürütmesi iptal edildi.",
                );
            }
            EvalAltResult::ErrorFunctionNotFound(fn_name, _) => {
                return PluginError::new(
                    PluginErrorCode::PluginHookNotDeclared,
                    format!("Kanca fonksiyonu bulunamadı: '{}'", fn_name),
                );
            }
            _ => {
                let msg = current.to_string();
                if msg.contains("PLUGIN_PERMISSION_DENIED") {
                    return PluginError::new(PluginErrorCode::PluginPermissionDenied, msg);
                }
                if msg.to_lowercase().contains("operation")
                    || msg.to_lowercase().contains("variable")
                    || msg.to_lowercase().contains("function")
                    || msg.to_lowercase().contains("stack")
                    || msg.to_lowercase().contains("data too large")
                    || msg.to_lowercase().contains("depth")
                {
                    return PluginError::new(PluginErrorCode::PluginResourceLimitExceeded, msg);
                }
                return PluginError::new(PluginErrorCode::PluginExecutionNotFound, msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rhai_function_count_limit() {
        let context = HostApiContext::default();
        let engine = build_sandboxed_rhai_engine(context);

        // Define 130 functions (limit is 128)
        let mut script = String::new();
        for i in 0..130 {
            script.push_str(&format!("fn f_{}() {{ return {}; }}\n", i, i));
        }
        let res = engine.compile(&script);
        assert!(res.is_err());
    }

    #[test]
    fn test_rhai_expression_depth_limit() {
        let context = HostApiContext::default();
        let engine = build_sandboxed_rhai_engine(context);

        // 35 levels of nested expressions: 1 + (1 + (1 + ...))
        let mut script = "1".to_string();
        for _ in 0..35 {
            script = format!("(1 + {})", script);
        }
        let res = engine.eval::<i64>(&script);
        assert!(res.is_err());
    }

    #[test]
    fn test_rhai_string_allocation_limit() {
        let context = HostApiContext::default();
        let engine = build_sandboxed_rhai_engine(context);

        // Grow string beyond 256 KB
        let script = r#"
            let s = "a";
            while s.len() < 300000 {
                s = s + s;
            }
            s
        "#;
        let res = engine.eval::<String>(script);
        assert!(res.is_err());
    }

    #[test]
    fn test_rhai_array_size_limit() {
        let context = HostApiContext::default();
        let engine = build_sandboxed_rhai_engine(context);

        // Push beyond 10,000 array elements
        let script = r#"
            let arr = [];
            for i in 0..10050 {
                arr.push(i);
            }
            arr
        "#;
        let res = engine.eval::<rhai::Array>(script);
        assert!(res.is_err());
    }

    #[test]
    fn test_rhai_map_size_limit() {
        let context = HostApiContext::default();
        let engine = build_sandboxed_rhai_engine(context);

        // Push beyond 1,000 map entries
        let script = r#"
            let m = #{};
            for i in 0..1050 {
                m["k" + i] = i;
            }
            m
        "#;
        let res = engine.eval::<rhai::Map>(script);
        assert!(res.is_err());
    }

    #[test]
    fn test_rhai_return_type_contract() {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });

        // () is accepted
        let unit_script = "fn on_custom_command(p) {}";
        let unit_res = execute_plugin_script(
            unit_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-1",
            cancel_flag.clone(),
        );
        assert!(unit_res.is_ok());
        assert_eq!(unit_res.unwrap().text, None);

        // String is accepted
        let str_script = "fn on_custom_command(p) { return \"success result\"; }";
        let str_res = execute_plugin_script(
            str_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-2",
            cancel_flag.clone(),
        );
        assert!(str_res.is_ok());
        assert_eq!(str_res.unwrap().text, Some("success result".to_string()));

        // Array return is rejected
        let arr_script = "fn on_custom_command(p) { return [1, 2]; }";
        let arr_res = execute_plugin_script(
            arr_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-3",
            cancel_flag.clone(),
        );
        assert_eq!(
            arr_res.unwrap_err().code,
            PluginErrorCode::PluginResultInvalid
        );

        // Map return is rejected
        let map_script = "fn on_custom_command(p) { return #{ a: 1 }; }";
        let map_res = execute_plugin_script(
            map_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-4",
            cancel_flag.clone(),
        );
        assert_eq!(
            map_res.unwrap_err().code,
            PluginErrorCode::PluginResultInvalid
        );

        // Function pointer return is rejected
        let fn_script = "fn helper() {} fn on_custom_command(p) { return Fn(\"helper\"); }";
        let fn_res = execute_plugin_script(
            fn_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[],
            "exec-5",
            cancel_flag,
        );
        assert_eq!(
            fn_res.unwrap_err().code,
            PluginErrorCode::PluginResultInvalid
        );
    }

    #[test]
    fn test_host_api_log_byte_flood_rejected() {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });

        // Log flood exceeding 64 KB total
        let flood_script = r#"
            fn on_custom_command(p) {
                let msg = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"; // 64 bytes
                let i = 0;
                while i < 1100 {
                    plugin_log(msg); // 1100 * 64 > 64 KB
                    i += 1;
                }
            }
        "#;
        let res = execute_plugin_script(
            flood_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[PluginPermission::PluginLog],
            "exec-log-flood",
            cancel_flag,
        );
        assert_eq!(
            res.unwrap_err().code,
            PluginErrorCode::PluginResourceLimitExceeded
        );
    }

    #[test]
    fn test_host_api_notification_count_and_string_limit() {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let payload = serde_json::json!({ "command_name": "test", "parameters": {} });

        // Notification string > 4 KB
        let large_notif_script = r#"
            fn on_custom_command(p) {
                let big_s = "a";
                while big_s.len() < 4097 {
                    big_s = big_s + big_s;
                }
                ui_notify(big_s);
            }
        "#;
        let res1 = execute_plugin_script(
            large_notif_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[PluginPermission::UiNotification],
            "exec-notif-size",
            cancel_flag.clone(),
        );
        assert_eq!(
            res1.unwrap_err().code,
            PluginErrorCode::PluginResourceLimitExceeded
        );

        // Notification count > 32
        let count_notif_script = r#"
            fn on_custom_command(p) {
                let i = 0;
                while i < 35 {
                    ui_notify("alert");
                    i += 1;
                }
            }
        "#;
        let res2 = execute_plugin_script(
            count_notif_script,
            PluginHook::OnCustomCommand,
            &payload,
            &[PluginPermission::UiNotification],
            "exec-notif-count",
            cancel_flag,
        );
        assert_eq!(
            res2.unwrap_err().code,
            PluginErrorCode::PluginResourceLimitExceeded
        );
    }

    #[test]
    fn test_rhai_stdout_and_debug_produce_no_output() {
        let context = HostApiContext::default();
        let engine = build_sandboxed_rhai_engine(context);

        // print and debug must execute harmlessly and without writing to stdout
        let script = r#"
            print("hello stdout");
            debug("hello debug");
        "#;
        assert!(engine.eval::<()>(script).is_ok());
    }
}
