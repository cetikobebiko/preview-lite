use crate::document_session::{
    DocumentSessionError, DocumentSessionStore, SessionJobOperationError, SessionJobOutcome,
};
use rusty_tesseract::{Args, Image};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
#[derive(Serialize, Deserialize, Debug)]
pub struct OcrWord {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub confidence: f32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OcrPageResult {
    pub words: Vec<OcrWord>,
}

#[tauri::command]
pub async fn detect_scanned_page(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    store: tauri::State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<bool>, DocumentSessionError> {
    detect_scanned_page_for_session(
        &store,
        &job_id,
        &session_id,
        expected_content_generation,
        page_index,
    )
    .await
}

pub(crate) async fn detect_scanned_page_for_session(
    store: &DocumentSessionStore,
    job_id: &str,
    session_id: &str,
    expected_content_generation: u64,
    page_index: u32,
) -> Result<SessionJobOutcome<bool>, DocumentSessionError> {
    store
        .run_snapshot_job(
            job_id,
            session_id,
            expected_content_generation,
            move |context| match crate::job_worker::run_job(
                &context,
                crate::job_worker::JobCommand::DetectScannedPage { page_index },
                std::time::Duration::from_secs(60),
            )? {
                crate::job_worker::JobResult::Boolean(value) => Ok(value),
                _ => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Scanned-page worker beklenmeyen sonuç döndürdü",
                )),
            },
        )
        .await
}

#[tauri::command]
pub fn get_available_ocr_languages() -> Result<Vec<String>, String> {
    let langs = rusty_tesseract::get_tesseract_langs()
        .map_err(|e| format!("Tesseract bulunamadı: {}", e))?;
    Ok(langs)
}

#[tauri::command]
pub async fn run_ocr_on_page(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    lang: String,
    store: tauri::State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<OcrPageResult>, DocumentSessionError> {
    run_ocr_on_page_for_session(
        &store,
        &job_id,
        &session_id,
        expected_content_generation,
        page_index,
        lang,
    )
    .await
}

pub(crate) async fn run_ocr_on_page_for_session(
    store: &DocumentSessionStore,
    job_id: &str,
    session_id: &str,
    expected_content_generation: u64,
    page_index: u32,
    lang: String,
) -> Result<SessionJobOutcome<OcrPageResult>, DocumentSessionError> {
    store
        .run_snapshot_job(
            job_id,
            session_id,
            expected_content_generation,
            move |context| {
                let image_path = match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::RenderOcrPage {
                        page_index,
                        output_name: "ocr-input.png".to_string(),
                        scale: 2.0,
                    },
                    std::time::Duration::from_secs(120),
                )? {
                    crate::job_worker::JobResult::File(path) => path,
                    _ => {
                        return Err(SessionJobOperationError::failed_with_code(
                            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                            "OCR preparation worker beklenmeyen sonuç döndürdü",
                        ))
                    }
                };
                let image_path = crate::job_worker::validate_output_file(&context, &image_path)?;
                let image = image::open(&image_path).map_err(|error| {
                    SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_OUTPUT_READ_FAILED",
                        format!("OCR preparation çıktısı okunamadı: {error}"),
                    )
                })?;

                let tesseract_image = Image::from_dynamic_image(&image)
                    .map_err(|error| SessionJobOperationError::failed(error.to_string()))?;
                let args = Args {
                    lang,
                    ..Default::default()
                };
                let output = rusty_tesseract::image_to_data(&tesseract_image, &args)
                    .map_err(|error| SessionJobOperationError::failed(error.to_string()))?;
                let words = output
                    .data
                    .into_iter()
                    .filter(|record| record.conf >= 0.0 && !record.text.trim().is_empty())
                    .map(|record| OcrWord {
                        text: record.text,
                        x: record.left,
                        y: record.top,
                        width: record.width,
                        height: record.height,
                        confidence: record.conf,
                    })
                    .collect();

                Ok::<OcrPageResult, SessionJobOperationError>(OcrPageResult { words })
            },
        )
        .await
}

#[tauri::command]
pub async fn export_as_searchable_pdf(
    _app_handle: AppHandle,
    _job_id: String,
    _session_id: String,
    _expected_content_generation: u64,
    _output_path: String,
    _lang: String,
    _store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, crate::document_session::DocumentSessionError> {
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "Aranabilir PDF dışa aktarma bu sürümde devre dışıdır",
    ))
}
