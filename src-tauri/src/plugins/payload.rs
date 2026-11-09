//! Hook payload schema V1.
//!
//! Every hook has one typed payload struct with `deny_unknown_fields`. The host parses
//! the raw JSON from the webview into that struct in `PluginState::start_execution`
//! before any worker starts, and sends the worker the canonical re-serialization. The
//! worker parses it again with the same code before the script runs.
//!
//! Invariant: no payload carries a path, file name, document text, password or any
//! other document content. Only counts and a closed format enum.

use super::error::{PluginError, PluginErrorCode};
use super::limits::CURRENT_PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION;
use super::manifest::PluginHook;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Output formats an `on_export_complete` payload may name. One entry per format an
/// application export flow can write; nothing else is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginExportFormatV1 {
    Pdf,
    Docx,
    Pptx,
    Html,
    Rtf,
    Md,
    Txt,
    Png,
    Jpeg,
    Svg,
    Webp,
}

impl PluginExportFormatV1 {
    pub const ALL: [Self; 11] = [
        Self::Pdf,
        Self::Docx,
        Self::Pptx,
        Self::Html,
        Self::Rtf,
        Self::Md,
        Self::Txt,
        Self::Png,
        Self::Jpeg,
        Self::Svg,
        Self::Webp,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Html => "html",
            Self::Rtf => "rtf",
            Self::Md => "md",
            Self::Txt => "txt",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Svg => "svg",
            Self::Webp => "webp",
        }
    }
}

/// `on_document_open`: `{ "schema_version": 1, "page_count": N }`, N ≥ 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentOpenPayloadV1 {
    pub schema_version: u32,
    pub page_count: u32,
}

/// `on_export_complete`: `{ "schema_version": 1, "format": F, "page_count": N }`, N ≥ 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportCompletePayloadV1 {
    pub schema_version: u32,
    pub format: PluginExportFormatV1,
    pub page_count: u32,
}

/// `on_custom_command`: `{ "schema_version": 1 }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomCommandPayloadV1 {
    pub schema_version: u32,
}

/// A payload that passed the schema of the hook it was sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginHookPayloadV1 {
    DocumentOpen(DocumentOpenPayloadV1),
    ExportComplete(ExportCompletePayloadV1),
    CustomCommand(CustomCommandPayloadV1),
}

fn invalid(hook: PluginHook, detail: impl std::fmt::Display) -> PluginError {
    PluginError::new(
        PluginErrorCode::PluginPayloadInvalid,
        format!("'{}' kanca yükü şemaya uymuyor: {}", hook.as_str(), detail),
    )
}

fn check_common(hook: PluginHook, schema_version: u32) -> Result<(), PluginError> {
    if schema_version != CURRENT_PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION {
        return Err(invalid(
            hook,
            format!(
                "desteklenmeyen schema_version {}; yalnız {} kabul edilir",
                schema_version, CURRENT_PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION
            ),
        ));
    }
    Ok(())
}

fn check_page_count(hook: PluginHook, page_count: u32) -> Result<(), PluginError> {
    if page_count == 0 {
        return Err(invalid(hook, "page_count en az 1 olmalı"));
    }
    Ok(())
}

fn from_value<T: DeserializeOwned>(
    hook: PluginHook,
    value: &serde_json::Value,
) -> Result<T, PluginError> {
    if !value.is_object() {
        return Err(invalid(hook, "yük bir JSON nesnesi olmalı"));
    }
    serde_json::from_value(value.clone()).map_err(|e| invalid(hook, e))
}

fn from_str<T: DeserializeOwned>(hook: PluginHook, raw: &str) -> Result<T, PluginError> {
    // Parsing straight into the struct also rejects duplicate keys ("duplicate field").
    serde_json::from_str(raw).map_err(|e| invalid(hook, e))
}

impl PluginHookPayloadV1 {
    /// Parses raw JSON text (as received from the webview) for `hook`.
    pub fn parse_json(hook: PluginHook, raw: &str) -> Result<Self, PluginError> {
        let raw = raw.trim();
        if !raw.starts_with('{') {
            return Err(invalid(hook, "yük bir JSON nesnesi olmalı"));
        }
        let payload = match hook {
            PluginHook::OnDocumentOpen => Self::DocumentOpen(from_str(hook, raw)?),
            PluginHook::OnExportComplete => Self::ExportComplete(from_str(hook, raw)?),
            PluginHook::OnCustomCommand => Self::CustomCommand(from_str(hook, raw)?),
        };
        payload.check()?;
        Ok(payload)
    }

    /// Parses an already decoded JSON value (worker side) for `hook`.
    pub fn parse_value(hook: PluginHook, value: &serde_json::Value) -> Result<Self, PluginError> {
        let payload = match hook {
            PluginHook::OnDocumentOpen => Self::DocumentOpen(from_value(hook, value)?),
            PluginHook::OnExportComplete => Self::ExportComplete(from_value(hook, value)?),
            PluginHook::OnCustomCommand => Self::CustomCommand(from_value(hook, value)?),
        };
        payload.check()?;
        Ok(payload)
    }

    fn check(&self) -> Result<(), PluginError> {
        let hook = self.hook();
        match self {
            Self::DocumentOpen(p) => {
                check_common(hook, p.schema_version)?;
                check_page_count(hook, p.page_count)
            }
            Self::ExportComplete(p) => {
                check_common(hook, p.schema_version)?;
                check_page_count(hook, p.page_count)
            }
            Self::CustomCommand(p) => check_common(hook, p.schema_version),
        }
    }

    pub fn hook(&self) -> PluginHook {
        match self {
            Self::DocumentOpen(_) => PluginHook::OnDocumentOpen,
            Self::ExportComplete(_) => PluginHook::OnExportComplete,
            Self::CustomCommand(_) => PluginHook::OnCustomCommand,
        }
    }

    /// Canonical JSON form (fixed field set, struct field order) sent to the worker.
    pub fn to_value(&self) -> serde_json::Value {
        let value = match self {
            Self::DocumentOpen(p) => serde_json::to_value(p),
            Self::ExportComplete(p) => serde_json::to_value(p),
            Self::CustomCommand(p) => serde_json::to_value(p),
        };
        // Plain structs of integers and a unit enum always serialize.
        value.unwrap_or(serde_json::Value::Null)
    }

    /// The payload as a Rhai object map: the hook function's argument.
    pub fn to_rhai_map(&self) -> rhai::Map {
        let mut map = rhai::Map::new();
        let int = |n: u32| rhai::Dynamic::from_int(rhai::INT::from(n));
        match self {
            Self::DocumentOpen(p) => {
                map.insert("schema_version".into(), int(p.schema_version));
                map.insert("page_count".into(), int(p.page_count));
            }
            Self::ExportComplete(p) => {
                map.insert("schema_version".into(), int(p.schema_version));
                map.insert("format".into(), p.format.as_str().into());
                map.insert("page_count".into(), int(p.page_count));
            }
            Self::CustomCommand(p) => {
                map.insert("schema_version".into(), int(p.schema_version));
            }
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOOKS: [PluginHook; 3] = [
        PluginHook::OnDocumentOpen,
        PluginHook::OnExportComplete,
        PluginHook::OnCustomCommand,
    ];

    fn valid_payload(hook: PluginHook) -> serde_json::Value {
        match hook {
            PluginHook::OnDocumentOpen => json!({ "schema_version": 1, "page_count": 12 }),
            PluginHook::OnExportComplete => {
                json!({ "schema_version": 1, "format": "docx", "page_count": 3 })
            }
            PluginHook::OnCustomCommand => json!({ "schema_version": 1 }),
        }
    }

    fn assert_invalid(result: Result<PluginHookPayloadV1, PluginError>) {
        let err = result.expect_err("payload must be rejected");
        assert_eq!(err.code, PluginErrorCode::PluginPayloadInvalid, "{err}");
    }

    #[test]
    fn valid_payloads_parse_from_text_and_value() {
        for hook in HOOKS {
            let value = valid_payload(hook);
            let from_text = PluginHookPayloadV1::parse_json(hook, &value.to_string()).unwrap();
            let from_value = PluginHookPayloadV1::parse_value(hook, &value).unwrap();
            assert_eq!(from_text, from_value);
            assert_eq!(from_text.hook(), hook);
            assert_eq!(from_text.to_value(), value);
        }
        let parsed = PluginHookPayloadV1::parse_value(
            PluginHook::OnExportComplete,
            &valid_payload(PluginHook::OnExportComplete),
        )
        .unwrap();
        let map = parsed.to_rhai_map();
        assert_eq!(map["format"].clone().into_string().unwrap(), "docx");
        assert_eq!(map["page_count"].as_int().unwrap(), 3);
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn every_export_format_round_trips() {
        for format in PluginExportFormatV1::ALL {
            let value = json!({ "schema_version": 1, "format": format.as_str(), "page_count": 1 });
            let parsed =
                PluginHookPayloadV1::parse_value(PluginHook::OnExportComplete, &value).unwrap();
            assert_eq!(parsed.to_value(), value);
        }
        for bad in ["PDF", "jpg", "xlsx", "", "pdf "] {
            let value = json!({ "schema_version": 1, "format": bad, "page_count": 1 });
            assert_invalid(PluginHookPayloadV1::parse_value(
                PluginHook::OnExportComplete,
                &value,
            ));
        }
    }

    #[test]
    fn unknown_fields_are_rejected_for_every_hook() {
        for hook in HOOKS {
            for (key, leaked) in [
                ("file_name", json!("secret.pdf")),
                ("path", json!("C:/docs/secret.pdf")),
                ("text", json!("belge içeriği")),
                ("password", json!("hunter2")),
                ("session_id", json!("s-1")),
            ] {
                let mut value = valid_payload(hook);
                value.as_object_mut().unwrap().insert(key.into(), leaked);
                assert_invalid(PluginHookPayloadV1::parse_value(hook, &value));
                assert_invalid(PluginHookPayloadV1::parse_json(hook, &value.to_string()));
            }
        }
    }

    #[test]
    fn payload_of_another_hook_is_rejected() {
        for sent_to in HOOKS {
            for built_for in HOOKS {
                if sent_to == built_for {
                    continue;
                }
                let value = valid_payload(built_for);
                assert_invalid(PluginHookPayloadV1::parse_value(sent_to, &value));
                assert_invalid(PluginHookPayloadV1::parse_json(sent_to, &value.to_string()));
            }
        }
    }

    #[test]
    fn wrong_or_missing_schema_version_is_rejected() {
        for hook in HOOKS {
            for version in [
                json!(0),
                json!(2),
                json!(-1),
                json!("1"),
                json!(1.5),
                json!(null),
            ] {
                let mut value = valid_payload(hook);
                value["schema_version"] = version;
                assert_invalid(PluginHookPayloadV1::parse_value(hook, &value));
                assert_invalid(PluginHookPayloadV1::parse_json(hook, &value.to_string()));
            }
            let mut value = valid_payload(hook);
            value.as_object_mut().unwrap().remove("schema_version");
            assert_invalid(PluginHookPayloadV1::parse_value(hook, &value));
        }
    }

    #[test]
    fn bad_page_counts_are_rejected() {
        for hook in [PluginHook::OnDocumentOpen, PluginHook::OnExportComplete] {
            for count in [
                json!(0),
                json!(-3),
                json!(2.5),
                json!("4"),
                json!(4_294_967_296_u64),
                json!(null),
            ] {
                let mut value = valid_payload(hook);
                value["page_count"] = count;
                assert_invalid(PluginHookPayloadV1::parse_value(hook, &value));
            }
            let mut value = valid_payload(hook);
            value.as_object_mut().unwrap().remove("page_count");
            assert_invalid(PluginHookPayloadV1::parse_value(hook, &value));
        }
    }

    #[test]
    fn non_object_and_duplicate_key_payloads_are_rejected() {
        for hook in HOOKS {
            for raw in ["[]", "1", "\"x\"", "null", "", "{]}"] {
                assert_invalid(PluginHookPayloadV1::parse_json(hook, raw));
            }
            for value in [json!([]), json!(1), json!(null)] {
                assert_invalid(PluginHookPayloadV1::parse_value(hook, &value));
            }
        }
        assert_invalid(PluginHookPayloadV1::parse_json(
            PluginHook::OnDocumentOpen,
            r#"{"schema_version":1,"page_count":2,"page_count":3}"#,
        ));
    }
}
