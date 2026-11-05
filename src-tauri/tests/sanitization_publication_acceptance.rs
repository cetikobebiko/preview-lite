//! Real-process acceptance coverage for 8C.5 verified atomic publication.
//!
//! The suite runs on every target that ships a PDFium runtime
//! (docs/engineering/platform-support.json): macOS arm64, Linux x86_64 and
//! Windows x86_64. macOS x86_64 has no PDFium runtime and is not a supported
//! target, so the suite compiles to zero tests there by design.

#![cfg(feature = "sanitization-test-support")]

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64"),
))]
mod pdfium_runtime_targets {
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
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;
    use tauri::Manager;

    /// Upper bound for one side of a phase handshake. Generous for a cold
    /// worker start on a slow runner; it only bounds a broken run, a passing
    /// run never waits for it.
    const PHASE_TIMEOUT: Duration = Duration::from_secs(300);

    static EXECUTION_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sanitization/clean-minimal.pdf")
    }

    fn digest(path: &Path) -> String {
        preview_alternative_lib::hex_lower(Sha256::digest(std::fs::read(path).unwrap()))
    }

    fn app() -> Rc<TestApp> {
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
        let app = Rc::new(app);
        TEST_APPS.with(|apps| apps.borrow_mut().push(Rc::clone(&app)));
        app
    }

    type TestApp = tauri::App<tauri::test::MockRuntime>;

    thread_local! {
        /// Apps built by the running test. `with_worker_shutdown` shuts each
        /// one down the way the application does on exit.
        static TEST_APPS: std::cell::RefCell<Vec<Rc<TestApp>>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Runs a test body, then shuts down every app it built with
    /// `shutdown_all_resources` (what `RunEvent::ExitRequested` does), even
    /// when the body panicked, and only then re-raises the panic.
    ///
    /// Without it the PDFium worker outlives the test: on Windows tokio
    /// reads the worker's anonymous pipes on blocking-pool threads, and the
    /// `#[tokio::test]` runtime waits forever for those reads when it is
    /// dropped. The suite then hung in its first test with no output.
    async fn with_worker_shutdown(body: impl std::future::Future<Output = ()>) {
        use futures_util::FutureExt as _;
        let outcome = std::panic::AssertUnwindSafe(body).catch_unwind().await;
        let apps = TEST_APPS.with(|apps| std::mem::take(&mut *apps.borrow_mut()));
        for app in apps {
            let store = app.state::<document_session::DocumentSessionStore>();
            let _ = within(
                "shutdown_all_resources",
                store.shutdown_all_resources(&app.state(), &app.state()),
            )
            .await;
        }
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    async fn activated_session(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> document_session::DocumentSessionSnapshot {
        activated_session_for(app, &fixture()).await
    }

    async fn activated_session_for(
        app: &tauri::App<tauri::test::MockRuntime>,
        path: &Path,
    ) -> document_session::DocumentSessionSnapshot {
        let opened = within(
            "open_document_session",
            document_session::open_document_session(
                path.to_string_lossy().into_owned(),
                app.state(),
            ),
        )
        .await
        .unwrap();
        let prepared = within(
            "prepare_candidate_activation",
            document_session::prepare_candidate_activation(
                opened.session_id,
                None,
                None,
                app.state(),
            ),
        )
        .await
        .unwrap();
        within(
            "commit_candidate_activation",
            document_session::commit_candidate_activation(
                prepared.activation_token,
                app.state(),
                app.state(),
                app.state(),
                app.state(),
            ),
        )
        .await
        .unwrap()
        .snapshot
    }

    /// Bounds one session/worker phase so a run that never returns fails
    /// naming the phase instead of hanging until the CI step timeout.
    async fn within<T>(phase: &str, future: impl std::future::Future<Output = T>) -> T {
        match tokio::time::timeout(std::time::Duration::from_secs(180), future).await {
            Ok(value) => value,
            Err(_) => panic!("HANG: {phase} did not return within 180 s"),
        }
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
        within(
            "execute_document_sanitization",
            sanitization_publication::execute_document_sanitization_core_with_test_support(
                uuid::Uuid::new_v4().to_string(),
                session.id.clone(),
                session.content_generation,
                policy(),
                target.to_string_lossy().into_owned(),
                &app.state::<document_session::DocumentSessionStore>(),
                support,
            ),
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

    /// Test side of a phase hold: `reached` fires when execution stops at the
    /// phase, `release` lets it continue.
    struct PhaseGate {
        reached: mpsc::Receiver<()>,
        release: mpsc::Sender<()>,
    }

    impl PhaseGate {
        /// `true` once execution is held at the phase. Channels rather than
        /// barriers: when execution fails before the phase, the support and
        /// its sender are dropped and this returns `false` at once, so the
        /// test reports the outcome instead of hanging the whole suite.
        async fn wait_reached(reached: mpsc::Receiver<()>) -> bool {
            tokio::task::spawn_blocking(move || reached.recv_timeout(PHASE_TIMEOUT).is_ok())
                .await
                .unwrap()
        }
    }

    fn blocking_support(
        phase: SanitizationExecutionTestPhase,
    ) -> (
        Arc<SanitizationExecutionTestSupport>,
        PhaseGate,
        Arc<AtomicUsize>,
    ) {
        let (reached_tx, reached) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        let hits = Arc::new(AtomicUsize::new(0));
        let hook_hits = Arc::clone(&hits);
        let support = SanitizationExecutionTestSupport {
            phase_hook: Some(Arc::new(move |observed, _| {
                if observed == phase {
                    hook_hits.fetch_add(1, Ordering::SeqCst);
                    let _ = reached_tx.send(());
                    let _ = release_rx.lock().unwrap().recv_timeout(PHASE_TIMEOUT);
                }
            })),
            ..Default::default()
        };
        (Arc::new(support), PhaseGate { reached, release }, hits)
    }

    /// Same file on disk. Display paths may differ in spelling from the
    /// requested path (Windows 8.3 short names such as `RUNNER~1` in the temp
    /// directory, or a `\\?\` verbatim prefix), so compare canonical forms.
    fn same_file(left: &Path, right: &Path) -> bool {
        match (left.canonicalize(), right.canonicalize()) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }

    /// Creates a file symlink. Windows needs SeCreateSymbolicLinkPrivilege (an
    /// elevated token or Developer Mode). Without it the symlink and dangling
    /// symlink preflight cases cannot run, and the test fails saying so
    /// rather than skipping them.
    fn create_file_symlink(original: &Path, link: &Path) {
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(original, link);
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_file(original, link);
        if let Err(error) = result {
            // ERROR_PRIVILEGE_NOT_HELD
            if cfg!(windows) && error.raw_os_error() == Some(1314) {
                panic!(
                    "NOT RUN: this Windows host cannot create symlinks \
                     (ERROR_PRIVILEGE_NOT_HELD); the symlink and dangling-symlink \
                     preflight cases need an elevated token or Developer Mode"
                );
            }
            panic!("symlink creation failed: {error}");
        }
    }

    #[tokio::test]
    async fn missing_and_existing_targets_publish_exact_verified_bytes_and_remain_session_neutral()
    {
        with_worker_shutdown(async {
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
        })
        .await;
    }

    #[tokio::test]
    async fn artifact_toctou_and_staged_corruption_fail_before_replacement_and_cleanup() {
        with_worker_shutdown(async {
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
        })
        .await;
    }

    #[tokio::test]
    async fn missing_and_existing_target_toctou_matrix_is_fail_closed() {
        with_worker_shutdown(async {
            let _serial = EXECUTION_TEST_MUTEX.lock().await;
            #[derive(Clone, Copy, Debug)]
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
                match outcome {
                    SessionJobOutcome::Failed { error, cleanup, .. } => {
                        assert_eq!(
                            error.code, "SANITIZATION_TARGET_CONFLICT",
                            "{race:?}: {error:?}"
                        );
                        assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                    }
                    other => panic!(
                        "{race:?}: the target race was not detected, publication outcome: {other:?}"
                    ),
                }
                let expected = match race {
                    Race::MissingCreated => b"external-created".as_slice(),
                    Race::ExistingModified => b"external-modified".as_slice(),
                    Race::ExistingReplacedSameDigest | Race::ExistingDeletedRecreated => {
                        b"baseline-target".as_slice()
                    }
                };
                assert_eq!(std::fs::read(&target).unwrap(), expected);
            }
        })
        .await;
    }

    #[tokio::test]
    async fn protected_private_symlink_hardlink_and_directory_targets_are_rejected_preflight() {
        with_worker_shutdown(async {
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
            create_file_symlink(&regular, &symlink_path);
            let dangling = output.path().join("dangling.pdf");
            create_file_symlink(&output.path().join("missing.pdf"), &dangling);
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
        })
        .await;
    }

    #[tokio::test]
    async fn malformed_verification_never_starts_publication() {
        with_worker_shutdown(async {
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
                        preview_alternative_lib::job_worker::JobResult::SanitizationVerification(
                            report,
                        )
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
        })
        .await;
    }

    #[tokio::test]
    async fn verifier_crash_and_timeout_never_start_publication() {
        with_worker_shutdown(async {
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
        })
        .await;
    }

    #[tokio::test]
    async fn atomic_fault_matrix_preserves_exact_typed_disk_truth_and_recovery_metadata() {
        with_worker_shutdown(async {
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
        })
        .await;
    }

    #[tokio::test]
    async fn cancellation_before_publication_cancels_but_after_commit_boundary_disk_truth_wins() {
        with_worker_shutdown(async {
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
                let (support, gate, _) = blocking_support(phase);
                let PhaseGate { reached, release } = gate;
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
                    let held = PhaseGate::wait_reached(reached).await;
                    if held {
                        document_session::cancel_session_job(
                            session.id.clone(),
                            job_id,
                            app.state(),
                        )
                        .await
                        .unwrap();
                    }
                    let _ = release.send(());
                    held
                };
                let (outcome, held) = tokio::join!(execution, cancellation);
                let outcome = outcome.unwrap();
                assert!(
                    held,
                    "execution never reached {phase:?}; outcome: {outcome:?}"
                );
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
                        other => {
                            panic!("post-boundary cancellation must preserve commit: {other:?}")
                        }
                    }
                    assert!(target.is_file());
                } else {
                    assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
                    assert!(!target.exists());
                }
            }
        })
        .await;
    }

    #[tokio::test]
    async fn save_as_waits_for_verified_export_guard_then_completes() {
        with_worker_shutdown(async {
            let _serial = EXECUTION_TEST_MUTEX.lock().await;
            let app = app();
            let session = activated_session(&app).await;
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("sanitized.pdf");
            let save_as_target = output.path().join("save-as.pdf");
            let (support, gate, _) =
                blocking_support(SanitizationExecutionTestPhase::BeforePublication);
            let PhaseGate { reached, release } = gate;
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
                let held = PhaseGate::wait_reached(reached).await;
                let first_lease_attempt = if held {
                    tokio::task::spawn_blocking(move || lease_rx.recv_timeout(PHASE_TIMEOUT).ok())
                        .await
                        .unwrap()
                } else {
                    None
                };
                let acquired_while_held = acquired_during.load(Ordering::SeqCst);
                let _ = release.send(());
                (held, first_lease_attempt, acquired_while_held)
            };
            let (execution_outcome, save_outcome, (held, first_lease_attempt, acquired_while_held)) =
                tokio::join!(execution, save_as, coordination);
            assert!(
                held,
                "execution never reached BeforePublication; outcome: {execution_outcome:?}"
            );
            assert_eq!(
                first_lease_attempt,
                Some(false),
                "Save As must find the verified-export lease held while publication is pending"
            );
            assert!(!acquired_while_held);
            assert!(matches!(
                execution_outcome,
                SessionJobOutcome::Completed { .. }
            ));
            let display_path = save_outcome.unwrap().display_path;
            assert!(
                same_file(Path::new(&display_path), &save_as_target),
                "display path {display_path} is not the Save As target {}",
                save_as_target.display()
            );
            assert!(acquired.load(Ordering::SeqCst));
            assert!(target.is_file());
            assert!(save_as_target.is_file());
            app.state::<document_session::DocumentSessionStore>()
                .set_verified_artifact_save_lease_test_hook(None);
        })
        .await;
    }

    // Payloads that live only in superseded revisions once the session's own
    // incremental mutations have run.
    const OLD_INFO_TITLE: &str = "SANITIZATION-OLD-INFO-TITLE-7731";
    const OLD_INFO_AUTHOR: &str = "SANITIZATION-OLD-INFO-AUTHOR-7731";
    const DELETED_NOTE: &str = "SANITIZATION-DELETED-NOTE-7731";
    const CURRENT_INFO_TITLE: &str = "Current title after history";

    fn hex(bytes: &[u8], upper: bool) -> Vec<u8> {
        bytes
            .iter()
            .map(|byte| {
                if upper {
                    format!("{byte:02X}")
                } else {
                    format!("{byte:02x}")
                }
            })
            .collect::<String>()
            .into_bytes()
    }

    /// Byte spellings a PDF writer can give a text string: PDFDocEncoding /
    /// ASCII literal, UTF-16BE (with and without BOM), and hex strings of both.
    fn payload_spellings(payload: &str) -> Vec<Vec<u8>> {
        let ascii = payload.as_bytes().to_vec();
        let utf16 = payload
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        let mut utf16_bom = vec![0xFE, 0xFF];
        utf16_bom.extend_from_slice(&utf16);
        let mut spellings = vec![ascii.clone(), utf16.clone()];
        for upper in [false, true] {
            spellings.push(hex(&ascii, upper));
            spellings.push(hex(&utf16, upper));
            spellings.push(hex(&utf16_bom, upper));
        }
        spellings
    }

    fn contains_any(haystack: &[u8], needles: &[Vec<u8>]) -> bool {
        needles.iter().any(|needle| {
            haystack
                .windows(needle.len())
                .any(|window| window == needle.as_slice())
        })
    }

    /// Every place a payload can survive in a file: the raw bytes, every
    /// decoded string object (literal or hex, any revision lopdf loaded) and
    /// every decompressed stream, object streams included.
    fn payload_survives(bytes: &[u8], payload: &str) -> bool {
        let spellings = payload_spellings(payload);
        if contains_any(bytes, &spellings) {
            return true;
        }
        fn object_has(object: &lopdf::Object, spellings: &[Vec<u8>]) -> bool {
            match object {
                lopdf::Object::String(value, _) => contains_any(value, spellings),
                lopdf::Object::Array(items) => items.iter().any(|item| object_has(item, spellings)),
                lopdf::Object::Dictionary(dict) => {
                    dict.iter().any(|(_, value)| object_has(value, spellings))
                }
                lopdf::Object::Stream(stream) => {
                    stream
                        .dict
                        .iter()
                        .any(|(_, value)| object_has(value, spellings))
                        || contains_any(&stream.content, spellings)
                        || stream
                            .decompressed_content()
                            .is_ok_and(|content| contains_any(&content, spellings))
                }
                _ => false,
            }
        }
        let document = lopdf::Document::load_mem(bytes).unwrap();
        document
            .objects
            .values()
            .any(|object| object_has(object, &spellings))
    }

    fn info_title(bytes: &[u8]) -> Option<String> {
        let document = lopdf::Document::load_mem(bytes).unwrap();
        let info = match document.trailer.get(b"Info").ok()? {
            lopdf::Object::Reference(id) => document.get_dictionary(*id).ok()?,
            lopdf::Object::Dictionary(dict) => dict,
            _ => return None,
        };
        lopdf::decode_text_string(info.get(b"Title").ok()?).ok()
    }

    #[tokio::test]
    async fn app_incremental_revision_payloads_are_absent_from_the_verified_publication() {
        with_worker_shutdown(async {
            let _serial = EXECUTION_TEST_MUTEX.lock().await;
            let app = app();
            let opened = activated_session(&app).await;
            let session_id = opened.id.clone();

            // The app's own incremental mutations: old /Info values, a note that
            // is added and then deleted, then the /Info values the user keeps.
            let with_old_info = document_session::update_session_metadata(
                session_id.clone(),
                opened.content_generation,
                serde_json::from_value(serde_json::json!({
                    "title": OLD_INFO_TITLE,
                    "author": OLD_INFO_AUTHOR,
                    "subject": "",
                    "keywords": "",
                }))
                .unwrap(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
            let with_note = document_session::add_session_annotation(
                session_id.clone(),
                with_old_info.content_generation,
                0,
                "Text".to_string(),
                serde_json::from_value(serde_json::json!({
                    "x": 0.1, "y": 0.1, "width": 0.05, "height": 0.05,
                }))
                .unwrap(),
                "#FFCC00".to_string(),
                Some(DELETED_NOTE.to_string()),
                None,
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
            let annotations = document_session::get_session_page_annotations(
                session_id.clone(),
                with_note.content_generation,
                0,
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
            let note = annotations
                .iter()
                .find(|annotation| annotation.contents.as_deref() == Some(DELETED_NOTE))
                .expect("the added note must be listed before it is deleted");
            let without_note = document_session::delete_session_annotation(
                session_id.clone(),
                with_note.content_generation,
                0,
                note.id.clone(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
            let session = document_session::update_session_metadata(
                session_id.clone(),
                without_note.content_generation,
                serde_json::from_value(serde_json::json!({
                    "title": CURRENT_INFO_TITLE,
                    "author": "",
                    "subject": "",
                    "keywords": "",
                }))
                .unwrap(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
            assert!(document_session::get_session_page_annotations(
                session_id.clone(),
                session.content_generation,
                0,
                app.state(),
                app.state(),
            )
            .await
            .unwrap()
            .iter()
            .all(|annotation| annotation.contents.as_deref() != Some(DELETED_NOTE)));

            // Precondition: the working copy is an incremental file whose
            // superseded revisions still carry every payload.
            let working_path = app
                .state::<document_session::DocumentSessionStore>()
                .secure_redaction_export_paths_for_acceptance(&session_id)
                .unwrap()
                .working_path;
            let working = std::fs::read(&working_path).unwrap();
            assert!(
                working
                    .windows(5)
                    .filter(|window| *window == b"%%EOF")
                    .count()
                    > 1,
                "the session mutations must have been saved as incremental revisions"
            );
            for payload in [OLD_INFO_TITLE, OLD_INFO_AUTHOR, DELETED_NOTE] {
                assert!(
                    payload_survives(&working, payload),
                    "precondition: {payload} must still be in an old revision of the working copy"
                );
            }
            assert_eq!(info_title(&working).as_deref(), Some(CURRENT_INFO_TITLE));

            let inspection = sanitization::inspect_document_sanitization_core_with_test_support(
                uuid::Uuid::new_v4().to_string(),
                session.id.clone(),
                session.content_generation,
                sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
                &app.state::<document_session::DocumentSessionStore>(),
                Duration::from_secs(180),
                preview_alternative_lib::sanitization_inspection::SanitizationInspectionTestBehavior::Real,
            )
            .await
            .unwrap();
            let inspection = match inspection {
                SessionJobOutcome::Completed { value, .. } => value,
                other => panic!("inspection of the incremental working copy failed: {other:?}"),
            };
            assert!(inspection.document_facts.incremental_revisions_detected);
            let history = inspection
                .controls
                .iter()
                .find(|control| control.control_id == "incremental_revisions")
                .unwrap();
            assert!(history.present && history.finding_count > 0);

            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("history-removed.pdf");
            let report = match execute(&app, &session, &target, None).await {
                SessionJobOutcome::Completed { value, cleanup, .. } => {
                    assert!(matches!(cleanup, SessionJobCleanupOutcome::Cleaned));
                    value
                }
                other => panic!("sanitizing the incremental working copy failed: {other:?}"),
            };

            // The independent verifier's own verdict on the published bytes.
            let verification = &report.verification;
            assert!(verification.is_verified);
            assert!(verification.blockers.is_empty());
            assert!(verification.unresolved_findings.is_empty());
            for (name, check) in [
                ("full_rewrite", &verification.full_rewrite),
                (
                    "incremental_history_removed",
                    &verification.incremental_history_removed,
                ),
                ("object_graph_scan", &verification.object_graph_scan),
                ("removed_payload_scan", &verification.removed_payload_scan),
                ("parser_reopen", &verification.parser_reopen),
                ("pdfium_reopen", &verification.pdfium_reopen),
                ("render_smoke", &verification.render_smoke),
            ] {
                assert_eq!(
                    check.status,
                    sanitization::SanitizationVerificationStatus::Passed,
                    "{name}: {check:?}"
                );
            }
            let control = verification
                .controls
                .iter()
                .find(|control| control.control_id == "incremental_revisions")
                .unwrap();
            assert!(control.detected_before);
            assert_eq!(control.forbidden_findings_after, 0);
            assert_eq!(
                control.status,
                sanitization::SanitizationVerificationStatus::Passed
            );
            let publication = report.publication.as_ref().unwrap();
            assert!(publication.published);
            assert_eq!(publication.published_digest, verification.transform_digest);
            assert_eq!(digest(&target), verification.transform_digest);

            // And on the published bytes themselves: one revision, no history
            // pointer, none of the superseded payloads in any spelling, and the
            // current /Info value kept (history removal, not metadata stripping).
            let published = std::fs::read(&target).unwrap();
            assert_eq!(
                published
                    .windows(5)
                    .filter(|window| *window == b"%%EOF")
                    .count(),
                1
            );
            assert!(!published.windows(5).any(|window| window == b"/Prev"));
            for payload in [OLD_INFO_TITLE, OLD_INFO_AUTHOR, DELETED_NOTE] {
                assert!(
                    !payload_survives(&published, payload),
                    "{payload} from a superseded revision survived publication"
                );
            }
            assert_eq!(info_title(&published).as_deref(), Some(CURRENT_INFO_TITLE));

            let after =
                document_session::get_document_session_snapshot(session.id.clone(), app.state())
                    .await
                    .unwrap();
            assert_session_neutral(&session, &after);
        })
        .await;
    }

    /// The controls the panel's minimum profile selects (every control with
    /// production removal support, sanitization-controls-v1.json).
    const UI_MINIMUM_PROFILE_CONTROLS: [&str; 16] = [
        "document_javascript",
        "additional_actions",
        "annotation_actions",
        "form_actions",
        "launch_actions",
        "uri_actions",
        "remote_navigation_actions",
        "form_submission_actions",
        "embedded_files",
        "associated_files",
        "file_attachment_annotations",
        "document_info",
        "xmp_metadata",
        "page_metadata",
        "object_metadata",
        "incremental_revisions",
    ];

    #[tokio::test]
    async fn ui_minimum_profile_removes_name_tree_javascript_and_embedded_files_and_keeps_destinations(
    ) {
        // Found in the running app: every document with an /EmbeddedFiles name
        // tree failed verification (preservation_checks) because all /Names leaf
        // arrays were counted as named destinations.
        with_worker_shutdown(async {
            let _serial = EXECUTION_TEST_MUTEX.lock().await;
            let app = app();
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/sanitization/combined-names.pdf");
            let session = activated_session_for(&app, &source).await;
            let policy = SanitizationPolicy {
                schema_version: sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
                profile: SanitizationProfile::VerifiedLimitedV1,
                controls: UI_MINIMUM_PROFILE_CONTROLS
                    .iter()
                    .map(|control| control.to_string())
                    .collect(),
                acknowledgements: SanitizationAcknowledgements {
                    acknowledge_signature_invalidation: false,
                },
                expected_inspection_version: sanitization::SANITIZATION_INSPECTION_VERSION
                    .to_string(),
            };
            let output = tempfile::tempdir().unwrap();
            let target = output.path().join("combined-names.sanitized.pdf");
            let outcome = within(
                "execute_document_sanitization",
                sanitization_publication::execute_document_sanitization_core(
                    uuid::Uuid::new_v4().to_string(),
                    session.id.clone(),
                    session.content_generation,
                    policy,
                    target.to_string_lossy().into_owned(),
                    &app.state::<document_session::DocumentSessionStore>(),
                ),
            )
            .await
            .unwrap();
            let report = match outcome {
                SessionJobOutcome::Completed { value, .. } => value,
                other => panic!("UI minimum profile must publish: {other:?}"),
            };
            assert!(report.verification.is_verified);
            assert_eq!(
                report.verification.preservation_checks.status,
                sanitization::SanitizationVerificationStatus::Passed
            );
            assert_eq!(
                report.assurance,
                sanitization::SanitizationAssurance::VerifiedLimited
            );
            assert!(report.publication.as_ref().unwrap().published);
            assert_eq!(digest(&target), report.verification.transform_digest);

            let published = std::fs::read(&target).unwrap();
            for payload in [
                "SANITIZATION_COMBINED_NAMES_JS_001",
                "SANITIZATION_COMBINED_NAMES_EMBEDDED_001",
            ] {
                assert!(
                    !payload_survives(&published, payload),
                    "{payload} survived publication"
                );
            }
            let document = lopdf::Document::load_mem(&published).unwrap();
            let names = match document.catalog().unwrap().get(b"Names").unwrap() {
                lopdf::Object::Reference(id) => document.get_dictionary(*id).unwrap(),
                lopdf::Object::Dictionary(names) => names,
                other => panic!("unexpected /Names {other:?}"),
            };
            assert!(names.has(b"Dests"), "the destination tree must be kept");
            assert!(!names.has(b"EmbeddedFiles"));
            assert!(!names.has(b"JavaScript"));
        })
        .await;
    }
}
