//! Real-process acceptance coverage for the authoritative secure-redaction
//! export chain.  This deliberately uses the public session lifecycle and the
//! production core rather than reproducing worker or publication behavior.

#![cfg(feature = "secure-redaction-test-support")]

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use parking_lot::{Mutex, RwLock};
    use preview_alternative_lib::document_session::{
        self, SessionJobCleanupOutcome, SessionJobOutcome,
    };
    use preview_alternative_lib::pdf_engine::render_service::RenderService;
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        PdfiumWorkerBroker, WorkerLauncher,
    };
    use preview_alternative_lib::redaction;
    use preview_alternative_lib::secure_redaction::{
        RedactionAcknowledgements, RedactionRegionInput, SecureRedactionPlan,
        SecureRedactionPolicy, VerificationCheckStatus,
    };
    use preview_alternative_lib::{
        ActiveRenderTasks, ActiveSearchId, PasswordCache, SearchCache, TextLayoutCache,
    };
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use tauri::Manager;

    // PDFium's process fixture is intentionally single-instance for this
    // acceptance target. The exported chain remains concurrent in production;
    // serialization here prevents two independent test apps from competing for
    // the fixture worker's startup timeout.
    static EXECUTION_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn sha256_file(path: &std::path::Path) -> String {
        let mut hasher = Sha256::new();
        hasher.update(std::fs::read(path).unwrap());
        preview_alternative_lib::hex_lower(hasher.finalize())
    }

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/redaction")
            .join(name)
            .to_string_lossy()
            .into_owned()
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

    fn request_parts() -> (
        SecureRedactionPolicy,
        RedactionAcknowledgements,
        Vec<RedactionRegionInput>,
    ) {
        let policy = serde_json::from_value(json!({
            "schema_version": "1.0.0",
            "profile": "secure_rasterized_pages_v1",
            "strategy": "rasterized_pages",
            "raster_dpi": 72,
            "appearance": { "kind": "solid", "color": "black", "opacity": 1.0 },
            "safety_margin_points": 0.0,
            "interactive_content_policy": "strip",
            "signature_policy": "block",
            "encryption_policy": "block"
        }))
        .unwrap();
        let acknowledgements = serde_json::from_value(json!({
            "schema_version": "1.0.0",
            "rasterized_pages_accepted": true,
            "text_search_loss_accepted": true,
            "accessibility_loss_accepted": true,
            "annotation_loss_accepted": true,
            "form_loss_accepted": true,
            "signature_invalidation_accepted": true,
            "encryption_change_accepted": true
        }))
        .unwrap();
        let regions = serde_json::from_value(json!([{
            "id": "acceptance-r0",
            "page_index": 0,
            "normalized_top_left_rect": { "x": 0.1, "y": 0.1, "width": 0.1, "height": 0.1 },
            "source": "acceptance"
        }]))
        .unwrap();
        (policy, acknowledgements, regions)
    }

    async fn activated_session(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> document_session::DocumentSessionSnapshot {
        let opened =
            document_session::open_document_session(fixture("simple-text.pdf"), app.state())
                .await
                .unwrap();
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

    async fn plan(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        policy: SecureRedactionPolicy,
        acknowledgements: RedactionAcknowledgements,
        regions: Vec<RedactionRegionInput>,
    ) -> SecureRedactionPlan {
        match redaction::plan_secure_redaction(
            uuid::Uuid::new_v4().to_string(),
            session.id.clone(),
            session.content_generation,
            policy,
            acknowledgements,
            regions,
            app.state(),
        )
        .await
        .unwrap()
        {
            SessionJobOutcome::Completed { value, cleanup, .. } => {
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                value
            }
            outcome => panic!("planning must complete: {outcome:?}"),
        }
    }

    async fn execute(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        plan: &SecureRedactionPlan,
        target: String,
    ) -> SessionJobOutcome<preview_alternative_lib::secure_redaction::SecureRedactionExecutionReport>
    {
        let (policy, acknowledgements, regions) = request_parts();
        let store = app.state::<document_session::DocumentSessionStore>();
        redaction::execute_secure_redaction_core(redaction::SecureRedactionExecutionArgs {
            job_id: uuid::Uuid::new_v4().to_string(),
            session_id: session.id.clone(),
            expected_content_generation: session.content_generation,
            policy,
            acknowledgements,
            regions,
            expected_plan_digest: plan.plan_digest.clone(),
            target_path: target,
            store: &store,
        })
        .await
        .unwrap()
    }

    async fn execute_with_support(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        plan: &SecureRedactionPlan,
        job_id: String,
        target: String,
        support: Arc<redaction::SecureRedactionExecutionTestSupport>,
    ) -> SessionJobOutcome<preview_alternative_lib::secure_redaction::SecureRedactionExecutionReport>
    {
        let (policy, acknowledgements, regions) = request_parts();
        let store = app.state::<document_session::DocumentSessionStore>();
        redaction::execute_secure_redaction_core_with_test_support(
            redaction::SecureRedactionExecutionArgs {
                job_id,
                session_id: session.id.clone(),
                expected_content_generation: session.content_generation,
                policy,
                acknowledgements,
                regions,
                expected_plan_digest: plan.plan_digest.clone(),
                target_path: target,
                store: &store,
            },
            Some(support),
        )
        .await
        .unwrap()
    }

    fn assert_clean_failed<T: std::fmt::Debug>(outcome: SessionJobOutcome<T>, expected_code: &str) {
        match outcome {
            SessionJobOutcome::Failed { error, cleanup, .. } => {
                assert_eq!(error.code, expected_code);
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
            }
            other => panic!("expected clean typed failure: {other:?}"),
        }
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

    fn blocking_phase_support(
        phase: redaction::SecureRedactionExecutionTestPhase,
    ) -> (
        Arc<redaction::SecureRedactionExecutionTestSupport>,
        Arc<Barrier>,
        Arc<Barrier>,
        Arc<AtomicUsize>,
    ) {
        let reached = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let hits = Arc::new(AtomicUsize::new(0));
        let hook_reached = Arc::clone(&reached);
        let hook_release = Arc::clone(&release);
        let hook_hits = Arc::clone(&hits);
        let support = redaction::SecureRedactionExecutionTestSupport {
            phase_hook: Some(Arc::new(move |observed, _| {
                if observed == phase {
                    hook_hits.fetch_add(1, Ordering::SeqCst);
                    hook_reached.wait();
                    hook_release.wait();
                }
            })),
            ..Default::default()
        };
        (Arc::new(support), reached, release, hits)
    }

    #[tokio::test]
    async fn full_execution_happy_path_is_worker_verified_digest_bound_and_session_neutral() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        assert!(plan.ready_for_execution && plan.executable);
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("secure-redacted.pdf");

        let outcome = execute(&app, &session, &plan, target.to_string_lossy().into_owned()).await;
        let report = match outcome {
            SessionJobOutcome::Completed { value, cleanup, .. } => {
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                value
            }
            other => panic!("execution must complete: {other:?}"),
        };
        let publication = report
            .publication_report
            .as_ref()
            .expect("publication report");
        assert!(report.published);
        assert!(matches!(
            publication.commit_state,
            preview_alternative_lib::secure_redaction::SecureRedactionCommitState::NewCommitted
                | preview_alternative_lib::secure_redaction::SecureRedactionCommitState::NewCommittedDurabilityUnconfirmed
        ));
        assert!(target.is_file());
        let digest = sha256_file(&target);
        assert_eq!(report.transform_report.output_digest, digest);
        assert_eq!(report.verification_report.transform_digest, digest);
        assert_eq!(publication.published_digest, digest);
        assert_eq!(report.transform_report.plan_digest, plan.plan_digest);
        assert_eq!(report.verification_report.plan_digest, plan.plan_digest);
        assert_eq!(report.job_id, report.transform_report.job_id);
        assert_eq!(report.job_id, report.verification_report.job_id);
        assert_eq!(report.session_id, session.id);
        assert!(report.verification_report.is_verified);
        assert!(report.verification_report.blockers.is_empty());
        assert!(report
            .verification_report
            .verification_checks
            .iter()
            .all(|check| check.status == VerificationCheckStatus::Passed));
        preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| {
            pdfium
                .load_pdf_from_file(&target, None)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .expect("published target must reopen as a PDF");

        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_eq!(after.id, session.id);
        assert_eq!(after.content_generation, session.content_generation);
        assert_eq!(after.state_revision, session.state_revision);
        assert_eq!(after.dirty, session.dirty);
    }

    #[tokio::test]
    async fn existing_target_is_atomically_replaced_with_verified_bytes() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("existing-target.pdf");
        std::fs::copy(fixture("simple-text.pdf"), &target).unwrap();
        let original = std::fs::read(&target).unwrap();

        let outcome = execute(&app, &session, &plan, target.to_string_lossy().into_owned()).await;
        let report = match outcome {
            SessionJobOutcome::Completed { value, cleanup, .. } => {
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                value
            }
            other => panic!("replacement must complete: {other:?}"),
        };
        let final_bytes = std::fs::read(&target).unwrap();
        assert_ne!(
            final_bytes, original,
            "verified publication must replace the old file"
        );
        assert_eq!(
            sha256_file(&target),
            report.publication_report.unwrap().published_digest
        );
    }

    #[tokio::test]
    async fn transform_and_verifier_failures_timeouts_and_invalid_authority_fail_closed() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();

        let cases = [
            (
                "transform-failure.pdf",
                redaction::SecureRedactionExecutionTestSupport {
                    transform_action: redaction::SecureRedactionWorkerTestAction::Fail {
                        code: "SESSION_JOB_WORKER_CRASHED".into(),
                        message: "controlled transform crash".into(),
                    },
                    ..Default::default()
                },
                "SESSION_JOB_WORKER_CRASHED",
            ),
            (
                "transform-timeout.pdf",
                redaction::SecureRedactionExecutionTestSupport {
                    transform_action: redaction::SecureRedactionWorkerTestAction::Timeout,
                    ..Default::default()
                },
                "SESSION_JOB_WORKER_TIMEOUT",
            ),
            (
                "verifier-failure.pdf",
                redaction::SecureRedactionExecutionTestSupport {
                    verifier_action: redaction::SecureRedactionWorkerTestAction::Fail {
                        code: "SESSION_JOB_WORKER_CRASHED".into(),
                        message: "controlled verifier crash".into(),
                    },
                    ..Default::default()
                },
                "SESSION_JOB_WORKER_CRASHED",
            ),
            (
                "verifier-timeout.pdf",
                redaction::SecureRedactionExecutionTestSupport {
                    verifier_action: redaction::SecureRedactionWorkerTestAction::Timeout,
                    ..Default::default()
                },
                "SESSION_JOB_WORKER_TIMEOUT",
            ),
        ];

        for (name, support, code) in cases {
            let target = output.path().join(name);
            std::fs::write(&target, b"racing authority must remain").unwrap();
            let before = std::fs::read(&target).unwrap();
            let outcome = execute_with_support(
                &app,
                &session,
                &plan,
                uuid::Uuid::new_v4().to_string(),
                target.to_string_lossy().into_owned(),
                Arc::new(support),
            )
            .await;
            assert_clean_failed(outcome, code);
            assert_eq!(std::fs::read(&target).unwrap(), before);
        }

        let target = output.path().join("invalid-transform-authority.pdf");
        let support = redaction::SecureRedactionExecutionTestSupport {
            transform_result_hook: Some(Arc::new(|result| match result {
                preview_alternative_lib::job_worker::JobResult::SecureRedactionTransform(
                    mut report,
                ) => {
                    report.job_id = uuid::Uuid::new_v4().to_string();
                    preview_alternative_lib::job_worker::JobResult::SecureRedactionTransform(report)
                }
                other => other,
            })),
            ..Default::default()
        };
        let outcome = execute_with_support(
            &app,
            &session,
            &plan,
            uuid::Uuid::new_v4().to_string(),
            target.to_string_lossy().into_owned(),
            Arc::new(support),
        )
        .await;
        assert_clean_failed(outcome, "REDACTION_TRANSFORM_REPORT_MISMATCH");
        assert!(!target.exists());
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn mandatory_verification_gate_rejects_false_blocked_failed_and_not_run_reports() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();

        for case in ["is_verified_false", "blocker", "failed", "not_run"] {
            let target = output.path().join(format!("gate-{case}.pdf"));
            std::fs::write(&target, b"original gate target").unwrap();
            let hook_case = case;
            let support = redaction::SecureRedactionExecutionTestSupport {
                verifier_result_hook: Some(Arc::new(move |result| match result {
                    preview_alternative_lib::job_worker::JobResult::SecureRedactionVerification(
                        mut report,
                    ) => {
                        report.is_verified = false;
                        if hook_case == "not_run" {
                            report.verification_checks[0].status = VerificationCheckStatus::NotRun;
                        } else if hook_case != "blocker" {
                            report.verification_checks[0].status = VerificationCheckStatus::Failed;
                        }
                        if hook_case == "blocker" || hook_case == "is_verified_false" {
                            report.blockers.push(
                                preview_alternative_lib::secure_redaction::RedactionBlocker {
                                    schema_version: "1.0.0".into(),
                                    code: "REDACTION_VERIFICATION_ACCEPTANCE_BLOCKER".into(),
                                    message: "controlled blocker".into(),
                                    details: None,
                                },
                            );
                        }
                        preview_alternative_lib::job_worker::JobResult::SecureRedactionVerification(
                            report,
                        )
                    }
                    other => other,
                })),
                ..Default::default()
            };
            let outcome = execute_with_support(
                &app,
                &session,
                &plan,
                uuid::Uuid::new_v4().to_string(),
                target.to_string_lossy().into_owned(),
                Arc::new(support),
            )
            .await;
            assert_clean_failed(outcome, "REDACTION_VERIFICATION_GATE_FAILED");
            assert_eq!(std::fs::read(&target).unwrap(), b"original gate target");
        }
    }

    #[tokio::test]
    async fn verified_artifact_and_staged_temp_tamper_are_rejected_before_replacement() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();

        let target = output.path().join("artifact-tamper.pdf");
        std::fs::write(&target, b"original artifact target").unwrap();
        let support = redaction::SecureRedactionExecutionTestSupport {
            phase_hook: Some(Arc::new(|phase, artifact| {
                if phase == redaction::SecureRedactionExecutionTestPhase::AfterVerifierValidated {
                    use std::io::Write as _;
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(artifact.unwrap())
                        .unwrap()
                        .write_all(b"post-verifier-tamper")
                        .unwrap();
                }
            })),
            ..Default::default()
        };
        let outcome = execute_with_support(
            &app,
            &session,
            &plan,
            uuid::Uuid::new_v4().to_string(),
            target.to_string_lossy().into_owned(),
            Arc::new(support),
        )
        .await;
        assert_clean_failed(outcome, "REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH");
        assert_eq!(std::fs::read(&target).unwrap(), b"original artifact target");

        let staged_target = output.path().join("staged-tamper.pdf");
        std::fs::write(&staged_target, b"original staged target").unwrap();
        let outcome = execute_with_support(
            &app,
            &session,
            &plan,
            uuid::Uuid::new_v4().to_string(),
            staged_target.to_string_lossy().into_owned(),
            Arc::new(redaction::SecureRedactionExecutionTestSupport {
                corrupt_staged_temp: true,
                ..Default::default()
            }),
        )
        .await;
        assert_clean_failed(outcome, "REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH");
        assert_eq!(
            std::fs::read(&staged_target).unwrap(),
            b"original staged target"
        );
    }

    #[tokio::test]
    async fn missing_and_existing_target_races_preserve_the_racing_writer() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();

        for initially_missing in [true, false] {
            let target = output.path().join(if initially_missing {
                "missing-race.pdf"
            } else {
                "existing-race.pdf"
            });
            if !initially_missing {
                std::fs::write(&target, b"captured original").unwrap();
            }
            let raced_target = target.clone();
            let support = redaction::SecureRedactionExecutionTestSupport {
                phase_hook: Some(Arc::new(move |phase, _| {
                    if phase == redaction::SecureRedactionExecutionTestPhase::BeforePublication {
                        std::fs::write(&raced_target, b"racing writer wins").unwrap();
                    }
                })),
                ..Default::default()
            };
            let outcome = execute_with_support(
                &app,
                &session,
                &plan,
                uuid::Uuid::new_v4().to_string(),
                target.to_string_lossy().into_owned(),
                Arc::new(support),
            )
            .await;
            assert_clean_failed(outcome, "REDACTION_TARGET_CONFLICT");
            assert_eq!(std::fs::read(&target).unwrap(), b"racing writer wins");
        }
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn cancellation_before_transform_and_before_publication_is_deterministic_and_neutral() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();

        for phase in [
            redaction::SecureRedactionExecutionTestPhase::BeforeTransform,
            redaction::SecureRedactionExecutionTestPhase::BeforePublication,
        ] {
            let target = output.path().join(format!("cancel-{phase:?}.pdf"));
            std::fs::write(&target, b"cancel original").unwrap();
            let job_id = uuid::Uuid::new_v4().to_string();
            let (support, reached, release, hits) = blocking_phase_support(phase);
            let execution = execute_with_support(
                &app,
                &session,
                &plan,
                job_id.clone(),
                target.to_string_lossy().into_owned(),
                support,
            );
            let cancellation = async {
                tokio::task::spawn_blocking(move || reached.wait())
                    .await
                    .unwrap();
                document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                    .await
                    .unwrap();
                tokio::task::spawn_blocking(move || release.wait())
                    .await
                    .unwrap();
            };
            let (outcome, ()) = tokio::join!(execution, cancellation);
            assert!(matches!(
                outcome,
                SessionJobOutcome::Cancelled {
                    cleanup: SessionJobCleanupOutcome::Cleaned,
                    ..
                }
            ));
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(std::fs::read(&target).unwrap(), b"cancel original");
            let after =
                document_session::get_document_session_snapshot(session.id.clone(), app.state())
                    .await
                    .unwrap();
            assert_session_neutral(&session, &after);
        }
    }

    #[tokio::test]
    async fn cancellation_while_transform_and_verifier_children_are_running_reaps_them() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();

        for (phase, transform_action, verifier_action) in [
            (
                redaction::SecureRedactionExecutionTestPhase::TransformWorkerStarted,
                redaction::SecureRedactionWorkerTestAction::HangUntilCancelled,
                redaction::SecureRedactionWorkerTestAction::Real,
            ),
            (
                redaction::SecureRedactionExecutionTestPhase::VerifierWorkerStarted,
                redaction::SecureRedactionWorkerTestAction::Real,
                redaction::SecureRedactionWorkerTestAction::HangUntilCancelled,
            ),
        ] {
            let target = output.path().join(format!("cancel-child-{phase:?}.pdf"));
            std::fs::write(&target, b"child cancel original").unwrap();
            let job_id = uuid::Uuid::new_v4().to_string();
            let (base, reached, release, hits) = blocking_phase_support(phase);
            let support = Arc::new(redaction::SecureRedactionExecutionTestSupport {
                phase_hook: base.phase_hook.clone(),
                transform_action,
                verifier_action,
                ..Default::default()
            });
            let execution = execute_with_support(
                &app,
                &session,
                &plan,
                job_id.clone(),
                target.to_string_lossy().into_owned(),
                support,
            );
            let cancellation = async {
                tokio::task::spawn_blocking(move || reached.wait())
                    .await
                    .unwrap();
                document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                    .await
                    .unwrap();
                tokio::task::spawn_blocking(move || release.wait())
                    .await
                    .unwrap();
            };
            let (outcome, ()) = tokio::join!(execution, cancellation);
            assert!(matches!(
                outcome,
                SessionJobOutcome::Cancelled {
                    cleanup: SessionJobCleanupOutcome::Cleaned,
                    ..
                }
            ));
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(std::fs::read(&target).unwrap(), b"child cancel original");
        }
    }

    #[tokio::test]
    async fn cancellation_after_publication_boundary_reports_committed_disk_truth() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("commit-boundary.pdf");
        let job_id = uuid::Uuid::new_v4().to_string();
        let (support, reached, release, hits) = blocking_phase_support(
            redaction::SecureRedactionExecutionTestPhase::PublicationStarted,
        );
        let execution = execute_with_support(
            &app,
            &session,
            &plan,
            job_id.clone(),
            target.to_string_lossy().into_owned(),
            support,
        );
        let cancellation = async {
            tokio::task::spawn_blocking(move || reached.wait())
                .await
                .unwrap();
            document_session::cancel_session_job(session.id.clone(), job_id, app.state())
                .await
                .unwrap();
            tokio::task::spawn_blocking(move || release.wait())
                .await
                .unwrap();
        };
        let (outcome, ()) = tokio::join!(execution, cancellation);
        match outcome {
            SessionJobOutcome::Completed {
                value,
                cancel_requested,
                cleanup,
                ..
            } => {
                assert!(cancel_requested);
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                assert!(value.published);
                assert_eq!(sha256_file(&target), value.transform_report.output_digest);
            }
            other => panic!("commit-boundary disk truth must complete: {other:?}"),
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn protected_original_working_and_private_targets_fail_preflight() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(
            &app,
            &session,
            policy.clone(),
            acknowledgements.clone(),
            regions.clone(),
        )
        .await;
        let store = app.state::<document_session::DocumentSessionStore>();
        let paths = store
            .secure_redaction_export_paths_for_acceptance(&session.id)
            .unwrap();

        for (target, expected_code) in [
            (paths.original_path, "REDACTION_TARGET_PROTECTED_PATH"),
            (paths.working_path, "REDACTION_TARGET_PROTECTED_PATH"),
            (
                paths.session_dir.join("private-export.pdf"),
                "REDACTION_TARGET_PRIVATE_PATH",
            ),
        ] {
            let error =
                redaction::execute_secure_redaction_core(redaction::SecureRedactionExecutionArgs {
                    job_id: uuid::Uuid::new_v4().to_string(),
                    session_id: session.id.clone(),
                    expected_content_generation: session.content_generation,
                    policy: policy.clone(),
                    acknowledgements: acknowledgements.clone(),
                    regions: regions.clone(),
                    expected_plan_digest: plan.plan_digest.clone(),
                    target_path: target.to_string_lossy().into_owned(),
                    store: &store,
                })
                .await
                .unwrap_err();
            assert_eq!(error.code, expected_code);
        }
    }

    #[tokio::test]
    async fn save_as_waits_for_secure_export_read_lease_then_completes() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let (policy, acknowledgements, regions) = request_parts();
        let plan = plan(&app, &session, policy, acknowledgements, regions).await;
        let output = tempfile::tempdir().unwrap();
        let secure_target = output.path().join("secure-export.pdf");
        let save_as_target = output.path().join("concurrent-save-as.pdf");
        let (support, reached, release, _) =
            blocking_phase_support(redaction::SecureRedactionExecutionTestPhase::BeforePublication);
        let (lease_tx, lease_rx) = std::sync::mpsc::channel::<bool>();
        let acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquired_hook = Arc::clone(&acquired);
        app.state::<document_session::DocumentSessionStore>()
            .set_secure_redaction_save_lease_test_hook(Some(Arc::new(move |did_acquire| {
                if did_acquire {
                    acquired_hook.store(true, Ordering::SeqCst);
                }
                let _ = lease_tx.send(did_acquire);
            })));

        let execution = execute_with_support(
            &app,
            &session,
            &plan,
            uuid::Uuid::new_v4().to_string(),
            secure_target.to_string_lossy().into_owned(),
            support,
        );
        let save_as = document_session::save_document_session_as(
            session.id.clone(),
            session.state_revision,
            save_as_target.to_string_lossy().into_owned(),
            app.state(),
        );
        let acquired_during_coordination = Arc::clone(&acquired);
        let coordination = async move {
            tokio::task::spawn_blocking(move || reached.wait())
                .await
                .unwrap();
            let first = tokio::task::spawn_blocking(move || lease_rx.recv().unwrap())
                .await
                .unwrap();
            assert!(
                !first,
                "Save As must signal before awaiting the paired write lease"
            );
            assert!(!acquired_during_coordination.load(Ordering::SeqCst));
            tokio::task::spawn_blocking(move || release.wait())
                .await
                .unwrap();
        };

        let (execution_outcome, save_outcome, ()) = tokio::join!(execution, save_as, coordination);
        assert!(matches!(
            execution_outcome,
            SessionJobOutcome::Completed { .. }
        ));
        let saved = save_outcome.unwrap();
        assert!(acquired.load(Ordering::SeqCst));
        assert_eq!(saved.display_path, save_as_target.to_string_lossy());
        assert!(secure_target.is_file());
        assert!(save_as_target.is_file());
        app.state::<document_session::DocumentSessionStore>()
            .set_secure_redaction_save_lease_test_hook(None);
    }
}
