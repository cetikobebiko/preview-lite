//! Worker steps of the copy-producing jobs, shared by the session commands and
//! the batch runner.
//!
//! Each function runs one job worker command against `context.snapshot_path`
//! and returns what it produced inside the job directory. Nothing here
//! publishes to a user path: the session commands publish with
//! `accept_job_output_file` / `write_atomically`, the batch runner with
//! `accept_job_output_set`.

use crate::document_session::{SessionJobContext, SessionJobOperationError};
use crate::job_worker::{run_job, JobCommand, JobResult};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) const OPTIMIZED_COPY_NAME: &str = "optimized-copy.pdf";
pub(crate) const SECURITY_COPY_NAME: &str = "security-copy.pdf";

const OPTIMIZE_TIMEOUT: Duration = Duration::from_secs(600);
const SECURITY_TIMEOUT: Duration = Duration::from_secs(300);
const DOCUMENT_EXPORT_TIMEOUT: Duration = Duration::from_secs(300);
const TEXT_EXPORT_TIMEOUT: Duration = Duration::from_secs(180);
const IMAGE_EXPORT_TIMEOUT: Duration = Duration::from_secs(300);

fn protocol_error(message: &str) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code("SESSION_JOB_WORKER_PROTOCOL_ERROR", message)
}

pub(crate) fn document_export_name(format: crate::document_export::DocumentExportFormat) -> String {
    format!("document-export.{}", format.extension())
}

/// Optimized copy of the snapshot; the path is the worker's unvalidated output.
pub(crate) fn produce_optimized_copy(
    context: &SessionJobContext,
    settings: crate::pdf_optimizer::OptimizeSettings,
) -> Result<(PathBuf, crate::pdf_optimizer::OptimizeReport), SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let result = run_job(
        context,
        JobCommand::OptimizedCopy {
            settings,
            output_name: OPTIMIZED_COPY_NAME.to_string(),
        },
        OPTIMIZE_TIMEOUT,
    )?;
    let JobResult::OptimizedCopy { path, report } = result else {
        return Err(protocol_error(
            "Optimizasyon worker'ı beklenmeyen sonuç döndürdü",
        ));
    };
    Ok((path, report))
}

/// Password-protected or unprotected copy of the snapshot.
pub(crate) fn produce_security_copy(
    context: &SessionJobContext,
    op: crate::document_security::SecurityOp,
) -> Result<PathBuf, SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let result = run_job(
        context,
        JobCommand::SecurityCopy {
            op,
            output_name: SECURITY_COPY_NAME.to_string(),
        },
        SECURITY_TIMEOUT,
    )?;
    let JobResult::File(source) = result else {
        return Err(protocol_error(
            "Güvenlik kopyası worker'ı beklenmeyen sonuç döndürdü",
        ));
    };
    Ok(source)
}

/// Word, PowerPoint, HTML, RTF or Markdown export of `pages`.
pub(crate) fn produce_document_export(
    context: &SessionJobContext,
    format: crate::document_export::DocumentExportFormat,
    title: String,
    pages: Vec<u32>,
) -> Result<PathBuf, SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let result = run_job(
        context,
        JobCommand::ExportDocument {
            format,
            title,
            pages,
            output_name: document_export_name(format),
        },
        DOCUMENT_EXPORT_TIMEOUT,
    )?;
    let JobResult::File(source) = result else {
        return Err(protocol_error(
            "Belge dışa aktarma worker'ı beklenmeyen sonuç döndürdü",
        ));
    };
    Ok(source)
}

/// Text of `pages` (`None`: all) as plain text or Markdown (`format`).
pub(crate) fn produce_text(
    context: &SessionJobContext,
    pages: Option<Vec<u32>>,
    format: String,
) -> Result<String, SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    match run_job(
        context,
        JobCommand::ExtractText { pages, format },
        TEXT_EXPORT_TIMEOUT,
    )? {
        JobResult::Text(text) => Ok(text),
        _ => Err(protocol_error(
            "Text export worker beklenmeyen sonuç döndürdü",
        )),
    }
}

/// Page images, each checked to lie inside the job directory.
pub(crate) fn produce_page_images(
    context: &SessionJobContext,
    pages: Vec<u32>,
    format: String,
    dpi: f32,
) -> Result<Vec<PathBuf>, SessionJobOperationError> {
    let result = run_job(
        context,
        JobCommand::ExportPagesAsImages { pages, format, dpi },
        IMAGE_EXPORT_TIMEOUT,
    )?;
    let JobResult::Files(files) = result else {
        return Err(protocol_error(
            "Image export worker beklenmeyen sonuç döndürdü",
        ));
    };
    files
        .iter()
        .map(|source| crate::job_worker::validate_output_file(context, source))
        .collect()
}
