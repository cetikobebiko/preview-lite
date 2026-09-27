//! Real-process acceptance coverage for the read-only sanitization inspection
//! session-job path and its parent-enforced worker lifecycle.

#![cfg(feature = "sanitization-test-support")]

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use lopdf::{dictionary, Dictionary, Document, Object, Stream};
    use parking_lot::{Mutex, RwLock};
    use preview_alternative_lib::document_session::{
        self, SessionJobCleanupOutcome, SessionJobOutcome,
    };
    use preview_alternative_lib::pdf_engine::render_service::RenderService;
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        PdfiumWorkerBroker, WorkerLauncher,
    };
    use preview_alternative_lib::sanitization;
    use preview_alternative_lib::sanitization_inspection::SanitizationInspectionTestBehavior;
    use preview_alternative_lib::sanitization_transform::{
        self, SanitizationTransformTestBehavior,
    };
    use preview_alternative_lib::sanitization_verifier::SanitizationVerificationTestBehavior;
    use preview_alternative_lib::{
        ActiveRenderTasks, ActiveSearchId, PasswordCache, SearchCache, TextLayoutCache,
    };
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeSet, HashMap};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::time::Duration;
    use tauri::Manager;

    static ACCEPTANCE_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sanitization")
            .join(name)
    }

    fn app() -> tauri::App<tauri::test::MockRuntime> {
        let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
            PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")),
            vec![OsString::from("--run-worker")],
        )));
        let password_cache = Arc::new(Mutex::new(HashMap::new()));
        let store =
            document_session::DocumentSessionStore::initialize_with_password_cache_and_broker(
                Arc::clone(&password_cache),
                broker,
            )
            .unwrap();
        let capacity = std::num::NonZeroUsize::new(16).unwrap();
        tauri::test::mock_builder()
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
            .unwrap()
    }

    async fn activated_session(
        app: &tauri::App<tauri::test::MockRuntime>,
        path: &Path,
        password: Option<&str>,
    ) -> document_session::DocumentSessionSnapshot {
        let opened = document_session::open_document_session(
            path.to_string_lossy().into_owned(),
            app.state(),
        )
        .await
        .unwrap();
        if let Some(password) = password {
            document_session::authenticate_document_session(
                opened.session_id.clone(),
                password.to_string(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
        }
        let prepared = document_session::prepare_candidate_activation(
            opened.session_id,
            None,
            None,
            app.state(),
        )
        .await
        .unwrap();
        document_session::commit_candidate_activation(
            prepared.activation_token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
        .snapshot
    }

    fn javascript_stream_fixture(directory: &Path) -> PathBuf {
        let mut document = Document::load(fixture("clean-minimal.pdf")).unwrap();
        let mut stream = Stream::new(
            Dictionary::new(),
            b"SANITIZATION_JS_STREAM_PAYLOAD_PRIVATE".to_vec(),
        );
        stream.compress().unwrap();
        let payload = document.add_object(stream);
        let action = document.add_object(dictionary! {
            "S" => "JavaScript",
            "JS" => Object::Reference(payload),
        });
        document
            .catalog_mut()
            .unwrap()
            .set("OpenAction", Object::Reference(action));
        let path = directory.join("javascript-stream.pdf");
        document.save(&path).unwrap();
        path
    }

    fn job_paths(
        app: &tauri::App<tauri::test::MockRuntime>,
        session_id: &str,
        job_id: &str,
    ) -> (PathBuf, PathBuf) {
        let paths = app
            .state::<document_session::DocumentSessionStore>()
            .secure_redaction_export_paths_for_acceptance(session_id)
            .unwrap();
        let job_dir = paths.jobs_root.join(format!("job-{job_id}"));
        let marker = job_dir.join("sanitization-test-phase");
        (job_dir, marker)
    }

    async fn wait_for_marker(path: &Path, expected: &[u8]) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if std::fs::read(path).is_ok_and(|value| value == expected) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("sanitization worker test phase marker was not observed");
    }

    async fn inspect_with_behavior(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        job_id: String,
        timeout: Duration,
        behavior: SanitizationInspectionTestBehavior,
    ) -> SessionJobOutcome<sanitization::SanitizationInspectionReport> {
        sanitization::inspect_document_sanitization_core_with_test_support(
            job_id,
            session.id.clone(),
            session.content_generation,
            sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            &app.state::<document_session::DocumentSessionStore>(),
            timeout,
            behavior,
        )
        .await
        .unwrap()
    }

    fn transform_authority(path: &Path) -> (String, String, sanitization::SanitizationPolicy) {
        let inspection = preview_alternative_lib::sanitization_inspection::inspect_snapshot(
            path,
            None,
            sanitization::SANITIZATION_REPORT_SCHEMA_VERSION,
            sanitization::SANITIZATION_POLICY_SCHEMA_VERSION,
            sanitization::SANITIZATION_INSPECTION_PROFILE,
        )
        .unwrap();
        let digest = sanitization::inspection_digest(&inspection).unwrap();
        let policy = sanitization::SanitizationPolicy {
            schema_version: sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            profile: sanitization::SanitizationProfile::CustomSupportedV1,
            controls: vec!["incremental_revisions".to_string()],
            acknowledgements: sanitization::SanitizationAcknowledgements {
                acknowledge_signature_invalidation: false,
            },
            expected_inspection_version: sanitization::SANITIZATION_INSPECTION_VERSION.to_string(),
        };
        (inspection.input_digest, digest, policy)
    }

    async fn transform_with_behavior(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        source_path: &Path,
        job_id: String,
        timeout: Duration,
        behavior: SanitizationTransformTestBehavior,
    ) -> SessionJobOutcome<sanitization::SanitizationTransformReport> {
        let (input_digest, inspection_digest, policy) = transform_authority(source_path);
        sanitization_transform::transform_document_sanitization_core_with_test_support(
            job_id,
            session.id.clone(),
            sanitization_transform::SanitizationTransformCoreParams {
                expected_content_generation: session.content_generation,
                expected_input_digest: input_digest,
                expected_inspection_digest: inspection_digest,
                policy,
                timeout,
            },
            &app.state::<document_session::DocumentSessionStore>(),
            behavior,
        )
        .await
        .unwrap()
    }

    async fn transform_and_verify_with_behavior(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        source_path: &Path,
        job_id: String,
        verification_timeout: Duration,
        verification_behavior: SanitizationVerificationTestBehavior,
    ) -> SessionJobOutcome<sanitization::SanitizationVerificationReport> {
        let (input_digest, inspection_digest, policy) = transform_authority(source_path);
        sanitization_transform::transform_and_verify_document_sanitization_core(
            job_id,
            session.id.clone(),
            session.content_generation,
            input_digest,
            inspection_digest,
            policy,
            &app.state::<document_session::DocumentSessionStore>(),
            Duration::from_secs(30),
            verification_timeout,
            SanitizationTransformTestBehavior::Real,
            verification_behavior,
        )
        .await
        .unwrap()
    }

    fn assert_session_neutral(
        before: &document_session::DocumentSessionSnapshot,
        after: &document_session::DocumentSessionSnapshot,
    ) {
        assert_eq!(after.id, before.id);
        assert_eq!(after.content_generation, before.content_generation);
        assert_eq!(after.state_revision, before.state_revision);
        assert_eq!(after.dirty, before.dirty);
        assert_eq!(after.display_path, before.display_path);
    }

    #[tokio::test]
    async fn crash_and_timeout_are_typed_cleaned_credential_safe_and_session_neutral() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let session =
            activated_session(&app, &fixture("encrypted.pdf"), Some("fixture-password")).await;
        let paths_before = app
            .state::<document_session::DocumentSessionStore>()
            .secure_redaction_export_paths_for_acceptance(&session.id)
            .unwrap();
        for (behavior, timeout, expected_code) in [
            (
                SanitizationInspectionTestBehavior::HangDuringTraversal,
                Duration::from_secs(2),
                "DOCUMENT_SESSION_PDF_ENGINE_ERROR",
            ),
            (
                SanitizationInspectionTestBehavior::CrashDuringTraversal,
                Duration::from_secs(10),
                "SESSION_JOB_WORKER_CRASHED",
            ),
        ] {
            let job_id = uuid::Uuid::new_v4().to_string();
            let (job_dir, _) = job_paths(&app, &session.id, &job_id);
            let outcome = inspect_with_behavior(&app, &session, job_id, timeout, behavior).await;
            let encoded = serde_json::to_string(&outcome).unwrap();
            match outcome {
                SessionJobOutcome::Failed { error, cleanup, .. } => {
                    assert_eq!(error.code, expected_code);
                    assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                }
                other => panic!("expected sanitization worker failure: {other:?}"),
            }
            assert!(!encoded.contains("fixture-password"));
            assert!(!encoded.contains(&job_dir.to_string_lossy().to_string()));
            assert!(!job_dir.exists());
            let after =
                document_session::get_document_session_snapshot(session.id.clone(), app.state())
                    .await
                    .unwrap();
            assert_session_neutral(&session, &after);
            let paths_after = app
                .state::<document_session::DocumentSessionStore>()
                .secure_redaction_export_paths_for_acceptance(&session.id)
                .unwrap();
            assert_eq!(paths_after.original_path, paths_before.original_path);
            assert_eq!(paths_after.working_path, paths_before.working_path);
        }
    }

    #[tokio::test]
    async fn traversal_and_decode_cancellation_are_cancelled_not_failed_and_reaped() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let directory = tempfile::tempdir().unwrap();
        let decode_fixture = javascript_stream_fixture(directory.path());
        for (path, behavior, marker_value) in [
            (
                fixture("unsupported-active-content.pdf"),
                SanitizationInspectionTestBehavior::HangDuringTraversal,
                b"traversal".as_slice(),
            ),
            (
                decode_fixture.clone(),
                SanitizationInspectionTestBehavior::HangDuringDecode,
                b"decode".as_slice(),
            ),
        ] {
            let app = app();
            let session = activated_session(&app, &path, None).await;
            let job_id = uuid::Uuid::new_v4().to_string();
            let (job_dir, marker) = job_paths(&app, &session.id, &job_id);
            let inspection = inspect_with_behavior(
                &app,
                &session,
                job_id.clone(),
                Duration::from_secs(30),
                behavior,
            );
            let cancellation = async {
                wait_for_marker(&marker, marker_value).await;
                document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                    .await
                    .unwrap();
            };
            let (outcome, ()) = tokio::join!(inspection, cancellation);
            assert!(matches!(
                outcome,
                SessionJobOutcome::Cancelled {
                    cleanup: SessionJobCleanupOutcome::Cleaned,
                    ..
                }
            ));
            assert!(!matches!(outcome, SessionJobOutcome::Failed { .. }));
            assert!(!job_dir.exists());
            let after =
                document_session::get_document_session_snapshot(session.id.clone(), app.state())
                    .await
                    .unwrap();
            assert_session_neutral(&session, &after);
        }
    }

    #[tokio::test]
    async fn transform_crash_timeout_and_cancel_are_typed_cleaned_and_session_neutral() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let source = fixture("clean-minimal.pdf");
        for (behavior, timeout, expected_code) in [
            (
                SanitizationTransformTestBehavior::CrashDuringMutation,
                Duration::from_secs(10),
                "SESSION_JOB_WORKER_CRASHED",
            ),
            (
                SanitizationTransformTestBehavior::HangDuringMutation,
                Duration::from_secs(2),
                "DOCUMENT_SESSION_PDF_ENGINE_ERROR",
            ),
        ] {
            let app = app();
            let session = activated_session(&app, &source, None).await;
            let job_id = uuid::Uuid::new_v4().to_string();
            let (job_dir, _) = job_paths(&app, &session.id, &job_id);
            let outcome =
                transform_with_behavior(&app, &session, &source, job_id, timeout, behavior).await;
            match outcome {
                SessionJobOutcome::Failed { error, cleanup, .. } => {
                    assert_eq!(error.code, expected_code);
                    assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                }
                other => panic!("expected transform worker failure: {other:?}"),
            }
            assert!(!job_dir.exists());
            let after =
                document_session::get_document_session_snapshot(session.id.clone(), app.state())
                    .await
                    .unwrap();
            assert_session_neutral(&session, &after);
        }

        let app = app();
        let session = activated_session(&app, &source, None).await;
        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, _) = job_paths(&app, &session.id, &job_id);
        let marker = job_dir.join("sanitization-transform-test-phase");
        let transform = transform_with_behavior(
            &app,
            &session,
            &source,
            job_id.clone(),
            Duration::from_secs(30),
            SanitizationTransformTestBehavior::HangDuringSerialization,
        );
        let cancellation = async {
            wait_for_marker(&marker, b"serialization").await;
            document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                .await
                .unwrap();
        };
        let (outcome, ()) = tokio::join!(transform, cancellation);
        assert!(matches!(
            outcome,
            SessionJobOutcome::Cancelled {
                cleanup: SessionJobCleanupOutcome::Cleaned,
                ..
            }
        ));
        assert!(!job_dir.exists());
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn transform_real_process_success_is_private_until_terminal_cleanup() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let source = fixture("clean-minimal.pdf");
        let session = activated_session(&app, &source, None).await;
        let before_paths = app
            .state::<document_session::DocumentSessionStore>()
            .secure_redaction_export_paths_for_acceptance(&session.id)
            .unwrap();
        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, _) = job_paths(&app, &session.id, &job_id);
        let (input_digest, inspection_digest, policy) = transform_authority(&source);
        let requested_controls: BTreeSet<String> = policy.controls.iter().cloned().collect();
        let expected_policy = policy.clone();
        let expected_input_digest = input_digest.clone();
        let expected_inspection_digest = inspection_digest.clone();
        let outcome = app
            .state::<document_session::DocumentSessionStore>()
            .run_snapshot_job(
                &job_id,
                &session.id,
                session.content_generation,
                move |context| {
                    let result = preview_alternative_lib::job_worker::run_job(
                        &context,
                        preview_alternative_lib::job_worker::JobCommand::SanitizePdf {
                            content_generation: context.captured_content_generation,
                            policy,
                            expected_input_digest: input_digest,
                            expected_inspection_digest: inspection_digest,
                            output_name: sanitization::SANITIZATION_TRANSFORM_OUTPUT_NAME
                                .to_string(),
                            test_behavior: SanitizationTransformTestBehavior::Real,
                        },
                        Duration::from_secs(30),
                    )?;
                    let (report, artifact) =
                        sanitization_transform::validate_transform_worker_result_for_acceptance(
                            &context,
                            result,
                            &expected_policy,
                            &expected_input_digest,
                            &expected_inspection_digest,
                        )?;
                    let metadata = std::fs::symlink_metadata(&artifact).unwrap();
                    assert!(artifact.exists());
                    assert!(metadata.file_type().is_file());
                    assert!(!metadata.file_type().is_symlink());
                    assert!(metadata.len() > 0);
                    assert_eq!(
                        artifact.file_name().and_then(|name| name.to_str()),
                        Some(sanitization::SANITIZATION_TRANSFORM_OUTPUT_NAME)
                    );
                    assert!(artifact.starts_with(
                        context
                            .snapshot_path
                            .parent()
                            .unwrap()
                            .canonicalize()
                            .unwrap()
                    ));
                    assert_eq!(metadata.len(), report.output_size_bytes);
                    assert_eq!(
                        format!("{:x}", Sha256::digest(std::fs::read(&artifact).unwrap())),
                        report.output_digest
                    );
                    assert_eq!(report.input_digest, expected_input_digest);
                    assert_eq!(report.profile, expected_policy.profile);
                    assert_eq!(
                        report
                            .controls
                            .iter()
                            .map(|control| control.control_id.clone())
                            .collect::<BTreeSet<_>>(),
                        requested_controls
                    );
                    Ok(report)
                },
            )
            .await
            .unwrap();
        match outcome {
            SessionJobOutcome::Completed { value, cleanup, .. } => {
                assert_eq!(value.status, "transformed_unverified");
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
            }
            other => panic!("expected real-process transform success: {other:?}"),
        }
        assert!(
            !job_dir.exists(),
            "terminal cleanup must remove private artifact"
        );
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
        let after_paths = app
            .state::<document_session::DocumentSessionStore>()
            .secure_redaction_export_paths_for_acceptance(&session.id)
            .unwrap();
        assert_eq!(after_paths.original_path, before_paths.original_path);
        assert_eq!(after_paths.working_path, before_paths.working_path);
    }

    #[tokio::test]
    async fn independent_transform_then_verify_happy_path_is_private_and_session_neutral() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let source = fixture("clean-minimal.pdf");
        let session = activated_session(&app, &source, None).await;
        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, _) = job_paths(&app, &session.id, &job_id);
        let outcome = transform_and_verify_with_behavior(
            &app,
            &session,
            &source,
            job_id,
            Duration::from_secs(30),
            SanitizationVerificationTestBehavior::Real,
        )
        .await;
        match outcome {
            SessionJobOutcome::Completed { value, cleanup, .. } => {
                assert!(value.is_verified);
                assert!(value.blockers.is_empty());
                assert!(value.controls.iter().all(|control| {
                    control.status == sanitization::SanitizationVerificationStatus::Passed
                }));
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
            }
            other => panic!("expected independent verification success: {other:?}"),
        }
        assert!(!job_dir.exists());
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn mandatory_skip_pdfium_failure_crash_and_timeout_fail_closed() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let source = fixture("clean-minimal.pdf");
        for (behavior, timeout, expected_code) in [
            (
                SanitizationVerificationTestBehavior::SkipMandatoryRender,
                Duration::from_secs(30),
                "SANITIZATION_VERIFICATION_FAILED",
            ),
            (
                SanitizationVerificationTestBehavior::FailPdfiumReopen,
                Duration::from_secs(30),
                "SANITIZATION_VERIFICATION_FAILED",
            ),
            (
                SanitizationVerificationTestBehavior::CrashDuringTraversal,
                Duration::from_secs(30),
                "SESSION_JOB_WORKER_CRASHED",
            ),
            (
                SanitizationVerificationTestBehavior::HangDuringTraversal,
                Duration::from_secs(2),
                "DOCUMENT_SESSION_PDF_ENGINE_ERROR",
            ),
        ] {
            let app = app();
            let session = activated_session(&app, &source, None).await;
            let job_id = uuid::Uuid::new_v4().to_string();
            let (job_dir, _) = job_paths(&app, &session.id, &job_id);
            let outcome = transform_and_verify_with_behavior(
                &app, &session, &source, job_id, timeout, behavior,
            )
            .await;
            match outcome {
                SessionJobOutcome::Failed { error, cleanup, .. } => {
                    assert_eq!(error.code, expected_code);
                    assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                }
                other => panic!("expected verifier failure: {other:?}"),
            }
            assert!(!job_dir.exists());
            let after =
                document_session::get_document_session_snapshot(session.id.clone(), app.state())
                    .await
                    .unwrap();
            assert_session_neutral(&session, &after);
        }
    }

    #[tokio::test]
    async fn verifier_cancellation_is_cancelled_reaped_cleaned_and_session_neutral() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let source = fixture("clean-minimal.pdf");
        let session = activated_session(&app, &source, None).await;
        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, _) = job_paths(&app, &session.id, &job_id);
        let marker = job_dir.join("sanitization-verification-test-phase");
        let verification = transform_and_verify_with_behavior(
            &app,
            &session,
            &source,
            job_id.clone(),
            Duration::from_secs(30),
            SanitizationVerificationTestBehavior::HangDuringTraversal,
        );
        let cancellation = async {
            wait_for_marker(&marker, b"traversal").await;
            document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                .await
                .unwrap();
        };
        let (outcome, ()) = tokio::join!(verification, cancellation);
        assert!(matches!(
            outcome,
            SessionJobOutcome::Cancelled {
                cleanup: SessionJobCleanupOutcome::Cleaned,
                ..
            }
        ));
        assert!(!job_dir.exists());
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn cancellation_before_worker_execution_is_cancelled_and_creates_no_authoritative_report()
    {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app, &fixture("clean-minimal.pdf"), None).await;
        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, _) = job_paths(&app, &session.id, &job_id);
        document_session::cancel_session_job(session.id.clone(), job_id.clone(), app.state())
            .await
            .unwrap();
        let outcome = inspect_with_behavior(
            &app,
            &session,
            job_id,
            Duration::from_secs(10),
            SanitizationInspectionTestBehavior::Real,
        )
        .await;
        assert!(matches!(
            outcome,
            SessionJobOutcome::Cancelled {
                cleanup: SessionJobCleanupOutcome::Cleaned,
                ..
            }
        ));
        assert!(!job_dir.exists());
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn wrong_session_stale_generation_and_wrong_owner_cancellation_fail_closed() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let owner = activated_session(&app, &fixture("clean-minimal.pdf"), None).await;
        let wrong = document_session::open_document_session(
            fixture("clean-minimal.pdf").to_string_lossy().into_owned(),
            app.state(),
        )
        .await
        .unwrap();

        let stale = sanitization::inspect_document_sanitization_core_with_test_support(
            uuid::Uuid::new_v4().to_string(),
            owner.id.clone(),
            owner.content_generation - 1,
            sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            &app.state::<document_session::DocumentSessionStore>(),
            Duration::from_secs(10),
            SanitizationInspectionTestBehavior::Real,
        )
        .await
        .unwrap_err();
        assert_eq!(stale.code, "STALE_GENERATION");

        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, marker) = job_paths(&app, &owner.id, &job_id);
        let inspection = inspect_with_behavior(
            &app,
            &owner,
            job_id.clone(),
            Duration::from_secs(30),
            SanitizationInspectionTestBehavior::HangDuringTraversal,
        );
        let ownership = async {
            wait_for_marker(&marker, b"traversal").await;
            let wrong_request = sanitization::inspect_document_sanitization_core_with_test_support(
                job_id.clone(),
                wrong.session_id.clone(),
                1,
                sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
                &app.state::<document_session::DocumentSessionStore>(),
                Duration::from_secs(10),
                SanitizationInspectionTestBehavior::Real,
            )
            .await
            .unwrap_err();
            assert_eq!(wrong_request.code, "DOCUMENT_SESSION_INVALID_STATE");
            let wrong_cancel = document_session::cancel_session_job(
                wrong.session_id.clone(),
                job_id.clone(),
                app.state(),
            )
            .await
            .unwrap_err();
            assert_eq!(wrong_cancel.code, "SESSION_JOB_OWNER_MISMATCH");
            document_session::cancel_session_job(owner.id.clone(), job_id, app.state())
                .await
                .unwrap();
        };
        let (outcome, ()) = tokio::join!(inspection, ownership);
        assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
        assert!(!job_dir.exists());
    }

    #[tokio::test]
    async fn session_close_during_inspection_waits_for_cancelled_job_and_accepts_no_report() {
        let _serial = ACCEPTANCE_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app, &fixture("clean-minimal.pdf"), None).await;
        let job_id = uuid::Uuid::new_v4().to_string();
        let (job_dir, marker) = job_paths(&app, &session.id, &job_id);
        let inspection = inspect_with_behavior(
            &app,
            &session,
            job_id.clone(),
            Duration::from_secs(30),
            SanitizationInspectionTestBehavior::HangDuringTraversal,
        );
        let lifecycle = async {
            wait_for_marker(&marker, b"traversal").await;
            let store = app.state::<document_session::DocumentSessionStore>();
            let close = store.close_all();
            let cancel = async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                    .await
                    .unwrap();
            };
            let (closed, ()) = tokio::join!(close, cancel);
            closed.unwrap();
        };
        let (outcome, ()) = tokio::join!(inspection, lifecycle);
        assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
        assert!(!job_dir.exists());
        let closed =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap_err();
        assert_eq!(closed.code, "DOCUMENT_SESSION_NOT_FOUND");
    }
}
