use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
// Removed std::sync::Mutex
use crate::document_session;
use futures_util::StreamExt;
use reqwest::Client;
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChunkData {
    pub text: String,
    pub page_index: u32,
    pub embedding: Vec<f32>,
}

pub struct VectorStore(pub parking_lot::RwLock<HashMap<String, (u64, Vec<ChunkData>)>>);

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

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (va, vb) in a.iter().zip(b.iter()) {
        dot += va * vb;
        norm_a += va * va;
        norm_b += vb * vb;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

#[derive(Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

async fn get_embedding(
    client: &Client,
    url: &str,
    model: &str,
    text: &str,
) -> Result<Vec<f32>, String> {
    let endpoint = format!("{}/api/embeddings", url.trim_end_matches('/'));
    let res = client
        .post(&endpoint)
        .json(&json!({
            "model": model,
            "prompt": text
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json_data: OllamaEmbeddingResponse = res.json().await.map_err(|e| e.to_string())?;
    Ok(json_data.embedding)
}

pub(crate) fn extract_semantic_page_texts(
    context: &crate::document_session::SessionJobContext,
) -> Result<Vec<(u32, String)>, crate::document_session::SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    match crate::job_worker::run_job(
        context,
        crate::job_worker::JobCommand::ExtractSemanticText,
        std::time::Duration::from_secs(180),
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
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn build_semantic_index(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    chunk_size: usize,
    chunk_overlap: usize,
    ollama_url: String,
    embed_model: String,
    session_store: State<'_, crate::document_session::DocumentSessionStore>,
    vector_store: State<'_, VectorStore>,
) -> Result<
    crate::document_session::SessionJobOutcome<()>,
    crate::document_session::DocumentSessionError,
> {
    let already_built = {
        let st = vector_store.0.read();
        st.get(&session_id)
            .is_some_and(|(generation, _)| *generation == expected_content_generation)
    };
    if already_built {
        return session_store
            .run_snapshot_job(
                &job_id,
                &session_id,
                expected_content_generation,
                move |_context| Ok(()),
            )
            .await;
    }

    let ollama_url_clone = ollama_url.clone();
    let embed_model_clone = embed_model.clone();

    let outcome = session_store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                tauri::async_runtime::block_on(async move {
                    let page_texts = extract_semantic_page_texts(&context)?;

                    let mut chunks = Vec::new();
                    let client = Client::new();

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
                                if let Ok(embedding) = get_embedding(
                                    &client,
                                    &ollama_url_clone,
                                    &embed_model_clone,
                                    &chunk_str,
                                )
                                .await
                                {
                                    chunks.push(ChunkData {
                                        text: chunk_str,
                                        page_index,
                                        embedding,
                                    });
                                }
                            }
                            start += chunk_size - chunk_overlap;
                        }
                    }
                    Ok::<Vec<ChunkData>, crate::document_session::SessionJobOperationError>(chunks)
                })
            },
        )
        .await?;
    Ok(outcome.map(|chunks| {
        let mut st = vector_store.0.write();
        st.insert(session_id, (expected_content_generation, chunks));
    }))
}

#[tauri::command]
pub async fn semantic_search(
    session_id: String,
    expected_content_generation: u64,
    query: String,
    top_k: usize,
    ollama_url: String,
    embed_model: String,
    vector_store: State<'_, VectorStore>,
) -> Result<Vec<SearchResultChunk>, document_session::DocumentSessionError> {
    let client = Client::new();
    let query_embed = get_embedding(&client, &ollama_url, &embed_model, &query)
        .await
        .map_err(|e| document_session::DocumentSessionError::new("SEMANTIC_SEARCH_ERROR", e))?;

    let st = vector_store.0.read();
    let (gen, chunks) = st.get(&session_id).ok_or_else(|| {
        document_session::DocumentSessionError::new("INDEX_NOT_FOUND", "Belge henüz indekslenmedi")
    })?;

    if *gen != expected_content_generation {
        return Err(document_session::DocumentSessionError::new(
            "INDEX_OUTDATED",
            "Belge indeksi güncel değil",
        ));
    }

    let mut results: Vec<SearchResultChunk> = chunks
        .iter()
        .map(|c| {
            let score = cosine_similarity(&query_embed, &c.embedding);
            SearchResultChunk {
                text: c.text.clone(),
                page_index: c.page_index,
                score,
            }
        })
        .collect();

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Top K
    results.truncate(top_k);

    Ok(results)
}

#[derive(Deserialize)]
struct OllamaChatStreamResponse {
    message: Option<ChatMessage>,
    done: bool,
}

#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn query_document_rag(
    app_handle: AppHandle,
    session_id: String,
    expected_content_generation: u64,
    prompt: String,
    history: Vec<ChatMessage>,
    ollama_url: String,
    llm_model: String,
    embed_model: String,
    vector_store: State<'_, VectorStore>,
) -> Result<(), document_session::DocumentSessionError> {
    // 1. Semantik arama ile bağlam (context) bul
    let top_chunks = semantic_search(
        session_id,
        expected_content_generation,
        prompt.clone(),
        3,
        ollama_url.clone(),
        embed_model,
        vector_store,
    )
    .await?;
    let mut context_text = String::new();
    for (i, c) in top_chunks.iter().enumerate() {
        context_text.push_str(&format!(
            "--- Parça {} (Sayfa {}) ---\n{}\n\n",
            i + 1,
            c.page_index + 1,
            c.text
        ));
    }

    // 2. Sistem yönergesini hazırla
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

    // 3. Ollama Chat API'ye gönder (Streaming)
    let client = Client::new();
    let endpoint = format!("{}/api/chat", ollama_url.trim_end_matches('/'));

    let mut res = client
        .post(&endpoint)
        .json(&json!({
            "model": llm_model,
            "messages": messages,
            "stream": true
        }))
        .send()
        .await
        .map_err(|e| {
            document_session::DocumentSessionError::new("RAG_REQUEST_FAILED", e.to_string())
        })?
        .bytes_stream();

    while let Some(chunk_result) = res.next().await {
        let chunk = chunk_result.map_err(|e| {
            document_session::DocumentSessionError::new("RAG_STREAM_ERROR", e.to_string())
        })?;
        // Gelen veri birden fazla JSON objesi içerebilir (newline ayrılmış)
        let s = String::from_utf8_lossy(&chunk);
        for line in s.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(json_data) = serde_json::from_str::<OllamaChatStreamResponse>(line) {
                if let Some(msg) = json_data.message {
                    // Svelte'ye token'ı gönder
                    let _ = app_handle.emit("ai_chat_token", msg.content);
                }
                if json_data.done {
                    let _ = app_handle.emit("ai_chat_done", "");
                }
            }
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct OllamaGenerateStreamResponse {
    response: String,
    done: bool,
}

#[tauri::command]
pub async fn summarize_selection_or_page(
    app_handle: AppHandle,
    text_content: String,
    target_length: String,
    ollama_url: String,
    llm_model: String,
) -> Result<(), String> {
    let prompt = format!(
        "Aşağıdaki metni {} olarak özetle veya açıkla:\n\n{}",
        if target_length == "short" {
            "kısa ve öz"
        } else {
            "detaylı"
        },
        text_content
    );

    let client = Client::new();
    let endpoint = format!("{}/api/generate", ollama_url.trim_end_matches('/'));

    let mut res = client
        .post(&endpoint)
        .json(&json!({
            "model": llm_model,
            "prompt": prompt,
            "stream": true
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes_stream();

    while let Some(chunk_result) = res.next().await {
        let chunk = chunk_result.map_err(|e| e.to_string())?;
        let s = String::from_utf8_lossy(&chunk);
        for line in s.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(json_data) = serde_json::from_str::<OllamaGenerateStreamResponse>(line) {
                let _ = app_handle.emit("ai_summary_token", json_data.response);
                if json_data.done {
                    let _ = app_handle.emit("ai_summary_done", "");
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < f32::EPSILON);

        let c = vec![0.0, 1.0, 0.0];
        let sim2 = cosine_similarity(&a, &c);
        assert!((sim2 - 0.0).abs() < f32::EPSILON);

        let d = vec![-1.0, 0.0, 0.0];
        let sim3 = cosine_similarity(&a, &d);
        assert!((sim3 - (-1.0)).abs() < f32::EPSILON);
    }
}
