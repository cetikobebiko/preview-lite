#[cfg(test)]
mod tests {
    use crate::ai::endpoint::{
        build_secure_reqwest_client, classify_ip, compute_endpoint_fingerprint, is_authorized,
        parse_and_validate_url, validate_and_classify_addresses, AiEndpointConsent,
        AiEndpointProfile, AiEndpointRuntimeContext, AiEndpointStore, EndpointMode, IpCategory,
        MAX_ENDPOINT_URL_LENGTH, MAX_HTTP2_HEADER_LIST_BYTES, SECURE_CLIENT_CONNECT_TIMEOUT,
    };
    use crate::ai::error::AiError;
    use crate::ai::http_client::{
        HttpTimeoutPolicy, SecureHttpClient, MAX_NON_STREAM_RESPONSE_BYTES,
    };
    use crate::ai::input_validation::{
        validate_caller_attempt_id, validate_chat_input, validate_chunk_count,
        validate_chunk_params, validate_extracted_doc, validate_model_name, validate_query,
        validate_request_id, validate_single_chunk_bytes, validate_summary_input, ChatMessageInput,
        MAX_CHUNK_OVERLAP, MAX_CHUNK_SIZE, MAX_EXTRACTED_DOC_BYTES, MAX_EXTRACTED_DOC_CHARS,
        MAX_HISTORY_MESSAGES, MAX_JOB_CHUNKS, MAX_JOB_PAGES, MAX_MODEL_NAME_CHARS,
        MAX_PROMPT_CHARS, MAX_QUERY_CHARS, MAX_SELECTION_CHARS, MAX_SINGLE_CHUNK_BYTES,
        MAX_SINGLE_HISTORY_MSG_BYTES, MAX_TOP_K,
    };
    use crate::ai::mock_server::{MockHttpServer, RawMockResponse};
    use crate::ai::request_registry::{
        AiRequestRegistry, CancellationCapability, RequestOwner, RequestState,
        MAX_CONCURRENT_REQUESTS,
    };
    use crate::ai::stream_parser::{
        NdjsonStreamParser, TerminalDeliveryState, MAX_INCOMPLETE_BUFFER_BYTES,
    };
    use crate::ai::vector_store::{
        validate_embedding_vector, ChunkData, VectorStore, MAX_EMBEDDING_DIM,
    };
    use crate::document_session::{DocumentSessionStore, TestDocumentSessionFixture};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Notify;

    static ENV_TEST_MUTEX: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    struct EnvVarGuard {
        _lock: parking_lot::MutexGuard<'static, ()>,
        saved_vars: Vec<(&'static str, Option<String>)>,
    }

    impl EnvVarGuard {
        fn new(vars: &[&'static str]) -> Self {
            let lock = ENV_TEST_MUTEX.lock();
            let mut saved_vars = Vec::new();
            for &var in vars {
                saved_vars.push((var, std::env::var(var).ok()));
            }
            Self {
                _lock: lock,
                saved_vars,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (var, val) in &self.saved_vars {
                match val {
                    Some(v) => std::env::set_var(var, v),
                    None => std::env::remove_var(var),
                }
            }
        }
    }

    fn setup_test_document_session(
        store: &DocumentSessionStore,
        pdf_fixture_name: &str,
    ) -> TestDocumentSessionFixture {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(pdf_fixture_name);
        crate::document_session::open_test_session(store, &fixture_path)
    }

    fn make_test_context(addr: std::net::SocketAddr) -> Arc<AiEndpointRuntimeContext> {
        make_test_context_with_connect_timeout(addr, SECURE_CLIENT_CONNECT_TIMEOUT)
    }

    fn make_test_context_with_connect_timeout(
        addr: std::net::SocketAddr,
        connect_timeout: Duration,
    ) -> Arc<AiEndpointRuntimeContext> {
        let host = "127.0.0.1";
        let addrs = vec![addr];
        let client = Arc::new(
            crate::ai::endpoint::build_secure_reqwest_client_with_connect_timeout(
                host,
                &addrs,
                connect_timeout,
            )
            .unwrap(),
        );
        let profile = Arc::new(AiEndpointProfile {
            policy_version: 1,
            mode: EndpointMode::Loopback,
            scheme: "http".to_string(),
            canonical_host: host.to_string(),
            effective_port: addr.port(),
            canonical_origin: format!("http://{}:{}", host, addr.port()),
            endpoint_fingerprint: "test_fp".to_string(),
            created_at_ms: 0,
        });
        let consent = Arc::new(AiEndpointConsent {
            endpoint_fingerprint: "test_fp".to_string(),
            remote_approved: true,
            private_network_approved: true,
            policy_version: 1,
            updated_at_ms: 0,
        });
        Arc::new(AiEndpointRuntimeContext {
            profile,
            consent,
            client,
            validated_addresses: addrs,
            resolved_at_ms: 0,
        })
    }

    // ------------------------------------------------------------------------
    // 0. Dedicated AI Text-Rich PDF Fixture Sanity (§1, §2)
    // ------------------------------------------------------------------------
    #[tokio::test]
    async fn test_ai_text_rich_fixture_sanity() {
        let session_store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let fixture = setup_test_document_session(&session_store, "ai-text-rich.pdf");

        let job_id = uuid::Uuid::new_v4().hyphenated().to_string();

        let outcome = session_store
            .run_snapshot_job(
                &job_id,
                &fixture.session_id,
                fixture.content_generation,
                |context| {
                    let page_texts = crate::ai::extract_semantic_page_texts(&context)?;

                    assert_eq!(
                        page_texts.len(),
                        3,
                        "Expected 3 pages from ai-text-rich.pdf"
                    );

                    let mut total_qualifying_chunks = 0;
                    for (_page_idx, text) in &page_texts {
                        assert!(
                            text.len() >= 100,
                            "Expected rich text per page, got len {}",
                            text.len()
                        );
                        let chars: Vec<char> = text.chars().collect();
                        let mut start = 0;
                        let chunk_size = 80;
                        let chunk_overlap = 10;
                        while start < chars.len() {
                            let end = std::cmp::min(start + chunk_size, chars.len());
                            let chunk_str: String = chars[start..end].iter().collect();
                            if chunk_str.trim().len() > 10 {
                                total_qualifying_chunks += 1;
                            }
                            if start + chunk_size >= chars.len() {
                                break;
                            }
                            start += chunk_size - chunk_overlap;
                        }
                    }

                    assert!(
                        total_qualifying_chunks >= 3,
                        "Expected >= 3 qualifying chunks with len > 10, got {}",
                        total_qualifying_chunks
                    );
                    Ok(())
                },
            )
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            crate::document_session::SessionJobOutcome::Completed { .. }
        ));
    }

    // ------------------------------------------------------------------------
    // 1. IP Classification Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_endpoint_ip_classification() {
        assert_eq!(
            classify_ip(&"127.0.0.1".parse().unwrap()),
            IpCategory::Loopback
        );
        assert_eq!(
            classify_ip(&"127.0.1.5".parse().unwrap()),
            IpCategory::Loopback
        );
        assert_eq!(classify_ip(&"::1".parse().unwrap()), IpCategory::Loopback);
        assert_eq!(
            classify_ip(&"::ffff:127.0.0.1".parse().unwrap()),
            IpCategory::Loopback
        );

        assert_eq!(
            classify_ip(&"10.0.0.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"172.16.0.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"192.168.1.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"::ffff:10.0.0.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"::ffff:192.168.1.1".parse().unwrap()),
            IpCategory::Private
        );
        assert_eq!(
            classify_ip(&"fd00::1".parse().unwrap()),
            IpCategory::Private
        );

        assert_eq!(
            classify_ip(&"8.8.8.8".parse().unwrap()),
            IpCategory::PublicRoutable
        );
        assert_eq!(
            classify_ip(&"1.1.1.1".parse().unwrap()),
            IpCategory::PublicRoutable
        );
        assert_eq!(
            classify_ip(&"::ffff:8.8.8.8".parse().unwrap()),
            IpCategory::PublicRoutable
        );
        assert_eq!(
            classify_ip(&"2606:4700:4700::1111".parse().unwrap()),
            IpCategory::PublicRoutable
        );

        assert_eq!(
            classify_ip(&"0.0.0.0".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"::ffff:0.0.0.0".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(classify_ip(&"::".parse().unwrap()), IpCategory::Rejected);
        assert_eq!(
            classify_ip(&"169.254.1.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"::ffff:169.254.1.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"100.64.0.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"192.0.2.1".parse().unwrap()),
            IpCategory::Rejected
        );
        assert_eq!(
            classify_ip(&"224.0.0.1".parse().unwrap()),
            IpCategory::Rejected
        );
    }

    // ------------------------------------------------------------------------
    // 2. URL Validation and Length Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_url_validation_rules() {
        assert!(matches!(
            parse_and_validate_url("http://user:pass@127.0.0.1:11434"),
            Err(AiError::EndpointCredentialsNotAllowed(_))
        ));

        assert!(matches!(
            parse_and_validate_url("http://127.0.0.1:11434?query=1"),
            Err(AiError::EndpointPathNotAllowed(_))
        ));

        assert!(matches!(
            parse_and_validate_url("http://127.0.0.1:11434#fragment"),
            Err(AiError::EndpointPathNotAllowed(_))
        ));

        assert!(matches!(
            parse_and_validate_url("http://127.0.0.1:11434/api/v1"),
            Err(AiError::EndpointPathNotAllowed(_))
        ));

        assert!(matches!(
            parse_and_validate_url("ftp://127.0.0.1:11434"),
            Err(AiError::EndpointSchemeNotAllowed(_))
        ));

        // URL length exact maximum 2048
        let base = "http://127.0.0.1:11434";
        let valid_len_url = format!("{}/", base);
        assert!(parse_and_validate_url(&valid_len_url).is_ok());

        let too_long_url = format!(
            "http://127.0.0.1:11434/{}",
            "a".repeat(MAX_ENDPOINT_URL_LENGTH)
        );
        assert!(matches!(
            parse_and_validate_url(&too_long_url),
            Err(AiError::EndpointInvalid(_))
        ));
    }

    // ------------------------------------------------------------------------
    // 3. Endpoint Consent and Fingerprinting Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_consent_fingerprint_invalidation() {
        let fp1 =
            compute_endpoint_fingerprint(1, &EndpointMode::Loopback, "http", "127.0.0.1", 11434);
        let fp2 =
            compute_endpoint_fingerprint(1, &EndpointMode::Loopback, "http", "127.0.0.1", 11435);
        assert_ne!(fp1, fp2);

        let consent = AiEndpointConsent {
            endpoint_fingerprint: fp1.clone(),
            remote_approved: true,
            private_network_approved: true,
            policy_version: 1,
            updated_at_ms: 0,
        };

        // Matching fingerprint is authorized
        assert!(is_authorized(
            &EndpointMode::RemotePublic,
            Some(&consent),
            &fp1
        ));

        // Mismatched fingerprint is rejected (even if consent object exists)
        assert!(!is_authorized(
            &EndpointMode::RemotePublic,
            Some(&consent),
            &fp2
        ));
        assert!(!is_authorized(
            &EndpointMode::PrivateNetwork,
            Some(&consent),
            &fp2
        ));

        // Policy version mismatch invalidates consent
        let expired_policy_consent = AiEndpointConsent {
            endpoint_fingerprint: fp1.clone(),
            remote_approved: true,
            private_network_approved: true,
            policy_version: 2,
            updated_at_ms: 0,
        };
        assert!(!is_authorized(
            &EndpointMode::RemotePublic,
            Some(&expired_policy_consent),
            &fp1
        ));
    }

    // ------------------------------------------------------------------------
    // 4. Chunk Parameters and Document Bounds Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_chunk_params_boundary() {
        assert!(validate_chunk_params(500, 50).is_ok());
        assert!(validate_chunk_params(MAX_CHUNK_SIZE, MAX_CHUNK_OVERLAP).is_ok());

        assert!(matches!(
            validate_chunk_params(0, 0),
            Err(AiError::InvalidChunkPolicy(_))
        ));
        assert!(matches!(
            validate_chunk_params(MAX_CHUNK_SIZE + 1, 50),
            Err(AiError::InvalidChunkPolicy(_))
        ));
        assert!(matches!(
            validate_chunk_params(500, 500),
            Err(AiError::InvalidChunkPolicy(_))
        ));
        assert!(matches!(
            validate_chunk_params(500, 501),
            Err(AiError::InvalidChunkPolicy(_))
        ));
        assert!(matches!(
            validate_chunk_params(3000, MAX_CHUNK_OVERLAP + 1),
            Err(AiError::InvalidChunkPolicy(_))
        ));
    }

    #[test]
    fn test_security_extracted_doc_boundaries() {
        assert!(validate_extracted_doc(100, 10_000, 50_000).is_ok());

        assert!(matches!(
            validate_extracted_doc(MAX_JOB_PAGES + 1, 10_000, 50_000),
            Err(AiError::IndexLimitExceeded(_))
        ));
        assert!(matches!(
            validate_extracted_doc(100, MAX_EXTRACTED_DOC_CHARS + 1, 50_000),
            Err(AiError::IndexLimitExceeded(_))
        ));
        assert!(matches!(
            validate_extracted_doc(100, 10_000, MAX_EXTRACTED_DOC_BYTES + 1),
            Err(AiError::IndexLimitExceeded(_))
        ));
    }

    #[test]
    fn test_security_single_chunk_and_count_boundaries() {
        assert!(validate_single_chunk_bytes(1000).is_ok());
        assert!(validate_single_chunk_bytes(MAX_SINGLE_CHUNK_BYTES).is_ok());
        assert!(matches!(
            validate_single_chunk_bytes(MAX_SINGLE_CHUNK_BYTES + 1),
            Err(AiError::IndexLimitExceeded(_))
        ));

        assert!(validate_chunk_count(100).is_ok());
        assert!(validate_chunk_count(MAX_JOB_CHUNKS).is_ok());
        assert!(matches!(
            validate_chunk_count(MAX_JOB_CHUNKS + 1),
            Err(AiError::IndexLimitExceeded(_))
        ));
    }

    // ------------------------------------------------------------------------
    // 5. Input Validation: Queries, Chat, Models, Summaries
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_model_name_validation() {
        assert!(validate_model_name("llama3.2:latest").is_ok());
        assert!(validate_model_name("nomic-embed-text:v1.5").is_ok());
        assert!(validate_model_name("hf.co/org/model_name-v1.0").is_ok());

        assert!(matches!(
            validate_model_name(""),
            Err(AiError::InvalidModel(_))
        ));
        assert!(matches!(
            validate_model_name(&"a".repeat(MAX_MODEL_NAME_CHARS + 1)),
            Err(AiError::InvalidModel(_))
        ));
        assert!(matches!(
            validate_model_name("model\nname"),
            Err(AiError::InvalidModel(_))
        ));
        assert!(matches!(
            validate_model_name("model;rm -rf /"),
            Err(AiError::InvalidModel(_))
        ));
        assert!(matches!(
            validate_model_name("model$bad"),
            Err(AiError::InvalidModel(_))
        ));
    }

    #[test]
    fn test_security_query_validation() {
        assert!(validate_query("valid search query", 5).is_ok());
        assert!(matches!(
            validate_query("", 5),
            Err(AiError::RequestInvalid(_))
        ));
        assert!(matches!(
            validate_query("   ", 5),
            Err(AiError::RequestInvalid(_))
        ));
        assert!(matches!(
            validate_query(&"q".repeat(MAX_QUERY_CHARS + 1), 5),
            Err(AiError::RequestInvalid(_))
        ));
        assert!(matches!(
            validate_query("valid query", 0),
            Err(AiError::RequestInvalid(_))
        ));
        assert!(matches!(
            validate_query("valid query", MAX_TOP_K + 1),
            Err(AiError::RequestInvalid(_))
        ));
    }

    #[test]
    fn test_security_chat_input_and_history_boundaries() {
        let msg = ChatMessageInput {
            role: "user".to_string(),
            content: "hello".to_string(),
        };
        assert!(validate_chat_input("valid prompt", &[msg]).is_ok());

        assert!(matches!(
            validate_chat_input("", &[]),
            Err(AiError::InvalidPrompt(_))
        ));
        assert!(matches!(
            validate_chat_input(&"p".repeat(MAX_PROMPT_CHARS + 1), &[]),
            Err(AiError::InvalidPrompt(_))
        ));

        // System role rejection
        let sys_msg = ChatMessageInput {
            role: "system".to_string(),
            content: "bad injected prompt".to_string(),
        };
        assert!(matches!(
            validate_chat_input("prompt", &[sys_msg]),
            Err(AiError::InvalidHistory(_))
        ));

        // Max history length
        let long_history: Vec<ChatMessageInput> = (0..MAX_HISTORY_MESSAGES + 1)
            .map(|_| ChatMessageInput {
                role: "user".to_string(),
                content: "a".to_string(),
            })
            .collect();
        assert!(matches!(
            validate_chat_input("prompt", &long_history),
            Err(AiError::InvalidHistory(_))
        ));

        // Single history msg byte size
        let oversized_msg = vec![ChatMessageInput {
            role: "user".to_string(),
            content: "a".repeat(MAX_SINGLE_HISTORY_MSG_BYTES + 1),
        }];
        assert!(matches!(
            validate_chat_input("prompt", &oversized_msg),
            Err(AiError::InvalidHistory(_))
        ));
    }

    #[test]
    fn test_security_summary_input_validation() {
        assert!(validate_summary_input("some text", "short").is_ok());
        assert!(validate_summary_input("some text", "detailed").is_ok());
        assert!(validate_summary_input("some text", "bullet").is_ok());

        assert!(matches!(
            validate_summary_input("", "short"),
            Err(AiError::InvalidPrompt(_))
        ));
        assert!(matches!(
            validate_summary_input(&"s".repeat(MAX_SELECTION_CHARS + 1), "short"),
            Err(AiError::InvalidPrompt(_))
        ));
        assert!(matches!(
            validate_summary_input("some text", "invalid_length_target"),
            Err(AiError::RequestInvalid(_))
        ));
    }

    // ------------------------------------------------------------------------
    // 6. Vector Store Validation and Lifecycle Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_vector_store_validation_and_lifecycle() {
        let valid_v = vec![0.1, 0.2, 0.3];
        assert!(validate_embedding_vector(&valid_v, Some(3)).is_ok());

        // Dimension mismatch
        assert!(matches!(
            validate_embedding_vector(&valid_v, Some(4)),
            Err(AiError::EmbeddingDimensionMismatch(_))
        ));

        // Zero dimension
        assert!(matches!(
            validate_embedding_vector(&[], None),
            Err(AiError::EmbeddingInvalid(_))
        ));

        // Exceeds max dim
        let large_v = vec![0.1; MAX_EMBEDDING_DIM + 1];
        assert!(matches!(
            validate_embedding_vector(&large_v, None),
            Err(AiError::EmbeddingInvalid(_))
        ));

        // NaN / Inf rejection
        let nan_v = vec![0.1, f32::NAN, 0.3];
        assert!(matches!(
            validate_embedding_vector(&nan_v, None),
            Err(AiError::EmbeddingInvalid(_))
        ));

        let inf_v = vec![0.1, f32::INFINITY, 0.3];
        assert!(matches!(
            validate_embedding_vector(&inf_v, None),
            Err(AiError::EmbeddingInvalid(_))
        ));

        let vs = VectorStore::new();
        assert_eq!(vs.current_global_memory_bytes(), 0);

        let chunks = vec![ChunkData {
            text: "chunk 1".to_string(),
            page_index: 0,
            embedding: vec![1.0, 0.0, 0.0],
        }];

        vs.insert_staging(
            "s1",
            1,
            "fp_test",
            "nomic-embed-text",
            500,
            50,
            3,
            chunks.clone(),
        )
        .unwrap();

        assert!(vs.current_global_memory_bytes() > 0);
        assert!(vs.matches_expected_configuration("s1", 1, "fp_test", "nomic-embed-text", 500, 50));
        assert!(!vs.matches_expected_configuration(
            "s1",
            2,
            "fp_test",
            "nomic-embed-text",
            500,
            50
        ));
        assert!(!vs.matches_expected_configuration(
            "s1",
            1,
            "fp_other",
            "nomic-embed-text",
            500,
            50
        ));

        // Invalidate session cleans up memory
        vs.remove_session("s1");
        assert_eq!(vs.current_global_memory_bytes(), 0);
        assert!(!vs.matches_expected_configuration(
            "s1",
            1,
            "fp_test",
            "nomic-embed-text",
            500,
            50
        ));
    }

    // ------------------------------------------------------------------------
    // 7. Request Registry Concurrency and Lifecycle Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_request_registry_lifecycle_and_bounds() {
        let reg = AiRequestRegistry::new();
        let owner = RequestOwner::SessionNative {
            session_id: "s1".to_string(),
            expected_content_generation: 1,
        };

        let mut guards = Vec::new();
        for i in 0..MAX_CONCURRENT_REQUESTS {
            let req_id = format!("req_{:04}", i);
            let (_handle, guard, _cancel_rx) = reg
                .register_request(
                    req_id,
                    "chat".to_string(),
                    owner.clone(),
                    CancellationCapability::StreamingWatch,
                )
                .unwrap();
            guards.push(guard);
        }

        // Concurrency limit reached
        let overflow_req = format!("req_{:04}", MAX_CONCURRENT_REQUESTS);
        let err = reg.register_request(
            overflow_req,
            "chat".to_string(),
            owner.clone(),
            CancellationCapability::StreamingWatch,
        );
        assert!(matches!(err, Err(AiError::RequestLimitExceeded(_))));

        // Finishing one allows registration of new request
        guards.pop().unwrap().finish(RequestState::Completed);
        let allowed_req = format!("req_{:04}", MAX_CONCURRENT_REQUESTS);
        let res = reg.register_request(
            allowed_req,
            "chat".to_string(),
            owner.clone(),
            CancellationCapability::StreamingWatch,
        );
        assert!(res.is_ok());
    }

    // ------------------------------------------------------------------------
    // 8. Streaming NDJSON Parser Bounds and Delivery Acceptance
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_streaming_parser_bounds_and_delivery() {
        let mut parser = NdjsonStreamParser::new(
            "req_stream_test".to_string(),
            "chat".to_string(),
            Some("s1".to_string()),
            Some(1),
            "fp_test".to_string(),
        );

        let app = tauri::test::mock_app();
        let handle = app.handle();

        let huge_incomplete_chunk = vec![b'a'; MAX_INCOMPLETE_BUFFER_BYTES + 10];
        fn dummy_extract(_v: &serde_json::Value) -> (Option<String>, bool) {
            (None, false)
        }
        let err = parser.push_chunk(handle, &huge_incomplete_chunk, dummy_extract);
        assert!(matches!(err, Err(AiError::EndpointStreamFrameTooLarge(_))));

        // Valid terminal state delivery
        let mut valid_parser = NdjsonStreamParser::new(
            "req_stream_valid".to_string(),
            "chat".to_string(),
            Some("s1".to_string()),
            Some(1),
            "fp_test".to_string(),
        );

        assert_eq!(
            valid_parser.terminal_state(),
            TerminalDeliveryState::Pending
        );
        valid_parser.emit_started(handle).unwrap();
        assert_eq!(
            valid_parser.terminal_state(),
            TerminalDeliveryState::Pending
        );

        valid_parser.emit_completed(handle, None).unwrap();
        assert_eq!(
            valid_parser.terminal_state(),
            TerminalDeliveryState::Delivered
        );

        // Second terminal event is no-op
        valid_parser.emit_completed(handle, None).unwrap();
        valid_parser.emit_cancelled(handle).unwrap();
        valid_parser
            .emit_failed(handle, &AiError::RequestInvalid("err".to_string()))
            .unwrap();
        assert_eq!(
            valid_parser.terminal_state(),
            TerminalDeliveryState::Delivered
        );
    }

    #[tokio::test]
    async fn test_security_redirect_rejection() {
        let server = MockHttpServer::start_raw(|_| RawMockResponse::Standard {
            status: 302,
            headers: vec![("Location", "http://127.0.0.1/evil".to_string())],
            body: vec![],
        })
        .await;

        let ctx = make_test_context(server.addr);
        let res = SecureHttpClient::send_post_json(
            &ctx,
            "/api/test",
            &json!({"test": 1}),
            Duration::from_secs(5),
            "application/json",
        )
        .await;

        assert!(matches!(res, Err(AiError::EndpointRedirectRejected(_))));
    }

    #[tokio::test]
    async fn test_security_content_type_essence_and_decompression() {
        // Test 1: text/application/json-malicious rejected
        let server1 = MockHttpServer::start(|_| {
            (
                200,
                vec![("Content-Type", "text/application/json-malicious")],
                b"{}".to_vec(),
            )
        })
        .await;
        let ctx1 = make_test_context(server1.addr);
        let res1 = SecureHttpClient::send_post_json(
            &ctx1,
            "/api/test",
            &json!({}),
            Duration::from_secs(5),
            "application/json",
        )
        .await;
        assert!(matches!(res1, Err(AiError::EndpointContentTypeInvalid(_))));

        // Test 2: application/json; charset=utf-8 accepted
        let server2 = MockHttpServer::start(|_| {
            (
                200,
                vec![("Content-Type", "application/json; charset=utf-8")],
                b"{}".to_vec(),
            )
        })
        .await;
        let ctx2 = make_test_context(server2.addr);
        let res2 = SecureHttpClient::send_post_json(
            &ctx2,
            "/api/test",
            &json!({}),
            Duration::from_secs(5),
            "application/json",
        )
        .await;
        assert!(res2.is_ok());

        // Test 3: Content-Encoding: gzip rejected
        let server3 = MockHttpServer::start(|_| {
            (
                200,
                vec![
                    ("Content-Type", "application/json"),
                    ("Content-Encoding", "gzip"),
                ],
                b"{}".to_vec(),
            )
        })
        .await;
        let ctx3 = make_test_context(server3.addr);
        let res3 = SecureHttpClient::send_post_json(
            &ctx3,
            "/api/test",
            &json!({}),
            Duration::from_secs(5),
            "application/json",
        )
        .await;
        assert!(matches!(
            res3,
            Err(AiError::EndpointDecompressionRejected(_))
        ));
    }

    #[tokio::test]
    async fn test_security_response_body_limit() {
        let server = MockHttpServer::start(|_| {
            (
                200,
                vec![("Content-Type", "application/json")],
                vec![b'a'; 6 * 1024 * 1024],
            )
        })
        .await;

        let ctx = make_test_context(server.addr);
        let res = SecureHttpClient::send_post_json(
            &ctx,
            "/api/test",
            &json!({}),
            Duration::from_secs(5),
            "application/json",
        )
        .await;
        assert!(res.is_ok());
        let body_res = SecureHttpClient::fetch_json_body(
            res.unwrap(),
            5 * 1024 * 1024,
            tokio::time::Instant::now() + Duration::from_secs(5),
        )
        .await;
        assert!(matches!(
            body_res,
            Err(AiError::EndpointResponseTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn test_security_rfc4122_v4_uuid_validation() {
        let valid_v4 = uuid::Uuid::new_v4().hyphenated().to_string();
        let valid_req = format!("req_{}", valid_v4);
        let valid_attempt = format!("attempt_{}", valid_v4);

        assert!(validate_request_id(&valid_req).is_ok());
        assert!(validate_caller_attempt_id(&valid_attempt).is_ok());

        // Trailing/leading whitespace rejected
        assert!(matches!(
            validate_request_id(&format!(" req_{}", valid_v4)),
            Err(AiError::RequestInvalid(_))
        ));
        assert!(matches!(
            validate_caller_attempt_id(&format!("attempt_{} ", valid_v4)),
            Err(AiError::RequestInvalid(_))
        ));

        // Uppercase rejected
        assert!(matches!(
            validate_request_id(&format!("req_{}", valid_v4.to_uppercase())),
            Err(AiError::RequestInvalid(_))
        ));

        // Missing prefix rejected
        assert!(matches!(
            validate_request_id(&valid_v4),
            Err(AiError::RequestInvalid(_))
        ));
        assert!(matches!(
            validate_caller_attempt_id(&valid_v4),
            Err(AiError::RequestInvalid(_))
        ));
    }

    // ------------------------------------------------------------------------
    // 9. Explicit Cancellation Acceptance Tests (C1–C6) (§3, §4, §5, §6, §7, §8)
    // ------------------------------------------------------------------------
    #[tokio::test]
    async fn test_security_c1_cancellation_pending_send() {
        let (req_received_tx, req_received_rx) = tokio::sync::oneshot::channel();
        let req_received_tx = Arc::new(parking_lot::Mutex::new(Some(req_received_tx)));

        let server = MockHttpServer::start_raw(move |_| {
            if let Some(tx) = req_received_tx.lock().take() {
                let _ = tx.send(());
            }
            RawMockResponse::StallHeaders { delay_ms: 10_000 }
        })
        .await;

        let ctx = make_test_context(server.addr);
        let vs = VectorStore::new();
        vs.insert_staging(
            "s1",
            1,
            "test_fp",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![ChunkData {
                text: "test chunk".to_string(),
                page_index: 0,
                embedding: vec![1.0, 0.0, 0.0],
            }],
        )
        .unwrap();

        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        let search_task = crate::ai::semantic_search_internal(
            "s1",
            1,
            "test query",
            5,
            "nomic-embed-text",
            500,
            50,
            &ctx,
            Some(&mut cancel_rx),
            deadline,
            &vs,
        );

        let cancel_task = async {
            req_received_rx.await.unwrap();
            assert_eq!(server.request_count(), 1);
            cancel_tx.send(true).unwrap();
        };

        let (result, _) = tokio::join!(search_task, cancel_task);
        assert!(matches!(result, Err(AiError::RequestCancelled(_))));
    }

    #[tokio::test]
    async fn test_security_c2_cancellation_stalled_body() {
        let stall_entered = Arc::new(Notify::new());
        let stall_entered_clone = stall_entered.clone();

        let server = MockHttpServer::start_raw(move |_| RawMockResponse::StallBody {
            headers: vec![("Content-Type", "application/json".to_string())],
            initial_chunk: b"{\"embedding\":".to_vec(),
            delay_ms: 10_000,
            on_stall_entered: Some(stall_entered_clone.clone()),
        })
        .await;

        let ctx = make_test_context(server.addr);
        let vs = VectorStore::new();
        vs.insert_staging(
            "s1",
            1,
            "test_fp",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![ChunkData {
                text: "test chunk".to_string(),
                page_index: 0,
                embedding: vec![1.0, 0.0, 0.0],
            }],
        )
        .unwrap();

        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        let search_task = crate::ai::semantic_search_internal(
            "s1",
            1,
            "test query",
            5,
            "nomic-embed-text",
            500,
            50,
            &ctx,
            Some(&mut cancel_rx),
            deadline,
            &vs,
        );

        let cancel_task = async {
            stall_entered.notified().await;
            assert_eq!(server.request_count(), 1);
            cancel_tx.send(true).unwrap();
        };

        let (result, _) = tokio::join!(search_task, cancel_task);
        assert!(matches!(result, Err(AiError::RequestCancelled(_))));
    }

    #[tokio::test]
    async fn test_security_c3_cancellation_streaming_iteration() {
        let stall_entered = Arc::new(Notify::new());
        let stall_entered_clone = stall_entered.clone();

        let server = MockHttpServer::start_raw(move |_| RawMockResponse::StallBody {
            headers: vec![("Content-Type", "application/x-ndjson".to_string())],
            initial_chunk: b"{\"message\":{\"content\":\"hello\"},\"done\":false}\n".to_vec(),
            delay_ms: 10_000,
            on_stall_entered: Some(stall_entered_clone.clone()),
        })
        .await;

        let ctx = make_test_context(server.addr);
        let response = SecureHttpClient::send_post_json(
            &ctx,
            "/api/chat",
            &json!({"model": "llama3.2", "messages": []}),
            Duration::from_secs(5),
            "application/x-ndjson",
        )
        .await
        .unwrap();

        let reg = AiRequestRegistry::new();
        let owner = RequestOwner::SessionNative {
            session_id: "s1".to_string(),
            expected_content_generation: 1,
        };
        let (_handle, guard, cancel_rx) = reg
            .register_request(
                "req_c3_prod".to_string(),
                "chat".to_string(),
                owner,
                CancellationCapability::StreamingWatch,
            )
            .unwrap();

        let parser = NdjsonStreamParser::new(
            "req_c3_prod".to_string(),
            "chat".to_string(),
            Some("s1".to_string()),
            Some(1),
            "test_fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        let stream_task = crate::ai::consume_ai_ndjson_stream(
            response,
            parser,
            cancel_rx,
            deadline,
            handle,
            guard,
            |v| {
                let delta = v
                    .pointer("/message/content")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());
                (delta, false)
            },
        );

        let cancel_task = async {
            stall_entered.notified().await;
            reg.cancel_request("req_c3_prod", None).unwrap();
        };

        let (result, _) = tokio::join!(stream_task, cancel_task);
        assert!(matches!(result, Err(AiError::RequestCancelled(_))));
        assert_eq!(reg.get_state("req_c3_prod"), Some(RequestState::Cancelled));
    }

    #[tokio::test]
    async fn test_security_c4_cancelled_index_build_publishes_nothing() {
        let vs = VectorStore::new();
        let reg = AiRequestRegistry::new();
        let session_store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let endpoint_store = AiEndpointStore::new();
        let fixture = setup_test_document_session(&session_store, "ai-text-rich.pdf");

        let request_notify = Arc::new(Notify::new());
        let request_notify_clone = request_notify.clone();

        let server = MockHttpServer::start_raw(move |_| {
            request_notify_clone.notify_one();
            RawMockResponse::StallBody {
                headers: vec![("Content-Type", "application/json".to_string())],
                initial_chunk: vec![],
                delay_ms: 10_000,
                on_stall_entered: None,
            }
        })
        .await;
        let ctx = make_test_context(server.addr);

        let owner = RequestOwner::SessionNative {
            session_id: fixture.session_id.clone(),
            expected_content_generation: fixture.content_generation,
        };
        let (handle, mut guard, cancel_rx) = reg
            .register_request(
                "req_c4".to_string(),
                "index".to_string(),
                owner,
                CancellationCapability::SessionJob,
            )
            .unwrap();

        let job_id = uuid::Uuid::new_v4().hyphenated().to_string();

        let index_task = crate::ai::run_index_job_internal(
            &session_store,
            &vs,
            &endpoint_store,
            &fixture.session_id,
            fixture.content_generation,
            &job_id,
            500,
            50,
            "nomic-embed-text",
            &ctx,
            cancel_rx,
            &handle,
            &mut guard,
            tokio::time::Instant::now() + Duration::from_secs(10),
        );

        let cancel_task = async {
            request_notify.notified().await;
            assert!(server.request_count() >= 1);
            reg.cancel_request("req_c4", None).unwrap();
        };

        let (outcome, _) = tokio::join!(index_task, cancel_task);

        assert!(matches!(
            outcome,
            Ok(crate::document_session::SessionJobOutcome::Cancelled { .. })
        ));

        assert_eq!(vs.current_global_memory_bytes(), 0);
        assert!(vs.0.read().get(&fixture.session_id).is_none());
        assert_eq!(reg.get_state("req_c4"), Some(RequestState::Cancelled));
    }

    #[test]
    fn test_security_c5_cancelled_request_never_completed() {
        let reg = AiRequestRegistry::new();
        let owner = RequestOwner::SessionNative {
            session_id: "s1".to_string(),
            expected_content_generation: 1,
        };
        let (_h, mut guard, _rx) = reg
            .register_request(
                "req_c5".to_string(),
                "search".to_string(),
                owner.clone(),
                CancellationCapability::StreamingWatch,
            )
            .unwrap();

        reg.cancel_request("req_c5", Some(&owner)).unwrap();
        guard.finish(RequestState::Cancelled);

        assert_ne!(reg.get_state("req_c5"), Some(RequestState::Completed));
        assert_eq!(reg.get_state("req_c5"), Some(RequestState::Cancelled));

        // Attempting to mark completed afterwards is a no-op
        guard.finish(RequestState::Completed);
        assert_eq!(reg.get_state("req_c5"), Some(RequestState::Cancelled));
    }

    #[test]
    fn test_security_c6_single_logical_terminal_event() {
        let mut parser = NdjsonStreamParser::new(
            "req_c6".to_string(),
            "chat".to_string(),
            None,
            None,
            "fp".to_string(),
        );
        let app = tauri::test::mock_app();
        let handle = app.handle();

        assert!(parser.emit_cancelled(handle).is_ok());
        assert_eq!(parser.terminal_state(), TerminalDeliveryState::Delivered);

        // Subsequent emit_completed and emit_failed calls are no-ops
        assert!(parser.emit_completed(handle, None).is_ok());
        assert!(parser
            .emit_failed(handle, &AiError::RequestInvalid("err".to_string()))
            .is_ok());
        assert_eq!(parser.terminal_state(), TerminalDeliveryState::Delivered);
    }

    // ------------------------------------------------------------------------
    // 10. Explicit Endpoint Authorization Acceptance Tests (E1–E6) (§15)
    // ------------------------------------------------------------------------
    #[tokio::test]
    async fn test_security_e1_localhost_loopback_authorized() {
        let (mode, addrs) =
            crate::ai::endpoint::resolve_and_classify_endpoint("http", "localhost", 11434)
                .await
                .unwrap();
        assert!(
            !addrs.is_empty(),
            "localhost must resolve to at least one address"
        );
        for sa in &addrs {
            assert!(
                sa.ip().is_loopback(),
                "Resolved localhost address {} is not loopback",
                sa.ip()
            );
            assert_eq!(classify_ip(&sa.ip()), IpCategory::Loopback);
        }
        assert_eq!(mode, EndpointMode::Loopback);
        let fingerprint = compute_endpoint_fingerprint(1, &mode, "http", "localhost", 11434);
        assert!(is_authorized(&mode, None, &fingerprint));
    }

    #[test]
    fn test_security_e2_mixed_hostname_resolution_rejected() {
        let mixed: Vec<std::net::SocketAddr> = vec![
            "127.0.0.1:80".parse().unwrap(),
            "192.168.1.50:80".parse().unwrap(),
        ];
        assert!(matches!(
            validate_and_classify_addresses("http", "example.local", &mixed),
            Err(AiError::EndpointDnsPolicyViolation(_))
        ));
    }

    #[test]
    fn test_security_e3_remote_public_without_consent_rejected() {
        let consent = AiEndpointConsent {
            endpoint_fingerprint: "fp_remote".to_string(),
            remote_approved: false,
            private_network_approved: false,
            policy_version: 1,
            updated_at_ms: 0,
        };
        assert!(!is_authorized(
            &EndpointMode::RemotePublic,
            Some(&consent),
            "fp_remote"
        ));
    }

    #[test]
    fn test_security_e4_remote_public_with_consent_accepted() {
        let consent = AiEndpointConsent {
            endpoint_fingerprint: "fp_remote".to_string(),
            remote_approved: true,
            private_network_approved: false,
            policy_version: 1,
            updated_at_ms: 0,
        };
        assert!(is_authorized(
            &EndpointMode::RemotePublic,
            Some(&consent),
            "fp_remote"
        ));
    }

    #[test]
    fn test_security_e5_private_without_consent_rejected() {
        let consent = AiEndpointConsent {
            endpoint_fingerprint: "fp_private".to_string(),
            remote_approved: true,
            private_network_approved: false,
            policy_version: 1,
            updated_at_ms: 0,
        };
        assert!(!is_authorized(
            &EndpointMode::PrivateNetwork,
            Some(&consent),
            "fp_private"
        ));
    }

    #[test]
    fn test_security_e6_private_with_both_consents_accepted() {
        let consent = AiEndpointConsent {
            endpoint_fingerprint: "fp_private".to_string(),
            remote_approved: true,
            private_network_approved: true,
            policy_version: 1,
            updated_at_ms: 0,
        };
        assert!(is_authorized(
            &EndpointMode::PrivateNetwork,
            Some(&consent),
            "fp_private"
        ));
    }

    // ------------------------------------------------------------------------
    // 11. Explicit Semantic-Index Cache Tests (I1–I7 & M1–M4 & Gap H) (§9, §10, §11)
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_i1_to_i5_and_m1_to_m4_cache_invariants() {
        let vs = VectorStore::new();
        let chunk = ChunkData {
            text: "test chunk".to_string(),
            page_index: 0,
            embedding: vec![1.0, 0.0, 0.0],
        };

        vs.insert_staging(
            "s_doc",
            1,
            "fp_canonical",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![chunk],
        )
        .unwrap();

        let initial_bytes = vs.current_global_memory_bytes();
        assert!(initial_bytes > 0);

        // Exact match succeeds
        assert!(vs.matches_expected_configuration(
            "s_doc",
            1,
            "fp_canonical",
            "nomic-embed-text",
            500,
            50
        ));

        // I1 / M3: Endpoint mismatch -> false & read-only (store intact)
        assert!(!vs.matches_expected_configuration(
            "s_doc",
            1,
            "fp_mismatch",
            "nomic-embed-text",
            500,
            50
        ));
        assert_eq!(vs.current_global_memory_bytes(), initial_bytes);
        assert!(vs.0.read().get("s_doc").is_some());

        // I2 / M2: Model mismatch -> false & read-only (store intact)
        assert!(!vs.matches_expected_configuration(
            "s_doc",
            1,
            "fp_canonical",
            "all-minilm",
            500,
            50
        ));
        assert_eq!(vs.current_global_memory_bytes(), initial_bytes);
        assert!(vs.0.read().get("s_doc").is_some());

        // I3 / M1: Chunk size mismatch -> false & read-only (store intact)
        assert!(!vs.matches_expected_configuration(
            "s_doc",
            1,
            "fp_canonical",
            "nomic-embed-text",
            1000,
            50
        ));
        assert_eq!(vs.current_global_memory_bytes(), initial_bytes);
        assert!(vs.0.read().get("s_doc").is_some());

        // I4: Chunk overlap mismatch -> false & read-only
        assert!(!vs.matches_expected_configuration(
            "s_doc",
            1,
            "fp_canonical",
            "nomic-embed-text",
            500,
            100
        ));
        assert_eq!(vs.current_global_memory_bytes(), initial_bytes);
        assert!(vs.0.read().get("s_doc").is_some());

        // I5 / M4: Content generation mismatch -> false & read-only
        assert!(!vs.matches_expected_configuration(
            "s_doc",
            2,
            "fp_canonical",
            "nomic-embed-text",
            500,
            50
        ));
        assert_eq!(vs.current_global_memory_bytes(), initial_bytes);
        assert!(vs.0.read().get("s_doc").is_some());
    }

    #[tokio::test]
    async fn test_security_i6_atomic_index_failure_rolls_back_without_partial_vectors() {
        let vs = VectorStore::new();
        let reg = AiRequestRegistry::new();
        let session_store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let endpoint_store = AiEndpointStore::new();
        let fixture = setup_test_document_session(&session_store, "ai-text-rich.pdf");

        let req_count = Arc::new(AtomicUsize::new(0));
        let req_count_clone = req_count.clone();

        let server = MockHttpServer::start_raw(move |_| {
            let count = req_count_clone.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                // First chunk succeeds
                RawMockResponse::Standard {
                    status: 200,
                    headers: vec![("Content-Type", "application/json".to_string())],
                    body: serde_json::to_vec(&json!({"embedding": [0.1, 0.2, 0.3]})).unwrap(),
                }
            } else {
                // Second chunk fails deterministically
                RawMockResponse::Standard {
                    status: 500,
                    headers: vec![("Content-Type", "application/json".to_string())],
                    body: b"{\"error\":\"server error on chunk 2\"}".to_vec(),
                }
            }
        })
        .await;
        let ctx = make_test_context(server.addr);

        let owner = RequestOwner::SessionNative {
            session_id: fixture.session_id.clone(),
            expected_content_generation: fixture.content_generation,
        };
        let (handle, mut guard, cancel_rx) = reg
            .register_request(
                "req_i6".to_string(),
                "index".to_string(),
                owner,
                CancellationCapability::SessionJob,
            )
            .unwrap();

        let job_id = uuid::Uuid::new_v4().hyphenated().to_string();
        let outcome = crate::ai::run_index_job_internal(
            &session_store,
            &vs,
            &endpoint_store,
            &fixture.session_id,
            fixture.content_generation,
            &job_id,
            80,
            10,
            "nomic-embed-text",
            &ctx,
            cancel_rx,
            &handle,
            &mut guard,
            tokio::time::Instant::now() + Duration::from_secs(10),
        )
        .await;

        assert!(
            matches!(
                outcome,
                Ok(crate::document_session::SessionJobOutcome::Failed { .. })
            ),
            "Expected SessionJobOutcome::Failed, got {:?}",
            outcome
        );

        // Verify request #1 succeeded and request #2 was reached
        assert!(
            server.request_count() >= 2,
            "Expected >= 2 embedding requests, got {}",
            server.request_count()
        );

        // Atomic rollback: vector store has zero entries and memory unchanged
        assert_eq!(vs.current_global_memory_bytes(), 0);
        assert!(vs.0.read().get(&fixture.session_id).is_none());
    }

    #[test]
    fn test_security_i7_same_identity_failed_rebuild_preserves_valid_index() {
        let vs = VectorStore::new();
        let chunk = ChunkData {
            text: "initial chunk".to_string(),
            page_index: 0,
            embedding: vec![1.0, 0.0, 0.0],
        };

        vs.insert_staging(
            "s_rebuild",
            1,
            "fp_rebuild",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![chunk],
        )
        .unwrap();

        let initial_bytes = vs.current_global_memory_bytes();
        assert!(initial_bytes > 0);

        // Attempt rebuild with invalid embedding dimensions (dim 2 != expected 3)
        let invalid_dim_chunk = ChunkData {
            text: "invalid dim chunk".to_string(),
            page_index: 0,
            embedding: vec![1.0, 0.0],
        };
        let res_dim = vs.insert_staging(
            "s_rebuild",
            1,
            "fp_rebuild",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![invalid_dim_chunk],
        );
        assert!(matches!(
            res_dim,
            Err(AiError::EmbeddingDimensionMismatch(_))
        ));

        // Attempt rebuild with NaN embedding
        let nan_chunk = ChunkData {
            text: "nan chunk".to_string(),
            page_index: 0,
            embedding: vec![f32::NAN, 0.0, 0.0],
        };
        let res_nan = vs.insert_staging(
            "s_rebuild",
            1,
            "fp_rebuild",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![nan_chunk],
        );
        assert!(matches!(res_nan, Err(AiError::EmbeddingInvalid(_))));

        // Old index and memory untouched!
        assert_eq!(vs.current_global_memory_bytes(), initial_bytes);
        let st = vs.0.read();
        assert_eq!(st.get("s_rebuild").unwrap().chunks[0].text, "initial chunk");
        assert_eq!(st.get("s_rebuild").unwrap().chunks.len(), 1);
    }

    #[test]
    fn test_security_same_endpoint_reconfigure_clears_vector_store() {
        let vs = VectorStore::new();
        vs.insert_staging(
            "s1",
            1,
            "fp1",
            "nomic-embed-text",
            500,
            50,
            3,
            vec![ChunkData {
                text: "test".to_string(),
                page_index: 0,
                embedding: vec![1.0, 0.0, 0.0],
            }],
        )
        .unwrap();

        assert!(vs.current_global_memory_bytes() > 0);

        vs.clear_all();
        assert_eq!(vs.current_global_memory_bytes(), 0);
        assert!(vs.0.read().get("s1").is_none());
    }

    // ------------------------------------------------------------------------
    // 12. Explicit Timeout Acceptance Matrix (T1–T5) (§2, §14)
    // ------------------------------------------------------------------------
    #[test]
    fn test_security_timeout_t1_connect_timeout_constant_and_builder() {
        assert_eq!(SECURE_CLIENT_CONNECT_TIMEOUT, Duration::from_secs(5));
        let client = build_secure_reqwest_client("127.0.0.1", &["127.0.0.1:80".parse().unwrap()]);
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_security_connect_refused_is_not_reported_as_timeout() {
        // Find an unassigned loopback port where no service is listening
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let unassigned_addr = listener.local_addr().unwrap();
        drop(listener); // Drop to close socket immediately

        // Windows retries SYN on a refused loopback port and reports the refusal
        // only after ~2 s, so the total budget must outlast that there.
        let total_timeout = if cfg!(windows) {
            Duration::from_secs(4)
        } else {
            Duration::from_millis(500)
        };
        let ctx = make_test_context(unassigned_addr);
        let res = SecureHttpClient::send_post_json(
            &ctx,
            "/api/test",
            &json!({}),
            total_timeout,
            "application/json",
        )
        .await;

        assert!(
            matches!(res, Err(AiError::EndpointConnectFailed(_))),
            "Expected EndpointConnectFailed on closed port, got {:?}",
            res
        );
        // Explicit invariant: connection refused must NEVER be classified as timeout
        assert!(
            !matches!(res, Err(AiError::EndpointConnectTimeout(_))),
            "Connection refused must not be classified as EndpointConnectTimeout"
        );
    }

    #[tokio::test]
    async fn test_security_timeout_t1_connect_timeout_runtime_syn_stall() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // Saturate TCP listen backlog queue so subsequent SYN packets are dropped by the kernel
        let mut holding_connections = Vec::new();
        let mut backlog_saturated = false;
        for _ in 0..1024 {
            match std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(50)) {
                Ok(stream) => {
                    holding_connections.push(stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    backlog_saturated = true;
                    break;
                }
                Err(e) => {
                    panic!("Unexpected error while saturating listen backlog: {e}");
                }
            }
        }

        assert!(
            backlog_saturated,
            "Failed to deterministically saturate TCP listen backlog"
        );

        // Configure client with explicit connect_timeout (100ms) and large header/total timeout (5s)
        let ctx = make_test_context_with_connect_timeout(addr, Duration::from_millis(100));
        let start = tokio::time::Instant::now();
        let res = SecureHttpClient::send_post_json_with_policy(
            &ctx,
            "/api/test",
            &json!({}),
            Duration::from_secs(5),
            "application/json",
            HttpTimeoutPolicy {
                header_timeout: Duration::from_secs(5),
                read_idle_timeout: Duration::from_secs(5),
            },
        )
        .await;
        let elapsed = start.elapsed();

        drop(holding_connections);
        drop(listener);

        assert!(
            matches!(res, Err(AiError::EndpointConnectTimeout(_))),
            "Expected EndpointConnectTimeout from runtime connect stall, got: {:?}, elapsed: {:?}",
            res,
            elapsed
        );
        assert_eq!(
            res.as_ref().unwrap_err().code(),
            "AI_ENDPOINT_CONNECT_TIMEOUT"
        );
        assert!(
            !matches!(res, Err(AiError::EndpointConnectFailed(_))),
            "Must not be classified as EndpointConnectFailed"
        );
        assert!(
            !matches!(res, Err(AiError::EndpointHeaderTimeout(_))),
            "Must not be classified as EndpointHeaderTimeout"
        );
        assert!(
            !matches!(res, Err(AiError::EndpointTotalTimeout(_))),
            "Must not be classified as EndpointTotalTimeout"
        );
    }

    #[test]
    fn test_security_tls_verification_bypass_absence() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ai_dir = manifest_dir.join("src/ai");

        let forbidden_patterns = [
            "danger_accept_invalid_certs",
            "danger_accept_invalid_hostnames",
            "accept_invalid_certs",
            "accept_invalid_hostnames",
        ];

        for entry in std::fs::read_dir(ai_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                let file_name = path.file_name().unwrap().to_str().unwrap();
                if file_name != "security_acceptance_tests.rs" && file_name != "mock_server.rs" {
                    let content = std::fs::read_to_string(&path).unwrap();
                    for pattern in &forbidden_patterns {
                        assert!(
                            !content.contains(pattern),
                            "Production file {:?} contains forbidden TLS bypass pattern: {}",
                            file_name,
                            pattern
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn test_security_timeout_t2_header_timeout() {
        let server =
            MockHttpServer::start_raw(|_| RawMockResponse::StallHeaders { delay_ms: 10_000 }).await;

        let ctx = make_test_context(server.addr);
        let policy = HttpTimeoutPolicy {
            header_timeout: Duration::from_millis(50),
            read_idle_timeout: Duration::from_millis(500),
        };

        let res = SecureHttpClient::send_post_json_with_policy(
            &ctx,
            "/api/test",
            &json!({}),
            Duration::from_millis(200),
            "application/json",
            policy,
        )
        .await;

        assert!(
            matches!(res, Err(AiError::EndpointHeaderTimeout(_))),
            "Expected EndpointHeaderTimeout, got {:?}",
            res
        );
    }

    #[tokio::test]
    async fn test_security_timeout_t3_read_idle_timeout() {
        let server = MockHttpServer::start_raw(|_| RawMockResponse::StallBody {
            headers: vec![("Content-Type", "application/json".to_string())],
            initial_chunk: b"{\"emb".to_vec(),
            delay_ms: 10_000,
            on_stall_entered: None,
        })
        .await;

        let ctx = make_test_context(server.addr);
        let policy = HttpTimeoutPolicy {
            header_timeout: Duration::from_millis(200),
            read_idle_timeout: Duration::from_millis(50),
        };

        let res = SecureHttpClient::send_post_json_with_policy(
            &ctx,
            "/api/test",
            &json!({}),
            Duration::from_millis(500),
            "application/json",
            policy,
        )
        .await
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        let body_res = SecureHttpClient::fetch_json_body_with_policy(
            res,
            MAX_NON_STREAM_RESPONSE_BYTES,
            deadline,
            Duration::from_millis(50),
        )
        .await;

        assert!(
            matches!(body_res, Err(AiError::EndpointReadTimeout(_))),
            "Expected EndpointReadTimeout, got {:?}",
            body_res
        );
    }

    #[tokio::test]
    async fn test_security_timeout_t4_total_request_timeout() {
        let server = MockHttpServer::start_raw(|_| RawMockResponse::ChunkedStream {
            headers: vec![("Content-Type", "application/json".to_string())],
            chunks: vec![
                (b"{\"a\":".to_vec(), 25),
                (b"1,".to_vec(), 25),
                (b"\"b\":".to_vec(), 25),
                (b"2,".to_vec(), 25),
                (b"\"c\":3}".to_vec(), 25),
            ],
            close_abruptly: false,
        })
        .await;

        let ctx = make_test_context(server.addr);
        let policy = HttpTimeoutPolicy {
            header_timeout: Duration::from_millis(200),
            read_idle_timeout: Duration::from_millis(200),
        };

        let res = SecureHttpClient::send_post_json_with_policy(
            &ctx,
            "/api/test",
            &json!({}),
            Duration::from_millis(100),
            "application/json",
            policy,
        )
        .await
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        let body_res = SecureHttpClient::fetch_json_body_with_policy(
            res,
            MAX_NON_STREAM_RESPONSE_BYTES,
            deadline,
            Duration::from_millis(200),
        )
        .await;

        assert!(
            matches!(body_res, Err(AiError::EndpointTotalTimeout(_))),
            "Expected EndpointTotalTimeout, got {:?}",
            body_res
        );
    }

    #[tokio::test]
    async fn test_security_timeout_t5_index_overall_timeout_aborts_without_publication() {
        let vs = VectorStore::new();
        let reg = AiRequestRegistry::new();
        let session_store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let endpoint_store = AiEndpointStore::new();
        let fixture = setup_test_document_session(&session_store, "ai-text-rich.pdf");

        let stall_notify = Arc::new(Notify::new());
        let stall_notify_clone = stall_notify.clone();

        let server = MockHttpServer::start_raw(move |_| RawMockResponse::StallBody {
            headers: vec![("Content-Type", "application/json".to_string())],
            initial_chunk: vec![],
            delay_ms: 10_000,
            on_stall_entered: Some(stall_notify_clone.clone()),
        })
        .await;
        let ctx = make_test_context(server.addr);

        let owner = RequestOwner::SessionNative {
            session_id: fixture.session_id.clone(),
            expected_content_generation: fixture.content_generation,
        };
        let (handle, mut guard, cancel_rx) = reg
            .register_request(
                "req_t5_timeout".to_string(),
                "index".to_string(),
                owner,
                CancellationCapability::SessionJob,
            )
            .unwrap();

        let job_id = uuid::Uuid::new_v4().hyphenated().to_string();

        // Start job with a deadline that will expire while the embedding stall is active
        let deadline = tokio::time::Instant::now() + Duration::from_millis(4000);

        let index_task = crate::ai::run_index_job_internal(
            &session_store,
            &vs,
            &endpoint_store,
            &fixture.session_id,
            fixture.content_generation,
            &job_id,
            500,
            50,
            "nomic-embed-text",
            &ctx,
            cancel_rx,
            &handle,
            &mut guard,
            deadline,
        );

        let stall_waiter = async {
            stall_notify.notified().await;
            assert!(server.request_count() >= 1);
        };

        let (outcome, _) = tokio::join!(index_task, stall_waiter);

        assert!(
            matches!(
                outcome,
                Ok(crate::document_session::SessionJobOutcome::Failed { .. })
            ),
            "Expected SessionJobOutcome::Failed on timeout, got {:?}",
            outcome
        );

        if let Ok(crate::document_session::SessionJobOutcome::Failed { error, .. }) = outcome {
            assert_eq!(error.code, "AI_ENDPOINT_TOTAL_TIMEOUT");
        }

        assert_eq!(vs.current_global_memory_bytes(), 0);
        assert!(vs.0.read().get(&fixture.session_id).is_none());
    }

    // ------------------------------------------------------------------------
    // 13. System Proxy Isolation Acceptance Tests (§3, §7, §8, §9, §13)
    // ------------------------------------------------------------------------
    #[tokio::test]
    async fn test_security_proxy_env_vars_ignored() {
        let trap_counter = Arc::new(AtomicUsize::new(0));
        let trap_counter_clone = trap_counter.clone();

        let trap_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let trap_addr = trap_listener.local_addr().unwrap();

        let trap_handle = tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = trap_listener.accept().await {
                    trap_counter_clone.fetch_add(1, Ordering::SeqCst);
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let resp = b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(resp).await;
                }
            }
        });

        let proxy_vars = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ];
        let _guard = EnvVarGuard::new(&proxy_vars);

        let proxy_url = format!("http://{}", trap_addr);
        std::env::set_var("HTTP_PROXY", &proxy_url);
        std::env::set_var("http_proxy", &proxy_url);
        std::env::set_var("ALL_PROXY", &proxy_url);
        std::env::set_var("all_proxy", &proxy_url);
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");

        // Start legitimate mock server
        let mock_server = MockHttpServer::start(|_| {
            (
                200,
                vec![("Content-Type", "application/json")],
                b"{\"status\":\"direct_ok\"}".to_vec(),
            )
        })
        .await;

        let mock_port = mock_server.addr.port();
        let target_url = format!("http://antigravity-proxy-test.invalid:{mock_port}/api/test");

        // Phase A: Positive Control - standard client without .no_proxy() hits the trap proxy
        let control_client = reqwest::Client::builder().build().unwrap();
        let _ = control_client.get(&target_url).send().await;
        let trap_count_after_control = trap_counter.load(Ordering::SeqCst);
        assert!(
            trap_count_after_control >= 1,
            "Positive control failed: standard client did not route through environment proxy"
        );

        // Phase B: Antigravity Secure Client with .no_proxy() bypasses trap proxy completely
        let secure_client =
            build_secure_reqwest_client("antigravity-proxy-test.invalid", &[mock_server.addr])
                .unwrap();

        let res = secure_client.get(&target_url).send().await.unwrap();
        assert_eq!(res.status(), 200);

        let trap_count_after_secure = trap_counter.load(Ordering::SeqCst);
        assert_eq!(
            trap_count_after_secure, trap_count_after_control,
            "Antigravity secure client leaked connection to proxy environment!"
        );
        assert_eq!(mock_server.request_count(), 1);

        trap_handle.abort();
    }

    // ------------------------------------------------------------------------
    // 14. Explicit DNS / Address-Pinning Acceptance Test with Ephemeral Port (§4, §10)
    // ------------------------------------------------------------------------
    #[tokio::test]
    async fn test_security_dns_address_pinning_uses_sanitized_resolved_addresses() {
        let mock_server = MockHttpServer::start(|_| {
            (
                200,
                vec![("Content-Type", "application/json")],
                b"{\"status\":\"pinned_ok\"}".to_vec(),
            )
        })
        .await;

        let mock_port = mock_server.addr.port();
        let url = format!("http://antigravity-pinned.invalid:{mock_port}/api/test");

        let client =
            build_secure_reqwest_client("antigravity-pinned.invalid", &[mock_server.addr]).unwrap();

        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(mock_server.request_count(), 1);
    }

    // ------------------------------------------------------------------------
    // 15. HTTP Security Matrix: Redirects, Decompression, Status, Limits
    // ------------------------------------------------------------------------
    #[tokio::test]
    async fn test_security_redirect_301_302_307_308_rejected() {
        for redirect_status in [301, 302, 307, 308] {
            let server = MockHttpServer::start_raw(move |_| RawMockResponse::Standard {
                status: redirect_status,
                headers: vec![("Location", "http://127.0.0.1/target".to_string())],
                body: vec![],
            })
            .await;

            let ctx = make_test_context(server.addr);
            let res = SecureHttpClient::send_post_json(
                &ctx,
                "/api/test",
                &json!({}),
                Duration::from_secs(5),
                "application/json",
            )
            .await;

            assert!(matches!(res, Err(AiError::EndpointRedirectRejected(_))));
        }
    }

    #[tokio::test]
    async fn test_security_decompression_brotli_deflate_rejected() {
        for enc in ["br", "deflate"] {
            let server = MockHttpServer::start(move |_| {
                (
                    200,
                    vec![
                        ("Content-Type", "application/json"),
                        ("Content-Encoding", enc),
                    ],
                    b"{}".to_vec(),
                )
            })
            .await;

            let ctx = make_test_context(server.addr);
            let res = SecureHttpClient::send_post_json(
                &ctx,
                "/api/test",
                &json!({}),
                Duration::from_secs(5),
                "application/json",
            )
            .await;

            assert!(matches!(
                res,
                Err(AiError::EndpointDecompressionRejected(_))
            ));
        }
    }

    #[tokio::test]
    async fn test_security_http_non_2xx_status() {
        for code in [404, 500, 503] {
            let server = MockHttpServer::start(move |_| {
                (
                    code,
                    vec![("Content-Type", "application/json")],
                    b"{\"error\":\"fail\"}".to_vec(),
                )
            })
            .await;

            let ctx = make_test_context(server.addr);
            let res = SecureHttpClient::send_post_json(
                &ctx,
                "/api/test",
                &json!({}),
                Duration::from_secs(5),
                "application/json",
            )
            .await;

            assert_eq!(res.unwrap_err(), AiError::EndpointHttpStatus(code));
        }
    }

    #[test]
    fn test_security_max_http2_header_list_bytes_pinned() {
        assert_eq!(MAX_HTTP2_HEADER_LIST_BYTES, 16 * 1024);
        let client = build_secure_reqwest_client("127.0.0.1", &["127.0.0.1:80".parse().unwrap()]);
        assert!(client.is_ok());
    }
}
