use super::memory_watchdog;
use crate::worker::ipc::{
    decode_response, encode_request, WorkerCapability, WorkerCommand, WorkerCredential,
    WorkerFailure, WorkerFailureKind, WorkerRequest, WorkerResponse, WorkerResult,
    WorkerSearchChunk, MAX_WORKER_RESPONSE_BYTES, WORKER_PROTOCOL_VERSION,
};
use parking_lot::Mutex as SyncMutex;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const STDERR_JOIN_TIMEOUT: Duration = Duration::from_secs(2);
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(3);
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const MAX_RESTARTS_PER_WINDOW: usize = 5;
const DOCUMENT_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const DOCUMENT_QUARANTINE_THRESHOLD: usize = 3;
// Timeouts of single-page commands are attributed to the page, so one page
// whose content is too heavy for PDFium is quarantined on its own instead of
// the whole document generation. The window is longer than the document
// window because the worker serves one command at a time and a page render
// may run for 20 s before it times out: three of them never fit in 60 s.
const PAGE_TIMEOUT_WINDOW: Duration = Duration::from_secs(300);
const PAGE_TIMEOUT_QUARANTINE_THRESHOLD: usize = 2;
// Every page timeout still kills the shared worker, so timeouts spread over
// pages escalate to a document quarantine. Initial bounds pending the
// worker-metrics baseline.
const DOCUMENT_TIMEOUT_DISTINCT_PAGE_THRESHOLD: usize = 3;
const DOCUMENT_TIMEOUT_TOTAL_THRESHOLD: usize = 6;
const ADMISSION_DRAIN_TIMEOUT: Duration = Duration::from_secs(3);
static NEXT_ADMISSION_CONTROLLER_ID: AtomicU64 = AtomicU64::new(1);

/// Diagnostic `tracing` target for worker lifecycle and cancellation metrics.
/// Events carry only command classes, durations, epochs and causes; never
/// paths, credentials, queries or payloads. Enable with
/// `RUST_LOG=pdfium_worker_metrics=debug`.
pub const WORKER_METRICS_TARGET: &str = "pdfium_worker_metrics";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorkerDocumentKey {
    pub session_id: String,
    pub content_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerDocumentRef {
    pub worker_epoch: u64,
    pub worker_document_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegisteredDocument {
    reference: WorkerDocumentRef,
    // Fixed for a worker document: the worker opens it read-only and every
    // mutation commits a new content generation, which is a new key.
    page_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminationCause {
    Cancelled,
    Abandoned,
    HealthFailure,
    CloseFailure,
    Shutdown,
}

impl TerminationCause {
    fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
            Self::HealthFailure => "health_failure",
            Self::CloseFailure => "close_failure",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerFailureReason {
    Crash,
    Timeout,
    ProtocolViolation,
    /// macOS: the worker's phys_footprint passed its budget and it was killed.
    MemoryLimit,
}

#[derive(Clone, Debug)]
pub struct BrokerError {
    pub code: &'static str,
    pub message: String,
}

impl BrokerError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self::new("PDFIUM_WORKER_UNAVAILABLE", message)
    }

    fn cancelled() -> Self {
        Self::new(
            "PDFIUM_WORKER_CANCELLED",
            "PDF worker request was cancelled before completion",
        )
    }
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BrokerError {}

#[derive(Clone, Debug)]
pub struct WorkerLauncher {
    executable: PathBuf,
    arguments: Vec<OsString>,
    pdfium_path: Option<OsString>,
    resolved_runtime_path: Option<PathBuf>,
    memory_limit_bytes: u64,
}

impl WorkerLauncher {
    pub fn current_executable() -> Result<Self, BrokerError> {
        let executable = std::env::current_exe().map_err(|error| {
            BrokerError::new(
                "PDFIUM_WORKER_SPAWN_FAILED",
                format!("Worker executable could not be resolved: {error}"),
            )
        })?;
        // Resolve the runtime path in the parent process to ensure main/worker parity
        let resolved_runtime_path = crate::pdf_engine::resolve_runtime_path().ok();
        Ok(Self {
            executable,
            arguments: vec![OsString::from("--run-worker")],
            pdfium_path: std::env::var_os("PDFIUM_PATH"),
            resolved_runtime_path,
            memory_limit_bytes: memory_watchdog::WORKER_FOOTPRINT_LIMIT_BYTES,
        })
    }

    #[doc(hidden)]
    pub fn for_test(executable: PathBuf, arguments: Vec<OsString>) -> Self {
        Self {
            executable,
            arguments,
            pdfium_path: None,
            resolved_runtime_path: None,
            memory_limit_bytes: memory_watchdog::WORKER_FOOTPRINT_LIMIT_BYTES,
        }
    }

    /// Lowers the macOS footprint budget so tests can cross it without
    /// allocating gigabytes.
    #[doc(hidden)]
    pub fn with_memory_limit_for_test(mut self, bytes: u64) -> Self {
        self.memory_limit_bytes = bytes;
        self
    }
}

#[derive(Clone)]
pub struct PdfiumWorkerBroker {
    state: Arc<Mutex<BrokerState>>,
    admission: Arc<AdmissionController>,
    launcher: Result<WorkerLauncher, BrokerError>,
    shutting_down: Arc<AtomicBool>,
    shutdown_complete: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
    shutdown_error: Arc<SyncMutex<Option<BrokerError>>>,
    shutdown_cancellation: CancellationToken,
    pending_reservation_cleanup: Arc<SyncMutex<Vec<(WorkerDocumentKey, WorkerDocumentRef)>>>,
}

struct BrokerState {
    worker: Option<RunningWorker>,
    next_request_id: u64,
    next_worker_epoch: u64,
    documents: HashMap<WorkerDocumentKey, RegisteredDocument>,
    last_termination: Option<TerminationCause>,
    // A replacement is charged only after a worker-health or startup failure.
    // Expected cancellation still tears down the active worker to preserve the
    // single in-flight IPC protocol, but it must not look like a crash loop.
    restart_required: bool,
    restart_events: VecDeque<Instant>,
    document_failures: HashMap<WorkerDocumentKey, VecDeque<FailureEvent>>,
    quarantined: HashMap<WorkerDocumentKey, WorkerFailureReason>,
    quarantined_pages: HashMap<WorkerDocumentKey, BTreeSet<u32>>,
    last_worker_stderr: Vec<u8>,
}

struct FailureEvent {
    at: Instant,
    reason: WorkerFailureReason,
    // Set only for timeouts of page-scoped commands; see `page_scope`.
    page: Option<u32>,
}

struct RunningWorker {
    epoch: u64,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr_drain: Option<JoinHandle<()>>,
    stderr_buffer: Arc<SyncMutex<Vec<u8>>>,
    // Request ID written to the worker whose complete response has not been
    // read yet. A later exchange that finds it set cannot trust the stdout
    // stream position, so the worker is replaced instead of read from.
    in_flight: Option<u64>,
    memory_limit_bytes: u64,
}

struct AdmissionState {
    closed: bool,
    active: Option<ActiveAdmission>,
    next_ticket: u64,
    waiters: VecDeque<AdmissionWaiter>,
    blocked_sessions: HashMap<String, usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdmissionMode {
    Read,
    Exclusive,
}

struct ActiveAdmission {
    ticket: u64,
    session_id: String,
    mode: AdmissionMode,
}

struct AdmissionWaiter {
    ticket: u64,
    session_id: String,
    mode: AdmissionMode,
    sender: oneshot::Sender<Result<AdmissionGrant, BrokerError>>,
}

struct AdmissionController {
    state: SyncMutex<AdmissionState>,
    controller_id: u64,
    idle_notify: Notify,
}

pub struct WorkerAdmissionPermit {
    controller: Arc<AdmissionController>,
    controller_id: u64,
    ticket: u64,
    session_id: String,
    mode: AdmissionMode,
}

#[derive(Debug)]
struct AdmissionGrant {
    ticket: u64,
}

struct AdmissionRegistration {
    controller: Arc<AdmissionController>,
    ticket: u64,
    armed: bool,
}

pub struct WorkerDocumentReservation {
    broker: PdfiumWorkerBroker,
    key: WorkerDocumentKey,
    reference: WorkerDocumentRef,
    page_count: u32,
    protected: bool,
    armed: bool,
}

impl WorkerDocumentReservation {
    pub fn document_ref(&self) -> WorkerDocumentRef {
        self.reference
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn protected(&self) -> bool {
        self.protected
    }

    pub fn commit(mut self) -> (WorkerDocumentRef, u32, bool) {
        self.armed = false;
        (self.reference, self.page_count, self.protected)
    }

    pub async fn abort(mut self, permit: &WorkerAdmissionPermit) -> Result<(), BrokerError> {
        self.armed = false;
        self.broker
            .abort_reserved_document(permit, &self.key, self.reference)
            .await
    }
}

impl Drop for WorkerDocumentReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Explicit `abort().await` remains mandatory on ordinary fallible paths. Drop cannot
        // perform async I/O, so it transfers exact key/reference ownership to the broker. The
        // broker drains this queue before the next operation and during bounded shutdown.
        self.broker
            .pending_reservation_cleanup
            .lock()
            .push((self.key.clone(), self.reference));
    }
}

impl Drop for WorkerAdmissionPermit {
    fn drop(&mut self) {
        self.controller.release(self.ticket);
    }
}

impl Drop for AdmissionRegistration {
    fn drop(&mut self) {
        if self.armed {
            self.controller.cancel_registration(self.ticket);
        }
    }
}

impl AdmissionRegistration {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Default for AdmissionController {
    fn default() -> Self {
        Self {
            state: SyncMutex::new(AdmissionState {
                closed: false,
                active: None,
                next_ticket: 1,
                waiters: VecDeque::new(),
                blocked_sessions: HashMap::new(),
            }),
            controller_id: NEXT_ADMISSION_CONTROLLER_ID.fetch_add(1, Ordering::Relaxed),
            idle_notify: Notify::new(),
        }
    }
}

impl AdmissionController {
    async fn acquire(
        self: &Arc<Self>,
        session_id: &str,
        mode: AdmissionMode,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerAdmissionPermit, BrokerError> {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(BrokerError::cancelled());
        }
        let started = Instant::now();
        let (mut registration, receiver) = self.register(session_id, mode)?;
        let result = if let Some(cancellation) = cancellation {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Err(BrokerError::cancelled()),
                result = receiver => result.unwrap_or_else(|_| Err(BrokerError::cancelled())),
            }
        } else {
            receiver
                .await
                .unwrap_or_else(|_| Err(BrokerError::cancelled()))
        };
        tracing::debug!(
            target: WORKER_METRICS_TARGET,
            event = "admission",
            mode = ?mode,
            outcome = match &result {
                Ok(_) => "granted",
                Err(error) if error.code == "PDFIUM_WORKER_CANCELLED" => "cancelled_in_queue",
                Err(_) => "rejected",
            },
            wait_ms = started.elapsed().as_secs_f64() * 1000.0,
        );
        let grant = result?;
        let permit = WorkerAdmissionPermit {
            controller: Arc::clone(self),
            controller_id: self.controller_id,
            ticket: grant.ticket,
            session_id: session_id.to_string(),
            mode,
        };
        registration.disarm();
        Ok(permit)
    }

    fn register(
        self: &Arc<Self>,
        session_id: &str,
        mode: AdmissionMode,
    ) -> Result<
        (
            AdmissionRegistration,
            oneshot::Receiver<Result<AdmissionGrant, BrokerError>>,
        ),
        BrokerError,
    > {
        let (ticket, receiver) = {
            let mut state = self.state.lock();
            if state.closed {
                return Err(BrokerError::unavailable(
                    "PDF worker admission is closed for shutdown",
                ));
            }
            if mode == AdmissionMode::Read && state.blocked_sessions.contains_key(session_id) {
                return Err(BrokerError::cancelled());
            }
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.checked_add(1).ok_or_else(|| {
                BrokerError::new(
                    "PDFIUM_WORKER_INTERNAL",
                    "PDF worker admission ticket counter exhausted",
                )
            })?;
            let (sender, receiver) = oneshot::channel();
            let waiter = AdmissionWaiter {
                ticket,
                session_id: session_id.to_string(),
                mode,
                sender,
            };
            if mode == AdmissionMode::Exclusive {
                increment_session_block(&mut state, session_id);
                cancel_queued_reads_for_session(&mut state, session_id);
                let first_read = state
                    .waiters
                    .iter()
                    .position(|queued| queued.mode == AdmissionMode::Read)
                    .unwrap_or(state.waiters.len());
                state.waiters.insert(first_read, waiter);
            } else {
                state.waiters.push_back(waiter);
            }
            dispatch_next(&mut state);
            (ticket, receiver)
        };
        Ok((
            AdmissionRegistration {
                controller: Arc::clone(self),
                ticket,
                armed: true,
            },
            receiver,
        ))
    }

    async fn acquire_read(
        self: &Arc<Self>,
        session_id: &str,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerAdmissionPermit, BrokerError> {
        self.acquire(session_id, AdmissionMode::Read, cancellation)
            .await
    }

    async fn acquire_exclusive(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<WorkerAdmissionPermit, BrokerError> {
        self.acquire(session_id, AdmissionMode::Exclusive, None)
            .await
    }

    fn cancel_registration(&self, ticket: u64) {
        let mut state = self.state.lock();
        if let Some(index) = state
            .waiters
            .iter()
            .position(|waiter| waiter.ticket == ticket)
        {
            if let Some(waiter) = state.waiters.remove(index) {
                if waiter.mode == AdmissionMode::Exclusive {
                    decrement_session_block(&mut state, &waiter.session_id);
                }
            }
        } else if state
            .active
            .as_ref()
            .is_some_and(|active| active.ticket == ticket)
        {
            if let Some(active) = state.active.take() {
                if active.mode == AdmissionMode::Exclusive {
                    decrement_session_block(&mut state, &active.session_id);
                }
            }
        }
        dispatch_next(&mut state);
        if state.active.is_none() {
            self.idle_notify.notify_waiters();
        }
    }

    fn cancel_session_reads(&self, session_id: &str) {
        let mut state = self.state.lock();
        cancel_queued_reads_for_session(&mut state, session_id);
        dispatch_next(&mut state);
    }

    fn release(&self, ticket: u64) {
        let mut state = self.state.lock();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.ticket == ticket)
        {
            if let Some(active) = state.active.take() {
                if active.mode == AdmissionMode::Exclusive {
                    decrement_session_block(&mut state, &active.session_id);
                }
            }
        }
        dispatch_next(&mut state);
        if state.active.is_none() {
            self.idle_notify.notify_waiters();
        }
    }

    fn validate(
        &self,
        permit: &WorkerAdmissionPermit,
        session_id: &str,
        required: AdmissionMode,
    ) -> Result<(), BrokerError> {
        if permit.controller_id != self.controller_id
            || permit.session_id != session_id
            || (required == AdmissionMode::Exclusive && permit.mode != AdmissionMode::Exclusive)
        {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_ADMISSION_INVALID",
                "PDF worker admission permit does not authorize this operation",
            ));
        }
        let state = self.state.lock();
        if !state.active.as_ref().is_some_and(|active| {
            active.ticket == permit.ticket
                && active.session_id == permit.session_id
                && active.mode == permit.mode
        }) {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_ADMISSION_INVALID",
                "PDF worker admission permit is no longer active",
            ));
        }
        Ok(())
    }

    fn close(&self) {
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        state.closed = true;
        while let Some(waiter) = state.waiters.pop_front() {
            if waiter.mode == AdmissionMode::Exclusive {
                decrement_session_block(&mut state, &waiter.session_id);
            }
            let _ = waiter.sender.send(Err(BrokerError::unavailable(
                "PDF worker admission was closed for shutdown",
            )));
        }
        if state.active.is_none() {
            self.idle_notify.notify_waiters();
        }
    }

    async fn wait_until_idle(&self, timeout: Duration) -> Result<(), BrokerError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.idle_notify.notified();
            if self.state.lock().active.is_none() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() || tokio::time::timeout(remaining, notified).await.is_err() {
                return Err(BrokerError::new(
                    "PDFIUM_WORKER_CLEANUP_FAILED",
                    "Active PDF worker admission did not drain before shutdown",
                ));
            }
        }
    }
}

fn increment_session_block(state: &mut AdmissionState, session_id: &str) {
    let count = state
        .blocked_sessions
        .entry(session_id.to_string())
        .or_insert(0);
    *count = count.saturating_add(1);
}

fn decrement_session_block(state: &mut AdmissionState, session_id: &str) {
    if let Some(count) = state.blocked_sessions.get_mut(session_id) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            state.blocked_sessions.remove(session_id);
        }
    }
}

fn cancel_queued_reads_for_session(state: &mut AdmissionState, session_id: &str) {
    state
        .waiters
        .retain(|waiter| waiter.session_id != session_id || waiter.mode != AdmissionMode::Read);
}

fn dispatch_next(state: &mut AdmissionState) {
    if state.closed || state.active.is_some() {
        return;
    }
    while let Some(waiter) = state.waiters.pop_front() {
        if waiter.mode == AdmissionMode::Read
            && state.blocked_sessions.contains_key(&waiter.session_id)
        {
            continue;
        }
        state.active = Some(ActiveAdmission {
            ticket: waiter.ticket,
            session_id: waiter.session_id.clone(),
            mode: waiter.mode,
        });
        if waiter
            .sender
            .send(Ok(AdmissionGrant {
                ticket: waiter.ticket,
            }))
            .is_ok()
        {
            break;
        }
        if let Some(active) = state.active.take() {
            if active.mode == AdmissionMode::Exclusive {
                decrement_session_block(state, &active.session_id);
            }
        }
    }
}

impl PdfiumWorkerBroker {
    pub fn new() -> Self {
        Self::with_launcher(WorkerLauncher::current_executable())
    }

    pub fn with_launcher(launcher: Result<WorkerLauncher, BrokerError>) -> Self {
        Self {
            state: Arc::new(Mutex::new(BrokerState {
                worker: None,
                next_request_id: 1,
                next_worker_epoch: 1,
                documents: HashMap::new(),
                last_termination: None,
                restart_required: false,
                restart_events: VecDeque::new(),
                document_failures: HashMap::new(),
                quarantined: HashMap::new(),
                quarantined_pages: HashMap::new(),
                last_worker_stderr: Vec::new(),
            })),
            admission: Arc::new(AdmissionController::default()),
            launcher,
            shutting_down: Arc::new(AtomicBool::new(false)),
            shutdown_complete: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
            shutdown_error: Arc::new(SyncMutex::new(None)),
            shutdown_cancellation: CancellationToken::new(),
            pending_reservation_cleanup: Arc::new(SyncMutex::new(Vec::new())),
        }
    }

    pub async fn admit_read(
        &self,
        key: &WorkerDocumentKey,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerAdmissionPermit, BrokerError> {
        self.admission
            .acquire_read(&key.session_id, cancellation)
            .await
    }

    pub async fn admit_exclusive(
        &self,
        session_id: &str,
    ) -> Result<WorkerAdmissionPermit, BrokerError> {
        self.admission.acquire_exclusive(session_id).await
    }

    pub fn cancel_queued_session_reads(&self, session_id: &str) {
        self.admission.cancel_session_reads(session_id);
    }

    pub fn begin_shutdown(&self) {
        self.admission.close();
        self.shutdown_cancellation.cancel();
    }

    pub async fn ensure_document(
        &self,
        permit: &WorkerAdmissionPermit,
        key: WorkerDocumentKey,
        path: String,
        credential: Option<String>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<(WorkerDocumentRef, u32, bool), BrokerError> {
        self.admission
            .validate(permit, &key.session_id, AdmissionMode::Read)?;
        self.ensure_accepting()?;
        let mut state = self.state.lock().await;
        self.drain_pending_reservation_cleanup(&mut state).await?;
        if state.quarantined.contains_key(&key) {
            return Err(document_quarantined_error());
        }
        self.ensure_spawned(&mut state).await?;
        let current_epoch = state
            .worker
            .as_ref()
            .map(|worker| worker.epoch)
            .ok_or_else(|| BrokerError::unavailable("PDF worker is not running"))?;
        if let Some(registered) = state.documents.get(&key).copied() {
            if registered.reference.worker_epoch == current_epoch {
                // The page count recorded at OpenDocument stays valid for this
                // key, so a reused document costs no worker round-trip.
                return Ok((registered.reference, registered.page_count, false));
            }
            state.documents.remove(&key);
        }

        let credential = credential
            .map(WorkerCredential::new)
            .transpose()
            .map_err(map_worker_failure)?;
        let result = self
            .exchange(
                &mut state,
                WorkerCommand::OpenDocument { path, credential },
                CommandTimeout::OpenDocument,
                cancellation,
                Some(&key),
                None,
            )
            .await?;
        let WorkerResult::DocumentOpened {
            document_id,
            page_count,
            protected,
        } = result
        else {
            return Err(self
                .protocol_violation(
                    &mut state,
                    Some(&key),
                    "OpenDocument returned an unexpected response",
                )
                .await);
        };
        let reference = WorkerDocumentRef {
            worker_epoch: current_epoch,
            worker_document_id: document_id,
        };
        state.documents.insert(
            key,
            RegisteredDocument {
                reference,
                page_count,
            },
        );
        Ok((reference, page_count, protected))
    }

    pub async fn reserve_document(
        &self,
        permit: &WorkerAdmissionPermit,
        key: WorkerDocumentKey,
        path: String,
        credential: Option<String>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerDocumentReservation, BrokerError> {
        let (reference, page_count, protected) = self
            .ensure_document(permit, key.clone(), path, credential, cancellation)
            .await?;
        Ok(WorkerDocumentReservation {
            broker: self.clone(),
            key,
            reference,
            page_count,
            protected,
            armed: true,
        })
    }

    pub async fn command_for_document(
        &self,
        permit: &WorkerAdmissionPermit,
        key: &WorkerDocumentKey,
        command: impl FnOnce(u64) -> WorkerCommand,
        timeout: CommandTimeout,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerResult, BrokerError> {
        self.admission
            .validate(permit, &key.session_id, AdmissionMode::Read)?;
        self.ensure_accepting()?;
        let mut state = self.state.lock().await;
        self.drain_pending_reservation_cleanup(&mut state).await?;
        if state.quarantined.contains_key(key) {
            return Err(document_quarantined_error());
        }
        let current_epoch = state
            .worker
            .as_ref()
            .map(|worker| worker.epoch)
            .ok_or_else(|| BrokerError::unavailable("PDF worker is not running"))?;
        let reference = state
            .documents
            .get(key)
            .map(|registered| registered.reference)
            .ok_or_else(|| {
                BrokerError::new(
                    "PDFIUM_WORKER_PROTOCOL_ERROR",
                    "Worker document is not open for this session generation",
                )
            })?;
        if reference.worker_epoch != current_epoch {
            state.documents.remove(key);
            return Err(BrokerError::new(
                "PDFIUM_WORKER_PROTOCOL_ERROR",
                "Worker document belongs to a stale worker epoch",
            ));
        }
        let command = command(reference.worker_document_id);
        let page = page_scope(&command);
        if page.is_some_and(|page| {
            state
                .quarantined_pages
                .get(key)
                .is_some_and(|pages| pages.contains(&page))
        }) {
            // Rejected before it reaches the worker: no kill, no restart charge.
            return Err(BrokerError::new(
                "PDFIUM_PAGE_QUARANTINED",
                "This page repeatedly timed out in the PDF worker",
            ));
        }
        self.exchange(&mut state, command, timeout, cancellation, Some(key), page)
            .await
    }

    /// Lets the user retry one quarantined page. Its failure history is kept,
    /// so another timeout inside the page window quarantines it again at once,
    /// and the document-level timeout ceiling keeps counting.
    pub async fn release_page_quarantine(
        &self,
        permit: &WorkerAdmissionPermit,
        key: &WorkerDocumentKey,
        page_index: u32,
    ) -> Result<bool, BrokerError> {
        self.admission
            .validate(permit, &key.session_id, AdmissionMode::Read)?;
        self.ensure_accepting()?;
        let mut state = self.state.lock().await;
        if state.quarantined.contains_key(key) {
            return Err(document_quarantined_error());
        }
        if state
            .documents
            .get(key)
            .is_some_and(|registered| page_index >= registered.page_count)
        {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_INVALID_PAGE_INDEX",
                "Page index is outside the document",
            ));
        }
        let Some(pages) = state.quarantined_pages.get_mut(key) else {
            return Ok(false);
        };
        let released = pages.remove(&page_index);
        if pages.is_empty() {
            state.quarantined_pages.remove(key);
        }
        Ok(released)
    }

    pub async fn invalidate_document(
        &self,
        permit: &WorkerAdmissionPermit,
        key: &WorkerDocumentKey,
    ) -> Result<(), BrokerError> {
        self.admission
            .validate(permit, &key.session_id, AdmissionMode::Exclusive)?;
        let mut state = self.state.lock().await;
        self.drain_pending_reservation_cleanup(&mut state).await?;
        self.invalidate_document_internal(&mut state, key, None)
            .await
    }

    async fn abort_reserved_document(
        &self,
        permit: &WorkerAdmissionPermit,
        key: &WorkerDocumentKey,
        reference: WorkerDocumentRef,
    ) -> Result<(), BrokerError> {
        self.admission
            .validate(permit, &key.session_id, AdmissionMode::Read)?;
        let mut state = self.state.lock().await;
        self.reap_abandoned_exchange(&mut state).await?;
        self.invalidate_document_internal(&mut state, key, Some(reference))
            .await
    }

    async fn invalidate_document_internal(
        &self,
        state: &mut BrokerState,
        key: &WorkerDocumentKey,
        expected: Option<WorkerDocumentRef>,
    ) -> Result<(), BrokerError> {
        state.document_failures.remove(key);
        state.quarantined.remove(key);
        state.quarantined_pages.remove(key);
        let Some(reference) = state
            .documents
            .get(key)
            .map(|registered| registered.reference)
        else {
            return Ok(());
        };
        if expected.is_some_and(|expected| expected != reference) {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_ADMISSION_INVALID",
                "Worker reservation no longer owns this document mapping",
            ));
        }
        state.documents.remove(key);
        let Some(worker) = state.worker.as_ref() else {
            return Ok(());
        };
        if worker.epoch != reference.worker_epoch {
            return Ok(());
        }
        match self
            .exchange(
                state,
                WorkerCommand::CloseDocument {
                    document_id: reference.worker_document_id,
                },
                CommandTimeout::CloseDocument,
                None,
                None,
                None,
            )
            .await
        {
            Ok(WorkerResult::DocumentClosed) => Ok(()),
            Ok(_) => Err(self
                .protocol_violation(state, None, "CloseDocument returned an unexpected response")
                .await),
            Err(error) => {
                state.restart_required = true;
                self.terminate_worker(state, TerminationCause::CloseFailure)
                    .await?;
                let _ = error;
                Ok(())
            }
        }
    }

    /// Replaces a worker that still owes a response to an exchange whose
    /// future was dropped. Reading from it would consume the stale frame, so
    /// the worker is killed instead; this is not a health failure.
    async fn reap_abandoned_exchange(&self, state: &mut BrokerState) -> Result<(), BrokerError> {
        let Some(request_id) = state.worker.as_ref().and_then(|worker| worker.in_flight) else {
            return Ok(());
        };
        tracing::debug!(
            worker_epoch = state.worker.as_ref().map(|worker| worker.epoch),
            request_id,
            "PDF worker with an abandoned in-flight request is being replaced"
        );
        self.terminate_worker(state, TerminationCause::Abandoned)
            .await
    }

    async fn drain_pending_reservation_cleanup(
        &self,
        state: &mut BrokerState,
    ) -> Result<(), BrokerError> {
        self.reap_abandoned_exchange(state).await?;
        let pending = std::mem::take(&mut *self.pending_reservation_cleanup.lock());
        for (key, reference) in pending {
            self.invalidate_document_internal(state, &key, Some(reference))
                .await?;
        }
        Ok(())
    }

    pub async fn invalidate_session(
        &self,
        permit: &WorkerAdmissionPermit,
        session_id: &str,
    ) -> Result<(), BrokerError> {
        self.admission
            .validate(permit, session_id, AdmissionMode::Exclusive)?;
        let keys = {
            let state = self.state.lock().await;
            state
                .documents
                .keys()
                .filter(|key| key.session_id == session_id)
                .cloned()
                .collect::<Vec<_>>()
        };
        for key in keys {
            self.invalidate_document(permit, &key).await?;
        }
        let mut state = self.state.lock().await;
        state
            .document_failures
            .retain(|key, _| key.session_id != session_id);
        state
            .quarantined
            .retain(|key, _| key.session_id != session_id);
        state
            .quarantined_pages
            .retain(|key, _| key.session_id != session_id);
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), BrokerError> {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            loop {
                let notified = self.shutdown_notify.notified();
                if self.shutdown_complete.load(Ordering::Acquire) {
                    return self.shutdown_error.lock().clone().map_or(Ok(()), Err);
                }
                notified.await;
            }
        }
        self.begin_shutdown();

        let mut outcome = self
            .admission
            .wait_until_idle(ADMISSION_DRAIN_TIMEOUT)
            .await;
        let state_lock = tokio::time::timeout(CHILD_REAP_TIMEOUT, self.state.lock()).await;
        match state_lock {
            Ok(mut state) => {
                if outcome.is_ok() {
                    outcome = self.drain_pending_reservation_cleanup(&mut state).await;
                } else {
                    self.pending_reservation_cleanup.lock().clear();
                }
                if state
                    .worker
                    .as_ref()
                    .is_some_and(|worker| worker.in_flight.is_some())
                {
                    let cleanup = self
                        .terminate_worker(&mut state, TerminationCause::Abandoned)
                        .await;
                    if outcome.is_ok() {
                        outcome = cleanup;
                    }
                }
                if state.worker.is_some() {
                    let graceful = self
                        .exchange_raw(
                            &mut state,
                            WorkerCommand::Shutdown,
                            CommandTimeout::Shutdown,
                            None,
                        )
                        .await;
                    let cleanup = if matches!(graceful, Ok(WorkerResult::ShutdownComplete)) {
                        self.reap_exited_worker(&mut state).await
                    } else {
                        self.terminate_worker(&mut state, TerminationCause::Shutdown)
                            .await
                    };
                    if outcome.is_ok() {
                        outcome = cleanup;
                    }
                }
                state.documents.clear();
                state.document_failures.clear();
                state.quarantined.clear();
                state.quarantined_pages.clear();
            }
            Err(_) => {
                if outcome.is_ok() {
                    outcome = Err(BrokerError::new(
                        "PDFIUM_WORKER_CLEANUP_FAILED",
                        "PDF worker state could not be acquired during shutdown",
                    ));
                }
            }
        }
        *self.shutdown_error.lock() = outcome.clone().err();
        self.shutdown_complete.store(true, Ordering::Release);
        self.shutdown_notify.notify_waiters();
        outcome
    }

    pub async fn worker_epoch(&self) -> Option<u64> {
        self.state
            .lock()
            .await
            .worker
            .as_ref()
            .map(|worker| worker.epoch)
    }

    #[doc(hidden)]
    pub async fn restart_attempts_for_test(&self) -> usize {
        self.state.lock().await.restart_events.len()
    }

    #[doc(hidden)]
    pub async fn kill_process_for_test(&self) -> Result<(), BrokerError> {
        let mut state = self.state.lock().await;
        let worker = state
            .worker
            .as_mut()
            .ok_or_else(|| BrokerError::unavailable("PDF worker is not running"))?;
        worker.child.start_kill().map_err(|error| {
            BrokerError::new(
                "PDFIUM_WORKER_CRASHED",
                format!("PDF worker could not be terminated for the test: {error}"),
            )
        })
    }

    #[doc(hidden)]
    pub async fn last_worker_stderr_for_test(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().await.last_worker_stderr).into_owned()
    }

    #[doc(hidden)]
    pub fn state_busy_for_test(&self) -> bool {
        self.state.try_lock().is_err()
    }

    fn ensure_accepting(&self) -> Result<(), BrokerError> {
        if self.shutting_down.load(Ordering::Acquire) || self.shutdown_cancellation.is_cancelled() {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_UNAVAILABLE",
                "PDF worker is shutting down",
            ));
        }
        Ok(())
    }

    async fn ensure_spawned(&self, state: &mut BrokerState) -> Result<(), BrokerError> {
        if state.worker.is_some() {
            return Ok(());
        }
        let spawn_started = Instant::now();
        let spawn_reason = match state.last_termination {
            None => "initial",
            Some(cause) => cause.label(),
        };
        let restart_attempt_recorded = if state.restart_required {
            let attempt = reserve_restart_attempt(state)?;
            state.restart_required = false;
            tracing::debug!(
                restart_attempt = attempt,
                restart_budget_remaining = MAX_RESTARTS_PER_WINDOW.saturating_sub(attempt),
                "starting PDF worker replacement after worker-health failure"
            );
            true
        } else {
            false
        };
        let launcher = self.launcher.clone()?;

        let mut command = Command::new(&launcher.executable);
        command
            .args(&launcher.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Pass the pre-resolved absolute runtime path to the worker for main/worker parity.
        // This avoids CWD/PATH divergence and ensures both processes load the exact same DLL.
        if let Some(resolved_path) = &launcher.resolved_runtime_path {
            command.env("PDFIUM_RESOLVED_PATH", resolved_path);
        } else if let Some(path) = launcher
            .pdfium_path
            .or_else(|| std::env::var_os("PDFIUM_PATH"))
        {
            // Fallback for tests or legacy: pass PDFIUM_PATH if resolved path not available
            command.env("PDFIUM_PATH", path);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                record_start_failure(state, restart_attempt_recorded);
                return Err(BrokerError::new(
                    "PDFIUM_WORKER_SPAWN_FAILED",
                    format!("PDF worker could not be started: {error}"),
                ));
            }
        };
        let (stdin, stdout, stderr) =
            match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
                _ => {
                    record_start_failure(state, restart_attempt_recorded);
                    return Err(BrokerError::new(
                        "PDFIUM_WORKER_SPAWN_FAILED",
                        "PDF worker stdio pipes were not created",
                    ));
                }
            };
        let stderr_buffer = Arc::new(SyncMutex::new(Vec::new()));
        let drain_buffer = Arc::clone(&stderr_buffer);
        let stderr_drain = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut buffer = [0u8; 8192];
            loop {
                match stderr.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let mut captured = drain_buffer.lock();
                        let remaining = (64 * 1024usize).saturating_sub(captured.len());
                        captured.extend_from_slice(&buffer[..read.min(remaining)]);
                    }
                }
            }
        });
        let epoch = state.next_worker_epoch;
        state.next_worker_epoch = state.next_worker_epoch.checked_add(1).ok_or_else(|| {
            BrokerError::new(
                "PDFIUM_WORKER_INTERNAL",
                "PDF worker epoch counter exhausted",
            )
        })?;
        state.worker = Some(RunningWorker {
            epoch,
            child,
            stdin: Some(stdin),
            stdout,
            stderr_drain: Some(stderr_drain),
            stderr_buffer,
            in_flight: None,
            memory_limit_bytes: launcher.memory_limit_bytes,
        });

        let hello = self
            .exchange_raw(state, WorkerCommand::Hello, CommandTimeout::Hello, None)
            .await;
        let result = match hello {
            Ok(result) => result,
            Err(error) => {
                record_start_failure(state, restart_attempt_recorded);
                let _ = self
                    .terminate_worker(state, TerminationCause::HealthFailure)
                    .await;
                return Err(error);
            }
        };
        let WorkerResult::Hello {
            capabilities,
            ready,
            ..
        } = result
        else {
            let error = self
                .protocol_violation(state, None, "Worker handshake response was invalid")
                .await;
            return Err(error);
        };
        if !ready {
            record_start_failure(state, restart_attempt_recorded);
            let _ = self
                .terminate_worker(state, TerminationCause::HealthFailure)
                .await;
            return Err(BrokerError::unavailable(
                "PDFium runtime is unavailable in the worker process",
            ));
        }
        let required = required_capabilities();
        if required
            .iter()
            .any(|capability| !capabilities.contains(capability))
        {
            let error = self
                .protocol_violation(state, None, "Worker capability set is incomplete")
                .await;
            return Err(error);
        }
        tracing::debug!(
            target: WORKER_METRICS_TARGET,
            event = "worker_spawn",
            worker_epoch = state.worker.as_ref().map(|worker| worker.epoch),
            reason = spawn_reason,
            spawn_to_hello_ms = spawn_started.elapsed().as_secs_f64() * 1000.0,
        );
        Ok(())
    }

    async fn exchange(
        &self,
        state: &mut BrokerState,
        command: WorkerCommand,
        timeout: CommandTimeout,
        cancellation: Option<&CancellationToken>,
        key: Option<&WorkerDocumentKey>,
        page: Option<u32>,
    ) -> Result<WorkerResult, BrokerError> {
        let started = Instant::now();
        let result = self
            .exchange_raw(state, command, timeout, cancellation)
            .await;
        let outcome = match &result {
            Ok(_) => "ok",
            Err(error) if error.code == "PDFIUM_WORKER_CANCELLED" => {
                if worker_stream_is_idle(state) {
                    "cancelled_drained"
                } else {
                    "cancelled_killed"
                }
            }
            Err(error) => error.code,
        };
        tracing::debug!(
            target: WORKER_METRICS_TARGET,
            event = "exchange",
            command_class = timeout.label(),
            outcome,
            duration_ms = started.elapsed().as_secs_f64() * 1000.0,
            worker_epoch = state.worker.as_ref().map(|worker| worker.epoch),
        );
        match result {
            Ok(result) => Ok(result),
            Err(error) => {
                if error.code == "PDFIUM_WORKER_CANCELLED" {
                    if worker_stream_is_idle(state) {
                        // The command was never sent or its response was
                        // drained within the grace period: the worker and its
                        // open documents remain valid.
                        return Err(error);
                    }
                    tracing::debug!(
                        worker_epoch = state.worker.as_ref().map(|worker| worker.epoch),
                        session_id = key.map(|key| key.session_id.as_str()),
                        content_generation = key.map(|key| key.content_generation),
                        "cancelled PDF worker command is being reaped without consuming restart budget"
                    );
                    self.terminate_worker(state, TerminationCause::Cancelled)
                        .await?;
                    return Err(error);
                }
                let reason = match error.code {
                    "PDFIUM_WORKER_TIMEOUT" => Some(WorkerFailureReason::Timeout),
                    "PDFIUM_WORKER_MEMORY_LIMIT" => Some(WorkerFailureReason::MemoryLimit),
                    "PDFIUM_WORKER_CRASHED" => Some(WorkerFailureReason::Crash),
                    "PDFIUM_WORKER_PROTOCOL_ERROR"
                    | "PDFIUM_WORKER_PROTOCOL_MISMATCH"
                    | "PDFIUM_WORKER_MESSAGE_TOO_LARGE" => {
                        Some(WorkerFailureReason::ProtocolViolation)
                    }
                    _ => None,
                };
                if let (Some(key), Some(reason)) = (key, reason) {
                    record_failure_at(state, key, reason, page, Instant::now());
                }
                if reason.is_some() {
                    state.restart_required = true;
                    tracing::warn!(
                        worker_epoch = state.worker.as_ref().map(|worker| worker.epoch),
                        session_id = key.map(|key| key.session_id.as_str()),
                        content_generation = key.map(|key| key.content_generation),
                        failure_category = ?reason,
                        page_index = page,
                        restart_budget_remaining = MAX_RESTARTS_PER_WINDOW.saturating_sub(state.restart_events.len()),
                        "PDF worker health failure requires a replacement"
                    );
                    self.terminate_worker(state, TerminationCause::HealthFailure)
                        .await?;
                }
                Err(error)
            }
        }
    }

    /// Sends one request and reads its response.
    ///
    /// Cancellation never relies on the worker reading a cancel command: PDFium
    /// may not return to the protocol loop until the native call finishes. A
    /// command whose class has a cancellation grace period keeps waiting for
    /// at most that long (bounded by the command timeout); a response that
    /// arrives in time is validated and discarded, leaving the stream at a
    /// frame boundary. Otherwise `in_flight` stays set and the caller must
    /// kill and reap the worker.
    async fn exchange_raw(
        &self,
        state: &mut BrokerState,
        command: WorkerCommand,
        timeout: CommandTimeout,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerResult, BrokerError> {
        let honor_shutdown_cancellation = !matches!(command, WorkerCommand::Shutdown);
        if honor_shutdown_cancellation && cancellation.is_some_and(CancellationToken::is_cancelled)
        {
            return Err(BrokerError::cancelled());
        }
        let request_id = state.next_request_id;
        state.next_request_id = state.next_request_id.checked_add(1).ok_or_else(|| {
            BrokerError::new(
                "PDFIUM_WORKER_INTERNAL",
                "PDF worker request identifier exhausted",
            )
        })?;
        let request = WorkerRequest {
            protocol_version: WORKER_PROTOCOL_VERSION,
            request_id,
            command,
        };
        let frame = encode_request(&request).map_err(map_frame_error)?;
        let worker = state
            .worker
            .as_mut()
            .ok_or_else(|| BrokerError::unavailable("PDF worker is not running"))?;
        if worker.in_flight.is_some() {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_PROTOCOL_ERROR",
                "PDF worker still owes a response to an abandoned request",
            ));
        }
        worker.in_flight = Some(request_id);
        let stdin = worker.stdin.as_mut().ok_or_else(|| {
            BrokerError::new("PDFIUM_WORKER_CRASHED", "PDF worker input pipe is closed")
        })?;
        stdin.write_all(&frame).await.map_err(|_| {
            BrokerError::new(
                "PDFIUM_WORKER_CRASHED",
                "PDF worker input pipe closed unexpectedly",
            )
        })?;
        stdin.flush().await.map_err(|_| {
            BrokerError::new(
                "PDFIUM_WORKER_CRASHED",
                "PDF worker input pipe could not be flushed",
            )
        })?;

        let timeout_error = || {
            BrokerError::new(
                "PDFIUM_WORKER_TIMEOUT",
                format!("PDF worker {} command timed out", timeout.label()),
            )
        };
        let shutdown_error =
            || BrokerError::unavailable("PDF worker command was stopped for shutdown");
        let worker_pid = worker.child.id();
        let memory_limit = worker.memory_limit_bytes;
        // The response future borrows the worker's stdout; keep it inside this
        // block so `in_flight` can be cleared once a whole frame was read.
        let (response, cancelled_and_drained) = {
            // macOS has no address-space limit for the worker, so its footprint
            // is watched while the command runs. Crossing the budget ends the
            // command like a worker failure; `in_flight` stays set and the
            // caller kills and replaces the worker.
            let response_future = async {
                tokio::select! {
                    biased;
                    result = tokio::time::timeout(
                        timeout.duration(),
                        read_response(&mut worker.stdout),
                    ) => result,
                    footprint = memory_watchdog::wait_until_over_limit(worker_pid, memory_limit) => {
                        Ok(Err(memory_limit_error(footprint, memory_limit)))
                    }
                }
            };
            tokio::pin!(response_future);
            if !honor_shutdown_cancellation {
                (response_future.await.map_err(|_| timeout_error())??, false)
            } else if let Some(cancellation) = cancellation {
                let completed = tokio::select! {
                    biased;
                    _ = self.shutdown_cancellation.cancelled() => return Err(shutdown_error()),
                    _ = cancellation.cancelled() => None,
                    result = &mut response_future => Some(result),
                };
                match completed {
                    Some(result) => (result.map_err(|_| timeout_error())??, false),
                    None => {
                        let grace = timeout.cancel_grace();
                        if grace.is_zero() {
                            return Err(BrokerError::cancelled());
                        }
                        let drained = tokio::select! {
                            biased;
                            _ = self.shutdown_cancellation.cancelled() => return Err(shutdown_error()),
                            drained = tokio::time::timeout(grace, &mut response_future) => drained,
                        };
                        match drained {
                            Ok(Ok(Ok(response))) => (response, true),
                            // A read failure is a real worker failure, not a cancellation.
                            Ok(Ok(Err(error))) => return Err(error),
                            // Grace or command deadline elapsed: the caller kills the worker.
                            Ok(Err(_)) | Err(_) => return Err(BrokerError::cancelled()),
                        }
                    }
                }
            } else {
                let response = tokio::select! {
                    biased;
                    _ = self.shutdown_cancellation.cancelled() => return Err(shutdown_error()),
                    result = &mut response_future => result.map_err(|_| timeout_error())??,
                };
                (response, false)
            }
        };
        worker.in_flight = None;
        if response.protocol_version != WORKER_PROTOCOL_VERSION {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_PROTOCOL_MISMATCH",
                "PDF worker response protocol version does not match",
            ));
        }
        if response.request_id != request_id {
            return Err(BrokerError::new(
                "PDFIUM_WORKER_PROTOCOL_ERROR",
                "PDF worker response request identifier does not match",
            ));
        }
        if cancelled_and_drained {
            // Validated but stale: the caller already gave up on this result.
            return Err(BrokerError::cancelled());
        }
        response.result.map_err(map_worker_failure)
    }

    async fn protocol_violation(
        &self,
        state: &mut BrokerState,
        key: Option<&WorkerDocumentKey>,
        message: &str,
    ) -> BrokerError {
        if let Some(key) = key {
            record_document_failure(state, key, WorkerFailureReason::ProtocolViolation);
        }
        state.restart_required = true;
        let _ = self
            .terminate_worker(state, TerminationCause::HealthFailure)
            .await;
        BrokerError::new("PDFIUM_WORKER_PROTOCOL_ERROR", message)
    }

    async fn terminate_worker(
        &self,
        state: &mut BrokerState,
        cause: TerminationCause,
    ) -> Result<(), BrokerError> {
        state.documents.clear();
        let Some(mut worker) = state.worker.take() else {
            return Ok(());
        };
        state.last_termination = Some(cause);
        tracing::debug!(
            target: WORKER_METRICS_TARGET,
            event = "worker_terminate",
            worker_epoch = worker.epoch,
            cause = cause.label(),
        );
        worker.stdin.take();
        let outcome = match worker.child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) | Err(_) => kill_and_reap(&mut worker.child, "PDF worker cleanup").await,
        };
        finish_worker_drain(state, &mut worker).await;
        outcome
    }

    async fn reap_exited_worker(&self, state: &mut BrokerState) -> Result<(), BrokerError> {
        state.documents.clear();
        let Some(mut worker) = state.worker.take() else {
            return Ok(());
        };
        worker.stdin.take();
        let outcome = match tokio::time::timeout(CHILD_REAP_TIMEOUT, worker.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) | Err(_) => {
                kill_and_reap(&mut worker.child, "PDF worker graceful-timeout cleanup").await
            }
        };
        finish_worker_drain(state, &mut worker).await;
        outcome
    }
}

async fn finish_worker_drain(state: &mut BrokerState, worker: &mut RunningWorker) {
    if let Some(task) = worker.stderr_drain.take() {
        let mut task = task;
        if tokio::time::timeout(STDERR_JOIN_TIMEOUT, &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
    state.last_worker_stderr = worker.stderr_buffer.lock().clone();
}

async fn kill_and_reap(child: &mut Child, context: &str) -> Result<(), BrokerError> {
    child.start_kill().map_err(|error| {
        BrokerError::new(
            "PDFIUM_WORKER_CLEANUP_FAILED",
            format!("{context} kill failed: {error}"),
        )
    })?;
    tokio::time::timeout(CHILD_REAP_TIMEOUT, child.wait())
        .await
        .map_err(|_| {
            BrokerError::new(
                "PDFIUM_WORKER_CLEANUP_FAILED",
                format!("{context} did not reap after kill"),
            )
        })?
        .map_err(|error| {
            BrokerError::new(
                "PDFIUM_WORKER_CLEANUP_FAILED",
                format!("{context} wait failed after kill: {error}"),
            )
        })?;
    Ok(())
}

/// A held worker read: broker admission plus whatever session lease the
/// caller needs, for one document key. Dropping it releases the admission.
pub trait WorkerReadLease {
    fn admission(&self) -> &WorkerAdmissionPermit;
    fn document_key(&self) -> &WorkerDocumentKey;
    fn page_count(&self) -> u32;
}

#[derive(Clone, Copy, Debug)]
pub struct SearchChunkPolicy {
    pub max_pages: u32,
    pub time_budget_ms: u32,
    pub max_results: usize,
}

/// Production chunking for viewer search: a chunk normally ends after
/// ~150 ms, well inside the search-chunk cancellation grace period.
pub const VIEWER_SEARCH_CHUNK_POLICY: SearchChunkPolicy = SearchChunkPolicy {
    max_pages: 64,
    time_budget_ms: 150,
    max_results: crate::worker::ipc::MAX_WORKER_SEARCH_RESULTS,
};

struct SearchAccumulator {
    matches: Vec<crate::SearchMatch>,
    total_matches: usize,
    max_results: usize,
}

impl SearchAccumulator {
    fn new(max_results: usize) -> Self {
        Self {
            matches: Vec::new(),
            total_matches: 0,
            max_results,
        }
    }

    fn remaining(&self) -> usize {
        self.max_results.saturating_sub(self.matches.len())
    }

    /// Validates a chunk against the request that produced it and appends it.
    /// Returns the next page to scan, if any.
    fn absorb(
        &mut self,
        start_page: u32,
        page_count: u32,
        chunk: WorkerSearchChunk,
    ) -> Result<Option<u32>, BrokerError> {
        let invalid = || {
            BrokerError::new(
                "PDFIUM_WORKER_PROTOCOL_ERROR",
                "Search chunk response is inconsistent with its request",
            )
        };
        let end_page = match chunk.next_page {
            Some(next) if next <= start_page || next >= page_count => return Err(invalid()),
            Some(next) => next,
            None => page_count,
        };
        if chunk.matches.len() > self.remaining() || chunk.matches.len() > chunk.total_matches {
            return Err(invalid());
        }
        for (offset, found) in chunk.matches.iter().enumerate() {
            if found.page_index < start_page
                || found.page_index >= end_page
                || found.global_match_index != self.total_matches + offset
            {
                return Err(invalid());
            }
        }
        self.total_matches = self
            .total_matches
            .checked_add(chunk.total_matches)
            .ok_or_else(invalid)?;
        self.matches.extend(chunk.matches);
        Ok(chunk.next_page)
    }

    fn finish(self) -> crate::SearchResult {
        crate::SearchResult {
            matches: self.matches,
            is_cancelled: false,
            total_matches: self.total_matches,
        }
    }
}

impl PdfiumWorkerBroker {
    /// Searches a document as a sequence of bounded page-range commands.
    ///
    /// Each chunk acquires its own read admission, so reads queued behind the
    /// search (renders, thumbnails, text layout) are served between chunks.
    /// Cancellation between chunks never touches the worker; cancellation
    /// during a chunk uses the search-chunk grace period. The aggregated
    /// result is identical to a single whole-document `SearchText`.
    pub async fn search_text_in_chunks<Lease, Error, Acquire, AcquireFuture>(
        &self,
        mut acquire: Acquire,
        query: &str,
        policy: SearchChunkPolicy,
        cancellation: &CancellationToken,
    ) -> Result<crate::SearchResult, Error>
    where
        Lease: WorkerReadLease,
        Error: From<BrokerError>,
        Acquire: FnMut() -> AcquireFuture,
        AcquireFuture: Future<Output = Result<Lease, Error>>,
    {
        let mut accumulator = SearchAccumulator::new(policy.max_results);
        let mut start_page = 0u32;
        let mut chunks = 0usize;
        let started = Instant::now();
        loop {
            if cancellation.is_cancelled() {
                return Err(BrokerError::cancelled().into());
            }
            let lease = acquire().await?;
            let page_count = lease.page_count();
            if start_page >= page_count {
                break;
            }
            let max_results = u32::try_from(accumulator.remaining()).unwrap_or(u32::MAX);
            let first_match_index = u64::try_from(accumulator.total_matches).unwrap_or(u64::MAX);
            let result = self
                .command_for_document(
                    lease.admission(),
                    lease.document_key(),
                    |document_id| WorkerCommand::SearchTextPages {
                        document_id,
                        query: query.to_string(),
                        start_page,
                        max_pages: policy.max_pages,
                        time_budget_ms: policy.time_budget_ms,
                        max_results,
                        first_match_index,
                    },
                    CommandTimeout::SearchChunk,
                    Some(cancellation),
                )
                .await;
            drop(lease);
            let WorkerResult::SearchChunk(chunk) = result? else {
                return Err(BrokerError::new(
                    "PDFIUM_WORKER_PROTOCOL_ERROR",
                    "Search chunk returned an unexpected response",
                )
                .into());
            };
            chunks += 1;
            match accumulator.absorb(start_page, page_count, chunk)? {
                Some(next_page) => start_page = next_page,
                None => break,
            }
        }
        tracing::debug!(
            target: WORKER_METRICS_TARGET,
            event = "search_completed",
            chunks,
            total_matches = accumulator.total_matches,
            duration_ms = started.elapsed().as_secs_f64() * 1000.0,
        );
        Ok(accumulator.finish())
    }
}

impl Default for PdfiumWorkerBroker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CommandTimeout {
    Hello,
    Ping,
    OpenDocument,
    CloseDocument,
    PageCount,
    RenderPage,
    RenderThumbnail,
    TextLayout,
    Search,
    SearchChunk,
    DocumentRead,
    Shutdown,
}

impl CommandTimeout {
    fn duration(self) -> Duration {
        match self {
            Self::Ping | Self::CloseDocument | Self::Shutdown => Duration::from_secs(2),
            Self::Hello | Self::PageCount => Duration::from_secs(5),
            Self::RenderThumbnail => Duration::from_secs(10),
            Self::OpenDocument | Self::TextLayout | Self::DocumentRead | Self::SearchChunk => {
                Duration::from_secs(15)
            }
            Self::RenderPage => Duration::from_secs(20),
            Self::Search => Duration::from_secs(30),
        }
    }

    /// How long a cancelled command may keep running so its response can be
    /// drained instead of killing the worker. Zero means kill immediately.
    /// Only commands that normally finish quickly get a grace period; the
    /// values are initial bounds pending the worker-metrics baseline.
    fn cancel_grace(self) -> Duration {
        match self {
            Self::RenderPage | Self::SearchChunk => Duration::from_millis(400),
            Self::RenderThumbnail | Self::TextLayout | Self::PageCount => {
                Duration::from_millis(250)
            }
            Self::Hello
            | Self::Ping
            | Self::OpenDocument
            | Self::CloseDocument
            | Self::Search
            | Self::DocumentRead
            | Self::Shutdown => Duration::ZERO,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::Ping => "ping",
            Self::OpenDocument => "open-document",
            Self::CloseDocument => "close-document",
            Self::PageCount => "page-count",
            Self::RenderPage => "render-page",
            Self::RenderThumbnail => "render-thumbnail",
            Self::TextLayout => "text-layout",
            Self::Search => "search",
            Self::SearchChunk => "search-chunk",
            Self::DocumentRead => "document-read",
            Self::Shutdown => "shutdown",
        }
    }
}

async fn read_response(stdout: &mut ChildStdout) -> Result<WorkerResponse, BrokerError> {
    let mut header = [0u8; 4];
    stdout.read_exact(&mut header).await.map_err(|_| {
        BrokerError::new(
            "PDFIUM_WORKER_CRASHED",
            "PDF worker response header was truncated",
        )
    })?;
    let announced = usize::try_from(u32::from_le_bytes(header)).map_err(|_| {
        BrokerError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "PDF worker response length overflowed",
        )
    })?;
    if announced > MAX_WORKER_RESPONSE_BYTES {
        return Err(BrokerError::new(
            "PDFIUM_WORKER_MESSAGE_TOO_LARGE",
            "PDF worker response exceeded the configured message limit",
        ));
    }
    let mut payload = vec![0u8; announced];
    stdout.read_exact(&mut payload).await.map_err(|_| {
        BrokerError::new(
            "PDFIUM_WORKER_CRASHED",
            "PDF worker response payload was truncated",
        )
    })?;
    decode_response(&payload).map_err(map_frame_error)
}

fn map_frame_error(error: crate::worker::ipc::FrameError) -> BrokerError {
    match error {
        crate::worker::ipc::FrameError::MessageTooLarge { .. } => BrokerError::new(
            "PDFIUM_WORKER_MESSAGE_TOO_LARGE",
            "PDF worker IPC message exceeded the configured limit",
        ),
        _ => BrokerError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            format!("PDF worker IPC framing failed: {error}"),
        ),
    }
}

fn map_worker_failure(failure: WorkerFailure) -> BrokerError {
    let code = match failure.kind {
        WorkerFailureKind::PasswordRequired => "DOCUMENT_SESSION_PASSWORD_REQUIRED",
        WorkerFailureKind::WrongPassword => "DOCUMENT_SESSION_INVALID_PASSWORD",
        WorkerFailureKind::UnsupportedSecurity => "DOCUMENT_SESSION_SECURITY_ERROR",
        WorkerFailureKind::FileNotFound => "DOCUMENT_SESSION_SOURCE_NOT_FOUND",
        WorkerFailureKind::InvalidPageIndex => "PDFIUM_WORKER_INVALID_PAGE_INDEX",
        WorkerFailureKind::InvalidRenderParameters => "PDFIUM_WORKER_INVALID_RENDER",
        WorkerFailureKind::InvalidInput => "PDFIUM_WORKER_INVALID_INPUT",
        WorkerFailureKind::MalformedPdf => "DOCUMENT_SESSION_INVALID_PDF",
        WorkerFailureKind::PdfiumUnavailable => "PDFIUM_WORKER_UNAVAILABLE",
        WorkerFailureKind::PdfiumLoadError => "PDFIUM_WORKER_LOAD_FAILED",
        WorkerFailureKind::PdfiumRenderError => "PDFIUM_WORKER_RENDER_FAILED",
        WorkerFailureKind::StructureTreeUnavailable => "PDFIUM_WORKER_STRUCTURE_TREE_UNAVAILABLE",
        WorkerFailureKind::MessageTooLarge => "PDFIUM_WORKER_MESSAGE_TOO_LARGE",
        WorkerFailureKind::ProtocolError => "PDFIUM_WORKER_PROTOCOL_ERROR",
        WorkerFailureKind::Internal => "PDFIUM_WORKER_INTERNAL",
    };
    BrokerError::new(code, failure.message)
}

/// True when the worker has no unread response, so its stdout is at a frame
/// boundary and it can safely receive the next command.
fn worker_stream_is_idle(state: &BrokerState) -> bool {
    state
        .worker
        .as_ref()
        .is_some_and(|worker| worker.in_flight.is_none())
}

fn memory_limit_error(footprint: u64, limit: u64) -> BrokerError {
    BrokerError::new(
        "PDFIUM_WORKER_MEMORY_LIMIT",
        format!(
            "PDF worker exceeded its memory budget ({} MiB > {} MiB) and was stopped",
            memory_watchdog::mib(footprint),
            memory_watchdog::mib(limit)
        ),
    )
}

fn document_quarantined_error() -> BrokerError {
    BrokerError::new(
        "PDFIUM_DOCUMENT_QUARANTINED",
        "This document generation repeatedly destabilized the PDF worker",
    )
}

/// Commands whose cost is bounded by a single page. Only their timeouts are
/// attributed to that page; every other command counts against the document.
/// Exhaustive on purpose: a new command must choose its scope explicitly.
fn page_scope(command: &WorkerCommand) -> Option<u32> {
    match command {
        WorkerCommand::RenderPage { page_index, .. }
        | WorkerCommand::RenderThumbnail { page_index, .. }
        | WorkerCommand::GetPageTextLayout { page_index, .. } => Some(*page_index),
        WorkerCommand::Hello
        | WorkerCommand::OpenDocument { .. }
        | WorkerCommand::CloseDocument { .. }
        | WorkerCommand::CloseAllDocuments
        | WorkerCommand::GetPageCount { .. }
        | WorkerCommand::SearchText { .. }
        | WorkerCommand::GetOutline { .. }
        | WorkerCommand::GetMetadata { .. }
        | WorkerCommand::GetPageAnnotations { .. }
        | WorkerCommand::GetStructureTree { .. }
        | WorkerCommand::AuditAccessibility { .. }
        | WorkerCommand::Ping
        | WorkerCommand::Shutdown
        | WorkerCommand::SearchTextPages { .. }
        | WorkerCommand::GetPageSizes { .. }
        | WorkerCommand::GetFormInfo { .. }
        | WorkerCommand::GetPageFormFields { .. } => None,
    }
}

fn record_document_failure(
    state: &mut BrokerState,
    key: &WorkerDocumentKey,
    reason: WorkerFailureReason,
) {
    record_failure_at(state, key, reason, None, Instant::now());
}

fn failure_window(event: &FailureEvent) -> Duration {
    if event.page.is_some() {
        PAGE_TIMEOUT_WINDOW
    } else {
        DOCUMENT_FAILURE_WINDOW
    }
}

fn record_failure_at(
    state: &mut BrokerState,
    key: &WorkerDocumentKey,
    reason: WorkerFailureReason,
    page: Option<u32>,
    now: Instant,
) {
    // Crash and protocol failures may mean a compromised or corrupted worker,
    // so they always count against the document, whichever page was involved.
    // A timeout or a memory-limit kill means the page is too heavy for PDFium;
    // both count toward the same page and document thresholds.
    let page = page.filter(|_| {
        matches!(
            reason,
            WorkerFailureReason::Timeout | WorkerFailureReason::MemoryLimit
        )
    });
    let failures = state.document_failures.entry(key.clone()).or_default();
    failures.retain(|event| now.duration_since(event.at) <= failure_window(event));
    failures.push_back(FailureEvent {
        at: now,
        reason,
        page,
    });
    let document_scoped = failures
        .iter()
        .filter(|event| event.reason == reason && event.page.is_none())
        .count();
    if document_scoped >= DOCUMENT_QUARANTINE_THRESHOLD {
        state.quarantined.insert(key.clone(), reason);
        return;
    }
    let Some(page) = page else {
        return;
    };
    let page_timeouts = failures.iter().filter_map(|event| event.page);
    let total = page_timeouts.clone().count();
    let distinct = page_timeouts.clone().collect::<BTreeSet<_>>().len();
    if total >= DOCUMENT_TIMEOUT_TOTAL_THRESHOLD
        || distinct >= DOCUMENT_TIMEOUT_DISTINCT_PAGE_THRESHOLD
    {
        state.quarantined.insert(key.clone(), reason);
        return;
    }
    if page_timeouts.filter(|timed_out| *timed_out == page).count()
        >= PAGE_TIMEOUT_QUARANTINE_THRESHOLD
    {
        state
            .quarantined_pages
            .entry(key.clone())
            .or_default()
            .insert(page);
    }
}

fn reserve_restart_attempt(state: &mut BrokerState) -> Result<usize, BrokerError> {
    prune_instants(&mut state.restart_events, RESTART_WINDOW);
    if state.restart_events.len() >= MAX_RESTARTS_PER_WINDOW {
        return Err(BrokerError::new(
            "PDFIUM_WORKER_RESTART_LIMIT",
            "PDF worker restart budget was exhausted",
        ));
    }
    state.restart_events.push_back(Instant::now());
    Ok(state.restart_events.len())
}

fn record_start_failure(state: &mut BrokerState, restart_attempt_recorded: bool) {
    if !restart_attempt_recorded {
        // The initial spawn is not a restart, but a failed startup can still
        // crash-loop. Count that failure once; its eventual replacement is a
        // separate attempt.
        let _ = reserve_restart_attempt(state);
    }
    state.restart_required = true;
}

fn prune_instants(events: &mut VecDeque<Instant>, window: Duration) {
    let now = Instant::now();
    while events
        .front()
        .is_some_and(|event| now.duration_since(*event) > window)
    {
        events.pop_front();
    }
}

fn required_capabilities() -> [WorkerCapability; 19] {
    [
        WorkerCapability::OpenDocument,
        WorkerCapability::CloseDocument,
        WorkerCapability::CloseAllDocuments,
        WorkerCapability::GetPageCount,
        WorkerCapability::RenderPage,
        WorkerCapability::RenderThumbnail,
        WorkerCapability::GetPageTextLayout,
        WorkerCapability::SearchText,
        WorkerCapability::GetOutline,
        WorkerCapability::GetMetadata,
        WorkerCapability::GetPageAnnotations,
        WorkerCapability::GetStructureTree,
        WorkerCapability::AuditAccessibility,
        WorkerCapability::Ping,
        WorkerCapability::Shutdown,
        WorkerCapability::SearchTextPages,
        WorkerCapability::GetPageSizes,
        WorkerCapability::GetFormInfo,
        WorkerCapability::GetPageFormFields,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pre_cancelled_read_never_registers_or_owns_active_slot() {
        let controller = Arc::new(AdmissionController::default());
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = controller
            .acquire_read("session-a", Some(&cancellation))
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "PDFIUM_WORKER_CANCELLED");
        let state = controller.state.lock();
        assert!(state.active.is_none());
        assert!(state.waiters.is_empty());
    }

    #[test]
    fn registration_drop_before_and_after_dispatch_releases_exact_ownership() {
        let controller = Arc::new(AdmissionController::default());
        let (delivered, delivered_receiver) = controller
            .register("session-a", AdmissionMode::Read)
            .unwrap();
        assert!(controller.state.lock().active.is_some());
        drop(delivered_receiver);
        drop(delivered);
        assert!(controller.state.lock().active.is_none());

        let (active_registration, active_receiver) = controller
            .register("session-a", AdmissionMode::Read)
            .unwrap();
        let (queued_registration, queued_receiver) = controller
            .register("session-b", AdmissionMode::Read)
            .unwrap();
        assert_eq!(controller.state.lock().waiters.len(), 1);
        drop(queued_receiver);
        drop(queued_registration);
        assert!(controller.state.lock().waiters.is_empty());
        drop(active_receiver);
        drop(active_registration);
        assert!(controller.state.lock().active.is_none());
    }

    #[tokio::test]
    async fn aborted_handoff_dispatches_the_next_waiter_without_slot_leak() {
        let controller = Arc::new(AdmissionController::default());
        let first = controller.acquire_read("session-a", None).await.unwrap();
        let (aborted, aborted_receiver) = controller
            .register("session-b", AdmissionMode::Read)
            .unwrap();
        let (mut next, next_receiver) = controller
            .register("session-c", AdmissionMode::Read)
            .unwrap();
        drop(aborted_receiver);
        drop(aborted);
        drop(first);
        let grant = next_receiver.await.unwrap().unwrap();
        assert_eq!(
            controller
                .state
                .lock()
                .active
                .as_ref()
                .map(|item| item.ticket),
            Some(grant.ticket)
        );
        next.disarm();
        controller.release(grant.ticket);
        assert!(controller.state.lock().active.is_none());
    }

    #[test]
    fn exclusive_registration_abort_uses_counted_session_block_ownership() {
        let controller = Arc::new(AdmissionController::default());
        let (first, first_receiver) = controller
            .register("session-a", AdmissionMode::Exclusive)
            .unwrap();
        let (second, second_receiver) = controller
            .register("session-a", AdmissionMode::Exclusive)
            .unwrap();
        assert_eq!(
            controller
                .state
                .lock()
                .blocked_sessions
                .get("session-a")
                .copied(),
            Some(2)
        );
        drop(second_receiver);
        drop(second);
        assert_eq!(
            controller
                .state
                .lock()
                .blocked_sessions
                .get("session-a")
                .copied(),
            Some(1)
        );
        drop(first_receiver);
        drop(first);
        let state = controller.state.lock();
        assert!(!state.blocked_sessions.contains_key("session-a"));
        assert!(state.active.is_none());
    }

    #[tokio::test]
    async fn close_rejects_all_queued_and_future_admissions() {
        let controller = Arc::new(AdmissionController::default());
        let active = controller.acquire_read("active", None).await.unwrap();
        let (_read_registration, read_receiver) = controller
            .register("queued-read", AdmissionMode::Read)
            .unwrap();
        let (_exclusive_registration, exclusive_receiver) = controller
            .register("queued-exclusive", AdmissionMode::Exclusive)
            .unwrap();
        controller.close();
        assert_eq!(
            read_receiver.await.unwrap().unwrap_err().code,
            "PDFIUM_WORKER_UNAVAILABLE"
        );
        assert_eq!(
            exclusive_receiver.await.unwrap().unwrap_err().code,
            "PDFIUM_WORKER_UNAVAILABLE"
        );
        assert_eq!(
            controller
                .acquire_read("late", None)
                .await
                .err()
                .unwrap()
                .code,
            "PDFIUM_WORKER_UNAVAILABLE"
        );
        drop(active);
        controller
            .wait_until_idle(Duration::from_millis(50))
            .await
            .unwrap();
        let state = controller.state.lock();
        assert!(state.active.is_none());
        assert!(state.waiters.is_empty());
        assert!(state.blocked_sessions.is_empty());
    }

    #[tokio::test]
    async fn exclusive_admission_cancels_queued_same_session_reads() {
        let controller = Arc::new(AdmissionController::default());
        let active = controller.acquire_read("session-a", None).await.unwrap();
        let queued_controller = Arc::clone(&controller);
        let queued = tokio::spawn(async move {
            queued_controller
                .acquire_read("session-a", None)
                .await
                .map(|_| ())
        });
        tokio::task::yield_now().await;
        let exclusive_controller = Arc::clone(&controller);
        let exclusive =
            tokio::spawn(async move { exclusive_controller.acquire_exclusive("session-a").await });
        loop {
            if controller
                .state
                .lock()
                .blocked_sessions
                .contains_key("session-a")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(active);
        assert!(queued.await.unwrap().is_err());
        assert!(exclusive.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn wrong_session_exclusive_does_not_cancel_other_session_queue() {
        let controller = Arc::new(AdmissionController::default());
        let active = controller.acquire_read("session-a", None).await.unwrap();
        let queued_controller = Arc::clone(&controller);
        let queued =
            tokio::spawn(async move { queued_controller.acquire_read("session-b", None).await });
        tokio::task::yield_now().await;
        controller.cancel_session_reads("session-a");
        drop(active);
        assert!(queued.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn exclusive_admission_is_not_starved_by_another_sessions_read_backlog() {
        let controller = Arc::new(AdmissionController::default());
        let active = controller.acquire_read("session-a", None).await.unwrap();
        let mut queued_reads = Vec::new();
        for _ in 0..32 {
            let queued_controller = Arc::clone(&controller);
            queued_reads.push(tokio::spawn(async move {
                queued_controller.acquire_read("session-a", None).await
            }));
        }
        tokio::task::yield_now().await;
        let exclusive_controller = Arc::clone(&controller);
        let exclusive =
            tokio::spawn(async move { exclusive_controller.acquire_exclusive("session-b").await });
        loop {
            if controller
                .state
                .lock()
                .blocked_sessions
                .contains_key("session-b")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(active);
        let exclusive = tokio::time::timeout(Duration::from_millis(250), exclusive)
            .await
            .expect("exclusive admission was starved")
            .unwrap()
            .unwrap();
        drop(exclusive);
        for read in queued_reads {
            assert!(read.await.unwrap().is_ok());
        }
    }

    fn search_match(global_match_index: usize, page_index: u32) -> crate::SearchMatch {
        crate::SearchMatch {
            global_match_index,
            page_index,
            rects: Vec::new(),
            snippet: String::new(),
        }
    }

    #[test]
    fn search_accumulator_rejects_chunks_inconsistent_with_their_request() {
        let chunk = |matches, total_matches, next_page| WorkerSearchChunk {
            matches,
            total_matches,
            next_page,
        };
        let mut accumulator = SearchAccumulator::new(3);
        assert_eq!(
            accumulator
                .absorb(0, 10, chunk(vec![search_match(0, 1)], 2, Some(4)))
                .unwrap(),
            Some(4)
        );
        // Next page must advance and stay inside the document.
        assert!(accumulator
            .absorb(4, 10, chunk(Vec::new(), 0, Some(4)))
            .is_err());
        assert!(accumulator
            .absorb(4, 10, chunk(Vec::new(), 0, Some(10)))
            .is_err());
        // Matches must lie in the scanned range and continue the numbering.
        assert!(accumulator
            .absorb(4, 10, chunk(vec![search_match(2, 3)], 1, None))
            .is_err());
        assert!(accumulator
            .absorb(4, 10, chunk(vec![search_match(5, 4)], 1, None))
            .is_err());
        // More matches than the remaining capacity, or than counted, are invalid.
        assert!(accumulator
            .absorb(
                4,
                10,
                chunk(
                    vec![search_match(2, 4), search_match(3, 4), search_match(4, 5)],
                    3,
                    None
                )
            )
            .is_err());
        assert!(accumulator
            .absorb(4, 10, chunk(vec![search_match(2, 4)], 0, None))
            .is_err());
        assert_eq!(
            accumulator
                .absorb(4, 10, chunk(vec![search_match(2, 9)], 5, None))
                .unwrap(),
            None
        );
        let result = accumulator.finish();
        assert_eq!(result.total_matches, 7);
        assert_eq!(result.matches.len(), 2);
    }

    #[test]
    fn quarantine_requires_repeated_failures_in_the_same_category() {
        let mut state = BrokerState {
            worker: None,
            next_request_id: 1,
            next_worker_epoch: 1,
            documents: HashMap::new(),
            last_termination: None,
            restart_required: false,
            restart_events: VecDeque::new(),
            document_failures: HashMap::new(),
            quarantined: HashMap::new(),
            quarantined_pages: HashMap::new(),
            last_worker_stderr: Vec::new(),
        };
        let key = WorkerDocumentKey {
            session_id: "session".to_string(),
            content_generation: 7,
        };
        record_document_failure(&mut state, &key, WorkerFailureReason::Timeout);
        record_document_failure(&mut state, &key, WorkerFailureReason::Crash);
        record_document_failure(&mut state, &key, WorkerFailureReason::ProtocolViolation);
        assert!(!state.quarantined.contains_key(&key));
        record_document_failure(&mut state, &key, WorkerFailureReason::Timeout);
        record_document_failure(&mut state, &key, WorkerFailureReason::Timeout);
        assert_eq!(
            state.quarantined.get(&key),
            Some(&WorkerFailureReason::Timeout)
        );
    }

    fn empty_broker_state() -> BrokerState {
        BrokerState {
            worker: None,
            next_request_id: 1,
            next_worker_epoch: 1,
            documents: HashMap::new(),
            last_termination: None,
            restart_required: false,
            restart_events: VecDeque::new(),
            document_failures: HashMap::new(),
            quarantined: HashMap::new(),
            quarantined_pages: HashMap::new(),
            last_worker_stderr: Vec::new(),
        }
    }

    fn policy_key() -> WorkerDocumentKey {
        WorkerDocumentKey {
            session_id: "session".to_string(),
            content_generation: 3,
        }
    }

    fn page_is_quarantined(state: &BrokerState, key: &WorkerDocumentKey, page: u32) -> bool {
        state
            .quarantined_pages
            .get(key)
            .is_some_and(|pages| pages.contains(&page))
    }

    #[test]
    fn crash_and_protocol_failures_stay_document_scoped_across_pages() {
        for reason in [
            WorkerFailureReason::Crash,
            WorkerFailureReason::ProtocolViolation,
        ] {
            let mut state = empty_broker_state();
            let key = policy_key();
            let now = Instant::now();
            for page in 0..3 {
                record_failure_at(&mut state, &key, reason, Some(page), now);
            }
            assert_eq!(state.quarantined.get(&key), Some(&reason));
            assert!(state.quarantined_pages.is_empty());
        }
    }

    #[test]
    fn repeated_page_timeouts_quarantine_only_that_page() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let now = Instant::now();
        let timeout = WorkerFailureReason::Timeout;
        record_failure_at(&mut state, &key, timeout, Some(4), now);
        assert!(!page_is_quarantined(&state, &key, 4));
        record_failure_at(&mut state, &key, timeout, Some(4), now);
        assert!(page_is_quarantined(&state, &key, 4));
        assert!(!page_is_quarantined(&state, &key, 5));
        assert!(!state.quarantined.contains_key(&key));
        // A third page timeout (after a retry) must not reach the old
        // document-scoped threshold of three timeouts.
        record_failure_at(&mut state, &key, timeout, Some(4), now);
        assert!(!state.quarantined.contains_key(&key));
    }

    #[test]
    fn timeouts_on_distinct_pages_escalate_to_the_document() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let now = Instant::now();
        let timeout = WorkerFailureReason::Timeout;
        record_failure_at(&mut state, &key, timeout, Some(1), now);
        record_failure_at(&mut state, &key, timeout, Some(2), now);
        assert!(!state.quarantined.contains_key(&key));
        record_failure_at(&mut state, &key, timeout, Some(3), now);
        assert_eq!(state.quarantined.get(&key), Some(&timeout));
    }

    #[test]
    fn total_page_timeouts_escalate_to_the_document() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let now = Instant::now();
        let timeout = WorkerFailureReason::Timeout;
        for _ in 0..3 {
            record_failure_at(&mut state, &key, timeout, Some(0), now);
            record_failure_at(&mut state, &key, timeout, Some(1), now);
        }
        assert_eq!(state.quarantined.get(&key), Some(&timeout));
    }

    #[test]
    fn page_timeouts_are_counted_over_the_page_window() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let start = Instant::now();
        let timeout = WorkerFailureReason::Timeout;
        record_failure_at(&mut state, &key, timeout, Some(2), start);
        // Two serialized render timeouts are more than 20 s apart; a 60 s
        // window would have to contain both, the page window does.
        record_failure_at(
            &mut state,
            &key,
            timeout,
            Some(2),
            start + Duration::from_secs(90),
        );
        assert!(page_is_quarantined(&state, &key, 2));

        let mut state = empty_broker_state();
        record_failure_at(&mut state, &key, timeout, Some(2), start);
        record_failure_at(
            &mut state,
            &key,
            timeout,
            Some(2),
            start + PAGE_TIMEOUT_WINDOW + Duration::from_secs(1),
        );
        assert!(!page_is_quarantined(&state, &key, 2));
    }

    #[test]
    fn document_scoped_failures_expire_after_the_document_window() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let start = Instant::now();
        let timeout = WorkerFailureReason::Timeout;
        record_failure_at(&mut state, &key, timeout, None, start);
        record_failure_at(&mut state, &key, timeout, None, start);
        record_failure_at(
            &mut state,
            &key,
            timeout,
            None,
            start + DOCUMENT_FAILURE_WINDOW + Duration::from_secs(1),
        );
        assert!(!state.quarantined.contains_key(&key));
        record_failure_at(
            &mut state,
            &key,
            timeout,
            None,
            start + DOCUMENT_FAILURE_WINDOW + Duration::from_secs(2),
        );
        record_failure_at(
            &mut state,
            &key,
            timeout,
            None,
            start + DOCUMENT_FAILURE_WINDOW + Duration::from_secs(3),
        );
        assert_eq!(state.quarantined.get(&key), Some(&timeout));
    }

    #[test]
    fn document_scoped_timeouts_keep_the_three_strike_rule() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let now = Instant::now();
        record_failure_at(&mut state, &key, WorkerFailureReason::Timeout, Some(0), now);
        record_failure_at(&mut state, &key, WorkerFailureReason::Timeout, None, now);
        record_failure_at(&mut state, &key, WorkerFailureReason::Timeout, None, now);
        assert!(!state.quarantined.contains_key(&key));
        record_failure_at(&mut state, &key, WorkerFailureReason::Timeout, None, now);
        assert_eq!(
            state.quarantined.get(&key),
            Some(&WorkerFailureReason::Timeout)
        );
    }

    #[test]
    fn released_page_requarantines_on_the_next_timeout_in_the_window() {
        let mut state = empty_broker_state();
        let key = policy_key();
        let now = Instant::now();
        let timeout = WorkerFailureReason::Timeout;
        record_failure_at(&mut state, &key, timeout, Some(6), now);
        record_failure_at(&mut state, &key, timeout, Some(6), now);
        assert!(page_is_quarantined(&state, &key, 6));
        // What `release_page_quarantine` does to the state.
        state.quarantined_pages.remove(&key);
        record_failure_at(&mut state, &key, timeout, Some(6), now);
        assert!(page_is_quarantined(&state, &key, 6));
    }

    #[test]
    fn only_single_page_commands_are_page_scoped() {
        assert_eq!(
            page_scope(&WorkerCommand::RenderPage {
                document_id: 1,
                page_index: 4,
                target: crate::worker::ipc::WorkerRenderTarget::Scale(1.0),
                rotation: 0,
            }),
            Some(4)
        );
        assert_eq!(
            page_scope(&WorkerCommand::RenderThumbnail {
                document_id: 1,
                page_index: 5,
                target_width: 150,
            }),
            Some(5)
        );
        assert_eq!(
            page_scope(&WorkerCommand::GetPageTextLayout {
                document_id: 1,
                page_index: 6,
            }),
            Some(6)
        );
        assert_eq!(
            page_scope(&WorkerCommand::SearchTextPages {
                document_id: 1,
                query: "q".to_string(),
                start_page: 7,
                max_pages: 1,
                time_budget_ms: 1,
                max_results: 1,
                first_match_index: 0,
            }),
            None
        );
        assert_eq!(
            page_scope(&WorkerCommand::GetPageAnnotations {
                document_id: 1,
                page_index: 8,
            }),
            None
        );
    }
}
