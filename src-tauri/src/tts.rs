//! Yerel metin okuma (Text-to-Speech).
//!
//! Güvenlik sınırları:
//! - Kullanıcı metni hiçbir kabuk/komut satırına gömülmez; 0600 izinli UTF-8
//!   geçici dosyaya yazılır ve TTS süreci metni bu dosyadan okur.
//! - Windows'ta PowerShell betiği sabittir; dosya yolu ortam değişkeniyle verilir.
//! - Yalnız bu modülün başlattığı süreç izlenir ve sonlandırılır; sistemdeki
//!   diğer `say` / `powershell` süreçlerine dokunulmaz.
//!
//! Komut/argüman üretimi cfg'siz saf fonksiyonlardadır; testler her platformda çalışır.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use parking_lot::Mutex;
use tempfile::TempPath;

pub const DEFAULT_MACOS_VOICE: &str = "Yelda";
pub const MAX_SPEECH_TEXT_CHARS: usize = 1_000_000;
pub const MAX_VOICE_CHARS: usize = 64;
pub const WINDOWS_TEXT_PATH_ENV: &str = "ANTIGRAVITY_TTS_TEXT_PATH";

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
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

/// `say -v <voice> -f <file>`: metin argüman olarak verilmez, bu yüzden
/// "-" ile başlayan metin seçenek olarak yorumlanamaz.
pub fn macos_say_args(voice: &str, text_path: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-v"),
        OsString::from(voice),
        OsString::from("-f"),
        text_path.as_os_str().to_owned(),
    ]
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
            args: macos_say_args(voice.unwrap_or(DEFAULT_MACOS_VOICE), &text_file),
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

struct SpeechProcess {
    child: Child,
    // Süreçle birlikte düşürülür; drop geçici dosyayı siler.
    _text_file: TempPath,
}

impl SpeechProcess {
    fn terminate(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct SpeechSlot {
    generation: u64,
    current: Option<SpeechProcess>,
}

/// Uygulamanın başlattığı tek TTS sürecini tutan Tauri state'i.
#[derive(Clone, Default)]
pub struct SpeechController {
    slot: Arc<Mutex<SpeechSlot>>,
}

impl SpeechController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mevcut süreci kill+wait ile sonlandırır, ardından yenisini başlatır.
    /// Kilit spawn boyunca tutulur; eşzamanlı çağrılarda tek süreç kalır.
    pub fn start(&self, invocation: SpeechInvocation) -> Result<(), String> {
        let mut slot = self.slot.lock();
        if let Some(previous) = slot.current.take() {
            previous.terminate();
        }

        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .envs(invocation.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let child = command
            .spawn()
            .map_err(|e| format!("Seslendirme başlatılamadı: {}", e))?;
        slot.generation = slot.generation.wrapping_add(1);
        let generation = slot.generation;
        slot.current = Some(SpeechProcess {
            child,
            _text_file: invocation.text_file,
        });
        drop(slot);

        self.spawn_reaper(generation);
        Ok(())
    }

    /// Yalnız bu controller'ın başlattığı süreci sonlandırır.
    pub fn stop(&self) {
        let process = self.slot.lock().current.take();
        if let Some(process) = process {
            process.terminate();
        }
    }

    #[cfg(test)]
    fn current_pid(&self) -> Option<u32> {
        self.slot.lock().current.as_ref().map(|p| p.child.id())
    }

    /// Süreç kendiliğinden bitince onu toplar ve geçici dosyayı siler.
    fn spawn_reaper(&self, generation: u64) {
        let slot = Arc::clone(&self.slot);
        std::thread::spawn(move || loop {
            std::thread::sleep(PROCESS_POLL_INTERVAL);
            let mut guard = slot.lock();
            if guard.generation != generation {
                return;
            }
            let finished = match guard.current.as_mut() {
                None => true,
                Some(process) => !matches!(process.child.try_wait(), Ok(None)),
            };
            if finished {
                // try_wait süreci topladı (veya hata verdi); kill+wait zararsızdır.
                if let Some(process) = guard.current.take() {
                    process.terminate();
                }
                return;
            }
        });
    }
}

#[tauri::command]
pub async fn speak_text(
    state: tauri::State<'_, SpeechController>,
    text: String,
    voice: Option<String>,
) -> Result<(), String> {
    let controller = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_text(&text)?;
        let Some(platform) = current_platform() else {
            return Err("Metin okuma platformunuzda desteklenmiyor.".to_string());
        };
        let invocation = prepare_speech(platform, &text, voice.as_deref())?;
        controller.start(invocation)
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

    #[test]
    fn macos_args_pass_text_via_file_not_argument() {
        let invocation = prepare_speech(SpeechPlatform::MacOs, "-v Evil hello", None).unwrap();
        let args: Vec<String> = invocation
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "-v");
        assert_eq!(args[1], DEFAULT_MACOS_VOICE);
        assert_eq!(args[2], "-f");
        assert_eq!(Path::new(&args[3]), invocation.text_file.as_ref() as &Path);
        assert!(Path::new(&args[3]).is_absolute());
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
    #[test]
    fn stop_kills_only_own_process_and_removes_temp_file() {
        let mut bystander = Command::new("sleep").arg("30").spawn().unwrap();
        let controller = SpeechController::new();

        let (first, first_path) = sleeper_invocation("sleep", &["30"]);
        controller.start(first).unwrap();
        let first_pid = controller.current_pid().unwrap();

        let (second, second_path) = sleeper_invocation("sleep", &["30"]);
        controller.start(second).unwrap();
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
        let controller = SpeechController::new();
        let (invocation, path) = sleeper_invocation("true", &[]);
        controller.start(invocation).unwrap();
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
