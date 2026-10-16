//! Yerel metin okuma (Text-to-Speech).
//!
//! Güvenlik sınırları:
//! - Kullanıcı metni hiçbir kabuk/komut satırına gömülmez; 0600 izinli UTF-8
//!   geçici dosyaya yazılır ve TTS süreci metni bu dosyadan okur.
//! - Windows'ta PowerShell betiği sabittir; dosya yolu ortam değişkeniyle verilir.
//! - Yalnız bu modülün başlattığı süreç izlenir ve sonlandırılır; sistemdeki
//!   diğer `say` / `powershell` süreçlerine dokunulmaz.
//!
//! Bitiş sözleşmesi: süreç kendiliğinden bittiğinde [`SPEECH_FINISHED_EVENT`]
//! olayı `speak_text`'in döndürdüğü `generation` ile yayınlanır. `stop()` veya
//! yeni bir `start()` ile sonlandırılan süreç olay göndermez; generation
//! ilerletildiği için reaper sessizce çıkar.
//!
//! Sıralama: `speak_text` komuta girer girmez bir [`SpeechTicket`] alır. Ses
//! çözümü ve geçici dosya hazırlanırken `stop()` veya daha yeni bir istek
//! gelirse bilet geçersizleşir ve süreç hiç başlatılmaz (`cancelled: true`).
//!
//! Komut/argüman üretimi cfg'siz saf fonksiyonlardadır; testler her platformda çalışır.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::Mutex;
use tempfile::TempPath;

pub const DEFAULT_MACOS_VOICE: &str = "Yelda";
pub const MAX_SPEECH_TEXT_CHARS: usize = 1_000_000;
pub const MAX_VOICE_CHARS: usize = 64;
pub const WINDOWS_TEXT_PATH_ENV: &str = "ANTIGRAVITY_TTS_TEXT_PATH";

/// Süreç kendiliğinden bittiğinde yayınlanan Tauri olayı.
pub const SPEECH_FINISHED_EVENT: &str = "tts://finished";
/// Hata mesajı için saklanan en fazla stderr baytı; fazlası okunup atılır.
pub const MAX_STDERR_BYTES: usize = 4 * 1024;
const MAX_STDERR_MESSAGE_CHARS: usize = 240;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Süreç bittikten sonra stderr okuyucusunun EOF'a ulaşması için beklenen süre.
const STDERR_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
/// Kurulu ses listesi bu süreden eskiyse her durumda yeniden sorgulanır
/// (kaldırılan sesler de fark edilir).
const VOICE_LIST_MAX_AGE: Duration = Duration::from_secs(10 * 60);
/// İstenen ses listede yoksa liste en fazla bu sıklıkla yeniden sorgulanır;
/// uygulama açıkken kurulan ses böylece kısa sürede kullanılmaya başlar.
const VOICE_LIST_MISS_REFRESH: Duration = Duration::from_secs(30);
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Sabit PowerShell betiği. Kullanıcı verisi içermez; metni yalnız
/// `$env:ANTIGRAVITY_TTS_TEXT_PATH` ile gösterilen dosyadan okur.
pub const WINDOWS_SPEECH_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
Add-Type -AssemblyName System.Speech; \
$text = Get-Content -LiteralPath $env:ANTIGRAVITY_TTS_TEXT_PATH -Raw -Encoding UTF8; \
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
try { $synth.Speak($text) } finally { $synth.Dispose() }";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechPlatform {
    MacOs,
    Windows,
}

pub fn current_platform() -> Option<SpeechPlatform> {
    if cfg!(target_os = "macos") {
        Some(SpeechPlatform::MacOs)
    } else if cfg!(target_os = "windows") {
        Some(SpeechPlatform::Windows)
    } else {
        None
    }
}

/// Süreç başlatmak için gereken her şey. `text_file` süreç bittiğinde düşürülür
/// ve geçici dosya silinir.
pub struct SpeechInvocation {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub envs: Vec<(OsString, OsString)>,
    pub text_file: TempPath,
}

pub fn validate_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Okunacak metin yok".to_string());
    }
    if text.chars().count() > MAX_SPEECH_TEXT_CHARS {
        return Err(format!(
            "Okunacak metin çok uzun (en fazla {} karakter)",
            MAX_SPEECH_TEXT_CHARS
        ));
    }
    Ok(())
}

/// Ses adı doğrulaması. macOS ses adları harf, rakam, boşluk ve
/// `( ) - _ .` karakterlerinden oluşur (ör. "Eddy (Turkish (Türkiye))").
pub fn validate_voice(voice: &str) -> Result<(), String> {
    let invalid = || Err("Geçersiz ses adı".to_string());
    if voice.trim().is_empty() || voice.chars().count() > MAX_VOICE_CHARS {
        return invalid();
    }
    if voice.starts_with('-') || voice.starts_with(' ') {
        return invalid();
    }
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, ' ' | '(' | ')' | '-' | '_' | '.');
    if !voice.chars().all(allowed) {
        return invalid();
    }
    Ok(())
}

/// `say [-v <voice>] -f <file>`: metin argüman olarak verilmez, bu yüzden
/// "-" ile başlayan metin seçenek olarak yorumlanamaz. `voice` yoksa sistem
/// varsayılan sesi kullanılır.
pub fn macos_say_args(voice: Option<&str>, text_path: &Path) -> Vec<OsString> {
    let mut args = Vec::with_capacity(4);
    if let Some(voice) = voice {
        args.push(OsString::from("-v"));
        args.push(OsString::from(voice));
    }
    args.push(OsString::from("-f"));
    args.push(text_path.as_os_str().to_owned());
    args
}

/// `say -v '?'` çıktısını ses adlarına ayırır. Satır biçimi:
/// `<ad> <locale>    # <örnek cümle>`; ad boşluk ve parantez içerebilir
/// (ör. "Eddy (Turkish (Türkiye))"), locale ise tek kelimedir.
pub fn parse_say_voice_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let head = line.split('#').next()?.trim();
            let (name, _locale) = head.rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// İstenen ses kuruluysa kurulu adını döndürür (büyük/küçük harf duyarsız).
pub fn resolve_macos_voice(requested: &str, installed: &[String]) -> Option<String> {
    let requested = requested.to_lowercase();
    installed
        .iter()
        .find(|name| name.to_lowercase() == requested)
        .cloned()
}

fn list_installed_macos_voices() -> Option<Vec<String>> {
    let output = Command::new("say")
        .args(["-v", "?"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let voices = parse_say_voice_list(&String::from_utf8_lossy(&output.stdout));
    (!voices.is_empty()).then_some(voices)
}

/// `-EncodedCommand` UTF-16LE base64 bekler; sabit betik için deterministiktir.
pub fn windows_encoded_script() -> String {
    let utf16: Vec<u8> = WINDOWS_SPEECH_SCRIPT
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

pub fn windows_powershell_args() -> Vec<OsString> {
    [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(OsString::from(windows_encoded_script())))
    .collect()
}

/// Metni BOM'suz UTF-8 olarak geçici dosyaya yazar (Unix'te 0600 izinli).
pub fn write_text_file(text: &str) -> Result<TempPath, String> {
    let mut file = tempfile::Builder::new()
        .prefix("antigravity-tts-")
        .suffix(".txt")
        .tempfile()
        .map_err(|e| format!("Seslendirme geçici dosyası oluşturulamadı: {}", e))?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.flush())
        .map_err(|e| format!("Seslendirme geçici dosyası yazılamadı: {}", e))?;
    // Dosya tanıtıcısı kapatılır ki alt süreç (Windows dahil) dosyayı okuyabilsin.
    Ok(file.into_temp_path())
}

/// `voice` macOS'ta `-v` ile verilecek sestir; `None` sistem varsayılanını
/// kullanır. Kurulu olup olmadığını çağıran taraf ([`SpeechController`]) çözer.
pub fn prepare_speech(
    platform: SpeechPlatform,
    text: &str,
    voice: Option<&str>,
) -> Result<SpeechInvocation, String> {
    validate_text(text)?;
    if let Some(voice) = voice {
        validate_voice(voice)?;
    }
    let text_file = write_text_file(text)?;
    let invocation = match platform {
        SpeechPlatform::MacOs => SpeechInvocation {
            program: OsString::from("say"),
            args: macos_say_args(voice, &text_file),
            envs: Vec::new(),
            text_file,
        },
        // Windows SAPI varsayılan sesi kullanılır; voice parametresi yalnız doğrulanır.
        SpeechPlatform::Windows => SpeechInvocation {
            program: OsString::from("powershell.exe"),
            args: windows_powershell_args(),
            envs: vec![(
                OsString::from(WINDOWS_TEXT_PATH_ENV),
                text_file.as_os_str().to_owned(),
            )],
            text_file,
        },
    };
    Ok(invocation)
}

/// Süreç kendiliğinden bittiğinde frontend'e bildirilen sonuç.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeechOutcome {
    Completed,
    Failed,
}

/// [`SPEECH_FINISHED_EVENT`] yükü. `message` yalnız `failed` için doludur.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SpeechFinished {
    pub generation: u64,
    pub status: SpeechOutcome,
    pub message: Option<String>,
}

/// `speak_text` yanıtı. `voice` gerçekten kullanılan macOS sesidir (`None`:
/// sistem varsayılanı); `voice_fallback` istenen sesin kurulu bulunmadığını belirtir.
/// `cancelled` ise istek başlatılmadan önce `stop()` veya daha yeni bir istek
/// tarafından geçersizleştirilmiştir; bu durumda `generation` `None`'dır.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SpeechStarted {
    pub generation: Option<u64>,
    pub voice: Option<String>,
    pub voice_fallback: bool,
    pub cancelled: bool,
}

/// `speak_text` girişinde alınan sıra bileti; yalnız en son bilet süreç başlatabilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechTicket(u64);

/// Kurulu macOS sesleri. Yalnız başarılı sorgular saklanır; başarısız sorgu
/// bir sonraki çağrıda yeniden denenir.
#[derive(Default)]
struct VoiceCache {
    voices: Option<Arc<[String]>>,
    fetched_at: Option<Instant>,
}

impl VoiceCache {
    fn resolve(
        &mut self,
        requested: &str,
        now: Instant,
        list: impl FnOnce() -> Option<Vec<String>>,
    ) -> Option<String> {
        let age = self.fetched_at.map(|at| now.saturating_duration_since(at));
        let cached = self
            .voices
            .as_deref()
            .and_then(|voices| resolve_macos_voice(requested, voices));
        let refresh = match (age, &cached) {
            (None, _) => true,
            (Some(age), _) if age >= VOICE_LIST_MAX_AGE => true,
            (Some(age), None) => age >= VOICE_LIST_MISS_REFRESH,
            (Some(_), Some(_)) => false,
        };
        if !refresh {
            return cached;
        }
        match list() {
            Some(voices) => {
                let resolved = resolve_macos_voice(requested, &voices);
                self.voices = Some(Arc::from(voices));
                self.fetched_at = Some(now);
                resolved
            }
            // Sorgu başarısızsa eldeki (eski) liste kullanılır.
            None => cached,
        }
    }
}

type SpeechEventSink = Arc<dyn Fn(SpeechFinished) + Send + Sync>;

/// stderr'in ilk [`MAX_STDERR_BYTES`] baytını saklar, kalanını boşaltır ki
/// süreç dolu boruda bloklanmasın. Sonuç EOF'ta kanala gönderilir.
fn spawn_stderr_collector(stderr: ChildStderr) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut captured = Vec::new();
        let _ = (&mut stderr)
            .take(MAX_STDERR_BYTES as u64)
            .read_to_end(&mut captured);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        let _ = sender.send(captured);
    });
    receiver
}

/// stderr'in ilk boş olmayan satırını kısaltılmış ve kontrol karakterlerinden
/// arındırılmış olarak döndürür.
fn summarize_stderr(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(
        line.chars()
            .filter(|c| !c.is_control())
            .take(MAX_STDERR_MESSAGE_CHARS)
            .collect(),
    )
}

fn failure_message(status: ExitStatus, stderr: &[u8]) -> String {
    let base = match status.code() {
        Some(code) => format!("Seslendirme süreci hata koduyla sonlandı ({code})"),
        None => "Seslendirme süreci beklenmedik şekilde sonlandırıldı".to_string(),
    };
    match summarize_stderr(stderr) {
        Some(detail) => format!("{base}: {detail}"),
        None => base,
    }
}

struct SpeechProcess {
    child: Child,
    stderr: Option<mpsc::Receiver<Vec<u8>>>,
    // Süreçle birlikte düşürülür; drop geçici dosyayı siler.
    _text_file: TempPath,
}

impl SpeechProcess {
    fn terminate(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Kendiliğinden biten (try_wait ile toplanan) süreci sonuca çevirir.
    fn into_finished(
        mut self,
        generation: u64,
        status: std::io::Result<ExitStatus>,
    ) -> SpeechFinished {
        // try_wait hata verdiyse süreç hâlâ çalışıyor olabilir; kill+wait zararsızdır.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let stderr = self
            .stderr
            .take()
            .and_then(|receiver| receiver.recv_timeout(STDERR_DRAIN_TIMEOUT).ok())
            .unwrap_or_default();
        let (status, message) = match status {
            Ok(status) if status.success() => (SpeechOutcome::Completed, None),
            Ok(status) => (
                SpeechOutcome::Failed,
                Some(failure_message(status, &stderr)),
            ),
            Err(error) => (
                SpeechOutcome::Failed,
                Some(format!("Seslendirme süreci izlenemedi: {error}")),
            ),
        };
        SpeechFinished {
            generation,
            status,
            message,
        }
    }
}

#[derive(Default)]
struct SpeechSlot {
    generation: u64,
    /// Son verilen bilet; `stop()` de ilerletir ki bekleyen başlatmalar iptal olsun.
    latest_ticket: u64,
    current: Option<SpeechProcess>,
}

/// Uygulamanın başlattığı tek TTS sürecini tutan Tauri state'i.
#[derive(Clone)]
pub struct SpeechController {
    slot: Arc<Mutex<SpeechSlot>>,
    on_finished: SpeechEventSink,
    installed_voices: Arc<Mutex<VoiceCache>>,
}

impl SpeechController {
    /// `on_finished`, süreç kendiliğinden bittiğinde kilit dışında çağrılır.
    pub fn new(on_finished: impl Fn(SpeechFinished) + Send + Sync + 'static) -> Self {
        Self {
            slot: Arc::default(),
            on_finished: Arc::new(on_finished),
            installed_voices: Arc::default(),
        }
    }

    /// Bitiş olaylarını [`SPEECH_FINISHED_EVENT`] olarak yayınlar.
    pub fn for_app<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Self {
        use tauri::Emitter;
        Self::new(move |event| {
            if let Err(error) = app.emit(SPEECH_FINISHED_EVENT, &event) {
                tracing::warn!(%error, generation = event.generation, "TTS bitiş olayı yayınlanamadı");
            }
        })
    }

    /// İstenen ses yalnız kurulu olduğu doğrulanırsa `-v` ile verilir; aksi
    /// halde (kurulu değil veya liste alınamadı) sistem varsayılanı kullanılır.
    fn resolve_voice(&self, platform: SpeechPlatform, requested: &str) -> (Option<String>, bool) {
        match platform {
            SpeechPlatform::MacOs => {
                let resolved = self.installed_voices.lock().resolve(
                    requested,
                    Instant::now(),
                    list_installed_macos_voices,
                );
                let fallback = resolved.is_none();
                (resolved, fallback)
            }
            // Windows SAPI varsayılan sesi kullanılır; voice yalnız doğrulanır.
            SpeechPlatform::Windows => (None, false),
        }
    }

    /// Yeni bir başlatma isteği için bilet verir; önceki biletleri geçersizleştirir.
    pub fn reserve(&self) -> SpeechTicket {
        let mut slot = self.slot.lock();
        slot.latest_ticket = slot.latest_ticket.wrapping_add(1);
        SpeechTicket(slot.latest_ticket)
    }

    /// Bilet hâlâ en sonuncuysa mevcut süreci kill+wait ile sonlandırır,
    /// yenisini başlatır ve generation'ını döndürür. Bilet `stop()` veya daha
    /// yeni bir `reserve()` ile geçersizleştiyse hiçbir şey başlatmaz ve
    /// `None` döner (geçici dosya düşürülerek silinir). Kilit spawn boyunca
    /// tutulur; eşzamanlı çağrılarda tek süreç kalır.
    pub fn start(
        &self,
        invocation: SpeechInvocation,
        ticket: SpeechTicket,
    ) -> Result<Option<u64>, String> {
        let mut slot = self.slot.lock();
        if slot.latest_ticket != ticket.0 {
            return Ok(None);
        }
        if let Some(previous) = slot.current.take() {
            previous.terminate();
        }

        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .envs(invocation.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command
            .spawn()
            .map_err(|e| format!("Seslendirme başlatılamadı: {}", e))?;
        let stderr = child.stderr.take().map(spawn_stderr_collector);
        slot.generation = slot.generation.wrapping_add(1);
        let generation = slot.generation;
        slot.current = Some(SpeechProcess {
            child,
            stderr,
            _text_file: invocation.text_file,
        });
        drop(slot);

        self.spawn_reaper(generation);
        Ok(Some(generation))
    }

    /// Yalnız bu controller'ın başlattığı süreci sonlandırır. Generation
    /// ilerletilir; böylece durdurulan süreç için bitiş olayı yayınlanmaz.
    /// Bilet de ilerletilir; hazırlanmakta olan başlatma süreç açmaz.
    pub fn stop(&self) {
        let process = {
            let mut slot = self.slot.lock();
            slot.generation = slot.generation.wrapping_add(1);
            slot.latest_ticket = slot.latest_ticket.wrapping_add(1);
            slot.current.take()
        };
        if let Some(process) = process {
            process.terminate();
        }
    }

    #[cfg(test)]
    fn current_pid(&self) -> Option<u32> {
        self.slot.lock().current.as_ref().map(|p| p.child.id())
    }

    /// Süreç kendiliğinden bitince onu toplar, geçici dosyayı siler ve bitiş
    /// olayını yayınlar. Generation değiştiyse (stop/yeni start) sessizce çıkar.
    fn spawn_reaper(&self, generation: u64) {
        let slot = Arc::clone(&self.slot);
        let on_finished = Arc::clone(&self.on_finished);
        std::thread::spawn(move || loop {
            std::thread::sleep(PROCESS_POLL_INTERVAL);
            let mut guard = slot.lock();
            if guard.generation != generation {
                return;
            }
            let status = match guard.current.as_mut() {
                None => return,
                Some(process) => match process.child.try_wait() {
                    Ok(None) => continue,
                    Ok(Some(status)) => Ok(status),
                    Err(error) => Err(error),
                },
            };
            let Some(process) = guard.current.take() else {
                return;
            };
            drop(guard);
            on_finished(process.into_finished(generation, status));
            return;
        });
    }
}

#[tauri::command]
pub async fn speak_text(
    state: tauri::State<'_, SpeechController>,
    text: String,
    voice: Option<String>,
) -> Result<SpeechStarted, String> {
    let controller = state.inner().clone();
    // Bilet, ses sorgusu ve dosya hazırlığından önce alınır; bu sırada gelen
    // stop/yeni istek bu başlatmayı iptal eder.
    let ticket = controller.reserve();
    tauri::async_runtime::spawn_blocking(move || {
        validate_text(&text)?;
        let Some(platform) = current_platform() else {
            return Err("Metin okuma platformunuzda desteklenmiyor.".to_string());
        };
        let requested = voice.as_deref().unwrap_or(DEFAULT_MACOS_VOICE);
        validate_voice(requested)?;
        let (voice, voice_fallback) = controller.resolve_voice(platform, requested);
        let invocation = prepare_speech(platform, &text, voice.as_deref())?;
        let generation = controller.start(invocation, ticket)?;
        Ok(SpeechStarted {
            generation,
            voice,
            voice_fallback,
            cancelled: generation.is_none(),
        })
    })
    .await
    .map_err(|e| format!("Seslendirme görevi başarısız: {}", e))?
}

#[tauri::command]
pub async fn stop_speaking(state: tauri::State<'_, SpeechController>) -> Result<(), String> {
    let controller = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || controller.stop())
        .await
        .map_err(|e| format!("Seslendirme durdurma görevi başarısız: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTILE_TEXTS: &[&str] = &[
        "it's a 'quoted' text",
        "‘left’ ’right‘ ‚low‛ ‛reversed‚",
        "$(Remove-Item -Recurse C:\\) $(rm -rf ~)",
        "`whoami` `Stop-Computer`",
        "line one\r\nline two\nline three\r'; Stop-Process -Name explorer; '",
        "-o /tmp/pwned.aiff hello",
        "--input-file=/etc/passwd",
        "\"; calc.exe; \"",
    ];

    fn all_strings(invocation: &SpeechInvocation) -> Vec<String> {
        std::iter::once(&invocation.program)
            .chain(invocation.args.iter())
            .chain(invocation.envs.iter().flat_map(|(k, v)| [k, v]))
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    fn decoded_windows_script(args: &[OsString]) -> String {
        let position = args
            .iter()
            .position(|a| a == "-EncodedCommand")
            .expect("-EncodedCommand eksik");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(args[position + 1].to_str().unwrap())
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    fn assert_text_isolated(platform: SpeechPlatform, text: &str) {
        let invocation = prepare_speech(platform, text, Some("Yelda")).unwrap();
        for value in all_strings(&invocation) {
            assert!(
                !value.contains(text),
                "kullanıcı metni komut/argümana karıştı: {value:?}"
            );
        }
        if platform == SpeechPlatform::Windows {
            let script = decoded_windows_script(&invocation.args);
            assert_eq!(script, WINDOWS_SPEECH_SCRIPT);
            assert!(!script.contains(text));
        }
        let written = std::fs::read(&invocation.text_file).unwrap();
        assert_eq!(written, text.as_bytes(), "dosya içeriği birebir olmalı");
    }

    #[test]
    fn hostile_texts_never_reach_command_line_or_script() {
        for platform in [SpeechPlatform::MacOs, SpeechPlatform::Windows] {
            for text in HOSTILE_TEXTS {
                assert_text_isolated(platform, text);
            }
        }
    }

    #[test]
    fn hundred_thousand_char_text_goes_to_file_only() {
        let text: String = "‘a’ $(b) `c`\n".chars().cycle().take(100_000).collect();
        assert_eq!(text.chars().count(), 100_000);
        for platform in [SpeechPlatform::MacOs, SpeechPlatform::Windows] {
            assert_text_isolated(platform, &text);
            let invocation = prepare_speech(platform, &text, None).unwrap();
            let total: usize = all_strings(&invocation).iter().map(String::len).sum();
            assert!(
                total < 4_096,
                "komut satırı metin boyutuyla büyümemeli: {total}"
            );
        }
    }

    #[test]
    fn windows_script_is_constant_and_runs_hidden_noninteractive() {
        let args = windows_powershell_args();
        for flag in ["-NoProfile", "-NonInteractive", "-EncodedCommand"] {
            assert!(args.iter().any(|a| a == flag), "{flag} eksik");
        }
        assert_eq!(decoded_windows_script(&args), WINDOWS_SPEECH_SCRIPT);
        assert!(WINDOWS_SPEECH_SCRIPT.contains(
            "Get-Content -LiteralPath $env:ANTIGRAVITY_TTS_TEXT_PATH -Raw -Encoding UTF8"
        ));
        assert!(WINDOWS_SPEECH_SCRIPT.contains(WINDOWS_TEXT_PATH_ENV));
    }

    fn arg_strings(invocation: &SpeechInvocation) -> Vec<String> {
        invocation
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn macos_args_pass_text_via_file_not_argument() {
        let invocation =
            prepare_speech(SpeechPlatform::MacOs, "-v Evil hello", Some("Yelda")).unwrap();
        let args = arg_strings(&invocation);
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "-v");
        assert_eq!(args[1], "Yelda");
        assert_eq!(args[2], "-f");
        assert_eq!(Path::new(&args[3]), invocation.text_file.as_ref() as &Path);
        assert!(Path::new(&args[3]).is_absolute());
    }

    #[test]
    fn macos_args_without_voice_use_system_default() {
        let invocation = prepare_speech(SpeechPlatform::MacOs, "-v Evil hello", None).unwrap();
        let args = arg_strings(&invocation);
        assert_eq!(args.len(), 2, "ses yoksa -v verilmemeli: {args:?}");
        assert_eq!(args[0], "-f");
        assert_eq!(Path::new(&args[1]), invocation.text_file.as_ref() as &Path);
    }

    const SAY_VOICE_LIST: &str = "\
Albert              en_US    # Hello! My name is Albert.
Eddy (Turkish (Türkiye)) tr_TR    # Merhaba, benim adım Eddy.
Bad News            en_US    # The light you see at the end of the tunnel is the headlamp.
Yelda               tr_TR    # Merhaba, benim adım Yelda.

garbage
";

    #[test]
    fn say_voice_list_is_parsed_into_names() {
        assert_eq!(
            parse_say_voice_list(SAY_VOICE_LIST),
            vec!["Albert", "Eddy (Turkish (Türkiye))", "Bad News", "Yelda"]
        );
        assert!(parse_say_voice_list("").is_empty());
    }

    #[test]
    fn voice_resolution_requires_installed_voice() {
        let installed = parse_say_voice_list(SAY_VOICE_LIST);
        assert_eq!(
            resolve_macos_voice("Yelda", &installed).as_deref(),
            Some("Yelda")
        );
        assert_eq!(
            resolve_macos_voice("yelda", &installed).as_deref(),
            Some("Yelda")
        );
        assert_eq!(
            resolve_macos_voice("eddy (turkish (türkiye))", &installed).as_deref(),
            Some("Eddy (Turkish (Türkiye))")
        );
        assert_eq!(resolve_macos_voice("Eddy", &installed), None);
        assert_eq!(resolve_macos_voice("Yelda", &[]), None);
    }

    fn voices(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn voice_cache_reuses_hits_and_refreshes_misses_with_rate_limit() {
        let mut cache = VoiceCache::default();
        let t0 = Instant::now();
        let mut calls = 0;

        // İlk çağrı sorgular; ses yok → sistem varsayılanı.
        let first = cache.resolve("Yelda", t0, || {
            calls += 1;
            voices(&["Albert"])
        });
        assert_eq!((first, calls), (None, 1));

        // Kısa süre içinde tekrar sorgulanmaz.
        let second = cache.resolve("Yelda", t0 + Duration::from_secs(5), || {
            calls += 1;
            voices(&["Albert", "Yelda"])
        });
        assert_eq!((second, calls), (None, 1));

        // Miss aralığı dolunca yeniden sorgulanır; yeni kurulan ses bulunur.
        let t1 = t0 + VOICE_LIST_MISS_REFRESH;
        let third = cache.resolve("Yelda", t1, || {
            calls += 1;
            voices(&["Albert", "Yelda"])
        });
        assert_eq!((third.as_deref(), calls), (Some("Yelda"), 2));

        // Bulunan ses cache'ten verilir.
        let fourth = cache.resolve("yelda", t1 + Duration::from_secs(60), || {
            calls += 1;
            None
        });
        assert_eq!((fourth.as_deref(), calls), (Some("Yelda"), 2));

        // Liste çok eskiyse isabet olsa da yenilenir; kaldırılan ses fark edilir.
        let fifth = cache.resolve("Yelda", t1 + VOICE_LIST_MAX_AGE, || {
            calls += 1;
            voices(&["Albert"])
        });
        assert_eq!((fifth, calls), (None, 3));
    }

    #[test]
    fn voice_cache_retries_failed_queries_and_keeps_last_good_list() {
        let mut cache = VoiceCache::default();
        let t0 = Instant::now();
        assert_eq!(cache.resolve("Yelda", t0, || None), None);
        // Başarısız sorgu saklanmaz; hemen sonraki çağrı yeniden dener.
        assert_eq!(
            cache.resolve("Yelda", t0, || voices(&["Yelda"])).as_deref(),
            Some("Yelda")
        );
        // Yenileme başarısız olursa eldeki liste kullanılır.
        assert_eq!(
            cache
                .resolve("Yelda", t0 + VOICE_LIST_MAX_AGE, || None)
                .as_deref(),
            Some("Yelda")
        );
    }

    #[test]
    fn stderr_summary_is_bounded_single_line() {
        assert_eq!(summarize_stderr(b""), None);
        assert_eq!(summarize_stderr(b"\n  \n"), None);
        assert_eq!(
            summarize_stderr(b"\n Voice `X' not found\x1b[0m \nsecond").as_deref(),
            Some("Voice `X' not found[0m")
        );
        let long = vec![b'e'; 10_000];
        assert_eq!(
            summarize_stderr(&long).unwrap().chars().count(),
            MAX_STDERR_MESSAGE_CHARS
        );
    }

    #[test]
    fn voice_validation_rejects_bad_values() {
        let too_long = "a".repeat(MAX_VOICE_CHARS + 1);
        for voice in [
            "-x",
            "--voice=x",
            "",
            "   ",
            " Yelda",
            too_long.as_str(),
            "Yelda;rm",
            "Yelda\n",
            "a$(b)",
            "x`y`",
            "Ye'lda",
            "Ye‘lda",
        ] {
            assert!(
                validate_voice(voice).is_err(),
                "kabul edilmemeli: {voice:?}"
            );
            assert!(prepare_speech(SpeechPlatform::MacOs, "merhaba", Some(voice)).is_err());
        }
        for voice in ["Yelda", "Eddy (Turkish (Türkiye))", "Bad News", "en_US.x-1"] {
            assert!(validate_voice(voice).is_ok(), "kabul edilmeli: {voice:?}");
        }
        assert!(validate_voice(&"a".repeat(MAX_VOICE_CHARS)).is_ok());
    }

    #[test]
    fn text_validation_rejects_empty_and_oversized() {
        assert!(validate_text("").is_err());
        assert!(validate_text(" \n\t").is_err());
        assert!(validate_text(&"x".repeat(MAX_SPEECH_TEXT_CHARS + 1)).is_err());
        assert!(validate_text(&"x".repeat(MAX_SPEECH_TEXT_CHARS)).is_ok());
    }

    #[cfg(unix)]
    fn sleeper_invocation(program: &str, args: &[&str]) -> (SpeechInvocation, std::path::PathBuf) {
        let text_file = write_text_file("merhaba").unwrap();
        let path = text_file.to_path_buf();
        (
            SpeechInvocation {
                program: program.into(),
                args: args.iter().map(OsString::from).collect(),
                envs: Vec::new(),
                text_file,
            },
            path,
        )
    }

    #[cfg(unix)]
    fn pid_alive(pid: u32) -> bool {
        // SAFETY: sinyal 0 yalnız varlık kontrolü yapar.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(unix)]
    fn start_now(controller: &SpeechController, invocation: SpeechInvocation) -> u64 {
        let ticket = controller.reserve();
        controller
            .start(invocation, ticket)
            .unwrap()
            .expect("güncel bilet süreç başlatmalı")
    }

    #[cfg(unix)]
    #[test]
    fn stop_cancels_a_start_that_is_still_being_prepared() {
        let (controller, events) = recording_controller();
        let ticket = controller.reserve();
        // speak_text ses/dosya hazırlarken kullanıcı durdurur.
        controller.stop();
        let (invocation, path) = sleeper_invocation("sleep", &["30"]);
        assert_eq!(controller.start(invocation, ticket).unwrap(), None);
        assert!(controller.current_pid().is_none(), "süreç başlatılmamalı");
        assert!(
            !path.exists(),
            "iptal edilen isteğin geçici dosyası silinmeli"
        );
        std::thread::sleep(REAPER_SETTLE);
        assert!(events.lock().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn newer_request_wins_even_if_older_start_finishes_preparing_later() {
        let (controller, events) = recording_controller();
        let older = controller.reserve();
        let newer = controller.reserve();

        let (newer_invocation, _) = sleeper_invocation("true", &[]);
        let newer_generation = controller.start(newer_invocation, newer).unwrap().unwrap();
        let (older_invocation, older_path) = sleeper_invocation("sleep", &["30"]);
        assert_eq!(controller.start(older_invocation, older).unwrap(), None);
        assert!(!older_path.exists());

        let recorded = wait_for_events(&events, 1);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].generation, newer_generation);
        assert_eq!(recorded[0].status, SpeechOutcome::Completed);
        assert!(
            controller.current_pid().is_none(),
            "eski istek süreç açmamalı"
        );
    }

    #[cfg(unix)]
    type RecordedEvents = Arc<Mutex<Vec<SpeechFinished>>>;

    #[cfg(unix)]
    fn recording_controller() -> (SpeechController, RecordedEvents) {
        let events: RecordedEvents = Arc::default();
        let sink = Arc::clone(&events);
        (
            SpeechController::new(move |event| sink.lock().push(event)),
            events,
        )
    }

    #[cfg(unix)]
    fn wait_for_events(events: &RecordedEvents, count: usize) -> Vec<SpeechFinished> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while events.lock().len() < count && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        events.lock().clone()
    }

    /// Reaper'ların en az iki tur yoklama yapmasına yetecek süre.
    #[cfg(unix)]
    const REAPER_SETTLE: Duration = Duration::from_millis(700);

    #[cfg(unix)]
    #[test]
    fn natural_finish_emits_completed_with_generation() {
        let (controller, events) = recording_controller();
        let (invocation, path) = sleeper_invocation("true", &[]);
        let generation = start_now(&controller, invocation);
        let recorded = wait_for_events(&events, 1);
        assert_eq!(
            recorded,
            vec![SpeechFinished {
                generation,
                status: SpeechOutcome::Completed,
                message: None,
            }]
        );
        assert!(!path.exists());
        assert!(controller.current_pid().is_none());
        std::thread::sleep(REAPER_SETTLE);
        assert_eq!(events.lock().len(), 1, "tek olay yayınlanmalı");
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_exit_emits_failed_with_generation() {
        let (controller, events) = recording_controller();
        let (invocation, path) = sleeper_invocation("false", &[]);
        let generation = start_now(&controller, invocation);
        let recorded = wait_for_events(&events, 1);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].generation, generation);
        assert_eq!(recorded[0].status, SpeechOutcome::Failed);
        assert_eq!(
            recorded[0].message.as_deref(),
            Some("Seslendirme süreci hata koduyla sonlandı (1)")
        );
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn failure_message_includes_bounded_stderr() {
        let (controller, events) = recording_controller();
        let (invocation, _path) =
            sleeper_invocation("sh", &["-c", "echo \"Voice 'Nope' not found\" >&2; exit 3"]);
        let generation = start_now(&controller, invocation);
        let recorded = wait_for_events(&events, 1);
        assert_eq!(recorded[0].generation, generation);
        assert_eq!(recorded[0].status, SpeechOutcome::Failed);
        assert_eq!(
            recorded[0].message.as_deref(),
            Some("Seslendirme süreci hata koduyla sonlandı (3): Voice 'Nope' not found")
        );

        // Sınırı aşan stderr süreci bloklamamalı; mesaj kısa kalmalı.
        let (invocation, _path) = sleeper_invocation(
            "sh",
            &["-c", "head -c 200000 /dev/zero | tr '\\0' 'x' >&2; exit 2"],
        );
        let generation = start_now(&controller, invocation);
        let recorded = wait_for_events(&events, 2);
        assert_eq!(recorded.len(), 2, "büyük stderr süreci bloklamamalı");
        assert_eq!(recorded[1].generation, generation);
        let message = recorded[1].message.as_deref().unwrap();
        assert!(message.starts_with("Seslendirme süreci hata koduyla sonlandı (2): xxx"));
        assert!(message.chars().count() < MAX_STDERR_MESSAGE_CHARS + 64);
    }

    #[cfg(unix)]
    #[test]
    fn stopped_or_replaced_process_emits_no_event() {
        let (controller, events) = recording_controller();

        let (stopped, _) = sleeper_invocation("sleep", &["30"]);
        let stopped_generation = start_now(&controller, stopped);
        controller.stop();

        let (replaced, _) = sleeper_invocation("sleep", &["30"]);
        let replaced_generation = start_now(&controller, replaced);
        let (current, _) = sleeper_invocation("true", &[]);
        let current_generation = start_now(&controller, current);

        assert!(stopped_generation < replaced_generation);
        assert!(replaced_generation < current_generation);
        let recorded = wait_for_events(&events, 1);
        std::thread::sleep(REAPER_SETTLE);
        let recorded_after = events.lock().clone();
        assert_eq!(recorded, recorded_after, "ek olay gelmemeli");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].generation, current_generation);
        assert_eq!(recorded[0].status, SpeechOutcome::Completed);
    }

    #[cfg(unix)]
    #[test]
    fn stop_after_natural_exit_before_reap_emits_no_event() {
        let (controller, events) = recording_controller();
        let (invocation, path) = sleeper_invocation("true", &[]);
        start_now(&controller, invocation);
        // Reaper ilk yoklamasını yapmadan durdurulur.
        controller.stop();
        std::thread::sleep(REAPER_SETTLE);
        assert!(events.lock().is_empty());
        assert!(!path.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn real_say_voice_list_is_parseable() {
        let voices = list_installed_macos_voices().expect("say -v '?' çalışmalı");
        assert!(!voices.is_empty());
        assert!(voices
            .iter()
            .all(|name| !name.contains('#') && name.trim() == name && !name.is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn stop_kills_only_own_process_and_removes_temp_file() {
        let mut bystander = Command::new("sleep").arg("30").spawn().unwrap();
        let (controller, _events) = recording_controller();

        let (first, first_path) = sleeper_invocation("sleep", &["30"]);
        start_now(&controller, first);
        let first_pid = controller.current_pid().unwrap();

        let (second, second_path) = sleeper_invocation("sleep", &["30"]);
        start_now(&controller, second);
        let second_pid = controller.current_pid().unwrap();
        assert!(!pid_alive(first_pid), "önceki süreç kill+wait edilmeli");
        assert!(!first_path.exists(), "önceki geçici dosya silinmeli");
        assert!(pid_alive(second_pid));

        controller.stop();
        assert!(!pid_alive(second_pid));
        assert!(!second_path.exists());
        assert!(controller.current_pid().is_none());
        assert!(pid_alive(bystander.id()), "yabancı süreç etkilenmemeli");

        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[cfg(unix)]
    #[test]
    fn finished_process_is_reaped_and_temp_file_removed() {
        let (controller, _events) = recording_controller();
        let (invocation, path) = sleeper_invocation("true", &[]);
        start_now(&controller, invocation);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while (path.exists() || controller.current_pid().is_some())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!path.exists(), "süreç bitince geçici dosya silinmeli");
        assert!(controller.current_pid().is_none());
    }
}
