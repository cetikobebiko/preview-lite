//! Manifest V1 schema and strict parsing/validation.

use super::error::{PluginError, PluginErrorCode};
use super::limits::{CURRENT_PLUGIN_MANIFEST_VERSION, MAX_MANIFEST_SIZE_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PluginPermission {
    #[serde(rename = "plugin:log")]
    PluginLog,
    #[serde(rename = "ui:notification")]
    UiNotification,
}

impl PluginPermission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PluginLog => "plugin:log",
            Self::UiNotification => "ui:notification",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginHook {
    OnDocumentOpen,
    OnCustomCommand,
    OnExportComplete,
}

impl PluginHook {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OnDocumentOpen => "on_document_open",
            Self::OnCustomCommand => "on_custom_command",
            Self::OnExportComplete => "on_export_complete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestV1 {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    pub main_script: String,
    #[serde(default)]
    pub permissions: Vec<PluginPermission>,
    #[serde(default)]
    pub hooks: Vec<PluginHook>,
}

impl PluginManifestV1 {
    /// Validates and parses raw manifest JSON bytes according to Manifest V1 specification.
    pub fn parse_and_validate(raw_bytes: &[u8]) -> Result<Self, PluginError> {
        // 1. Size limit check (max 64 KB)
        if raw_bytes.len() as u64 > MAX_MANIFEST_SIZE_BYTES {
            return Err(PluginError::new(
                PluginErrorCode::PluginManifestTooLarge,
                format!(
                    "Manifest boyutu {} byte; izin verilen üst sınır {} byte.",
                    raw_bytes.len(),
                    MAX_MANIFEST_SIZE_BYTES
                ),
            ));
        }

        // 2. UTF-8 & BOM handling
        let clean_bytes = if raw_bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            &raw_bytes[3..]
        } else {
            raw_bytes
        };

        let json_str = std::str::from_utf8(clean_bytes).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                format!("Manifest geçerli UTF-8 formatında değil: {}", e),
            )
        })?;

        // 3. Duplicate key validation
        check_no_duplicate_keys(clean_bytes)?;

        // 4. Strict Deserialization (deny_unknown_fields)
        let manifest: PluginManifestV1 = serde_json::from_str(json_str).map_err(|e| {
            let err_msg = e.to_string();
            if err_msg.contains("unknown field") {
                PluginError::new(
                    PluginErrorCode::PluginManifestUnknownField,
                    format!("Manifest bilinmeyen alan içeriyor: {}", err_msg),
                )
            } else if err_msg.contains("unknown variant") {
                if err_msg.contains("permission")
                    || err_msg.contains("plugin:log")
                    || err_msg.contains("ui:notification")
                    || err_msg.contains("doc:read")
                {
                    PluginError::new(
                        PluginErrorCode::PluginPermissionUnknown,
                        format!("Bilinmeyen veya izin verilmeyen eklenti izni: {}", err_msg),
                    )
                } else if err_msg.contains("hook") || err_msg.contains("on_") {
                    PluginError::new(
                        PluginErrorCode::PluginHookUnknown,
                        format!(
                            "Bilinmeyen veya izin verilmeyen eklenti kancası: {}",
                            err_msg
                        ),
                    )
                } else {
                    PluginError::new(
                        PluginErrorCode::PluginManifestInvalid,
                        format!("Manifest format hatası: {}", err_msg),
                    )
                }
            } else {
                PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    format!("Manifest JSON formatı geçersiz: {}", err_msg),
                )
            }
        })?;

        // 5. Semantic Validation
        manifest.validate_fields()?;

        Ok(manifest)
    }

    fn validate_fields(&self) -> Result<(), PluginError> {
        // manifest_version check (strictly 1)
        if self.manifest_version != CURRENT_PLUGIN_MANIFEST_VERSION {
            return Err(PluginError::new(
                PluginErrorCode::PluginManifestVersionUnsupported,
                format!(
                    "Desteklenmeyen manifest sürümü: {}. Yalnızca sürüm {} desteklenir.",
                    self.manifest_version, CURRENT_PLUGIN_MANIFEST_VERSION
                ),
            ));
        }

        // id validation (1..64 chars, [a-z0-9_-])
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(PluginError::new(
                PluginErrorCode::PluginIdInvalid,
                format!(
                    "Eklenti ID'si geçersiz ('{}'). 1-64 karakter uzunluğunda olmalı ve yalnızca küçük harf, rakam, tire veya alt çizgi içermelidir.",
                    self.id
                ),
            ));
        }

        // name validation (1..128 chars, non-empty after trim)
        let trimmed_name = self.name.trim();
        if trimmed_name.is_empty() || self.name.len() > 128 {
            return Err(PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                "Eklenti adı 1-128 karakter arasında olmalı ve boş bırakılamaz.",
            ));
        }

        // version validation (1..32 chars, semver-like: digits, dots, hyphens, alphanumeric)
        if self.version.is_empty()
            || self.version.len() > 32
            || !self
                .version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+' || c == '_')
        {
            return Err(PluginError::new(
                PluginErrorCode::PluginVersionInvalid,
                format!(
                    "Eklenti sürüm dizesi geçersiz ('{}'). 1-32 karakter uzunluğunda olmalıdır.",
                    self.version
                ),
            ));
        }

        // description validation (max 512 chars)
        if let Some(desc) = &self.description {
            if desc.len() > 512 {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "Eklenti açıklaması en fazla 512 karakter olabilir.",
                ));
            }
        }

        // Package V1 main_script invariant: MUST be exactly "main.rhai"
        if self.main_script != "main.rhai" {
            return Err(PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                format!(
                    "Package V1 ana betik adı kesinlikle 'main.rhai' olmalıdır, bulunan: '{}'",
                    self.main_script
                ),
            ));
        }

        // permissions uniqueness validation
        let mut seen_perms = HashSet::new();
        for perm in &self.permissions {
            if !seen_perms.insert(*perm) {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    format!("Tekrarlanan eklenti izni: '{}'", perm.as_str()),
                ));
            }
        }

        // hooks uniqueness validation
        let mut seen_hooks = HashSet::new();
        for hook in &self.hooks {
            if !seen_hooks.insert(*hook) {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    format!("Tekrarlanan eklenti kancası: '{}'", hook.as_str()),
                ));
            }
        }

        Ok(())
    }
}

/// Recursively checks that a JSON object contains no duplicate keys.
fn check_no_duplicate_keys(bytes: &[u8]) -> Result<(), PluginError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value: serde_json::Value =
        serde::de::Deserialize::deserialize(&mut deserializer).map_err(|e| {
            PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                format!("JSON ayrıştırma hatası: {}", e),
            )
        })?;

    check_duplicates_in_json_str(bytes)?;

    let _ = value;
    Ok(())
}

fn check_duplicates_in_json_str(bytes: &[u8]) -> Result<(), PluginError> {
    let mut parser = json_duplicate_detector::DuplicateDetector::new(bytes);
    parser.parse()
}

pub mod json_duplicate_detector {
    use super::*;

    pub struct DuplicateDetector<'a> {
        chars: std::str::Chars<'a>,
        peeked: Option<char>,
    }

    impl<'a> DuplicateDetector<'a> {
        pub fn new(bytes: &'a [u8]) -> Self {
            let s = std::str::from_utf8(bytes).unwrap_or("");
            Self {
                chars: s.chars(),
                peeked: None,
            }
        }

        fn next_char(&mut self) -> Option<char> {
            if let Some(c) = self.peeked.take() {
                Some(c)
            } else {
                self.chars.next()
            }
        }

        fn peek_char(&mut self) -> Option<char> {
            if self.peeked.is_none() {
                self.peeked = self.chars.next();
            }
            self.peeked
        }

        fn skip_whitespace(&mut self) {
            while let Some(c) = self.peek_char() {
                if c.is_whitespace() {
                    self.next_char();
                } else {
                    break;
                }
            }
        }

        pub fn parse(&mut self) -> Result<(), PluginError> {
            self.skip_whitespace();
            self.parse_value()
        }

        fn parse_value(&mut self) -> Result<(), PluginError> {
            self.skip_whitespace();
            match self.peek_char() {
                Some('{') => self.parse_object(),
                Some('[') => self.parse_array(),
                Some('"') => {
                    self.parse_string()?;
                    Ok(())
                }
                Some('t') | Some('f') | Some('n') => self.parse_literal(),
                Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
                Some(c) => Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    format!("Beklenmeyen JSON karakteri: '{}'", c),
                )),
                None => Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "JSON beklenmedik şekilde sonlandı.",
                )),
            }
        }

        fn parse_object(&mut self) -> Result<(), PluginError> {
            self.next_char(); // consume '{'
            let mut keys = HashSet::new();
            self.skip_whitespace();
            if let Some('}') = self.peek_char() {
                self.next_char();
                return Ok(());
            }

            loop {
                self.skip_whitespace();
                let key = self.parse_string()?;
                if !keys.insert(key.clone()) {
                    return Err(PluginError::new(
                        PluginErrorCode::PluginManifestInvalid,
                        format!("Manifestte yinelenen JSON anahtarı bulundu: '{}'", key),
                    ));
                }
                self.skip_whitespace();
                match self.next_char() {
                    Some(':') => {}
                    _ => {
                        return Err(PluginError::new(
                            PluginErrorCode::PluginManifestInvalid,
                            "JSON anahtarından sonra ':' bekleniyordu.",
                        ))
                    }
                }
                self.parse_value()?;
                self.skip_whitespace();
                match self.next_char() {
                    Some(',') => {
                        self.skip_whitespace();
                        if let Some('}') = self.peek_char() {
                            return Err(PluginError::new(
                                PluginErrorCode::PluginManifestInvalid,
                                "JSON nesnesinde sonda fazladan virgül bulundu.",
                            ));
                        }
                    }
                    Some('}') => break,
                    _ => {
                        return Err(PluginError::new(
                            PluginErrorCode::PluginManifestInvalid,
                            "JSON nesnesinde ',' veya '}' bekleniyordu.",
                        ))
                    }
                }
            }
            Ok(())
        }

        fn parse_array(&mut self) -> Result<(), PluginError> {
            self.next_char(); // consume '['
            self.skip_whitespace();
            if let Some(']') = self.peek_char() {
                self.next_char();
                return Ok(());
            }

            loop {
                self.parse_value()?;
                self.skip_whitespace();
                match self.next_char() {
                    Some(',') => {
                        self.skip_whitespace();
                        if let Some(']') = self.peek_char() {
                            return Err(PluginError::new(
                                PluginErrorCode::PluginManifestInvalid,
                                "JSON dizisinde sonda fazladan virgül bulundu.",
                            ));
                        }
                    }
                    Some(']') => break,
                    _ => {
                        return Err(PluginError::new(
                            PluginErrorCode::PluginManifestInvalid,
                            "JSON dizisinde ',' veya ']' bekleniyordu.",
                        ))
                    }
                }
            }
            Ok(())
        }

        fn parse_string(&mut self) -> Result<String, PluginError> {
            self.skip_whitespace();
            if self.next_char() != Some('"') {
                return Err(PluginError::new(
                    PluginErrorCode::PluginManifestInvalid,
                    "JSON dizesi '\"' ile başlamalıdır.",
                ));
            }
            let mut s = String::new();
            let mut escape = false;
            while let Some(c) = self.next_char() {
                if escape {
                    s.push(c);
                    escape = false;
                } else if c == '\\' {
                    escape = true;
                } else if c == '"' {
                    return Ok(s);
                } else {
                    s.push(c);
                }
            }
            Err(PluginError::new(
                PluginErrorCode::PluginManifestInvalid,
                "Kapatılmamış JSON dizesi.",
            ))
        }

        fn parse_literal(&mut self) -> Result<(), PluginError> {
            while let Some(c) = self.peek_char() {
                if c.is_alphabetic() {
                    self.next_char();
                } else {
                    break;
                }
            }
            Ok(())
        }

        fn parse_number(&mut self) -> Result<(), PluginError> {
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E' {
                    self.next_char();
                } else {
                    break;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_unsupported_schema_version_rejected() {
        let json = r#"{
            "manifest_version": 2,
            "id": "v2-plugin",
            "name": "Version 2 Plugin",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestVersionUnsupported);
    }

    #[test]
    fn test_manifest_missing_required_fields_rejected() {
        // Missing name & version
        let json = r#"{
            "manifest_version": 1,
            "id": "test-plugin",
            "main_script": "main.rhai"
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_manifest_invalid_utf8_rejected() {
        let invalid_utf8_bytes = vec![0xFF, 0xFE, 0xFD, 0x00, 0x12];
        let err = PluginManifestV1::parse_and_validate(&invalid_utf8_bytes).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginManifestInvalid);
    }

    #[test]
    fn test_manifest_id_boundary_cases_rejected() {
        // Empty ID
        let json_empty = r#"{
            "manifest_version": 1,
            "id": "",
            "name": "Empty ID",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }"#;
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_empty.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginIdInvalid
        );

        // Dot ID
        let json_dot = r#"{
            "manifest_version": 1,
            "id": ".",
            "name": "Dot ID",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }"#;
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_dot.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginIdInvalid
        );

        // Double Dot ID
        let json_ddot = r#"{
            "manifest_version": 1,
            "id": "..",
            "name": "Double Dot ID",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }"#;
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_ddot.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginIdInvalid
        );

        // Oversized ID (>64 chars)
        let long_id = "a".repeat(65);
        let json_long = format!(
            r#"{{
            "manifest_version": 1,
            "id": "{long_id}",
            "name": "Long ID",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }}"#
        );
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_long.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginIdInvalid
        );
    }

    #[test]
    fn test_manifest_version_boundary_cases_rejected() {
        // Empty version
        let json_empty = r#"{
            "manifest_version": 1,
            "id": "valid-id",
            "name": "Test",
            "version": "",
            "main_script": "main.rhai"
        }"#;
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_empty.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginVersionInvalid
        );

        // Oversized version (>32 chars)
        let long_v = "1.".repeat(20);
        let json_long = format!(
            r#"{{
            "manifest_version": 1,
            "id": "valid-id",
            "name": "Test",
            "version": "{long_v}",
            "main_script": "main.rhai"
        }}"#
        );
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_long.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginVersionInvalid
        );
    }

    #[test]
    fn test_manifest_name_and_desc_oversize_rejected() {
        // Name > 128 chars
        let long_name = "n".repeat(129);
        let json_name = format!(
            r#"{{
            "manifest_version": 1,
            "id": "valid-id",
            "name": "{long_name}",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }}"#
        );
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_name.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginManifestInvalid
        );

        // Description > 512 chars
        let long_desc = "d".repeat(513);
        let json_desc = format!(
            r#"{{
            "manifest_version": 1,
            "id": "valid-id",
            "name": "Valid Name",
            "description": "{long_desc}",
            "version": "1.0.0",
            "main_script": "main.rhai"
        }}"#
        );
        assert_eq!(
            PluginManifestV1::parse_and_validate(json_desc.as_bytes())
                .unwrap_err()
                .code,
            PluginErrorCode::PluginManifestInvalid
        );
    }

    #[test]
    fn test_manifest_unknown_hook_rejected() {
        let json = r#"{
            "manifest_version": 1,
            "id": "test-plugin",
            "name": "Test",
            "version": "1.0.0",
            "main_script": "main.rhai",
            "hooks": ["on_arbitrary_unsupported_hook"]
        }"#;
        let err = PluginManifestV1::parse_and_validate(json.as_bytes()).unwrap_err();
        assert_eq!(err.code, PluginErrorCode::PluginHookUnknown);
    }
}
