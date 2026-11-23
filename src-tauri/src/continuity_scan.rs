//! iPhone/iPad Continuity Camera ile tarayıp açık PDF'e sayfa ekleme.
//!
//! Akış ([`scan_into_session`]):
//! 1. Aynı anda tek tarama ([`ContinuityScanGate`]).
//! 2. Oturum instance'ının özel `scans/` kökünde 0700 bir tarama dizini açılır. Dizin her
//!    durumda (başarı, iptal, hata) silinir; çökmede instance temizliği kaldırır.
//! 3. Yardımcı (`antigravity-macos-helper scan`) bir session job'un çocuk süreci olarak
//!    çalışır: job iptali, uygulama kapanışı ve zaman aşımı süreci öldürür. Job, belgenin
//!    generation'ını rezervasyonda doğrular.
//! 4. Yardımcının stdout bildirimi ve dizin içeriği doğrulanır (yalnız beklenen adlar,
//!    symlink yok, boyut sınırları). Görseller yeni bir PDF'e çevrilir
//!    ([`crate::images_to_pdf::build_pdf_fitted`]); kullanıcı PDF'i açılmaz.
//! 5. Sonuç `ImportPages` ile tek, geri alınabilir bir mutasyon olarak eklenir. Kaynak
//!    mutasyon job dizinine kopyalanır; worker kullanıcı yolunu ya da tarama dizinini
//!    görmez ve dosyayı güvenilmeyen girdi olarak PDFium ile açar.
//!
//! Generation tarama sırasında değişirse mutasyon reddedilir ve belge değişmez.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::document_session::{
    create_secure_dir, ChildOutputLimits, DocumentSessionError, DocumentSessionSnapshot,
    DocumentSessionStore, SessionJobCancellationReason, SessionJobOperationError,
    SessionJobOutcome,
};
use crate::job_worker::MutationRecipe;

/// Bir taramada en fazla sayfa/görsel (yardımcıdaki `scanMaxPages` ile aynı).
pub const SCAN_MAX_PAGES: usize = 100;
/// Tek çıktı dosyası ve tüm çıktı için bayt sınırları (yardımcıdakilerle aynı).
pub const SCAN_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub const SCAN_MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

const SCAN_PDF_NAME: &str = "scan.pdf";
/// Görsellerden üretilen ve içe aktarılan PDF; yardımcı bu adı hiç yazmaz.
const SCAN_IMPORT_NAME: &str = "scan-input.pdf";
/// Kâğıt kenarı sınırı (pt), `images_to_pdf` ile aynı.
const MAX_PAGE_POINTS: f32 = 14_400.0;
const HELPER_OUTPUT_LIMITS: ChildOutputLimits = ChildOutputLimits {
    max_stdout_bytes: 16 * 1024,
    max_stderr_bytes: 16 * 1024,
};

/// Aynı anda en fazla bir tarama: menü ve iPhone tarafı tek bir etkileşimdir.
#[derive(Default)]
pub struct ContinuityScanGate(AtomicBool);

pub struct ContinuityScanGateGuard<'a>(&'a AtomicBool);

impl ContinuityScanGate {
    pub fn try_acquire(&self) -> Result<ContinuityScanGateGuard<'_>, DocumentSessionError> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ContinuityScanGateGuard(&self.0))
            .map_err(|_| {
                DocumentSessionError::new(
                    "CONTINUITY_SCAN_BUSY",
                    "Başka bir Continuity Camera taraması sürüyor",
                )
            })
    }
}

impl Drop for ContinuityScanGateGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Görsel taramaların sığdırılacağı kâğıt (pt): eklendiği yerin komşu sayfası.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanPageFit {
    pub width: f32,
    pub height: f32,
}

impl ScanPageFit {
    fn validate(self) -> Result<Self, DocumentSessionError> {
        if [self.width, self.height]
            .iter()
            .all(|side| side.is_finite() && (1.0..=MAX_PAGE_POINTS).contains(side))
        {
            Ok(self)
        } else {
            Err(DocumentSessionError::new(
                "CONTINUITY_SCAN_INVALID_REQUEST",
                format!("Sayfa kenarı 1 ile {MAX_PAGE_POINTS} pt arasında olmalı"),
            ))
        }
    }
}

pub struct ContinuityScanRequest {
    pub job_id: String,
    pub session_id: String,
    pub expected_content_generation: u64,
    pub at_index: u32,
    pub page_fit: ScanPageFit,
}

/// Tarama dizini verildiğinde yardımcıyı çalıştıracak komutu kurar. Üretimde
/// `macos_helper::scan_command_args`; testlerde sahte bir yardımcı.
pub type ScanHelperCommand = Arc<dyn Fn(&Path) -> Result<Command, String> + Send + Sync>;

#[derive(Clone)]
pub struct ScanHelper {
    pub command: ScanHelperCommand,
    /// Kesin üst süre; dolunca süreç öldürülür (`CONTINUITY_SCAN_TIMEOUT`).
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanContentKind {
    Pdf,
    Images,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinuityScanImport {
    pub snapshot: DocumentSessionSnapshot,
    pub kind: ScanContentKind,
}

/// Yardımcının stdout'a yazdığı tek satır.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperReport {
    kind: ScanContentKind,
    files: Vec<String>,
}

struct PreparedScan {
    source: PathBuf,
    kind: ScanContentKind,
}

fn failed(code: &str, message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

fn output_invalid(message: impl Into<String>) -> SessionJobOperationError {
    failed("CONTINUITY_SCAN_OUTPUT_INVALID", message)
}

fn stderr_detail(stderr: &[u8]) -> String {
    const MAX_CHARS: usize = 500;
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.chars().count() > MAX_CHARS {
        format!("{}…", text.chars().take(MAX_CHARS).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Yardımcının çıkış kodunu job hatasına çevirir (kodlar `main.swift` başlığında).
/// 2 (menü seçimsiz kapandı) iptal sayılır.
pub(crate) fn helper_exit_error(code: Option<i32>, stderr: &[u8]) -> SessionJobOperationError {
    let detail = stderr_detail(stderr);
    let with_detail = |message: &str| {
        if detail.is_empty() {
            message.to_string()
        } else {
            format!("{message}: {detail}")
        }
    };
    match code {
        Some(2) => SessionJobOperationError::Cancelled(SessionJobCancellationReason::UserRequested),
        Some(68) => failed(
            "CONTINUITY_SCAN_NO_DEVICE",
            "Continuity Camera için uygun bir iPhone veya iPad bulunamadı. Cihaz aynı Apple \
             hesabıyla oturum açmış, Wi-Fi ve Bluetooth açık ve Mac'e yakın olmalıdır.",
        ),
        Some(75) => failed(
            "CONTINUITY_SCAN_TIMEOUT",
            with_detail("Continuity Camera zaman aşımına uğradı"),
        ),
        Some(66) => failed(
            "CONTINUITY_SCAN_UNSUPPORTED_CONTENT",
            with_detail("Taramadan desteklenmeyen bir içerik geldi"),
        ),
        Some(73) => failed(
            "CONTINUITY_SCAN_LIMIT_EXCEEDED",
            with_detail("Tarama sınırları aşıldı"),
        ),
        Some(69) => failed(
            "FEATURE_UNAVAILABLE",
            with_detail("Continuity Camera bu macOS sürümünde kullanılamaz"),
        ),
        other => failed(
            "CONTINUITY_SCAN_FAILED",
            with_detail(&match other {
                Some(code) => format!("Continuity Camera yardımcısı {code} koduyla kapandı"),
                None => "Continuity Camera yardımcısı bir sinyalle sonlandı".to_string(),
            }),
        ),
    }
}

fn parse_helper_report(stdout: &[u8]) -> Result<HelperReport, SessionJobOperationError> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| output_invalid("Tarama yardımcısının çıktısı UTF-8 değil"))?
        .trim();
    if text.is_empty() || text.lines().count() != 1 {
        return Err(output_invalid(
            "Tarama yardımcısı tek satırlık bir sonuç bildirmedi",
        ));
    }
    serde_json::from_str(text)
        .map_err(|error| output_invalid(format!("Tarama sonucu okunamadı: {error}")))
}

fn expected_name_is_valid(kind: ScanContentKind, index: usize, name: &str) -> bool {
    match kind {
        ScanContentKind::Pdf => index == 0 && name == SCAN_PDF_NAME,
        ScanContentKind::Images => {
            let stem = format!("scan-{:03}", index + 1);
            name == format!("{stem}.jpg") || name == format!("{stem}.png")
        }
    }
}

/// Bildirilen dosyaların sabit kalıpta olduğunu, dizinde tam olarak onların bulunduğunu,
/// her birinin symlink olmayan, boş olmayan bir dosya olduğunu ve sınırları aşmadığını
/// doğrular; dosya yollarını sırayla döndürür.
fn validate_scan_output(
    directory: &Path,
    report: &HelperReport,
) -> Result<Vec<PathBuf>, SessionJobOperationError> {
    let count = report.files.len();
    let count_ok = match report.kind {
        ScanContentKind::Pdf => count == 1,
        ScanContentKind::Images => (1..=SCAN_MAX_PAGES).contains(&count),
    };
    if !count_ok {
        return Err(output_invalid(format!(
            "Tarama yardımcısı beklenmeyen sayıda dosya bildirdi: {count}"
        )));
    }
    for (index, name) in report.files.iter().enumerate() {
        if !expected_name_is_valid(report.kind, index, name) {
            return Err(output_invalid(format!(
                "Tarama yardımcısı beklenmeyen bir dosya adı bildirdi: {name:?}"
            )));
        }
    }
    let mut present = Vec::new();
    let entries = std::fs::read_dir(directory)
        .map_err(|error| output_invalid(format!("Tarama dizini okunamadı: {error}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| output_invalid(format!("Tarama dizini okunamadı: {error}")))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| output_invalid(format!("Beklenmeyen dosya: {name:?}")))?;
        present.push(name);
    }
    present.sort();
    let mut expected = report.files.clone();
    expected.sort();
    if present != expected {
        return Err(output_invalid(format!(
            "Tarama dizini bildirilenden farklı dosyalar içeriyor: {present:?}"
        )));
    }
    let mut total = 0u64;
    let mut paths = Vec::with_capacity(count);
    for name in &report.files {
        let path = directory.join(name);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| output_invalid(format!("{name} okunamadı: {error}")))?;
        if !metadata.file_type().is_file() {
            return Err(output_invalid(format!("{name} normal bir dosya değil")));
        }
        if metadata.len() == 0 {
            return Err(output_invalid(format!("{name} boş")));
        }
        total = total.saturating_add(metadata.len());
        if metadata.len() > SCAN_MAX_FILE_BYTES || total > SCAN_MAX_TOTAL_BYTES {
            return Err(failed(
                "CONTINUITY_SCAN_LIMIT_EXCEEDED",
                "Taranan içerik boyut sınırını aşıyor",
            ));
        }
        paths.push(path);
    }
    Ok(paths)
}

/// İçe aktarılacak PDF: tarama PDF'i ya da görsellerden özel dizinde üretilen yeni PDF.
fn prepare_import_source(
    directory: &Path,
    kind: ScanContentKind,
    files: &[PathBuf],
    fit: ScanPageFit,
) -> Result<PathBuf, SessionJobOperationError> {
    match kind {
        ScanContentKind::Pdf => Ok(files[0].clone()),
        ScanContentKind::Images => {
            let paths = files
                .iter()
                .map(|path| {
                    path.to_str()
                        .map(str::to_string)
                        .ok_or_else(|| output_invalid("Tarama dizini yolu UTF-8 değil"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let bytes = crate::images_to_pdf::build_pdf_fitted(&paths, fit.width, fit.height)
                .map_err(|message| {
                    failed(
                        "CONTINUITY_SCAN_CONVERT_FAILED",
                        format!("Taranan görseller PDF'e çevrilemedi: {message}"),
                    )
                })?;
            let target = directory.join(SCAN_IMPORT_NAME);
            let write = || -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                file.write_all(&bytes)?;
                file.sync_all()
            };
            write().map_err(|error| {
                failed(
                    "CONTINUITY_SCAN_CONVERT_FAILED",
                    format!("Taranan görsellerin PDF'i yazılamadı: {error}"),
                )
            })?;
            Ok(target)
        }
    }
}

/// Yardımcıyı çalıştırır ve çıktısını içe aktarılmaya hazır bir PDF'e çevirir.
/// Session job'un `spawn_blocking` işçisinde çalışır.
fn run_helper_and_prepare(
    context: &crate::document_session::SessionJobContext,
    helper: &ScanHelper,
    directory: &Path,
    fit: ScanPageFit,
) -> Result<PreparedScan, SessionJobOperationError> {
    context.cancellation.check_cancelled()?;
    let mut command = (helper.command)(directory)
        .map_err(|message| failed("CONTINUITY_SCAN_HELPER_UNAVAILABLE", message))?;
    let started = Instant::now();
    let output = context
        .child_processes
        .run_with_bounded_output_timeout(
            &mut command,
            helper.timeout,
            &context.cancellation,
            HELPER_OUTPUT_LIMITS,
        )
        .map_err(|error| match error {
            SessionJobOperationError::Failed(_) if started.elapsed() >= helper.timeout => failed(
                "CONTINUITY_SCAN_TIMEOUT",
                "Continuity Camera zaman aşımına uğradı; tarama durduruldu",
            ),
            other => other,
        })?;
    context.cancellation.check_cancelled()?;
    if !output.status.success() {
        return Err(helper_exit_error(output.status.code(), &output.stderr));
    }
    let report = parse_helper_report(&output.stdout)?;
    let files = validate_scan_output(directory, &report)?;
    let source = prepare_import_source(directory, report.kind, &files, fit)?;
    context.cancellation.check_cancelled()?;
    Ok(PreparedScan {
        source,
        kind: report.kind,
    })
}

/// Tarar ve sonucu `at_index`'e ekler. İptal (uygulamadan ya da menü seçimsiz kapanınca)
/// `Cancelled`, yardımcı hataları `Failed` sonucu olur; ikisinde de belge değişmez.
/// Mutasyon hataları (ör. generation değişti) `Err` döner.
pub async fn scan_into_session(
    store: &DocumentSessionStore,
    gate: &ContinuityScanGate,
    helper: ScanHelper,
    request: ContinuityScanRequest,
) -> Result<SessionJobOutcome<ContinuityScanImport>, DocumentSessionError> {
    let fit = request.page_fit.validate()?;
    let _guard = gate.try_acquire()?;
    let scans_root = store.scans_root();
    create_secure_dir(&scans_root)?;
    let scan_dir = tempfile::Builder::new()
        .prefix("scan-")
        .tempdir_in(&scans_root)
        .map_err(|error| {
            DocumentSessionError::new(
                "DOCUMENT_SESSION_TEMP_CREATE_FAILED",
                format!("Tarama dizini oluşturulamadı: {error}"),
            )
        })?;
    let directory = scan_dir.path().to_path_buf();
    let outcome = store
        .run_snapshot_job(
            &request.job_id,
            &request.session_id,
            request.expected_content_generation,
            move |context| run_helper_and_prepare(&context, &helper, &directory, fit),
        )
        .await?;
    let result = match outcome {
        SessionJobOutcome::Completed {
            job_id,
            session_id,
            content_generation,
            value,
            cancel_requested,
            cancellation_reason,
            cleanup,
        } => {
            if cancel_requested {
                // Yardımcı bittikten sonra gelen iptal de belgeyi değiştirmez.
                SessionJobOutcome::Cancelled {
                    job_id,
                    session_id,
                    content_generation,
                    cancellation: cancellation_reason
                        .unwrap_or(SessionJobCancellationReason::UserRequested),
                    cleanup,
                }
            } else {
                let snapshot = store
                    .mutate_isolated(
                        session_id.clone(),
                        request.expected_content_generation,
                        MutationRecipe::ImportPages {
                            source_path: Some(value.source),
                            input_name: None,
                            source_pages: Vec::new(),
                            at_index: request.at_index,
                            replace_count: 0,
                        },
                    )
                    .await?;
                SessionJobOutcome::Completed {
                    job_id,
                    session_id,
                    content_generation,
                    value: ContinuityScanImport {
                        snapshot,
                        kind: value.kind,
                    },
                    cancel_requested: false,
                    cancellation_reason: None,
                    cleanup,
                }
            }
        }
        SessionJobOutcome::Failed {
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
        SessionJobOutcome::Cancelled {
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
    };
    // Erken dönüşlerde `TempDir` düşerken siler; burada hata görünür kılınır.
    if let Err(error) = scan_dir.close() {
        eprintln!("Continuity Camera tarama dizini silinemedi: {error}");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(kind: ScanContentKind, files: &[&str]) -> HelperReport {
        HelperReport {
            kind,
            files: files.iter().map(|name| name.to_string()).collect(),
        }
    }

    fn failure_code(error: SessionJobOperationError) -> String {
        match error {
            SessionJobOperationError::Failed(failure) => failure.code,
            SessionJobOperationError::Cancelled(reason) => format!("cancelled:{reason:?}"),
        }
    }

    #[test]
    fn helper_exit_codes_map_to_stable_error_codes() {
        let cases = [
            (Some(2), "cancelled:UserRequested"),
            (Some(68), "CONTINUITY_SCAN_NO_DEVICE"),
            (Some(75), "CONTINUITY_SCAN_TIMEOUT"),
            (Some(66), "CONTINUITY_SCAN_UNSUPPORTED_CONTENT"),
            (Some(73), "CONTINUITY_SCAN_LIMIT_EXCEEDED"),
            (Some(69), "FEATURE_UNAVAILABLE"),
            (Some(65), "CONTINUITY_SCAN_FAILED"),
            (Some(1), "CONTINUITY_SCAN_FAILED"),
            (None, "CONTINUITY_SCAN_FAILED"),
        ];
        for (code, expected) in cases {
            assert_eq!(
                failure_code(helper_exit_error(code, b"ayrinti")),
                expected,
                "{code:?}"
            );
        }
        let SessionJobOperationError::Failed(failure) =
            helper_exit_error(Some(66), b"  tip yok \n")
        else {
            panic!("failure expected");
        };
        assert!(
            failure.message.ends_with(": tip yok"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn helper_report_must_be_a_single_known_json_line() {
        let parsed =
            parse_helper_report(b"{\"kind\":\"images\",\"files\":[\"scan-001.jpg\"]}\n").unwrap();
        assert_eq!(parsed.kind, ScanContentKind::Images);
        assert_eq!(parsed.files, ["scan-001.jpg"]);
        for bad in [
            &b""[..],
            b"\n",
            b"{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"]}\n{\"kind\":\"pdf\",\"files\":[]}",
            b"{\"kind\":\"video\",\"files\":[]}",
            b"{\"kind\":\"pdf\",\"files\":[\"scan.pdf\"],\"extra\":1}",
            b"\xff\xfe",
            b"not json",
        ] {
            assert_eq!(
                failure_code(parse_helper_report(bad).unwrap_err()),
                "CONTINUITY_SCAN_OUTPUT_INVALID",
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn output_validation_accepts_only_reported_fixed_names() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("scan-001.jpg"), b"jpg").unwrap();
        std::fs::write(directory.path().join("scan-002.png"), b"png").unwrap();
        let files = validate_scan_output(
            directory.path(),
            &report(ScanContentKind::Images, &["scan-001.jpg", "scan-002.png"]),
        )
        .unwrap();
        assert_eq!(
            files,
            [
                directory.path().join("scan-001.jpg"),
                directory.path().join("scan-002.png")
            ]
        );

        for (kind, names) in [
            (
                ScanContentKind::Images,
                vec!["scan-002.jpg", "scan-001.png"],
            ),
            (ScanContentKind::Images, vec!["../scan-001.jpg"]),
            (ScanContentKind::Images, vec!["scan-001.heic"]),
            (ScanContentKind::Images, vec!["scan.pdf"]),
            (ScanContentKind::Images, vec![]),
            (ScanContentKind::Pdf, vec!["scan-001.jpg"]),
            (ScanContentKind::Pdf, vec!["scan.pdf", "scan.pdf"]),
            (ScanContentKind::Pdf, vec!["/etc/passwd"]),
        ] {
            assert_eq!(
                failure_code(
                    validate_scan_output(directory.path(), &report(kind, &names)).unwrap_err()
                ),
                "CONTINUITY_SCAN_OUTPUT_INVALID",
                "{kind:?} {names:?}"
            );
        }
        let too_many: Vec<String> = (1..=SCAN_MAX_PAGES + 1)
            .map(|index| format!("scan-{index:03}.jpg"))
            .collect();
        let too_many = HelperReport {
            kind: ScanContentKind::Images,
            files: too_many,
        };
        assert_eq!(
            failure_code(validate_scan_output(directory.path(), &too_many).unwrap_err()),
            "CONTINUITY_SCAN_OUTPUT_INVALID"
        );
    }

    #[test]
    fn output_validation_rejects_extra_missing_empty_and_non_regular_files() {
        // Bildirilmeyen ek dosya.
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("scan.pdf"), b"%PDF-1.4").unwrap();
        std::fs::write(directory.path().join("extra.txt"), b"x").unwrap();
        let pdf = report(ScanContentKind::Pdf, &["scan.pdf"]);
        assert_eq!(
            failure_code(validate_scan_output(directory.path(), &pdf).unwrap_err()),
            "CONTINUITY_SCAN_OUTPUT_INVALID"
        );
        // Bildirilen ama yazılmamış dosya (boş çıktı).
        let empty_dir = tempfile::tempdir().unwrap();
        assert_eq!(
            failure_code(validate_scan_output(empty_dir.path(), &pdf).unwrap_err()),
            "CONTINUITY_SCAN_OUTPUT_INVALID"
        );
        // Sıfır baytlık dosya.
        std::fs::write(empty_dir.path().join("scan.pdf"), b"").unwrap();
        assert_eq!(
            failure_code(validate_scan_output(empty_dir.path(), &pdf).unwrap_err()),
            "CONTINUITY_SCAN_OUTPUT_INVALID"
        );
        // Dizin.
        let dir_entry = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir_entry.path().join("scan.pdf")).unwrap();
        assert_eq!(
            failure_code(validate_scan_output(dir_entry.path(), &pdf).unwrap_err()),
            "CONTINUITY_SCAN_OUTPUT_INVALID"
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_validation_rejects_symlinks_and_oversized_files() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.pdf");
        std::fs::write(&secret, b"%PDF-1.4 secret").unwrap();
        let directory = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&secret, directory.path().join("scan.pdf")).unwrap();
        assert_eq!(
            failure_code(
                validate_scan_output(
                    directory.path(),
                    &report(ScanContentKind::Pdf, &["scan.pdf"])
                )
                .unwrap_err()
            ),
            "CONTINUITY_SCAN_OUTPUT_INVALID"
        );

        let large = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(large.path().join("scan.pdf")).unwrap();
        file.set_len(SCAN_MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(
            failure_code(
                validate_scan_output(large.path(), &report(ScanContentKind::Pdf, &["scan.pdf"]))
                    .unwrap_err()
            ),
            "CONTINUITY_SCAN_LIMIT_EXCEEDED"
        );
    }

    #[test]
    fn page_fit_must_be_a_finite_bounded_paper() {
        assert!(ScanPageFit {
            width: 595.0,
            height: 842.0
        }
        .validate()
        .is_ok());
        for (width, height) in [
            (0.0, 842.0),
            (595.0, -1.0),
            (f32::NAN, 842.0),
            (595.0, f32::INFINITY),
            (20_000.0, 842.0),
        ] {
            let error = ScanPageFit { width, height }.validate().unwrap_err();
            assert_eq!(
                error.code, "CONTINUITY_SCAN_INVALID_REQUEST",
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn gate_allows_one_scan_at_a_time() {
        let gate = ContinuityScanGate::default();
        let guard = gate.try_acquire().unwrap();
        assert_eq!(
            gate.try_acquire().err().unwrap().code,
            "CONTINUITY_SCAN_BUSY"
        );
        drop(guard);
        assert!(gate.try_acquire().is_ok());
    }
}
