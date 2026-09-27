use crate::ai::endpoint::AiEndpointRuntimeContext;
use crate::ai::error::AiError;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, USER_AGENT};
use reqwest::Response;
use serde_json::Value;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

pub const MAX_NON_STREAM_RESPONSE_BYTES: usize = 5 * 1024 * 1024; // 5 MB
pub const MAX_STREAM_TOTAL_BYTES: usize = 10 * 1024 * 1024; // 10 MB

pub(crate) const DEFAULT_HEADER_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const DEFAULT_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub struct HttpTimeoutPolicy {
    pub header_timeout: Duration,
    pub read_idle_timeout: Duration,
}

impl Default for HttpTimeoutPolicy {
    fn default() -> Self {
        Self {
            header_timeout: DEFAULT_HEADER_TIMEOUT,
            read_idle_timeout: DEFAULT_READ_IDLE_TIMEOUT,
        }
    }
}

pub struct SecureHttpClient;

impl SecureHttpClient {
    pub async fn send_post_json(
        ctx: &Arc<AiEndpointRuntimeContext>,
        path: &str,
        payload: &Value,
        total_timeout: Duration,
        expected_content_type: &str,
    ) -> Result<Response, AiError> {
        Self::send_post_json_with_policy(
            ctx,
            path,
            payload,
            total_timeout,
            expected_content_type,
            HttpTimeoutPolicy::default(),
        )
        .await
    }

    pub async fn send_post_json_with_policy(
        ctx: &Arc<AiEndpointRuntimeContext>,
        path: &str,
        payload: &Value,
        total_timeout: Duration,
        expected_content_type: &str,
        policy: HttpTimeoutPolicy,
    ) -> Result<Response, AiError> {
        let deadline = tokio::time::Instant::now() + total_timeout;
        let endpoint_url = format!(
            "{}{}",
            ctx.profile.canonical_origin.trim_end_matches('/'),
            path
        );

        let req = ctx
            .client
            .post(&endpoint_url)
            .header(USER_AGENT, "Antigravity-AI-Client/1.0")
            .header(ACCEPT_ENCODING, "identity")
            .header(ACCEPT, expected_content_type)
            .header(CONTENT_TYPE, "application/json")
            .json(payload);

        let send_future = ctx.client.execute(
            req.build()
                .map_err(|e| AiError::EndpointInvalid(e.to_string()))?,
        );

        let header_timeout = total_timeout.min(policy.header_timeout);
        let response = match timeout(header_timeout, send_future).await {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => {
                if e.is_connect() {
                    if e.is_timeout() {
                        return Err(AiError::EndpointConnectTimeout(
                            "Connect timeout".to_string(),
                        ));
                    }
                    return Err(AiError::EndpointConnectFailed(e.to_string()));
                }
                return Err(AiError::EndpointHeaderTimeout(e.to_string()));
            }
            Err(_) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(AiError::EndpointTotalTimeout(
                        "Total request timeout reached".to_string(),
                    ));
                }
                return Err(AiError::EndpointHeaderTimeout(format!(
                    "Header timeout {:?}",
                    policy.header_timeout
                )));
            }
        };

        let status = response.status();
        if status.is_redirection() || (300..400).contains(&status.as_u16()) {
            return Err(AiError::EndpointRedirectRejected(format!(
                "HTTP {}",
                status
            )));
        }
        if !status.is_success() {
            return Err(AiError::EndpointHttpStatus(status.as_u16()));
        }

        if let Some(enc) = response.headers().get(CONTENT_ENCODING) {
            let enc_str = enc.to_str().unwrap_or("").trim().to_lowercase();
            if !enc_str.is_empty() && enc_str != "identity" {
                return Err(AiError::EndpointDecompressionRejected(enc_str));
            }
        }

        let ct_header = response.headers().get(CONTENT_TYPE).ok_or_else(|| {
            AiError::EndpointContentTypeInvalid("Missing Content-Type header".to_string())
        })?;
        let ct_raw = ct_header.to_str().map_err(|_| {
            AiError::EndpointContentTypeInvalid("Invalid header characters".to_string())
        })?;
        let essence = ct_raw.split(';').next().unwrap_or("").trim().to_lowercase();
        let matched = if expected_content_type == "application/json" {
            essence == "application/json"
        } else if expected_content_type == "application/x-ndjson" {
            essence == "application/x-ndjson" || essence == "application/json"
        } else {
            essence == expected_content_type
        };
        if !matched {
            return Err(AiError::EndpointContentTypeInvalid(ct_raw.to_string()));
        }

        Ok(response)
    }

    pub async fn fetch_json_body(
        response: Response,
        max_bytes: usize,
        deadline: tokio::time::Instant,
    ) -> Result<Value, AiError> {
        Self::fetch_json_body_with_policy(response, max_bytes, deadline, DEFAULT_READ_IDLE_TIMEOUT)
            .await
    }

    pub async fn fetch_json_body_with_policy(
        response: Response,
        max_bytes: usize,
        deadline: tokio::time::Instant,
        read_idle_timeout: Duration,
    ) -> Result<Value, AiError> {
        let mut stream = response.bytes_stream();
        let mut body_bytes = Vec::new();

        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(AiError::EndpointTotalTimeout(
                    "Total request timeout reached".to_string(),
                ));
            }
            let read_timeout = read_idle_timeout.min(deadline.saturating_duration_since(now));

            let chunk_res = match timeout(read_timeout, stream.next()).await {
                Ok(Some(res)) => res,
                Ok(None) => break,
                Err(_) => {
                    let now_check = tokio::time::Instant::now();
                    if now_check >= deadline {
                        return Err(AiError::EndpointTotalTimeout(
                            "Total request timeout reached".to_string(),
                        ));
                    }
                    return Err(AiError::EndpointReadTimeout(format!(
                        "Read idle timeout {:?}",
                        read_idle_timeout
                    )));
                }
            };

            let chunk =
                chunk_res.map_err(|e| AiError::EndpointStreamProtocolError(e.to_string()))?;
            if body_bytes.len() + chunk.len() > max_bytes {
                return Err(AiError::EndpointResponseTooLarge(format!(
                    "Response body exceeds limit of {} bytes",
                    max_bytes
                )));
            }
            body_bytes.extend_from_slice(&chunk);
        }

        serde_json::from_slice(&body_bytes)
            .map_err(|e| AiError::EndpointStreamProtocolError(format!("Invalid JSON: {}", e)))
    }
}
