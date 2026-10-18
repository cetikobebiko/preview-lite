//! Derlenmiş macOS yardımcısı (`macos-helper/main.swift`) ile köprü.
//!
//! Yardımcı build.rs'te swiftc ile derlenir ve Tauri externalBin olarak
//! uygulama binary'sinin yanına (`Contents/MacOS/`, geliştirmede
//! `target/<profil>/`) kopyalanır; çalışma zamanında `swift` yorumlayıcısı
//! gerekmez. Dosya yolları ve değerler yalnız ayrı süreç argümanı olarak
//! geçirilir, hiçbir zaman kaynak koda gömülmez.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// build.rs `MACOS_HELPER_NAME` ve `tauri.macos.conf.json` externalBin ile aynı.
pub(crate) const HELPER_NAME: &str = "antigravity-macos-helper";

/// Servis seçildikten sonra delegeden yanıt gelmezse paylaşım sürecinin
/// dosyayı canlı tutacağı üst süre. AirDrop aktarımı ve Mail/Mesajlar
/// oluşturma penceresi kullanıcı etkileşimi içerdiğinden kısa tutulmaz.
pub(crate) const SHARE_SERVICE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Picker'da servis seçimi için kullanıcıya bırakılan süre.
pub(crate) const SHARE_PICKER_IDLE_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Rust tarafındaki kesin üst sınır: picker bekleme + servis süresi + pay.
/// Yardımcıdaki servis üst süresi her zaman bundan önce dolar.
pub(crate) const SHARE_PROCESS_TIMEOUT: Duration =
    Duration::from_secs(SHARE_PICKER_IDLE_TIMEOUT.as_secs() + SHARE_SERVICE_TIMEOUT.as_secs() + 30);

const SHARE_DEFAULT_STEM: &str = "document";
const SHARE_PDF_EXTENSION: &str = ".pdf";
/// APFS dosya adı sınırı 255 UTF-8 bayttır; pay bırakılır.
pub(crate) const SHARE_FILE_NAME_MAX_BYTES: usize = 200;

/// Yardımcıyı uygulama binary'sinin dizininde arar (PATH kullanılmaz). Test
/// binary'leri `target/<profil>/deps/` altında çalıştığından bir üst dizine de
/// bakılır; job worker çözümlemesiyle aynı kural.
pub(crate) fn helper_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("macOS yardımcı aracı çözümlenemedi: {error}"))?;
    let directory = current
        .parent()
        .ok_or("macOS yardımcı aracı dizini çözümlenemedi")?;
    let mut candidates = vec![directory.join(HELPER_NAME)];
    if directory.file_name().is_some_and(|name| name == "deps") {
        if let Some(parent) = directory.parent() {
            candidates.push(parent.join(HELPER_NAME));
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "macOS yardımcı aracı bulunamadı: {}",
                directory.join(HELPER_NAME).display()
            )
        })
}

/// Verilen argümanlarla yardımcıyı çalıştıracak komutu kurar.
pub(crate) fn helper_command(args: Vec<OsString>) -> Result<std::process::Command, String> {
    let mut command = std::process::Command::new(helper_executable()?);
    command.args(args).stdin(std::process::Stdio::null());
    Ok(command)
}

fn absolute_path_argument(path: &Path) -> Result<OsString, String> {
    // Göreli bir yol `-` ile başlayıp seçenek gibi yorumlanabilir.
    if !path.is_absolute() {
        return Err(format!(
            "macOS yardımcısına yalnız mutlak yol geçirilebilir: {}",
            path.display()
        ));
    }
    Ok(path.as_os_str().to_os_string())
}

/// Vision OCR argümanları: `ocr <görsel yolu>`.
pub(crate) fn ocr_command_args(image_path: &Path) -> Result<Vec<OsString>, String> {
    Ok(vec![
        OsString::from("ocr"),
        absolute_path_argument(image_path)?,
    ])
}

/// Paylaşım picker'ının bağlanacağı pencere içerik alanı. Koordinatlar
/// birincil ekranın sol üst köşesine göre global mantıksal noktalardır
/// (AppKit point; tao'nun pencere konumu ile aynı orijin). Yardımcı bunu
/// AppKit'in sol alt orijinine çevirir.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShareAnchor {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl ShareAnchor {
    /// Tauri'nin fiziksel piksel içerik konumu/boyutundan mantıksal dikdörtgen
    /// üretir. Boş alan veya geçersiz ölçek için `None` döner; yardımcı bu
    /// durumda picker'ı ekranın ortasında açar.
    pub(crate) fn from_physical(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> Option<Self> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 || width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            x: f64::from(x) / scale_factor,
            y: f64::from(y) / scale_factor,
            width: f64::from(width) / scale_factor,
            height: f64::from(height) / scale_factor,
        })
    }
}

fn coordinate_argument(value: f64) -> OsString {
    OsString::from(format!("{value:.2}"))
}

/// Paylaşım argümanları:
/// `share <dosya yolu> <servis üst süresi sn> [<x> <y> <genişlik> <yükseklik>]`.
pub(crate) fn share_command_args(
    file_path: &Path,
    anchor: Option<ShareAnchor>,
) -> Result<Vec<OsString>, String> {
    let mut args = vec![
        OsString::from("share"),
        absolute_path_argument(file_path)?,
        OsString::from(SHARE_SERVICE_TIMEOUT.as_secs().to_string()),
    ];
    if let Some(anchor) = anchor {
        args.extend([anchor.x, anchor.y, anchor.width, anchor.height].map(coordinate_argument));
    }
    Ok(args)
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

fn truncate_to_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Oturumun görüntü yolundan paylaşım için güvenli bir dosya adı üretir.
///
/// Son yol bileşeni alınır; kontrol ve bidi yön karakterleri atılır, `\` ve
/// `:` `_` ile değiştirilir, baştaki noktalar (gizli dosya / `..`) ve uçtaki
/// boşluklar kırpılır, ad `.pdf` ile biter ve toplam uzunluk
/// [`SHARE_FILE_NAME_MAX_BYTES`] baytı aşmaz. Geriye bir şey kalmazsa
/// `document.pdf` kullanılır.
pub(crate) fn sanitize_share_file_name(display_path: Option<&str>) -> String {
    let last_component = display_path
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let cleaned: String = last_component
        .chars()
        .filter(|character| !character.is_control() && !is_bidi_control(*character))
        .map(|character| match character {
            '\\' | ':' => '_',
            character if character.is_whitespace() => ' ',
            character => character,
        })
        .collect();
    let cleaned = cleaned.trim();
    let has_pdf_extension = cleaned.len() >= SHARE_PDF_EXTENSION.len()
        && cleaned.is_char_boundary(cleaned.len() - SHARE_PDF_EXTENSION.len())
        && cleaned[cleaned.len() - SHARE_PDF_EXTENSION.len()..]
            .eq_ignore_ascii_case(SHARE_PDF_EXTENSION);
    let (stem, extension) = if has_pdf_extension {
        cleaned.split_at(cleaned.len() - SHARE_PDF_EXTENSION.len())
    } else {
        (cleaned, SHARE_PDF_EXTENSION)
    };
    let stem =
        stem.trim_start_matches(|character: char| character == '.' || character.is_whitespace());
    let stem = truncate_to_bytes(stem, SHARE_FILE_NAME_MAX_BYTES - extension.len())
        .trim_end_matches(|character: char| character == '.' || character.is_whitespace());
    let stem = if stem.is_empty() {
        SHARE_DEFAULT_STEM
    } else {
        stem
    };
    format!("{stem}{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::path::PathBuf;

    // Unix absolute paths: on Windows they are not absolute and the helper
    // (macOS-only in production) rejects them before building arguments.
    #[cfg(unix)]
    const HOSTILE_PATHS: &[&str] = &[
        "/tmp/a\"b/vision-ocr-input.png",
        "/tmp/back\\slash/vision-ocr-input.png",
        "/tmp/\\(exit(9))/vision-ocr-input.png",
        "/tmp/\")\nexit(9)\n//\"/vision-ocr-input.png",
        "/tmp/-e/--help/vision-ocr-input.png",
    ];

    #[cfg(unix)]
    #[test]
    fn ocr_args_pass_path_as_a_single_untouched_argument() {
        for hostile in HOSTILE_PATHS {
            let path = PathBuf::from(hostile);
            let args = ocr_command_args(&path).unwrap();
            assert_eq!(args, vec![OsString::from("ocr"), path.into_os_string()]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn share_args_pass_path_and_service_timeout_as_separate_arguments() {
        for hostile in HOSTILE_PATHS {
            let path = PathBuf::from(hostile);
            let args = share_command_args(&path, None).unwrap();
            assert_eq!(
                args,
                vec![
                    OsString::from("share"),
                    path.into_os_string(),
                    OsString::from("600")
                ]
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn share_args_append_window_anchor_as_four_coordinates() {
        let path = PathBuf::from(HOSTILE_PATHS[0]);
        let anchor = ShareAnchor {
            x: -1440.0,
            y: 25.5,
            width: 1280.0,
            height: 774.25,
        };
        let args = share_command_args(&path, Some(anchor)).unwrap();
        assert_eq!(
            args,
            [
                "share",
                HOSTILE_PATHS[0],
                "600",
                "-1440.00",
                "25.50",
                "1280.00",
                "774.25"
            ]
            .map(OsString::from)
            .to_vec()
        );
    }

    #[test]
    fn share_anchor_converts_physical_pixels_to_logical_points() {
        assert_eq!(
            ShareAnchor::from_physical(200, 106, 2560, 1548, 2.0),
            Some(ShareAnchor {
                x: 100.0,
                y: 53.0,
                width: 1280.0,
                height: 774.0
            })
        );
        // Birincil ekranın solundaki/üstündeki ekranlarda konum negatiftir.
        assert_eq!(
            ShareAnchor::from_physical(-1920, -300, 1920, 1080, 1.0),
            Some(ShareAnchor {
                x: -1920.0,
                y: -300.0,
                width: 1920.0,
                height: 1080.0
            })
        );
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                ShareAnchor::from_physical(0, 0, 100, 100, scale),
                None,
                "{scale}"
            );
        }
        assert_eq!(ShareAnchor::from_physical(0, 0, 0, 100, 1.0), None);
        assert_eq!(ShareAnchor::from_physical(0, 0, 100, 0, 1.0), None);
    }

    #[test]
    fn helper_args_reject_relative_paths() {
        assert!(ocr_command_args(Path::new("-e")).is_err());
        assert!(ocr_command_args(Path::new("relative/image.png")).is_err());
        assert!(share_command_args(Path::new("--help"), None).is_err());
    }

    #[test]
    fn share_process_timeout_outlasts_helper_service_timeout() {
        assert!(SHARE_PROCESS_TIMEOUT > SHARE_SERVICE_TIMEOUT + SHARE_PICKER_IDLE_TIMEOUT);
    }

    #[test]
    fn helper_name_matches_build_script_and_bundle_config() {
        let build_script = include_str!("../build.rs");
        assert!(build_script.contains(&format!(
            "const MACOS_HELPER_NAME: &str = \"{HELPER_NAME}\";"
        )));
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.macos.conf.json")).unwrap();
        assert_eq!(
            config["bundle"]["externalBin"],
            serde_json::json!([format!("binaries/{HELPER_NAME}")])
        );
    }

    #[test]
    fn helper_source_reads_values_only_from_process_arguments() {
        let source = include_str!("../macos-helper/main.swift");
        assert!(source.contains("let arguments = CommandLine.arguments"));
        assert!(source.contains("runOCR(imagePath: arguments[2])"));
        assert!(source.contains(
            "runShare(filePath: arguments[2], serviceTimeout: serviceTimeout, anchor: anchor)"
        ));
        assert!(source.contains("parseAnchor(Array(arguments[4...]))"));
        assert!(source.contains("try handler.perform([request])"));
        assert!(!source.contains("try?"));
        assert!(source.contains("delegateFor sharingService"));
        assert!(source.contains("didShareItems"));
        assert!(source.contains("didFailToShareItems"));
    }

    #[cfg(target_os = "macos")]
    fn run_helper(args: Vec<OsString>) -> std::process::Output {
        helper_command(args)
            .expect("build.rs yardımcıyı derleyip target dizinine kopyalamış olmalı")
            .output()
            .unwrap()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_helper_runs_ocr_through_hostile_directory_names() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("a\"b\\c\\(exit(9))");
        std::fs::create_dir_all(&directory).unwrap();
        let image_path = directory.join("vision-ocr-input.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]))
            .save(&image_path)
            .unwrap();

        let output = run_helper(ocr_command_args(&image_path).unwrap());
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        // Boş sayfa: başarı + boş çıktı ("metin tespit edilemedi" yolu).
        assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_helper_reports_failures_on_stderr_with_nonzero_exit() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.png");
        let output = run_helper(ocr_command_args(&missing).unwrap());
        assert_eq!(output.status.code(), Some(65));
        assert!(String::from_utf8_lossy(&output.stderr).contains("OCR görseli okunamadı"));

        // Paylaşım UI açılmadan önce dosya doğrulanır.
        let missing_pdf = root.path().join("missing.pdf");
        let anchor = ShareAnchor {
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        };
        for anchor in [None, Some(anchor)] {
            let output = run_helper(share_command_args(&missing_pdf, anchor).unwrap());
            assert_eq!(output.status.code(), Some(65), "{anchor:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("okunamadı"));
        }

        for args in [
            vec![],
            vec!["bogus"],
            vec!["ocr"],
            vec!["share", "/tmp/x.pdf"],
            vec!["share", "/tmp/x.pdf", "abc"],
            vec!["share", "/tmp/x.pdf", "600", "1", "2", "3"],
            vec!["share", "/tmp/x.pdf", "600", "1", "2", "3", "4", "5"],
            vec!["share", "/tmp/x.pdf", "600", "a", "2", "3", "4"],
            vec!["share", "/tmp/x.pdf", "600", "nan", "2", "3", "4"],
            vec!["share", "/tmp/x.pdf", "600", "1", "inf", "3", "4"],
            vec!["share", "/tmp/x.pdf", "600", "1", "2", "0", "4"],
            vec!["share", "/tmp/x.pdf", "600", "1", "2", "3", "-4"],
        ] {
            let output = run_helper(args.into_iter().map(OsString::from).collect());
            assert_eq!(output.status.code(), Some(64), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("Kullanım"));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn build_script_emits_per_arch_and_universal_helpers() {
        let binaries = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
        for (triple, expected_archs) in [
            ("aarch64-apple-darwin", vec!["arm64"]),
            ("x86_64-apple-darwin", vec!["x86_64"]),
            ("universal-apple-darwin", vec!["arm64", "x86_64"]),
        ] {
            let path = binaries.join(format!("{HELPER_NAME}-{triple}"));
            let output = std::process::Command::new("lipo")
                .arg("-archs")
                .arg(&path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}: {output:?}", path.display());
            let mut archs: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .map(str::to_string)
                .collect();
            archs.sort();
            assert_eq!(archs, expected_archs, "{}", path.display());
        }
    }

    #[test]
    fn sanitize_keeps_ordinary_names() {
        assert_eq!(
            sanitize_share_file_name(Some("/Volumes/Arşiv/Belgeler/Yıllık Rapor 2026.pdf")),
            "Yıllık Rapor 2026.pdf"
        );
        assert_eq!(sanitize_share_file_name(Some("Tarama.PDF")), "Tarama.PDF");
    }

    #[test]
    fn sanitize_uses_default_for_missing_or_empty_names() {
        for input in [
            None,
            Some(""),
            Some("/"),
            Some("   "),
            Some(".."),
            Some(".pdf"),
            Some("\u{7}"),
        ] {
            assert_eq!(sanitize_share_file_name(input), "document.pdf", "{input:?}");
        }
    }

    #[test]
    fn sanitize_replaces_separators_and_strips_hidden_prefix() {
        assert_eq!(sanitize_share_file_name(Some("a\\b:c.pdf")), "a_b_c.pdf");
        assert_eq!(
            sanitize_share_file_name(Some("dir/../..gizli.pdf")),
            "gizli.pdf"
        );
        assert_eq!(sanitize_share_file_name(Some("notlar")), "notlar.pdf");
        assert_eq!(sanitize_share_file_name(Some(". .rapor.pdf")), "rapor.pdf");
        assert_eq!(
            sanitize_share_file_name(Some("sözleşme. .pdf")),
            "sözleşme.pdf"
        );
    }

    #[test]
    fn sanitize_removes_control_and_bidi_characters() {
        assert_eq!(
            sanitize_share_file_name(Some("ra\u{0}p\nor\u{1b}\t.pdf")),
            "rapor.pdf"
        );
        assert_eq!(
            sanitize_share_file_name(Some("fatura\u{202E}fdp.exe")),
            "faturafdp.exe.pdf"
        );
        assert_eq!(
            sanitize_share_file_name(Some("a\u{2028}b\u{00A0}c.pdf")),
            "a b c.pdf"
        );
    }

    #[test]
    fn sanitize_truncates_on_char_boundary_and_keeps_extension() {
        let long = format!("{}.pdf", "ş".repeat(400));
        let name = sanitize_share_file_name(Some(&long));
        assert!(name.len() <= SHARE_FILE_NAME_MAX_BYTES, "{}", name.len());
        assert!(name.ends_with(".pdf"));
        assert!(name.trim_end_matches(".pdf").chars().all(|c| c == 'ş'));

        let ascii = "x".repeat(1000);
        let name = sanitize_share_file_name(Some(&ascii));
        assert_eq!(name.len(), SHARE_FILE_NAME_MAX_BYTES);
        assert!(name.ends_with(".pdf"));
    }

    #[test]
    fn sanitized_name_is_always_a_single_normal_component() {
        let inputs = [
            "../../etc/passwd",
            "a/b/c",
            "/",
            "..\\..\\x",
            ". . .",
            "\u{202E}",
            "con:\u{0}",
        ];
        for input in inputs {
            let name = sanitize_share_file_name(Some(input));
            let mut components = Path::new(&name).components();
            assert!(
                matches!(
                    (components.next(), components.next()),
                    (Some(std::path::Component::Normal(_)), None)
                ),
                "{input:?} -> {name:?}"
            );
            assert!(!name.starts_with('.'), "{input:?} -> {name:?}");
        }
    }
}
