use crate::document_session::{
    ChildOutputLimits, DocumentSessionError, DocumentSessionStore, SessionJobFailure,
    SessionJobOperationError, SessionJobOutcome,
};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
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
    let stdout = run_tesseract_tsv_output(context, program, image_path, lang, timeout, limits)?;
    parse_tesseract_tsv(&stdout, limits.max_words).map_err(TesseractTsvError::into_job_error)
}

/// `run_tesseract_tsv`'nin ayrıştırılmamış stdout'u; çıkış kodu sıfır değilse hata.
fn run_tesseract_tsv_output(
    context: &crate::document_session::SessionJobContext,
    program: &OsStr,
    image_path: &Path,
    lang: &str,
    timeout: Duration,
    limits: TesseractOutputLimits,
) -> Result<String, SessionJobOperationError> {
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
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn output_too_large(max_bytes: usize) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(
        "OCR_OUTPUT_TOO_LARGE",
        format!("Tesseract çıktısı {max_bytes} bayt sınırını aşıyor"),
    )
}

#[derive(Debug, PartialEq)]
pub(crate) enum TesseractTsvError {
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

    fn into_job_error(self) -> SessionJobOperationError {
        SessionJobOperationError::failed_with_code(self.code(), self.message())
    }
}

const TSV_COLUMNS: usize = 12;

/// Tesseract `tsv` çıktısını ayrıştırır. Sütunlar: level, page_num,
/// block_num, par_num, line_num, word_num, left, top, width, height, conf,
/// text. Sayısal alanları çözülemeyen satırlar atlanır; text ilk 11 sekmeden
/// sonraki her şeydir (sekme içerebilir). Yalnız conf ≥ 0 ve boş olmayan
/// metinli satırlar kelime olarak döner.
fn parse_tesseract_tsv(output: &str, max_words: usize) -> Result<Vec<OcrWord>, TesseractTsvError> {
    parse_tesseract_layout(output, max_words).map(|layout| layout.words)
}

/// Ayrıştırılmış TSV: kelimeler okuma sırasıyla ve (varsa) seviye 1 satırının
/// verdiği görüntü boyutu.
#[derive(Debug, PartialEq)]
pub(crate) struct TsvLayout {
    pub(crate) image_size: Option<(i32, i32)>,
    pub(crate) words: Vec<OcrWord>,
}

pub(crate) fn parse_tesseract_layout(
    output: &str,
    max_words: usize,
) -> Result<TsvLayout, TesseractTsvError> {
    let mut layout = TsvLayout {
        image_size: None,
        words: Vec::new(),
    };
    let mut lines = output.lines().map(|line| line.trim_end_matches('\r'));
    match lines.next() {
        None => return Ok(layout),
        Some(header) if header.trim().is_empty() => return Ok(layout),
        Some(header) if header.split('\t').next().map(str::trim) == Some("level") => {}
        Some(_) => return Err(TesseractTsvError::MissingHeader),
    }
    for line in lines {
        let Some((level, word)) = parse_tesseract_tsv_row(line) else {
            continue;
        };
        if level == 1 && layout.image_size.is_none() {
            layout.image_size = Some((word.width, word.height));
        }
        if word.confidence >= 0.0 && !word.text.is_empty() {
            if layout.words.len() == max_words {
                return Err(TesseractTsvError::TooManyWords(max_words));
            }
            layout.words.push(word);
        }
    }
    Ok(layout)
}

fn parse_tesseract_tsv_row(line: &str) -> Option<(i32, OcrWord)> {
    let fields: Vec<&str> = line.splitn(TSV_COLUMNS, '\t').collect();
    // Seviye 1-4 satırlarında text sütunu bazı sürümlerde hiç yazılmaz.
    if fields.len() < TSV_COLUMNS - 1 {
        return None;
    }
    let int = |index: usize| fields[index].trim().parse::<i32>().ok();
    for index in 0..6 {
        int(index)?;
    }
    Some((
        int(0)?,
        OcrWord {
            x: int(6)?,
            y: int(7)?,
            width: int(8)?,
            height: int(9)?,
            confidence: fields[10].trim().parse::<f32>().ok()?,
            text: fields.get(11).map_or("", |text| text.trim()).to_string(),
        },
    ))
}

/// Ana sürecin job dizinine yazdığı TSV dosyasını sınırlı okur ve metin katmanı
/// kelimelerine çevirir (job worker'da).
pub(crate) fn read_tsv_layout_words(
    path: &Path,
) -> Result<Vec<crate::searchable_pdf::LayoutWord>, (&'static str, String)> {
    let size = std::fs::metadata(path)
        .map_err(|error| {
            (
                "SESSION_JOB_OUTPUT_READ_FAILED",
                format!("OCR çıktısı okunamadı: {error}"),
            )
        })?
        .len();
    if size > TESSERACT_OUTPUT_LIMITS.max_bytes as u64 {
        return Err((
            "OCR_OUTPUT_TOO_LARGE",
            format!(
                "Tesseract çıktısı {} bayt sınırını aşıyor",
                TESSERACT_OUTPUT_LIMITS.max_bytes
            ),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        (
            "SESSION_JOB_OUTPUT_READ_FAILED",
            format!("OCR çıktısı okunamadı: {error}"),
        )
    })?;
    let layout = parse_tesseract_layout(
        &String::from_utf8_lossy(&bytes),
        TESSERACT_OUTPUT_LIMITS.max_words,
    )
    .map_err(|error| (error.code(), error.message()))?;
    Ok(layout
        .words
        .into_iter()
        .map(|word| crate::searchable_pdf::LayoutWord {
            text: word.text,
            left: word.x,
            top: word.y,
            width: word.width,
            height: word.height,
        })
        .collect())
}

/// Aranabilir kopyada sayfa başına ve toplam sınırlar.
#[derive(Clone, Copy, Debug)]
struct SearchableJobLimits {
    total: Duration,
    per_page: Duration,
    output: TesseractOutputLimits,
}

const SEARCHABLE_JOB_LIMITS: SearchableJobLimits = SearchableJobLimits {
    total: Duration::from_secs(30 * 60),
    per_page: TESSERACT_RUN_TIMEOUT,
    output: TESSERACT_OUTPUT_LIMITS,
};
const SEARCHABLE_PLAN_TIMEOUT: Duration = Duration::from_secs(120);
const SEARCHABLE_WRITE_TIMEOUT: Duration = Duration::from_secs(600);
const SEARCHABLE_OUTPUT_NAME: &str = "searchable-copy.pdf";

/// Bir sayfada bu kodlarla biten OCR, sayfayı atlatır; diğer hatalar bütün işi düşürür.
const SKIPPABLE_PAGE_FAILURES: &[&str] = &[
    "OCR_ENGINE_FAILED",
    "OCR_OUTPUT_INVALID",
    "OCR_OUTPUT_TOO_LARGE",
    "SESSION_JOB_RENDER_FAILED",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedSearchablePage {
    pub page_index: u32,
    pub reason: crate::searchable_pdf::SkipReason,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchablePdfReport {
    pub processed_pages: Vec<u32>,
    pub skipped_pages: Vec<SkippedSearchablePage>,
    pub words_written: u64,
    pub words_skipped: u64,
    pub signatures_invalidated: bool,
}

/// Seçilen taranmış sayfaları Tesseract ile tanır, kelimeleri görünmez metin olarak
/// yerleştirir ve kopyayı `output_path`'e atomik olarak yazar. Oturumdaki belge değişmez.
/// `pages` yoksa bütün sayfalar değerlendirilir; metni olan sayfalar her durumda atlanır.
#[tauri::command]
pub async fn export_as_searchable_pdf(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    output_path: String,
    lang: String,
    pages: Option<Vec<u32>>,
    store: tauri::State<'_, DocumentSessionStore>,
) -> Result<SessionJobOutcome<SearchablePdfReport>, DocumentSessionError> {
    let target = PathBuf::from(output_path);
    crate::document_session::reject_private_target(&target, &store.temp_root)?;
    let source = store.session_snapshot(&session_id).await?.display_path;
    reject_source_target(Path::new(&source), &target)?;
    let installed = OCR_LANGUAGE_CACHE
        .get_or_probe(
            || probe_tesseract_languages(&tesseract_program(), OCR_LANGUAGE_PROBE_TIMEOUT),
            OCR_LANGUAGE_PROBE_TIMEOUT + OCR_LANGUAGE_PROBE_GRACE,
            OCR_LANGUAGE_FAILURE_RETRY_AFTER,
        )
        .await
        .map_err(|message| {
            let code = if message.starts_with("Tesseract bulunamadı") {
                "OCR_ENGINE_NOT_FOUND"
            } else {
                "OCR_ENGINE_FAILED"
            };
            DocumentSessionError::new(code, message)
        })?;
    check_installed_ocr_language(&lang, &installed)?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                searchable_export_job(
                    &context,
                    &tesseract_program(),
                    &lang,
                    pages,
                    &target,
                    SEARCHABLE_JOB_LIMITS,
                )
            },
        )
        .await
}

/// Hedef, oturumun kendi dosyası olamaz: kopya oturumun altındaki dosyanın yerine geçerdi.
fn reject_source_target(source: &Path, target: &Path) -> Result<(), DocumentSessionError> {
    match crate::document_io::same_persistence_target(source, target) {
        Ok(false) => Ok(()),
        Ok(true) => Err(DocumentSessionError::new(
            "SEARCHABLE_PDF_TARGET_IS_SOURCE",
            "Aranabilir kopya açık belgenin üzerine kaydedilemez; başka bir dosya adı seçin",
        )),
        Err(error) => Err(DocumentSessionError::new(
            "SEARCHABLE_PDF_TARGET_INVALID",
            format!("Hedef dosya doğrulanamadı: {error}"),
        )),
    }
}

/// `lang` güvenli olmalı ve her bileşeni kurulu olmalı.
fn check_installed_ocr_language(
    lang: &str,
    installed: &[String],
) -> Result<(), DocumentSessionError> {
    if !is_valid_ocr_lang(lang, Some(installed)) {
        return Err(DocumentSessionError::new(
            "OCR_LANGUAGE_INVALID",
            "OCR dili geçersiz",
        ));
    }
    let missing: Vec<&str> = lang
        .split('+')
        .filter(|component| !installed.iter().any(|known| known == component))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(DocumentSessionError::new(
            "OCR_LANGUAGE_MISSING",
            format!(
                "Tesseract dil verisi kurulu değil: {}. Dil paketini kurduysanız uygulamayı yeniden başlatın.",
                missing.join(", ")
            ),
        ))
    }
}

/// Sayfa OCR'ının sonucu: başarı, sayfayı atlatan hata ya da işi düşüren hata.
enum PageOcrError {
    Skip(SessionJobFailure),
    Fatal(SessionJobOperationError),
}

impl From<SessionJobOperationError> for PageOcrError {
    fn from(error: SessionJobOperationError) -> Self {
        match error {
            SessionJobOperationError::Failed(failure)
                if SKIPPABLE_PAGE_FAILURES.contains(&failure.code.as_str()) =>
            {
                Self::Skip(failure)
            }
            other => Self::Fatal(other),
        }
    }
}

fn protocol_error(message: &str) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code("SESSION_JOB_WORKER_PROTOCOL_ERROR", message)
}

/// Aranabilir kopya job'unun gövdesi: planla, aday sayfaları sırayla render et ve tanı,
/// worker'da yaz ve doğrula, hedefe yayımla. Herhangi bir hata, iptal ya da zaman aşımında
/// hedefe hiçbir şey yazılmaz.
fn searchable_export_job(
    context: &crate::document_session::SessionJobContext,
    program: &OsStr,
    lang: &str,
    pages: Option<Vec<u32>>,
    target: &Path,
    limits: SearchableJobLimits,
) -> Result<SearchablePdfReport, SessionJobOperationError> {
    use crate::job_worker::{run_job, JobCommand, JobResult};
    use crate::searchable_pdf::SkipReason;

    let started = Instant::now();
    context.cancellation.check_cancelled()?;
    let plan = match run_job(
        context,
        JobCommand::PlanSearchableCopy { pages },
        SEARCHABLE_PLAN_TIMEOUT,
    )? {
        JobResult::SearchablePlan(plan) => plan,
        _ => {
            return Err(protocol_error(
                "Aranabilir kopya planı beklenmeyen sonuç döndürdü",
            ))
        }
    };
    let mut skipped: Vec<SkippedSearchablePage> = plan
        .pages
        .iter()
        .filter_map(|page| {
            page.skip.map(|reason| SkippedSearchablePage {
                page_index: page.page_index,
                reason,
                detail: None,
            })
        })
        .collect();
    let candidates: Vec<u32> = plan
        .pages
        .iter()
        .filter(|page| page.skip.is_none())
        .map(|page| page.page_index)
        .collect();
    if candidates.is_empty() {
        return Err(SessionJobOperationError::failed_with_code(
            "SEARCHABLE_PDF_NO_SCANNED_PAGES",
            "Seçilen sayfalarda metin katmanı eklenecek taranmış sayfa yok",
        ));
    }
    if candidates.len() > crate::searchable_pdf::MAX_SEARCHABLE_PAGES {
        return Err(SessionJobOperationError::failed_with_code(
            "SEARCHABLE_PDF_TOO_MANY_PAGES",
            format!(
                "Tek seferde en fazla {} taranmış sayfa işlenebilir; daha küçük bir sayfa aralığı seçin",
                crate::searchable_pdf::MAX_SEARCHABLE_PAGES
            ),
        ));
    }
    let job_dir = context
        .snapshot_path
        .parent()
        .ok_or_else(|| protocol_error("Job snapshot dizini çözümlenemedi"))?
        .to_path_buf();

    let mut inputs = Vec::with_capacity(candidates.len());
    let mut total_words = 0usize;
    let mut first_failure: Option<SessionJobFailure> = None;
    for page_index in candidates {
        context.cancellation.check_cancelled()?;
        let remaining = limits
            .total
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                SessionJobOperationError::failed_with_code(
                    "SEARCHABLE_PDF_TIMEOUT",
                    format!(
                        "Aranabilir kopya {} dk içinde tamamlanmadı; daha küçük bir sayfa aralığı seçin",
                        limits.total.as_secs() / 60
                    ),
                )
            })?;
        match ocr_searchable_page(
            context,
            program,
            lang,
            page_index,
            remaining.min(limits.per_page),
            limits.output,
            &job_dir,
        ) {
            Ok((input, 0)) => {
                let _ = std::fs::remove_file(job_dir.join(&input.tsv_name));
                skipped.push(SkippedSearchablePage {
                    page_index,
                    reason: SkipReason::NoWords,
                    detail: None,
                });
            }
            Ok((input, words)) => {
                total_words += words;
                if total_words > crate::searchable_pdf::MAX_SEARCHABLE_WORDS {
                    return Err(SessionJobOperationError::failed_with_code(
                        "OCR_OUTPUT_TOO_LARGE",
                        format!(
                            "OCR çıktısı toplam {} kelime sınırını aşıyor; daha küçük bir sayfa aralığı seçin",
                            crate::searchable_pdf::MAX_SEARCHABLE_WORDS
                        ),
                    ));
                }
                inputs.push(input);
            }
            Err(PageOcrError::Skip(failure)) => {
                skipped.push(SkippedSearchablePage {
                    page_index,
                    reason: SkipReason::OcrFailed,
                    detail: Some(failure.message.clone()),
                });
                first_failure.get_or_insert(failure);
            }
            Err(PageOcrError::Fatal(error)) => return Err(error),
        }
    }
    if inputs.is_empty() {
        return Err(match first_failure {
            Some(failure) => SessionJobOperationError::failed_with_code(
                "SEARCHABLE_PDF_OCR_FAILED",
                format!("Hiçbir sayfa tanınamadı: {}", failure.message),
            ),
            None => SessionJobOperationError::failed_with_code(
                "SEARCHABLE_PDF_NO_TEXT_FOUND",
                "Taranmış sayfalarda tanınan metin yok",
            ),
        });
    }

    context.cancellation.check_cancelled()?;
    let processed_pages: Vec<u32> = inputs.iter().map(|input| input.page_index).collect();
    let (path, report) = match run_job(
        context,
        JobCommand::SearchableCopy {
            pages: inputs,
            output_name: SEARCHABLE_OUTPUT_NAME.to_string(),
        },
        SEARCHABLE_WRITE_TIMEOUT,
    )? {
        JobResult::SearchableCopy { path, report } => (path, report),
        _ => {
            return Err(protocol_error(
                "Aranabilir kopya worker'ı beklenmeyen sonuç döndürdü",
            ))
        }
    };
    crate::document_session::accept_job_output_file(context, &path, target)?;
    skipped.sort_by_key(|page| page.page_index);
    Ok(SearchablePdfReport {
        processed_pages,
        skipped_pages: skipped,
        words_written: report.words_written,
        words_skipped: report.words_skipped,
        signatures_invalidated: report.signatures_invalidated,
    })
}

/// Tek sayfa: worker'da render, Tesseract, TSV job dizinine. PNG her durumda silinir.
/// Başarıda TSV'yi tanımlayan girdi ve kelime sayısı döner.
fn ocr_searchable_page(
    context: &crate::document_session::SessionJobContext,
    program: &OsStr,
    lang: &str,
    page_index: u32,
    timeout: Duration,
    limits: TesseractOutputLimits,
    job_dir: &Path,
) -> Result<(crate::job_worker::SearchablePageInput, usize), PageOcrError> {
    use crate::job_worker::{run_job, JobCommand, JobResult};

    let image = match run_job(
        context,
        JobCommand::RenderOcrPage {
            page_index,
            output_name: format!("searchable-ocr-{page_index}.png"),
            scale: crate::searchable_pdf::OCR_RENDER_SCALE,
        },
        OCR_RENDER_TIMEOUT,
    )? {
        JobResult::File(path) => {
            crate::job_worker::validate_output_file(context, &path).map_err(PageOcrError::Fatal)?
        }
        _ => {
            return Err(PageOcrError::Fatal(protocol_error(
                "OCR preparation worker beklenmeyen sonuç döndürdü",
            )))
        }
    };
    let dimensions = png_dimensions(&image);
    let stdout = match dimensions {
        Ok(_) => run_tesseract_tsv_output(context, program, &image, lang, timeout, limits)
            .map_err(PageOcrError::from),
        Err(_) => Ok(String::new()),
    };
    let _ = std::fs::remove_file(&image);
    let (bitmap_width, bitmap_height) = dimensions?;
    let stdout = stdout?;
    let layout = parse_tesseract_layout(&stdout, limits.max_words)
        .map_err(|error| PageOcrError::from(error.into_job_error()))?;
    if let Some(size) = layout.image_size {
        if size != (bitmap_width as i32, bitmap_height as i32) {
            return Err(PageOcrError::Skip(SessionJobFailure::new(
                "OCR_OUTPUT_INVALID",
                "Tesseract çıktısının görüntü boyutu render edilen sayfayla uyuşmuyor",
            )));
        }
    }
    let tsv_name = format!("searchable-ocr-{page_index}.tsv");
    std::fs::write(job_dir.join(&tsv_name), stdout.as_bytes()).map_err(|error| {
        PageOcrError::Fatal(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_WRITE_FAILED",
            format!("OCR çıktısı job dizinine yazılamadı: {error}"),
        ))
    })?;
    Ok((
        crate::job_worker::SearchablePageInput {
            page_index,
            bitmap_width,
            bitmap_height,
            tsv_name,
        },
        layout.words.len(),
    ))
}

/// Worker'ın yazdığı PNG'nin boyutu, IHDR başlığından (görüntü decode edilmez).
fn png_dimensions(path: &Path) -> Result<(u32, u32), PageOcrError> {
    use std::io::Read;
    let invalid = || {
        PageOcrError::Fatal(SessionJobOperationError::failed_with_code(
            "SESSION_JOB_OUTPUT_READ_FAILED",
            "OCR görüntüsünün başlığı okunamadı",
        ))
    };
    let mut header = [0u8; 24];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|_| invalid())?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return Err(invalid());
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    if width == 0 || height == 0 {
        return Err(invalid());
    }
    Ok((width, height))
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

    #[test]
    fn layout_parser_reads_the_image_size_from_the_page_row() {
        let tsv = format!(
            "{TSV_HEADER}\n1\t1\t0\t0\t0\t0\t0\t0\t1190\t1684\t-1\t\n5\t1\t1\t1\t1\t1\t10\t20\t30\t40\t91\tçağrı\n"
        );
        let layout = parse_tesseract_layout(&tsv, 10).unwrap();
        assert_eq!(layout.image_size, Some((1190, 1684)));
        assert_eq!(layout.words, vec![word("çağrı", 10, 20, 30, 40, 91.0)]);
        let no_page_row = format!("{TSV_HEADER}\n5\t1\t1\t1\t1\t1\t10\t20\t30\t40\t91\tx\n");
        assert_eq!(
            parse_tesseract_layout(&no_page_row, 10).unwrap().image_size,
            None
        );
    }

    #[test]
    fn searchable_target_must_not_be_the_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("belge.pdf");
        std::fs::write(&source, b"%PDF-1.4").unwrap();
        let code =
            |target: &Path| reject_source_target(&source, target).map_err(|error| error.code);
        assert_eq!(
            code(&source),
            Err("SEARCHABLE_PDF_TARGET_IS_SOURCE".to_string())
        );
        assert_eq!(code(&dir.path().join("belge-aranabilir.pdf")), Ok(()));
        assert_eq!(
            code(&dir.path().join(".").join("belge.pdf")),
            Err("SEARCHABLE_PDF_TARGET_IS_SOURCE".to_string())
        );
        #[cfg(unix)]
        {
            let link = dir.path().join("link.pdf");
            std::os::unix::fs::symlink(&source, &link).unwrap();
            // Atomik yazım symlink hedefleri kabul etmez; oturum dosyası da böylece korunur.
            assert_eq!(
                code(&link),
                Err("SEARCHABLE_PDF_TARGET_INVALID".to_string())
            );
        }
    }

    #[test]
    fn searchable_language_must_be_valid_and_installed() {
        let installed = vec![
            "eng".to_string(),
            "tur".to_string(),
            "script/Latin".to_string(),
        ];
        let code =
            |lang: &str| check_installed_ocr_language(lang, &installed).map_err(|error| error.code);
        assert_eq!(code("tur"), Ok(()));
        assert_eq!(code("tur+eng"), Ok(()));
        assert_eq!(code("script/Latin"), Ok(()));
        assert_eq!(code("deu"), Err("OCR_LANGUAGE_MISSING".to_string()));
        assert_eq!(code("tur+deu"), Err("OCR_LANGUAGE_MISSING".to_string()));
        assert_eq!(code("-c x"), Err("OCR_LANGUAGE_INVALID".to_string()));
        assert_eq!(code("../tur"), Err("OCR_LANGUAGE_INVALID".to_string()));
    }

    #[test]
    fn png_dimensions_reads_the_ihdr_header_only() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("page.png");
        image::GrayImage::new(37, 21).save(&png).unwrap();
        assert_eq!(png_dimensions(&png).ok(), Some((37, 21)));
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, b"not a png at all, not at all").unwrap();
        assert!(png_dimensions(&bad).is_err());
    }

    /// Gerçek Tesseract ile uçtan uca: `cargo test --lib real_tesseract -- --ignored`.
    /// Tesseract ve `tur` dil verisi kurulu olmalı.
    #[test]
    #[ignore = "requires an installed Tesseract with the tur language"]
    fn real_tesseract_makes_the_scanned_fixture_searchable() {
        use crate::searchable_pdf::tests::fixture;
        let dir = tempfile::tempdir().unwrap();
        let job_dir = dir.path().join("job");
        std::fs::create_dir(&job_dir).unwrap();
        std::fs::copy(fixture("scanned-turkish.pdf"), job_dir.join("snapshot.pdf")).unwrap();
        let context = crate::document_session::SessionJobContext::detached(
            job_dir.join("snapshot.pdf"),
            None,
            "real-tesseract",
        );
        let target = dir.path().join("aranabilir.pdf");
        let report = searchable_export_job(
            &context,
            &tesseract_program(),
            "tur",
            None,
            &target,
            SEARCHABLE_JOB_LIMITS,
        )
        .unwrap();
        assert_eq!(report.processed_pages, vec![0, 1, 2, 3, 4]);
        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let copy = pdfium.load_pdf_from_file(&target, None).unwrap();
            for index in 0..5u16 {
                let text: String = copy
                    .pages()
                    .get(index)
                    .unwrap()
                    .text()
                    .unwrap()
                    .chars()
                    .iter()
                    .filter_map(|c| c.unicode_char())
                    .collect();
                for word in ["Çalışma", "Muğla", "öğüt", "İstanbul"] {
                    assert!(text.contains(word), "page {index} lacks {word}: {text}");
                }
            }
            Ok(())
        })
        .unwrap();
    }

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

        mod searchable_job {
            use super::super::super::*;
            use crate::document_session::SessionJobContext;
            use crate::searchable_pdf::tests::{fixture, fixture_words};
            use crate::searchable_pdf::SkipReason;
            use std::os::unix::fs::PermissionsExt;
            use std::path::PathBuf;

            struct JobFixture {
                dir: tempfile::TempDir,
                program: PathBuf,
                context: SessionJobContext,
                target: PathBuf,
                /// Sahte tesseract her çalıştığında buraya bir satır yazar.
                runs: PathBuf,
            }

            impl JobFixture {
                /// `script` sahte tesseract'ın gövdesi; `$name` render edilen sayfanın adı
                /// (`searchable-ocr-<i>`), `$tsv` o sayfa için hazırlanan TSV dosyası.
                fn new(pdf: &str, credential: Option<&str>, script: &str) -> Self {
                    let dir = tempfile::tempdir().unwrap();
                    let job_dir = dir.path().join("job");
                    let tsv_dir = dir.path().join("tsv");
                    std::fs::create_dir(&job_dir).unwrap();
                    std::fs::create_dir(&tsv_dir).unwrap();
                    std::fs::copy(fixture(pdf), job_dir.join("snapshot.pdf")).unwrap();
                    for (page_index, words) in fixture_words() {
                        // Bütün fixture sayfaları OCR ölçeğinde nokta başına 2 pikseldir.
                        let mut tsv = format!("{}\n", super::super::TSV_HEADER);
                        for (index, (text, [l, t, r, b])) in words.iter().enumerate() {
                            tsv.push_str(&format!(
                                "5\t1\t1\t1\t1\t{}\t{}\t{}\t{}\t{}\t93.5\t{text}\n",
                                index + 1,
                                (l * 2.0).floor(),
                                (t * 2.0).floor(),
                                ((r - l) * 2.0).ceil(),
                                ((b - t) * 2.0).ceil(),
                            ));
                        }
                        std::fs::write(
                            tsv_dir.join(format!("searchable-ocr-{page_index}.tsv")),
                            tsv,
                        )
                        .unwrap();
                    }
                    let runs = dir.path().join("runs");
                    let program = dir.path().join("fake-tesseract");
                    std::fs::write(
                        &program,
                        format!(
                            "#!/bin/sh\nname=$(basename \"$1\" .png)\ntsv='{}'/$name.tsv\necho \"$name $*\" >> '{}'\n{script}\n",
                            tsv_dir.display(),
                            runs.display()
                        ),
                    )
                    .unwrap();
                    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let context = SessionJobContext::detached(
                        job_dir.join("snapshot.pdf"),
                        credential.map(str::to_string),
                        "searchable-test",
                    );
                    let target = dir.path().join("cikti").join("aranabilir.pdf");
                    std::fs::create_dir(target.parent().unwrap()).unwrap();
                    Self {
                        dir,
                        program,
                        context,
                        target,
                        runs,
                    }
                }

                fn run(
                    &self,
                    pages: Option<Vec<u32>>,
                    limits: SearchableJobLimits,
                ) -> Result<SearchablePdfReport, SessionJobOperationError> {
                    searchable_export_job(
                        &self.context,
                        self.program.as_os_str(),
                        "tur",
                        pages,
                        &self.target,
                        limits,
                    )
                }

                fn runs(&self) -> Vec<String> {
                    std::fs::read_to_string(&self.runs)
                        .map(|text| text.lines().map(str::to_string).collect())
                        .unwrap_or_default()
                }

                fn leftover_images(&self) -> Vec<PathBuf> {
                    std::fs::read_dir(self.dir.path().join("job"))
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
                        .collect()
                }
            }

            const CAT_TSV: &str = "cat \"$tsv\"";

            fn failure_code(error: SessionJobOperationError) -> String {
                match error {
                    SessionJobOperationError::Failed(failure) => failure.code,
                    SessionJobOperationError::Cancelled(reason) => format!("CANCELLED:{reason:?}"),
                }
            }

            fn copy_text(path: &Path, page: u16) -> String {
                crate::pdf_engine::with_pdfium(|pdfium| -> Result<String, String> {
                    let document = pdfium
                        .load_pdf_from_file(path, None)
                        .map_err(|error| error.to_string())?;
                    let page = document.pages().get(page).unwrap();
                    let text = page.text().unwrap();
                    let chars = text.chars();
                    let collected: String = chars.iter().filter_map(|c| c.unicode_char()).collect();
                    Ok(collected)
                })
                .unwrap()
            }

            #[test]
            fn scanned_pages_become_searchable_and_the_rest_is_reported() {
                let fixture = JobFixture::new("scanned-turkish.pdf", None, CAT_TSV);
                let report = fixture.run(None, SEARCHABLE_JOB_LIMITS).unwrap();
                assert_eq!(report.processed_pages, vec![0, 1, 2, 3, 4]);
                assert_eq!(
                    report.skipped_pages,
                    vec![
                        SkippedSearchablePage {
                            page_index: 5,
                            reason: SkipReason::HasText,
                            detail: None
                        },
                        SkippedSearchablePage {
                            page_index: 6,
                            reason: SkipReason::NoImage,
                            detail: None
                        },
                    ]
                );
                let total: usize = fixture_words().iter().map(|(_, words)| words.len()).sum();
                assert_eq!(report.words_written, total as u64);
                assert!(!report.signatures_invalidated);
                // Tesseract yalnız taranmış sayfalarda, sırayla ve seçilen dille çalıştı.
                let runs = fixture.runs();
                assert_eq!(runs.len(), 5);
                for (index, run) in runs.iter().enumerate() {
                    assert!(
                        run.starts_with(&format!("searchable-ocr-{index} ")),
                        "{run}"
                    );
                    assert!(run.contains(" -l tur "), "{run}");
                }
                for page in 0..5u16 {
                    let text = copy_text(&fixture.target, page);
                    assert!(
                        text.contains("Çalışma") && text.contains("şubelerinden"),
                        "page {page}: {text}"
                    );
                }
                assert!(
                    fixture.leftover_images().is_empty(),
                    "OCR images are removed"
                );
                assert_eq!(fixture.context.child_processes.active_child_count(), 0);
                // Oturumun snapshot'ı değişmedi.
                assert_eq!(
                    std::fs::read(fixture.context.snapshot_path.clone()).unwrap(),
                    std::fs::read(crate::searchable_pdf::tests::fixture("scanned-turkish.pdf"))
                        .unwrap()
                );
            }

            #[test]
            fn page_range_limits_the_ocr_to_the_selected_pages() {
                let fixture = JobFixture::new("scanned-turkish.pdf", None, CAT_TSV);
                let report = fixture
                    .run(Some(vec![3, 1, 5]), SEARCHABLE_JOB_LIMITS)
                    .unwrap();
                assert_eq!(report.processed_pages, vec![3, 1]);
                assert_eq!(report.skipped_pages.len(), 1);
                assert_eq!(fixture.runs().len(), 2);
                assert!(
                    copy_text(&fixture.target, 0).trim().is_empty(),
                    "unselected page untouched"
                );
                assert!(copy_text(&fixture.target, 1).contains("Muğla"));
            }

            #[test]
            fn selection_without_scanned_pages_fails_before_ocr() {
                let fixture = JobFixture::new("scanned-turkish.pdf", None, CAT_TSV);
                let error = fixture
                    .run(Some(vec![5, 6]), SEARCHABLE_JOB_LIMITS)
                    .unwrap_err();
                assert_eq!(failure_code(error), "SEARCHABLE_PDF_NO_SCANNED_PAGES");
                assert!(fixture.runs().is_empty());
                assert!(!fixture.target.exists());
            }

            #[test]
            fn encrypted_snapshot_is_rejected_before_ocr() {
                let fixture = JobFixture::new("encrypted.pdf", Some("fixture-password"), CAT_TSV);
                let error = fixture.run(None, SEARCHABLE_JOB_LIMITS).unwrap_err();
                assert_eq!(failure_code(error), "SEARCHABLE_PDF_ENCRYPTED");
                assert!(fixture.runs().is_empty());
                assert!(!fixture.target.exists());
            }

            #[test]
            fn a_failing_page_is_skipped_and_reported() {
                let fixture = JobFixture::new(
                    "scanned-turkish.pdf",
                    None,
                    "if [ \"$name\" = searchable-ocr-2 ]; then echo 'bozuk sayfa' >&2; exit 1; fi\ncat \"$tsv\"",
                );
                let report = fixture.run(None, SEARCHABLE_JOB_LIMITS).unwrap();
                assert_eq!(report.processed_pages, vec![0, 1, 3, 4]);
                let failed = report
                    .skipped_pages
                    .iter()
                    .find(|page| page.page_index == 2)
                    .unwrap();
                assert_eq!(failed.reason, SkipReason::OcrFailed);
                assert!(failed.detail.as_deref().unwrap().contains("bozuk sayfa"));
                assert!(copy_text(&fixture.target, 2).trim().is_empty());
            }

            #[test]
            fn every_page_failing_writes_nothing() {
                let fixture =
                    JobFixture::new("scanned-turkish.pdf", None, "echo 'kötü' >&2\nexit 1");
                let error = fixture.run(None, SEARCHABLE_JOB_LIMITS).unwrap_err();
                assert_eq!(failure_code(error), "SEARCHABLE_PDF_OCR_FAILED");
                assert!(!fixture.target.exists());
                assert!(fixture.leftover_images().is_empty());
            }

            #[test]
            fn pages_without_words_write_nothing() {
                let header = super::super::TSV_HEADER.replace('\t', "\\t");
                let fixture = JobFixture::new(
                    "scanned-turkish.pdf",
                    None,
                    &format!("printf '{header}\\n'"),
                );
                let error = fixture.run(None, SEARCHABLE_JOB_LIMITS).unwrap_err();
                assert_eq!(failure_code(error), "SEARCHABLE_PDF_NO_TEXT_FOUND");
                assert!(!fixture.target.exists());
            }

            #[test]
            fn mismatched_tsv_image_size_skips_the_page() {
                let header = super::super::TSV_HEADER.replace('\t', "\\t");
                let fixture = JobFixture::new(
                    "scanned-turkish.pdf",
                    None,
                    &format!(
                        "if [ \"$name\" = searchable-ocr-0 ]; then printf '{header}\\n1\\t1\\t0\\t0\\t0\\t0\\t0\\t0\\t10\\t10\\t-1\\t\\n'; exit 0; fi\ncat \"$tsv\""
                    ),
                );
                let report = fixture.run(None, SEARCHABLE_JOB_LIMITS).unwrap();
                assert_eq!(report.processed_pages, vec![1, 2, 3, 4]);
                assert_eq!(report.skipped_pages[0].page_index, 0);
                assert_eq!(report.skipped_pages[0].reason, SkipReason::OcrFailed);
            }

            #[test]
            fn missing_tesseract_fails_the_job() {
                let fixture = JobFixture::new("scanned-turkish.pdf", None, CAT_TSV);
                let missing = fixture.dir.path().join("no-such-tesseract");
                let error = searchable_export_job(
                    &fixture.context,
                    missing.as_os_str(),
                    "tur",
                    None,
                    &fixture.target,
                    SEARCHABLE_JOB_LIMITS,
                )
                .unwrap_err();
                assert_eq!(failure_code(error), "OCR_ENGINE_NOT_FOUND");
                assert!(!fixture.target.exists());
            }

            #[test]
            fn exhausted_total_time_fails_without_ocr() {
                let fixture = JobFixture::new("scanned-turkish.pdf", None, CAT_TSV);
                let limits = SearchableJobLimits {
                    total: Duration::ZERO,
                    ..SEARCHABLE_JOB_LIMITS
                };
                let error = fixture.run(None, limits).unwrap_err();
                assert_eq!(failure_code(error), "SEARCHABLE_PDF_TIMEOUT");
                assert!(fixture.runs().is_empty());
                assert!(!fixture.target.exists());
            }

            #[test]
            fn page_timeout_kills_tesseract_and_writes_nothing() {
                let probe = tempfile::tempdir().unwrap();
                let pid_file = probe.path().join("pid");
                let fixture = JobFixture::new(
                    "scanned-turkish.pdf",
                    None,
                    &super::sleeping_script(&pid_file),
                );
                let limits = SearchableJobLimits {
                    per_page: Duration::from_secs(2),
                    ..SEARCHABLE_JOB_LIMITS
                };
                let started = Instant::now();
                let error = fixture.run(None, limits).unwrap_err();
                assert!(started.elapsed() < Duration::from_secs(30));
                // Çocuk süreç zaman aşımı, mevcut OCR akışıyla aynı kodla bütün işi düşürür.
                match error {
                    SessionJobOperationError::Failed(failure) => {
                        assert_eq!(failure.code, "DOCUMENT_SESSION_PDF_ENGINE_ERROR");
                        assert!(
                            failure.message.contains("zaman aşımı"),
                            "{}",
                            failure.message
                        );
                    }
                    other => panic!("unexpected: {other:?}"),
                }
                let pid = std::fs::read_to_string(&pid_file).unwrap();
                assert!(
                    !super::process_alive(pid.trim()),
                    "tesseract still running after timeout"
                );
                assert!(!fixture.target.exists());
                assert!(fixture.leftover_images().is_empty());
                assert_eq!(fixture.context.child_processes.active_child_count(), 0);
            }

            #[test]
            fn cancellation_kills_tesseract_and_writes_nothing() {
                let probe = tempfile::tempdir().unwrap();
                let pid_file = probe.path().join("pid");
                let fixture = JobFixture::new(
                    "scanned-turkish.pdf",
                    None,
                    &super::sleeping_script(&pid_file),
                );
                let token = fixture.context.cancellation.token().clone();
                let (result, pid) = std::thread::scope(|scope| {
                    let running = scope.spawn(|| fixture.run(None, SEARCHABLE_JOB_LIMITS));
                    let pid = super::wait_for_pid(&pid_file);
                    token.cancel();
                    (running.join().unwrap(), pid)
                });
                assert!(
                    matches!(result, Err(SessionJobOperationError::Cancelled(_))),
                    "{result:?}"
                );
                assert!(
                    !super::process_alive(&pid),
                    "tesseract {pid} still running after cancel"
                );
                assert!(!fixture.target.exists());
                assert_eq!(fixture.context.child_processes.active_child_count(), 0);
            }
        }
    }
}
