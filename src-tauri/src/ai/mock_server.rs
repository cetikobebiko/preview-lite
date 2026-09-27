use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

pub enum RawMockResponse {
    Standard {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    },
    StallHeaders {
        delay_ms: u64,
    },
    StallBody {
        headers: Vec<(&'static str, String)>,
        initial_chunk: Vec<u8>,
        delay_ms: u64,
        on_stall_entered: Option<Arc<Notify>>,
    },
    ChunkedStream {
        headers: Vec<(&'static str, String)>,
        chunks: Vec<(Vec<u8>, u64)>,
        close_abruptly: bool,
    },
}

pub struct MockHttpServer {
    pub addr: SocketAddr,
    pub request_count: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl MockHttpServer {
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(String) -> (u16, Vec<(&'static str, &'static str)>, Vec<u8>) + Send + Sync + 'static,
    {
        Self::start_raw(move |req| {
            let (status, headers, body) = handler(req);
            RawMockResponse::Standard {
                status,
                headers: headers
                    .into_iter()
                    .map(|(k, v)| (k, v.to_string()))
                    .collect(),
                body,
            }
        })
        .await
    }

    pub async fn start_raw<F>(handler: F) -> Self
    where
        F: Fn(String) -> RawMockResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let request_count_clone = request_count.clone();

        let handler = std::sync::Arc::new(handler);
        let handle = tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let handler = handler.clone();
                    let count = request_count_clone.clone();
                    tokio::spawn(async move {
                        let mut buf = [0u8; 4096];
                        if let Ok(n) = socket.read(&mut buf).await {
                            if n == 0 {
                                return;
                            }
                            count.fetch_add(1, Ordering::SeqCst);
                            let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
                            match handler(req_str) {
                                RawMockResponse::Standard {
                                    status,
                                    headers,
                                    body,
                                } => {
                                    let mut response_head = format!(
                                        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n",
                                        status,
                                        body.len()
                                    );
                                    for (k, v) in headers {
                                        response_head.push_str(&format!("{}: {}\r\n", k, v));
                                    }
                                    response_head.push_str("\r\n");
                                    let _ = socket.write_all(response_head.as_bytes()).await;
                                    let _ = socket.write_all(&body).await;
                                }
                                RawMockResponse::StallHeaders { delay_ms } => {
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                }
                                RawMockResponse::StallBody {
                                    headers,
                                    initial_chunk,
                                    delay_ms,
                                    on_stall_entered,
                                } => {
                                    let mut response_head =
                                        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n"
                                            .to_string();
                                    for (k, v) in headers {
                                        response_head.push_str(&format!("{}: {}\r\n", k, v));
                                    }
                                    response_head.push_str("\r\n");
                                    let _ = socket.write_all(response_head.as_bytes()).await;
                                    if !initial_chunk.is_empty() {
                                        let chunk_head = format!("{:x}\r\n", initial_chunk.len());
                                        let _ = socket.write_all(chunk_head.as_bytes()).await;
                                        let _ = socket.write_all(&initial_chunk).await;
                                        let _ = socket.write_all(b"\r\n").await;
                                    }
                                    if let Some(notify) = on_stall_entered {
                                        notify.notify_one();
                                    }
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                }
                                RawMockResponse::ChunkedStream {
                                    headers,
                                    chunks,
                                    close_abruptly,
                                } => {
                                    let mut response_head =
                                        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n"
                                            .to_string();
                                    for (k, v) in headers {
                                        response_head.push_str(&format!("{}: {}\r\n", k, v));
                                    }
                                    response_head.push_str("\r\n");
                                    let _ = socket.write_all(response_head.as_bytes()).await;
                                    for (chunk_bytes, delay_ms) in chunks {
                                        if delay_ms > 0 {
                                            tokio::time::sleep(Duration::from_millis(delay_ms))
                                                .await;
                                        }
                                        let chunk_head = format!("{:x}\r\n", chunk_bytes.len());
                                        if socket.write_all(chunk_head.as_bytes()).await.is_err() {
                                            break;
                                        }
                                        if socket.write_all(&chunk_bytes).await.is_err() {
                                            break;
                                        }
                                        if socket.write_all(b"\r\n").await.is_err() {
                                            break;
                                        }
                                    }
                                    if !close_abruptly {
                                        let _ = socket.write_all(b"0\r\n\r\n").await;
                                    }
                                }
                            }
                        }
                    });
                }
            }
        });

        Self {
            addr,
            request_count,
            handle,
        }
    }

    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::vector_store::{cosine_similarity, validate_embedding_vector};

    #[tokio::test]
    async fn test_mock_server_basic() {
        let server = MockHttpServer::start(|_req| {
            (
                200,
                vec![("Content-Type", "application/json")],
                b"{\"embedding\": [0.1, 0.2, 0.3]}".to_vec(),
            )
        })
        .await;

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}", server.addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(body.get("embedding").is_some());
        assert_eq!(server.request_count(), 1);
    }

    #[test]
    fn test_nan_and_inf_embedding_rejection() {
        let nan_vec = vec![1.0, f32::NAN, 0.5];
        assert!(validate_embedding_vector(&nan_vec, None).is_err());

        let inf_vec = vec![1.0, f32::INFINITY, 0.5];
        assert!(validate_embedding_vector(&inf_vec, None).is_err());

        let neg_inf = vec![1.0, f32::NEG_INFINITY, 0.5];
        assert!(validate_embedding_vector(&neg_inf, None).is_err());
    }

    #[test]
    fn test_dimension_mismatch_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert!(cosine_similarity(&a, &b).is_err());
    }
}
