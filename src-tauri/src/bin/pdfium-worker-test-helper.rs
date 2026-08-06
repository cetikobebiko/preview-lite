use preview_alternative_lib::worker::ipc::{
    decode_request, encode_response, read_payload, write_frame, WorkerCapability, WorkerFailure,
    WorkerFailureKind, WorkerResponse, WorkerResult, MAX_WORKER_REQUEST_BYTES,
    WORKER_PROTOCOL_VERSION,
};
use std::io::{self, Write};
use std::time::Duration;

fn capabilities() -> Vec<WorkerCapability> {
    vec![
        WorkerCapability::OpenDocument,
        WorkerCapability::CloseDocument,
        WorkerCapability::CloseAllDocuments,
        WorkerCapability::GetPageCount,
        WorkerCapability::RenderPage,
        WorkerCapability::RenderThumbnail,
        WorkerCapability::GetPageTextLayout,
        WorkerCapability::SearchText,
        WorkerCapability::GetOutline,
        WorkerCapability::GetMetadata,
        WorkerCapability::GetPageAnnotations,
        WorkerCapability::GetStructureTree,
        WorkerCapability::ValidatePdfUa,
        WorkerCapability::Ping,
        WorkerCapability::Shutdown,
    ]
}

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "normal".to_string());
    if mode == "stderr-flood" {
        let mut stderr = io::stderr().lock();
        for _ in 0..512 {
            let _ = stderr.write_all(&[b'x'; 8192]);
        }
        let _ = stderr.flush();
    }

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let hello_payload = read_payload(&mut stdin, MAX_WORKER_REQUEST_BYTES).unwrap();
    let hello = decode_request(&hello_payload).unwrap();

    match mode.as_str() {
        "hang-hello" => {
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        "partial-hello" => {
            let _ = stdout.write_all(&16u32.to_le_bytes());
            let _ = stdout.write_all(&[1, 2]);
            let _ = stdout.flush();
            return;
        }
        "oversized-hello" => {
            let announced =
                u32::try_from(preview_alternative_lib::worker::ipc::MAX_WORKER_RESPONSE_BYTES + 1)
                    .unwrap();
            let _ = stdout.write_all(&announced.to_le_bytes());
            let _ = stdout.flush();
            return;
        }
        _ => {}
    }

    let hello_response = WorkerResponse {
        protocol_version: if mode == "wrong-version-hello" {
            WORKER_PROTOCOL_VERSION + 1
        } else {
            WORKER_PROTOCOL_VERSION
        },
        request_id: if mode == "wrong-id-hello" {
            hello.request_id + 1
        } else {
            hello.request_id
        },
        result: Ok(WorkerResult::Hello {
            capabilities: capabilities(),
            ready: mode != "not-ready",
            diagnostics: Vec::new(),
        }),
    };
    let frame = encode_response(&hello_response).unwrap();
    write_frame(&mut stdout, &frame).unwrap();
    if mode == "exit-after-hello" {
        return;
    }

    loop {
        let payload = match read_payload(&mut stdin, MAX_WORKER_REQUEST_BYTES) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        let request = match decode_request(&payload) {
            Ok(request) => request,
            Err(_) => return,
        };
        if mode == "hang-command" {
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        if mode == "hang-close"
            && matches!(
                &request.command,
                preview_alternative_lib::worker::ipc::WorkerCommand::CloseDocument { .. }
            )
        {
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        if mode == "hang-page-count"
            && matches!(
                &request.command,
                preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount { .. }
            )
        {
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let result = match request.command {
            preview_alternative_lib::worker::ipc::WorkerCommand::OpenDocument { .. } => {
                Ok(WorkerResult::DocumentOpened {
                    document_id: 1,
                    page_count: 1,
                    protected: false,
                })
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount { .. } => {
                Ok(WorkerResult::PageCount(1))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetOutline { .. } => {
                Ok(WorkerResult::Outline(Vec::new()))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetMetadata { .. } => {
                Ok(WorkerResult::Metadata(Default::default()))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetPageAnnotations { .. } => {
                Ok(WorkerResult::PageAnnotations(Vec::new()))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetStructureTree { .. } => {
                Err(WorkerFailure::new(
                    WorkerFailureKind::StructureTreeUnavailable,
                    "fixture has no structure tree",
                ))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::ValidatePdfUa { .. } => Ok(
                WorkerResult::PdfUaValidation(preview_alternative_lib::a11y::AccessibilityReport {
                    is_tagged: false,
                    has_document_title: false,
                    issues: vec!["fixture is untagged".to_string()],
                    score: 50,
                }),
            ),
            preview_alternative_lib::worker::ipc::WorkerCommand::CloseDocument { .. } => {
                Ok(WorkerResult::DocumentClosed)
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::CloseAllDocuments => {
                Ok(WorkerResult::AllDocumentsClosed)
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::Ping => Ok(WorkerResult::Pong),
            preview_alternative_lib::worker::ipc::WorkerCommand::Shutdown => {
                Ok(WorkerResult::ShutdownComplete)
            }
            _ => Err(WorkerFailure::new(
                WorkerFailureKind::Internal,
                "fake worker command is not implemented",
            )),
        };
        let response = WorkerResponse {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id: if mode == "wrong-id-command" {
                request.request_id + 1
            } else {
                request.request_id
            },
            result,
        };
        let frame = encode_response(&response).unwrap();
        if write_frame(&mut stdout, &frame).is_err() {
            return;
        }
        if matches!(response.result, Ok(WorkerResult::ShutdownComplete)) {
            if mode == "ack-shutdown-hang" {
                std::thread::sleep(Duration::from_secs(60));
            }
            return;
        }
    }
}
