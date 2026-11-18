//! Continuity Camera taraması uçtan uca: sahte bir yardımcı + gerçek PDFium worker'ı.
//!
//! Sahte yardımcı, gerçek yardımcının sözleşmesini taklit eden bir kabuk betiğidir:
//! tarama dizinine sabit adlarla yazar, stdout'a tek satır JSON basar ya da çıkış koduyla
//! hata bildirir. Her senaryoda tarama dizininin silindiği, iptal/hata durumlarında
//! belgenin değişmediği ve başarıda eklemenin tek adımda geri alındığı doğrulanır.

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos_arm64 {
    use parking_lot::{Mutex, RwLock};
    use preview_alternative_lib::continuity_scan::{
        scan_into_session, ContinuityScanGate, ContinuityScanImport, ContinuityScanRequest,
        ScanContentKind, ScanHelper, ScanPageFit,
    };
    use preview_alternative_lib::document_session::{
        self, DocumentSessionStore, SessionJobCancellationReason, SessionJobOutcome,
    };
    use preview_alternative_lib::pdf_engine::render_service::RenderService;
    use preview_alternative_lib::pdf_engine::subprocess_manager::{
        PdfiumWorkerBroker, WorkerLauncher,
    };
    use preview_alternative_lib::{
        ActiveRenderTasks, ActiveSearchId, PasswordCache, SearchCache, TextLayoutCache,
    };
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tauri::Manager;

    const A4: ScanPageFit = ScanPageFit {
        width: 595.0,
        height: 842.0,
    };

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name)
    }

    struct Harness {
        app: tauri::App<tauri::test::MockRuntime>,
        session_id: String,
        generation: u64,
        _files: tempfile::TempDir,
        files: PathBuf,
    }

    impl Harness {
        async fn open() -> Self {
            let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
                PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative")),
                vec![OsString::from("--run-worker")],
            )));
            let password_cache = Arc::new(Mutex::new(HashMap::new()));
            let store = DocumentSessionStore::initialize_with_password_cache_and_broker(
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
                .manage(ContinuityScanGate::default())
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();

            let files = tempfile::tempdir().unwrap();
            let document = files.path().join("belge.pdf");
            std::fs::copy(fixture("three-distinct-pages.pdf"), &document).unwrap();
            let opened = document_session::open_document_session(
                document.to_string_lossy().into_owned(),
                app.state(),
            )
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
            let path = files.path().to_path_buf();
            Self {
                app,
                session_id: opened.session_id,
                generation: committed.snapshot.content_generation,
                _files: files,
                files: path,
            }
        }

        fn store(&self) -> &DocumentSessionStore {
            self.app.state::<DocumentSessionStore>().inner()
        }

        async fn page_count(&self) -> u32 {
            document_session::get_session_page_count(
                self.session_id.clone(),
                self.generation,
                self.app.state(),
                self.app.state(),
            )
            .await
            .unwrap()
        }

        /// Kabuk betiği olarak sahte yardımcı; `$1` tarama dizinidir.
        fn helper(&self, name: &str, body: &str, timeout: Duration) -> ScanHelper {
            let program = self.files.join(name);
            std::fs::write(&program, format!("#!/bin/sh\nset -e\nDIR=\"$1\"\n{body}\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            ScanHelper {
                command: Arc::new(move |directory: &Path| {
                    let mut command = Command::new(&program);
                    command.arg(directory);
                    Ok(command)
                }),
                timeout,
            }
        }

        async fn scan(
            &self,
            helper: ScanHelper,
            at_index: u32,
        ) -> Result<SessionJobOutcome<ContinuityScanImport>, document_session::DocumentSessionError>
        {
            scan_into_session(
                self.store(),
                self.app.state::<ContinuityScanGate>().inner(),
                helper,
                ContinuityScanRequest {
                    job_id: uuid::Uuid::new_v4().to_string(),
                    session_id: self.session_id.clone(),
                    expected_content_generation: self.generation,
                    at_index,
                    page_fit: A4,
                },
            )
            .await
        }

        fn assert_no_scan_directories(&self) {
            let root = self.store().scans_root();
            let leftovers: Vec<_> = std::fs::read_dir(&root)
                .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
                .unwrap_or_default();
            assert!(leftovers.is_empty(), "tarama dizini kaldı: {leftovers:?}");
        }

        async fn undo(&mut self) {
            let snapshot = document_session::undo_session_mutation(
                self.session_id.clone(),
                self.generation,
                self.app.state(),
                self.app.state(),
            )
            .await
            .unwrap();
            self.generation = snapshot.content_generation;
        }
    }

    /// `Harness` `Send` değildir; eşzamanlı testler tarama görevine yalnız `AppHandle` taşır.
    async fn scan_with(
        handle: tauri::AppHandle<tauri::test::MockRuntime>,
        session_id: String,
        generation: u64,
        helper: ScanHelper,
        job_id: String,
    ) -> Result<SessionJobOutcome<ContinuityScanImport>, document_session::DocumentSessionError> {
        scan_into_session(
            handle.state::<DocumentSessionStore>().inner(),
            handle.state::<ContinuityScanGate>().inner(),
            helper,
            ContinuityScanRequest {
                job_id,
                session_id,
                expected_content_generation: generation,
                at_index: 0,
                page_fit: A4,
            },
        )
        .await
    }

    fn write_jpeg(path: &Path, width: u32, height: u32) {
        image::RgbImage::from_pixel(width, height, image::Rgb([120, 30, 200]))
            .save_with_format(path, image::ImageFormat::Jpeg)
            .unwrap();
    }

    fn failure_code<T>(outcome: SessionJobOutcome<T>) -> String {
        match outcome {
            SessionJobOutcome::Failed { error, .. } => error.code,
            SessionJobOutcome::Completed { .. } => panic!("unexpected completion"),
            SessionJobOutcome::Cancelled { cancellation, .. } => {
                panic!("unexpected cancellation: {cancellation:?}")
            }
        }
    }

    #[tokio::test]
    async fn scanned_pdf_is_inserted_as_one_undoable_mutation() {
        let mut harness = Harness::open().await;
        assert_eq!(harness.page_count().await, 3);
        let source = fixture("three-distinct-pages.pdf");
        let helper = harness.helper(
            "pdf-helper",
            &format!(
                "cp '{}' \"$DIR/scan.pdf\"\necho '{{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"]}}'",
                source.display()
            ),
            Duration::from_secs(60),
        );
        let outcome = harness.scan(helper, 1).await.unwrap();
        let SessionJobOutcome::Completed { value, .. } = outcome else {
            panic!("completion expected: {outcome:?}");
        };
        assert_eq!(value.kind, ScanContentKind::Pdf);
        assert!(value.snapshot.content_generation > harness.generation);
        harness.generation = value.snapshot.content_generation;
        assert_eq!(harness.page_count().await, 6);
        harness.assert_no_scan_directories();

        harness.undo().await;
        assert_eq!(harness.page_count().await, 3);
    }

    #[tokio::test]
    async fn scanned_photos_become_pages_fitted_to_the_given_paper() {
        let mut harness = Harness::open().await;
        let photos = harness.files.join("photos");
        std::fs::create_dir(&photos).unwrap();
        write_jpeg(&photos.join("a.jpg"), 400, 300);
        write_jpeg(&photos.join("b.jpg"), 300, 400);
        let helper = harness.helper(
            "image-helper",
            &format!(
                "cp '{0}/a.jpg' \"$DIR/scan-001.jpg\"\ncp '{0}/b.jpg' \"$DIR/scan-002.jpg\"\n\
                 echo '{{\"kind\":\"images\",\"files\":[\"scan-001.jpg\",\"scan-002.jpg\"]}}'",
                photos.display()
            ),
            Duration::from_secs(60),
        );
        let outcome = harness.scan(helper, 3).await.unwrap();
        let SessionJobOutcome::Completed { value, .. } = outcome else {
            panic!("completion expected: {outcome:?}");
        };
        assert_eq!(value.kind, ScanContentKind::Images);
        harness.generation = value.snapshot.content_generation;
        assert_eq!(harness.page_count().await, 5);
        let sizes = document_session::get_session_page_sizes(
            harness.session_id.clone(),
            harness.generation,
            harness.app.state(),
        )
        .await
        .unwrap();
        // Yatay fotoğraf yatay A4'e, dikey fotoğraf dikey A4'e; sona eklendiler.
        let landscape = &sizes[3];
        let portrait = &sizes[4];
        assert!((landscape.width_pt - 842.0).abs() < 0.5 && (landscape.height_pt - 595.0).abs() < 0.5, "{landscape:?}");
        assert!((portrait.width_pt - 595.0).abs() < 0.5 && (portrait.height_pt - 842.0).abs() < 0.5, "{portrait:?}");
        harness.assert_no_scan_directories();

        harness.undo().await;
        assert_eq!(harness.page_count().await, 3);
    }

    #[tokio::test]
    async fn closing_the_menu_without_a_choice_cancels_without_touching_the_document() {
        let harness = Harness::open().await;
        let helper = harness.helper("cancel-helper", "echo 'seçim yok' >&2\nexit 2", Duration::from_secs(60));
        let outcome = harness.scan(helper, 0).await.unwrap();
        assert!(
            matches!(
                outcome,
                SessionJobOutcome::Cancelled {
                    cancellation: SessionJobCancellationReason::UserRequested,
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(harness.page_count().await, 3);
        harness.assert_no_scan_directories();
    }

    #[tokio::test]
    async fn helper_errors_leave_the_document_and_no_scan_directory() {
        let harness = Harness::open().await;
        let cases = [
            ("no-device", "exit 68", "CONTINUITY_SCAN_NO_DEVICE"),
            ("unsupported", "echo 'tip' >&2\nexit 66", "CONTINUITY_SCAN_UNSUPPORTED_CONTENT"),
            ("limit", "exit 73", "CONTINUITY_SCAN_LIMIT_EXCEEDED"),
            ("helper-timeout", "exit 75", "CONTINUITY_SCAN_TIMEOUT"),
            ("crash", "exit 9", "CONTINUITY_SCAN_FAILED"),
            // Bildirilmeyen dosya, symlink ve PDF olmayan içerik.
            (
                "extra-file",
                "printf '%%PDF-1.4' > \"$DIR/scan.pdf\"\ntouch \"$DIR/x\"\necho '{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"]}'",
                "CONTINUITY_SCAN_OUTPUT_INVALID",
            ),
            (
                "symlink",
                "ln -s /etc/hosts \"$DIR/scan.pdf\"\necho '{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"]}'",
                "CONTINUITY_SCAN_OUTPUT_INVALID",
            ),
            (
                "bad-name",
                "echo '{\"kind\":\"images\",\"files\":[\"../escape.jpg\"]}'",
                "CONTINUITY_SCAN_OUTPUT_INVALID",
            ),
            (
                "not-a-pdf",
                "printf 'merhaba' > \"$DIR/scan.pdf\"\necho '{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"]}'",
                "",
            ),
        ];
        for (name, body, expected) in cases {
            let helper = harness.helper(name, body, Duration::from_secs(60));
            match harness.scan(helper, 1).await {
                Ok(outcome) => assert_eq!(failure_code(outcome), expected, "{name}"),
                // Kopyalama/worker doğrulaması içe aktarmayı reddeder.
                Err(error) => assert!(expected.is_empty(), "{name}: {error:?}"),
            }
            assert_eq!(harness.page_count().await, 3, "{name}");
            harness.assert_no_scan_directories();
        }
    }

    #[tokio::test]
    async fn timeout_kills_the_helper_and_cleans_up() {
        let harness = Harness::open().await;
        let pid_file = harness.files.join("timeout.pid");
        let helper = harness.helper(
            "slow-helper",
            &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
            Duration::from_secs(5),
        );
        let started = Instant::now();
        let outcome = harness.scan(helper, 0).await.unwrap();
        assert_eq!(failure_code(outcome), "CONTINUITY_SCAN_TIMEOUT");
        assert!(started.elapsed() < Duration::from_secs(20));
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        assert!(!process_alive(pid.trim()), "yardımcı hâlâ çalışıyor");
        assert_eq!(harness.page_count().await, 3);
        harness.assert_no_scan_directories();
    }

    fn process_alive(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn wait_for(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !path.exists() {
            assert!(Instant::now() < deadline, "{} oluşmadı", path.display());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_the_job_kills_the_waiting_helper() {
        let harness = Harness::open().await;
        let pid_file = harness.files.join("cancel.pid");
        let helper = harness.helper(
            "waiting-helper",
            &format!("echo $$ > '{}.tmp'\nmv '{0}.tmp' '{0}'\nexec sleep 30", pid_file.display()),
            Duration::from_secs(120),
        );
        let job_id = uuid::Uuid::new_v4().to_string();
        let task = tokio::spawn(scan_with(
            harness.app.handle().clone(),
            harness.session_id.clone(),
            harness.generation,
            helper,
            job_id.clone(),
        ));
        tokio::task::spawn_blocking({
            let pid_file = pid_file.clone();
            move || wait_for(&pid_file)
        })
        .await
        .unwrap();
        // İkinci tarama kapıda reddedilir.
        let second = harness
            .scan(harness.helper("unused", "exit 0", Duration::from_secs(5)), 0)
            .await
            .unwrap_err();
        assert_eq!(second.code, "CONTINUITY_SCAN_BUSY");

        document_session::cancel_session_job(
            harness.session_id.clone(),
            job_id,
            harness.app.state(),
        )
        .await
        .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(30), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }), "{outcome:?}");
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        assert!(!process_alive(pid.trim()), "yardımcı iptalde öldürülmedi");
        assert_eq!(harness.page_count().await, 3);
        harness.assert_no_scan_directories();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_document_change_during_the_scan_rejects_the_result() {
        let harness = Harness::open().await;
        let started = harness.files.join("generation.started");
        let release = harness.files.join("generation.release");
        let source = fixture("three-distinct-pages.pdf");
        let helper = harness.helper(
            "generation-helper",
            &format!(
                "touch '{}'\nwhile [ ! -e '{}' ]; do sleep 0.05; done\ncp '{}' \"$DIR/scan.pdf\"\n\
                 echo '{{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"]}}'",
                started.display(),
                release.display(),
                source.display()
            ),
            Duration::from_secs(120),
        );
        let task = tokio::spawn(scan_with(
            harness.app.handle().clone(),
            harness.session_id.clone(),
            harness.generation,
            helper,
            uuid::Uuid::new_v4().to_string(),
        ));
        tokio::task::spawn_blocking({
            let started = started.clone();
            move || wait_for(&started)
        })
        .await
        .unwrap();
        // Kullanıcı tarama beklerken belgeyi düzenler.
        let rotated = document_session::rotate_session_page(
            harness.session_id.clone(),
            harness.generation,
            0,
            90,
            harness.app.state(),
            harness.app.state(),
        )
        .await
        .unwrap();
        std::fs::write(&release, b"").unwrap();
        let error = tokio::time::timeout(Duration::from_secs(60), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "STALE_GENERATION", "{error:?}");
        let pages = document_session::get_session_page_count(
            harness.session_id.clone(),
            rotated.content_generation,
            harness.app.state(),
            harness.app.state(),
        )
        .await
        .unwrap();
        assert_eq!(pages, 3, "tarama eklenmemeli: {error:?}");
        harness.assert_no_scan_directories();
    }

    #[tokio::test]
    async fn a_stale_generation_is_rejected_before_the_helper_runs() {
        let harness = Harness::open().await;
        let marker = harness.files.join("ran");
        let helper = harness.helper(
            "never-runs",
            &format!("touch '{}'\nexit 0", marker.display()),
            Duration::from_secs(5),
        );
        let error = scan_into_session(
            harness.store(),
            harness.app.state::<ContinuityScanGate>().inner(),
            helper,
            ContinuityScanRequest {
                job_id: uuid::Uuid::new_v4().to_string(),
                session_id: harness.session_id.clone(),
                expected_content_generation: harness.generation + 7,
                at_index: 0,
                page_fit: A4,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "STALE_GENERATION");
        assert!(!marker.exists());
        harness.assert_no_scan_directories();
    }
}
