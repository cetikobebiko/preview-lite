use crate::ai::error::AiError;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_EMBEDDING_DIM: usize = 4_096;
pub const MAX_SESSION_VECTORS: usize = 50_000;
pub const MAX_SESSION_BYTES: usize = 100 * 1024 * 1024; // 100 MB
pub const MAX_GLOBAL_BYTES: usize = 500 * 1024 * 1024; // 500 MB

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChunkData {
    pub text: String,
    pub page_index: u32,
    pub embedding: Vec<f32>,
}

pub fn compute_chunk_memory_bytes(chunk: &ChunkData) -> Result<usize, AiError> {
    let text_bytes = chunk.text.capacity();
    let vector_bytes = chunk
        .embedding
        .capacity()
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or(AiError::VectorStoreLimitExceeded)?;

    let container_overhead = std::mem::size_of::<ChunkData>()
        .checked_add(128)
        .ok_or(AiError::VectorStoreLimitExceeded)?;

    text_bytes
        .checked_add(vector_bytes)
        .and_then(|sum| sum.checked_add(container_overhead))
        .ok_or(AiError::VectorStoreLimitExceeded)
}

pub fn validate_embedding_vector(
    vector: &[f32],
    expected_dim: Option<usize>,
) -> Result<(), AiError> {
    if vector.is_empty() {
        return Err(AiError::EmbeddingInvalid("Vector is empty".to_string()));
    }
    if vector.len() > MAX_EMBEDDING_DIM {
        return Err(AiError::EmbeddingInvalid(format!(
            "Vector dimension {} exceeds max {}",
            vector.len(),
            MAX_EMBEDDING_DIM
        )));
    }
    if let Some(exp) = expected_dim {
        if vector.len() != exp {
            return Err(AiError::EmbeddingDimensionMismatch(format!(
                "Expected dimension {}, got {}",
                exp,
                vector.len()
            )));
        }
    }
    for (idx, val) in vector.iter().enumerate() {
        if !val.is_finite() {
            return Err(AiError::EmbeddingInvalid(format!(
                "Vector contains non-finite value (NaN or Inf) at index {}",
                idx
            )));
        }
    }
    Ok(())
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Result<f32, AiError> {
    if a.is_empty() || b.is_empty() {
        return Err(AiError::EmbeddingInvalid("Vector is empty".to_string()));
    }
    if a.len() > MAX_EMBEDDING_DIM || b.len() > MAX_EMBEDDING_DIM {
        return Err(AiError::EmbeddingInvalid(format!(
            "Vector dimension exceeds maximum {}",
            MAX_EMBEDDING_DIM
        )));
    }
    if a.len() != b.len() {
        return Err(AiError::EmbeddingDimensionMismatch(format!(
            "Dimension mismatch: {} vs {}",
            a.len(),
            b.len()
        )));
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (idx, (&va, &vb)) in a.iter().zip(b.iter()).enumerate() {
        if !va.is_finite() || !vb.is_finite() {
            return Err(AiError::EmbeddingInvalid(format!(
                "Vector contains non-finite value (NaN or Inf) at index {}",
                idx
            )));
        }
        dot += va * vb;
        norm_a += va * va;
        norm_b += vb * vb;
    }
    if norm_a <= 0.0 || norm_b <= 0.0 {
        return Ok(0.0);
    }
    Ok(dot / (norm_a.sqrt() * norm_b.sqrt()))
}

#[derive(Clone, Debug)]
pub struct IndexedSessionData {
    pub expected_content_generation: u64,
    pub endpoint_fingerprint: String,
    pub embed_model: String,
    pub chunk_size: usize,
    pub chunk_overlap: usize,
    pub canonical_dimension: usize,
    pub chunks: Vec<ChunkData>,
    pub total_memory_bytes: usize,
}

impl IndexedSessionData {
    pub fn matches_expected_configuration(
        &self,
        expected_generation: u64,
        expected_endpoint_fingerprint: &str,
        expected_embed_model: &str,
        expected_chunk_size: usize,
        expected_chunk_overlap: usize,
    ) -> bool {
        self.expected_content_generation == expected_generation
            && self.endpoint_fingerprint == expected_endpoint_fingerprint
            && self.embed_model == expected_embed_model
            && self.chunk_size == expected_chunk_size
            && self.chunk_overlap == expected_chunk_overlap
    }
}

pub struct VectorStore(pub RwLock<HashMap<String, IndexedSessionData>>);

impl VectorStore {
    pub fn new() -> Self {
        Self(RwLock::new(HashMap::new()))
    }

    pub fn current_global_memory_bytes(&self) -> usize {
        let guard = self.0.read();
        guard.values().map(|s| s.total_memory_bytes).sum()
    }

    pub fn matches_expected_configuration(
        &self,
        session_id: &str,
        expected_generation: u64,
        expected_endpoint_fingerprint: &str,
        expected_embed_model: &str,
        expected_chunk_size: usize,
        expected_chunk_overlap: usize,
    ) -> bool {
        let guard = self.0.read();
        guard.get(session_id).is_some_and(|data| {
            data.matches_expected_configuration(
                expected_generation,
                expected_endpoint_fingerprint,
                expected_embed_model,
                expected_chunk_size,
                expected_chunk_overlap,
            )
        })
    }

    pub fn validate_staging_payload(
        &self,
        chunks: &[ChunkData],
        canonical_dimension: usize,
    ) -> Result<usize, AiError> {
        let mut staging_bytes = 0usize;
        for c in chunks {
            validate_embedding_vector(&c.embedding, Some(canonical_dimension))?;
            let cb = compute_chunk_memory_bytes(c)?;
            staging_bytes = staging_bytes
                .checked_add(cb)
                .ok_or(AiError::VectorStoreLimitExceeded)?;
        }

        if chunks.len() > MAX_SESSION_VECTORS || staging_bytes > MAX_SESSION_BYTES {
            return Err(AiError::VectorStoreLimitExceeded);
        }

        Ok(staging_bytes)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_staging(
        &self,
        session_id: &str,
        expected_generation: u64,
        endpoint_fingerprint: &str,
        embed_model: &str,
        chunk_size: usize,
        chunk_overlap: usize,
        canonical_dimension: usize,
        chunks: Vec<ChunkData>,
    ) -> Result<(), AiError> {
        let staging_bytes = self.validate_staging_payload(&chunks, canonical_dimension)?;

        let mut guard = self.0.write();
        let existing_bytes = guard
            .get(session_id)
            .map(|s| s.total_memory_bytes)
            .unwrap_or(0);

        let current_total: usize = guard.values().map(|s| s.total_memory_bytes).sum();
        let without_existing = current_total.saturating_sub(existing_bytes);

        let projected_global = without_existing
            .checked_add(staging_bytes)
            .ok_or(AiError::VectorStoreLimitExceeded)?;

        if projected_global > MAX_GLOBAL_BYTES {
            return Err(AiError::VectorStoreLimitExceeded);
        }

        let session_data = IndexedSessionData {
            expected_content_generation: expected_generation,
            endpoint_fingerprint: endpoint_fingerprint.to_string(),
            embed_model: embed_model.to_string(),
            chunk_size,
            chunk_overlap,
            canonical_dimension,
            chunks,
            total_memory_bytes: staging_bytes,
        };

        guard.insert(session_id.to_string(), session_data);
        Ok(())
    }

    pub fn remove_session(&self, session_id: &str) {
        let mut guard = self.0.write();
        guard.remove(session_id);
    }

    pub fn prune_generation(&self, session_id: &str, valid_generation: u64) {
        let mut guard = self.0.write();
        if let Some(data) = guard.get(session_id) {
            if data.expected_content_generation != valid_generation {
                guard.remove(session_id);
            }
        }
    }

    pub fn prune_endpoint(&self, session_id: &str, valid_fingerprint: &str) {
        let mut guard = self.0.write();
        if let Some(data) = guard.get(session_id) {
            if data.endpoint_fingerprint != valid_fingerprint {
                guard.remove(session_id);
            }
        }
    }

    pub fn prune_model(&self, session_id: &str, valid_model: &str) {
        let mut guard = self.0.write();
        if let Some(data) = guard.get(session_id) {
            if data.embed_model != valid_model {
                guard.remove(session_id);
            }
        }
    }

    pub fn prune_stale_data(
        &self,
        session_id: &str,
        expected_generation: u64,
        endpoint_fingerprint: &str,
        embed_model: &str,
        chunk_size: usize,
        chunk_overlap: usize,
    ) {
        let mut guard = self.0.write();
        if let Some(data) = guard.get(session_id) {
            if !data.matches_expected_configuration(
                expected_generation,
                endpoint_fingerprint,
                embed_model,
                chunk_size,
                chunk_overlap,
            ) {
                guard.remove(session_id);
            }
        }
    }

    pub fn clear_all(&self) {
        let mut guard = self.0.write();
        guard.clear();
    }
}

impl Default for VectorStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_accounting() {
        let chunk = ChunkData {
            text: "Hello world".to_string(),
            page_index: 0,
            embedding: vec![1.0, 2.0, 3.0],
        };
        let mem = compute_chunk_memory_bytes(&chunk).unwrap();
        assert!(mem > 0);
    }

    #[test]
    fn test_validate_embedding_vector() {
        let valid = vec![0.1, 0.2, 0.3];
        assert!(validate_embedding_vector(&valid, Some(3)).is_ok());
        assert!(validate_embedding_vector(&valid, Some(4)).is_err());

        let nan_vec = vec![0.1, f32::NAN, 0.3];
        assert!(matches!(
            validate_embedding_vector(&nan_vec, None),
            Err(AiError::EmbeddingInvalid(_))
        ));

        let inf_vec = vec![0.1, f32::INFINITY, 0.3];
        assert!(matches!(
            validate_embedding_vector(&inf_vec, None),
            Err(AiError::EmbeddingInvalid(_))
        ));
    }
}
