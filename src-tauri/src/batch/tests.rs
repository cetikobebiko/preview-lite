use super::*;
use std::fs;
use std::sync::mpsc;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pdf")
        .join(name)
}

fn copy_fixture(name: &str, directory: &Path, as_name: &str) -> PathBuf {
    let target = directory.join(as_name);
    fs::copy(fixture(name), &target).unwrap();
    target
}

fn digest(path: &Path) -> String {
    crate::document_io::sha256_file(path).unwrap()
}

/// Collects events; optionally cancels the batch when a given file starts.
#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<BatchProgressEvent>>,
    cancel_on_start: Mutex<Option<(BatchRunner, usize)>>,
    block_first: Mutex<Option<mpsc::Receiver<()>>>,
}

impl BatchEventSink for RecordingSink {
    fn emit(&self, event: &BatchProgressEvent) {
        if let Some(receiver) = self.block_first.lock().take() {
            let _ = receiver.recv();
        }
        if event.kind == BatchEventKind::FileStarted {
            if let Some((runner, index)) = self.cancel_on_start.lock().as_ref() {
                if event.file.as_ref().map(|file| file.index) == Some(*index) {
                    assert!(runner.cancel(&event.batch_id));
                }
            }
        }
        self.events.lock().push(event.clone());
    }
}

struct Harness {
    _temp: tempfile::TempDir,
    inputs: PathBuf,
    outputs: PathBuf,
    environment: BatchEnvironment,
    runner: BatchRunner,
}

impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let inputs = temp.path().join("girdiler");
        let outputs = temp.path().join("çıktılar");
        let private = temp.path().join("private");
        for directory in [&inputs, &outputs, &private] {
            fs::create_dir_all(directory).unwrap();
        }
        let environment = BatchEnvironment {
            batches_root: private.join("batches"),
            private: PrivateRoots::new([private]),
        };
        Self {
            _temp: temp,
            inputs,
            outputs,
            environment,
            runner: BatchRunner::default(),
        }
    }

    fn request(
        &self,
        inputs: &[&Path],
        operation: BatchOperation,
        policy: ConflictPolicy,
    ) -> BatchRequest {
        BatchRequest {
            inputs: inputs
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            operation,
            output_dir: self.outputs.to_string_lossy().into_owned(),
            conflict_policy: policy,
        }
    }

    async fn run(&self, request: BatchRequest, sink: Arc<RecordingSink>) -> BatchStatus {
        let id = self.runner.start(request, &self.environment, sink).unwrap();
        assert!(
            self.runner.wait(&id, Duration::from_secs(120)).await,
            "batch did not finish"
        );
        self.runner.status(Some(&id)).unwrap()
    }

    fn output_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.outputs)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn batches_left(&self) -> usize {
        fs::read_dir(&self.environment.batches_root)
            .map(|entries| entries.count())
            .unwrap_or(0)
    }
}

fn optimize() -> BatchOperation {
    use crate::pdf_optimizer::{ImageCompression, ImageSettings, OptimizeSettings};
    let images = ImageSettings {
        downsample: true,
        target_ppi: 150.0,
        above_ppi: 225.0,
        compression: ImageCompression::Jpeg { quality: 60 },
    };
    BatchOperation::Optimize {
        settings: OptimizeSettings {
            color: images,
            gray: images,
            remove_unused_xobjects: true,
            discard_thumbnails: true,
            discard_alternate_images: true,
            flate_unencoded: true,
            object_streams: true,
        },
    }
}

fn file_reason(status: &BatchStatus, index: usize) -> (BatchFileState, Option<String>) {
    let file = &status.files[index];
    (
        file.state,
        file.reason.as_ref().map(|reason| reason.code.clone()),
    )
}

fn assert_event_contract(events: &[BatchProgressEvent]) {
    assert!(!events.is_empty());
    for (position, event) in events.iter().enumerate() {
        assert_eq!(
            event.sequence,
            position as u64 + 1,
            "sequence must be gap-free"
        );
    }
    let terminal: Vec<_> = events.iter().filter(|event| event.terminal).collect();
    assert_eq!(terminal.len(), 1, "exactly one terminal event");
    assert!(
        events.last().unwrap().terminal,
        "nothing after the terminal event"
    );
    assert!(events.last().unwrap().status.is_some());
}

fn assert_pdf_opens(path: &Path) {
    let pdfium = crate::pdf_engine::get_pdfium().unwrap();
    let document = pdfium.load_pdf_from_file(path, None).unwrap();
    assert!(!document.pages().is_empty());
}

#[tokio::test]
async fn password_and_broken_files_do_not_stop_the_valid_one() {
    let harness = Harness::new();
    let locked = copy_fixture("encrypted.pdf", &harness.inputs, "kilitli.pdf");
    let broken = copy_fixture("malformed.pdf", &harness.inputs, "bozuk.pdf");
    let valid = copy_fixture("three-distinct-pages.pdf", &harness.inputs, "rapor.pdf");
    let before: Vec<_> = [&locked, &broken, &valid]
        .iter()
        .map(|path| digest(path))
        .collect();

    let sink = Arc::new(RecordingSink::default());
    let status = harness
        .run(
            harness.request(
                &[&locked, &broken, &valid],
                optimize(),
                ConflictPolicy::Number,
            ),
            sink.clone(),
        )
        .await;

    assert_eq!(status.state, BatchState::Completed);
    assert_eq!(
        file_reason(&status, 0),
        (
            BatchFileState::Skipped,
            Some("BATCH_INPUT_PASSWORD_REQUIRED".into())
        )
    );
    assert_eq!(file_reason(&status, 1).0, BatchFileState::Failed);
    assert_eq!(file_reason(&status, 2), (BatchFileState::Completed, None));
    assert_eq!(
        status.counts,
        BatchCounts {
            completed: 1,
            skipped: 1,
            failed: 1,
            cancelled: 0
        }
    );

    assert_eq!(harness.output_names(), ["rapor_küçültülmüş.pdf"]);
    assert_eq!(status.files[2].outputs.len(), 1);
    assert_pdf_opens(&harness.outputs.join("rapor_küçültülmüş.pdf"));

    let after: Vec<_> = [&locked, &broken, &valid]
        .iter()
        .map(|path| digest(path))
        .collect();
    assert_eq!(before, after, "inputs must never change");
    assert_eq!(
        harness.batches_left(),
        0,
        "private batch directories are removed"
    );

    let events = sink.events.lock();
    assert_event_contract(&events);
    let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds,
        [
            BatchEventKind::FileStarted,
            BatchEventKind::FileSkipped,
            BatchEventKind::FileStarted,
            BatchEventKind::FileFailed,
            BatchEventKind::FileStarted,
            BatchEventKind::FileCompleted,
            BatchEventKind::BatchCompleted,
        ]
    );
}

#[tokio::test]
async fn cancel_keeps_finished_outputs_and_leaves_nothing_of_the_running_file() {
    let harness = Harness::new();
    let inputs: Vec<PathBuf> = (0..3)
        .map(|index| copy_fixture("minimal.pdf", &harness.inputs, &format!("belge{index}.pdf")))
        .collect();
    let sink = Arc::new(RecordingSink::default());
    *sink.cancel_on_start.lock() = Some((harness.runner.clone(), 1));
    let refs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
    let status = harness
        .run(
            harness.request(
                &refs,
                BatchOperation::ExportText {
                    format: TextExportFormat::Txt,
                },
                ConflictPolicy::Number,
            ),
            sink.clone(),
        )
        .await;

    assert_eq!(status.state, BatchState::Cancelled);
    assert_eq!(file_reason(&status, 0).0, BatchFileState::Completed);
    assert_eq!(file_reason(&status, 1).0, BatchFileState::Cancelled);
    assert_eq!(file_reason(&status, 2).0, BatchFileState::Cancelled);
    assert_eq!(harness.output_names(), ["belge0.txt"]);
    assert_eq!(harness.batches_left(), 0);
    let events = sink.events.lock();
    assert_event_contract(&events);
    assert_eq!(events.last().unwrap().kind, BatchEventKind::BatchCancelled);
    // A cancel after the end is a no-op.
    assert!(!harness.runner.cancel(&status.batch_id));
}

#[tokio::test]
async fn outputs_in_the_input_folder_never_replace_an_input_even_with_overwrite() {
    let harness = Harness::new();
    let first = copy_fixture("minimal.pdf", &harness.inputs, "rapor.pdf");
    // The optimized name of `rapor.pdf` is this second input.
    let second = copy_fixture(
        "three-distinct-pages.pdf",
        &harness.inputs,
        "rapor_küçültülmüş.pdf",
    );
    let second_digest = digest(&second);
    let mut request = harness.request(&[&first, &second], optimize(), ConflictPolicy::Overwrite);
    request.output_dir = harness.inputs.to_string_lossy().into_owned();
    let status = harness
        .run(request, Arc::new(RecordingSink::default()))
        .await;

    assert_eq!(status.counts.completed, 2, "{status:?}");
    assert_eq!(
        digest(&second),
        second_digest,
        "an input must never be overwritten"
    );
    let mut names: Vec<_> = fs::read_dir(&harness.inputs)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "rapor.pdf",
            "rapor_küçültülmüş (2).pdf",
            "rapor_küçültülmüş.pdf",
            "rapor_küçültülmüş_küçültülmüş.pdf",
        ]
    );
}

#[tokio::test]
async fn conflict_policies_number_skip_and_overwrite() {
    let harness = Harness::new();
    let input = copy_fixture("minimal.pdf", &harness.inputs, "not.pdf");
    let existing = harness.outputs.join("not.txt");
    fs::write(&existing, b"eski").unwrap();
    let text = || BatchOperation::ExportText {
        format: TextExportFormat::Txt,
    };

    let skipped = harness
        .run(
            harness.request(&[&input], text(), ConflictPolicy::Skip),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(
        file_reason(&skipped, 0),
        (BatchFileState::Skipped, Some("BATCH_OUTPUT_EXISTS".into()))
    );
    assert_eq!(fs::read(&existing).unwrap(), b"eski");

    let numbered = harness
        .run(
            harness.request(&[&input], text(), ConflictPolicy::Number),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(file_reason(&numbered, 0).0, BatchFileState::Completed);
    assert_eq!(fs::read(&existing).unwrap(), b"eski");
    assert!(harness.outputs.join("not (2).txt").is_file());

    let replaced = harness
        .run(
            harness.request(&[&input], text(), ConflictPolicy::Overwrite),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(file_reason(&replaced, 0).0, BatchFileState::Completed);
    assert_ne!(fs::read(&existing).unwrap(), b"eski");
}

#[tokio::test]
async fn page_images_are_published_as_one_set() {
    let harness = Harness::new();
    let input = copy_fixture("three-distinct-pages.pdf", &harness.inputs, "r.pdf");
    let status = harness
        .run(
            harness.request(
                &[&input],
                BatchOperation::ExportImages {
                    format: ImageExportFormat::Png,
                    dpi: 36.0,
                },
                ConflictPolicy::Number,
            ),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(file_reason(&status, 0), (BatchFileState::Completed, None));
    assert_eq!(
        harness.output_names(),
        ["r_sayfa_1.png", "r_sayfa_2.png", "r_sayfa_3.png"]
    );
}

#[tokio::test]
async fn owner_restricted_files_skip_optimize_but_export_text() {
    let harness = Harness::new();
    let input = harness.inputs.join("kısıtlı.pdf");
    fs::write(&input, crate::lopdf_compat::owner_only_encrypted_pdf(1)).unwrap();

    let optimized = harness
        .run(
            harness.request(&[&input], optimize(), ConflictPolicy::Number),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(
        file_reason(&optimized, 0),
        (
            BatchFileState::Skipped,
            Some("BATCH_INPUT_OWNER_RESTRICTED".into())
        )
    );

    let text = harness
        .run(
            harness.request(
                &[&input],
                BatchOperation::ExportText {
                    format: TextExportFormat::Md,
                },
                ConflictPolicy::Number,
            ),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(file_reason(&text, 0), (BatchFileState::Completed, None));
    assert_eq!(harness.output_names(), ["kısıtlı.md"]);
}

#[tokio::test]
async fn duplicates_and_invalid_inputs_are_reported_per_file() {
    let harness = Harness::new();
    let input = copy_fixture("minimal.pdf", &harness.inputs, "a.pdf");
    let notes = harness.inputs.join("notlar.txt");
    fs::write(&notes, b"x").unwrap();
    let status = harness
        .run(
            harness.request(
                &[&input, &input, &notes],
                BatchOperation::ExportText {
                    format: TextExportFormat::Txt,
                },
                ConflictPolicy::Number,
            ),
            Arc::new(RecordingSink::default()),
        )
        .await;
    assert_eq!(file_reason(&status, 0).0, BatchFileState::Completed);
    assert_eq!(
        file_reason(&status, 1),
        (
            BatchFileState::Skipped,
            Some("BATCH_INPUT_DUPLICATE".into())
        )
    );
    assert_eq!(
        file_reason(&status, 2),
        (BatchFileState::Skipped, Some("BATCH_INPUT_NOT_PDF".into()))
    );
}

#[test]
fn requests_outside_the_limits_are_refused_before_anything_runs() {
    let harness = Harness::new();
    let input = copy_fixture("minimal.pdf", &harness.inputs, "a.pdf");
    let sink: Arc<dyn BatchEventSink> = Arc::new(RecordingSink::default());
    let start = |request: BatchRequest| {
        harness
            .runner
            .start(request, &harness.environment, Arc::clone(&sink))
            .unwrap_err()
            .code
    };

    assert_eq!(
        start(harness.request(&[], optimize(), ConflictPolicy::Number)),
        "BATCH_EMPTY"
    );
    let many: Vec<&Path> =
        std::iter::repeat_n(input.as_path(), input::MAX_BATCH_FILES + 1).collect();
    assert_eq!(
        start(harness.request(&many, optimize(), ConflictPolicy::Number)),
        "BATCH_TOO_MANY_FILES"
    );
    let mut private_output = harness.request(&[&input], optimize(), ConflictPolicy::Number);
    fs::create_dir_all(
        harness
            .environment
            .batches_root
            .parent()
            .unwrap()
            .join("out"),
    )
    .unwrap();
    private_output.output_dir = harness
        .environment
        .batches_root
        .parent()
        .unwrap()
        .join("out")
        .to_string_lossy()
        .into_owned();
    assert_eq!(start(private_output), "BATCH_OUTPUT_DIR_PRIVATE");
    let mut missing_output = harness.request(&[&input], optimize(), ConflictPolicy::Number);
    missing_output.output_dir = harness.outputs.join("yok").to_string_lossy().into_owned();
    assert_eq!(start(missing_output), "BATCH_OUTPUT_DIR_INVALID");
    assert_eq!(
        start(harness.request(
            &[&input],
            BatchOperation::ExportImages {
                format: ImageExportFormat::Png,
                dpi: 0.0
            },
            ConflictPolicy::Number
        )),
        crate::export::EXPORT_DPI_INVALID_CODE
    );
    assert!(
        harness.runner.status(None).is_none(),
        "no batch was created"
    );
}

#[tokio::test]
async fn only_one_batch_runs_at_a_time() {
    let harness = Harness::new();
    let input = copy_fixture("minimal.pdf", &harness.inputs, "a.pdf");
    let (release, blocked) = mpsc::channel();
    let sink = Arc::new(RecordingSink::default());
    *sink.block_first.lock() = Some(blocked);
    let text = || BatchOperation::ExportText {
        format: TextExportFormat::Txt,
    };
    let id = harness
        .runner
        .start(
            harness.request(&[&input], text(), ConflictPolicy::Number),
            &harness.environment,
            sink,
        )
        .unwrap();
    let error = harness
        .runner
        .start(
            harness.request(&[&input], text(), ConflictPolicy::Number),
            &harness.environment,
            Arc::new(RecordingSink::default()),
        )
        .unwrap_err();
    assert_eq!(error.code, "BATCH_ALREADY_RUNNING");
    assert_eq!(
        harness.runner.status(Some(&id)).unwrap().state,
        BatchState::Running
    );
    release.send(()).unwrap();
    assert!(harness.runner.wait(&id, Duration::from_secs(60)).await);
}

#[tokio::test]
async fn the_active_session_does_not_change_while_a_batch_runs() {
    let store = DocumentSessionStore::initialize().unwrap();
    let documents = tempfile::tempdir().unwrap();
    let open = copy_fixture("minimal.pdf", documents.path(), "açık.pdf");
    crate::document_session::activate_test_session(&store, &open);
    let (id_before, epoch_before, snapshot_before) =
        crate::document_session::active_identity_for_test(&store);
    let snapshot_before = snapshot_before.unwrap();

    let harness = Harness::new();
    let environment = BatchEnvironment::new(&store, None);
    let input = copy_fixture("three-distinct-pages.pdf", &harness.inputs, "b.pdf");
    let id = harness
        .runner
        .start(
            harness.request(&[&input, &open], optimize(), ConflictPolicy::Number),
            &environment,
            Arc::new(RecordingSink::default()),
        )
        .unwrap();
    assert!(harness.runner.wait(&id, Duration::from_secs(120)).await);
    assert_eq!(
        harness.runner.status(Some(&id)).unwrap().counts.completed,
        2
    );

    let (id_after, epoch_after, snapshot_after) =
        crate::document_session::active_identity_for_test(&store);
    let snapshot_after = snapshot_after.unwrap();
    assert_eq!(id_before, id_after);
    assert_eq!(epoch_before, epoch_after);
    assert_eq!(
        snapshot_before.content_generation,
        snapshot_after.content_generation
    );
    assert_eq!(
        snapshot_before.state_revision,
        snapshot_after.state_revision
    );
    assert_eq!(snapshot_before.dirty, snapshot_after.dirty);
    assert_eq!(snapshot_before.lifecycle, snapshot_after.lifecycle);
    assert_eq!(
        fs::read_dir(store.batches_root()).unwrap().count(),
        0,
        "the batch used the instance's private root and removed its directory"
    );
}

#[test]
fn a_crashed_batch_directory_is_removed_on_the_next_start() {
    // A store from a crashed process: its instance lock is no longer held, and a
    // batch directory with a snapshot was left behind.
    let crashed_root = std::env::temp_dir().join(crate::document_session::TEMP_ROOT_NAME);
    fs::create_dir_all(&crashed_root).unwrap();
    let nonce = Uuid::new_v4().to_string();
    let instance = crashed_root.join(format!("instance-{nonce}"));
    let leftover = instance.join("batches").join("batch-x").join("file-0");
    fs::create_dir_all(&leftover).unwrap();
    fs::copy(fixture("minimal.pdf"), leftover.join("snapshot.pdf")).unwrap();
    fs::write(crashed_root.join(format!("instance-{nonce}.lock")), b"").unwrap();

    let _store = DocumentSessionStore::initialize().unwrap();
    assert!(
        !instance.exists(),
        "orphaned batch directory must be cleaned up"
    );
}

#[test]
fn requests_parse_from_the_frontend_shape_and_status_never_carries_a_password() {
    let request: BatchRequest = serde_json::from_value(serde_json::json!({
        "inputs": ["/a.pdf"],
        "operation": {
            "kind": "security",
            "op": {
                "op": "protect",
                "settings": {
                    "userPassword": "gizli-parola-123",
                    "ownerPassword": "",
                    "method": "aes256",
                    "permissions": {
                        "print": "high_resolution", "modify": true, "copy": true,
                        "annotate": true, "fillForms": true, "assemble": true
                    },
                    "encryptMetadata": true
                }
            }
        },
        "outputDir": "/out",
        "conflictPolicy": "skip"
    }))
    .unwrap();
    assert_eq!(request.conflict_policy, ConflictPolicy::Skip);
    assert!(!format!("{request:?}").contains("gizli-parola-123"));
    assert_eq!(request.operation.kind(), "security");

    let rejected = serde_json::from_value::<BatchRequest>(serde_json::json!({
        "inputs": [], "operation": { "kind": "export_text", "format": "txt" },
        "outputDir": "/out", "parameters": "{}"
    }));
    assert!(rejected.is_err(), "unknown fields are refused");
}

#[tokio::test]
async fn a_protected_copy_reports_without_the_password() {
    let harness = Harness::new();
    let input = copy_fixture("minimal.pdf", &harness.inputs, "a.pdf");
    let request: BatchRequest = serde_json::from_value(serde_json::json!({
        "inputs": [input.to_string_lossy()],
        "operation": {
            "kind": "security",
            "op": { "op": "protect", "settings": {
                "userPassword": "gizli-parola-123", "ownerPassword": "", "method": "aes256",
                "permissions": {
                    "print": "high_resolution", "modify": true, "copy": true,
                    "annotate": true, "fillForms": true, "assemble": true
                },
                "encryptMetadata": true
            } }
        },
        "outputDir": harness.outputs.to_string_lossy(),
    }))
    .unwrap();
    let sink = Arc::new(RecordingSink::default());
    let status = harness.run(request, sink.clone()).await;
    assert_eq!(file_reason(&status, 0), (BatchFileState::Completed, None));
    assert_eq!(harness.output_names(), ["a_korumalı.pdf"]);
    let events = serde_json::to_string(&*sink.events.lock()).unwrap();
    let serialized = serde_json::to_string(&status).unwrap() + events.as_str();
    assert!(!serialized.contains("gizli-parola-123"));
}
