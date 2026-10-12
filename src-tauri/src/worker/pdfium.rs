use crate::pdf_engine::render_service::{checked_rgba_len, MAX_PIXEL_DIMENSION, MAX_SCALE};
use crate::worker::ipc::{
    decode_request, encode_response, read_payload, write_frame, WorkerCapability, WorkerCommand,
    WorkerDiagnostic, WorkerFailure, WorkerFailureKind, WorkerPageSize, WorkerRenderTarget,
    WorkerRequest, WorkerResponse, WorkerResult, WorkerSearchChunk, MAX_WORKER_QUERY_CHARS,
    MAX_WORKER_REQUEST_BYTES, MAX_WORKER_SEARCH_CHUNK_BUDGET_MS, MAX_WORKER_SEARCH_CHUNK_PAGES,
    MAX_WORKER_SEARCH_RESULTS, MAX_WORKER_TEXT_ITEMS, WORKER_PROTOCOL_VERSION,
};
use crate::{BoundingBox, SearchMatch, SearchResult, TextLayoutData};
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unicode_normalization::{char::canonical_combining_class, UnicodeNormalization};

struct WorkerDocument<'a> {
    document: PdfDocument<'a>,
    accessibility: Result<lopdf::Document, String>,
    _credential: Option<crate::worker::ipc::WorkerCredential>,
}

const MAX_WORKER_CREDENTIAL_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_WORKER_ACCESSIBILITY_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const WORKER_PDFIUM_RUNTIME_HEADROOM_BYTES: u64 = 512 * 1024 * 1024;
const WORKER_MAX_RENDER_PEAK_BYTES: u64 =
    (crate::pdf_engine::render_service::MAX_RENDER_BYTES as u64 * 3)
        + MAX_WORKER_CREDENTIAL_DOCUMENT_BYTES
        + WORKER_PDFIUM_RUNTIME_HEADROOM_BYTES;
const WORKER_ADDRESS_SPACE_LIMIT_BYTES: u64 = WORKER_MAX_RENDER_PEAK_BYTES + 512 * 1024 * 1024;

pub fn run_worker() {
    let diagnostics = apply_resource_limits();
    // Check for pre-resolved runtime path from parent process (PDFIUM_RESOLVED_PATH)
    // This ensures main/worker parity and avoids CWD/PATH divergence.
    let resolved_runtime_path = std::env::var_os("PDFIUM_RESOLVED_PATH").map(PathBuf::from);
    let pdfium =
        crate::pdf_engine::get_pdfium_with_resolved_path(resolved_runtime_path).map_err(|_| {
            WorkerFailure::new(
                WorkerFailureKind::PdfiumUnavailable,
                "PDFium runtime is unavailable in the worker process",
            )
        });

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut documents = HashMap::<u64, WorkerDocument<'_>>::new();
    let mut next_document_id = 1u64;

    loop {
        let payload = match read_payload(&mut stdin, MAX_WORKER_REQUEST_BYTES) {
            Ok(payload) => payload,
            Err(error) => {
                eprintln!("pdfium worker protocol input failed: {error}");
                break;
            }
        };
        let request = match decode_request(&payload) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("pdfium worker request decode failed: {error}");
                break;
            }
        };

        let should_shutdown = matches!(request.command, WorkerCommand::Shutdown);
        let response = if request.protocol_version != WORKER_PROTOCOL_VERSION {
            failure_response(
                &request,
                WorkerFailure::new(
                    WorkerFailureKind::ProtocolError,
                    "Worker protocol version mismatch",
                ),
            )
        } else {
            let result = handle_command(
                &request.command,
                &pdfium,
                &mut documents,
                &mut next_document_id,
                &diagnostics,
            );
            WorkerResponse {
                protocol_version: WORKER_PROTOCOL_VERSION,
                request_id: request.request_id,
                result,
            }
        };

        if let Err(error) = send_response(&mut stdout, &response) {
            eprintln!("pdfium worker response write failed: {error}");
            break;
        }
        if should_shutdown {
            break;
        }
    }
}

fn handle_command<'a>(
    command: &WorkerCommand,
    pdfium: &'a Result<Pdfium, WorkerFailure>,
    documents: &mut HashMap<u64, WorkerDocument<'a>>,
    next_document_id: &mut u64,
    diagnostics: &[WorkerDiagnostic],
) -> Result<WorkerResult, WorkerFailure> {
    match command {
        WorkerCommand::Hello => Ok(WorkerResult::Hello {
            capabilities: all_capabilities(),
            ready: pdfium.is_ok(),
            diagnostics: diagnostics.to_vec(),
        }),
        WorkerCommand::Ping => Ok(WorkerResult::Pong),
        WorkerCommand::Shutdown => {
            documents.clear();
            Ok(WorkerResult::ShutdownComplete)
        }
        WorkerCommand::OpenDocument { path, credential } => {
            validate_path(path)?;
            let pdfium = pdfium.as_ref().map_err(Clone::clone)?;
            if !Path::new(path).is_file() {
                return Err(WorkerFailure::new(
                    WorkerFailureKind::FileNotFound,
                    "PDF file was not found",
                ));
            }

            let (document, accessibility) = if let Some(credential) = credential.as_ref() {
                let metadata = fs::metadata(path).map_err(|_| {
                    WorkerFailure::new(
                        WorkerFailureKind::FileNotFound,
                        "PDF file metadata could not be read",
                    )
                })?;
                if metadata.len() > MAX_WORKER_CREDENTIAL_DOCUMENT_BYTES {
                    return Err(WorkerFailure::new(
                        WorkerFailureKind::InvalidInput,
                        "Password-protected PDF exceeds the worker document size limit",
                    ));
                }
                let bytes = fs::read(path).map_err(|_| {
                    WorkerFailure::new(
                        WorkerFailureKind::FileNotFound,
                        "PDF file could not be read",
                    )
                })?;
                let accessibility =
                    load_accessibility_document_from_bytes(&bytes, Some(credential.expose()));
                (
                    pdfium.load_pdf_from_byte_vec(bytes, Some(credential.expose())),
                    accessibility,
                )
            } else {
                (
                    pdfium.load_pdf_from_file(path, None),
                    load_accessibility_document_from_path(path),
                )
            };
            let document = document.map_err(|error| map_load_error(error, credential.is_some()))?;
            let page_count = u32::from(document.pages().len());
            if page_count == 0 {
                return Err(WorkerFailure::new(
                    WorkerFailureKind::MalformedPdf,
                    "PDF contains no pages",
                ));
            }
            let protected = match document.permissions().security_handler_revision() {
                Ok(PdfSecurityHandlerRevision::Unprotected) => false,
                Ok(_) => true,
                Err(_) => {
                    return Err(WorkerFailure::new(
                        WorkerFailureKind::UnsupportedSecurity,
                        "PDF security handler state could not be determined",
                    ))
                }
            };
            let document_id = *next_document_id;
            *next_document_id = next_document_id.checked_add(1).ok_or_else(|| {
                WorkerFailure::new(
                    WorkerFailureKind::Internal,
                    "Worker document identifier exhausted",
                )
            })?;
            documents.insert(
                document_id,
                WorkerDocument {
                    document,
                    accessibility,
                    _credential: credential.clone(),
                },
            );
            Ok(WorkerResult::DocumentOpened {
                document_id,
                page_count,
                protected,
            })
        }
        WorkerCommand::CloseDocument { document_id } => {
            documents.remove(document_id);
            Ok(WorkerResult::DocumentClosed)
        }
        WorkerCommand::CloseAllDocuments => {
            documents.clear();
            Ok(WorkerResult::AllDocumentsClosed)
        }
        WorkerCommand::GetPageCount { document_id } => {
            let document = get_document(documents, *document_id)?;
            Ok(WorkerResult::PageCount(u32::from(
                document.document.pages().len(),
            )))
        }
        WorkerCommand::RenderPage {
            document_id,
            page_index,
            target,
            rotation,
        } => {
            let document = get_document(documents, *document_id)?;
            let rendered = render_page(&document.document, *page_index, *target, *rotation)?;
            Ok(rendered)
        }
        WorkerCommand::RenderThumbnail {
            document_id,
            page_index,
            target_width,
        } => {
            let document = get_document(documents, *document_id)?;
            let rendered = render_page(
                &document.document,
                *page_index,
                WorkerRenderTarget::TargetWidth(*target_width),
                0,
            )?;
            Ok(rendered)
        }
        WorkerCommand::GetPageTextLayout {
            document_id,
            page_index,
        } => {
            let document = get_document(documents, *document_id)?;
            Ok(WorkerResult::TextLayout(extract_text_layout(
                &document.document,
                *page_index,
            )?))
        }
        WorkerCommand::SearchText {
            document_id,
            query,
            max_results,
        } => {
            validate_query(query)?;
            let document = get_document(documents, *document_id)?;
            let requested_limit = usize::try_from(*max_results).unwrap_or(usize::MAX);
            let limit = requested_limit.min(MAX_WORKER_SEARCH_RESULTS);
            Ok(WorkerResult::Search(search_document(
                &document.document,
                query,
                limit,
            )?))
        }
        WorkerCommand::SearchTextPages {
            document_id,
            query,
            start_page,
            max_pages,
            time_budget_ms,
            max_results,
            first_match_index,
        } => {
            validate_query(query)?;
            if *max_pages == 0
                || *max_pages > MAX_WORKER_SEARCH_CHUNK_PAGES
                || *time_budget_ms == 0
                || *time_budget_ms > MAX_WORKER_SEARCH_CHUNK_BUDGET_MS
            {
                return Err(WorkerFailure::new(
                    WorkerFailureKind::InvalidInput,
                    "Search chunk bounds are outside the worker limits",
                ));
            }
            let document = get_document(documents, *document_id)?;
            let requested_limit = usize::try_from(*max_results).unwrap_or(usize::MAX);
            let first_match_index = usize::try_from(*first_match_index).map_err(|_| {
                WorkerFailure::new(
                    WorkerFailureKind::InvalidInput,
                    "Search chunk match index exceeds the platform limit",
                )
            })?;
            let range = search_page_range(
                &document.document,
                query,
                PageRangeBounds {
                    start_page: *start_page,
                    max_pages: *max_pages,
                    time_budget: Some(Duration::from_millis(u64::from(*time_budget_ms))),
                },
                requested_limit.min(MAX_WORKER_SEARCH_RESULTS),
                first_match_index,
            )?;
            Ok(WorkerResult::SearchChunk(WorkerSearchChunk {
                matches: range.matches,
                total_matches: range.total_matches,
                next_page: range.next_page,
            }))
        }
        WorkerCommand::GetOutline { document_id } => {
            let document = get_document(documents, *document_id)?;
            Ok(WorkerResult::Outline(extract_outline(&document.document)?))
        }
        WorkerCommand::GetPageSizes { document_id } => {
            let document = get_document(documents, *document_id)?;
            Ok(WorkerResult::PageSizes(extract_page_sizes(
                &document.document,
            )?))
        }
        WorkerCommand::GetMetadata { document_id } => {
            let document = get_document(documents, *document_id)?;
            Ok(WorkerResult::Metadata(extract_metadata(&document.document)))
        }
        WorkerCommand::GetPageAnnotations {
            document_id,
            page_index,
        } => {
            let document = get_document(documents, *document_id)?;
            let annotations =
                crate::annotations::read_page_annotations(&document.document, *page_index)
                    .map_err(|message| {
                        WorkerFailure::new(WorkerFailureKind::PdfiumLoadError, message)
                    })?;
            Ok(WorkerResult::PageAnnotations(annotations))
        }
        WorkerCommand::GetStructureTree { document_id } => {
            let document = get_document(documents, *document_id)?;
            let accessibility = document.accessibility.as_ref().map_err(|message| {
                WorkerFailure::new(WorkerFailureKind::PdfiumLoadError, message.clone())
            })?;
            let tree = crate::a11y::inspect_structure_tree_from_document(accessibility).map_err(
                |message| WorkerFailure::new(WorkerFailureKind::StructureTreeUnavailable, message),
            )?;
            Ok(WorkerResult::AccessibilityStructure(tree))
        }
        WorkerCommand::AuditAccessibility { document_id } => {
            let document = get_document(documents, *document_id)?;
            let accessibility = document.accessibility.as_ref().map_err(|message| {
                WorkerFailure::new(WorkerFailureKind::PdfiumLoadError, message.clone())
            })?;
            Ok(WorkerResult::AccessibilityAudit(
                crate::a11y::audit_accessibility_document(accessibility),
            ))
        }
    }
}

fn load_accessibility_document_from_path(path: &str) -> Result<lopdf::Document, String> {
    let length = fs::metadata(path)
        .map_err(|_| "PDF accessibility source metadata is unavailable".to_string())?
        .len();
    if length > MAX_WORKER_ACCESSIBILITY_DOCUMENT_BYTES {
        return Err("PDF exceeds the worker accessibility read limit".to_string());
    }
    let document = lopdf::Document::load(path)
        .map_err(|_| "PDF accessibility data could not be parsed in the worker".to_string())?;
    prepare_accessibility_document(document, None)
}

fn load_accessibility_document_from_bytes(
    bytes: &[u8],
    credential: Option<&str>,
) -> Result<lopdf::Document, String> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_WORKER_ACCESSIBILITY_DOCUMENT_BYTES {
        return Err("PDF exceeds the worker accessibility read limit".to_string());
    }
    let document = lopdf::Document::load_mem(bytes)
        .map_err(|_| "PDF accessibility data could not be parsed in the worker".to_string())?;
    prepare_accessibility_document(document, credential)
}

fn prepare_accessibility_document(
    mut document: lopdf::Document,
    credential: Option<&str>,
) -> Result<lopdf::Document, String> {
    if document.is_encrypted() {
        document
            .decrypt(credential.unwrap_or_default().as_bytes())
            .map_err(|_| {
                "PDF accessibility data could not be decrypted in the worker".to_string()
            })?;
    }
    Ok(document)
}

fn extract_metadata(document: &PdfDocument<'_>) -> crate::PdfMetadata {
    let metadata = document.metadata();
    let value = |tag| {
        metadata
            .get(tag)
            .map(|entry| entry.value().to_string())
            .unwrap_or_default()
    };
    crate::PdfMetadata {
        title: value(PdfDocumentMetadataTagType::Title),
        author: value(PdfDocumentMetadataTagType::Author),
        subject: value(PdfDocumentMetadataTagType::Subject),
        keywords: value(PdfDocumentMetadataTagType::Keywords),
        creator: value(PdfDocumentMetadataTagType::Creator),
    }
}

fn extract_outline(document: &PdfDocument<'_>) -> Result<Vec<crate::OutlineNode>, WorkerFailure> {
    const MAX_OUTLINE_DEPTH: usize = 256;
    const MAX_OUTLINE_NODES: usize = 100_000;

    fn build_node(
        bookmark: &PdfBookmark<'_>,
        depth: usize,
        node_count: &mut usize,
    ) -> Result<crate::OutlineNode, WorkerFailure> {
        if depth > MAX_OUTLINE_DEPTH || *node_count >= MAX_OUTLINE_NODES {
            return Err(WorkerFailure::new(
                WorkerFailureKind::MessageTooLarge,
                "PDF outline exceeds the worker traversal limit",
            ));
        }
        *node_count += 1;
        let title = bookmark.title().unwrap_or_else(|| "Untitled".to_string());
        let page_index = bookmark
            .destination()
            .and_then(|destination| destination.page_index().ok())
            .map(u32::from)
            .unwrap_or(0);
        let mut children = Vec::new();
        let mut current = bookmark.first_child();
        while let Some(child) = current {
            children.push(build_node(&child, depth + 1, node_count)?);
            current = child.next_sibling();
        }
        Ok(crate::OutlineNode {
            title,
            page_index,
            children,
        })
    }

    let mut result = Vec::new();
    let mut node_count = 0;
    let mut current = document.bookmarks().root();
    while let Some(bookmark) = current {
        result.push(build_node(&bookmark, 0, &mut node_count)?);
        current = bookmark.next_sibling();
    }
    Ok(result)
}

fn render_page(
    document: &PdfDocument<'_>,
    page_index: u32,
    target: WorkerRenderTarget,
    rotation: i32,
) -> Result<WorkerResult, WorkerFailure> {
    let pages = document.pages();
    if page_index >= u32::from(pages.len()) {
        return Err(WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "Page index is outside the document",
        ));
    }
    let page_index_u16 = u16::try_from(page_index).map_err(|_| {
        WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "Page index cannot be represented by PDFium",
        )
    })?;
    let page = pages.get(page_index_u16).map_err(|_| {
        WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "PDFium could not load the requested page",
        )
    })?;

    let target_width = match target {
        WorkerRenderTarget::Scale(scale) => {
            if !scale.is_finite() || scale <= 0.0 || scale > MAX_SCALE {
                return Err(WorkerFailure::new(
                    WorkerFailureKind::InvalidRenderParameters,
                    "Render scale is invalid",
                ));
            }
            let width = f64::from(page.width().value) * scale;
            if !width.is_finite() || width < 1.0 || width > f64::from(MAX_PIXEL_DIMENSION) {
                return Err(WorkerFailure::new(
                    WorkerFailureKind::InvalidRenderParameters,
                    "Rendered page dimensions are invalid",
                ));
            }
            width as u32
        }
        WorkerRenderTarget::TargetWidth(width) if width > 0 && width <= MAX_PIXEL_DIMENSION => {
            width
        }
        WorkerRenderTarget::TargetWidth(_) => {
            return Err(WorkerFailure::new(
                WorkerFailureKind::InvalidRenderParameters,
                "Render target width is invalid",
            ))
        }
    };

    let page_width = f64::from(page.width().value);
    let page_height = f64::from(page.height().value);
    if !page_width.is_finite()
        || !page_height.is_finite()
        || page_width <= 0.0
        || page_height <= 0.0
    {
        return Err(WorkerFailure::new(
            WorkerFailureKind::InvalidRenderParameters,
            "PDF page dimensions are invalid",
        ));
    }
    let estimated_height =
        ((f64::from(target_width) * page_height / page_width).ceil() as u32).max(1);
    let estimated = if matches!(rotation, 90 | 270) {
        (estimated_height, target_width)
    } else {
        (target_width, estimated_height)
    };
    checked_rgba_len(estimated.0, estimated.1).map_err(|message| {
        WorkerFailure::new(WorkerFailureKind::InvalidRenderParameters, message)
    })?;

    let pdf_rotation = match rotation {
        0 => PdfPageRenderRotation::None,
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => {
            return Err(WorkerFailure::new(
                WorkerFailureKind::InvalidRenderParameters,
                "Render rotation must be 0, 90, 180, or 270 degrees",
            ))
        }
    };
    // FPDF_REVERSE_BYTE_ORDER makes PDFium write straight RGBA, so the bitmap
    // buffer is the wire payload as-is. The clear fill (FPDFBitmap_FillRect)
    // ignores that flag; white is channel-symmetric, so it stays correct.
    let config = PdfRenderConfig::new()
        .set_target_width(target_width as i32)
        .rotate(pdf_rotation, true)
        .set_clear_color(PdfColor::WHITE)
        .set_reverse_byte_order(true);
    let bitmap = page.render_with_config(&config).map_err(|_| {
        WorkerFailure::new(
            WorkerFailureKind::PdfiumRenderError,
            "PDFium could not render the requested page",
        )
    })?;
    let width = bitmap.width() as u32;
    let height = bitmap.height() as u32;
    let expected = checked_rgba_len(width, height)
        .map_err(|message| WorkerFailure::new(WorkerFailureKind::PdfiumRenderError, message))?;
    let rgba = bitmap.as_raw_bytes();
    if rgba.len() != expected {
        return Err(WorkerFailure::new(
            WorkerFailureKind::PdfiumRenderError,
            "PDFium returned an invalid bitmap buffer",
        ));
    }
    Ok(WorkerResult::PageRendered {
        width,
        height,
        data: rgba,
    })
}

fn extract_text_layout(
    document: &PdfDocument<'_>,
    page_index: u32,
) -> Result<Vec<TextLayoutData>, WorkerFailure> {
    let page = get_page(document, page_index)?;
    let page_width = page.width().value;
    let page_height = page.height().value;
    if !page_width.is_finite()
        || !page_height.is_finite()
        || page_width <= 0.0
        || page_height <= 0.0
    {
        return Err(WorkerFailure::new(
            WorkerFailureKind::Internal,
            "PDF page dimensions are invalid for text layout",
        ));
    }
    let text_page = page.text().map_err(|_| {
        WorkerFailure::new(
            WorkerFailureKind::PdfiumLoadError,
            "PDFium could not extract page text",
        )
    })?;

    let mut layouts = Vec::new();
    let mut current_word_text = String::with_capacity(50);
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut prev_char_max_x = f32::MIN;
    let mut prev_char_min_y = f32::MAX;

    for character in text_page.chars().iter() {
        let character_value = match character.unicode_char().unwrap_or(' ') {
            '\u{00A0}' => ' ',
            '\u{00AD}' | '\u{200B}' | '\0' => continue,
            value => value,
        };
        let bounds = match character.loose_bounds() {
            Ok(bounds) => bounds,
            Err(_) => continue,
        };
        let ch_min_x = bounds.left().value;
        let ch_min_y = bounds.bottom().value;
        let ch_max_x = bounds.right().value;
        let ch_max_y = bounds.top().value;
        let ch_width = ch_max_x - ch_min_x;
        let ch_height = ch_max_y - ch_min_y;
        if !ch_min_x.is_finite()
            || !ch_min_y.is_finite()
            || !ch_max_x.is_finite()
            || !ch_max_y.is_finite()
            || ch_width <= 0.0
            || ch_height <= 0.0
        {
            continue;
        }

        let mut starts_new_word = false;
        if !current_word_text.is_empty() {
            let gap_x = ch_min_x - prev_char_max_x;
            let gap_y = (ch_min_y - prev_char_min_y).abs();
            starts_new_word = character_value.is_whitespace()
                || gap_x > ch_height * 0.35
                || gap_y > ch_height * 0.5;
        }
        if starts_new_word {
            push_layout(
                &mut layouts,
                &current_word_text,
                [min_x, min_y, max_x, max_y],
                [page_width, page_height],
            )?;
            current_word_text.clear();
            min_x = f32::MAX;
            min_y = f32::MAX;
            max_x = f32::MIN;
            max_y = f32::MIN;
        }
        if !character_value.is_whitespace() {
            current_word_text.push(character_value);
            min_x = min_x.min(ch_min_x);
            min_y = min_y.min(ch_min_y);
            max_x = max_x.max(ch_max_x);
            max_y = max_y.max(ch_max_y);
        }
        prev_char_max_x = ch_max_x;
        prev_char_min_y = ch_min_y;
    }
    push_layout(
        &mut layouts,
        &current_word_text,
        [min_x, min_y, max_x, max_y],
        [page_width, page_height],
    )?;
    Ok(layouts)
}

fn push_layout(
    layouts: &mut Vec<TextLayoutData>,
    text: &str,
    [min_x, min_y, max_x, max_y]: [f32; 4],
    [page_width, page_height]: [f32; 2],
) -> Result<(), WorkerFailure> {
    if text.trim().is_empty() || min_x >= max_x || min_y >= max_y {
        return Ok(());
    }
    if layouts.len() >= MAX_WORKER_TEXT_ITEMS {
        return Err(WorkerFailure::new(
            WorkerFailureKind::MessageTooLarge,
            "Text layout item limit was exceeded",
        ));
    }
    let layout = TextLayoutData {
        x: min_x / page_width,
        y: (page_height - max_y) / page_height,
        width: (max_x - min_x) / page_width,
        height: (max_y - min_y) / page_height,
        text: text.to_string(),
    };
    if [layout.x, layout.y, layout.width, layout.height]
        .iter()
        .all(|value| value.is_finite())
    {
        layouts.push(layout);
    }
    Ok(())
}

fn search_document(
    document: &PdfDocument<'_>,
    query: &str,
    limit: usize,
) -> Result<SearchResult, WorkerFailure> {
    if limit == 0 {
        return Ok(SearchResult {
            matches: Vec::new(),
            is_cancelled: false,
            total_matches: 0,
        });
    }
    let range = search_page_range(
        document,
        query,
        PageRangeBounds {
            start_page: 0,
            max_pages: u32::MAX,
            time_budget: None,
        },
        limit,
        0,
    )?;
    Ok(SearchResult {
        matches: range.matches,
        is_cancelled: false,
        total_matches: range.total_matches,
    })
}

struct PageRangeBounds {
    start_page: u32,
    max_pages: u32,
    time_budget: Option<Duration>,
}

struct PageRangeSearch {
    matches: Vec<SearchMatch>,
    total_matches: usize,
    next_page: Option<u32>,
}

/// Matches never span pages, so searching consecutive page ranges and
/// concatenating them yields exactly the whole-document result.
fn search_page_range(
    document: &PdfDocument<'_>,
    query: &str,
    bounds: PageRangeBounds,
    limit: usize,
    first_match_index: usize,
) -> Result<PageRangeSearch, WorkerFailure> {
    let started = Instant::now();
    let page_count = u32::from(document.pages().len());
    if bounds.start_page > page_count {
        return Err(WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "Search start page is outside the document",
        ));
    }
    let mut matches = Vec::new();
    let mut total_matches = 0usize;
    let query_characters = query.chars().collect::<Vec<_>>();
    let (query_normalized, _) = normalize_search_characters(&query_characters);
    if query_normalized.is_empty() {
        return Ok(PageRangeSearch {
            matches,
            total_matches,
            next_page: None,
        });
    }

    let mut page_index = bounds.start_page;
    let mut scanned_pages = 0u32;
    while page_index < page_count {
        if scanned_pages >= bounds.max_pages
            || bounds
                .time_budget
                .is_some_and(|budget| scanned_pages > 0 && started.elapsed() >= budget)
        {
            return Ok(PageRangeSearch {
                matches,
                total_matches,
                next_page: Some(page_index),
            });
        }
        search_page(
            document,
            page_index,
            &query_normalized,
            limit,
            first_match_index,
            &mut matches,
            &mut total_matches,
        )?;
        page_index += 1;
        scanned_pages += 1;
    }
    Ok(PageRangeSearch {
        matches,
        total_matches,
        next_page: None,
    })
}

fn search_page(
    document: &PdfDocument<'_>,
    page_index: u32,
    query_normalized: &[char],
    limit: usize,
    first_match_index: usize,
    matches: &mut Vec<SearchMatch>,
    total_matches: &mut usize,
) -> Result<(), WorkerFailure> {
    let page = get_page(document, page_index)?;
    let Ok(text_page) = page.text() else {
        return Ok(());
    };
    let page_width = page.width().value;
    let page_height = page.height().value;
    if !page_width.is_finite()
        || !page_height.is_finite()
        || page_width <= 0.0
        || page_height <= 0.0
    {
        return Ok(());
    }
    let characters = text_page.chars();
    let original: Vec<_> = characters.iter().collect();
    if original.is_empty() {
        return Ok(());
    }
    let original_characters = original
        .iter()
        .map(|character| character.unicode_char().unwrap_or(' '))
        .collect::<Vec<_>>();
    let (normalized, normalized_to_original) = normalize_search_characters(&original_characters);
    if normalized.len() < query_normalized.len() {
        return Ok(());
    }

    let mut start = 0usize;
    while start + query_normalized.len() <= normalized.len() {
        if normalized[start..start + query_normalized.len()] != *query_normalized {
            start += 1;
            continue;
        }
        let original_start = normalized_to_original[start].0;
        let original_end = normalized_to_original[start + query_normalized.len() - 1].1;
        *total_matches = total_matches.saturating_add(1);
        if matches.len() >= limit {
            start += query_normalized.len().max(1);
            continue;
        }

        let snippet_start = original_start.saturating_sub(40);
        let snippet_end = (original_end + 41).min(original.len());
        let snippet = original[snippet_start..snippet_end]
            .iter()
            .map(|character| character.unicode_char().unwrap_or(' '))
            .collect::<String>()
            .replace('\n', " ")
            .replace('\r', "");
        let rects = search_rects(
            &original,
            original_start,
            original_end,
            page_width,
            page_height,
        );
        matches.push(SearchMatch {
            global_match_index: first_match_index.saturating_add(*total_matches - 1),
            page_index,
            rects,
            snippet,
        });
        start += query_normalized.len().max(1);
    }
    Ok(())
}

fn normalize_search_characters(characters: &[char]) -> (Vec<char>, Vec<(usize, usize)>) {
    let mut normalized = Vec::new();
    let mut mapping = Vec::new();
    let mut start = 0usize;
    while start < characters.len() {
        let mut end = start + 1;
        while end < characters.len() && canonical_combining_class(characters[end]) != 0 {
            end += 1;
        }
        let cluster = characters[start..end].iter().collect::<String>();
        let folded = cluster.to_lowercase().nfkc().collect::<String>();
        for value in folded.chars() {
            if matches!(value, '\u{00AD}' | '\u{200B}' | '\0') {
                continue;
            }
            if value == '\u{0307}' && normalized.last() == Some(&'i') {
                continue;
            }
            normalized.push(value);
            mapping.push((start, end - 1));
        }
        start = end;
    }
    (normalized, mapping)
}

fn search_rects(
    characters: &[PdfPageTextChar<'_>],
    start: usize,
    end: usize,
    page_width: f32,
    page_height: f32,
) -> Vec<BoundingBox> {
    let mut rects = Vec::new();
    let mut line_bounds: Option<(f32, f32, f32, f32, f32)> = None;
    for character in &characters[start..=end] {
        let Ok(bounds) = character.loose_bounds() else {
            continue;
        };
        let min_x = bounds.left().value.min(bounds.right().value);
        let max_x = bounds.left().value.max(bounds.right().value);
        let min_y = bounds.bottom().value.min(bounds.top().value);
        let max_y = bounds.bottom().value.max(bounds.top().value);
        if ![min_x, max_x, min_y, max_y]
            .iter()
            .all(|value| value.is_finite())
        {
            continue;
        }
        let center = (min_y + max_y) / 2.0;
        match line_bounds {
            Some((line_min_x, line_min_y, line_max_x, line_max_y, line_center))
                if (center - line_center).abs() <= 5.0 =>
            {
                line_bounds = Some((
                    line_min_x.min(min_x),
                    line_min_y.min(min_y),
                    line_max_x.max(max_x),
                    line_max_y.max(max_y),
                    line_center,
                ));
            }
            Some(previous) => {
                push_search_rect(&mut rects, previous, page_width, page_height);
                line_bounds = Some((min_x, min_y, max_x, max_y, center));
            }
            None => line_bounds = Some((min_x, min_y, max_x, max_y, center)),
        }
    }
    if let Some(bounds) = line_bounds {
        push_search_rect(&mut rects, bounds, page_width, page_height);
    }
    rects
}

fn push_search_rect(
    rects: &mut Vec<BoundingBox>,
    bounds: (f32, f32, f32, f32, f32),
    page_width: f32,
    page_height: f32,
) {
    let (min_x, min_y, max_x, max_y, _) = bounds;
    let rect = BoundingBox {
        x: min_x / page_width,
        y: (page_height - max_y) / page_height,
        width: (max_x - min_x) / page_width,
        height: (max_y - min_y) / page_height,
    };
    if [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
    {
        rects.push(rect);
    }
}

fn get_document<'documents, 'pdfium>(
    documents: &'documents HashMap<u64, WorkerDocument<'pdfium>>,
    document_id: u64,
) -> Result<&'documents WorkerDocument<'pdfium>, WorkerFailure> {
    documents.get(&document_id).ok_or_else(|| {
        WorkerFailure::new(
            WorkerFailureKind::InvalidInput,
            "Worker document identifier is stale or unknown",
        )
    })
}

fn get_page<'document>(
    document: &'document PdfDocument<'_>,
    page_index: u32,
) -> Result<PdfPage<'document>, WorkerFailure> {
    if page_index >= u32::from(document.pages().len()) {
        return Err(WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "Page index is outside the document",
        ));
    }
    let page_index = u16::try_from(page_index).map_err(|_| {
        WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "Page index cannot be represented by PDFium",
        )
    })?;
    document.pages().get(page_index).map_err(|_| {
        WorkerFailure::new(
            WorkerFailureKind::InvalidPageIndex,
            "PDFium could not load the requested page",
        )
    })
}

fn map_load_error(error: PdfiumError, supplied_credential: bool) -> WorkerFailure {
    match error {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)
            if supplied_credential =>
        {
            WorkerFailure::new(
                WorkerFailureKind::WrongPassword,
                "PDF password is incorrect",
            )
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            WorkerFailure::new(
                WorkerFailureKind::PasswordRequired,
                "PDF requires a password",
            )
        }
        _ => WorkerFailure::new(
            WorkerFailureKind::MalformedPdf,
            "PDFium could not parse the PDF document",
        ),
    }
}

fn validate_path(path: &str) -> Result<(), WorkerFailure> {
    if path.is_empty() || path.chars().count() > crate::worker::ipc::MAX_WORKER_PATH_CHARS {
        return Err(WorkerFailure::new(
            WorkerFailureKind::InvalidInput,
            "PDF path is empty or exceeds the worker limit",
        ));
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<(), WorkerFailure> {
    if query.trim().is_empty() || query.chars().count() > MAX_WORKER_QUERY_CHARS {
        return Err(WorkerFailure::new(
            WorkerFailureKind::InvalidInput,
            "Search query is empty or exceeds the worker limit",
        ));
    }
    Ok(())
}

fn extract_page_sizes(document: &PdfDocument<'_>) -> Result<Vec<WorkerPageSize>, WorkerFailure> {
    let pages = document.pages();
    (0..pages.len())
        .map(|index| {
            let size = pages.page_size(index).map_err(|_| {
                WorkerFailure::new(
                    WorkerFailureKind::PdfiumLoadError,
                    "PDFium could not read the page size",
                )
            })?;
            let (width, height) = (size.width().value, size.height().value);
            Ok(
                if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
                    WorkerPageSize {
                        width_pt: width,
                        height_pt: height,
                    }
                } else {
                    WorkerPageSize {
                        width_pt: 0.0,
                        height_pt: 0.0,
                    }
                },
            )
        })
        .collect()
}

fn all_capabilities() -> Vec<WorkerCapability> {
    vec![
        WorkerCapability::OpenDocument,
        WorkerCapability::CloseDocument,
        WorkerCapability::CloseAllDocuments,
        WorkerCapability::GetPageCount,
        WorkerCapability::RenderPage,
        WorkerCapability::RenderThumbnail,
        WorkerCapability::GetPageTextLayout,
        WorkerCapability::SearchText,
        WorkerCapability::SearchTextPages,
        WorkerCapability::GetOutline,
        WorkerCapability::GetMetadata,
        WorkerCapability::GetPageAnnotations,
        WorkerCapability::GetStructureTree,
        WorkerCapability::AuditAccessibility,
        WorkerCapability::Ping,
        WorkerCapability::Shutdown,
        WorkerCapability::GetPageSizes,
    ]
}

fn failure_response(request: &WorkerRequest, failure: WorkerFailure) -> WorkerResponse {
    WorkerResponse {
        protocol_version: WORKER_PROTOCOL_VERSION,
        request_id: request.request_id,
        result: Err(failure),
    }
}

fn send_response(
    stdout: &mut impl io::Write,
    response: &WorkerResponse,
) -> Result<(), crate::worker::ipc::FrameError> {
    match encode_response(response) {
        Ok(frame) => write_frame(stdout, &frame),
        Err(_) => {
            let fallback = WorkerResponse {
                protocol_version: WORKER_PROTOCOL_VERSION,
                request_id: response.request_id,
                result: Err(WorkerFailure::new(
                    WorkerFailureKind::MessageTooLarge,
                    "Worker response exceeds the configured message limit",
                )),
            };
            let frame = encode_response(&fallback)?;
            write_frame(stdout, &frame)
        }
    }
}

pub(crate) fn apply_resource_limits() -> Vec<WorkerDiagnostic> {
    let mut diagnostics = Vec::new();
    #[cfg(target_family = "unix")]
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: WORKER_ADDRESS_SPACE_LIMIT_BYTES as libc::rlim_t,
            rlim_max: WORKER_ADDRESS_SPACE_LIMIT_BYTES as libc::rlim_t,
        };
        if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
            diagnostics.push(WorkerDiagnostic {
                code: "PDFIUM_WORKER_RESOURCE_LIMIT_UNAVAILABLE".to_string(),
                message: "Worker address-space resource limit could not be applied".to_string(),
            });
        }
    }
    #[cfg(target_family = "windows")]
    diagnostics.push(WorkerDiagnostic {
        code: "PDFIUM_WORKER_WINDOWS_JOB_OBJECT_UNAVAILABLE".to_string(),
        message: "Windows Job Object containment is not enabled for this worker".to_string(),
    });
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_path_and_query_are_typed_input_errors() {
        let path = "x".repeat(crate::worker::ipc::MAX_WORKER_PATH_CHARS + 1);
        assert_eq!(
            validate_path(&path).unwrap_err().kind,
            WorkerFailureKind::InvalidInput
        );
        let query = "x".repeat(MAX_WORKER_QUERY_CHARS + 1);
        assert_eq!(
            validate_query(&query).unwrap_err().kind,
            WorkerFailureKind::InvalidInput
        );
    }

    #[test]
    fn address_space_limit_covers_the_bounded_peak_render_model() {
        let peak = std::hint::black_box(WORKER_MAX_RENDER_PEAK_BYTES);
        let limit = std::hint::black_box(WORKER_ADDRESS_SPACE_LIMIT_BYTES);
        assert!(peak <= limit);
        assert_eq!(
            WORKER_MAX_RENDER_PEAK_BYTES,
            3 * crate::pdf_engine::render_service::MAX_RENDER_BYTES as u64
                + MAX_WORKER_CREDENTIAL_DOCUMENT_BYTES
                + WORKER_PDFIUM_RUNTIME_HEADROOM_BYTES
        );
    }

    #[test]
    fn empty_query_is_rejected_at_worker_boundary() {
        assert_eq!(
            validate_query(" \t").unwrap_err().kind,
            WorkerFailureKind::InvalidInput
        );
    }

    #[test]
    fn search_normalization_covers_unicode_whitespace_and_discretionary_marks() {
        let normalize =
            |value: &str| normalize_search_characters(&value.chars().collect::<Vec<_>>()).0;
        assert_eq!(normalize("A\u{00a0}B"), normalize("A B"));
        assert_eq!(normalize("soft\u{00ad}hyphen"), normalize("softhyphen"));
        assert_eq!(normalize("CAFÉ"), normalize("CAFE\u{0301}"));
        assert_eq!(normalize("İ"), normalize("i"));
        assert_eq!(normalize("I"), normalize("i"));
        assert_ne!(normalize("ı"), normalize("i"));
        assert_eq!(
            normalize("FIRST\nSECOND"),
            vec!['f', 'i', 'r', 's', 't', '\n', 's', 'e', 'c', 'o', 'n', 'd']
        );
    }

    /// Writes an A4 text page with a few filled and stroked vector paths, the
    /// render-baseline workload shared with `worker_pdfium_integration.rs`.
    fn write_a4_text_pdf(path: &std::path::Path) {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream, StringFormat};

        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let mut operations = vec![
            Operation::new("rg", vec![0.85.into(), 0.2.into(), 0.2.into()]),
            Operation::new("re", vec![40.into(), 760.into(), 515.into(), 40.into()]),
            Operation::new("f", vec![]),
            Operation::new("RG", vec![0.1.into(), 0.3.into(), 0.8.into()]),
            Operation::new("w", vec![2.into()]),
            Operation::new("m", vec![40.into(), 60.into()]),
            Operation::new(
                "c",
                vec![
                    200.into(),
                    160.into(),
                    400.into(),
                    0.into(),
                    555.into(),
                    90.into(),
                ],
            ),
            Operation::new("S", vec![]),
            Operation::new("rg", vec![0.into(), 0.into(), 0.into()]),
        ];
        for line in 0..60i64 {
            operations.extend([
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 10.into()]),
                Operation::new("Td", vec![40.into(), (740 - line * 11).into()]),
                Operation::new(
                    "Tj",
                    vec![Object::String(
                        format!(
                            "{line:02} The quick brown fox jumps over the lazy dog; render baseline text line."
                        )
                        .into_bytes(),
                        StringFormat::Literal,
                    )],
                ),
                Operation::new("ET", vec![]),
            ]);
        }
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            "Contents" => content_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    fn timing_summary(label: &str, mut samples: Vec<f64>) {
        samples.sort_by(f64::total_cmp);
        let percentile = |p: f64| samples[((samples.len() - 1) as f64 * p).round() as usize];
        eprintln!(
            "render-baseline {label}: n={} min_ms={:.2} median_ms={:.2} p95_ms={:.2}",
            samples.len(),
            samples[0],
            percentile(0.5),
            percentile(0.95)
        );
    }

    /// Manual worker-internal render baseline: the real `render_page` plus
    /// response encoding for one A4 page at scale 2.0, next to the bare PDFium
    /// render it wraps. Run from `src-tauri` with a release-profile build:
    /// `TAURI_RESOURCE_DIR=$PWD cargo test --release --lib -- --ignored --nocapture render_page_a4_timing`
    #[test]
    #[ignore = "manual render baseline measurement"]
    fn render_page_a4_timing() {
        const WARMUP: usize = 5;
        const SAMPLES: usize = 40;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a4-baseline.pdf");
        write_a4_text_pdf(&path);
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let document = pdfium.load_pdf_from_file(&path, None).unwrap();
        let target = WorkerRenderTarget::Scale(2.0);
        let measure = |run: &mut dyn FnMut()| {
            for _ in 0..WARMUP {
                run();
            }
            (0..SAMPLES)
                .map(|_| {
                    let started = Instant::now();
                    run();
                    started.elapsed().as_secs_f64() * 1000.0
                })
                .collect::<Vec<_>>()
        };

        let page = document.pages().get(0).unwrap();
        let config = PdfRenderConfig::new()
            .set_target_width(1190)
            .rotate(PdfPageRenderRotation::None, true)
            .set_clear_color(PdfColor::WHITE)
            .set_reverse_byte_order(true);
        timing_summary(
            "pdfium_render_and_buffer_copy",
            measure(&mut || {
                let bitmap = page.render_with_config(&config).unwrap();
                std::hint::black_box(bitmap.as_raw_bytes());
            }),
        );
        timing_summary(
            "worker_render_page",
            measure(&mut || {
                std::hint::black_box(render_page(&document, 0, target, 0).unwrap());
            }),
        );
        timing_summary(
            "worker_render_page_and_encode_response",
            measure(&mut || {
                let response = WorkerResponse {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    request_id: 1,
                    result: render_page(&document, 0, target, 0),
                };
                std::hint::black_box(encode_response(&response).unwrap());
            }),
        );
        let WorkerResult::PageRendered { width, height, .. } =
            render_page(&document, 0, target, 0).unwrap()
        else {
            panic!("expected rendered page");
        };
        eprintln!("render-baseline a4 bitmap: {width}x{height}");
    }
}
