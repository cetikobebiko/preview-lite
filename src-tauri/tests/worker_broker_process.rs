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
    let broker = PdfiumWorkerBroker::with_launcher(Ok(helper_launcher("hang-command")));
    let key = key("cancel-active");
    let admission = broker.admit_read(&key, None).await.unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
