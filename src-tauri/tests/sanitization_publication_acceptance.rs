//! Real-process acceptance coverage for 8C.5 verified atomic publication.

#![cfg(feature = "sanitization-test-support")]

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
    use preview_alternative_lib::sanitization::{
        self, SanitizationAcknowledgements, SanitizationPolicy, SanitizationProfile,
    };
    use preview_alternative_lib::sanitization_publication::{
        self, SanitizationExecutionTestPhase, SanitizationExecutionTestSupport,
    };
    use preview_alternative_lib::{
        ActiveRenderTasks, ActiveSearchId, PasswordCache, SearchCache, TextLayoutCache,
    };
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use tauri::Manager;

    static EXECUTION_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sanitization/clean-minimal.pdf")
    }

    fn digest(path: &Path) -> String {
        preview_alternative_lib::hex_lower(Sha256::digest(std::fs::read(path).unwrap()))
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
    ) -> document_session::DocumentSessionSnapshot {
        let opened = document_session::open_document_session(
            fixture().to_string_lossy().into_owned(),
            app.state(),
        )
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

    fn policy() -> SanitizationPolicy {
        SanitizationPolicy {
            schema_version: sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            profile: SanitizationProfile::CustomSupportedV1,
            controls: vec!["incremental_revisions".to_string()],
            acknowledgements: SanitizationAcknowledgements {
                acknowledge_signature_invalidation: false,
            },
            expected_inspection_version: sanitization::SANITIZATION_INSPECTION_VERSION.to_string(),
        }
    }

    async fn execute(
        app: &tauri::App<tauri::test::MockRuntime>,
        session: &document_session::DocumentSessionSnapshot,
        target: &Path,
        support: Option<Arc<SanitizationExecutionTestSupport>>,
    ) -> SessionJobOutcome<sanitization::SanitizationExecutionReport> {
        sanitization_publication::execute_document_sanitization_core_with_test_support(
            uuid::Uuid::new_v4().to_string(),
            session.id.clone(),
            session.content_generation,
            policy(),
            target.to_string_lossy().into_owned(),
            &app.state::<document_session::DocumentSessionStore>(),
            support,
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

    fn assert_failed<T: std::fmt::Debug>(outcome: SessionJobOutcome<T>, code: &str) {
        match outcome {
            SessionJobOutcome::Failed { error, cleanup, .. } => {
                assert_eq!(error.code, code, "{error:?}");
                assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
            }
            other => panic!("expected typed failure, got {other:?}"),
        }
    }

    fn blocking_support(
        phase: SanitizationExecutionTestPhase,
    ) -> (
        Arc<SanitizationExecutionTestSupport>,
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
        let support = SanitizationExecutionTestSupport {
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
    async fn missing_and_existing_targets_publish_exact_verified_bytes_and_remain_session_neutral()
    {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let output = tempfile::tempdir().unwrap();
        for existing in [false, true] {
            let target = output.path().join(if existing {
                "existing-target.pdf"
            } else {
                "missing-target.pdf"
            });
            if existing {
                std::fs::write(&target, b"external-old-bytes").unwrap();
            }
            let outcome = execute(&app, &session, &target, None).await;
            let report = match outcome {
                SessionJobOutcome::Completed {
                    value,
                    cancel_requested,
                    cleanup,
                    ..
                } => {
                    assert!(!cancel_requested);
                    assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                    value
                }
                other => panic!("full execution must complete: {other:?}"),
            };
            let publication = report.publication.as_ref().unwrap();
            assert!(publication.published);
            assert!(report.verification.is_verified);
            assert_eq!(
                report.transform.output_digest,
                report.verification.transform_digest
            );
            assert_eq!(report.transform.output_digest, publication.published_digest);
            assert_eq!(report.transform.output_digest, digest(&target));
            assert!(!std::fs::read(&target)
                .unwrap()
                .windows(5)
                .any(|window| window == b"/Prev"));
            assert_eq!(
                report.assurance,
                sanitization::SanitizationAssurance::VerifiedCustom
            );
            let mut invalid = report.clone();
            invalid.publication.as_mut().unwrap().published = false;
            assert!(sanitization::validate_execution_report(&invalid, &policy()).is_err());
            let mut invalid = report.clone();
            invalid.publication.as_mut().unwrap().published_digest = "0".repeat(64);
            assert!(sanitization::validate_execution_report(&invalid, &policy()).is_err());
            let mut invalid = report;
            invalid.assurance = sanitization::SanitizationAssurance::NotPublished;
            assert!(sanitization::validate_execution_report(&invalid, &policy()).is_err());
        }
        let after =
            document_session::get_document_session_snapshot(session.id.clone(), app.state())
                .await
                .unwrap();
        assert_session_neutral(&session, &after);
    }

    #[tokio::test]
    async fn artifact_toctou_and_staged_corruption_fail_before_replacement_and_cleanup() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        for corrupt_staged in [false, true] {
            let app = app();
            let session = activated_session(&app).await;
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("preserved.pdf");
            std::fs::write(&target, b"preserve-external-writer").unwrap();
            let support = if corrupt_staged {
                SanitizationExecutionTestSupport {
                    corrupt_staged_temp: true,
                    ..Default::default()
                }
            } else {
                SanitizationExecutionTestSupport {
                    phase_hook: Some(Arc::new(|phase, artifact| {
                        if phase == SanitizationExecutionTestPhase::AfterVerifierValidated {
                            std::fs::OpenOptions::new()
                                .append(true)
                                .open(artifact.unwrap())
                                .unwrap()
                                .write_all(b"artifact-toctou")
                                .unwrap();
                        }
                    })),
                    ..Default::default()
                }
            };
            use std::io::Write as _;
            let outcome = execute(&app, &session, &target, Some(Arc::new(support))).await;
            assert_failed(outcome, "SANITIZATION_OUTPUT_DIGEST_MISMATCH");
            assert_eq!(std::fs::read(&target).unwrap(), b"preserve-external-writer");
            assert_eq!(
                std::fs::read_dir(output.path()).unwrap().count(),
                1,
                "adjacent staged temp must be cleaned"
            );
        }
    }

    #[tokio::test]
    async fn missing_and_existing_target_toctou_matrix_is_fail_closed() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        #[derive(Clone, Copy)]
        enum Race {
            MissingCreated,
            ExistingModified,
            ExistingReplacedSameDigest,
            ExistingDeletedRecreated,
        }
        for race in [
            Race::MissingCreated,
            Race::ExistingModified,
            Race::ExistingReplacedSameDigest,
            Race::ExistingDeletedRecreated,
        ] {
            let app = app();
            let session = activated_session(&app).await;
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("raced.pdf");
            if !matches!(race, Race::MissingCreated) {
                std::fs::write(&target, b"baseline-target").unwrap();
            }
            let hook_target = target.clone();
            let support = SanitizationExecutionTestSupport {
                phase_hook: Some(Arc::new(move |phase, _| {
                    if phase != SanitizationExecutionTestPhase::BeforePublication {
                        return;
                    }
                    match race {
                        Race::MissingCreated => {
                            std::fs::write(&hook_target, b"external-created").unwrap();
                        }
                        Race::ExistingModified => {
                            std::fs::write(&hook_target, b"external-modified").unwrap();
                        }
                        Race::ExistingReplacedSameDigest => {
                            let replacement = hook_target.with_extension("replacement");
                            std::fs::write(&replacement, b"baseline-target").unwrap();
                            std::fs::rename(replacement, &hook_target).unwrap();
                        }
                        Race::ExistingDeletedRecreated => {
                            std::fs::remove_file(&hook_target).unwrap();
                            std::fs::write(&hook_target, b"baseline-target").unwrap();
                        }
                    }
                })),
                ..Default::default()
            };
            let outcome = execute(&app, &session, &target, Some(Arc::new(support))).await;
            assert_failed(outcome, "SANITIZATION_TARGET_CONFLICT");
            let expected = match race {
                Race::MissingCreated => b"external-created".as_slice(),
                Race::ExistingModified => b"external-modified".as_slice(),
                Race::ExistingReplacedSameDigest | Race::ExistingDeletedRecreated => {
                    b"baseline-target".as_slice()
                }
            };
            assert_eq!(std::fs::read(&target).unwrap(), expected);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn protected_private_symlink_hardlink_and_directory_targets_are_rejected_preflight() {
        use std::os::unix::fs::symlink;

        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let store = app.state::<document_session::DocumentSessionStore>();
        let paths = store
            .secure_redaction_export_paths_for_acceptance(&session.id)
            .unwrap();
        let output = tempfile::tempdir().unwrap();
        let regular = output.path().join("regular.pdf");
        std::fs::write(&regular, b"regular").unwrap();
        let symlink_path = output.path().join("symlink.pdf");
        symlink(&regular, &symlink_path).unwrap();
        let dangling = output.path().join("dangling.pdf");
        symlink(output.path().join("missing.pdf"), &dangling).unwrap();
        let hardlink = output.path().join("hardlink.pdf");
        std::fs::hard_link(&regular, &hardlink).unwrap();
        let directory = output.path().join("directory.pdf");
        std::fs::create_dir(&directory).unwrap();

        for (target, code) in [
            (paths.original_path, "SANITIZATION_TARGET_PROTECTED_PATH"),
            (paths.working_path, "SANITIZATION_TARGET_PROTECTED_PATH"),
            (
                paths.session_dir.join("private-output.pdf"),
                "SANITIZATION_TARGET_PRIVATE_PATH",
            ),
            (symlink_path, "SANITIZATION_TARGET_PREFLIGHT_FAILED"),
            (dangling, "SANITIZATION_TARGET_PREFLIGHT_FAILED"),
            (hardlink, "SANITIZATION_TARGET_PREFLIGHT_FAILED"),
            (directory, "SANITIZATION_TARGET_PREFLIGHT_FAILED"),
        ] {
            let error = sanitization_publication::execute_document_sanitization_core(
                uuid::Uuid::new_v4().to_string(),
                session.id.clone(),
                session.content_generation,
                policy(),
                target.to_string_lossy().into_owned(),
                &store,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, code, "target={}", target.display());
        }
    }

    #[tokio::test]
    async fn malformed_verification_never_starts_publication() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("not-published.pdf");
        let publication_started = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&publication_started);
        let support = SanitizationExecutionTestSupport {
            verifier_result_hook: Some(Arc::new(|result| match result {
                preview_alternative_lib::job_worker::JobResult::SanitizationVerification(
                    mut report,
                ) => {
                    report.is_verified = false;
                    preview_alternative_lib::job_worker::JobResult::SanitizationVerification(report)
                }
                other => other,
            })),
            phase_hook: Some(Arc::new(move |phase, _| {
                if phase == SanitizationExecutionTestPhase::PublicationStarted {
                    observed.store(true, Ordering::SeqCst);
                }
            })),
            ..Default::default()
        };
        let outcome = execute(&app, &session, &target, Some(Arc::new(support))).await;
        assert_failed(outcome, "SANITIZATION_VERIFICATION_REPORT_INVALID");
        assert!(!publication_started.load(Ordering::SeqCst));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn verifier_crash_and_timeout_never_start_publication() {
        use preview_alternative_lib::sanitization_verifier::SanitizationVerificationTestBehavior;

        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        for (behavior, timeout, expected_code) in [
            (
                SanitizationVerificationTestBehavior::CrashDuringTraversal,
                std::time::Duration::from_secs(10),
                "SESSION_JOB_WORKER_CRASHED",
            ),
            (
                SanitizationVerificationTestBehavior::HangDuringTraversal,
                std::time::Duration::from_secs(1),
                "DOCUMENT_SESSION_PDF_ENGINE_ERROR",
            ),
        ] {
            let app = app();
            let session = activated_session(&app).await;
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("verifier-failed.pdf");
            let publication_started = Arc::new(AtomicBool::new(false));
            let observed = Arc::clone(&publication_started);
            let support = SanitizationExecutionTestSupport {
                verification_behavior: behavior,
                verification_timeout: timeout,
                phase_hook: Some(Arc::new(move |phase, _| {
                    if phase == SanitizationExecutionTestPhase::PublicationStarted {
                        observed.store(true, Ordering::SeqCst);
                    }
                })),
                ..Default::default()
            };
            let outcome = execute(&app, &session, &target, Some(Arc::new(support))).await;
            assert_failed(outcome, expected_code);
            assert!(!publication_started.load(Ordering::SeqCst));
            assert!(!target.exists());
        }
    }

    #[tokio::test]
    async fn atomic_fault_matrix_preserves_exact_typed_disk_truth_and_recovery_metadata() {
        use preview_alternative_lib::document_io::VerifiedArtifactPublicationTestFault as Fault;

        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        for (fault, expected_state, replacement_committed) in [
            (Fault::TempCreate, "prepared", false),
            (Fault::CopyWrite, "original_preserved", false),
            (Fault::TempSync, "original_preserved", false),
            (Fault::Replace, "original_preserved", false),
            (Fault::PostCommitIdentity, "indeterminate", true),
            (Fault::PostCommitDigest, "indeterminate", true),
            (
                Fault::RecoveryRequiredAfterCommit,
                "recovery_required",
                true,
            ),
        ] {
            let app = app();
            let session = activated_session(&app).await;
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("fault-target.pdf");
            std::fs::write(&target, b"original-target-bytes").unwrap();
            let support = SanitizationExecutionTestSupport {
                publication_fault: fault,
                ..Default::default()
            };
            let outcome = execute(&app, &session, &target, Some(Arc::new(support))).await;
            match outcome {
                SessionJobOutcome::Failed { error, cleanup, .. } => {
                    assert_eq!(error.code, "SANITIZATION_PUBLICATION_FAILED");
                    assert_eq!(error.commit_state.as_deref(), Some(expected_state));
                    assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                    if replacement_committed {
                        assert_ne!(std::fs::read(&target).unwrap(), b"original-target-bytes");
                        assert!(error.replacement_digest.is_some());
                    } else {
                        assert_eq!(std::fs::read(&target).unwrap(), b"original-target-bytes");
                    }
                    if fault == Fault::RecoveryRequiredAfterCommit {
                        let recovery = PathBuf::from(&*error.recovery_path.unwrap());
                        assert!(recovery.is_file());
                        assert_eq!(std::fs::read(recovery).unwrap(), b"original-target-bytes");
                    } else {
                        assert!(error.recovery_path.is_none());
                    }
                }
                other => panic!("fault must produce typed failure: {other:?}"),
            }
            assert_eq!(
                std::fs::read_dir(output.path())
                    .unwrap()
                    .filter(|entry| entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".antigravity-pdf-"))
                    .count(),
                0,
                "ordinary adjacent staged temps must be cleaned"
            );
        }

        let app = app();
        let session = activated_session(&app).await;
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("durability-target.pdf");
        std::fs::write(&target, b"original-target-bytes").unwrap();
        let support = SanitizationExecutionTestSupport {
            publication_fault: Fault::ParentDurability,
            ..Default::default()
        };
        let outcome = execute(&app, &session, &target, Some(Arc::new(support))).await;
        match outcome {
            SessionJobOutcome::Completed { value, .. } => assert!(matches!(
                value.publication.unwrap().commit_state,
                sanitization::SanitizationPublicationCommitState::NewCommittedDurabilityUnconfirmed
            )),
            other => panic!("durability uncertainty must preserve committed success: {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancellation_before_publication_cancels_but_after_commit_boundary_disk_truth_wins() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        for (phase, expect_completed) in [
            (SanitizationExecutionTestPhase::BeforePublication, false),
            (SanitizationExecutionTestPhase::PublicationStarted, true),
        ] {
            let app = app();
            let session = activated_session(&app).await;
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("cancel-boundary.pdf");
            let job_id = uuid::Uuid::new_v4().to_string();
            let (support, reached, release, _) = blocking_support(phase);
            let store = app.state::<document_session::DocumentSessionStore>();
            let execution =
                sanitization_publication::execute_document_sanitization_core_with_test_support(
                    job_id.clone(),
                    session.id.clone(),
                    session.content_generation,
                    policy(),
                    target.to_string_lossy().into_owned(),
                    &store,
                    Some(support),
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
            let outcome = outcome.unwrap();
            if expect_completed {
                match outcome {
                    SessionJobOutcome::Completed {
                        value,
                        cancel_requested,
                        ..
                    } => {
                        assert!(cancel_requested);
                        assert!(value.publication.unwrap().published);
                    }
                    other => panic!("post-boundary cancellation must preserve commit: {other:?}"),
                }
                assert!(target.is_file());
            } else {
                assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
                assert!(!target.exists());
            }
        }
    }

    #[tokio::test]
    async fn save_as_waits_for_verified_export_guard_then_completes() {
        let _serial = EXECUTION_TEST_MUTEX.lock().await;
        let app = app();
        let session = activated_session(&app).await;
        let output = tempfile::tempdir().unwrap();
        let target = output.path().join("sanitized.pdf");
        let save_as_target = output.path().join("save-as.pdf");
        let (support, reached, release, _) =
            blocking_support(SanitizationExecutionTestPhase::BeforePublication);
        let (lease_tx, lease_rx) = std::sync::mpsc::channel::<bool>();
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired_hook = Arc::clone(&acquired);
        app.state::<document_session::DocumentSessionStore>()
            .set_verified_artifact_save_lease_test_hook(Some(Arc::new(move |did_acquire| {
                if did_acquire {
                    acquired_hook.store(true, Ordering::SeqCst);
                }
                let _ = lease_tx.send(did_acquire);
            })));

        let execution = execute(&app, &session, &target, Some(support));
        let save_as = document_session::save_document_session_as(
            session.id.clone(),
            session.state_revision,
            save_as_target.to_string_lossy().into_owned(),
            app.state(),
        );
        let acquired_during = Arc::clone(&acquired);
        let coordination = async move {
            tokio::task::spawn_blocking(move || reached.wait())
                .await
                .unwrap();
            let first = tokio::task::spawn_blocking(move || lease_rx.recv().unwrap())
                .await
                .unwrap();
            assert!(!first);
            assert!(!acquired_during.load(Ordering::SeqCst));
            tokio::task::spawn_blocking(move || release.wait())
                .await
                .unwrap();
        };
        let (execution_outcome, save_outcome, ()) = tokio::join!(execution, save_as, coordination);
        assert!(matches!(
            execution_outcome,
            SessionJobOutcome::Completed { .. }
        ));
        assert_eq!(
            save_outcome.unwrap().display_path,
            save_as_target.to_string_lossy()
        );
        assert!(acquired.load(Ordering::SeqCst));
        assert!(target.is_file());
        assert!(save_as_target.is_file());
        app.state::<document_session::DocumentSessionStore>()
            .set_verified_artifact_save_lease_test_hook(None);
    }
}
