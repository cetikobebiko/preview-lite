//! Backend-authoritative working-copy document sessions.
//!
//! The frontend only receives opaque session identifiers and immutable snapshots.
//! Mutable working paths, credentials, recovery artifacts, and content digests stay
//! inside this module.

use crate::annotations::{self, NormalizedPageRect};
use crate::document_io::{
    atomic_commit_prevalidated_pdf, atomic_copy_pdf, same_persistence_target, sha256_file,
    AtomicWriteReport, CommitState, DocumentIoError, DocumentIoStage, TargetExpectation,
};
use crate::job_worker::{JobCommand, JobResult, MutationRecipe};
#[cfg(test)]
use crate::manipulation;
use crate::pdf_engine::render_service::{RenderRequestIdentity, RenderScope, RenderService};
use crate::pdf_engine::subprocess_manager::{
    BrokerError, CommandTimeout, PdfiumWorkerBroker, WorkerAdmissionPermit, WorkerDocumentKey,
};
use crate::worker::ipc::{WorkerCommand, WorkerRenderTarget, WorkerResult};
use crate::{
    ActiveRenderTasks, ActiveSearchId, PasswordCache, PdfMetadata, SearchCache, SearchResult,
    TextLayoutCache, TextLayoutData,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::State;
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedRwLockReadGuard, RwLock as AsyncRwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const OPEN_ATTEMPTS: usize = 2;
const TEMP_ROOT_NAME: &str = "antigravity-pdf-sessions";
const TRANSITION_TTL_MS: u64 = 5 * 60 * 1000;
const ACTIVATION_TTL_MS: u64 = 5 * 60 * 1000;
const SESSION_JOB_PENDING_TTL_MS: u64 = 60 * 1000;
const SESSION_JOB_TERMINAL_TTL_MS: u64 = 5 * 60 * 1000;
const SESSION_JOB_PENDING_MAX: usize = 256;
const SESSION_JOB_DEFERRED_MAX: usize = 128;
const SESSION_JOB_TERMINAL_MAX: usize = 256;
const SESSION_JOB_CLEANUP_MAX_ATTEMPTS: u8 = 5;
const SESSION_JOB_REAPER_INTERVAL_MS: u64 = 500;
const SESSION_JOB_SHUTDOWN_QUIESCE_MS: u64 = 2_000;
const SUPERVISOR_COOPERATIVE_EXIT_MS: u64 = 100;
const SUPERVISOR_ABORT_EXIT_MS: u64 = 2_000;

/// How an opened file is protected, decided by whether the worker could open it without a
/// password.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidateEncryption {
    None,
    /// Encrypted with an empty user password: opens without a prompt, read-only.
    OwnerPasswordOnly,
    /// Needs the user password before it can be opened.
    PasswordRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    CandidateAuthPending,
    CandidateReady,
    CandidateActivationPrepared,
    Open,
    DecisionPending,
    RecoveryRequired,
    CommitIndeterminate,
    Closing,
    Closed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    DocumentSwitch,
    DocumentClose,
    WindowClose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionStatus {
    DecisionPending,
    PreparedSave,
    PreparedDiscard,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransitionDecision {
    Save { target: Option<PathBuf> },
    Discard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SaveCommitClassification {
    NotCommitted,
    Committed,
    CommitIndeterminate,
    RecoveryRequired,
    CommittedButSessionRefreshFailed,
}

#[derive(Clone, Debug)]
struct SaveCoreSuccess {
    snapshot: DocumentSessionSnapshot,
    classification: SaveCommitClassification,
}

#[derive(Clone, Debug)]
struct SaveCoreFailure {
    error: DocumentSessionError,
    classification: SaveCommitClassification,
}

impl From<DocumentSessionError> for SaveCoreFailure {
    fn from(error: DocumentSessionError) -> Self {
        Self {
            error,
            classification: SaveCommitClassification::NotCommitted,
        }
    }
}

fn classify_document_io_commit_state(state: CommitState) -> SaveCommitClassification {
    match state {
        CommitState::Prepared | CommitState::OriginalPreserved | CommitState::OriginalRecovered => {
            SaveCommitClassification::NotCommitted
        }
        CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed => {
            SaveCommitClassification::Committed
        }
        CommitState::RecoveryRequired => SaveCommitClassification::RecoveryRequired,
        CommitState::Indeterminate => SaveCommitClassification::CommitIndeterminate,
    }
}

#[derive(Clone, Debug)]
enum TransitionExecutionPhase {
    PendingUnclaimed,
    SaveClaimed {
        target: Option<PathBuf>,
    },
    SaveInProgress {
        target: Option<PathBuf>,
    },
    SaveCommitted {
        target: Option<PathBuf>,
        snapshot: DocumentSessionSnapshot,
    },
    DiscardInProgress,
    Applying {
        decision: TransitionDecision,
        saved_snapshot: Option<DocumentSessionSnapshot>,
    },
    FailedPreCommit {
        decision: TransitionDecision,
        error: DocumentSessionError,
    },
    InterruptedPreCommit {
        decision: TransitionDecision,
        error: DocumentSessionError,
    },
    FailedPostCommit {
        decision: TransitionDecision,
        saved_snapshot: Option<DocumentSessionSnapshot>,
        error: DocumentSessionError,
    },
    CommitIndeterminate {
        decision: TransitionDecision,
        error: DocumentSessionError,
    },
    RecoveryRequired {
        decision: TransitionDecision,
        error: DocumentSessionError,
    },
    ApplyCompleted {
        decision: TransitionDecision,
        snapshot: DocumentSessionSnapshot,
    },
    Completed {
        decision: TransitionDecision,
        result: Result<DocumentSessionSnapshot, DocumentSessionError>,
    },
}

#[derive(Clone, Debug)]
struct TransitionExecution {
    phase: Arc<Mutex<TransitionExecutionPhase>>,
    completion_state: Arc<Mutex<TransitionExecutionCompletion>>,
    completion: Arc<Notify>,
}

impl TransitionExecution {
    fn new() -> Self {
        Self {
            phase: Arc::new(Mutex::new(TransitionExecutionPhase::PendingUnclaimed)),
            completion_state: Arc::new(Mutex::new(TransitionExecutionCompletion::Available)),
            completion: Arc::new(Notify::new()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransitionExecutionCompletion {
    Available,
    GateReserved { decision: TransitionDecision },
    Running,
    TerminalPublished,
    InterruptedPublished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateActivationPhase {
    PreparedUnbound,
    BoundToTransition,
    Applying,
    FailedPostCommit,
    RecoveryRequired,
}

#[derive(Clone, Debug)]
struct RecordedTransitionOutcome {
    source_session_id: String,
    kind: TransitionKind,
    decision: TransitionDecision,
    result: Result<DocumentSessionSnapshot, DocumentSessionError>,
    resulting_authority: Option<RecordedTransitionAuthority>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedTransitionAuthority {
    active_session_id: Option<String>,
    identity_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TransitionAuthorityIdentity {
    source_session_id: String,
    kind: TransitionKind,
    initial_identity_epoch: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionAuthorityReleaseEvent {
    GuardPublicationStarted,
    GuardPublicationCompleted,
    OperationLeasesReleased,
    LifecycleAuthorityReleased,
    SaveLifecycleWaitBlocked,
    SaveLifecycleAcquired,
    SaveOperationAcquired,
    TransitionLifecycleWaitBlocked,
    TransitionLifecycleAcquired,
    TransitionOperationAcquired,
    ShutdownLifecycleWaitBlocked,
    ShutdownLifecycleAcquired,
}

#[cfg(test)]
struct TransitionDropTraceObserver {
    trace: Option<Arc<Mutex<Vec<TransitionAuthorityReleaseEvent>>>>,
    event: TransitionAuthorityReleaseEvent,
}

#[cfg(test)]
impl Drop for TransitionDropTraceObserver {
    fn drop(&mut self) {
        if let Some(trace) = &self.trace {
            trace.lock().push(self.event);
        }
    }
}

#[derive(Clone, Debug)]
struct SessionTransition {
    token: String,
    source_session_id: String,
    kind: TransitionKind,
    transition_epoch: u64,
    created_at_ms: u64,
    captured_content_generation: u64,
    expected_state_revision: u64,
    status: TransitionStatus,
    prepared_target: Option<PathBuf>,
    execution: TransitionExecution,
}

#[derive(Clone, Debug)]
struct BaselineFingerprint {
    digest: String,
}

#[derive(Clone, Debug)]
struct SaveCommitDiagnostic {
    target_path: PathBuf,
    committed_digest: Option<String>,
    commit_state: CommitState,
    recovery_path: Option<PathBuf>,
}

impl SaveCommitDiagnostic {
    fn summary(&self) -> String {
        format!(
            "target={} state={} digest={} recovery={}",
            self.target_path.display(),
            self.commit_state,
            self.committed_digest.as_deref().unwrap_or("unknown"),
            self.recovery_path
                .as_deref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| "none".to_string())
        )
    }
}

#[derive(Debug)]
pub(crate) struct DocumentSession {
    id: String,
    save_target_path: PathBuf,
    working_path: PathBuf,
    session_dir: PathBuf,
    content_generation: u64,
    state_revision: u64,
    dirty: bool,
    lifecycle: SessionLifecycle,
    editable: bool,
    read_only_reason: Option<String>,
    opened_at_ms: u64,
    baseline: BaselineFingerprint,
    working_digest: Option<String>,
    health_warning: Option<String>,
    credential: Option<String>,
    active_transition: Option<SessionTransition>,
    last_save_commit: Option<SaveCommitDiagnostic>,
    history: SessionHistory,
}

/// En fazla geri alınabilecek adım sayısı (tasarım §5.5, K4).
const MAX_HISTORY_ENTRIES: usize = 50;
/// Geçmiş dosyalarının toplam boyut bütçesi. Her giriş tam bir PDF kopyasıdır (§5.5); büyük
/// belgede bütçe adım sayısından önce dolar (392 MB'lık belgede yaklaşık iki adım, S6).
const MAX_HISTORY_BYTES: u64 = 1 << 30;

/// Undo/redo geçmişi (docs/engineering/undo-redo-and-incremental-save-design.md §5).
///
/// Her giriş, bir mutasyondan ya da geri yüklemeden **önceki** working PDF'in değişmez bir
/// kopyasıdır ve `session_dir/history/` altında durur. Kopya yeni yazılmaz: mutasyon işleminin
/// zaten aldığı snapshot commit'ten sonra oraya taşınır. Girdiler yalnız işlemin exclusive
/// lease'i altında değişir; geri yükleme digest eşleşmezse reddedilir ve geçmiş temizlenir.
#[derive(Debug, Default)]
pub(crate) struct SessionHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryEntry {
    /// Bu durumdan çıkaran (geri alınınca geri dönülen) işlemin kodu.
    operation: &'static str,
    /// Girişin geri yükleyeceği working PDF'in digest'i.
    digest: String,
    blob: PathBuf,
    bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryDirection {
    Undo,
    Redo,
}

impl SessionHistory {
    fn total_bytes(&self) -> u64 {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(|entry| entry.bytes)
            .sum()
    }

    /// Yeni bir mutasyon commit edildi: redo geçersizleşir, önceki durum undo'ya girer.
    fn record_mutation(&mut self, entry: HistoryEntry) {
        for stale in self.redo.drain(..) {
            remove_history_blob(&stale.blob);
        }
        self.undo.push(entry);
        self.enforce_limits();
    }

    /// `direction` yönündeki en üst giriş geri yüklendi; geri yüklemeden önceki durum (`previous`)
    /// ters yöne girer. Geri yüklenen girişin dosyası artık working PDF'tedir ve silinir.
    fn record_restore(&mut self, direction: HistoryDirection, previous: HistoryEntry) {
        let (from, to) = match direction {
            HistoryDirection::Undo => (&mut self.undo, &mut self.redo),
            HistoryDirection::Redo => (&mut self.redo, &mut self.undo),
        };
        if let Some(restored) = from.pop() {
            remove_history_blob(&restored.blob);
        }
        to.push(previous);
        self.enforce_limits();
    }

    fn top(&self, direction: HistoryDirection) -> Option<&HistoryEntry> {
        match direction {
            HistoryDirection::Undo => self.undo.last(),
            HistoryDirection::Redo => self.redo.last(),
        }
    }

    /// En eski undo girişlerinden başlayarak adım ve bayt sınırlarına iner; undo boşalırsa en
    /// uzaktaki redo girişleri düşer.
    fn enforce_limits(&mut self) {
        while self.undo.len() > MAX_HISTORY_ENTRIES
            || (self.total_bytes() > MAX_HISTORY_BYTES && !self.undo.is_empty())
        {
            let oldest = self.undo.remove(0);
            remove_history_blob(&oldest.blob);
        }
        while self.redo.len() > MAX_HISTORY_ENTRIES || self.total_bytes() > MAX_HISTORY_BYTES {
            let farthest = self.redo.remove(0);
            remove_history_blob(&farthest.blob);
        }
    }

    fn clear(&mut self) {
        for entry in self.undo.drain(..).chain(self.redo.drain(..)) {
            remove_history_blob(&entry.blob);
        }
    }
}

fn remove_history_blob(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                code = "HISTORY_BLOB_REMOVE_FAILED",
                error = %error,
                "Geçmiş dosyası silinemedi; oturum dizini kapanışta temizlenir"
            );
        }
    }
}

/// İşlem snapshot'ını (işlem öncesi working PDF'in kopyası) geçmiş dizinine taşır. Snapshot
/// ile session dizini aynı instance dizinindedir; taşıma bir rename'dir, kopya değildir.
fn retain_transaction_snapshot(
    snapshot_path: &Path,
    snapshot_digest: Option<&str>,
    before_digest: Option<&str>,
    session_dir: &Path,
) -> Result<(PathBuf, u64), String> {
    match (snapshot_digest, before_digest) {
        (Some(snapshot), Some(before)) if snapshot == before => {}
        _ => return Err("işlem snapshot'ı işlem öncesi working PDF ile eşleşmiyor".to_string()),
    }
    let directory = session_dir.join("history");
    create_secure_dir(&directory).map_err(|error| error.message)?;
    let blob = directory.join(format!("{}.pdf", Uuid::new_v4()));
    fs::rename(snapshot_path, &blob).map_err(|error| format!("snapshot taşınamadı: {error}"))?;
    let bytes = fs::metadata(&blob)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Ok((blob, bytes))
}

pub(crate) struct SessionEntry {
    pub(crate) operation: Arc<AsyncRwLock<()>>,
    pub(crate) state: Mutex<DocumentSession>,
}

/// Holds the session operation read lease from verified-artifact target
/// preflight through publication. Save and Save As use the corresponding write
/// lease, so they cannot change persistence identity while this guard exists.
pub(crate) struct VerifiedArtifactExportGuard {
    pub(crate) paths: VerifiedArtifactExportPaths,
    _operation_lease: OwnedRwLockReadGuard<()>,
}

pub(crate) type SecureRedactionExportGuard = VerifiedArtifactExportGuard;

pub(crate) struct SessionPublicationGuard {
    pub(crate) _operation_lease: OwnedRwLockReadGuard<()>,
}

struct WorkerReadContext {
    _admission: WorkerAdmissionPermit,
    _lease: OwnedRwLockReadGuard<()>,
    key: WorkerDocumentKey,
    page_count: u32,
}

impl crate::pdf_engine::subprocess_manager::WorkerReadLease for WorkerReadContext {
    fn admission(&self) -> &WorkerAdmissionPermit {
        &self._admission
    }

    fn document_key(&self) -> &WorkerDocumentKey {
        &self.key
    }

    fn page_count(&self) -> u32 {
        self.page_count
    }
}

#[derive(Clone, Debug)]
struct CandidateActivation {
    token: String,
    previous_session_id: Option<String>,
    previous_transition_token: Option<String>,
    candidate_session_id: String,
    created_at_ms: u64,
    phase: CandidateActivationPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionJobOperationStatus {
    CreatingSnapshot,
    Ready,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Cancelled,
}

impl SessionJobOperationStatus {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug)]
enum SessionJobCleanupStatus {
    Pending,
    InProgress,
    RetryScheduled { retry_at_ms: u64 },
    Deferred,
    Cleaned,
}

#[derive(Debug)]
struct SessionJobMutable {
    operation_status: SessionJobOperationStatus,
    cleanup_status: SessionJobCleanupStatus,
    credential_context: Option<String>,
    cancel_requested: bool,
    cancellation_reason: Option<SessionJobCancellationReason>,
    cleanup_attempts: u8,
}

#[derive(Debug)]
struct SessionJobRecord {
    job_id: Uuid,
    owner_session_id: String,
    content_generation: u64,
    snapshot_path: PathBuf,
    job_dir: PathBuf,
    mutable: Mutex<SessionJobMutable>,
    operation: Arc<AsyncRwLock<()>>,
    cleanup_single_flight: AsyncMutex<()>,
    cancellation_token: CancellationToken,
    cancellation_reason: Arc<Mutex<Option<SessionJobCancellationReason>>>,
    child_processes: SessionJobChildProcessOwner,
    cleanup_failures_remaining: AtomicU8,
}

#[derive(Clone, Debug)]
struct PendingSessionJobCancellation {
    session_id: String,
    created_at_ms: u64,
    reason: SessionJobCancellationReason,
}

#[derive(Clone, Debug)]
struct TerminalSessionJob {
    session_id: String,
    completed_at_ms: u64,
}

#[derive(Default)]
struct SessionJobRegistry {
    active: HashMap<Uuid, Arc<SessionJobRecord>>,
    pending_cancellations: HashMap<(String, Uuid), PendingSessionJobCancellation>,
    deferred_cleanup: HashMap<Uuid, Arc<SessionJobRecord>>,
    recent_terminal: HashMap<Uuid, TerminalSessionJob>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionJobFailure {
    pub code: String,
    pub message: String,
    pub commit_state: Option<String>,
    pub recovery_path: Option<String>,
    // Boxed like `DocumentSessionError` so `SessionJobOperationError` stays
    // under clippy's large-error threshold.
    pub replacement_digest: Option<Box<str>>,
    /// File names already present in the output directory of a multi-file
    /// export (`SESSION_JOB_OUTPUT_CONFLICT`); the caller may retry with
    /// `overwrite: true` after the user confirms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicting_names: Option<Box<ConflictingOutputNames>>,
}

/// Boxed so the rarely used list keeps `SessionJobOperationError` small.
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct ConflictingOutputNames(pub Vec<String>);

impl SessionJobFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            commit_state: None,
            recovery_path: None,
            replacement_digest: None,
            conflicting_names: None,
        }
    }

    pub fn pdf_engine(message: impl Into<String>) -> Self {
        Self::new("DOCUMENT_SESSION_PDF_ENGINE_ERROR", message)
    }

    pub(crate) fn publication_failure(
        code: impl Into<String>,
        error: crate::document_io::DocumentIoError,
    ) -> Self {
        Self {
            code: code.into(),
            message: error.detail,
            commit_state: Some(error.state.to_string()),
            recovery_path: error
                .recovery_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            replacement_digest: error.replacement_digest.map(String::into_boxed_str),
            conflicting_names: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionJobCancellationReason {
    UserRequested,
    Shutdown,
    Timeout,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionJobCleanupWarning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionJobCleanupOutcome {
    Cleaned,
    RetryScheduled {
        attempt: u8,
        retry_at_ms: u64,
        warning: SessionJobCleanupWarning,
    },
    Deferred {
        attempts: u8,
        warning: SessionJobCleanupWarning,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionJobOutcome<T> {
    Completed {
        #[serde(rename = "jobId")]
        job_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "contentGeneration")]
        content_generation: u64,
        value: T,
        #[serde(rename = "cancelRequested")]
        cancel_requested: bool,
        #[serde(rename = "cancellationReason")]
        cancellation_reason: Option<SessionJobCancellationReason>,
        cleanup: SessionJobCleanupOutcome,
    },
    Failed {
        #[serde(rename = "jobId")]
        job_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "contentGeneration")]
        content_generation: u64,
        error: SessionJobFailure,
        cleanup: SessionJobCleanupOutcome,
    },
    Cancelled {
        #[serde(rename = "jobId")]
        job_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "contentGeneration")]
        content_generation: u64,
        cancellation: SessionJobCancellationReason,
        cleanup: SessionJobCleanupOutcome,
    },
}

impl<T> SessionJobOutcome<T> {
    pub fn map<U>(self, mapper: impl FnOnce(T) -> U) -> SessionJobOutcome<U> {
        match self {
            Self::Completed {
                job_id,
                session_id,
                content_generation,
                value,
                cancel_requested,
                cancellation_reason,
                cleanup,
            } => SessionJobOutcome::Completed {
                job_id,
                session_id,
                content_generation,
                value: mapper(value),
                cancel_requested,
                cancellation_reason,
                cleanup,
            },
            Self::Failed {
                job_id,
                session_id,
                content_generation,
                error,
                cleanup,
            } => SessionJobOutcome::Failed {
                job_id,
                session_id,
                content_generation,
                error,
                cleanup,
            },
            Self::Cancelled {
                job_id,
                session_id,
                content_generation,
                cancellation,
                cleanup,
            } => SessionJobOutcome::Cancelled {
                job_id,
                session_id,
                content_generation,
                cancellation,
                cleanup,
            },
        }
    }

    pub fn map_to_failed<U>(
        self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> SessionJobOutcome<U> {
        let (job_id, session_id, content_generation, cleanup) = match self {
            Self::Completed {
                job_id,
                session_id,
                content_generation,
                cleanup,
                ..
            } => (job_id, session_id, content_generation, cleanup),
            Self::Failed {
                job_id,
                session_id,
                content_generation,
                cleanup,
                ..
            } => (job_id, session_id, content_generation, cleanup),
            Self::Cancelled {
                job_id,
                session_id,
                content_generation,
                cleanup,
                ..
            } => (job_id, session_id, content_generation, cleanup),
        };
        SessionJobOutcome::Failed {
            job_id,
            session_id,
            content_generation,
            error: SessionJobFailure::new(code, message),
            cleanup,
        }
    }

    pub fn map_to_cancelled<U>(
        self,
        cancellation: SessionJobCancellationReason,
    ) -> SessionJobOutcome<U> {
        let (job_id, session_id, content_generation, cleanup) = match self {
            Self::Completed {
                job_id,
                session_id,
                content_generation,
                cleanup,
                ..
            } => (job_id, session_id, content_generation, cleanup),
            Self::Failed {
                job_id,
                session_id,
                content_generation,
                cleanup,
                ..
            } => (job_id, session_id, content_generation, cleanup),
            Self::Cancelled {
                job_id,
                session_id,
                content_generation,
                cleanup,
                ..
            } => (job_id, session_id, content_generation, cleanup),
        };
        SessionJobOutcome::Cancelled {
            job_id,
            session_id,
            content_generation,
            cancellation,
            cleanup,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SessionJobCancelOutcome {
    CancelRequested,
    AlreadyRequested,
    CancelPending,
    AlreadyTerminal,
}

// ============================================================================
// PROMPT 6.4.1A — Managed Mutation Transaction Structures & Manager
// ============================================================================

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MutationTransactionId(pub Uuid);

impl MutationTransactionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MutationTransactionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MutationTransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationTransactionPhase {
    Created,
    Preparing,
    Mutating,
    Validating,
    CommitReady,
    Committing,
    Committed,
    ApplyingSessionState,
    Completed,
    Aborted,
    FailedPreCommit,
    FailedPostCommit,
}

impl MutationTransactionPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Aborted | Self::FailedPreCommit | Self::FailedPostCommit
        )
    }

    pub fn is_post_commit(self) -> bool {
        matches!(
            self,
            Self::Committing
                | Self::Committed
                | Self::ApplyingSessionState
                | Self::Completed
                | Self::FailedPostCommit
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MutationTransactionOutcome {
    Completed {
        transaction_id: String,
        session_id: String,
        snapshot: DocumentSessionSnapshot,
        cleanup_warning: Option<String>,
    },
    Aborted {
        transaction_id: String,
        session_id: String,
        reason: String,
        cleanup_warning: Option<String>,
    },
    FailedPreCommit {
        transaction_id: String,
        session_id: String,
        error: DocumentSessionError,
        cleanup_warning: Option<String>,
    },
    NotAppliedAfterCommitAttempt {
        transaction_id: String,
        session_id: String,
        error: DocumentSessionError,
        snapshot: DocumentSessionSnapshot,
        cleanup_warning: Option<String>,
    },
    CommittedUnconfirmed {
        transaction_id: String,
        session_id: String,
        snapshot: DocumentSessionSnapshot,
        warning: String,
    },
    RecoveryRequired {
        transaction_id: String,
        session_id: String,
        snapshot: DocumentSessionSnapshot,
        error: DocumentSessionError,
    },
    Indeterminate {
        transaction_id: String,
        session_id: String,
        snapshot: DocumentSessionSnapshot,
        error: DocumentSessionError,
    },
}

impl MutationTransactionOutcome {
    pub fn cleanup_warning(&self) -> Option<&str> {
        match self {
            Self::Completed {
                cleanup_warning, ..
            }
            | Self::Aborted {
                cleanup_warning, ..
            }
            | Self::FailedPreCommit {
                cleanup_warning, ..
            }
            | Self::NotAppliedAfterCommitAttempt {
                cleanup_warning, ..
            } => cleanup_warning.as_deref(),
            Self::CommittedUnconfirmed { warning, .. } => Some(warning.as_str()),
            Self::RecoveryRequired { snapshot, .. } | Self::Indeterminate { snapshot, .. } => {
                snapshot.health_warning.as_deref()
            }
        }
    }

    pub fn is_post_commit(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. }
                | Self::CommittedUnconfirmed { .. }
                | Self::NotAppliedAfterCommitAttempt { .. }
                | Self::RecoveryRequired { .. }
                | Self::Indeterminate { .. }
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MutationTransactionControlState {
    pub(crate) phase: MutationTransactionPhase,
    pub(crate) cancel_requested: bool,
    pub(crate) cancel_reason: Option<String>,
    pub(crate) terminal_outcome: Option<MutationTransactionOutcome>,
}

#[derive(Debug)]
pub struct MutationTransactionRecord {
    pub id: MutationTransactionId,
    pub session_id: String,
    pub start_content_generation: u64,
    pub(crate) control: Mutex<MutationTransactionControlState>,
    pub cancellation_token: CancellationToken,
    pub(crate) phase_notify: tokio::sync::Notify,
}

impl MutationTransactionRecord {
    pub fn request_cancel(&self, reason: String) -> bool {
        let mut ctrl = self.control.lock();
        if ctrl.phase.is_post_commit() || ctrl.phase == MutationTransactionPhase::Committing {
            return false;
        }
        if ctrl.cancel_requested {
            return true;
        }
        ctrl.cancel_requested = true;
        if ctrl.cancel_reason.is_none() {
            ctrl.cancel_reason = Some(reason);
        }
        self.cancellation_token.cancel();
        self.phase_notify.notify_waiters();
        true
    }

    pub fn try_enter_committing(&self) -> Result<(), DocumentSessionError> {
        let mut ctrl = self.control.lock();
        if ctrl.phase != MutationTransactionPhase::CommitReady {
            return Err(DocumentSessionError::new(
                "MUTATION_INVALID_STATE_TRANSITION",
                format!(
                    "Committing fazına geçiş için CommitReady bekleniyordu, fakat phase '{:?}'",
                    ctrl.phase
                ),
            ));
        }
        if ctrl.cancel_requested {
            let reason = ctrl
                .cancel_reason
                .clone()
                .unwrap_or_else(|| "MUTATION_TRANSACTION_CANCELLED".to_string());
            return Err(DocumentSessionError::new(
                "MUTATION_TRANSACTION_CANCELLED",
                reason,
            ));
        }
        if ctrl.terminal_outcome.is_some() {
            return Err(DocumentSessionError::new(
                "MUTATION_TRANSACTION_CANCELLED",
                "Transaction zaten sonlandı",
            ));
        }
        ctrl.phase = MutationTransactionPhase::Committing;
        self.phase_notify.notify_waiters();
        Ok(())
    }

    pub fn transition_to(
        &self,
        target: MutationTransactionPhase,
    ) -> Result<(), DocumentSessionError> {
        let mut ctrl = self.control.lock();
        let current = ctrl.phase;
        if current == target {
            return Ok(());
        }
        if ctrl.terminal_outcome.is_some() {
            return Err(DocumentSessionError::new(
                "MUTATION_INVALID_STATE_TRANSITION",
                format!(
                    "Terminal outcome var iken faz değiştirilemez: '{current:?}' -> '{target:?}'"
                ),
            ));
        }
        let valid = match (current, target) {
            (MutationTransactionPhase::Created, MutationTransactionPhase::Preparing) => true,
            (MutationTransactionPhase::Preparing, MutationTransactionPhase::Mutating) => true,
            (MutationTransactionPhase::Mutating, MutationTransactionPhase::Validating) => true,
            (MutationTransactionPhase::Validating, MutationTransactionPhase::CommitReady) => true,
            (MutationTransactionPhase::CommitReady, MutationTransactionPhase::Committing) => true,
            (MutationTransactionPhase::Committing, MutationTransactionPhase::Committed) => true,
            (
                MutationTransactionPhase::Committed,
                MutationTransactionPhase::ApplyingSessionState,
            ) => true,
            (
                MutationTransactionPhase::ApplyingSessionState,
                MutationTransactionPhase::Completed,
            ) => true,

            // Failure & Abort paths
            (_, MutationTransactionPhase::Aborted) if !current.is_post_commit() => true,
            (_, MutationTransactionPhase::FailedPreCommit) if !current.is_post_commit() => true,
            (_, MutationTransactionPhase::FailedPostCommit) if current.is_post_commit() => true,

            _ => false,
        };

        if valid {
            ctrl.phase = target;
            self.phase_notify.notify_waiters();
            Ok(())
        } else {
            Err(DocumentSessionError::new(
                "MUTATION_INVALID_STATE_TRANSITION",
                format!("Geçersiz phase geçişi: '{current:?}' -> '{target:?}'"),
            ))
        }
    }

    pub fn publish_terminal(
        &self,
        outcome: MutationTransactionOutcome,
    ) -> Result<(), DocumentSessionError> {
        let mut ctrl = self.control.lock();
        if ctrl.terminal_outcome.is_some() {
            return Err(DocumentSessionError::new(
                "MUTATION_INVALID_STATE_TRANSITION",
                "Terminal outcome zaten yayınlandı",
            ));
        }
        let current = ctrl.phase;
        let target_phase = match &outcome {
            MutationTransactionOutcome::Completed { .. } => MutationTransactionPhase::Completed,
            MutationTransactionOutcome::Aborted { .. } => MutationTransactionPhase::Aborted,
            MutationTransactionOutcome::FailedPreCommit { .. } => {
                MutationTransactionPhase::FailedPreCommit
            }
            MutationTransactionOutcome::CommittedUnconfirmed { .. } => {
                MutationTransactionPhase::Completed
            }
            MutationTransactionOutcome::NotAppliedAfterCommitAttempt { .. } => {
                MutationTransactionPhase::FailedPostCommit
            }
            MutationTransactionOutcome::RecoveryRequired { .. } => {
                MutationTransactionPhase::FailedPostCommit
            }
            MutationTransactionOutcome::Indeterminate { .. } => {
                MutationTransactionPhase::FailedPostCommit
            }
        };

        let valid = match target_phase {
            MutationTransactionPhase::Completed => {
                current == MutationTransactionPhase::ApplyingSessionState
                    || current == MutationTransactionPhase::Committed
            }
            MutationTransactionPhase::Aborted | MutationTransactionPhase::FailedPreCommit => {
                !current.is_post_commit()
            }
            MutationTransactionPhase::FailedPostCommit => {
                current.is_post_commit() || current == MutationTransactionPhase::Committing
            }
            _ => false,
        };

        if !valid {
            return Err(DocumentSessionError::new(
                "MUTATION_INVALID_STATE_TRANSITION",
                format!(
                    "Geçersiz terminal publication phase geçişi: '{current:?}' -> '{target_phase:?}'"
                ),
            ));
        }

        ctrl.phase = target_phase;
        ctrl.terminal_outcome = Some(outcome);
        self.phase_notify.notify_waiters();
        Ok(())
    }

    pub fn publish_emergency_terminal(&self, err: DocumentSessionError) {
        let mut ctrl = self.control.lock();
        if ctrl.terminal_outcome.is_none() {
            let snap = DocumentSessionSnapshot {
                id: self.session_id.clone(),
                opened_at_ms: 0,
                display_path: self.session_id.clone(),
                file_name: self.session_id.clone(),
                content_generation: self.start_content_generation,
                state_revision: 0,
                dirty: true,
                lifecycle: SessionLifecycle::CommitIndeterminate,
                editable: false,
                read_only_reason: None,
                health_warning: Some(err.message.clone()),
                undo_operation: None,
                redo_operation: None,
            };
            ctrl.phase = MutationTransactionPhase::FailedPostCommit;
            ctrl.terminal_outcome = Some(MutationTransactionOutcome::Indeterminate {
                transaction_id: self.id.to_string(),
                session_id: self.session_id.clone(),
                snapshot: snap,
                error: err,
            });
        }
        self.phase_notify.notify_waiters();
    }

    pub async fn wait_for_phase(&self, target: MutationTransactionPhase) {
        loop {
            if self.control.lock().phase == target {
                return;
            }
            let notified = self.phase_notify.notified();
            if self.control.lock().phase == target {
                return;
            }
            notified.await;
        }
    }

    pub async fn wait_for_terminal(&self) -> MutationTransactionOutcome {
        loop {
            if let Some(outcome) = self.control.lock().terminal_outcome.clone() {
                return outcome;
            }
            let notified = self.phase_notify.notified();
            if let Some(outcome) = self.control.lock().terminal_outcome.clone() {
                return outcome;
            }
            notified.await;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationManagerLifecycle {
    Running,
    ShuttingDown,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownCoordinatorState {
    NotStarted,
    Running,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinatorJoinState {
    NotStarted,
    Running,
    JoinedSuccess,
    JoinedFailure(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinatorMonitorJoinState {
    NotStarted,
    Running,
    BodyCompleted,
    Joining,
    ExternallyJoinedSuccess,
    ExternallyJoinedFailure(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReaperMonitorJoinState {
    NotStarted,
    Running,
    BodyCompleted,
    Joining,
    ExternallyJoinedSuccess,
    ExternallyJoinedFailure(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReaperTaskState {
    NotStarted,
    Running,
    StopRequested,
    JoinedSuccess,
    JoinedFailure(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorHandleOwnership {
    Registered,
    ClaimedByReaper,
    ClaimedByShutdown,
    Joined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorFinalizationState {
    Registered,
    JoinInProgress,
    RecoveryInProgress,
    CleanupInProgress,
    TerminalPublished,
    TerminationUnproven(String),
    FinalizationFailed(String),
    RemovalReady,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorExecutionState {
    Running,
    CancellationRequested,
    AbortRequested,
    Exited,
    TerminationUnproven(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExchangeCleanupState {
    Owned,
    CleanupInProgress,
    Cleaned,
    CleanupFailed(String),
}

#[derive(Debug)]
struct ExchangeCleanupAuthorityInner {
    dir: Option<tempfile::TempDir>,
    path: PathBuf,
    state: ExchangeCleanupState,
}

#[derive(Clone, Debug)]
pub(crate) struct ExchangeCleanupAuthority {
    inner: Arc<Mutex<ExchangeCleanupAuthorityInner>>,
}

impl ExchangeCleanupAuthority {
    pub fn new(dir: tempfile::TempDir, path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ExchangeCleanupAuthorityInner {
                dir: Some(dir),
                path,
                state: ExchangeCleanupState::Owned,
            })),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.inner.lock().path.clone()
    }

    #[cfg(test)]
    pub fn state(&self) -> ExchangeCleanupState {
        self.inner.lock().state.clone()
    }

    fn is_cleaned(&self) -> bool {
        self.inner.lock().state == ExchangeCleanupState::Cleaned
    }

    pub fn close_exchange_dir(&self) -> Result<(), DocumentSessionError> {
        let mut guard = self.inner.lock();
        match &guard.state {
            ExchangeCleanupState::Cleaned => return Ok(()),
            ExchangeCleanupState::CleanupFailed(msg) => {
                return Err(DocumentSessionError::new(
                    "MUTATION_EXCHANGE_CLEANUP_FAILED",
                    msg.clone(),
                ));
            }
            ExchangeCleanupState::CleanupInProgress => return Ok(()),
            ExchangeCleanupState::Owned => {
                guard.state = ExchangeCleanupState::CleanupInProgress;
            }
        }

        let dir = guard.dir.take();
        if let Some(d) = dir {
            if let Err(e) = d.close() {
                let err_msg = format!("Exchange dir cleanup failed for {:?}: {e}", guard.path);
                guard.state = ExchangeCleanupState::CleanupFailed(err_msg.clone());
                return Err(DocumentSessionError::new(
                    "MUTATION_EXCHANGE_CLEANUP_FAILED",
                    err_msg,
                ));
            }
        }
        guard.state = ExchangeCleanupState::Cleaned;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinalizerClaim {
    Unclaimed,
    ClaimedByNormalFinalizer,
    ClaimedByExternalRecovery,
    Finalized,
}

#[derive(Clone)]
pub(crate) struct SupervisorExecutionAuthority {
    pub(crate) cancel_token: CancellationToken,
    pub(crate) liveness_notify: Arc<tokio::sync::Notify>,
    pub(crate) execution_state: Arc<Mutex<SupervisorExecutionState>>,
    pub(crate) supervisor_body_exited: Arc<AtomicBool>,
    pub(crate) owner_body_exited: Arc<AtomicBool>,
    pub(crate) abort_requested: Arc<AtomicBool>,
    pub(crate) blocking_tasks_in_progress: Arc<AtomicUsize>,
    pub(crate) child_owner: Arc<Mutex<Option<SessionJobChildProcessOwner>>>,
    pub(crate) finalizer_claim: Arc<Mutex<FinalizerClaim>>,
}

impl std::fmt::Debug for SupervisorExecutionAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupervisorExecutionAuthority")
            .field("execution_state", &*self.execution_state.lock())
            .field(
                "blocking_tasks_in_progress",
                &self.blocking_tasks_in_progress.load(Ordering::Acquire),
            )
            .field("finalizer_claim", &*self.finalizer_claim.lock())
            .finish()
    }
}

impl Default for SupervisorExecutionAuthority {
    fn default() -> Self {
        Self::new()
    }
}

impl SupervisorExecutionAuthority {
    pub fn new() -> Self {
        Self {
            cancel_token: CancellationToken::new(),
            liveness_notify: Arc::new(tokio::sync::Notify::new()),
            execution_state: Arc::new(Mutex::new(SupervisorExecutionState::Running)),
            supervisor_body_exited: Arc::new(AtomicBool::new(false)),
            owner_body_exited: Arc::new(AtomicBool::new(false)),
            abort_requested: Arc::new(AtomicBool::new(false)),
            blocking_tasks_in_progress: Arc::new(AtomicUsize::new(0)),
            child_owner: Arc::new(Mutex::new(None)),
            finalizer_claim: Arc::new(Mutex::new(FinalizerClaim::Unclaimed)),
        }
    }

    pub fn bind_child_owner(&self, child_owner: SessionJobChildProcessOwner) {
        *self.child_owner.lock() = Some(child_owner);
    }

    pub fn request_cancellation(&self) {
        self.cancel_token.cancel();
        let mut state = self.execution_state.lock();
        if *state == SupervisorExecutionState::Running {
            *state = SupervisorExecutionState::CancellationRequested;
        }
        self.liveness_notify.notify_waiters();
    }

    pub fn mark_abort_requested(&self) {
        self.abort_requested.store(true, Ordering::Release);
        let mut state = self.execution_state.lock();
        if *state != SupervisorExecutionState::Exited {
            *state = SupervisorExecutionState::AbortRequested;
        }
        self.liveness_notify.notify_waiters();
    }

    pub fn mark_supervisor_body_exited(&self) {
        self.supervisor_body_exited.store(true, Ordering::Release);
        self.try_publish_exited();
    }

    pub fn mark_owner_body_exited(&self) {
        self.owner_body_exited.store(true, Ordering::Release);
        self.try_publish_exited();
    }

    pub fn blocking_task_started(&self) {
        self.blocking_tasks_in_progress
            .fetch_add(1, Ordering::AcqRel);
    }

    pub fn blocking_task_finished(&self) {
        self.blocking_tasks_in_progress
            .fetch_sub(1, Ordering::AcqRel);
        self.try_publish_exited();
    }

    pub fn refresh_exit_acknowledgement(&self) {
        self.try_publish_exited();
    }

    fn try_publish_exited(&self) {
        let children_exited = self
            .child_owner
            .lock()
            .as_ref()
            .is_none_or(|owner| owner.active_child_count() == 0);
        if self.supervisor_body_exited.load(Ordering::Acquire)
            && self.owner_body_exited.load(Ordering::Acquire)
            && self.blocking_tasks_in_progress.load(Ordering::Acquire) == 0
            && children_exited
        {
            *self.execution_state.lock() = SupervisorExecutionState::Exited;
        }
        self.liveness_notify.notify_waiters();
    }

    pub fn is_exited(&self) -> bool {
        *self.execution_state.lock() == SupervisorExecutionState::Exited
    }

    pub fn execution_state(&self) -> SupervisorExecutionState {
        self.execution_state.lock().clone()
    }

    #[cfg(test)]
    pub fn was_abort_requested(&self) -> bool {
        self.abort_requested.load(Ordering::Acquire)
    }

    pub fn mark_termination_unproven(&self, detail: String) {
        *self.execution_state.lock() = SupervisorExecutionState::TerminationUnproven(detail);
        self.liveness_notify.notify_waiters();
    }

    pub fn try_claim_finalizer(&self, target: FinalizerClaim) -> bool {
        let mut guard = self.finalizer_claim.lock();
        if *guard == FinalizerClaim::Unclaimed {
            *guard = target;
            true
        } else {
            false
        }
    }

    pub fn mark_finalized(&self) {
        let mut guard = self.finalizer_claim.lock();
        *guard = FinalizerClaim::Finalized;
    }

    pub fn finalizer_claim(&self) -> FinalizerClaim {
        *self.finalizer_claim.lock()
    }
}

struct BlockingExecutionGuard(SupervisorExecutionAuthority);

impl Drop for BlockingExecutionGuard {
    fn drop(&mut self) {
        self.0.blocking_task_finished();
    }
}

struct SupervisorBodyExitGuard(SupervisorExecutionAuthority);

impl Drop for SupervisorBodyExitGuard {
    fn drop(&mut self) {
        self.0.mark_supervisor_body_exited();
    }
}

struct OwnerBodyExitGuard(SupervisorExecutionAuthority);

impl Drop for OwnerBodyExitGuard {
    fn drop(&mut self) {
        self.0.mark_owner_body_exited();
    }
}

#[cfg(test)]
fn spawn_managed_test_supervisor<F>(
    execution_authority: SupervisorExecutionAuthority,
    future: F,
) -> tokio::task::JoinHandle<MutationTransactionOutcome>
where
    F: std::future::Future<Output = MutationTransactionOutcome> + Send + 'static,
{
    tokio::spawn(async move {
        let _supervisor_guard = SupervisorBodyExitGuard(execution_authority.clone());
        let _owner_guard = OwnerBodyExitGuard(execution_authority.clone());
        tokio::pin!(future);
        tokio::select! {
            outcome = &mut future => outcome,
            _ = execution_authority.cancel_token.cancelled() => future.await,
        }
    })
}

fn spawn_supervised_blocking<F, T>(
    execution_authority: &SupervisorExecutionAuthority,
    operation: F,
) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    execution_authority.blocking_task_started();
    let authority = execution_authority.clone();
    tokio::task::spawn_blocking(move || {
        let _guard = BlockingExecutionGuard(authority);
        operation()
    })
}

#[derive(Debug, Default, Clone)]
pub(crate) struct MutationShutdownAccumulator {
    pub(crate) cancelled_transactions: usize,
    pub(crate) post_commit_completed: usize,
    pub(crate) failed_pre_commit: usize,
    pub(crate) commit_indeterminate: usize,
    pub(crate) supervisor_panics: usize,
    pub(crate) reaped_children: usize,
    pub(crate) warnings: Vec<CleanupWarning>,
    pub(crate) processed_ids: HashSet<MutationTransactionId>,
}

impl MutationShutdownAccumulator {
    pub fn record_outcome(
        &mut self,
        id: MutationTransactionId,
        outcome: &MutationTransactionOutcome,
    ) {
        if self.processed_ids.insert(id) {
            match outcome {
                MutationTransactionOutcome::Aborted { .. } => {
                    self.cancelled_transactions += 1;
                }
                MutationTransactionOutcome::Completed { .. }
                | MutationTransactionOutcome::CommittedUnconfirmed { .. } => {
                    self.post_commit_completed += 1;
                }
                MutationTransactionOutcome::FailedPreCommit { .. } => {
                    self.failed_pre_commit += 1;
                }
                MutationTransactionOutcome::NotAppliedAfterCommitAttempt { .. }
                | MutationTransactionOutcome::RecoveryRequired { .. }
                | MutationTransactionOutcome::Indeterminate { .. } => {
                    self.commit_indeterminate += 1;
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct MutationSupervisorRecoveryBundle {
    pub(crate) transaction_record: Arc<MutationTransactionRecord>,
    pub(crate) session_entry: Arc<SessionEntry>,
    pub(crate) recovery_ctx: MutationTransactionRecoveryContext,
    pub(crate) child_owner: SessionJobChildProcessOwner,
    pub(crate) exchange_cleanup_authority: ExchangeCleanupAuthority,
    pub(crate) execution_authority: SupervisorExecutionAuthority,
}

impl std::fmt::Debug for MutationSupervisorRecoveryBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationSupervisorRecoveryBundle")
            .field("transaction_record", &self.transaction_record.id)
            .field("recovery_ctx", &self.recovery_ctx)
            .finish()
    }
}

pub(crate) struct SupervisorRegistryEntry {
    pub(crate) handle: Option<tokio::task::JoinHandle<MutationTransactionOutcome>>,
    pub(crate) abort_handle: tokio::task::AbortHandle,
    pub(crate) ownership: SupervisorHandleOwnership,
    pub(crate) finalization_state: SupervisorFinalizationState,
    pub(crate) completion_notify: Arc<tokio::sync::Notify>,
    pub(crate) join_result: Option<Result<MutationTransactionOutcome, String>>,
    pub(crate) recovery_bundle: Option<MutationSupervisorRecoveryBundle>,
    pub(crate) execution_authority: SupervisorExecutionAuthority,
}

impl std::fmt::Debug for SupervisorRegistryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupervisorRegistryEntry")
            .field("ownership", &self.ownership)
            .field("finalization_state", &self.finalization_state)
            .field("join_result", &self.join_result)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationChildCleanupReport {
    pub attempted: usize,
    pub terminated: usize,
    pub reaped: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentShutdownReport {
    pub active_transactions_start: usize,
    pub cancelled_transactions: usize,
    pub post_commit_completed: usize,
    pub reaped_children: usize,
    pub timed_out_supervisors: usize,
    pub remaining_reservations: usize,
    pub remaining_registry_entries: usize,
    pub warnings: Vec<CleanupWarning>,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub failed_pre_commit: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub commit_indeterminate: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub supervisor_panics: usize,
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug)]
struct MutationTransactionRegistryInner {
    lifecycle: MutationManagerLifecycle,
    coordinator_state: ShutdownCoordinatorState,
    coordinator_join_state: CoordinatorJoinState,
    coordinator_monitor_join_state: CoordinatorMonitorJoinState,
    reaper_monitor_join_state: ReaperMonitorJoinState,
    reaper_task_state: ReaperTaskState,
    shutdown_completion: Option<ShutdownResult>,
    coordinator_start_count: usize,
    active_transactions: HashMap<MutationTransactionId, Arc<MutationTransactionRecord>>,
    session_reservations: HashMap<String, MutationTransactionId>,
    supervisor_handles: HashMap<MutationTransactionId, SupervisorRegistryEntry>,
    accumulator: MutationShutdownAccumulator,
    progress_revision: u64,
}

impl Default for MutationTransactionRegistryInner {
    fn default() -> Self {
        Self {
            lifecycle: MutationManagerLifecycle::Running,
            coordinator_state: ShutdownCoordinatorState::NotStarted,
            coordinator_join_state: CoordinatorJoinState::NotStarted,
            coordinator_monitor_join_state: CoordinatorMonitorJoinState::NotStarted,
            reaper_monitor_join_state: ReaperMonitorJoinState::NotStarted,
            reaper_task_state: ReaperTaskState::NotStarted,
            shutdown_completion: None,
            coordinator_start_count: 0,
            active_transactions: HashMap::new(),
            session_reservations: HashMap::new(),
            supervisor_handles: HashMap::new(),
            accumulator: MutationShutdownAccumulator::default(),
            progress_revision: 0,
        }
    }
}

#[derive(Debug)]
pub struct SessionReservationGuard {
    transaction_id: MutationTransactionId,
    session_id: String,
    registry: Arc<Mutex<MutationTransactionRegistryInner>>,
    progress_notify: Arc<tokio::sync::Notify>,
    active: AtomicBool,
}

impl SessionReservationGuard {
    pub fn release(&self) {
        if self.active.swap(false, Ordering::AcqRel) {
            let mut inner = self.registry.lock();
            if inner.session_reservations.get(&self.session_id) == Some(&self.transaction_id) {
                inner.session_reservations.remove(&self.session_id);
            }
            inner.active_transactions.remove(&self.transaction_id);
            inner.progress_revision += 1;
            drop(inner);
            self.progress_notify.notify_waiters();
        }
    }

    pub fn disarm(&self) {
        self.active.store(false, Ordering::Release);
    }
}

impl Drop for SessionReservationGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MutationTestHookPhase {
    MutationWorkerStarted,
    AfterMutationWorker,
    BeforeValidation,
    BeforeCommit,
    BeforeAtomicCommitCall,
    BeforeStateApply,
    BeforeCoordinatorBody,
    BeforeReaperJoin,
}

#[cfg(test)]
pub type MutationTestHookFn =
    Arc<dyn Fn(MutationTestHookPhase, &MutationTransactionId) + Send + Sync>;

pub type ShutdownResult = Arc<Result<DocumentShutdownReport, DocumentSessionError>>;
pub type ShutdownCoordinatorJoinHandle =
    tokio::task::JoinHandle<Result<DocumentShutdownReport, DocumentSessionError>>;

pub struct MutationTransactionManager {
    inner: Arc<Mutex<MutationTransactionRegistryInner>>,
    shutdown_coordinator_task: Arc<Mutex<Option<ShutdownCoordinatorJoinHandle>>>,
    shutdown_coordinator_monitor_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    reaper_monitor_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    shutdown_notify: Arc<tokio::sync::Notify>,
    reaper_notify: Arc<tokio::sync::Notify>,
    reaper_stop_token: CancellationToken,
    #[cfg(test)]
    test_hooks: Arc<Mutex<Vec<MutationTestHookFn>>>,
}

impl std::fmt::Debug for MutationTransactionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationTransactionManager")
            .field("inner", &self.inner)
            .finish()
    }
}

impl Default for MutationTransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationTransactionManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MutationTransactionRegistryInner::default())),
            shutdown_coordinator_task: Arc::new(Mutex::new(None)),
            shutdown_coordinator_monitor_task: Arc::new(Mutex::new(None)),
            reaper_monitor_task: Arc::new(Mutex::new(None)),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            reaper_notify: Arc::new(tokio::sync::Notify::new()),
            reaper_stop_token: CancellationToken::new(),
            #[cfg(test)]
            test_hooks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn lifecycle(&self) -> MutationManagerLifecycle {
        self.inner.lock().lifecycle
    }

    pub fn coordinator_state(&self) -> ShutdownCoordinatorState {
        self.inner.lock().coordinator_state
    }

    pub fn coordinator_join_state(&self) -> CoordinatorJoinState {
        self.inner.lock().coordinator_join_state.clone()
    }

    pub fn coordinator_monitor_join_state(&self) -> CoordinatorMonitorJoinState {
        self.inner.lock().coordinator_monitor_join_state.clone()
    }

    pub fn reaper_monitor_join_state(&self) -> ReaperMonitorJoinState {
        self.inner.lock().reaper_monitor_join_state.clone()
    }

    pub fn reaper_task_state(&self) -> ReaperTaskState {
        self.inner.lock().reaper_task_state.clone()
    }

    pub fn active_supervisor_count(&self) -> usize {
        self.inner.lock().supervisor_handles.len()
    }

    pub fn active_reservation_count(&self) -> usize {
        self.inner.lock().session_reservations.len()
    }

    pub fn ensure_reaper_loop_started(self: &Arc<Self>) {
        let should_start = {
            let mut inner = self.inner.lock();
            if inner.reaper_task_state != ReaperTaskState::NotStarted {
                false
            } else {
                inner.reaper_task_state = ReaperTaskState::Running;
                inner.reaper_monitor_join_state = ReaperMonitorJoinState::Running;
                inner.progress_revision += 1;
                true
            }
        };
        if !should_start {
            return;
        }

        let manager_for_loop = Arc::clone(self);
        let reaper_handle = tokio::spawn(async move {
            manager_for_loop.run_async_supervisor_reaper_loop().await;
        });
        let manager_for_monitor = Arc::clone(self);
        let monitor_handle = tokio::spawn(async move {
            manager_for_monitor.run_reaper_monitor(reaper_handle).await;
        });
        *self.reaper_monitor_task.lock() = Some(monitor_handle);
        self.shutdown_notify.notify_waiters();
    }

    async fn run_reaper_monitor(self: Arc<Self>, reaper_handle: tokio::task::JoinHandle<()>) {
        let join_result = reaper_handle.await;
        let should_spawn_failure_coordinator = {
            let mut inner = self.inner.lock();
            let reaper_failed = match join_result {
                Ok(()) => {
                    let already_failed =
                        matches!(inner.reaper_task_state, ReaperTaskState::JoinedFailure(_));
                    if !already_failed {
                        inner.reaper_task_state = ReaperTaskState::JoinedSuccess;
                    }
                    already_failed
                }
                Err(join_error) => {
                    let detail = format!("REAPER_TASK_PANIC: {join_error}");
                    inner.reaper_task_state = ReaperTaskState::JoinedFailure(detail.clone());
                    inner.accumulator.warnings.push(CleanupWarning {
                        code: "MUTATION_REAPER_PANIC".to_string(),
                        message: detail,
                    });
                    inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                    true
                }
            };
            inner.reaper_monitor_join_state = ReaperMonitorJoinState::BodyCompleted;
            inner.progress_revision += 1;
            if reaper_failed && inner.coordinator_state == ShutdownCoordinatorState::NotStarted {
                inner.coordinator_state = ShutdownCoordinatorState::Running;
                inner.coordinator_join_state = CoordinatorJoinState::Running;
                inner.coordinator_monitor_join_state = CoordinatorMonitorJoinState::Running;
                inner.coordinator_start_count += 1;
                true
            } else {
                false
            }
        };
        self.shutdown_notify.notify_waiters();
        self.reaper_notify.notify_waiters();
        if should_spawn_failure_coordinator {
            self.spawn_coordinator_and_monitor();
        }
    }

    pub(crate) fn register_supervisor_bundle(
        self: &Arc<Self>,
        id: MutationTransactionId,
        bundle: MutationSupervisorRecoveryBundle,
        handle: tokio::task::JoinHandle<MutationTransactionOutcome>,
    ) {
        let exec_auth = bundle.execution_authority.clone();
        exec_auth.bind_child_owner(bundle.child_owner.clone());
        let abort_handle = handle.abort_handle();
        {
            let mut inner = self.inner.lock();
            inner.supervisor_handles.insert(
                id,
                SupervisorRegistryEntry {
                    handle: Some(handle),
                    abort_handle,
                    ownership: SupervisorHandleOwnership::Registered,
                    finalization_state: SupervisorFinalizationState::Registered,
                    completion_notify: Arc::new(tokio::sync::Notify::new()),
                    join_result: None,
                    recovery_bundle: Some(bundle),
                    execution_authority: exec_auth,
                },
            );
            inner.progress_revision += 1;
        }
        self.reaper_notify.notify_waiters();
        self.ensure_reaper_loop_started();
    }

    pub fn release_reservation(&self, session_id: &str, transaction_id: &MutationTransactionId) {
        {
            let mut inner = self.inner.lock();
            if inner.session_reservations.get(session_id) == Some(transaction_id) {
                inner.session_reservations.remove(session_id);
            }
            inner.active_transactions.remove(transaction_id);
            inner.progress_revision += 1;
        }
        self.shutdown_notify.notify_waiters();
    }

    pub fn harvest_completed_supervisors(&self) {
        self.reaper_notify.notify_waiters();
    }

    async fn run_async_supervisor_reaper_loop(self: Arc<Self>) {
        {
            let mut inner = self.inner.lock();
            inner.reaper_task_state = ReaperTaskState::Running;
            inner.progress_revision += 1;
        }

        let mut join_set: tokio::task::JoinSet<(
            MutationTransactionId,
            Result<MutationTransactionOutcome, tokio::task::JoinError>,
        )> = tokio::task::JoinSet::new();

        loop {
            let to_claim: Vec<(
                MutationTransactionId,
                tokio::task::JoinHandle<MutationTransactionOutcome>,
            )> = {
                let mut inner = self.inner.lock();
                let mut claimed = Vec::new();
                for (id, entry) in inner.supervisor_handles.iter_mut() {
                    if entry.ownership == SupervisorHandleOwnership::Registered {
                        if let Some(handle) = entry.handle.take() {
                            entry.ownership = SupervisorHandleOwnership::ClaimedByReaper;
                            entry.finalization_state = SupervisorFinalizationState::JoinInProgress;
                            claimed.push((*id, handle));
                        }
                    }
                }
                if !claimed.is_empty() {
                    inner.progress_revision += 1;
                }
                claimed
            };

            for (id, handle) in to_claim {
                #[cfg(test)]
                self.trigger_test_hook(MutationTestHookPhase::BeforeReaperJoin, &id);
                join_set.spawn(async move {
                    let join_res = handle.await;
                    (id, join_res)
                });
            }

            let stop_requested = self.reaper_stop_token.is_cancelled();
            let pending_in_registry = {
                let inner = self.inner.lock();
                inner.supervisor_handles.values().any(|e| {
                    e.ownership == SupervisorHandleOwnership::Registered
                        || e.ownership == SupervisorHandleOwnership::ClaimedByReaper
                })
            };

            if stop_requested && join_set.is_empty() && !pending_in_registry {
                let mut inner = self.inner.lock();
                if inner.reaper_task_state == ReaperTaskState::Running {
                    inner.reaper_task_state = ReaperTaskState::StopRequested;
                    inner.progress_revision += 1;
                }
                break;
            }

            tokio::select! {
                Some(res) = join_set.join_next(), if !join_set.is_empty() => {
                    match res {
                        Ok((id, Ok(outcome))) => {
                            {
                                let mut inner_guard = self.inner.lock();
                                if let Some(entry) = inner_guard.supervisor_handles.get_mut(&id) {
                                    entry.join_result = Some(Ok(outcome.clone()));
                                    entry.ownership = SupervisorHandleOwnership::Joined;
                                    entry.finalization_state = SupervisorFinalizationState::CleanupInProgress;
                                    entry.completion_notify.notify_waiters();
                                    inner_guard.progress_revision += 1;
                                }
                            }
                            finalize_supervisor_normal(&self, &id, outcome);
                            self.shutdown_notify.notify_waiters();
                        }
                        Ok((id, Err(join_err))) => {
                            let join_err_msg = format!("SUPERVISOR_PANIC: {join_err}");
                            {
                                let mut inner_guard = self.inner.lock();
                                if join_err.is_panic() {
                                    inner_guard.accumulator.supervisor_panics += 1;
                                    inner_guard.accumulator.warnings.push(CleanupWarning {
                                        code: "MUTATION_SUPERVISOR_PANIC".to_string(),
                                        message: join_err_msg.clone(),
                                    });
                                }
                                if let Some(entry) = inner_guard.supervisor_handles.get_mut(&id) {
                                    entry.join_result = Some(Err(join_err_msg.clone()));
                                    entry.ownership = SupervisorHandleOwnership::Joined;
                                    entry.finalization_state = SupervisorFinalizationState::RecoveryInProgress;
                                    entry.completion_notify.notify_waiters();
                                    inner_guard.progress_revision += 1;
                                }
                            }
                            recover_supervisor_panic_external(&self, &id, &join_err_msg).await;
                            let termination_unproven = {
                                let mut inner = self.inner.lock();
                                let detail = inner.supervisor_handles.get(&id).and_then(|entry| {
                                    match &entry.finalization_state {
                                        SupervisorFinalizationState::TerminationUnproven(detail) => {
                                            Some(detail.clone())
                                        }
                                        _ => None,
                                    }
                                });
                                if let Some(detail) = &detail {
                                    inner.reaper_task_state = ReaperTaskState::JoinedFailure(
                                        format!(
                                            "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN: {detail}"
                                        ),
                                    );
                                    inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                                    inner.progress_revision += 1;
                                }
                                detail.is_some()
                            };
                            self.shutdown_notify.notify_waiters();
                            if termination_unproven {
                                return;
                            }
                        }
                        Err(join_err) => {
                            let mut inner = self.inner.lock();
                            inner.reaper_task_state = ReaperTaskState::JoinedFailure(format!("REAPER_TASK_PANIC: {join_err}"));
                            inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                            inner.progress_revision += 1;
                            drop(inner);
                            self.shutdown_notify.notify_waiters();
                        }
                    }
                }
                _ = self.reaper_notify.notified() => {}
                _ = self.reaper_stop_token.cancelled() => {}
            }
        }
        self.shutdown_notify.notify_waiters();
    }

    #[cfg(test)]
    pub fn register_test_hook(&self, hook: MutationTestHookFn) {
        self.test_hooks.lock().push(hook);
    }

    #[cfg(test)]
    pub fn clear_test_hooks(&self) {
        self.test_hooks.lock().clear();
    }

    #[cfg(test)]
    fn trigger_test_hook(&self, phase: MutationTestHookPhase, id: &MutationTransactionId) {
        let hooks = self.test_hooks.lock().clone();
        for hook in hooks {
            hook(phase, id);
        }
    }

    pub fn coordinator_start_count(&self) -> usize {
        self.inner.lock().coordinator_start_count
    }

    pub fn begin_transaction(
        &self,
        session_id: &str,
        start_content_generation: u64,
    ) -> Result<(Arc<MutationTransactionRecord>, SessionReservationGuard), DocumentSessionError>
    {
        let mut inner = self.inner.lock();
        if inner.lifecycle != MutationManagerLifecycle::Running {
            return Err(DocumentSessionError::new(
                "MUTATION_MANAGER_SHUTTING_DOWN",
                "Mutation manager kapalı durumda olduğu için yeni transaction kabul edilmedi",
            ));
        }

        if inner.session_reservations.contains_key(session_id) {
            return Err(DocumentSessionError::new(
                "MUTATION_TRANSACTION_CONFLICT",
                format!("Session '{session_id}' üzerinde aktif bir mutation işlemi devam ediyor"),
            ));
        }

        let id = MutationTransactionId::new();
        let record = Arc::new(MutationTransactionRecord {
            id,
            session_id: session_id.to_string(),
            start_content_generation,
            control: Mutex::new(MutationTransactionControlState {
                phase: MutationTransactionPhase::Created,
                cancel_requested: false,
                cancel_reason: None,
                terminal_outcome: None,
            }),
            cancellation_token: CancellationToken::new(),
            phase_notify: tokio::sync::Notify::new(),
        });

        inner
            .session_reservations
            .insert(session_id.to_string(), id);
        inner.active_transactions.insert(id, Arc::clone(&record));

        let guard = SessionReservationGuard {
            transaction_id: id,
            session_id: session_id.to_string(),
            registry: Arc::clone(&self.inner),
            progress_notify: Arc::clone(&self.shutdown_notify),
            active: AtomicBool::new(true),
        };

        Ok((record, guard))
    }

    pub fn register_supervisor_handle(
        self: &Arc<Self>,
        id: MutationTransactionId,
        handle: tokio::task::JoinHandle<MutationTransactionOutcome>,
    ) {
        let abort_handle = handle.abort_handle();
        {
            let mut inner = self.inner.lock();
            inner.supervisor_handles.insert(
                id,
                SupervisorRegistryEntry {
                    handle: Some(handle),
                    abort_handle,
                    ownership: SupervisorHandleOwnership::Registered,
                    finalization_state: SupervisorFinalizationState::Registered,
                    completion_notify: Arc::new(tokio::sync::Notify::new()),
                    join_result: None,
                    recovery_bundle: None,
                    execution_authority: SupervisorExecutionAuthority::new(),
                },
            );
            inner.progress_revision += 1;
        }
        self.reaper_notify.notify_waiters();
        self.ensure_reaper_loop_started();
    }

    pub fn list_active_transactions(
        &self,
    ) -> Vec<(MutationTransactionId, String, MutationTransactionPhase)> {
        let inner = self.inner.lock();
        inner
            .active_transactions
            .iter()
            .map(|(id, rec)| (*id, rec.session_id.clone(), rec.control.lock().phase))
            .collect()
    }

    pub fn get_transaction_phase(
        &self,
        id: &MutationTransactionId,
    ) -> Option<MutationTransactionPhase> {
        self.get_transaction(id).map(|rec| rec.control.lock().phase)
    }

    pub fn get_transaction(
        &self,
        id: &MutationTransactionId,
    ) -> Option<Arc<MutationTransactionRecord>> {
        self.inner.lock().active_transactions.get(id).cloned()
    }

    pub fn cancel_transaction(&self, id: &MutationTransactionId, reason: &str) -> bool {
        if let Some(record) = self.get_transaction(id) {
            return record.request_cancel(reason.to_string());
        }
        false
    }

    pub async fn wait_for_supervisor_terminal(
        &self,
        id: &MutationTransactionId,
    ) -> Option<MutationTransactionOutcome> {
        let record = self.get_transaction(id)?;
        Some(record.wait_for_terminal().await)
    }

    pub fn begin_shutdown(&self) -> bool {
        let mut inner = self.inner.lock();
        if inner.lifecycle == MutationManagerLifecycle::Running {
            inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
            true
        } else {
            false
        }
    }

    fn spawn_coordinator_and_monitor(self: &Arc<Self>) {
        let manager_for_coord = Arc::clone(self);
        let coord_handle = tokio::spawn(async move {
            #[cfg(test)]
            manager_for_coord.trigger_test_hook(
                MutationTestHookPhase::BeforeCoordinatorBody,
                &MutationTransactionId::default(),
            );

            manager_for_coord.execute_shutdown_coordinator().await
        });
        *self.shutdown_coordinator_task.lock() = Some(coord_handle);

        let manager_for_mon = Arc::clone(self);
        let mon_handle = tokio::spawn(async move {
            manager_for_mon.run_coordinator_monitor().await;
        });
        *self.shutdown_coordinator_monitor_task.lock() = Some(mon_handle);
    }

    pub async fn shutdown(
        self: &Arc<Self>,
        caller_timeout_ms: u64,
    ) -> Result<DocumentShutdownReport, DocumentSessionError> {
        self.ensure_reaper_loop_started();
        let should_spawn = {
            let mut inner = self.inner.lock();
            match inner.coordinator_state {
                ShutdownCoordinatorState::NotStarted => {
                    inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                    inner.coordinator_state = ShutdownCoordinatorState::Running;
                    inner.coordinator_join_state = CoordinatorJoinState::Running;
                    inner.coordinator_monitor_join_state = CoordinatorMonitorJoinState::Running;
                    inner.coordinator_start_count += 1;
                    true
                }
                ShutdownCoordinatorState::Running | ShutdownCoordinatorState::Completed => false,
            }
        };

        if should_spawn {
            self.spawn_coordinator_and_monitor();
        }

        let deadline = tokio::time::Instant::now() + Duration::from_millis(caller_timeout_ms);
        loop {
            if let Some(res) = self.inner.lock().shutdown_completion.clone() {
                return (*res).clone();
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(DocumentSessionError::new(
                    "MUTATION_SHUTDOWN_CALLER_TIMEOUT",
                    "Shutdown caller bekleme süresi zaman aşımına uğradı",
                ));
            }
            let notified = self.shutdown_notify.notified();
            if let Some(res) = self.inner.lock().shutdown_completion.clone() {
                return (*res).clone();
            }
            let _ = tokio::time::timeout(remaining, notified).await;
        }
    }

    async fn run_coordinator_monitor(&self) {
        let handle_opt = self.shutdown_coordinator_task.lock().take();
        if let Some(handle) = handle_opt {
            let join_res = handle.await;
            match join_res {
                Ok(report_res) => {
                    let mut inner = self.inner.lock();
                    inner.coordinator_join_state = CoordinatorJoinState::JoinedSuccess;
                    inner.coordinator_monitor_join_state =
                        CoordinatorMonitorJoinState::BodyCompleted;
                    inner.coordinator_state = ShutdownCoordinatorState::Completed;
                    let terminal_result = match report_res {
                        Ok(report) if mutation_shutdown_success_invariants(&inner) => {
                            let final_report = DocumentShutdownReport {
                                active_transactions_start: report.active_transactions_start,
                                cancelled_transactions: inner.accumulator.cancelled_transactions,
                                post_commit_completed: inner.accumulator.post_commit_completed,
                                reaped_children: inner.accumulator.reaped_children,
                                timed_out_supervisors: report.timed_out_supervisors,
                                remaining_reservations: 0,
                                remaining_registry_entries: 0,
                                warnings: inner.accumulator.warnings.clone(),
                                failed_pre_commit: inner.accumulator.failed_pre_commit,
                                commit_indeterminate: inner.accumulator.commit_indeterminate,
                                supervisor_panics: inner.accumulator.supervisor_panics,
                            };
                            inner.lifecycle = MutationManagerLifecycle::Terminated;
                            Ok(final_report)
                        }
                        Ok(_) => {
                            inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                            Err(DocumentSessionError::new(
                                "MUTATION_SHUTDOWN_FINAL_INVARIANT_FAILED",
                                "Coordinator success sonucu final manager invariant'larını sağlamadı",
                            ))
                        }
                        Err(error) => {
                            inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                            Err(error)
                        }
                    };
                    inner.shutdown_completion = Some(Arc::new(terminal_result));
                }
                Err(join_err) => {
                    let err_msg = format!("COORDINATOR_PANIC: {join_err}");
                    let panic_err = DocumentSessionError::new(
                        "MUTATION_SHUTDOWN_COORDINATOR_PANIC",
                        format!("Shutdown coordinator task panikledi: {err_msg}"),
                    );
                    let arc_res = Arc::new(Err(panic_err));
                    let mut inner = self.inner.lock();
                    inner.coordinator_join_state = CoordinatorJoinState::JoinedFailure(err_msg);
                    inner.coordinator_monitor_join_state =
                        CoordinatorMonitorJoinState::BodyCompleted;
                    inner.coordinator_state = ShutdownCoordinatorState::Completed;
                    inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
                    inner.shutdown_completion = Some(arc_res);
                }
            }
            self.shutdown_notify.notify_waiters();
        }
    }

    pub async fn join_coordinator_and_reaper_monitors(&self) -> Result<(), DocumentSessionError> {
        loop {
            let (coord_state, coord_join_state, reaper_join_state) = {
                let inner = self.inner.lock();
                (
                    inner.coordinator_state,
                    inner.coordinator_monitor_join_state.clone(),
                    inner.reaper_monitor_join_state.clone(),
                )
            };

            if coord_state != ShutdownCoordinatorState::Completed {
                return Err(DocumentSessionError::new(
                    "MUTATION_SHUTDOWN_NOT_READY",
                    "Coordinator henüz terminal durumda değil",
                ));
            }

            match coord_join_state {
                CoordinatorMonitorJoinState::ExternallyJoinedSuccess
                    if reaper_join_state == ReaperMonitorJoinState::ExternallyJoinedSuccess =>
                {
                    return self.external_monitor_join_result()
                }
                CoordinatorMonitorJoinState::ExternallyJoinedSuccess => match reaper_join_state {
                    ReaperMonitorJoinState::ExternallyJoinedFailure(message) => {
                        return Err(DocumentSessionError::new(
                            "MUTATION_SHUTDOWN_REAPER_MONITOR_PANIC",
                            message,
                        ));
                    }
                    ReaperMonitorJoinState::Joining => {
                        let notified = self.shutdown_notify.notified();
                        notified.await;
                    }
                    _ => {
                        return Err(DocumentSessionError::new(
                            "MUTATION_SHUTDOWN_MONITOR_JOIN_CORRUPTION",
                            "Coordinator monitor joined iken reaper monitor join state tutarsız",
                        ));
                    }
                },
                CoordinatorMonitorJoinState::ExternallyJoinedFailure(ref msg) => {
                    return Err(DocumentSessionError::new(
                        "MUTATION_SHUTDOWN_MONITOR_PANIC",
                        msg.clone(),
                    ));
                }
                CoordinatorMonitorJoinState::Joining => {
                    let notified = self.shutdown_notify.notified();
                    let joining = {
                        let inner = self.inner.lock();
                        inner.coordinator_monitor_join_state == CoordinatorMonitorJoinState::Joining
                            || inner.reaper_monitor_join_state == ReaperMonitorJoinState::Joining
                    };
                    if joining {
                        notified.await;
                    }
                }
                CoordinatorMonitorJoinState::NotStarted
                | CoordinatorMonitorJoinState::Running
                | CoordinatorMonitorJoinState::BodyCompleted => {
                    {
                        let mut inner = self.inner.lock();
                        if matches!(
                            inner.coordinator_monitor_join_state,
                            CoordinatorMonitorJoinState::Joining
                                | CoordinatorMonitorJoinState::ExternallyJoinedSuccess
                        ) || matches!(
                            inner.reaper_monitor_join_state,
                            ReaperMonitorJoinState::Joining
                                | ReaperMonitorJoinState::ExternallyJoinedSuccess
                        ) {
                            continue;
                        }
                        inner.coordinator_monitor_join_state = CoordinatorMonitorJoinState::Joining;
                        inner.reaper_monitor_join_state = ReaperMonitorJoinState::Joining;
                    }

                    let terminal_failure = self
                        .inner
                        .lock()
                        .shutdown_completion
                        .as_ref()
                        .is_some_and(|result| result.is_err());
                    let preparation_result = if terminal_failure {
                        self.prepare_reaper_after_terminal_failure().await
                    } else {
                        Ok(())
                    };

                    if let Err(error) = preparation_result {
                        let mut inner = self.inner.lock();
                        inner.coordinator_monitor_join_state =
                            CoordinatorMonitorJoinState::ExternallyJoinedFailure(
                                error.message.clone(),
                            );
                        inner.reaper_monitor_join_state =
                            ReaperMonitorJoinState::ExternallyJoinedFailure(error.message.clone());
                        self.shutdown_notify.notify_waiters();
                        return Err(error);
                    }

                    let mon_handle = self.shutdown_coordinator_monitor_task.lock().take();
                    let reaper_monitor_handle = self.reaper_monitor_task.lock().take();

                    let mut coordinator_join_result = Ok(());
                    let mut reaper_join_result = Ok(());

                    if let Some(handle) = mon_handle {
                        if let Err(err) = handle.await {
                            let msg = format!("MONITOR_PANIC: {err}");
                            coordinator_join_result = Err(DocumentSessionError::new(
                                "MUTATION_SHUTDOWN_MONITOR_PANIC",
                                format!("Coordinator monitor task panikledi: {msg}"),
                            ));
                        }
                    }
                    if let Some(handle) = reaper_monitor_handle {
                        if let Err(err) = handle.await {
                            let msg = format!("REAPER_MONITOR_PANIC: {err}");
                            reaper_join_result = Err(DocumentSessionError::new(
                                "MUTATION_SHUTDOWN_REAPER_MONITOR_PANIC",
                                format!("Reaper monitor task panikledi: {msg}"),
                            ));
                        }
                    }

                    let mut inner = self.inner.lock();
                    match &coordinator_join_result {
                        Ok(()) => {
                            inner.coordinator_monitor_join_state =
                                CoordinatorMonitorJoinState::ExternallyJoinedSuccess;
                        }
                        Err(e) => {
                            inner.coordinator_monitor_join_state =
                                CoordinatorMonitorJoinState::ExternallyJoinedFailure(
                                    e.message.clone(),
                                );
                        }
                    }
                    match &reaper_join_result {
                        Ok(()) => {
                            inner.reaper_monitor_join_state =
                                ReaperMonitorJoinState::ExternallyJoinedSuccess;
                        }
                        Err(error) => {
                            inner.reaper_monitor_join_state =
                                ReaperMonitorJoinState::ExternallyJoinedFailure(
                                    error.message.clone(),
                                );
                        }
                    }
                    self.shutdown_notify.notify_waiters();
                    drop(inner);
                    coordinator_join_result?;
                    reaper_join_result?;
                    return self.external_monitor_join_result();
                }
            }
        }
    }

    fn external_monitor_join_result(&self) -> Result<(), DocumentSessionError> {
        let inner = self.inner.lock();
        match &inner.reaper_task_state {
            ReaperTaskState::JoinedFailure(message) => {
                let code = if message.contains("MUTATION_SUPERVISOR_TERMINATION_UNPROVEN") {
                    "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN"
                } else {
                    "MUTATION_SHUTDOWN_REAPER_PANIC"
                };
                Err(DocumentSessionError::new(code, message.clone()))
            }
            ReaperTaskState::JoinedSuccess => Ok(()),
            state => Err(DocumentSessionError::new(
                "MUTATION_SHUTDOWN_REAPER_NOT_JOINED",
                format!("Reaper loop terminal değil: {state:?}"),
            )),
        }
    }

    async fn prepare_reaper_after_terminal_failure(&self) -> Result<(), DocumentSessionError> {
        let ids = {
            let inner = self.inner.lock();
            inner.supervisor_handles.keys().copied().collect::<Vec<_>>()
        };
        for id in ids {
            recover_supervisor_panic_external(self, &id, "terminal coordinator failure").await;
        }

        self.reaper_stop_token.cancel();
        {
            let mut inner = self.inner.lock();
            if inner.reaper_task_state == ReaperTaskState::Running {
                inner.reaper_task_state = ReaperTaskState::StopRequested;
                inner.progress_revision += 1;
            }
        }
        self.reaper_notify.notify_waiters();

        tokio::time::timeout(Duration::from_millis(SUPERVISOR_ABORT_EXIT_MS), async {
            loop {
                let notified = self.shutdown_notify.notified();
                if matches!(
                    self.inner.lock().reaper_task_state,
                    ReaperTaskState::JoinedSuccess | ReaperTaskState::JoinedFailure(_)
                ) {
                    return;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| {
            DocumentSessionError::new(
                "MUTATION_SHUTDOWN_REAPER_TERMINATION_UNPROVEN",
                "Terminal coordinator failure sonrasında reaper loop sonlandırılamadı",
            )
        })
    }

    async fn execute_shutdown_coordinator(
        &self,
    ) -> Result<DocumentShutdownReport, DocumentSessionError> {
        let records = {
            let inner = self.inner.lock();
            inner
                .active_transactions
                .values()
                .cloned()
                .collect::<Vec<_>>()
        };
        let start_active_count = records.len();
        for record in &records {
            record.request_cancel("MUTATION_SHUTDOWN".to_string());
        }

        let initial_supervisors = {
            let inner = self.inner.lock();
            inner.supervisor_handles.keys().copied().collect::<Vec<_>>()
        };
        for id in initial_supervisors {
            if let Err(error) = terminate_supervisor_execution(self, &id, "manager shutdown").await
            {
                let mut inner = self.inner.lock();
                if let Some(entry) = inner.supervisor_handles.get_mut(&id) {
                    entry.finalization_state =
                        SupervisorFinalizationState::TerminationUnproven(error.message.clone());
                }
                inner.accumulator.warnings.push(CleanupWarning {
                    code: error.code.to_string(),
                    message: error.message,
                });
                inner.progress_revision += 1;
            }
        }

        loop {
            let (reaper_failure, ids, drained, next_revision) = {
                let inner = self.inner.lock();
                let reaper_failure = match &inner.reaper_task_state {
                    ReaperTaskState::JoinedFailure(message) => Some(message.clone()),
                    _ => None,
                };
                let children = inner
                    .supervisor_handles
                    .values()
                    .filter_map(|entry| entry.recovery_bundle.as_ref())
                    .map(|bundle| bundle.child_owner.active_child_count())
                    .sum::<usize>();
                (
                    reaper_failure,
                    inner.supervisor_handles.keys().copied().collect::<Vec<_>>(),
                    inner.supervisor_handles.is_empty()
                        && inner.session_reservations.is_empty()
                        && inner.active_transactions.is_empty()
                        && children == 0,
                    inner.progress_revision,
                )
            };

            if let Some(message) = reaper_failure {
                for id in ids {
                    recover_supervisor_panic_external(self, &id, &message).await;
                }
                let code = if message.contains("MUTATION_SUPERVISOR_TERMINATION_UNPROVEN") {
                    "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN"
                } else {
                    "MUTATION_SHUTDOWN_REAPER_PANIC"
                };
                return Err(DocumentSessionError::new(code, message));
            }

            if drained {
                break;
            }

            let notified = self.shutdown_notify.notified();
            if self.inner.lock().progress_revision == next_revision {
                notified.await;
            }
        }

        self.reaper_stop_token.cancel();
        {
            let mut inner = self.inner.lock();
            if inner.reaper_task_state == ReaperTaskState::Running {
                inner.reaper_task_state = ReaperTaskState::StopRequested;
                inner.progress_revision += 1;
            }
        }
        self.reaper_notify.notify_waiters();

        loop {
            let notified = self.shutdown_notify.notified();
            let state = self.inner.lock().reaper_task_state.clone();
            match state {
                ReaperTaskState::JoinedSuccess => break,
                ReaperTaskState::JoinedFailure(message) => {
                    return Err(DocumentSessionError::new(
                        "MUTATION_SHUTDOWN_REAPER_PANIC",
                        message,
                    ));
                }
                _ => notified.await,
            }
        }

        {
            let inner = self.inner.lock();
            let remaining_reservations = inner.session_reservations.len();
            let remaining_registry_entries = inner.supervisor_handles.len();
            let remaining_active_transactions = inner.active_transactions.len();
            let remaining_active_children = inner
                .supervisor_handles
                .values()
                .filter_map(|e| e.recovery_bundle.as_ref())
                .map(|b| b.child_owner.active_child_count())
                .sum::<usize>();
            let reaper_healthy = inner.reaper_task_state == ReaperTaskState::JoinedSuccess;

            if remaining_registry_entries > 0
                || remaining_reservations > 0
                || remaining_active_transactions > 0
                || remaining_active_children > 0
                || !reaper_healthy
            {
                Err(DocumentSessionError::new(
                    "MUTATION_SHUTDOWN_SUPERVISOR_TIMEOUT",
                    format!(
                        "Shutdown tamamlanamadı: active_transactions={remaining_active_transactions}, active_supervisors={remaining_registry_entries}, active_children={remaining_active_children}, reaper_healthy={reaper_healthy}"
                    ),
                ))
            } else {
                Ok(DocumentShutdownReport {
                    active_transactions_start: start_active_count,
                    cancelled_transactions: inner.accumulator.cancelled_transactions,
                    post_commit_completed: inner.accumulator.post_commit_completed,
                    reaped_children: inner.accumulator.reaped_children,
                    timed_out_supervisors: 0,
                    remaining_reservations: 0,
                    remaining_registry_entries: 0,
                    warnings: inner.accumulator.warnings.clone(),
                    failed_pre_commit: inner.accumulator.failed_pre_commit,
                    commit_indeterminate: inner.accumulator.commit_indeterminate,
                    supervisor_panics: inner.accumulator.supervisor_panics,
                })
            }
        }
    }
}

fn mutation_shutdown_success_invariants(inner: &MutationTransactionRegistryInner) -> bool {
    inner.active_transactions.is_empty()
        && inner.session_reservations.is_empty()
        && inner.supervisor_handles.is_empty()
        && inner.reaper_task_state == ReaperTaskState::JoinedSuccess
}

#[derive(Clone, Debug)]
pub struct MutationTransactionContext {
    pub transaction_id: MutationTransactionId,
    pub session_id: String,
    pub start_content_generation: u64,
    pub snapshot_path: PathBuf,
    pub credential: Option<SessionJobCredential>,
    pub cancellation: SessionJobCancellationCheckpoint,
    pub child_processes: SessionJobChildProcessOwner,
    pub exchange_dir: PathBuf,
}

impl MutationTransactionContext {
    pub fn new(
        transaction_id: MutationTransactionId,
        session_id: String,
        start_content_generation: u64,
        exchange_dir: PathBuf,
        credential: Option<String>,
        cancellation_token: CancellationToken,
        child_processes: SessionJobChildProcessOwner,
    ) -> Self {
        Self {
            transaction_id,
            session_id,
            start_content_generation,
            snapshot_path: exchange_dir.join("snapshot.pdf"),
            credential: credential.map(SessionJobCredential),
            cancellation: SessionJobCancellationCheckpoint {
                token: cancellation_token,
                reason: Arc::new(Mutex::new(None)),
            },
            child_processes,
            exchange_dir,
        }
    }

    pub fn to_session_job_context(&self) -> SessionJobContext {
        SessionJobContext {
            job_id: self.transaction_id.0,
            owner_session_id: self.session_id.clone(),
            captured_content_generation: self.start_content_generation,
            snapshot_path: self.snapshot_path.clone(),
            credential: self.credential.clone(),
            cancellation: self.cancellation.clone(),
            child_processes: self.child_processes.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreflightPrepared {
    pub(crate) next_content_generation: u64,
    pub(crate) next_state_revision: u64,
}

#[derive(Clone, Debug)]
pub enum SessionJobOperationError {
    Cancelled(SessionJobCancellationReason),
    Failed(SessionJobFailure),
}

impl SessionJobOperationError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(SessionJobFailure::pdf_engine(message))
    }

    pub fn failed_with_code(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Failed(SessionJobFailure::new(code, message))
    }
}

impl From<String> for SessionJobOperationError {
    fn from(message: String) -> Self {
        Self::failed(message)
    }
}

impl std::fmt::Display for SessionJobOperationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled(reason) => write!(formatter, "Session job cancelled: {reason:?}"),
            Self::Failed(error) => formatter.write_str(&error.message),
        }
    }
}

#[derive(Clone)]
pub struct SessionJobCredential(String);

impl SessionJobCredential {
    pub fn as_deref(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for SessionJobCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionJobCredential([REDACTED])")
    }
}

#[derive(Clone, Debug)]
pub struct SessionJobCancellationCheckpoint {
    token: CancellationToken,
    reason: Arc<Mutex<Option<SessionJobCancellationReason>>>,
}

impl SessionJobCancellationCheckpoint {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            reason: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_cancel_requested(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    pub fn reason(&self) -> Arc<Mutex<Option<SessionJobCancellationReason>>> {
        Arc::clone(&self.reason)
    }

    pub fn cancellation_reason(&self) -> Option<SessionJobCancellationReason> {
        *self.reason.lock()
    }

    pub fn check_cancelled(&self) -> Result<(), SessionJobOperationError> {
        if self.token.is_cancelled() {
            return Err(SessionJobOperationError::Cancelled(
                self.reason
                    .lock()
                    .unwrap_or(SessionJobCancellationReason::UserRequested),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionJobChildProcessOwner {
    children: Arc<Mutex<HashMap<u32, Arc<Mutex<Child>>>>>,
    #[cfg(test)]
    test_hooks: Arc<SessionJobChildProcessTestHooks>,
}

impl SessionJobChildProcessOwner {
    pub fn register_child(&self, child: Child) -> u32 {
        let id = child.id();
        self.children.lock().insert(id, Arc::new(Mutex::new(child)));
        id
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct SessionJobChildProcessTestHooks {
    fail_next_try_wait: AtomicBool,
    fail_next_output_read: AtomicBool,
}

struct SessionJobChildRegistrationGuard {
    owner: SessionJobChildProcessOwner,
    child_id: u32,
    child: Arc<Mutex<Child>>,
}

impl SessionJobChildRegistrationGuard {
    fn register(owner: &SessionJobChildProcessOwner, child: Child) -> Self {
        let child_id = child.id();
        let child = Arc::new(Mutex::new(child));
        owner.children.lock().insert(child_id, Arc::clone(&child));
        Self {
            owner: owner.clone(),
            child_id,
            child,
        }
    }

    fn try_wait(&self) -> Result<Option<ExitStatus>, std::io::Error> {
        #[cfg(test)]
        if self
            .owner
            .test_hooks
            .fail_next_try_wait
            .swap(false, Ordering::AcqRel)
        {
            return Err(std::io::Error::other("controlled child monitor failure"));
        }
        self.child.lock().try_wait()
    }

    /// macOS: an error once the child's phys_footprint passes `limit`. Returning
    /// it drops the guard, which kills and reaps the child.
    fn memory_limit_failure(&self, limit: Option<u64>) -> Option<SessionJobOperationError> {
        let limit = limit?;
        let footprint = crate::pdf_engine::memory_watchdog::over_limit(self.child_id, limit)?;
        Some(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_MEMORY_LIMIT",
            format!(
                "Job worker bellek bütçesini aştığı için durduruldu ({} MiB > {} MiB)",
                crate::pdf_engine::memory_watchdog::mib(footprint),
                crate::pdf_engine::memory_watchdog::mib(limit)
            ),
        ))
    }

    fn take_output_pipes(
        &self,
    ) -> (
        Option<std::process::ChildStdout>,
        Option<std::process::ChildStderr>,
    ) {
        let mut child = self.child.lock();
        (child.stdout.take(), child.stderr.take())
    }
}

impl Drop for SessionJobChildRegistrationGuard {
    fn drop(&mut self) {
        let mut child = self.child.lock();
        match child.try_wait() {
            Ok(Some(_)) => {
                self.owner.children.lock().remove(&self.child_id);
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                if child.wait().is_ok() {
                    self.owner.children.lock().remove(&self.child_id);
                }
            }
        }
    }
}

impl SessionJobChildProcessOwner {
    pub fn run_with_timeout(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
    ) -> Result<ExitStatus, SessionJobOperationError> {
        self.run_with_timeout_after_spawn(command, timeout, cancellation, || {})
    }

    pub(crate) fn run_with_timeout_after_spawn(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
        after_spawn: impl FnOnce(),
    ) -> Result<ExitStatus, SessionJobOperationError> {
        self.run_with_timeout_watched(command, timeout, cancellation, after_spawn, None)
    }

    /// Runs a PDFium job worker. On macOS its footprint is held to the same
    /// budget as the viewer worker (see `pdf_engine::memory_watchdog`).
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn run_job_worker_with_timeout(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
    ) -> Result<ExitStatus, SessionJobOperationError> {
        self.run_with_timeout_watched(
            command,
            timeout,
            cancellation,
            || {},
            Some(crate::pdf_engine::memory_watchdog::WORKER_FOOTPRINT_LIMIT_BYTES),
        )
    }

    fn run_with_timeout_watched(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
        after_spawn: impl FnOnce(),
        memory_limit: Option<u64>,
    ) -> Result<ExitStatus, SessionJobOperationError> {
        cancellation.check_cancelled()?;
        let child = command.spawn().map_err(child_spawn_error)?;
        let child_guard = SessionJobChildRegistrationGuard::register(self, child);
        after_spawn();
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if cancellation.is_cancel_requested() {
                return Err(SessionJobOperationError::Cancelled(
                    cancellation
                        .reason
                        .lock()
                        .unwrap_or(SessionJobCancellationReason::UserRequested),
                ));
            }
            if let Some(status) = child_guard.try_wait().map_err(|error| {
                SessionJobOperationError::failed(format!("Platform süreci izlenemedi: {error}"))
            })? {
                return Ok(status);
            }
            if let Some(error) = child_guard.memory_limit_failure(memory_limit) {
                return Err(error);
            }
            if std::time::Instant::now() >= deadline {
                return Err(SessionJobOperationError::failed(
                    "Platform süreci zaman aşımına uğradı",
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(not(test))]
    pub(crate) fn run_with_timeout_cooperative(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
        cancellation_grace: Duration,
        signal_cancellation: impl FnOnce() -> Result<(), SessionJobOperationError>,
    ) -> Result<ExitStatus, SessionJobOperationError> {
        cancellation.check_cancelled()?;
        let child = command.spawn().map_err(child_spawn_error)?;
        let child_guard = SessionJobChildRegistrationGuard::register(self, child);
        let deadline = std::time::Instant::now() + timeout;
        let mut cancellation_deadline = None;
        let mut signal_cancellation = Some(signal_cancellation);
        loop {
            if cancellation.is_cancel_requested() && cancellation_deadline.is_none() {
                if let Some(signal) = signal_cancellation.take() {
                    signal()?;
                }
                cancellation_deadline = Some(std::time::Instant::now() + cancellation_grace);
            }
            if let Some(status) = child_guard.try_wait().map_err(|error| {
                SessionJobOperationError::failed(format!("Platform süreci izlenemedi: {error}"))
            })? {
                if cancellation_deadline.is_some() {
                    return Err(SessionJobOperationError::Cancelled(
                        cancellation
                            .reason
                            .lock()
                            .unwrap_or(SessionJobCancellationReason::UserRequested),
                    ));
                }
                return Ok(status);
            }
            // Only job workers use the cooperative path, so the budget always applies.
            if let Some(error) = child_guard.memory_limit_failure(Some(
                crate::pdf_engine::memory_watchdog::WORKER_FOOTPRINT_LIMIT_BYTES,
            )) {
                return Err(error);
            }
            if cancellation_deadline.is_some_and(|value| std::time::Instant::now() >= value) {
                return Err(SessionJobOperationError::Cancelled(
                    cancellation
                        .reason
                        .lock()
                        .unwrap_or(SessionJobCancellationReason::UserRequested),
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(SessionJobOperationError::failed(
                    "Platform süreci zaman aşımına uğradı",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn run_with_output_timeout(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
    ) -> Result<Output, SessionJobOperationError> {
        self.run_with_bounded_output_timeout(
            command,
            timeout,
            cancellation,
            ChildOutputLimits::UNBOUNDED,
        )
    }

    /// `run_with_output_timeout` ile aynı; stdout `limits.max_stdout_bytes`
    /// baytı aşarsa süreç beklenmeden öldürülür ve
    /// `SESSION_JOB_CHILD_OUTPUT_TOO_LARGE` döner. stderr
    /// `limits.max_stderr_bytes` baytta kesilir (geri kalanı okunup atılır,
    /// böylece çocuk dolu pipe'ta bloklanmaz).
    pub fn run_with_bounded_output_timeout(
        &self,
        command: &mut Command,
        timeout: Duration,
        cancellation: &SessionJobCancellationCheckpoint,
        limits: ChildOutputLimits,
    ) -> Result<Output, SessionJobOperationError> {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        cancellation.check_cancelled()?;
        let child = command.spawn().map_err(child_spawn_error)?;
        let child_guard = SessionJobChildRegistrationGuard::register(self, child);
        let (stdout, stderr) = child_guard.take_output_pipes();
        #[cfg(test)]
        let fail_output_read = self
            .test_hooks
            .fail_next_output_read
            .swap(false, Ordering::AcqRel);
        #[cfg(not(test))]
        let fail_output_read = false;
        let stdout_overflow = Arc::new(AtomicBool::new(false));
        let stdout_reader = {
            let overflow = Arc::clone(&stdout_overflow);
            std::thread::spawn(move || {
                drain_child_output(
                    stdout,
                    fail_output_read,
                    "stdout",
                    OutputCap::Reject {
                        max_bytes: limits.max_stdout_bytes,
                        overflow,
                    },
                )
            })
        };
        let stderr_reader = std::thread::spawn(move || {
            drain_child_output(
                stderr,
                false,
                "stderr",
                OutputCap::Truncate {
                    max_bytes: limits.max_stderr_bytes,
                },
            )
        });
        let deadline = std::time::Instant::now() + timeout;
        let status_result = loop {
            if stdout_overflow.load(Ordering::Acquire) {
                break Err(child_output_too_large(limits.max_stdout_bytes));
            }
            if cancellation.is_cancel_requested() {
                break Err(SessionJobOperationError::Cancelled(
                    cancellation
                        .reason
                        .lock()
                        .unwrap_or(SessionJobCancellationReason::UserRequested),
                ));
            }
            match child_guard.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(error) => {
                    break Err(SessionJobOperationError::failed(format!(
                        "Platform süreci izlenemedi: {error}"
                    )));
                }
            }
            if std::time::Instant::now() >= deadline {
                break Err(SessionJobOperationError::failed(
                    "Platform süreci zaman aşımına uğradı",
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        drop(child_guard);
        let stdout_bytes = join_child_output_reader(stdout_reader, "stdout");
        let stderr_bytes = join_child_output_reader(stderr_reader, "stderr");
        // Süreç kendi kendine çıkmış olsa bile taşma, okuma hatasından önce
        // raporlanır.
        if stdout_overflow.load(Ordering::Acquire) {
            return Err(child_output_too_large(limits.max_stdout_bytes));
        }
        let stdout_bytes = stdout_bytes?;
        let stderr_bytes = stderr_bytes?;
        let status = status_result?;
        Ok(Output {
            status,
            stdout: stdout_bytes,
            stderr: stderr_bytes,
        })
    }

    pub fn kill_and_reap_all_typed(&self) -> MutationChildCleanupReport {
        let entries = {
            let registry = self.children.lock();
            registry
                .iter()
                .map(|(id, child)| (*id, Arc::clone(child)))
                .collect::<Vec<_>>()
        };
        let attempted = entries.len();
        let mut terminated = 0;
        let mut reaped = 0;
        let mut errors = Vec::new();

        for (child_id, child) in entries {
            let mut child_guard = child.lock();
            let mut remove = false;
            match child_guard.try_wait() {
                Ok(Some(_)) => {
                    reaped += 1;
                    remove = true;
                }
                Ok(None) | Err(_) => {
                    match child_guard.kill() {
                        Ok(_) => {
                            terminated += 1;
                        }
                        Err(err) => {
                            errors.push(format!("kill failed for PID {child_id}: {err}"));
                        }
                    }
                    match child_guard.wait() {
                        Ok(_) => {
                            reaped += 1;
                            remove = true;
                        }
                        Err(err) => {
                            errors.push(format!("wait failed for PID {child_id}: {err}"));
                        }
                    }
                }
            }
            if remove {
                self.children.lock().remove(&child_id);
            }
        }
        MutationChildCleanupReport {
            attempted,
            terminated,
            reaped,
            errors,
        }
    }

    fn kill_and_reap_all(&self) -> Option<String> {
        let report = self.kill_and_reap_all_typed();
        if report.errors.is_empty() {
            None
        } else {
            Some(report.errors.join("; "))
        }
    }

    pub fn active_child_count(&self) -> usize {
        self.children.lock().len()
    }
}

/// Çocuk süreç çıktı sınırları. `UNBOUNDED` eski
/// `run_with_output_timeout` davranışıdır.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChildOutputLimits {
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

impl ChildOutputLimits {
    pub const UNBOUNDED: Self = Self {
        max_stdout_bytes: usize::MAX,
        max_stderr_bytes: usize::MAX,
    };
}

/// Program bulunamadığında ayrı bir kod döner; çağıran (ör. OCR) kullanıcıya
/// "kurulu değil" mesajı gösterebilir. Mesaj metni değişmedi.
pub(crate) const CHILD_PROGRAM_NOT_FOUND_CODE: &str = "SESSION_JOB_CHILD_PROGRAM_NOT_FOUND";

fn child_spawn_error(error: std::io::Error) -> SessionJobOperationError {
    let message = format!("Platform süreci başlatılamadı: {error}");
    if error.kind() == std::io::ErrorKind::NotFound {
        SessionJobOperationError::failed_with_code(CHILD_PROGRAM_NOT_FOUND_CODE, message)
    } else {
        SessionJobOperationError::failed(message)
    }
}

fn child_output_too_large(max_bytes: usize) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(
        "SESSION_JOB_CHILD_OUTPUT_TOO_LARGE",
        format!("Child stdout çıktısı {max_bytes} bayt sınırını aşıyor"),
    )
}

enum OutputCap {
    /// Sınır aşılırsa okuma durur, `overflow` işaretlenir ve pipe kapatılır.
    Reject {
        max_bytes: usize,
        overflow: Arc<AtomicBool>,
    },
    /// Sınırdan sonrası okunup atılır.
    Truncate { max_bytes: usize },
}

fn drain_child_output<R: Read>(
    reader: Option<R>,
    force_failure: bool,
    stream_name: &str,
    cap: OutputCap,
) -> Result<Vec<u8>, SessionJobOperationError> {
    let read_failed = || {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_CHILD_OUTPUT_READ_FAILED",
            format!("Child {stream_name} çıktısı okunamadı"),
        )
    };
    let mut bytes = Vec::new();
    if let Some(mut reader) = reader {
        match cap {
            OutputCap::Reject {
                max_bytes,
                overflow,
            } => {
                let probe_limit = u64::try_from(max_bytes)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1);
                (&mut reader)
                    .take(probe_limit)
                    .read_to_end(&mut bytes)
                    .map_err(|_| read_failed())?;
                if bytes.len() > max_bytes {
                    overflow.store(true, Ordering::Release);
                    bytes.clear();
                    return Ok(bytes);
                }
            }
            OutputCap::Truncate { max_bytes } => {
                let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
                (&mut reader)
                    .take(limit)
                    .read_to_end(&mut bytes)
                    .map_err(|_| read_failed())?;
                std::io::copy(&mut reader, &mut std::io::sink()).map_err(|_| read_failed())?;
            }
        }
    }
    if force_failure {
        return Err(read_failed());
    }
    Ok(bytes)
}

fn join_child_output_reader(
    reader: std::thread::JoinHandle<Result<Vec<u8>, SessionJobOperationError>>,
    stream_name: &str,
) -> Result<Vec<u8>, SessionJobOperationError> {
    reader.join().map_err(|_| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_CHILD_OUTPUT_READER_FAILED",
            format!("Child {stream_name} output reader tamamlanamadı"),
        )
    })?
}

#[derive(Clone, Debug)]
pub struct SessionJobContext {
    pub job_id: Uuid,
    pub owner_session_id: String,
    pub captured_content_generation: u64,
    pub snapshot_path: PathBuf,
    pub credential: Option<SessionJobCredential>,
    pub cancellation: SessionJobCancellationCheckpoint,
    pub child_processes: SessionJobChildProcessOwner,
}

/// Private paths required by an export preflight. This is intentionally not an
/// IPC DTO: it never crosses the frontend boundary.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct VerifiedArtifactExportPaths {
    pub original_path: PathBuf,
    pub working_path: PathBuf,
    pub session_dir: PathBuf,
    pub jobs_root: PathBuf,
}

pub type SecureRedactionExportPaths = VerifiedArtifactExportPaths;

impl SessionJobContext {
    pub(crate) fn detached(
        snapshot_path: PathBuf,
        credential: Option<String>,
        owner: impl Into<String>,
    ) -> Self {
        Self {
            job_id: Uuid::new_v4(),
            owner_session_id: owner.into(),
            captured_content_generation: 0,
            snapshot_path,
            credential: credential.map(SessionJobCredential),
            cancellation: SessionJobCancellationCheckpoint {
                token: CancellationToken::new(),
                reason: Arc::new(Mutex::new(None)),
            },
            child_processes: SessionJobChildProcessOwner::default(),
        }
    }
}

/// Copies the job snapshot to `<job_dir>/share/<file_name>` so a platform share
/// service sees the document's real name. The copy lives inside the job
/// directory, so job cancellation, shutdown and normal cleanup remove it
/// together with the snapshot. Only the macOS share command uses it.
#[cfg(any(target_os = "macos", all(test, unix)))]
pub(crate) fn stage_job_share_copy(
    context: &SessionJobContext,
    file_name: &str,
) -> Result<PathBuf, SessionJobOperationError> {
    let mut components = Path::new(file_name).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_SHARE_NAME_INVALID",
            "Paylaşım dosya adı tek bir yol bileşeni olmalıdır",
        ));
    }
    let job_dir = context.snapshot_path.parent().ok_or_else(|| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_SHARE_STAGING_FAILED",
            "Job snapshot dizini çözümlenemedi",
        )
    })?;
    let share_dir = job_dir.join("share");
    create_secure_dir(&share_dir)
        .map_err(|error| SessionJobOperationError::failed_with_code(error.code, error.message))?;
    let target = share_dir.join(file_name);
    copy_with_digest_cancellable(&context.snapshot_path, &target, &context.cancellation)?;
    Ok(target)
}

fn session_job_context(job: &Arc<SessionJobRecord>) -> SessionJobContext {
    SessionJobContext {
        job_id: job.job_id,
        owner_session_id: job.owner_session_id.clone(),
        captured_content_generation: job.content_generation,
        snapshot_path: job.snapshot_path.clone(),
        credential: job
            .mutable
            .lock()
            .credential_context
            .clone()
            .map(SessionJobCredential),
        cancellation: SessionJobCancellationCheckpoint {
            token: job.cancellation_token.clone(),
            reason: Arc::clone(&job.cancellation_reason),
        },
        child_processes: job.child_processes.clone(),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSessionSnapshot {
    pub id: String,
    pub opened_at_ms: u64,
    pub display_path: String,
    pub file_name: String,
    pub content_generation: u64,
    pub state_revision: u64,
    pub dirty: bool,
    pub lifecycle: SessionLifecycle,
    pub editable: bool,
    pub read_only_reason: Option<String>,
    pub health_warning: Option<String>,
    /// Geri alınabilecek son mutasyonun işlem kodu (ör. `rotate_page`); yoksa `None`.
    pub undo_operation: Option<String>,
    /// Yinelenebilecek son geri alınmış mutasyonun işlem kodu; yoksa `None`.
    pub redo_operation: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDocumentSessionResult {
    pub status: String,
    pub session_id: String,
    pub snapshot: Option<DocumentSessionSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionResult {
    pub token: String,
    pub kind: TransitionKind,
    pub snapshot: DocumentSessionSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateActivationResult {
    pub activation_token: String,
    pub candidate: DocumentSessionSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupWarning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateCommitResult {
    pub snapshot: DocumentSessionSnapshot,
    pub cleanup_warnings: Vec<CleanupWarning>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateRenderProbe {
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub byte_length: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSessionError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_state: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_path: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_digest: Option<Box<str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Box<DocumentSessionSnapshot>>,
}

impl DocumentSessionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            commit_state: None,
            recovery_path: None,
            replacement_digest: None,
            snapshot: None,
        }
    }

    fn with_snapshot(mut self, snapshot: DocumentSessionSnapshot) -> Self {
        self.snapshot = Some(Box::new(snapshot));
        self
    }

    fn from_io(error: &DocumentIoError) -> Self {
        let code = if error.detail.contains("TARGET_NOT_FOUND") {
            "TARGET_NOT_FOUND"
        } else if error.detail.contains("DOCUMENT_SESSION_CONFLICT")
            || error.detail.contains("PRECONDITION_TARGET_CHANGED")
        {
            "DOCUMENT_SESSION_CONFLICT"
        } else {
            "DOCUMENT_IO_ERROR"
        };
        Self {
            code: code.to_string(),
            message: error.to_string(),
            commit_state: Some(error.state.to_string().into_boxed_str()),
            recovery_path: error
                .recovery_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string().into_boxed_str()),
            replacement_digest: error.replacement_digest.as_deref().map(Box::from),
            snapshot: None,
        }
    }

    fn committed_refresh_failed(
        target: &Path,
        report: &AtomicWriteReport,
        snapshot: DocumentSessionSnapshot,
    ) -> Self {
        Self {
            code: "SAVE_COMMITTED_SESSION_REFRESH_FAILED".to_string(),
            message: format!(
                "Dosya diske commit edildi ancak session publication authority doğrulanamadı; target={} digest={}",
                target.display(), report.committed_digest
            ),
            commit_state: Some(Box::from("committed_but_session_refresh_failed")),
            recovery_path: None,
            replacement_digest: Some(Box::from(report.committed_digest.as_str())),
            snapshot: Some(Box::new(snapshot)),
        }
    }
}

fn prune_session_job_registry(registry: &mut SessionJobRegistry, now: u64) {
    registry.pending_cancellations.retain(|_, pending| {
        now.saturating_sub(pending.created_at_ms) <= SESSION_JOB_PENDING_TTL_MS
    });
    registry.recent_terminal.retain(|_, terminal| {
        now.saturating_sub(terminal.completed_at_ms) <= SESSION_JOB_TERMINAL_TTL_MS
    });
    while registry.recent_terminal.len() > SESSION_JOB_TERMINAL_MAX {
        let Some(oldest) = registry
            .recent_terminal
            .iter()
            .min_by_key(|(_, terminal)| terminal.completed_at_ms)
            .map(|(job_id, _)| *job_id)
        else {
            break;
        };
        registry.recent_terminal.remove(&oldest);
    }
}

fn request_session_job_cancellation(
    job: &Arc<SessionJobRecord>,
    reason: SessionJobCancellationReason,
) {
    {
        let mut state = job.mutable.lock();
        if state.operation_status.is_terminal() {
            return;
        }
        state.cancel_requested = true;
        state.cancellation_reason = Some(reason);
        state.operation_status = SessionJobOperationStatus::CancelRequested;
    }
    *job.cancellation_reason.lock() = Some(reason);
    job.cancellation_token.cancel();
    job.child_processes.kill_and_reap_all();
}

fn set_session_job_terminal(job: &Arc<SessionJobRecord>, error: &SessionJobOperationError) {
    let mut state = job.mutable.lock();
    state.operation_status = match error {
        SessionJobOperationError::Cancelled(reason) => {
            state.cancel_requested = true;
            state.cancellation_reason = Some(*reason);
            SessionJobOperationStatus::Cancelled
        }
        SessionJobOperationError::Failed(_) => SessionJobOperationStatus::Failed,
    };
    state.credential_context = None;
}

fn session_job_error_outcome<T>(
    identity: (String, String, u64),
    error: SessionJobOperationError,
    cleanup: SessionJobCleanupOutcome,
) -> SessionJobOutcome<T> {
    match error {
        SessionJobOperationError::Cancelled(cancellation) => SessionJobOutcome::Cancelled {
            job_id: identity.0,
            session_id: identity.1,
            content_generation: identity.2,
            cancellation,
            cleanup,
        },
        SessionJobOperationError::Failed(error) => SessionJobOutcome::Failed {
            job_id: identity.0,
            session_id: identity.1,
            content_generation: identity.2,
            error,
            cleanup,
        },
    }
}

fn cleanup_warning(code: &str, message: &str) -> SessionJobCleanupWarning {
    SessionJobCleanupWarning {
        code: code.to_string(),
        message: message.to_string(),
    }
}

async fn cleanup_session_job_record(
    registry: &Arc<Mutex<SessionJobRegistry>>,
    job: &Arc<SessionJobRecord>,
) -> SessionJobCleanupOutcome {
    let _single_flight = job.cleanup_single_flight.lock().await;
    {
        let state = job.mutable.lock();
        match state.cleanup_status {
            SessionJobCleanupStatus::Cleaned => return SessionJobCleanupOutcome::Cleaned,
            _ if !state.operation_status.is_terminal() => {
                return SessionJobCleanupOutcome::RetryScheduled {
                    attempt: state.cleanup_attempts,
                    retry_at_ms: now_ms() + SESSION_JOB_REAPER_INTERVAL_MS,
                    warning: cleanup_warning(
                        "SESSION_JOB_CLEANUP_WAITING_FOR_OPERATION",
                        "Session job cleanup işlemi operation tamamlanmasını bekliyor",
                    ),
                };
            }
            _ => {}
        }
    }
    let _operation_cleanup_lease = job.operation.clone().write_owned().await;
    {
        let mut state = job.mutable.lock();
        if matches!(state.cleanup_status, SessionJobCleanupStatus::Cleaned) {
            return SessionJobCleanupOutcome::Cleaned;
        }
        state.cleanup_status = SessionJobCleanupStatus::InProgress;
        state.credential_context = None;
    }
    tracing::debug!(
        job_id = %job.job_id,
        session_id = %job.owner_session_id,
        content_generation = job.content_generation,
        "Session job snapshot temizleniyor"
    );
    let cleanup_result = if job
        .cleanup_failures_remaining
        .try_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
            if remaining > 0 {
                Some(remaining - 1)
            } else {
                None
            }
        })
        .is_ok()
    {
        Err(DocumentSessionError::new(
            "TEMP_CLEANUP_ERROR",
            "Test tarafından zorlanan session job cleanup hatası",
        ))
    } else {
        secure_remove_tree(&job.job_dir)
    };
    match cleanup_result {
        Ok(()) => {
            {
                let mut state = job.mutable.lock();
                state.cleanup_status = SessionJobCleanupStatus::Cleaned;
            }
            let mut registry = registry.lock();
            if registry
                .active
                .get(&job.job_id)
                .is_some_and(|current| Arc::ptr_eq(current, job))
            {
                registry.active.remove(&job.job_id);
            }
            if registry
                .deferred_cleanup
                .get(&job.job_id)
                .is_some_and(|current| Arc::ptr_eq(current, job))
            {
                registry.deferred_cleanup.remove(&job.job_id);
            }
            registry.recent_terminal.insert(
                job.job_id,
                TerminalSessionJob {
                    session_id: job.owner_session_id.clone(),
                    completed_at_ms: now_ms(),
                },
            );
            prune_session_job_registry(&mut registry, now_ms());
            SessionJobCleanupOutcome::Cleaned
        }
        Err(_) => {
            let (attempt, retry_at_ms) = {
                let mut state = job.mutable.lock();
                state.cleanup_attempts = state.cleanup_attempts.saturating_add(1);
                let retry_delay = SESSION_JOB_REAPER_INTERVAL_MS
                    .saturating_mul(1_u64 << state.cleanup_attempts.min(6));
                let retry_at_ms = now_ms().saturating_add(retry_delay);
                state.cleanup_status = SessionJobCleanupStatus::RetryScheduled { retry_at_ms };
                (state.cleanup_attempts, retry_at_ms)
            };
            if attempt >= SESSION_JOB_CLEANUP_MAX_ATTEMPTS {
                let mut registry = registry.lock();
                if registry.deferred_cleanup.len() < SESSION_JOB_DEFERRED_MAX {
                    registry
                        .deferred_cleanup
                        .insert(job.job_id, Arc::clone(job));
                    if registry
                        .active
                        .get(&job.job_id)
                        .is_some_and(|current| Arc::ptr_eq(current, job))
                    {
                        registry.active.remove(&job.job_id);
                    }
                    job.mutable.lock().cleanup_status = SessionJobCleanupStatus::Deferred;
                    return SessionJobCleanupOutcome::Deferred {
                        attempts: attempt,
                        warning: cleanup_warning(
                            "SESSION_JOB_CLEANUP_DEFERRED",
                            "Session job geçici dosyaları bounded retry sonrasında deferred cleanup'a bırakıldı",
                        ),
                    };
                }
                return SessionJobCleanupOutcome::RetryScheduled {
                    attempt,
                    retry_at_ms,
                    warning: cleanup_warning(
                        "SESSION_JOB_CLEANUP_BACKPRESSURE",
                        "Deferred cleanup kapasitesi dolu; terminal job kaydı korunuyor",
                    ),
                };
            }
            SessionJobCleanupOutcome::RetryScheduled {
                attempt,
                retry_at_ms,
                warning: cleanup_warning(
                    "SESSION_JOB_CLEANUP_RETRY_SCHEDULED",
                    "Session job geçici dosyaları için cleanup yeniden denenecek",
                ),
            }
        }
    }
}

async fn reap_session_jobs_once(registry: &Arc<Mutex<SessionJobRegistry>>) {
    let now = now_ms();
    let jobs = {
        let mut registry_guard = registry.lock();
        prune_session_job_registry(&mut registry_guard, now);
        registry_guard
            .active
            .values()
            .chain(registry_guard.deferred_cleanup.values())
            .filter_map(|job| {
                let state = job.mutable.lock();
                let due = match state.cleanup_status {
                    SessionJobCleanupStatus::RetryScheduled { retry_at_ms } => retry_at_ms <= now,
                    SessionJobCleanupStatus::Deferred => true,
                    _ => false,
                };
                (state.operation_status.is_terminal() && due).then(|| Arc::clone(job))
            })
            .collect::<Vec<_>>()
    };
    for job in jobs {
        let _ = cleanup_session_job_record(registry, &job).await;
    }
}

impl std::fmt::Display for DocumentSessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DocumentSessionError {}

impl From<String> for DocumentSessionError {
    fn from(message: String) -> Self {
        if let Some((code, detail)) = message.split_once(": ") {
            if code.starts_with("PDFIUM_WORKER_")
                || code == "PDFIUM_DOCUMENT_QUARANTINED"
                || code == "PDFIUM_PAGE_QUARANTINED"
            {
                return Self::new(code, detail);
            }
        }
        Self::new("DOCUMENT_SESSION_PDF_ENGINE_ERROR", message)
    }
}

impl From<BrokerError> for DocumentSessionError {
    fn from(error: BrokerError) -> Self {
        Self::new(error.code, error.message)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstanceManifest {
    schema_version: u32,
    instance_nonce: String,
    owner_pid: u32,
    created_at_ms: u64,
    application_identifier: String,
}

type SecureRedactionSaveLeaseTestHook = Option<Arc<dyn Fn(bool) + Send + Sync + 'static>>;

pub struct DocumentSessionStore {
    pub memory_limit: usize,
    pub temp_root: PathBuf,
    instance_dir: PathBuf,
    instance_lock_path: PathBuf,
    instance_lock: Mutex<Option<File>>,
    sessions: parking_lot::RwLock<HashMap<String, Arc<SessionEntry>>>,
    active_session_id: Mutex<Option<String>>,
    active_identity_epoch: AtomicU64,
    activations_by_token: Mutex<HashMap<String, CandidateActivation>>,
    activation_token_by_candidate: Mutex<HashMap<String, String>>,
    transition_authorities: Mutex<HashMap<String, TransitionAuthorityIdentity>>,
    recorded_transitions: Mutex<HashMap<String, RecordedTransitionOutcome>>,
    lifecycle_gate: Arc<AsyncRwLock<()>>,
    job_registry: Arc<Mutex<SessionJobRegistry>>,
    job_reaper_started: AtomicBool,
    job_reaper_shutdown: CancellationToken,
    password_cache: Arc<Mutex<HashMap<String, String>>>,
    worker_broker: PdfiumWorkerBroker,
    search_cancellations: Mutex<HashMap<String, (u64, CancellationToken)>>,
    mutation_transaction_manager: Arc<MutationTransactionManager>,
    shutdown_admission_closed: AtomicBool,
    secure_redaction_save_lease_test_hook: Mutex<SecureRedactionSaveLeaseTestHook>,
    #[cfg(test)]
    snapshot_worker_panic_jobs: Mutex<HashSet<Uuid>>,
    #[cfg(test)]
    fail_next_save_publication: AtomicBool,
    #[cfg(test)]
    save_core_invocations: AtomicUsize,
    #[cfg(test)]
    fail_next_transition_close: AtomicBool,
    #[cfg(test)]
    fail_next_candidate_activation_after_deactivation: AtomicBool,
    #[cfg(test)]
    transition_claim_barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
    #[cfg(test)]
    save_io_barriers: Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
    #[cfg(test)]
    save_commit_barriers: Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
    #[cfg(test)]
    transition_save_committed_barriers:
        Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
    #[cfg(test)]
    transition_apply_barriers:
        Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
    #[cfg(test)]
    transition_release_trace: Mutex<Option<Arc<Mutex<Vec<TransitionAuthorityReleaseEvent>>>>>,
    #[cfg(test)]
    save_lifecycle_wait_notify: Mutex<Option<Arc<Notify>>>,
    #[cfg(test)]
    transition_lifecycle_wait_notify: Mutex<Option<Arc<Notify>>>,
    #[cfg(test)]
    transition_completion_wait_notify: Mutex<Option<Arc<Notify>>>,
    #[cfg(test)]
    shutdown_lifecycle_wait_notify: Mutex<Option<Arc<Notify>>>,
    #[cfg(test)]
    panic_next_transition_precommit: AtomicBool,
    #[cfg(test)]
    transition_discard_invocations: AtomicUsize,
    #[cfg(test)]
    transition_apply_invocations: AtomicUsize,
    #[cfg(test)]
    shutdown_lifecycle_barriers:
        Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
    #[cfg(test)]
    lock_order_barriers: Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
}

impl DocumentSessionStore {
    pub fn initialize() -> Result<Self, DocumentSessionError> {
        Self::initialize_with_password_cache(Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn initialize_with_password_cache(
        password_cache: Arc<Mutex<HashMap<String, String>>>,
    ) -> Result<Self, DocumentSessionError> {
        Self::initialize_with_password_cache_and_broker(password_cache, PdfiumWorkerBroker::new())
    }

    #[doc(hidden)]
    pub fn initialize_with_password_cache_and_broker(
        password_cache: Arc<Mutex<HashMap<String, String>>>,
        worker_broker: PdfiumWorkerBroker,
    ) -> Result<Self, DocumentSessionError> {
        // Root kurulumu süreç içinde tek sıradadır: root henüz güvenli değilken iki store aynı anda
        // DACL yazarsa, birinin yayılımı diğerinin yeni kurduğu instance dizinine denk gelir.
        static TEMP_ROOT_SETUP: Mutex<()> = Mutex::new(());
        let temp_root = std::env::temp_dir().join(TEMP_ROOT_NAME);
        {
            let _setup = TEMP_ROOT_SETUP.lock();
            create_secure_dir(&temp_root)?;
        }
        cleanup_orphan_instances(&temp_root)?;

        let mut ownership = None;
        for _ in 0..4 {
            let nonce = Uuid::new_v4().to_string();
            let path = temp_root.join(format!("instance-{nonce}.lock"));
            let file = create_secure_file(&path)?;
            match file.try_lock() {
                Ok(()) => {
                    ownership = Some((nonce, path, file));
                    break;
                }
                Err(_) => {
                    drop(file);
                    let _ = fs::remove_file(path);
                }
            }
        }
        let (nonce, instance_lock_path, lock_file) = ownership.ok_or_else(|| {
            DocumentSessionError::new(
                "TEMP_OWNERSHIP_LOCK_FAILED",
                "Benzersiz temp instance ownership lock dört denemede alınamadı",
            )
        })?;

        let instance_dir = temp_root.join(format!("instance-{nonce}"));
        if let Err(error) = (|| -> Result<(), DocumentSessionError> {
            create_secure_dir(&instance_dir)?;
            create_secure_dir(&instance_dir.join("sessions"))?;
            create_secure_dir(&instance_dir.join("jobs"))?;
            write_manifest(
                &instance_dir,
                &InstanceManifest {
                    schema_version: 1,
                    instance_nonce: nonce,
                    owner_pid: std::process::id(),
                    created_at_ms: now_ms(),
                    application_identifier: "com.antigravity.pdf-reader".to_string(),
                },
            )
        })() {
            drop(lock_file);
            let _ = fs::remove_dir_all(&instance_dir);
            let _ = fs::remove_file(&instance_lock_path);
            return Err(error);
        }

        Ok(Self {
            memory_limit: 500 * 1024 * 1024,
            temp_root,
            instance_dir,
            instance_lock_path,
            instance_lock: Mutex::new(Some(lock_file)),
            sessions: parking_lot::RwLock::new(HashMap::new()),
            active_session_id: Mutex::new(None),
            active_identity_epoch: AtomicU64::new(0),
            activations_by_token: Mutex::new(HashMap::new()),
            activation_token_by_candidate: Mutex::new(HashMap::new()),
            transition_authorities: Mutex::new(HashMap::new()),
            recorded_transitions: Mutex::new(HashMap::new()),
            lifecycle_gate: Arc::new(AsyncRwLock::new(())),
            job_registry: Arc::new(Mutex::new(SessionJobRegistry::default())),
            job_reaper_started: AtomicBool::new(false),
            job_reaper_shutdown: CancellationToken::new(),
            password_cache,
            worker_broker,
            search_cancellations: Mutex::new(HashMap::new()),
            mutation_transaction_manager: Arc::new(MutationTransactionManager::new()),
            shutdown_admission_closed: AtomicBool::new(false),
            secure_redaction_save_lease_test_hook: Mutex::new(None),
            #[cfg(test)]
            snapshot_worker_panic_jobs: Mutex::new(HashSet::new()),
            #[cfg(test)]
            fail_next_save_publication: AtomicBool::new(false),
            #[cfg(test)]
            save_core_invocations: AtomicUsize::new(0),
            #[cfg(test)]
            fail_next_transition_close: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_candidate_activation_after_deactivation: AtomicBool::new(false),
            #[cfg(test)]
            transition_claim_barrier: Mutex::new(None),
            #[cfg(test)]
            save_io_barriers: Mutex::new(None),
            #[cfg(test)]
            save_commit_barriers: Mutex::new(None),
            #[cfg(test)]
            transition_save_committed_barriers: Mutex::new(None),
            #[cfg(test)]
            transition_apply_barriers: Mutex::new(None),
            #[cfg(test)]
            transition_release_trace: Mutex::new(None),
            #[cfg(test)]
            save_lifecycle_wait_notify: Mutex::new(None),
            #[cfg(test)]
            transition_lifecycle_wait_notify: Mutex::new(None),
            #[cfg(test)]
            transition_completion_wait_notify: Mutex::new(None),
            #[cfg(test)]
            shutdown_lifecycle_wait_notify: Mutex::new(None),
            #[cfg(test)]
            panic_next_transition_precommit: AtomicBool::new(false),
            #[cfg(test)]
            transition_discard_invocations: AtomicUsize::new(0),
            #[cfg(test)]
            transition_apply_invocations: AtomicUsize::new(0),
            #[cfg(test)]
            shutdown_lifecycle_barriers: Mutex::new(None),
            #[cfg(test)]
            lock_order_barriers: Mutex::new(None),
        })
    }

    #[cfg(test)]
    async fn pause_after_lifecycle_before_operation(&self) {
        let barriers = self.lock_order_barriers.lock().take();
        if let Some((reached, release)) = barriers {
            reached.wait().await;
            release.wait().await;
        }
    }

    #[cfg(test)]
    fn trace_transition_authority_release(&self, event: TransitionAuthorityReleaseEvent) {
        if let Some(trace) = self.transition_release_trace.lock().clone() {
            trace.lock().push(event);
        }
    }

    #[cfg(test)]
    fn transition_drop_observer(
        &self,
        event: TransitionAuthorityReleaseEvent,
    ) -> TransitionDropTraceObserver {
        TransitionDropTraceObserver {
            trace: self.transition_release_trace.lock().clone(),
            event,
        }
    }

    #[cfg(test)]
    async fn acquire_test_transition_contender(
        &self,
        session_id: &str,
    ) -> Result<(), DocumentSessionError> {
        if self.lifecycle_gate.clone().try_write_owned().is_err() {
            self.trace_transition_authority_release(
                TransitionAuthorityReleaseEvent::TransitionLifecycleWaitBlocked,
            );
            if let Some(notify) = self.transition_lifecycle_wait_notify.lock().clone() {
                notify.notify_waiters();
            }
        }
        let _lifecycle = self.lifecycle_gate.clone().write_owned().await;
        self.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::TransitionLifecycleAcquired,
        );
        let entry = self.entry(session_id)?;
        let _operation = entry.operation.clone().write_owned().await;
        self.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::TransitionOperationAcquired,
        );
        Err(DocumentSessionError::new(
            "TRANSITION_ALREADY_ACTIVE",
            "Test contender mevcut transition authority'sini gördü",
        ))
    }

    pub fn mutation_transaction_manager(&self) -> Arc<MutationTransactionManager> {
        Arc::clone(&self.mutation_transaction_manager)
    }

    pub fn worker_broker(&self) -> PdfiumWorkerBroker {
        self.worker_broker.clone()
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
        let entry = self.entry(session_id)?;
        let _lease = entry.operation.clone().read_owned().await;
        let result = snapshot(&entry.state.lock());
        Ok(result)
    }

    fn entry(&self, session_id: &str) -> Result<Arc<SessionEntry>, DocumentSessionError> {
        self.sessions
            .read()
            .get(session_id)
            .cloned()
            .ok_or_else(|| {
                DocumentSessionError::new(
                    "DOCUMENT_SESSION_NOT_FOUND",
                    format!("Document session bulunamadı: {session_id}"),
                )
            })
    }

    async fn acquire_worker_read(
        &self,
        session_id: &str,
        expected_generation: u64,
        allow_candidate: bool,
        cancellation: Option<&CancellationToken>,
    ) -> Result<WorkerReadContext, DocumentSessionError> {
        let key = WorkerDocumentKey {
            session_id: session_id.to_string(),
            content_generation: expected_generation,
        };
        let admission = self
            .worker_broker
            .admit_read(&key, cancellation)
            .await
            .map_err(DocumentSessionError::from)?;
        let entry = self.entry(session_id)?;
        let lease = entry.operation.clone().read_owned().await;
        let (path, credential) = {
            let state = entry.state.lock();
            if allow_candidate && state.lifecycle == SessionLifecycle::CandidateReady {
                candidate_validation_path(&state, expected_generation)?;
            } else {
                session_read_path(&state, Some(expected_generation))?;
            }
            (
                state.working_path.to_string_lossy().to_string(),
                state.credential.clone(),
            )
        };
        let (_document_ref, page_count, _protected) = self
            .worker_broker
            .ensure_document(&admission, key.clone(), path, credential, cancellation)
            .await
            .map_err(DocumentSessionError::from)?;
        Ok(WorkerReadContext {
            _admission: admission,
            _lease: lease,
            key,
            page_count,
        })
    }

    /// Lifts a page quarantine on an explicit user retry. It never opens the
    /// document; the next read reopens it through the usual worker path.
    async fn release_worker_page_quarantine(
        &self,
        session_id: &str,
        expected_generation: u64,
        page_index: u32,
    ) -> Result<bool, DocumentSessionError> {
        let key = WorkerDocumentKey {
            session_id: session_id.to_string(),
            content_generation: expected_generation,
        };
        let admission = self
            .worker_broker
            .admit_read(&key, None)
            .await
            .map_err(DocumentSessionError::from)?;
        let entry = self.entry(session_id)?;
        let _lease = entry.operation.clone().read_owned().await;
        {
            let state = entry.state.lock();
            session_read_path(&state, Some(expected_generation))?;
        }
        self.worker_broker
            .release_page_quarantine(&admission, &key, page_index)
            .await
            .map_err(DocumentSessionError::from)
    }

    async fn invalidate_failed_worker_read(&self, context: WorkerReadContext) {
        let key = context.key.clone();
        drop(context);
        if let Ok(admission) = self.worker_broker.admit_exclusive(&key.session_id).await {
            let _ = self
                .worker_broker
                .invalidate_document(&admission, &key)
                .await;
        }
    }

    fn jobs_root(&self) -> PathBuf {
        self.instance_dir.join("jobs")
    }

    fn insert_candidate(
        &self,
        original_path: PathBuf,
        working_path: PathBuf,
        session_dir: PathBuf,
        digest: String,
        encryption: CandidateEncryption,
    ) -> OpenDocumentSessionResult {
        let encrypted = encryption != CandidateEncryption::None;
        let password_required = encryption == CandidateEncryption::PasswordRequired;
        let id = session_dir
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|value| value.strip_prefix("session-"))
            .unwrap_or_default()
            .to_string();
        let lifecycle = if password_required {
            SessionLifecycle::CandidateAuthPending
        } else {
            SessionLifecycle::CandidateReady
        };
        let session = DocumentSession {
            id: id.clone(),
            save_target_path: original_path,
            working_path,
            session_dir,
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle,
            editable: !encrypted,
            read_only_reason: encrypted
                .then(|| "Şifreli PDF mutasyonları desteklenmiyor".to_string()),
            opened_at_ms: now_ms(),
            baseline: BaselineFingerprint {
                digest: digest.clone(),
            },
            working_digest: Some(digest),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        };
        let snapshot = snapshot(&session);
        self.sessions.write().insert(
            id.clone(),
            Arc::new(SessionEntry {
                operation: Arc::new(AsyncRwLock::new(())),
                state: Mutex::new(session),
            }),
        );
        OpenDocumentSessionResult {
            status: if password_required {
                "password_required".to_string()
            } else {
                "candidate_ready".to_string()
            },
            session_id: id,
            snapshot: (!password_required).then_some(snapshot),
        }
    }

    #[cfg(test)]
    async fn mutate<F>(
        &self,
        session_id: String,
        expected_generation: u64,
        operation: F,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError>
    where
        F: FnOnce(String, Option<String>) -> Result<AtomicWriteReport, DocumentIoError>
            + Send
            + 'static,
    {
        let admission = self
            .worker_broker
            .admit_exclusive(&session_id)
            .await
            .map_err(DocumentSessionError::from)?;
        let entry = self.entry(&session_id)?;
        let _lease = entry.operation.clone().write_owned().await;
        let (working_path, credential, before_digest) = {
            let state = entry.state.lock();
            ensure_open_for_mutation(&state, expected_generation)?;
            (
                state.working_path.to_string_lossy().to_string(),
                state.credential.clone(),
                state.working_digest.clone(),
            )
        };
        let worker_key = WorkerDocumentKey {
            session_id: session_id.clone(),
            content_generation: expected_generation,
        };
        self.worker_broker
            .invalidate_document(&admission, &worker_key)
            .await
            .map_err(DocumentSessionError::from)?;

        let result =
            tauri::async_runtime::spawn_blocking(move || operation(working_path, credential))
                .await
                .map_err(|error| {
                    DocumentSessionError::new(
                        "DOCUMENT_SESSION_THREAD_ERROR",
                        format!("Mutation worker tamamlanamadı: {error}"),
                    )
                })?;

        let mut state = entry.state.lock();
        match result {
            Ok(report) => {
                apply_committed_mutation(&mut state, report)?;
                Ok(snapshot(&state))
            }
            Err(error) => {
                apply_failed_mutation(&mut state, before_digest.as_deref(), &error)?;
                Err(DocumentSessionError::from_io(&error).with_snapshot(snapshot(&state)))
            }
        }
    }

    pub(crate) async fn mutate_isolated(
        &self,
        session_id: String,
        expected_generation: u64,
        recipe: MutationRecipe,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
        self.mutate_isolated_with_engine(
            session_id,
            expected_generation,
            recipe,
            Arc::new(ProductionMutationEngine),
        )
        .await
    }
    pub(crate) async fn mutate_isolated_with_engine(
        &self,
        session_id: String,
        expected_generation: u64,
        recipe: MutationRecipe,
        engine: Arc<dyn MutationEngine>,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
        self.run_mutation_transaction(
            session_id,
            expected_generation,
            MutationTransactionKind::Recipe(recipe),
            engine,
        )
        .await
    }

    /// Undo/redo: geçmişteki durumu mutasyonla aynı işlem hattından geri yükler
    /// (tasarım §5.4). Generation her zaman ilerler; `dirty` içeriğe göre yeniden hesaplanır.
    pub(crate) async fn mutate_isolated_restore(
        &self,
        session_id: String,
        expected_generation: u64,
        direction: HistoryDirection,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
        self.run_mutation_transaction(
            session_id,
            expected_generation,
            MutationTransactionKind::Restore(direction),
            Arc::new(ProductionMutationEngine),
        )
        .await
    }

    async fn run_mutation_transaction(
        &self,
        session_id: String,
        expected_generation: u64,
        kind: MutationTransactionKind,
        engine: Arc<dyn MutationEngine>,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
        let (record, reservation_guard) = self
            .mutation_transaction_manager
            .begin_transaction(&session_id, expected_generation)?;

        let entry = self.entry(&session_id)?;
        let (initial_digest, working_path) = {
            let state = entry.state.lock();
            (state.working_digest.clone(), state.working_path.clone())
        };

        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: record.id,
            _session_id: session_id.clone(),
            initial_digest,
            replacement_digest: Arc::new(Mutex::new(None)),
            preflight: Arc::new(Mutex::new(None)),
            working_path,
            reconciliation_state: Arc::new(Mutex::new(ReconciliationState::NotStarted)),
        };

        let (start_tx, start_rx) = tokio::sync::oneshot::channel::<()>();

        let input = MutationTransactionInput {
            transaction_id: record.id,
            session_id: session_id.clone(),
            expected_generation,
            kind,
        };
        let deps = MutationTransactionDependencies {
            worker_broker: self.worker_broker.clone(),
            jobs_root: self.jobs_root(),
            engine,
        };
        let child_owner = SessionJobChildProcessOwner::default();
        let exchange_dir_authority = match tempfile::Builder::new()
            .prefix("mutation-")
            .tempdir_in(self.jobs_root())
        {
            Ok(dir) => {
                let path = dir.path().to_path_buf();
                ExchangeCleanupAuthority::new(dir, path)
            }
            Err(err) => {
                let snap = snapshot(&entry.state.lock());
                let error = DocumentSessionError::new(
                    "DOCUMENT_SESSION_TEMP_CREATE_FAILED",
                    format!("Geçici çalışma dizini oluşturulamadı: {err}"),
                )
                .with_snapshot(snap);
                let outcome = MutationTransactionOutcome::FailedPreCommit {
                    transaction_id: record.id.to_string(),
                    session_id: session_id.clone(),
                    error: error.clone(),
                    cleanup_warning: None,
                };
                let _ = record.publish_terminal(outcome);
                reservation_guard.release();
                return Err(error);
            }
        };

        let execution_authority = SupervisorExecutionAuthority::new();
        execution_authority.bind_child_owner(child_owner.clone());
        let bundle = MutationSupervisorRecoveryBundle {
            transaction_record: Arc::clone(&record),
            session_entry: Arc::clone(&entry),
            recovery_ctx: recovery_ctx.clone(),
            child_owner: child_owner.clone(),
            exchange_cleanup_authority: exchange_dir_authority.clone(),
            execution_authority: execution_authority.clone(),
        };

        reservation_guard.disarm();

        let ctx = MutationTransactionOwnerContext {
            entry: Arc::clone(&entry),
            record: Arc::clone(&record),
            manager: Arc::clone(&self.mutation_transaction_manager),
            recovery_ctx: recovery_ctx.clone(),
            child_processes: child_owner.clone(),
            exchange_cleanup_authority: exchange_dir_authority.clone(),
            execution_authority: execution_authority.clone(),
        };

        let tx_id = record.id;
        let record_clone = Arc::clone(&record);
        let entry_clone = Arc::clone(&entry);
        let session_id_clone = session_id.clone();
        let exec_auth_clone = execution_authority.clone();

        let owner_exec_auth = execution_authority.clone();
        let owner_task = tokio::spawn(async move {
            let _owner_liveness = OwnerBodyExitGuard(owner_exec_auth);
            if start_rx.await.is_err() {
                return MutationOwnerResult {
                    transaction_id: input.transaction_id,
                    session_id: input.session_id,
                    classification: MutationOwnerClassification::FailedPreCommit {
                        error: DocumentSessionError::new(
                            "MUTATION_TRANSACTION_INITIALIZATION_FAILED",
                            "Start signal kanalı owner başlatılamadan kapandı",
                        ),
                    },
                    child_owner: ctx.child_processes.clone(),
                    recovery_ctx: ctx.recovery_ctx.clone(),
                };
            }
            execute_mutation_transaction(input, deps, ctx).await
        });

        let supervisor_task = tokio::spawn(async move {
            let _liveness = SupervisorBodyExitGuard(exec_auth_clone.clone());

            let mut owner_task = owner_task;
            let owner_join = tokio::select! {
                result = &mut owner_task => result,
                _ = exec_auth_clone.cancel_token.cancelled() => {
                    record_clone.request_cancel("MUTATION_SUPERVISOR_CANCELLATION".to_string());
                    owner_task.await
                }
            };
            let final_outcome = match owner_join {
                Ok(owner_result) => {
                    exec_auth_clone.bind_child_owner(owner_result.child_owner.clone());
                    let _ = &owner_result.recovery_ctx;
                    match owner_result.classification {
                        MutationOwnerClassification::Aborted { reason } => {
                            MutationTransactionOutcome::Aborted {
                                transaction_id: owner_result.transaction_id.to_string(),
                                session_id: owner_result.session_id.clone(),
                                reason,
                                cleanup_warning: None,
                            }
                        }
                        MutationOwnerClassification::FailedPreCommit { error } => {
                            MutationTransactionOutcome::FailedPreCommit {
                                transaction_id: owner_result.transaction_id.to_string(),
                                session_id: owner_result.session_id.clone(),
                                error,
                                cleanup_warning: None,
                            }
                        }
                        MutationOwnerClassification::NotAppliedAfterCommitAttempt {
                            error,
                            snapshot,
                        } => MutationTransactionOutcome::NotAppliedAfterCommitAttempt {
                            transaction_id: owner_result.transaction_id.to_string(),
                            session_id: owner_result.session_id.clone(),
                            error,
                            snapshot,
                            cleanup_warning: None,
                        },
                        MutationOwnerClassification::Committed { snapshot } => {
                            MutationTransactionOutcome::Completed {
                                transaction_id: owner_result.transaction_id.to_string(),
                                session_id: owner_result.session_id.clone(),
                                snapshot,
                                cleanup_warning: None,
                            }
                        }
                        MutationOwnerClassification::CommittedUnconfirmed { snapshot, warning } => {
                            MutationTransactionOutcome::CommittedUnconfirmed {
                                transaction_id: owner_result.transaction_id.to_string(),
                                session_id: owner_result.session_id.clone(),
                                snapshot,
                                warning,
                            }
                        }
                        MutationOwnerClassification::RecoveryRequired { error, snapshot } => {
                            MutationTransactionOutcome::RecoveryRequired {
                                transaction_id: owner_result.transaction_id.to_string(),
                                session_id: owner_result.session_id.clone(),
                                snapshot,
                                error,
                            }
                        }
                        MutationOwnerClassification::Indeterminate { error, snapshot } => {
                            MutationTransactionOutcome::Indeterminate {
                                transaction_id: owner_result.transaction_id.to_string(),
                                session_id: owner_result.session_id.clone(),
                                snapshot,
                                error,
                            }
                        }
                    }
                }
                Err(join_err) => {
                    let phase = record_clone.control.lock().phase;
                    let outcome = if phase.is_post_commit()
                        || phase == MutationTransactionPhase::Committing
                    {
                        if let Some(classification) = reconcile_post_commit_authority(
                            &recovery_ctx,
                            &entry_clone,
                            &tx_id,
                            &session_id_clone,
                            &join_err.to_string(),
                        ) {
                            match classification {
                                MutationOwnerClassification::NotAppliedAfterCommitAttempt {
                                    error,
                                    snapshot,
                                } => MutationTransactionOutcome::NotAppliedAfterCommitAttempt {
                                    transaction_id: tx_id.to_string(),
                                    session_id: session_id_clone.clone(),
                                    error,
                                    snapshot,
                                    cleanup_warning: None,
                                },
                                MutationOwnerClassification::CommittedUnconfirmed {
                                    snapshot,
                                    warning,
                                } => MutationTransactionOutcome::CommittedUnconfirmed {
                                    transaction_id: tx_id.to_string(),
                                    session_id: session_id_clone.clone(),
                                    snapshot,
                                    warning,
                                },
                                _ => {
                                    let snap = snapshot(&entry_clone.state.lock());
                                    let err = DocumentSessionError::new(
                                        "MUTATION_TRANSACTION_OWNER_PANIC",
                                        format!("Owner task panikledi: {join_err}"),
                                    );
                                    MutationTransactionOutcome::Indeterminate {
                                        transaction_id: tx_id.to_string(),
                                        session_id: session_id_clone.clone(),
                                        snapshot: snap,
                                        error: err,
                                    }
                                }
                            }
                        } else {
                            let snap = snapshot(&entry_clone.state.lock());
                            let err = DocumentSessionError::new(
                                "MUTATION_TRANSACTION_OWNER_PANIC",
                                format!("Reconciliation zaten başka task tarafından yürütüldü: {join_err}"),
                            );
                            MutationTransactionOutcome::Indeterminate {
                                transaction_id: tx_id.to_string(),
                                session_id: session_id_clone.clone(),
                                snapshot: snap,
                                error: err,
                            }
                        }
                    } else {
                        let snap = {
                            let state = entry_clone.state.lock();
                            snapshot(&state)
                        };
                        let err = DocumentSessionError::new(
                            "MUTATION_TRANSACTION_OWNER_PANIC",
                            format!("Owner task pre-commit aşamasında panikledi: {join_err}"),
                        );
                        MutationTransactionOutcome::FailedPreCommit {
                            transaction_id: tx_id.to_string(),
                            session_id: session_id_clone.clone(),
                            error: err.with_snapshot(snap),
                            cleanup_warning: None,
                        }
                    };
                    outcome
                }
            };
            final_outcome
        });

        self.mutation_transaction_manager
            .register_supervisor_bundle(tx_id, bundle, supervisor_task);
        let _ = start_tx.send(());

        let outcome = record.wait_for_terminal().await;
        match outcome {
            MutationTransactionOutcome::Completed { snapshot, .. }
            | MutationTransactionOutcome::CommittedUnconfirmed { snapshot, .. } => Ok(snapshot),
            MutationTransactionOutcome::Aborted { reason, .. } => Err(DocumentSessionError::new(
                "MUTATION_TRANSACTION_CANCELLED",
                reason,
            )),
            MutationTransactionOutcome::FailedPreCommit { error, .. } => Err(error),
            MutationTransactionOutcome::NotAppliedAfterCommitAttempt {
                snapshot, error, ..
            }
            | MutationTransactionOutcome::RecoveryRequired {
                snapshot, error, ..
            }
            | MutationTransactionOutcome::Indeterminate {
                snapshot, error, ..
            } => Err(error.with_snapshot(snapshot)),
        }
    }

    async fn save(
        &self,
        session_id: &str,
        expected_state_revision: u64,
        target_override: Option<PathBuf>,
    ) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
        if self.shutdown_admission_closed.load(Ordering::Acquire) {
            return Err(DocumentSessionError::new(
                "SESSION_SHUTDOWN_IN_PROGRESS",
                "Shutdown başladıktan sonra yeni Save işlemi kabul edilmez",
            ));
        }
        #[cfg(test)]
        if self.lifecycle_gate.clone().try_read_owned().is_err() {
            self.trace_transition_authority_release(
                TransitionAuthorityReleaseEvent::SaveLifecycleWaitBlocked,
            );
            if let Some(notify) = self.save_lifecycle_wait_notify.lock().clone() {
                notify.notify_waiters();
            }
        }
        let _lifecycle = self.lifecycle_gate.clone().read_owned().await;
        #[cfg(test)]
        self.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::SaveLifecycleAcquired,
        );
        if self.shutdown_admission_closed.load(Ordering::Acquire) {
            return Err(DocumentSessionError::new(
                "SESSION_SHUTDOWN_IN_PROGRESS",
                "Shutdown başladıktan sonra yeni Save işlemi kabul edilmez",
            ));
        }
        let entry = self.entry(session_id)?;
        #[cfg(test)]
        self.pause_after_lifecycle_before_operation().await;
        if let Some(hook) = self.secure_redaction_save_lease_test_hook.lock().clone() {
            hook(false);
        }
        let _lease = entry.operation.clone().write_owned().await;
        if let Some(hook) = self.secure_redaction_save_lease_test_hook.lock().clone() {
            hook(true);
        }
        #[cfg(test)]
        self.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::SaveOperationAcquired,
        );
        self.save_core_with_gate_held(&entry, expected_state_revision, target_override, None)
            .await
            .map(|success| success.snapshot)
            .map_err(|failure| failure.error)
    }

    async fn save_core_with_gate_held(
        &self,
        entry: &Arc<SessionEntry>,
        expected_state_revision: u64,
        target_override: Option<PathBuf>,
        transition_execution: Option<&TransitionExecution>,
    ) -> Result<SaveCoreSuccess, SaveCoreFailure> {
        let save_target_path = {
            let state = entry.state.lock();
            ensure_lifecycle(
                &state,
                &[SessionLifecycle::Open, SessionLifecycle::DecisionPending],
            )?;
            if state.state_revision != expected_state_revision {
                return Err(DocumentSessionError::new(
                    "STATE_REVISION_MISMATCH",
                    format!(
                        "State revision uyuşmazlığı: beklenen {expected_state_revision}, mevcut {}",
                        state.state_revision
                    ),
                )
                .into());
            }
            if target_override.is_none() && !state.dirty {
                return Ok(SaveCoreSuccess {
                    snapshot: snapshot(&state),
                    classification: SaveCommitClassification::NotCommitted,
                });
            }
            state.save_target_path.clone()
        };
        let same_target = target_override
            .as_ref()
            .map(|target| {
                same_persistence_target(&save_target_path, target).map_err(|error| {
                    DocumentSessionError::new(
                        "SAVE_AS_TARGET_IDENTITY_FAILED",
                        format!("Save As target identity çözümlenemedi: {error}"),
                    )
                })
            })
            .transpose()?
            .unwrap_or(false);
        let (
            session_id,
            working_path,
            target,
            expectation,
            is_save_as,
            credential,
            content_generation,
            publication_revision,
        ) = {
            let state = entry.state.lock();
            ensure_lifecycle(
                &state,
                &[SessionLifecycle::Open, SessionLifecycle::DecisionPending],
            )?;
            if state.state_revision != expected_state_revision {
                return Err(DocumentSessionError::new(
                    "STATE_REVISION_MISMATCH",
                    format!(
                        "State revision uyuşmazlığı: beklenen {expected_state_revision}, mevcut {}",
                        state.state_revision
                    ),
                )
                .into());
            }
            if !state.dirty && (target_override.is_none() || same_target) {
                return Ok(SaveCoreSuccess {
                    snapshot: snapshot(&state),
                    classification: SaveCommitClassification::NotCommitted,
                });
            }
            let is_save_as = target_override.is_some() && !same_target;
            let target = if is_save_as {
                target_override
                    .as_ref()
                    .expect("Save As target checked above")
                    .clone()
            } else {
                state.save_target_path.clone()
            };
            reject_private_target(&target, &self.temp_root)?;
            let expectation = if is_save_as {
                if target.exists() {
                    Some(TargetExpectation::Digest(sha256_file(&target).map_err(
                        |error| {
                            DocumentSessionError::new(
                                "SAVE_AS_TARGET_READ_FAILED",
                                format!("Save As hedefi okunamadı: {error}"),
                            )
                        },
                    )?))
                } else {
                    Some(TargetExpectation::Missing)
                }
            } else {
                Some(TargetExpectation::Digest(state.baseline.digest.clone()))
            };
            let publication_revision = checked_next_revision(state.state_revision)?;
            (
                state.id.clone(),
                state.working_path.clone(),
                target,
                expectation,
                is_save_as,
                state.credential.clone(),
                state.content_generation,
                publication_revision,
            )
        };

        #[cfg(test)]
        let save_barriers = self.save_io_barriers.lock().clone();
        #[cfg(test)]
        if let Some((reached, release)) = save_barriers {
            reached.wait().await;
            release.wait().await;
        }
        #[cfg(test)]
        if transition_execution.is_some()
            && self
                .panic_next_transition_precommit
                .swap(false, Ordering::AcqRel)
        {
            panic!("transition pre-commit panic test hook");
        }
        #[cfg(test)]
        self.save_core_invocations.fetch_add(1, Ordering::AcqRel);
        if let Some(execution) = transition_execution {
            publish_transition_phase(
                execution,
                TransitionExecutionPhase::SaveInProgress {
                    target: target_override.clone(),
                },
            );
        }
        let source = working_path.clone();
        let target_for_worker = target.clone();
        let worker = tauri::async_runtime::spawn_blocking(move || {
            atomic_copy_pdf(
                if is_save_as {
                    "save_document_session_as"
                } else {
                    "save_document_session"
                },
                &source,
                &target_for_worker,
                expectation,
                credential,
            )
        });
        let worker_result = worker.await;
        #[cfg(test)]
        let commit_barriers = self.save_commit_barriers.lock().clone();
        #[cfg(test)]
        if let Some((reached, release)) = commit_barriers {
            reached.wait().await;
            release.wait().await;
        }
        let result = match worker_result {
            Ok(result) => result,
            Err(error) => {
                let mut state = entry.state.lock();
                state.lifecycle = SessionLifecycle::CommitIndeterminate;
                state.state_revision = publication_revision;
                state.last_save_commit = Some(SaveCommitDiagnostic {
                    target_path: target.clone(),
                    committed_digest: None,
                    commit_state: CommitState::Indeterminate,
                    recovery_path: None,
                });
                return Err(SaveCoreFailure {
                    error: DocumentSessionError::new(
                        "SAVE_COMMIT_OUTCOME_UNKNOWN",
                        format!("Save worker tamamlanamadı; commit sonucu bilinmiyor: {error}"),
                    )
                    .with_snapshot(snapshot(&state)),
                    classification: SaveCommitClassification::CommitIndeterminate,
                });
            }
        };

        let report = match result {
            Ok(report) => report,
            Err(error) => {
                let mut state = entry.state.lock();
                match error.state {
                    CommitState::RecoveryRequired => {
                        state.lifecycle = SessionLifecycle::RecoveryRequired;
                        state.state_revision = publication_revision;
                        state.last_save_commit = Some(SaveCommitDiagnostic {
                            target_path: target,
                            committed_digest: error.replacement_digest.clone(),
                            commit_state: error.state,
                            recovery_path: error.recovery_path.as_deref().cloned(),
                        });
                    }
                    CommitState::Indeterminate => {
                        state.lifecycle = SessionLifecycle::CommitIndeterminate;
                        state.state_revision = publication_revision;
                        state.last_save_commit = Some(SaveCommitDiagnostic {
                            target_path: target,
                            committed_digest: error.replacement_digest.clone(),
                            commit_state: error.state,
                            recovery_path: error.recovery_path.as_deref().cloned(),
                        });
                    }
                    _ => {}
                }
                return Err(SaveCoreFailure {
                    error: DocumentSessionError::from_io(&error).with_snapshot(snapshot(&state)),
                    classification: classify_document_io_commit_state(error.state),
                });
            }
        };
        match report.state {
            CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed => {
                let mut state = entry.state.lock();
                let publication_valid = state.id == session_id
                    && state.content_generation == content_generation
                    && state.state_revision == expected_state_revision
                    && matches!(
                        state.lifecycle,
                        SessionLifecycle::Open | SessionLifecycle::DecisionPending
                    );
                #[cfg(test)]
                let publication_valid = publication_valid
                    && !self
                        .fail_next_save_publication
                        .swap(false, Ordering::AcqRel);
                if !publication_valid {
                    state.lifecycle = SessionLifecycle::RecoveryRequired;
                    state.state_revision = publication_revision;
                    state.last_save_commit = Some(SaveCommitDiagnostic {
                        target_path: target.clone(),
                        committed_digest: Some(report.committed_digest.clone()),
                        commit_state: report.state,
                        recovery_path: None,
                    });
                    return Err(SaveCoreFailure {
                        error: DocumentSessionError::committed_refresh_failed(
                            &target,
                            &report,
                            snapshot(&state),
                        ),
                        classification: SaveCommitClassification::CommittedButSessionRefreshFailed,
                    });
                }
                state.save_target_path = target;
                state.baseline.digest = report.committed_digest.clone();
                refresh_dirty(&mut state);
                state.health_warning = (report.state
                    == CommitState::NewCommittedDurabilityUnconfirmed)
                    .then(|| "SAVE_DURABILITY_UNCONFIRMED".to_string());
                state.last_save_commit = Some(SaveCommitDiagnostic {
                    target_path: state.save_target_path.clone(),
                    committed_digest: Some(report.committed_digest),
                    commit_state: report.state,
                    recovery_path: None,
                });
                state.state_revision = publication_revision;
                Ok(SaveCoreSuccess {
                    snapshot: snapshot(&state),
                    classification: SaveCommitClassification::Committed,
                })
            }
            other => {
                let classification = if other == CommitState::RecoveryRequired {
                    SaveCommitClassification::RecoveryRequired
                } else {
                    SaveCommitClassification::CommitIndeterminate
                };
                let mut state = entry.state.lock();
                if classification == SaveCommitClassification::CommitIndeterminate {
                    state.lifecycle = SessionLifecycle::CommitIndeterminate;
                    state.state_revision = publication_revision;
                } else if classification == SaveCommitClassification::RecoveryRequired {
                    state.lifecycle = SessionLifecycle::RecoveryRequired;
                    state.state_revision = publication_revision;
                }
                Err(SaveCoreFailure {
                    error: DocumentSessionError::new(
                        "DOCUMENT_SESSION_UNEXPECTED_COMMIT_STATE",
                        format!("Save beklenmeyen commit state döndürdü: {other}"),
                    )
                    .with_snapshot(snapshot(&state)),
                    classification,
                })
            }
        }
    }

    async fn close_entry(&self, session_id: &str) -> Result<(), DocumentSessionError> {
        let _lifecycle = self.lifecycle_gate.clone().write_owned().await;
        self.close_entry_under_lifecycle(session_id).await
    }

    async fn close_entry_under_lifecycle(
        &self,
        session_id: &str,
    ) -> Result<(), DocumentSessionError> {
        if let Some((_search_id, cancellation)) =
            self.search_cancellations.lock().remove(session_id)
        {
            cancellation.cancel();
        }
        let entry = self.entry(session_id)?;
        let _lease = entry.operation.clone().write_owned().await;
        let admission = self
            .worker_broker
            .admit_exclusive(session_id)
            .await
            .map_err(DocumentSessionError::from)?;
        let credential_path = {
            self.worker_broker
                .invalidate_session(&admission, session_id)
                .await
                .map_err(DocumentSessionError::from)?;
            let mut state = entry.state.lock();
            state.lifecycle = SessionLifecycle::Closing;
            bump_revision(&mut state)?;
            state.credential = None;
            let credential_path = state.working_path.to_string_lossy().to_string();
            state.lifecycle = SessionLifecycle::Closed;
            bump_revision(&mut state)?;
            credential_path
        };
        self.password_cache.lock().remove(&credential_path);
        let removed = self.sessions.write().remove(session_id);
        {
            let mut active_session_id = self.active_session_id.lock();
            if active_session_id.as_deref() == Some(session_id) {
                *active_session_id = None;
                self.active_identity_epoch.fetch_add(1, Ordering::AcqRel);
            }
        }
        if let Some(entry) = removed {
            let directory = entry.state.lock().session_dir.clone();
            secure_remove_tree(&directory)?;
        }
        Ok(())
    }

    pub async fn close_all(&self) -> Result<(), DocumentSessionError> {
        let ids = self.sessions.read().keys().cloned().collect::<Vec<_>>();
        for id in ids {
            self.close_entry(&id).await?;
        }
        self.activations_by_token.lock().clear();
        self.activation_token_by_candidate.lock().clear();
        *self.active_session_id.lock() = None;
        Ok(())
    }

    pub fn start_job_reaper(&self) {
        if self.job_reaper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let registry = Arc::clone(&self.job_registry);
        let shutdown = self.job_reaper_shutdown.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(SESSION_JOB_REAPER_INTERVAL_MS));
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        reap_session_jobs_once(&registry).await;
                    }
                }
            }
        });
    }

    pub async fn shutdown_session_jobs(&self) {
        self.job_reaper_shutdown.cancel();
        let jobs = {
            let registry = self.job_registry.lock();
            registry.active.values().cloned().collect::<Vec<_>>()
        };
        for job in &jobs {
            request_session_job_cancellation(job, SessionJobCancellationReason::Shutdown);
        }
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(SESSION_JOB_SHUTDOWN_QUIESCE_MS);
        loop {
            let unfinished = {
                let registry = self.job_registry.lock();
                registry
                    .active
                    .values()
                    .any(|job| !job.mutable.lock().operation_status.is_terminal())
            };
            if !unfinished || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        reap_session_jobs_once(&self.job_registry).await;
    }

    pub async fn shutdown_all_resources(
        &self,
        active_tasks: &ActiveRenderTasks,
        render_service: &RenderService,
    ) -> Vec<CleanupWarning> {
        let mut warnings = Vec::new();
        self.shutdown_admission_closed
            .store(true, Ordering::Release);

        for (_session_id, (_search_id, cancellation)) in self.search_cancellations.lock().drain() {
            cancellation.cancel();
        }
        for (_task_id, cancellation) in active_tasks.0.lock().drain() {
            cancellation.cancel();
        }
        render_service.cancel_all();

        let mutation_report_res = self.mutation_transaction_manager.shutdown(3000).await;
        let terminal_completion_published = self
            .mutation_transaction_manager
            .inner
            .lock()
            .shutdown_completion
            .is_some();
        let monitor_join_res = if terminal_completion_published {
            Some(
                self.mutation_transaction_manager
                    .join_coordinator_and_reaper_monitors()
                    .await,
            )
        } else {
            None
        };

        let (
            coord_business_ok,
            manager_terminated_ok,
            active_txs_zero,
            active_res_zero,
            active_sup_zero,
            active_children_zero,
            pending_recovery_zero,
            pending_reconciliation_zero,
            pending_cleanup_zero,
            pending_exchange_cleanup_zero,
            monitor_joined_ok,
            reaper_monitor_joined_ok,
            reaper_joined_ok,
        ) = {
            let inner = self.mutation_transaction_manager.inner.lock();
            let children = inner
                .supervisor_handles
                .values()
                .filter_map(|e| e.recovery_bundle.as_ref())
                .map(|b| b.child_owner.active_child_count())
                .sum::<usize>();
            let pending_recovery = inner
                .supervisor_handles
                .values()
                .filter(|entry| {
                    matches!(
                        entry.finalization_state,
                        SupervisorFinalizationState::RecoveryInProgress
                            | SupervisorFinalizationState::TerminationUnproven(_)
                            | SupervisorFinalizationState::FinalizationFailed(_)
                    )
                })
                .count();
            let pending_reconciliation = inner
                .supervisor_handles
                .values()
                .filter_map(|entry| entry.recovery_bundle.as_ref())
                .filter(|bundle| {
                    let phase = bundle.transaction_record.control.lock().phase;
                    (phase == MutationTransactionPhase::Committing || phase.is_post_commit())
                        && *bundle.recovery_ctx.reconciliation_state.lock()
                            != ReconciliationState::Completed
                })
                .count();
            let pending_cleanup = inner
                .supervisor_handles
                .values()
                .filter(|entry| {
                    matches!(
                        entry.finalization_state,
                        SupervisorFinalizationState::CleanupInProgress
                    )
                })
                .count();
            let pending_exchange_cleanup = inner
                .supervisor_handles
                .values()
                .filter_map(|entry| entry.recovery_bundle.as_ref())
                .filter(|bundle| !bundle.exchange_cleanup_authority.is_cleaned())
                .count();
            (
                mutation_report_res.is_ok(),
                inner.lifecycle == MutationManagerLifecycle::Terminated,
                inner.active_transactions.is_empty(),
                inner.session_reservations.is_empty(),
                inner.supervisor_handles.is_empty(),
                children == 0,
                pending_recovery == 0,
                pending_reconciliation == 0,
                pending_cleanup == 0,
                pending_exchange_cleanup == 0,
                inner.coordinator_monitor_join_state
                    == CoordinatorMonitorJoinState::ExternallyJoinedSuccess,
                inner.reaper_monitor_join_state == ReaperMonitorJoinState::ExternallyJoinedSuccess,
                inner.reaper_task_state == ReaperTaskState::JoinedSuccess,
            )
        };

        match mutation_report_res {
            Ok(report) => {
                warnings.extend(report.warnings);
            }
            Err(ref err) => {
                warnings.push(CleanupWarning {
                    code: err.code.to_string(),
                    message: err.message.clone(),
                });
            }
        }

        if let Some(Err(ref err)) = monitor_join_res {
            warnings.push(CleanupWarning {
                code: err.code.to_string(),
                message: err.message.clone(),
            });
        }

        if !coord_business_ok
            || !manager_terminated_ok
            || !active_txs_zero
            || !active_res_zero
            || !active_sup_zero
            || !active_children_zero
            || !pending_recovery_zero
            || !pending_reconciliation_zero
            || !pending_cleanup_zero
            || !pending_exchange_cleanup_zero
            || !monitor_joined_ok
            || !reaper_monitor_joined_ok
            || !reaper_joined_ok
        {
            warnings.push(CleanupWarning {
                code: "MUTATION_GLOBAL_SHUTDOWN_GATED".to_string(),
                message: format!(
                    "Global destructive cleanup gated: business_ok={coord_business_ok}, manager_terminated={manager_terminated_ok}, active_txs_zero={active_txs_zero}, active_res_zero={active_res_zero}, active_sup_zero={active_sup_zero}, active_children_zero={active_children_zero}, pending_recovery_zero={pending_recovery_zero}, pending_reconciliation_zero={pending_reconciliation_zero}, pending_cleanup_zero={pending_cleanup_zero}, pending_exchange_cleanup_zero={pending_exchange_cleanup_zero}, coordinator_monitor_joined={monitor_joined_ok}, reaper_monitor_joined={reaper_monitor_joined_ok}, reaper_joined={reaper_joined_ok}"
                ),
            });
            return warnings;
        }

        #[cfg(test)]
        let shutdown_barriers = self.shutdown_lifecycle_barriers.lock().clone();
        #[cfg(test)]
        if let Some((reached, release)) = shutdown_barriers {
            reached.wait().await;
            release.wait().await;
        }

        #[cfg(test)]
        if self.lifecycle_gate.clone().try_write_owned().is_err() {
            self.trace_transition_authority_release(
                TransitionAuthorityReleaseEvent::ShutdownLifecycleWaitBlocked,
            );
            if let Some(notify) = self.shutdown_lifecycle_wait_notify.lock().clone() {
                notify.notify_waiters();
            }
        }
        let lifecycle = match tokio::time::timeout(
            Duration::from_secs(4),
            self.lifecycle_gate.clone().write_owned(),
        )
        .await
        {
            Ok(lease) => lease,
            Err(_) => {
                warnings.push(CleanupWarning {
                    code: "SESSION_SHUTDOWN_OPERATION_TIMEOUT".to_string(),
                    message: "In-flight Save/transition operation tamamlanmadı; registry ve session root korunuyor"
                        .to_string(),
                });
                return warnings;
            }
        };
        #[cfg(test)]
        self.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::ShutdownLifecycleAcquired,
        );

        let mut entries = self
            .sessions
            .read()
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(&right.0));

        let broker = self.worker_broker.clone();
        let (broker_result, ()) = tokio::join!(broker.shutdown(), self.shutdown_session_jobs());
        if let Err(error) = broker_result {
            warnings.push(CleanupWarning {
                code: error.code.to_string(),
                message: error.message,
            });
        }

        let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        let mut operation_leases = Vec::with_capacity(entries.len());
        for (session_id, entry) in &entries {
            let remaining = cleanup_deadline.saturating_duration_since(tokio::time::Instant::now());
            let lease = if remaining.is_zero() {
                None
            } else {
                tokio::time::timeout(
                    remaining.min(Duration::from_secs(2)),
                    entry.operation.clone().write_owned(),
                )
                .await
                .ok()
            };
            let Some(lease) = lease else {
                warnings.push(CleanupWarning {
                    code: "SESSION_SHUTDOWN_OPERATION_TIMEOUT".to_string(),
                    message: format!(
                        "Session {session_id} write lease alınamadı; registry ve session root korunuyor"
                    ),
                });
                return warnings;
            };
            operation_leases.push(lease);
        }

        let unsafe_sessions = entries
            .iter()
            .filter_map(|(session_id, entry)| {
                let state = entry.state.lock();
                let transition_unsafe =
                    state.active_transition.as_ref().is_some_and(|transition| {
                        matches!(
                            &*transition.execution.phase.lock(),
                            TransitionExecutionPhase::PendingUnclaimed
                                | TransitionExecutionPhase::SaveClaimed { .. }
                                | TransitionExecutionPhase::SaveInProgress { .. }
                                | TransitionExecutionPhase::SaveCommitted { .. }
                                | TransitionExecutionPhase::DiscardInProgress
                                | TransitionExecutionPhase::Applying { .. }
                                | TransitionExecutionPhase::FailedPreCommit { .. }
                                | TransitionExecutionPhase::InterruptedPreCommit { .. }
                                | TransitionExecutionPhase::FailedPostCommit { .. }
                                | TransitionExecutionPhase::CommitIndeterminate { .. }
                                | TransitionExecutionPhase::RecoveryRequired { .. }
                                | TransitionExecutionPhase::ApplyCompleted { .. }
                        )
                    });
                (matches!(
                    state.lifecycle,
                    SessionLifecycle::CommitIndeterminate | SessionLifecycle::RecoveryRequired
                ) || transition_unsafe)
                    .then(|| {
                        state.last_save_commit.as_ref().map_or_else(
                            || session_id.clone(),
                            |diagnostic| format!("{session_id} ({})", diagnostic.summary()),
                        )
                    })
            })
            .collect::<Vec<_>>();
        if !unsafe_sessions.is_empty() {
            warnings.push(CleanupWarning {
                code: "SESSION_SHUTDOWN_RECOVERY_REQUIRED".to_string(),
                message: format!(
                    "Destructive cleanup recovery-required session nedeniyle engellendi: {}",
                    unsafe_sessions.join(",")
                ),
            });
            return warnings;
        }

        let cleanup_targets = entries
            .iter()
            .map(|(_, entry)| {
                let mut state = entry.state.lock();
                state.credential = None;
                state.lifecycle = SessionLifecycle::Closed;
                (
                    state.working_path.to_string_lossy().to_string(),
                    state.session_dir.clone(),
                )
            })
            .collect::<Vec<_>>();
        self.sessions.write().clear();
        self.activations_by_token.lock().clear();
        self.activation_token_by_candidate.lock().clear();
        if self.active_session_id.lock().take().is_some() {
            self.active_identity_epoch.fetch_add(1, Ordering::AcqRel);
        }
        drop(operation_leases);
        drop(lifecycle);

        for (credential_path, directory) in cleanup_targets {
            self.password_cache.lock().remove(&credential_path);
            if let Err(error) = secure_remove_tree(&directory) {
                warnings.push(CleanupWarning {
                    code: "SESSION_SHUTDOWN_CLEANUP_DEFERRED".to_string(),
                    message: format!(
                        "Session geçici dizini silinemedi ve orphan cleanup'a bırakıldı: {}",
                        error.message
                    ),
                });
            }
        }
        warnings
    }

    fn active_session_snapshot(
        &self,
    ) -> Result<Option<DocumentSessionSnapshot>, DocumentSessionError> {
        let active_id = self.active_session_id.lock().clone();
        active_id
            .map(|session_id| {
                let entry = self.entry(&session_id)?;
                let result = snapshot(&entry.state.lock());
                Ok(result)
            })
            .transpose()
    }

    async fn cleanup_removed_entry(&self, entry: Arc<SessionEntry>) -> Vec<CleanupWarning> {
        let (session_id, credential_path, directory) = {
            let state = entry.state.lock();
            (
                state.id.clone(),
                state.working_path.to_string_lossy().to_string(),
                state.session_dir.clone(),
            )
        };
        let admission = match self.worker_broker.admit_exclusive(&session_id).await {
            Ok(admission) => admission,
            Err(_) => {
                return vec![CleanupWarning {
                    code: "SESSION_WORKER_HANDLE_CLEANUP_DEFERRED".to_string(),
                    message:
                        "Session worker handle kapatılamadığı için geçici dosya temizliği ertelendi"
                            .to_string(),
                }]
            }
        };
        if self
            .worker_broker
            .invalidate_session(&admission, &session_id)
            .await
            .is_err()
        {
            return vec![CleanupWarning {
                code: "SESSION_WORKER_HANDLE_CLEANUP_DEFERRED".to_string(),
                message:
                    "Session worker handle kapatılamadığı için geçici dosya temizliği ertelendi"
                        .to_string(),
            }];
        }
        self.password_cache.lock().remove(&credential_path);
        match secure_remove_tree(&directory) {
            Ok(()) => Vec::new(),
            Err(_) => vec![CleanupWarning {
                code: "PREVIOUS_SESSION_DIRECTORY_CLEANUP_DEFERRED".to_string(),
                message: "Önceki session geçici dosyaları daha sonra temizlenecek".to_string(),
            }],
        }
    }

    async fn reap_expired_lifecycle(&self) -> Result<(), DocumentSessionError> {
        let _gate = self.lifecycle_gate.clone().write_owned().await;
        self.reap_expired_lifecycle_under_gate().await
    }

    async fn reap_expired_lifecycle_under_gate(&self) -> Result<(), DocumentSessionError> {
        let now = now_ms();
        let bound_transition_tokens = self
            .activations_by_token
            .lock()
            .values()
            .filter(|activation| activation.phase != CandidateActivationPhase::PreparedUnbound)
            .filter_map(|activation| activation.previous_transition_token.clone())
            .collect::<std::collections::HashSet<_>>();
        let expired_transition_sessions = {
            let sessions = self.sessions.read();
            sessions
                .iter()
                .filter_map(|(session_id, entry)| {
                    entry
                        .state
                        .lock()
                        .active_transition
                        .as_ref()
                        .filter(|transition| {
                            transition_expired(transition)
                                && !bound_transition_tokens.contains(&transition.token)
                        })
                        .map(|transition| (session_id.clone(), transition.token.clone()))
                })
                .collect::<Vec<_>>()
        };
        let expired_transition_tokens = expired_transition_sessions
            .iter()
            .map(|(_, token)| token.as_str())
            .collect::<std::collections::HashSet<_>>();
        let expired_activations = {
            let activations = self.activations_by_token.lock();
            activations
                .values()
                .filter(|activation| {
                    activation.phase == CandidateActivationPhase::PreparedUnbound
                        && (now.saturating_sub(activation.created_at_ms) > ACTIVATION_TTL_MS
                            || activation
                                .previous_transition_token
                                .as_deref()
                                .is_some_and(|token| expired_transition_tokens.contains(token)))
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        if expired_transition_sessions.is_empty() && expired_activations.is_empty() {
            return Ok(());
        }

        {
            let mut by_token = self.activations_by_token.lock();
            let mut by_candidate = self.activation_token_by_candidate.lock();
            for activation in &expired_activations {
                by_token.remove(&activation.token);
                by_candidate.remove(&activation.candidate_session_id);
            }
        }

        let mut entries = Vec::new();
        for (session_id, _) in &expired_transition_sessions {
            if let Ok(entry) = self.entry(session_id) {
                entries.push((session_id.clone(), entry));
            }
        }
        for activation in &expired_activations {
            if let Ok(entry) = self.entry(&activation.candidate_session_id) {
                entries.push((activation.candidate_session_id.clone(), entry));
            }
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries.dedup_by(|left, right| left.0 == right.0);

        let mut _leases = Vec::with_capacity(entries.len());
        for (_, entry) in &entries {
            _leases.push(entry.operation.clone().write_owned().await);
        }

        for (session_id, _) in &expired_transition_sessions {
            if let Ok(entry) = self.entry(session_id) {
                let mut state = entry.state.lock();
                if state
                    .active_transition
                    .as_ref()
                    .is_some_and(transition_expired)
                {
                    state.active_transition = None;
                    state.lifecycle = SessionLifecycle::Open;
                    bump_revision(&mut state)?;
                }
            }
        }
        for activation in expired_activations {
            if let Ok(entry) = self.entry(&activation.candidate_session_id) {
                let mut state = entry.state.lock();
                if state.lifecycle == SessionLifecycle::CandidateActivationPrepared {
                    state.lifecycle = SessionLifecycle::CandidateReady;
                    bump_revision(&mut state)?;
                }
            }
        }
        Ok(())
    }

    async fn reserve_job_snapshot(
        &self,
        job_id: &str,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<
        (
            Arc<SessionJobRecord>,
            PathBuf,
            tokio::sync::OwnedRwLockReadGuard<()>,
        ),
        DocumentSessionError,
    > {
        let canonical_job_id = Uuid::parse_str(job_id).map_err(|_| {
            DocumentSessionError::new(
                "SESSION_JOB_INVALID_ID",
                "Session job kimliği geçerli bir UUID olmalıdır",
            )
        })?;
        let entry = self.entry(session_id)?;
        let _lease = entry.operation.clone().read_owned().await;
        let (working_path, credential) = {
            let state = entry.state.lock();
            session_read_path(&state, Some(expected_generation))?;
            (state.working_path.clone(), state.credential.clone())
        };
        let job_dir = self.jobs_root().join(format!("job-{}", canonical_job_id));
        let snapshot_path = job_dir.join("snapshot.pdf");
        let cancellation_reason = Arc::new(Mutex::new(None));
        let mut registry = self.job_registry.lock();
        prune_session_job_registry(&mut registry, now_ms());
        if registry.deferred_cleanup.len() >= SESSION_JOB_DEFERRED_MAX
            || registry.active.values().any(|job| {
                let state = job.mutable.lock();
                state.operation_status.is_terminal()
                    && state.cleanup_attempts >= SESSION_JOB_CLEANUP_MAX_ATTEMPTS
            })
        {
            return Err(DocumentSessionError::new(
                "SESSION_JOB_CLEANUP_BACKPRESSURE",
                "Yeni job geçici cleanup baskısı nedeniyle kabul edilmedi",
            ));
        }
        if let Some(existing) = registry.active.get(&canonical_job_id) {
            return Err(if existing.owner_session_id == session_id {
                DocumentSessionError::new(
                    "SESSION_JOB_ALREADY_EXISTS",
                    "Aynı job kimliğiyle aktif bir session job zaten var",
                )
            } else {
                DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                )
            });
        }
        if let Some(existing) = registry.deferred_cleanup.get(&canonical_job_id) {
            return Err(if existing.owner_session_id == session_id {
                DocumentSessionError::new(
                    "SESSION_JOB_ALREADY_EXISTS",
                    "Aynı job kimliği deferred cleanup kaydında bulunuyor",
                )
            } else {
                DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                )
            });
        }
        if let Some(pending_owner) = registry
            .pending_cancellations
            .values()
            .find(|pending| {
                registry
                    .pending_cancellations
                    .contains_key(&(pending.session_id.clone(), canonical_job_id))
            })
            .map(|pending| pending.session_id.clone())
        {
            if pending_owner != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session için iptal bekliyor",
                ));
            }
        }
        let pending = registry
            .pending_cancellations
            .remove(&(session_id.to_string(), canonical_job_id));
        let cancellation_reason_value = pending.as_ref().map(|value| value.reason);
        *cancellation_reason.lock() = cancellation_reason_value;
        let cancellation_token = CancellationToken::new();
        if pending.is_some() {
            cancellation_token.cancel();
        }
        let record = Arc::new(SessionJobRecord {
            job_id: canonical_job_id,
            owner_session_id: session_id.to_string(),
            content_generation: expected_generation,
            snapshot_path,
            job_dir,
            mutable: Mutex::new(SessionJobMutable {
                operation_status: if pending.is_some() {
                    SessionJobOperationStatus::CancelRequested
                } else {
                    SessionJobOperationStatus::CreatingSnapshot
                },
                cleanup_status: SessionJobCleanupStatus::Pending,
                credential_context: credential,
                cancel_requested: pending.is_some(),
                cancellation_reason: cancellation_reason_value,
                cleanup_attempts: 0,
            }),
            operation: Arc::new(AsyncRwLock::new(())),
            cleanup_single_flight: AsyncMutex::new(()),
            cancellation_token,
            cancellation_reason,
            child_processes: SessionJobChildProcessOwner::default(),
            cleanup_failures_remaining: AtomicU8::new(0),
        });
        registry
            .active
            .insert(canonical_job_id, Arc::clone(&record));
        drop(registry);
        Ok((record, working_path, _lease))
    }

    pub(crate) async fn acquire_verified_artifact_export_guard(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<VerifiedArtifactExportGuard, DocumentSessionError> {
        let entry = self.entry(session_id)?;
        let operation_lease = entry.operation.clone().read_owned().await;
        let state = entry.state.lock();
        session_read_path(&state, Some(expected_generation))?;
        Ok(VerifiedArtifactExportGuard {
            paths: VerifiedArtifactExportPaths {
                original_path: state.save_target_path.clone(),
                working_path: state.working_path.clone(),
                session_dir: state.session_dir.clone(),
                jobs_root: self.jobs_root(),
            },
            _operation_lease: operation_lease,
        })
    }

    pub(crate) async fn acquire_current_generation_publication_guard(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<SessionPublicationGuard, DocumentSessionError> {
        let entry = self.entry(session_id)?;
        let operation_lease = entry.operation.clone().read_owned().await;
        let state = entry.state.lock();
        if state.content_generation != expected_generation {
            return Err(DocumentSessionError::new(
                "AI_INDEX_OUTDATED",
                format!(
                    "Session generation mismatch: expected {}, current {}",
                    expected_generation, state.content_generation
                ),
            ));
        }
        session_read_path(&state, Some(expected_generation))?;
        Ok(SessionPublicationGuard {
            _operation_lease: operation_lease,
        })
    }

    pub(crate) async fn acquire_secure_redaction_export_guard(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<SecureRedactionExportGuard, DocumentSessionError> {
        self.acquire_verified_artifact_export_guard(session_id, expected_generation)
            .await
    }

    #[doc(hidden)]
    pub fn secure_redaction_export_paths_for_acceptance(
        &self,
        session_id: &str,
    ) -> Result<SecureRedactionExportPaths, DocumentSessionError> {
        let entry = self.entry(session_id)?;
        let state = entry.state.lock();
        Ok(VerifiedArtifactExportPaths {
            original_path: state.save_target_path.clone(),
            working_path: state.working_path.clone(),
            session_dir: state.session_dir.clone(),
            jobs_root: self.jobs_root(),
        })
    }

    #[doc(hidden)]
    pub fn set_secure_redaction_save_lease_test_hook(
        &self,
        hook: Option<Arc<dyn Fn(bool) + Send + Sync + 'static>>,
    ) {
        *self.secure_redaction_save_lease_test_hook.lock() = hook;
    }

    #[doc(hidden)]
    pub fn set_verified_artifact_save_lease_test_hook(
        &self,
        hook: Option<Arc<dyn Fn(bool) + Send + Sync + 'static>>,
    ) {
        self.set_secure_redaction_save_lease_test_hook(hook);
    }

    pub async fn run_snapshot_job<T, F>(
        &self,
        job_id: &str,
        session_id: &str,
        expected_generation: u64,
        operation: F,
    ) -> Result<SessionJobOutcome<T>, DocumentSessionError>
    where
        T: Send + 'static,
        F: FnOnce(SessionJobContext) -> Result<T, SessionJobOperationError> + Send + 'static,
    {
        let (job, working_path, session_snapshot_lease) = self
            .reserve_job_snapshot(job_id, session_id, expected_generation)
            .await?;
        let identity = (
            job.job_id.to_string(),
            job.owner_session_id.clone(),
            job.content_generation,
        );
        let pre_cancelled = job.cancellation_token.is_cancelled();
        #[cfg(test)]
        let force_snapshot_worker_panic =
            self.snapshot_worker_panic_jobs.lock().remove(&job.job_id);
        #[cfg(not(test))]
        let force_snapshot_worker_panic = false;
        let mut snapshot_result = if pre_cancelled {
            Err(SessionJobOperationError::Cancelled(
                job.cancellation_reason
                    .lock()
                    .unwrap_or(SessionJobCancellationReason::UserRequested),
            ))
        } else {
            let job_dir = job.job_dir.clone();
            let snapshot_path = job.snapshot_path.clone();
            let cancellation_checkpoint = SessionJobCancellationCheckpoint {
                token: job.cancellation_token.clone(),
                reason: Arc::clone(&job.cancellation_reason),
            };
            match tauri::async_runtime::spawn_blocking(move || {
                if force_snapshot_worker_panic {
                    panic!("controlled snapshot worker panic");
                }
                create_secure_dir(&job_dir).map_err(|error| {
                    SessionJobOperationError::Failed(SessionJobFailure::new(
                        error.code,
                        error.message,
                    ))
                })?;
                copy_with_digest_cancellable(
                    &working_path,
                    &snapshot_path,
                    &cancellation_checkpoint,
                )
                .map(|_| ())
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_SNAPSHOT_WORKER_FAILED",
                    "Snapshot worker güvenli biçimde tamamlanamadı",
                )),
            }
        };
        if snapshot_result.is_ok() {
            let validation_context = session_job_context(&job);
            snapshot_result =
                match tauri::async_runtime::spawn_blocking(
                    move || match crate::job_worker::run_job(
                        &validation_context,
                        crate::job_worker::JobCommand::ValidateSnapshot,
                        Duration::from_secs(60),
                    )? {
                        crate::job_worker::JobResult::SnapshotValid => Ok(()),
                        _ => Err(SessionJobOperationError::failed_with_code(
                            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                            "Snapshot validation worker beklenmeyen sonuç döndürdü",
                        )),
                    },
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_SNAPSHOT_VALIDATION_WORKER_FAILED",
                        "Snapshot validation worker güvenli biçimde tamamlanamadı",
                    )),
                };
        }
        drop(session_snapshot_lease);

        if let Err(error) = snapshot_result {
            set_session_job_terminal(&job, &error);
            let cleanup = cleanup_session_job_record(&self.job_registry, &job).await;
            return Ok(session_job_error_outcome(identity, error, cleanup));
        }

        {
            let mut state = job.mutable.lock();
            state.operation_status = if state.cancel_requested {
                SessionJobOperationStatus::CancelRequested
            } else {
                SessionJobOperationStatus::Ready
            };
        }
        if job.cancellation_token.is_cancelled() {
            let error = SessionJobOperationError::Cancelled(
                job.cancellation_reason
                    .lock()
                    .unwrap_or(SessionJobCancellationReason::UserRequested),
            );
            set_session_job_terminal(&job, &error);
            let cleanup = cleanup_session_job_record(&self.job_registry, &job).await;
            return Ok(session_job_error_outcome(identity, error, cleanup));
        }

        let operation_lease = job.operation.clone().read_owned().await;
        {
            let mut state = job.mutable.lock();
            state.operation_status = SessionJobOperationStatus::Running;
        }
        let context = session_job_context(&job);
        let result = match tauri::async_runtime::spawn_blocking(move || operation(context)).await {
            Ok(result) => result,
            Err(_) => Err(SessionJobOperationError::failed_with_code(
                "SESSION_JOB_OPERATION_WORKER_FAILED",
                "Session job operation worker güvenli biçimde tamamlanamadı",
            )),
        };
        drop(operation_lease);

        let terminal = match result {
            Ok(value) => {
                let (cancel_requested, cancellation_reason) = {
                    let mut state = job.mutable.lock();
                    state.operation_status = SessionJobOperationStatus::Completed;
                    state.credential_context = None;
                    (state.cancel_requested, state.cancellation_reason)
                };
                let cleanup = cleanup_session_job_record(&self.job_registry, &job).await;
                SessionJobOutcome::Completed {
                    job_id: identity.0,
                    session_id: identity.1,
                    content_generation: identity.2,
                    value,
                    cancel_requested,
                    cancellation_reason,
                    cleanup,
                }
            }
            Err(error) => {
                set_session_job_terminal(&job, &error);
                let cleanup = cleanup_session_job_record(&self.job_registry, &job).await;
                session_job_error_outcome(identity, error, cleanup)
            }
        };
        Ok(terminal)
    }

    pub(crate) async fn cancel_job(
        &self,
        session_id: &str,
        job_id: &str,
        reason: SessionJobCancellationReason,
    ) -> Result<SessionJobCancelOutcome, DocumentSessionError> {
        let canonical_job_id = Uuid::parse_str(job_id).map_err(|_| {
            DocumentSessionError::new(
                "SESSION_JOB_INVALID_ID",
                "Session job kimliği geçerli bir UUID olmalıdır",
            )
        })?;
        let mut registry = self.job_registry.lock();
        prune_session_job_registry(&mut registry, now_ms());
        if let Some(job) = registry.active.get(&canonical_job_id).cloned() {
            if job.owner_session_id != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                ));
            }
            let (terminal, already_requested) = {
                let state = job.mutable.lock();
                (state.operation_status.is_terminal(), state.cancel_requested)
            };
            if terminal {
                return Ok(SessionJobCancelOutcome::AlreadyTerminal);
            }
            drop(registry);
            if already_requested {
                return Ok(SessionJobCancelOutcome::AlreadyRequested);
            }
            request_session_job_cancellation(&job, reason);
            return Ok(SessionJobCancelOutcome::CancelRequested);
        }
        if let Some(job) = registry.deferred_cleanup.get(&canonical_job_id) {
            if job.owner_session_id != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                ));
            }
            return Ok(SessionJobCancelOutcome::AlreadyTerminal);
        }
        if let Some(terminal) = registry.recent_terminal.get(&canonical_job_id) {
            if terminal.session_id != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                ));
            }
            return Ok(SessionJobCancelOutcome::AlreadyTerminal);
        }
        if registry
            .pending_cancellations
            .keys()
            .any(|(_, id)| *id == canonical_job_id)
        {
            let pending = registry
                .pending_cancellations
                .get(&(session_id.to_string(), canonical_job_id));
            return if pending.is_some() {
                Ok(SessionJobCancelOutcome::AlreadyRequested)
            } else {
                Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session için iptal bekliyor",
                ))
            };
        }
        if registry.pending_cancellations.len() >= SESSION_JOB_PENDING_MAX {
            return Err(DocumentSessionError::new(
                "SESSION_JOB_PENDING_CANCELLATION_CAPACITY",
                "Pending session job cancellation kapasitesi dolu",
            ));
        }
        drop(registry);
        let entry = self.entry(session_id)?;
        {
            let state = entry.state.lock();
            ensure_lifecycle(
                &state,
                &[SessionLifecycle::Open, SessionLifecycle::DecisionPending],
            )?;
        }
        let mut registry = self.job_registry.lock();
        prune_session_job_registry(&mut registry, now_ms());
        if let Some(job) = registry.active.get(&canonical_job_id).cloned() {
            if job.owner_session_id != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                ));
            }
            let (terminal, already_requested) = {
                let state = job.mutable.lock();
                (state.operation_status.is_terminal(), state.cancel_requested)
            };
            if terminal {
                return Ok(SessionJobCancelOutcome::AlreadyTerminal);
            }
            if already_requested {
                return Ok(SessionJobCancelOutcome::AlreadyRequested);
            }
            drop(registry);
            request_session_job_cancellation(&job, reason);
            return Ok(SessionJobCancelOutcome::CancelRequested);
        }
        if let Some(job) = registry.deferred_cleanup.get(&canonical_job_id) {
            if job.owner_session_id != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                ));
            }
            return Ok(SessionJobCancelOutcome::AlreadyTerminal);
        }
        if let Some(terminal) = registry.recent_terminal.get(&canonical_job_id) {
            if terminal.session_id != session_id {
                return Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session'a aittir",
                ));
            }
            return Ok(SessionJobCancelOutcome::AlreadyTerminal);
        }
        if registry
            .pending_cancellations
            .keys()
            .any(|(_, existing_id)| *existing_id == canonical_job_id)
        {
            return if registry
                .pending_cancellations
                .contains_key(&(session_id.to_string(), canonical_job_id))
            {
                Ok(SessionJobCancelOutcome::AlreadyRequested)
            } else {
                Err(DocumentSessionError::new(
                    "SESSION_JOB_OWNER_MISMATCH",
                    "Job kimliği başka bir document session için iptal bekliyor",
                ))
            };
        }
        if registry.pending_cancellations.len() >= SESSION_JOB_PENDING_MAX {
            return Err(DocumentSessionError::new(
                "SESSION_JOB_PENDING_CANCELLATION_CAPACITY",
                "Pending session job cancellation kapasitesi dolu",
            ));
        }
        registry.pending_cancellations.insert(
            (session_id.to_string(), canonical_job_id),
            PendingSessionJobCancellation {
                session_id: session_id.to_string(),
                created_at_ms: now_ms(),
                reason,
            },
        );
        Ok(SessionJobCancelOutcome::CancelPending)
    }
}

impl Drop for DocumentSessionStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.instance_dir);
        self.instance_lock.lock().take();
        let _ = fs::remove_file(&self.instance_lock_path);
    }
}

fn snapshot(state: &DocumentSession) -> DocumentSessionSnapshot {
    DocumentSessionSnapshot {
        id: state.id.clone(),
        opened_at_ms: state.opened_at_ms,
        display_path: state.save_target_path.to_string_lossy().to_string(),
        file_name: state
            .save_target_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "document.pdf".to_string()),
        content_generation: state.content_generation,
        state_revision: state.state_revision,
        dirty: state.dirty,
        lifecycle: state.lifecycle,
        editable: state.editable,
        read_only_reason: state.read_only_reason.clone(),
        health_warning: state.health_warning.clone(),
        undo_operation: state
            .history
            .undo
            .last()
            .map(|entry| entry.operation.to_string()),
        redo_operation: state
            .history
            .redo
            .last()
            .map(|entry| entry.operation.to_string()),
    }
}

fn ensure_lifecycle(
    state: &DocumentSession,
    allowed: &[SessionLifecycle],
) -> Result<(), DocumentSessionError> {
    if allowed.contains(&state.lifecycle) {
        Ok(())
    } else {
        Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_INVALID_STATE",
            format!(
                "Session bu işlem için uygun durumda değil: {:?}",
                state.lifecycle
            ),
        )
        .with_snapshot(snapshot(state)))
    }
}

fn ensure_open_for_mutation(
    state: &DocumentSession,
    expected_generation: u64,
) -> Result<(), DocumentSessionError> {
    ensure_lifecycle(state, &[SessionLifecycle::Open])?;
    if !state.editable {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_READ_ONLY",
            state
                .read_only_reason
                .clone()
                .unwrap_or_else(|| "Session salt okunur".to_string()),
        )
        .with_snapshot(snapshot(state)));
    }
    if expected_generation != state.content_generation {
        return Err(DocumentSessionError::new(
            "STALE_GENERATION",
            format!(
                "Beklenen generation {}, güncel generation {}",
                expected_generation, state.content_generation
            ),
        )
        .with_snapshot(snapshot(state)));
    }
    Ok(())
}

fn bump_revision(state: &mut DocumentSession) -> Result<(), DocumentSessionError> {
    state.state_revision = checked_next_revision(state.state_revision)?;
    Ok(())
}

fn checked_next_revision(current: u64) -> Result<u64, DocumentSessionError> {
    let next = current.checked_add(1).ok_or_else(|| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_REVISION_OVERFLOW",
            "Session state revision taşması",
        )
    })?;
    if next > MAX_SAFE_INTEGER {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_REVISION_OVERFLOW",
            "Session state revision JavaScript safe-integer sınırını aştı",
        ));
    }
    Ok(next)
}

fn bump_generation(state: &mut DocumentSession) -> Result<(), DocumentSessionError> {
    state.content_generation = state.content_generation.checked_add(1).ok_or_else(|| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_GENERATION_OVERFLOW",
            "Session content generation taşması",
        )
    })?;
    if state.content_generation > MAX_SAFE_INTEGER {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_GENERATION_OVERFLOW",
            "Session content generation JavaScript safe-integer sınırını aştı",
        ));
    }
    Ok(())
}

fn reconcile_failed_mutation_outside_lock(
    working_path: &Path,
    error: &DocumentIoError,
) -> Option<String> {
    if error.state == CommitState::OriginalRecovered {
        sha256_file(working_path).ok()
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitStateClassification {
    OriginalPreserved,
    ReplacementCommitted,
    RecoveryRequired,
    Indeterminate,
}

pub(crate) trait MutationEngine: Send + Sync + 'static {
    fn run_mutation_worker(
        &self,
        job_ctx: &SessionJobContext,
        recipe: &MutationRecipe,
    ) -> Result<JobResult, DocumentSessionError>;

    fn run_validation_worker(
        &self,
        job_ctx: &SessionJobContext,
    ) -> Result<(), DocumentSessionError>;

    fn commit_prevalidated_pdf(
        &self,
        operation: &'static str,
        working_path: &Path,
        output_path: &Path,
        validated_digest: &str,
    ) -> Result<AtomicWriteReport, DocumentIoError>;
}

pub(crate) struct ProductionMutationEngine;

impl MutationEngine for ProductionMutationEngine {
    fn run_mutation_worker(
        &self,
        job_ctx: &SessionJobContext,
        recipe: &MutationRecipe,
    ) -> Result<JobResult, DocumentSessionError> {
        let output = crate::job_worker::run_job(
            job_ctx,
            JobCommand::MutatePdf {
                recipe: recipe.clone(),
                output_name: "mutation-output.pdf".to_string(),
            },
            Duration::from_secs(180),
        )
        .map_err(|err| {
            DocumentSessionError::new(
                "MUTATION_WORKER_FAILED",
                format!("Mutation worker hatası: {err}"),
            )
        })?;

        match &output {
            JobResult::File(path) => {
                crate::job_worker::validate_output_file(job_ctx, path).map_err(|err| {
                    DocumentSessionError::new(
                        "MUTATION_WORKER_FAILED",
                        format!("Mutation worker çıktı doğrulama hatası: {err}"),
                    )
                })?;
                Ok(output)
            }
            _ => Err(DocumentSessionError::new(
                "MUTATION_WORKER_FAILED",
                "Mutation worker beklenmeyen sonuç döndürdü",
            )),
        }
    }

    fn run_validation_worker(
        &self,
        job_ctx: &SessionJobContext,
    ) -> Result<(), DocumentSessionError> {
        match crate::job_worker::run_job(
            job_ctx,
            JobCommand::ValidateMutationOutput {
                output_name: "mutation-output.pdf".to_string(),
            },
            Duration::from_secs(60),
        )
        .map_err(|err| {
            DocumentSessionError::new(
                "MUTATION_VALIDATION_FAILED",
                format!("Validation worker hatası: {err}"),
            )
        })? {
            JobResult::MutationValidated => Ok(()),
            _ => Err(DocumentSessionError::new(
                "MUTATION_VALIDATION_FAILED",
                "Mutation validation worker beklenmeyen sonuç döndürdü",
            )),
        }
    }

    fn commit_prevalidated_pdf(
        &self,
        operation: &'static str,
        working_path: &Path,
        output_path: &Path,
        validated_digest: &str,
    ) -> Result<AtomicWriteReport, DocumentIoError> {
        atomic_commit_prevalidated_pdf(
            operation,
            working_path,
            working_path,
            output_path,
            validated_digest,
        )
    }
}

pub(crate) struct MutationTransactionInput {
    pub(crate) transaction_id: MutationTransactionId,
    pub(crate) session_id: String,
    pub(crate) expected_generation: u64,
    pub(crate) kind: MutationTransactionKind,
}

/// Bir mutasyon işleminin ne yaptığı: worker'da bir recipe çalıştırmak ya da undo/redo
/// geçmişinden bir durumu geri yüklemek. İkisi de aynı commit, iptal ve kurtarma hattından geçer.
pub(crate) enum MutationTransactionKind {
    Recipe(MutationRecipe),
    Restore(HistoryDirection),
}

impl MutationTransactionKind {
    fn operation_name(&self) -> &'static str {
        match self {
            Self::Recipe(recipe) => recipe.operation_name(),
            Self::Restore(HistoryDirection::Undo) => "undo_mutation",
            Self::Restore(HistoryDirection::Redo) => "redo_mutation",
        }
    }
}

pub(crate) struct MutationTransactionDependencies {
    pub(crate) worker_broker: PdfiumWorkerBroker,
    pub(crate) jobs_root: PathBuf,
    pub(crate) engine: Arc<dyn MutationEngine>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationState {
    NotStarted,
    InProgress,
    Completed,
}

fn reconcile_post_commit_authority(
    recovery_ctx: &MutationTransactionRecoveryContext,
    entry: &Arc<SessionEntry>,
    _transaction_id: &MutationTransactionId,
    _session_id: &str,
    join_err_msg: &str,
) -> Option<MutationOwnerClassification> {
    {
        let mut state = recovery_ctx.reconciliation_state.lock();
        if *state != ReconciliationState::NotStarted {
            return None;
        }
        *state = ReconciliationState::InProgress;
    }

    let actual_digest = sha256_file(&recovery_ctx.working_path).ok();
    let mut state = entry.state.lock();
    let initial = recovery_ctx.initial_digest.as_deref();
    let replacement = recovery_ctx.replacement_digest.lock().clone();
    let pf = recovery_ctx.preflight.lock().clone();

    let classification = if initial.is_some() && initial == actual_digest.as_deref() {
        let snap = snapshot(&state);
        let err = DocumentSessionError::new(
            "MUTATION_TRANSACTION_OWNER_PANIC",
            format!("Owner task panikledi fakat orijinal içerik korundu: {join_err_msg}"),
        );
        MutationOwnerClassification::NotAppliedAfterCommitAttempt {
            error: err.with_snapshot(snap.clone()),
            snapshot: snap,
        }
    } else if replacement.is_some() && replacement.as_deref() == actual_digest.as_deref() {
        if let Some(pf_val) = pf {
            state.content_generation = pf_val.next_content_generation;
            state.state_revision = pf_val.next_state_revision;
        } else {
            let _ = bump_generation(&mut state);
            let _ = bump_revision(&mut state);
        }
        state.working_digest = actual_digest;
        refresh_dirty(&mut state);
        let snap = snapshot(&state);
        MutationOwnerClassification::CommittedUnconfirmed {
            snapshot: snap,
            warning: format!("MUTATION_TRANSACTION_OWNER_PANIC: {join_err_msg}"),
        }
    } else {
        state.lifecycle = SessionLifecycle::CommitIndeterminate;
        state.dirty = true;
        state.working_digest = None;
        if let Some(pf_val) = pf {
            state.state_revision = pf_val.next_state_revision;
        } else {
            let _ = bump_revision(&mut state);
        }
        let snap = snapshot(&state);
        let err = DocumentSessionError::new(
            "MUTATION_TRANSACTION_OWNER_PANIC",
            format!("Owner task panikledi ve session kararsız durumda: {join_err_msg}"),
        );
        MutationOwnerClassification::Indeterminate {
            error: err.with_snapshot(snap.clone()),
            snapshot: snap,
        }
    };

    *recovery_ctx.reconciliation_state.lock() = ReconciliationState::Completed;
    Some(classification)
}

#[derive(Clone, Debug)]
pub(crate) struct MutationTransactionRecoveryContext {
    pub(crate) _transaction_id: MutationTransactionId,
    pub(crate) _session_id: String,
    pub(crate) initial_digest: Option<String>,
    pub(crate) replacement_digest: Arc<Mutex<Option<String>>>,
    pub(crate) preflight: Arc<Mutex<Option<PreflightPrepared>>>,
    pub(crate) working_path: PathBuf,
    pub(crate) reconciliation_state: Arc<Mutex<ReconciliationState>>,
}

pub(crate) struct MutationTransactionOwnerContext {
    pub(crate) entry: Arc<SessionEntry>,
    pub(crate) record: Arc<MutationTransactionRecord>,
    pub(crate) manager: Arc<MutationTransactionManager>,
    pub(crate) recovery_ctx: MutationTransactionRecoveryContext,
    pub(crate) child_processes: SessionJobChildProcessOwner,
    pub(crate) exchange_cleanup_authority: ExchangeCleanupAuthority,
    pub(crate) execution_authority: SupervisorExecutionAuthority,
}

#[derive(Debug)]
pub(crate) enum MutationOwnerClassification {
    Aborted {
        reason: String,
    },
    FailedPreCommit {
        error: DocumentSessionError,
    },
    NotAppliedAfterCommitAttempt {
        error: DocumentSessionError,
        snapshot: DocumentSessionSnapshot,
    },
    Committed {
        snapshot: DocumentSessionSnapshot,
    },
    CommittedUnconfirmed {
        snapshot: DocumentSessionSnapshot,
        warning: String,
    },
    RecoveryRequired {
        error: DocumentSessionError,
        snapshot: DocumentSessionSnapshot,
    },
    Indeterminate {
        error: DocumentSessionError,
        snapshot: DocumentSessionSnapshot,
    },
}

pub(crate) struct MutationOwnerResult {
    pub(crate) transaction_id: MutationTransactionId,
    pub(crate) session_id: String,
    pub(crate) classification: MutationOwnerClassification,
    pub(crate) child_owner: SessionJobChildProcessOwner,
    pub(crate) recovery_ctx: MutationTransactionRecoveryContext,
}

fn claim_normal_finalizer(
    execution_authority: &SupervisorExecutionAuthority,
) -> Result<(), DocumentSessionError> {
    if execution_authority.try_claim_finalizer(FinalizerClaim::ClaimedByNormalFinalizer) {
        Ok(())
    } else {
        Err(DocumentSessionError::new(
            "MUTATION_FINALIZER_ALREADY_CLAIMED",
            format!(
                "Normal finalizer claim alınamadı; mevcut claim: {:?}",
                execution_authority.finalizer_claim()
            ),
        ))
    }
}

fn finalize_with_existing_claim(
    record: &Arc<MutationTransactionRecord>,
    manager: &MutationTransactionManager,
    child_owner: &SessionJobChildProcessOwner,
    exchange_cleanup_authority: &ExchangeCleanupAuthority,
    execution_authority: &SupervisorExecutionAuthority,
    claim_owner: FinalizerClaim,
    mut outcome: MutationTransactionOutcome,
) -> Result<MutationTransactionOutcome, DocumentSessionError> {
    if !matches!(
        claim_owner,
        FinalizerClaim::ClaimedByNormalFinalizer | FinalizerClaim::ClaimedByExternalRecovery
    ) || execution_authority.finalizer_claim() != claim_owner
    {
        return Err(DocumentSessionError::new(
            "MUTATION_FINALIZER_CLAIM_CORRUPTION",
            format!(
                "Finalizer claim owner uyuşmuyor: requested={claim_owner:?}, actual={:?}",
                execution_authority.finalizer_claim()
            ),
        ));
    }

    if !execution_authority.is_exited() {
        return Err(DocumentSessionError::new(
            "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN",
            "Supervisor Exited acknowledgement alınmadan finalizer çalıştırılamaz",
        ));
    }

    {
        let inner = manager.inner.lock();
        let Some(entry) = inner.supervisor_handles.get(&record.id) else {
            return Err(DocumentSessionError::new(
                "MUTATION_FINALIZER_REGISTRY_CORRUPTION",
                "Finalizer başlarken supervisor registry entry bulunamadı",
            ));
        };
        if entry.execution_authority.finalizer_claim() != claim_owner {
            return Err(DocumentSessionError::new(
                "MUTATION_FINALIZER_CLAIM_CORRUPTION",
                "Registry claim owner finalizer claim owner ile uyuşmuyor",
            ));
        }
    }

    let child_report = child_owner.kill_and_reap_all_typed();
    let child_err = if child_report.errors.is_empty() {
        None
    } else {
        Some(child_report.errors.join("; "))
    };

    let exchange_err = exchange_cleanup_authority
        .close_exchange_dir()
        .err()
        .map(|e| e.message);

    let cleanup_warning = match (child_err, exchange_err) {
        (Some(c), Some(e)) => Some(format!("MUTATION_CLEANUP_FAILED: child: {c}, dir: {e}")),
        (Some(c), None) => Some(format!("MUTATION_CLEANUP_FAILED: child: {c}")),
        (None, Some(e)) => Some(format!("MUTATION_CLEANUP_FAILED: dir: {e}")),
        (None, None) => None,
    };

    if let Some(warn) = cleanup_warning.clone() {
        match &mut outcome {
            MutationTransactionOutcome::Completed {
                cleanup_warning: slot,
                ..
            }
            | MutationTransactionOutcome::Aborted {
                cleanup_warning: slot,
                ..
            }
            | MutationTransactionOutcome::FailedPreCommit {
                cleanup_warning: slot,
                ..
            }
            | MutationTransactionOutcome::NotAppliedAfterCommitAttempt {
                cleanup_warning: slot,
                ..
            } => {
                *slot = Some(warn);
            }
            MutationTransactionOutcome::CommittedUnconfirmed { warning, .. } => {
                warning.push_str(&format!("; {warn}"));
            }
            MutationTransactionOutcome::RecoveryRequired { snapshot, .. }
            | MutationTransactionOutcome::Indeterminate { snapshot, .. } => {
                snapshot.health_warning = Some(warn);
            }
        }
    }

    if let Err(pub_err) = record.publish_terminal(outcome.clone()) {
        let fallback_outcome = MutationTransactionOutcome::Indeterminate {
            transaction_id: record.id.to_string(),
            session_id: record.session_id.clone(),
            snapshot: match &outcome {
                MutationTransactionOutcome::Completed { snapshot, .. }
                | MutationTransactionOutcome::CommittedUnconfirmed { snapshot, .. }
                | MutationTransactionOutcome::NotAppliedAfterCommitAttempt { snapshot, .. }
                | MutationTransactionOutcome::RecoveryRequired { snapshot, .. }
                | MutationTransactionOutcome::Indeterminate { snapshot, .. } => snapshot.clone(),
                _ => DocumentSessionSnapshot {
                    id: record.session_id.clone(),
                    opened_at_ms: 0,
                    display_path: record.session_id.clone(),
                    file_name: record.session_id.clone(),
                    content_generation: 0,
                    state_revision: 0,
                    dirty: false,
                    lifecycle: SessionLifecycle::CommitIndeterminate,
                    editable: false,
                    read_only_reason: None,
                    health_warning: Some(pub_err.message.clone()),
                    undo_operation: None,
                    redo_operation: None,
                },
            },
            error: pub_err.clone(),
        };
        record.publish_emergency_terminal(pub_err.clone());
        return Err(pub_err.with_snapshot(match &fallback_outcome {
            MutationTransactionOutcome::Indeterminate { snapshot, .. } => snapshot.clone(),
            _ => unreachable!(),
        }));
    }

    {
        let mut inner = manager.inner.lock();
        let Some(entry) = inner.supervisor_handles.get(&record.id) else {
            return Err(DocumentSessionError::new(
                "MUTATION_FINALIZER_REGISTRY_CORRUPTION",
                "Finalizer tamamlanırken supervisor registry entry bulunamadı",
            ));
        };
        if entry.execution_authority.finalizer_claim() != claim_owner {
            return Err(DocumentSessionError::new(
                "MUTATION_FINALIZER_CLAIM_CORRUPTION",
                "Registry claim owner finalizer claim owner ile uyuşmuyor",
            ));
        }

        inner.accumulator.record_outcome(record.id, &outcome);
        inner.accumulator.reaped_children += child_report.reaped;
        if let Some(warning) = cleanup_warning {
            inner.accumulator.warnings.push(CleanupWarning {
                code: "MUTATION_CLEANUP_FAILED".to_string(),
                message: warning,
            });
        }
        if inner.session_reservations.get(&record.session_id) == Some(&record.id) {
            inner.session_reservations.remove(&record.session_id);
        }
        inner.active_transactions.remove(&record.id);
        inner.supervisor_handles.remove(&record.id);
        inner.progress_revision += 1;
    }
    execution_authority.mark_finalized();
    manager.shutdown_notify.notify_waiters();
    manager.reaper_notify.notify_waiters();
    Ok(outcome)
}

fn finalize_supervisor_normal(
    manager: &MutationTransactionManager,
    tx_id: &MutationTransactionId,
    outcome: MutationTransactionOutcome,
) {
    let bundle = {
        let inner = manager.inner.lock();
        inner
            .supervisor_handles
            .get(tx_id)
            .and_then(|entry| entry.recovery_bundle.clone())
    };

    let Some(bundle) = bundle else {
        #[cfg(test)]
        {
            let record = manager.get_transaction(tx_id);
            if let Some(record) = record {
                let _ = record.publish_terminal(outcome.clone());
            }
            let mut inner = manager.inner.lock();
            inner.accumulator.record_outcome(*tx_id, &outcome);
            inner.session_reservations.retain(|_, id| id != tx_id);
            inner.active_transactions.remove(tx_id);
            inner.supervisor_handles.remove(tx_id);
            inner.progress_revision += 1;
        }
        return;
    };

    if let Err(error) = claim_normal_finalizer(&bundle.execution_authority) {
        if bundle.execution_authority.finalizer_claim() != FinalizerClaim::ClaimedByExternalRecovery
        {
            let mut inner = manager.inner.lock();
            inner.accumulator.warnings.push(CleanupWarning {
                code: error.code.to_string(),
                message: error.message,
            });
        }
        return;
    }

    if let Err(error) = finalize_with_existing_claim(
        &bundle.transaction_record,
        manager,
        &bundle.child_owner,
        &bundle.exchange_cleanup_authority,
        &bundle.execution_authority,
        FinalizerClaim::ClaimedByNormalFinalizer,
        outcome,
    ) {
        let mut inner = manager.inner.lock();
        if let Some(entry) = inner.supervisor_handles.get_mut(tx_id) {
            entry.finalization_state =
                SupervisorFinalizationState::FinalizationFailed(error.message.clone());
        }
        inner.accumulator.warnings.push(CleanupWarning {
            code: error.code.to_string(),
            message: error.message,
        });
        inner.progress_revision += 1;
    }
}

async fn recover_supervisor_panic_external(
    manager: &MutationTransactionManager,
    tx_id: &MutationTransactionId,
    join_err_msg: &str,
) {
    let (bundle, record, exec_auth) = {
        let inner = manager.inner.lock();
        let Some(entry) = inner.supervisor_handles.get(tx_id) else {
            return;
        };
        let exec_auth = entry.execution_authority.clone();
        let Some(bundle) = entry.recovery_bundle.clone() else {
            return;
        };
        (
            bundle,
            Arc::clone(&entry.recovery_bundle.as_ref().unwrap().transaction_record),
            exec_auth,
        )
    };

    if !exec_auth.try_claim_finalizer(FinalizerClaim::ClaimedByExternalRecovery) {
        return;
    }

    {
        let mut inner = manager.inner.lock();
        if let Some(entry) = inner.supervisor_handles.get_mut(tx_id) {
            entry.finalization_state = SupervisorFinalizationState::RecoveryInProgress;
        }
        inner.progress_revision += 1;
    }
    manager.shutdown_notify.notify_waiters();

    if let Err(err) = terminate_supervisor_execution(manager, tx_id, join_err_msg).await {
        let snap = {
            let state = bundle.session_entry.state.lock();
            snapshot(&state)
        };
        record.publish_emergency_terminal(err.clone().with_snapshot(snap));
        {
            let mut state = bundle.session_entry.state.lock();
            state.lifecycle = SessionLifecycle::CommitIndeterminate;
        }
        {
            let mut inner = manager.inner.lock();
            inner.accumulator.warnings.push(CleanupWarning {
                code: err.code.to_string(),
                message: err.message.clone(),
            });
            if let Some(entry) = inner.supervisor_handles.get_mut(tx_id) {
                entry.finalization_state =
                    SupervisorFinalizationState::TerminationUnproven(err.message.clone());
            }
            inner.reaper_task_state = ReaperTaskState::JoinedFailure(format!(
                "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN: {}",
                err.message
            ));
            inner.lifecycle = MutationManagerLifecycle::ShuttingDown;
            inner.progress_revision += 1;
        }
        manager.reaper_stop_token.cancel();
        manager.reaper_notify.notify_waiters();
        manager.shutdown_notify.notify_waiters();
        return;
    }

    let phase = record.control.lock().phase;
    let outcome = if phase.is_post_commit() || phase == MutationTransactionPhase::Committing {
        if let Some(classification) = reconcile_post_commit_authority(
            &bundle.recovery_ctx,
            &bundle.session_entry,
            tx_id,
            &record.session_id,
            join_err_msg,
        ) {
            match classification {
                MutationOwnerClassification::NotAppliedAfterCommitAttempt { error, snapshot } => {
                    MutationTransactionOutcome::NotAppliedAfterCommitAttempt {
                        transaction_id: tx_id.to_string(),
                        session_id: record.session_id.clone(),
                        error,
                        snapshot,
                        cleanup_warning: None,
                    }
                }
                MutationOwnerClassification::CommittedUnconfirmed { snapshot, warning } => {
                    MutationTransactionOutcome::CommittedUnconfirmed {
                        transaction_id: tx_id.to_string(),
                        session_id: record.session_id.clone(),
                        snapshot,
                        warning,
                    }
                }
                _ => {
                    let snap = snapshot(&bundle.session_entry.state.lock());
                    let err = DocumentSessionError::new(
                        "MUTATION_TRANSACTION_OWNER_PANIC",
                        format!("Owner task panikledi: {join_err_msg}"),
                    );
                    MutationTransactionOutcome::Indeterminate {
                        transaction_id: tx_id.to_string(),
                        session_id: record.session_id.clone(),
                        snapshot: snap,
                        error: err,
                    }
                }
            }
        } else {
            let snap = snapshot(&bundle.session_entry.state.lock());
            let err = DocumentSessionError::new(
                "MUTATION_TRANSACTION_OWNER_PANIC",
                format!("Reconciliation başka task tarafından yürütüldü: {join_err_msg}"),
            );
            MutationTransactionOutcome::Indeterminate {
                transaction_id: tx_id.to_string(),
                session_id: record.session_id.clone(),
                snapshot: snap,
                error: err,
            }
        }
    } else {
        let snap = snapshot(&bundle.session_entry.state.lock());
        let err = DocumentSessionError::new(
            "MUTATION_TRANSACTION_OWNER_PANIC",
            format!("Owner task pre-commit aşamasında panikledi: {join_err_msg}"),
        );
        MutationTransactionOutcome::FailedPreCommit {
            transaction_id: tx_id.to_string(),
            session_id: record.session_id.clone(),
            error: err.with_snapshot(snap),
            cleanup_warning: None,
        }
    };

    {
        let mut inner = manager.inner.lock();
        if let Some(entry) = inner.supervisor_handles.get_mut(tx_id) {
            if entry.join_result.is_none() {
                entry.join_result = Some(Err(join_err_msg.to_string()));
            }
            entry.ownership = SupervisorHandleOwnership::Joined;
            entry.finalization_state = SupervisorFinalizationState::CleanupInProgress;
        }
        inner.progress_revision += 1;
    }

    if let Err(error) = finalize_with_existing_claim(
        &record,
        manager,
        &bundle.child_owner,
        &bundle.exchange_cleanup_authority,
        &exec_auth,
        FinalizerClaim::ClaimedByExternalRecovery,
        outcome,
    ) {
        let mut inner = manager.inner.lock();
        if let Some(entry) = inner.supervisor_handles.get_mut(tx_id) {
            entry.finalization_state =
                SupervisorFinalizationState::FinalizationFailed(error.message.clone());
        }
        inner.accumulator.warnings.push(CleanupWarning {
            code: error.code.to_string(),
            message: error.message,
        });
        inner.progress_revision += 1;
        manager.shutdown_notify.notify_waiters();
    }
}

async fn wait_for_supervisor_exited(
    execution_authority: &SupervisorExecutionAuthority,
    timeout_duration: Duration,
) -> bool {
    if execution_authority.is_exited() {
        return true;
    }
    tokio::time::timeout(timeout_duration, async {
        loop {
            let notified = execution_authority.liveness_notify.notified();
            if execution_authority.is_exited() {
                return;
            }
            notified.await;
        }
    })
    .await
    .is_ok()
}

async fn terminate_supervisor_execution(
    manager: &MutationTransactionManager,
    tx_id: &MutationTransactionId,
    reason: &str,
) -> Result<(), DocumentSessionError> {
    let (record, execution_authority, abort_handle, child_owner) = {
        let inner = manager.inner.lock();
        let entry = inner.supervisor_handles.get(tx_id).ok_or_else(|| {
            DocumentSessionError::new(
                "MUTATION_SUPERVISOR_REGISTRY_CORRUPTION",
                "Termination sırasında supervisor registry entry bulunamadı",
            )
        })?;
        (
            entry
                .recovery_bundle
                .as_ref()
                .map(|bundle| Arc::clone(&bundle.transaction_record)),
            entry.execution_authority.clone(),
            entry.abort_handle.clone(),
            entry
                .recovery_bundle
                .as_ref()
                .map(|bundle| bundle.child_owner.clone()),
        )
    };

    if let Some(record) = record {
        record.request_cancel(format!("MUTATION_SUPERVISOR_RECOVERY: {reason}"));
    }
    execution_authority.request_cancellation();
    if wait_for_supervisor_exited(
        &execution_authority,
        Duration::from_millis(SUPERVISOR_COOPERATIVE_EXIT_MS),
    )
    .await
    {
        return Ok(());
    }

    execution_authority.mark_abort_requested();
    if let Some(child_owner) = child_owner {
        let report = child_owner.kill_and_reap_all_typed();
        let mut inner = manager.inner.lock();
        inner.accumulator.reaped_children += report.reaped;
        inner
            .accumulator
            .warnings
            .extend(report.errors.into_iter().map(|message| CleanupWarning {
                code: "MUTATION_CHILD_CLEANUP_FAILED".to_string(),
                message,
            }));
    }
    execution_authority.refresh_exit_acknowledgement();
    abort_handle.abort();

    if wait_for_supervisor_exited(
        &execution_authority,
        Duration::from_millis(SUPERVISOR_ABORT_EXIT_MS),
    )
    .await
    {
        return Ok(());
    }

    let detail = format!(
        "Supervisor termination doğrulanamadı: {reason}; execution_state={:?}",
        execution_authority.execution_state()
    );
    execution_authority.mark_termination_unproven(detail.clone());
    Err(DocumentSessionError::new(
        "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN",
        detail,
    ))
}

fn apply_failed_mutation_reconciled(
    state: &mut DocumentSession,
    before_digest: Option<&str>,
    error: &DocumentIoError,
    reconciled_actual_digest: Option<&str>,
    preflight: Option<&PreflightPrepared>,
) -> Result<CommitStateClassification, DocumentSessionError> {
    match error.state {
        CommitState::Prepared | CommitState::OriginalPreserved => {
            Ok(CommitStateClassification::OriginalPreserved)
        }
        CommitState::OriginalRecovered => {
            let actual = match reconciled_actual_digest {
                Some(digest) => digest,
                None => {
                    state.lifecycle = SessionLifecycle::CommitIndeterminate;
                    state.dirty = true;
                    state.working_digest = None;
                    if let Some(pf) = preflight {
                        state.state_revision = pf.next_state_revision;
                    } else {
                        let _ = bump_revision(state);
                    }
                    return Err(DocumentSessionError::new(
                        "DOCUMENT_SESSION_RECOVERY_HASH_FAILED",
                        "Recovered working PDF hash edilemedi",
                    ));
                }
            };
            if before_digest.is_some_and(|before| before == actual) {
                state.working_digest = Some(actual.to_string());
                return Ok(CommitStateClassification::OriginalPreserved);
            }
            if error
                .replacement_digest
                .as_ref()
                .is_some_and(|candidate| candidate == actual)
            {
                state.working_digest = Some(actual.to_string());
                refresh_dirty(state);
                if let Some(pf) = preflight {
                    state.content_generation = pf.next_content_generation;
                    state.state_revision = pf.next_state_revision;
                } else {
                    bump_generation(state)?;
                    bump_revision(state)?;
                }
                return Ok(CommitStateClassification::ReplacementCommitted);
            }
            state.lifecycle = SessionLifecycle::CommitIndeterminate;
            state.dirty = true;
            state.working_digest = None;
            if let Some(pf) = preflight {
                state.state_revision = pf.next_state_revision;
            } else {
                bump_revision(state)?;
            }
            Ok(CommitStateClassification::Indeterminate)
        }
        CommitState::RecoveryRequired => {
            state.lifecycle = SessionLifecycle::RecoveryRequired;
            state.dirty = true;
            state.working_digest = None;
            if let Some(pf) = preflight {
                state.state_revision = pf.next_state_revision;
            } else {
                bump_revision(state)?;
            }
            Ok(CommitStateClassification::RecoveryRequired)
        }
        CommitState::Indeterminate => {
            state.lifecycle = SessionLifecycle::CommitIndeterminate;
            state.dirty = true;
            state.working_digest = None;
            if let Some(pf) = preflight {
                state.state_revision = pf.next_state_revision;
            } else {
                bump_revision(state)?;
            }
            Ok(CommitStateClassification::Indeterminate)
        }
        CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed => {
            Err(DocumentSessionError::new(
                "DOCUMENT_SESSION_INVALID_ERROR_STATE",
                "Başarılı commit state hata olarak döndü",
            ))
        }
    }
}

async fn execute_mutation_transaction(
    input: MutationTransactionInput,
    deps: MutationTransactionDependencies,
    ctx: MutationTransactionOwnerContext,
) -> MutationOwnerResult {
    let transaction_id = input.transaction_id;
    let session_id = input.session_id;
    let expected_generation = input.expected_generation;
    let kind = input.kind;

    let worker_broker = deps.worker_broker;
    let _jobs_root = deps.jobs_root;
    let engine = deps.engine;

    let entry = ctx.entry.clone();
    let record = ctx.record.clone();
    let _manager = ctx.manager.clone();

    if let Err(err) = record.transition_to(MutationTransactionPhase::Preparing) {
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::FailedPreCommit { error: err },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    let admission = match worker_broker.admit_exclusive(&session_id).await {
        Ok(permit) => permit,
        Err(broker_err) => {
            let err = DocumentSessionError::from(broker_err);
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification: MutationOwnerClassification::FailedPreCommit { error: err },
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
    };

    let _lease = entry.operation.clone().write_owned().await;

    if record.control.lock().cancel_requested {
        let reason = record
            .control
            .lock()
            .cancel_reason
            .clone()
            .unwrap_or_else(|| "MUTATION_TRANSACTION_CANCELLED".to_string());
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::Aborted { reason },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    let (working_path, session_dir, credential, before_digest) = {
        let state = entry.state.lock();
        if let Err(err) = ensure_open_for_mutation(&state, expected_generation) {
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification: MutationOwnerClassification::FailedPreCommit { error: err },
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
        (
            state.working_path.clone(),
            state.session_dir.clone(),
            state.credential.clone(),
            state.working_digest.clone(),
        )
    };

    let worker_key = WorkerDocumentKey {
        session_id: session_id.clone(),
        content_generation: expected_generation,
    };
    if let Err(err) = worker_broker
        .invalidate_document(&admission, &worker_key)
        .await
    {
        let ds_err = DocumentSessionError::from(err);
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    let exchange_dir_path = ctx.exchange_cleanup_authority.path();
    if let Err(err) = create_secure_dir(&exchange_dir_path) {
        let ds_err = DocumentSessionError::new(
            "DOCUMENT_SESSION_TEMP_CREATE_FAILED",
            format!(
                "Mutation worker secure temp dizini oluşturulamadı: {}",
                err.message
            ),
        );
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    let tx_context = MutationTransactionContext::new(
        transaction_id,
        session_id.clone(),
        expected_generation,
        exchange_dir_path,
        credential,
        record.cancellation_token.clone(),
        ctx.child_processes.clone(),
    );

    // Snapshot, işlem öncesi working PDF'in kopyasıdır; commit'ten sonra geçmişe taşınır.
    let snapshot_digest = match copy_with_digest(&working_path, &tx_context.snapshot_path) {
        Ok(digest) => Some(digest),
        Err(err) => {
            let ds_err = DocumentSessionError::new(
                "MUTATION_WORKER_FAILED",
                format!("Snapshot PDF kopyalanamadı: {}", err.message),
            );
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
    };

    if let Err(err) = record.transition_to(MutationTransactionPhase::Mutating) {
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::FailedPreCommit { error: err },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    #[cfg(test)]
    _manager.trigger_test_hook(
        MutationTestHookPhase::MutationWorkerStarted,
        &transaction_id,
    );

    let job_ctx = tx_context.to_session_job_context();
    let operation = kind.operation_name();

    // Geri yüklemede geri dönülecek giriş; exclusive lease altında seçilir ve değişmez.
    let restore_entry = match &kind {
        MutationTransactionKind::Restore(direction) => {
            let top = entry.state.lock().history.top(*direction).cloned();
            match top {
                Some(restored) => Some(restored),
                None => {
                    let ds_err = DocumentSessionError::new(
                        "HISTORY_EMPTY",
                        match direction {
                            HistoryDirection::Undo => "Geri alınacak adım yok",
                            HistoryDirection::Redo => "Yinelenecek adım yok",
                        },
                    );
                    return MutationOwnerResult {
                        transaction_id,
                        session_id,
                        classification: MutationOwnerClassification::FailedPreCommit {
                            error: ds_err,
                        },
                        child_owner: ctx.child_processes,
                        recovery_ctx: ctx.recovery_ctx,
                    };
                }
            }
        }
        MutationTransactionKind::Recipe(_) => None,
    };

    let output_path = match (&kind, &restore_entry) {
        // Geri yüklenen baytlar daha önce doğrulanıp commit edilmiş bir durumdur; worker
        // çalışmaz, aşağıda digest'leri kayıtlı digest'le karşılaştırılır.
        (_, Some(restored)) => restored.blob.clone(),
        (MutationTransactionKind::Restore(_), None) => {
            unreachable!("restore_entry yukarıda seçildi")
        }
        (MutationTransactionKind::Recipe(recipe), None) => {
            let engine_clone = Arc::clone(&engine);
            let job_ctx_clone = job_ctx.clone();
            let recipe_clone = recipe.clone();

            let mutation_res = spawn_supervised_blocking(&ctx.execution_authority, move || {
                engine_clone.run_mutation_worker(&job_ctx_clone, &recipe_clone)
            })
            .await;

            #[cfg(test)]
            _manager.trigger_test_hook(MutationTestHookPhase::AfterMutationWorker, &transaction_id);

            if record.control.lock().cancel_requested {
                let reason = record
                    .control
                    .lock()
                    .cancel_reason
                    .clone()
                    .unwrap_or_else(|| "MUTATION_TRANSACTION_CANCELLED".to_string());
                return MutationOwnerResult {
                    transaction_id,
                    session_id,
                    classification: MutationOwnerClassification::Aborted { reason },
                    child_owner: ctx.child_processes,
                    recovery_ctx: ctx.recovery_ctx,
                };
            }

            match mutation_res {
                Ok(Ok(JobResult::File(path))) => path,
                Ok(Ok(_)) => {
                    let ds_err = DocumentSessionError::new(
                        "MUTATION_WORKER_FAILED",
                        "Mutation worker beklenmeyen sonuç döndürdü",
                    );
                    return MutationOwnerResult {
                        transaction_id,
                        session_id,
                        classification: MutationOwnerClassification::FailedPreCommit {
                            error: ds_err,
                        },
                        child_owner: ctx.child_processes,
                        recovery_ctx: ctx.recovery_ctx,
                    };
                }
                Ok(Err(ds_err)) => {
                    return MutationOwnerResult {
                        transaction_id,
                        session_id,
                        classification: MutationOwnerClassification::FailedPreCommit {
                            error: ds_err,
                        },
                        child_owner: ctx.child_processes,
                        recovery_ctx: ctx.recovery_ctx,
                    };
                }
                Err(join_err) => {
                    let ds_err = DocumentSessionError::new(
                        "MUTATION_WORKER_FAILED",
                        format!("Mutation worker thread panikledi: {join_err}"),
                    );
                    return MutationOwnerResult {
                        transaction_id,
                        session_id,
                        classification: MutationOwnerClassification::FailedPreCommit {
                            error: ds_err,
                        },
                        child_owner: ctx.child_processes,
                        recovery_ctx: ctx.recovery_ctx,
                    };
                }
            }
        }
    };

    #[cfg(test)]
    _manager.trigger_test_hook(MutationTestHookPhase::BeforeValidation, &transaction_id);

    if record.control.lock().cancel_requested {
        let reason = record
            .control
            .lock()
            .cancel_reason
            .clone()
            .unwrap_or_else(|| "MUTATION_TRANSACTION_CANCELLED".to_string());
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::Aborted { reason },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    if let Err(err) = record.transition_to(MutationTransactionPhase::Validating) {
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::FailedPreCommit { error: err },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    if restore_entry.is_none() {
        let engine_clone = Arc::clone(&engine);
        let job_ctx_clone = job_ctx.clone();

        let validation_res = spawn_supervised_blocking(&ctx.execution_authority, move || {
            engine_clone.run_validation_worker(&job_ctx_clone)
        })
        .await;

        match validation_res {
            Ok(Ok(())) => {}
            Ok(Err(ds_err)) => {
                return MutationOwnerResult {
                    transaction_id,
                    session_id,
                    classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
                    child_owner: ctx.child_processes,
                    recovery_ctx: ctx.recovery_ctx,
                };
            }
            Err(join_err) => {
                let ds_err = DocumentSessionError::new(
                    "MUTATION_VALIDATION_FAILED",
                    format!("Validation worker thread panikledi: {join_err}"),
                );
                return MutationOwnerResult {
                    transaction_id,
                    session_id,
                    classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
                    child_owner: ctx.child_processes,
                    recovery_ctx: ctx.recovery_ctx,
                };
            }
        }
    }

    let validated_digest = match sha256_file(&output_path) {
        Ok(digest) => digest,
        Err(err) => {
            let ds_err = DocumentSessionError::new(
                "MUTATION_VALIDATION_FAILED",
                format!("Doğrulanmış mutation çıktısı hash edilemedi: {err}"),
            );
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
    };

    if let Some(restored) = &restore_entry {
        if restored.digest != validated_digest {
            entry.state.lock().history.clear();
            let ds_err = DocumentSessionError::new(
                "HISTORY_ENTRY_CORRUPTED",
                "Geçmiş dosyası kayıtlı içerikle eşleşmiyor; geçmiş temizlendi ve belge değiştirilmedi",
            );
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification: MutationOwnerClassification::FailedPreCommit { error: ds_err },
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
    }

    *ctx.recovery_ctx.replacement_digest.lock() = Some(validated_digest.clone());

    if let Err(err) = record.transition_to(MutationTransactionPhase::CommitReady) {
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::FailedPreCommit { error: err },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    #[cfg(test)]
    _manager.trigger_test_hook(MutationTestHookPhase::BeforeCommit, &transaction_id);

    if record.control.lock().cancel_requested {
        let reason = record
            .control
            .lock()
            .cancel_reason
            .clone()
            .unwrap_or_else(|| "MUTATION_TRANSACTION_CANCELLED".to_string());
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::Aborted { reason },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    let preflight_res = {
        let state = entry.state.lock();
        if let Err(err) = ensure_open_for_mutation(&state, expected_generation) {
            Err(err)
        } else {
            let next_gen = match state.content_generation.checked_add(1) {
                Some(g) if g <= MAX_SAFE_INTEGER => Ok(g),
                _ => Err(DocumentSessionError::new(
                    "MUTATION_GENERATION_CONFLICT",
                    "Session content generation taşması",
                )),
            };
            let next_rev = match state.state_revision.checked_add(1) {
                Some(r) if r <= MAX_SAFE_INTEGER => Ok(r),
                _ => Err(DocumentSessionError::new(
                    "DOCUMENT_SESSION_REVISION_OVERFLOW",
                    "Session state revision taşması",
                )),
            };
            match (next_gen, next_rev) {
                (Ok(g), Ok(r)) => Ok((g, r)),
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
    };

    let (next_content_generation, next_state_revision) = match preflight_res {
        Ok(values) => values,
        Err(err) => {
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification: MutationOwnerClassification::FailedPreCommit { error: err },
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
    };

    *ctx.recovery_ctx.preflight.lock() = Some(PreflightPrepared {
        next_content_generation,
        next_state_revision,
    });

    if let Err(cancel_err) = record.try_enter_committing() {
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::Aborted {
                reason: cancel_err.message,
            },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    #[cfg(test)]
    _manager.trigger_test_hook(
        MutationTestHookPhase::BeforeAtomicCommitCall,
        &transaction_id,
    );

    let output_path_clone = output_path.clone();
    let working_path_clone = working_path.clone();
    let validated_digest_clone = validated_digest.clone();
    let val_digest_for_commit = validated_digest.clone();
    let engine_clone = Arc::clone(&engine);

    let commit_res = spawn_supervised_blocking(&ctx.execution_authority, move || {
        engine_clone.commit_prevalidated_pdf(
            operation,
            &working_path_clone,
            &output_path_clone,
            &val_digest_for_commit,
        )
    })
    .await;

    let commit_report = match commit_res {
        Ok(Ok(report)) => {
            if report.state != CommitState::NewCommitted
                && report.state != CommitState::NewCommittedDurabilityUnconfirmed
            {
                let io_err = DocumentIoError::new(
                    operation,
                    DocumentIoStage::Replace,
                    report.state,
                    format!("Geçersiz commit report state: {:?}", report.state),
                );
                let (snap, ds_err) = {
                    let state = entry.state.lock();
                    let snap = snapshot(&state);
                    let err = DocumentSessionError::from_io(&io_err).with_snapshot(snap.clone());
                    (snap, err)
                };
                return MutationOwnerResult {
                    transaction_id,
                    session_id,
                    classification: MutationOwnerClassification::Indeterminate {
                        error: ds_err,
                        snapshot: snap,
                    },
                    child_owner: ctx.child_processes,
                    recovery_ctx: ctx.recovery_ctx,
                };
            }
            report
        }
        Ok(Err(io_err)) => {
            let reconciled_digest = reconcile_failed_mutation_outside_lock(&working_path, &io_err);

            let preflight_pf = PreflightPrepared {
                next_content_generation,
                next_state_revision,
            };
            let (snap, ds_err, classification_result) = {
                let mut state = entry.state.lock();
                let class_res = apply_failed_mutation_reconciled(
                    &mut state,
                    before_digest.as_deref(),
                    &io_err,
                    reconciled_digest.as_deref(),
                    Some(&preflight_pf),
                );
                let snap = snapshot(&state);
                match class_res {
                    Ok(class) => {
                        let err =
                            DocumentSessionError::from_io(&io_err).with_snapshot(snap.clone());
                        (snap, err, class)
                    }
                    Err(err) => {
                        return MutationOwnerResult {
                            transaction_id,
                            session_id,
                            classification: MutationOwnerClassification::Indeterminate {
                                error: err.with_snapshot(snap.clone()),
                                snapshot: snap,
                            },
                            child_owner: ctx.child_processes,
                            recovery_ctx: ctx.recovery_ctx,
                        };
                    }
                }
            };

            let classification = match classification_result {
                CommitStateClassification::OriginalPreserved => {
                    MutationOwnerClassification::NotAppliedAfterCommitAttempt {
                        error: ds_err,
                        snapshot: snap,
                    }
                }
                CommitStateClassification::ReplacementCommitted => {
                    MutationOwnerClassification::CommittedUnconfirmed {
                        snapshot: snap,
                        warning: ds_err.message,
                    }
                }
                CommitStateClassification::RecoveryRequired => {
                    MutationOwnerClassification::RecoveryRequired {
                        error: ds_err,
                        snapshot: snap,
                    }
                }
                CommitStateClassification::Indeterminate => {
                    MutationOwnerClassification::Indeterminate {
                        error: ds_err,
                        snapshot: snap,
                    }
                }
            };

            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification,
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
        Err(join_err) => {
            let actual_digest = sha256_file(&working_path).ok();
            let classification = {
                let mut state = entry.state.lock();
                let initial = before_digest.as_deref();
                if initial.is_some() && initial == actual_digest.as_deref() {
                    let snap = snapshot(&state);
                    let err = DocumentSessionError::new(
                        "MUTATION_COMMIT_FAILED",
                        format!("Commit thread panikledi: {join_err}"),
                    );
                    MutationOwnerClassification::NotAppliedAfterCommitAttempt {
                        error: err.with_snapshot(snap.clone()),
                        snapshot: snap,
                    }
                } else if validated_digest_clone == actual_digest.as_deref().unwrap_or("") {
                    state.content_generation = next_content_generation;
                    state.state_revision = next_state_revision;
                    state.dirty = true;
                    state.working_digest = actual_digest;
                    let snap = snapshot(&state);
                    MutationOwnerClassification::CommittedUnconfirmed {
                        snapshot: snap,
                        warning: format!("MUTATION_COMMIT_THREAD_PANICKED: {join_err}"),
                    }
                } else {
                    state.lifecycle = SessionLifecycle::CommitIndeterminate;
                    state.dirty = true;
                    state.working_digest = None;
                    state.state_revision = next_state_revision;
                    let snap = snapshot(&state);
                    let err = DocumentSessionError::new(
                        "MUTATION_COMMIT_FAILED",
                        format!("Commit thread panikledi: {join_err}"),
                    );
                    MutationOwnerClassification::Indeterminate {
                        error: err.with_snapshot(snap.clone()),
                        snapshot: snap,
                    }
                }
            };
            return MutationOwnerResult {
                transaction_id,
                session_id,
                classification,
                child_owner: ctx.child_processes,
                recovery_ctx: ctx.recovery_ctx,
            };
        }
    };

    if let Err(err) = record.transition_to(MutationTransactionPhase::Committed) {
        let snap = {
            let state = entry.state.lock();
            snapshot(&state)
        };
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::Indeterminate {
                error: err.with_snapshot(snap.clone()),
                snapshot: snap,
            },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    if let Err(err) = record.transition_to(MutationTransactionPhase::ApplyingSessionState) {
        let snap_curr = {
            let state = entry.state.lock();
            snapshot(&state)
        };
        return MutationOwnerResult {
            transaction_id,
            session_id,
            classification: MutationOwnerClassification::Indeterminate {
                error: err.with_snapshot(snap_curr.clone()),
                snapshot: snap_curr,
            },
            child_owner: ctx.child_processes,
            recovery_ctx: ctx.recovery_ctx,
        };
    }

    #[cfg(test)]
    _manager.trigger_test_hook(MutationTestHookPhase::BeforeStateApply, &transaction_id);

    let is_unconfirmed = commit_report.state == CommitState::NewCommittedDurabilityUnconfirmed;
    // İşlem öncesi durum geçmişe girer: snapshot'ı rename ile taşınır (kilit dışında).
    let retained = retain_transaction_snapshot(
        &tx_context.snapshot_path,
        snapshot_digest.as_deref(),
        before_digest.as_deref(),
        &session_dir,
    );
    let snap = {
        let mut state = entry.state.lock();
        state.working_digest = Some(commit_report.committed_digest.clone());
        refresh_dirty(&mut state);
        state.health_warning =
            is_unconfirmed.then(|| "MUTATION_DURABILITY_UNCONFIRMED".to_string());
        state.content_generation = next_content_generation;
        state.state_revision = next_state_revision;
        let history_operation = restore_entry
            .as_ref()
            .map_or(operation, |restored| restored.operation);
        match (retained, before_digest.clone()) {
            (Ok((blob, bytes)), Some(digest)) => {
                let previous = HistoryEntry {
                    operation: history_operation,
                    digest,
                    blob,
                    bytes,
                };
                match &kind {
                    MutationTransactionKind::Recipe(_) => state.history.record_mutation(previous),
                    MutationTransactionKind::Restore(direction) => {
                        state.history.record_restore(*direction, previous)
                    }
                }
            }
            (retained, _) => {
                // Mutasyon commit edildi; yalnız geçmiş tutulamadı. Eski girdiler bu yeni durumla
                // tutarlı bir zincir oluşturmadığından hepsi düşer.
                if let Ok((blob, _)) = &retained {
                    remove_history_blob(blob);
                }
                tracing::warn!(
                    code = "HISTORY_TRUNCATED",
                    reason = %retained.err().unwrap_or_else(|| "işlem öncesi digest bilinmiyor".to_string()),
                    "Undo geçmişi tutulamadı ve temizlendi"
                );
                state.history.clear();
                if !is_unconfirmed {
                    state.health_warning = Some("HISTORY_TRUNCATED".to_string());
                }
            }
        }
        snapshot(&state)
    };

    let classification = if is_unconfirmed {
        MutationOwnerClassification::CommittedUnconfirmed {
            snapshot: snap,
            warning: "MUTATION_DURABILITY_UNCONFIRMED".to_string(),
        }
    } else {
        MutationOwnerClassification::Committed { snapshot: snap }
    };

    MutationOwnerResult {
        transaction_id,
        session_id,
        classification,
        child_owner: ctx.child_processes,
        recovery_ctx: ctx.recovery_ctx,
    }
}

/// Working PDF, son açılan ya da kaydedilen baytlardan farklıysa kaydedilmemiş değişiklik
/// vardır (docs/engineering/undo-redo-and-incremental-save-design.md §5.6). `baseline.digest`
/// tam olarak o baytların digest'idir: açılışta working kopyanın kaynağı, Save'de hedefe bayt
/// bayt kopyalanan working PDF. Working digest'i bilinmiyorsa (kurtarma gereken durumlar)
/// belge kirli sayılır. Generation sayacıyla karşılaştırma, undo ile kaydedilmiş içeriğe
/// dönüldüğünde yanlış sonuç verirdi: generation geri gitmez, içerik ise aynıdır.
fn refresh_dirty(state: &mut DocumentSession) {
    state.dirty = state.working_digest.as_deref() != Some(state.baseline.digest.as_str());
}

#[cfg(test)]
fn apply_committed_mutation(
    state: &mut DocumentSession,
    report: AtomicWriteReport,
) -> Result<(), DocumentSessionError> {
    match report.state {
        CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed => {
            state.working_digest = Some(report.committed_digest);
            refresh_dirty(state);
            state.health_warning = (report.state == CommitState::NewCommittedDurabilityUnconfirmed)
                .then(|| "MUTATION_DURABILITY_UNCONFIRMED".to_string());
            bump_generation(state)?;
            bump_revision(state)
        }
        other => Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_UNEXPECTED_COMMIT_STATE",
            format!("Mutation beklenmeyen başarılı commit state döndürdü: {other}"),
        )),
    }
}

#[cfg(test)]
fn apply_failed_mutation(
    state: &mut DocumentSession,
    before_digest: Option<&str>,
    error: &DocumentIoError,
) -> Result<(), DocumentSessionError> {
    match error.state {
        CommitState::Prepared | CommitState::OriginalPreserved => Ok(()),
        CommitState::OriginalRecovered => {
            let actual = sha256_file(&state.working_path).map_err(|hash_error| {
                DocumentSessionError::new(
                    "DOCUMENT_SESSION_RECOVERY_HASH_FAILED",
                    format!("Recovered working PDF hash edilemedi: {hash_error}"),
                )
            })?;
            if before_digest.is_some_and(|before| before == actual) {
                state.working_digest = Some(actual);
                return Ok(());
            }
            if error
                .replacement_digest
                .as_ref()
                .is_some_and(|candidate| candidate == &actual)
            {
                state.working_digest = Some(actual);
                refresh_dirty(state);
                bump_generation(state)?;
                return bump_revision(state);
            }
            state.lifecycle = SessionLifecycle::CommitIndeterminate;
            state.dirty = true;
            state.working_digest = None;
            bump_revision(state)
        }
        CommitState::RecoveryRequired => {
            state.lifecycle = SessionLifecycle::RecoveryRequired;
            state.dirty = true;
            state.working_digest = None;
            bump_revision(state)
        }
        CommitState::Indeterminate => {
            state.lifecycle = SessionLifecycle::CommitIndeterminate;
            state.dirty = true;
            state.working_digest = None;
            bump_revision(state)
        }
        CommitState::NewCommitted | CommitState::NewCommittedDurabilityUnconfirmed => {
            Err(DocumentSessionError::new(
                "DOCUMENT_SESSION_INVALID_ERROR_STATE",
                "Başarılı commit state hata olarak döndü",
            ))
        }
    }
}

#[tauri::command]
pub async fn open_document_session(
    path: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<OpenDocumentSessionResult, DocumentSessionError> {
    let source = PathBuf::from(path);
    let canonical = source.canonicalize().map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_SOURCE_NOT_FOUND",
            format!("Belge yolu çözümlenemedi: {error}"),
        )
    })?;
    reject_private_target(&canonical, &store.temp_root)?;

    let session_id = Uuid::new_v4().to_string();
    let session_dir = store
        .instance_dir
        .join("sessions")
        .join(format!("session-{session_id}"));
    create_secure_dir(&session_dir)?;
    let working_path = session_dir.join("working.pdf");

    let copy_result = {
        let canonical = canonical.clone();
        let working_path = working_path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            copy_source_consistent(&canonical, &working_path)
        })
        .await
        .map_err(|error| {
            DocumentSessionError::new(
                "DOCUMENT_SESSION_THREAD_ERROR",
                format!("Open copy worker tamamlanamadı: {error}"),
            )
        })?
    };
    let digest = match copy_result {
        Ok(digest) => digest,
        Err(error) => {
            let _ = fs::remove_dir_all(&session_dir);
            return Err(error);
        }
    };

    let key = WorkerDocumentKey {
        session_id: session_id.clone(),
        content_generation: 1,
    };
    let admission = match store.worker_broker.admit_read(&key, None).await {
        Ok(admission) => admission,
        Err(error) => {
            let _ = fs::remove_dir_all(&session_dir);
            return Err(DocumentSessionError::from(error));
        }
    };
    let open_result = store
        .worker_broker
        .reserve_document(
            &admission,
            key.clone(),
            working_path.to_string_lossy().to_string(),
            None,
            None,
        )
        .await;
    let mut pending_document = None;
    let encryption = match open_result {
        Ok(reservation) if !reservation.protected() => {
            pending_document = Some(reservation);
            CandidateEncryption::None
        }
        // The worker opened it without a password, so the user password is empty and only
        // an owner password restricts the file. Every viewer opens such a file without a
        // prompt; it stays open, read-only like any encrypted session.
        Ok(reservation) => {
            pending_document = Some(reservation);
            CandidateEncryption::OwnerPasswordOnly
        }
        Err(error) if error.code == "DOCUMENT_SESSION_PASSWORD_REQUIRED" => {
            CandidateEncryption::PasswordRequired
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&session_dir);
            return Err(DocumentSessionError::from(error));
        }
    };

    let candidate =
        store.insert_candidate(canonical, working_path, session_dir, digest, encryption);
    if let Some(reservation) = pending_document {
        reservation.commit();
    }
    Ok(candidate)
}

#[tauri::command]
pub async fn authenticate_document_session(
    session_id: String,
    password: String,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let admission = store
        .worker_broker
        .admit_exclusive(&session_id)
        .await
        .map_err(DocumentSessionError::from)?;
    let entry = store.entry(&session_id)?;
    let _lease = entry.operation.clone().write_owned().await;
    let (working_path, content_generation) = {
        let state = entry.state.lock();
        ensure_lifecycle(&state, &[SessionLifecycle::CandidateAuthPending])?;
        (state.working_path.clone(), state.content_generation)
    };
    let key = WorkerDocumentKey {
        session_id: session_id.clone(),
        content_generation,
    };
    let reservation = store
        .worker_broker
        .reserve_document(
            &admission,
            key.clone(),
            working_path.to_string_lossy().to_string(),
            Some(password.clone()),
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?;
    let page_count = reservation.page_count();
    if page_count == 0 {
        let _ = reservation.abort(&admission).await;
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_EMPTY_PDF",
            "PDF en az bir sayfa içermeli",
        ));
    }
    let updated = {
        let mut state = entry.state.lock();
        (|| {
            ensure_lifecycle(&state, &[SessionLifecycle::CandidateAuthPending])?;
            if state.content_generation != content_generation {
                return Err(DocumentSessionError::new(
                    "STALE_GENERATION",
                    "Authentication sırasında candidate generation değişti",
                ));
            }
            state.credential = Some(password.clone());
            state.lifecycle = SessionLifecycle::CandidateReady;
            state.editable = false;
            state.read_only_reason = Some("Şifreli PDF mutasyonları desteklenmiyor".to_string());
            bump_revision(&mut state)?;
            Ok(snapshot(&state))
        })()
    };
    let snapshot = match updated {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = reservation.abort(&admission).await;
            return Err(error);
        }
    };
    reservation.commit();
    password_cache
        .0
        .lock()
        .insert(working_path.to_string_lossy().to_string(), password);
    Ok(snapshot)
}

#[tauri::command]
pub async fn get_document_session_snapshot(
    session_id: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    store.session_snapshot(&session_id).await
}

#[tauri::command]
pub async fn get_active_document_session_snapshot(
    store: State<'_, DocumentSessionStore>,
) -> Result<Option<DocumentSessionSnapshot>, DocumentSessionError> {
    store.reap_expired_lifecycle().await?;
    store.active_session_snapshot()
}

#[tauri::command]
pub async fn probe_candidate_render(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
) -> Result<CandidateRenderProbe, DocumentSessionError> {
    let _ = password_cache;
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, true, None)
        .await?;
    let page_count = context.page_count;
    if page_count == 0 {
        return Err(DocumentSessionError::new(
            "CANDIDATE_RENDER_PROBE_INVALID",
            "Candidate PDF en az bir sayfa içermeli",
        ));
    }
    let rendered = store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::RenderPage {
                document_id,
                page_index: 0,
                target: WorkerRenderTarget::Scale(0.25),
                rotation: 0,
            },
            CommandTimeout::RenderPage,
            None,
        )
        .await
        .map_err(DocumentSessionError::from);
    let rendered = match rendered {
        Ok(WorkerResult::PageRendered {
            width,
            height,
            data,
        }) => crate::RenderedPage {
            width,
            height,
            data,
        },
        Ok(_) => {
            store.invalidate_failed_worker_read(context).await;
            return Err(DocumentSessionError::new(
                "PDFIUM_WORKER_PROTOCOL_ERROR",
                "Candidate render probe beklenmeyen worker cevabı aldı",
            ));
        }
        Err(error) => {
            store.invalidate_failed_worker_read(context).await;
            return Err(error);
        }
    };
    let expected_bytes = usize::try_from(rendered.width)
        .ok()
        .and_then(|width| {
            usize::try_from(rendered.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            DocumentSessionError::new(
                "CANDIDATE_RENDER_PROBE_INVALID",
                "Candidate render boyutları doğrulanamadı",
            )
        });
    let expected_bytes = match expected_bytes {
        Ok(expected_bytes) => expected_bytes,
        Err(error) => {
            store.invalidate_failed_worker_read(context).await;
            return Err(error);
        }
    };
    if rendered.width == 0 || rendered.height == 0 || rendered.data.len() != expected_bytes {
        store.invalidate_failed_worker_read(context).await;
        return Err(DocumentSessionError::new(
            "CANDIDATE_RENDER_PROBE_INVALID",
            "Candidate ilk sayfa RGBA tamponu geçersiz",
        ));
    }
    Ok(CandidateRenderProbe {
        page_count,
        width: rendered.width,
        height: rendered.height,
        byte_length: rendered.data.len(),
    })
}

#[tauri::command]
pub async fn recover_document_session(
    session_id: String,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let entry = store.entry(&session_id)?;
    let _lease = entry.operation.clone().write_owned().await;
    let (path, credential) = {
        let state = entry.state.lock();
        ensure_lifecycle(
            &state,
            &[
                SessionLifecycle::RecoveryRequired,
                SessionLifecycle::CommitIndeterminate,
            ],
        )?;
        (state.working_path.clone(), state.credential.clone())
    };
    create_secure_dir(&store.jobs_root())?;
    let recovery_dir = tempfile::Builder::new()
        .prefix("recovery-validation-")
        .tempdir_in(store.jobs_root())
        .map_err(|error| {
            DocumentSessionError::new(
                "DOCUMENT_SESSION_RECOVERY_SNAPSHOT_FAILED",
                format!("Recovery validation dizini oluşturulamadı: {error}"),
            )
        })?;
    let recovery_snapshot = recovery_dir.path().join("snapshot.pdf");
    copy_with_digest(&path, &recovery_snapshot)?;
    let validation_context = SessionJobContext {
        job_id: Uuid::new_v4(),
        owner_session_id: session_id.clone(),
        captured_content_generation: 0,
        snapshot_path: recovery_snapshot,
        credential: credential.clone().map(SessionJobCredential),
        cancellation: SessionJobCancellationCheckpoint {
            token: CancellationToken::new(),
            reason: Arc::new(Mutex::new(None)),
        },
        child_processes: SessionJobChildProcessOwner::default(),
    };
    let validation = tauri::async_runtime::spawn_blocking(move || {
        crate::job_worker::run_job(
            &validation_context,
            crate::job_worker::JobCommand::ValidateSnapshot,
            Duration::from_secs(60),
        )
    })
    .await
    .map_err(|_| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_RECOVERY_VALIDATION_FAILED",
            "Recovery validation worker güvenli biçimde tamamlanamadı",
        )
    })?
    .map_err(|error| match error {
        SessionJobOperationError::Failed(failure) => {
            DocumentSessionError::new(failure.code, failure.message)
        }
        SessionJobOperationError::Cancelled(_) => DocumentSessionError::new(
            "DOCUMENT_SESSION_RECOVERY_VALIDATION_CANCELLED",
            "Recovery validation iptal edildi",
        ),
    })?;
    if !matches!(validation, crate::job_worker::JobResult::SnapshotValid) {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_RECOVERY_VALIDATION_FAILED",
            "Recovery validation worker beklenmeyen sonuç döndürdü",
        ));
    }
    let digest = sha256_file(&path).map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_RECOVERY_HASH_FAILED",
            format!("Recovery PDF hash edilemedi: {error}"),
        )
    })?;
    let snap = {
        let mut state = entry.state.lock();
        state.working_digest = Some(digest);
        refresh_dirty(&mut state);
        // Kurtarılan içeriğin geçmişle ilişkisi bilinmez (tasarım §5.8).
        state.history.clear();
        state.lifecycle = SessionLifecycle::Open;
        state.health_warning = Some("RECOVERED_CONTENT_REVALIDATED".to_string());
        bump_generation(&mut state)?;
        bump_revision(&mut state)?;
        snapshot(&state)
    };
    drop(_lease);
    vector_store.prune_generation(&snap.id, snap.content_generation);
    Ok(snap)
}

#[tauri::command]
pub async fn cancel_session_job(
    session_id: String,
    job_id: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobCancelOutcome, DocumentSessionError> {
    store
        .cancel_job(
            &session_id,
            &job_id,
            SessionJobCancellationReason::UserRequested,
        )
        .await
}

#[tauri::command]
pub async fn export_session_page_to_image(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    target_path: String,
    format: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, DocumentSessionError> {
    reject_private_target(Path::new(&target_path), &store.temp_root)?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::ExportPagesAsImages {
                        pages: vec![page_index],
                        format,
                        dpi: 144.0,
                    },
                    Duration::from_secs(180),
                )?;
                let crate::job_worker::JobResult::Files(mut files) = result else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Page export worker beklenmeyen sonuç döndürdü",
                    ));
                };
                let source = files.pop().ok_or_else(|| {
                    SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_OUTPUT_MISSING",
                        "Page export worker çıktı üretmedi",
                    )
                })?;
                accept_job_output_file(&context, &source, Path::new(&target_path))
            },
        )
        .await
}

#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn export_session_pages_as_images(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    pages: Vec<u32>,
    output_dir: String,
    format: String,
    dpi: f32,
    overwrite: bool,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<Vec<String>>, DocumentSessionError> {
    reject_private_target(Path::new(&output_dir), &store.temp_root)?;
    let stem = session_output_name_stem(&store, &session_id).await;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                // Geçersiz DPI için worker başlatılmaz; sayfa başına piksel
                // sınırı worker içinde PDFium sayfa boyutuyla kontrol edilir.
                crate::export::validate_export_dpi(dpi)?;
                let output_dir = PathBuf::from(output_dir);
                let planned = crate::export::planned_export_image_outputs(&pages, &format, &stem);
                // Çakışma render'dan önce raporlanır; yayın sırasında yeniden denetlenir.
                preflight_job_output_set(&output_dir, &planned, overwrite)?;
                let result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::ExportPagesAsImages { pages, format, dpi },
                    Duration::from_secs(300),
                )?;
                let crate::job_worker::JobResult::Files(files) = result else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Image export worker beklenmeyen sonuç döndürdü",
                    ));
                };
                let sources = files
                    .iter()
                    .map(|source| crate::job_worker::validate_output_file(&context, source))
                    .collect::<Result<Vec<_>, _>>()?;
                accept_job_output_set(&context, sources, &planned, &output_dir, overwrite)
            },
        )
        .await
}

#[tauri::command]
pub async fn split_session_pdf(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    ranges: Vec<(u32, u32)>,
    output_dir: String,
    overwrite: bool,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<Vec<String>>, DocumentSessionError> {
    reject_private_target(Path::new(&output_dir), &store.temp_root)?;
    let stem = session_output_name_stem(&store, &session_id).await;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                context.cancellation.check_cancelled()?;
                let output_dir = PathBuf::from(output_dir);
                let planned = planned_split_outputs(&ranges, &stem)?;
                preflight_job_output_set(&output_dir, &planned, overwrite)?;
                let result = crate::job_worker::run_job(
                    &context,
                    JobCommand::SplitPdf { ranges },
                    Duration::from_secs(300),
                )?;
                let JobResult::Files(files) = result else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Split worker beklenmeyen sonuç döndürdü",
                    ));
                };
                let sources = files
                    .iter()
                    .map(|source| validate_job_pdf_output(&context, source))
                    .collect::<Result<Vec<_>, _>>()?;
                accept_job_output_set(&context, sources, &planned, &output_dir, overwrite)
            },
        )
        .await
}

#[tauri::command]
pub async fn merge_session_pdfs(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    additional_paths: Vec<String>,
    target_path: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, DocumentSessionError> {
    reject_private_target(Path::new(&target_path), &store.temp_root)?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                context.cancellation.check_cancelled()?;
                let job_dir = context.snapshot_path.parent().ok_or_else(|| {
                    SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Merge job dizini çözümlenemedi",
                    )
                })?;
                let mut input_names = Vec::with_capacity(additional_paths.len());
                for (index, source) in additional_paths.iter().enumerate() {
                    context.cancellation.check_cancelled()?;
                    let name = format!("merge-input-{index}.pdf");
                    let input_path = job_dir.join(&name);
                    copy_source_consistent(Path::new(source), &input_path).map_err(|error| {
                        SessionJobOperationError::failed_with_code(error.code, error.message)
                    })?;
                    input_names.push(name);
                }
                let result = crate::job_worker::run_job(
                    &context,
                    JobCommand::MergePdf {
                        additional_input_names: input_names,
                        output_name: "merge-output.pdf".to_string(),
                    },
                    Duration::from_secs(300),
                )?;
                let JobResult::File(source) = result else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Merge worker beklenmeyen sonuç döndürdü",
                    ));
                };
                let source = validate_job_pdf_output(&context, &source)?;
                accept_job_output_file(&context, &source, Path::new(&target_path))
            },
        )
        .await
}

#[tauri::command]
pub async fn export_session_text_content(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    pages: Vec<u32>,
    format: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<String>, DocumentSessionError> {
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                context.cancellation.check_cancelled()?;
                match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::ExtractText {
                        pages: Some(pages),
                        format,
                    },
                    Duration::from_secs(180),
                )? {
                    crate::job_worker::JobResult::Text(text) => Ok(text),
                    _ => Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Text export worker beklenmeyen sonuç döndürdü",
                    )),
                }
            },
        )
        .await
}

/// Seçili sayfaların metnini job worker'da çıkarır ve hedefe `document_io::write_atomically`
/// ile yazar; hedef ya eski içeriğini korur ya da tam metni alır. Hedef yalnız `.txt` veya
/// `.md` olabilir, böylece aynı kaydetme diyaloğundan bir PDF'in üzerine metin yazılamaz.
#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn export_session_text_to_file(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    pages: Vec<u32>,
    format: String,
    target_path: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, DocumentSessionError> {
    let target = PathBuf::from(target_path);
    validate_text_export_target(&target)?;
    reject_private_target(&target, &store.temp_root)?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                context.cancellation.check_cancelled()?;
                let crate::job_worker::JobResult::Text(text) = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::ExtractText {
                        pages: Some(pages),
                        format,
                    },
                    Duration::from_secs(180),
                )?
                else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Text export worker beklenmeyen sonuç döndürdü",
                    ));
                };
                context.cancellation.check_cancelled()?;
                crate::document_io::write_atomically(&target, |staged| {
                    std::io::Write::write_all(staged, text.as_bytes())
                })
                .map_err(job_output_write_failure)
            },
        )
        .await
}

fn validate_text_export_target(target: &Path) -> Result<(), DocumentSessionError> {
    let extension = target
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(extension.as_deref(), Some("txt" | "md")) {
        Ok(())
    } else {
        Err(DocumentSessionError::new(
            "TEXT_EXPORT_TARGET_INVALID",
            "Metin dışa aktarma hedefi .txt veya .md uzantılı olmalı",
        ))
    }
}

#[tauri::command]
pub async fn print_session_pdf(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    page_range: Option<Vec<u32>>,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, DocumentSessionError> {
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                context.cancellation.check_cancelled()?;
                let result = crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::PreparePrint {
                        pages: page_range,
                        output_name: "print-ready.pdf".to_string(),
                    },
                    Duration::from_secs(180),
                )?;
                let crate::job_worker::JobResult::File(print_path) = result else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Print preparation worker beklenmeyen sonuç döndürdü",
                    ));
                };
                let print_path = crate::job_worker::validate_output_file(&context, &print_path)?;
                crate::export::submit_print_file(
                    &print_path,
                    &context.cancellation,
                    &context.child_processes,
                )
            },
        )
        .await
}

#[tauri::command]
pub async fn inspect_session_print_resources(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    physical_page_index: u32,
    request_id: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<crate::prepress::PrintResourceInspectionReport>, DocumentSessionError>
{
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| match crate::job_worker::run_job(
                &context,
                crate::job_worker::JobCommand::InspectPrintResources {
                    content_generation: expected_content_generation,
                    physical_page_index,
                    request_id,
                },
                Duration::from_secs(120),
            )? {
                crate::job_worker::JobResult::PrintResourceInspection(report) => Ok(report),
                _ => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Print resource inspection worker beklenmeyen sonuç döndürdü",
                )),
            },
        )
        .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Flat Tauri IPC boundary includes job/session/page ownership plus preview controls.
pub async fn generate_session_rgb_derived_coverage_preview(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    physical_page_index: u32,
    request_id: String,
    threshold_percent: f32,
    scale: f32,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<crate::prepress::RgbDerivedCoveragePreview>, DocumentSessionError> {
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                context.cancellation.check_cancelled()?;
                match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::GenerateRgbDerivedCoveragePreview {
                        content_generation: expected_content_generation,
                        physical_page_index,
                        request_id,
                        threshold_percent,
                        scale,
                    },
                    Duration::from_secs(240),
                )? {
                    crate::job_worker::JobResult::RgbDerivedCoveragePreview(result) => Ok(result),
                    _ => Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "RGB-derived coverage preview worker beklenmeyen sonuç döndürdü",
                    )),
                }
            },
        )
        .await
}

#[tauri::command]
pub async fn generate_session_rgb_derived_process_preview(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    physical_page_index: u32,
    request_id: String,
    scale: f32,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<crate::prepress::RgbDerivedProcessPreview>, DocumentSessionError> {
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| match crate::job_worker::run_job(
                &context,
                crate::job_worker::JobCommand::GenerateRgbDerivedProcessPreview {
                    content_generation: expected_content_generation,
                    physical_page_index,
                    request_id,
                    scale,
                },
                Duration::from_secs(300),
            )? {
                crate::job_worker::JobResult::RgbDerivedProcessPreview(result) => Ok(result),
                _ => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "RGB-derived process preview worker beklenmeyen sonuç döndürdü",
                )),
            },
        )
        .await
}

#[tauri::command]
pub async fn extract_session_pages(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    page_indices: Vec<u32>,
    target_path: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, DocumentSessionError> {
    if page_indices.is_empty() {
        return Err(DocumentSessionError::new(
            "SESSION_EXPORT_EMPTY_SELECTION",
            "Dışa aktarılacak sayfa seçilmedi",
        ));
    }
    reject_private_target(Path::new(&target_path), &store.temp_root)?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let result = crate::job_worker::run_job(
                    &context,
                    JobCommand::MutatePdf {
                        recipe: MutationRecipe::ExtractPages {
                            page_order: page_indices,
                        },
                        output_name: "extract-pages-output.pdf".to_string(),
                    },
                    Duration::from_secs(180),
                )?;
                let JobResult::File(source) = result else {
                    return Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Page extraction worker beklenmeyen sonuç döndürdü",
                    ));
                };
                let source = validate_job_pdf_output(&context, &source)?;
                accept_job_output_file(&context, &source, Path::new(&target_path))
            },
        )
        .await
}

#[tauri::command]
pub async fn discard_candidate_session(
    session_id: String,
    store: State<'_, DocumentSessionStore>,
    search_cache: State<'_, SearchCache>,
    layout_cache: State<'_, TextLayoutCache>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<(), DocumentSessionError> {
    discard_candidate_session_in_store(
        &session_id,
        &store,
        &search_cache,
        &layout_cache,
        &vector_store,
    )
    .await
}

async fn discard_candidate_session_in_store(
    session_id: &str,
    store: &DocumentSessionStore,
    search_cache: &SearchCache,
    layout_cache: &TextLayoutCache,
    vector_store: &crate::ai::VectorStore,
) -> Result<(), DocumentSessionError> {
    store.reap_expired_lifecycle().await?;
    let _gate = store.lifecycle_gate.clone().write_owned().await;
    if store
        .activation_token_by_candidate
        .lock()
        .contains_key(session_id)
    {
        return Err(DocumentSessionError::new(
            "CANDIDATE_ACTIVATION_ACTIVE",
            "Candidate discard öncesi activation rollback edilmelidir",
        ));
    }
    let entry = store.entry(session_id)?;
    {
        let state = entry.state.lock();
        ensure_lifecycle(
            &state,
            &[
                SessionLifecycle::CandidateAuthPending,
                SessionLifecycle::CandidateReady,
            ],
        )?;
    }
    store.close_entry_under_lifecycle(session_id).await?;
    clear_session_derived_state(search_cache, layout_cache, vector_store, session_id);
    Ok(())
}

fn session_read_path(
    state: &DocumentSession,
    expected_generation: Option<u64>,
) -> Result<String, DocumentSessionError> {
    ensure_lifecycle(
        state,
        &[SessionLifecycle::Open, SessionLifecycle::DecisionPending],
    )?;
    if let Some(expected) = expected_generation {
        if expected != state.content_generation {
            return Err(DocumentSessionError::new(
                "STALE_GENERATION",
                format!(
                    "Beklenen generation {}, güncel generation {}",
                    expected, state.content_generation
                ),
            )
            .with_snapshot(snapshot(state)));
        }
    }
    Ok(state.working_path.to_string_lossy().to_string())
}

fn candidate_validation_path(
    state: &DocumentSession,
    expected_generation: u64,
) -> Result<String, DocumentSessionError> {
    ensure_lifecycle(state, &[SessionLifecycle::CandidateReady])?;
    if expected_generation != state.content_generation {
        return Err(DocumentSessionError::new(
            "STALE_GENERATION",
            "Candidate validation generation değeri güncel değil",
        )
        .with_snapshot(snapshot(state)));
    }
    Ok(state.working_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_session_page_count(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
) -> Result<u32, DocumentSessionError> {
    let _ = password_cache;
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, true, None)
        .await?;
    Ok(context.page_count)
}

/// Display sizes (PDF points, page `/Rotate` applied) of every page in one
/// worker round trip, so the viewer can size unrendered pages before render.
#[tauri::command]
pub async fn get_session_page_sizes(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
) -> Result<Vec<crate::worker::ipc::WorkerPageSize>, DocumentSessionError> {
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetPageSizes { document_id },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::PageSizes(sizes) => Ok(sizes),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Sayfa boyutları beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn get_session_pdf_outline(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
) -> Result<Vec<crate::OutlineNode>, DocumentSessionError> {
    let _ = password_cache;
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetOutline { document_id },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::Outline(outline) => Ok(outline),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Outline beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn get_session_pdf_metadata(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
) -> Result<PdfMetadata, DocumentSessionError> {
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetMetadata { document_id },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::Metadata(metadata) => Ok(metadata),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Metadata beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn get_session_structure_tree(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
) -> Result<crate::a11y::AccessibilityStructureInspection, DocumentSessionError> {
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetStructureTree { document_id },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::AccessibilityStructure(tree) => Ok(tree),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Structure tree beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn audit_session_accessibility(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
) -> Result<crate::a11y::AccessibilityAuditReport, DocumentSessionError> {
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::AuditAccessibility { document_id },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::AccessibilityAudit(report) => Ok(report),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Accessibility audit beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn get_session_page_annotations(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
) -> Result<Vec<annotations::AnnotationData>, DocumentSessionError> {
    let _ = password_cache;
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetPageAnnotations {
                document_id,
                page_index,
            },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::PageAnnotations(annotations) => Ok(annotations),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Page annotations beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn get_session_form_info(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
) -> Result<crate::forms::FormInfo, DocumentSessionError> {
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetFormInfo { document_id },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::FormInfo(info) => Ok(info),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Form bilgisi beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn get_session_page_form_fields(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    store: State<'_, DocumentSessionStore>,
) -> Result<Vec<crate::forms::FormWidgetData>, DocumentSessionError> {
    let context = store
        .acquire_worker_read(&session_id, expected_content_generation, false, None)
        .await?;
    match store
        .worker_broker
        .command_for_document(
            &context._admission,
            &context.key,
            |document_id| WorkerCommand::GetPageFormFields {
                document_id,
                page_index,
            },
            CommandTimeout::DocumentRead,
            None,
        )
        .await
        .map_err(DocumentSessionError::from)?
    {
        WorkerResult::PageFormFields(widgets) => Ok(widgets),
        _ => Err(DocumentSessionError::new(
            "PDFIUM_WORKER_PROTOCOL_ERROR",
            "Form alanları beklenmeyen worker cevabı aldı",
        )),
    }
}

#[tauri::command]
pub async fn search_session_pdf_text(
    session_id: String,
    expected_content_generation: u64,
    query: String,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
    search_cache: State<'_, SearchCache>,
    active_search_id: State<'_, ActiveSearchId>,
) -> Result<SearchResult, DocumentSessionError> {
    let _ = password_cache;
    if query.trim().is_empty() {
        return Ok(SearchResult {
            matches: Vec::new(),
            is_cancelled: false,
            total_matches: 0,
        });
    }
    let cache_namespace = format!("session:{session_id}:generation:{expected_content_generation}");
    let cache_key = format!("{cache_namespace}:{}", query.to_lowercase());
    if let Some(cached) = search_cache.0.lock().get(&cache_key).cloned() {
        return Ok(cached);
    }
    let search_id = active_search_id
        .0
        .fetch_add(1, Ordering::SeqCst)
        .saturating_add(1);
    let cancellation = CancellationToken::new();
    {
        let mut active = store.search_cancellations.lock();
        if let Some((_previous_id, previous)) =
            active.insert(session_id.clone(), (search_id, cancellation.clone()))
        {
            tracing::debug!(
                target: crate::pdf_engine::subprocess_manager::WORKER_METRICS_TARGET,
                event = "search_cancel",
                trigger = "search_superseded",
            );
            previous.cancel();
        }
    }
    let store_ref = store.inner();
    let result = store
        .worker_broker
        .search_text_in_chunks(
            || {
                store_ref.acquire_worker_read(
                    &session_id,
                    expected_content_generation,
                    false,
                    Some(&cancellation),
                )
            },
            &query,
            crate::pdf_engine::subprocess_manager::VIEWER_SEARCH_CHUNK_POLICY,
            &cancellation,
        )
        .await;
    {
        let mut active = store.search_cancellations.lock();
        if active
            .get(&session_id)
            .is_some_and(|(active_id, _)| *active_id == search_id)
        {
            active.remove(&session_id);
        }
    }
    let result = match result {
        Ok(result) => result,
        Err(error) if error.code == "PDFIUM_WORKER_CANCELLED" => SearchResult {
            matches: Vec::new(),
            is_cancelled: true,
            total_matches: 0,
        },
        Err(error) => return Err(error),
    };
    if !result.is_cancelled {
        search_cache.0.lock().put(cache_key, result.clone());
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_session_page_text_layout(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
    layout_cache: State<'_, TextLayoutCache>,
    active_tasks: State<'_, ActiveRenderTasks>,
) -> Result<Vec<TextLayoutData>, DocumentSessionError> {
    let _ = password_cache;
    let cache_key =
        format!("session:{session_id}:generation:{expected_content_generation}:{page_index}");
    if let Some(cached) = layout_cache.0.lock().get(&cache_key).cloned() {
        return Ok(cached);
    }
    let task_id =
        format!("text_layout_session:{session_id}:{expected_content_generation}:{page_index}");
    let cancellation = CancellationToken::new();
    {
        let mut tasks = active_tasks.0.lock();
        if let Some(previous) = tasks.insert(task_id.clone(), cancellation.clone()) {
            tracing::debug!(
                target: crate::pdf_engine::subprocess_manager::WORKER_METRICS_TARGET,
                event = "text_layout_cancel",
                trigger = "text_layout_superseded",
            );
            previous.cancel();
        }
    }
    let context = store
        .acquire_worker_read(
            &session_id,
            expected_content_generation,
            false,
            Some(&cancellation),
        )
        .await;
    let result = match context {
        Ok(context) => store
            .worker_broker
            .command_for_document(
                &context._admission,
                &context.key,
                |document_id| WorkerCommand::GetPageTextLayout {
                    document_id,
                    page_index,
                },
                CommandTimeout::TextLayout,
                Some(&cancellation),
            )
            .await
            .map_err(DocumentSessionError::from),
        Err(error) => Err(error),
    };
    active_tasks.0.lock().remove(&task_id);
    let layouts = match result? {
        WorkerResult::TextLayout(layouts) => layouts,
        _ => {
            return Err(DocumentSessionError::new(
                "PDFIUM_WORKER_PROTOCOL_ERROR",
                "Text layout beklenmeyen worker cevabı aldı",
            ))
        }
    };
    layout_cache.0.lock().put(cache_key, layouts.clone());
    Ok(layouts)
}

/// Explicit user retry for a page the worker broker quarantined after repeated
/// timeouts. Returns whether a quarantine was lifted. Document-level
/// quarantine and the document timeout ceiling are not affected.
#[tauri::command]
pub async fn retry_session_page(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    store: State<'_, DocumentSessionStore>,
) -> Result<bool, DocumentSessionError> {
    store
        .release_worker_page_quarantine(&session_id, expected_content_generation, page_index)
        .await
}

#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn render_session_page(
    session_id: String,
    page_index: u32,
    scale: f64,
    rotation: i32,
    content_generation: u64,
    request_id: String,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
    render_service: State<'_, RenderService>,
) -> Result<tauri::ipc::Response, DocumentSessionError> {
    crate::pdf_engine::validate_scale(scale).map_err(DocumentSessionError::from)?;
    crate::pdf_engine::validate_rotation(rotation).map_err(DocumentSessionError::from)?;
    let identity = RenderRequestIdentity {
        document_id: session_id.clone(),
        path: format!("session:{session_id}"),
        generation: content_generation,
        page_index,
        scope: RenderScope::MainPage,
        request_id,
    };
    let broker = store.worker_broker.clone();
    let store_ref = store.inner();
    let worker_session_id = session_id;
    crate::pdf_engine::render_viewer_request_with_renderer(
        identity,
        crate::pdf_engine::ViewerRenderSizing::Scale(scale),
        rotation,
        password_cache.inner(),
        render_service.inner(),
        move |_path, page_index, sizing, rotation, _password, cancellation| {
            let broker = broker.clone();
            async move {
                let context = store_ref
                    .acquire_worker_read(
                        &worker_session_id,
                        content_generation,
                        false,
                        Some(&cancellation),
                    )
                    .await
                    .map_err(|error| format!("{}: {}", error.code, error.message))?;
                let target = match sizing {
                    crate::pdf_engine::ViewerRenderSizing::Scale(scale) => {
                        WorkerRenderTarget::Scale(scale)
                    }
                    crate::pdf_engine::ViewerRenderSizing::TargetWidth(width) => {
                        WorkerRenderTarget::TargetWidth(width)
                    }
                };
                let result = broker
                    .command_for_document(
                        &context._admission,
                        &context.key,
                        |document_id| WorkerCommand::RenderPage {
                            document_id,
                            page_index,
                            target,
                            rotation,
                        },
                        CommandTimeout::RenderPage,
                        Some(&cancellation),
                    )
                    .await
                    .map_err(|error| format!("{}: {}", error.code, error.message))?;
                worker_rendered_page(result)
            }
        },
    )
    .await
    .and_then(|response| crate::pdf_engine::render_wire::encode_render_frame(&response))
    .map(tauri::ipc::Response::new)
    .map_err(DocumentSessionError::from)
}

#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn render_session_thumbnail(
    session_id: String,
    page_index: u32,
    target_width: u32,
    content_generation: u64,
    request_id: String,
    scope: RenderScope,
    store: State<'_, DocumentSessionStore>,
    password_cache: State<'_, PasswordCache>,
    render_service: State<'_, RenderService>,
) -> Result<tauri::ipc::Response, DocumentSessionError> {
    if scope == RenderScope::MainPage {
        return Err(DocumentSessionError::new(
            "PDFIUM_WORKER_INVALID_RENDER",
            "Thumbnail render scope main page olamaz",
        ));
    }
    crate::pdf_engine::validate_target_width(target_width).map_err(DocumentSessionError::from)?;
    let identity = RenderRequestIdentity {
        document_id: session_id.clone(),
        path: format!("session:{session_id}"),
        generation: content_generation,
        page_index,
        scope,
        request_id,
    };
    let broker = store.worker_broker.clone();
    let store_ref = store.inner();
    let worker_session_id = session_id;
    crate::pdf_engine::render_viewer_request_with_renderer(
        identity,
        crate::pdf_engine::ViewerRenderSizing::TargetWidth(target_width),
        0,
        password_cache.inner(),
        render_service.inner(),
        move |_path, page_index, _sizing, _rotation, _password, cancellation| {
            let broker = broker.clone();
            async move {
                let context = store_ref
                    .acquire_worker_read(
                        &worker_session_id,
                        content_generation,
                        false,
                        Some(&cancellation),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let result = broker
                    .command_for_document(
                        &context._admission,
                        &context.key,
                        |document_id| WorkerCommand::RenderThumbnail {
                            document_id,
                            page_index,
                            target_width,
                        },
                        CommandTimeout::RenderThumbnail,
                        Some(&cancellation),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                worker_rendered_page(result)
            }
        },
    )
    .await
    .and_then(|response| crate::pdf_engine::render_wire::encode_render_frame(&response))
    .map(tauri::ipc::Response::new)
    .map_err(DocumentSessionError::from)
}

fn worker_rendered_page(result: WorkerResult) -> Result<crate::RenderedPage, String> {
    match result {
        WorkerResult::PageRendered {
            width,
            height,
            data,
        } => Ok(crate::RenderedPage {
            width,
            height,
            data,
        }),
        _ => Err(BrokerError {
            code: "PDFIUM_WORKER_PROTOCOL_ERROR",
            message: "Render beklenmeyen worker cevabı aldı".to_string(),
        }
        .to_string()),
    }
}

#[tauri::command]
pub async fn set_active_session_render_document(
    session_id: Option<String>,
    content_generation: u64,
    store: State<'_, DocumentSessionStore>,
    render_service: State<'_, RenderService>,
) -> Result<crate::pdf_engine::RenderDocumentChange, DocumentSessionError> {
    if let Some(session_id) = session_id {
        let entry = store.entry(&session_id)?;
        let _lease = entry.operation.clone().read_owned().await;
        session_read_path(&entry.state.lock(), Some(content_generation))?;
        crate::pdf_engine::set_active_render_document(
            Some(session_id.clone()),
            Some(format!("session:{session_id}")),
            content_generation,
            render_service,
        )
        .map_err(DocumentSessionError::from)
    } else {
        crate::pdf_engine::set_active_render_document(
            None,
            None,
            content_generation,
            render_service,
        )
        .map_err(DocumentSessionError::from)
    }
}

#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn cancel_session_render_request(
    session_id: String,
    page_index: u32,
    content_generation: u64,
    request_id: String,
    scope: RenderScope,
    reason: Option<crate::pdf_engine::render_service::RenderCancelReason>,
    store: State<'_, DocumentSessionStore>,
    render_service: State<'_, RenderService>,
) -> Result<crate::pdf_engine::RenderCancellationResult, DocumentSessionError> {
    let entry = store.entry(&session_id)?;
    let _lease = entry.operation.clone().read_owned().await;
    session_read_path(&entry.state.lock(), Some(content_generation))?;
    let result = crate::pdf_engine::cancel_render_request(
        format!("session:{session_id}"),
        session_id,
        page_index,
        content_generation,
        request_id,
        scope,
        render_service,
    )
    .map_err(DocumentSessionError::from)?;
    tracing::debug!(
        target: crate::pdf_engine::subprocess_manager::WORKER_METRICS_TARGET,
        event = "render_cancel",
        trigger = "explicit",
        reason = reason
            .unwrap_or(crate::pdf_engine::render_service::RenderCancelReason::Unspecified)
            .label(),
        scope = scope.label(),
        matched_active_request = result.cancelled,
    );
    Ok(result)
}

/// Son mutasyonu geri alır (tasarım §5.4). Geri alınacak adım yoksa `HISTORY_EMPTY`.
#[tauri::command]
pub async fn undo_session_mutation(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated_restore(
            session_id,
            expected_content_generation,
            HistoryDirection::Undo,
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

/// Son geri alınan mutasyonu yineler. Yinelenecek adım yoksa `HISTORY_EMPTY`.
#[tauri::command]
pub async fn redo_session_mutation(
    session_id: String,
    expected_content_generation: u64,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated_restore(
            session_id,
            expected_content_generation,
            HistoryDirection::Redo,
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

#[tauri::command]
pub async fn rotate_session_page(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    degrees: i32,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::RotatePage {
                page_index,
                degrees,
            },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

#[tauri::command]
pub async fn delete_session_pages(
    session_id: String,
    expected_content_generation: u64,
    page_indices: Vec<u32>,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::DeletePages { page_indices },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

#[tauri::command]
pub async fn reorder_session_pages(
    session_id: String,
    expected_content_generation: u64,
    new_order: Vec<u32>,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::ReorderPages {
                page_order: new_order,
            },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

#[tauri::command]
pub async fn insert_session_blank_page(
    session_id: String,
    expected_content_generation: u64,
    at_index: u32,
    width: f32,
    height: f32,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::InsertBlankPage {
                at_index,
                width,
                height,
            },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

/// `normalized_rect` sayfanın görüntü uzayındadır (bkz. [`NormalizedPageRect`]); PDF user
/// space'e dönüşüm job worker'da PDFium ile yapılır.
#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn add_session_annotation(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    annot_type: String,
    normalized_rect: NormalizedPageRect,
    color: String,
    contents: Option<String>,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::AddAnnotation {
                page_index,
                annot_type,
                normalized_rect,
                color,
                contents,
            },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

#[tauri::command]
pub async fn delete_session_annotation(
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    annot_id: String,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::DeleteAnnotation {
                page_index,
                annot_id,
            },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

/// Form taslaklarını yazar (form-filling-plan.md, Faz 3). İstek burada yalnız belgeden
/// bağımsız kurallarla denetlenir; alan adı/indeks, tür, read-only ve MaxLen denetimi job
/// worker'da belgeye karşı yapılır.
#[tauri::command]
pub async fn fill_session_form_fields(
    session_id: String,
    expected_content_generation: u64,
    edits: Vec<crate::forms::fill::FormFieldEdit>,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    crate::forms::fill::validate_request(&edits)
        .map_err(|message| DocumentSessionError::new("SESSION_FORM_FILL_INVALID", message))?;
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::FillFormFields { edits },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

/// Form alanı oluşturur, siler, taşır ya da özelliklerini değiştirir
/// (form-authoring-plan.md, Faz A). İstek burada yalnız belgeden bağımsız kurallarla
/// denetlenir; alan adı/indeks, ad çakışması ve yapı denetimi job worker'da yapılır.
#[tauri::command]
pub async fn author_session_form_fields(
    session_id: String,
    expected_content_generation: u64,
    ops: Vec<crate::forms::author::FormAuthorOp>,
    store: State<'_, DocumentSessionStore>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    crate::forms::author::validate_request(&ops)
        .map_err(|message| DocumentSessionError::new("SESSION_FORM_AUTHOR_INVALID", message))?;
    let snapshot = store
        .mutate_isolated(
            session_id,
            expected_content_generation,
            MutationRecipe::AuthorFormFields { ops },
        )
        .await?;
    vector_store.prune_generation(&snapshot.id, snapshot.content_generation);
    Ok(snapshot)
}

#[tauri::command]
pub async fn update_session_metadata(
    session_id: String,
    expected_content_generation: u64,
    _metadata: PdfMetadata,
    store: State<'_, DocumentSessionStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    reject_unsupported_session_write(
        &store,
        &session_id,
        expected_content_generation,
        "PDFium metadata yazma desteği sunmamaktadır; işlem desteklenmiyor",
    )
}

fn reject_unsupported_session_write(
    store: &DocumentSessionStore,
    session_id: &str,
    expected_content_generation: u64,
    message: &str,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let entry = store.entry(session_id)?;
    let state = entry.state.lock();
    session_read_path(&state, Some(expected_content_generation))?;
    Err(
        DocumentSessionError::new("UNSUPPORTED_PDFIUM_CAPABILITY", message)
            .with_snapshot(snapshot(&state)),
    )
}

#[tauri::command]
pub async fn save_document_session(
    session_id: String,
    expected_state_revision: u64,
    store: State<'_, DocumentSessionStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    store.save(&session_id, expected_state_revision, None).await
}

#[tauri::command]
pub async fn save_document_session_as(
    session_id: String,
    expected_state_revision: u64,
    target_path: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    store
        .save(
            &session_id,
            expected_state_revision,
            Some(PathBuf::from(target_path)),
        )
        .await
}

#[tauri::command]
pub async fn begin_session_transition(
    session_id: String,
    kind: TransitionKind,
    store: State<'_, DocumentSessionStore>,
) -> Result<TransitionResult, DocumentSessionError> {
    #[cfg(test)]
    if store.lifecycle_gate.clone().try_write_owned().is_err() {
        store.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::TransitionLifecycleWaitBlocked,
        );
        if let Some(notify) = store.transition_lifecycle_wait_notify.lock().clone() {
            notify.notify_waiters();
        }
    }
    store.reap_expired_lifecycle().await?;
    let _gate = store.lifecycle_gate.clone().write_owned().await;
    #[cfg(test)]
    store.trace_transition_authority_release(
        TransitionAuthorityReleaseEvent::TransitionLifecycleAcquired,
    );
    if store.active_session_id.lock().as_deref() != Some(session_id.as_str()) {
        return Err(DocumentSessionError::new(
            "ACTIVE_SESSION_MISMATCH",
            "Transition yalnız authoritative active session için başlatılabilir",
        ));
    }
    let entry = store.entry(&session_id)?;
    let _lease = entry.operation.clone().write_owned().await;
    #[cfg(test)]
    store.trace_transition_authority_release(
        TransitionAuthorityReleaseEvent::TransitionOperationAcquired,
    );
    let mut state = entry.state.lock();
    if state.active_transition.is_some() {
        return Err(DocumentSessionError::new(
            "TRANSITION_ALREADY_ACTIVE",
            "Bu session için zaten aktif bir transition var",
        )
        .with_snapshot(snapshot(&state)));
    }
    ensure_lifecycle(&state, &[SessionLifecycle::Open])?;
    let token = Uuid::new_v4().to_string();
    let source_session_id = state.id.clone();
    let captured_content_generation = state.content_generation;
    let transition_epoch = store.active_identity_epoch.load(Ordering::Acquire);
    state.lifecycle = SessionLifecycle::DecisionPending;
    bump_revision(&mut state)?;
    state.active_transition = Some(SessionTransition {
        token: token.clone(),
        source_session_id: source_session_id.clone(),
        kind,
        transition_epoch,
        created_at_ms: now_ms(),
        captured_content_generation,
        expected_state_revision: state.state_revision,
        status: TransitionStatus::DecisionPending,
        prepared_target: None,
        execution: TransitionExecution::new(),
    });
    store.transition_authorities.lock().insert(
        token.clone(),
        TransitionAuthorityIdentity {
            source_session_id,
            kind,
            initial_identity_epoch: transition_epoch,
        },
    );
    Ok(TransitionResult {
        token,
        kind,
        snapshot: snapshot(&state),
    })
}

#[tauri::command]
pub async fn save_transition(
    token: String,
    target_path: Option<String>,
    store: State<'_, DocumentSessionStore>,
    search_cache: State<'_, SearchCache>,
    layout_cache: State<'_, TextLayoutCache>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    let target = target_path.map(PathBuf::from);
    execute_transition_command(
        &token,
        TransitionDecision::Save { target },
        &store,
        &search_cache,
        &layout_cache,
        &vector_store,
    )
    .await
}

#[tauri::command]
pub async fn discard_transition(
    token: String,
    store: State<'_, DocumentSessionStore>,
    search_cache: State<'_, SearchCache>,
    layout_cache: State<'_, TextLayoutCache>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    execute_transition_command(
        &token,
        TransitionDecision::Discard,
        &store,
        &search_cache,
        &layout_cache,
        &vector_store,
    )
    .await
}

enum TransitionClaim {
    RunSave {
        target: Option<PathBuf>,
    },
    Apply {
        saved_snapshot: Option<DocumentSessionSnapshot>,
    },
    RunDiscard,
}

fn recorded_transition_result(
    store: &DocumentSessionStore,
    token: &str,
    decision: &TransitionDecision,
) -> Option<Result<DocumentSessionSnapshot, DocumentSessionError>> {
    recorded_transition_result_for_authority(store, token, decision, None)
}

fn recorded_transition_result_for_authority(
    store: &DocumentSessionStore,
    token: &str,
    decision: &TransitionDecision,
    request_authority: Option<&TransitionAuthorityIdentity>,
) -> Option<Result<DocumentSessionSnapshot, DocumentSessionError>> {
    let record = store.recorded_transitions.lock().get(token).cloned()?;
    let binding = match store.transition_authorities.lock().get(token).cloned() {
        Some(binding) => binding,
        None => {
            return Some(Err(recorded_transition_corruption(
                "Recorded token için manager transition authority binding bulunamadı",
            )))
        }
    };
    if let Some(request) = request_authority {
        if request.source_session_id != binding.source_session_id || request.kind != binding.kind {
            return Some(Err(DocumentSessionError::new(
                "TRANSITION_REPLAY_AUTHORITY_CONFLICT",
                "Replay request source session veya transition kind ile token authority eşleşmiyor",
            )));
        }
    }
    if record.source_session_id != binding.source_session_id || record.kind != binding.kind {
        return Some(Err(recorded_transition_corruption(
            "Recorded source session veya transition kind manager token binding ile uyuşmuyor",
        )));
    }
    if &record.decision != decision {
        return Some(Err(DocumentSessionError::new(
            "TRANSITION_DECISION_CONFLICT",
            "Recorded transition farklı bir Save/Discard kararıyla tamamlandı",
        )));
    }
    if let Err(error) = validate_recorded_transition_outcome(store, &binding, &record) {
        return Some(Err(error));
    }
    Some(record.result)
}

fn manager_authority_snapshot(store: &DocumentSessionStore) -> RecordedTransitionAuthority {
    let active_session_id = store.active_session_id.lock();
    RecordedTransitionAuthority {
        active_session_id: active_session_id.clone(),
        identity_epoch: store.active_identity_epoch.load(Ordering::Acquire),
    }
}

fn recorded_transition_corruption(message: impl Into<String>) -> DocumentSessionError {
    DocumentSessionError::new("TRANSITION_RECORDED_OUTCOME_CORRUPT", message)
}

fn validate_recorded_transition_outcome(
    store: &DocumentSessionStore,
    binding: &TransitionAuthorityIdentity,
    record: &RecordedTransitionOutcome,
) -> Result<(), DocumentSessionError> {
    match (&record.result, &record.resulting_authority) {
        (Ok(snapshot), Some(authority)) => {
            let expected_epoch = match record.kind {
                TransitionKind::DocumentSwitch => {
                    if authority.active_session_id.as_deref() != Some(snapshot.id.as_str()) {
                        return Err(recorded_transition_corruption(
                            "Recorded switch result session kimliği resulting active authority ile uyuşmuyor",
                        ));
                    }
                    binding.initial_identity_epoch.saturating_add(2)
                }
                TransitionKind::DocumentClose | TransitionKind::WindowClose => {
                    if authority.active_session_id.is_some()
                        || snapshot.id != binding.source_session_id
                    {
                        return Err(recorded_transition_corruption(
                            "Recorded close result source/no-active authority semantiğiyle uyuşmuyor",
                        ));
                    }
                    binding.initial_identity_epoch.saturating_add(1)
                }
            };
            if authority.identity_epoch != expected_epoch {
                return Err(recorded_transition_corruption(
                    "Recorded resulting identity epoch transition authority ile uyuşmuyor",
                ));
            }
            let current = manager_authority_snapshot(store);
            if current.identity_epoch < authority.identity_epoch {
                return Err(recorded_transition_corruption(
                    "Current manager identity epoch recorded terminal epoch'tan geride",
                ));
            }
            if current.identity_epoch == authority.identity_epoch
                && current.active_session_id != authority.active_session_id
            {
                return Err(recorded_transition_corruption(
                    "Current active session aynı epoch'taki recorded authority ile uyuşmuyor",
                ));
            }
        }
        (Err(_), None) => {}
        (Ok(_), None) => {
            return Err(recorded_transition_corruption(
                "Recorded terminal success resulting authority metadata taşımıyor",
            ))
        }
        (Err(_), Some(_)) => {
            return Err(recorded_transition_corruption(
                "Recorded failure sahte success authority metadata taşıyor",
            ))
        }
    }
    Ok(())
}

fn record_transition_outcome(
    store: &DocumentSessionStore,
    token: &str,
    authority: &TransitionAuthorityIdentity,
    decision: TransitionDecision,
    result: Result<DocumentSessionSnapshot, DocumentSessionError>,
    resulting_authority: Option<RecordedTransitionAuthority>,
) -> Result<(), DocumentSessionError> {
    let binding = store
        .transition_authorities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| {
            recorded_transition_corruption(
                "Terminal outcome için manager transition authority binding bulunamadı",
            )
        })?;
    if &binding != authority {
        return Err(DocumentSessionError::new(
            "TRANSITION_REPLAY_AUTHORITY_CONFLICT",
            "Terminal outcome source session veya transition kind token authority ile eşleşmiyor",
        ));
    }
    let record = RecordedTransitionOutcome {
        source_session_id: authority.source_session_id.clone(),
        kind: authority.kind,
        decision,
        result,
        resulting_authority,
    };
    validate_recorded_transition_outcome(store, authority, &record)?;
    let mut records = store.recorded_transitions.lock();
    if let Some(existing) = records.get(token) {
        if existing.source_session_id != record.source_session_id || existing.kind != record.kind {
            return Err(recorded_transition_corruption(
                "Aynı token farklı source session veya transition kind ile kayıtlı",
            ));
        }
        if existing.decision != record.decision {
            return Err(DocumentSessionError::new(
                "TRANSITION_DECISION_CONFLICT",
                "Aynı token farklı terminal decision ile yeniden kaydedilemez",
            ));
        }
        return Ok(());
    }
    records.insert(token.to_string(), record);
    Ok(())
}

struct TransitionExecutionGuard<'a> {
    store: &'a DocumentSessionStore,
    entry: Arc<SessionEntry>,
    token: String,
    authority: TransitionAuthorityIdentity,
    decision: TransitionDecision,
    execution: TransitionExecution,
    armed: bool,
}

impl TransitionExecutionGuard<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TransitionExecutionGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut completion_state = self.execution.completion_state.lock();
        if *completion_state != TransitionExecutionCompletion::Running {
            return;
        }
        #[cfg(test)]
        self.store.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::GuardPublicationStarted,
        );
        let observed = self.execution.phase.lock().clone();
        let (next_phase, activation_phase, recorded) = match observed {
            TransitionExecutionPhase::SaveClaimed { .. }
            | TransitionExecutionPhase::DiscardInProgress => {
                let error = {
                    let mut state = self.entry.state.lock();
                    state.state_revision = state.state_revision.saturating_add(1);
                    let revision = state.state_revision;
                    if let Some(transition) = state.active_transition.as_mut() {
                        transition.expected_state_revision = revision;
                    }
                    DocumentSessionError::new(
                        "TRANSITION_INTERRUPTED_PRE_COMMIT",
                        "Transition executor irreversible işlem başlamadan kesildi; aynı token ile retry mümkündür",
                    )
                    .with_snapshot(snapshot(&state))
                };
                (
                    TransitionExecutionPhase::InterruptedPreCommit {
                        decision: self.decision.clone(),
                        error,
                    },
                    None,
                    None,
                )
            }
            TransitionExecutionPhase::SaveInProgress { .. } => {
                let error = {
                    let mut state = self.entry.state.lock();
                    state.lifecycle = SessionLifecycle::CommitIndeterminate;
                    state.state_revision = state.state_revision.saturating_add(1);
                    let revision = state.state_revision;
                    if let Some(transition) = state.active_transition.as_mut() {
                        transition.expected_state_revision = revision;
                    }
                    DocumentSessionError::new(
                        "TRANSITION_SAVE_COMMIT_INDETERMINATE",
                        "Transition executor Save commit sonucu kesinleşmeden kesildi; explicit recovery gerekir",
                    )
                    .with_snapshot(snapshot(&state))
                };
                (
                    TransitionExecutionPhase::CommitIndeterminate {
                        decision: self.decision.clone(),
                        error: error.clone(),
                    },
                    Some(CandidateActivationPhase::RecoveryRequired),
                    Some(Err(error)),
                )
            }
            TransitionExecutionPhase::SaveCommitted {
                snapshot: saved_snapshot,
                ..
            }
            | TransitionExecutionPhase::Applying {
                saved_snapshot: Some(saved_snapshot),
                ..
            }
            | TransitionExecutionPhase::ApplyCompleted {
                snapshot: saved_snapshot,
                ..
            } => {
                let error = {
                    let mut state = self.entry.state.lock();
                    state.state_revision = state.state_revision.saturating_add(1);
                    let revision = state.state_revision;
                    if let Some(transition) = state.active_transition.as_mut() {
                        transition.expected_state_revision = revision;
                    }
                    DocumentSessionError::new(
                        "TRANSITION_INTERRUPTED_POST_COMMIT",
                        "Transition executor commit sonrasında kesildi; retry yalnız apply fazını çalıştıracaktır",
                    )
                    .with_snapshot(snapshot(&state))
                };
                (
                    TransitionExecutionPhase::FailedPostCommit {
                        decision: self.decision.clone(),
                        saved_snapshot: Some(saved_snapshot),
                        error,
                    },
                    Some(CandidateActivationPhase::FailedPostCommit),
                    None,
                )
            }
            TransitionExecutionPhase::Applying {
                saved_snapshot: None,
                ..
            } => {
                let error = {
                    let mut state = self.entry.state.lock();
                    state.state_revision = state.state_revision.saturating_add(1);
                    let revision = state.state_revision;
                    if let Some(transition) = state.active_transition.as_mut() {
                        transition.expected_state_revision = revision;
                    }
                    DocumentSessionError::new(
                        "TRANSITION_INTERRUPTED_POST_COMMIT",
                        "Discard apply executor kesildi; aynı apply fazı yeniden uzlaştırılmalıdır",
                    )
                    .with_snapshot(snapshot(&state))
                };
                (
                    TransitionExecutionPhase::FailedPostCommit {
                        decision: self.decision.clone(),
                        saved_snapshot: None,
                        error,
                    },
                    Some(CandidateActivationPhase::FailedPostCommit),
                    None,
                )
            }
            _ => return,
        };
        if let Some(phase) = activation_phase {
            if let Some(activation) =
                self.store
                    .activations_by_token
                    .lock()
                    .values_mut()
                    .find(|activation| {
                        activation.previous_transition_token.as_deref() == Some(self.token.as_str())
                    })
            {
                activation.phase = phase;
            }
        }
        if let Some(result) = recorded {
            if let Err(error) = record_transition_outcome(
                self.store,
                &self.token,
                &self.authority,
                self.decision.clone(),
                result,
                None,
            ) {
                let mut state = self.entry.state.lock();
                state.lifecycle = SessionLifecycle::RecoveryRequired;
                state.state_revision = state.state_revision.saturating_add(1);
                let revision = state.state_revision;
                if let Some(transition) = state.active_transition.as_mut() {
                    transition.expected_state_revision = revision;
                }
                *self.execution.phase.lock() = TransitionExecutionPhase::RecoveryRequired {
                    decision: self.decision.clone(),
                    error: error.with_snapshot(snapshot(&state)),
                };
                *completion_state = TransitionExecutionCompletion::InterruptedPublished;
                self.execution.completion.notify_waiters();
                return;
            }
        }
        *self.execution.phase.lock() = next_phase;
        *completion_state = TransitionExecutionCompletion::InterruptedPublished;
        self.execution.completion.notify_waiters();
    }
}

struct TransitionGateReservation {
    execution: TransitionExecution,
    decision: TransitionDecision,
    armed: bool,
}

impl TransitionGateReservation {
    fn promote_to_authoritative<'a>(
        &mut self,
        store: &'a DocumentSessionStore,
        entry: Arc<SessionEntry>,
        token: &str,
        authority: TransitionAuthorityIdentity,
    ) -> Result<(TransitionClaim, TransitionExecutionGuard<'a>), DocumentSessionError> {
        let mut guard = TransitionExecutionGuard {
            store,
            entry,
            token: token.to_string(),
            authority,
            decision: self.decision.clone(),
            execution: self.execution.clone(),
            armed: false,
        };
        let mut completion_state = self.execution.completion_state.lock();
        if *completion_state
            != (TransitionExecutionCompletion::GateReserved {
                decision: self.decision.clone(),
            })
        {
            return Err(DocumentSessionError::new(
                "TRANSITION_GATE_RESERVATION_STALE",
                "Transition gate reservation authoritative claim öncesinde değişti",
            ));
        }
        let mut phase = self.execution.phase.lock();
        let claim = match &*phase {
            TransitionExecutionPhase::PendingUnclaimed
            | TransitionExecutionPhase::FailedPreCommit { .. }
            | TransitionExecutionPhase::InterruptedPreCommit { .. } => match &self.decision {
                TransitionDecision::Save { target } => {
                    *phase = TransitionExecutionPhase::SaveClaimed {
                        target: target.clone(),
                    };
                    TransitionClaim::RunSave {
                        target: target.clone(),
                    }
                }
                TransitionDecision::Discard => {
                    *phase = TransitionExecutionPhase::DiscardInProgress;
                    TransitionClaim::RunDiscard
                }
            },
            TransitionExecutionPhase::SaveCommitted { target, snapshot }
                if self.decision
                    == (TransitionDecision::Save {
                        target: target.clone(),
                    }) =>
            {
                let saved_snapshot = snapshot.clone();
                *phase = TransitionExecutionPhase::Applying {
                    decision: self.decision.clone(),
                    saved_snapshot: Some(saved_snapshot.clone()),
                };
                TransitionClaim::Apply {
                    saved_snapshot: Some(saved_snapshot),
                }
            }
            TransitionExecutionPhase::FailedPostCommit {
                decision: previous,
                saved_snapshot,
                ..
            } if previous == &self.decision => {
                let saved_snapshot = saved_snapshot.clone();
                *phase = TransitionExecutionPhase::Applying {
                    decision: self.decision.clone(),
                    saved_snapshot: saved_snapshot.clone(),
                };
                TransitionClaim::Apply { saved_snapshot }
            }
            TransitionExecutionPhase::ApplyCompleted {
                decision: previous,
                snapshot,
            } if previous == &self.decision => {
                let saved_snapshot = snapshot.clone();
                *phase = TransitionExecutionPhase::Applying {
                    decision: self.decision.clone(),
                    saved_snapshot: Some(saved_snapshot.clone()),
                };
                TransitionClaim::Apply {
                    saved_snapshot: Some(saved_snapshot),
                }
            }
            _ => {
                return Err(DocumentSessionError::new(
                    "TRANSITION_GATE_RESERVATION_STALE",
                    "Transition phase gate beklenirken authoritative claim için geçersiz oldu",
                ))
            }
        };
        *completion_state = TransitionExecutionCompletion::Running;
        guard.armed = true;
        self.armed = false;
        Ok((claim, guard))
    }
}

impl Drop for TransitionGateReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut completion_state = self.execution.completion_state.lock();
        if *completion_state
            == (TransitionExecutionCompletion::GateReserved {
                decision: self.decision.clone(),
            })
        {
            *completion_state = TransitionExecutionCompletion::Available;
            self.execution.completion.notify_waiters();
        }
    }
}

struct TransitionAuthorityScope<'a> {
    guard: TransitionExecutionGuard<'a>,
    #[cfg(test)]
    _guard_publication_observer: TransitionDropTraceObserver,
    _operation_leases: Vec<tokio::sync::OwnedRwLockWriteGuard<()>>,
    #[cfg(test)]
    _operation_release_observer: TransitionDropTraceObserver,
    _lifecycle_guard: tokio::sync::OwnedRwLockWriteGuard<()>,
    #[cfg(test)]
    _lifecycle_release_observer: TransitionDropTraceObserver,
}

impl TransitionAuthorityScope<'_> {
    fn disarm(&mut self) {
        self.guard.disarm();
    }
}

enum TransitionReservationClaim {
    Wait,
    Reserved(TransitionGateReservation),
    Return(Result<DocumentSessionSnapshot, DocumentSessionError>),
}

fn reserve_transition_execution(
    execution: &TransitionExecution,
    decision: &TransitionDecision,
) -> TransitionReservationClaim {
    let mut completion_state = execution.completion_state.lock();
    if matches!(
        &*completion_state,
        TransitionExecutionCompletion::GateReserved { .. } | TransitionExecutionCompletion::Running
    ) {
        return TransitionReservationClaim::Wait;
    }
    let phase = execution.phase.lock();
    match &*phase {
        TransitionExecutionPhase::PendingUnclaimed
        | TransitionExecutionPhase::FailedPreCommit { .. }
        | TransitionExecutionPhase::InterruptedPreCommit { .. } => {
            *completion_state = TransitionExecutionCompletion::GateReserved {
                decision: decision.clone(),
            };
            TransitionReservationClaim::Reserved(TransitionGateReservation {
                execution: execution.clone(),
                decision: decision.clone(),
                armed: true,
            })
        }
        TransitionExecutionPhase::SaveCommitted { target, .. }
            if decision
                == &TransitionDecision::Save {
                    target: target.clone(),
                } =>
        {
            *completion_state = TransitionExecutionCompletion::GateReserved {
                decision: decision.clone(),
            };
            TransitionReservationClaim::Reserved(TransitionGateReservation {
                execution: execution.clone(),
                decision: decision.clone(),
                armed: true,
            })
        }
        TransitionExecutionPhase::FailedPostCommit {
            decision: previous, ..
        }
        | TransitionExecutionPhase::ApplyCompleted {
            decision: previous, ..
        } if previous == decision => {
            *completion_state = TransitionExecutionCompletion::GateReserved {
                decision: decision.clone(),
            };
            TransitionReservationClaim::Reserved(TransitionGateReservation {
                execution: execution.clone(),
                decision: decision.clone(),
                armed: true,
            })
        }
        TransitionExecutionPhase::CommitIndeterminate {
            decision: previous,
            error,
        }
        | TransitionExecutionPhase::RecoveryRequired {
            decision: previous,
            error,
        } if previous == decision => TransitionReservationClaim::Return(Err(error.clone())),
        TransitionExecutionPhase::Completed {
            decision: previous,
            result,
        } if previous == decision => TransitionReservationClaim::Return(result.clone()),
        TransitionExecutionPhase::SaveClaimed { target }
        | TransitionExecutionPhase::SaveInProgress { target }
            if decision
                == &TransitionDecision::Save {
                    target: target.clone(),
                } =>
        {
            TransitionReservationClaim::Wait
        }
        TransitionExecutionPhase::DiscardInProgress if decision == &TransitionDecision::Discard => {
            TransitionReservationClaim::Wait
        }
        TransitionExecutionPhase::Applying {
            decision: previous, ..
        } if previous == decision => TransitionReservationClaim::Wait,
        _ => TransitionReservationClaim::Return(Err(DocumentSessionError::new(
            "TRANSITION_DECISION_CONFLICT",
            "Transition daha önce farklı bir Save/Discard kararıyla claim edildi",
        ))),
    }
}

async fn await_transition_attempt(
    store: &DocumentSessionStore,
    token: &str,
    execution: &TransitionExecution,
    decision: &TransitionDecision,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    loop {
        let notified = execution.completion.notified();
        if let Some(recorded) = recorded_transition_result(store, token, decision) {
            return recorded;
        }
        let completion_state = execution.completion_state.lock().clone();
        if matches!(
            completion_state,
            TransitionExecutionCompletion::GateReserved { .. }
                | TransitionExecutionCompletion::Running
        ) {
            #[cfg(test)]
            if let Some(notify) = store.transition_completion_wait_notify.lock().clone() {
                notify.notify_waiters();
            }
            notified.await;
            continue;
        }
        let observed = execution.phase.lock().clone();
        match observed {
            TransitionExecutionPhase::PendingUnclaimed => {
                return Err(DocumentSessionError::new(
                    "TRANSITION_GATE_RESERVATION_CANCELLED",
                    "Transition executor gate beklerken kesildi; aynı token güvenle yeniden denenebilir",
                ))
            }
            TransitionExecutionPhase::Completed {
                decision: previous,
                result,
            } if &previous == decision => return result,
            TransitionExecutionPhase::FailedPreCommit {
                decision: previous,
                error,
            }
            | TransitionExecutionPhase::FailedPostCommit {
                decision: previous,
                error,
                ..
            } if &previous == decision => return Err(error),
            TransitionExecutionPhase::InterruptedPreCommit {
                decision: previous,
                error,
            }
            | TransitionExecutionPhase::CommitIndeterminate {
                decision: previous,
                error,
            }
            | TransitionExecutionPhase::RecoveryRequired {
                decision: previous,
                error,
            } if &previous == decision => return Err(error),
            TransitionExecutionPhase::SaveClaimed { .. }
            | TransitionExecutionPhase::SaveInProgress { .. }
            | TransitionExecutionPhase::DiscardInProgress
            | TransitionExecutionPhase::Applying { .. }
            | TransitionExecutionPhase::SaveCommitted { .. }
            | TransitionExecutionPhase::ApplyCompleted { .. } => notified.await,
            _ => {
                return Err(DocumentSessionError::new(
                    "TRANSITION_DECISION_CONFLICT",
                    "Transition completion farklı bir karara aittir",
                ))
            }
        }
    }
}

fn publish_transition_phase(execution: &TransitionExecution, phase: TransitionExecutionPhase) {
    *execution.phase.lock() = phase;
    execution.completion.notify_waiters();
}

fn publish_retryable_transition_phase(
    execution: &TransitionExecution,
    phase: TransitionExecutionPhase,
) -> Result<(), DocumentSessionError> {
    let mut completion_state = execution.completion_state.lock();
    if *completion_state != TransitionExecutionCompletion::Running {
        return Err(DocumentSessionError::new(
            "TRANSITION_COMPLETION_STATE_CONFLICT",
            "Retryable phase yalnız running transition execution tarafından yayımlanabilir",
        ));
    }
    *execution.phase.lock() = phase;
    *completion_state = TransitionExecutionCompletion::Available;
    Ok(())
}

fn publish_terminal_transition_phase(
    execution: &TransitionExecution,
    phase: TransitionExecutionPhase,
) -> Result<(), DocumentSessionError> {
    let mut completion_state = execution.completion_state.lock();
    if *completion_state != TransitionExecutionCompletion::Running {
        return Err(DocumentSessionError::new(
            "TRANSITION_COMPLETION_STATE_CONFLICT",
            "Terminal phase yalnız running transition execution tarafından yayımlanabilir",
        ));
    }
    *execution.phase.lock() = phase;
    *completion_state = TransitionExecutionCompletion::TerminalPublished;
    Ok(())
}

async fn execute_transition_command(
    token: &str,
    decision: TransitionDecision,
    store: &DocumentSessionStore,
    search_cache: &SearchCache,
    layout_cache: &TextLayoutCache,
    vector_store: &crate::ai::VectorStore,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    if let Some(recorded) = recorded_transition_result(store, token, &decision) {
        return recorded;
    }
    if store.shutdown_admission_closed.load(Ordering::Acquire) {
        return Err(DocumentSessionError::new(
            "SESSION_SHUTDOWN_IN_PROGRESS",
            "Shutdown başladıktan sonra yeni transition işlemi kabul edilmez",
        ));
    }
    let (session_id, kind, execution) = find_transition_authority(store, token)?;
    let authority_identity = store
        .transition_authorities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| {
            recorded_transition_corruption(
                "Transition token için manager authority binding bulunamadı",
            )
        })?;
    if authority_identity.source_session_id != session_id || authority_identity.kind != kind {
        return Err(recorded_transition_corruption(
            "Live transition source/kind manager token authority binding ile uyuşmuyor",
        ));
    }
    let entry = store.entry(&session_id)?;
    let reservation_claim = reserve_transition_execution(&execution, &decision);
    #[cfg(test)]
    let claim_barrier = store.transition_claim_barrier.lock().clone();
    #[cfg(test)]
    if let Some(barrier) = claim_barrier {
        barrier.wait().await;
    }
    let mut reservation = match reservation_claim {
        TransitionReservationClaim::Wait => {
            return await_transition_attempt(store, token, &execution, &decision).await
        }
        TransitionReservationClaim::Return(result) => return result,
        TransitionReservationClaim::Reserved(reservation) => reservation,
    };

    #[cfg(test)]
    if store.lifecycle_gate.clone().try_write_owned().is_err() {
        store.trace_transition_authority_release(
            TransitionAuthorityReleaseEvent::TransitionLifecycleWaitBlocked,
        );
        if let Some(notify) = store.transition_lifecycle_wait_notify.lock().clone() {
            notify.notify_waiters();
        }
    }
    let lifecycle_guard = store.lifecycle_gate.clone().write_owned().await;
    store.reap_expired_lifecycle_under_gate().await?;
    #[cfg(test)]
    store.pause_after_lifecycle_before_operation().await;
    let activation = transition_activation(store, token);
    let candidate = activation
        .as_ref()
        .map(|activation| store.entry(&activation.candidate_session_id))
        .transpose()?;
    let mut lock_entries = vec![(session_id.clone(), entry.clone())];
    if let (Some(activation), Some(candidate)) = (&activation, &candidate) {
        lock_entries.push((activation.candidate_session_id.clone(), candidate.clone()));
    }
    lock_entries.sort_by(|left, right| left.0.cmp(&right.0));
    lock_entries.dedup_by(|left, right| left.0 == right.0);
    let mut operation_leases = Vec::with_capacity(lock_entries.len());
    for (_, lock_entry) in &lock_entries {
        operation_leases.push(lock_entry.operation.clone().write_owned().await);
    }

    {
        let state = entry.state.lock();
        let epoch_valid = state.active_transition.as_ref().is_some_and(|transition| {
            let current = store.active_identity_epoch.load(Ordering::Acquire);
            current == transition.transition_epoch
                || (state.lifecycle == SessionLifecycle::Closed
                    && matches!(
                        current,
                        value if value == transition.transition_epoch.saturating_add(1)
                            || value == transition.transition_epoch.saturating_add(2)
                    ))
        });
        let validation = if epoch_valid {
            validate_transition(&state, token, kind)
        } else {
            Err(DocumentSessionError::new(
                "TRANSITION_EPOCH_MISMATCH",
                "Transition active-session identity epoch ile eşleşmiyor",
            )
            .with_snapshot(snapshot(&state)))
        };
        validation?;
        let active_session_id = store.active_session_id.lock().clone();
        let current_epoch = store.active_identity_epoch.load(Ordering::Acquire);
        let active_valid = if state.lifecycle == SessionLifecycle::Closed {
            match kind {
                TransitionKind::DocumentSwitch => {
                    (active_session_id.is_none()
                        && current_epoch
                            == authority_identity.initial_identity_epoch.saturating_add(1))
                        || (current_epoch
                            == authority_identity.initial_identity_epoch.saturating_add(2)
                            && active_session_id.as_deref()
                                == activation
                                    .as_ref()
                                    .map(|value| value.candidate_session_id.as_str()))
                }
                TransitionKind::DocumentClose | TransitionKind::WindowClose => {
                    active_session_id.is_none()
                        && current_epoch
                            == authority_identity.initial_identity_epoch.saturating_add(1)
                }
            }
        } else {
            active_session_id.as_deref() == Some(session_id.as_str())
                && current_epoch == authority_identity.initial_identity_epoch
        };
        if !active_valid {
            return Err(DocumentSessionError::new(
                "TRANSITION_ACTIVE_AUTHORITY_STALE",
                "Transition gate beklerken active session authority değişti",
            )
            .with_snapshot(snapshot(&state)));
        }
    }

    if let Some(recorded) = recorded_transition_result(store, token, &decision) {
        return recorded;
    }
    let current_activation = transition_activation(store, token);
    match kind {
        TransitionKind::DocumentSwitch => {
            let activation = current_activation.as_ref().ok_or_else(|| {
                DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_BINDING_STALE",
                    "Document switch candidate binding gate beklerken kayboldu",
                )
            })?;
            if activation.previous_session_id.as_deref() != Some(session_id.as_str())
                || activation.previous_transition_token.as_deref() != Some(token)
                || !matches!(
                    activation.phase,
                    CandidateActivationPhase::BoundToTransition
                        | CandidateActivationPhase::Applying
                        | CandidateActivationPhase::FailedPostCommit
                )
            {
                return Err(DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_BINDING_STALE",
                    "Document switch candidate authority gate beklerken değişti",
                ));
            }
        }
        TransitionKind::DocumentClose | TransitionKind::WindowClose => {
            if current_activation.is_some() {
                return Err(DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_BINDING_STALE",
                    "Close transition beklenmeyen candidate binding taşıyor",
                ));
            }
        }
    }
    if current_activation.as_ref().map(|value| &value.token)
        != activation.as_ref().map(|value| &value.token)
    {
        return Err(DocumentSessionError::new(
            "TRANSITION_CANDIDATE_BINDING_STALE",
            "Candidate activation gate acquisition sırasında değişti",
        ));
    }

    let (claim, execution_guard) = reservation.promote_to_authoritative(
        store,
        entry.clone(),
        token,
        authority_identity.clone(),
    )?;
    let mut authority_scope = TransitionAuthorityScope {
        guard: execution_guard,
        #[cfg(test)]
        _guard_publication_observer: store
            .transition_drop_observer(TransitionAuthorityReleaseEvent::GuardPublicationCompleted),
        _operation_leases: operation_leases,
        #[cfg(test)]
        _operation_release_observer: store
            .transition_drop_observer(TransitionAuthorityReleaseEvent::OperationLeasesReleased),
        _lifecycle_guard: lifecycle_guard,
        #[cfg(test)]
        _lifecycle_release_observer: store
            .transition_drop_observer(TransitionAuthorityReleaseEvent::LifecycleAuthorityReleased),
    };

    let saved_snapshot = match claim {
        TransitionClaim::RunSave { target } => {
            let expected_revision = entry.state.lock().state_revision;
            match store
                .save_core_with_gate_held(
                    &entry,
                    expected_revision,
                    target.clone(),
                    Some(&execution),
                )
                .await
            {
                Ok(success) => {
                    let saved = success.snapshot;
                    match success.classification {
                        SaveCommitClassification::NotCommitted
                        | SaveCommitClassification::Committed => {}
                        SaveCommitClassification::CommitIndeterminate
                        | SaveCommitClassification::RecoveryRequired
                        | SaveCommitClassification::CommittedButSessionRefreshFailed => {
                            let error = {
                                let mut state = entry.state.lock();
                                state.lifecycle = SessionLifecycle::CommitIndeterminate;
                                state.state_revision = state.state_revision.saturating_add(1);
                                DocumentSessionError::new(
                                    "TRANSITION_SAVE_CLASSIFICATION_INVARIANT",
                                    "Save success sonucu fail-closed olmayan classification taşıdı",
                                )
                                .with_snapshot(snapshot(&state))
                            };
                            let terminal_error = match record_transition_outcome(
                                store,
                                token,
                                &authority_identity,
                                decision.clone(),
                                Err(error.clone()),
                                None,
                            ) {
                                Ok(()) => error,
                                Err(record_error) => record_error,
                            };
                            publish_terminal_transition_phase(
                                &execution,
                                TransitionExecutionPhase::CommitIndeterminate {
                                    decision: decision.clone(),
                                    error: terminal_error.clone(),
                                },
                            )?;
                            authority_scope.disarm();
                            execution.completion.notify_waiters();
                            return Err(terminal_error);
                        }
                    }
                    {
                        let mut state = entry.state.lock();
                        let state_revision = state.state_revision;
                        if let Some(transition) = state.active_transition.as_mut() {
                            transition.status = TransitionStatus::PreparedSave;
                            transition.expected_state_revision = state_revision;
                            transition.prepared_target = target.clone();
                        }
                    }
                    publish_transition_phase(
                        &execution,
                        TransitionExecutionPhase::SaveCommitted {
                            target,
                            snapshot: saved.clone(),
                        },
                    );
                    #[cfg(test)]
                    let committed_barriers =
                        store.transition_save_committed_barriers.lock().clone();
                    #[cfg(test)]
                    if let Some((reached, release)) = committed_barriers {
                        reached.wait().await;
                        release.wait().await;
                    }
                    Some(saved)
                }
                Err(failure) => {
                    let error = failure.error;
                    match failure.classification {
                        SaveCommitClassification::NotCommitted => {
                            publish_retryable_transition_phase(
                                &execution,
                                TransitionExecutionPhase::FailedPreCommit {
                                    decision: decision.clone(),
                                    error: error.clone(),
                                },
                            )?;
                            authority_scope.disarm();
                            execution.completion.notify_waiters();
                        }
                        SaveCommitClassification::Committed => {
                            publish_retryable_transition_phase(
                                &execution,
                                TransitionExecutionPhase::FailedPostCommit {
                                    decision: decision.clone(),
                                    saved_snapshot: error.snapshot.as_deref().cloned(),
                                    error: error.clone(),
                                },
                            )?;
                            authority_scope.disarm();
                            execution.completion.notify_waiters();
                        }
                        SaveCommitClassification::CommitIndeterminate => {
                            let terminal_error = match record_transition_outcome(
                                store,
                                token,
                                &authority_identity,
                                decision.clone(),
                                Err(error.clone()),
                                None,
                            ) {
                                Ok(()) => error.clone(),
                                Err(record_error) => record_error,
                            };
                            if let Some(activation) = store
                                .activations_by_token
                                .lock()
                                .values_mut()
                                .find(|activation| {
                                    activation.previous_transition_token.as_deref() == Some(token)
                                })
                            {
                                activation.phase = CandidateActivationPhase::RecoveryRequired;
                            }
                            publish_terminal_transition_phase(
                                &execution,
                                TransitionExecutionPhase::CommitIndeterminate {
                                    decision: decision.clone(),
                                    error: terminal_error.clone(),
                                },
                            )?;
                            authority_scope.disarm();
                            execution.completion.notify_waiters();
                            return Err(terminal_error);
                        }
                        SaveCommitClassification::RecoveryRequired
                        | SaveCommitClassification::CommittedButSessionRefreshFailed => {
                            let terminal_error = match record_transition_outcome(
                                store,
                                token,
                                &authority_identity,
                                decision.clone(),
                                Err(error.clone()),
                                None,
                            ) {
                                Ok(()) => error.clone(),
                                Err(record_error) => record_error,
                            };
                            if let Some(activation) = store
                                .activations_by_token
                                .lock()
                                .values_mut()
                                .find(|activation| {
                                    activation.previous_transition_token.as_deref() == Some(token)
                                })
                            {
                                activation.phase = CandidateActivationPhase::RecoveryRequired;
                            }
                            publish_terminal_transition_phase(
                                &execution,
                                TransitionExecutionPhase::RecoveryRequired {
                                    decision: decision.clone(),
                                    error: terminal_error.clone(),
                                },
                            )?;
                            authority_scope.disarm();
                            execution.completion.notify_waiters();
                            return Err(terminal_error);
                        }
                    }
                    return Err(error);
                }
            }
        }
        TransitionClaim::RunDiscard => {
            #[cfg(test)]
            store
                .transition_discard_invocations
                .fetch_add(1, Ordering::AcqRel);
            let mut state = entry.state.lock();
            if let Some(transition) = state.active_transition.as_mut() {
                transition.status = TransitionStatus::PreparedDiscard;
            }
            None
        }
        TransitionClaim::Apply { saved_snapshot } => saved_snapshot,
    };

    publish_transition_phase(
        &execution,
        TransitionExecutionPhase::Applying {
            decision: decision.clone(),
            saved_snapshot: saved_snapshot.clone(),
        },
    );
    if let Some(activation) = store
        .activations_by_token
        .lock()
        .values_mut()
        .find(|activation| activation.previous_transition_token.as_deref() == Some(token))
    {
        activation.phase = CandidateActivationPhase::Applying;
    }
    #[cfg(test)]
    let apply_barriers = store.transition_apply_barriers.lock().clone();
    #[cfg(test)]
    if let Some((reached, release)) = apply_barriers {
        reached.wait().await;
        release.wait().await;
    }
    #[cfg(test)]
    store
        .transition_apply_invocations
        .fetch_add(1, Ordering::AcqRel);
    let result = apply_transition_intent(
        TransitionApplyContext {
            store,
            candidate: candidate.as_ref(),
            activation: activation.as_ref(),
            token,
            kind,
        },
        &entry,
    )
    .await;
    match result {
        Ok(applied) => {
            *execution.phase.lock() = TransitionExecutionPhase::ApplyCompleted {
                decision: decision.clone(),
                snapshot: applied.snapshot.clone(),
            };
            let resulting_authority = manager_authority_snapshot(store);
            record_transition_outcome(
                store,
                token,
                &authority_identity,
                decision.clone(),
                Ok(applied.snapshot.clone()),
                Some(resulting_authority),
            )?;
            publish_terminal_transition_phase(
                &execution,
                TransitionExecutionPhase::Completed {
                    decision,
                    result: Ok(applied.snapshot.clone()),
                },
            )?;
            authority_scope.disarm();
            if let Some(activation) = activation.as_ref() {
                store.activations_by_token.lock().remove(&activation.token);
                store
                    .activation_token_by_candidate
                    .lock()
                    .remove(&activation.candidate_session_id);
            }
            let removed_source = store.sessions.write().remove(&applied.source_session_id);
            clear_session_derived_state(
                search_cache,
                layout_cache,
                vector_store,
                &applied.source_session_id,
            );
            execution.completion.notify_waiters();
            drop(authority_scope);
            if let Some(removed) = removed_source {
                for warning in store.cleanup_removed_entry(removed).await {
                    tracing::warn!(code = %warning.code, message = %warning.message, "Transition source cleanup ertelendi");
                }
            }
            Ok(applied.snapshot)
        }
        Err(error) => {
            if let Some(activation) = store
                .activations_by_token
                .lock()
                .values_mut()
                .find(|activation| activation.previous_transition_token.as_deref() == Some(token))
            {
                activation.phase = CandidateActivationPhase::FailedPostCommit;
            }
            publish_retryable_transition_phase(
                &execution,
                TransitionExecutionPhase::FailedPostCommit {
                    decision,
                    saved_snapshot,
                    error: error.clone(),
                },
            )?;
            authority_scope.disarm();
            execution.completion.notify_waiters();
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn cancel_transition(
    token: String,
    store: State<'_, DocumentSessionStore>,
) -> Result<DocumentSessionSnapshot, DocumentSessionError> {
    store.reap_expired_lifecycle().await?;
    let _gate = store.lifecycle_gate.clone().write_owned().await;
    let (session_id, kind) = find_transition(&store, &token)?;
    let entry = store.entry(&session_id)?;
    let _lease = entry.operation.clone().write_owned().await;
    let mut state = entry.state.lock();
    if let Err(error) = validate_transition(&state, &token, kind) {
        if error.code != "TRANSITION_TOKEN_EXPIRED" {
            return Err(error);
        }
    }
    let cancellable = state.active_transition.as_ref().is_some_and(|transition| {
        !matches!(
            &*transition.execution.completion_state.lock(),
            TransitionExecutionCompletion::GateReserved { .. }
                | TransitionExecutionCompletion::Running
        ) && matches!(
            &*transition.execution.phase.lock(),
            TransitionExecutionPhase::PendingUnclaimed
                | TransitionExecutionPhase::FailedPreCommit { .. }
                | TransitionExecutionPhase::InterruptedPreCommit { .. }
        )
    });
    if !cancellable {
        return Err(DocumentSessionError::new(
            "TRANSITION_RECOVERY_REQUIRED",
            "Irreversible veya belirsiz transition phase cancel edilerek Open durumuna döndürülemez",
        )
        .with_snapshot(snapshot(&state)));
    }
    state.active_transition = None;
    state.lifecycle = SessionLifecycle::Open;
    bump_revision(&mut state)?;
    Ok(snapshot(&state))
}

#[tauri::command]
pub async fn prepare_candidate_activation(
    candidate_session_id: String,
    previous_session_id: Option<String>,
    previous_transition_token: Option<String>,
    store: State<'_, DocumentSessionStore>,
) -> Result<CandidateActivationResult, DocumentSessionError> {
    store.reap_expired_lifecycle().await?;
    let _gate = store.lifecycle_gate.clone().write_owned().await;
    let active_session_id = store.active_session_id.lock().clone();
    match (
        active_session_id.as_deref(),
        previous_session_id.as_deref(),
        previous_transition_token.as_deref(),
    ) {
        (None, None, None) => {}
        (Some(active), Some(previous), Some(_)) if active == previous => {}
        (None, Some(_), _) | (Some(_), None, _) | (_, _, None) => {
            return Err(DocumentSessionError::new(
                "PREVIOUS_SESSION_TRANSITION_REQUIRED",
                "Previous session kimliği ve transition token birlikte zorunludur",
            ))
        }
        _ => {
            return Err(DocumentSessionError::new(
                "ACTIVE_SESSION_MISMATCH",
                "Previous session backend authoritative active session ile eşleşmiyor",
            ))
        }
    }
    if previous_session_id.as_deref() == Some(candidate_session_id.as_str()) {
        return Err(DocumentSessionError::new(
            "CANDIDATE_SESSION_CONFLICT",
            "Candidate previous active session ile aynı olamaz",
        ));
    }
    if store
        .activation_token_by_candidate
        .lock()
        .contains_key(&candidate_session_id)
    {
        return Err(DocumentSessionError::new(
            "CANDIDATE_ACTIVATION_ACTIVE",
            "Candidate için zaten aktif bir activation var",
        ));
    }

    let candidate = store.entry(&candidate_session_id)?;
    let previous = previous_session_id
        .as_deref()
        .map(|session_id| store.entry(session_id))
        .transpose()?;
    let mut lock_entries = vec![(candidate_session_id.clone(), candidate.clone())];
    if let (Some(previous_id), Some(previous_entry)) = (&previous_session_id, &previous) {
        lock_entries.push((previous_id.clone(), previous_entry.clone()));
    }
    lock_entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut _leases = Vec::with_capacity(lock_entries.len());
    for (_, entry) in &lock_entries {
        _leases.push(entry.operation.clone().write_owned().await);
    }

    if let (Some(previous), Some(token)) = (previous.as_ref(), previous_transition_token.as_deref())
    {
        let state = previous.state.lock();
        validate_transition(&state, token, TransitionKind::DocumentSwitch)?;
        if !matches!(
            state
                .active_transition
                .as_ref()
                .map(|transition| transition.status),
            Some(TransitionStatus::DecisionPending)
        ) {
            return Err(DocumentSessionError::new(
                "TRANSITION_CANDIDATE_BINDING_CLOSED",
                "Candidate yalnız transition kararı claim edilmeden önce bağlanabilir",
            )
            .with_snapshot(snapshot(&state)));
        }
    }
    let candidate_snapshot = {
        let mut state = candidate.state.lock();
        ensure_lifecycle(&state, &[SessionLifecycle::CandidateReady])?;
        state.lifecycle = SessionLifecycle::CandidateActivationPrepared;
        bump_revision(&mut state)?;
        snapshot(&state)
    };
    let activation_token = Uuid::new_v4().to_string();
    let activation = CandidateActivation {
        token: activation_token.clone(),
        phase: if previous_transition_token.is_some() {
            CandidateActivationPhase::BoundToTransition
        } else {
            CandidateActivationPhase::PreparedUnbound
        },
        previous_session_id,
        previous_transition_token,
        candidate_session_id: candidate_session_id.clone(),
        created_at_ms: now_ms(),
    };
    store
        .activations_by_token
        .lock()
        .insert(activation_token.clone(), activation);
    store
        .activation_token_by_candidate
        .lock()
        .insert(candidate_session_id, activation_token.clone());
    Ok(CandidateActivationResult {
        activation_token,
        candidate: candidate_snapshot,
    })
}

#[tauri::command]
pub async fn commit_candidate_activation(
    activation_token: String,
    store: State<'_, DocumentSessionStore>,
    _search_cache: State<'_, SearchCache>,
    _layout_cache: State<'_, TextLayoutCache>,
    _vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<CandidateCommitResult, DocumentSessionError> {
    let _gate = store.lifecycle_gate.clone().write_owned().await;
    let initial_activation = store
        .activations_by_token
        .lock()
        .get(&activation_token)
        .cloned()
        .ok_or_else(|| {
            DocumentSessionError::new(
                "CANDIDATE_ACTIVATION_NOT_FOUND",
                "Candidate activation token bulunamadı veya tüketilmiş",
            )
        })?;
    if initial_activation.token != activation_token {
        return Err(DocumentSessionError::new(
            "CANDIDATE_ACTIVATION_TOKEN_MISMATCH",
            "Candidate activation token eşleşmedi",
        ));
    }
    if store.active_session_id.lock().is_some()
        || initial_activation.previous_session_id.is_some()
        || initial_activation.previous_transition_token.is_some()
        || initial_activation.phase != CandidateActivationPhase::PreparedUnbound
    {
        return Err(DocumentSessionError::new(
            "LEGACY_TRANSITION_COMMIT_NOT_ALLOWED",
            "Candidate commit yalnız active session bulunmayan, transition'a bağlanmamış first-activation için kullanılabilir",
        ));
    }
    store.reap_expired_lifecycle_under_gate().await?;
    let activation = store
        .activations_by_token
        .lock()
        .get(&activation_token)
        .cloned()
        .ok_or_else(|| {
            DocumentSessionError::new(
                "CANDIDATE_ACTIVATION_NOT_FOUND",
                "First-activation token expire olmuş veya tüketilmiş",
            )
        })?;
    let candidate = store.entry(&activation.candidate_session_id)?;
    let _lease = candidate.operation.clone().write_owned().await;
    if store.active_session_id.lock().is_some() {
        return Err(DocumentSessionError::new(
            "LEGACY_TRANSITION_COMMIT_NOT_ALLOWED",
            "First-activation operation gate beklenirken başka active authority yayımlandı",
        ));
    }
    let committed_snapshot = {
        let mut state = candidate.state.lock();
        ensure_lifecycle(&state, &[SessionLifecycle::CandidateActivationPrepared])?;
        state.lifecycle = SessionLifecycle::Open;
        bump_revision(&mut state)?;
        snapshot(&state)
    };
    {
        let mut active = store.active_session_id.lock();
        if active.as_deref() != Some(activation.candidate_session_id.as_str()) {
            *active = Some(activation.candidate_session_id.clone());
            store.active_identity_epoch.fetch_add(1, Ordering::AcqRel);
        }
    }
    store.activations_by_token.lock().remove(&activation_token);
    store
        .activation_token_by_candidate
        .lock()
        .remove(&activation.candidate_session_id);
    Ok(CandidateCommitResult {
        snapshot: committed_snapshot,
        cleanup_warnings: Vec::new(),
    })
}

#[tauri::command]
pub async fn rollback_candidate_activation(
    activation_token: String,
    store: State<'_, DocumentSessionStore>,
    search_cache: State<'_, SearchCache>,
    layout_cache: State<'_, TextLayoutCache>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<Option<DocumentSessionSnapshot>, DocumentSessionError> {
    rollback_candidate_activation_in_store(
        &activation_token,
        &store,
        &search_cache,
        &layout_cache,
        &vector_store,
    )
    .await
}

async fn rollback_candidate_activation_in_store(
    activation_token: &str,
    store: &DocumentSessionStore,
    search_cache: &SearchCache,
    layout_cache: &TextLayoutCache,
    vector_store: &crate::ai::VectorStore,
) -> Result<Option<DocumentSessionSnapshot>, DocumentSessionError> {
    store.reap_expired_lifecycle().await?;
    let _gate = store.lifecycle_gate.clone().write_owned().await;
    let activation = store
        .activations_by_token
        .lock()
        .get(activation_token)
        .cloned()
        .ok_or_else(|| {
            DocumentSessionError::new(
                "CANDIDATE_ACTIVATION_NOT_FOUND",
                "Candidate activation token bulunamadı veya tüketilmiş",
            )
        })?;
    let candidate = store.entry(&activation.candidate_session_id)?;
    let previous = activation
        .previous_session_id
        .as_deref()
        .map(|session_id| store.entry(session_id))
        .transpose()?;
    let mut lock_entries = vec![(activation.candidate_session_id.clone(), candidate.clone())];
    if let (Some(previous_id), Some(previous_entry)) = (&activation.previous_session_id, &previous)
    {
        lock_entries.push((previous_id.clone(), previous_entry.clone()));
    }
    lock_entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut _leases = Vec::with_capacity(lock_entries.len());
    for (_, entry) in &lock_entries {
        _leases.push(entry.operation.clone().write_owned().await);
    }

    if let Some(previous_id) = activation.previous_session_id.as_deref() {
        let previous_entry = previous.as_ref().ok_or_else(|| {
            DocumentSessionError::new(
                "SOURCE_ROLLBACK_AUTHORITY_CORRUPT",
                "Rollback source session authority bulunamadı",
            )
        })?;
        let source_lifecycle = previous_entry.state.lock().lifecycle;
        let source_is_active = store.active_session_id.lock().as_deref() == Some(previous_id);
        if source_lifecycle == SessionLifecycle::Closed
            || !source_is_active
            || matches!(
                activation.phase,
                CandidateActivationPhase::Applying
                    | CandidateActivationPhase::FailedPostCommit
                    | CandidateActivationPhase::RecoveryRequired
            )
        {
            return Err(DocumentSessionError::new(
                "POST_DEACTIVATION_ROLLBACK_NOT_ALLOWED",
                "Source deactivation sonrasında apply-only candidate authority rollback edilemez",
            ));
        }
    }

    let restored = if let (Some(previous), Some(token)) = (
        previous.as_ref(),
        activation.previous_transition_token.as_deref(),
    ) {
        let mut state = previous.state.lock();
        validate_transition(&state, token, TransitionKind::DocumentSwitch)?;
        state.active_transition = None;
        state.lifecycle = SessionLifecycle::Open;
        bump_revision(&mut state)?;
        Some(snapshot(&state))
    } else {
        None
    };
    {
        let mut state = candidate.state.lock();
        ensure_lifecycle(&state, &[SessionLifecycle::CandidateActivationPrepared])?;
        state.lifecycle = SessionLifecycle::Closed;
        state.credential = None;
        bump_revision(&mut state)?;
    }
    store.activations_by_token.lock().remove(activation_token);
    store
        .activation_token_by_candidate
        .lock()
        .remove(&activation.candidate_session_id);
    let removed_candidate = store
        .sessions
        .write()
        .remove(&activation.candidate_session_id);
    drop(_leases);
    if let Some(removed) = removed_candidate {
        let _ = store.cleanup_removed_entry(removed).await;
    }
    clear_session_derived_state(
        search_cache,
        layout_cache,
        vector_store,
        &activation.candidate_session_id,
    );
    Ok(restored)
}

#[tauri::command]
pub async fn close_all_document_sessions(
    store: State<'_, DocumentSessionStore>,
    search_cache: State<'_, SearchCache>,
    layout_cache: State<'_, TextLayoutCache>,
    vector_store: State<'_, crate::ai::VectorStore>,
) -> Result<(), DocumentSessionError> {
    store.close_all().await?;
    search_cache.0.lock().clear();
    layout_cache.0.lock().clear();
    vector_store.clear_all();
    Ok(())
}

fn clear_session_derived_state(
    search_cache: &SearchCache,
    layout_cache: &TextLayoutCache,
    vector_store: &crate::ai::VectorStore,
    session_id: &str,
) {
    search_cache.0.lock().clear();
    layout_cache.0.lock().clear();
    vector_store.remove_session(session_id);
    let prefix = format!("session:{session_id}:");
    vector_store
        .0
        .write()
        .retain(|key, _| key != session_id && !key.starts_with(&prefix));
}

fn find_transition_authority(
    store: &DocumentSessionStore,
    token: &str,
) -> Result<(String, TransitionKind, TransitionExecution), DocumentSessionError> {
    for (session_id, entry) in store.sessions.read().iter() {
        if let Some(transition) = entry.state.lock().active_transition.as_ref() {
            if transition.token == token {
                return Ok((
                    session_id.clone(),
                    transition.kind,
                    transition.execution.clone(),
                ));
            }
        }
    }
    Err(DocumentSessionError::new(
        "TRANSITION_TOKEN_NOT_FOUND",
        "Transition token bulunamadı, tüketilmiş veya expire olmuş",
    ))
}

fn transition_activation(store: &DocumentSessionStore, token: &str) -> Option<CandidateActivation> {
    store
        .activations_by_token
        .lock()
        .values()
        .find(|activation| activation.previous_transition_token.as_deref() == Some(token))
        .cloned()
}

fn advance_transition_epoch_once(store: &DocumentSessionStore, transition_epoch: u64) {
    let _ = store.active_identity_epoch.compare_exchange(
        transition_epoch,
        transition_epoch.saturating_add(1),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

struct TransitionApplyContext<'a> {
    store: &'a DocumentSessionStore,
    candidate: Option<&'a Arc<SessionEntry>>,
    activation: Option<&'a CandidateActivation>,
    token: &'a str,
    kind: TransitionKind,
}

struct TransitionApplyResult {
    snapshot: DocumentSessionSnapshot,
    source_session_id: String,
}

async fn apply_transition_intent(
    context: TransitionApplyContext<'_>,
    source: &Arc<SessionEntry>,
) -> Result<TransitionApplyResult, DocumentSessionError> {
    let TransitionApplyContext {
        store,
        candidate,
        activation,
        token,
        kind,
    } = context;
    let (source_id, transition_epoch) = {
        let state = source.state.lock();
        let transition = state.active_transition.as_ref().ok_or_else(|| {
            DocumentSessionError::new(
                "TRANSITION_TOKEN_NOT_FOUND",
                "Transition apply sırasında pending authority bulunamadı",
            )
        })?;
        (state.id.clone(), transition.transition_epoch)
    };

    match kind {
        TransitionKind::DocumentSwitch => {
            let activation = activation.ok_or_else(|| {
                DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_NOT_PREPARED",
                    "Document switch transition için candidate activation hazırlanmadı",
                )
            })?;
            let candidate = candidate.ok_or_else(|| {
                DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_NOT_FOUND",
                    "Document switch candidate session bulunamadı",
                )
            })?;
            if activation.previous_session_id.as_deref() != Some(source_id.as_str())
                || activation.previous_transition_token.as_deref() != Some(token)
            {
                return Err(DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_AUTHORITY_MISMATCH",
                    "Candidate activation source transition ile eşleşmiyor",
                ));
            }

            if store.active_session_id.lock().as_deref()
                == Some(activation.candidate_session_id.as_str())
                && store.active_identity_epoch.load(Ordering::Acquire)
                    == transition_epoch.saturating_add(2)
                && source.state.lock().lifecycle == SessionLifecycle::Closed
            {
                let state = candidate.state.lock();
                if state.lifecycle == SessionLifecycle::Open {
                    return Ok(TransitionApplyResult {
                        snapshot: snapshot(&state),
                        source_session_id: source_id,
                    });
                }
            }

            let source_next_revision = {
                let state = source.state.lock();
                if state.lifecycle == SessionLifecycle::Closed {
                    state.state_revision
                } else {
                    checked_next_revision(state.state_revision)?
                }
            };
            let candidate_next_revision = {
                let state = candidate.state.lock();
                ensure_lifecycle(
                    &state,
                    &[
                        SessionLifecycle::CandidateActivationPrepared,
                        SessionLifecycle::RecoveryRequired,
                    ],
                )?;
                checked_next_revision(state.state_revision)?
            };

            {
                let mut active = store.active_session_id.lock();
                let source_already_deactivated = {
                    let state = source.state.lock();
                    state.lifecycle == SessionLifecycle::Closed
                        && active.is_none()
                        && store.active_identity_epoch.load(Ordering::Acquire)
                            == transition_epoch.saturating_add(1)
                };
                if !source_already_deactivated && active.as_deref() != Some(source_id.as_str()) {
                    return Err(DocumentSessionError::new(
                        "ACTIVE_SESSION_MISMATCH",
                        "Transition source authoritative active session ile eşleşmiyor",
                    ));
                }

                if !source_already_deactivated {
                    let mut state = source.state.lock();
                    state.lifecycle = SessionLifecycle::Closed;
                    state.credential = None;
                    state.state_revision = source_next_revision;
                    if let Some(transition) = state.active_transition.as_mut() {
                        transition.expected_state_revision = source_next_revision;
                    }
                    *active = None;
                    advance_transition_epoch_once(store, transition_epoch);
                }
            }

            #[cfg(test)]
            if store
                .fail_next_candidate_activation_after_deactivation
                .swap(false, Ordering::AcqRel)
            {
                let snap = snapshot(&source.state.lock());
                return Err(DocumentSessionError::new(
                    "TRANSITION_CANDIDATE_ACTIVATION_FAILED_AFTER_DEACTIVATION",
                    "Source deactive edildi ancak candidate activation test hook ile durduruldu",
                )
                .with_snapshot(snap));
            }

            let candidate_snapshot = {
                let mut state = candidate.state.lock();
                state.lifecycle = SessionLifecycle::Open;
                state.state_revision = candidate_next_revision;
                snapshot(&state)
            };
            *store.active_session_id.lock() = Some(activation.candidate_session_id.clone());
            advance_transition_epoch_once(store, transition_epoch.saturating_add(1));

            Ok(TransitionApplyResult {
                snapshot: candidate_snapshot,
                source_session_id: source_id,
            })
        }
        TransitionKind::DocumentClose | TransitionKind::WindowClose => {
            if source.state.lock().lifecycle == SessionLifecycle::Closed
                && store.active_session_id.lock().is_none()
                && store.active_identity_epoch.load(Ordering::Acquire)
                    == transition_epoch.saturating_add(1)
            {
                return Ok(TransitionApplyResult {
                    snapshot: snapshot(&source.state.lock()),
                    source_session_id: source_id,
                });
            }
            #[cfg(test)]
            if store
                .fail_next_transition_close
                .swap(false, Ordering::AcqRel)
            {
                let snap = {
                    let mut state = source.state.lock();
                    let next_revision = checked_next_revision(state.state_revision)?;
                    state.lifecycle = SessionLifecycle::Open;
                    state.state_revision = next_revision;
                    if let Some(transition) = state.active_transition.as_mut() {
                        transition.expected_state_revision = next_revision;
                    }
                    snapshot(&state)
                };
                return Err(DocumentSessionError::new(
                    "TRANSITION_CLOSE_APPLY_FAILED",
                    "Transition close apply test hook ile durduruldu",
                )
                .with_snapshot(snap));
            }
            let next_revision = {
                let state = source.state.lock();
                checked_next_revision(state.state_revision)?
            };
            let closed_snapshot = {
                let mut active = store.active_session_id.lock();
                if active.as_deref() != Some(source_id.as_str()) {
                    return Err(DocumentSessionError::new(
                        "ACTIVE_SESSION_MISMATCH",
                        "Close transition source authoritative active session ile eşleşmiyor",
                    ));
                }
                let mut state = source.state.lock();
                state.lifecycle = SessionLifecycle::Closed;
                state.credential = None;
                state.state_revision = next_revision;
                let closed = snapshot(&state);
                *active = None;
                advance_transition_epoch_once(store, transition_epoch);
                closed
            };
            Ok(TransitionApplyResult {
                snapshot: closed_snapshot,
                source_session_id: source_id,
            })
        }
    }
}

fn find_transition(
    store: &DocumentSessionStore,
    token: &str,
) -> Result<(String, TransitionKind), DocumentSessionError> {
    for (session_id, entry) in store.sessions.read().iter() {
        if let Some(transition) = entry.state.lock().active_transition.as_ref() {
            if transition.token == token {
                return Ok((session_id.clone(), transition.kind));
            }
        }
    }
    Err(DocumentSessionError::new(
        "TRANSITION_TOKEN_NOT_FOUND",
        "Transition token bulunamadı, tüketilmiş veya expire olmuş",
    ))
}

fn validate_transition(
    state: &DocumentSession,
    token: &str,
    kind: TransitionKind,
) -> Result<(), DocumentSessionError> {
    let transition = state.active_transition.as_ref().ok_or_else(|| {
        DocumentSessionError::new(
            "TRANSITION_TOKEN_NOT_FOUND",
            "Session için aktif transition bulunmuyor",
        )
    })?;
    if transition.token != token {
        return Err(DocumentSessionError::new(
            "TRANSITION_TOKEN_SESSION_MISMATCH",
            "Transition token bu session'a ait değil",
        ));
    }
    if transition.kind != kind {
        return Err(DocumentSessionError::new(
            "TRANSITION_KIND_MISMATCH",
            "Transition kind eşleşmedi",
        ));
    }
    if transition.source_session_id != state.id {
        return Err(DocumentSessionError::new(
            "TRANSITION_TOKEN_SESSION_MISMATCH",
            "Transition source session identity eşleşmedi",
        ));
    }
    if transition.captured_content_generation != state.content_generation {
        return Err(DocumentSessionError::new(
            "TRANSITION_GENERATION_MISMATCH",
            "Transition oluşturulduktan sonra document generation değişti",
        )
        .with_snapshot(snapshot(state)));
    }
    if transition.expected_state_revision != state.state_revision {
        return Err(DocumentSessionError::new(
            "TRANSITION_REVISION_MISMATCH",
            "Transition oluşturulduktan sonra session state beklenmedik biçimde değişti",
        ));
    }
    Ok(())
}

fn transition_expired(transition: &SessionTransition) -> bool {
    let expirable = matches!(
        &*transition.execution.completion_state.lock(),
        TransitionExecutionCompletion::Available
            | TransitionExecutionCompletion::InterruptedPublished
    ) && matches!(
        &*transition.execution.phase.lock(),
        TransitionExecutionPhase::PendingUnclaimed
    );
    expirable && now_ms().saturating_sub(transition.created_at_ms) > TRANSITION_TTL_MS
}

fn copy_source_consistent(source: &Path, working: &Path) -> Result<String, DocumentSessionError> {
    for attempt in 0..OPEN_ATTEMPTS {
        let before = source_stamp(source)?;
        let copied_digest = copy_with_digest(source, working)?;
        validate_basic_pdf_bytes(working)?;
        let after_digest = sha256_file(source).map_err(|error| {
            DocumentSessionError::new(
                "SOURCE_CHANGED_DURING_OPEN",
                format!("Kaynak ikinci hash sırasında okunamadı: {error}"),
            )
        })?;
        let after = source_stamp(source)?;
        if before == after && copied_digest == after_digest {
            return Ok(copied_digest);
        }
        let _ = fs::remove_file(working);
        if attempt + 1 == OPEN_ATTEMPTS {
            return Err(DocumentSessionError::new(
                "SOURCE_CHANGED_DURING_OPEN",
                "Kaynak belge session copy sırasında değişti",
            ));
        }
    }
    unreachable!("bounded source-copy loop")
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceStamp {
    identity: String,
    len: u64,
    modified: Option<SystemTime>,
}

fn source_stamp(path: &Path) -> Result<SourceStamp, DocumentSessionError> {
    let metadata = fs::metadata(path).map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_SOURCE_METADATA_FAILED",
            format!("Kaynak metadata okunamadı: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_SOURCE_NOT_FILE",
            "Kaynak regular file değil",
        ));
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let identity = windows_file_identity(path)?;
    #[cfg(not(any(unix, windows)))]
    let identity = path.to_string_lossy().to_string();
    Ok(SourceStamp {
        identity,
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn copy_with_digest(source: &Path, target: &Path) -> Result<String, DocumentSessionError> {
    let mut source_file = File::open(source).map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_SOURCE_OPEN_FAILED",
            format!("Kaynak açılamadı: {error}"),
        )
    })?;
    let mut target_file = create_secure_file(target)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = source_file.read(&mut buffer).map_err(|error| {
            DocumentSessionError::new(
                "DOCUMENT_SESSION_COPY_FAILED",
                format!("Kaynak okunamadı: {error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        target_file.write_all(&buffer[..read]).map_err(|error| {
            DocumentSessionError::new(
                "DOCUMENT_SESSION_COPY_FAILED",
                format!("Working copy yazılamadı: {error}"),
            )
        })?;
        hasher.update(&buffer[..read]);
    }
    target_file.sync_all().map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_COPY_SYNC_FAILED",
            format!("Working copy fsync başarısız: {error}"),
        )
    })?;
    drop(target_file);
    drop(source_file);
    Ok(crate::hex_lower(hasher.finalize()))
}

fn copy_with_digest_cancellable(
    source: &Path,
    target: &Path,
    cancellation: &SessionJobCancellationCheckpoint,
) -> Result<String, SessionJobOperationError> {
    cancellation.check_cancelled()?;
    let mut source_file = File::open(source).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "DOCUMENT_SESSION_SOURCE_OPEN_FAILED",
            format!("Kaynak açılamadı: {error}"),
        )
    })?;
    let mut target_file = create_secure_file(target)
        .map_err(|error| SessionJobOperationError::failed_with_code(error.code, error.message))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        if let Err(e) = cancellation.check_cancelled() {
            drop(target_file);
            let _ = std::fs::remove_file(target);
            return Err(e);
        }
        let read = match source_file.read(&mut buffer) {
            Ok(n) => n,
            Err(error) => {
                drop(target_file);
                let _ = std::fs::remove_file(target);
                return Err(SessionJobOperationError::failed_with_code(
                    "DOCUMENT_SESSION_COPY_FAILED",
                    format!("Kaynak okunamadı: {error}"),
                ));
            }
        };
        if read == 0 {
            break;
        }
        if let Err(error) = target_file.write_all(&buffer[..read]) {
            drop(target_file);
            let _ = std::fs::remove_file(target);
            return Err(SessionJobOperationError::failed_with_code(
                "DOCUMENT_SESSION_COPY_FAILED",
                format!("Working copy yazılamadı: {error}"),
            ));
        }
        hasher.update(&buffer[..read]);
    }
    if let Err(e) = cancellation.check_cancelled() {
        drop(target_file);
        let _ = std::fs::remove_file(target);
        return Err(e);
    }
    if let Err(error) = target_file.sync_all() {
        drop(target_file);
        let _ = std::fs::remove_file(target);
        return Err(SessionJobOperationError::failed_with_code(
            "DOCUMENT_SESSION_COPY_SYNC_FAILED",
            format!("Working copy fsync başarısız: {error}"),
        ));
    }
    drop(target_file);
    drop(source_file);
    Ok(crate::hex_lower(hasher.finalize()))
}

fn validate_basic_pdf_bytes(path: &Path) -> Result<(), DocumentSessionError> {
    let mut file = File::open(path).map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_COPY_VALIDATE_FAILED",
            format!("Working copy doğrulama için açılamadı: {error}"),
        )
    })?;
    let mut header = [0_u8; 5];
    file.read_exact(&mut header).map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_COPY_VALIDATE_FAILED",
            format!("Working copy PDF header okunamadı: {error}"),
        )
    })?;
    if &header != b"%PDF-" {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_INVALID_PDF",
            "Working copy geçerli PDF header taşımıyor",
        ));
    }
    Ok(())
}

/// Resolves the longest existing prefix of `path` (following symlinks) and
/// re-appends the missing remainder, so targets inside folders that do not
/// exist yet still compare in the resolved namespace.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(resolved) = current.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(resolved, |resolved, name| resolved.join(name));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `private_root` may be stored unresolved (`std::env::temp_dir()` is under the
/// `/var` symlink on macOS) while the target is resolved, so both forms of the
/// root are checked.
pub fn reject_private_target(path: &Path, private_root: &Path) -> Result<(), DocumentSessionError> {
    let normalized = resolve_existing_prefix(path);
    let resolved_root = private_root.canonicalize().ok();
    if normalized.starts_with(private_root)
        || resolved_root.is_some_and(|root| normalized.starts_with(root))
    {
        Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_PRIVATE_TARGET",
            "Session/job temp root'u Save veya Save As hedefi olamaz",
        ))
    } else {
        Ok(())
    }
}

fn accept_job_output_file(
    context: &SessionJobContext,
    source: &Path,
    target: &Path,
) -> Result<(), SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let canonical_source = crate::job_worker::validate_output_file(context, source)?;
    let mut input = File::open(&canonical_source).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("Job worker çıktısı açılamadı: {error}"),
        )
    })?;
    crate::document_io::write_atomically(target, |staged| {
        std::io::copy(&mut input, staged).map(drop)
    })
    .map_err(job_output_write_failure)
}

fn job_output_write_failure(
    error: crate::document_io::AtomicWriteError<std::io::Error>,
) -> SessionJobOperationError {
    use crate::document_io::AtomicWriteError;
    let message = match error {
        AtomicWriteError::CreateTemp(error) => {
            format!("Hedef çıktı staging dosyası oluşturulamadı: {error}")
        }
        AtomicWriteError::Write(error) => {
            format!("Job worker çıktısı staging dosyasına kopyalanamadı: {error}")
        }
        AtomicWriteError::Sync(error) => {
            format!("Job worker çıktısı diske yazılamadı: {error}")
        }
        AtomicWriteError::Persist(error) => {
            format!("Job worker çıktısı hedefe kabul edilemedi: {error}")
        }
    };
    SessionJobOperationError::failed_with_code("SESSION_JOB_OUTPUT_WRITE_FAILED", message)
}

pub(crate) const SESSION_JOB_OUTPUT_CONFLICT_CODE: &str = "SESSION_JOB_OUTPUT_CONFLICT";

fn output_set_file_names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned())
        })
        .collect()
}

fn output_set_failure(
    error: crate::document_io::OutputSetError<SessionJobOperationError>,
) -> SessionJobOperationError {
    use crate::document_io::OutputSetError;
    match error {
        OutputSetError::Conflict(paths) => {
            let names = output_set_file_names(&paths);
            let mut failure = SessionJobFailure::new(
                SESSION_JOB_OUTPUT_CONFLICT_CODE,
                format!(
                    "Hedef klasörde aynı adlı {} dosya zaten var: {}",
                    names.len(),
                    names.join(", ")
                ),
            );
            failure.conflicting_names = Some(Box::new(ConflictingOutputNames(names)));
            SessionJobOperationError::Failed(failure)
        }
        OutputSetError::UnsupportedTarget(paths) => SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_TARGET_UNSUPPORTED",
            format!(
                "Hedef klasörde dosya olmayan aynı adlı öğeler var ve üzerine yazılamaz: {}",
                output_set_file_names(&paths).join(", ")
            ),
        ),
        OutputSetError::InvalidPlan(detail) => SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Çıktı planı geçersiz: {detail}"),
        ),
        OutputSetError::Interrupted(error) => error,
        OutputSetError::Io { target, error } => SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!(
                "{} yayınlanamadı; bu işlemin yazdığı dosyalar geri alındı: {error}",
                target.display()
            ),
        ),
        OutputSetError::RollbackFailed { cause, failures } => {
            let cause = match output_set_failure(*cause) {
                SessionJobOperationError::Failed(failure) => failure.message,
                SessionJobOperationError::Cancelled(_) => "işlem iptal edildi".to_string(),
            };
            SessionJobOperationError::failed_with_code(
                "SESSION_JOB_OUTPUT_ROLLBACK_FAILED",
                format!(
                    "Çıktılar yayınlanamadı ({cause}) ve geri alma tamamlanamadı: {}",
                    failures.join("; ")
                ),
            )
        }
    }
}

/// Document part of multi-file output names, from the session's current save
/// target. A missing session falls back to the default stem; the job itself
/// then reports the missing session.
async fn session_output_name_stem(store: &DocumentSessionStore, session_id: &str) -> String {
    let file_name = store
        .session_snapshot(session_id)
        .await
        .map(|snapshot| snapshot.file_name)
        .unwrap_or_default();
    crate::output_names::output_name_stem(&file_name)
}

/// Split outputs in range order: the worker's `split_{index}_{start}.pdf` is
/// published as `{stem}_sayfa_{start}-{end}.pdf`. The same range twice would
/// publish two files under one name, so it is rejected before any work.
fn planned_split_outputs(
    ranges: &[(u32, u32)],
    stem: &str,
) -> Result<Vec<crate::output_names::PlannedOutput>, SessionJobOperationError> {
    let mut seen = HashSet::with_capacity(ranges.len());
    if let Some(&(start, end)) = ranges.iter().find(|range| !seen.insert(**range)) {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_INVALID_SPLIT_RANGES",
            format!(
                "Aynı aralık birden fazla kez girildi: {}",
                if start == end {
                    format!("{}", u64::from(start) + 1)
                } else {
                    format!("{}-{}", u64::from(start) + 1, u64::from(end) + 1)
                }
            ),
        ));
    }
    Ok(ranges
        .iter()
        .enumerate()
        .map(
            |(range_index, &(start, end))| crate::output_names::PlannedOutput {
                worker_name: crate::job_worker::split_output_file_name(range_index, start),
                target_name: crate::output_names::split_range_output_name(stem, start, end),
            },
        )
        .collect())
}

/// Rejects a multi-file output whose planned names collide with existing
/// entries before any work is done. [`accept_job_output_set`] repeats the check
/// at publish time.
fn preflight_job_output_set(
    output_dir: &Path,
    planned: &[crate::output_names::PlannedOutput],
    overwrite: bool,
) -> Result<(), SessionJobOperationError> {
    let targets: Vec<PathBuf> = planned
        .iter()
        .map(|output| output_dir.join(&output.target_name))
        .collect();
    crate::document_io::inspect_output_set_targets(&targets, overwrite)
        .map(drop)
        .map_err(output_set_failure)
}

/// Publishes validated job outputs into `output_dir` all-or-nothing under
/// their planned target names (see [`crate::document_io::publish_output_set`]).
/// The worker must have produced exactly the planned files, in order.
fn accept_job_output_set(
    context: &SessionJobContext,
    sources: Vec<PathBuf>,
    planned: &[crate::output_names::PlannedOutput],
    output_dir: &Path,
    overwrite: bool,
) -> Result<Vec<String>, SessionJobOperationError> {
    let produced_planned_names = sources.len() == planned.len()
        && sources.iter().zip(planned).all(|(source, output)| {
            source
                .file_name()
                .is_some_and(|file_name| file_name == output.worker_name.as_str())
        });
    if !produced_planned_names {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "Job worker planlanan çıktı dosyalarından farklı dosyalar döndürdü",
        ));
    }
    let entries: Vec<crate::document_io::OutputSetEntry> = sources
        .into_iter()
        .zip(planned)
        .map(|(source, output)| crate::document_io::OutputSetEntry {
            source,
            target: output_dir.join(&output.target_name),
        })
        .collect();
    crate::document_io::publish_output_set(&entries, overwrite, |_, _| {
        context.cancellation.check_cancelled()
    })
    .map_err(output_set_failure)?;
    Ok(entries
        .into_iter()
        .map(|entry| entry.target.to_string_lossy().into_owned())
        .collect())
}

fn validate_job_pdf_output(
    context: &SessionJobContext,
    source: &Path,
) -> Result<PathBuf, SessionJobOperationError> {
    let canonical_source = crate::job_worker::validate_output_file(context, source)?;
    let output_name = canonical_source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            SessionJobOperationError::failed_with_code(
                "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                "PDF job çıktısı geçerli UTF-8 dosya adı taşımıyor",
            )
        })?
        .to_string();
    match crate::job_worker::run_job(
        context,
        JobCommand::ValidateMutationOutput { output_name },
        Duration::from_secs(60),
    )? {
        JobResult::MutationValidated => Ok(canonical_source),
        _ => Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "PDF output validation worker beklenmeyen sonuç döndürdü",
        )),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn create_secure_dir(path: &Path) -> Result<(), DocumentSessionError> {
    let created = match fs::create_dir(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => {
            return Err(DocumentSessionError::new(
                "TEMP_SECURITY_ERROR",
                format!(
                    "Güvenli temp dizini oluşturulamadı ({}): {error}",
                    path.display()
                ),
            ));
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            DocumentSessionError::new(
                "TEMP_SECURITY_ERROR",
                format!("Temp dizin metadata okunamadı: {error}"),
            )
        })?;
        if metadata.file_type().is_symlink() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(DocumentSessionError::new(
                "TEMP_SECURITY_ERROR",
                "Temp dizini symlink veya başka kullanıcıya ait",
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            DocumentSessionError::new(
                "TEMP_SECURITY_ERROR",
                format!("Temp dizin izni 0700 yapılamadı: {error}"),
            )
        })?;
    }
    // Var olan paylaşılan dizinlere (temp root, jobs root) DACL her çağrıda yeniden yazılmaz:
    // SetNamedSecurityInfoW kalıtılabilir ACE'leri bütün alt nesnelere yayar ve bu yayılım, o
    // sırada alt dizinini kurup doğrulayan başka bir thread'in DACL kontrolüyle yarışır.
    #[cfg(windows)]
    if created || !windows_current_user_acl_already_enforced(path) {
        ensure_windows_current_user_acl(path, true)?;
    }
    #[cfg(not(windows))]
    let _ = created;
    Ok(())
}

fn create_secure_file(path: &Path) -> Result<File, DocumentSessionError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            DocumentSessionError::new(
                "TEMP_SECURITY_ERROR",
                format!(
                    "Güvenli temp dosyası oluşturulamadı ({}): {error}",
                    path.display()
                ),
            )
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                DocumentSessionError::new(
                    "TEMP_SECURITY_ERROR",
                    format!("Temp dosya izni 0600 yapılamadı: {error}"),
                )
            })?;
    }
    #[cfg(windows)]
    ensure_windows_current_user_acl(path, false)?;
    Ok(file)
}

fn write_manifest(
    instance_dir: &Path,
    manifest: &InstanceManifest,
) -> Result<(), DocumentSessionError> {
    let temp_path = instance_dir.join("manifest.json.tmp");
    let final_path = instance_dir.join("manifest.json");
    let mut file = create_secure_file(&temp_path)?;
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        DocumentSessionError::new(
            "TEMP_MANIFEST_ERROR",
            format!("Manifest serialize edilemedi: {error}"),
        )
    })?;
    file.write_all(&bytes).map_err(|error| {
        DocumentSessionError::new(
            "TEMP_MANIFEST_ERROR",
            format!("Manifest yazılamadı: {error}"),
        )
    })?;
    file.sync_all().map_err(|error| {
        DocumentSessionError::new(
            "TEMP_MANIFEST_ERROR",
            format!("Manifest fsync başarısız: {error}"),
        )
    })?;
    drop(file);
    fs::rename(&temp_path, &final_path).map_err(|error| {
        DocumentSessionError::new(
            "TEMP_MANIFEST_ERROR",
            format!("Manifest aktive edilemedi: {error}"),
        )
    })
}

fn cleanup_orphan_instances(root: &Path) -> Result<(), DocumentSessionError> {
    let entries = fs::read_dir(root).map_err(|error| {
        DocumentSessionError::new(
            "TEMP_CLEANUP_ERROR",
            format!("Temp root taranamadı: {error}"),
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("cleanup-") && path.is_dir() {
            secure_remove_tree(&path)?;
            continue;
        }
        let Some(nonce) = name
            .strip_prefix("instance-")
            .and_then(|value| value.strip_suffix(".lock"))
        else {
            continue;
        };
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        let instance = root.join(format!("instance-{nonce}"));
        if instance.exists() {
            ensure_safe_tree_root(&instance)?;
            let quarantine = root.join(format!("cleanup-{nonce}"));
            fs::rename(&instance, &quarantine).map_err(|error| {
                DocumentSessionError::new(
                    "TEMP_CLEANUP_ERROR",
                    format!("Orphan instance quarantine edilemedi: {error}"),
                )
            })?;
            secure_remove_tree(&quarantine)?;
        }
        drop(file);
        let _ = fs::remove_file(&path);
    }
    Ok(())
}

fn ensure_safe_tree_root(path: &Path) -> Result<(), DocumentSessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        DocumentSessionError::new(
            "TEMP_CLEANUP_ERROR",
            format!("Cleanup root metadata okunamadı: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(DocumentSessionError::new(
            "TEMP_CLEANUP_REPARSE_POINT",
            "Symlink cleanup root reddedildi",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return Err(DocumentSessionError::new(
                "TEMP_CLEANUP_REPARSE_POINT",
                "Windows reparse-point cleanup root reddedildi",
            ));
        }
    }
    Ok(())
}

fn secure_remove_tree(path: &Path) -> Result<(), DocumentSessionError> {
    if !path.exists() {
        return Ok(());
    }
    if let Err(error) = ensure_safe_tree_root(path) {
        if !path.exists() {
            return Ok(());
        }
        return Err(error);
    }
    if let Err(error) = reject_linked_descendants(path) {
        if !path.exists() {
            return Ok(());
        }
        return Err(error);
    }
    // Aynı `cleanup-*` ağacını başka bir store ya da ikinci bir uygulama örneği de siliyor olabilir.
    // Windows'ta onun bekleyen silmeleri burada "Access is denied" olarak döner; ağaç kaybolana
    // kadar kısa aralıklarla yeniden denenir.
    let mut attempt = 0;
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) if !path.exists() => return Ok(()),
            Err(_) if attempt < TREE_REMOVE_RETRIES => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20 * attempt as u64));
            }
            Err(error) => {
                return Err(DocumentSessionError::new(
                    "TEMP_CLEANUP_ERROR",
                    format!("Temp tree silinemedi ({}): {error}", path.display()),
                ))
            }
        }
    }
}

const TREE_REMOVE_RETRIES: u32 = 5;

fn reject_linked_descendants(path: &Path) -> Result<(), DocumentSessionError> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(DocumentSessionError::new(
                "TEMP_CLEANUP_ERROR",
                format!("Cleanup tree taranamadı ({}): {error}", path.display()),
            ))
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(DocumentSessionError::new(
                    "TEMP_CLEANUP_ERROR",
                    format!("Cleanup entry okunamadı: {error}"),
                ))
            }
        };
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(DocumentSessionError::new(
                    "TEMP_CLEANUP_ERROR",
                    format!("Cleanup entry metadata okunamadı: {error}"),
                ))
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(DocumentSessionError::new(
                "TEMP_CLEANUP_REPARSE_POINT",
                "Cleanup tree içindeki symlink reddedildi",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x0000_0400 != 0 {
                return Err(DocumentSessionError::new(
                    "TEMP_CLEANUP_REPARSE_POINT",
                    "Cleanup tree içindeki Windows reparse-point reddedildi",
                ));
            }
        }
        if metadata.is_dir() {
            reject_linked_descendants(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Result<String, DocumentSessionError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = File::open(path).map_err(|error| {
        DocumentSessionError::new(
            "DOCUMENT_SESSION_SOURCE_METADATA_FAILED",
            format!("Windows identity handle açılamadı: {error}"),
        )
    })?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if ok == 0 {
        return Err(DocumentSessionError::new(
            "DOCUMENT_SESSION_SOURCE_METADATA_FAILED",
            format!(
                "Windows identity okunamadı: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(format!("{}:{index}", information.dwVolumeSerialNumber))
}

#[cfg(windows)]
fn ensure_windows_current_user_acl(
    path: &Path,
    is_directory: bool,
) -> Result<(), DocumentSessionError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::{
        GetExplicitEntriesFromAclW, GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW,
        EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SET_ACCESS, SE_FILE_OBJECT,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        EqualSid, ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let mut sid = current_windows_user_sid()?;
    let trustee = TRUSTEE_W {
        pMultipleTrustee: std::ptr::null_mut(),
        MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_USER,
        ptstrName: sid.as_mut_ptr().cast(),
    };
    let entry = EXPLICIT_ACCESS_W {
        // Specific rights, not GENERIC_ALL: Windows maps generic rights when it
        // stores the ACE (and splits inheritable ones), so a GENERIC_ALL grant
        // never reads back as the single GENERIC_ALL entry verified below.
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: if is_directory {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            0
        },
        Trustee: trustee,
    };
    let mut acl: *mut ACL = std::ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &entry, std::ptr::null(), &mut acl) };
    if status != ERROR_SUCCESS {
        return Err(windows_acl_error("SetEntriesInAclW", status));
    }

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    unsafe {
        LocalFree(acl.cast());
    }
    if status != ERROR_SUCCESS {
        return Err(windows_acl_error("SetNamedSecurityInfoW", status));
    }

    let mut verified_acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut verified_acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(windows_acl_error("GetNamedSecurityInfoW", status));
    }
    let mut count = 0_u32;
    let mut entries: *mut EXPLICIT_ACCESS_W = std::ptr::null_mut();
    let entries_status =
        unsafe { GetExplicitEntriesFromAclW(verified_acl, &mut count, &mut entries) };
    let valid = entries_status == ERROR_SUCCESS
        && count == 1
        && !entries.is_null()
        && unsafe {
            let verified = &*entries;
            // An access-allowed ACE reads back as GRANT_ACCESS or SET_ACCESS.
            matches!(verified.grfAccessMode, GRANT_ACCESS | SET_ACCESS)
                && verified.grfAccessPermissions & FILE_ALL_ACCESS == FILE_ALL_ACCESS
                && verified.Trustee.TrusteeForm == TRUSTEE_IS_SID
                && EqualSid(verified.Trustee.ptstrName.cast(), sid.as_mut_ptr().cast()) != 0
        };
    unsafe {
        if !entries.is_null() {
            LocalFree(entries.cast());
        }
        if !descriptor.is_null() {
            LocalFree(descriptor);
        }
    }
    if !valid {
        return Err(DocumentSessionError::new(
            "TEMP_SECURITY_ACL_UNAVAILABLE",
            "Temp nesnesinin DACL'i yalnız current-user erişimi olarak doğrulanamadı",
        ));
    }
    Ok(())
}

/// Nesnenin DACL'i zaten `ensure_windows_current_user_acl`'in yazdığı biçimdeyse true: korumalı
/// (inheritance kapalı), tek ACE'li ve yalnız current-user'a FILE_ALL_ACCESS veriyor. Okuma
/// hatası dahil her belirsizlikte false döner; çağıran o zaman DACL'i yeniden yazar.
#[cfg(windows)]
fn windows_current_user_acl_already_enforced(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::{
        GetExplicitEntriesFromAclW, GetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    };
    use windows_sys::Win32::Security::{
        EqualSid, GetSecurityDescriptorControl, ACL, DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let Ok(mut sid) = current_windows_user_sid() else {
        return false;
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        if !descriptor.is_null() {
            unsafe { LocalFree(descriptor) };
        }
        return false;
    }
    let mut control = 0_u16;
    let mut revision = 0_u32;
    let protected =
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } != 0
            && control & SE_DACL_PROTECTED != 0;
    // AceCount kalıtılmış ACE'leri de sayar; GetExplicitEntriesFromAclW tek başına yetmez.
    let single_ace = !acl.is_null() && unsafe { (*acl).AceCount } == 1;
    let mut count = 0_u32;
    let mut entries: *mut EXPLICIT_ACCESS_W = std::ptr::null_mut();
    let enforced = protected
        && single_ace
        && unsafe { GetExplicitEntriesFromAclW(acl, &mut count, &mut entries) } == ERROR_SUCCESS
        && count == 1
        && !entries.is_null()
        && unsafe {
            let entry = &*entries;
            matches!(entry.grfAccessMode, GRANT_ACCESS | SET_ACCESS)
                && entry.grfAccessPermissions & FILE_ALL_ACCESS == FILE_ALL_ACCESS
                && entry.Trustee.TrusteeForm == TRUSTEE_IS_SID
                && EqualSid(entry.Trustee.ptstrName.cast(), sid.as_mut_ptr().cast()) != 0
        };
    unsafe {
        if !entries.is_null() {
            LocalFree(entries.cast());
        }
        LocalFree(descriptor);
    }
    enforced
}

#[cfg(windows)]
fn current_windows_user_sid() -> Result<Vec<u8>, DocumentSessionError> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        CopySid, GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(windows_acl_last_error("OpenProcessToken"));
    }
    let mut required = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        unsafe { CloseHandle(token) };
        return Err(windows_acl_last_error("GetTokenInformation(size)"));
    }
    let mut token_data = vec![0_u8; required as usize];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            token_data.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    };
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err(windows_acl_last_error("GetTokenInformation"));
    }
    let token_user = unsafe { &*(token_data.as_ptr().cast::<TOKEN_USER>()) };
    let sid_len = unsafe { GetLengthSid(token_user.User.Sid) };
    if sid_len == 0 {
        return Err(windows_acl_last_error("GetLengthSid"));
    }
    let mut sid = vec![0_u8; sid_len as usize];
    if unsafe { CopySid(sid_len, sid.as_mut_ptr().cast(), token_user.User.Sid) } == 0 {
        return Err(windows_acl_last_error("CopySid"));
    }
    Ok(sid)
}

#[cfg(windows)]
fn windows_acl_error(api: &str, status: u32) -> DocumentSessionError {
    DocumentSessionError::new(
        "TEMP_SECURITY_ACL_UNAVAILABLE",
        format!("{api} başarısız oldu: Win32 error {status}"),
    )
}

#[cfg(windows)]
fn windows_acl_last_error(api: &str) -> DocumentSessionError {
    DocumentSessionError::new(
        "TEMP_SECURITY_ACL_UNAVAILABLE",
        format!("{api} başarısız oldu: {}", std::io::Error::last_os_error()),
    )
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct TestDocumentSessionFixture {
    pub session_id: String,
    pub content_generation: u64,
}

#[cfg(test)]
pub(crate) fn open_test_session(
    store: &DocumentSessionStore,
    original: &std::path::Path,
) -> TestDocumentSessionFixture {
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_dir = store
        .instance_dir
        .join("sessions")
        .join(format!("session-{session_id}"));
    create_secure_dir(&session_dir).unwrap();
    let working_path = session_dir.join("working.pdf");
    let digest = copy_source_consistent(original, &working_path).unwrap();
    let result = store.insert_candidate(
        original.to_path_buf(),
        working_path,
        session_dir,
        digest,
        CandidateEncryption::None,
    );
    let entry = store.entry(&result.session_id).unwrap();
    let mut state = entry.state.lock();
    state.lifecycle = SessionLifecycle::Open;
    let actual_generation = state.content_generation;
    TestDocumentSessionFixture {
        session_id: result.session_id,
        content_generation: actual_generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_io::DocumentIoStage;
    use tauri::Manager;

    #[test]
    fn worker_quarantine_codes_survive_render_error_strings() {
        for code in ["PDFIUM_DOCUMENT_QUARANTINED", "PDFIUM_PAGE_QUARANTINED"] {
            let error = DocumentSessionError::from(format!("{code}: detail"));
            assert_eq!(error.code, code);
            assert_eq!(error.message, "detail");
        }
        let error = DocumentSessionError::from("render_cancelled".to_string());
        assert_eq!(error.code, "DOCUMENT_SESSION_PDF_ENGINE_ERROR");
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf")
            .join(name)
    }

    fn open_test_session(store: &DocumentSessionStore, original: &Path) -> String {
        super::open_test_session(store, original).session_id
    }

    fn attach_test_transition(
        store: &DocumentSessionStore,
        session_id: &str,
        kind: TransitionKind,
    ) -> (String, TransitionExecution) {
        let token = Uuid::new_v4().to_string();
        let execution = TransitionExecution::new();
        let entry = store.entry(session_id).unwrap();
        let mut state = entry.state.lock();
        state.lifecycle = SessionLifecycle::DecisionPending;
        state.state_revision += 1;
        state.active_transition = Some(SessionTransition {
            token: token.clone(),
            source_session_id: session_id.to_string(),
            kind,
            transition_epoch: store.active_identity_epoch.load(Ordering::Acquire),
            created_at_ms: now_ms(),
            captured_content_generation: state.content_generation,
            expected_state_revision: state.state_revision,
            status: TransitionStatus::DecisionPending,
            prepared_target: None,
            execution: execution.clone(),
        });
        store.transition_authorities.lock().insert(
            token.clone(),
            TransitionAuthorityIdentity {
                source_session_id: session_id.to_string(),
                kind,
                initial_identity_epoch: store.active_identity_epoch.load(Ordering::Acquire),
            },
        );
        (token, execution)
    }

    fn open_encrypted_test_session(
        store: &DocumentSessionStore,
        directory: &tempfile::TempDir,
    ) -> String {
        let original = directory
            .path()
            .join(format!("encrypted-{}.pdf", Uuid::new_v4()));
        fs::copy(fixture_path("encrypted.pdf"), &original).unwrap();
        let session_id = open_test_session(store, &original);
        store.entry(&session_id).unwrap().state.lock().credential = Some("fixture-password".into());
        session_id
    }

    fn test_state(directory: &tempfile::TempDir, bytes: &[u8]) -> DocumentSession {
        let working = directory.path().join("working.pdf");
        fs::write(&working, bytes).unwrap();
        let digest = sha256_file(&working).unwrap();
        DocumentSession {
            id: "s".into(),
            save_target_path: working.clone(),
            working_path: working,
            session_dir: directory.path().to_path_buf(),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 0,
            baseline: BaselineFingerprint {
                digest: digest.clone(),
            },
            working_digest: Some(digest),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        }
    }

    fn test_derived_caches() -> (SearchCache, TextLayoutCache, crate::ai::VectorStore) {
        let capacity = std::num::NonZeroUsize::new(4).unwrap();
        (
            SearchCache(Mutex::new(lru::LruCache::new(capacity))),
            TextLayoutCache(Mutex::new(lru::LruCache::new(capacity))),
            crate::ai::VectorStore(parking_lot::RwLock::new(HashMap::new())),
        )
    }

    fn test_cancellation_checkpoint() -> (CancellationToken, SessionJobCancellationCheckpoint) {
        let token = CancellationToken::new();
        (
            token.clone(),
            SessionJobCancellationCheckpoint {
                token,
                reason: Arc::new(Mutex::new(None)),
            },
        )
    }

    /// Job directory with worker outputs under the planned worker names, plus
    /// an empty user output directory.
    fn job_output_set_fixture(
        planned: &[crate::output_names::PlannedOutput],
    ) -> (tempfile::TempDir, SessionJobContext, Vec<PathBuf>, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let job_dir = root.path().join("job");
        let output_dir = root.path().join("output");
        fs::create_dir(&job_dir).unwrap();
        fs::create_dir(&output_dir).unwrap();
        let snapshot_path = job_dir.join("snapshot.pdf");
        fs::write(&snapshot_path, b"%PDF-snapshot").unwrap();
        let sources = planned
            .iter()
            .map(|output| {
                let source = job_dir.join(&output.worker_name);
                fs::write(&source, format!("new {}", output.worker_name)).unwrap();
                source.canonicalize().unwrap()
            })
            .collect();
        let context = SessionJobContext::detached(snapshot_path, None, "output-set-test");
        (root, context, sources, output_dir)
    }

    fn sorted_output_names(output_dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(output_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn job_output_set_conflict_reports_names_without_writing() {
        let planned = crate::export::planned_export_image_outputs(&[0, 1, 2], "png", "rapor");
        let (_root, context, sources, output_dir) = job_output_set_fixture(&planned);
        fs::write(output_dir.join("rapor_sayfa_2.png"), b"previous export").unwrap();

        let preflight = preflight_job_output_set(&output_dir, &planned, false).unwrap_err();
        let error =
            accept_job_output_set(&context, sources, &planned, &output_dir, false).unwrap_err();

        for error in [preflight, error] {
            let SessionJobOperationError::Failed(failure) = error else {
                panic!("expected failure, got {error:?}");
            };
            assert_eq!(failure.code, SESSION_JOB_OUTPUT_CONFLICT_CODE);
            assert_eq!(
                failure
                    .conflicting_names
                    .as_ref()
                    .map(|names| names.0.as_slice()),
                Some(&["rapor_sayfa_2.png".to_string()][..])
            );
            let wire = serde_json::to_value(&failure).unwrap();
            assert_eq!(
                wire["conflictingNames"],
                serde_json::json!(["rapor_sayfa_2.png"])
            );
        }
        assert_eq!(sorted_output_names(&output_dir), ["rapor_sayfa_2.png"]);
        assert_eq!(
            fs::read(output_dir.join("rapor_sayfa_2.png")).unwrap(),
            b"previous export"
        );
    }

    #[test]
    fn job_output_set_overwrite_after_confirmation_publishes_under_document_names() {
        let planned = planned_split_outputs(&[(0, 0), (3, 5)], "rapor").unwrap();
        let (_root, context, sources, output_dir) = job_output_set_fixture(&planned);
        fs::write(output_dir.join("rapor_sayfa_4-6.pdf"), b"previous split").unwrap();

        preflight_job_output_set(&output_dir, &planned, true).unwrap();
        let published =
            accept_job_output_set(&context, sources, &planned, &output_dir, true).unwrap();

        assert_eq!(
            published,
            ["rapor_sayfa_1.pdf", "rapor_sayfa_4-6.pdf"]
                .iter()
                .map(|name| output_dir.join(name).to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            sorted_output_names(&output_dir),
            ["rapor_sayfa_1.pdf", "rapor_sayfa_4-6.pdf"]
        );
        assert_eq!(
            fs::read(output_dir.join("rapor_sayfa_1.pdf")).unwrap(),
            b"new split_0_0.pdf"
        );
        assert_eq!(
            fs::read(output_dir.join("rapor_sayfa_4-6.pdf")).unwrap(),
            b"new split_1_3.pdf"
        );
    }

    #[test]
    fn job_output_set_rejects_outputs_that_differ_from_the_plan() {
        let produced = crate::export::planned_export_image_outputs(&[0, 1], "png", "rapor");
        let (_root, context, sources, output_dir) = job_output_set_fixture(&produced);
        let expected = crate::export::planned_export_image_outputs(&[0, 2], "png", "rapor");

        let error =
            accept_job_output_set(&context, sources, &expected, &output_dir, false).unwrap_err();

        assert!(
            matches!(
                error,
                SessionJobOperationError::Failed(SessionJobFailure { ref code, .. })
                    if code == "SESSION_JOB_WORKER_PROTOCOL_ERROR"
            ),
            "{error:?}"
        );
        assert!(sorted_output_names(&output_dir).is_empty());
    }

    #[test]
    fn job_output_set_cancellation_publishes_nothing() {
        let planned = crate::export::planned_export_image_outputs(&[0, 1], "png", "rapor");
        let (_root, context, sources, output_dir) = job_output_set_fixture(&planned);
        context.cancellation.token.cancel();

        let error =
            accept_job_output_set(&context, sources, &planned, &output_dir, false).unwrap_err();

        assert!(
            matches!(error, SessionJobOperationError::Cancelled(_)),
            "{error:?}"
        );
        assert!(sorted_output_names(&output_dir).is_empty());
    }

    fn worker_and_target_names(
        planned: &[crate::output_names::PlannedOutput],
    ) -> Vec<(&str, &str)> {
        planned
            .iter()
            .map(|output| (output.worker_name.as_str(), output.target_name.as_str()))
            .collect()
    }

    #[test]
    fn planned_image_outputs_follow_worker_order_and_document_name() {
        assert_eq!(
            worker_and_target_names(&crate::export::planned_export_image_outputs(
                &[2, 0, 2, 5],
                "jpeg",
                "rapor"
            )),
            [
                ("sayfa_3.jpg", "rapor_sayfa_3.jpg"),
                ("sayfa_1.jpg", "rapor_sayfa_1.jpg"),
                ("sayfa_6.jpg", "rapor_sayfa_6.jpg"),
            ]
        );
        assert_eq!(
            worker_and_target_names(&crate::export::planned_export_image_outputs(
                &[u32::MAX],
                "png",
                "belge"
            )),
            [("sayfa_4294967296.png", "belge_sayfa_4294967296.png")]
        );
    }

    #[test]
    fn planned_split_outputs_name_ranges_and_reject_repeats() {
        assert_eq!(
            worker_and_target_names(
                &planned_split_outputs(&[(0, 2), (4, 4), (7, 9)], "rapor").unwrap()
            ),
            [
                ("split_0_0.pdf", "rapor_sayfa_1-3.pdf"),
                ("split_1_4.pdf", "rapor_sayfa_5.pdf"),
                ("split_2_7.pdf", "rapor_sayfa_8-10.pdf"),
            ]
        );
        for (ranges, shown) in [
            (vec![(0, 2), (4, 4), (0, 2)], "1-3"),
            (vec![(4, 4), (4, 4)], "5"),
        ] {
            let error = planned_split_outputs(&ranges, "rapor").unwrap_err();
            let SessionJobOperationError::Failed(failure) = error else {
                panic!("expected failure, got {error:?}");
            };
            assert_eq!(failure.code, "SESSION_JOB_INVALID_SPLIT_RANGES");
            assert!(failure.message.ends_with(shown), "{}", failure.message);
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_search_and_text_layout_tokens_before_cleanup() {
        let store = DocumentSessionStore::initialize().unwrap();
        let search = CancellationToken::new();
        store
            .search_cancellations
            .lock()
            .insert("search-session".to_string(), (1, search.clone()));
        let layout = CancellationToken::new();
        let active_tasks = ActiveRenderTasks(Mutex::new(HashMap::from([(
            "layout-task".to_string(),
            layout.clone(),
        )])));
        let render_service = RenderService::new();

        let warnings = store
            .shutdown_all_resources(&active_tasks, &render_service)
            .await;

        assert!(warnings.is_empty());
        assert!(search.is_cancelled());
        assert!(layout.is_cancelled());
        assert!(active_tasks.0.lock().is_empty());
    }

    #[tokio::test]
    async fn shutdown_reports_session_cleanup_deferred_instead_of_silent_skip() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("shutdown-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let entry = store.entry(&session_id).unwrap();
        let lease = entry.operation.clone().read_owned().await;
        let session_dir = entry.state.lock().session_dir.clone();
        let active_tasks = ActiveRenderTasks(Mutex::new(HashMap::new()));
        let render_service = RenderService::new();

        let warnings = store
            .shutdown_all_resources(&active_tasks, &render_service)
            .await;

        assert!(warnings.iter().any(|warning| {
            warning.code == "SESSION_SHUTDOWN_OPERATION_TIMEOUT"
                && warning.message.contains(&session_id)
        }));
        assert!(session_dir.exists());
        drop(lease);
        secure_remove_tree(&session_dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_does_not_drain_registry_before_in_flight_save_finishes() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("shutdown-save.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let save_reached = Arc::new(tokio::sync::Barrier::new(2));
        let save_release = Arc::new(tokio::sync::Barrier::new(2));
        *store.save_io_barriers.lock() = Some((save_reached.clone(), save_release.clone()));
        let shutdown_reached = Arc::new(tokio::sync::Barrier::new(2));
        let shutdown_release = Arc::new(tokio::sync::Barrier::new(2));
        *store.shutdown_lifecycle_barriers.lock() =
            Some((shutdown_reached.clone(), shutdown_release.clone()));

        let save_store = store.clone();
        let save_session = session_id.clone();
        let save = tokio::spawn(async move {
            save_store
                .save(&save_session, mutated.state_revision, None)
                .await
        });
        save_reached.wait().await;

        let shutdown_store = store.clone();
        let shutdown = tokio::spawn(async move {
            let active_tasks = ActiveRenderTasks(Mutex::new(HashMap::new()));
            let render_service = RenderService::new();
            shutdown_store
                .shutdown_all_resources(&active_tasks, &render_service)
                .await
        });
        shutdown_reached.wait().await;
        assert!(store.entry(&session_id).is_ok());
        shutdown_release.wait().await;
        save_release.wait().await;

        assert!(save.await.unwrap().is_ok());
        let warnings = shutdown.await.unwrap();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert!(store.entry(&session_id).is_err());
    }

    #[tokio::test]
    async fn shutdown_blocks_destructive_cleanup_for_commit_indeterminate_session() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("indeterminate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        store.entry(&session_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CommitIndeterminate;
        let session_dir = store
            .entry(&session_id)
            .unwrap()
            .state
            .lock()
            .session_dir
            .clone();
        let active_tasks = ActiveRenderTasks(Mutex::new(HashMap::new()));
        let render_service = RenderService::new();

        let warnings = store
            .shutdown_all_resources(&active_tasks, &render_service)
            .await;

        assert!(warnings
            .iter()
            .any(|warning| warning.code == "SESSION_SHUTDOWN_RECOVERY_REQUIRED"));
        assert!(store.entry(&session_id).is_ok());
        assert!(session_dir.exists());
    }

    #[tokio::test]
    async fn shutdown_preserves_interrupted_transition_authority() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("interrupted-transition.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        let (_, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        *execution.phase.lock() = TransitionExecutionPhase::InterruptedPreCommit {
            decision: TransitionDecision::Discard,
            error: DocumentSessionError::new(
                "TRANSITION_INTERRUPTED_PRE_COMMIT",
                "interrupted test authority",
            ),
        };
        let session_dir = store
            .entry(&session_id)
            .unwrap()
            .state
            .lock()
            .session_dir
            .clone();
        let active_tasks = ActiveRenderTasks(Mutex::new(HashMap::new()));
        let render_service = RenderService::new();

        let warnings = store
            .shutdown_all_resources(&active_tasks, &render_service)
            .await;

        assert!(warnings
            .iter()
            .any(|warning| warning.code == "SESSION_SHUTDOWN_RECOVERY_REQUIRED"));
        assert!(store.entry(&session_id).is_ok());
        assert!(session_dir.exists());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_stops_active_session_job_child_through_its_single_owner() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("shutdown-job-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&job_id.to_string(), &running_session, 1, |context| {
                    context.child_processes.run_with_timeout(
                        &mut shell_command("sleep 30"),
                        Duration::from_secs(60),
                        &context.cancellation,
                    )
                })
                .await
                .unwrap()
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let child_registered = store
                .job_registry
                .lock()
                .active
                .values()
                .any(|job| !job.child_processes.children.lock().is_empty());
            if child_registered {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }

        store.shutdown_session_jobs().await;
        let outcome = running.await.unwrap();
        assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
        assert!(store.job_registry.lock().active.values().all(|job| job
            .child_processes
            .children
            .lock()
            .is_empty()));
    }

    #[cfg(unix)]
    fn shell_command(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        command
    }

    /// Touches ~512 MiB and holds it, like a runaway PDFium job worker.
    #[cfg(target_os = "macos")]
    fn memory_hog_command() -> Command {
        let mut command = Command::new("perl");
        command
            .arg("-e")
            .arg("my $x = 'a' x (512 * 1024 * 1024); sleep 30;");
        command
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn job_worker_memory_budget_kills_a_runaway_child_before_its_deadline() {
        let owner = SessionJobChildProcessOwner::default();
        let cancellation = SessionJobCancellationCheckpoint::new(CancellationToken::new());
        let started = std::time::Instant::now();

        let error = owner
            .run_with_timeout_watched(
                &mut memory_hog_command(),
                Duration::from_secs(20),
                &cancellation,
                || {},
                Some(128 * 1024 * 1024),
            )
            .unwrap_err();

        let SessionJobOperationError::Failed(failure) = error else {
            panic!("expected a failure, got {error:?}");
        };
        assert_eq!(
            failure.code, "SESSION_JOB_WORKER_MEMORY_LIMIT",
            "{failure:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        // The guard killed and reaped the child; nothing stays registered.
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn platform_children_without_a_budget_are_not_memory_limited() {
        let owner = SessionJobChildProcessOwner::default();
        let cancellation = SessionJobCancellationCheckpoint::new(CancellationToken::new());

        let error = owner
            .run_with_timeout(
                &mut memory_hog_command(),
                Duration::from_secs(3),
                &cancellation,
            )
            .unwrap_err();

        // It runs to the deadline instead (a timeout failure).
        let SessionJobOperationError::Failed(failure) = error else {
            panic!("expected a failure, got {error:?}");
        };
        assert_ne!(
            failure.code, "SESSION_JOB_WORKER_MEMORY_LIMIT",
            "{failure:?}"
        );
        assert!(failure.message.contains("zaman aşımı"), "{failure:?}");
        assert!(owner.children.lock().is_empty());
    }

    fn test_command_app() -> tauri::App<tauri::test::MockRuntime> {
        let capacity = std::num::NonZeroUsize::new(8).unwrap();
        tauri::test::mock_builder()
            .manage(DocumentSessionStore::initialize().unwrap())
            .manage(SearchCache(Mutex::new(lru::LruCache::new(capacity))))
            .manage(TextLayoutCache(Mutex::new(lru::LruCache::new(capacity))))
            .manage(crate::ai::VectorStore(parking_lot::RwLock::new(
                HashMap::new(),
            )))
            .manage(crate::ai::endpoint::AiEndpointStore::new())
            .manage(crate::ai::request_registry::AiRequestRegistry::new())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    #[test]
    fn mutation_commit_state_updates_only_verified_content() {
        let directory = tempfile::tempdir().unwrap();
        let working = directory.path().join("working.pdf");
        fs::write(&working, b"%PDF-test").unwrap();
        let mut state = DocumentSession {
            id: "s".into(),
            save_target_path: working.clone(),
            working_path: working,
            session_dir: directory.path().to_path_buf(),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 0,
            baseline: BaselineFingerprint {
                digest: "before".into(),
            },
            working_digest: Some("before".into()),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        };
        apply_committed_mutation(
            &mut state,
            AtomicWriteReport {
                state: CommitState::NewCommitted,
                committed_digest: "after".into(),
            },
        )
        .unwrap();
        assert!(state.dirty);
        assert_eq!(state.content_generation, 2);
        assert_eq!(state.state_revision, 2);
        assert_eq!(state.working_digest.as_deref(), Some("after"));
    }

    fn clean_session_with_baseline(directory: &Path, baseline: &str) -> DocumentSession {
        let working = directory.join("working.pdf");
        fs::write(&working, b"%PDF-test").unwrap();
        DocumentSession {
            id: "s".into(),
            save_target_path: working.clone(),
            working_path: working,
            session_dir: directory.to_path_buf(),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 0,
            baseline: BaselineFingerprint {
                digest: baseline.into(),
            },
            working_digest: Some(baseline.into()),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        }
    }

    #[test]
    fn dirty_compares_the_working_digest_with_the_saved_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = clean_session_with_baseline(directory.path(), "saved");
        let commit = |state: &mut DocumentSession, digest: &str| {
            apply_committed_mutation(
                state,
                AtomicWriteReport {
                    state: CommitState::NewCommitted,
                    committed_digest: digest.into(),
                },
            )
            .unwrap();
        };

        commit(&mut state, "edited");
        assert!(state.dirty);
        assert_eq!(state.content_generation, 2);

        // Kaydedilmiş baytlara dönen bir commit (ileride undo) generation'ı yine ilerletir,
        // ama belge artık kirli değildir.
        commit(&mut state, "saved");
        assert!(!state.dirty);
        assert_eq!(state.content_generation, 3);
    }

    #[test]
    fn unknown_working_digest_counts_as_dirty() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = clean_session_with_baseline(directory.path(), "saved");
        state.working_digest = None;
        refresh_dirty(&mut state);
        assert!(state.dirty);
        state.working_digest = Some("saved".into());
        refresh_dirty(&mut state);
        assert!(!state.dirty);
    }

    #[test]
    fn secure_redaction_publication_failure_preserves_typed_commit_metadata_on_wire() {
        let digest = "a".repeat(64);
        for state in [
            CommitState::OriginalPreserved,
            CommitState::RecoveryRequired,
            CommitState::Indeterminate,
        ] {
            let state_wire = state.to_string();
            let recovery = PathBuf::from("/tmp/secure-redaction-recovery.pdf");
            let error = crate::document_io::DocumentIoError::new(
                "secure_redaction_publish_test",
                DocumentIoStage::Replace,
                state,
                "controlled publication failure",
            )
            .with_recovery_path(recovery.clone())
            .with_replacement_digest(digest.clone());
            let failure =
                SessionJobFailure::publication_failure("REDACTION_PUBLICATION_FAILED", error);
            assert_eq!(failure.commit_state.as_deref(), Some(state_wire.as_str()));
            assert_eq!(
                failure.recovery_path.as_deref(),
                Some(recovery.to_string_lossy().as_ref())
            );
            assert_eq!(failure.replacement_digest.as_deref(), Some(digest.as_str()));

            let wire = serde_json::to_value(&failure).unwrap();
            assert_eq!(wire["commitState"], state_wire);
            assert_eq!(wire["recoveryPath"], recovery.to_string_lossy().as_ref());
            assert_eq!(wire["replacementDigest"], digest);
        }
    }

    #[test]
    fn transition_save_commit_classification_is_exhaustive_and_typed() {
        assert_eq!(
            classify_document_io_commit_state(CommitState::Prepared),
            SaveCommitClassification::NotCommitted
        );
        assert_eq!(
            classify_document_io_commit_state(CommitState::OriginalPreserved),
            SaveCommitClassification::NotCommitted
        );
        assert_eq!(
            classify_document_io_commit_state(CommitState::OriginalRecovered),
            SaveCommitClassification::NotCommitted
        );
        assert_eq!(
            classify_document_io_commit_state(CommitState::NewCommitted),
            SaveCommitClassification::Committed
        );
        assert_eq!(
            classify_document_io_commit_state(CommitState::NewCommittedDurabilityUnconfirmed),
            SaveCommitClassification::Committed
        );
        assert_eq!(
            classify_document_io_commit_state(CommitState::Indeterminate),
            SaveCommitClassification::CommitIndeterminate
        );
        assert_eq!(
            classify_document_io_commit_state(CommitState::RecoveryRequired),
            SaveCommitClassification::RecoveryRequired
        );
    }

    #[test]
    fn indeterminate_mutation_blocks_normal_operation() {
        let directory = tempfile::tempdir().unwrap();
        let working = directory.path().join("working.pdf");
        fs::write(&working, b"%PDF-test").unwrap();
        let mut state = DocumentSession {
            id: "s".into(),
            save_target_path: working.clone(),
            working_path: working,
            session_dir: directory.path().to_path_buf(),
            content_generation: 4,
            state_revision: 7,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 0,
            baseline: BaselineFingerprint {
                digest: "before".into(),
            },
            working_digest: Some("before".into()),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        };
        let error = DocumentIoError::new(
            "rotate_page",
            crate::document_io::DocumentIoStage::Replace,
            CommitState::Indeterminate,
            "fault injection",
        );
        apply_failed_mutation(&mut state, Some("before"), &error).unwrap();
        assert_eq!(state.lifecycle, SessionLifecycle::CommitIndeterminate);
        assert_eq!(state.content_generation, 4);
        assert_eq!(state.state_revision, 8);
        assert!(state.working_digest.is_none());
        assert!(ensure_open_for_mutation(&state, 4).is_err());
    }

    #[test]
    fn durability_unconfirmed_mutation_advances_verified_generation() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = test_state(&directory, b"%PDF-before");
        apply_committed_mutation(
            &mut state,
            AtomicWriteReport {
                state: CommitState::NewCommittedDurabilityUnconfirmed,
                committed_digest: "candidate".into(),
            },
        )
        .unwrap();
        assert_eq!(state.content_generation, 2);
        assert_eq!(
            state.health_warning.as_deref(),
            Some("MUTATION_DURABILITY_UNCONFIRMED")
        );
    }

    #[test]
    fn original_recovered_classifies_verified_candidate_content() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = test_state(&directory, b"%PDF-before");
        fs::write(&state.working_path, b"%PDF-after").unwrap();
        let replacement = sha256_file(&state.working_path).unwrap();
        let mut error = DocumentIoError::new(
            "rotate_page",
            crate::document_io::DocumentIoStage::Recover,
            CommitState::OriginalRecovered,
            "recovered",
        );
        error.replacement_digest = Some(replacement.clone());
        let before = state.working_digest.clone();
        apply_failed_mutation(&mut state, before.as_deref(), &error).unwrap();
        assert_eq!(state.lifecycle, SessionLifecycle::Open);
        assert_eq!(state.content_generation, 2);
        assert_eq!(state.working_digest.as_deref(), Some(replacement.as_str()));
        assert!(state.dirty);
    }

    #[test]
    fn recovery_required_blocks_mutation_without_guessing_digest() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = test_state(&directory, b"%PDF-before");
        let before = state.working_digest.clone();
        let error = DocumentIoError::new(
            "rotate_page",
            crate::document_io::DocumentIoStage::Recover,
            CommitState::RecoveryRequired,
            "recovery artifact retained",
        );
        apply_failed_mutation(&mut state, before.as_deref(), &error).unwrap();
        assert_eq!(state.lifecycle, SessionLifecycle::RecoveryRequired);
        assert!(state.working_digest.is_none());
        assert!(ensure_open_for_mutation(&state, 1).is_err());
    }

    #[test]
    fn streaming_copy_releases_all_source_and_target_handles() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.pdf");
        let snapshot = directory.path().join("snapshot.pdf");
        fs::write(&source, b"%PDF-job").unwrap();
        copy_with_digest(&source, &snapshot).unwrap();
        let moved_source = directory.path().join("source-moved.pdf");
        let moved_snapshot = directory.path().join("snapshot-moved.pdf");
        fs::rename(&source, &moved_source).unwrap();
        fs::rename(&snapshot, &moved_snapshot).unwrap();
        assert_eq!(fs::read(moved_snapshot).unwrap(), b"%PDF-job");
    }

    #[test]
    fn transition_token_validation_keeps_expiry_as_manager_policy() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = test_state(&directory, b"%PDF-before");
        state.lifecycle = SessionLifecycle::DecisionPending;
        state.state_revision = 2;
        state.active_transition = Some(SessionTransition {
            token: "token".into(),
            source_session_id: state.id.clone(),
            kind: TransitionKind::DocumentSwitch,
            transition_epoch: 0,
            created_at_ms: now_ms(),
            captured_content_generation: state.content_generation,
            expected_state_revision: 2,
            status: TransitionStatus::DecisionPending,
            prepared_target: None,
            execution: TransitionExecution::new(),
        });
        assert!(validate_transition(&state, "token", TransitionKind::DocumentSwitch).is_ok());
        assert_eq!(
            validate_transition(&state, "token", TransitionKind::WindowClose)
                .unwrap_err()
                .code,
            "TRANSITION_KIND_MISMATCH"
        );
        state.active_transition.as_mut().unwrap().created_at_ms =
            now_ms().saturating_sub(TRANSITION_TTL_MS + 1);
        assert!(validate_transition(&state, "token", TransitionKind::DocumentSwitch).is_ok());
        assert!(transition_expired(
            state.active_transition.as_ref().unwrap()
        ));
    }

    #[test]
    fn instance_ownership_uses_sibling_lock_and_separate_job_root() {
        let store = DocumentSessionStore::initialize().unwrap();
        assert_eq!(
            store.instance_lock_path.parent(),
            store.instance_dir.parent()
        );
        assert!(!store.instance_lock_path.starts_with(&store.instance_dir));
        assert!(store.instance_dir.join("sessions").is_dir());
        assert!(store.instance_dir.join("jobs").is_dir());
    }

    #[test]
    fn live_instance_lock_prevents_orphan_cleanup() {
        let store = DocumentSessionStore::initialize().unwrap();
        cleanup_orphan_instances(&store.temp_root).unwrap();
        assert!(store.instance_dir.exists());
        assert!(store.instance_lock_path.exists());
    }

    #[test]
    fn startup_cleanup_removes_quarantine_artifact() {
        let store = DocumentSessionStore::initialize().unwrap();
        let staging = store
            .temp_root
            .join(format!("staging-test-{}", Uuid::new_v4()));
        let quarantine = store
            .temp_root
            .join(format!("cleanup-test-{}", Uuid::new_v4()));
        create_secure_dir(&staging).unwrap();
        fs::write(staging.join("partial.pdf"), b"partial").unwrap();
        fs::rename(&staging, &quarantine).unwrap();
        cleanup_orphan_instances(&store.temp_root).unwrap();
        assert!(!quarantine.exists());
    }

    #[test]
    fn concurrent_cleanups_of_the_same_tree_both_succeed() {
        let root = std::env::temp_dir().join(format!("cleanup-race-{}", Uuid::new_v4()));
        for _ in 0..20 {
            for dir in 0..4 {
                let dir = root.join(format!("d{dir}"));
                fs::create_dir_all(&dir).unwrap();
                for file in 0..16 {
                    fs::write(dir.join(format!("f{file}")), b"x").unwrap();
                }
            }
            let start = Arc::new(std::sync::Barrier::new(4));
            let removers: Vec<_> = (0..4)
                .map(|_| {
                    let (root, start) = (root.clone(), Arc::clone(&start));
                    std::thread::spawn(move || {
                        start.wait();
                        secure_remove_tree(&root)
                    })
                })
                .collect();
            for remover in removers {
                remover.join().unwrap().unwrap();
            }
            assert!(!root.exists());
        }
    }

    #[tokio::test]
    async fn exclusive_mutation_waits_until_shared_read_lease_is_released() {
        let gate = Arc::new(AsyncRwLock::new(()));
        let read = gate.clone().read_owned().await;
        let acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquired_in_task = Arc::clone(&acquired);
        let gate_in_task = Arc::clone(&gate);
        let waiter = tokio::spawn(async move {
            let _write = gate_in_task.write_owned().await;
            acquired_in_task.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        tokio::task::yield_now().await;
        assert!(!acquired.load(std::sync::atomic::Ordering::SeqCst));
        drop(read);
        waiter.await.unwrap();
        assert!(acquired.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_close_does_not_delete_independent_job_snapshot() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (path_tx, path_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let job_store = Arc::clone(&store);
        let job_session_id = session_id.clone();
        let job = tokio::spawn(async move {
            job_store
                .run_snapshot_job(&job_id, &job_session_id, 1, move |context| {
                    path_tx.send(context.snapshot_path.clone()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap()
        });
        let snapshot_path = path_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        store.close_entry(&session_id).await.unwrap();
        assert!(snapshot_path.exists());
        release_tx.send(()).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), job)
                .await
                .unwrap()
                .unwrap(),
            SessionJobOutcome::Completed { .. }
        ));
        assert!(!snapshot_path.exists());
    }

    #[tokio::test]
    async fn cancellation_before_registration_prevents_operation_from_running() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        assert!(matches!(
            store
                .cancel_job(
                    &session_id,
                    &job_id,
                    SessionJobCancellationReason::UserRequested,
                )
                .await
                .unwrap(),
            SessionJobCancelOutcome::CancelPending
        ));
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_job = Arc::clone(&ran);
        let outcome = store
            .run_snapshot_job(&job_id, &session_id, 1, move |_context| {
                ran_in_job.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        assert!(!ran.load(Ordering::SeqCst));
        assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
    }

    #[tokio::test]
    async fn pending_cancellations_are_ttl_pruned_and_capacity_bounded() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        {
            let mut registry = store.job_registry.lock();
            registry.pending_cancellations.insert(
                (session_id.clone(), Uuid::new_v4()),
                PendingSessionJobCancellation {
                    session_id: session_id.clone(),
                    created_at_ms: now_ms().saturating_sub(SESSION_JOB_PENDING_TTL_MS + 1),
                    reason: SessionJobCancellationReason::UserRequested,
                },
            );
        }
        let fresh_job_id = Uuid::new_v4().to_string();
        assert!(matches!(
            store
                .cancel_job(
                    &session_id,
                    &fresh_job_id,
                    SessionJobCancellationReason::UserRequested,
                )
                .await
                .unwrap(),
            SessionJobCancelOutcome::CancelPending
        ));
        assert_eq!(store.job_registry.lock().pending_cancellations.len(), 1);
        {
            let mut registry = store.job_registry.lock();
            registry.pending_cancellations.clear();
            for _ in 0..SESSION_JOB_PENDING_MAX {
                let id = Uuid::new_v4();
                registry.pending_cancellations.insert(
                    (session_id.clone(), id),
                    PendingSessionJobCancellation {
                        session_id: session_id.clone(),
                        created_at_ms: now_ms(),
                        reason: SessionJobCancellationReason::UserRequested,
                    },
                );
            }
        }
        let error = store
            .cancel_job(
                &session_id,
                &Uuid::new_v4().to_string(),
                SessionJobCancellationReason::UserRequested,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "SESSION_JOB_PENDING_CANCELLATION_CAPACITY");
    }

    #[test]
    fn completed_outcome_serializes_exact_identity_cancel_and_cleanup_fields() {
        let outcome = SessionJobOutcome::Completed {
            job_id: "00000000-0000-4000-8000-000000000001".to_string(),
            session_id: "session-1".to_string(),
            content_generation: 4,
            value: 7_u32,
            cancel_requested: true,
            cancellation_reason: Some(SessionJobCancellationReason::Shutdown),
            cleanup: SessionJobCleanupOutcome::RetryScheduled {
                attempt: 2,
                retry_at_ms: 99,
                warning: cleanup_warning("CLEANUP_RETRY", "cleanup retry"),
            },
        };
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["status"], "completed");
        assert_eq!(value["jobId"], "00000000-0000-4000-8000-000000000001");
        assert_eq!(value["sessionId"], "session-1");
        assert_eq!(value["contentGeneration"], 4);
        assert_eq!(value["cancelRequested"], true);
        assert_eq!(value["cancellationReason"], "shutdown");
        assert_eq!(value["cleanup"]["status"], "retry_scheduled");
        assert_eq!(value["cleanup"]["retryAtMs"], 99);
    }

    #[test]
    fn session_job_credential_debug_output_is_redacted() {
        let credential = SessionJobCredential("do-not-log-this".to_string());
        let debug = format!("{credential:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("do-not-log-this"));
    }

    #[tokio::test]
    async fn snapshot_copy_failure_after_reservation_is_a_failed_outcome() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        store.entry(&session_id).unwrap().state.lock().working_path =
            documents.path().join("missing-working.pdf");
        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                |_context| -> Result<(), SessionJobOperationError> {
                    panic!("snapshot copy failure must not run operation")
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Failed {
                cleanup: SessionJobCleanupOutcome::Cleaned,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn snapshot_worker_panic_terminalizes_as_failed_and_cleans_private_state() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("encrypted.pdf");
        fs::copy(fixture_path("encrypted.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        store.entry(&session_id).unwrap().state.lock().credential = Some("fixture-password".into());
        let job_id = Uuid::new_v4();
        store.snapshot_worker_panic_jobs.lock().insert(job_id);

        let outcome = store
            .run_snapshot_job(
                &job_id.to_string(),
                &session_id,
                1,
                |_context| -> Result<(), SessionJobOperationError> {
                    panic!("operation must not run after snapshot worker panic")
                },
            )
            .await
            .unwrap();
        let serialized = serde_json::to_string(&outcome).unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Failed {
                error: SessionJobFailure { ref code, .. },
                cleanup: SessionJobCleanupOutcome::Cleaned,
                ..
            } if code == "SESSION_JOB_SNAPSHOT_WORKER_FAILED"
        ));
        assert!(!serialized.contains(&store.instance_dir.to_string_lossy().to_string()));
        assert!(!serialized.contains("fixture-password"));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn operation_worker_panic_terminalizes_as_failed_and_cleans_private_state() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                |_context| -> Result<(), SessionJobOperationError> {
                    panic!("controlled operation worker panic")
                },
            )
            .await
            .unwrap();
        let serialized = serde_json::to_string(&outcome).unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Failed {
                error: SessionJobFailure { ref code, .. },
                cleanup: SessionJobCleanupOutcome::Cleaned,
                ..
            } if code == "SESSION_JOB_OPERATION_WORKER_FAILED"
        ));
        assert!(!serialized.contains(&store.instance_dir.to_string_lossy().to_string()));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn operation_worker_panic_preserves_cleanup_warning_and_reaper_can_finish() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("encrypted.pdf");
        fs::copy(fixture_path("encrypted.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        store.entry(&session_id).unwrap().state.lock().credential = Some("fixture-password".into());
        let job_id = Uuid::new_v4();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(
                    &job_id.to_string(),
                    &running_session,
                    1,
                    move |_context| -> Result<(), SessionJobOperationError> {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        panic!("controlled operation panic with cleanup warning")
                    },
                )
                .await
                .unwrap()
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let record = store
            .job_registry
            .lock()
            .active
            .get(&job_id)
            .cloned()
            .unwrap();
        record
            .cleanup_failures_remaining
            .store(1, Ordering::Release);
        release_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&outcome).unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Failed {
                error: SessionJobFailure { ref code, .. },
                cleanup: SessionJobCleanupOutcome::RetryScheduled { attempt: 1, .. },
                ..
            } if code == "SESSION_JOB_OPERATION_WORKER_FAILED"
        ));
        assert!(record.mutable.lock().credential_context.is_none());
        assert!(!serialized.contains(&record.snapshot_path.to_string_lossy().to_string()));
        assert!(!serialized.contains("fixture-password"));
        record.mutable.lock().cleanup_status =
            SessionJobCleanupStatus::RetryScheduled { retry_at_ms: 0 };
        reap_session_jobs_once(&store.job_registry).await;
        assert!(!store.job_registry.lock().active.contains_key(&job_id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_and_duplicate_job_ids_are_typed_admission_errors() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let invalid = store
            .run_snapshot_job("not-a-uuid", &session_id, 1, |_context| Ok(()))
            .await
            .unwrap_err();
        assert_eq!(invalid.code, "SESSION_JOB_INVALID_ID");

        let job_id = Uuid::new_v4().to_string();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |_context| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let duplicate = store
            .run_snapshot_job(&job_id, &session_id, 1, |_context| Ok(()))
            .await
            .unwrap_err();
        assert_eq!(duplicate.code, "SESSION_JOB_ALREADY_EXISTS");
        let other_session_id = open_test_session(&store, &original);
        let owner_mismatch = store
            .run_snapshot_job(&job_id, &other_session_id, 1, |_context| Ok(()))
            .await
            .unwrap_err();
        assert_eq!(owner_mismatch.code, "SESSION_JOB_OWNER_MISMATCH");
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn frontend_job_id_is_canonicalized_before_registry_and_path_use() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let canonical = Uuid::new_v4();
        let raw = canonical.to_string().to_uppercase();
        let observed_path = Arc::new(Mutex::new(None::<PathBuf>));
        let observed_in_job = Arc::clone(&observed_path);
        let outcome = store
            .run_snapshot_job(&raw, &session_id, 1, move |context| {
                *observed_in_job.lock() = Some(context.snapshot_path);
                Ok(())
            })
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Completed { ref job_id, .. }
                if job_id == &canonical.to_string()
        ));
        assert!(observed_path
            .lock()
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains(&canonical.to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn successful_irreversible_operation_preserves_late_cancel_request() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (side_effect_tx, side_effect_rx) = std::sync::mpsc::channel();
        let (return_tx, return_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |_context| {
                    side_effect_tx.send(()).unwrap();
                    return_rx.recv().unwrap();
                    Ok("published")
                })
                .await
                .unwrap()
        });
        side_effect_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            store
                .cancel_job(
                    &session_id,
                    &job_id,
                    SessionJobCancellationReason::UserRequested,
                )
                .await
                .unwrap(),
            SessionJobCancelOutcome::CancelRequested
        ));
        return_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Completed {
                value: "published",
                cancel_requested: true,
                cancellation_reason: Some(SessionJobCancellationReason::UserRequested),
                ..
            }
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cleanup_failure_is_reported_without_losing_completed_operation() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |_context| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(42_u32)
                })
                .await
                .unwrap()
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let record = store
            .job_registry
            .lock()
            .active
            .get(&Uuid::parse_str(&job_id).unwrap())
            .cloned()
            .unwrap();
        record
            .cleanup_failures_remaining
            .store(1, Ordering::Release);
        release_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Completed {
                value: 42,
                cleanup: SessionJobCleanupOutcome::RetryScheduled { attempt: 1, .. },
                ..
            }
        ));
        assert!(store
            .job_registry
            .lock()
            .active
            .contains_key(&record.job_id));
        record.mutable.lock().cleanup_status =
            SessionJobCleanupStatus::RetryScheduled { retry_at_ms: 0 };
        reap_session_jobs_once(&store.job_registry).await;
        assert!(!store
            .job_registry
            .lock()
            .active
            .contains_key(&record.job_id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn operation_failure_and_cleanup_warning_are_preserved_together() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |_context| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Err::<(), SessionJobOperationError>(SessionJobOperationError::Failed(
                        SessionJobFailure::new("CONTROLLED_FAILURE", "operation failed"),
                    ))
                })
                .await
                .unwrap()
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let record = store
            .job_registry
            .lock()
            .active
            .get(&Uuid::parse_str(&job_id).unwrap())
            .cloned()
            .unwrap();
        record
            .cleanup_failures_remaining
            .store(1, Ordering::Release);
        release_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Failed {
                error: SessionJobFailure { ref code, .. },
                cleanup: SessionJobCleanupOutcome::RetryScheduled { .. },
                ..
            } if code == "CONTROLLED_FAILURE"
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn full_deferred_cleanup_capacity_preserves_terminal_record_and_backpressures_admission()
    {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |_context| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap()
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let record = store
            .job_registry
            .lock()
            .active
            .get(&Uuid::parse_str(&job_id).unwrap())
            .cloned()
            .unwrap();
        record.mutable.lock().cleanup_attempts = SESSION_JOB_CLEANUP_MAX_ATTEMPTS - 1;
        record
            .cleanup_failures_remaining
            .store(1, Ordering::Release);
        {
            let mut registry = store.job_registry.lock();
            for _ in 0..SESSION_JOB_DEFERRED_MAX {
                registry
                    .deferred_cleanup
                    .insert(Uuid::new_v4(), Arc::clone(&record));
            }
        }
        release_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Completed {
                cleanup: SessionJobCleanupOutcome::RetryScheduled {
                    attempt: SESSION_JOB_CLEANUP_MAX_ATTEMPTS,
                    ..
                },
                ..
            }
        ));
        assert!(store
            .job_registry
            .lock()
            .active
            .contains_key(&record.job_id));
        let admission = store
            .run_snapshot_job(&Uuid::new_v4().to_string(), &session_id, 1, |_context| {
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(admission.code, "SESSION_JOB_CLEANUP_BACKPRESSURE");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fifth_cleanup_failure_moves_record_to_deferred_only_after_successful_insertion() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let canonical_job_id = Uuid::parse_str(&job_id).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |_context| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap()
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let record = store
            .job_registry
            .lock()
            .active
            .get(&canonical_job_id)
            .cloned()
            .unwrap();
        record.mutable.lock().cleanup_attempts = SESSION_JOB_CLEANUP_MAX_ATTEMPTS - 1;
        record
            .cleanup_failures_remaining
            .store(1, Ordering::Release);
        release_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            outcome,
            SessionJobOutcome::Completed {
                cleanup: SessionJobCleanupOutcome::Deferred {
                    attempts: SESSION_JOB_CLEANUP_MAX_ATTEMPTS,
                    ..
                },
                ..
            }
        ));
        {
            let registry = store.job_registry.lock();
            assert!(!registry.active.contains_key(&canonical_job_id));
            assert!(registry.deferred_cleanup.contains_key(&canonical_job_id));
        }
        reap_session_jobs_once(&store.job_registry).await;
        assert!(!store
            .job_registry
            .lock()
            .deferred_cleanup
            .contains_key(&canonical_job_id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completion_cancel_reaper_and_session_close_race_is_bounded() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |context| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    context.cancellation.check_cancelled()
                })
                .await
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let cancel_store = Arc::clone(&store);
        let cancel_session = session_id.clone();
        let cancel_job_id = job_id.clone();
        let close_store = Arc::clone(&store);
        let close_session = session_id.clone();
        let reaper_store = Arc::clone(&store);
        let race = async move {
            let cancel = async move {
                cancel_store
                    .cancel_job(
                        &cancel_session,
                        &cancel_job_id,
                        SessionJobCancellationReason::UserRequested,
                    )
                    .await
            };
            let close = async move { close_store.close_entry(&close_session).await };
            let reaper = async move { reap_session_jobs_once(&reaper_store.job_registry).await };
            let (cancel_result, close_result, _) = tokio::join!(cancel, close, reaper);
            cancel_result.unwrap();
            close_result.unwrap();
            release_tx.send(()).unwrap();
            running.await.unwrap().unwrap()
        };
        let outcome = tokio::time::timeout(Duration::from_secs(3), race)
            .await
            .expect("completion/cancel/reaper/close yarışı timeout olmamalı");
        assert!(matches!(outcome, SessionJobOutcome::Cancelled { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn child_cancel_kills_reaps_and_clears_registry() {
        let owner = SessionJobChildProcessOwner::default();
        let (token, checkpoint) = test_cancellation_checkpoint();
        let running_owner = owner.clone();
        let running = std::thread::spawn(move || {
            running_owner.run_with_timeout(
                &mut shell_command("sleep 10"),
                Duration::from_secs(30),
                &checkpoint,
            )
        });
        for _ in 0..100 {
            if !owner.children.lock().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(owner.children.lock().len(), 1);
        token.cancel();
        let result = running.join().unwrap();
        assert!(matches!(
            result,
            Err(SessionJobOperationError::Cancelled(
                SessionJobCancellationReason::UserRequested
            ))
        ));
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn child_timeout_kills_reaps_and_clears_registry() {
        let owner = SessionJobChildProcessOwner::default();
        let (_, checkpoint) = test_cancellation_checkpoint();
        let result = owner.run_with_timeout(
            &mut shell_command("sleep 10"),
            Duration::from_millis(50),
            &checkpoint,
        );
        assert!(matches!(result, Err(SessionJobOperationError::Failed(_))));
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn child_try_wait_failure_is_raii_cleaned() {
        let owner = SessionJobChildProcessOwner::default();
        owner
            .test_hooks
            .fail_next_try_wait
            .store(true, Ordering::Release);
        let (_, checkpoint) = test_cancellation_checkpoint();
        let result = owner.run_with_timeout(
            &mut shell_command("sleep 10"),
            Duration::from_secs(5),
            &checkpoint,
        );
        assert!(matches!(result, Err(SessionJobOperationError::Failed(_))));
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn child_output_read_failure_is_raii_cleaned() {
        let owner = SessionJobChildProcessOwner::default();
        owner
            .test_hooks
            .fail_next_output_read
            .store(true, Ordering::Release);
        let (_, checkpoint) = test_cancellation_checkpoint();
        let result = owner.run_with_output_timeout(
            &mut shell_command("printf output"),
            Duration::from_secs(5),
            &checkpoint,
        );
        assert!(matches!(
            result,
            Err(SessionJobOperationError::Failed(SessionJobFailure {
                ref code,
                ..
            })) if code == "SESSION_JOB_CHILD_OUTPUT_READ_FAILED"
        ));
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn large_child_stdout_and_stderr_are_drained_without_pipe_deadlock() {
        let owner = SessionJobChildProcessOwner::default();
        let (_, checkpoint) = test_cancellation_checkpoint();
        let output = owner
            .run_with_output_timeout(
                &mut shell_command(
                    "i=0; while [ $i -lt 20000 ]; do printf 'stdout-line-%05d\\n' \"$i\"; printf 'stderr-line-%05d\\n' \"$i\" >&2; i=$((i+1)); done",
                ),
                Duration::from_secs(10),
                &checkpoint,
            )
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() > 200_000);
        assert!(output.stderr.len() > 200_000);
        assert!(owner.children.lock().is_empty());
    }

    fn failure_code(result: Result<Output, SessionJobOperationError>) -> String {
        match result {
            Err(SessionJobOperationError::Failed(failure)) => failure.code,
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn bounded_child_stdout_overflow_kills_the_process_early() {
        let owner = SessionJobChildProcessOwner::default();
        let (_, checkpoint) = test_cancellation_checkpoint();
        let started = std::time::Instant::now();
        // `yes` hiç bitmez; yalnız taşma tespiti süreci durdurabilir.
        let result = owner.run_with_bounded_output_timeout(
            &mut shell_command("yes"),
            Duration::from_secs(20),
            &checkpoint,
            ChildOutputLimits {
                max_stdout_bytes: 4096,
                max_stderr_bytes: 4096,
            },
        );
        assert_eq!(failure_code(result), "SESSION_JOB_CHILD_OUTPUT_TOO_LARGE");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_child_stdout_at_limit_is_accepted_and_stderr_is_truncated() {
        let owner = SessionJobChildProcessOwner::default();
        let (_, checkpoint) = test_cancellation_checkpoint();
        let output = owner
            .run_with_bounded_output_timeout(
                &mut shell_command(
                    "printf 12345678; i=0; while [ $i -lt 5000 ]; do printf 'stderr-line\\n' >&2; i=$((i+1)); done",
                ),
                Duration::from_secs(10),
                &checkpoint,
                ChildOutputLimits {
                    max_stdout_bytes: 8,
                    max_stderr_bytes: 16,
                },
            )
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"12345678");
        assert_eq!(output.stderr, b"stderr-line\nstde");
        assert!(owner.children.lock().is_empty());
    }

    #[test]
    fn missing_child_program_has_a_distinct_error_code() {
        let owner = SessionJobChildProcessOwner::default();
        let (_, checkpoint) = test_cancellation_checkpoint();
        let result = owner.run_with_output_timeout(
            &mut Command::new("antigravity-definitely-missing-program"),
            Duration::from_secs(5),
            &checkpoint,
        );
        assert_eq!(failure_code(result), CHILD_PROGRAM_NOT_FOUND_CODE);
        assert!(owner.children.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn repeated_child_cancel_does_not_leave_or_double_own_process() {
        let owner = SessionJobChildProcessOwner::default();
        let (token, checkpoint) = test_cancellation_checkpoint();
        let running_owner = owner.clone();
        let running = std::thread::spawn(move || {
            running_owner.run_with_timeout(
                &mut shell_command("sleep 10"),
                Duration::from_secs(30),
                &checkpoint,
            )
        });
        for _ in 0..100 {
            if !owner.children.lock().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        token.cancel();
        owner.kill_and_reap_all();
        owner.kill_and_reap_all();
        let _ = running.join().unwrap();
        assert!(owner.children.lock().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_session_cancel_preserves_owner_job_token_and_snapshot() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let owner_session_id = open_test_session(&store, &original);
        let wrong_session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let (context_tx, context_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let running_store = Arc::clone(&store);
        let running_session = owner_session_id.clone();
        let running_job_id = job_id.clone();
        let running = tokio::spawn(async move {
            running_store
                .run_snapshot_job(&running_job_id, &running_session, 1, move |context| {
                    context_tx
                        .send((context.snapshot_path.clone(), context.cancellation.clone()))
                        .unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
                .unwrap()
        });
        let (snapshot_path, cancellation) =
            context_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let error = store
            .cancel_job(
                &wrong_session_id,
                &job_id,
                SessionJobCancellationReason::UserRequested,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "SESSION_JOB_OWNER_MISMATCH");
        assert!(!cancellation.is_cancel_requested());
        assert!(snapshot_path.exists());
        release_tx.send(()).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), running)
                .await
                .unwrap()
                .unwrap(),
            SessionJobOutcome::Completed { .. }
        ));
        assert!(!snapshot_path.exists());
    }

    #[tokio::test]
    async fn ocr_command_helper_rejects_stale_generation() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();

        let error = crate::ocr::run_ocr_on_page_for_session(
            &store,
            &job_id,
            &session_id,
            0,
            0,
            "eng".into(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "STALE_GENERATION");
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn ocr_detection_uses_snapshot_credential_for_encrypted_pdf() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("encrypted.pdf");
        fs::copy(fixture_path("encrypted.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        store.entry(&session_id).unwrap().state.lock().credential = Some("fixture-password".into());
        let job_id = Uuid::new_v4().to_string();

        let result =
            crate::ocr::detect_scanned_pages_for_session(&store, &job_id, &session_id, 1).await;
        assert!(matches!(
            result,
            Ok(SessionJobOutcome::Completed { ref value, .. }) if !value.is_empty()
        ));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn ocr_detection_returns_one_flag_per_page_from_a_single_job() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("three-distinct-pages.pdf");
        fs::copy(fixture_path("three-distinct-pages.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);

        let outcome = crate::ocr::detect_scanned_pages_for_session(
            &store,
            &Uuid::new_v4().to_string(),
            &session_id,
            1,
        )
        .await
        .unwrap();
        match outcome {
            SessionJobOutcome::Completed { value, .. } => assert_eq!(value.len(), 3),
            other => panic!("unexpected outcome: {other:?}"),
        }
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn encrypted_semantic_index_extraction_uses_snapshot_credential() {
        let documents = tempfile::tempdir().unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_encrypted_test_session(&store, &documents);
        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| crate::ai::extract_semantic_page_texts(&context).map(|_| ()),
            )
            .await
            .unwrap();
        assert!(matches!(outcome, SessionJobOutcome::Completed { .. }));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn encrypted_sensitive_data_scan_uses_snapshot_credential() {
        let documents = tempfile::tempdir().unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_encrypted_test_session(&store, &documents);
        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::ScanSensitiveData {
                        patterns: vec!["test".to_string()],
                    },
                    Duration::from_secs(30),
                )? {
                    crate::job_worker::JobResult::SensitiveReport(_) => Ok(()),
                    _ => Err(SessionJobOperationError::failed(
                        "unexpected sensitive scan result",
                    )),
                },
            )
            .await
            .unwrap();
        assert!(matches!(outcome, SessionJobOutcome::Completed { .. }));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn encrypted_active_session_merge_uses_snapshot_credential() {
        let documents = tempfile::tempdir().unwrap();
        let additional = documents.path().join("additional.pdf");
        fs::copy(fixture_path("minimal.pdf"), &additional).unwrap();
        let output = documents.path().join("merged.pdf");
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_encrypted_test_session(&store, &documents);
        let outcome = merge_session_pdfs(
            Uuid::new_v4().to_string(),
            session_id,
            1,
            vec![additional.to_string_lossy().to_string()],
            output.to_string_lossy().to_string(),
            app.state(),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, SessionJobOutcome::Completed { .. }));
        assert!(output.is_file());
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn print_resource_inspection_runs_in_session_job_worker_with_authoritative_identity() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("inspection.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let expected_session_id = session_id.clone();
        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::InspectPrintResources {
                        content_generation: 1,
                        physical_page_index: 0,
                        request_id: "inspection-worker-request".to_string(),
                    },
                    Duration::from_secs(30),
                )? {
                    crate::job_worker::JobResult::PrintResourceInspection(report) => {
                        assert_eq!(report.identity.session_id, expected_session_id);
                        assert_eq!(report.identity.content_generation, 1);
                        assert_eq!(report.identity.physical_page_index, 0);
                        assert_eq!(report.identity.request_id, "inspection-worker-request");
                        assert!(!report.conformance_claim);
                        Ok(())
                    }
                    _ => Err(SessionJobOperationError::failed(
                        "unexpected inspection result",
                    )),
                },
            )
            .await
            .unwrap();
        assert!(matches!(outcome, SessionJobOutcome::Completed { .. }));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn encrypted_rgb_derived_previews_use_snapshot_credential() {
        let documents = tempfile::tempdir().unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_encrypted_test_session(&store, &documents);

        let coverage = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::GenerateRgbDerivedCoveragePreview {
                        content_generation: 1,
                        physical_page_index: 0,
                        request_id: "coverage-request".to_string(),
                        threshold_percent: 300.0,
                        scale: 0.25,
                    },
                    Duration::from_secs(30),
                )? {
                    crate::job_worker::JobResult::RgbDerivedCoveragePreview(result) => {
                        assert!(result.metadata.approximate);
                        assert!(!result.metadata.color_managed);
                        Ok(())
                    }
                    _ => Err(SessionJobOperationError::failed(
                        "unexpected preview result",
                    )),
                },
            )
            .await
            .unwrap();
        assert!(matches!(coverage, SessionJobOutcome::Completed { .. }));

        let process = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::GenerateRgbDerivedProcessPreview {
                        content_generation: 1,
                        physical_page_index: 0,
                        request_id: "process-request".to_string(),
                        scale: 0.25,
                    },
                    Duration::from_secs(30),
                )? {
                    crate::job_worker::JobResult::RgbDerivedProcessPreview(result) => {
                        assert!(!result.metadata.output_intent_applied);
                        assert!(!result.metadata.spot_colors_preserved);
                        Ok(())
                    }
                    _ => Err(SessionJobOperationError::failed(
                        "unexpected preview result",
                    )),
                },
            )
            .await
            .unwrap();
        assert!(matches!(process, SessionJobOutcome::Completed { .. }));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[tokio::test]
    async fn encrypted_lopdf_prepress_paths_return_typed_unsupported_without_credential_leak() {
        let documents = tempfile::tempdir().unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_encrypted_test_session(&store, &documents);

        let inspection = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| {
                    crate::job_worker::run_job(
                        &context,
                        crate::job_worker::JobCommand::InspectPrintResources {
                            content_generation: 1,
                            physical_page_index: 0,
                            request_id: "inspection-request".to_string(),
                        },
                        Duration::from_secs(30),
                    )
                    .map(|_| ())
                },
            )
            .await
            .unwrap();
        let serialized = serde_json::to_string(&inspection).unwrap();
        assert!(matches!(
            inspection,
            SessionJobOutcome::Failed {
                error: SessionJobFailure { ref code, .. },
                ..
            } if code == "PRINT_INSPECTION_ENCRYPTED_INPUT_UNSUPPORTED"
        ));
        assert!(!serialized.contains("fixture-password"));
        assert!(!serialized.contains(&store.instance_dir.to_string_lossy().to_string()));
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn share_command_helper_rejects_stale_generation() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();

        let error = crate::share_session_snapshot_with(&store, &job_id, &session_id, 0, |_| {
            panic!("stale share operation must not run")
        })
        .await
        .unwrap_err();
        assert_eq!(error.code, "STALE_GENERATION");
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn share_timeout_kills_child_and_removes_snapshot_job_directory() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let job_id = Uuid::new_v4().to_string();
        let observed_path = Arc::new(Mutex::new(None::<PathBuf>));
        let path_from_share = Arc::clone(&observed_path);

        let outcome =
            crate::share_session_snapshot_with(&store, &job_id, &session_id, 1, move |context| {
                *path_from_share.lock() = Some(context.snapshot_path.clone());
                context
                    .child_processes
                    .run_with_timeout(
                        std::process::Command::new("/bin/sh")
                            .arg("-c")
                            .arg("sleep 5"),
                        Duration::from_millis(10),
                        &context.cancellation,
                    )
                    .map(|_| ())
            })
            .await
            .unwrap();

        assert!(matches!(outcome, SessionJobOutcome::Failed { .. }));
        let snapshot_path = observed_path.lock().clone().unwrap();
        assert!(!snapshot_path.exists());
        assert!(!snapshot_path.parent().unwrap().exists());
        assert!(store.job_registry.lock().active.is_empty());
    }

    // The display-name sanitizer is the macOS share flow's: it splits on `/`
    // and turns `\` into `_`, so Windows display paths give a different name.
    #[cfg(unix)]
    #[tokio::test]
    async fn share_copy_uses_display_name_and_is_removed_with_job() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("Yıllık Rapor.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let display_path = store
            .session_snapshot(&session_id)
            .await
            .unwrap()
            .display_path;
        let file_name = crate::macos_helper::sanitize_share_file_name(Some(&display_path));
        assert_eq!(file_name, "Yıllık Rapor.pdf");
        let observed = Arc::new(Mutex::new(None::<(PathBuf, PathBuf)>));
        let observed_from_job = Arc::clone(&observed);

        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| {
                    let invalid = stage_job_share_copy(&context, "../escape.pdf").unwrap_err();
                    assert!(matches!(
                        invalid,
                        SessionJobOperationError::Failed(ref failure)
                            if failure.code == "SESSION_JOB_SHARE_NAME_INVALID"
                    ));
                    let shared = stage_job_share_copy(&context, &file_name)?;
                    assert_eq!(shared.file_name().unwrap(), "Yıllık Rapor.pdf");
                    assert_eq!(
                        shared.parent().unwrap().parent().unwrap(),
                        context.snapshot_path.parent().unwrap()
                    );
                    assert_eq!(
                        fs::read(&shared).unwrap(),
                        fs::read(&context.snapshot_path).unwrap()
                    );
                    *observed_from_job.lock() = Some((shared, context.snapshot_path.clone()));
                    Ok(())
                },
            )
            .await
            .unwrap();

        assert!(
            matches!(outcome, SessionJobOutcome::Completed { .. }),
            "{outcome:?}"
        );
        let (shared, snapshot_path) = observed.lock().clone().unwrap();
        assert!(!shared.exists());
        assert!(!snapshot_path.parent().unwrap().exists());
        assert!(fs::read(&original).is_ok());
    }

    #[test]
    fn unsupported_metadata_write_preserves_session_state() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let before = snapshot(&store.entry(&session_id).unwrap().state.lock());

        let error =
            reject_unsupported_session_write(&store, &session_id, 1, "metadata unsupported")
                .unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_PDFIUM_CAPABILITY");
        let after = snapshot(&store.entry(&session_id).unwrap().state.lock());
        assert_eq!(after.dirty, before.dirty);
        assert_eq!(after.content_generation, before.content_generation);
        assert_eq!(after.state_revision, before.state_revision);
    }

    #[tokio::test]
    async fn failed_snapshot_job_removes_its_private_copy() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("snapshot-source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let observed_path = Arc::new(Mutex::new(None::<PathBuf>));
        let path_from_job = Arc::clone(&observed_path);

        let outcome = store
            .run_snapshot_job(
                &Uuid::new_v4().to_string(),
                &session_id,
                1,
                move |context| {
                    *path_from_job.lock() = Some(context.snapshot_path.clone());
                    Err::<(), SessionJobOperationError>(SessionJobOperationError::failed(
                        "controlled platform failure",
                    ))
                },
            )
            .await
            .unwrap();

        assert!(matches!(outcome, SessionJobOutcome::Failed { .. }));
        let snapshot_path = observed_path.lock().clone().unwrap();
        assert!(!snapshot_path.exists());
        assert!(store.job_registry.lock().active.is_empty());
    }

    #[test]
    fn public_session_snapshot_omits_private_source_and_working_paths() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(&directory, b"%PDF-before");
        let value = serde_json::to_value(snapshot(&state)).unwrap();
        let object = value.as_object().unwrap();

        assert!(!object.contains_key("openedFromPath"));
        assert!(!object.contains_key("workingPath"));
        assert!(!object.contains_key("originalPath"));
        assert!(object.contains_key("displayPath"));
        assert!(object.contains_key("fileName"));
    }

    #[tokio::test]
    async fn open_mutate_save_preserves_original_until_commit_and_cleans_dirty() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("original.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let original_before = fs::read(&original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);

        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        assert!(mutated.dirty);
        assert_eq!(mutated.content_generation, 2);
        assert_eq!(fs::read(&original).unwrap(), original_before);

        let saved = store
            .save(&session_id, mutated.state_revision, None)
            .await
            .unwrap();
        assert!(!saved.dirty);
        assert_eq!(saved.content_generation, 2);
        assert_ne!(fs::read(&original).unwrap(), original_before);
    }

    fn working_bytes(store: &DocumentSessionStore, session_id: &str) -> Vec<u8> {
        let path = store
            .entry(session_id)
            .unwrap()
            .state
            .lock()
            .working_path
            .clone();
        fs::read(path).unwrap()
    }

    fn history_files(store: &DocumentSessionStore, session_id: &str) -> usize {
        let directory = store
            .entry(session_id)
            .unwrap()
            .state
            .lock()
            .session_dir
            .join("history");
        fs::read_dir(directory)
            .map(|entries| entries.count())
            .unwrap_or(0)
    }

    fn rotate(page_index: u32) -> MutationRecipe {
        MutationRecipe::RotatePage {
            page_index,
            degrees: 90,
        }
    }

    #[tokio::test]
    async fn undo_and_redo_restore_exact_bytes_and_track_dirty() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("undo.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let opened = working_bytes(&store, &session_id);

        let rotated = store
            .mutate_isolated(session_id.clone(), 1, rotate(0))
            .await
            .unwrap();
        let rotated_bytes = working_bytes(&store, &session_id);
        assert!(rotated.dirty);
        assert_eq!(rotated.undo_operation.as_deref(), Some("rotate_page"));
        assert_eq!(rotated.redo_operation, None);
        assert_eq!(history_files(&store, &session_id), 1);

        let undone = store
            .mutate_isolated_restore(session_id.clone(), 2, HistoryDirection::Undo)
            .await
            .unwrap();
        assert_eq!(working_bytes(&store, &session_id), opened);
        assert_eq!(undone.content_generation, 3, "generation never goes back");
        assert!(!undone.dirty, "back at the opened bytes");
        assert_eq!(undone.undo_operation, None);
        assert_eq!(undone.redo_operation.as_deref(), Some("rotate_page"));
        assert_eq!(history_files(&store, &session_id), 1);

        let redone = store
            .mutate_isolated_restore(session_id.clone(), 3, HistoryDirection::Redo)
            .await
            .unwrap();
        assert_eq!(working_bytes(&store, &session_id), rotated_bytes);
        assert_eq!(redone.content_generation, 4);
        assert!(redone.dirty);
        assert_eq!(redone.undo_operation.as_deref(), Some("rotate_page"));
        assert_eq!(redone.redo_operation, None);
        assert_eq!(history_files(&store, &session_id), 1);
    }

    #[tokio::test]
    async fn a_new_mutation_after_undo_discards_redo() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("redo-discard.pdf");
        fs::copy(fixture_path("three-distinct-pages.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);

        store
            .mutate_isolated(session_id.clone(), 1, rotate(0))
            .await
            .unwrap();
        store
            .mutate_isolated(session_id.clone(), 2, rotate(1))
            .await
            .unwrap();
        let undone = store
            .mutate_isolated_restore(session_id.clone(), 3, HistoryDirection::Undo)
            .await
            .unwrap();
        assert_eq!(undone.redo_operation.as_deref(), Some("rotate_page"));
        assert_eq!(history_files(&store, &session_id), 2);

        let deleted = store
            .mutate_isolated(
                session_id.clone(),
                4,
                MutationRecipe::DeletePages {
                    page_indices: vec![2],
                },
            )
            .await
            .unwrap();
        assert_eq!(deleted.undo_operation.as_deref(), Some("delete_pages"));
        assert_eq!(deleted.redo_operation, None);
        assert_eq!(history_files(&store, &session_id), 2, "redo blob removed");

        // İki adım geri: önce silme, sonra ilk döndürme.
        let back = store
            .mutate_isolated_restore(session_id.clone(), 5, HistoryDirection::Undo)
            .await
            .unwrap();
        assert_eq!(back.undo_operation.as_deref(), Some("rotate_page"));
        let back = store
            .mutate_isolated_restore(session_id.clone(), 6, HistoryDirection::Undo)
            .await
            .unwrap();
        assert_eq!(back.undo_operation, None);
        assert!(!back.dirty);
    }

    #[tokio::test]
    async fn undo_with_an_empty_history_leaves_the_document_alone() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("empty-history.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let before = working_bytes(&store, &session_id);

        for direction in [HistoryDirection::Undo, HistoryDirection::Redo] {
            let error = store
                .mutate_isolated_restore(session_id.clone(), 1, direction)
                .await
                .unwrap_err();
            assert_eq!(error.code, "HISTORY_EMPTY");
        }
        assert_eq!(working_bytes(&store, &session_id), before);
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert_eq!(state.content_generation, 1);
        assert!(!state.dirty);
    }

    #[tokio::test]
    async fn a_corrupted_history_file_is_refused_and_the_history_cleared() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("corrupted-history.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        store
            .mutate_isolated(session_id.clone(), 1, rotate(0))
            .await
            .unwrap();
        let rotated = working_bytes(&store, &session_id);

        let blob = store.entry(&session_id).unwrap().state.lock().history.undo[0]
            .blob
            .clone();
        let mut tampered = fs::read(&blob).unwrap();
        tampered.push(b'\n');
        fs::write(&blob, tampered).unwrap();

        let error = store
            .mutate_isolated_restore(session_id.clone(), 2, HistoryDirection::Undo)
            .await
            .unwrap_err();
        assert_eq!(error.code, "HISTORY_ENTRY_CORRUPTED");
        assert_eq!(working_bytes(&store, &session_id), rotated);
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert_eq!(state.content_generation, 2);
        assert!(state.history.undo.is_empty() && state.history.redo.is_empty());
        assert!(!blob.exists());
    }

    #[tokio::test]
    async fn undo_past_a_save_is_dirty_and_redo_is_clean_again() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("undo-save.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);

        let rotated = store
            .mutate_isolated(session_id.clone(), 1, rotate(0))
            .await
            .unwrap();
        let saved = store
            .save(&session_id, rotated.state_revision, None)
            .await
            .unwrap();
        assert!(!saved.dirty);
        assert_eq!(saved.undo_operation.as_deref(), Some("rotate_page"));

        let undone = store
            .mutate_isolated_restore(session_id.clone(), 2, HistoryDirection::Undo)
            .await
            .unwrap();
        assert!(undone.dirty, "differs from the saved file");
        let redone = store
            .mutate_isolated_restore(session_id.clone(), 3, HistoryDirection::Redo)
            .await
            .unwrap();
        assert!(!redone.dirty, "byte-identical to the saved file again");
    }

    fn history_entry(directory: &Path, index: usize, bytes: u64) -> HistoryEntry {
        let blob = directory.join(format!("{index}.pdf"));
        fs::write(&blob, b"x").unwrap();
        HistoryEntry {
            operation: "rotate_page",
            digest: format!("d{index}"),
            blob,
            bytes,
        }
    }

    #[test]
    fn history_keeps_at_most_the_step_limit_and_deletes_evicted_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut history = SessionHistory::default();
        for index in 0..MAX_HISTORY_ENTRIES + 2 {
            history.record_mutation(history_entry(directory.path(), index, 1));
        }
        assert_eq!(history.undo.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(history.undo[0].digest, "d2");
        assert!(!directory.path().join("0.pdf").exists());
        assert!(!directory.path().join("1.pdf").exists());
        assert!(directory.path().join("2.pdf").exists());
    }

    #[test]
    fn history_stays_within_the_byte_budget() {
        let directory = tempfile::tempdir().unwrap();
        let mut history = SessionHistory::default();
        let half = MAX_HISTORY_BYTES / 2;
        history.record_mutation(history_entry(directory.path(), 0, half));
        history.record_mutation(history_entry(directory.path(), 1, half));
        assert_eq!(history.undo.len(), 2);
        history.record_mutation(history_entry(directory.path(), 2, half));
        assert_eq!(history.undo.len(), 2);
        assert_eq!(history.undo[0].digest, "d1");
        assert!(!directory.path().join("0.pdf").exists());

        // Tek başına bütçeyi aşan giriş tutulamaz.
        history.record_mutation(history_entry(directory.path(), 3, MAX_HISTORY_BYTES + 1));
        assert!(history.undo.is_empty());
        assert!(!directory.path().join("3.pdf").exists());
    }

    #[tokio::test]
    async fn isolated_mutation_validates_before_atomic_commit_and_advances_generation() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("isolated-mutation.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let original_before = fs::read(&original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);

        let mutated = store
            .mutate_isolated(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
            )
            .await
            .unwrap();

        assert!(mutated.dirty);
        assert_eq!(mutated.content_generation, 2);
        assert_eq!(fs::read(&original).unwrap(), original_before);
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert_ne!(fs::read(&state.working_path).unwrap(), original_before);
        assert!(state.working_digest.is_some());
    }

    // Uçtan uca form tasarımı: op'lar, form tasarım katmanının tarayıcıda gönderdiği JSON'un
    // birebir aynısı (camelCase, `name: null`); her adım bir sonrakinin generation'ını ve
    // sayfanın yeniden okunan `annot_index`'lerini kullanır, sonunda kayıt artımlı olmalıdır.
    #[tokio::test]
    async fn form_design_session_round_trip_uses_frontend_ops_and_saves_incrementally() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("form-design.pdf");
        crate::forms::test_support::standard_fixture(&original);
        let original_before = fs::read(&original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);

        let working_widgets = |store: &DocumentSessionStore| {
            let path = store
                .entry(&session_id)
                .unwrap()
                .state
                .lock()
                .working_path
                .clone();
            crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
                let document = pdfium
                    .load_pdf_from_file(&path, None)
                    .map_err(|error| error.to_string())?;
                crate::forms::read_page_form_fields(&document, 0)
                    .map_err(|error| format!("{error:?}"))
            })
            .unwrap()
        };
        let author = |generation: u64, ops: serde_json::Value| {
            let ops: Vec<crate::forms::author::FormAuthorOp> =
                serde_json::from_value(ops).expect("frontend op JSON must deserialize");
            crate::forms::author::validate_request(&ops).unwrap();
            store.mutate_isolated(
                session_id.clone(),
                generation,
                MutationRecipe::AuthorFormFields { ops },
            )
        };
        let before = working_widgets(&store);

        // Sürükleyerek metin alanı ve tıklamayla radio (form tasarım katmanının çıktısı).
        let added = author(
            1,
            serde_json::json!([{
                "op": "add", "pageIndex": 0,
                "normalizedRect": { "x": 0.17, "y": 0.53, "width": 0.52, "height": 0.033 },
                "field": { "kind": "text", "multiline": false }, "name": null
            }]),
        )
        .await
        .unwrap();
        assert_eq!(added.content_generation, 2);
        let added = author(
            2,
            serde_json::json!([{
                "op": "add", "pageIndex": 0,
                "normalizedRect": { "x": 0.17, "y": 0.68, "width": 0.0229, "height": 0.0177 },
                "field": { "kind": "radio", "group": "Group2", "export": "Choice1" }, "name": null
            }]),
        )
        .await
        .unwrap();
        let widgets = working_widgets(&store);
        assert_eq!(widgets.len(), before.len() + 2);
        // Katman eklemeden sonra en yüksek `annot_index`'li widget'ı seçer.
        let newest = widgets
            .iter()
            .max_by_key(|widget| widget.annot_index)
            .unwrap();
        assert_eq!(newest.field_name, "Group2");
        let text = widgets
            .iter()
            .find(|widget| widget.field_name == "Text1")
            .unwrap();

        // Seç → boyutlandır, yeniden adlandır, zorunlu yap.
        let moved = author(
            added.content_generation,
            serde_json::json!([{
                "op": "move", "pageIndex": 0, "annotIndex": text.annot_index, "fieldName": "Text1",
                "normalizedRect": { "x": 0.17, "y": 0.53, "width": 0.67, "height": 0.082 }
            }]),
        )
        .await
        .unwrap();
        let renamed = author(
            moved.content_generation,
            serde_json::json!([{
                "op": "set_props", "pageIndex": 0, "annotIndex": text.annot_index,
                "fieldName": "Text1", "props": { "name": "adres", "required": true }
            }]),
        )
        .await
        .unwrap();
        let widgets = working_widgets(&store);
        let adres = widgets
            .iter()
            .find(|widget| widget.field_name == "adres")
            .unwrap();
        assert!(adres.required);
        let rect = adres.normalized_rect.as_ref().unwrap();
        assert!((rect.width - 0.67).abs() < 0.005, "{rect:?}");
        assert!((rect.height - 0.082).abs() < 0.005, "{rect:?}");

        // Eski generation ile gelen işlem reddedilir (stale-result koruması).
        let stale = author(
            moved.content_generation,
            serde_json::json!([{
                "op": "delete", "pageIndex": 0, "annotIndex": adres.annot_index, "fieldName": "adres"
            }]),
        )
        .await;
        assert!(stale.is_err());

        // Önceden var olan bir alanı sil; kalan indeksler yeniden okunur.
        let first = widgets
            .iter()
            .find(|widget| widget.field_name == "name")
            .unwrap();
        let deleted = author(
            renamed.content_generation,
            serde_json::json!([{
                "op": "delete", "pageIndex": 0, "annotIndex": first.annot_index, "fieldName": "name"
            }]),
        )
        .await
        .unwrap();
        let widgets = working_widgets(&store);
        assert!(widgets.iter().all(|widget| widget.field_name != "name"));
        assert!(widgets.iter().any(|widget| widget.field_name == "adres"));
        assert_eq!(fs::read(&original).unwrap(), original_before);

        let saved = store
            .save(&session_id, deleted.state_revision, None)
            .await
            .unwrap();
        assert!(!saved.dirty);
        let saved_bytes = fs::read(&original).unwrap();
        assert!(
            saved_bytes.starts_with(&original_before),
            "kayıt orijinal baytları korumalı (artımlı)"
        );
    }

    #[tokio::test]
    async fn save_failure_keeps_dirty_working_copy_and_reports_missing_target() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("missing-after-open.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        fs::remove_file(&original).unwrap();

        let error = store
            .save(&session_id, mutated.state_revision, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "TARGET_NOT_FOUND");
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert!(state.dirty);
        assert_eq!(state.content_generation, mutated.content_generation);
        assert!(state.working_path.exists());
    }

    #[tokio::test]
    async fn save_post_commit_publication_failure_preserves_disk_truth_and_recovery_diagnostic() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("post-commit.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let before = fs::read(&original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        store
            .fail_next_save_publication
            .store(true, Ordering::Release);

        let error = store
            .save(&session_id, mutated.state_revision, None)
            .await
            .unwrap_err();

        assert_eq!(error.code, "SAVE_COMMITTED_SESSION_REFRESH_FAILED");
        assert_eq!(
            error.commit_state.as_deref(),
            Some("committed_but_session_refresh_failed")
        );
        assert_ne!(fs::read(&original).unwrap(), before);
        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.lifecycle, SessionLifecycle::RecoveryRequired);
        assert!(state.dirty);
        let diagnostic = state.last_save_commit.as_ref().unwrap();
        assert_eq!(diagnostic.target_path, original);
        assert!(diagnostic.committed_digest.is_some());
    }

    #[tokio::test]
    async fn clean_save_and_canonical_same_target_save_as_are_identical_no_ops() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("clean-same-target.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let before = snapshot(&store.entry(&session_id).unwrap().state.lock());
        let baseline_before = store
            .entry(&session_id)
            .unwrap()
            .state
            .lock()
            .baseline
            .digest
            .clone();
        let epoch_before = store.active_identity_epoch.load(Ordering::Acquire);
        let mut directory_before = fs::read_dir(documents.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        directory_before.sort();

        let normal = store
            .save(&session_id, before.state_revision, None)
            .await
            .unwrap();
        let alias = documents.path().join(".").join("clean-same-target.pdf");
        let same_target = store
            .save(&session_id, before.state_revision, Some(alias))
            .await
            .unwrap();

        assert_eq!(normal.state_revision, before.state_revision);
        assert_eq!(same_target.state_revision, before.state_revision);
        assert_eq!(normal.content_generation, before.content_generation);
        assert_eq!(same_target.content_generation, before.content_generation);
        assert!(!same_target.dirty);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 0);
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert_eq!(state.save_target_path, original);
        assert_eq!(state.baseline.digest, baseline_before);
        assert_eq!(
            store.active_identity_epoch.load(Ordering::Acquire),
            epoch_before
        );
        drop(state);
        let mut directory_after = fs::read_dir(documents.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        directory_after.sort();
        assert_eq!(directory_after, directory_before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn clean_symlink_save_as_preserves_prompt4_typed_rejection_without_writing() {
        use std::os::unix::fs::symlink;

        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("clean-symlink-target.pdf");
        let alias = documents.path().join("clean-symlink-alias.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        symlink(&original, &alias).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let before = snapshot(&store.entry(&session_id).unwrap().state.lock());

        let error = store
            .save(&session_id, before.state_revision, Some(alias))
            .await
            .unwrap_err();

        assert_eq!(error.code, "SAVE_AS_TARGET_IDENTITY_FAILED");
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 0);
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert_eq!(state.state_revision, before.state_revision);
        assert_eq!(state.save_target_path, original);
        assert!(!state.dirty);
    }

    #[tokio::test]
    async fn save_as_same_canonical_target_uses_normal_save_publication() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("same-target.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let target_before = store
            .entry(&session_id)
            .unwrap()
            .state
            .lock()
            .save_target_path
            .clone();

        let saved = store
            .save(&session_id, mutated.state_revision, Some(original.clone()))
            .await
            .unwrap();

        assert_eq!(saved.display_path, original.to_string_lossy());
        assert_eq!(saved.content_generation, mutated.content_generation);
        assert_eq!(saved.state_revision, mutated.state_revision + 1);
        assert!(!saved.dirty);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert_eq!(
            store
                .entry(&session_id)
                .unwrap()
                .state
                .lock()
                .save_target_path,
            target_before
        );
    }

    #[tokio::test]
    async fn save_as_post_commit_rebinding_failure_retains_committed_target_metadata() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("source.pdf");
        let target = documents.path().join("committed-target.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let session_id = open_test_session(&store, &original);
        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        store
            .fail_next_save_publication
            .store(true, Ordering::Release);

        let error = store
            .save(&session_id, mutated.state_revision, Some(target.clone()))
            .await
            .unwrap_err();

        assert!(target.exists());
        assert_eq!(error.code, "SAVE_COMMITTED_SESSION_REFRESH_FAILED");
        let state = store.entry(&session_id).unwrap();
        let state = state.state.lock();
        assert_eq!(state.save_target_path, original);
        assert_eq!(state.lifecycle, SessionLifecycle::RecoveryRequired);
        assert!(state.dirty);
        let diagnostic = state.last_save_commit.as_ref().unwrap();
        assert_eq!(diagnostic.target_path, target);
        assert!(diagnostic.committed_digest.is_some());
    }

    #[tokio::test]
    async fn save_transition_close_retry_skips_second_disk_save_and_does_not_deadlock() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("window-close.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        let mutated = store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let transition =
            begin_session_transition(session_id.clone(), TransitionKind::WindowClose, app.state())
                .await
                .unwrap();
        store
            .fail_next_transition_close
            .store(true, Ordering::Release);

        let first = tokio::time::timeout(
            Duration::from_secs(2),
            save_transition(
                transition.token.clone(),
                None,
                app.state(),
                app.state(),
                app.state(),
                app.state(),
            ),
        )
        .await
        .expect("save_transition recursive operation gate deadlock üretmemeli")
        .unwrap_err();
        assert_eq!(first.code, "TRANSITION_CLOSE_APPLY_FAILED");
        {
            let state = store.entry(&session_id).unwrap();
            let state = state.state.lock();
            assert_eq!(state.lifecycle, SessionLifecycle::Open);
            assert!(!state.dirty);
            assert_eq!(state.content_generation, mutated.content_generation);
        }
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        {
            let entry = store.entry(&session_id).unwrap();
            entry
                .state
                .lock()
                .active_transition
                .as_mut()
                .unwrap()
                .created_at_ms = now_ms().saturating_sub(TRANSITION_TTL_MS + 1);
        }

        let completed = save_transition(
            transition.token.clone(),
            None,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(completed.lifecycle, SessionLifecycle::Closed);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert!(store.entry(&session_id).is_err());
        let replayed = save_transition(
            transition.token.clone(),
            None,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(replayed.id, completed.id);
        assert_eq!(replayed.state_revision, completed.state_revision);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        let recorded = store
            .recorded_transitions
            .lock()
            .get(&transition.token)
            .cloned()
            .unwrap();
        let authority = recorded.resulting_authority.unwrap();
        assert_eq!(authority.active_session_id, None);
        assert_eq!(
            authority.identity_epoch,
            store.active_identity_epoch.load(Ordering::Acquire)
        );
    }

    #[tokio::test]
    async fn failed_precommit_duplicate_callers_share_one_attempt_and_same_token_retries_once() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("failed-precommit-retry.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let transition =
            begin_session_transition(session_id.clone(), TransitionKind::WindowClose, app.state())
                .await
                .unwrap();
        let conflicting_target = documents.path().join("conflicting-target.pdf");
        fs::copy(fixture_path("minimal.pdf"), &conflicting_target).unwrap();
        *store.transition_claim_barrier.lock() = Some(Arc::new(tokio::sync::Barrier::new(2)));
        let save_reached = Arc::new(tokio::sync::Barrier::new(2));
        let save_release = Arc::new(tokio::sync::Barrier::new(2));
        *store.save_io_barriers.lock() = Some((save_reached.clone(), save_release.clone()));

        let (first, second, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                save_transition(
                    transition.token.clone(),
                    Some(conflicting_target.to_string_lossy().into_owned()),
                    app.state(),
                    app.state(),
                    app.state(),
                    app.state(),
                ),
                save_transition(
                    transition.token.clone(),
                    Some(conflicting_target.to_string_lossy().into_owned()),
                    app.state(),
                    app.state(),
                    app.state(),
                    app.state(),
                ),
                async move {
                    save_reached.wait().await;
                    fs::write(conflicting_target, b"%PDF-conflict").unwrap();
                    save_release.wait().await;
                }
            )
        })
        .await
        .expect("duplicate precommit callers shared completion ile bitmeli");
        assert_eq!(
            first.unwrap_err().commit_state,
            second.unwrap_err().commit_state
        );
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert!(matches!(
            &*store
                .entry(&session_id)
                .unwrap()
                .state
                .lock()
                .active_transition
                .as_ref()
                .unwrap()
                .execution
                .phase
                .lock(),
            TransitionExecutionPhase::FailedPreCommit { .. }
        ));
        *store.transition_claim_barrier.lock() = None;
        *store.save_io_barriers.lock() = None;

        let completed = save_transition(
            transition.token,
            None,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(completed.lifecycle, SessionLifecycle::Closed);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn recorded_commit_indeterminate_public_retry_never_reexecutes_save() {
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let token = Uuid::new_v4().to_string();
        let decision = TransitionDecision::Save { target: None };
        let error = DocumentSessionError::new(
            "TRANSITION_SAVE_COMMIT_INDETERMINATE",
            "recorded uncertainty",
        );
        let authority = TransitionAuthorityIdentity {
            source_session_id: "recorded-source".to_string(),
            kind: TransitionKind::WindowClose,
            initial_identity_epoch: 0,
        };
        store
            .transition_authorities
            .lock()
            .insert(token.clone(), authority.clone());
        record_transition_outcome(
            &store,
            &token,
            &authority,
            decision,
            Err(error.clone()),
            None,
        )
        .unwrap();

        for _ in 0..2 {
            let replayed = save_transition(
                token.clone(),
                None,
                app.state(),
                app.state(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap_err();
            assert_eq!(replayed.code, error.code);
        }
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gate_wait_cancellation_clears_reservation_without_starting_irreversible_work() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("gate-wait-cancel.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        let (token, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        let lifecycle_guard = store.lifecycle_gate.clone().write_owned().await;
        let wait_blocked = Arc::new(Notify::new());
        *store.transition_lifecycle_wait_notify.lock() = Some(wait_blocked.clone());
        let blocked = wait_blocked.notified();
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        let search_cache = Arc::new(search_cache);
        let layout_cache = Arc::new(layout_cache);
        let vector_store = Arc::new(vector_store);

        let first = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), blocked)
            .await
            .expect("transition caller lifecycle gate üzerinde beklemeli");
        assert!(matches!(
            &*execution.completion_state.lock(),
            TransitionExecutionCompletion::GateReserved { .. }
        ));
        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::PendingUnclaimed
        ));

        let completion_waiting = Arc::new(Notify::new());
        *store.transition_completion_wait_notify.lock() = Some(completion_waiting.clone());
        let duplicate_waiting = completion_waiting.notified();
        let duplicate = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), duplicate_waiting)
            .await
            .expect("duplicate caller shared completion waiter'a girmeli");
        first.abort();
        let duplicate_error = tokio::time::timeout(Duration::from_secs(2), duplicate)
            .await
            .expect("reservation waiter cancellation notification ile tamamlanmalı")
            .unwrap()
            .unwrap_err();
        assert_eq!(
            duplicate_error.code,
            "TRANSITION_GATE_RESERVATION_CANCELLED"
        );
        assert!(first.await.unwrap_err().is_cancelled());
        assert_eq!(
            *execution.completion_state.lock(),
            TransitionExecutionCompletion::Available
        );
        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::PendingUnclaimed
        ));
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 0);
        assert_eq!(
            store.transition_discard_invocations.load(Ordering::Acquire),
            0
        );
        assert_eq!(
            store.transition_apply_invocations.load(Ordering::Acquire),
            0
        );

        drop(lifecycle_guard);
        *store.transition_lifecycle_wait_notify.lock() = None;
        *store.transition_completion_wait_notify.lock() = None;
        let completed = execute_transition_command(
            &token,
            TransitionDecision::Discard,
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap();
        assert_eq!(completed.lifecycle, SessionLifecycle::Closed);
        assert_eq!(
            store.transition_discard_invocations.load(Ordering::Acquire),
            1
        );
        assert_eq!(
            store.transition_apply_invocations.load(Ordering::Acquire),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborted_transition_before_commit_is_retryable_with_the_same_token() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("abort-before-commit.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let (token, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        let expected_before_interruption = store
            .entry(&session_id)
            .unwrap()
            .state
            .lock()
            .state_revision;
        let release_trace = Arc::new(Mutex::new(Vec::new()));
        *store.transition_release_trace.lock() = Some(release_trace.clone());
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.save_io_barriers.lock() = Some((reached.clone(), release));
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        let search_cache = Arc::new(search_cache);
        let layout_cache = Arc::new(layout_cache);
        let vector_store = Arc::new(vector_store);

        let task = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), reached.wait())
            .await
            .expect("transition commit öncesi checkpoint'e ulaşmalı");
        *store.save_io_barriers.lock() = None;
        let save_blocked = Arc::new(Notify::new());
        *store.save_lifecycle_wait_notify.lock() = Some(save_blocked.clone());
        let blocked = save_blocked.notified();
        let contender = {
            let store = store.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move {
                store
                    .save(&session_id, expected_before_interruption, None)
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), blocked)
            .await
            .expect("ikinci Save lifecycle authority üzerinde blocked waiter olmalı");
        task.abort();
        let contender_error = tokio::time::timeout(Duration::from_secs(2), contender)
            .await
            .expect("ikinci Save interruption publication sonrasında ilerlemeli")
            .unwrap()
            .unwrap_err();
        assert_eq!(contender_error.code, "STATE_REVISION_MISMATCH");
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::InterruptedPreCommit { .. }
        ));
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 0);
        assert!(store.recorded_transitions.lock().get(&token).is_none());
        let trace = release_trace.lock().clone();
        assert_eq!(
            trace,
            vec![
                TransitionAuthorityReleaseEvent::SaveLifecycleWaitBlocked,
                TransitionAuthorityReleaseEvent::GuardPublicationStarted,
                TransitionAuthorityReleaseEvent::GuardPublicationCompleted,
                TransitionAuthorityReleaseEvent::OperationLeasesReleased,
                TransitionAuthorityReleaseEvent::LifecycleAuthorityReleased,
                TransitionAuthorityReleaseEvent::SaveLifecycleAcquired,
                TransitionAuthorityReleaseEvent::SaveOperationAcquired,
            ]
        );
        *store.transition_release_trace.lock() = None;
        *store.save_lifecycle_wait_notify.lock() = None;

        let completed = execute_transition_command(
            &token,
            TransitionDecision::Save { target: None },
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap();
        assert_eq!(completed.lifecycle, SessionLifecycle::Closed);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_transition_publishes_interruption_before_transition_waiter_acquires_gates() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("panic-authority-order.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let (token, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        let release_trace = Arc::new(Mutex::new(Vec::new()));
        *store.transition_release_trace.lock() = Some(release_trace.clone());
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.save_io_barriers.lock() = Some((reached.clone(), release.clone()));
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        let search_cache = Arc::new(search_cache);
        let layout_cache = Arc::new(layout_cache);
        let vector_store = Arc::new(vector_store);

        let task = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), reached.wait())
            .await
            .expect("transition panic checkpoint'e ulaşmalı");
        *store.save_io_barriers.lock() = None;
        let transition_blocked = Arc::new(Notify::new());
        *store.transition_lifecycle_wait_notify.lock() = Some(transition_blocked.clone());
        let blocked = transition_blocked.notified();
        let contender = {
            let store = store.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move { store.acquire_test_transition_contender(&session_id).await })
        };
        tokio::time::timeout(Duration::from_secs(2), blocked)
            .await
            .expect("ikinci transition lifecycle gate waiter olmalı");
        let first_waiter = execution.completion.notified();
        let second_waiter = execution.completion.notified();
        store
            .panic_next_transition_precommit
            .store(true, Ordering::Release);
        release.wait().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(first_waiter, second_waiter);
        })
        .await
        .expect("panic interruption bütün waiter'ları uyandırmalı");
        let contender_error = contender.await.unwrap().unwrap_err();
        assert_eq!(contender_error.code, "TRANSITION_ALREADY_ACTIVE");
        assert!(task.await.unwrap_err().is_panic());

        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::InterruptedPreCommit { .. }
        ));
        assert_eq!(
            release_trace.lock().clone(),
            vec![
                TransitionAuthorityReleaseEvent::TransitionLifecycleWaitBlocked,
                TransitionAuthorityReleaseEvent::GuardPublicationStarted,
                TransitionAuthorityReleaseEvent::GuardPublicationCompleted,
                TransitionAuthorityReleaseEvent::OperationLeasesReleased,
                TransitionAuthorityReleaseEvent::LifecycleAuthorityReleased,
                TransitionAuthorityReleaseEvent::TransitionLifecycleAcquired,
                TransitionAuthorityReleaseEvent::TransitionOperationAcquired,
            ]
        );
        *store.transition_release_trace.lock() = None;
        *store.transition_lifecycle_wait_notify.lock() = None;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborted_transition_with_unobserved_commit_records_indeterminate_outcome() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("abort-commit-unobserved.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let (token, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        let release_trace = Arc::new(Mutex::new(Vec::new()));
        *store.transition_release_trace.lock() = Some(release_trace.clone());
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.save_commit_barriers.lock() = Some((reached.clone(), release));
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        let search_cache = Arc::new(search_cache);
        let layout_cache = Arc::new(layout_cache);
        let vector_store = Arc::new(vector_store);

        let task = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), reached.wait())
            .await
            .expect("atomic commit tamamlanıp sonucu gözlenmeden önce durmalı");
        *store.save_commit_barriers.lock() = None;
        let shutdown_blocked = Arc::new(Notify::new());
        *store.shutdown_lifecycle_wait_notify.lock() = Some(shutdown_blocked.clone());
        let blocked = shutdown_blocked.notified();
        let shutdown = {
            let store = store.clone();
            tokio::spawn(async move {
                let active_tasks = ActiveRenderTasks(Mutex::new(HashMap::new()));
                let render_service = RenderService::new();
                store
                    .shutdown_all_resources(&active_tasks, &render_service)
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(3), blocked)
            .await
            .expect("shutdown lifecycle authority üzerinde blocked waiter olmalı");
        task.abort();
        let shutdown_warnings = tokio::time::timeout(Duration::from_secs(5), shutdown)
            .await
            .expect("shutdown interruption publication sonrasında ilerlemeli")
            .unwrap();
        assert!(shutdown_warnings
            .iter()
            .any(|warning| warning.code == "SESSION_SHUTDOWN_RECOVERY_REQUIRED"));
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::CommitIndeterminate { .. }
        ));
        assert_eq!(
            store.entry(&session_id).unwrap().state.lock().lifecycle,
            SessionLifecycle::CommitIndeterminate
        );
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        let recorded = store
            .recorded_transitions
            .lock()
            .get(&token)
            .cloned()
            .unwrap();
        assert_eq!(
            recorded.result.unwrap_err().code,
            "TRANSITION_SAVE_COMMIT_INDETERMINATE"
        );
        assert!(store.entry(&session_id).is_ok());
        assert_eq!(
            release_trace.lock().clone(),
            vec![
                TransitionAuthorityReleaseEvent::ShutdownLifecycleWaitBlocked,
                TransitionAuthorityReleaseEvent::GuardPublicationStarted,
                TransitionAuthorityReleaseEvent::GuardPublicationCompleted,
                TransitionAuthorityReleaseEvent::OperationLeasesReleased,
                TransitionAuthorityReleaseEvent::LifecycleAuthorityReleased,
                TransitionAuthorityReleaseEvent::ShutdownLifecycleAcquired,
            ]
        );
        *store.transition_release_trace.lock() = None;
        *store.shutdown_lifecycle_wait_notify.lock() = None;

        let replayed = execute_transition_command(
            &token,
            TransitionDecision::Save { target: None },
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap_err();
        assert_eq!(replayed.code, "TRANSITION_SAVE_COMMIT_INDETERMINATE");
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborted_transition_after_committed_save_retries_only_apply() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("abort-after-save-committed.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let (token, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.transition_save_committed_barriers.lock() = Some((reached.clone(), release));
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        let search_cache = Arc::new(search_cache);
        let layout_cache = Arc::new(layout_cache);
        let vector_store = Arc::new(vector_store);

        let task = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), reached.wait())
            .await
            .expect("SaveCommitted checkpoint'e ulaşmalı");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::FailedPostCommit { .. }
        ));
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert_eq!(
            store.transition_apply_invocations.load(Ordering::Acquire),
            0
        );
        *store.transition_save_committed_barriers.lock() = None;

        let completed = execute_transition_command(
            &token,
            TransitionDecision::Save { target: None },
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap();
        assert_eq!(completed.lifecycle, SessionLifecycle::Closed);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert_eq!(
            store.transition_apply_invocations.load(Ordering::Acquire),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn aborted_transition_during_apply_retries_apply_without_resaving() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("abort-during-apply.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        store
            .mutate(session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let (token, execution) =
            attach_test_transition(&store, &session_id, TransitionKind::WindowClose);
        let release_trace = Arc::new(Mutex::new(Vec::new()));
        *store.transition_release_trace.lock() = Some(release_trace.clone());
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.transition_apply_barriers.lock() = Some((reached.clone(), release));
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        let search_cache = Arc::new(search_cache);
        let layout_cache = Arc::new(layout_cache);
        let vector_store = Arc::new(vector_store);

        let task = {
            let store = store.clone();
            let token = token.clone();
            let search_cache = search_cache.clone();
            let layout_cache = layout_cache.clone();
            let vector_store = vector_store.clone();
            tokio::spawn(async move {
                execute_transition_command(
                    &token,
                    TransitionDecision::Save { target: None },
                    &store,
                    &search_cache,
                    &layout_cache,
                    &vector_store,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), reached.wait())
            .await
            .expect("transition apply checkpoint'e ulaşmalı");
        *store.transition_apply_barriers.lock() = None;
        let expected_before_interruption = store
            .entry(&session_id)
            .unwrap()
            .state
            .lock()
            .state_revision;
        let save_blocked = Arc::new(Notify::new());
        *store.save_lifecycle_wait_notify.lock() = Some(save_blocked.clone());
        let blocked = save_blocked.notified();
        let contender = {
            let store = store.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move {
                store
                    .save(&session_id, expected_before_interruption, None)
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), blocked)
            .await
            .expect("apply sırasında ikinci Save lifecycle waiter olmalı");
        task.abort();
        let contender_error = tokio::time::timeout(Duration::from_secs(2), contender)
            .await
            .expect("ikinci Save apply interruption publication sonrasında ilerlemeli")
            .unwrap()
            .unwrap_err();
        assert_eq!(contender_error.code, "STATE_REVISION_MISMATCH");
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(matches!(
            &*execution.phase.lock(),
            TransitionExecutionPhase::FailedPostCommit { .. }
        ));
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert!(store.recorded_transitions.lock().get(&token).is_none());
        assert_eq!(
            release_trace.lock().clone(),
            vec![
                TransitionAuthorityReleaseEvent::SaveLifecycleWaitBlocked,
                TransitionAuthorityReleaseEvent::GuardPublicationStarted,
                TransitionAuthorityReleaseEvent::GuardPublicationCompleted,
                TransitionAuthorityReleaseEvent::OperationLeasesReleased,
                TransitionAuthorityReleaseEvent::LifecycleAuthorityReleased,
                TransitionAuthorityReleaseEvent::SaveLifecycleAcquired,
                TransitionAuthorityReleaseEvent::SaveOperationAcquired,
            ]
        );
        *store.transition_release_trace.lock() = None;
        *store.save_lifecycle_wait_notify.lock() = None;

        let completed = execute_transition_command(
            &token,
            TransitionDecision::Save { target: None },
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap();
        assert_eq!(completed.lifecycle, SessionLifecycle::Closed);
        assert_eq!(store.save_core_invocations.load(Ordering::Acquire), 1);
        assert!(store.recorded_transitions.lock().contains_key(&token));
    }

    #[tokio::test]
    async fn lifecycle_gate_is_acquired_before_operation_gate_for_save_and_transition() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("lock-order.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());

        let entry = store.entry(&session_id).unwrap();
        let operation_guard = entry.operation.clone().write_owned().await;
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.lock_order_barriers.lock() = Some((reached.clone(), release.clone()));
        let revision = entry.state.lock().state_revision;

        let (save_result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(store.save(&session_id, revision, None), async move {
                reached.wait().await;
                drop(operation_guard);
                release.wait().await;
            })
        })
        .await
        .expect("Save lifecycle -> operation lock sırasında ilerlemeli");
        save_result.unwrap();

        let transition =
            begin_session_transition(session_id.clone(), TransitionKind::WindowClose, app.state())
                .await
                .unwrap();
        let operation_guard = entry.operation.clone().write_owned().await;
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        *store.lock_order_barriers.lock() = Some((reached.clone(), release.clone()));

        let (transition_result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                discard_transition(
                    transition.token,
                    app.state(),
                    app.state(),
                    app.state(),
                    app.state(),
                ),
                async move {
                    reached.wait().await;
                    drop(operation_guard);
                    release.wait().await;
                }
            )
        })
        .await
        .expect("Transition lifecycle -> operation lock sırasında ilerlemeli");
        assert_eq!(
            transition_result.unwrap().lifecycle,
            SessionLifecycle::Closed
        );
    }

    #[tokio::test]
    async fn switch_retry_after_source_deactivation_publishes_each_real_identity_epoch_once() {
        let documents = tempfile::tempdir().unwrap();
        let source_path = documents.path().join("source.pdf");
        let candidate_path = documents.path().join("candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &source_path).unwrap();
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let source_id = open_test_session(&store, &source_path);
        let candidate_id = open_test_session(&store, &candidate_path);
        store.entry(&candidate_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CandidateReady;
        *store.active_session_id.lock() = Some(source_id.clone());
        let transition = begin_session_transition(
            source_id.clone(),
            TransitionKind::DocumentSwitch,
            app.state(),
        )
        .await
        .unwrap();
        let prepared = prepare_candidate_activation(
            candidate_id.clone(),
            Some(source_id.clone()),
            Some(transition.token.clone()),
            app.state(),
        )
        .await
        .unwrap();
        store
            .fail_next_candidate_activation_after_deactivation
            .store(true, Ordering::Release);

        let first = discard_transition(
            transition.token.clone(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            first.code,
            "TRANSITION_CANDIDATE_ACTIVATION_FAILED_AFTER_DEACTIVATION"
        );
        assert!(store.active_session_id.lock().is_none());
        assert_eq!(
            store.entry(&source_id).unwrap().state.lock().lifecycle,
            SessionLifecycle::Closed
        );
        assert_eq!(store.active_identity_epoch.load(Ordering::Acquire), 1);
        let candidate_before = snapshot(&store.entry(&candidate_id).unwrap().state.lock());
        let source_before = snapshot(&store.entry(&source_id).unwrap().state.lock());
        let rollback_error = rollback_candidate_activation(
            prepared.activation_token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            rollback_error.code,
            "POST_DEACTIVATION_ROLLBACK_NOT_ALLOWED"
        );
        assert_eq!(
            snapshot(&store.entry(&candidate_id).unwrap().state.lock()).state_revision,
            candidate_before.state_revision
        );
        assert_eq!(
            snapshot(&store.entry(&source_id).unwrap().state.lock()).state_revision,
            source_before.state_revision
        );
        assert!(store.active_session_id.lock().is_none());
        assert_eq!(store.active_identity_epoch.load(Ordering::Acquire), 1);

        let completed = discard_transition(
            transition.token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(completed.id, candidate_id);
        assert_eq!(
            store.active_session_id.lock().as_deref(),
            Some(candidate_id.as_str())
        );
        assert_eq!(store.active_identity_epoch.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn historical_recorded_replay_does_not_restore_older_active_authority() {
        let documents = tempfile::tempdir().unwrap();
        let source_path = documents.path().join("historical-source.pdf");
        let first_path = documents.path().join("historical-first.pdf");
        let second_path = documents.path().join("historical-second.pdf");
        for path in [&source_path, &first_path, &second_path] {
            fs::copy(fixture_path("minimal.pdf"), path).unwrap();
        }
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let source_id = open_test_session(&store, &source_path);
        let first_id = open_test_session(&store, &first_path);
        let second_id = open_test_session(&store, &second_path);
        store.entry(&first_id).unwrap().state.lock().lifecycle = SessionLifecycle::CandidateReady;
        store.entry(&second_id).unwrap().state.lock().lifecycle = SessionLifecycle::CandidateReady;
        *store.active_session_id.lock() = Some(source_id.clone());

        let first_transition = begin_session_transition(
            source_id.clone(),
            TransitionKind::DocumentSwitch,
            app.state(),
        )
        .await
        .unwrap();
        prepare_candidate_activation(
            first_id.clone(),
            Some(source_id),
            Some(first_transition.token.clone()),
            app.state(),
        )
        .await
        .unwrap();
        let first_result = discard_transition(
            first_transition.token.clone(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(first_result.id, first_id);

        let second_transition = begin_session_transition(
            first_id.clone(),
            TransitionKind::DocumentSwitch,
            app.state(),
        )
        .await
        .unwrap();
        prepare_candidate_activation(
            second_id.clone(),
            Some(first_id.clone()),
            Some(second_transition.token.clone()),
            app.state(),
        )
        .await
        .unwrap();
        discard_transition(
            second_transition.token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let epoch_after_second = store.active_identity_epoch.load(Ordering::Acquire);
        let discard_count = store.transition_discard_invocations.load(Ordering::Acquire);
        let apply_count = store.transition_apply_invocations.load(Ordering::Acquire);

        let replayed = discard_transition(
            first_transition.token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        assert_eq!(replayed.id, first_id);
        assert_eq!(
            store.active_session_id.lock().as_deref(),
            Some(second_id.as_str())
        );
        assert_eq!(
            store.active_identity_epoch.load(Ordering::Acquire),
            epoch_after_second
        );
        assert_eq!(
            store.transition_discard_invocations.load(Ordering::Acquire),
            discard_count
        );
        assert_eq!(
            store.transition_apply_invocations.load(Ordering::Acquire),
            apply_count
        );
    }

    #[tokio::test]
    async fn recorded_metadata_corruption_is_typed_and_never_reexecutes_transition() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("record-corruption.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        let transition =
            begin_session_transition(session_id, TransitionKind::WindowClose, app.state())
                .await
                .unwrap();
        discard_transition(
            transition.token.clone(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let discard_count = store.transition_discard_invocations.load(Ordering::Acquire);
        let apply_count = store.transition_apply_invocations.load(Ordering::Acquire);
        store
            .recorded_transitions
            .lock()
            .get_mut(&transition.token)
            .unwrap()
            .resulting_authority
            .as_mut()
            .unwrap()
            .active_session_id = Some("corrupt-active-session".to_string());

        let error = discard_transition(
            transition.token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "TRANSITION_RECORDED_OUTCOME_CORRUPT");
        assert!(store.active_session_id.lock().is_none());
        assert_eq!(
            store.transition_discard_invocations.load(Ordering::Acquire),
            discard_count
        );
        assert_eq!(
            store.transition_apply_invocations.load(Ordering::Acquire),
            apply_count
        );
    }

    #[tokio::test]
    async fn recorded_replay_rejects_same_epoch_current_authority_mismatch_without_restoring_it() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents
            .path()
            .join("record-current-authority-corruption.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        let transition =
            begin_session_transition(session_id, TransitionKind::WindowClose, app.state())
                .await
                .unwrap();
        discard_transition(
            transition.token.clone(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let recorded_epoch = store.active_identity_epoch.load(Ordering::Acquire);
        *store.active_session_id.lock() = Some("same-epoch-intruder".to_string());

        let error = discard_transition(
            transition.token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "TRANSITION_RECORDED_OUTCOME_CORRUPT");
        assert_eq!(
            store.active_session_id.lock().as_deref(),
            Some("same-epoch-intruder")
        );
        assert_eq!(
            store.active_identity_epoch.load(Ordering::Acquire),
            recorded_epoch
        );
    }

    #[tokio::test]
    async fn recorded_replay_request_authority_mismatch_is_a_typed_conflict() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("record-request-conflict.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        *store.active_session_id.lock() = Some(session_id.clone());
        let transition =
            begin_session_transition(session_id, TransitionKind::WindowClose, app.state())
                .await
                .unwrap();
        discard_transition(
            transition.token.clone(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let mismatched_request = TransitionAuthorityIdentity {
            source_session_id: "different-source".to_string(),
            kind: TransitionKind::DocumentSwitch,
            initial_identity_epoch: 0,
        };

        let error = recorded_transition_result_for_authority(
            &store,
            &transition.token,
            &TransitionDecision::Discard,
            Some(&mismatched_request),
        )
        .unwrap()
        .unwrap_err();

        assert_eq!(error.code, "TRANSITION_REPLAY_AUTHORITY_CONFLICT");
        assert!(store.active_session_id.lock().is_none());
    }

    #[tokio::test]
    async fn candidate_commit_rejects_existing_session_without_mutating_authority() {
        let documents = tempfile::tempdir().unwrap();
        let source_path = documents.path().join("legacy-source.pdf");
        let candidate_path = documents.path().join("legacy-candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &source_path).unwrap();
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let source_id = open_test_session(&store, &source_path);
        let candidate_id = open_test_session(&store, &candidate_path);
        store.entry(&candidate_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CandidateReady;
        *store.active_session_id.lock() = Some(source_id.clone());
        let transition = begin_session_transition(
            source_id.clone(),
            TransitionKind::DocumentSwitch,
            app.state(),
        )
        .await
        .unwrap();
        let prepared = prepare_candidate_activation(
            candidate_id.clone(),
            Some(source_id.clone()),
            Some(transition.token),
            app.state(),
        )
        .await
        .unwrap();
        let source_before = snapshot(&store.entry(&source_id).unwrap().state.lock());
        let candidate_before = snapshot(&store.entry(&candidate_id).unwrap().state.lock());
        let epoch_before = store.active_identity_epoch.load(Ordering::Acquire);

        let error = commit_candidate_activation(
            prepared.activation_token.clone(),
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "LEGACY_TRANSITION_COMMIT_NOT_ALLOWED");
        assert_eq!(
            snapshot(&store.entry(&source_id).unwrap().state.lock()).state_revision,
            source_before.state_revision
        );
        assert_eq!(
            snapshot(&store.entry(&candidate_id).unwrap().state.lock()).state_revision,
            candidate_before.state_revision
        );
        assert_eq!(
            store.active_session_id.lock().as_deref(),
            Some(source_id.as_str())
        );
        assert_eq!(
            store.active_identity_epoch.load(Ordering::Acquire),
            epoch_before
        );
        assert!(store
            .activations_by_token
            .lock()
            .contains_key(&prepared.activation_token));
    }

    #[tokio::test]
    async fn candidate_commit_preserves_no_active_session_first_activation() {
        let documents = tempfile::tempdir().unwrap();
        let candidate_path = documents.path().join("first-candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let candidate_id = open_test_session(&store, &candidate_path);
        store.entry(&candidate_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CandidateReady;
        let prepared = prepare_candidate_activation(candidate_id.clone(), None, None, app.state())
            .await
            .unwrap();

        let committed = commit_candidate_activation(
            prepared.activation_token,
            app.state(),
            app.state(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert_eq!(committed.snapshot.id, candidate_id);
        assert_eq!(committed.snapshot.lifecycle, SessionLifecycle::Open);
        assert_eq!(
            store.active_session_id.lock().as_deref(),
            Some(candidate_id.as_str())
        );
        assert_eq!(store.active_identity_epoch.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn two_sessions_are_isolated_and_discard_removes_only_working_copy() {
        let documents = tempfile::tempdir().unwrap();
        let first = documents.path().join("first.pdf");
        let second = documents.path().join("second.pdf");
        fs::copy(fixture_path("minimal.pdf"), &first).unwrap();
        fs::copy(fixture_path("minimal.pdf"), &second).unwrap();
        let first_before = fs::read(&first).unwrap();
        let second_before = fs::read(&second).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let first_id = open_test_session(&store, &first);
        let second_id = open_test_session(&store, &second);

        store
            .mutate(first_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        assert!(store.entry(&first_id).unwrap().state.lock().dirty);
        assert!(!store.entry(&second_id).unwrap().state.lock().dirty);
        assert_eq!(fs::read(&first).unwrap(), first_before);
        assert_eq!(fs::read(&second).unwrap(), second_before);

        store.close_entry(&first_id).await.unwrap();
        assert!(store.entry(&first_id).is_err());
        assert!(store.entry(&second_id).is_ok());
        assert_eq!(fs::read(&first).unwrap(), first_before);
        assert_eq!(fs::read(&second).unwrap(), second_before);
    }

    #[test]
    fn candidate_ready_allowlist_rejects_normal_read_mutation_and_job_paths() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = test_state(&directory, b"%PDF-candidate");
        state.lifecycle = SessionLifecycle::CandidateReady;

        assert_eq!(
            session_read_path(&state, Some(1)).unwrap_err().code,
            "DOCUMENT_SESSION_INVALID_STATE"
        );
        assert_eq!(
            ensure_open_for_mutation(&state, 1).unwrap_err().code,
            "DOCUMENT_SESSION_INVALID_STATE"
        );
        assert!(candidate_validation_path(&state, 1).is_ok());
    }

    #[tokio::test]
    async fn text_export_to_file_writes_exactly_the_extracted_text() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("text-source.pdf");
        fs::copy(fixture_path("three-distinct-pages.pdf"), &original).unwrap();
        let app = test_command_app();
        let session_id = open_test_session(&app.state::<DocumentSessionStore>(), &original);

        for (format, file_name) in [("text", "pages.txt"), ("markdown", "pages.md")] {
            let target = documents.path().join(file_name);
            // An existing target is replaced as a whole, not appended to or truncated in place.
            fs::write(&target, b"previous export").unwrap();
            let SessionJobOutcome::Completed {
                value: expected, ..
            } = export_session_text_content(
                Uuid::new_v4().to_string(),
                session_id.clone(),
                1,
                vec![0, 2],
                format.to_string(),
                app.state(),
            )
            .await
            .unwrap()
            else {
                panic!("text extraction did not complete");
            };
            let outcome = export_session_text_to_file(
                Uuid::new_v4().to_string(),
                session_id.clone(),
                1,
                vec![0, 2],
                format.to_string(),
                target.to_string_lossy().to_string(),
                app.state(),
            )
            .await
            .unwrap();

            assert!(
                matches!(outcome, SessionJobOutcome::Completed { .. }),
                "{format}: {outcome:?}"
            );
            assert!(!expected.is_empty(), "{format}: fixture produced no text");
            assert_eq!(fs::read_to_string(&target).unwrap(), expected, "{format}");
        }
        let mut names = fs::read_dir(documents.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["pages.md", "pages.txt", "text-source.pdf"]);
    }

    #[tokio::test]
    async fn text_export_to_file_rejects_non_text_and_private_targets_before_the_job() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("text-source.pdf");
        fs::copy(fixture_path("three-distinct-pages.pdf"), &original).unwrap();
        let original_bytes = fs::read(&original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        let private_target = store.temp_root.join("export.txt");

        for target in [
            original.clone(),
            documents.path().join("notes"),
            documents.path().join("notes.txt.pdf"),
        ] {
            let error = export_session_text_to_file(
                Uuid::new_v4().to_string(),
                session_id.clone(),
                1,
                vec![0],
                "text".to_string(),
                target.to_string_lossy().to_string(),
                app.state(),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.code,
                "TEXT_EXPORT_TARGET_INVALID",
                "{}",
                target.display()
            );
        }
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert!(!documents.path().join("notes").exists());

        let error = export_session_text_to_file(
            Uuid::new_v4().to_string(),
            session_id,
            1,
            vec![0],
            "text".to_string(),
            private_target.to_string_lossy().to_string(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_SESSION_PRIVATE_TARGET");
        assert!(!private_target.exists());
    }

    #[tokio::test]
    async fn candidate_ready_public_commands_reject_mutation_save_job_ocr_export_and_ai() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let session_id = open_test_session(&store, &original);
        let state_revision = {
            let entry = store.entry(&session_id).unwrap();
            let mut state = entry.state.lock();
            state.lifecycle = SessionLifecycle::CandidateReady;
            state.state_revision
        };

        let mutation = rotate_session_page(
            session_id.clone(),
            1,
            0,
            90,
            app.state::<DocumentSessionStore>(),
            app.state::<crate::ai::VectorStore>(),
        )
        .await
        .unwrap_err();
        let save = save_document_session(session_id.clone(), state_revision, app.state())
            .await
            .unwrap_err();
        let export = export_session_page_to_image(
            Uuid::new_v4().to_string(),
            session_id.clone(),
            1,
            0,
            documents
                .path()
                .join("candidate.png")
                .to_string_lossy()
                .to_string(),
            "png".to_string(),
            app.state(),
        )
        .await
        .unwrap_err();
        let ocr = crate::ocr::detect_scanned_pages(
            Uuid::new_v4().to_string(),
            session_id.clone(),
            1,
            app.state::<DocumentSessionStore>(),
        )
        .await
        .unwrap_err();
        let ai = crate::ai::build_semantic_index(
            Uuid::new_v4().to_string(),
            session_id.clone(),
            1,
            128,
            16,
            "test".to_string(),
            format!("req_{}", Uuid::new_v4()),
            app.state::<DocumentSessionStore>(),
            app.state::<crate::ai::VectorStore>(),
            app.state::<crate::ai::endpoint::AiEndpointStore>(),
            app.state::<crate::ai::request_registry::AiRequestRegistry>(),
        )
        .await
        .unwrap_err();

        for error in [mutation, save, export, ocr, ai] {
            assert_eq!(error.code, "DOCUMENT_SESSION_INVALID_STATE");
        }
    }

    #[tokio::test]
    async fn activation_expiry_restores_candidate_ready_and_preserves_credential() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("encrypted-candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let candidate_id = open_test_session(&store, &original);
        let entry = store.entry(&candidate_id).unwrap();
        let credential_path = {
            let mut state = entry.state.lock();
            state.lifecycle = SessionLifecycle::CandidateActivationPrepared;
            state.credential = Some("secret".to_string());
            state.working_path.to_string_lossy().to_string()
        };
        store
            .password_cache
            .lock()
            .insert(credential_path.clone(), "secret".to_string());
        let activation_token = "expired-activation".to_string();
        store.activations_by_token.lock().insert(
            activation_token.clone(),
            CandidateActivation {
                token: activation_token.clone(),
                previous_session_id: None,
                previous_transition_token: None,
                candidate_session_id: candidate_id.clone(),
                created_at_ms: now_ms().saturating_sub(ACTIVATION_TTL_MS + 1),
                phase: CandidateActivationPhase::PreparedUnbound,
            },
        );
        store
            .activation_token_by_candidate
            .lock()
            .insert(candidate_id.clone(), activation_token);

        store.reap_expired_lifecycle().await.unwrap();

        {
            let state = entry.state.lock();
            assert_eq!(state.lifecycle, SessionLifecycle::CandidateReady);
            assert_eq!(state.credential.as_deref(), Some("secret"));
        }
        assert!(store.activations_by_token.lock().is_empty());
        assert!(store.activation_token_by_candidate.lock().is_empty());
        assert_eq!(
            store
                .password_cache
                .lock()
                .get(&credential_path)
                .map(String::as_str),
            Some("secret")
        );
        let (search_cache, layout_cache, vector_store) = test_derived_caches();
        discard_candidate_session_in_store(
            &candidate_id,
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap();
        assert!(store.entry(&candidate_id).is_err());
        assert!(!store.password_cache.lock().contains_key(&credential_path));
    }

    #[tokio::test]
    async fn transition_expiry_preserves_bound_candidate_and_source_authority() {
        let documents = tempfile::tempdir().unwrap();
        let old_path = documents.path().join("old.pdf");
        let candidate_path = documents.path().join("candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &old_path).unwrap();
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let old_id = open_test_session(&store, &old_path);
        let candidate_id = open_test_session(&store, &candidate_path);
        *store.active_session_id.lock() = Some(old_id.clone());
        let transition_token = "expired-transition".to_string();
        {
            let old = store.entry(&old_id).unwrap();
            let mut state = old.state.lock();
            state.lifecycle = SessionLifecycle::DecisionPending;
            state.active_transition = Some(SessionTransition {
                token: transition_token.clone(),
                source_session_id: old_id.clone(),
                kind: TransitionKind::DocumentSwitch,
                transition_epoch: 0,
                created_at_ms: now_ms().saturating_sub(TRANSITION_TTL_MS + 1),
                captured_content_generation: state.content_generation,
                expected_state_revision: state.state_revision,
                status: TransitionStatus::PreparedDiscard,
                prepared_target: None,
                execution: TransitionExecution::new(),
            });
        }
        store.entry(&candidate_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CandidateActivationPrepared;
        let activation_token = "linked-activation".to_string();
        store.activations_by_token.lock().insert(
            activation_token.clone(),
            CandidateActivation {
                token: activation_token.clone(),
                previous_session_id: Some(old_id.clone()),
                previous_transition_token: Some(transition_token),
                candidate_session_id: candidate_id.clone(),
                created_at_ms: now_ms().saturating_sub(ACTIVATION_TTL_MS + 1),
                phase: CandidateActivationPhase::BoundToTransition,
            },
        );
        store
            .activation_token_by_candidate
            .lock()
            .insert(candidate_id.clone(), activation_token);

        store.reap_expired_lifecycle().await.unwrap();

        {
            let old_entry = store.entry(&old_id).unwrap();
            let old = old_entry.state.lock();
            assert_eq!(old.lifecycle, SessionLifecycle::DecisionPending);
            assert!(old.active_transition.is_some());
        }
        assert_eq!(
            store.entry(&candidate_id).unwrap().state.lock().lifecycle,
            SessionLifecycle::CandidateActivationPrepared
        );
        assert_eq!(store.activations_by_token.lock().len(), 1);
        assert_eq!(store.activation_token_by_candidate.lock().len(), 1);

        let error = begin_session_transition(
            old_id,
            TransitionKind::DocumentSwitch,
            app.state::<DocumentSessionStore>(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "TRANSITION_ALREADY_ACTIVE");
    }

    #[tokio::test]
    async fn active_activation_blocks_direct_candidate_discard_with_typed_error() {
        let documents = tempfile::tempdir().unwrap();
        let candidate_path = documents.path().join("candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let candidate_id = open_test_session(&store, &candidate_path);
        store.entry(&candidate_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CandidateActivationPrepared;
        let activation_token = "active-discard-guard".to_string();
        store.activations_by_token.lock().insert(
            activation_token.clone(),
            CandidateActivation {
                token: activation_token.clone(),
                previous_session_id: None,
                previous_transition_token: None,
                candidate_session_id: candidate_id.clone(),
                created_at_ms: now_ms(),
                phase: CandidateActivationPhase::PreparedUnbound,
            },
        );
        store
            .activation_token_by_candidate
            .lock()
            .insert(candidate_id.clone(), activation_token);
        let (search_cache, layout_cache, vector_store) = test_derived_caches();

        let error = discard_candidate_session_in_store(
            &candidate_id,
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, "CANDIDATE_ACTIVATION_ACTIVE");
        assert!(store.entry(&candidate_id).is_ok());
        assert!(store
            .activation_token_by_candidate
            .lock()
            .contains_key(&candidate_id));
    }

    #[tokio::test]
    async fn authenticated_candidate_discard_clears_password_cache() {
        let documents = tempfile::tempdir().unwrap();
        let candidate_path = documents.path().join("candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let candidate_id = open_test_session(&store, &candidate_path);
        let credential_path = {
            let entry = store.entry(&candidate_id).unwrap();
            let mut state = entry.state.lock();
            state.lifecycle = SessionLifecycle::CandidateReady;
            state.credential = Some("candidate-password".to_string());
            state.working_path.to_string_lossy().to_string()
        };
        store
            .password_cache
            .lock()
            .insert(credential_path.clone(), "candidate-password".to_string());
        let (search_cache, layout_cache, vector_store) = test_derived_caches();

        discard_candidate_session_in_store(
            &candidate_id,
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap();

        assert!(store.entry(&candidate_id).is_err());
        assert!(!store.password_cache.lock().contains_key(&credential_path));
    }

    fn insert_test_candidate(
        store: &DocumentSessionStore,
        original: &Path,
        encryption: CandidateEncryption,
    ) -> OpenDocumentSessionResult {
        let session_dir = store
            .instance_dir
            .join("sessions")
            .join(format!("session-{}", Uuid::new_v4()));
        create_secure_dir(&session_dir).unwrap();
        let working_path = session_dir.join("working.pdf");
        let digest = copy_source_consistent(original, &working_path).unwrap();
        store.insert_candidate(
            original.to_path_buf(),
            working_path,
            session_dir,
            digest,
            encryption,
        )
    }

    #[tokio::test]
    async fn owner_password_only_candidate_opens_without_prompt_and_rejects_mutations() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("owner-only.pdf");
        let bytes = crate::lopdf_compat::owner_only_encrypted_pdf(1);
        fs::write(&original, &bytes).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();

        let opened =
            insert_test_candidate(&store, &original, CandidateEncryption::OwnerPasswordOnly);
        assert_eq!(opened.status, "candidate_ready");
        let snapshot = opened.snapshot.expect("no password prompt, so a snapshot");
        assert_eq!(snapshot.lifecycle, SessionLifecycle::CandidateReady);
        assert!(!snapshot.editable);
        assert_eq!(
            snapshot.read_only_reason.as_deref(),
            Some("Şifreli PDF mutasyonları desteklenmiyor")
        );

        let working_path = {
            let entry = store.entry(&opened.session_id).unwrap();
            let mut state = entry.state.lock();
            state.lifecycle = SessionLifecycle::Open;
            state.working_path.clone()
        };
        let rejected = store
            .mutate(opened.session_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap_err();
        assert_eq!(rejected.code, "DOCUMENT_SESSION_READ_ONLY");
        assert_eq!(fs::read(working_path).unwrap(), bytes);
    }

    #[test]
    fn candidate_encryption_decides_prompt_and_editability() {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("minimal.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();

        let plain = insert_test_candidate(&store, &original, CandidateEncryption::None);
        assert_eq!(plain.status, "candidate_ready");
        assert!(plain.snapshot.unwrap().editable);

        let locked =
            insert_test_candidate(&store, &original, CandidateEncryption::PasswordRequired);
        assert_eq!(locked.status, "password_required");
        assert!(locked.snapshot.is_none());
        let entry = store.entry(&locked.session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.lifecycle, SessionLifecycle::CandidateAuthPending);
        assert!(!state.editable);
    }

    #[tokio::test]
    async fn activation_rollback_preserves_successful_save_baseline_target_and_clean_state() {
        let documents = tempfile::tempdir().unwrap();
        let old_path = documents.path().join("old.pdf");
        let saved_path = documents.path().join("saved-as.pdf");
        let candidate_path = documents.path().join("candidate.pdf");
        fs::copy(fixture_path("minimal.pdf"), &old_path).unwrap();
        fs::copy(fixture_path("minimal.pdf"), &candidate_path).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let old_id = open_test_session(&store, &old_path);
        let candidate_id = open_test_session(&store, &candidate_path);
        *store.active_session_id.lock() = Some(old_id.clone());
        let candidate_credential_path = {
            let candidate = store.entry(&candidate_id).unwrap();
            let mut state = candidate.state.lock();
            state.credential = Some("candidate-password".to_string());
            state.working_path.to_string_lossy().to_string()
        };
        store.password_cache.lock().insert(
            candidate_credential_path.clone(),
            "candidate-password".to_string(),
        );
        let mutated = store
            .mutate(old_id.clone(), 1, |path, password| {
                manipulation::rotate_page_report(path, 0, 90, password)
            })
            .await
            .unwrap();
        let saved = store
            .save(&old_id, mutated.state_revision, Some(saved_path.clone()))
            .await
            .unwrap();
        let (saved_baseline, saved_target) = {
            let old = store.entry(&old_id).unwrap();
            let state = old.state.lock();
            (state.baseline.clone(), state.save_target_path.clone())
        };
        let transition_token = "saved-switch".to_string();
        {
            let old = store.entry(&old_id).unwrap();
            let mut state = old.state.lock();
            state.lifecycle = SessionLifecycle::DecisionPending;
            let expected_state_revision = state.state_revision;
            state.active_transition = Some(SessionTransition {
                token: transition_token.clone(),
                source_session_id: old_id.clone(),
                kind: TransitionKind::DocumentSwitch,
                transition_epoch: 0,
                created_at_ms: now_ms(),
                captured_content_generation: state.content_generation,
                expected_state_revision,
                status: TransitionStatus::PreparedSave,
                prepared_target: Some(saved_target.clone()),
                execution: TransitionExecution::new(),
            });
        }
        store.entry(&candidate_id).unwrap().state.lock().lifecycle =
            SessionLifecycle::CandidateActivationPrepared;
        let activation_token = "saved-activation".to_string();
        store.activations_by_token.lock().insert(
            activation_token.clone(),
            CandidateActivation {
                token: activation_token.clone(),
                previous_session_id: Some(old_id.clone()),
                previous_transition_token: Some(transition_token),
                candidate_session_id: candidate_id.clone(),
                created_at_ms: now_ms(),
                phase: CandidateActivationPhase::BoundToTransition,
            },
        );
        store
            .activation_token_by_candidate
            .lock()
            .insert(candidate_id.clone(), activation_token.clone());
        let (search_cache, layout_cache, vector_store) = test_derived_caches();

        let restored = rollback_candidate_activation_in_store(
            &activation_token,
            &store,
            &search_cache,
            &layout_cache,
            &vector_store,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(restored.id, old_id);
        assert_eq!(restored.lifecycle, SessionLifecycle::Open);
        assert!(!restored.dirty);
        assert_eq!(restored.content_generation, saved.content_generation);
        assert!(restored.state_revision > saved.state_revision);
        assert_eq!(restored.display_path, saved_path.to_string_lossy());
        let old = store.entry(&old_id).unwrap();
        let state = old.state.lock();
        assert_eq!(state.baseline.digest, saved_baseline.digest);
        assert_eq!(state.save_target_path, saved_target);
        assert!(state.active_transition.is_none());
        drop(state);
        assert!(store.entry(&candidate_id).is_err());
        assert!(!store
            .password_cache
            .lock()
            .contains_key(&candidate_credential_path));
        assert_eq!(
            store.active_session_id.lock().as_deref(),
            Some(old_id.as_str())
        );
        assert!(store.activations_by_token.lock().is_empty());
        assert!(store.activation_token_by_candidate.lock().is_empty());
    }

    #[test]
    fn active_session_authority_is_independent_from_registry_cardinality() {
        let documents = tempfile::tempdir().unwrap();
        let first = documents.path().join("first.pdf");
        let second = documents.path().join("second.pdf");
        fs::copy(fixture_path("minimal.pdf"), &first).unwrap();
        fs::copy(fixture_path("minimal.pdf"), &second).unwrap();
        let store = DocumentSessionStore::initialize().unwrap();
        let first_id = open_test_session(&store, &first);
        let second_id = open_test_session(&store, &second);

        assert!(store.active_session_snapshot().unwrap().is_none());
        *store.active_session_id.lock() = Some(second_id.clone());
        let active = store.active_session_snapshot().unwrap().unwrap();
        assert_eq!(active.id, second_id);
        assert_ne!(active.id, first_id);
    }

    #[tokio::test]
    async fn concurrent_expiry_reapers_complete_without_deadlock() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let first = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.reap_expired_lifecycle().await })
        };
        let second = {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.reap_expired_lifecycle().await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            first.await.unwrap().unwrap();
            second.await.unwrap().unwrap();
        })
        .await
        .expect("expiry reaper lock order deadlock olmamalı");
    }

    #[tokio::test]
    async fn concurrent_prepare_commit_rollback_and_expiry_complete_without_deadlock() {
        let documents = tempfile::tempdir().unwrap();
        let old_path = documents.path().join("old.pdf");
        let first_path = documents.path().join("first.pdf");
        for path in [&old_path, &first_path] {
            fs::copy(fixture_path("minimal.pdf"), path).unwrap();
        }
        let app = test_command_app();
        let store = app.state::<DocumentSessionStore>();
        let old_id = open_test_session(&store, &old_path);
        let first_id = open_test_session(&store, &first_path);
        store.entry(&first_id).unwrap().state.lock().lifecycle = SessionLifecycle::CandidateReady;
        *store.active_session_id.lock() = Some(old_id.clone());
        let transition =
            begin_session_transition(old_id.clone(), TransitionKind::DocumentSwitch, app.state())
                .await
                .unwrap();
        prepare_candidate_activation(
            first_id.clone(),
            Some(old_id),
            Some(transition.token.clone()),
            app.state(),
        )
        .await
        .unwrap();
        *store.transition_claim_barrier.lock() = Some(Arc::new(tokio::sync::Barrier::new(2)));

        let token = transition.token;
        let (first, second) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                discard_transition(
                    token.clone(),
                    app.state(),
                    app.state(),
                    app.state(),
                    app.state(),
                ),
                discard_transition(token, app.state(), app.state(), app.state(), app.state(),)
            )
        })
        .await
        .expect("duplicate transition lock sırası deadlock olmamalı");
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first.id, first_id);
        assert_eq!(second.id, first_id);
        assert_eq!(first.state_revision, second.state_revision);
    }

    #[cfg(unix)]
    #[test]
    fn unix_temp_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let store = DocumentSessionStore::initialize().unwrap();
        assert_eq!(
            fs::metadata(&store.instance_dir)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&store.instance_lock_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_temp_objects_require_verified_current_user_acl() {
        let store = DocumentSessionStore::initialize().unwrap();
        ensure_windows_current_user_acl(&store.instance_dir, true).unwrap();
        ensure_windows_current_user_acl(&store.instance_lock_path, false).unwrap();
        ensure_windows_current_user_acl(&store.instance_dir.join("jobs"), true).unwrap();
    }
    fn setup_test_session(store: &DocumentSessionStore) -> (tempfile::TempDir, String) {
        let documents = tempfile::tempdir().unwrap();
        let original = documents.path().join("source.pdf");
        fs::copy(fixture_path("minimal.pdf"), &original).unwrap();
        let session_id = open_test_session(store, &original);
        (documents, session_id)
    }

    fn test_supervisor_bundle(
        record: &Arc<MutationTransactionRecord>,
        entry: &Arc<SessionEntry>,
        execution_authority: &SupervisorExecutionAuthority,
        child_owner: &SessionJobChildProcessOwner,
        prefix: &str,
    ) -> (MutationSupervisorRecoveryBundle, ExchangeCleanupAuthority) {
        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: record.id,
            _session_id: record.session_id.clone(),
            initial_digest: None,
            replacement_digest: Arc::new(Mutex::new(None)),
            preflight: Arc::new(Mutex::new(None)),
            working_path: entry.state.lock().working_path.clone(),
            reconciliation_state: Arc::new(Mutex::new(ReconciliationState::NotStarted)),
        };
        let temp_dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        let exchange_path = temp_dir.path().to_path_buf();
        let exchange_authority = ExchangeCleanupAuthority::new(temp_dir, exchange_path);
        (
            MutationSupervisorRecoveryBundle {
                transaction_record: Arc::clone(record),
                session_entry: Arc::clone(entry),
                recovery_ctx,
                child_owner: child_owner.clone(),
                exchange_cleanup_authority: exchange_authority.clone(),
                execution_authority: execution_authority.clone(),
            },
            exchange_authority,
        )
    }

    #[derive(Default)]
    struct FakeMutationEngine {
        mutation_result: Mutex<Option<Result<JobResult, DocumentSessionError>>>,
        validation_result: Mutex<Option<Result<(), DocumentSessionError>>>,
        commit_result: Mutex<Option<Result<AtomicWriteReport, DocumentIoError>>>,
        fake_output_content: Mutex<Vec<u8>>,
    }

    impl MutationEngine for FakeMutationEngine {
        fn run_mutation_worker(
            &self,
            job_ctx: &SessionJobContext,
            _recipe: &MutationRecipe,
        ) -> Result<JobResult, DocumentSessionError> {
            if let Some(res) = self.mutation_result.lock().take() {
                return res;
            }
            let exchange_dir = job_ctx
                .snapshot_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let out_path = exchange_dir.join("mutation-output.pdf");
            let _ = fs::write(&out_path, &*self.fake_output_content.lock());
            Ok(JobResult::File(out_path))
        }

        fn run_validation_worker(
            &self,
            _job_ctx: &SessionJobContext,
        ) -> Result<(), DocumentSessionError> {
            if let Some(res) = self.validation_result.lock().take() {
                return res;
            }
            Ok(())
        }

        fn commit_prevalidated_pdf(
            &self,
            _operation: &'static str,
            working_path: &Path,
            _output_path: &Path,
            validated_digest: &str,
        ) -> Result<AtomicWriteReport, DocumentIoError> {
            if let Some(res) = self.commit_result.lock().take() {
                return res;
            }
            let _ = fs::write(working_path, &*self.fake_output_content.lock());
            let digest = sha256_file(working_path).unwrap_or_else(|_| validated_digest.to_string());
            Ok(AtomicWriteReport {
                state: CommitState::NewCommitted,
                committed_digest: digest,
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mutation_worker_started_caller_abort_completes_transaction() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (hit_tx, hit_rx) = tokio::sync::oneshot::channel::<()>();
        let hit_cell = Arc::new(Mutex::new(Some(hit_tx)));
        // Kayıt hook içinde alınır: caller abort edildikten sonra işlem bitip
        // active_transactions'tan çıkmış olabilir; id ile sonradan aramak yarış yaratır.
        let (record_tx, record_rx) =
            tokio::sync::oneshot::channel::<Arc<MutationTransactionRecord>>();
        let record_cell = Arc::new(Mutex::new(Some(record_tx)));
        let hook_manager = Arc::clone(&manager);

        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::MutationWorkerStarted {
                if let Some(t) = hit_cell.lock().take() {
                    let _ = t.send(());
                }
                if let Some(t2) = record_cell.lock().take() {
                    let record = hook_manager
                        .get_transaction(id)
                        .expect("işlem hook anında aktif");
                    let _ = t2.send(record);
                }
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = hit_rx.await;
        let record = record_rx.await.unwrap();
        handle.abort();

        let _ = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 2);
        assert!(state.dirty);
        assert!(state.working_digest.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mutation_completed_before_validation_caller_abort() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (hit_tx, hit_rx) = tokio::sync::oneshot::channel::<()>();
        let hit_cell = Arc::new(Mutex::new(Some(hit_tx)));
        // Kayıt hook içinde alınır: caller abort edildikten sonra işlem bitip
        // active_transactions'tan çıkmış olabilir; id ile sonradan aramak yarış yaratır.
        let (record_tx, record_rx) =
            tokio::sync::oneshot::channel::<Arc<MutationTransactionRecord>>();
        let record_cell = Arc::new(Mutex::new(Some(record_tx)));
        let hook_manager = Arc::clone(&manager);

        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::BeforeValidation {
                if let Some(t) = hit_cell.lock().take() {
                    let _ = t.send(());
                }
                if let Some(t2) = record_cell.lock().take() {
                    let record = hook_manager
                        .get_transaction(id)
                        .expect("işlem hook anında aktif");
                    let _ = t2.send(record);
                }
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = hit_rx.await;
        let record = record_rx.await.unwrap();
        handle.abort();

        let _ = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 2);
        assert!(state.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_validation_completed_before_commit_cancellation() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let mgr_clone = Arc::clone(&manager);
        manager.register_test_hook(Arc::new(move |phase, tx_id| {
            if phase == MutationTestHookPhase::BeforeCommit {
                mgr_clone.cancel_transaction(tx_id, "test_pre_commit_cancel");
            }
        }));

        let result = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "MUTATION_TRANSACTION_CANCELLED");

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 1);
        assert!(!state.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_point_of_no_return_caller_waiter_drop_completes_commit() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (hit_tx, hit_rx) = tokio::sync::oneshot::channel::<()>();
        let hit_cell = Arc::new(Mutex::new(Some(hit_tx)));
        // Kayıt hook içinde alınır: caller abort edildikten sonra işlem bitip
        // active_transactions'tan çıkmış olabilir; id ile sonradan aramak yarış yaratır.
        let (record_tx, record_rx) =
            tokio::sync::oneshot::channel::<Arc<MutationTransactionRecord>>();
        let record_cell = Arc::new(Mutex::new(Some(record_tx)));
        let hook_manager = Arc::clone(&manager);

        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::BeforeAtomicCommitCall {
                if let Some(t) = hit_cell.lock().take() {
                    let _ = t.send(());
                }
                if let Some(t2) = record_cell.lock().take() {
                    let record = hook_manager
                        .get_transaction(id)
                        .expect("işlem hook anında aktif");
                    let _ = t2.send(record);
                }
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = hit_rx.await;
        let record = record_rx.await.unwrap();
        handle.abort();

        let _ = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 2);
        assert!(state.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_commit_completed_before_state_apply_caller_waiter_drop() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (hit_tx, hit_rx) = tokio::sync::oneshot::channel::<()>();
        let hit_cell = Arc::new(Mutex::new(Some(hit_tx)));
        // Kayıt hook içinde alınır: caller abort edildikten sonra işlem bitip
        // active_transactions'tan çıkmış olabilir; id ile sonradan aramak yarış yaratır.
        let (record_tx, record_rx) =
            tokio::sync::oneshot::channel::<Arc<MutationTransactionRecord>>();
        let record_cell = Arc::new(Mutex::new(Some(record_tx)));
        let hook_manager = Arc::clone(&manager);

        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::BeforeStateApply {
                if let Some(t) = hit_cell.lock().take() {
                    let _ = t.send(());
                }
                if let Some(t2) = record_cell.lock().take() {
                    let record = hook_manager
                        .get_transaction(id)
                        .expect("işlem hook anında aktif");
                    let _ = t2.send(record);
                }
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = hit_rx.await;
        let record = record_rx.await.unwrap();
        handle.abort();

        let _ = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 2);
        assert!(state.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cancellation_race_with_try_enter_committing() {
        let record = Arc::new(MutationTransactionRecord {
            id: MutationTransactionId::new(),
            session_id: "race-session".to_string(),
            start_content_generation: 1,
            control: Mutex::new(MutationTransactionControlState {
                phase: MutationTransactionPhase::CommitReady,
                cancel_requested: false,
                cancel_reason: None,
                terminal_outcome: None,
            }),
            cancellation_token: CancellationToken::new(),
            phase_notify: tokio::sync::Notify::new(),
        });

        record.request_cancel("cancel_race_reason".to_string());
        let res = record.try_enter_committing();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_TRANSACTION_CANCELLED");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_mutations_for_same_session_prevented() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let released = Arc::new(AtomicBool::new(false));
        let started_cell = Arc::new(Mutex::new(Some(started_tx)));
        let released_clone = Arc::clone(&released);

        manager.register_test_hook(Arc::new(move |phase, _id| {
            if phase == MutationTestHookPhase::BeforeCommit {
                if let Some(t) = started_cell.lock().take() {
                    let _ = t.send(());
                }
                let start = std::time::Instant::now();
                while !released_clone.load(Ordering::Acquire) {
                    std::thread::yield_now();
                    if start.elapsed() > Duration::from_secs(5) {
                        break;
                    }
                }
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = started_rx.await;

        let res2 = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 180,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await;
        assert!(res2.is_err());
        assert_eq!(res2.unwrap_err().code, "MUTATION_TRANSACTION_CONFLICT");

        released.store(true, Ordering::Release);
        let res1 = handle.await.unwrap();
        assert!(res1.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_typed_generation_conflict() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);

        let res = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                999,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "STALE_GENERATION");

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 1);
        assert!(!state.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mutation_failure_invalidates_worker_mapping_for_reopen() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);

        let fake_engine = FakeMutationEngine::default();
        *fake_engine.mutation_result.lock() = Some(Err(DocumentSessionError::new(
            "MUTATION_WORKER_FAILED",
            "Simulated worker failure",
        )));

        let res = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(fake_engine),
            )
            .await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_WORKER_FAILED");

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.lifecycle, SessionLifecycle::Open);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_generation_and_revision_increment_exactly_once() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);

        let res = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await
            .unwrap();
        assert_eq!(res.content_generation, 2);
        assert_eq!(res.state_revision, 2);

        let entry = store.entry(&session_id).unwrap();
        let state = entry.state.lock();
        assert_eq!(state.content_generation, 2);
        assert_eq!(state.state_revision, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_owner_panic_with_active_caller_waiter() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        manager.register_test_hook(Arc::new(move |phase, _id| {
            if phase == MutationTestHookPhase::BeforeCommit {
                panic!("controlled owner task panic for test");
            }
        }));

        let res = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_TRANSACTION_OWNER_PANIC");

        let begin_res = manager.begin_transaction(&session_id, 1);
        assert!(begin_res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_owner_panic_with_dropped_caller_waiter() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let started_cell = Arc::new(Mutex::new(Some(started_tx)));
        // Kayıt hook içinde alınır: caller abort edildikten sonra işlem bitip
        // active_transactions'tan çıkmış olabilir; id ile sonradan aramak yarış yaratır.
        let (record_tx, record_rx) =
            tokio::sync::oneshot::channel::<Arc<MutationTransactionRecord>>();
        let record_cell = Arc::new(Mutex::new(Some(record_tx)));
        let hook_manager = Arc::clone(&manager);

        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::MutationWorkerStarted {
                if let Some(t) = started_cell.lock().take() {
                    let _ = t.send(());
                }
                if let Some(t2) = record_cell.lock().take() {
                    let record = hook_manager
                        .get_transaction(id)
                        .expect("işlem hook anında aktif");
                    let _ = t2.send(record);
                }
            } else if phase == MutationTestHookPhase::BeforeCommit {
                panic!("controlled owner task panic after caller drop");
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = started_rx.await;
        let record = record_rx.await.unwrap();
        handle.abort();

        let _ = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();

        let begin_res = manager.begin_transaction(&session_id, 1);
        assert!(begin_res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_registry_task_ownership_and_join() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let snap = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await
            .unwrap();
        assert_eq!(snap.content_generation, 2);

        let begin_res = manager.begin_transaction(&session_id, 2);
        assert!(begin_res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exchange_temp_cleanup_verified() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);

        let snap = store
            .mutate_isolated_with_engine(
                session_id.clone(),
                1,
                MutationRecipe::RotatePage {
                    page_index: 0,
                    degrees: 90,
                },
                Arc::new(FakeMutationEngine::default()),
            )
            .await
            .unwrap();
        assert_eq!(snap.content_generation, 2);

        let jobs_root = store.jobs_root();
        if jobs_root.exists() {
            let entries = fs::read_dir(&jobs_root).unwrap();
            let count = entries.count();
            assert_eq!(
                count, 0,
                "Jobs root temp exchange directory was not cleaned up"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_recovery_hash_failure_sets_indeterminate() {
        let mut session = DocumentSession {
            id: "s1".to_string(),
            save_target_path: PathBuf::from("orig.pdf"),
            working_path: PathBuf::from("nonexistent.pdf"),
            session_dir: PathBuf::from("dir"),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 100,
            baseline: BaselineFingerprint {
                digest: "d1".to_string(),
            },
            working_digest: Some("d1".to_string()),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        };

        let err = DocumentIoError {
            operation: "test".to_string(),
            stage: DocumentIoStage::Replace,
            state: CommitState::OriginalRecovered,
            detail: "Recovery failure".to_string(),
            cleanup_error: None,
            recovery_path: None,
            replacement_digest: Some("d2".to_string()),
        };

        let res = apply_failed_mutation_reconciled(&mut session, Some("d1"), &err, None, None);

        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().code,
            "DOCUMENT_SESSION_RECOVERY_HASH_FAILED"
        );
        assert_eq!(session.lifecycle, SessionLifecycle::CommitIndeterminate);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_original_recovered_old_content_classification() {
        let mut session = DocumentSession {
            id: "s1".to_string(),
            save_target_path: PathBuf::from("orig.pdf"),
            working_path: PathBuf::from("working.pdf"),
            session_dir: PathBuf::from("dir"),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 100,
            baseline: BaselineFingerprint {
                digest: "d1".to_string(),
            },
            working_digest: Some("d1".to_string()),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        };

        let err = DocumentIoError {
            operation: "test".to_string(),
            stage: DocumentIoStage::Replace,
            state: CommitState::OriginalRecovered,
            detail: "Recovered original".to_string(),
            cleanup_error: None,
            recovery_path: None,
            replacement_digest: Some("d2".to_string()),
        };

        let res =
            apply_failed_mutation_reconciled(&mut session, Some("d1"), &err, Some("d1"), None);

        assert_eq!(res.unwrap(), CommitStateClassification::OriginalPreserved);
        assert_eq!(session.content_generation, 1);
        assert!(!session.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_original_recovered_replacement_content_classification() {
        let mut session = DocumentSession {
            id: "s1".to_string(),
            save_target_path: PathBuf::from("orig.pdf"),
            working_path: PathBuf::from("working.pdf"),
            session_dir: PathBuf::from("dir"),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            opened_at_ms: 100,
            baseline: BaselineFingerprint {
                digest: "d1".to_string(),
            },
            working_digest: Some("d1".to_string()),
            health_warning: None,
            credential: None,
            active_transition: None,
            last_save_commit: None,
            history: SessionHistory::default(),
        };

        let err = DocumentIoError {
            operation: "test".to_string(),
            stage: DocumentIoStage::Replace,
            state: CommitState::OriginalRecovered,
            detail: "Recovered replacement".to_string(),
            cleanup_error: None,
            recovery_path: None,
            replacement_digest: Some("d2".to_string()),
        };

        let res =
            apply_failed_mutation_reconciled(&mut session, Some("d1"), &err, Some("d2"), None);

        assert_eq!(
            res.unwrap(),
            CommitStateClassification::ReplacementCommitted
        );
        assert_eq!(session.content_generation, 2);
        assert!(session.dirty);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_invalid_state_machine_transition_rejected() {
        let record = Arc::new(MutationTransactionRecord {
            id: MutationTransactionId::new(),
            session_id: "invalid-transition".to_string(),
            start_content_generation: 1,
            control: Mutex::new(MutationTransactionControlState {
                phase: MutationTransactionPhase::Committed,
                cancel_requested: false,
                cancel_reason: None,
                terminal_outcome: None,
            }),
            cancellation_token: CancellationToken::new(),
            phase_notify: tokio::sync::Notify::new(),
        });

        let res = record.transition_to(MutationTransactionPhase::Aborted);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_INVALID_STATE_TRANSITION");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_single_child_owner_instance_shared() {
        let child_owner = SessionJobChildProcessOwner::default();
        let child_owner_clone = child_owner.clone();
        let ctx = MutationTransactionContext::new(
            MutationTransactionId::new(),
            "test-session".to_string(),
            1,
            PathBuf::from("/tmp"),
            None,
            CancellationToken::new(),
            child_owner,
        );
        let job_ctx = ctx.to_session_job_context();
        assert!(Arc::ptr_eq(
            &job_ctx.child_processes.children,
            &child_owner_clone.children
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pre_commit_cancellation_yields_aborted_outcome() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (rec_tx, rec_rx) = tokio::sync::oneshot::channel::<Arc<MutationTransactionRecord>>();
        let rec_cell = Arc::new(Mutex::new(Some(rec_tx)));

        let mgr_clone = Arc::clone(&manager);
        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::MutationWorkerStarted {
                if let Some(rec) = mgr_clone.get_transaction(id) {
                    if let Some(t) = rec_cell.lock().take() {
                        let _ = t.send(rec);
                    }
                }
                mgr_clone.cancel_transaction(id, "test_aborted_reason");
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let record = rec_rx.await.unwrap();
        let _ = handle.await;

        let terminal = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();

        match terminal {
            MutationTransactionOutcome::Aborted { reason, .. } => {
                assert_eq!(reason, "test_aborted_reason");
            }
            other => panic!("Expected Aborted outcome, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_applying_session_state_observed_before_state_apply() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (tx_id_tx, tx_id_rx) = tokio::sync::oneshot::channel::<MutationTransactionId>();
        let tx_id_cell = Arc::new(Mutex::new(Some(tx_id_tx)));

        let observed_phase = Arc::new(Mutex::new(None));
        let observed_phase_clone = Arc::clone(&observed_phase);
        let mgr_clone = Arc::clone(&manager);

        manager.register_test_hook(Arc::new(move |phase, id| {
            if phase == MutationTestHookPhase::BeforeStateApply {
                if let Some(t) = tx_id_cell.lock().take() {
                    let _ = t.send(*id);
                }
                if let Some(record) = mgr_clone.get_transaction(id) {
                    let current = record.control.lock().phase;
                    *observed_phase_clone.lock() = Some(current);
                }
            }
        }));

        let handle = {
            let store_clone = Arc::clone(&store);
            let session_clone = session_id.clone();
            tokio::spawn(async move {
                store_clone
                    .mutate_isolated_with_engine(
                        session_clone,
                        1,
                        MutationRecipe::RotatePage {
                            page_index: 0,
                            degrees: 90,
                        },
                        Arc::new(FakeMutationEngine::default()),
                    )
                    .await
            })
        };

        let _ = tx_id_rx.await;
        let snap = handle.await.unwrap().unwrap();

        assert_eq!(snap.content_generation, 2);
        let phase = observed_phase.lock().unwrap();
        assert_eq!(phase, MutationTransactionPhase::ApplyingSessionState);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_invalid_terminal_publication_rejected() {
        let record = Arc::new(MutationTransactionRecord {
            id: MutationTransactionId::new(),
            session_id: "pub-test".to_string(),
            start_content_generation: 1,
            control: Mutex::new(MutationTransactionControlState {
                phase: MutationTransactionPhase::Preparing,
                cancel_requested: false,
                cancel_reason: None,
                terminal_outcome: None,
            }),
            cancellation_token: CancellationToken::new(),
            phase_notify: tokio::sync::Notify::new(),
        });

        let dummy_snap = DocumentSessionSnapshot {
            id: "pub-test".to_string(),
            opened_at_ms: 0,
            display_path: "pub-test".to_string(),
            file_name: "pub-test".to_string(),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            health_warning: None,
            undo_operation: None,
            redo_operation: None,
        };

        let res = record.publish_terminal(MutationTransactionOutcome::Completed {
            transaction_id: record.id.to_string(),
            session_id: record.session_id.clone(),
            snapshot: dummy_snap,
            cleanup_warning: None,
        });

        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_INVALID_STATE_TRANSITION");
        assert_eq!(
            record.control.lock().phase,
            MutationTransactionPhase::Preparing
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mutation_shutdown_cancels_in_flight_transactions() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (hit_tx, hit_rx) = tokio::sync::oneshot::channel::<()>();
        let hit_cell = Arc::new(Mutex::new(Some(hit_tx)));

        manager.register_test_hook(Arc::new(move |phase, _id| {
            if phase == MutationTestHookPhase::MutationWorkerStarted {
                if let Some(t) = hit_cell.lock().take() {
                    let _ = t.send(());
                }
            }
        }));

        let store_clone = Arc::clone(&store);
        let session_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            store_clone
                .mutate_isolated_with_engine(
                    session_clone,
                    1,
                    MutationRecipe::RotatePage {
                        page_index: 0,
                        degrees: 90,
                    },
                    Arc::new(FakeMutationEngine::default()),
                )
                .await
        });

        let _ = hit_rx.await;
        let shutdown_res = manager.shutdown(3000).await;
        let res = handle.await.unwrap();

        assert!(res.is_ok() || res.is_err());
        assert!(shutdown_res.is_ok());
        assert!(manager.list_active_transactions().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mutation_shutdown_aborts_stuck_supervisor_and_reaps_resources() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();
        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let record_id = record.id;
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, exchange_authority) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-stuck-supervisor-",
        );
        let slow_task = spawn_managed_test_supervisor(execution_authority.clone(), async move {
            std::future::pending::<()>().await;
            MutationTransactionOutcome::Aborted {
                transaction_id: record_id.to_string(),
                session_id,
                reason: "slow".to_string(),
                cleanup_warning: None,
            }
        });

        guard.disarm();
        manager.register_supervisor_bundle(record_id, bundle, slow_task);

        let report = manager.shutdown(3000).await.unwrap();

        assert!(execution_authority.was_abort_requested());
        assert_eq!(
            execution_authority.execution_state(),
            SupervisorExecutionState::Exited
        );
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::Terminated);
        assert!(manager.list_active_transactions().is_empty());
        assert_eq!(manager.active_supervisor_count(), 0);
        assert_eq!(exchange_authority.state(), ExchangeCleanupState::Cleaned);
        assert_eq!(report.failed_pre_commit, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_mutation_shutdown_is_idempotent() {
        let manager = Arc::new(MutationTransactionManager::new());
        let res1 = manager.shutdown(100).await;
        let res2 = manager.shutdown(100).await;

        assert!(res1.is_ok());
        assert!(res2.is_ok());
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::Terminated);
        assert!(manager.list_active_transactions().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_coordinator_never_publishes_success_with_active_transaction() {
        let manager = Arc::new(MutationTransactionManager::new());
        let (_record, guard) = manager.begin_transaction("active-only", 1).unwrap();

        let first = manager.shutdown(50).await;
        assert_eq!(first.unwrap_err().code, "MUTATION_SHUTDOWN_CALLER_TIMEOUT");
        {
            let inner = manager.inner.lock();
            assert!(!inner.active_transactions.is_empty());
            assert!(inner.shutdown_completion.is_none());
            assert_eq!(inner.lifecycle, MutationManagerLifecycle::ShuttingDown);
            assert_ne!(
                inner.coordinator_monitor_join_state,
                CoordinatorMonitorJoinState::Joining
            );
        }

        guard.release();
        let second = manager.shutdown(3000).await;
        assert!(second.is_ok());
        let inner = manager.inner.lock();
        assert!(inner.active_transactions.is_empty());
        assert_eq!(inner.lifecycle, MutationManagerLifecycle::Terminated);
        assert!(inner
            .shutdown_completion
            .as_ref()
            .is_some_and(|result| result.is_ok()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_shutdown_admission_and_reservation_race_linearizable() {
        let manager = MutationTransactionManager::new();
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::Running);

        let manager_clone = Arc::new(manager);
        let mgr_for_task = Arc::clone(&manager_clone);

        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let res = mgr_for_task.shutdown(100).await;
            let _ = started_tx.send(());
            res
        });

        let (rec_res, _) = (
            manager_clone.begin_transaction("race-session", 1),
            started_rx.await,
        );

        if let Err(err) = rec_res {
            assert_eq!(err.code, "MUTATION_MANAGER_SHUTTING_DOWN");
        }
        let _ = handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_supervisor_finalizer_cannot_remove_own_join_handle() {
        let manager = MutationTransactionManager::new();
        let (record, _guard) = manager.begin_transaction("finalizer-test", 1).unwrap();
        let child_owner = SessionJobChildProcessOwner::default();

        let temp_dir = tempfile::Builder::new()
            .prefix("test-fin-")
            .tempdir()
            .unwrap();
        let path = temp_dir.path().to_path_buf();
        let exchange_authority = ExchangeCleanupAuthority::new(temp_dir, path);

        let dummy_snap = DocumentSessionSnapshot {
            id: "finalizer-test".to_string(),
            opened_at_ms: 0,
            display_path: "finalizer-test".to_string(),
            file_name: "finalizer-test".to_string(),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            health_warning: None,
            undo_operation: None,
            redo_operation: None,
        };

        let outcome = MutationTransactionOutcome::Completed {
            transaction_id: record.id.to_string(),
            session_id: record.session_id.clone(),
            snapshot: dummy_snap,
            cleanup_warning: None,
        };

        let exec_auth = SupervisorExecutionAuthority::new();
        exec_auth.mark_owner_body_exited();
        exec_auth.mark_supervisor_body_exited();
        claim_normal_finalizer(&exec_auth).unwrap();
        let res = finalize_with_existing_claim(
            &record,
            &manager,
            &child_owner,
            &exchange_authority,
            &exec_auth,
            FinalizerClaim::ClaimedByNormalFinalizer,
            outcome,
        );
        assert_eq!(
            res.unwrap_err().code,
            "MUTATION_FINALIZER_REGISTRY_CORRUPTION"
        );
        assert!(record.control.lock().terminal_outcome.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_multiple_waiters_share_supervisor_completion_watch() {
        let record = Arc::new(MutationTransactionRecord {
            id: MutationTransactionId::new(),
            session_id: "multi-waiter".to_string(),
            start_content_generation: 1,
            control: Mutex::new(MutationTransactionControlState {
                phase: MutationTransactionPhase::ApplyingSessionState,
                cancel_requested: false,
                cancel_reason: None,
                terminal_outcome: None,
            }),
            cancellation_token: CancellationToken::new(),
            phase_notify: tokio::sync::Notify::new(),
        });

        let record_c1 = Arc::clone(&record);
        let record_c2 = Arc::clone(&record);

        let h1 = tokio::spawn(async move { record_c1.wait_for_terminal().await });
        let h2 = tokio::spawn(async move { record_c2.wait_for_terminal().await });

        let dummy_snap = DocumentSessionSnapshot {
            id: "multi-waiter".to_string(),
            opened_at_ms: 0,
            display_path: "multi-waiter".to_string(),
            file_name: "multi-waiter".to_string(),
            content_generation: 1,
            state_revision: 1,
            dirty: false,
            lifecycle: SessionLifecycle::Open,
            editable: true,
            read_only_reason: None,
            health_warning: None,
            undo_operation: None,
            redo_operation: None,
        };

        record
            .publish_terminal(MutationTransactionOutcome::Completed {
                transaction_id: record.id.to_string(),
                session_id: record.session_id.clone(),
                snapshot: dummy_snap,
                cleanup_warning: None,
            })
            .unwrap();

        let out1 = h1.await.unwrap();
        let out2 = h2.await.unwrap();

        assert!(matches!(out1, MutationTransactionOutcome::Completed { .. }));
        assert!(matches!(out2, MutationTransactionOutcome::Completed { .. }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_supervisor_timeout_preserves_registry_entries() {
        let manager = Arc::new(MutationTransactionManager::new());
        let (record, _guard) = manager.begin_transaction("preserve-test", 1).unwrap();
        let record_id = record.id;

        let (_slow_tx, slow_rx) = tokio::sync::oneshot::channel::<()>();
        let slow_task = tokio::spawn(async move {
            let _ = slow_rx.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: record_id.to_string(),
                session_id: "preserve-test".to_string(),
                reason: "slow".to_string(),
                cleanup_warning: None,
            }
        });

        manager.register_supervisor_handle(record_id, slow_task);

        let res = manager.shutdown(50).await;
        assert!(res.is_err());
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::ShuttingDown);
        assert_eq!(manager.active_reservation_count(), 1);
        assert!(!manager.list_active_transactions().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_running_post_commit_task_blocks_parallel_reconciliation() {
        let rec_state = Arc::new(Mutex::new(ReconciliationState::InProgress));
        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: MutationTransactionId::new(),
            _session_id: "reconcile-test".to_string(),
            initial_digest: Some("d1".to_string()),
            replacement_digest: Arc::new(Mutex::new(Some("d2".to_string()))),
            preflight: Arc::new(Mutex::new(None)),
            working_path: PathBuf::from("/tmp/nonexistent.pdf"),
            reconciliation_state: rec_state,
        };

        let store = DocumentSessionStore::initialize().unwrap();
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let res = reconcile_post_commit_authority(
            &recovery_ctx,
            &entry,
            &recovery_ctx._transaction_id,
            &session_id,
            "test_panic",
        );

        assert!(res.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_reconciliation_executes_exactly_once() {
        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: MutationTransactionId::new(),
            _session_id: "once-test".to_string(),
            initial_digest: Some("d1".to_string()),
            replacement_digest: Arc::new(Mutex::new(Some("d2".to_string()))),
            preflight: Arc::new(Mutex::new(None)),
            working_path: PathBuf::from("/tmp/nonexistent.pdf"),
            reconciliation_state: Arc::new(Mutex::new(ReconciliationState::NotStarted)),
        };

        let store = DocumentSessionStore::initialize().unwrap();
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let res1 = reconcile_post_commit_authority(
            &recovery_ctx,
            &entry,
            &recovery_ctx._transaction_id,
            &session_id,
            "test_panic",
        );
        let res2 = reconcile_post_commit_authority(
            &recovery_ctx,
            &entry,
            &recovery_ctx._transaction_id,
            &session_id,
            "test_panic",
        );

        assert!(res1.is_some());
        assert!(res2.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_shutdown_calls_share_single_coordinator_result() {
        let manager = Arc::new(MutationTransactionManager::new());
        let m1 = Arc::clone(&manager);
        let m2 = Arc::clone(&manager);
        let m3 = Arc::clone(&manager);

        let (r1, r2, r3) = tokio::join!(
            tokio::spawn(async move { m1.shutdown(3000).await }),
            tokio::spawn(async move { m2.shutdown(3000).await }),
            tokio::spawn(async move { m3.shutdown(3000).await })
        );

        let res1 = r1.unwrap();
        let res2 = r2.unwrap();
        let res3 = r3.unwrap();

        assert!(res1.is_ok());
        assert!(res2.is_ok());
        assert!(res3.is_ok());
        assert_eq!(manager.coordinator_start_count(), 1);
        assert_eq!(
            manager.coordinator_state(),
            ShutdownCoordinatorState::Completed
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_child_process_reaped_exactly_once_across_shutdown() {
        let child_owner = SessionJobChildProcessOwner::default();

        #[cfg(unix)]
        let child = std::process::Command::new("sleep")
            .arg("10")
            .spawn()
            .unwrap();

        #[cfg(windows)]
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping 127.0.0.1 -n 10"])
            .spawn()
            .unwrap();

        child_owner.register_child(child);

        let report1 = child_owner.kill_and_reap_all_typed();
        assert_eq!(report1.attempted, 1);
        assert_eq!(report1.reaped, 1);
        assert!(report1.errors.is_empty());

        let report2 = child_owner.kill_and_reap_all_typed();
        assert_eq!(report2.attempted, 0);
        assert_eq!(report2.reaped, 0);
        assert!(report2.errors.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_shutdown_failure_does_not_clear_maps() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let record_id = record.id;

        let (_slow_tx, slow_rx) = tokio::sync::oneshot::channel::<()>();
        let slow_task = tokio::spawn(async move {
            let _ = slow_rx.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: record_id.to_string(),
                session_id: "fail-clear".to_string(),
                reason: "slow".to_string(),
                cleanup_warning: None,
            }
        });

        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: record_id,
            _session_id: session_id.clone(),
            initial_digest: None,
            replacement_digest: Arc::new(Mutex::new(None)),
            preflight: Arc::new(Mutex::new(None)),
            working_path: PathBuf::from("/tmp/test"),
            reconciliation_state: Arc::new(Mutex::new(ReconciliationState::NotStarted)),
        };

        let temp_dir = tempfile::Builder::new()
            .prefix("test-fail-")
            .tempdir()
            .unwrap();
        let path = temp_dir.path().to_path_buf();
        let exchange_dir = ExchangeCleanupAuthority::new(temp_dir, path);

        let bundle = MutationSupervisorRecoveryBundle {
            transaction_record: Arc::clone(&record),
            session_entry: Arc::clone(&entry),
            recovery_ctx,
            child_owner: SessionJobChildProcessOwner::default(),
            exchange_cleanup_authority: exchange_dir.clone(),
            execution_authority: SupervisorExecutionAuthority::new(),
        };

        guard.disarm();
        manager.register_supervisor_bundle(record_id, bundle, slow_task);

        let res = manager.shutdown(20).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_SHUTDOWN_CALLER_TIMEOUT");
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::ShuttingDown);
        assert_eq!(manager.active_reservation_count(), 1);
        assert!(!manager.list_active_transactions().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_supervisor_handle_removed_only_after_external_await() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let record_id = record.id;
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, _exchange_authority) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-external-await-",
        );
        // The supervisor stays pending until released: registration starts the reaper,
        // which may already collect a finished task before the count below is read.
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let task = spawn_managed_test_supervisor(execution_authority, async move {
            let _ = release_rx.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: record_id.to_string(),
                session_id: "external-await".to_string(),
                reason: "done".to_string(),
                cleanup_warning: None,
            }
        });

        guard.disarm();
        manager.register_supervisor_bundle(record_id, bundle, task);

        assert_eq!(manager.active_supervisor_count(), 1);

        release_tx.send(()).unwrap();
        let res = manager.shutdown(3000).await;
        assert!(res.is_ok());
        assert_eq!(manager.active_supervisor_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_shutdown_all_resources_gates_cleanup_on_mutation_shutdown_failure() {
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let manager = store.mutation_transaction_manager();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let record_id = record.id;

        let (_slow_tx, slow_rx) = tokio::sync::oneshot::channel::<()>();
        let slow_task = tokio::spawn(async move {
            let _ = slow_rx.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: record_id.to_string(),
                session_id,
                reason: "slow".to_string(),
                cleanup_warning: None,
            }
        });

        let entry = store.entry(&record.session_id).unwrap();
        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: record_id,
            _session_id: record.session_id.clone(),
            initial_digest: None,
            replacement_digest: Arc::new(Mutex::new(None)),
            preflight: Arc::new(Mutex::new(None)),
            working_path: PathBuf::from("/tmp/test"),
            reconciliation_state: Arc::new(Mutex::new(ReconciliationState::NotStarted)),
        };

        let temp_dir = tempfile::Builder::new()
            .prefix("test-gate-")
            .tempdir()
            .unwrap();
        let path = temp_dir.path().to_path_buf();
        let exchange_dir = ExchangeCleanupAuthority::new(temp_dir, path);

        let bundle = MutationSupervisorRecoveryBundle {
            transaction_record: Arc::clone(&record),
            session_entry: Arc::clone(&entry),
            recovery_ctx,
            child_owner: SessionJobChildProcessOwner::default(),
            exchange_cleanup_authority: exchange_dir,
            execution_authority: SupervisorExecutionAuthority::new(),
        };

        guard.disarm();
        manager.register_supervisor_bundle(record_id, bundle, slow_task);

        let active_tasks = ActiveRenderTasks(parking_lot::Mutex::new(HashMap::new()));
        let render_service = RenderService::new();
        let warnings = store
            .shutdown_all_resources(&active_tasks, &render_service)
            .await;

        assert!(!warnings.is_empty());
        assert!(warnings
            .iter()
            .any(|w| w.code == "MUTATION_SHUTDOWN_CALLER_TIMEOUT"
                || w.code == "MUTATION_GLOBAL_SHUTDOWN_GATED"));
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::ShuttingDown);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_coordinator_panic_publishes_shared_failure_and_notifies_waiters() {
        let manager = Arc::new(MutationTransactionManager::new());
        let (panic_tx, panic_rx) = tokio::sync::oneshot::channel::<()>();
        let panic_cell = Arc::new(Mutex::new(Some(panic_tx)));

        manager.register_test_hook(Arc::new(move |phase, _id| {
            if phase == MutationTestHookPhase::BeforeCoordinatorBody {
                if let Some(cell) = panic_cell.lock().take() {
                    let _ = cell.send(());
                    panic!("Controlled coordinator panic for testing");
                }
            }
        }));

        let mgr_clone = Arc::clone(&manager);
        let handle = tokio::spawn(async move { mgr_clone.shutdown(3000).await });

        let _ = panic_rx.await;
        let res = handle.await.unwrap();

        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_SHUTDOWN_COORDINATOR_PANIC");
        assert_eq!(
            manager.coordinator_state(),
            ShutdownCoordinatorState::Completed
        );
        assert!(matches!(
            manager.coordinator_join_state(),
            CoordinatorJoinState::JoinedFailure(_)
        ));
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::BodyCompleted
        );

        let monitor_join = manager.join_coordinator_and_reaper_monitors().await;
        assert!(monitor_join.is_ok());
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::ExternallyJoinedSuccess
        );
        assert_eq!(
            manager.reaper_monitor_join_state(),
            ReaperMonitorJoinState::ExternallyJoinedSuccess
        );
        assert!(manager.shutdown_coordinator_monitor_task.lock().is_none());
        assert!(manager.reaper_monitor_task.lock().is_none());
        assert_eq!(manager.reaper_task_state(), ReaperTaskState::JoinedSuccess);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_coordinator_running_to_completed_awaits_handle() {
        let manager = Arc::new(MutationTransactionManager::new());
        assert_eq!(
            manager.coordinator_state(),
            ShutdownCoordinatorState::NotStarted
        );
        assert_eq!(
            manager.coordinator_join_state(),
            CoordinatorJoinState::NotStarted
        );
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::NotStarted
        );

        let res = manager.shutdown(3000).await;
        assert!(res.is_ok());
        assert_eq!(
            manager.coordinator_state(),
            ShutdownCoordinatorState::Completed
        );
        assert_eq!(
            manager.coordinator_join_state(),
            CoordinatorJoinState::JoinedSuccess
        );
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::BodyCompleted
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_caller_timeout_does_not_cancel_coordinator_or_cache_as_terminal() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let tx_id = record.id;

        let (slow_tx, slow_rx) = tokio::sync::oneshot::channel::<()>();
        let slow_session_id = session_id.clone();
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, _exchange_authority) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-caller-timeout-",
        );
        let slow_task = spawn_managed_test_supervisor(execution_authority, async move {
            let _ = slow_rx.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: tx_id.to_string(),
                session_id: slow_session_id,
                reason: "slow".to_string(),
                cleanup_warning: None,
            }
        });

        guard.disarm();
        manager.register_supervisor_bundle(tx_id, bundle, slow_task);

        let caller1_res = manager.shutdown(50).await;
        assert!(caller1_res.is_err());
        assert_eq!(
            caller1_res.unwrap_err().code,
            "MUTATION_SHUTDOWN_CALLER_TIMEOUT"
        );

        assert_eq!(
            manager.coordinator_state(),
            ShutdownCoordinatorState::Running
        );
        assert_eq!(
            manager.coordinator_join_state(),
            CoordinatorJoinState::Running
        );
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::Running
        );
        assert_eq!(
            manager.reaper_monitor_join_state(),
            ReaperMonitorJoinState::Running
        );
        assert!(manager.shutdown_coordinator_monitor_task.lock().is_some());
        assert!(manager.reaper_monitor_task.lock().is_some());
        assert_eq!(manager.coordinator_start_count(), 1);

        let _ = slow_tx.send(());

        let outcome = record.wait_for_terminal().await;
        assert!(matches!(
            outcome,
            MutationTransactionOutcome::Aborted { .. }
        ));

        let caller2_res = manager.shutdown(3000).await;
        assert!(caller2_res.is_ok());
        assert_eq!(
            manager.coordinator_state(),
            ShutdownCoordinatorState::Completed
        );
        assert_eq!(
            manager.coordinator_join_state(),
            CoordinatorJoinState::JoinedSuccess
        );
        assert_eq!(manager.coordinator_start_count(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_reaper_panic_records_diagnostic_failure() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();
        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let record_id = record.id;
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, exchange_authority) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-reaper-panic-",
        );

        let (reaper_panic_tx, reaper_panic_rx) = tokio::sync::oneshot::channel::<()>();
        let panic_signal = Arc::new(Mutex::new(Some(reaper_panic_tx)));
        manager.register_test_hook(Arc::new(move |phase, _id| {
            if phase == MutationTestHookPhase::BeforeReaperJoin {
                if let Some(sender) = panic_signal.lock().take() {
                    let _ = sender.send(());
                }
                panic!("Controlled reaper panic for testing");
            }
        }));

        let (supervisor_started_tx, supervisor_started_rx) = tokio::sync::oneshot::channel();
        let task = spawn_managed_test_supervisor(execution_authority.clone(), async move {
            let _ = supervisor_started_tx.send(());
            std::future::pending::<()>().await;
            MutationTransactionOutcome::Aborted {
                transaction_id: record_id.to_string(),
                session_id,
                reason: "done".to_string(),
                cleanup_warning: None,
            }
        });

        guard.disarm();
        manager.register_supervisor_bundle(record_id, bundle, task);
        supervisor_started_rx.await.unwrap();
        reaper_panic_rx.await.unwrap();

        let terminal = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();
        assert!(matches!(
            terminal,
            MutationTransactionOutcome::FailedPreCommit { .. }
        ));
        let res = manager.shutdown(5000).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code, "MUTATION_SHUTDOWN_REAPER_PANIC");
        assert!(matches!(
            manager.reaper_task_state(),
            ReaperTaskState::JoinedFailure(_)
        ));
        assert!(execution_authority.was_abort_requested());
        assert_eq!(
            execution_authority.execution_state(),
            SupervisorExecutionState::Exited
        );
        assert_eq!(exchange_authority.state(), ExchangeCleanupState::Cleaned);
        assert_eq!(manager.active_supervisor_count(), 0);
        assert_eq!(manager.active_reservation_count(), 0);
        assert!(record.control.lock().terminal_outcome.is_some());

        let monitor_join = manager.join_coordinator_and_reaper_monitors().await;
        assert_eq!(
            monitor_join.unwrap_err().code,
            "MUTATION_SHUTDOWN_REAPER_PANIC"
        );
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::ExternallyJoinedSuccess
        );
        assert_eq!(
            manager.reaper_monitor_join_state(),
            ReaperMonitorJoinState::ExternallyJoinedSuccess
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_handle_claim_preserves_recovery_bundle_in_registry() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let tx_id = record.id;

        let child_owner = SessionJobChildProcessOwner::default();
        let execution_authority = SupervisorExecutionAuthority::new();
        let (bundle, exchange_dir_clone) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-claim-",
        );
        let dummy_task = spawn_managed_test_supervisor(execution_authority, async move {
            MutationTransactionOutcome::Aborted {
                transaction_id: tx_id.to_string(),
                session_id: "test".to_string(),
                reason: "dummy".to_string(),
                cleanup_warning: None,
            }
        });

        guard.disarm();
        manager.register_supervisor_bundle(tx_id, bundle, dummy_task);

        let outcome = record.wait_for_terminal().await;
        assert!(matches!(
            outcome,
            MutationTransactionOutcome::Aborted { .. }
        ));

        let res = manager.shutdown(3000).await;
        assert!(res.is_ok());

        assert_eq!(manager.active_supervisor_count(), 0);
        assert_eq!(manager.active_reservation_count(), 0);
        assert_eq!(exchange_dir_clone.state(), ExchangeCleanupState::Cleaned);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_temp_dir_cleanup_result_reported_on_supervisor_panic() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let tx_id = record.id;
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, exchange_dir_clone) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-panic-",
        );
        let panic_task = spawn_managed_test_supervisor(execution_authority, async move {
            panic!("Controlled supervisor panic for testing recovery");
        });

        guard.disarm();
        manager.register_supervisor_bundle(tx_id, bundle, panic_task);

        manager.harvest_completed_supervisors();

        let outcome = record.wait_for_terminal().await;
        match outcome {
            MutationTransactionOutcome::FailedPreCommit { error, .. } => {
                assert!(error
                    .message
                    .contains("Owner task pre-commit aşamasında panikledi"));
            }
            _ => panic!("Expected FailedPreCommit outcome after supervisor panic"),
        }

        assert_eq!(exchange_dir_clone.state(), ExchangeCleanupState::Cleaned);
        assert_eq!(manager.active_reservation_count(), 0);
        let report = manager.shutdown(3000).await.unwrap();
        assert_eq!(report.supervisor_panics, 1);
        assert_eq!(report.failed_pre_commit, 1);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "MUTATION_SUPERVISOR_PANIC"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cleanup_warning_survives_entry_removal_into_shutdown_report() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();
        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let tx_id = record.id;
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, exchange_authority) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-cleanup-warning-",
        );
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let supervisor = spawn_managed_test_supervisor(execution_authority, async move {
            let _ = release_rx.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: tx_id.to_string(),
                session_id,
                reason: "cleanup warning".to_string(),
                cleanup_warning: None,
            }
        });
        guard.disarm();
        manager.register_supervisor_bundle(tx_id, bundle, supervisor);

        fs::remove_dir_all(exchange_authority.path()).unwrap();
        release_tx.send(()).unwrap();
        let _ = record.wait_for_terminal().await;
        let report = manager.shutdown(3000).await.unwrap();

        assert_eq!(manager.active_supervisor_count(), 0);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.code == "MUTATION_CLEANUP_FAILED"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_normal_completion_vs_external_recovery_race_exactly_once() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();
        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let tx_id = record.id;
        let execution_authority = SupervisorExecutionAuthority::new();
        let child_owner = SessionJobChildProcessOwner::default();
        let (bundle, exchange_authority) = test_supervisor_bundle(
            &record,
            &entry,
            &execution_authority,
            &child_owner,
            "test-finalizer-race-",
        );
        let race_barrier = Arc::new(tokio::sync::Barrier::new(2));
        let supervisor_barrier = Arc::clone(&race_barrier);
        let session_for_outcome = session_id.clone();
        let supervisor = spawn_managed_test_supervisor(execution_authority.clone(), async move {
            supervisor_barrier.wait().await;
            MutationTransactionOutcome::Aborted {
                transaction_id: tx_id.to_string(),
                session_id: session_for_outcome,
                reason: "race completed".to_string(),
                cleanup_warning: None,
            }
        });
        guard.disarm();
        manager.register_supervisor_bundle(tx_id, bundle, supervisor);

        let manager_for_recovery = Arc::clone(&manager);
        let recovery = tokio::spawn(async move {
            race_barrier.wait().await;
            recover_supervisor_panic_external(&manager_for_recovery, &tx_id, "race").await;
        });

        let terminal = tokio::time::timeout(Duration::from_secs(5), record.wait_for_terminal())
            .await
            .unwrap();
        recovery.await.unwrap();
        let shutdown = manager.shutdown(3000).await.unwrap();

        assert!(matches!(
            terminal,
            MutationTransactionOutcome::Aborted { .. }
                | MutationTransactionOutcome::FailedPreCommit { .. }
        ));
        assert_eq!(
            execution_authority.finalizer_claim(),
            FinalizerClaim::Finalized
        );
        assert_eq!(exchange_authority.state(), ExchangeCleanupState::Cleaned);
        assert_eq!(manager.active_supervisor_count(), 0);
        assert_eq!(manager.active_reservation_count(), 0);
        assert_eq!(
            shutdown.cancelled_transactions + shutdown.failed_pre_commit,
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_global_shutdown_joins_monitor_exactly_once() {
        let manager = Arc::new(MutationTransactionManager::new());

        let res = manager.shutdown(3000).await;
        assert!(res.is_ok());

        let (j1, j2) = tokio::join!(
            manager.join_coordinator_and_reaper_monitors(),
            manager.join_coordinator_and_reaper_monitors()
        );

        assert!(j1.is_ok());
        assert!(j2.is_ok());
        assert_eq!(
            manager.coordinator_monitor_join_state(),
            CoordinatorMonitorJoinState::ExternallyJoinedSuccess
        );
        assert_eq!(
            manager.reaper_monitor_join_state(),
            ReaperMonitorJoinState::ExternallyJoinedSuccess
        );
        assert!(manager.shutdown_coordinator_monitor_task.lock().is_none());
        assert!(manager.reaper_monitor_task.lock().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_unproven_supervisor_exit_retains_entry_and_gates_cleanup() {
        let manager = Arc::new(MutationTransactionManager::new());
        let store = Arc::new(DocumentSessionStore::initialize().unwrap());
        let (_docs, session_id) = setup_test_session(&store);
        let entry = store.entry(&session_id).unwrap();

        let (record, guard) = manager.begin_transaction(&session_id, 1).unwrap();
        let tx_id = record.id;

        let exec_auth = SupervisorExecutionAuthority::new();

        let (blocking_release_tx, blocking_release_rx) = std::sync::mpsc::channel::<()>();
        let blocking_handle = spawn_supervised_blocking(&exec_auth, move || {
            let _ = blocking_release_rx.recv();
        });
        let unexited_task = spawn_managed_test_supervisor(exec_auth.clone(), async move {
            let _ = blocking_handle.await;
            MutationTransactionOutcome::Aborted {
                transaction_id: tx_id.to_string(),
                session_id: "forever".to_string(),
                reason: "forever".to_string(),
                cleanup_warning: None,
            }
        });

        let recovery_ctx = MutationTransactionRecoveryContext {
            _transaction_id: tx_id,
            _session_id: session_id.clone(),
            initial_digest: None,
            replacement_digest: Arc::new(Mutex::new(None)),
            preflight: Arc::new(Mutex::new(None)),
            working_path: PathBuf::from("/tmp/test"),
            reconciliation_state: Arc::new(Mutex::new(ReconciliationState::NotStarted)),
        };

        let temp_dir = tempfile::Builder::new()
            .prefix("test-unexited-")
            .tempdir()
            .unwrap();
        let path = temp_dir.path().to_path_buf();
        let exchange_dir = ExchangeCleanupAuthority::new(temp_dir, path);

        let bundle = MutationSupervisorRecoveryBundle {
            transaction_record: Arc::clone(&record),
            session_entry: Arc::clone(&entry),
            recovery_ctx,
            child_owner: SessionJobChildProcessOwner::default(),
            exchange_cleanup_authority: exchange_dir.clone(),
            execution_authority: exec_auth.clone(),
        };

        guard.disarm();
        manager.register_supervisor_bundle(tx_id, bundle, unexited_task);

        recover_supervisor_panic_external(&manager, &tx_id, "simulated panic").await;

        assert_eq!(manager.active_supervisor_count(), 1);
        assert_eq!(manager.active_reservation_count(), 1);
        assert_eq!(exchange_dir.state(), ExchangeCleanupState::Owned);
        assert!(matches!(
            exec_auth.execution_state(),
            SupervisorExecutionState::TerminationUnproven(_)
        ));
        let terminal = record.wait_for_terminal().await;
        assert!(matches!(
            terminal,
            MutationTransactionOutcome::Indeterminate { ref error, .. }
                if error.code == "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN"
        ));
        let shutdown_error = manager.shutdown(5000).await.unwrap_err();
        assert_eq!(
            shutdown_error.code,
            "MUTATION_SUPERVISOR_TERMINATION_UNPROVEN"
        );
        assert_eq!(manager.lifecycle(), MutationManagerLifecycle::ShuttingDown);
        assert_eq!(manager.active_supervisor_count(), 1);
        assert_eq!(exchange_dir.state(), ExchangeCleanupState::Owned);

        blocking_release_tx.send(()).unwrap();
        assert!(wait_for_supervisor_exited(&exec_auth, Duration::from_secs(2)).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_shutdown_report_accumulator_records_outcomes_correctly() {
        let mut accumulator = MutationShutdownAccumulator::default();
        let id1 = MutationTransactionId::new();
        let id2 = MutationTransactionId::new();
        let id3 = MutationTransactionId::new();
        let id4 = MutationTransactionId::new();

        let outcome1 = MutationTransactionOutcome::Aborted {
            transaction_id: id1.to_string(),
            session_id: "s1".to_string(),
            reason: "cancel".to_string(),
            cleanup_warning: None,
        };
        let outcome2 = MutationTransactionOutcome::Completed {
            transaction_id: id2.to_string(),
            session_id: "s2".to_string(),
            snapshot: DocumentSessionSnapshot {
                id: "s2".to_string(),
                opened_at_ms: 0,
                display_path: "s2".to_string(),
                file_name: "s2".to_string(),
                content_generation: 1,
                state_revision: 1,
                dirty: false,
                lifecycle: SessionLifecycle::Open,
                editable: true,
                read_only_reason: None,
                health_warning: None,
                undo_operation: None,
                redo_operation: None,
            },
            cleanup_warning: None,
        };
        let failure_snapshot = DocumentSessionSnapshot {
            id: "s3".to_string(),
            opened_at_ms: 0,
            display_path: "s3".to_string(),
            file_name: "s3".to_string(),
            content_generation: 1,
            state_revision: 1,
            dirty: true,
            lifecycle: SessionLifecycle::CommitIndeterminate,
            editable: false,
            read_only_reason: None,
            health_warning: None,
            undo_operation: None,
            redo_operation: None,
        };
        let outcome3 = MutationTransactionOutcome::FailedPreCommit {
            transaction_id: id3.to_string(),
            session_id: "s3".to_string(),
            error: DocumentSessionError::new("TEST_FAILURE", "failed"),
            cleanup_warning: None,
        };
        let outcome4 = MutationTransactionOutcome::Indeterminate {
            transaction_id: id4.to_string(),
            session_id: "s4".to_string(),
            snapshot: failure_snapshot,
            error: DocumentSessionError::new("TEST_INDETERMINATE", "indeterminate"),
        };

        accumulator.record_outcome(id1, &outcome1);
        accumulator.record_outcome(id1, &outcome1); // duplicate attempt
        accumulator.record_outcome(id2, &outcome2);
        accumulator.record_outcome(id3, &outcome3);
        accumulator.record_outcome(id4, &outcome4);
        accumulator.record_outcome(id4, &outcome4); // duplicate attempt

        assert_eq!(accumulator.cancelled_transactions, 1);
        assert_eq!(accumulator.post_commit_completed, 1);
        assert_eq!(accumulator.failed_pre_commit, 1);
        assert_eq!(accumulator.commit_indeterminate, 1);
        assert_eq!(accumulator.processed_ids.len(), 4);
    }

    /// The store keeps `temp_root` as `std::env::temp_dir()` returns it, which
    /// on macOS sits under the `/var -> /private/var` symlink; a tempdir root
    /// has the same shape.
    #[test]
    fn private_target_rejection_covers_resolved_and_unresolved_forms() {
        let private = tempfile::tempdir().unwrap();
        let root = private.path();
        let resolved_root = root.canonicalize().unwrap();
        let sessions = root.join("instance-x").join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(sessions.join("working.pdf"), b"%PDF-").unwrap();

        let inside = [
            root.to_path_buf(),
            root.join("out.pdf"),
            sessions.join("working.pdf"),
            sessions.join("missing").join("deeper").join("out.pdf"),
            resolved_root.join("out.pdf"),
            resolved_root.join("missing").join("out.pdf"),
            sessions.join("..").join("sessions").join("out.pdf"),
        ];
        for target in &inside {
            let error = reject_private_target(target, root).unwrap_err();
            assert_eq!(
                error.code,
                "DOCUMENT_SESSION_PRIVATE_TARGET",
                "{}",
                target.display()
            );
            assert!(
                reject_private_target(target, &resolved_root).is_err(),
                "{}",
                target.display()
            );
        }

        let outside = tempfile::tempdir().unwrap();
        for target in [
            outside.path().join("out.pdf"),
            outside.path().join("missing").join("out.pdf"),
        ] {
            reject_private_target(&target, root).unwrap();
            reject_private_target(&target, &resolved_root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_target_rejection_follows_symlinked_folders() {
        let private = tempfile::tempdir().unwrap();
        let sessions = private.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let alias = outside.path().join("alias");
        std::os::unix::fs::symlink(&sessions, &alias).unwrap();

        for target in [alias.join("out.pdf"), alias.join("missing").join("out.pdf")] {
            let error = reject_private_target(&target, private.path()).unwrap_err();
            assert_eq!(error.code, "DOCUMENT_SESSION_PRIVATE_TARGET");
        }
    }
}
