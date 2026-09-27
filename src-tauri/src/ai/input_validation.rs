use crate::ai::error::AiError;
use uuid::Uuid;

pub const MAX_MODEL_NAME_CHARS: usize = 128;
pub const MAX_MODEL_NAME_BYTES: usize = 128;
pub const MAX_QUERY_CHARS: usize = 1_000;
pub const MAX_QUERY_BYTES: usize = 4_096;
pub const MAX_PROMPT_CHARS: usize = 10_000;
pub const MAX_PROMPT_BYTES: usize = 32_768;
pub const MAX_HISTORY_MESSAGES: usize = 50;
pub const MAX_SINGLE_HISTORY_MSG_BYTES: usize = 32_768;
pub const MAX_TOTAL_HISTORY_BYTES: usize = 102_400; // 100 KB
pub const MAX_SELECTION_CHARS: usize = 50_000;
pub const MAX_SELECTION_BYTES: usize = 200_000;
pub const MAX_JOB_CHUNKS: usize = 10_000;
pub const MAX_SINGLE_CHUNK_BYTES: usize = 16_384; // 4 * MAX_CHUNK_SIZE
pub const MAX_EXTRACTED_DOC_CHARS: usize = 5_000_000;
pub const MAX_EXTRACTED_DOC_BYTES: usize = 20_000_000;
pub const MAX_JOB_PAGES: usize = 1_000;
pub const MAX_CHUNK_SIZE: usize = 4_096; // Unicode chars
pub const MAX_CHUNK_OVERLAP: usize = 2_048; // Unicode chars
pub const MAX_TOP_K: usize = 20;

pub fn validate_model_name(name: &str) -> Result<(), AiError> {
    if name.is_empty() {
        return Err(AiError::InvalidModel(
            "Model name cannot be empty".to_string(),
        ));
    }
    if name.chars().count() > MAX_MODEL_NAME_CHARS || name.len() > MAX_MODEL_NAME_BYTES {
        return Err(AiError::InvalidModel(format!(
            "Model name exceeds length limit (max {} chars/bytes)",
            MAX_MODEL_NAME_CHARS
        )));
    }
    for c in name.chars() {
        if c.is_control() || c == '\n' || c == '\r' {
            return Err(AiError::InvalidModel(
                "Model name contains invalid control characters or newlines".to_string(),
            ));
        }
        if !c.is_ascii_alphanumeric() && c != '_' && c != '.' && c != ':' && c != '/' && c != '-' {
            return Err(AiError::InvalidModel(format!(
                "Model name contains forbidden character: '{}'",
                c
            )));
        }
    }
    Ok(())
}

pub fn validate_chunk_params(chunk_size: usize, chunk_overlap: usize) -> Result<(), AiError> {
    if chunk_size == 0 {
        return Err(AiError::InvalidChunkPolicy(
            "chunk_size must be > 0".to_string(),
        ));
    }
    if chunk_size > MAX_CHUNK_SIZE {
        return Err(AiError::InvalidChunkPolicy(format!(
            "chunk_size {} exceeds maximum allowed of {} characters",
            chunk_size, MAX_CHUNK_SIZE
        )));
    }
    if chunk_overlap >= chunk_size {
        return Err(AiError::InvalidChunkPolicy(
            "chunk_overlap must be strictly less than chunk_size".to_string(),
        ));
    }
    if chunk_overlap > MAX_CHUNK_OVERLAP {
        return Err(AiError::InvalidChunkPolicy(format!(
            "chunk_overlap {} exceeds maximum allowed of {} characters",
            chunk_overlap, MAX_CHUNK_OVERLAP
        )));
    }
    Ok(())
}

pub fn validate_extracted_doc(
    total_pages: usize,
    total_chars: usize,
    total_bytes: usize,
) -> Result<(), AiError> {
    if total_pages > MAX_JOB_PAGES {
        return Err(AiError::IndexLimitExceeded(format!(
            "Total pages {} exceeds limit of {}",
            total_pages, MAX_JOB_PAGES
        )));
    }
    if total_chars > MAX_EXTRACTED_DOC_CHARS {
        return Err(AiError::IndexLimitExceeded(format!(
            "Extracted document characters {} exceeds limit of {}",
            total_chars, MAX_EXTRACTED_DOC_CHARS
        )));
    }
    if total_bytes > MAX_EXTRACTED_DOC_BYTES {
        return Err(AiError::IndexLimitExceeded(format!(
            "Extracted document bytes {} exceeds limit of {}",
            total_bytes, MAX_EXTRACTED_DOC_BYTES
        )));
    }
    Ok(())
}

pub fn validate_chunk_count(chunk_count: usize) -> Result<(), AiError> {
    if chunk_count > MAX_JOB_CHUNKS {
        return Err(AiError::IndexLimitExceeded(format!(
            "Chunk count {} exceeds limit of {}",
            chunk_count, MAX_JOB_CHUNKS
        )));
    }
    Ok(())
}

pub fn validate_single_chunk_bytes(chunk_bytes: usize) -> Result<(), AiError> {
    if chunk_bytes > MAX_SINGLE_CHUNK_BYTES {
        return Err(AiError::IndexLimitExceeded(format!(
            "Single chunk size {} bytes exceeds limit of {} bytes",
            chunk_bytes, MAX_SINGLE_CHUNK_BYTES
        )));
    }
    Ok(())
}

pub fn validate_query(query: &str, top_k: usize) -> Result<(), AiError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(AiError::RequestInvalid("Query cannot be empty".to_string()));
    }
    if trimmed.chars().count() > MAX_QUERY_CHARS || trimmed.len() > MAX_QUERY_BYTES {
        return Err(AiError::RequestInvalid(format!(
            "Query length exceeds limit (max {} chars / {} bytes)",
            MAX_QUERY_CHARS, MAX_QUERY_BYTES
        )));
    }
    if top_k == 0 || top_k > MAX_TOP_K {
        return Err(AiError::RequestInvalid(format!(
            "top_k must be between 1 and {}, got {}",
            MAX_TOP_K, top_k
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ChatMessageInput {
    pub role: String,
    pub content: String,
}

pub fn validate_chat_input(prompt: &str, history: &[ChatMessageInput]) -> Result<(), AiError> {
    let p_trimmed = prompt.trim();
    if p_trimmed.is_empty() {
        return Err(AiError::InvalidPrompt("Prompt cannot be empty".to_string()));
    }
    if p_trimmed.chars().count() > MAX_PROMPT_CHARS {
        return Err(AiError::InvalidPrompt(format!(
            "Prompt exceeds char limit (max {} chars, got {})",
            MAX_PROMPT_CHARS,
            p_trimmed.chars().count()
        )));
    }
    if p_trimmed.len() > MAX_PROMPT_BYTES {
        return Err(AiError::InvalidPrompt(format!(
            "Prompt exceeds byte limit (max {} bytes, got {})",
            MAX_PROMPT_BYTES,
            p_trimmed.len()
        )));
    }

    if history.len() > MAX_HISTORY_MESSAGES {
        return Err(AiError::InvalidHistory(format!(
            "History count {} exceeds limit of {}",
            history.len(),
            MAX_HISTORY_MESSAGES
        )));
    }

    let mut total_history_bytes = 0usize;
    for (i, msg) in history.iter().enumerate() {
        let role = msg.role.trim().to_lowercase();
        if role != "user" && role != "assistant" {
            return Err(AiError::InvalidHistory(format!(
                "Invalid role '{}' at history index {}. Frontend cannot send 'system' role.",
                msg.role, i
            )));
        }
        if msg.content.len() > MAX_SINGLE_HISTORY_MSG_BYTES {
            return Err(AiError::InvalidHistory(format!(
                "History message at index {} exceeds single message limit of {} bytes",
                i, MAX_SINGLE_HISTORY_MSG_BYTES
            )));
        }
        total_history_bytes = total_history_bytes
            .checked_add(msg.content.len())
            .ok_or_else(|| AiError::InvalidHistory("History byte overflow".to_string()))?;
    }

    if total_history_bytes > MAX_TOTAL_HISTORY_BYTES {
        return Err(AiError::InvalidHistory(format!(
            "Total history size {} bytes exceeds limit of {} bytes",
            total_history_bytes, MAX_TOTAL_HISTORY_BYTES
        )));
    }

    Ok(())
}

pub fn validate_summary_input(text_content: &str, target_length: &str) -> Result<(), AiError> {
    let trimmed = text_content.trim();
    if trimmed.is_empty() {
        return Err(AiError::InvalidPrompt(
            "Summary text cannot be empty".to_string(),
        ));
    }
    if trimmed.chars().count() > MAX_SELECTION_CHARS || trimmed.len() > MAX_SELECTION_BYTES {
        return Err(AiError::InvalidPrompt(format!(
            "Selection length ({} chars / {} bytes) exceeds limit (max {} chars / {} bytes)",
            trimmed.chars().count(),
            trimmed.len(),
            MAX_SELECTION_CHARS,
            MAX_SELECTION_BYTES
        )));
    }
    let len_norm = target_length.trim().to_lowercase();
    if len_norm != "short" && len_norm != "detailed" && len_norm != "bullet" {
        return Err(AiError::RequestInvalid(format!(
            "Invalid target_length '{}'. Must be 'short', 'detailed', or 'bullet'",
            target_length
        )));
    }
    Ok(())
}

pub fn validate_request_id(id: &str) -> Result<Uuid, AiError> {
    if id.trim() != id {
        return Err(AiError::RequestInvalid(
            "Leading or trailing whitespace in requestId".to_string(),
        ));
    }
    if !id.starts_with("req_") {
        return Err(AiError::RequestInvalid(
            "Invalid requestId prefix, must start with req_".to_string(),
        ));
    }
    if id.len() != 4 + 36 {
        return Err(AiError::RequestInvalid(
            "Invalid requestId length".to_string(),
        ));
    }
    let raw_uuid = &id[4..];
    let parsed = Uuid::parse_str(raw_uuid)
        .map_err(|_| AiError::RequestInvalid("Invalid UUID format in requestId".to_string()))?;
    if parsed.get_version() != Some(uuid::Version::Random) {
        return Err(AiError::RequestInvalid(
            "requestId UUID must be version 4 (Random)".to_string(),
        ));
    }
    if parsed.get_variant() != uuid::Variant::RFC4122 {
        return Err(AiError::RequestInvalid(
            "requestId UUID must be RFC4122 variant".to_string(),
        ));
    }
    if raw_uuid != parsed.hyphenated().to_string() {
        return Err(AiError::RequestInvalid(
            "requestId UUID must be canonical lowercase hyphenated".to_string(),
        ));
    }
    Ok(parsed)
}

pub fn validate_caller_attempt_id(id: &str) -> Result<Uuid, AiError> {
    if id.trim() != id {
        return Err(AiError::RequestInvalid(
            "Leading or trailing whitespace in callerAttemptId".to_string(),
        ));
    }
    if !id.starts_with("attempt_") {
        return Err(AiError::RequestInvalid(
            "Invalid callerAttemptId prefix, must start with attempt_".to_string(),
        ));
    }
    if id.len() != 8 + 36 {
        return Err(AiError::RequestInvalid(
            "Invalid callerAttemptId length".to_string(),
        ));
    }
    let raw_uuid = &id[8..];
    let parsed = Uuid::parse_str(raw_uuid).map_err(|_| {
        AiError::RequestInvalid("Invalid UUID format in callerAttemptId".to_string())
    })?;
    if parsed.get_version() != Some(uuid::Version::Random) {
        return Err(AiError::RequestInvalid(
            "callerAttemptId UUID must be version 4 (Random)".to_string(),
        ));
    }
    if parsed.get_variant() != uuid::Variant::RFC4122 {
        return Err(AiError::RequestInvalid(
            "callerAttemptId UUID must be RFC4122 variant".to_string(),
        ));
    }
    if raw_uuid != parsed.hyphenated().to_string() {
        return Err(AiError::RequestInvalid(
            "callerAttemptId UUID must be canonical lowercase hyphenated".to_string(),
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_model_name() {
        assert!(validate_model_name("llama3.2:latest").is_ok());
        assert!(validate_model_name("nomic-embed-text:latest").is_ok());
        assert!(matches!(
            validate_model_name(""),
            Err(AiError::InvalidModel(_))
        ));
        assert!(matches!(
            validate_model_name("llama3\n"),
            Err(AiError::InvalidModel(_))
        ));
        assert!(matches!(
            validate_model_name("model$bad"),
            Err(AiError::InvalidModel(_))
        ));
    }

    #[test]
    fn test_validate_request_id() {
        let valid_uuid = Uuid::new_v4().to_string();
        let valid_id = format!("req_{}", valid_uuid);
        assert!(validate_request_id(&valid_id).is_ok());

        assert!(validate_request_id("invalid_id").is_err());
        assert!(validate_request_id("req_not-a-uuid").is_err());
    }

    #[test]
    fn test_validate_chat_input() {
        let prompt = "Explain quantum computing";
        let history = vec![
            ChatMessageInput {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
            ChatMessageInput {
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
            },
        ];
        assert!(validate_chat_input(prompt, &history).is_ok());

        // System role rejection
        let sys_history = vec![ChatMessageInput {
            role: "system".to_string(),
            content: "Be helpful".to_string(),
        }];
        assert!(matches!(
            validate_chat_input(prompt, &sys_history),
            Err(AiError::InvalidHistory(_))
        ));
    }
}
