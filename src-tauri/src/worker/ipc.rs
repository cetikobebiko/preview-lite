use bincode::Options;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, Read, Write};

pub const WORKER_PROTOCOL_VERSION: u16 = 2;
pub const MAX_WORKER_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_WORKER_RESPONSE_BYTES: usize =
    crate::pdf_engine::render_service::MAX_RENDER_BYTES + 64 * 1024;
pub const MAX_WORKER_PATH_CHARS: usize = 4096;
pub const MAX_WORKER_PASSWORD_CHARS: usize = 4096;
pub const MAX_WORKER_QUERY_CHARS: usize = 8192;
pub const MAX_WORKER_SEARCH_RESULTS: usize = 10_000;
pub const MAX_WORKER_TEXT_ITEMS: usize = 250_000;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerCredential(String);

impl WorkerCredential {
    pub fn new(value: String) -> Result<Self, WorkerFailure> {
        if value.chars().count() > MAX_WORKER_PASSWORD_CHARS {
            return Err(WorkerFailure::new(
                WorkerFailureKind::InvalidInput,
                "PDF credential length exceeds the worker limit",
            ));
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WorkerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorkerCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerCapability {
    OpenDocument,
    CloseDocument,
    CloseAllDocuments,
    GetPageCount,
    RenderPage,
    RenderThumbnail,
    GetPageTextLayout,
    SearchText,
    GetOutline,
    GetMetadata,
    GetPageAnnotations,
    GetStructureTree,
    ValidatePdfUa,
    Ping,
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkerRequest {
    pub protocol_version: u16,
    pub request_id: u64,
    pub command: WorkerCommand,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum WorkerCommand {
    Hello,
    OpenDocument {
        path: String,
        credential: Option<WorkerCredential>,
    },
    CloseDocument {
        document_id: u64,
    },
    CloseAllDocuments,
    GetPageCount {
        document_id: u64,
    },
    RenderPage {
        document_id: u64,
        page_index: u32,
        target: WorkerRenderTarget,
        rotation: i32,
    },
    RenderThumbnail {
        document_id: u64,
        page_index: u32,
        target_width: u32,
    },
    GetPageTextLayout {
        document_id: u64,
        page_index: u32,
    },
    SearchText {
        document_id: u64,
        query: String,
        max_results: u32,
    },
    GetOutline {
        document_id: u64,
    },
    GetMetadata {
        document_id: u64,
    },
    GetPageAnnotations {
        document_id: u64,
        page_index: u32,
    },
    GetStructureTree {
        document_id: u64,
    },
    ValidatePdfUa {
        document_id: u64,
    },
    Ping,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum WorkerRenderTarget {
    Scale(f64),
    TargetWidth(u32),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerResponse {
    pub protocol_version: u16,
    pub request_id: u64,
    pub result: Result<WorkerResult, WorkerFailure>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum WorkerResult {
    Hello {
        capabilities: Vec<WorkerCapability>,
        ready: bool,
        diagnostics: Vec<WorkerDiagnostic>,
    },
    DocumentOpened {
        document_id: u64,
        page_count: u32,
        protected: bool,
    },
    DocumentClosed,
    AllDocumentsClosed,
    PageCount(u32),
    PageRendered {
        width: u32,
        height: u32,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    TextLayout(Vec<crate::TextLayoutData>),
    Search(crate::SearchResult),
    Outline(Vec<crate::OutlineNode>),
    Metadata(crate::PdfMetadata),
    PageAnnotations(Vec<crate::annotations::AnnotationData>),
    StructureTree(crate::a11y::TagNode),
    PdfUaValidation(crate::a11y::AccessibilityReport),
    Pong,
    ShutdownComplete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerFailureKind {
    PasswordRequired,
    WrongPassword,
    UnsupportedSecurity,
    FileNotFound,
    InvalidPageIndex,
    InvalidRenderParameters,
    InvalidInput,
    MalformedPdf,
    PdfiumUnavailable,
    PdfiumLoadError,
    PdfiumRenderError,
    StructureTreeUnavailable,
    MessageTooLarge,
    ProtocolError,
    Internal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerFailure {
    pub kind: WorkerFailureKind,
    pub message: String,
}

impl WorkerFailure {
    pub fn new(kind: WorkerFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    TruncatedHeader,
    TruncatedPayload,
    MessageTooLarge { announced: usize, maximum: usize },
    LengthOverflow,
    Serialize(String),
    Deserialize(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "worker IPC I/O error: {error}"),
            Self::TruncatedHeader => formatter.write_str("worker IPC length header was truncated"),
            Self::TruncatedPayload => formatter.write_str("worker IPC payload was truncated"),
            Self::MessageTooLarge { announced, maximum } => write!(
                formatter,
                "worker IPC message is too large ({announced} bytes; maximum {maximum})"
            ),
            Self::LengthOverflow => formatter.write_str("worker IPC message length overflow"),
            Self::Serialize(message) => {
                write!(formatter, "worker IPC serialization failed: {message}")
            }
            Self::Deserialize(message) => {
                write!(formatter, "worker IPC deserialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

fn options(limit: usize) -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .with_limit(limit as u64)
}

fn serialize_bounded<T: Serialize>(value: &T, maximum: usize) -> Result<Vec<u8>, FrameError> {
    let payload = options(maximum)
        .serialize(value)
        .map_err(|error| FrameError::Serialize(error.to_string()))?;
    if payload.len() > maximum {
        return Err(FrameError::MessageTooLarge {
            announced: payload.len(),
            maximum,
        });
    }
    Ok(payload)
}

fn deserialize_bounded<'a, T: Deserialize<'a>>(
    payload: &'a [u8],
    maximum: usize,
) -> Result<T, FrameError> {
    if payload.len() > maximum {
        return Err(FrameError::MessageTooLarge {
            announced: payload.len(),
            maximum,
        });
    }
    options(maximum)
        .deserialize(payload)
        .map_err(|error| FrameError::Deserialize(error.to_string()))
}

fn encode_frame<T: Serialize>(value: &T, maximum: usize) -> Result<Vec<u8>, FrameError> {
    let payload = serialize_bounded(value, maximum)?;
    let length = u32::try_from(payload.len()).map_err(|_| FrameError::LengthOverflow)?;
    let mut frame = Vec::with_capacity(4usize.saturating_add(payload.len()));
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn encode_request(request: &WorkerRequest) -> Result<Vec<u8>, FrameError> {
    encode_frame(request, MAX_WORKER_REQUEST_BYTES)
}

pub fn encode_response(response: &WorkerResponse) -> Result<Vec<u8>, FrameError> {
    encode_frame(response, MAX_WORKER_RESPONSE_BYTES)
}

pub fn decode_request(payload: &[u8]) -> Result<WorkerRequest, FrameError> {
    deserialize_bounded(payload, MAX_WORKER_REQUEST_BYTES)
}

pub fn decode_response(payload: &[u8]) -> Result<WorkerResponse, FrameError> {
    deserialize_bounded(payload, MAX_WORKER_RESPONSE_BYTES)
}

pub fn read_payload<R: Read>(reader: &mut R, maximum: usize) -> Result<Vec<u8>, FrameError> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::TruncatedHeader)
        }
        Err(error) => return Err(FrameError::Io(error)),
    }

    let announced_u32 = u32::from_le_bytes(header);
    let announced = usize::try_from(announced_u32).map_err(|_| FrameError::LengthOverflow)?;
    if announced > maximum {
        return Err(FrameError::MessageTooLarge { announced, maximum });
    }

    let mut payload = vec![0u8; announced];
    match reader.read_exact(&mut payload) {
        Ok(()) => Ok(payload),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Err(FrameError::TruncatedPayload)
        }
        Err(error) => Err(FrameError::Io(error)),
    }
}

pub fn write_frame<W: Write>(writer: &mut W, frame: &[u8]) -> Result<(), FrameError> {
    writer.write_all(frame).map_err(FrameError::Io)?;
    writer.flush().map_err(FrameError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_debug_is_redacted() {
        let credential = WorkerCredential::new("fixture-password".to_string()).unwrap();
        let debug = format!("{credential:?}");
        assert!(!debug.contains("fixture-password"));
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn request_round_trip_preserves_correlation_fields() {
        let request = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: 42,
            command: WorkerCommand::Ping,
        };
        let frame = encode_request(&request).unwrap();
        let decoded = decode_request(&frame[4..]).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn interactive_read_requests_carry_only_worker_document_identity() {
        let commands = [
            WorkerCommand::GetOutline { document_id: 7 },
            WorkerCommand::GetMetadata { document_id: 7 },
            WorkerCommand::GetPageAnnotations {
                document_id: 7,
                page_index: 2,
            },
            WorkerCommand::GetStructureTree { document_id: 7 },
            WorkerCommand::ValidatePdfUa { document_id: 7 },
        ];
        for (index, command) in commands.into_iter().enumerate() {
            let request = WorkerRequest {
                protocol_version: WORKER_PROTOCOL_VERSION,
                request_id: u64::try_from(index).unwrap(),
                command,
            };
            let frame = encode_request(&request).unwrap();
            assert_eq!(decode_request(&frame[4..]).unwrap(), request);
        }
    }

    #[test]
    fn trailing_request_data_is_rejected() {
        let request = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: 1,
            command: WorkerCommand::Ping,
        };
        let mut frame = encode_request(&request).unwrap();
        frame.push(0);
        assert!(decode_request(&frame[4..]).is_err());
    }

    #[test]
    fn oversized_announced_frame_is_rejected_before_allocation() {
        let announced = u32::try_from(MAX_WORKER_REQUEST_BYTES + 1).unwrap();
        let bytes = announced.to_le_bytes().to_vec();
        let error = read_payload(&mut bytes.as_slice(), MAX_WORKER_REQUEST_BYTES).unwrap_err();
        assert!(matches!(error, FrameError::MessageTooLarge { .. }));
    }

    #[test]
    fn truncated_header_and_payload_are_distinct() {
        assert!(matches!(
            read_payload(&mut [1u8, 2].as_slice(), MAX_WORKER_REQUEST_BYTES),
            Err(FrameError::TruncatedHeader)
        ));
        let mut bytes = 4u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[1, 2]);
        assert!(matches!(
            read_payload(&mut bytes.as_slice(), MAX_WORKER_REQUEST_BYTES),
            Err(FrameError::TruncatedPayload)
        ));
    }

    #[test]
    fn oversized_credential_is_rejected() {
        let value = "x".repeat(MAX_WORKER_PASSWORD_CHARS + 1);
        let failure = WorkerCredential::new(value).unwrap_err();
        assert_eq!(failure.kind, WorkerFailureKind::InvalidInput);
    }

    #[test]
    fn oversized_request_payload_is_rejected_before_framing() {
        let request = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: 9,
            command: WorkerCommand::OpenDocument {
                path: "x".repeat(MAX_WORKER_REQUEST_BYTES),
                credential: None,
            },
        };
        assert!(matches!(
            encode_request(&request),
            Err(FrameError::Serialize(_)) | Err(FrameError::MessageTooLarge { .. })
        ));
    }

    #[test]
    fn malformed_bincode_is_rejected() {
        assert!(matches!(
            decode_request(&[0xff, 0x01, 0x02]),
            Err(FrameError::Deserialize(_))
        ));
    }
}
