//! TEMPORARY diagnostic for the Windows acceptance hang (PR #22). Not a
//! certification test; removed once the hang is understood.
//!
//! Walks the exact setup the sanitization/secure-redaction acceptance suites
//! use, logging each phase with a timestamp to stderr. A watchdog thread that
//! does not depend on the async runtime names the phase it is stuck in and
//! ends the process, so a synchronous hang still produces a diagnosis.

#![cfg(feature = "sanitization-test-support")]

use parking_lot::{Mutex, RwLock};
use preview_alternative_lib::document_session::{self, SessionJobOutcome};
use preview_alternative_lib::pdf_engine::render_service::RenderService;
use preview_alternative_lib::pdf_engine::subprocess_manager::{
    PdfiumWorkerBroker, WorkerLauncher,
};
use preview_alternative_lib::sanitization;
use preview_alternative_lib::sanitization_inspection::SanitizationInspectionTestBehavior;
use preview_alternative_lib::{
    ActiveRenderTasks, ActiveSearchId, PasswordCache, SearchCache, TextLayoutCache,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;

static START: Mutex<Option<Instant>> = Mutex::new(None);
static PHASE: Mutex<(&'static str, Option<Instant>)> = Mutex::new(("not started", None));

fn elapsed() -> f64 {
    START
        .lock()
        .map_or(0.0, |start| start.elapsed().as_secs_f64())
}

fn phase(name: &'static str) {
    eprintln!("[probe {:>7.2}s] >> {name}", elapsed());
    *PHASE.lock() = (name, Some(Instant::now()));
}

fn note(message: impl AsRef<str>) {
    eprintln!("[probe {:>7.2}s]    {}", elapsed(), message.as_ref());
}

fn start_watchdog() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(15));
        let (name, since) = *PHASE.lock();
        let stuck = since.map_or(0, |since| since.elapsed().as_secs());
        eprintln!("[probe {:>7.2}s] .. still in '{name}' for {stuck}s", elapsed());
        if stuck >= 240 {
            eprintln!("[probe] HANG: '{name}' did not finish within 240 s; exiting");
            std::process::exit(3);
        }
    });
}

fn worker_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_preview-alternative"))
}

#[test]
fn windows_session_activation_and_inspection_probe() {
    *START.lock() = Some(Instant::now());
    start_watchdog();

    note(format!("os={} arch={}", std::env::consts::OS, std::env::consts::ARCH));
    note(format!("worker exe={}", worker_executable().display()));
    for key in ["PDFIUM_PATH", "PDFIUM_RESOLVED_PATH", "TEMP", "TMP"] {
        note(format!("{key}={:?}", std::env::var_os(key)));
    }
    note(format!("temp_dir={}", std::env::temp_dir().display()));

    // 1. The worker executable on its own: it must start and exit on stdin EOF.
    phase("direct worker spawn (--run-worker, stdin closed)");
    let mut child = Command::new(worker_executable())
        .arg("--run-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("worker spawn");
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    match status {
        Some(status) => note(format!("worker exited on EOF: {status}")),
        None => {
            note("worker did NOT exit within 30 s of stdin EOF; killing it");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    note(format!(
        "worker stderr (first 2000 bytes): {:?}",
        &stderr[..stderr.len().min(2000)]
    ));

    // 2. The acceptance suites' app() setup, step by step.
    phase("tokio runtime (current_thread, as #[tokio::test])");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    phase("PdfiumWorkerBroker::with_launcher");
    let broker = PdfiumWorkerBroker::with_launcher(Ok(WorkerLauncher::for_test(
        worker_executable(),
        vec![OsString::from("--run-worker")],
    )));

    phase("DocumentSessionStore::initialize_with_password_cache_and_broker");
    let password_cache = Arc::new(Mutex::new(HashMap::new()));
    let store = {
        let _guard = runtime.enter();
        document_session::DocumentSessionStore::initialize_with_password_cache_and_broker(
            Arc::clone(&password_cache),
            broker,
        )
        .unwrap()
    };

    phase("tauri mock app build");
    let capacity = std::num::NonZeroUsize::new(16).unwrap();
    let app = {
        let _guard = runtime.enter();
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
    };

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sanitization/clean-minimal.pdf");
    runtime.block_on(async {
        phase("open_document_session");
        let opened = document_session::open_document_session(
            fixture.to_string_lossy().into_owned(),
            app.state(),
        )
        .await
        .unwrap();
        note(format!("session {}", opened.session_id));

        phase("prepare_candidate_activation");
        let prepared = document_session::prepare_candidate_activation(
            opened.session_id,
            None,
            None,
            app.state(),
        )
        .await
        .unwrap();

        phase("commit_candidate_activation");
        let session = document_session::commit_candidate_activation(
            prepared.activation_token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap()
        .snapshot;

        phase("inspect_document_sanitization (real worker)");
        let outcome = sanitization::inspect_document_sanitization_core_with_test_support(
            uuid::Uuid::new_v4().to_string(),
            session.id.clone(),
            session.content_generation,
            sanitization::SANITIZATION_POLICY_SCHEMA_VERSION.to_string(),
            &app.state::<document_session::DocumentSessionStore>(),
            Duration::from_secs(60),
            SanitizationInspectionTestBehavior::Real,
        )
        .await
        .unwrap();
        note(format!(
            "inspection completed: {}",
            matches!(outcome, SessionJobOutcome::Completed { .. })
        ));

        phase("close_all");
        app.state::<document_session::DocumentSessionStore>()
            .close_all()
            .await
            .unwrap();
    });

    phase("drop app");
    drop(app);
    phase("done");
}
