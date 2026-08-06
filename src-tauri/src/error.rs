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
        };
        state.serialize_field("type", err_type)?;

        let message = match self {
            AppError::Pdfium(m) | AppError::Thread(m) | AppError::Image(m) => m,
        };
        state.serialize_field("message", message)?;

        state.end()
    }
}
