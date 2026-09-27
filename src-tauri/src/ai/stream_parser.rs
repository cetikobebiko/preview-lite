use crate::ai::error::{AiError, AiErrorWire};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Emitter;

pub const MAX_STREAM_FRAME_BYTES: usize = 64 * 1024; // 64 KB
pub const MAX_INCOMPLETE_BUFFER_BYTES: usize = 64 * 1024; // 64 KB
pub const MAX_STREAM_TOTAL_BYTES: usize = 10 * 1024 * 1024; // 10 MB
pub const MAX_EMITTED_EVENTS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalDeliveryState {
    Pending,
    Delivering,
    Delivered,
    FailedDelivery,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStreamEvent {
    pub schema_version: u32,
    pub request_id: String,
    pub operation: String,
    pub session_id: Option<String>,
    pub content_generation: Option<u64>,
    pub endpoint_fingerprint: String,
    pub sequence: u64,
    pub kind: String,
    pub delta: Option<String>,
    pub result: Option<Value>,
    pub error: Option<AiErrorWire>,
    pub terminal: bool,
}

pub struct NdjsonStreamParser {
    buffer: Vec<u8>,
    total_bytes_read: usize,
    sequence: u64,
    request_id: String,
    operation: String,
    session_id: Option<String>,
    content_generation: Option<u64>,
    endpoint_fingerprint: String,
    terminal_delivery: TerminalDeliveryState,
}

impl NdjsonStreamParser {
    pub fn new(
        request_id: String,
        operation: String,
        session_id: Option<String>,
        content_generation: Option<u64>,
        endpoint_fingerprint: String,
    ) -> Self {
        Self {
            buffer: Vec::new(),
            total_bytes_read: 0,
            sequence: 0,
            request_id,
            operation,
            session_id,
            content_generation,
            endpoint_fingerprint,
            terminal_delivery: TerminalDeliveryState::Pending,
        }
    }

    pub fn terminal_state(&self) -> TerminalDeliveryState {
        self.terminal_delivery
    }

    pub fn is_terminal_emitted(&self) -> bool {
        self.terminal_delivery != TerminalDeliveryState::Pending
    }

    pub fn emit_started<R: tauri::Runtime>(
        &mut self,
        app_handle: &tauri::AppHandle<R>,
    ) -> Result<(), AiError> {
        if self.terminal_delivery != TerminalDeliveryState::Pending {
            return Err(AiError::EndpointStreamProtocolError(
                "Terminal event already emitted or delivering".to_string(),
            ));
        }
        self.sequence += 1;
        let evt = AiStreamEvent {
            schema_version: 1,
            request_id: self.request_id.clone(),
            operation: self.operation.clone(),
            session_id: self.session_id.clone(),
            content_generation: self.content_generation,
            endpoint_fingerprint: self.endpoint_fingerprint.clone(),
            sequence: self.sequence,
            kind: "started".to_string(),
            delta: None,
            result: None,
            error: None,
            terminal: false,
        };
        app_handle.emit("ai_stream_event", &evt).map_err(|e| {
            AiError::EndpointStreamProtocolError(format!("Failed to emit AI stream event: {}", e))
        })?;
        Ok(())
    }

    pub fn push_chunk<R: tauri::Runtime>(
        &mut self,
        app_handle: &tauri::AppHandle<R>,
        chunk: &[u8],
        extractor: impl Fn(&Value) -> (Option<String>, bool),
    ) -> Result<bool, AiError> {
        if self.terminal_delivery != TerminalDeliveryState::Pending {
            return Err(AiError::EndpointStreamProtocolError(
                "Cannot process chunks after terminal event".to_string(),
            ));
        }

        self.total_bytes_read = self
            .total_bytes_read
            .checked_add(chunk.len())
            .ok_or_else(|| AiError::StreamTotalLimitExceeded("Stream byte overflow".to_string()))?;

        if self.total_bytes_read > MAX_STREAM_TOTAL_BYTES {
            return Err(AiError::StreamTotalLimitExceeded(format!(
                "Stream total size {} exceeds limit of {} bytes",
                self.total_bytes_read, MAX_STREAM_TOTAL_BYTES
            )));
        }

        self.buffer.extend_from_slice(chunk);

        while let Some(line_end_idx) = self.buffer.iter().position(|&b| b == b'\n') {
            let mut line_bytes = self.buffer.drain(..=line_end_idx).collect::<Vec<u8>>();
            if line_bytes.ends_with(b"\n") {
                line_bytes.pop();
            }
            if line_bytes.ends_with(b"\r") {
                line_bytes.pop();
            }

            if line_bytes.len() > MAX_STREAM_FRAME_BYTES {
                return Err(AiError::EndpointStreamFrameTooLarge(format!(
                    "Frame size {} bytes exceeds 64 KB limit",
                    line_bytes.len()
                )));
            }

            if line_bytes.iter().all(|b| b.is_ascii_whitespace()) {
                continue;
            }

            let json_val: Value = serde_json::from_slice(&line_bytes).map_err(|e| {
                AiError::EndpointStreamProtocolError(format!("NDJSON parse error: {}", e))
            })?;

            let (text_delta, is_done) = extractor(&json_val);

            if let Some(delta_str) = text_delta {
                if !delta_str.is_empty() {
                    self.sequence += 1;
                    if self.sequence > MAX_EMITTED_EVENTS {
                        return Err(AiError::StreamTotalLimitExceeded(
                            "Exceeded maximum emitted event budget".to_string(),
                        ));
                    }
                    let evt = AiStreamEvent {
                        schema_version: 1,
                        request_id: self.request_id.clone(),
                        operation: self.operation.clone(),
                        session_id: self.session_id.clone(),
                        content_generation: self.content_generation,
                        endpoint_fingerprint: self.endpoint_fingerprint.clone(),
                        sequence: self.sequence,
                        kind: "delta".to_string(),
                        delta: Some(delta_str),
                        result: None,
                        error: None,
                        terminal: false,
                    };
                    app_handle.emit("ai_stream_event", &evt).map_err(|e| {
                        AiError::EndpointStreamProtocolError(format!(
                            "Failed to emit AI stream event: {}",
                            e
                        ))
                    })?;
                }
            }

            if is_done {
                self.emit_completed(app_handle, None)?;
                return Ok(true);
            }
        }

        if self.buffer.len() > MAX_INCOMPLETE_BUFFER_BYTES {
            return Err(AiError::EndpointStreamFrameTooLarge(format!(
                "Incomplete frame buffer size {} bytes exceeds limit of {} bytes",
                self.buffer.len(),
                MAX_INCOMPLETE_BUFFER_BYTES
            )));
        }

        Ok(false)
    }

    pub fn check_eof(&self) -> Result<(), AiError> {
        if !self.buffer.is_empty() {
            return Err(AiError::StreamUnexpectedEof(
                "Incomplete frame buffer remaining at stream EOF".to_string(),
            ));
        }
        if self.terminal_delivery == TerminalDeliveryState::Pending {
            return Err(AiError::StreamUnexpectedEof(
                "Stream ended without terminal done completion frame".to_string(),
            ));
        }
        Ok(())
    }

    pub fn emit_completed<R: tauri::Runtime>(
        &mut self,
        app_handle: &tauri::AppHandle<R>,
        result: Option<Value>,
    ) -> Result<(), AiError> {
        if self.terminal_delivery != TerminalDeliveryState::Pending {
            return Ok(());
        }
        self.terminal_delivery = TerminalDeliveryState::Delivering;
        self.sequence += 1;
        let evt = AiStreamEvent {
            schema_version: 1,
            request_id: self.request_id.clone(),
            operation: self.operation.clone(),
            session_id: self.session_id.clone(),
            content_generation: self.content_generation,
            endpoint_fingerprint: self.endpoint_fingerprint.clone(),
            sequence: self.sequence,
            kind: "completed".to_string(),
            delta: None,
            result,
            error: None,
            terminal: true,
        };
        match app_handle.emit("ai_stream_event", &evt) {
            Ok(_) => {
                self.terminal_delivery = TerminalDeliveryState::Delivered;
                Ok(())
            }
            Err(e) => {
                self.terminal_delivery = TerminalDeliveryState::FailedDelivery;
                Err(AiError::EndpointStreamProtocolError(format!(
                    "Failed to emit AI stream event: {}",
                    e
                )))
            }
        }
    }

    pub fn emit_failed<R: tauri::Runtime>(
        &mut self,
        app_handle: &tauri::AppHandle<R>,
        err: &AiError,
    ) -> Result<(), AiError> {
        if self.terminal_delivery != TerminalDeliveryState::Pending {
            return Ok(());
        }
        self.terminal_delivery = TerminalDeliveryState::Delivering;
        self.sequence += 1;
        let evt = AiStreamEvent {
            schema_version: 1,
            request_id: self.request_id.clone(),
            operation: self.operation.clone(),
            session_id: self.session_id.clone(),
            content_generation: self.content_generation,
            endpoint_fingerprint: self.endpoint_fingerprint.clone(),
            sequence: self.sequence,
            kind: "failed".to_string(),
            delta: None,
            result: None,
            error: Some(err.to_wire()),
            terminal: true,
        };
        match app_handle.emit("ai_stream_event", &evt) {
            Ok(_) => {
                self.terminal_delivery = TerminalDeliveryState::Delivered;
                Ok(())
            }
            Err(e) => {
                self.terminal_delivery = TerminalDeliveryState::FailedDelivery;
                Err(AiError::EndpointStreamProtocolError(format!(
                    "Failed to emit AI stream event: {}",
                    e
                )))
            }
        }
    }

    pub fn emit_cancelled<R: tauri::Runtime>(
        &mut self,
        app_handle: &tauri::AppHandle<R>,
    ) -> Result<(), AiError> {
        if self.terminal_delivery != TerminalDeliveryState::Pending {
            return Ok(());
        }
        self.terminal_delivery = TerminalDeliveryState::Delivering;
        self.sequence += 1;
        let cancel_err = AiError::RequestCancelled("Cancelled".to_string());
        let evt = AiStreamEvent {
            schema_version: 1,
            request_id: self.request_id.clone(),
            operation: self.operation.clone(),
            session_id: self.session_id.clone(),
            content_generation: self.content_generation,
            endpoint_fingerprint: self.endpoint_fingerprint.clone(),
            sequence: self.sequence,
            kind: "cancelled".to_string(),
            delta: None,
            result: None,
            error: Some(cancel_err.to_wire()),
            terminal: true,
        };
        match app_handle.emit("ai_stream_event", &evt) {
            Ok(_) => {
                self.terminal_delivery = TerminalDeliveryState::Delivered;
                Ok(())
            }
            Err(e) => {
                self.terminal_delivery = TerminalDeliveryState::FailedDelivery;
                Err(AiError::EndpointStreamProtocolError(format!(
                    "Failed to emit AI stream event: {}",
                    e
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_event_structure() {
        let evt = AiStreamEvent {
            schema_version: 1,
            request_id: "req_123".to_string(),
            operation: "chat".to_string(),
            session_id: Some("s1".to_string()),
            content_generation: Some(1),
            endpoint_fingerprint: "fp".to_string(),
            sequence: 1,
            kind: "started".to_string(),
            delta: None,
            result: None,
            error: None,
            terminal: false,
        };
        assert_eq!(evt.schema_version, 1);
        assert!(!evt.terminal);
    }

    #[test]
    fn test_security_incomplete_residual_buffer_overflow() {
        let mut parser = NdjsonStreamParser::new(
            "req_1".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );

        let app = tauri::test::mock_app();
        let handle = app.handle();

        let oversized_chunk = vec![b'a'; MAX_INCOMPLETE_BUFFER_BYTES + 10];
        let res = parser.push_chunk(handle, &oversized_chunk, |_| (None, false));
        assert!(matches!(res, Err(AiError::EndpointStreamFrameTooLarge(_))));
    }

    #[test]
    fn test_stream_parser_a1_split_frame() {
        let mut parser = NdjsonStreamParser::new(
            "req_a1".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        // Part 1
        let res1 = parser.push_chunk(handle, b"{\"message\":{\"content\":\"Hello ", |val| {
            let delta = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            (delta, done)
        });
        assert!(!res1.unwrap());

        // Part 2
        let res2 = parser.push_chunk(handle, b"World!\"},\"done\":true}\n", |val| {
            let delta = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            (delta, done)
        });
        assert!(res2.unwrap());
        assert_eq!(parser.terminal_state(), TerminalDeliveryState::Delivered);
    }

    #[test]
    fn test_stream_parser_a2_multiple_frames_in_one_chunk() {
        let mut parser = NdjsonStreamParser::new(
            "req_a2".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        let combined = b"{\"message\":{\"content\":\"A\"},\"done\":false}\n{\"message\":{\"content\":\"B\"},\"done\":false}\n{\"message\":{\"content\":\"C\"},\"done\":true}\n";
        let res = parser.push_chunk(handle, combined, |val| {
            let delta = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            (delta, done)
        });
        assert!(res.unwrap());
        assert_eq!(parser.terminal_state(), TerminalDeliveryState::Delivered);
    }

    #[test]
    fn test_stream_parser_a3_combined_chunk_exceeds_frame_limit() {
        let mut parser = NdjsonStreamParser::new(
            "req_a3".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        // Create two frames, each ~35 KB (total 70 KB > 64 KB chunk)
        let pad_a = "a".repeat(35 * 1024);
        let pad_b = "b".repeat(35 * 1024);
        let frame1 = format!(
            "{{\"message\":{{\"content\":\"{}\"}},\"done\":false}}\n",
            pad_a
        );
        let frame2 = format!(
            "{{\"message\":{{\"content\":\"{}\"}},\"done\":true}}\n",
            pad_b
        );
        let mut combined = Vec::new();
        combined.extend_from_slice(frame1.as_bytes());
        combined.extend_from_slice(frame2.as_bytes());
        assert!(combined.len() > MAX_STREAM_FRAME_BYTES);

        let res = parser.push_chunk(handle, &combined, |val| {
            let delta = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            (delta, done)
        });
        assert!(res.unwrap());
        assert_eq!(parser.terminal_state(), TerminalDeliveryState::Delivered);
    }

    #[test]
    fn test_stream_parser_a4_complete_plus_partial_frame() {
        let mut parser = NdjsonStreamParser::new(
            "req_a4".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        let chunk1 = b"{\"message\":{\"content\":\"First\"},\"done\":false}\n{\"message\":{\"content\":\"Sec";
        let res1 = parser.push_chunk(handle, chunk1, |val| {
            let delta = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            (delta, done)
        });
        assert!(!res1.unwrap());

        let chunk2 = b"ond\"},\"done\":true}\n";
        let res2 = parser.push_chunk(handle, chunk2, |val| {
            let delta = val
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
            (delta, done)
        });
        assert!(res2.unwrap());
    }

    #[test]
    fn test_stream_parser_a5_oversized_single_frame() {
        let mut parser = NdjsonStreamParser::new(
            "req_a5".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        let pad = "x".repeat(MAX_STREAM_FRAME_BYTES + 100);
        let oversized = format!(
            "{{\"message\":{{\"content\":\"{}\"}},\"done\":false}}\n",
            pad
        );
        let res = parser.push_chunk(handle, oversized.as_bytes(), |_| (None, false));
        assert!(matches!(res, Err(AiError::EndpointStreamFrameTooLarge(_))));
    }

    #[test]
    fn test_stream_parser_a6_eof_before_complete_frame() {
        let mut parser = NdjsonStreamParser::new(
            "req_a6".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        let _ = parser.push_chunk(handle, b"{\"message\":{\"content\":", |_| (None, false));
        assert!(matches!(
            parser.check_eof(),
            Err(AiError::StreamUnexpectedEof(_))
        ));
    }

    #[test]
    fn test_stream_parser_a7_eof_before_done() {
        let mut parser = NdjsonStreamParser::new(
            "req_a7".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        let _ = parser.push_chunk(
            handle,
            b"{\"message\":{\"content\":\"hi\"},\"done\":false}\n",
            |val| {
                let delta = val
                    .pointer("/message/content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let done = val.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
                (delta, done)
            },
        );
        // Stream ended without done: true frame
        assert!(matches!(
            parser.check_eof(),
            Err(AiError::StreamUnexpectedEof(_))
        ));
    }

    #[test]
    fn test_terminal_event_locking() {
        let mut parser = NdjsonStreamParser::new(
            "req_term".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );

        let app = tauri::test::mock_app();
        let handle = app.handle();
        assert!(parser.emit_completed(handle, None).is_ok());
        assert_eq!(parser.terminal_state(), TerminalDeliveryState::Delivered);

        // Second terminal delivery attempt is a no-op returning Ok(())
        assert!(parser.emit_completed(handle, None).is_ok());

        // Pushing chunks after terminal event should fail
        assert!(matches!(
            parser.push_chunk(handle, b"{\"done\": true}\n", |_| (None, true)),
            Err(AiError::EndpointStreamProtocolError(_))
        ));
    }
}
