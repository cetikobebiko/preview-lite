pub mod endpoint;
pub mod error;
pub mod http_client;
pub mod input_validation;
pub mod mock_server;
pub mod request_registry;
pub mod security_acceptance_tests;
pub mod stream_parser;
pub mod vector_store;

use endpoint::{
    build_secure_reqwest_client, is_authorized, parse_and_validate_url,
    resolve_and_classify_endpoint, AiEndpointConsent, AiEndpointProfile, AiEndpointRuntimeContext,
    AiEndpointStatusDto, AiEndpointStore, EndpointMode,
};
use error::{AiError, AiErrorWire};
use http_client::SecureHttpClient;
use input_validation::{
    validate_caller_attempt_id, validate_chat_input, validate_chunk_count, validate_chunk_params,
    validate_extracted_doc, validate_model_name, validate_query, validate_request_id,
    validate_single_chunk_bytes, validate_summary_input, ChatMessageInput,
};
use request_registry::{AiRequestRegistry, CancellationCapability, RequestOwner, RequestState};
use stream_parser::NdjsonStreamParser;
pub use vector_store::{ChunkData, IndexedSessionData, VectorStore};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, State};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SearchResultChunk {
    pub text: String,
    pub page_index: u32,
    pub score: f32,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub(crate) fn extract_semantic_page_texts(
    context: &crate::document_session::SessionJobContext,
) -> Result<Vec<(u32, String)>, crate::document_session::SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    match crate::job_worker::run_job(
        context,
        crate::job_worker::JobCommand::ExtractSemanticText,
        Duration::from_secs(180),
    )? {
        crate::job_worker::JobResult::PageTexts(page_texts) => Ok(page_texts
            .into_iter()
            .map(|page| (page.page_index, page.text))
            .collect()),
        _ => Err(
            crate::document_session::SessionJobOperationError::failed_with_code(
                "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                "Semantic extraction worker beklenmeyen sonuç döndürdü",
            ),
        ),
    }
}

#[tauri::command]
pub async fn preview_ai_endpoint(url: String) -> Result<AiEndpointProfile, AiErrorWire> {
    let (scheme, host, port) = parse_and_validate_url(&url).map_err(|e| e.to_wire())?;

    let (mode, _addrs) = resolve_and_classify_endpoint(&scheme, &host, port)
        .await
        .map_err(|e| e.to_wire())?;

    let fingerprint = endpoint::compute_endpoint_fingerprint(1, &mode, &scheme, &host, port);

    let profile = AiEndpointProfile {
        policy_version: 1,
        mode,
        scheme: scheme.clone(),
        canonical_host: host.clone(),
        effective_port: port,
        canonical_origin: format!("{}://{}:{}", scheme, host, port),
        endpoint_fingerprint: fingerprint,
        created_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    };

    Ok(profile)
}

#[tauri::command]
pub async fn configure_ai_endpoint(
    url: String,
    remote_approved: bool,
    private_network_approved: bool,
    endpoint_store: State<'_, AiEndpointStore>,
    request_registry: State<'_, AiRequestRegistry>,
    vector_store: State<'_, VectorStore>,
) -> Result<AiEndpointProfile, AiErrorWire> {
    let (scheme, host, port) = parse_and_validate_url(&url).map_err(|e| e.to_wire())?;

    let (mode, addrs) = resolve_and_classify_endpoint(&scheme, &host, port)
        .await
        .map_err(|e| e.to_wire())?;

    let fingerprint = endpoint::compute_endpoint_fingerprint(1, &mode, &scheme, &host, port);

    let consent = Arc::new(AiEndpointConsent {
        endpoint_fingerprint: fingerprint.clone(),
        remote_approved,
        private_network_approved,
        policy_version: 1,
        updated_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    });

    if !is_authorized(&mode, Some(&consent), &fingerprint) {
        if mode == EndpointMode::PrivateNetwork && !private_network_approved {
            return Err(AiError::EndpointPrivateConsentRequired(url).to_wire());
        }
        return Err(AiError::EndpointRemoteConsentRequired(url).to_wire());
    }

    let profile = Arc::new(AiEndpointProfile {
        policy_version: 1,
        mode,
        scheme: scheme.clone(),
        canonical_host: host.clone(),
        effective_port: port,
        canonical_origin: format!("{}://{}:{}", scheme, host, port),
        endpoint_fingerprint: fingerprint,
        created_at_ms: consent.updated_at_ms,
    });

    let client = Arc::new(build_secure_reqwest_client(&host, &addrs).map_err(|e| e.to_wire())?);

    let ctx = Arc::new(AiEndpointRuntimeContext {
        profile: profile.clone(),
        consent,
        client,
        validated_addresses: addrs,
        resolved_at_ms: profile.created_at_ms,
    });

    endpoint_store.set_runtime_context(ctx);
    request_registry.cancel_all_for_endpoint_change();
    vector_store.clear_all();

    Ok((*profile).clone())
}

#[tauri::command]
pub async fn get_ai_endpoint_status(
    endpoint_store: State<'_, AiEndpointStore>,
) -> Result<AiEndpointStatusDto, AiErrorWire> {
    let ctx = endpoint_store
        .get_active_context()
        .map_err(|e| e.to_wire())?;
    let is_auth = is_authorized(
        &ctx.profile.mode,
        Some(&ctx.consent),
        &ctx.profile.endpoint_fingerprint,
    );
    Ok(AiEndpointStatusDto {
        profile: (*ctx.profile).clone(),
        consent: (*ctx.consent).clone(),
        is_authorized: is_auth,
    })
}

#[tauri::command]
pub async fn clear_ai_endpoint_config(
    endpoint_store: State<'_, AiEndpointStore>,
    request_registry: State<'_, AiRequestRegistry>,
    vector_store: State<'_, VectorStore>,
) -> Result<AiEndpointProfile, AiErrorWire> {
    let ctx = endpoint_store.clear_config();
    request_registry.cancel_all_for_endpoint_change();
    vector_store.clear_all();
    Ok((*ctx.profile).clone())
}

#[tauri::command]
pub async fn cancel_ai_request(
    request_id: String,
    session_id: Option<String>,
    expected_content_generation: Option<u64>,
    caller_attempt_id: Option<String>,
    request_registry: State<'_, AiRequestRegistry>,
) -> Result<(), AiErrorWire> {
    validate_request_id(&request_id).map_err(|e| e.to_wire())?;

    let expected_owner = if let (Some(sid), Some(gen)) = (session_id, expected_content_generation) {
        Some(RequestOwner::SessionNative {
            session_id: sid,
            expected_content_generation: gen,
        })
    } else if let Some(att_id) = caller_attempt_id {
        let _ = validate_caller_attempt_id(&att_id).map_err(|e| e.to_wire())?;
        Some(RequestOwner::SelectionScoped {
            caller_attempt_id: att_id,
        })
    } else {
        None
    };

    request_registry
        .cancel_request(&request_id, expected_owner.as_ref())
        .map_err(|e| e.to_wire())
}

pub enum IndexCoordinationOutcome<T> {
    Completed(crate::document_session::SessionJobOutcome<T>),
    Cancelled(crate::document_session::SessionJobOutcome<T>),
    Failed(crate::document_session::SessionJobOutcome<T>),
    TimedOut(crate::document_session::SessionJobOutcome<T>),
}

fn normalize_coordination_outcome<T>(
    outcome: crate::document_session::SessionJobOutcome<T>,
    cancel_outcome: Option<crate::document_session::SessionJobCancelOutcome>,
    timeout_triggered: bool,
) -> (IndexCoordinationOutcome<T>, RequestState) {
    match outcome {
        crate::document_session::SessionJobOutcome::Completed {
            cancel_requested,
            cancellation_reason,
            ..
        } => {
            if cancel_requested {
                match cancellation_reason {
                    Some(crate::document_session::SessionJobCancellationReason::Timeout) => {
                        let failed = outcome.map_to_failed(
                            "AI_ENDPOINT_TOTAL_TIMEOUT",
                            "Yapay zeka indeksleme işlemi 300 saniyelik toplam zaman sınırını aştı",
                        );
                        (
                            IndexCoordinationOutcome::TimedOut(failed),
                            RequestState::Failed,
                        )
                    }
                    Some(reason) => {
                        let cancelled = outcome.map_to_cancelled(reason);
                        (
                            IndexCoordinationOutcome::Cancelled(cancelled),
                            RequestState::Cancelled,
                        )
                    }
                    None => {
                        let cancelled = outcome.map_to_cancelled(
                            crate::document_session::SessionJobCancellationReason::UserRequested,
                        );
                        (
                            IndexCoordinationOutcome::Cancelled(cancelled),
                            RequestState::Cancelled,
                        )
                    }
                }
            } else if cancel_outcome.as_ref().is_some_and(|res| {
                matches!(
                    res,
                    crate::document_session::SessionJobCancelOutcome::AlreadyTerminal
                )
            }) {
                (
                    IndexCoordinationOutcome::Completed(outcome),
                    RequestState::Completed,
                )
            } else if timeout_triggered {
                let failed = outcome.map_to_failed(
                    "AI_ENDPOINT_TOTAL_TIMEOUT",
                    "Yapay zeka indeksleme işlemi 300 saniyelik toplam zaman sınırını aştı",
                );
                (
                    IndexCoordinationOutcome::TimedOut(failed),
                    RequestState::Failed,
                )
            } else {
                (
                    IndexCoordinationOutcome::Completed(outcome),
                    RequestState::Completed,
                )
            }
        }
        crate::document_session::SessionJobOutcome::Cancelled { cancellation, .. } => {
            if matches!(
                cancellation,
                crate::document_session::SessionJobCancellationReason::Timeout
            ) {
                let failed = outcome.map_to_failed(
                    "AI_ENDPOINT_TOTAL_TIMEOUT",
                    "Yapay zeka indeksleme işlemi 300 saniyelik toplam zaman sınırını aştı",
                );
                (
                    IndexCoordinationOutcome::TimedOut(failed),
                    RequestState::Failed,
                )
            } else {
                (
                    IndexCoordinationOutcome::Cancelled(outcome),
                    RequestState::Cancelled,
                )
            }
        }
        crate::document_session::SessionJobOutcome::Failed { .. } => (
            IndexCoordinationOutcome::Failed(outcome),
            RequestState::Failed,
        ),
    }
}

pub(crate) async fn wait_for_cancellation(
    rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), AiError> {
    if *rx.borrow() {
        return Ok(());
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return Ok(());
        }
    }
    Err(AiError::CancellationChannelClosed(
        "Cancellation channel dropped unexpectedly".to_string(),
    ))
}

async fn run_index_job_with_coordination<T, F>(
    session_store: &crate::document_session::DocumentSessionStore,
    session_id: &str,
    expected_generation: u64,
    job_id: &str,
    outer_deadline: tokio::time::Instant,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    job_operation: F,
) -> Result<IndexCoordinationOutcome<T>, crate::document_session::DocumentSessionError>
where
    T: Send + 'static,
    F: FnOnce(
            crate::document_session::SessionJobContext,
        ) -> Result<T, crate::document_session::SessionJobOperationError>
        + Send
        + 'static,
{
    let job_future =
        session_store.run_snapshot_job(job_id, session_id, expected_generation, job_operation);
    tokio::pin!(job_future);

    tokio::select! {
        outcome_res = &mut job_future => {
            let outcome = outcome_res?;
            let (coord_outcome, _) = normalize_coordination_outcome(outcome, None, false);
            Ok(coord_outcome)
        }
        _ = wait_for_cancellation(&mut cancel_rx) => {
            let cancel_res = session_store
                .cancel_job(
                    session_id,
                    job_id,
                    crate::document_session::SessionJobCancellationReason::UserRequested,
                )
                .await?;
            let outcome = job_future.await?;
            let (coord_outcome, _) = normalize_coordination_outcome(outcome, Some(cancel_res), false);
            Ok(coord_outcome)
        }
        _ = tokio::time::sleep_until(outer_deadline) => {
            let cancel_res = session_store
                .cancel_job(
                    session_id,
                    job_id,
                    crate::document_session::SessionJobCancellationReason::Timeout,
                )
                .await?;
            let outcome = job_future.await?;
            let (coord_outcome, _) = normalize_coordination_outcome(outcome, Some(cancel_res), true);
            Ok(coord_outcome)
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::result_large_err)]
pub(crate) async fn run_index_job_internal(
    session_store: &crate::document_session::DocumentSessionStore,
    vector_store: &VectorStore,
    endpoint_store: &AiEndpointStore,
    session_id: &str,
    expected_content_generation: u64,
    job_id: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    embed_model: &str,
    ctx: &Arc<AiEndpointRuntimeContext>,
    cancel_rx: tokio::sync::watch::Receiver<bool>,
    request_handle: &crate::ai::request_registry::ActiveRequestHandle,
    guard: &mut crate::ai::request_registry::RequestRegistryGuard,
    outer_deadline: tokio::time::Instant,
) -> Result<
    crate::document_session::SessionJobOutcome<()>,
    crate::document_session::DocumentSessionError,
> {
    let captured_fingerprint = ctx.profile.endpoint_fingerprint.clone();

    let already_built = {
        let st = vector_store.0.read();
        st.get(session_id).is_some_and(|data| {
            data.expected_content_generation == expected_content_generation
                && data.endpoint_fingerprint == captured_fingerprint
                && data.embed_model == embed_model
                && data.chunk_size == chunk_size
                && data.chunk_overlap == chunk_overlap
        })
    };

    if already_built {
        let coord_outcome = run_index_job_with_coordination(
            session_store,
            session_id,
            expected_content_generation,
            job_id,
            outer_deadline,
            cancel_rx,
            |_context| Ok(()),
        )
        .await?;

        return match coord_outcome {
            IndexCoordinationOutcome::Completed(outcome) => {
                if request_handle.is_cancel_requested() {
                    guard.finish(RequestState::Cancelled);
                    Ok(outcome.map_to_cancelled(
                        crate::document_session::SessionJobCancellationReason::UserRequested,
                    ))
                } else {
                    guard.finish(RequestState::Completed);
                    Ok(outcome)
                }
            }
            IndexCoordinationOutcome::Cancelled(outcome) => {
                guard.finish(RequestState::Cancelled);
                Ok(outcome)
            }
            IndexCoordinationOutcome::Failed(outcome)
            | IndexCoordinationOutcome::TimedOut(outcome) => {
                guard.finish(RequestState::Failed);
                Ok(outcome)
            }
        };
    }

    let ctx_clone = ctx.clone();
    let embed_model_clone = embed_model.to_string();

    let coord_outcome = run_index_job_with_coordination(
        session_store,
        session_id,
        expected_content_generation,
        job_id,
        outer_deadline,
        cancel_rx,
        move |context| {
            tauri::async_runtime::block_on(async move {
                let page_texts = extract_semantic_page_texts(&context)?;
                let total_pages = page_texts.len();
                let total_chars: usize = page_texts.iter().map(|(_, t)| t.chars().count()).sum();
                let total_bytes: usize = page_texts.iter().map(|(_, t)| t.len()).sum();

                validate_extracted_doc(total_pages, total_chars, total_bytes).map_err(|e| {
                    crate::document_session::SessionJobOperationError::failed_with_code(
                        e.code(),
                        e.message(),
                    )
                })?;

                let mut chunks = Vec::new();
                let mut canonical_dim: Option<usize> = None;

                for (page_index, text) in page_texts {
                    context.cancellation.check_cancelled()?;
                    if text.trim().is_empty() {
                        continue;
                    }

                    let chars: Vec<char> = text.chars().collect();
                    let mut start = 0;
                    while start < chars.len() {
                        context.cancellation.check_cancelled()?;
                        let end = std::cmp::min(start + chunk_size, chars.len());
                        let chunk_str: String = chars[start..end].iter().collect();

                        if chunk_str.trim().len() > 10 {
                            validate_single_chunk_bytes(chunk_str.len()).map_err(|e| {
                                crate::document_session::SessionJobOperationError::failed_with_code(
                                    e.code(),
                                    e.message(),
                                )
                            })?;

                            let payload = json!({
                                "model": embed_model_clone,
                                "prompt": chunk_str
                            });

                            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

                            let send_future = SecureHttpClient::send_post_json(
                                &ctx_clone,
                                "/api/embeddings",
                                &payload,
                                Duration::from_secs(60),
                                "application/json",
                            );

                            let response = tokio::select! {
                                res = send_future => match res {
                                    Ok(r) => r,
                                    Err(e) => {
                                        return Err(crate::document_session::SessionJobOperationError::failed_with_code(
                                            e.code(),
                                            e.message(),
                                        ));
                                    }
                                },
                                _ = context.cancellation.token().cancelled() => {
                                    return Err(crate::document_session::SessionJobOperationError::Cancelled(
                                        context.cancellation.cancellation_reason().unwrap_or(
                                            crate::document_session::SessionJobCancellationReason::UserRequested,
                                        ),
                                    ));
                                }
                            };

                            let body_future = SecureHttpClient::fetch_json_body(
                                response,
                                http_client::MAX_NON_STREAM_RESPONSE_BYTES,
                                deadline,
                            );

                            let json_val = tokio::select! {
                                res = body_future => match res {
                                    Ok(v) => v,
                                    Err(e) => {
                                        return Err(crate::document_session::SessionJobOperationError::failed_with_code(
                                            e.code(),
                                            e.message(),
                                        ));
                                    }
                                },
                                _ = context.cancellation.token().cancelled() => {
                                    return Err(crate::document_session::SessionJobOperationError::Cancelled(
                                        context.cancellation.cancellation_reason().unwrap_or(
                                            crate::document_session::SessionJobCancellationReason::UserRequested,
                                        ),
                                    ));
                                }
                            };

                            let embedding: Vec<f32> = serde_json::from_value(
                                json_val.get("embedding").cloned().unwrap_or(json!([])),
                            )
                            .map_err(|_| {
                                crate::document_session::SessionJobOperationError::failed_with_code(
                                    "AI_ENDPOINT_STREAM_PROTOCOL_ERROR",
                                    "Invalid embedding array in response",
                                )
                            })?;

                            let expected_dim = canonical_dim;
                            vector_store::validate_embedding_vector(&embedding, expected_dim).map_err(|e| {
                                crate::document_session::SessionJobOperationError::failed_with_code(
                                    e.code(),
                                    e.message(),
                                )
                            })?;

                            if canonical_dim.is_none() {
                                canonical_dim = Some(embedding.len());
                            }

                            chunks.push(ChunkData {
                                text: chunk_str,
                                page_index,
                                embedding,
                            });
                        }

                        validate_chunk_count(chunks.len()).map_err(|e| {
                            crate::document_session::SessionJobOperationError::failed_with_code(
                                e.code(),
                                e.message(),
                            )
                        })?;

                        start += chunk_size - chunk_overlap;
                    }
                }

                if chunks.is_empty() {
                    return Err(crate::document_session::SessionJobOperationError::failed_with_code(
                        "AI_INDEX_INCOMPLETE",
                        "Belgeden indekslenebilir metin parçası çıkarılamadı",
                    ));
                }

                let dim = canonical_dim.unwrap_or(0);
                Ok::<(Vec<ChunkData>, usize), crate::document_session::SessionJobOperationError>((chunks, dim))
            })
        },
    )
    .await?;

    match coord_outcome {
        IndexCoordinationOutcome::Cancelled(outcome) => {
            guard.finish(RequestState::Cancelled);
            Ok(outcome.map(|_| ()))
        }
        IndexCoordinationOutcome::Failed(outcome) | IndexCoordinationOutcome::TimedOut(outcome) => {
            guard.finish(RequestState::Failed);
            Ok(outcome.map(|_| ()))
        }
        IndexCoordinationOutcome::Completed(outcome) => {
            let (chunks, canonical_dim, job_id, session_id_out, content_gen_out, cleanup_out) =
                match outcome {
                    crate::document_session::SessionJobOutcome::Completed {
                        value: (chunks, dim),
                        job_id,
                        session_id,
                        content_generation,
                        cleanup,
                        ..
                    } => (chunks, dim, job_id, session_id, content_generation, cleanup),
                    _ => unreachable!(),
                };

            // Phase A: Outside locks validation
            if let Err(e) = vector_store.validate_staging_payload(&chunks, canonical_dim) {
                guard.finish(RequestState::Failed);
                return Ok(crate::document_session::SessionJobOutcome::Failed {
                    job_id,
                    session_id: session_id_out,
                    content_generation: content_gen_out,
                    error: crate::document_session::SessionJobFailure::new(e.code(), e.message()),
                    cleanup: cleanup_out,
                });
            }

            // Phase B: Atomic Publication Critical Section
            if request_handle.is_cancel_requested() {
                guard.finish(RequestState::Cancelled);
                return Ok(crate::document_session::SessionJobOutcome::Cancelled {
                    job_id,
                    session_id: session_id_out,
                    content_generation: content_gen_out,
                    cancellation:
                        crate::document_session::SessionJobCancellationReason::UserRequested,
                    cleanup: cleanup_out,
                });
            }

            // 1. Session Publication Guard (holds session read lease)
            let session_guard = match session_store
                .acquire_current_generation_publication_guard(
                    session_id,
                    expected_content_generation,
                )
                .await
            {
                Ok(g) => g,
                Err(e) => {
                    guard.finish(RequestState::Failed);
                    return Ok(crate::document_session::SessionJobOutcome::Failed {
                        job_id,
                        session_id: session_id_out,
                        content_generation: content_gen_out,
                        error: crate::document_session::SessionJobFailure::new(e.code, e.message),
                        cleanup: cleanup_out,
                    });
                }
            };

            // 2. Synchronous matching fingerprint guard
            let publish_result =
                endpoint_store.with_matching_fingerprint(&captured_fingerprint, || {
                    // 3. Live cancellation check
                    if request_handle.is_cancel_requested() {
                        return Err(crate::document_session::DocumentSessionError::new(
                            "AI_REQUEST_CANCELLED",
                            "İstek iptal edildi",
                        ));
                    }

                    // 4. VectorStore insert
                    vector_store
                        .insert_staging(
                            session_id,
                            expected_content_generation,
                            &captured_fingerprint,
                            embed_model,
                            chunk_size,
                            chunk_overlap,
                            canonical_dim,
                            chunks,
                        )
                        .map_err(|e| {
                            crate::document_session::DocumentSessionError::new(
                                e.code(),
                                e.message(),
                            )
                        })?;

                    Ok(())
                });

            drop(session_guard);

            match publish_result {
                Ok(()) => {
                    guard.finish(RequestState::Completed);
                    Ok(crate::document_session::SessionJobOutcome::Completed {
                        job_id,
                        session_id: session_id_out,
                        content_generation: content_gen_out,
                        value: (),
                        cancel_requested: false,
                        cancellation_reason: None,
                        cleanup: cleanup_out,
                    })
                }
                Err(e) => {
                    if e.code == "AI_REQUEST_CANCELLED" {
                        guard.finish(RequestState::Cancelled);
                        Ok(crate::document_session::SessionJobOutcome::Cancelled {
                            job_id,
                            session_id: session_id_out,
                            content_generation: content_gen_out,
                            cancellation:
                                crate::document_session::SessionJobCancellationReason::UserRequested,
                            cleanup: cleanup_out,
                        })
                    } else {
                        guard.finish(RequestState::Failed);
                        Ok(crate::document_session::SessionJobOutcome::Failed {
                            job_id,
                            session_id: session_id_out,
                            content_generation: content_gen_out,
                            error: crate::document_session::SessionJobFailure::new(
                                e.code, e.message,
                            ),
                            cleanup: cleanup_out,
                        })
                    }
                }
            }
        }
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn build_semantic_index(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    chunk_size: usize,
    chunk_overlap: usize,
    embed_model: String,
    request_id: String,
    session_store: State<'_, crate::document_session::DocumentSessionStore>,
    vector_store: State<'_, VectorStore>,
    endpoint_store: State<'_, AiEndpointStore>,
    request_registry: State<'_, AiRequestRegistry>,
) -> Result<
    crate::document_session::SessionJobOutcome<()>,
    crate::document_session::DocumentSessionError,
> {
    let outer_deadline = tokio::time::Instant::now() + Duration::from_secs(300);

    validate_chunk_params(chunk_size, chunk_overlap)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_model_name(&embed_model)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_request_id(&request_id)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let req_owner = RequestOwner::SessionNative {
        session_id: session_id.clone(),
        expected_content_generation,
    };
    let (request_handle, mut guard, cancel_rx) = request_registry
        .register_request(
            request_id.clone(),
            "index".to_string(),
            req_owner,
            CancellationCapability::SessionJob,
        )
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let ctx = endpoint_store.get_active_context().map_err(|e| {
        guard.finish(RequestState::Failed);
        crate::document_session::DocumentSessionError::new(e.code(), e.message())
    })?;

    if !is_authorized(
        &ctx.profile.mode,
        Some(&ctx.consent),
        &ctx.profile.endpoint_fingerprint,
    ) {
        guard.finish(RequestState::Failed);
        return Err(crate::document_session::DocumentSessionError::new(
            "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
            "Uzak AI endpoint kullanımı için onay gereklidir",
        ));
    }

    run_index_job_internal(
        &session_store,
        &vector_store,
        &endpoint_store,
        &session_id,
        expected_content_generation,
        &job_id,
        chunk_size,
        chunk_overlap,
        &embed_model,
        &ctx,
        cancel_rx,
        &request_handle,
        &mut guard,
        outer_deadline,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn semantic_search_internal(
    session_id: &str,
    expected_content_generation: u64,
    query: &str,
    top_k: usize,
    embed_model: &str,
    expected_chunk_size: usize,
    expected_chunk_overlap: usize,
    ctx: &Arc<AiEndpointRuntimeContext>,
    mut cancel_rx: Option<&mut tokio::sync::watch::Receiver<bool>>,
    deadline: tokio::time::Instant,
    vector_store: &VectorStore,
) -> Result<Vec<SearchResultChunk>, AiError> {
    let payload = json!({
        "model": embed_model,
        "prompt": query
    });

    let send_future = SecureHttpClient::send_post_json(
        ctx,
        "/api/embeddings",
        &payload,
        Duration::from_secs(60),
        "application/json",
    );

    let response = if let Some(ref mut rx) = cancel_rx {
        tokio::select! {
            res = send_future => res?,
            cancel_res = wait_for_cancellation(rx) => {
                match cancel_res {
                    Ok(_) => return Err(AiError::RequestCancelled("Semantic search cancelled".to_string())),
                    Err(e) => return Err(e),
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(AiError::EndpointTotalTimeout("Total request timeout reached".to_string()));
            }
        }
    } else {
        tokio::select! {
            res = send_future => res?,
            _ = tokio::time::sleep_until(deadline) => {
                return Err(AiError::EndpointTotalTimeout("Total request timeout reached".to_string()));
            }
        }
    };

    let body_future = SecureHttpClient::fetch_json_body(
        response,
        http_client::MAX_NON_STREAM_RESPONSE_BYTES,
        deadline,
    );

    let json_val = if let Some(ref mut rx) = cancel_rx {
        tokio::select! {
            res = body_future => res?,
            cancel_res = wait_for_cancellation(rx) => {
                match cancel_res {
                    Ok(_) => return Err(AiError::RequestCancelled("Semantic search cancelled".to_string())),
                    Err(e) => return Err(e),
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(AiError::EndpointTotalTimeout("Total request timeout reached".to_string()));
            }
        }
    } else {
        tokio::select! {
            res = body_future => res?,
            _ = tokio::time::sleep_until(deadline) => {
                return Err(AiError::EndpointTotalTimeout("Total request timeout reached".to_string()));
            }
        }
    };

    let query_embed: Vec<f32> = serde_json::from_value(
        json_val.get("embedding").cloned().unwrap_or(json!([])),
    )
    .map_err(|_| {
        AiError::EndpointStreamProtocolError("Invalid embedding in query response".to_string())
    })?;

    let st = vector_store.0.read();
    let data = st
        .get(session_id)
        .ok_or_else(|| AiError::IndexNotFound("Belge henüz indekslenmedi".to_string()))?;

    if !data.matches_expected_configuration(
        expected_content_generation,
        &ctx.profile.endpoint_fingerprint,
        embed_model,
        expected_chunk_size,
        expected_chunk_overlap,
    ) {
        // Read-only validation: never prune or mutate vector store on query mismatch (M1–M4)
        return Err(AiError::IndexOutdated(
            "Belge indeksi veya endpoint/model yapılandırması güncel değil".to_string(),
        ));
    }

    vector_store::validate_embedding_vector(&query_embed, Some(data.canonical_dimension))?;

    let mut results: Vec<SearchResultChunk> = Vec::new();
    for c in &data.chunks {
        let score = vector_store::cosine_similarity(&query_embed, &c.embedding)?;
        results.push(SearchResultChunk {
            text: c.text.clone(),
            page_index: c.page_index,
            score,
        });
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_k);

    Ok(results)
}

// Clippy argument count suppression justified by public Tauri IPC boundary contract.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn semantic_search(
    session_id: String,
    expected_content_generation: u64,
    query: String,
    top_k: usize,
    embed_model: String,
    expected_chunk_size: usize,
    expected_chunk_overlap: usize,
    request_id: String,
    vector_store: State<'_, VectorStore>,
    endpoint_store: State<'_, AiEndpointStore>,
    request_registry: State<'_, AiRequestRegistry>,
) -> Result<Vec<SearchResultChunk>, crate::document_session::DocumentSessionError> {
    validate_query(&query, top_k)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_model_name(&embed_model)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_chunk_params(expected_chunk_size, expected_chunk_overlap)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_request_id(&request_id)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let req_owner = RequestOwner::SessionNative {
        session_id: session_id.clone(),
        expected_content_generation,
    };
    let (_handle, mut guard, mut cancel_rx) = request_registry
        .register_request(
            request_id.clone(),
            "search".to_string(),
            req_owner,
            CancellationCapability::StreamingWatch,
        )
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let ctx = endpoint_store.get_active_context().map_err(|e| {
        guard.finish(RequestState::Failed);
        crate::document_session::DocumentSessionError::new(e.code(), e.message())
    })?;

    if !is_authorized(
        &ctx.profile.mode,
        Some(&ctx.consent),
        &ctx.profile.endpoint_fingerprint,
    ) {
        guard.finish(RequestState::Failed);
        return Err(crate::document_session::DocumentSessionError::new(
            "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
            "Uzak AI endpoint kullanımı için onay gereklidir",
        ));
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    let search_res = semantic_search_internal(
        &session_id,
        expected_content_generation,
        &query,
        top_k,
        &embed_model,
        expected_chunk_size,
        expected_chunk_overlap,
        &ctx,
        Some(&mut cancel_rx),
        deadline,
        &vector_store,
    )
    .await;

    match search_res {
        Ok(results) => {
            guard.finish(RequestState::Completed);
            Ok(results)
        }
        Err(e) => {
            if matches!(e, AiError::RequestCancelled(_)) {
                guard.finish(RequestState::Cancelled);
            } else {
                guard.finish(RequestState::Failed);
            }
            Err(crate::document_session::DocumentSessionError::new(
                e.code(),
                e.message(),
            ))
        }
    }
}

pub(crate) async fn consume_ai_ndjson_stream<R, F>(
    response: reqwest::Response,
    mut parser: NdjsonStreamParser,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    deadline: tokio::time::Instant,
    app_handle: &tauri::AppHandle<R>,
    mut guard: crate::ai::request_registry::RequestRegistryGuard,
    extract_fn: F,
) -> Result<(), AiError>
where
    R: tauri::Runtime,
    F: Fn(&serde_json::Value) -> (Option<String>, bool),
{
    if let Err(e) = parser.emit_started(app_handle) {
        guard.finish(RequestState::Failed);
        return Err(e);
    }

    let mut bytes_stream = response.bytes_stream();
    let mut done_received = false;

    loop {
        let now = tokio::time::Instant::now();
        let read_idle = Duration::from_secs(30).min(deadline.saturating_duration_since(now));

        tokio::select! {
            chunk_res = tokio::time::timeout(read_idle, bytes_stream.next()) => {
                match chunk_res {
                    Ok(Some(Ok(chunk))) => {
                        let parse_result = parser.push_chunk(app_handle, &chunk, &extract_fn);
                        match parse_result {
                            Ok(is_done) => {
                                if is_done {
                                    done_received = true;
                                    break;
                                }
                            }
                            Err(e) => {
                                if let Err(emit_err) = parser.emit_failed(app_handle, &e) {
                                    tracing::warn!("Failed to emit failed event on push_chunk error: {:?}", emit_err);
                                }
                                guard.finish(RequestState::Failed);
                                return Err(e);
                            }
                        }
                    }
                    Ok(Some(Err(e))) => {
                        let err = AiError::EndpointStreamProtocolError(e.to_string());
                        if let Err(emit_err) = parser.emit_failed(app_handle, &err) {
                            tracing::warn!("Failed to emit failed event on byte stream error: {:?}", emit_err);
                        }
                        guard.finish(RequestState::Failed);
                        return Err(err);
                    }
                    Ok(None) => {
                        // EOF reached
                        break;
                    }
                    Err(_) => {
                        let now_check = tokio::time::Instant::now();
                        let err = if now_check >= deadline {
                            AiError::EndpointTotalTimeout("Total request timeout reached".to_string())
                        } else {
                            AiError::EndpointReadTimeout("Stream read-idle timeout 30s reached".to_string())
                        };
                        if let Err(emit_err) = parser.emit_failed(app_handle, &err) {
                            tracing::warn!("Failed to emit failed event on stream idle timeout: {:?}", emit_err);
                        }
                        guard.finish(RequestState::Failed);
                        return Err(err);
                    }
                }
            }
            cancel_res = wait_for_cancellation(&mut cancel_rx) => {
                match cancel_res {
                    Ok(_) => {
                        if let Err(emit_err) = parser.emit_cancelled(app_handle) {
                            tracing::warn!("Failed to emit cancelled event on stream cancel: {:?}", emit_err);
                        }
                        guard.finish(RequestState::Cancelled);
                        return Err(AiError::RequestCancelled("Stream cancelled".to_string()));
                    }
                    Err(err) => {
                        if let Err(emit_err) = parser.emit_failed(app_handle, &err) {
                            tracing::warn!("Failed to emit failed event on stream cancel error: {:?}", emit_err);
                        }
                        guard.finish(RequestState::Failed);
                        return Err(err);
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let err = AiError::EndpointTotalTimeout("Total request timeout reached".to_string());
                if let Err(emit_err) = parser.emit_failed(app_handle, &err) {
                    tracing::warn!("Failed to emit failed event on stream deadline: {:?}", emit_err);
                }
                guard.finish(RequestState::Failed);
                return Err(err);
            }
        }
    }

    if !done_received {
        let err = AiError::StreamUnexpectedEof(
            "Unexpected stream EOF before protocol completion".to_string(),
        );
        if let Err(emit_err) = parser.emit_failed(app_handle, &err) {
            tracing::warn!(
                "Failed to emit failed event on stream unexpected EOF: {:?}",
                emit_err
            );
        }
        guard.finish(RequestState::Failed);
        return Err(err);
    }

    guard.finish(RequestState::Completed);
    Ok(())
}

// Clippy argument count suppression justified by public Tauri IPC boundary contract.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn query_document_rag(
    app_handle: AppHandle,
    session_id: String,
    expected_content_generation: u64,
    prompt: String,
    history: Vec<ChatMessage>,
    llm_model: String,
    embed_model: String,
    expected_chunk_size: usize,
    expected_chunk_overlap: usize,
    request_id: String,
    vector_store: State<'_, VectorStore>,
    endpoint_store: State<'_, AiEndpointStore>,
    request_registry: State<'_, AiRequestRegistry>,
) -> Result<(), crate::document_session::DocumentSessionError> {
    validate_model_name(&llm_model)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_model_name(&embed_model)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_chunk_params(expected_chunk_size, expected_chunk_overlap)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;
    validate_request_id(&request_id)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let chat_inputs: Vec<ChatMessageInput> = history
        .iter()
        .map(|h| ChatMessageInput {
            role: h.role.clone(),
            content: h.content.clone(),
        })
        .collect();

    validate_chat_input(&prompt, &chat_inputs)
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let req_owner = RequestOwner::SessionNative {
        session_id: session_id.clone(),
        expected_content_generation,
    };

    let (_handle, mut guard, mut cancel_rx) = request_registry
        .register_request(
            request_id.clone(),
            "chat".to_string(),
            req_owner,
            CancellationCapability::StreamingWatch,
        )
        .map_err(|e| crate::document_session::DocumentSessionError::new(e.code(), e.message()))?;

    let ctx = endpoint_store.get_active_context().map_err(|e| {
        guard.finish(RequestState::Failed);
        crate::document_session::DocumentSessionError::new(e.code(), e.message())
    })?;

    if !is_authorized(
        &ctx.profile.mode,
        Some(&ctx.consent),
        &ctx.profile.endpoint_fingerprint,
    ) {
        guard.finish(RequestState::Failed);
        return Err(crate::document_session::DocumentSessionError::new(
            "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
            "Uzak AI endpoint kullanımı için onay gereklidir",
        ));
    }

    let parser = NdjsonStreamParser::new(
        request_id.clone(),
        "chat".to_string(),
        Some(session_id.clone()),
        Some(expected_content_generation),
        ctx.profile.endpoint_fingerprint.clone(),
    );

    let search_deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    let top_chunks = match semantic_search_internal(
        &session_id,
        expected_content_generation,
        &prompt,
        3,
        &embed_model,
        expected_chunk_size,
        expected_chunk_overlap,
        &ctx,
        Some(&mut cancel_rx),
        search_deadline,
        &vector_store,
    )
    .await
    {
        Ok(chunks) => chunks,
        Err(e) => {
            let mut p = parser;
            if let Err(emit_err) = p.emit_failed(&app_handle, &e) {
                tracing::warn!(
                    "Failed to emit failed event on search error: {:?}",
                    emit_err
                );
            }
            if matches!(e, AiError::RequestCancelled(_)) {
                guard.finish(RequestState::Cancelled);
                return Ok(());
            }
            guard.finish(RequestState::Failed);
            return Err(crate::document_session::DocumentSessionError::new(
                e.code(),
                e.message(),
            ));
        }
    };

    let mut context_text = String::new();
    for (i, c) in top_chunks.iter().enumerate() {
        context_text.push_str(&format!(
            "--- Parça {} (Sayfa {}) ---\n{}\n\n",
            i + 1,
            c.page_index + 1,
            c.text
        ));
    }

    let system_prompt = format!(
        "Sen akıllı bir belge asistanısın. Kullanıcının sorusuna yalnızca aşağıdaki BELGE BAĞLAMI'nı kullanarak cevap ver. \n\nBELGE BAĞLAMI:\n{}",
        context_text
    );

    let mut messages = Vec::new();
    messages.push(json!({"role": "system", "content": system_prompt}));
    for h in history {
        messages.push(json!({"role": h.role, "content": h.content}));
    }
    messages.push(json!({"role": "user", "content": prompt}));

    let payload = json!({
        "model": llm_model,
        "messages": messages,
        "stream": true
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);

    let send_future = SecureHttpClient::send_post_json(
        &ctx,
        "/api/chat",
        &payload,
        Duration::from_secs(120),
        "application/x-ndjson",
    );

    let mut parser_mut = parser;

    let response = tokio::select! {
        res = send_future => match res {
            Ok(r) => r,
            Err(e) => {
                if let Err(emit_err) = parser_mut.emit_failed(&app_handle, &e) {
                    tracing::warn!("Failed to emit failed event on chat send error: {:?}", emit_err);
                }
                guard.finish(RequestState::Failed);
                return Err(crate::document_session::DocumentSessionError::new(e.code(), e.message()));
            }
        },
        cancel_res = wait_for_cancellation(&mut cancel_rx) => {
            match cancel_res {
                Ok(_) => {
                    if let Err(emit_err) = parser_mut.emit_cancelled(&app_handle) {
                        tracing::warn!("Failed to emit cancelled event on chat send cancel: {:?}", emit_err);
                    }
                    guard.finish(RequestState::Cancelled);
                    return Ok(());
                }
                Err(err) => {
                    if let Err(emit_err) = parser_mut.emit_failed(&app_handle, &err) {
                        tracing::warn!("Failed to emit failed event on chat send cancel error: {:?}", emit_err);
                    }
                    guard.finish(RequestState::Failed);
                    return Err(crate::document_session::DocumentSessionError::new(
                        err.code(),
                        err.message(),
                    ));
                }
            }
        }
        _ = tokio::time::sleep_until(deadline) => {
            let err = AiError::EndpointTotalTimeout("Total request timeout reached".to_string());
            if let Err(emit_err) = parser_mut.emit_failed(&app_handle, &err) {
                tracing::warn!("Failed to emit failed event on chat total timeout: {:?}", emit_err);
            }
            guard.finish(RequestState::Failed);
            return Err(crate::document_session::DocumentSessionError::new(err.code(), err.message()));
        }
    };

    let stream_res = consume_ai_ndjson_stream(
        response,
        parser_mut,
        cancel_rx,
        deadline,
        &app_handle,
        guard,
        |json_val| {
            let delta = json_val
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            let done = json_val
                .get("done")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            (delta, done)
        },
    )
    .await;

    match stream_res {
        Ok(()) => Ok(()),
        Err(AiError::RequestCancelled(_)) => Ok(()),
        Err(e) => Err(crate::document_session::DocumentSessionError::new(
            e.code(),
            e.message(),
        )),
    }
}

// Clippy argument count suppression justified by public Tauri IPC boundary contract.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn summarize_selection_or_page(
    app_handle: AppHandle,
    text_content: String,
    target_length: String,
    llm_model: String,
    request_id: String,
    caller_attempt_id: String,
    endpoint_store: State<'_, AiEndpointStore>,
    request_registry: State<'_, AiRequestRegistry>,
) -> Result<(), AiErrorWire> {
    validate_summary_input(&text_content, &target_length).map_err(|e| e.to_wire())?;
    validate_model_name(&llm_model).map_err(|e| e.to_wire())?;
    validate_request_id(&request_id).map_err(|e| e.to_wire())?;
    validate_caller_attempt_id(&caller_attempt_id).map_err(|e| e.to_wire())?;

    let req_owner = RequestOwner::SelectionScoped { caller_attempt_id };

    let (_handle, mut guard, mut cancel_rx) = request_registry
        .register_request(
            request_id.clone(),
            "summary".to_string(),
            req_owner,
            CancellationCapability::StreamingWatch,
        )
        .map_err(|e| e.to_wire())?;

    let ctx = endpoint_store.get_active_context().map_err(|e| {
        guard.finish(RequestState::Failed);
        e.to_wire()
    })?;

    if !is_authorized(
        &ctx.profile.mode,
        Some(&ctx.consent),
        &ctx.profile.endpoint_fingerprint,
    ) {
        guard.finish(RequestState::Failed);
        return Err(
            AiError::EndpointRemoteConsentRequired(ctx.profile.canonical_origin.clone()).to_wire(),
        );
    }

    let mut parser = NdjsonStreamParser::new(
        request_id.clone(),
        "summary".to_string(),
        None,
        None,
        ctx.profile.endpoint_fingerprint.clone(),
    );

    let prompt = format!(
        "Aşağıdaki metni {} olarak özetle veya açıkla:\n\n{}",
        if target_length == "short" {
            "kısa ve öz"
        } else if target_length == "bullet" {
            "maddeler halinde"
        } else {
            "detaylı"
        },
        text_content
    );

    let payload = json!({
        "model": llm_model,
        "prompt": prompt,
        "stream": true
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

    let send_future = SecureHttpClient::send_post_json(
        &ctx,
        "/api/generate",
        &payload,
        Duration::from_secs(60),
        "application/x-ndjson",
    );

    let response = tokio::select! {
        res = send_future => match res {
            Ok(r) => r,
            Err(e) => {
                if let Err(emit_err) = parser.emit_failed(&app_handle, &e) {
                    tracing::warn!("Failed to emit failed event on summary send error: {:?}", emit_err);
                }
                guard.finish(RequestState::Failed);
                return Err(e.to_wire());
            }
        },
        cancel_res = wait_for_cancellation(&mut cancel_rx) => {
            match cancel_res {
                Ok(_) => {
                    if let Err(emit_err) = parser.emit_cancelled(&app_handle) {
                        tracing::warn!("Failed to emit cancelled event on summary send cancel: {:?}", emit_err);
                    }
                    guard.finish(RequestState::Cancelled);
                    return Ok(());
                }
                Err(err) => {
                    if let Err(emit_err) = parser.emit_failed(&app_handle, &err) {
                        tracing::warn!("Failed to emit failed event on summary send cancel error: {:?}", emit_err);
                    }
                    guard.finish(RequestState::Failed);
                    return Err(err.to_wire());
                }
            }
        }
        _ = tokio::time::sleep_until(deadline) => {
            let err = AiError::EndpointTotalTimeout("Total request timeout reached".to_string());
            if let Err(emit_err) = parser.emit_failed(&app_handle, &err) {
                tracing::warn!("Failed to emit failed event on summary total timeout: {:?}", emit_err);
            }
            guard.finish(RequestState::Failed);
            return Err(err.to_wire());
        }
    };

    let stream_res = consume_ai_ndjson_stream(
        response,
        parser,
        cancel_rx,
        deadline,
        &app_handle,
        guard,
        |json_val| {
            let delta = json_val
                .get("response")
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            let done = json_val
                .get("done")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            (delta, done)
        },
    )
    .await;

    match stream_res {
        Ok(()) => Ok(()),
        Err(AiError::RequestCancelled(_)) => Ok(()),
        Err(e) => Err(e.to_wire()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = vector_store::cosine_similarity(&a, &b).unwrap();
        assert!((sim - 1.0).abs() < f32::EPSILON);
    }
}
