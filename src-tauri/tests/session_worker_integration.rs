#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use parking_lot::{Mutex, RwLock};
    use preview_alternative_lib::document_session;
    use preview_alternative_lib::pdf_engine::render_service::{RenderScope, RenderService};
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        PdfiumWorkerBroker, WorkerLauncher,
    };
    use preview_alternative_lib::{
        ActiveRenderTasks, ActiveSearchId, PasswordCache, SearchCache, TextLayoutCache,
    };
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use tauri::Manager;

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name)
            .to_string_lossy()
            .to_string()
    }

    #[tokio::test]
    async fn production_session_commands_use_worker_identity_and_preserve_frontend_contracts() {
        let source_path = fixture("reorder-rich.pdf");
        let original_bytes = std::fs::read(&source_path).unwrap();
        let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
            PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")),
            vec![OsString::from("--run-worker")],
        )));
        let password_cache = Arc::new(Mutex::new(HashMap::new()));
        let store =
            document_session::DocumentSessionStore::initialize_with_password_cache_and_broker(
                Arc::clone(&password_cache),
                broker.clone(),
            )
            .unwrap();
        let capacity = std::num::NonZeroUsize::new(16).unwrap();
        let app = tauri::test::mock_builder()
            .manage(store)
            .manage(PasswordCache(password_cache))
            .manage(SearchCache(Mutex::new(lru::LruCache::new(capacity))))
            .manage(TextLayoutCache(Mutex::new(lru::LruCache::new(capacity))))
            .manage(ActiveSearchId(Arc::new(AtomicU64::new(0))))
            .manage(ActiveRenderTasks(Mutex::new(HashMap::new())))
            .manage(preview_alternative_lib::ai::VectorStore(RwLock::new(
                HashMap::new(),
            )))
            .manage(RenderService::new())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let opened = document_session::open_document_session(source_path.clone(), app.state())
            .await
            .unwrap();
        assert_eq!(opened.status, "candidate_ready");
        let candidate = opened.snapshot.unwrap();
        assert_eq!(candidate.content_generation, 1);

        let probe = document_session::probe_candidate_render(
            opened.session_id.clone(),
            candidate.content_generation,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(probe.page_count, 3);
        assert_eq!(
            probe.byte_length,
            probe.width as usize * probe.height as usize * 4
        );

        let activation = document_session::prepare_candidate_activation(
            opened.session_id.clone(),
            None,
            None,
            app.state(),
        )
        .await
        .unwrap();
        let committed = document_session::commit_candidate_activation(
            activation.activation_token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(committed.snapshot.content_generation, 1);

        document_session::set_active_session_render_document(
            Some(opened.session_id.clone()),
            1,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let rendered = document_session::render_session_page(
            opened.session_id.clone(),
            0,
            1.0,
            90,
            1,
            "session-render-request".to_string(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(rendered.page_index, 0);
        assert_eq!(rendered.generation, 1);
        assert_eq!(rendered.request_id, "session-render-request");
        assert_eq!(
            rendered.data.len(),
            rendered.width as usize * rendered.height as usize * 4
        );

        let thumbnail = document_session::render_session_thumbnail(
            opened.session_id.clone(),
            0,
            180,
            1,
            "thumbnail-request".to_string(),
            RenderScope::SidebarThumbnail,
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(thumbnail.width, 180);
        assert_eq!(thumbnail.request_id, "thumbnail-request");

        let layout = document_session::get_session_page_text_layout(
            opened.session_id.clone(),
            1,
            0,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert!(layout.iter().any(|item| item.text.contains("PAGE-ONE")));

        let search = document_session::search_session_pdf_text(
            opened.session_id.clone(),
            1,
            "page-one".to_string(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(search.total_matches, 1);
        assert_eq!(search.matches[0].page_index, 0);

        assert_eq!(
            document_session::get_session_page_count(
                opened.session_id.clone(),
                1,
                app.state(),
                app.state(),
            )
            .await
            .unwrap(),
            3
        );
        let outline = document_session::get_session_pdf_outline(
            opened.session_id.clone(),
            1,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert!(outline.is_empty());
        let metadata =
            document_session::get_session_pdf_metadata(opened.session_id.clone(), 1, app.state())
                .await
                .unwrap();
        assert!(metadata.title.is_empty());
        assert!(metadata.author.is_empty());
        let annotations = document_session::get_session_page_annotations(
            opened.session_id.clone(),
            1,
            2,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(
            annotations[0].contents.as_deref(),
            Some("Reorder fixture annotation")
        );
        let pdf_ua =
            document_session::validate_session_pdf_ua(opened.session_id.clone(), 1, app.state())
                .await
                .unwrap();
        assert!(!pdf_ua.is_tagged);
        assert!(!pdf_ua.has_document_title);
        assert_eq!(pdf_ua.score, 50);
        let structure_error =
            document_session::get_session_structure_tree(opened.session_id.clone(), 1, app.state())
                .await
                .unwrap_err();
        assert_eq!(
            structure_error.code,
            "PDFIUM_WORKER_STRUCTURE_TREE_UNAVAILABLE"
        );

        let mutated =
            document_session::rotate_session_page(committed.snapshot.id, 1, 0, 90, app.state())
                .await
                .unwrap();
        assert_eq!(mutated.content_generation, 2);
        assert!(mutated.dirty);
        assert_eq!(std::fs::read(&source_path).unwrap(), original_bytes);

        document_session::set_active_session_render_document(
            Some(mutated.id.clone()),
            2,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let reopened_after_mutation = document_session::render_session_page(
            mutated.id.clone(),
            1,
            1.0,
            0,
            2,
            "post-mutation-render".to_string(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(reopened_after_mutation.generation, 2);

        let epoch_before_crash = broker.worker_epoch().await.unwrap();
        broker.kill_process_for_test().await.unwrap();
        let crashed = document_session::render_session_page(
            mutated.id.clone(),
            2,
            1.1,
            0,
            2,
            "crashed-render".to_string(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            crashed.code, "PDFIUM_WORKER_CRASHED",
            "unexpected render error message: {}",
            crashed.message
        );
        let after_crash =
            document_session::get_document_session_snapshot(mutated.id.clone(), app.state())
                .await
                .unwrap();
        assert_eq!(after_crash.content_generation, 2);
        assert!(after_crash.dirty);

        let retried = document_session::render_session_page(
            mutated.id,
            2,
            1.2,
            0,
            2,
            "explicit-retry-render".to_string(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(retried.generation, 2);
        assert_ne!(broker.worker_epoch().await.unwrap(), epoch_before_crash);

        let encrypted =
            document_session::open_document_session(fixture("encrypted.pdf"), app.state())
                .await
                .unwrap();
        assert_eq!(encrypted.status, "password_required");
        let authenticated = document_session::authenticate_document_session(
            encrypted.session_id.clone(),
            "fixture-password".to_string(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(authenticated.content_generation, 1);
        let encrypted_probe = document_session::probe_candidate_render(
            encrypted.session_id.clone(),
            1,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(encrypted_probe.page_count, 1);
        document_session::discard_candidate_session(
            encrypted.session_id,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        document_session::close_all_document_sessions(
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        broker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_mutation_failure_triggers_worker_lazy_reopen() {
        let source_path = fixture("minimal.pdf");
        let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
            PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")),
            vec![OsString::from("--run-worker")],
        )));
        let password_cache = Arc::new(Mutex::new(HashMap::new()));
        let store =
            document_session::DocumentSessionStore::initialize_with_password_cache_and_broker(
                Arc::clone(&password_cache),
                broker.clone(),
            )
            .unwrap();
        let capacity = std::num::NonZeroUsize::new(16).unwrap();
        let app = tauri::test::mock_builder()
            .manage(store)
            .manage(PasswordCache(password_cache))
            .manage(SearchCache(Mutex::new(lru::LruCache::new(capacity))))
            .manage(TextLayoutCache(Mutex::new(lru::LruCache::new(capacity))))
            .manage(ActiveSearchId(Arc::new(AtomicU64::new(0))))
            .manage(ActiveRenderTasks(Mutex::new(HashMap::new())))
            .manage(preview_alternative_lib::ai::VectorStore(RwLock::new(
                HashMap::new(),
            )))
            .manage(RenderService::new())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let opened = document_session::open_document_session(source_path.clone(), app.state())
            .await
            .unwrap();
        let activation = document_session::prepare_candidate_activation(
            opened.session_id.clone(),
            None,
            None,
            app.state(),
        )
        .await
        .unwrap();
        let committed = document_session::commit_candidate_activation(
            activation.activation_token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        // Render page generation 1
        document_session::set_active_session_render_document(
            Some(committed.snapshot.id.clone()),
            1,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let render1 = document_session::render_session_page(
            committed.snapshot.id.clone(),
            0,
            1.0,
            0,
            1,
            "pre-mutation-render".to_string(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(render1.generation, 1);

        // Intentionally pass an invalid recipe / failure to trigger worker document invalidation
        let mut invalid_recipe_json = HashMap::new();
        invalid_recipe_json.insert("type", "invalid_op");

        let err = document_session::rotate_session_page(
            committed.snapshot.id.clone(),
            1,
            9999, // invalid page index
            90,
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "MUTATION_WORKER_FAILED");

        // Next render must succeed via lazy reopen
        let render2 = document_session::render_session_page(
            committed.snapshot.id.clone(),
            0,
            1.0,
            0,
            1,
            "lazy-reopen-render".to_string(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(render2.generation, 1);

        broker.shutdown().await.unwrap();
    }
}
