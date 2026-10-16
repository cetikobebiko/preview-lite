use crate::document_session::{
    ChildOutputLimits, DocumentSessionError, DocumentSessionStore, SessionJobFailure,
    SessionJobOperationError, SessionJobOutcome,
};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::AppHandle;
#[derive(Serialize, Deserialize, Debug, PartialEq)]
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
/// Sorgu kendi zaman aşımında süreci öldürür; cache'in dış zaman aşımı yalnız
/// öldürme/reap takılırsa devreye giren yedektir.
const OCR_LANGUAGE_PROBE_GRACE: Duration = Duration::from_secs(5);
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
        // Tesseract sürecini sorgunun kendi zaman aşımı öldürür. Bu dış zaman
        // aşımı yedektir: dolarsa blocking thread arka planda tamamlanır,
        // sonucu cache'e yazılmaz, hata cache'lenir.
        let result = match tokio::time::timeout(
            timeout,
            tauri::async_runtime::spawn_blocking(probe),
        )
        .await
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

    /// Başarılı bir sorgu cache'lendiyse dil listesini döner; sorgu başlatmaz.
    async fn ready_languages(&self) -> Option<Vec<String>> {
        match self.state.lock().await.as_ref() {
            Some(CachedOcrLanguages::Ready(langs)) => Some(langs.clone()),
            _ => None,
        }
    }
}

const MAX_OCR_LANG_LEN: usize = 128;

/// `lang` tesseract'a `-l` değeri olarak geçer. Her `+` bileşeni ya
/// `[A-Za-z0-9_]+` desenine uymalı ya da `--list-langs` çıktısında (ör.
/// `script/Latin`) birebir bulunmalıdır; böylece seçenek ya da yol enjekte
/// edilemez.
fn is_valid_ocr_lang(lang: &str, installed: Option<&[String]>) -> bool {
    if lang.is_empty() || lang.len() > MAX_OCR_LANG_LEN {
        return false;
    }
    lang.split('+').all(|component| {
        let simple = !component.is_empty()
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        simple || installed.is_some_and(|langs| langs.iter().any(|known| known == component))
    })
}

fn tesseract_list_langs_command(program: &OsStr) -> Command {
    let mut command = Command::new(program);
    command.arg("--list-langs").stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `tesseract --list-langs` çalıştırır; `timeout` dolarsa süreç öldürülür ve
/// reap edilir. Çıkış kodu sıfır değilse hata döner.
fn probe_tesseract_languages(program: &OsStr, timeout: Duration) -> Result<Vec<String>, String> {
    let children = crate::document_session::SessionJobChildProcessOwner::default();
    let cancellation = crate::document_session::SessionJobCancellationCheckpoint::new(
        tokio_util::sync::CancellationToken::new(),
    );
    let output = children
        .run_with_bounded_output_timeout(
            &mut tesseract_list_langs_command(program),
            timeout,
            &cancellation,
            TESSERACT_LIST_LANGS_OUTPUT_LIMITS,
        )
        .map_err(|error| match tesseract_not_found(&error) {
            Some(not_found) => not_found.message,
            None => format!("Tesseract bulunamadı: {error}"),
        })?;
    if !output.status.success() {
        let detail = crate::child_stderr_detail(&output.stderr);
        return Err(if detail.is_empty() {
            format!(
                "Tesseract bulunamadı: dil listesi alınamadı ({})",
                output.status
            )
        } else {
            format!("Tesseract bulunamadı: {detail}")
        });
    }
    Ok(parse_tesseract_langs(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// `--list-langs` çıktısı: `List of available languages ...` başlığından
/// sonra satır başına bir dil. Başlığı stderr'e yazan sürümlerde ilk dil
/// atılmasın diye ilk satır yalnız başlıksa atlanır; boş satırlar yok sayılır.
fn parse_tesseract_langs(output: &str) -> Vec<String> {
    let mut lines = output.lines().map(str::trim).peekable();
    if lines
        .peek()
        .is_some_and(|line| line.starts_with("List of available languages"))
    {
        lines.next();
    }
    lines
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[tauri::command]
pub async fn get_available_ocr_languages() -> Result<Vec<String>, String> {
    OCR_LANGUAGE_CACHE
        .get_or_probe(
            || probe_tesseract_languages(&tesseract_program(), OCR_LANGUAGE_PROBE_TIMEOUT),
            OCR_LANGUAGE_PROBE_TIMEOUT + OCR_LANGUAGE_PROBE_GRACE,
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
    let installed = OCR_LANGUAGE_CACHE.ready_languages().await;
    if !is_valid_ocr_lang(&lang, installed.as_deref()) {
        return Err(DocumentSessionError::new(
            "OCR_LANGUAGE_INVALID",
            "OCR dili geçersiz",
        ));
    }
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
                    OCR_RENDER_TIMEOUT,
                )? {
                    crate::job_worker::JobResult::File(path) => path,
                    _ => {
                        return Err(SessionJobOperationError::failed_with_code(
                            "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                            "OCR preparation worker beklenmeyen sonuç döndürdü",
                        ))
                    }
                };
                // Worker'ın job dizinine yazdığı PNG doğrudan tesseract'a
                // verilir; görüntü ana süreçte decode edilmez ve oturum
                // geçici alanı dışına kopyalanmaz.
                let image_path = crate::job_worker::validate_output_file(&context, &image_path)?;
                let words = run_tesseract_tsv(
                    &context,
                    &tesseract_program(),
                    &image_path,
                    &lang,
                    TESSERACT_RUN_TIMEOUT,
                    TESSERACT_OUTPUT_LIMITS,
                )?;
                Ok::<OcrPageResult, SessionJobOperationError>(OcrPageResult { words })
            },
        )
        .await
}

const OCR_RENDER_TIMEOUT: Duration = Duration::from_secs(120);
const TESSERACT_RUN_TIMEOUT: Duration = Duration::from_secs(120);
const TESSERACT_OUTPUT_LIMITS: TesseractOutputLimits = TesseractOutputLimits {
    max_bytes: 16 * 1024 * 1024,
    max_words: 100_000,
};

const TESSERACT_STDERR_LIMIT: usize = 64 * 1024;
const TESSERACT_LIST_LANGS_OUTPUT_LIMITS: ChildOutputLimits = ChildOutputLimits {
    max_stdout_bytes: 1024 * 1024,
    max_stderr_bytes: TESSERACT_STDERR_LIMIT,
};

#[cfg(target_os = "windows")]
const TESSERACT_PROGRAM: &str = "tesseract.exe";
#[cfg(not(target_os = "windows"))]
const TESSERACT_PROGRAM: &str = "tesseract";

/// Dil listesi ve OCR'ın çalıştırdığı tesseract. PATH'te bulunursa çıplak ad
/// döner (`Command::new` PATH'te arar; eski rusty-tesseract davranışı).
/// Bulunamazsa bilinen kurulum dizinlerine bakılır: Finder/Dock'tan açılan
/// macOS uygulamalarının PATH'i `/usr/bin:/bin:/usr/sbin:/sbin` olduğundan
/// Homebrew/MacPorts kurulumu aksi halde görünmez. Hiçbiri yoksa çıplak ad
/// döner ve başlatma `SESSION_JOB_CHILD_PROGRAM_NOT_FOUND` ile başarısız olur.
/// Sonuç cache'lenmez; uygulama açıkken kurulan tesseract da bulunur.
fn tesseract_program() -> OsString {
    resolve_tesseract_program(
        std::env::var_os("PATH").as_deref(),
        &tesseract_fallback_dirs(),
        TESSERACT_PROGRAM,
    )
}

fn resolve_tesseract_program(
    path_var: Option<&OsStr>,
    fallback_dirs: &[PathBuf],
    program: &str,
) -> OsString {
    let in_path = path_var
        .map(|paths| std::env::split_paths(paths).any(|dir| is_executable(&dir.join(program))))
        .unwrap_or(false);
    if in_path {
        return OsString::from(program);
    }
    fallback_dirs
        .iter()
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
        .map_or_else(|| OsString::from(program), PathBuf::into_os_string)
}

fn tesseract_fallback_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .map(PathBuf::from)
            .to_vec()
    }
    #[cfg(target_os = "windows")]
    {
        // UB-Mannheim kurulumu varsayılan olarak PATH'e eklenmez.
        let mut dirs = Vec::new();
        for (variable, components) in [
            ("ProgramFiles", &["Tesseract-OCR"][..]),
            ("ProgramFiles(x86)", &["Tesseract-OCR"][..]),
            ("LOCALAPPDATA", &["Programs", "Tesseract-OCR"][..]),
        ] {
            if let Some(base) = std::env::var_os(variable) {
                dirs.push(
                    components
                        .iter()
                        .fold(PathBuf::from(base), |dir, part| dir.join(part)),
                );
            }
        }
        dirs
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        ["/usr/local/bin", "/usr/bin"].map(PathBuf::from).to_vec()
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Başlatma hatası "program bulunamadı" ise kullanıcıya gösterilecek hatayı
/// döner.
fn tesseract_not_found(error: &SessionJobOperationError) -> Option<SessionJobFailure> {
    match error {
        SessionJobOperationError::Failed(failure)
            if failure.code == crate::document_session::CHILD_PROGRAM_NOT_FOUND_CODE =>
        {
            Some(SessionJobFailure::new(
                "OCR_ENGINE_NOT_FOUND",
                "Tesseract bulunamadı: tesseract kurulu değil ya da PATH'te yok",
            ))
        }
        _ => None,
    }
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug)]
struct TesseractOutputLimits {
    max_bytes: usize,
    max_words: usize,
}

/// rusty-tesseract `Args::default()` (dpi 150, psm 3, oem 3, config yok) ile
/// aynı argümanlar; `image_to_data` gibi sonda `tsv` konfigürasyonu.
fn tesseract_tsv_command(program: &OsStr, image_path: &Path, lang: &str) -> Command {
    let mut command = Command::new(program);
    command
        .arg(image_path)
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .args(["--dpi", "150", "--psm", "3", "--oem", "3"])
        .arg("tsv")
        .stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Tesseract'ı job'un çocuk süreci olarak çalıştırır: iptal ya da zaman
/// aşımında süreç öldürülür ve reap edilir.
fn run_tesseract_tsv(
    context: &crate::document_session::SessionJobContext,
    program: &OsStr,
    image_path: &Path,
    lang: &str,
    timeout: Duration,
    limits: TesseractOutputLimits,
) -> Result<Vec<OcrWord>, SessionJobOperationError> {
    let mut command = tesseract_tsv_command(program, image_path, lang);
    let output = context
        .child_processes
        .run_with_bounded_output_timeout(
            &mut command,
            timeout,
            &context.cancellation,
            ChildOutputLimits {
                max_stdout_bytes: limits.max_bytes,
                max_stderr_bytes: TESSERACT_STDERR_LIMIT,
            },
        )
        .map_err(|error| {
            if let Some(not_found) = tesseract_not_found(&error) {
                return SessionJobOperationError::Failed(not_found);
            }
            match error {
                SessionJobOperationError::Failed(failure)
                    if failure.code == "SESSION_JOB_CHILD_OUTPUT_TOO_LARGE" =>
                {
                    output_too_large(limits.max_bytes)
                }
                other => other,
            }
        })?;
    if !output.status.success() {
        let detail = crate::child_stderr_detail(&output.stderr);
        return Err(SessionJobOperationError::failed_with_code(
            "OCR_ENGINE_FAILED",
            if detail.is_empty() {
                format!("Tesseract OCR yürütülemedi ({}).", output.status)
            } else {
                format!("Tesseract OCR yürütülemedi: {detail}")
            },
        ));
    }
    // Sınırlı okuyucu stdout'u zaten `max_bytes` ile keser; bu kontrol
    // savunma amaçlıdır.
    if output.stdout.len() > limits.max_bytes {
        return Err(output_too_large(limits.max_bytes));
    }
    parse_tesseract_tsv(&String::from_utf8_lossy(&output.stdout), limits.max_words)
        .map_err(|error| SessionJobOperationError::failed_with_code(error.code(), error.message()))
}

fn output_too_large(max_bytes: usize) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(
        "OCR_OUTPUT_TOO_LARGE",
        format!("Tesseract çıktısı {max_bytes} bayt sınırını aşıyor"),
    )
}

#[derive(Debug, PartialEq)]
enum TesseractTsvError {
    MissingHeader,
    TooManyWords(usize),
}

impl TesseractTsvError {
    fn code(&self) -> &'static str {
        match self {
            Self::MissingHeader => "OCR_OUTPUT_INVALID",
            Self::TooManyWords(_) => "OCR_OUTPUT_TOO_LARGE",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::MissingHeader => "Tesseract TSV çıktısı beklenen başlığı içermiyor".to_string(),
            Self::TooManyWords(limit) => {
                format!("Tesseract çıktısı {limit} kelime sınırını aşıyor")
            }
        }
    }
}

const TSV_COLUMNS: usize = 12;

/// Tesseract `tsv` çıktısını ayrıştırır. Sütunlar: level, page_num,
/// block_num, par_num, line_num, word_num, left, top, width, height, conf,
/// text. Sayısal alanları çözülemeyen satırlar atlanır; text ilk 11 sekmeden
/// sonraki her şeydir (sekme içerebilir). Yalnız conf ≥ 0 ve boş olmayan
/// metinli satırlar kelime olarak döner.
fn parse_tesseract_tsv(output: &str, max_words: usize) -> Result<Vec<OcrWord>, TesseractTsvError> {
    let mut lines = output.lines().map(|line| line.trim_end_matches('\r'));
    match lines.next() {
        None => return Ok(Vec::new()),
        Some(header) if header.trim().is_empty() => return Ok(Vec::new()),
        Some(header) if header.split('\t').next().map(str::trim) == Some("level") => {}
        Some(_) => return Err(TesseractTsvError::MissingHeader),
    }
    let mut words = Vec::new();
    for line in lines {
        let Some(word) = parse_tesseract_tsv_row(line) else {
            continue;
        };
        if word.confidence >= 0.0 && !word.text.is_empty() {
            if words.len() == max_words {
                return Err(TesseractTsvError::TooManyWords(max_words));
            }
            words.push(word);
        }
    }
    Ok(words)
}

fn parse_tesseract_tsv_row(line: &str) -> Option<OcrWord> {
    let fields: Vec<&str> = line.splitn(TSV_COLUMNS, '\t').collect();
    // Seviye 1-4 satırlarında text sütunu bazı sürümlerde hiç yazılmaz.
    if fields.len() < TSV_COLUMNS - 1 {
        return None;
    }
    let int = |index: usize| fields[index].trim().parse::<i32>().ok();
    for index in 0..6 {
        int(index)?;
    }
    Some(OcrWord {
        x: int(6)?,
        y: int(7)?,
        width: int(8)?,
        height: int(9)?,
        confidence: fields[10].trim().parse::<f32>().ok()?,
        text: fields.get(11).map_or("", |text| text.trim()).to_string(),
    })
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

    #[test]
    fn list_langs_parser_skips_header_and_takes_one_language_per_line() {
        let tesseract5 = "List of available languages in \"/usr/share/tessdata/\" (4):\r\n\
                          eng\r\n\
                          osd\r\n\
                          script/Latin\r\n\
                          tur\r\n";
        assert_eq!(
            parse_tesseract_langs(tesseract5),
            ["eng", "osd", "script/Latin", "tur"]
        );
        assert_eq!(
            parse_tesseract_langs("List of available languages (2):\neng\n\ntur"),
            ["eng", "tur"]
        );
    }

    #[test]
    fn list_langs_parser_keeps_first_line_when_header_is_missing() {
        assert_eq!(parse_tesseract_langs("eng\ntur\n"), ["eng", "tur"]);
        assert!(parse_tesseract_langs("").is_empty());
        assert!(parse_tesseract_langs("List of available languages (0):\n").is_empty());
    }

    #[test]
    fn list_langs_command_only_passes_list_langs() {
        let command = tesseract_list_langs_command(OsStr::new(TESSERACT_PROGRAM));
        assert_eq!(command.get_program(), OsStr::new(TESSERACT_PROGRAM));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("--list-langs")]
        );
    }

    #[test]
    fn list_langs_probe_missing_program_says_tesseract_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-tesseract");
        let message =
            probe_tesseract_languages(missing.as_os_str(), Duration::from_secs(10)).unwrap_err();
        assert_eq!(
            message,
            "Tesseract bulunamadı: tesseract kurulu değil ya da PATH'te yok"
        );
    }

    #[cfg(unix)]
    fn write_program(dir: &Path, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join("tesseract");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(mode)).unwrap();
        program
    }

    #[cfg(unix)]
    #[test]
    fn tesseract_on_path_resolves_to_bare_name_like_rusty_tesseract() {
        let on_path = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        write_program(on_path.path(), 0o755);
        write_program(fallback.path(), 0o755);
        let path_var = std::env::join_paths([Path::new("/nonexistent"), on_path.path()]).unwrap();
        assert_eq!(
            resolve_tesseract_program(
                Some(&path_var),
                &[fallback.path().to_path_buf()],
                "tesseract"
            ),
            OsString::from("tesseract")
        );
    }

    #[cfg(unix)]
    #[test]
    fn tesseract_missing_from_path_uses_first_executable_fallback() {
        let empty = tempfile::tempdir().unwrap();
        let not_executable = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        write_program(not_executable.path(), 0o644);
        let expected = write_program(fallback.path(), 0o755);
        let path_var = std::env::join_paths([empty.path()]).unwrap();
        assert_eq!(
            resolve_tesseract_program(
                Some(&path_var),
                &[
                    PathBuf::from("/nonexistent"),
                    not_executable.path().to_path_buf(),
                    fallback.path().to_path_buf(),
                ],
                "tesseract"
            ),
            expected.into_os_string()
        );
    }

    #[cfg(unix)]
    #[test]
    fn tesseract_not_found_anywhere_keeps_bare_name() {
        let empty = tempfile::tempdir().unwrap();
        let path_var = std::env::join_paths([empty.path()]).unwrap();
        assert_eq!(
            resolve_tesseract_program(Some(&path_var), &[empty.path().to_path_buf()], "tesseract"),
            OsString::from("tesseract")
        );
        assert_eq!(
            resolve_tesseract_program(None, &[], "tesseract"),
            OsString::from("tesseract")
        );
    }

    #[test]
    fn tesseract_resolution_includes_gui_launch_install_dirs() {
        let dirs = tesseract_fallback_dirs();
        #[cfg(target_os = "macos")]
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        #[cfg(target_os = "macos")]
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        assert!(dirs.iter().all(|dir| dir.is_absolute()));
    }

    const TSV_HEADER: &str =
        "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext";

    fn word(text: &str, x: i32, y: i32, width: i32, height: i32, confidence: f32) -> OcrWord {
        OcrWord {
            text: text.to_string(),
            x,
            y,
            width,
            height,
            confidence,
        }
    }

    #[test]
    fn tsv_parser_keeps_confident_non_empty_words() {
        let output = format!(
            "{TSV_HEADER}\n\
             1\t1\t0\t0\t0\t0\t0\t0\t696\t89\t-1\t\n\
             4\t1\t1\t1\t1\t0\t18\t29\t653\t35\t-1\n\
             5\t1\t1\t1\t1\t1\t18\t29\t144\t35\t95.5\tLOREM\n\
             5\t1\t1\t1\t1\t2\t181\t29\t123\t35\t0\tİPSUM\r\n\
             5\t1\t1\t1\t1\t3\t323\t29\t153\t35\t-1\tlow\n\
             5\t1\t1\t1\t1\t4\t490\t29\t50\t35\t91\t   \n"
        );
        assert_eq!(
            parse_tesseract_tsv(&output, 100),
            Ok(vec![
                word("LOREM", 18, 29, 144, 35, 95.5),
                word("İPSUM", 181, 29, 123, 35, 0.0),
            ])
        );
    }

    #[test]
    fn tsv_parser_accepts_empty_and_header_only_output() {
        assert_eq!(parse_tesseract_tsv("", 100), Ok(vec![]));
        assert_eq!(parse_tesseract_tsv("\n", 100), Ok(vec![]));
        assert_eq!(
            parse_tesseract_tsv(&format!("{TSV_HEADER}\n"), 100),
            Ok(vec![])
        );
    }

    #[test]
    fn tsv_parser_skips_malformed_rows() {
        let output = format!(
            "{TSV_HEADER}\n\
             garbage\n\
             5\t1\t1\t1\t1\t1\t18\t29\n\
             5\t1\t1\t1\t1\tx\t18\t29\t144\t35\t90\tbadint\n\
             5\t1\t1\t1\t1\t1\t18\t29\t144\t35\tnotconf\tbadconf\n\
             5\t1\t1\t1\t1\t1\t1\t2\t3\t4\t50\tok\n"
        );
        assert_eq!(
            parse_tesseract_tsv(&output, 100),
            Ok(vec![word("ok", 1, 2, 3, 4, 50.0)])
        );
    }

    #[test]
    fn tsv_parser_keeps_tabs_inside_text_column() {
        let output = format!("{TSV_HEADER}\n5\t1\t1\t1\t1\t1\t1\t2\t3\t4\t80\ta\tb\tc\t\n");
        assert_eq!(
            parse_tesseract_tsv(&output, 100),
            Ok(vec![word("a\tb\tc", 1, 2, 3, 4, 80.0)])
        );
    }

    #[test]
    fn tsv_parser_rejects_output_without_header() {
        assert_eq!(
            parse_tesseract_tsv("5\t1\t1\t1\t1\t1\t1\t2\t3\t4\t80\tword\n", 100),
            Err(TesseractTsvError::MissingHeader)
        );
    }

    #[test]
    fn tsv_parser_enforces_word_limit() {
        let row = "5\t1\t1\t1\t1\t1\t1\t2\t3\t4\t80\tw\n";
        let output = format!("{TSV_HEADER}\n{}", row.repeat(3));
        assert_eq!(
            parse_tesseract_tsv(&output, 3).map(|words| words.len()),
            Ok(3)
        );
        assert_eq!(
            parse_tesseract_tsv(&output, 2),
            Err(TesseractTsvError::TooManyWords(2))
        );
    }

    #[test]
    fn ocr_lang_accepts_simple_and_combined_codes() {
        for lang in ["eng", "tur", "eng+tur", "chi_sim", "osd", "deu+frk+eng"] {
            assert!(is_valid_ocr_lang(lang, None), "{lang}");
        }
    }

    #[test]
    fn ocr_lang_rejects_options_paths_and_separators() {
        let too_long = "a".repeat(MAX_OCR_LANG_LEN + 1);
        for lang in [
            "",
            "+",
            "eng+",
            "+eng",
            "eng++tur",
            "-l",
            "--tessdata-dir",
            "eng tur",
            "eng\ttur",
            "../eng",
            "/tmp/eng",
            "script/Latin",
            "eng;rm",
            "eng\0",
            "türkçe",
            too_long.as_str(),
        ] {
            assert!(!is_valid_ocr_lang(lang, None), "{lang:?}");
        }
    }

    #[test]
    fn ocr_lang_accepts_installed_names_outside_pattern_only_from_cache() {
        let installed = vec!["eng".to_string(), "script/Latin".to_string()];
        assert!(is_valid_ocr_lang("script/Latin", Some(&installed)));
        assert!(is_valid_ocr_lang("eng+script/Latin", Some(&installed)));
        assert!(!is_valid_ocr_lang("script/Greek", Some(&installed)));
        assert!(!is_valid_ocr_lang("../script/Latin", Some(&installed)));
    }

    #[test]
    fn tesseract_command_matches_rusty_tesseract_image_to_data_arguments() {
        let command = tesseract_tsv_command(
            OsStr::new(TESSERACT_PROGRAM),
            Path::new("/job/ocr-input.png"),
            "eng+tur",
        );
        assert_eq!(command.get_program(), OsStr::new(TESSERACT_PROGRAM));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "/job/ocr-input.png",
                "stdout",
                "-l",
                "eng+tur",
                "--dpi",
                "150",
                "--psm",
                "3",
                "--oem",
                "3",
                "tsv"
            ]
            .map(OsStr::new)
        );
    }

    #[cfg(unix)]
    mod fake_tesseract {
        use super::super::*;
        use crate::document_session::SessionJobContext;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        const LIMITS: TesseractOutputLimits = TESSERACT_OUTPUT_LIMITS;

        struct Fixture {
            _dir: tempfile::TempDir,
            program: PathBuf,
            image: PathBuf,
            context: SessionJobContext,
        }

        impl Fixture {
            fn new(script_body: &str) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let program = dir.path().join("fake-tesseract");
                std::fs::write(&program, format!("#!/bin/sh\n{script_body}\n")).unwrap();
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
                let image = dir.path().join("ocr-input.png");
                std::fs::write(&image, b"png").unwrap();
                let context =
                    SessionJobContext::detached(dir.path().join("snapshot.pdf"), None, "ocr-test");
                Self {
                    _dir: dir,
                    program,
                    image,
                    context,
                }
            }

            fn run(
                &self,
                timeout: Duration,
                limits: TesseractOutputLimits,
            ) -> Result<Vec<OcrWord>, SessionJobOperationError> {
                run_tesseract_tsv(
                    &self.context,
                    self.program.as_os_str(),
                    &self.image,
                    "eng+tur",
                    timeout,
                    limits,
                )
            }
        }

        fn failure_code(error: SessionJobOperationError) -> (String, String) {
            match error {
                SessionJobOperationError::Failed(failure) => (failure.code, failure.message),
                SessionJobOperationError::Cancelled(reason) => {
                    panic!("unexpected cancellation: {reason:?}")
                }
            }
        }

        fn wait_for_pid(pid_file: &Path) -> String {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(pid) = std::fs::read_to_string(pid_file) {
                    if pid.ends_with('\n') {
                        return pid.trim().to_string();
                    }
                }
                assert!(Instant::now() < deadline, "fake tesseract did not start");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn process_alive(pid: &str) -> bool {
            Command::new("kill")
                .args(["-0", pid])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        }

        fn sleeping_script(pid_file: &Path) -> String {
            format!("echo $$ > '{}'\nexec sleep 60", pid_file.display())
        }

        #[test]
        fn runs_program_on_job_image_and_parses_stdout() {
            let probe = tempfile::tempdir().unwrap();
            let args_file = probe.path().join("args");
            let fixture = Fixture::new(&format!(
                "printf '%s\\n' \"$@\" > '{}'\nprintf '{}\\n5\\t1\\t1\\t1\\t1\\t1\\t10\\t20\\t30\\t40\\t88.5\\tMerhaba\\n'",
                args_file.display(),
                super::TSV_HEADER.replace('\t', "\\t"),
            ));
            let words = fixture.run(Duration::from_secs(10), LIMITS).unwrap();
            assert_eq!(words, vec![super::word("Merhaba", 10, 20, 30, 40, 88.5)]);
            let args = std::fs::read_to_string(&args_file).unwrap();
            assert_eq!(
                args.lines().collect::<Vec<_>>(),
                [
                    fixture.image.to_str().unwrap(),
                    "stdout",
                    "-l",
                    "eng+tur",
                    "--dpi",
                    "150",
                    "--psm",
                    "3",
                    "--oem",
                    "3",
                    "tsv"
                ]
            );
            assert_eq!(fixture.context.child_processes.active_child_count(), 0);
        }

        #[test]
        fn missing_program_reports_tesseract_not_found() {
            let fixture = Fixture::new("exit 0");
            let missing = fixture.image.with_file_name("no-such-tesseract");
            let error = run_tesseract_tsv(
                &fixture.context,
                missing.as_os_str(),
                &fixture.image,
                "eng",
                Duration::from_secs(10),
                LIMITS,
            )
            .unwrap_err();
            let (code, message) = failure_code(error);
            assert_eq!(code, "OCR_ENGINE_NOT_FOUND");
            assert!(message.starts_with("Tesseract bulunamadı"), "{message}");
        }

        #[test]
        fn endless_stdout_is_cut_off_before_the_timeout() {
            let fixture = Fixture::new("exec yes");
            let limits = TesseractOutputLimits {
                max_bytes: 4096,
                max_words: 10,
            };
            let started = Instant::now();
            let (code, _) = failure_code(fixture.run(Duration::from_secs(30), limits).unwrap_err());
            assert_eq!(code, "OCR_OUTPUT_TOO_LARGE");
            assert!(started.elapsed() < Duration::from_secs(10));
            assert_eq!(fixture.context.child_processes.active_child_count(), 0);
        }

        #[test]
        fn non_zero_exit_reports_stderr() {
            let fixture = Fixture::new("echo 'Failed loading language xyz' >&2\nexit 1");
            let (code, message) =
                failure_code(fixture.run(Duration::from_secs(10), LIMITS).unwrap_err());
            assert_eq!(code, "OCR_ENGINE_FAILED");
            assert!(message.contains("Failed loading language xyz"), "{message}");
        }

        #[test]
        fn oversized_stdout_is_rejected() {
            let fixture = Fixture::new(&format!(
                "printf '{}\\n'",
                super::TSV_HEADER.replace('\t', "\\t")
            ));
            let limits = TesseractOutputLimits {
                max_bytes: 8,
                max_words: 10,
            };
            let (code, _) = failure_code(fixture.run(Duration::from_secs(10), limits).unwrap_err());
            assert_eq!(code, "OCR_OUTPUT_TOO_LARGE");
        }

        #[test]
        fn timeout_kills_the_tesseract_process() {
            let probe = tempfile::tempdir().unwrap();
            let pid_file = probe.path().join("pid");
            let fixture = Fixture::new(&sleeping_script(&pid_file));
            let started = Instant::now();
            // Zaman aşımı, sahte süreç PID'ini yazmadan dolmasın diye geniş
            // tutulur; PID, süreç hâlâ çalışırken ayrı thread'den okunur.
            let (result, pid) = std::thread::scope(|scope| {
                let running = scope.spawn(|| fixture.run(Duration::from_secs(3), LIMITS));
                let pid = wait_for_pid(&pid_file);
                (running.join().unwrap(), pid)
            });
            let (_, message) = failure_code(result.unwrap_err());
            assert!(message.contains("zaman aşımı"), "{message}");
            assert!(started.elapsed() < Duration::from_secs(30));
            assert!(
                !process_alive(&pid),
                "tesseract {pid} still running after timeout"
            );
            assert_eq!(fixture.context.child_processes.active_child_count(), 0);
        }

        #[test]
        fn cancellation_kills_the_tesseract_process() {
            let probe = tempfile::tempdir().unwrap();
            let pid_file = probe.path().join("pid");
            let fixture = Fixture::new(&sleeping_script(&pid_file));
            let token = fixture.context.cancellation.token().clone();
            let started = Instant::now();
            let (result, pid) = std::thread::scope(|scope| {
                let running = scope.spawn(|| fixture.run(Duration::from_secs(60), LIMITS));
                let pid = wait_for_pid(&pid_file);
                assert!(process_alive(&pid));
                token.cancel();
                (running.join().unwrap(), pid)
            });
            assert!(
                matches!(result, Err(SessionJobOperationError::Cancelled(_))),
                "{result:?}"
            );
            assert!(started.elapsed() < Duration::from_secs(30));
            assert!(
                !process_alive(&pid),
                "tesseract {pid} still running after cancel"
            );
            assert_eq!(fixture.context.child_processes.active_child_count(), 0);
        }

        #[test]
        fn list_langs_probe_runs_program_and_parses_stdout() {
            let probe = tempfile::tempdir().unwrap();
            let args_file = probe.path().join("args");
            let fixture = Fixture::new(&format!(
                "printf '%s\\n' \"$@\" > '{}'\n\
                 printf 'List of available languages (2):\\neng\\ntur\\n'",
                args_file.display(),
            ));
            let langs =
                probe_tesseract_languages(fixture.program.as_os_str(), Duration::from_secs(10));
            assert_eq!(langs, Ok(vec!["eng".to_string(), "tur".to_string()]));
            assert_eq!(
                std::fs::read_to_string(&args_file).unwrap(),
                "--list-langs\n"
            );
        }

        #[test]
        fn list_langs_probe_non_zero_exit_reports_stderr() {
            let fixture = Fixture::new("echo 'tessdata missing' >&2\nexit 1");
            let message =
                probe_tesseract_languages(fixture.program.as_os_str(), Duration::from_secs(10))
                    .unwrap_err();
            assert!(message.starts_with("Tesseract bulunamadı"), "{message}");
            assert!(message.contains("tessdata missing"), "{message}");
        }

        #[test]
        fn list_langs_probe_timeout_kills_the_tesseract_process() {
            let probe = tempfile::tempdir().unwrap();
            let pid_file = probe.path().join("pid");
            let fixture = Fixture::new(&sleeping_script(&pid_file));
            let started = Instant::now();
            let (result, pid) = std::thread::scope(|scope| {
                let running = scope.spawn(|| {
                    probe_tesseract_languages(fixture.program.as_os_str(), Duration::from_secs(3))
                });
                let pid = wait_for_pid(&pid_file);
                (running.join().unwrap(), pid)
            });
            let message = result.unwrap_err();
            assert!(message.starts_with("Tesseract bulunamadı"), "{message}");
            assert!(message.contains("zaman aşımı"), "{message}");
            assert!(started.elapsed() < Duration::from_secs(30));
            assert!(
                !process_alive(&pid),
                "tesseract {pid} still running after language probe timeout"
            );
        }
    }
}
