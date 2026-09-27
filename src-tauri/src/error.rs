use serde::ser::SerializeStruct;
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("PDFium Hatası: {0}")]
    Pdfium(String),

    #[error("Thread Hatası: {0}")]
    Thread(String),

    #[error("Görsel Hatası: {0}")]
    Image(String),

    #[error("PDFium Runtime Unavailable: {0}")]
    RuntimeUnavailable(String),

    #[error("PDFium Runtime Architecture Mismatch: expected {expected}, found {found}")]
    RuntimeArchitectureMismatch { expected: String, found: String },

    #[error("PDFium Runtime Integrity Mismatch: {0}")]
    RuntimeIntegrityMismatch(String),

    #[error("PDFium Runtime Load Failed: {0}")]
    RuntimeLoadFailed(String),

    #[error("PDFium Runtime Unsupported: {0}")]
    RuntimeUnsupported(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("AppError", 2)?;

        let err_type = match self {
            AppError::Pdfium(_) => "PdfiumError",
            AppError::Thread(_) => "ThreadError",
            AppError::Image(_) => "ImageError",
            AppError::RuntimeUnavailable(_) => "RuntimeUnavailable",
            AppError::RuntimeArchitectureMismatch { .. } => "RuntimeArchitectureMismatch",
            AppError::RuntimeIntegrityMismatch(_) => "RuntimeIntegrityMismatch",
            AppError::RuntimeLoadFailed(_) => "RuntimeLoadFailed",
            AppError::RuntimeUnsupported(_) => "RuntimeUnsupported",
        };
        state.serialize_field("type", err_type)?;

        let message = match self {
            AppError::Pdfium(m)
            | AppError::Thread(m)
            | AppError::Image(m)
            | AppError::RuntimeUnavailable(m)
            | AppError::RuntimeIntegrityMismatch(m)
            | AppError::RuntimeLoadFailed(m)
            | AppError::RuntimeUnsupported(m) => m,
            AppError::RuntimeArchitectureMismatch { expected, found } => {
                &format!("expected {expected}, found {found}")
            }
        };
        state.serialize_field("message", message)?;

        state.end()
    }
}
