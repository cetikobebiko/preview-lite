use preview_alternative_lib::pdf_engine::subprocess_manager::{
    CommandTimeout, PdfiumWorkerBroker, WorkerDocumentKey, WorkerLauncher,
};
use preview_alternative_lib::worker::ipc::{WorkerCommand, WorkerResult};
use std::ffi::OsString;
use std::path::PathBuf;
use tauri::Manager;

fn helper_launcher(mode: &str) -> WorkerLauncher {
    WorkerLauncher::for_test(
        PathBuf::from(env!("CARGO_BIN_EXE_pdfium-worker-test-helper")),
        vec![OsString::from(mode)],
    )
}

fn key(label: &str) -> WorkerDocumentKey {
    WorkerDocumentKey {
        session_id: label.to_string(),
        content_generation: 1,
    }
}

async fn open_with_mode(mode: &str) -> Result<PdfiumWorkerBroker, String> {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher(mode)));
    let key = key(mode);
    let admission = broker
        .admit_read(&key, None)
        .await
        .map_err(|error| error.code.to_string())?;
    broker
        .ensure_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .map_err(|error| error.code.to_string())?;
    drop(admission);
    Ok(broker)
}

async fn audit_with_mode(mode: &str, timeout: CommandTimeout) -> Result<WorkerResult, String> {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher(mode)));
    let key = key(mode);
    let admission = broker
        .admit_read(&key, None)
        .await
        .map_err(|error| error.code.to_string())?;
    broker
        .ensure_document(&admission, key.clone(), "fake.pdf".to_string(), None, None)
        .await
        .map_err(|error| error.code.to_string())?;
    broker
        .command_for_document(
            &admission,
            &key,
            |document_id| WorkerCommand::AuditAccessibility { document_id },
            timeout,
            None,
        )
        .await
        .map_err(|error| error.code.to_string())
}

#[tokio::test]
async fn inert_broker_survives_spawn_failure_and_returns_typed_error() {
    let launcher = WorkerLauncher::for_test(
        PathBuf::from("definitely-missing-pdfium-worker-executable"),
        Vec::new(),
    );
    let broker = PdfiumWorkerBroker::with_launcher(Ok(launcher));
    let app = tauri::test::mock_builder()
        .manage(broker.clone())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    assert_eq!(app.state::<PdfiumWorkerBroker>().worker_epoch().await, None);
    let key = key("spawn-failure");
    let admission = broker.admit_read(&key, None).await.unwrap();
    let error = broker
        .ensure_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_SPAWN_FAILED");
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn handshake_request_id_and_version_are_correlated() {
    for (mode, expected) in [
        ("wrong-id-hello", "PDFIUM_WORKER_PROTOCOL_ERROR"),
        ("wrong-version-hello", "PDFIUM_WORKER_PROTOCOL_MISMATCH"),
    ] {
        let error = open_with_mode(mode).await.err().unwrap();
        assert_eq!(error, expected);
    }
    let command_error = open_with_mode("wrong-id-command").await.err().unwrap();
    assert_eq!(command_error, "PDFIUM_WORKER_PROTOCOL_ERROR");
}

#[tokio::test]
async fn malformed_and_oversized_handshake_frames_are_rejected() {
    let partial = open_with_mode("partial-hello").await.err().unwrap();
    assert_eq!(partial, "PDFIUM_WORKER_CRASHED");
    let oversized = open_with_mode("oversized-hello").await.err().unwrap();
    assert_eq!(oversized, "PDFIUM_WORKER_MESSAGE_TOO_LARGE");
}

#[tokio::test]
async fn accessibility_audit_protocol_oversize_crash_and_timeout_are_typed() {
    let normal = audit_with_mode("normal", CommandTimeout::DocumentRead)
        .await
        .unwrap();
    assert!(matches!(normal, WorkerResult::AccessibilityAudit(_)));

    for (mode, timeout, expected) in [
        (
            "wrong-version-audit",
            CommandTimeout::DocumentRead,
            "PDFIUM_WORKER_PROTOCOL_MISMATCH",
        ),
        (
            "oversized-audit",
            CommandTimeout::DocumentRead,
            "PDFIUM_WORKER_MESSAGE_TOO_LARGE",
        ),
        (
            "crash-audit",
            CommandTimeout::DocumentRead,
            "PDFIUM_WORKER_CRASHED",
        ),
        (
            "hang-audit",
            CommandTimeout::CloseDocument,
            "PDFIUM_WORKER_TIMEOUT",
        ),
    ] {
        assert_eq!(audit_with_mode(mode, timeout).await.unwrap_err(), expected);
    }
}

#[tokio::test]
async fn handshake_timeout_kills_reaps_and_clears_worker_epoch() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("hang-hello")));
    let key = key("hang-hello");
    let admission = broker.admit_read(&key, None).await.unwrap();
    let error = broker
        .ensure_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_TIMEOUT");
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn worker_stderr_is_drained_while_handshake_runs() {
    let broker = open_with_mode("stderr-flood").await.unwrap();
    assert!(broker.worker_epoch().await.is_some());
    broker.shutdown().await.unwrap();
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn graceful_shutdown_is_idempotent() {
    let broker = open_with_mode("normal").await.unwrap();
    broker.shutdown().await.unwrap();
    broker.shutdown().await.unwrap();
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn shutdown_ack_without_process_exit_is_killed_and_reaped() {
    let broker = open_with_mode("ack-shutdown-hang").await.unwrap();
    broker.shutdown().await.unwrap();
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn shutdown_closes_admission_and_prevents_worker_restart() {
    let broker = open_with_mode("normal").await.unwrap();
    broker.shutdown().await.unwrap();
    let key = key("after-shutdown");
    let error = broker.admit_read(&key, None).await.err().unwrap();
    assert_eq!(error.code, "PDFIUM_WORKER_UNAVAILABLE");
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn shutdown_cancels_active_command_then_reaps_worker() {
    let broker = open_with_mode("hang-page-count").await.unwrap();
    let key = key("hang-page-count");
    let task_broker = broker.clone();
    let task_key = key.clone();
    let command = tokio::spawn(async move {
        let admission = task_broker.admit_read(&task_key, None).await.unwrap();
        task_broker
            .command_for_document(
                &admission,
                &task_key,
                |document_id| preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount {
                    document_id,
                },
                preview_alternative_lib::pdf_engine::subprocess_manager::CommandTimeout::PageCount,
                None,
            )
            .await
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while !broker.state_busy_for_test() && tokio::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(broker.state_busy_for_test());

    broker.shutdown().await.unwrap();
    let error = command.await.unwrap().unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_UNAVAILABLE");
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn permits_enforce_broker_session_and_destructive_mode_authority() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("normal")));
    let other_broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("normal")));
    let key_a = key("authority-a");
    let key_b = key("authority-b");
    let read_a = broker.admit_read(&key_a, None).await.unwrap();

    let wrong_session = broker
        .ensure_document(&read_a, key_b.clone(), "fake.pdf".to_string(), None, None)
        .await
        .unwrap_err();
    assert_eq!(wrong_session.code, "PDFIUM_WORKER_ADMISSION_INVALID");

    broker
        .ensure_document(&read_a, key_a.clone(), "fake.pdf".to_string(), None, None)
        .await
        .unwrap();
    let read_invalidation = broker
        .invalidate_document(&read_a, &key_a)
        .await
        .unwrap_err();
    assert_eq!(read_invalidation.code, "PDFIUM_WORKER_ADMISSION_INVALID");

    let other_read = other_broker.admit_read(&key_a, None).await.unwrap();
    let foreign = broker
        .command_for_document(
            &other_read,
            &key_a,
            |document_id| preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount {
                document_id,
            },
            preview_alternative_lib::pdf_engine::subprocess_manager::CommandTimeout::PageCount,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(foreign.code, "PDFIUM_WORKER_ADMISSION_INVALID");
    drop(other_read);
    other_broker.shutdown().await.unwrap();

    drop(read_a);
    let exclusive_a = broker.admit_exclusive(&key_a.session_id).await.unwrap();
    let wrong_exclusive = broker
        .invalidate_document(&exclusive_a, &key_b)
        .await
        .unwrap_err();
    assert_eq!(wrong_exclusive.code, "PDFIUM_WORKER_ADMISSION_INVALID");
    broker
        .invalidate_document(&exclusive_a, &key_a)
        .await
        .unwrap();
    drop(exclusive_a);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn active_command_cancellation_reaps_worker_without_replay() {
    // Cancel only once the worker has read OpenDocument: a command cancelled
    // before it is sent leaves the worker running by design, so a fixed delay
    // would race worker spawn and Hello.
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("command-received");
    let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
        PathBuf::from(env!("CARGO_BIN_EXE_pdfium-worker-test-helper")),
        vec![
            OsString::from("hang-command"),
            marker.clone().into_os_string(),
        ],
    )));
    let key = key("cancel-active");
    let admission = broker.admit_read(&key, None).await.unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(marker.exists(), "worker never received OpenDocument");
        trigger.cancel();
    });
    let error = broker
        .ensure_document(
            &admission,
            key,
            "fake.pdf".to_string(),
            None,
            Some(&cancellation),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
    assert_eq!(broker.worker_epoch().await, None);
    assert_eq!(broker.restart_attempts_for_test().await, 0);
}

#[tokio::test]
async fn expected_cancellation_replaces_workers_without_spending_restart_budget() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("hang-command")));
    let key = key("cancel-does-not-restart");
    let admission = broker.admit_read(&key, None).await.unwrap();

    for _ in 0..7 {
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let error = broker
            .ensure_document(
                &admission,
                key.clone(),
                "fake.pdf".to_string(),
                None,
                Some(&cancellation),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
        assert_eq!(broker.restart_attempts_for_test().await, 0);
    }
}

#[tokio::test]
async fn one_worker_crash_charges_one_replacement_attempt() {
    let broker = open_with_mode("normal").await.unwrap();
    let key = key("normal");
    let admission = broker.admit_read(&key, None).await.unwrap();

    broker.kill_process_for_test().await.unwrap();
    let error = broker
        .command_for_document(
            &admission,
            &key,
            |document_id| preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount {
                document_id,
            },
            preview_alternative_lib::pdf_engine::subprocess_manager::CommandTimeout::PageCount,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_CRASHED");
    assert_eq!(broker.restart_attempts_for_test().await, 0);

    broker
        .ensure_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .unwrap();
    assert_eq!(broker.restart_attempts_for_test().await, 1);
}

#[tokio::test]
async fn repeated_spawn_failures_exhaust_global_restart_budget() {
    let launcher = WorkerLauncher::for_test(
        PathBuf::from("definitely-missing-pdfium-worker-executable"),
        Vec::new(),
    );
    let broker = PdfiumWorkerBroker::with_launcher(Ok(launcher));
    let key = key("restart-budget");
    let admission = broker.admit_read(&key, None).await.unwrap();
    for _ in 0..5 {
        let error = broker
            .ensure_document(&admission, key.clone(), "fake.pdf".to_string(), None, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "PDFIUM_WORKER_SPAWN_FAILED");
    }
    let limited = broker
        .ensure_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .unwrap_err();
    assert_eq!(limited.code, "PDFIUM_WORKER_RESTART_LIMIT");
}

#[tokio::test]
async fn close_timeout_kills_and_reaps_before_invalidation_succeeds() {
    let broker = open_with_mode("hang-close").await.unwrap();
    let key = key("hang-close");
    let admission = broker.admit_exclusive(&key.session_id).await.unwrap();
    broker.invalidate_document(&admission, &key).await.unwrap();
    assert_eq!(broker.worker_epoch().await, None);
}

#[tokio::test]
async fn uncommitted_document_reservation_closes_and_removes_mapping() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("normal")));
    let key = key("reservation-abort");
    let admission = broker.admit_read(&key, None).await.unwrap();
    let reservation = broker
        .reserve_document(&admission, key.clone(), "fake.pdf".to_string(), None, None)
        .await
        .unwrap();
    drop(reservation);
    drop(admission);

    let admission = broker.admit_read(&key, None).await.unwrap();
    let error = broker
        .command_for_document(
            &admission,
            &key,
            |document_id| preview_alternative_lib::worker::ipc::WorkerCommand::GetPageCount {
                document_id,
            },
            preview_alternative_lib::pdf_engine::subprocess_manager::CommandTimeout::PageCount,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_PROTOCOL_ERROR");
    drop(admission);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_reservation_abort_close_timeout_kills_and_reaps() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("hang-close")));
    let key = key("reservation-close-timeout");
    let admission = broker.admit_read(&key, None).await.unwrap();
    let reservation = broker
        .reserve_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .unwrap();
    reservation.abort(&admission).await.unwrap();
    assert_eq!(broker.worker_epoch().await, None);
    drop(admission);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn reservation_task_abort_during_shutdown_transfers_cleanup_to_broker() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("normal")));
    let key = key("reservation-task-abort");
    let task_broker = broker.clone();
    let task_key = key.clone();
    let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let admission = task_broker.admit_read(&task_key, None).await.unwrap();
        let _reservation = task_broker
            .reserve_document(&admission, task_key, "fake.pdf".to_string(), None, None)
            .await
            .unwrap();
        ready_sender.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    ready_receiver.await.unwrap();
    broker.begin_shutdown();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    broker.shutdown().await.unwrap();
    assert_eq!(broker.worker_epoch().await, None);
}

async fn page_count_command(
    broker: &PdfiumWorkerBroker,
    key: &WorkerDocumentKey,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<WorkerResult, preview_alternative_lib::pdf_engine::subprocess_manager::BrokerError> {
    let admission = broker.admit_read(key, None).await?;
    broker
        .command_for_document(
            &admission,
            key,
            |document_id| WorkerCommand::GetPageCount { document_id },
            CommandTimeout::PageCount,
            cancellation,
        )
        .await
}

fn cancel_after(millis: u64) -> tokio_util::sync::CancellationToken {
    let cancellation = tokio_util::sync::CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
        trigger.cancel();
    });
    cancellation
}

#[tokio::test]
async fn reused_document_returns_recorded_page_count_without_worker_round_trip() {
    // The helper hangs on GetPageCount, so any round-trip would time out.
    let broker = open_with_mode("hang-page-count").await.unwrap();
    let key = key("hang-page-count");
    let epoch = broker.worker_epoch().await;
    let admission = broker.admit_read(&key, None).await.unwrap();
    let (_, page_count, _) = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        broker.ensure_document(&admission, key, "fake.pdf".to_string(), None, None),
    )
    .await
    .expect("cached ensure must not wait on the worker")
    .unwrap();
    assert_eq!(page_count, 1);
    assert_eq!(broker.worker_epoch().await, epoch);
    drop(admission);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_short_command_is_drained_and_keeps_worker_and_documents() {
    let broker = open_with_mode("slow-page-count").await.unwrap();
    let key = key("slow-page-count");
    let epoch = broker.worker_epoch().await;
    assert!(epoch.is_some());

    let cancellation = cancel_after(20);
    let error = page_count_command(&broker, &key, Some(&cancellation))
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
    assert_eq!(broker.worker_epoch().await, epoch);

    // The stream is back at a frame boundary and the document mapping
    // survived: the next command is answered by the same worker.
    let result = page_count_command(&broker, &key, None).await.unwrap();
    assert!(matches!(result, WorkerResult::PageCount(1)));
    assert_eq!(broker.worker_epoch().await, epoch);
    assert_eq!(broker.restart_attempts_for_test().await, 0);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_command_past_grace_is_killed_without_restart_charge() {
    let broker = open_with_mode("hang-page-count").await.unwrap();
    let key = key("hang-page-count");
    let started = std::time::Instant::now();
    let cancellation = cancel_after(20);
    let error = page_count_command(&broker, &key, Some(&cancellation))
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
    assert!(
        elapsed >= std::time::Duration::from_millis(250),
        "worker was killed before the grace period: {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "grace period was not bounded: {elapsed:?}"
    );
    assert_eq!(broker.worker_epoch().await, None);

    // A cancellation replacement is neither a restart charge nor a
    // document failure.
    let admission = broker.admit_read(&key, None).await.unwrap();
    broker
        .ensure_document(&admission, key, "fake.pdf".to_string(), None, None)
        .await
        .unwrap();
    assert_eq!(broker.restart_attempts_for_test().await, 0);
    drop(admission);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn pre_cancelled_command_is_never_sent_to_the_worker() {
    let broker = open_with_mode("hang-page-count").await.unwrap();
    let key = key("hang-page-count");
    let epoch = broker.worker_epoch().await;
    let cancellation = tokio_util::sync::CancellationToken::new();
    let admission = broker.admit_read(&key, None).await.unwrap();
    cancellation.cancel();
    let started = std::time::Instant::now();
    let error = broker
        .command_for_document(
            &admission,
            &key,
            |document_id| WorkerCommand::GetPageCount { document_id },
            CommandTimeout::PageCount,
            Some(&cancellation),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
    assert!(started.elapsed() < std::time::Duration::from_millis(200));
    assert_eq!(broker.worker_epoch().await, epoch);
    drop(admission);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoned_in_flight_exchange_is_replaced_without_health_penalty() {
    let broker = open_with_mode("slow-page-count").await.unwrap();
    let key = key("slow-page-count");
    let epoch = broker.worker_epoch().await;
    let task_broker = broker.clone();
    let task_key = key.clone();
    let task = tokio::spawn(async move {
        let _ = page_count_command(&task_broker, &task_key, None).await;
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while !broker.state_busy_for_test() && tokio::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(broker.state_busy_for_test());
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Let the helper write the now-unwanted response into the pipe.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Reading that stale frame would be a request-ID mismatch charged as a
    // protocol violation. The broker must replace the worker instead.
    let admission = broker.admit_read(&key, None).await.unwrap();
    broker
        .ensure_document(&admission, key.clone(), "fake.pdf".to_string(), None, None)
        .await
        .unwrap();
    drop(admission);
    assert_ne!(broker.worker_epoch().await, epoch);
    let result = page_count_command(&broker, &key, None).await.unwrap();
    assert!(matches!(result, WorkerResult::PageCount(1)));
    assert_eq!(broker.restart_attempts_for_test().await, 0);
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_during_cancellation_grace_stops_and_reaps_worker() {
    let broker = open_with_mode("hang-page-count").await.unwrap();
    let key = key("hang-page-count");
    let cancellation = tokio_util::sync::CancellationToken::new();
    let task_broker = broker.clone();
    let task_key = key.clone();
    let task_cancellation = cancellation.clone();
    let command = tokio::spawn(async move {
        page_count_command(&task_broker, &task_key, Some(&task_cancellation)).await
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while !broker.state_busy_for_test() && tokio::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    cancellation.cancel();
    broker.shutdown().await.unwrap();
    let error = command.await.unwrap().unwrap_err();
    assert!(
        matches!(
            error.code,
            "PDFIUM_WORKER_UNAVAILABLE" | "PDFIUM_WORKER_CANCELLED"
        ),
        "unexpected code {}",
        error.code
    );
    assert_eq!(broker.worker_epoch().await, None);
}

struct TestReadLease {
    admission: preview_alternative_lib::pdf_engine::subprocess_manager::WorkerAdmissionPermit,
    key: WorkerDocumentKey,
    page_count: u32,
}

impl preview_alternative_lib::pdf_engine::subprocess_manager::WorkerReadLease for TestReadLease {
    fn admission(
        &self,
    ) -> &preview_alternative_lib::pdf_engine::subprocess_manager::WorkerAdmissionPermit {
        &self.admission
    }

    fn document_key(&self) -> &WorkerDocumentKey {
        &self.key
    }

    fn page_count(&self) -> u32 {
        self.page_count
    }
}

async fn acquire_test_lease(
    broker: PdfiumWorkerBroker,
    key: WorkerDocumentKey,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<TestReadLease, preview_alternative_lib::pdf_engine::subprocess_manager::BrokerError> {
    let admission = broker.admit_read(&key, Some(&cancellation)).await?;
    let (_, page_count, _) = broker
        .ensure_document(
            &admission,
            key.clone(),
            "fake.pdf".to_string(),
            None,
            Some(&cancellation),
        )
        .await?;
    Ok(TestReadLease {
        admission,
        key,
        page_count,
    })
}

async fn chunked_search(
    broker: &PdfiumWorkerBroker,
    key: &WorkerDocumentKey,
    max_results: usize,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<
    preview_alternative_lib::SearchResult,
    preview_alternative_lib::pdf_engine::subprocess_manager::BrokerError,
> {
    broker
        .search_text_in_chunks(
            || acquire_test_lease(broker.clone(), key.clone(), cancellation.clone()),
            "needle",
            preview_alternative_lib::pdf_engine::subprocess_manager::SearchChunkPolicy {
                max_pages: 1,
                time_budget_ms: 100,
                max_results,
            },
            cancellation,
        )
        .await
}

#[tokio::test]
async fn chunked_search_aggregates_ordered_matches_and_counts_past_the_limit() {
    let broker = open_with_mode("chunked-search").await.unwrap();
    let key = key("chunked-search");
    let cancellation = tokio_util::sync::CancellationToken::new();
    let result = chunked_search(&broker, &key, 4, &cancellation)
        .await
        .unwrap();
    assert!(!result.is_cancelled);
    assert_eq!(result.total_matches, 6);
    assert_eq!(
        result
            .matches
            .iter()
            .map(|found| (found.global_match_index, found.page_index))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 1), (2, 2), (3, 3)]
    );
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn reads_queued_behind_a_search_are_served_between_chunks() {
    let broker = open_with_mode("chunked-search").await.unwrap();
    let key = key("chunked-search");
    let search_broker = broker.clone();
    let search_key = key.clone();
    let search = tokio::spawn(async move {
        let cancellation = tokio_util::sync::CancellationToken::new();
        let result = chunked_search(&search_broker, &search_key, 100, &cancellation).await;
        (result, std::time::Instant::now())
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    let read_started = std::time::Instant::now();
    let read = page_count_command(&broker, &key, None).await.unwrap();
    let read_finished = std::time::Instant::now();
    assert!(matches!(read, WorkerResult::PageCount(1)));

    let (result, search_finished) = search.await.unwrap();
    assert_eq!(result.unwrap().total_matches, 6);
    assert!(
        read_finished < search_finished,
        "the read waited for the whole search"
    );
    assert!(
        read_finished - read_started < std::time::Duration::from_millis(300),
        "the read waited longer than about one chunk: {:?}",
        read_finished - read_started
    );
    broker.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelling_a_search_mid_chunk_keeps_the_worker() {
    let broker = open_with_mode("chunked-search").await.unwrap();
    let key = key("chunked-search");
    let epoch = broker.worker_epoch().await;
    let cancellation = cancel_after(120);
    let error = chunked_search(&broker, &key, 100, &cancellation)
        .await
        .unwrap_err();
    assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
    assert_eq!(broker.worker_epoch().await, epoch);
    assert_eq!(broker.restart_attempts_for_test().await, 0);

    let fresh = tokio_util::sync::CancellationToken::new();
    let result = chunked_search(&broker, &key, 100, &fresh).await.unwrap();
    assert_eq!(result.total_matches, 6);
    assert_eq!(broker.worker_epoch().await, epoch);
    broker.shutdown().await.unwrap();
}

async fn render_page_with_short_timeout(
    broker: &PdfiumWorkerBroker,
    key: &WorkerDocumentKey,
    page_index: u32,
) -> Result<(), String> {
    let admission = broker
        .admit_read(key, None)
        .await
        .map_err(|error| error.code.to_string())?;
    broker
        .ensure_document(&admission, key.clone(), "fake.pdf".to_string(), None, None)
        .await
        .map_err(|error| error.code.to_string())?;
    // The page scope comes from the command, not from the timeout class, so a
    // short class keeps the test fast without a test-only timeout override.
    let result = broker
        .command_for_document(
            &admission,
            key,
            |document_id| WorkerCommand::RenderPage {
                document_id,
                page_index,
                target: preview_alternative_lib::worker::ipc::WorkerRenderTarget::Scale(1.0),
                rotation: 0,
            },
            CommandTimeout::Ping,
            None,
        )
        .await
        .map_err(|error| error.code.to_string())?;
    assert!(matches!(result, WorkerResult::PageRendered { .. }));
    Ok(())
}

async fn release_page(
    broker: &PdfiumWorkerBroker,
    key: &WorkerDocumentKey,
    page_index: u32,
) -> Result<bool, String> {
    let admission = broker
        .admit_read(key, None)
        .await
        .map_err(|error| error.code.to_string())?;
    broker
        .release_page_quarantine(&admission, key, page_index)
        .await
        .map_err(|error| error.code.to_string())
}

#[tokio::test]
async fn repeated_page_timeouts_quarantine_the_page_and_keep_the_document_usable() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("hang-render-pages=1")));
    let key = key("page-quarantine");

    for _ in 0..2 {
        assert_eq!(
            render_page_with_short_timeout(&broker, &key, 1).await,
            Err("PDFIUM_WORKER_TIMEOUT".to_string())
        );
    }
    // The other pages of the same generation still render.
    render_page_with_short_timeout(&broker, &key, 0)
        .await
        .unwrap();

    let epoch = broker.worker_epoch().await;
    let restarts = broker.restart_attempts_for_test().await;
    assert_eq!(
        render_page_with_short_timeout(&broker, &key, 1).await,
        Err("PDFIUM_PAGE_QUARANTINED".to_string())
    );
    // Rejected before the worker saw it: no kill and no restart charge.
    assert_eq!(broker.worker_epoch().await, epoch);
    assert_eq!(broker.restart_attempts_for_test().await, restarts);

    // A retry grants exactly one more attempt inside the page window.
    assert_eq!(release_page(&broker, &key, 1).await, Ok(true));
    assert_eq!(release_page(&broker, &key, 1).await, Ok(false));
    assert_eq!(
        render_page_with_short_timeout(&broker, &key, 1).await,
        Err("PDFIUM_WORKER_TIMEOUT".to_string())
    );
    assert_eq!(
        render_page_with_short_timeout(&broker, &key, 1).await,
        Err("PDFIUM_PAGE_QUARANTINED".to_string())
    );
    render_page_with_short_timeout(&broker, &key, 0)
        .await
        .unwrap();
    assert_eq!(
        release_page(&broker, &key, 6).await,
        Err("PDFIUM_WORKER_INVALID_PAGE_INDEX".to_string())
    );

    // Generation invalidation (mutation, close, candidate discard) clears it.
    let exclusive = broker.admit_exclusive(&key.session_id).await.unwrap();
    broker.invalidate_document(&exclusive, &key).await.unwrap();
    drop(exclusive);
    assert_eq!(release_page(&broker, &key, 1).await, Ok(false));
}

#[tokio::test]
async fn timeouts_on_distinct_pages_quarantine_the_document() {
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("hang-render-pages=1,2,3")));
    let key = key("document-quarantine");

    for page in 1..=3 {
        assert_eq!(
            render_page_with_short_timeout(&broker, &key, page).await,
            Err("PDFIUM_WORKER_TIMEOUT".to_string())
        );
    }
    assert_eq!(
        render_page_with_short_timeout(&broker, &key, 0).await,
        Err("PDFIUM_DOCUMENT_QUARANTINED".to_string())
    );
    assert_eq!(
        release_page(&broker, &key, 1).await,
        Err("PDFIUM_DOCUMENT_QUARANTINED".to_string())
    );
    // Another generation of the same session is a different document.
    let next_generation = WorkerDocumentKey {
        session_id: key.session_id.clone(),
        content_generation: key.content_generation + 1,
    };
    render_page_with_short_timeout(&broker, &next_generation, 0)
        .await
        .unwrap();
}
