use crate::document_session::{
    DocumentSessionError, DocumentSessionStore, SessionJobOperationError, SessionJobOutcome,
};
use rusty_tesseract::{Args, Image};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
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

/// Belge başına tek snapshot job: her sayfa için taranmış-sayfa bayrağı döner.
/// Sayfa başına job (her biri snapshot kopyası + SHA-256 + iki worker süreci)
/// başlatmamak için frontend sonucu (session, content generation) başına cache'ler.
#[tauri::command]
pub async fn detect_scanned_pages(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    store: tauri::State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<Vec<bool>>, DocumentSessionError> {
    detect_scanned_pages_for_session(&store, &job_id, &session_id, expected_content_generation)
        .await
}

pub(crate) async fn detect_scanned_pages_for_session(
    store: &DocumentSessionStore,
    job_id: &str,
    session_id: &str,
    expected_content_generation: u64,
) -> Result<SessionJobOutcome<Vec<bool>>, DocumentSessionError> {
    store
        .run_snapshot_job(
            job_id,
            session_id,
            expected_content_generation,
            move |context| match crate::job_worker::run_job(
                &context,
                crate::job_worker::JobCommand::DetectScannedPages,
                DETECT_SCANNED_PAGES_TIMEOUT,
            )? {
                crate::job_worker::JobResult::ScannedPages(flags) => Ok(flags),
                _ => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Scanned-page worker beklenmeyen sonuç döndürdü",
                )),
            },
        )
        .await
}

const DETECT_SCANNED_PAGES_TIMEOUT: Duration = Duration::from_secs(120);
const OCR_LANGUAGE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const OCR_LANGUAGE_FAILURE_RETRY_AFTER: Duration = Duration::from_secs(60);

static OCR_LANGUAGE_CACHE: OcrLanguageCache = OcrLanguageCache::new();

enum CachedOcrLanguages {
    Ready(Vec<String>),
    Failed { message: String, at: Instant },
}

/// Uygulama düzeyi Tesseract dil listesi cache'i.
///
/// Başarılı sonuç süreç ömrü boyunca tutulur. Hata (zaman aşımı dahil) da
/// cache'lenir ve `retry_after` dolana kadar tekrar denenmez. Kilit sorgu
/// boyunca tutulur; eşzamanlı çağrılar tek tesseract sürecini paylaşır.
pub(crate) struct OcrLanguageCache {
    state: tokio::sync::Mutex<Option<CachedOcrLanguages>>,
}

impl OcrLanguageCache {
    pub(crate) const fn new() -> Self {
        Self {
            state: tokio::sync::Mutex::const_new(None),
        }
    }

    pub(crate) async fn get_or_probe<F>(
        &self,
        probe: F,
        timeout: Duration,
        retry_after: Duration,
    ) -> Result<Vec<String>, String>
    where
        F: FnOnce() -> Result<Vec<String>, String> + Send + 'static,
    {
        let mut state = self.state.lock().await;
        match state.as_ref() {
            Some(CachedOcrLanguages::Ready(langs)) => return Ok(langs.clone()),
            Some(CachedOcrLanguages::Failed { message, at }) if at.elapsed() < retry_after => {
                return Err(message.clone())
            }
            _ => {}
        }
        // Zaman aşımında blocking thread (ve başlattığı tesseract süreci) kendi
        // kendine tamamlanır; sonuç cache'e yazılmaz, hata cache'lenir.
        let result =
            match tokio::time::timeout(timeout, tauri::async_runtime::spawn_blocking(probe)).await
            {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err("Tesseract dil sorgusu beklenmedik biçimde sonlandı".to_string()),
                Err(_) => Err(format!(
                    "Tesseract dil sorgusu {} sn içinde tamamlanmadı",
                    timeout.as_secs()
                )),
            };
        *state = Some(match &result {
            Ok(langs) => CachedOcrLanguages::Ready(langs.clone()),
            Err(message) => CachedOcrLanguages::Failed {
                message: message.clone(),
                at: Instant::now(),
            },
        });
        result
    }
}

fn probe_tesseract_languages() -> Result<Vec<String>, String> {
    rusty_tesseract::get_tesseract_langs().map_err(|e| format!("Tesseract bulunamadı: {}", e))
}

#[tauri::command]
pub async fn get_available_ocr_languages() -> Result<Vec<String>, String> {
    OCR_LANGUAGE_CACHE
        .get_or_probe(
            probe_tesseract_languages,
            OCR_LANGUAGE_PROBE_TIMEOUT,
            OCR_LANGUAGE_FAILURE_RETRY_AFTER,
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const LONG: Duration = Duration::from_secs(3600);

    fn counting_probe(
        calls: &Arc<AtomicUsize>,
        result: Result<Vec<String>, String>,
    ) -> impl FnOnce() -> Result<Vec<String>, String> + Send + 'static {
        let calls = Arc::clone(calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            result
        }
    }

    #[test]
    fn language_command_is_async() {
        fn assert_future<F: std::future::Future<Output = Result<Vec<String>, String>>>(_: F) {}
        // Future poll edilmeden düşürülür; tesseract çalıştırılmaz.
        assert_future(get_available_ocr_languages());
    }

    #[tokio::test]
    async fn language_probe_success_is_cached_for_the_process() {
        let cache = OcrLanguageCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let langs = vec!["eng".to_string(), "tur".to_string()];

        for _ in 0..3 {
            let result = cache
                .get_or_probe(counting_probe(&calls, Ok(langs.clone())), LONG, LONG)
                .await;
            assert_eq!(result, Ok(langs.clone()));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn language_probe_error_is_returned_and_cached_until_retry_window() {
        let cache = OcrLanguageCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let failure = Err("Tesseract bulunamadı: missing".to_string());

        let first = cache
            .get_or_probe(counting_probe(&calls, failure.clone()), LONG, LONG)
            .await;
        let second = cache
            .get_or_probe(counting_probe(&calls, Ok(vec!["eng".into()])), LONG, LONG)
            .await;
        assert_eq!(first, failure);
        assert_eq!(second, failure);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let retried = cache
            .get_or_probe(
                counting_probe(&calls, Ok(vec!["eng".into()])),
                LONG,
                Duration::ZERO,
            )
            .await;
        assert_eq!(retried, Ok(vec!["eng".to_string()]));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn language_probe_timeout_returns_and_caches_an_error() {
        let cache = OcrLanguageCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let slow_calls = Arc::clone(&calls);

        let result = cache
            .get_or_probe(
                move || {
                    slow_calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(300));
                    Ok(vec!["eng".to_string()])
                },
                Duration::from_millis(20),
                LONG,
            )
            .await;
        let message = result.unwrap_err();
        assert!(message.contains("tamamlanmadı"), "{message}");

        let cached = cache
            .get_or_probe(counting_probe(&calls, Ok(vec!["eng".into()])), LONG, LONG)
            .await;
        assert_eq!(cached, Err(message));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_language_requests_share_one_probe() {
        let cache = Arc::new(OcrLanguageCache::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let tasks = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    cache
                        .get_or_probe(
                            move || {
                                calls.fetch_add(1, Ordering::SeqCst);
                                std::thread::sleep(Duration::from_millis(50));
                                Ok(vec!["eng".to_string()])
                            },
                            LONG,
                            LONG,
                        )
                        .await
                })
            })
            .collect::<Vec<_>>();
        for task in tasks {
            assert_eq!(task.await.unwrap(), Ok(vec!["eng".to_string()]));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
