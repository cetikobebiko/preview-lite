//! Toplu işlem (batch processing): one copy-producing operation applied to many
//! PDFs, one file at a time.
//!
//! Each input is copied with `copy_source_consistent` into a private
//! `<instance>/batches/batch-<id>/file-<n>/` directory and opened only by the
//! ephemeral job worker; the user's path never reaches a worker. The runner
//! never creates a document session, so the active session, its generation,
//! the identity epoch, recent files and plugin hooks are untouched. Outputs are
//! published per file with `accept_job_output_set` (atomic, no-clobber unless
//! overwrite was chosen, rolled back on cancellation). A crashed run is removed
//! with its orphaned instance directory on the next start.

mod input;
mod output;

pub use input::{BatchInputFile, BatchInputRejection, BatchInputScan, BatchReason};
pub use output::ConflictPolicy;

use crate::document_session::{
    DocumentSessionError, DocumentSessionStore, SessionJobCancellationCheckpoint,
    SessionJobChildProcessOwner, SessionJobContext, SessionJobOperationError,
};
use crate::job_worker::{JobCommand, JobResult};
use crate::output_names::PlannedOutput;
use input::{AdmittedInput, PrivateRoots};
use output::{NamePlan, ProtectedTargets, Resolution};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const BATCH_PROGRESS_EVENT: &str = "batch_progress";
const INSPECT_TIMEOUT: Duration = Duration::from_secs(60);
const PUBLISH_ATTEMPTS: u32 = 3;
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextExportFormat {
    Txt,
    Md,
}

impl TextExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Md => "md",
        }
    }

    /// The `ExtractText` format string the text export modal sends.
    fn worker_format(self) -> &'static str {
        match self {
            Self::Txt => "text",
            Self::Md => "markdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageExportFormat {
    Png,
    Jpeg,
}

impl ImageExportFormat {
    fn request(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

/// The v1 operations. Each produces copies; inputs are never modified. The
/// derived `Debug` is safe to log: `ProtectionSettings` redacts its passwords.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchOperation {
    Optimize {
        settings: crate::pdf_optimizer::OptimizeSettings,
    },
    ExportDocument {
        format: crate::document_export::DocumentExportFormat,
    },
    ExportText {
        format: TextExportFormat,
    },
    ExportImages {
        format: ImageExportFormat,
        dpi: f32,
    },
    Security {
        op: crate::document_security::SecurityOp,
    },
}

impl BatchOperation {
    fn kind(&self) -> &'static str {
        match self {
            Self::Optimize { .. } => "optimize",
            Self::ExportDocument { .. } => "export_document",
            Self::ExportText { .. } => "export_text",
            Self::ExportImages { .. } => "export_images",
            Self::Security { .. } => "security",
        }
    }

    fn validate(&self) -> Result<(), DocumentSessionError> {
        match self {
            Self::Optimize { settings } => crate::pdf_optimizer::validate_request(settings)
                .map_err(|message| DocumentSessionError::new("SESSION_OPTIMIZE_INVALID", message)),
            Self::Security { op } => crate::document_security::validate_request(op)
                .map_err(|message| DocumentSessionError::new("SESSION_SECURITY_INVALID", message)),
            Self::ExportImages { dpi, .. } => {
                crate::export::validate_export_dpi(*dpi).map_err(|error| match error {
                    SessionJobOperationError::Failed(failure) => {
                        DocumentSessionError::new(failure.code, failure.message)
                    }
                    SessionJobOperationError::Cancelled(_) => {
                        DocumentSessionError::new("BATCH_INVALID_REQUEST", "Geçersiz istek")
                    }
                })
            }
            Self::ExportDocument { .. } | Self::ExportText { .. } => Ok(()),
        }
    }

    /// Optimizer and security copies are written from a PDFium copy without the
    /// original encryption, so on an owner-password file they would drop the
    /// owner's restrictions. A batch does not do that in bulk.
    fn refuses_owner_restricted(&self) -> bool {
        matches!(self, Self::Optimize { .. } | Self::Security { .. })
    }

    fn name_plan(&self, stem: &str, pages: &[u32]) -> NamePlan {
        let single = |base: String, extension: &str| NamePlan::Single {
            base,
            extension: extension.to_string(),
        };
        match self {
            Self::Optimize { .. } => single(format!("{stem}_küçültülmüş"), "pdf"),
            Self::Security {
                op: crate::document_security::SecurityOp::Protect { .. },
            } => single(format!("{stem}_korumalı"), "pdf"),
            Self::Security {
                op: crate::document_security::SecurityOp::Remove,
            } => single(format!("{stem}_korumasız"), "pdf"),
            Self::ExportDocument { format } => single(stem.to_string(), format.extension()),
            Self::ExportText { format } => single(stem.to_string(), format.extension()),
            Self::ExportImages { format, .. } => NamePlan::Pages {
                stem: stem.to_string(),
                pages: pages.to_vec(),
                extension: format.extension().to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchRequest {
    pub inputs: Vec<String>,
    pub operation: BatchOperation,
    pub output_dir: String,
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchFileState {
    Pending,
    Running,
    Completed,
    Skipped,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFileStatus {
    pub index: usize,
    pub input_path: String,
    pub state: BatchFileState,
    pub outputs: Vec<String>,
    pub reason: Option<BatchReason>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchCounts {
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub cancelled: usize,
}

/// Everything the frontend may know about a batch. Operation settings (and so
/// any password) are never part of it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchStatus {
    pub batch_id: String,
    pub state: BatchState,
    pub operation: String,
    pub output_dir: String,
    pub total: usize,
    pub last_sequence: u64,
    pub counts: BatchCounts,
    pub files: Vec<BatchFileStatus>,
    pub warnings: Vec<String>,
    pub reason: Option<BatchReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchEventKind {
    FileStarted,
    FileCompleted,
    FileSkipped,
    FileFailed,
    BatchCompleted,
    BatchCancelled,
    BatchFailed,
}

impl BatchEventKind {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::BatchCompleted | Self::BatchCancelled | Self::BatchFailed
        )
    }
}

/// One `batch_progress` event. Sequence numbers start at 1 and increase by one;
/// exactly one event per batch has `terminal: true`, and it is the last.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgressEvent {
    pub batch_id: String,
    pub sequence: u64,
    pub kind: BatchEventKind,
    pub terminal: bool,
    pub total: usize,
    pub counts: BatchCounts,
    pub file: Option<BatchFileStatus>,
    /// Terminal events only: the final batch status.
    pub status: Option<BatchStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchStarted {
    pub batch_id: String,
}

pub(crate) trait BatchEventSink: Send + Sync {
    fn emit(&self, event: &BatchProgressEvent);
}

struct TauriEventSink(tauri::AppHandle);

impl BatchEventSink for TauriEventSink {
    fn emit(&self, event: &BatchProgressEvent) {
        use tauri::Emitter;
        if let Err(error) = self.0.emit(BATCH_PROGRESS_EVENT, event) {
            tracing::warn!(target: "batch", "batch_progress emit failed: {error}");
        }
    }
}

struct RecordState {
    status: BatchStatus,
    terminal_sent: bool,
}

struct BatchRecord {
    id: String,
    output_dir: PathBuf,
    token: CancellationToken,
    children: SessionJobChildProcessOwner,
    state: Mutex<RecordState>,
    /// Held while one event is built and emitted, so events leave in sequence
    /// order while the state lock stays free for status reads and cancellation.
    emit_order: Mutex<()>,
    sink: Arc<dyn BatchEventSink>,
    done: tokio::sync::watch::Sender<bool>,
}

fn recount(status: &mut BatchStatus) {
    let mut counts = BatchCounts::default();
    for file in &status.files {
        match file.state {
            BatchFileState::Completed => counts.completed += 1,
            BatchFileState::Skipped => counts.skipped += 1,
            BatchFileState::Failed => counts.failed += 1,
            BatchFileState::Cancelled => counts.cancelled += 1,
            BatchFileState::Pending | BatchFileState::Running => {}
        }
    }
    status.counts = counts;
}

impl BatchRecord {
    fn is_running(&self) -> bool {
        !self.state.lock().terminal_sent
    }

    fn status(&self) -> BatchStatus {
        self.state.lock().status.clone()
    }

    /// Applies `change` and emits one event under the same lock, so status and
    /// event sequence never disagree. Nothing is emitted after the terminal event.
    fn publish(
        &self,
        kind: BatchEventKind,
        file_index: Option<usize>,
        change: impl FnOnce(&mut BatchStatus),
    ) {
        let _order = self.emit_order.lock();
        let event = {
            let mut guard = self.state.lock();
            if guard.terminal_sent {
                return;
            }
            change(&mut guard.status);
            recount(&mut guard.status);
            guard.status.last_sequence += 1;
            let terminal = kind.is_terminal();
            if terminal {
                guard.terminal_sent = true;
            }
            let status = &guard.status;
            BatchProgressEvent {
                batch_id: status.batch_id.clone(),
                sequence: status.last_sequence,
                kind,
                terminal,
                total: status.total,
                counts: status.counts,
                file: file_index.and_then(|index| status.files.get(index).cloned()),
                status: terminal.then(|| status.clone()),
            }
        };
        self.sink.emit(&event);
    }

    fn file_event(
        &self,
        index: usize,
        state: BatchFileState,
        outputs: Vec<String>,
        reason: Option<BatchReason>,
    ) {
        let kind = match state {
            BatchFileState::Running => BatchEventKind::FileStarted,
            BatchFileState::Completed => BatchEventKind::FileCompleted,
            BatchFileState::Skipped => BatchEventKind::FileSkipped,
            BatchFileState::Failed => BatchEventKind::FileFailed,
            BatchFileState::Pending | BatchFileState::Cancelled => return,
        };
        self.publish(kind, Some(index), |status| {
            if let Some(file) = status.files.get_mut(index) {
                file.state = state;
                file.outputs = outputs;
                file.reason = reason;
            }
        });
    }

    fn finish(&self, state: BatchState, reason: Option<BatchReason>, warnings: Vec<String>) {
        let kind = match state {
            BatchState::Completed | BatchState::Running => BatchEventKind::BatchCompleted,
            BatchState::Cancelled => BatchEventKind::BatchCancelled,
            BatchState::Failed => BatchEventKind::BatchFailed,
        };
        self.publish(kind, None, |status| {
            status.state = state;
            status.reason = reason;
            status.warnings.extend(warnings);
            for file in &mut status.files {
                if matches!(
                    file.state,
                    BatchFileState::Pending | BatchFileState::Running
                ) {
                    file.state = BatchFileState::Cancelled;
                }
            }
        });
        self.done.send_replace(true);
    }
}

/// Paths the runner needs from the session store, kept apart so tests can run
/// without a Tauri app.
#[derive(Clone, Debug)]
pub(crate) struct BatchEnvironment {
    pub(crate) batches_root: PathBuf,
    pub(crate) private: PrivateRoots,
}

impl BatchEnvironment {
    pub(crate) fn new(store: &DocumentSessionStore, app_data_dir: Option<PathBuf>) -> Self {
        Self {
            batches_root: store.batches_root(),
            private: PrivateRoots::new(
                std::iter::once(store.temp_root.clone()).chain(app_data_dir),
            ),
        }
    }
}

struct PreparedInput {
    display_path: String,
    admitted: Result<AdmittedInput, BatchReason>,
}

struct Prepared {
    inputs: Vec<PreparedInput>,
    output_dir: PathBuf,
    identities: Vec<String>,
}

fn invalid(code: &str, message: impl Into<String>) -> DocumentSessionError {
    DocumentSessionError::new(code, message)
}

fn prepare(
    request: &BatchRequest,
    private: &PrivateRoots,
) -> Result<Prepared, DocumentSessionError> {
    if request.inputs.is_empty() {
        return Err(invalid("BATCH_EMPTY", "İşlenecek dosya seçilmedi"));
    }
    if request.inputs.len() > input::MAX_BATCH_FILES {
        return Err(invalid(
            "BATCH_TOO_MANY_FILES",
            format!(
                "Bir toplu işlem en fazla {} dosya alır",
                input::MAX_BATCH_FILES
            ),
        ));
    }
    request.operation.validate()?;

    let output_dir = Path::new(&request.output_dir)
        .canonicalize()
        .map_err(|error| {
            invalid(
                "BATCH_OUTPUT_DIR_INVALID",
                format!("Çıktı klasörü bulunamadı: {error}"),
            )
        })?;
    if !output_dir.is_dir() {
        return Err(invalid(
            "BATCH_OUTPUT_DIR_INVALID",
            "Çıktı yolu bir klasör değil",
        ));
    }
    if private.contains(&output_dir) {
        return Err(invalid(
            "BATCH_OUTPUT_DIR_PRIVATE",
            "Çıktı klasörü uygulamanın özel alanında olamaz",
        ));
    }

    let mut seen = HashSet::new();
    let mut total: u64 = 0;
    let mut identities = Vec::new();
    let inputs = request
        .inputs
        .iter()
        .map(|raw| {
            let admitted = input::admit_input_file(Path::new(raw), private).and_then(|input| {
                if seen.insert(input.identity.clone()) {
                    Ok(input)
                } else {
                    Err(BatchReason::new(
                        "BATCH_INPUT_DUPLICATE",
                        "Aynı dosya listede birden fazla kez var",
                    ))
                }
            });
            if let Ok(input) = &admitted {
                total = total.saturating_add(input.size);
                identities.push(input.identity.clone());
            }
            PreparedInput {
                display_path: admitted
                    .as_ref()
                    .map(|input| input.path.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| raw.clone()),
                admitted,
            }
        })
        .collect::<Vec<_>>();
    if total > input::MAX_BATCH_TOTAL_BYTES {
        return Err(invalid(
            "BATCH_TOTAL_TOO_LARGE",
            format!(
                "Seçilen dosyaların toplamı {} GiB sınırını aşıyor",
                input::MAX_BATCH_TOTAL_BYTES / (1024 * 1024 * 1024)
            ),
        ));
    }
    Ok(Prepared {
        inputs,
        output_dir,
        identities,
    })
}

enum FileOutcome {
    Completed(Vec<String>),
    Skipped(BatchReason),
    Failed(BatchReason),
    Cancelled,
}

enum Produced {
    Single(PathBuf),
    Pages(Vec<PathBuf>),
}

fn job_error(error: DocumentSessionError) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(error.code, error.message)
}

struct FileTask<'a> {
    record: &'a BatchRecord,
    input: &'a AdmittedInput,
    operation: &'a BatchOperation,
    output_dir: &'a Path,
    policy: ConflictPolicy,
    file_dir: &'a Path,
}

fn process_file(task: &FileTask<'_>, protected: &mut ProtectedTargets) -> FileOutcome {
    match try_process_file(task, protected) {
        Ok(outcome) => outcome,
        Err(SessionJobOperationError::Cancelled(_)) => FileOutcome::Cancelled,
        Err(SessionJobOperationError::Failed(failure)) => {
            let reason = BatchReason::new(failure.code, failure.message);
            if reason.code == crate::job_worker::BATCH_INPUT_PASSWORD_REQUIRED {
                FileOutcome::Skipped(reason)
            } else {
                FileOutcome::Failed(reason)
            }
        }
    }
}

fn output_exists_reason() -> BatchReason {
    BatchReason::new(
        "BATCH_OUTPUT_EXISTS",
        "Çıktı klasöründe aynı adlı dosya var; atlama seçildi",
    )
}

fn try_process_file(
    task: &FileTask<'_>,
    protected: &mut ProtectedTargets,
) -> Result<FileOutcome, SessionJobOperationError> {
    crate::document_session::create_secure_dir(task.file_dir).map_err(job_error)?;
    let snapshot_path = task.file_dir.join("snapshot.pdf");
    crate::document_session::copy_source_consistent(&task.input.path, &snapshot_path)
        .map_err(job_error)?;
    let context = SessionJobContext {
        job_id: Uuid::new_v4(),
        owner_session_id: format!("batch-{}", task.record.id),
        captured_content_generation: 0,
        snapshot_path,
        credential: None,
        cancellation: SessionJobCancellationCheckpoint::new(task.record.token.clone()),
        child_processes: task.record.children.clone(),
    };
    let (page_count, restricted) =
        match crate::job_worker::run_job(&context, JobCommand::InspectBatchInput, INSPECT_TIMEOUT)?
        {
            JobResult::BatchInputInspection {
                page_count,
                protected,
            } => (page_count, protected),
            _ => {
                return Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Girdi inceleme worker'ı beklenmeyen sonuç döndürdü",
                ))
            }
        };
    if restricted && task.operation.refuses_owner_restricted() {
        return Ok(FileOutcome::Skipped(BatchReason::new(
            "BATCH_INPUT_OWNER_RESTRICTED",
            "Belgede sahip parolası kısıtlaması var; bu işlem kısıtlamaları kaldıracağı için atlandı",
        )));
    }

    let file_name = task
        .input
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = crate::output_names::output_name_stem(&file_name);
    let pages: Vec<u32> = (0..page_count).collect();
    let plan = task.operation.name_plan(&stem, &pages);
    if task.policy == ConflictPolicy::Skip
        && output::resolve(&plan, task.output_dir, task.policy, protected, None)
            == Some(Resolution::Skip)
    {
        return Ok(FileOutcome::Skipped(output_exists_reason()));
    }

    let produced = match task.operation {
        BatchOperation::Optimize { settings } => {
            Produced::Single(crate::copy_jobs::produce_optimized_copy(&context, *settings)?.0)
        }
        BatchOperation::Security { op } => Produced::Single(
            crate::copy_jobs::produce_security_copy(&context, op.clone())?,
        ),
        BatchOperation::ExportDocument { format } => {
            Produced::Single(crate::copy_jobs::produce_document_export(
                &context,
                *format,
                stem.clone(),
                pages.clone(),
            )?)
        }
        BatchOperation::ExportText { format } => {
            let text =
                crate::copy_jobs::produce_text(&context, None, format.worker_format().to_string())?;
            context.cancellation.check_cancelled()?;
            let path = task
                .file_dir
                .join(format!("text-export.{}", format.extension()));
            std::fs::write(&path, text).map_err(|error| {
                SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_OUTPUT_WRITE_FAILED",
                    format!("Metin çıktısı yazılamadı: {error}"),
                )
            })?;
            Produced::Single(path)
        }
        BatchOperation::ExportImages { format, dpi } => {
            Produced::Pages(crate::copy_jobs::produce_page_images(
                &context,
                pages.clone(),
                format.request().to_string(),
                *dpi,
            )?)
        }
    };
    let sources = match &produced {
        Produced::Single(path) => vec![crate::job_worker::validate_output_file(&context, path)?],
        Produced::Pages(paths) => paths.clone(),
    };

    let mut after = None;
    for _ in 0..PUBLISH_ATTEMPTS {
        context.cancellation.check_cancelled()?;
        let Some(resolution) =
            output::resolve(&plan, task.output_dir, task.policy, protected, after)
        else {
            return Ok(FileOutcome::Failed(BatchReason::new(
                "BATCH_OUTPUT_NAMES_EXHAUSTED",
                "Çıktı için boş bir ad bulunamadı",
            )));
        };
        let Resolution::Publish {
            number,
            names,
            overwrite,
        } = resolution
        else {
            return Ok(FileOutcome::Skipped(output_exists_reason()));
        };
        let planned = match (&produced, task.operation) {
            (Produced::Pages(_), BatchOperation::ExportImages { format, .. }) => {
                crate::export::planned_export_image_outputs(
                    &pages,
                    format.request(),
                    &plan.stem_for(number),
                )
            }
            _ => sources
                .iter()
                .zip(&names)
                .map(|(source, target_name)| PlannedOutput {
                    worker_name: source
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    target_name: target_name.clone(),
                })
                .collect(),
        };
        match crate::document_session::accept_job_output_set(
            &context,
            sources.clone(),
            &planned,
            task.output_dir,
            overwrite,
        ) {
            Ok(published) => {
                protected.record_published(published.iter().map(PathBuf::from));
                return Ok(FileOutcome::Completed(published));
            }
            Err(SessionJobOperationError::Failed(failure))
                if failure.code == crate::document_session::SESSION_JOB_OUTPUT_CONFLICT_CODE =>
            {
                // Another writer took a name between the check and the publish.
                if task.policy == ConflictPolicy::Skip {
                    return Ok(FileOutcome::Skipped(output_exists_reason()));
                }
                after = Some(number.unwrap_or(1));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(FileOutcome::Failed(BatchReason::new(
        "BATCH_OUTPUT_CONFLICT",
        "Çıktı adı başka bir yazımla çakışmaya devam etti",
    )))
}

fn remove_dir_if_present(path: &Path, warnings: &mut Vec<String>) {
    if std::fs::symlink_metadata(path).is_err() {
        return;
    }
    if let Err(error) = crate::document_session::secure_remove_tree(path) {
        warnings.push(format!(
            "Geçici toplu işlem dizini silinemedi ({}); bir sonraki toplu işlemde yeniden denenecek",
            error.code
        ));
    }
}

fn run_batch(
    record: Arc<BatchRecord>,
    prepared: Prepared,
    operation: BatchOperation,
    policy: ConflictPolicy,
    batch_dir: PathBuf,
) {
    let mut warnings = Vec::new();
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut protected = ProtectedTargets::new(prepared.identities.iter().cloned());
        for (index, entry) in prepared.inputs.iter().enumerate() {
            if record.token.is_cancelled() {
                break;
            }
            let input = match &entry.admitted {
                Ok(input) => input,
                Err(reason) => {
                    record.file_event(
                        index,
                        BatchFileState::Skipped,
                        Vec::new(),
                        Some(reason.clone()),
                    );
                    continue;
                }
            };
            record.file_event(index, BatchFileState::Running, Vec::new(), None);
            let file_dir = batch_dir.join(format!("file-{index}"));
            let task = FileTask {
                record: &record,
                input,
                operation: &operation,
                output_dir: &prepared.output_dir,
                policy,
                file_dir: &file_dir,
            };
            let outcome = process_file(&task, &mut protected);
            remove_dir_if_present(&file_dir, &mut warnings);
            match outcome {
                FileOutcome::Completed(outputs) => {
                    record.file_event(index, BatchFileState::Completed, outputs, None)
                }
                FileOutcome::Skipped(reason) => {
                    record.file_event(index, BatchFileState::Skipped, Vec::new(), Some(reason))
                }
                FileOutcome::Failed(reason) => {
                    record.file_event(index, BatchFileState::Failed, Vec::new(), Some(reason))
                }
                FileOutcome::Cancelled => break,
            }
        }
    }));
    remove_dir_if_present(&batch_dir, &mut warnings);
    match body {
        Ok(()) if record.token.is_cancelled() => {
            record.finish(BatchState::Cancelled, None, warnings)
        }
        Ok(()) => record.finish(BatchState::Completed, None, warnings),
        Err(_) => record.finish(
            BatchState::Failed,
            Some(BatchReason::new(
                "BATCH_RUNNER_FAILED",
                "Toplu işlem beklenmedik biçimde durdu",
            )),
            warnings,
        ),
    }
}

/// One batch at a time. The last batch's status stays readable until the next
/// one starts, so a reopened wizard can show its report.
#[derive(Clone, Default)]
pub struct BatchRunner {
    current: Arc<Mutex<Option<Arc<BatchRecord>>>>,
}

impl BatchRunner {
    pub(crate) fn start(
        &self,
        request: BatchRequest,
        environment: &BatchEnvironment,
        sink: Arc<dyn BatchEventSink>,
    ) -> Result<String, DocumentSessionError> {
        let mut current = self.current.lock();
        if current.as_ref().is_some_and(|record| record.is_running()) {
            return Err(invalid(
                "BATCH_ALREADY_RUNNING",
                "Bir toplu işlem zaten çalışıyor",
            ));
        }
        let prepared = prepare(&request, &environment.private)?;

        // Nothing runs now, so anything left under the root is an earlier batch's
        // directory whose removal failed.
        let mut warnings = Vec::new();
        crate::document_session::create_secure_dir(&environment.batches_root)?;
        if let Ok(entries) = std::fs::read_dir(&environment.batches_root) {
            for entry in entries.flatten() {
                remove_dir_if_present(&entry.path(), &mut warnings);
            }
        }

        let id = Uuid::new_v4().to_string();
        let batch_dir = environment.batches_root.join(format!("batch-{id}"));
        crate::document_session::create_secure_dir(&batch_dir)?;
        let files = prepared
            .inputs
            .iter()
            .enumerate()
            .map(|(index, input)| BatchFileStatus {
                index,
                input_path: input.display_path.clone(),
                state: BatchFileState::Pending,
                outputs: Vec::new(),
                reason: None,
            })
            .collect::<Vec<_>>();
        let record = Arc::new(BatchRecord {
            id: id.clone(),
            output_dir: prepared.output_dir.clone(),
            token: CancellationToken::new(),
            children: SessionJobChildProcessOwner::default(),
            state: Mutex::new(RecordState {
                status: BatchStatus {
                    batch_id: id.clone(),
                    state: BatchState::Running,
                    operation: request.operation.kind().to_string(),
                    output_dir: prepared.output_dir.to_string_lossy().into_owned(),
                    total: files.len(),
                    last_sequence: 0,
                    counts: BatchCounts::default(),
                    files,
                    warnings,
                    reason: None,
                },
                terminal_sent: false,
            }),
            emit_order: Mutex::new(()),
            sink,
            done: tokio::sync::watch::channel(false).0,
        });
        let thread_record = Arc::clone(&record);
        let operation = request.operation;
        let policy = request.conflict_policy;
        let thread_dir = batch_dir.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("batch-runner".to_string())
            .spawn(move || run_batch(thread_record, prepared, operation, policy, thread_dir))
        {
            let _ = crate::document_session::secure_remove_tree(&batch_dir);
            return Err(invalid(
                "BATCH_START_FAILED",
                format!("Toplu işlem başlatılamadı: {error}"),
            ));
        }
        *current = Some(record);
        Ok(id)
    }

    fn record(&self, batch_id: Option<&str>) -> Option<Arc<BatchRecord>> {
        self.current
            .lock()
            .as_ref()
            .filter(|record| batch_id.is_none_or(|id| record.id == id))
            .cloned()
    }

    /// Requests cancellation; returns false for an unknown or finished batch.
    pub(crate) fn cancel(&self, batch_id: &str) -> bool {
        match self.record(Some(batch_id)) {
            Some(record) if record.is_running() => {
                record.token.cancel();
                true
            }
            _ => false,
        }
    }

    pub(crate) fn status(&self, batch_id: Option<&str>) -> Option<BatchStatus> {
        self.record(batch_id).map(|record| record.status())
    }

    /// Waits until the batch has sent its terminal event.
    pub(crate) async fn wait(&self, batch_id: &str, timeout: Duration) -> bool {
        let Some(record) = self.record(Some(batch_id)) else {
            return true;
        };
        let mut receiver = record.done.subscribe();
        tokio::time::timeout(timeout, receiver.wait_for(|done| *done))
            .await
            .is_ok_and(|result| result.is_ok())
    }

    /// App exit: cancels a running batch and waits a bounded time for its
    /// worker to stop and its private directory to be removed.
    pub async fn shutdown(&self) {
        let Some(record) = self.record(None) else {
            return;
        };
        if record.is_running() {
            record.token.cancel();
            let _ = self.wait(&record.id, SHUTDOWN_WAIT).await;
        }
    }
}

fn private_app_data_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok()
}

#[tauri::command]
pub async fn scan_batch_inputs(
    paths: Vec<String>,
    recursive: bool,
    app: tauri::AppHandle,
    store: tauri::State<'_, DocumentSessionStore>,
) -> Result<BatchInputScan, DocumentSessionError> {
    let environment = BatchEnvironment::new(&store, private_app_data_dir(&app));
    tauri::async_runtime::spawn_blocking(move || {
        input::scan_inputs(&paths, recursive, &environment.private)
    })
    .await
    .map_err(|error| {
        invalid(
            "BATCH_THREAD_ERROR",
            format!("Tarama tamamlanamadı: {error}"),
        )
    })
}

#[tauri::command]
pub async fn start_batch(
    request: BatchRequest,
    app: tauri::AppHandle,
    store: tauri::State<'_, DocumentSessionStore>,
    runner: tauri::State<'_, BatchRunner>,
) -> Result<BatchStarted, DocumentSessionError> {
    let environment = BatchEnvironment::new(&store, private_app_data_dir(&app));
    let runner = runner.inner().clone();
    let sink: Arc<dyn BatchEventSink> = Arc::new(TauriEventSink(app));
    tauri::async_runtime::spawn_blocking(move || runner.start(request, &environment, sink))
        .await
        .map_err(|error| {
            invalid(
                "BATCH_THREAD_ERROR",
                format!("Başlatma tamamlanamadı: {error}"),
            )
        })?
        .map(|batch_id| BatchStarted { batch_id })
}

#[tauri::command]
pub async fn cancel_batch(
    batch_id: String,
    runner: tauri::State<'_, BatchRunner>,
) -> Result<bool, DocumentSessionError> {
    Ok(runner.cancel(&batch_id))
}

#[tauri::command]
pub async fn get_batch_status(
    batch_id: Option<String>,
    runner: tauri::State<'_, BatchRunner>,
) -> Result<Option<BatchStatus>, DocumentSessionError> {
    Ok(runner.status(batch_id.as_deref()))
}

/// Opens the batch's output folder in the system file manager. The folder is
/// the one recorded when the batch started; the frontend cannot pass a path.
#[tauri::command]
pub async fn reveal_batch_output_dir(
    batch_id: String,
    runner: tauri::State<'_, BatchRunner>,
) -> Result<(), DocumentSessionError> {
    let record = runner
        .record(Some(&batch_id))
        .ok_or_else(|| invalid("BATCH_NOT_FOUND", "Toplu işlem bulunamadı"))?;
    let directory = record.output_dir.clone();
    if !directory.is_dir() {
        return Err(invalid(
            "BATCH_OUTPUT_DIR_INVALID",
            "Çıktı klasörü artık yok",
        ));
    }
    #[cfg(windows)]
    let directory = {
        // Explorer does not accept verbatim (`\\?\`) paths from `canonicalize`.
        let text = directory.to_string_lossy().into_owned();
        PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text).to_string())
    };
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    let mut child = std::process::Command::new(program)
        .arg(&directory)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| invalid("BATCH_REVEAL_FAILED", format!("Klasör açılamadı: {error}")))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests;
