use crate::annotations::NormalizedPageRect;
use crate::document_session::{SessionJobContext, SessionJobOperationError};
use crate::prepress::{
    PrepressResultIdentity, PrintResourceInspectionReport, RgbDerivedCoveragePreview,
    RgbDerivedProcessPreview,
};
use crate::redaction::SensitiveScanReport;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::{Command, Stdio};
#[cfg(not(test))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tempfile::NamedTempFile;

pub const JOB_WORKER_PROTOCOL_VERSION: u16 = 10;
pub const SECURE_REDACTION_TRANSFORM_OUTPUT_NAME: &str = "secure-redaction-transform.pdf";
const PREPRESS_CANCELLATION_MARKER: &str = "prepress-cancel.requested";
const MAX_JOB_REQUEST_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_JOB_RESPONSE_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_JOB_TEXT_CHARS: usize = 32 * 1024 * 1024;
pub(crate) const MAX_JOB_PAGES: usize = 10_000;
#[cfg(not(test))]
static NEXT_JOB_WORKER_EXCHANGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobPageText {
    pub page_index: u32,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum MutationRecipe {
    RotatePage {
        page_index: u32,
        degrees: i32,
    },
    DeletePages {
        page_indices: Vec<u32>,
    },
    ReorderPages {
        page_order: Vec<u32>,
    },
    ExtractPages {
        page_order: Vec<u32>,
    },
    InsertBlankPage {
        at_index: u32,
        width: f32,
        height: f32,
    },
    AddAnnotation {
        page_index: u32,
        annot_type: String,
        /// Sayfa görüntü uzayında normalize dikdörtgen; bkz. [`NormalizedPageRect`].
        normalized_rect: NormalizedPageRect,
        color: String,
        contents: Option<String>,
    },
    DeleteAnnotation {
        page_index: u32,
        annot_id: String,
    },
    /// Form taslaklarını PDFium form-fill ortamıyla yazar; bkz. `forms::fill`.
    FillFormFields {
        edits: Vec<crate::forms::fill::FormFieldEdit>,
    },
    /// Form alanı ekler, siler, taşır ya da özelliklerini değiştirir; bkz. `forms::author`.
    AuthorFormFields {
        ops: Vec<crate::forms::author::FormAuthorOp>,
    },
}

impl MutationRecipe {
    pub fn operation_name(&self) -> &'static str {
        match self {
            Self::RotatePage { .. } => "rotate_page",
            Self::DeletePages { .. } => "delete_pages",
            Self::ReorderPages { .. } => "reorder_pages",
            Self::ExtractPages { .. } => "extract_pages",
            Self::InsertBlankPage { .. } => "insert_blank_page",
            Self::AddAnnotation { .. } => "add_annotation",
            Self::DeleteAnnotation { .. } => "delete_annotation",
            Self::FillFormFields { .. } => "fill_form_fields",
            Self::AuthorFormFields { .. } => "author_form_fields",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum JobCommand {
    ValidateSnapshot,
    MutatePdf {
        recipe: MutationRecipe,
        output_name: String,
    },
    ValidateMutationOutput {
        output_name: String,
    },
    SplitPdf {
        ranges: Vec<(u32, u32)>,
    },
    MergePdf {
        additional_input_names: Vec<String>,
        output_name: String,
    },
    DetectScannedPages,
    RenderOcrPage {
        page_index: u32,
        output_name: String,
        scale: f32,
    },
    ExtractText {
        pages: Option<Vec<u32>>,
        format: String,
    },
    ExtractSemanticText,
    ExportPagesAsImages {
        pages: Vec<u32>,
        format: String,
        dpi: f32,
    },
    PreparePrint {
        pages: Option<Vec<u32>>,
        output_name: String,
    },
    InspectPrintResources {
        content_generation: u64,
        physical_page_index: u32,
        request_id: String,
    },
    GenerateRgbDerivedCoveragePreview {
        content_generation: u64,
        physical_page_index: u32,
        request_id: String,
        threshold_percent: f32,
        scale: f32,
    },
    GenerateRgbDerivedProcessPreview {
        content_generation: u64,
        physical_page_index: u32,
        request_id: String,
        scale: f32,
    },
    ScanSensitiveData {
        patterns: Vec<String>,
    },
    PlanSecureRedaction {
        content_generation: u64,
        policy: crate::secure_redaction::SecureRedactionPolicy,
        acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
        regions: Vec<crate::secure_redaction::RedactionRegionInput>,
    },
    /// Private worker-only operation; no public Tauri command exposes this.
    ApplySecureRedaction {
        content_generation: u64,
        policy: crate::secure_redaction::SecureRedactionPolicy,
        acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
        regions: Vec<crate::secure_redaction::RedactionRegionInput>,
        expected_input_digest: String,
        expected_plan_digest: String,
        output_name: String,
    },
    /// Private worker-only verifier. The artifact name is derived internally;
    /// callers cannot supply an output path or verification result.
    VerifySecureRedaction {
        content_generation: u64,
        policy: crate::secure_redaction::SecureRedactionPolicy,
        acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
        regions: Vec<crate::secure_redaction::RedactionRegionInput>,
        transform_report: crate::secure_redaction::SecureRedactionTransformReport,
    },
    /// Private, read-only sanitization inspection. The worker receives only the
    /// immutable snapshot path supplied by the session-job protocol.
    InspectDocumentSanitization {
        schema_version: String,
        policy_schema_version: String,
        inspection_profile: String,
        #[cfg(feature = "sanitization-test-support")]
        #[serde(default)]
        test_behavior: crate::sanitization_inspection::SanitizationInspectionTestBehavior,
    },
    /// Private transform core. The output is fixed inside the private job
    /// directory and is intentionally not a publication path.
    SanitizePdf {
        content_generation: u64,
        policy: crate::sanitization::SanitizationPolicy,
        expected_input_digest: String,
        expected_inspection_digest: String,
        output_name: String,
        #[cfg(feature = "sanitization-test-support")]
        #[serde(default)]
        test_behavior: crate::sanitization_transform::SanitizationTransformTestBehavior,
    },
    /// Private verifier core. It derives the canonical artifact path from the
    /// job directory and never accepts a publication target.
    VerifySanitizedPdf {
        content_generation: u64,
        policy: crate::sanitization::SanitizationPolicy,
        expected_input_digest: String,
        expected_transform_digest: String,
        transform_report: crate::sanitization::SanitizationTransformReport,
        output_name: String,
        #[cfg(feature = "sanitization-test-support")]
        #[serde(default)]
        test_behavior: crate::sanitization_verifier::SanitizationVerificationTestBehavior,
    },
}

#[derive(Deserialize, Serialize)]
struct JobRequest {
    protocol_version: u16,
    snapshot_path: PathBuf,
    credential: Option<String>,
    authoritative_job_id: String,
    authoritative_session_id: String,
    command: JobCommand,
}

impl std::fmt::Debug for JobRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobRequest")
            .field("protocol_version", &self.protocol_version)
            .field("snapshot_path", &self.snapshot_path)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("authoritative_job_id", &self.authoritative_job_id)
            .field("authoritative_session_id", &self.authoritative_session_id)
            .field("command", &self.command)
            .finish()
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum JobResult {
    SnapshotValid,
    MutationValidated,
    ScannedPages(Vec<bool>),
    File(PathBuf),
    Files(Vec<PathBuf>),
    Text(String),
    PageTexts(Vec<JobPageText>),
    PrintResourceInspection(PrintResourceInspectionReport),
    RgbDerivedCoveragePreview(RgbDerivedCoveragePreview),
    RgbDerivedProcessPreview(RgbDerivedProcessPreview),
    SensitiveReport(SensitiveScanReport),
    SecureRedactionPlan(crate::secure_redaction::SecureRedactionPlan),
    SecureRedactionTransform(crate::secure_redaction::SecureRedactionTransformReport),
    SecureRedactionVerification(crate::secure_redaction::SecureRedactionVerificationReport),
    SanitizationInspection(crate::sanitization::SanitizationInspectionReport),
    SanitizationTransform(crate::sanitization::SanitizationTransformReport),
    SanitizationVerification(crate::sanitization::SanitizationVerificationReport),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct JobResponse {
    protocol_version: u16,
    result: Result<JobResult, JobFailure>,
}

impl JobFailure {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub(crate) fn validate_request_protocol_version(version: u16) -> Result<(), JobFailure> {
    if version != JOB_WORKER_PROTOCOL_VERSION {
        Err(JobFailure::new(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!(
                "Job worker request protocol sürümü eşleşmedi: expected {}, got {}",
                JOB_WORKER_PROTOCOL_VERSION, version
            ),
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_response_protocol_version(
    version: u16,
) -> Result<(), SessionJobOperationError> {
    if version != JOB_WORKER_PROTOCOL_VERSION {
        Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!(
                "Job worker response protocol sürümü eşleşmedi: expected {}, got {}",
                JOB_WORKER_PROTOCOL_VERSION, version
            ),
        ))
    } else {
        Ok(())
    }
}

pub fn run_job(
    context: &SessionJobContext,
    command: JobCommand,
    _timeout: Duration,
) -> Result<JobResult, SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let job_dir = context.snapshot_path.parent().ok_or_else(|| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "Job snapshot dizini çözümlenemedi",
        )
    })?;
    #[cfg(test)]
    {
        execute_command(
            job_dir,
            &context.snapshot_path,
            context
                .credential
                .as_ref()
                .map(|credential| credential.as_deref()),
            &context.job_id.to_string(),
            &context.owner_session_id,
            command,
        )
        .map_err(|failure| {
            SessionJobOperationError::failed_with_code(failure.code, failure.message)
        })
    }
    #[cfg(not(test))]
    {
        let cooperative_prepress = matches!(
            &command,
            JobCommand::InspectPrintResources { .. }
                | JobCommand::GenerateRgbDerivedCoveragePreview { .. }
                | JobCommand::GenerateRgbDerivedProcessPreview { .. }
        );
        let exchange_id = NEXT_JOB_WORKER_EXCHANGE_ID.fetch_add(1, Ordering::Relaxed);
        let request_path = job_dir.join(format!("worker-request-{exchange_id}.json"));
        let response_path = job_dir.join(format!("worker-response-{exchange_id}.json"));
        let request = JobRequest {
            protocol_version: JOB_WORKER_PROTOCOL_VERSION,
            snapshot_path: context.snapshot_path.clone(),
            credential: context
                .credential
                .as_ref()
                .map(|credential| credential.as_deref().to_string()),
            authoritative_job_id: context.job_id.to_string(),
            authoritative_session_id: context.owner_session_id.clone(),
            command,
        };
        write_json_create_new(&request_path, &request, MAX_JOB_REQUEST_BYTES)?;

        let executable = job_worker_executable()?;
        let mut child = Command::new(executable);
        child
            .arg("--run-job-worker")
            .arg("--job-request")
            .arg(&request_path)
            .arg("--job-response")
            .arg(&response_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let status = if cooperative_prepress {
            let marker = job_dir.join(PREPRESS_CANCELLATION_MARKER);
            context.child_processes.run_with_timeout_cooperative(
                &mut child,
                _timeout,
                &context.cancellation,
                Duration::from_millis(500),
                || create_cancellation_marker(&marker),
            )?
        } else {
            context
                .child_processes
                .run_with_timeout(&mut child, _timeout, &context.cancellation)?
        };
        if !status.success() {
            return Err(SessionJobOperationError::failed_with_code(
                "SESSION_JOB_WORKER_CRASHED",
                format!("Job worker typed outcome üretmeden kapandı: {status}"),
            ));
        }

        let response: JobResponse = read_json_bounded(&response_path, MAX_JOB_RESPONSE_BYTES)?;
        validate_response_protocol_version(response.protocol_version)?;
        response.result.map_err(|failure| {
            SessionJobOperationError::failed_with_code(failure.code, failure.message)
        })
    }
}

#[cfg(not(test))]
fn job_worker_executable() -> Result<PathBuf, SessionJobOperationError> {
    let current = std::env::current_exe().map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_SPAWN_FAILED",
            format!("Job worker executable çözümlenemedi: {error}"),
        )
    })?;
    if current
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "deps")
    {
        let binary_name = format!("preview-alternative{}", std::env::consts::EXE_SUFFIX);
        if let Some(candidate) = current
            .parent()
            .and_then(Path::parent)
            .map(|parent| parent.join(binary_name))
            .filter(|path| path.is_file())
        {
            return Ok(candidate);
        }
    }
    Ok(current)
}

pub fn validate_output_file(
    context: &SessionJobContext,
    source: &Path,
) -> Result<PathBuf, SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let job_dir = context.snapshot_path.parent().ok_or_else(|| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "Job output dizini çözümlenemedi",
        )
    })?;
    let canonical_job_dir = job_dir.canonicalize().map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("Job output dizini doğrulanamadı: {error}"),
        )
    })?;
    let canonical_source = source.canonicalize().map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("Job worker çıktısı doğrulanamadı: {error}"),
        )
    })?;
    if !canonical_source.starts_with(&canonical_job_dir) || !canonical_source.is_file() {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Main process job dizini dışındaki worker çıktısını kabul etmedi",
        ));
    }
    Ok(canonical_source)
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn validate_pdf_file_isolated(
    source: &Path,
    credential: Option<String>,
    timeout: Duration,
) -> Result<(), SessionJobOperationError> {
    let exchange_dir = tempfile::Builder::new()
        .prefix("antigravity-pdf-validation-")
        .tempdir()
        .map_err(|error| {
            SessionJobOperationError::failed_with_code(
                "SESSION_JOB_VALIDATION_TEMP_FAILED",
                format!("Validation worker temp dizini oluşturulamadı: {error}"),
            )
        })?;
    let snapshot_path = exchange_dir.path().join("snapshot.pdf");
    fs::copy(source, &snapshot_path).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_VALIDATION_COPY_FAILED",
            format!("Validation worker snapshot kopyası oluşturulamadı: {error}"),
        )
    })?;
    let context =
        SessionJobContext::detached(snapshot_path, credential, "atomic-output-validation");
    match run_job(&context, JobCommand::ValidateSnapshot, timeout)? {
        JobResult::SnapshotValid => Ok(()),
        _ => Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            "Validation worker beklenmeyen sonuç döndürdü",
        )),
    }
}

pub fn run_job_worker() {
    let response_path = match argument_path("--job-response") {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{message:?}");
            std::process::exit(2);
        }
    };
    let response = execute_worker_request();
    if let Err(error) = write_worker_response(&response_path, &response) {
        let fallback = JobResponse {
            protocol_version: JOB_WORKER_PROTOCOL_VERSION,
            result: Err(error.clone()),
        };
        if !response_path.exists() && write_worker_response(&response_path, &fallback).is_ok() {
            return;
        }
        eprintln!("job worker response yazılamadı: {}", error.message);
        std::process::exit(3);
    }
}

fn execute_worker_request() -> JobResponse {
    let result = (|| {
        let request_path = argument_path("--job-request")?;
        let response_path = argument_path("--job-response")?;
        validate_protocol_paths(&request_path, &response_path)?;
        let request: JobRequest = read_json_bounded_worker(&request_path, MAX_JOB_REQUEST_BYTES)?;
        validate_request_protocol_version(request.protocol_version)?;
        let job_dir = request_path.parent().ok_or_else(|| {
            JobFailure::new(
                "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                "Job worker request dizini çözümlenemedi",
            )
        })?;
        if request.snapshot_path.parent() != Some(job_dir)
            || request
                .snapshot_path
                .file_name()
                .and_then(|name| name.to_str())
                != Some("snapshot.pdf")
        {
            return Err(JobFailure::new(
                "SESSION_JOB_WORKER_PATH_REJECTED",
                "Job worker yalnız kendi immutable snapshot girdisini kabul eder",
            ));
        }
        crate::worker::pdfium::apply_resource_limits();
        execute_command(
            job_dir,
            &request.snapshot_path,
            request.credential.as_deref(),
            &request.authoritative_job_id,
            &request.authoritative_session_id,
            request.command,
        )
    })();
    JobResponse {
        protocol_version: JOB_WORKER_PROTOCOL_VERSION,
        result,
    }
}

fn execute_command(
    job_dir: &Path,
    snapshot_path: &Path,
    credential: Option<&str>,
    authoritative_job_id: &str,
    authoritative_session_id: &str,
    command: JobCommand,
) -> Result<JobResult, JobFailure> {
    match command {
        JobCommand::ValidateSnapshot => {
            let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
            let document = crate::pdf_engine::open_pdfium_doc(
                &pdfium,
                &snapshot_path.to_string_lossy(),
                credential,
            )
            .map_err(pdf_open_failed)?;
            if document.pages().is_empty() {
                return Err(JobFailure::new(
                    "SESSION_JOB_SNAPSHOT_INVALID",
                    "Session job snapshot en az bir sayfa içermelidir",
                ));
            }
            Ok(JobResult::SnapshotValid)
        }
        JobCommand::MutatePdf {
            recipe,
            output_name,
        } => {
            let output_path = checked_output_path(job_dir, &output_name)?;
            execute_mutation(snapshot_path, credential, &output_path, recipe)?;
            Ok(JobResult::File(output_path))
        }
        JobCommand::ValidateMutationOutput { output_name } => {
            let output_path = checked_existing_output_path(job_dir, &output_name)?;
            let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
            let document = pdfium
                .load_pdf_from_file(&output_path, None)
                .map_err(|error| {
                    JobFailure::new(
                        "SESSION_MUTATION_VALIDATION_OPEN_FAILED",
                        format!("Mutation çıktısı PDFium ile yeniden açılamadı: {error}"),
                    )
                })?;
            if document.pages().is_empty() {
                return Err(JobFailure::new(
                    "SESSION_MUTATION_VALIDATION_INVARIANT_FAILED",
                    "Mutation çıktısı en az bir sayfa içermelidir",
                ));
            }
            Ok(JobResult::MutationValidated)
        }
        JobCommand::SplitPdf { ranges } => {
            let outputs = split_pdf(snapshot_path, credential, job_dir, &ranges)?;
            Ok(JobResult::Files(outputs))
        }
        JobCommand::MergePdf {
            additional_input_names,
            output_name,
        } => {
            let output_path = checked_output_path(job_dir, &output_name)?;
            merge_pdf(
                snapshot_path,
                credential,
                job_dir,
                &additional_input_names,
                &output_path,
            )?;
            Ok(JobResult::File(output_path))
        }
        JobCommand::DetectScannedPages => with_document(snapshot_path, credential, |document| {
            let page_count = usize::from(document.pages().len());
            if page_count > MAX_JOB_PAGES {
                return Err(job_failed(
                    "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
                    "Taranmış sayfa tespiti sayfa limiti aşıldı",
                ));
            }
            let mut flags = Vec::with_capacity(page_count);
            for page in document.pages().iter() {
                // Metni okunamayan sayfa, eski sayfa başına akıştaki frontend
                // fallback'i gibi "taranmamış" sayılır; tek sayfa tüm belgeyi düşürmez.
                let scanned = page
                    .text()
                    .map(|text| is_scanned_page_text(&text.all()))
                    .unwrap_or(false);
                flags.push(scanned);
            }
            Ok(JobResult::ScannedPages(flags))
        }),
        JobCommand::RenderOcrPage {
            page_index,
            output_name,
            scale,
        } => {
            let output_path = checked_output_path(job_dir, &output_name)?;
            with_document(snapshot_path, credential, |document| {
                let page = get_page(document, page_index)?;
                let target_width = checked_target_width(page.width().value, scale)?;
                let bitmap = page
                    .render_with_config(&PdfRenderConfig::new().set_target_width(target_width))
                    .map_err(|_| {
                        job_failed("SESSION_JOB_RENDER_FAILED", "OCR sayfası render edilemedi")
                    })?;
                bitmap
                    .as_image()
                    .save_with_format(&output_path, image::ImageFormat::Png)
                    .map_err(|error| {
                        JobFailure::new(
                            "SESSION_JOB_OUTPUT_WRITE_FAILED",
                            format!("OCR hazırlık çıktısı yazılamadı: {error}"),
                        )
                    })?;
                Ok(JobResult::File(output_path))
            })
        }
        JobCommand::ExtractText { pages, format } => {
            let text = extract_text(snapshot_path, credential, pages.as_deref(), &format)?;
            Ok(JobResult::Text(text))
        }
        JobCommand::ExtractSemanticText => with_document(snapshot_path, credential, |document| {
            let mut page_texts = Vec::new();
            for (page_index, page) in document.pages().iter().enumerate() {
                if page_texts.len() >= MAX_JOB_PAGES {
                    return Err(job_failed(
                        "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
                        "Semantic extraction sayfa limiti aşıldı",
                    ));
                }
                let text = page.text().map(|text| text.all()).unwrap_or_default();
                page_texts.push(JobPageText {
                    page_index: page_index as u32,
                    text,
                });
            }
            Ok(JobResult::PageTexts(page_texts))
        }),
        JobCommand::ExportPagesAsImages { pages, format, dpi } => {
            if pages.len() > MAX_JOB_PAGES {
                return Err(job_failed(
                    "SESSION_JOB_INPUT_LIMIT_EXCEEDED",
                    "Image export sayfa limiti aşıldı",
                ));
            }
            let output_dir = job_dir.join("worker-output-images");
            fs::create_dir(&output_dir).map_err(|error| {
                JobFailure::new(
                    "SESSION_JOB_OUTPUT_WRITE_FAILED",
                    format!("Image export geçici dizini oluşturulamadı: {error}"),
                )
            })?;
            let paths = crate::export::export_pages_as_images_with_password(
                snapshot_path.to_string_lossy().to_string(),
                pages,
                output_dir.to_string_lossy().to_string(),
                format,
                dpi,
                credential.map(str::to_string),
                None,
            )
            .map_err(operation_failure)?;
            Ok(JobResult::Files(
                paths.into_iter().map(PathBuf::from).collect(),
            ))
        }
        JobCommand::PreparePrint { pages, output_name } => {
            let output_path = checked_output_path(job_dir, &output_name)?;
            prepare_print_pdf(snapshot_path, credential, pages.as_deref(), &output_path)?;
            Ok(JobResult::File(output_path))
        }
        JobCommand::InspectPrintResources {
            content_generation,
            physical_page_index,
            request_id,
        } => {
            let marker = job_dir.join(PREPRESS_CANCELLATION_MARKER);
            let report = crate::prepress::inspect_print_resources(
                snapshot_path.to_string_lossy().to_string(),
                credential.map(str::to_string),
                PrepressResultIdentity {
                    session_id: authoritative_session_id.to_string(),
                    content_generation,
                    physical_page_index,
                    request_id,
                    methodology_version: crate::prepress::PRINT_INSPECTION_VERSION.to_string(),
                },
                crate::prepress::PrepressCancellation::marker(&marker),
            )
            .map_err(operation_failure)?;
            Ok(JobResult::PrintResourceInspection(report))
        }
        JobCommand::GenerateRgbDerivedCoveragePreview {
            content_generation,
            physical_page_index,
            request_id,
            threshold_percent,
            scale,
        } => {
            let marker = job_dir.join(PREPRESS_CANCELLATION_MARKER);
            let result = crate::prepress::generate_rgb_derived_coverage_preview(
                snapshot_path.to_string_lossy().to_string(),
                credential.map(str::to_string),
                PrepressResultIdentity {
                    session_id: authoritative_session_id.to_string(),
                    content_generation,
                    physical_page_index,
                    request_id,
                    methodology_version: crate::prepress::RGB_PREVIEW_METHODOLOGY_VERSION
                        .to_string(),
                },
                threshold_percent,
                scale,
                crate::prepress::PrepressCancellation::marker(&marker),
            )
            .map_err(operation_failure)?;
            Ok(JobResult::RgbDerivedCoveragePreview(result))
        }
        JobCommand::GenerateRgbDerivedProcessPreview {
            content_generation,
            physical_page_index,
            request_id,
            scale,
        } => {
            let marker = job_dir.join(PREPRESS_CANCELLATION_MARKER);
            let result = crate::prepress::generate_rgb_derived_process_preview(
                snapshot_path.to_string_lossy().to_string(),
                credential.map(str::to_string),
                PrepressResultIdentity {
                    session_id: authoritative_session_id.to_string(),
                    content_generation,
                    physical_page_index,
                    request_id,
                    methodology_version: crate::prepress::RGB_PREVIEW_METHODOLOGY_VERSION
                        .to_string(),
                },
                scale,
                crate::prepress::PrepressCancellation::marker(&marker),
            )
            .map_err(operation_failure)?;
            Ok(JobResult::RgbDerivedProcessPreview(result))
        }
        JobCommand::ScanSensitiveData { patterns } => {
            let report =
                crate::redaction::scan_sensitive_file(snapshot_path, credential, &patterns)
                    .map_err(operation_failure)?;
            Ok(JobResult::SensitiveReport(report))
        }
        JobCommand::PlanSecureRedaction {
            content_generation,
            policy,
            acknowledgements,
            regions,
        } => {
            let plan = crate::secure_redaction_inspection::inspect_and_plan_secure_redaction(
                snapshot_path,
                credential,
                content_generation,
                policy,
                acknowledgements,
                regions,
            )
            .map_err(operation_failure)?;
            Ok(JobResult::SecureRedactionPlan(plan))
        }
        JobCommand::ApplySecureRedaction {
            content_generation,
            policy,
            acknowledgements,
            regions,
            expected_input_digest,
            expected_plan_digest,
            output_name,
        } => execute_secure_redaction_transform(SecureRedactionTransformJobArgs {
            job_dir,
            snapshot_path,
            credential,
            job_id: authoritative_job_id,
            session_id: authoritative_session_id,
            content_generation,
            policy,
            acknowledgements,
            regions,
            expected_input_digest: &expected_input_digest,
            expected_plan_digest: &expected_plan_digest,
            output_name: &output_name,
        })
        .map(JobResult::SecureRedactionTransform),
        JobCommand::VerifySecureRedaction {
            content_generation,
            policy,
            acknowledgements,
            regions,
            transform_report,
        } => crate::secure_redaction_verifier::verify_secure_redaction(
            crate::secure_redaction_verifier::SecureRedactionVerificationArgs {
                job_dir,
                snapshot_path,
                credential,
                job_id: authoritative_job_id,
                session_id: authoritative_session_id,
                content_generation,
                policy,
                acknowledgements,
                regions,
                transform_report,
            },
        )
        .map(JobResult::SecureRedactionVerification)
        .map_err(operation_failure),
        JobCommand::InspectDocumentSanitization {
            schema_version,
            policy_schema_version,
            inspection_profile,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior,
        } => {
            #[cfg(not(feature = "sanitization-test-support"))]
            let inspection = crate::sanitization_inspection::inspect_snapshot(
                snapshot_path,
                credential,
                &schema_version,
                &policy_schema_version,
                &inspection_profile,
            );
            #[cfg(feature = "sanitization-test-support")]
            let inspection = crate::sanitization_inspection::inspect_snapshot_with_test_behavior(
                snapshot_path,
                credential,
                &schema_version,
                &policy_schema_version,
                &inspection_profile,
                test_behavior,
            );
            inspection
                .map(JobResult::SanitizationInspection)
                .map_err(operation_failure)
        }
        JobCommand::SanitizePdf {
            content_generation,
            policy,
            expected_input_digest,
            expected_inspection_digest,
            output_name,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior,
        } => crate::sanitization_transform::execute_transform(
            crate::sanitization_transform::SanitizationTransformExecutionArgs {
                job_dir,
                snapshot_path,
                credential,
                content_generation,
                policy,
                expected_input_digest: &expected_input_digest,
                expected_inspection_digest: &expected_inspection_digest,
                output_name: &output_name,
                #[cfg(feature = "sanitization-test-support")]
                test_behavior,
            },
        )
        .map(JobResult::SanitizationTransform),
        JobCommand::VerifySanitizedPdf {
            content_generation,
            policy,
            expected_input_digest,
            expected_transform_digest,
            transform_report,
            output_name,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior,
        } => crate::sanitization_verifier::verify_sanitized_pdf(
            job_dir,
            content_generation,
            policy,
            &expected_input_digest,
            &expected_transform_digest,
            transform_report,
            &output_name,
            #[cfg(feature = "sanitization-test-support")]
            test_behavior,
        )
        .map(JobResult::SanitizationVerification)
        .map_err(operation_failure),
    }
}

struct SecureRedactionTransformJobArgs<'a> {
    pub job_dir: &'a Path,
    pub snapshot_path: &'a Path,
    pub credential: Option<&'a str>,
    pub job_id: &'a str,
    pub session_id: &'a str,
    pub content_generation: u64,
    pub policy: crate::secure_redaction::SecureRedactionPolicy,
    pub acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
    pub regions: Vec<crate::secure_redaction::RedactionRegionInput>,
    pub expected_input_digest: &'a str,
    pub expected_plan_digest: &'a str,
    pub output_name: &'a str,
}

fn execute_secure_redaction_transform(
    args: SecureRedactionTransformJobArgs<'_>,
) -> Result<crate::secure_redaction::SecureRedactionTransformReport, JobFailure> {
    let SecureRedactionTransformJobArgs {
        job_dir,
        snapshot_path,
        credential,
        job_id,
        session_id,
        content_generation,
        policy,
        acknowledgements,
        regions,
        expected_input_digest,
        expected_plan_digest,
        output_name,
    } = args;
    use crate::secure_redaction::{
        evaluate_secure_redaction_rebuild_geometry, make_redaction_error, PageRedactionStrategy,
        SecureRedactionErrorCode, SecureRedactionTransformEvidence, TransformPageFeatureLoss,
        TransformRasterPageEvidence, MAX_SECURE_REDACTION_OUTPUT_BYTES, REDACTION_SCHEMA_VERSION,
    };

    if output_name != SECURE_REDACTION_TRANSFORM_OUTPUT_NAME {
        return Err(job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Secure redaction output adı sabit internal artifact adı olmalıdır",
        ));
    }
    crate::secure_redaction::validate_sha256_digest(expected_input_digest, "expected_input_digest")
        .map_err(operation_failure)?;
    crate::secure_redaction::validate_sha256_digest(expected_plan_digest, "expected_plan_digest")
        .map_err(operation_failure)?;

    // The digest is taken again from the immutable physical snapshot immediately before opening it.
    let actual_input_digest = crate::document_io::sha256_file(snapshot_path).map_err(|error| {
        operation_failure(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanStale,
            format!("Snapshot SHA-256 digest hesaplanamadı: {error}"),
        ))
    })?;
    if actual_input_digest != expected_input_digest {
        return Err(operation_failure(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanStale,
            "Immutable snapshot digest expected_input_digest ile eşleşmedi",
        )));
    }

    let plan = crate::secure_redaction_inspection::inspect_and_plan_secure_redaction(
        snapshot_path,
        credential,
        content_generation,
        policy,
        acknowledgements,
        regions,
    )
    .map_err(operation_failure)?;
    crate::secure_redaction::validate_secure_redaction_plan(&plan).map_err(operation_failure)?;
    if !plan.executable || !plan.ready_for_execution || !plan.blockers.is_empty() {
        return Err(operation_failure(make_redaction_error(
            SecureRedactionErrorCode::RedactionBlockedDocumentFeature,
            "Re-inspected secure-redaction plan is not ready for execution",
        )));
    }
    if plan.plan_digest != expected_plan_digest {
        return Err(operation_failure(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanDigestMismatch,
            "Re-inspected secure-redaction plan digest expected_plan_digest ile eşleşmedi",
        )));
    }

    for page_plan in &plan.page_plans {
        if page_plan.strategy == PageRedactionStrategy::RasterizePage {
            evaluate_secure_redaction_rebuild_geometry(&page_plan.geometry)
                .map_err(operation_failure)?;
        }
    }

    let final_path = secure_redaction_output_path(job_dir)?;
    let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
    let source =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
            .map_err(pdf_open_failed)?;
    let mut output = pdfium.create_new_pdf().map_err(|error| {
        job_failed(
            "SESSION_JOB_PDF_WRITE_FAILED",
            format!("Yeni PDF oluşturulamadı: {error}"),
        )
    })?;

    let mut affected_pages = Vec::new();
    let mut unaffected_pages = Vec::new();
    let mut raster_geometry = Vec::new();
    let mut page_feature_losses = Vec::new();
    let mut applied_regions = Vec::new();

    for page_plan in &plan.page_plans {
        let source_index = u16::try_from(page_plan.page_index).map_err(|_| {
            job_failed(
                "REDACTION_RESOURCE_LIMIT_EXCEEDED",
                "Page index PDFium limitini aşıyor",
            )
        })?;
        match page_plan.strategy {
            PageRedactionStrategy::ImportUnaffectedPage => {
                let destination = output.pages().len();
                output
                    .pages_mut()
                    .copy_page_from_document(&source, source_index, destination)
                    .map_err(|error| {
                        job_failed(
                            "SESSION_JOB_PDF_WRITE_FAILED",
                            format!("Unaffected page import failed: {error}"),
                        )
                    })?;
                unaffected_pages.push(page_plan.page_index);
            }
            PageRedactionStrategy::RasterizePage => {
                let source_page = source.pages().get(source_index).map_err(|error| {
                    job_failed(
                        "SESSION_JOB_PDF_OPEN_FAILED",
                        format!("Affected source page could not be loaded: {error}"),
                    )
                })?;
                let payload =
                    crate::secure_redaction_raster::build_lossless_raster_payload_for_page(
                        &plan,
                        &source,
                        page_plan.page_index,
                        None,
                    )
                    .map_err(operation_failure)?;
                let image =
                    image::RgbaImage::from_raw(payload.width, payload.height, payload.pixels)
                        .ok_or_else(|| {
                            job_failed(
                                "REDACTION_RESOURCE_LIMIT_EXCEEDED",
                                "Lossless RGBA payload length is invalid",
                            )
                        })?;
                let dynamic = image::DynamicImage::ImageRgba8(image);
                let size = PdfPagePaperSize::Custom(
                    PdfPoints::new(page_plan.geometry.media_box.width as f32),
                    PdfPoints::new(page_plan.geometry.media_box.height as f32),
                );
                let mut page = output
                    .pages_mut()
                    .create_page_at_end(size)
                    .map_err(|error| {
                        job_failed(
                            "SESSION_JOB_PDF_WRITE_FAILED",
                            format!("Raster output page could not be created: {error}"),
                        )
                    })?;
                apply_rebuilt_page_geometry(&mut page, &page_plan.geometry)?;
                let placement = crate::secure_redaction_raster::raster_image_placement_matrix(
                    &source_page,
                    &page_plan.geometry,
                    payload.width,
                    payload.height,
                )
                .map_err(operation_failure)?;
                let mut image_object = page
                    .objects_mut()
                    .create_image_object(PdfPoints::ZERO, PdfPoints::ZERO, &dynamic, None, None)
                    .map_err(|error| {
                        job_failed(
                            "SESSION_JOB_PDF_WRITE_FAILED",
                            format!("Lossless image insertion failed: {error}"),
                        )
                    })?;
                image_object.reset_matrix(placement).map_err(|error| {
                    job_failed(
                        "SESSION_JOB_PDF_WRITE_FAILED",
                        format!("Lossless image placement failed: {error}"),
                    )
                })?;
                page.regenerate_content().map_err(|error| {
                    job_failed(
                        "SESSION_JOB_PDF_WRITE_FAILED",
                        format!("Raster page content generation failed: {error}"),
                    )
                })?;
                affected_pages.push(page_plan.page_index);
                raster_geometry.push(TransformRasterPageEvidence {
                    page_index: page_plan.page_index,
                    width: payload.width,
                    height: payload.height,
                    dpi: plan.policy.raster_dpi,
                });
                applied_regions.extend(page_plan.canonical_regions.clone());
            }
        }
        page_feature_losses.push(TransformPageFeatureLoss {
            page_index: page_plan.page_index,
            feature_losses: page_plan.feature_losses.clone(),
        });
    }

    let staging = NamedTempFile::new_in(job_dir).map_err(|error| {
        job_failed(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Secure transform staging file oluşturulamadı: {error}"),
        )
    })?;
    output.save_to_file(staging.path()).map_err(|error| {
        job_failed(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Secure transform full save başarısız: {error}"),
        )
    })?;
    // Setter success is not sufficient: reopen the serialized artifact and
    // compare each rebuilt page against the canonical plan before promotion.
    let reopened =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &staging.path().to_string_lossy(), None)
            .map_err(pdf_open_failed)?;
    for (output_index, page_plan) in plan.page_plans.iter().enumerate() {
        if page_plan.strategy != PageRedactionStrategy::RasterizePage {
            continue;
        }
        let reopened_page = reopened.pages().get(output_index as u16).map_err(|error| {
            job_failed(
                "SESSION_JOB_PDF_OPEN_FAILED",
                format!("Saved affected page could not be reopened: {error}"),
            )
        })?;
        crate::secure_redaction_raster::validate_pdfium_page_geometry(
            &reopened_page,
            &page_plan.geometry,
            page_plan.page_index,
        )
        .map_err(operation_failure)?;
    }
    staging.as_file().sync_all().map_err(|error| {
        job_failed(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Secure transform staging flush başarısız: {error}"),
        )
    })?;
    let output_size_bytes = staging
        .as_file()
        .metadata()
        .map_err(|error| {
            job_failed(
                "SESSION_JOB_OUTPUT_READ_FAILED",
                format!("Secure transform staging metadata okunamadı: {error}"),
            )
        })?
        .len();
    if output_size_bytes > MAX_SECURE_REDACTION_OUTPUT_BYTES {
        return Err(operation_failure(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Secure transform staging artifact size ({output_size_bytes}) exceeds limit ({MAX_SECURE_REDACTION_OUTPUT_BYTES})"
            ),
        )));
    }
    let output_digest = crate::document_io::sha256_file(staging.path()).map_err(|error| {
        job_failed(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("Secure transform output digest hesaplanamadı: {error}"),
        )
    })?;
    staging.persist_noclobber(&final_path).map_err(|error| {
        job_failed(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!(
                "Secure transform artifact promote başarısız: {}",
                error.error
            ),
        )
    })?;

    let report = crate::secure_redaction::SecureRedactionTransformReport {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        job_id: job_id.to_string(),
        session_id: session_id.to_string(),
        input_digest: actual_input_digest,
        plan_digest: plan.plan_digest,
        output_relative_name: SECURE_REDACTION_TRANSFORM_OUTPUT_NAME.to_string(),
        output_digest,
        output_size_bytes,
        page_count: plan.page_plans.len() as u32,
        affected_pages,
        unaffected_pages,
        raster_dpi: plan.policy.raster_dpi,
        applied_canonical_regions: applied_regions,
        affected_page_raster_geometry: raster_geometry,
        page_feature_losses,
        background_policy: "opaque_white_v1".to_string(),
        warnings: plan.warnings.clone(),
        transform_evidence: SecureRedactionTransformEvidence {
            new_document_constructed: true,
            affected_pages_rasterized: true,
            affected_page_original_objects_not_imported: true,
            unaffected_pages_imported: true,
            full_rewrite_used: true,
            incremental_save_not_used: true,
            opaque_white_background_policy: true,
            lossless_raster_path: true,
            output_digest_calculated: true,
        },
        status: "transformed_unverified".to_string(),
    };
    crate::secure_redaction::validate_transform_report(&report).map_err(operation_failure)?;
    Ok(report)
}

fn secure_redaction_output_path(job_dir: &Path) -> Result<PathBuf, JobFailure> {
    let job_meta = fs::symlink_metadata(job_dir).map_err(|error| {
        job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            format!("Job directory doğrulanamadı: {error}"),
        )
    })?;
    if !job_meta.file_type().is_dir() || job_meta.file_type().is_symlink() {
        return Err(job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Secure transform job directory güvenli bir dizin değil",
        ));
    }
    let path = job_dir.join(SECURE_REDACTION_TRANSFORM_OUTPUT_NAME);
    if fs::symlink_metadata(&path).is_ok() {
        return Err(job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Secure transform artifact zaten mevcut veya güvenli olmayan bir dosya tipi",
        ));
    }
    Ok(path)
}

fn pdf_rect_from_canonical(rect: &crate::secure_redaction::PdfPointRect) -> PdfRect {
    PdfRect::new_from_values(
        rect.bottom as f32,
        rect.left as f32,
        (rect.bottom + rect.height) as f32,
        (rect.left + rect.width) as f32,
    )
}

fn apply_rebuilt_page_geometry(
    page: &mut PdfPage<'_>,
    geometry: &crate::secure_redaction::CanonicalPageGeometry,
) -> Result<(), JobFailure> {
    let boundaries = page.boundaries_mut();
    boundaries
        .set_media(pdf_rect_from_canonical(&geometry.media_box))
        .map_err(|error| {
            job_failed(
                "SESSION_JOB_PDF_WRITE_FAILED",
                format!("MediaBox rebuild failed: {error}"),
            )
        })?;
    if let Some(crop) = &geometry.crop_box {
        boundaries
            .set_crop(pdf_rect_from_canonical(crop))
            .map_err(|error| {
                job_failed(
                    "SESSION_JOB_PDF_WRITE_FAILED",
                    format!("CropBox rebuild failed: {error}"),
                )
            })?;
    }
    if let Some(trim) = &geometry.trim_box {
        boundaries
            .set_trim(pdf_rect_from_canonical(trim))
            .map_err(|error| {
                job_failed(
                    "SESSION_JOB_PDF_WRITE_FAILED",
                    format!("TrimBox rebuild failed: {error}"),
                )
            })?;
    }
    if let Some(bleed) = &geometry.bleed_box {
        boundaries
            .set_bleed(pdf_rect_from_canonical(bleed))
            .map_err(|error| {
                job_failed(
                    "SESSION_JOB_PDF_WRITE_FAILED",
                    format!("BleedBox rebuild failed: {error}"),
                )
            })?;
    }
    if let Some(art) = &geometry.art_box {
        boundaries
            .set_art(pdf_rect_from_canonical(art))
            .map_err(|error| {
                job_failed(
                    "SESSION_JOB_PDF_WRITE_FAILED",
                    format!("ArtBox rebuild failed: {error}"),
                )
            })?;
    }
    let rotation = match geometry.rotation {
        crate::secure_redaction::PageRotation::Deg0 => PdfPageRenderRotation::None,
        crate::secure_redaction::PageRotation::Deg90 => PdfPageRenderRotation::Degrees90,
        crate::secure_redaction::PageRotation::Deg180 => PdfPageRenderRotation::Degrees180,
        crate::secure_redaction::PageRotation::Deg270 => PdfPageRenderRotation::Degrees270,
    };
    page.set_rotation(rotation);
    Ok(())
}

fn execute_mutation(
    snapshot_path: &Path,
    credential: Option<&str>,
    output_path: &Path,
    recipe: MutationRecipe,
) -> Result<(), JobFailure> {
    let snapshot = snapshot_path.to_string_lossy().to_string();
    let output = output_path.to_string_lossy().to_string();
    let password = credential.map(str::to_string);
    let report = match recipe {
        MutationRecipe::ReorderPages { page_order } => {
            crate::manipulation::reorder_pdf_to_target_report(
                snapshot,
                page_order,
                output,
                password,
                crate::manipulation::ReorderMode::FullPermutation,
            )
        }
        MutationRecipe::ExtractPages { page_order } => {
            crate::manipulation::reorder_pdf_to_target_report(
                snapshot,
                page_order,
                output,
                password,
                crate::manipulation::ReorderMode::OrderedSubset,
            )
        }
        MutationRecipe::RotatePage {
            page_index,
            degrees,
        } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::manipulation::rotate_page_report(output, page_index, degrees, password)
        }
        MutationRecipe::DeletePages { page_indices } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::manipulation::delete_pages_report(output, page_indices, password)
        }
        MutationRecipe::InsertBlankPage {
            at_index,
            width,
            height,
        } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::manipulation::insert_blank_page_report(output, at_index, width, height, password)
        }
        MutationRecipe::AddAnnotation {
            page_index,
            annot_type,
            normalized_rect,
            color,
            contents,
        } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::annotations::add_annotation_report(
                output,
                page_index,
                annot_type,
                normalized_rect,
                color,
                contents,
                password,
            )
        }
        MutationRecipe::DeleteAnnotation {
            page_index,
            annot_id,
        } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::annotations::delete_annotation_report(output, page_index, annot_id, password)
        }
        MutationRecipe::FillFormFields { edits } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::forms::fill::fill_form_fields_report(output, edits, password)
        }
        MutationRecipe::AuthorFormFields { ops } => {
            copy_mutation_input(snapshot_path, output_path)?;
            crate::forms::author::author_form_fields_report(output, ops, password)
        }
    };
    report.map(|_| ()).map_err(|error| {
        JobFailure::new(
            "SESSION_MUTATION_WORKER_FAILED",
            format!("Mutation worker işlemi başarısız: {error}"),
        )
    })
}

fn copy_mutation_input(snapshot_path: &Path, output_path: &Path) -> Result<(), JobFailure> {
    fs::copy(snapshot_path, output_path).map_err(|error| {
        JobFailure::new(
            "SESSION_MUTATION_OUTPUT_WRITE_FAILED",
            format!("Mutation snapshot çıktıya kopyalanamadı: {error}"),
        )
    })?;
    Ok(())
}

/// Output file name of the `range_index`-th split range starting at `start`.
pub(crate) fn split_output_file_name(range_index: usize, start: u32) -> String {
    format!("split_{range_index}_{start}.pdf")
}

fn split_pdf(
    snapshot_path: &Path,
    credential: Option<&str>,
    job_dir: &Path,
    ranges: &[(u32, u32)],
) -> Result<Vec<PathBuf>, JobFailure> {
    if ranges.is_empty() || ranges.len() > MAX_JOB_PAGES {
        return Err(job_failed(
            "SESSION_JOB_INVALID_SPLIT_RANGES",
            "Split range listesi boş veya sınırın üzerinde",
        ));
    }
    let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
    let document =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
            .map_err(pdf_open_failed)?;
    let page_count = u32::from(document.pages().len());
    let mut outputs = Vec::with_capacity(ranges.len());
    for (range_index, &(start, end)) in ranges.iter().enumerate() {
        if start > end || end >= page_count {
            return Err(job_failed(
                "SESSION_JOB_INVALID_SPLIT_RANGES",
                format!("Split range aralık dışında: {start}..={end}, sayfa={page_count}"),
            ));
        }
        let output_path =
            checked_output_path(job_dir, &split_output_file_name(range_index, start))?;
        let mut output = pdfium.create_new_pdf().map_err(|error| {
            job_failed(
                "SESSION_JOB_SPLIT_FAILED",
                format!("Split PDF oluşturulamadı: {error}"),
            )
        })?;
        for page_index in start..=end {
            let destination = output.pages().len();
            output
                .pages_mut()
                .copy_page_from_document(&document, page_index as u16, destination)
                .map_err(|error| {
                    job_failed(
                        "SESSION_JOB_SPLIT_FAILED",
                        format!("Split sayfası kopyalanamadı: {error}"),
                    )
                })?;
        }
        output.save_to_file(&output_path).map_err(|error| {
            job_failed(
                "SESSION_JOB_OUTPUT_WRITE_FAILED",
                format!("Split çıktısı kaydedilemedi: {error}"),
            )
        })?;
        outputs.push(output_path);
    }
    Ok(outputs)
}

fn merge_pdf(
    snapshot_path: &Path,
    credential: Option<&str>,
    job_dir: &Path,
    additional_input_names: &[String],
    output_path: &Path,
) -> Result<(), JobFailure> {
    if additional_input_names.len() >= MAX_JOB_PAGES {
        return Err(job_failed(
            "SESSION_JOB_INPUT_LIMIT_EXCEEDED",
            "Merge girdi limiti aşıldı",
        ));
    }
    let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
    let mut output = pdfium.create_new_pdf().map_err(|error| {
        job_failed(
            "SESSION_JOB_MERGE_FAILED",
            format!("Merge PDF oluşturulamadı: {error}"),
        )
    })?;
    let source =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
            .map_err(pdf_open_failed)?;
    copy_all_pages(&source, &mut output, "Session snapshot")?;
    for name in additional_input_names {
        let path = checked_existing_output_path(job_dir, name)?;
        let document = pdfium.load_pdf_from_file(&path, None).map_err(|error| {
            job_failed(
                "SESSION_JOB_MERGE_ADDITIONAL_INPUT_OPEN_FAILED",
                format!("Ek merge girdisi açılamadı: {error}"),
            )
        })?;
        copy_all_pages(&document, &mut output, "Ek PDF")?;
    }
    if output.pages().is_empty() {
        return Err(job_failed(
            "SESSION_JOB_MERGE_FAILED",
            "Merge çıktısı en az bir sayfa içermelidir",
        ));
    }
    output.save_to_file(output_path).map_err(|error| {
        job_failed(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Merge çıktısı kaydedilemedi: {error}"),
        )
    })
}

fn copy_all_pages(
    source: &PdfDocument<'_>,
    output: &mut PdfDocument<'_>,
    label: &str,
) -> Result<(), JobFailure> {
    for page_index in 0..source.pages().len() {
        let destination = output.pages().len();
        output
            .pages_mut()
            .copy_page_from_document(source, page_index, destination)
            .map_err(|error| {
                job_failed(
                    "SESSION_JOB_MERGE_FAILED",
                    format!("{label} sayfası merge edilemedi: {error}"),
                )
            })?;
    }
    Ok(())
}

fn with_document<T>(
    snapshot_path: &Path,
    credential: Option<&str>,
    operation: impl FnOnce(&PdfDocument<'_>) -> Result<T, JobFailure>,
) -> Result<T, JobFailure> {
    let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
    let document =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
            .map_err(pdf_open_failed)?;
    operation(&document)
}

fn is_scanned_page_text(text: &str) -> bool {
    text.trim().len() < 20
}

fn get_page<'a>(document: &'a PdfDocument<'_>, page_index: u32) -> Result<PdfPage<'a>, JobFailure> {
    let page_index = u16::try_from(page_index)
        .map_err(|_| job_failed("SESSION_JOB_INVALID_PAGE", "Sayfa numarası sınır dışında"))?;
    document
        .pages()
        .get(page_index)
        .map_err(|_| job_failed("SESSION_JOB_INVALID_PAGE", "Sayfa bulunamadı"))
}

fn extract_text(
    snapshot_path: &Path,
    credential: Option<&str>,
    selected_pages: Option<&[u32]>,
    format: &str,
) -> Result<String, JobFailure> {
    with_document(snapshot_path, credential, |document| {
        let pages = selected_pages
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| (0..u32::from(document.pages().len())).collect());
        if pages.len() > MAX_JOB_PAGES {
            return Err(job_failed(
                "SESSION_JOB_INPUT_LIMIT_EXCEEDED",
                "Text export sayfa limiti aşıldı",
            ));
        }
        let markdown = format.eq_ignore_ascii_case("markdown");
        let mut output = String::new();
        for page_index in pages {
            let page = get_page(document, page_index)?;
            let page_text = page.text().map(|text| text.all()).unwrap_or_default();
            if markdown {
                output.push_str(&format!("\n## Sayfa {}\n\n", page_index + 1));
            } else {
                output.push_str(&format!("\n--- Sayfa {} ---\n\n", page_index + 1));
            }
            output.push_str(&page_text);
            if output.chars().count() > MAX_JOB_TEXT_CHARS {
                return Err(job_failed(
                    "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
                    "Text extraction çıktı limiti aşıldı",
                ));
            }
        }
        Ok(output)
    })
}

fn prepare_print_pdf(
    snapshot_path: &Path,
    credential: Option<&str>,
    selected_pages: Option<&[u32]>,
    output_path: &Path,
) -> Result<(), JobFailure> {
    if selected_pages.is_none_or(|pages| pages.is_empty()) {
        fs::copy(snapshot_path, output_path).map_err(|error| {
            JobFailure::new(
                "SESSION_JOB_OUTPUT_WRITE_FAILED",
                format!("Print snapshot hazırlanamadı: {error}"),
            )
        })?;
        return Ok(());
    }
    let pdfium = crate::pdf_engine::get_pdfium().map_err(pdfium_unavailable)?;
    let source =
        crate::pdf_engine::open_pdfium_doc(&pdfium, &snapshot_path.to_string_lossy(), credential)
            .map_err(pdf_open_failed)?;
    let mut target = pdfium.create_new_pdf().map_err(|_| {
        job_failed(
            "SESSION_JOB_PRINT_PREPARATION_FAILED",
            "Print PDF oluşturulamadı",
        )
    })?;
    for (target_index, source_index) in selected_pages.unwrap().iter().copied().enumerate() {
        let source_index = u16::try_from(source_index)
            .map_err(|_| job_failed("SESSION_JOB_INVALID_PAGE", "Print sayfa numarası geçersiz"))?;
        let target_index = u16::try_from(target_index).map_err(|_| {
            job_failed(
                "SESSION_JOB_INPUT_LIMIT_EXCEEDED",
                "Print sayfa limiti aşıldı",
            )
        })?;
        target
            .pages_mut()
            .copy_page_from_document(&source, source_index, target_index)
            .map_err(|_| {
                job_failed(
                    "SESSION_JOB_PRINT_PREPARATION_FAILED",
                    "Print sayfası kopyalanamadı",
                )
            })?;
    }
    target.save_to_file(output_path).map_err(|_| {
        job_failed(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            "Print PDF job çıktısı yazılamadı",
        )
    })
}

fn checked_target_width(page_width: f32, scale: f32) -> Result<i32, JobFailure> {
    if !scale.is_finite() || !(0.1..=8.0).contains(&scale) {
        return Err(job_failed(
            "SESSION_JOB_INVALID_RENDER_PARAMETERS",
            "Render scale sınır dışında",
        ));
    }
    let width = page_width * scale;
    if !width.is_finite() || !(1.0..=16_384.0).contains(&width) {
        return Err(job_failed(
            "SESSION_JOB_INVALID_RENDER_PARAMETERS",
            "Render genişliği sınır dışında",
        ));
    }
    Ok(width.round() as i32)
}

fn checked_output_path(job_dir: &Path, name: &str) -> Result<PathBuf, JobFailure> {
    let candidate = Path::new(name);
    if candidate.components().count() != 1 || candidate.file_name().is_none() {
        return Err(job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Job worker çıktı adı tek bir dosya adı olmalıdır",
        ));
    }
    Ok(job_dir.join(candidate))
}

fn checked_existing_output_path(job_dir: &Path, name: &str) -> Result<PathBuf, JobFailure> {
    let path = checked_output_path(job_dir, name)?;
    let canonical_job_dir = job_dir.canonicalize().map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("Job dizini doğrulanamadı: {error}"),
        )
    })?;
    let canonical_path = path.canonicalize().map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("Job çıktısı doğrulanamadı: {error}"),
        )
    })?;
    if !canonical_path.starts_with(canonical_job_dir) || !canonical_path.is_file() {
        return Err(job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Mutation validation job dizini dışındaki çıktıyı reddetti",
        ));
    }
    Ok(canonical_path)
}

fn validate_protocol_paths(request: &Path, response: &Path) -> Result<(), JobFailure> {
    let request_name = request.file_name().and_then(|name| name.to_str());
    let response_name = response.file_name().and_then(|name| name.to_str());
    let request_id = request_name
        .and_then(|name| name.strip_prefix("worker-request-"))
        .and_then(|name| name.strip_suffix(".json"));
    let response_id = response_name
        .and_then(|name| name.strip_prefix("worker-response-"))
        .and_then(|name| name.strip_suffix(".json"));
    if !request.is_absolute()
        || !response.is_absolute()
        || request.parent() != response.parent()
        || request_id.is_none()
        || request_id != response_id
        || request_id.is_some_and(|id| id.parse::<u64>().is_err())
    {
        return Err(job_failed(
            "SESSION_JOB_WORKER_PATH_REJECTED",
            "Job worker IPC yolları reddedildi",
        ));
    }
    Ok(())
}

fn argument_path(flag: &str) -> Result<PathBuf, JobFailure> {
    let mut args = std::env::args_os();
    while let Some(argument) = args.next() {
        if argument == flag {
            return args.next().map(PathBuf::from).ok_or_else(|| {
                job_failed("SESSION_JOB_WORKER_PROTOCOL_ERROR", "Worker argümanı eksik")
            });
        }
    }
    Err(job_failed(
        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
        format!("Worker argümanı bulunamadı: {flag}"),
    ))
}

fn write_worker_response(path: &Path, response: &JobResponse) -> Result<(), JobFailure> {
    let bytes = serde_json::to_vec(response).map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Worker response encode edilemedi: {error}"),
        )
    })?;
    if bytes.len() as u64 > MAX_JOB_RESPONSE_BYTES {
        return Err(job_failed(
            "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
            "Worker response boyut limiti aşıldı",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Worker response dosyası oluşturulamadı: {error}"),
        )
    })?;
    file.write_all(&bytes).map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Worker response yazılamadı: {error}"),
        )
    })?;
    file.sync_all().map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("Worker response diske yazılamadı: {error}"),
        )
    })
}

#[cfg(not(test))]
fn create_cancellation_marker(path: &Path) -> Result<(), SessionJobOperationError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(file) => file.sync_all().map_err(|error| {
            SessionJobOperationError::failed_with_code(
                "SESSION_JOB_CANCELLATION_SIGNAL_FAILED",
                format!("Prepress cancellation marker diske yazılamadı: {error}"),
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_CANCELLATION_SIGNAL_FAILED",
            format!("Prepress cancellation marker oluşturulamadı: {error}"),
        )),
    }
}

#[cfg(not(test))]
fn write_json_create_new<T: Serialize>(
    path: &Path,
    value: &T,
    maximum: u64,
) -> Result<(), SessionJobOperationError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Job worker request encode edilemedi: {error}"),
        )
    })?;
    if bytes.len() as u64 > maximum {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_INPUT_LIMIT_EXCEEDED",
            "Job worker request boyut limiti aşıldı",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Job worker request dosyası oluşturulamadı: {error}"),
        )
    })?;
    file.write_all(&bytes).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Job worker request yazılamadı: {error}"),
        )
    })?;
    file.sync_all().map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Job worker request diske yazılamadı: {error}"),
        )
    })
}

#[cfg(not(test))]
fn read_json_bounded<T: for<'de> Deserialize<'de>>(
    path: &Path,
    maximum: u64,
) -> Result<T, SessionJobOperationError> {
    let metadata = fs::metadata(path).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Job worker response bulunamadı: {error}"),
        )
    })?;
    if metadata.len() > maximum {
        return Err(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_LIMIT_EXCEEDED",
            "Job worker response boyut limiti aşıldı",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| {
            SessionJobOperationError::failed_with_code(
                "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                format!("Job worker response okunamadı: {error}"),
            )
        })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Job worker response decode edilemedi: {error}"),
        )
    })
}

fn read_json_bounded_worker<T: for<'de> Deserialize<'de>>(
    path: &Path,
    maximum: u64,
) -> Result<T, JobFailure> {
    let metadata = fs::metadata(path).map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Worker request bulunamadı: {error}"),
        )
    })?;
    if metadata.len() > maximum {
        return Err(job_failed(
            "SESSION_JOB_INPUT_LIMIT_EXCEEDED",
            "Worker request boyut limiti aşıldı",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| {
            JobFailure::new(
                "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                format!("Worker request okunamadı: {error}"),
            )
        })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        JobFailure::new(
            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
            format!("Worker request decode edilemedi: {error}"),
        )
    })
}

fn operation_failure(error: SessionJobOperationError) -> JobFailure {
    match error {
        SessionJobOperationError::Failed(failure) => JobFailure::new(failure.code, failure.message),
        SessionJobOperationError::Cancelled(_) => {
            job_failed("SESSION_JOB_WORKER_CANCELLED", "Worker işlemi iptal edildi")
        }
    }
}

fn pdfium_unavailable(message: String) -> JobFailure {
    JobFailure::new("SESSION_JOB_PDF_ENGINE_UNAVAILABLE", message)
}

fn pdf_open_failed(message: String) -> JobFailure {
    JobFailure::new("SESSION_JOB_PDF_OPEN_FAILED", message)
}

fn job_failed(code: impl Into<String>, message: impl Into<String>) -> JobFailure {
    JobFailure::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanned_page_heuristic_matches_previous_per_page_threshold() {
        assert!(is_scanned_page_text(""));
        assert!(is_scanned_page_text("   \n\t  "));
        assert!(is_scanned_page_text("  nineteen characters "));
        assert!(!is_scanned_page_text("twenty characters!!!"));
        assert!(!is_scanned_page_text(
            "Bu sayfa gerçek bir metin katmanı içerir."
        ));
    }

    #[test]
    fn output_path_rejects_parent_traversal() {
        let root = Path::new("/tmp/job");
        assert!(checked_output_path(root, "../escape.png").is_err());
        assert!(checked_output_path(root, "nested/escape.png").is_err());
        assert_eq!(
            checked_output_path(root, "page.png").unwrap(),
            root.join("page.png")
        );
    }

    #[test]
    fn protocol_paths_must_share_the_private_job_directory() {
        // temp_dir() is absolute on every platform; "/tmp" is not on Windows.
        let root = std::env::temp_dir();
        assert!(validate_protocol_paths(
            &root.join("job").join("worker-request-1.json"),
            &root.join("job").join("worker-response-1.json")
        )
        .is_ok());
        assert!(validate_protocol_paths(
            &root.join("job").join("worker-request-1.json"),
            &root.join("other").join("worker-response-1.json")
        )
        .is_err());
        assert!(validate_protocol_paths(
            Path::new("job/worker-request-1.json"),
            Path::new("job/worker-response-1.json")
        )
        .is_err());
    }

    #[test]
    fn test_worker_protocol_v3_scan_sensitive_data_request_roundtrip() {
        let request = JobRequest {
            protocol_version: JOB_WORKER_PROTOCOL_VERSION,
            snapshot_path: PathBuf::from("/tmp/job/snapshot.pdf"),
            credential: Some("pass".to_string()),
            authoritative_job_id: "job-id".to_string(),
            authoritative_session_id: "session-id".to_string(),
            command: JobCommand::ScanSensitiveData {
                patterns: vec!["\\b[0-9]{11}\\b".to_string()],
            },
        };

        let json = serde_json::to_string(&request).unwrap();
        let deserialized: JobRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.protocol_version, JOB_WORKER_PROTOCOL_VERSION);
        if let JobCommand::ScanSensitiveData { patterns } = deserialized.command {
            assert_eq!(patterns, vec!["\\b[0-9]{11}\\b"]);
        } else {
            panic!("Expected ScanSensitiveData command");
        }
    }

    #[test]
    fn protocol_v9_roundtrips_explicit_prepress_identity_without_credentials_in_arguments() {
        let command = JobCommand::GenerateRgbDerivedCoveragePreview {
            content_generation: 9,
            physical_page_index: 4,
            request_id: "preview-request".to_string(),
            threshold_percent: 275.0,
            scale: 1.25,
        };
        let json = serde_json::to_string(&command).unwrap();
        assert!(!json.contains("credential"));
        let decoded: JobCommand = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(decoded, JobCommand::GenerateRgbDerivedCoveragePreview {
            content_generation: 9,
            physical_page_index: 4,
            ref request_id,
            threshold_percent: 275.0,
            scale: 1.25,
        } if request_id == "preview-request")
        );
        assert_eq!(JOB_WORKER_PROTOCOL_VERSION, 10);
    }

    #[test]
    fn test_worker_protocol_v4_sensitive_report_response_and_geometry_status_serialization() {
        use crate::redaction::{
            BoxOrigin, CoordinateSpace, GeometryStatus, GeometryUnit, IntrinsicRotation,
            PageGeometry, PdfPageBox, PdfRect, SensitiveScanItem, SensitiveScanReport,
        };

        let page_geom = PageGeometry {
            coordinate_space: CoordinateSpace::PdfUserSpace,
            unit: GeometryUnit::PdfPoint,
            origin: BoxOrigin::BottomLeft,
            media_box: PdfPageBox {
                left: 0.0,
                bottom: 0.0,
                width: 400.0,
                height: 600.0,
            },
            crop_box: None,
            effective_box: PdfPageBox {
                left: 0.0,
                bottom: 0.0,
                width: 400.0,
                height: 600.0,
            },
            intrinsic_rotation: IntrinsicRotation::Deg0,
        };

        let report = SensitiveScanReport {
            schema_version: 2,
            items: vec![
                SensitiveScanItem {
                    page_index: 0,
                    pattern_index: 0,
                    text_preview: "12345678901".to_string(),
                    geometry_status: GeometryStatus::Resolved,
                    page_geometry: Some(page_geom),
                    source_rects_pdf_points: vec![PdfRect {
                        x: 10.0,
                        y: 20.0,
                        width: 30.0,
                        height: 40.0,
                    }],
                    warnings: vec![],
                },
                SensitiveScanItem {
                    page_index: 1,
                    pattern_index: 1,
                    text_preview: "test@domain.com".to_string(),
                    geometry_status: GeometryStatus::Unresolved,
                    page_geometry: None,
                    source_rects_pdf_points: vec![],
                    warnings: vec![],
                },
            ],
            warnings: vec![],
            truncated: false,
        };

        let response = JobResponse {
            protocol_version: JOB_WORKER_PROTOCOL_VERSION,
            result: Ok(JobResult::SensitiveReport(report)),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"geometry_status\":\"resolved\""));
        assert!(json.contains("\"geometry_status\":\"unresolved\""));
        assert!(json.contains("\"intrinsic_rotation\":0"));

        let deserialized: JobResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.protocol_version, JOB_WORKER_PROTOCOL_VERSION);
        assert_eq!(format!("{:?}", deserialized), format!("{:?}", response));
    }

    #[test]
    fn test_worker_protocol_version_validation_helpers() {
        // Current version should succeed
        assert!(validate_request_protocol_version(JOB_WORKER_PROTOCOL_VERSION).is_ok());
        assert!(validate_response_protocol_version(JOB_WORKER_PROTOCOL_VERSION).is_ok());

        // Previous version request mismatch -> SESSION_JOB_WORKER_PROTOCOL_ERROR
        let req_err =
            validate_request_protocol_version(JOB_WORKER_PROTOCOL_VERSION - 1).unwrap_err();
        assert_eq!(req_err.code, "SESSION_JOB_WORKER_PROTOCOL_ERROR");

        // Version 2 response mismatch -> SESSION_JOB_WORKER_PROTOCOL_ERROR
        let resp_err = validate_response_protocol_version(2).unwrap_err();
        if let SessionJobOperationError::Failed(f) = resp_err {
            assert_eq!(f.code, "SESSION_JOB_WORKER_PROTOCOL_ERROR");
        } else {
            panic!("Expected SessionJobOperationError::Failed");
        }
    }

    #[test]
    fn fill_form_fields_mutation_round_trips_through_job_worker_dispatch() {
        use crate::forms::fill::{FormFieldEdit, FormFieldValue};
        let directory = tempfile::tempdir().unwrap();
        let snapshot = directory.path().join("snapshot.pdf");
        crate::forms::test_support::standard_fixture(&snapshot);
        let command = |edits: Vec<FormFieldEdit>, output_name: &str| {
            let command = JobCommand::MutatePdf {
                recipe: MutationRecipe::FillFormFields { edits },
                output_name: output_name.to_string(),
            };
            // The job IPC carries commands as JSON.
            serde_json::from_str::<JobCommand>(&serde_json::to_string(&command).unwrap()).unwrap()
        };

        let result = execute_command(
            directory.path(),
            &snapshot,
            None,
            "form-fill-job",
            "form-fill-session",
            command(
                vec![
                    FormFieldEdit {
                        page_index: 0,
                        annot_index: 0,
                        field_name: "name".to_string(),
                        value: FormFieldValue::Text {
                            text: "Grace".to_string(),
                        },
                    },
                    FormFieldEdit {
                        page_index: 0,
                        annot_index: 7,
                        field_name: "color".to_string(),
                        value: FormFieldValue::Radio,
                    },
                ],
                "filled.pdf",
            ),
        )
        .unwrap();
        let output = directory.path().join("filled.pdf");
        assert!(matches!(result, JobResult::File(ref path) if path == &output));
        let widgets = crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(&output, None)
                .map_err(|error| error.to_string())?;
            crate::forms::read_page_form_fields(&document, 0).map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        assert_eq!(widgets[0].value.as_deref(), Some("Grace"));
        assert_eq!(
            widgets.iter().find(|w| w.annot_index == 7).unwrap().checked,
            Some(true)
        );

        let failure = execute_command(
            directory.path(),
            &snapshot,
            None,
            "form-fill-job",
            "form-fill-session",
            command(
                vec![FormFieldEdit {
                    page_index: 0,
                    annot_index: 3,
                    field_name: "locked".to_string(),
                    value: FormFieldValue::Text {
                        text: "x".to_string(),
                    },
                }],
                "rejected.pdf",
            ),
        )
        .unwrap_err();
        assert_eq!(failure.code, "SESSION_MUTATION_WORKER_FAILED");
        assert!(failure.message.contains("read-only"), "{}", failure.message);
    }

    #[test]
    fn author_form_fields_mutation_round_trips_through_job_worker_dispatch() {
        use crate::forms::author::{FormAuthorOp, NewField};
        let directory = tempfile::tempdir().unwrap();
        let snapshot = directory.path().join("snapshot.pdf");
        crate::forms::test_support::standard_fixture(&snapshot);
        let command = |ops: Vec<FormAuthorOp>, output_name: &str| {
            let command = JobCommand::MutatePdf {
                recipe: MutationRecipe::AuthorFormFields { ops },
                output_name: output_name.to_string(),
            };
            serde_json::from_str::<JobCommand>(&serde_json::to_string(&command).unwrap()).unwrap()
        };
        let rect = crate::annotations::NormalizedPageRect {
            x: 0.55,
            y: 0.3,
            width: 0.3,
            height: 0.04,
        };

        let result = execute_command(
            directory.path(),
            &snapshot,
            None,
            "form-author-job",
            "form-author-session",
            command(
                vec![FormAuthorOp::Add {
                    page_index: 0,
                    normalized_rect: rect,
                    field: NewField::Text { multiline: false },
                    name: Some("Email".to_string()),
                }],
                "authored.pdf",
            ),
        )
        .unwrap();
        let output = directory.path().join("authored.pdf");
        assert!(matches!(result, JobResult::File(ref path) if path == &output));
        let widgets = crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
            let document = pdfium
                .load_pdf_from_file(&output, None)
                .map_err(|error| error.to_string())?;
            crate::forms::read_page_form_fields(&document, 0).map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        assert!(widgets.iter().any(|widget| widget.field_name == "Email"));

        let failure = execute_command(
            directory.path(),
            &snapshot,
            None,
            "form-author-job",
            "form-author-session",
            command(
                vec![FormAuthorOp::Delete {
                    page_index: 0,
                    annot_index: 0,
                    field_name: "notes".to_string(),
                }],
                "rejected.pdf",
            ),
        )
        .unwrap_err();
        assert_eq!(failure.code, "SESSION_MUTATION_WORKER_FAILED");
        assert!(failure.message.contains("'name'"), "{}", failure.message);
    }

    #[test]
    fn add_annotation_mutation_round_trips_rect_through_job_worker_dispatch() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = directory.path().join("snapshot.pdf");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/annotations.pdf"),
            &snapshot,
        )
        .unwrap();
        // Asimetrik rect: x≠y ve width≠height olmalı ki eksen takası yakalansın.
        // 612x792 sayfada user space [100 500 150 520]'ye denk gelir.
        let rect = crate::annotations::NormalizedPageRect {
            x: 100.0 / 612.0,
            y: 272.0 / 792.0,
            width: 50.0 / 612.0,
            height: 20.0 / 792.0,
        };

        for (index, annot_type) in ["Highlight", "Underline", "Text"].into_iter().enumerate() {
            let output_name = format!("annotation-{index}.pdf");
            let result = execute_command(
                directory.path(),
                &snapshot,
                None,
                "annotation-round-trip-job",
                "annotation-round-trip-session",
                JobCommand::MutatePdf {
                    recipe: MutationRecipe::AddAnnotation {
                        page_index: 0,
                        annot_type: annot_type.to_string(),
                        normalized_rect: rect.clone(),
                        color: "#FFCC00".to_string(),
                        contents: Some(format!("{annot_type} worker round-trip")),
                    },
                    output_name: output_name.clone(),
                },
            )
            .unwrap_or_else(|error| panic!("{annot_type} mutation başarısız: {error:?}"));
            let output = directory.path().join(&output_name);
            assert!(matches!(result, JobResult::File(ref path) if path == &output));

            let annotations = crate::pdf_engine::with_pdfium(|pdfium| -> Result<_, String> {
                let document = pdfium
                    .load_pdf_from_file(&output, None)
                    .map_err(|error| error.to_string())?;
                crate::annotations::read_page_annotations(&document, 0)
            })
            .unwrap();
            let added = annotations.last().unwrap();
            assert_eq!(added.annot_type, annot_type);
            let read = added.normalized_rect.as_ref().unwrap();
            for (field, read, written) in [
                ("x", read.x, rect.x),
                ("y", read.y, rect.y),
                ("width", read.width, rect.width),
                ("height", read.height, rect.height),
            ] {
                assert!(
                    (read - written).abs() <= 1e-5,
                    "{annot_type} {field}: okunan {read}, yazılan {written}"
                );
            }
        }
    }
}
