use preview_alternative_lib::a11y::{
    AccessibilityAuditReport, AccessibilityCoverage, AccessibilityDocumentFacts,
    AccessibilityReferenceProfile, AccessibilityStructureInspection, AccessibilitySummaryStatus,
    AccessibilityTextState, PdfUaDeclaration, ACCESSIBILITY_AUDIT_SCHEMA_VERSION,
    ACCESSIBILITY_AUDIT_VERSION, ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION,
};
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
        WorkerCapability::AuditAccessibility,
        WorkerCapability::Ping,
        WorkerCapability::Shutdown,
        WorkerCapability::SearchTextPages,
        WorkerCapability::GetPageSizes,
    ]
}

/// Pages in the "chunked-search" fake document; every page holds one match.
const CHUNKED_SEARCH_PAGES: u32 = 6;
/// Pages in the "hang-render-pages=<i,j,...>" fake document.
const HANG_RENDER_PAGES: u32 = 6;

/// Page indexes whose RenderPage never answers in "hang-render-pages=1,3" mode.
fn hanging_render_pages(mode: &str) -> Option<Vec<u32>> {
    let pages = mode.strip_prefix("hang-render-pages=")?;
    Some(pages.split(',').map(|page| page.parse().unwrap()).collect())
}

fn accessibility_audit_fixture() -> AccessibilityAuditReport {
    AccessibilityAuditReport {
        schema_version: ACCESSIBILITY_AUDIT_SCHEMA_VERSION.to_string(),
        audit_version: ACCESSIBILITY_AUDIT_VERSION.to_string(),
        reference_profile: AccessibilityReferenceProfile::Undetermined,
        summary_status: AccessibilitySummaryStatus::AuditIncomplete,
        document_facts: AccessibilityDocumentFacts {
            header_version: Some("1.7".to_string()),
            catalog_version: None,
            effective_version: Some("1.7".to_string()),
            pdfua_declaration: PdfUaDeclaration {
                part: None,
                revision: None,
                state: AccessibilityTextState::Missing,
            },
            page_count: 1,
            encrypted: false,
            has_structure_tree: false,
            has_mark_info: false,
            has_marked_entry: false,
            marked: false,
            display_document_title: false,
            document_language: AccessibilityTextState::Missing,
            xmp_title: AccessibilityTextState::Missing,
            info_title: AccessibilityTextState::Missing,
        },
        checks: Vec::new(),
        coverage: AccessibilityCoverage {
            implemented_check_ids: Vec::new(),
            not_implemented_check_ids: Vec::new(),
            incomplete_check_ids: Vec::new(),
            machine_check_count: 0,
            human_review_count: 0,
            categories: Vec::new(),
            limitations: vec!["test helper has no parsed accessibility source".to_string()],
        },
        human_review: Vec::new(),
        warnings: vec!["test helper synthetic audit".to_string()],
        truncated: false,
    }
}

fn accessibility_structure_fixture() -> AccessibilityStructureInspection {
    AccessibilityStructureInspection {
        schema_version: ACCESSIBILITY_STRUCTURE_SCHEMA_VERSION.to_string(),
        inspection_version: "bounded-structure-inspection/1".to_string(),
        root: None,
        findings: vec!["fixture has no structure tree".to_string()],
        warnings: Vec::new(),
        truncated: false,
        limitations: Vec::new(),
    }
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

    let hanging_pages = hanging_render_pages(&mode);
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
            // An optional second argument names a file created once a command
            // was read, so a test can cancel a command known to be in flight.
            if let Some(marker) = std::env::args().nth(2) {
                std::fs::write(marker, b"received").unwrap();
            }
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
        if mode == "slow-page-count"
            && matches!(
                &request.command,
                preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount { .. }
            )
        {
            // Finishes well inside the page-count cancellation grace period.
            std::thread::sleep(Duration::from_millis(100));
        }
        if let preview_alternative_lib::worker::ipc::WorkerCommand::RenderPage {
            page_index, ..
        } = &request.command
        {
            if hanging_pages
                .as_ref()
                .is_some_and(|pages| pages.contains(page_index))
            {
                std::thread::sleep(Duration::from_secs(60));
                return;
            }
        }
        let is_audit = matches!(
            &request.command,
            preview_alternative_lib::worker::ipc::WorkerCommand::AuditAccessibility { .. }
        );
        if is_audit {
            match mode.as_str() {
                "hang-audit" => {
                    std::thread::sleep(Duration::from_secs(60));
                    return;
                }
                "crash-audit" => return,
                "oversized-audit" => {
                    let announced = u32::try_from(
                        preview_alternative_lib::worker::ipc::MAX_WORKER_RESPONSE_BYTES + 1,
                    )
                    .unwrap();
                    let _ = stdout.write_all(&announced.to_le_bytes());
                    let _ = stdout.flush();
                    return;
                }
                _ => {}
            }
        }
        let result = match request.command {
            preview_alternative_lib::worker::ipc::WorkerCommand::OpenDocument { .. } => {
                Ok(WorkerResult::DocumentOpened {
                    document_id: 1,
                    page_count: if mode == "chunked-search" {
                        CHUNKED_SEARCH_PAGES
                    } else if hanging_pages.is_some() {
                        HANG_RENDER_PAGES
                    } else {
                        1
                    },
                    protected: false,
                })
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::SearchTextPages {
                start_page,
                max_results,
                first_match_index,
                ..
            } if mode == "chunked-search" => {
                // One page per chunk, each taking long enough for other reads
                // to queue behind it.
                std::thread::sleep(Duration::from_millis(80));
                let next = start_page + 1;
                Ok(WorkerResult::SearchChunk(
                    preview_alternative_lib::worker::ipc::WorkerSearchChunk {
                        matches: if max_results > 0 {
                            vec![preview_alternative_lib::SearchMatch {
                                global_match_index: usize::try_from(first_match_index).unwrap(),
                                page_index: start_page,
                                rects: Vec::new(),
                                snippet: format!("page {start_page}"),
                            }]
                        } else {
                            Vec::new()
                        },
                        total_matches: 1,
                        next_page: (next < CHUNKED_SEARCH_PAGES).then_some(next),
                    },
                ))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount { .. } => {
                Ok(WorkerResult::PageCount(1))
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::RenderPage { .. }
                if hanging_pages.is_some() =>
            {
                Ok(WorkerResult::PageRendered {
                    width: 1,
                    height: 1,
                    data: vec![0, 0, 0, 255],
                })
            }
            preview_alternative_lib::worker::ipc::WorkerCommand::GetPageSizes { .. } => {
                Ok(WorkerResult::PageSizes(vec![
                    preview_alternative_lib::worker::ipc::WorkerPageSize {
                        width_pt: 612.0,
                        height_pt: 792.0,
                    },
                ]))
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
            preview_alternative_lib::worker::ipc::WorkerCommand::GetStructureTree { .. } => Ok(
                WorkerResult::AccessibilityStructure(accessibility_structure_fixture()),
            ),
            preview_alternative_lib::worker::ipc::WorkerCommand::AuditAccessibility { .. } => Ok(
                WorkerResult::AccessibilityAudit(accessibility_audit_fixture()),
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
            protocol_version: if mode == "wrong-version-audit" && is_audit {
                WORKER_PROTOCOL_VERSION + 1
            } else {
                WORKER_PROTOCOL_VERSION
            },
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
