#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        CommandTimeout, PdfiumWorkerBroker, WorkerDocumentKey, WorkerLauncher,
    };
    use preview_alternative_lib::worker::ipc::{WorkerCommand, WorkerRenderTarget, WorkerResult};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn broker() -> PdfiumWorkerBroker {
        let launcher = WorkerLauncher::for_test(
            PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")),
            vec![OsString::from("--run-worker")],
        );
        PdfiumWorkerBroker::with_launcher(Ok(launcher))
    }

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pdf")
            .join(name)
            .to_string_lossy()
            .to_string()
    }

    fn fixture_corpus(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fuzz")
            .join("corpus")
            .join(name)
            .to_string_lossy()
            .to_string()
    }

    #[tokio::test]
    async fn forced_worker_kill_preserves_parent_and_allows_lazy_reopen() {
        let broker = broker();
        let key = WorkerDocumentKey {
            session_id: "crash-regression".to_string(),
            content_generation: 1,
        };
        let admission = broker.admit_read(&key, None).await.unwrap();
        let (_document, page_count, _) = broker
            .ensure_document(&admission, key.clone(), fixture("minimal.pdf"), None, None)
            .await
            .unwrap();
        assert_eq!(page_count, 1);
        let first_epoch = broker.worker_epoch().await.unwrap();

        broker.kill_process_for_test().await.unwrap();
        let crashed = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::GetPageCount { document_id },
                CommandTimeout::PageCount,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(crashed.code, "PDFIUM_WORKER_CRASHED");
        drop(admission);

        let admission = broker.admit_read(&key, None).await.unwrap();
        let (_document, reopened_page_count, _) = broker
            .ensure_document(&admission, key.clone(), fixture("minimal.pdf"), None, None)
            .await
            .unwrap();
        assert_eq!(reopened_page_count, 1);
        let second_epoch = broker.worker_epoch().await.unwrap();
        assert_ne!(first_epoch, second_epoch);

        let audit = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::AuditAccessibility { document_id },
                CommandTimeout::DocumentRead,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(audit, WorkerResult::AccessibilityAudit(_)));

        let rendered = broker
            .command_for_document(
                &admission,
                &key,
                |document_id| WorkerCommand::RenderPage {
                    document_id,
                    page_index: 0,
                    target: WorkerRenderTarget::Scale(1.0),
                    rotation: 0,
                },
                CommandTimeout::RenderPage,
                None,
            )
            .await
            .unwrap();
        match rendered {
            WorkerResult::PageRendered {
                width,
                height,
                data,
            } => {
                assert!(width > 0);
                assert!(height > 0);
                assert_eq!(
                    data.len(),
                    usize::try_from(width).unwrap() * usize::try_from(height).unwrap() * 4
                );
            }
            other => panic!("unexpected render response: {other:?}"),
        }
        drop(admission);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn committed_malformed_fixture_returns_typed_error_without_parent_crash() {
        let broker = broker();
        let key = WorkerDocumentKey {
            session_id: "malformed-regression".to_string(),
            content_generation: 1,
        };
        let admission = broker.admit_read(&key, None).await.unwrap();
        let error = broker
            .ensure_document(&admission, key, fixture("malformed.pdf"), None, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_SESSION_INVALID_PDF");
        drop(admission);
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn fuzz_seed_minimal_opens_without_crash_and_returns_bounded_result() {
        let broker = broker();
        let key = WorkerDocumentKey {
            session_id: "fuzz-minimal-integration".to_string(),
            content_generation: 1,
        };
        let admission = broker.admit_read(&key, None).await.unwrap();
        let result = broker
            .ensure_document(
                &admission,
                key.clone(),
                fixture_corpus("minimal.pdf"),
                None,
                None,
            )
            .await;
        match result {
            Ok((_document, page_count, _protected)) => {
                assert!(page_count >= 1);
            }
            Err(error) => {
                assert!(
                    error.code == "DOCUMENT_SESSION_INVALID_PDF"
                        || error.code == "PDFIUM_WORKER_CRASHED",
                    "unexpected error code: {}",
                    error.code
                );
            }
        }
        drop(admission);
        broker.shutdown().await.unwrap();
    }
}
