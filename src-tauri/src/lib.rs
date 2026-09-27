use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tauri::Manager;

pub mod a11y;
pub mod ai;
pub mod annotations;
pub mod batch;
pub mod document_io;
pub mod document_session;
pub mod error;
pub mod export;
pub mod external_validation;
pub mod job_worker;
pub mod manipulation;
pub mod ocr;
pub mod pdf_engine;
pub mod plugins;
pub mod prepress;
pub mod redaction;
pub mod sanitization;
pub mod sanitization_inspection;
pub mod sanitization_publication;
pub mod sanitization_transform;
pub mod sanitization_verifier;
pub mod secure_redaction;
pub mod secure_redaction_inspection;
pub mod secure_redaction_raster;
pub mod secure_redaction_verifier;
pub mod worker;

use pdf_engine::render_page_rgba;
use pdf_engine::render_service::RenderService;
pub use pdf_engine::{clear_search_cache, get_cached_password, get_pdfium, open_pdfium_doc};

#[derive(Serialize)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct PdfMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchMatch {
    pub global_match_index: usize,
    pub page_index: u32,
    pub rects: Vec<BoundingBox>,
    pub snippet: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub is_cancelled: bool,
    pub total_matches: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OutlineNode {
    pub title: String,
    pub page_index: u32,
    pub children: Vec<OutlineNode>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TextLayoutData {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: String,
}

#[derive(Deserialize, Debug)]
pub struct AnnotationPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Deserialize, Debug)]
pub struct AnnotationItem {
    pub page_index: u32,
    pub tool_type: String,
    pub points: Option<Vec<AnnotationPoint>>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub w: Option<f32>,
    pub h: Option<f32>,
    pub end_x: Option<f32>,
    pub end_y: Option<f32>,
    pub text: Option<String>,
    pub color: String,
    pub size: f32,
    pub opacity: Option<f32>,
    pub canvas_width: f32,
    pub canvas_height: f32,
}

pub struct PasswordCache(pub std::sync::Arc<parking_lot::Mutex<HashMap<String, String>>>);
pub struct SearchCache(pub parking_lot::Mutex<lru::LruCache<String, SearchResult>>);
pub struct ActiveSearchId(pub std::sync::Arc<std::sync::atomic::AtomicU64>);
pub struct ActiveRenderTasks(
    pub parking_lot::Mutex<HashMap<String, tokio_util::sync::CancellationToken>>,
);
pub struct AppShutdownToken(pub tokio_util::sync::CancellationToken);
pub struct AppShutdownCoordinator(pub std::sync::atomic::AtomicBool);
pub struct TextLayoutCache(pub parking_lot::Mutex<lru::LruCache<String, Vec<TextLayoutData>>>);

// Görsele ve SVG'ye Dönüştürme (Export Pages to PNG, JPEG, SVG)
#[tauri::command]
#[allow(dead_code)]
async fn export_pdf_page_to_image(
    path: String,
    page_index: u32,
    target_path: String,
    format: String,
) -> Result<(), String> {
    export_pdf_page_to_image_with_password(path, page_index, target_path, format, None).await
}

pub(crate) async fn export_pdf_page_to_image_with_password(
    path: String,
    page_index: u32,
    target_path: String,
    format: String,
    password: Option<String>,
) -> Result<(), String> {
    let rendered = render_page_rgba(path, page_index, 2.0, 0, password).await?;
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        rendered.width,
        rendered.height,
        rendered.data,
    )
    .ok_or("Görsel tamponu oluşturulamadı")?;

    let fmt = format.to_lowercase();
    if fmt == "svg" {
        let mut png_bytes = Vec::new();
        let dynamic_img = image::DynamicImage::ImageRgba8(img);
        dynamic_img
            .write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("PNG kodlama hatası: {}", e))?;

        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let b64 = STANDARD.encode(&png_bytes);
        let svg_content = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}">
  <image width="{}" height="{}" href="data:image/png;base64,{}"/>
</svg>"#,
            rendered.width, rendered.height, rendered.width, rendered.height, b64
        );
        std::fs::write(&target_path, svg_content)
            .map_err(|e| format!("SVG dosyası yazılamadı: {}", e))?;
    } else {
        let image_format = match fmt.as_str() {
            "png" => image::ImageFormat::Png,
            "jpg" | "jpeg" => image::ImageFormat::Jpeg,
            "webp" => image::ImageFormat::WebP,
            "bmp" => image::ImageFormat::Bmp,
            _ => image::ImageFormat::Png,
        };

        img.save_with_format(&target_path, image_format)
            .map_err(|e| format!("Görsel kaydetme hatası: {}", e))?;
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Cross-Platform Native API Adaptations (macOS vs Windows/Linux Fallbacks)
// -----------------------------------------------------------------------------

// 1. Süreklilik Kamerası (Continuity Camera / iPhone Camera Document Scan)
#[cfg(target_os = "macos")]
#[tauri::command]
fn trigger_continuity_camera_scan(_source_pdf_path: String) -> Result<String, String> {
    Err("FEATURE_UNAVAILABLE: Continuity Camera özelliği aktif açık belgeler için desteklenmemektedir".to_string())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn trigger_continuity_camera_scan(_source_pdf_path: String) -> Result<String, String> {
    Err("FEATURE_UNAVAILABLE: iPhone Süreklilik Kamerası sadece macOS platformunda desteklenmektedir.".to_string())
}

// 2. Yerel Metin Okuma (Text-to-Speech)
#[cfg(target_os = "macos")]
#[tauri::command]
fn speak_text(text: String, voice: Option<String>) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Okunacak metin yok".to_string());
    }

    let selected_voice = voice.unwrap_or_else(|| "Yelda".to_string());

    let _ = stop_speaking();

    std::process::Command::new("say")
        .arg("-v")
        .arg(selected_voice)
        .arg(&text)
        .spawn()
        .map_err(|e| format!("Seslendirme başlatılamadı: {}", e))?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn speak_text(text: String, _voice: Option<String>) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Okunacak metin yok".to_string());
    }
    // Windows / Linux Fallback using PowerShell SAPI
    #[cfg(target_os = "windows")]
    {
        let ps_cmd = format!("Add-Type -AssemblyName System.Speech; $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; $synth.Speak('{}')", text.replace('\'', "''"));
        let _ = std::process::Command::new("powershell")
            .arg("-Command")
            .arg(ps_cmd)
            .spawn();
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    Err("Metin okuma platformunuzda desteklenmiyor.".to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn stop_speaking() -> Result<(), String> {
    let _ = std::process::Command::new("killall").arg("say").output();
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn stop_speaking() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .arg("/F")
            .arg("/IM")
            .arg("powershell.exe")
            .output();
    }
    Ok(())
}

// 3. macOS Paylaşım Menüsü (Share Sheet)
#[cfg(target_os = "macos")]
fn run_child_with_timeout(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::ExitStatus, String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("Platform süreci başlatılamadı: {error}"))?;
    let deadline = std::time::Instant::now() + timeout;

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Platform süreci izlenemedi: {error}"))?
        {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Platform süreci zaman aşımına uğradı".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(target_os = "macos")]
fn macos_share_command(path: &str) -> Result<std::process::Command, String> {
    if !Path::new(path).is_file() {
        return Err(format!("Paylaşılacak dosya bulunamadı: {path}"));
    }

    let script = r#"
        import AppKit

        class PickerDelegate: NSObject, NSSharingServicePickerDelegate {
            func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker, didChoose service: NSSharingService?) {
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
                    exit(0)
                }
            }
        }

        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let fileURL = URL(fileURLWithPath: CommandLine.arguments[1])
        let picker = NSSharingServicePicker(items: [fileURL])
        let delegate = PickerDelegate()
        picker.delegate = delegate
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1, height: 1),
            styleMask: .borderless,
            backing: .buffered,
            defer: false
        )
        window.makeKeyAndOrderFront(nil)
        app.activate(ignoringOtherApps: true)
        picker.show(relativeTo: .zero, of: window.contentView!, preferredEdge: .minY)
        app.run()
    "#;
    let mut command = std::process::Command::new("swift");
    command.arg("-e").arg(script).arg(path);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn share_external_path_macos(path: &str) -> Result<(), String> {
    let status = run_child_with_timeout(
        &mut macos_share_command(path)?,
        std::time::Duration::from_secs(120),
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("macOS Paylaşım menüsü {status} durumuyla kapandı"))
    }
}

#[cfg(target_os = "macos")]
async fn share_session_snapshot_with<F>(
    store: &document_session::DocumentSessionStore,
    job_id: &str,
    session_id: &str,
    expected_content_generation: u64,
    operation: F,
) -> Result<document_session::SessionJobOutcome<()>, document_session::DocumentSessionError>
where
    F: FnOnce(
            document_session::SessionJobContext,
        ) -> Result<(), document_session::SessionJobOperationError>
        + Send
        + 'static,
{
    store
        .run_snapshot_job(job_id, session_id, expected_content_generation, operation)
        .await
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn share_file_macos(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    store: tauri::State<'_, document_session::DocumentSessionStore>,
) -> Result<document_session::SessionJobOutcome<()>, document_session::DocumentSessionError> {
    share_session_snapshot_with(
        &store,
        &job_id,
        &session_id,
        expected_content_generation,
        |context| {
            context.cancellation.check_cancelled()?;
            let mut command = macos_share_command(&context.snapshot_path.to_string_lossy())
                .map_err(document_session::SessionJobOperationError::from)?;
            let status = context.child_processes.run_with_timeout(
                &mut command,
                std::time::Duration::from_secs(120),
                &context.cancellation,
            )?;
            if status.success() {
                Ok(())
            } else {
                Err(document_session::SessionJobOperationError::failed(format!(
                    "macOS Paylaşım menüsü {status} durumuyla kapandı"
                )))
            }
        },
    )
    .await
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn share_file_macos(
    _job_id: String,
    _session_id: String,
    _expected_content_generation: u64,
) -> Result<(), document_session::DocumentSessionError> {
    Err(document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "macOS Paylaşım Menüsü (Share Sheet) yalnızca macOS üzerinde desteklenmektedir.",
    ))
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn share_external_file_macos(path: String) -> Result<(), String> {
    share_external_path_macos(&path)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn share_external_file_macos(_path: String) -> Result<(), String> {
    Err(
        "FEATURE_UNAVAILABLE: Harici dosya paylaşımı yalnızca macOS üzerinde desteklenmektedir."
            .to_string(),
    )
}

// 4. Yerel OCR Engine (Apple Vision API / Native OCR Bridge)
#[cfg(target_os = "macos")]
#[tauri::command]
async fn perform_ocr_on_page(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    page_index: u32,
    store: tauri::State<'_, document_session::DocumentSessionStore>,
) -> Result<document_session::SessionJobOutcome<String>, document_session::DocumentSessionError> {
    store.run_snapshot_job(&job_id, &session_id, expected_content_generation, move |context| {
        tauri::async_runtime::block_on(async move {
            context.cancellation.check_cancelled()?;
            let temp_img_path = match job_worker::run_job(
                &context,
                job_worker::JobCommand::RenderOcrPage {
                    page_index,
                    output_name: "vision-ocr-input.png".to_string(),
                    scale: 1.5,
                },
                std::time::Duration::from_secs(120),
            )? {
                job_worker::JobResult::File(path) => path,
                _ => {
                    return Err(document_session::SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Vision OCR preparation worker beklenmeyen sonuç döndürdü",
                    ))
                }
            };
            let temp_img_path = job_worker::validate_output_file(&context, &temp_img_path)?;

            let output = context.child_processes.run_with_output_timeout(
                std::process::Command::new("swift").arg("-e").arg(format!(
                    r#"
                    import Vision
                    import AppKit

                    let imageURL = URL(fileURLWithPath: "{}")
                    guard let image = NSImage(contentsOf: imageURL),
                          let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else {{
                        exit(1)
                    }}

                    let request = VNRecognizeTextRequest {{ request, error in
                        guard let observations = request.results as? [VNRecognizedTextObservation] else {{ return }}
                        let text = observations.compactMap {{ $0.topCandidates(1).first?.string }}.joined(separator: "\n")
                        print(text)
                    }}
                    request.recognitionLevel = .accurate

                    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
                    try? handler.perform([request])
                    "#,
                    temp_img_path.to_string_lossy()
                )),
                std::time::Duration::from_secs(120),
                &context.cancellation,
            )?;

            if output.status.success() {
                let recognized = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if recognized.is_empty() {
                    Ok("Sayfada metin tespit edilemedi.".to_string())
                } else {
                    Ok(recognized)
                }
            } else {
                Err(document_session::SessionJobOperationError::failed(
                    "macOS Apple Vision OCR yürütülemedi.",
                ))
            }
        })
    }).await
}

#[cfg(target_os = "macos")]
#[cfg_attr(not(test), allow(dead_code))]
fn write_vision_ocr_temp_image(rendered: RenderedPage) -> Result<tempfile::NamedTempFile, String> {
    write_vision_ocr_temp_image_in(rendered, None)
}

#[cfg(target_os = "macos")]
#[cfg_attr(not(test), allow(dead_code))]
fn write_vision_ocr_temp_image_in(
    rendered: RenderedPage,
    directory: Option<&Path>,
) -> Result<tempfile::NamedTempFile, String> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("antigravity-vision-ocr-").suffix(".png");
    let temp_image = match directory {
        Some(directory) => builder.tempfile_in(directory),
        None => builder.tempfile(),
    }
    .map_err(|error| format!("Geçici OCR resmi oluşturulamadı: {error}"))?;
    let image = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        rendered.width,
        rendered.height,
        rendered.data,
    )
    .ok_or("ImageData dönüşüm hatası")?;
    image
        .save_with_format(temp_image.path(), image::ImageFormat::Png)
        .map_err(|error| format!("Geçici OCR resmi kaydedilemedi: {error}"))?;
    Ok(temp_image)
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn perform_ocr_on_page(
    _job_id: String,
    _session_id: String,
    _expected_content_generation: u64,
    _page_index: u32,
) -> Result<String, document_session::DocumentSessionError> {
    Err(document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "Apple Vision yerel OCR motoru sadece macOS üzerinde desteklenmektedir.",
    ))
}

// Görsel İşleme Komutları
fn apply_image_transformations(
    mut img: image::DynamicImage,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> image::DynamicImage {
    let normalized_angle = rotation.rem_euclid(360);
    match normalized_angle {
        90 => img = img.rotate90(),
        180 => img = img.rotate180(),
        270 => img = img.rotate270(),
        _ => {}
    }
    if flip_h {
        img = img.fliph();
    }
    if flip_v {
        img = img.flipv();
    }
    img
}

#[tauri::command]
fn read_image_file(
    path: String,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> Result<RenderedPage, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("Görsel dosyası bulunamadı: {}", path));
    }

    let loaded_img = image::open(&path).map_err(|e| format!("Görsel okunamadı: {}", e))?;
    let transformed = apply_image_transformations(loaded_img, rotation, flip_h, flip_v);
    let rgba = transformed.to_rgba8();

    Ok(RenderedPage {
        width: rgba.width(),
        height: rgba.height(),
        data: rgba.into_raw(),
    })
}

#[tauri::command]
fn convert_and_save_image(
    source_path: String,
    target_path: String,
    format: String,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> Result<(), String> {
    let p = Path::new(&source_path);
    if !p.exists() {
        return Err(format!("Kaynak dosya bulunamadı: {}", source_path));
    }

    let loaded_img = image::open(&source_path).map_err(|e| format!("Görsel açılamadı: {}", e))?;
    let transformed = apply_image_transformations(loaded_img, rotation, flip_h, flip_v);

    let target_format = format.to_lowercase();

    if target_format == "pdf" {
        return save_image_as_pdf(&transformed, &target_path);
    }

    let img_format = match target_format.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        "bmp" => image::ImageFormat::Bmp,
        _ => return Err(format!("Desteklenmeyen format: {}", format)),
    };

    transformed
        .save_with_format(&target_path, img_format)
        .map_err(|e| format!("Görsel kaydetme hatası: {}", e))?;

    Ok(())
}

fn save_image_as_pdf(img: &image::DynamicImage, target_path: &str) -> Result<(), String> {
    let rgb_img = img.to_rgb8();
    let width = rgb_img.width() as f32;
    let height = rgb_img.height() as f32;
    let pixels = rgb_img.into_raw();

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&pixels, 6);

    let image_stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width as i64,
            "Height" => height as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        compressed,
    );
    let image_id = doc.add_object(image_stream);

    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    width.into(),
                    0.into(),
                    0.into(),
                    height.into(),
                    0.into(),
                    0.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im1".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };
    let encoded_content = content
        .encode()
        .map_err(|e| format!("İçerik kodlama hatası: {}", e))?;
    let content_stream = Stream::new(dictionary! {}, encoded_content);
    let content_id = doc.add_object(content_stream);

    let page_dict = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        "Contents" => content_id,
        "Resources" => dictionary! {
            "XObject" => dictionary! {
                "Im1" => image_id,
            },
        },
    };
    let page_id = doc.add_object(page_dict);

    let pages_dict = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    doc.save(target_path)
        .map_err(|e| format!("PDF olarak kaydetme hatası: {}", e))?;
    Ok(())
}

#[allow(dead_code)]
fn hex_to_rgb(hex: &str) -> (f32, f32, f32) {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
        (r, g, b)
    } else {
        (1.0, 0.0, 0.0)
    }
}

#[tauri::command]
#[allow(dead_code)]
async fn burn_annotations_to_pdf(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    annotations_json: String,
    target_path: String,
    store: tauri::State<'_, document_session::DocumentSessionStore>,
) -> Result<document_session::SessionJobOutcome<()>, document_session::DocumentSessionError> {
    document_session::reject_private_target(Path::new(&target_path), &store.temp_root)?;
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                document_session::reject_unsupported_encrypted_snapshot(
                    context.credential.as_ref(),
                    "Annotation burn",
                )?;
                let items: Vec<AnnotationItem> =
                    serde_json::from_str(&annotations_json).map_err(|e| {
                        document_session::SessionJobOperationError::failed(format!(
                            "Annotation JSON okuma hatası: {e}"
                        ))
                    })?;

                let mut doc = Document::load(&context.snapshot_path).map_err(|e| {
                    document_session::SessionJobOperationError::failed(format!(
                        "PDF yüklenemedi: {e}"
                    ))
                })?;
                let pages = doc.get_pages();

                let mut page_map: HashMap<u32, Vec<&AnnotationItem>> = HashMap::new();
                for item in &items {
                    page_map.entry(item.page_index).or_default().push(item);
                }

                for (page_idx, annos) in page_map {
                    context.cancellation.check_cancelled()?;
                    let page_num = page_idx + 1;
                    if let Some(&page_id) = pages.get(&page_num) {
                        let mut ops = vec![Operation::new("q", vec![])];

                        for anno in annos {
                            context.cancellation.check_cancelled()?;
                            let (r, g, b) = hex_to_rgb(&anno.color);
                            ops.push(Operation::new("RG", vec![r.into(), g.into(), b.into()]));
                            ops.push(Operation::new("rg", vec![r.into(), g.into(), b.into()]));

                            let stroke_w = if anno.tool_type == "highlighter" {
                                anno.size * 3.0
                            } else {
                                anno.size
                            };
                            ops.push(Operation::new("w", vec![stroke_w.into()]));

                            let _cw = anno.canvas_width.max(1.0);
                            let ch = anno.canvas_height.max(1.0);

                            match anno.tool_type.as_str() {
                                "pen" | "highlighter" => {
                                    if let Some(pts) = &anno.points {
                                        if pts.len() > 1 {
                                            let first = &pts[0];
                                            ops.push(Operation::new(
                                                "m",
                                                vec![first.x.into(), (ch - first.y).into()],
                                            ));
                                            for pt in &pts[1..] {
                                                ops.push(Operation::new(
                                                    "l",
                                                    vec![pt.x.into(), (ch - pt.y).into()],
                                                ));
                                            }
                                            ops.push(Operation::new("S", vec![]));
                                        }
                                    }
                                }
                                "rectangle" => {
                                    if let (Some(x), Some(y), Some(w), Some(h)) =
                                        (anno.x, anno.y, anno.w, anno.h)
                                    {
                                        let pdf_y = ch - y - h;
                                        ops.push(Operation::new(
                                            "re",
                                            vec![x.into(), pdf_y.into(), w.into(), h.into()],
                                        ));
                                        ops.push(Operation::new("S", vec![]));
                                    }
                                }
                                "arrow" => {
                                    if let (Some(sx), Some(sy), Some(ex), Some(ey)) =
                                        (anno.x, anno.y, anno.end_x, anno.end_y)
                                    {
                                        let pdf_sy = ch - sy;
                                        let pdf_ey = ch - ey;
                                        ops.push(Operation::new(
                                            "m",
                                            vec![sx.into(), pdf_sy.into()],
                                        ));
                                        ops.push(Operation::new(
                                            "l",
                                            vec![ex.into(), pdf_ey.into()],
                                        ));
                                        ops.push(Operation::new("S", vec![]));
                                    }
                                }
                                "text" => {
                                    if let (Some(x), Some(y), Some(txt)) =
                                        (anno.x, anno.y, &anno.text)
                                    {
                                        let font_size = anno.size * 2.0;
                                        let pdf_y = ch - y - font_size;
                                        ops.push(Operation::new("BT", vec![]));
                                        ops.push(Operation::new(
                                            "Tf",
                                            vec!["/F1".into(), font_size.into()],
                                        ));
                                        ops.push(Operation::new(
                                            "Td",
                                            vec![x.into(), pdf_y.into()],
                                        ));
                                        ops.push(Operation::new(
                                            "Tj",
                                            vec![Object::String(
                                                txt.as_bytes().to_vec(),
                                                lopdf::StringFormat::Literal,
                                            )],
                                        ));
                                        ops.push(Operation::new("ET", vec![]));
                                    }
                                }
                                _ => {}
                            }
                        }

                        ops.push(Operation::new("Q", vec![]));

                        let content = lopdf::content::Content { operations: ops };
                        let encoded = content.encode().unwrap_or_else(|_| vec![]);

                        let content_id = doc.add_object(Object::Stream(lopdf::Stream::new(
                            lopdf::Dictionary::new(),
                            encoded,
                        )));

                        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
                        let cur_contents = page.get(b"Contents");
                        match cur_contents {
                            Ok(Object::Array(arr)) => {
                                let mut new_arr = arr.clone();
                                new_arr.push(Object::Reference(content_id));
                                page.set("Contents", Object::Array(new_arr));
                            }
                            Ok(Object::Reference(old_id)) => {
                                page.set(
                                    "Contents",
                                    Object::Array(vec![
                                        Object::Reference(*old_id),
                                        Object::Reference(content_id),
                                    ]),
                                );
                            }
                            _ => {
                                page.set("Contents", Object::Reference(content_id));
                            }
                        }
                    }
                }

                context.cancellation.check_cancelled()?;
                doc.save(&target_path).map_err(|e| {
                    document_session::SessionJobOperationError::failed(format!(
                        "Annotation kaydedilemedi: {e}"
                    ))
                })?;
                Ok(())
            },
        )
        .await
}

pub fn run_smoke() {
    use crate::worker::ipc::{
        decode_response, encode_request, read_payload, write_frame, WorkerCommand,
        WorkerCredential, WorkerRenderTarget, WorkerRequest, WorkerResponse, WorkerResult,
        MAX_WORKER_RESPONSE_BYTES, WORKER_PROTOCOL_VERSION,
    };
    use std::io::{self, BufRead, Write};

    // Smoke must exercise the production worker architecture: the main `--smoke`
    // process spawns the real PDFium worker (`--run-worker`) and routes every
    // operation through the worker IPC protocol, rather than loading PDFium
    // in-process. This keeps the hook on the same production backend path that
    // the application uses (contract §8/§10/§11/§16).

    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("SMOKE_ERROR: worker executable could not be resolved: {error}");
            std::process::exit(1);
        }
    };

    let mut command = std::process::Command::new(&executable);
    command
        .arg("--run-worker")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());

    // Propagate the pre-resolved absolute runtime path for main/worker parity.
    // This prevents the worker from diverging to CWD/PATH/system runtime sources.
    if let Ok(resolved) = crate::pdf_engine::resolve_runtime_path() {
        command.env("PDFIUM_RESOLVED_PATH", resolved);
    } else if let Some(path) = std::env::var_os("PDFIUM_PATH") {
        command.env("PDFIUM_PATH", path);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("SMOKE_ERROR: failed to spawn PDFium worker: {error}");
            std::process::exit(1);
        }
    };

    let mut worker_stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            eprintln!("SMOKE_ERROR: worker stdin pipe unavailable");
            let _ = child.kill();
            std::process::exit(1);
        }
    };
    let mut worker_stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            eprintln!("SMOKE_ERROR: worker stdout pipe unavailable");
            let _ = child.kill();
            std::process::exit(1);
        }
    };

    fn send_request(
        request: &WorkerRequest,
        writer: &mut std::process::ChildStdin,
    ) -> Result<(), String> {
        let frame =
            encode_request(request).map_err(|error| format!("request encode failed: {error}"))?;
        write_frame(writer, &frame).map_err(|error| format!("request write failed: {error}"))
    }

    fn recv_response(reader: &mut std::process::ChildStdout) -> Result<WorkerResponse, String> {
        let payload = read_payload(reader, MAX_WORKER_RESPONSE_BYTES)
            .map_err(|error| format!("response read failed: {error}"))?;
        decode_response(&payload).map_err(|error| format!("response decode failed: {error}"))
    }

    // Handshake: prove the worker actually initialized PDFium (real runtime,
    // real worker process). If the worker reports `ready == false`, the smoke
    // must fail closed rather than degrade to a direct in-process fallback.
    let hello = WorkerRequest {
        protocol_version: WORKER_PROTOCOL_VERSION,
        request_id: 0,
        command: WorkerCommand::Hello,
    };
    if let Err(error) = send_request(&hello, &mut worker_stdin) {
        eprintln!("SMOKE_ERROR: {error}");
        let _ = child.kill();
        std::process::exit(1);
    }
    let ready = match recv_response(&mut worker_stdout) {
        Ok(response) => match response.result {
            Ok(WorkerResult::Hello { ready, .. }) => ready,
            _ => {
                eprintln!("SMOKE_ERROR: worker handshake returned an unexpected response");
                let _ = child.kill();
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("SMOKE_ERROR: {error}");
            let _ = child.kill();
            std::process::exit(1);
        }
    };
    if !ready {
        eprintln!("SMOKE_ERROR: PDFium runtime is unavailable in the worker process");
        let _ = child.kill();
        std::process::exit(1);
    }

    let mut next_request_id = 1u64;
    let mut document_id: Option<u64> = None;

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) => {
                eprintln!("SMOKE_ERROR: stdin read failed: {error}");
                break;
            }
        };
        let _ = bytes;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let cmd: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("SMOKE_ERROR: invalid JSON: {error}");
                continue;
            }
        };

        let cmd_type = cmd.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let req_id = cmd.get("id").cloned();

        match cmd_type {
            "open" => {
                let path = cmd.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let credential = match cmd.get("password").and_then(|v| v.as_str()) {
                    Some(password) => match WorkerCredential::new(password.to_string()) {
                        Ok(credential) => Some(credential),
                        Err(error) => {
                            let response = serde_json::json!({
                                "id": req_id,
                                "status": "error",
                                "error": error.message
                            });
                            writeln!(writer, "{response}").ok();
                            writer.flush().ok();
                            continue;
                        }
                    },
                    None => None,
                };

                let request = WorkerRequest {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    request_id: next_request_id,
                    command: WorkerCommand::OpenDocument {
                        path: path.to_string(),
                        credential,
                    },
                };
                next_request_id += 1;

                let response = send_request(&request, &mut worker_stdin)
                    .and_then(|()| recv_response(&mut worker_stdout));
                match response {
                    Ok(WorkerResponse {
                        result:
                            Ok(WorkerResult::DocumentOpened {
                                document_id: id,
                                page_count,
                                ..
                            }),
                        ..
                    }) => {
                        document_id = Some(id);
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "ok",
                            "page_count": page_count
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Ok(WorkerResponse {
                        result: Err(failure),
                        ..
                    }) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": failure.message
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Ok(_) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": "unexpected worker response for open"
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Err(error) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": error
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                }
            }
            "render_thumbnail" => {
                let page_index = cmd.get("page_index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let scale = cmd.get("scale").and_then(|v| v.as_f64()).unwrap_or(0.25);
                let Some(id) = document_id else {
                    let response = serde_json::json!({
                        "id": req_id,
                        "status": "error",
                        "error": "no open document"
                    });
                    writeln!(writer, "{response}").ok();
                    writer.flush().ok();
                    continue;
                };

                let request = WorkerRequest {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    request_id: next_request_id,
                    command: WorkerCommand::RenderPage {
                        document_id: id,
                        page_index,
                        target: WorkerRenderTarget::Scale(scale),
                        rotation: 0,
                    },
                };
                next_request_id += 1;

                let response = send_request(&request, &mut worker_stdin)
                    .and_then(|()| recv_response(&mut worker_stdout));
                match response {
                    Ok(WorkerResponse {
                        result:
                            Ok(WorkerResult::PageRendered {
                                width,
                                height,
                                data,
                            }),
                        ..
                    }) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "ok",
                            "width": width,
                            "height": height,
                            "data_len": data.len()
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Ok(WorkerResponse {
                        result: Err(failure),
                        ..
                    }) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": failure.message
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Ok(_) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": "unexpected worker response for render"
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Err(error) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": error
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                }
            }
            "extract_text" => {
                let page_index = cmd.get("page_index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let Some(id) = document_id else {
                    let response = serde_json::json!({
                        "id": req_id,
                        "status": "error",
                        "error": "no open document"
                    });
                    writeln!(writer, "{response}").ok();
                    writer.flush().ok();
                    continue;
                };

                let request = WorkerRequest {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    request_id: next_request_id,
                    command: WorkerCommand::GetPageTextLayout {
                        document_id: id,
                        page_index,
                    },
                };
                next_request_id += 1;

                let response = send_request(&request, &mut worker_stdin)
                    .and_then(|()| recv_response(&mut worker_stdout));
                match response {
                    Ok(WorkerResponse {
                        result: Ok(WorkerResult::TextLayout(items)),
                        ..
                    }) => {
                        let text = items
                            .iter()
                            .map(|item| item.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "ok",
                            "text": text
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Ok(WorkerResponse {
                        result: Err(failure),
                        ..
                    }) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": failure.message
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Ok(_) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": "unexpected worker response for text"
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                    Err(error) => {
                        let response = serde_json::json!({
                            "id": req_id,
                            "status": "error",
                            "error": error
                        });
                        writeln!(writer, "{response}").ok();
                        writer.flush().ok();
                    }
                }
            }
            "close" | "shutdown" => {
                let response = serde_json::json!({
                    "id": req_id,
                    "status": "ok"
                });
                writeln!(writer, "{response}").ok();
                writer.flush().ok();
                break;
            }
            "ping" => {
                let request = WorkerRequest {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    request_id: next_request_id,
                    command: WorkerCommand::Ping,
                };
                next_request_id += 1;
                let response = send_request(&request, &mut worker_stdin)
                    .and_then(|()| recv_response(&mut worker_stdout));
                let status = match response {
                    Ok(WorkerResponse {
                        result: Ok(WorkerResult::Pong),
                        ..
                    }) => "ok",
                    _ => "error",
                };
                let response = serde_json::json!({
                    "id": req_id,
                    "status": status
                });
                writeln!(writer, "{response}").ok();
                writer.flush().ok();
            }
            _ => {
                let response = serde_json::json!({
                    "id": req_id,
                    "status": "error",
                    "error": format!("unknown command: {cmd_type}")
                });
                writeln!(writer, "{response}").ok();
                writer.flush().ok();
            }
        }
    }

    // Graceful worker shutdown: request Shutdown, drain the reply, close the
    // input pipe, and reap the child so the smoke ends with a clean teardown.
    let shutdown = WorkerRequest {
        protocol_version: WORKER_PROTOCOL_VERSION,
        request_id: next_request_id,
        command: WorkerCommand::Shutdown,
    };
    let _ = send_request(&shutdown, &mut worker_stdin);
    let _ = recv_response(&mut worker_stdout);
    drop(worker_stdin);
    let _ = child.wait();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let registry = tracing_subscriber::registry();

    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());

    #[cfg(debug_assertions)]
    {
        registry
            .with(tracing_subscriber::fmt::layer().pretty().with_filter(
                EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()),
            ))
            .init();
    }
    #[cfg(not(debug_assertions))]
    {
        let file_appender = tracing_appender::rolling::daily("logs", "antigravity.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        std::mem::forget(_guard); // Prevent guard from dropping and stopping the logging thread
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(non_blocking)
                    .with_filter(
                        EnvFilter::from_default_env().add_directive(tracing::Level::WARN.into()),
                    ),
            )
            .init();
    }

    let (batch_tx, batch_rx) = tokio::sync::mpsc::channel(100);
    let batch_queue = batch::BatchQueue { sender: batch_tx };
    let shutdown_token = tokio_util::sync::CancellationToken::new();
    let shutdown_token_clone = shutdown_token.clone();
    let password_cache = std::sync::Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let document_sessions = document_session::DocumentSessionStore::initialize_with_password_cache(
        std::sync::Arc::clone(&password_cache),
    )
    .expect("Güvenli document-session temp alanı başlatılamadı");
    let pdfium_worker_broker = document_sessions.worker_broker();

    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            app.state::<document_session::DocumentSessionStore>()
                .start_job_reaper();

            if let Ok(resource_dir) = app.path().resource_dir() {
                let runtime_path = resource_dir
                    .join("lib")
                    .join(crate::pdf_engine::runtime_metadata::RUNTIME_FILENAME);
                std::env::set_var("PDFIUM_PATH", &runtime_path);
                std::env::set_var("TAURI_RESOURCE_DIR", &resource_dir);
            }

            tauri::async_runtime::spawn(async move {
                batch::start_batch_worker(handle, batch_rx, shutdown_token_clone).await;
            });
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_log::Builder::new().skip_logger().build())
        .manage(PasswordCache(password_cache))
        .manage(SearchCache(parking_lot::Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(50).unwrap(),
        ))))
        .manage(ActiveSearchId(std::sync::Arc::new(
            std::sync::atomic::AtomicU64::new(0),
        )))
        .manage(ai::VectorStore(parking_lot::RwLock::new(HashMap::new())))
        .manage(ai::endpoint::AiEndpointStore::new())
        .manage(ai::request_registry::AiRequestRegistry::new())
        .manage(ActiveRenderTasks(parking_lot::Mutex::new(HashMap::new())))
        .manage(AppShutdownToken(shutdown_token))
        .manage(AppShutdownCoordinator(std::sync::atomic::AtomicBool::new(
            false,
        )))
        .manage(TextLayoutCache(parking_lot::Mutex::new(
            lru::LruCache::new(std::num::NonZeroUsize::new(50).unwrap()),
        )))
        .manage(RenderService::new())
        .manage(pdfium_worker_broker)
        .manage(document_sessions)
        .manage(plugins::PluginState::new())
        .manage(batch_queue)
        .invoke_handler(tauri::generate_handler![
            document_session::open_document_session,
            document_session::authenticate_document_session,
            document_session::get_document_session_snapshot,
            document_session::get_active_document_session_snapshot,
            document_session::probe_candidate_render,
            document_session::recover_document_session,
            document_session::cancel_session_job,
            document_session::export_session_page_to_image,
            document_session::export_session_pages_as_images,
            document_session::export_session_text_content,
            document_session::print_session_pdf,
            document_session::inspect_session_print_resources,
            document_session::generate_session_rgb_derived_coverage_preview,
            document_session::generate_session_rgb_derived_process_preview,
            document_session::extract_session_pages,
            document_session::split_session_pdf,
            document_session::discard_candidate_session,
            document_session::get_session_page_count,
            document_session::get_session_pdf_outline,
            document_session::get_session_pdf_metadata,
            document_session::get_session_structure_tree,
            document_session::audit_session_accessibility,
            document_session::get_session_page_annotations,
            document_session::search_session_pdf_text,
            document_session::get_session_page_text_layout,
            document_session::render_session_page,
            document_session::render_session_thumbnail,
            document_session::set_active_session_render_document,
            document_session::cancel_session_render_request,
            document_session::rotate_session_page,
            document_session::delete_session_pages,
            document_session::reorder_session_pages,
            document_session::insert_session_blank_page,
            document_session::add_session_annotation,
            document_session::delete_session_annotation,
            document_session::update_session_metadata,
            document_session::save_document_session,
            document_session::save_document_session_as,
            document_session::begin_session_transition,
            document_session::save_transition,
            document_session::discard_transition,
            document_session::cancel_transition,
            document_session::prepare_candidate_activation,
            document_session::commit_candidate_activation,
            document_session::rollback_candidate_activation,
            document_session::close_all_document_sessions,
            ocr::detect_scanned_page,
            ocr::get_available_ocr_languages,
            ocr::run_ocr_on_page,
            perform_ocr_on_page,
            share_file_macos,
            share_external_file_macos,
            trigger_continuity_camera_scan,
            read_image_file,
            convert_and_save_image,
            speak_text,
            stop_speaking,
            document_session::merge_session_pdfs,
            plugins::inspect_plugin_package,
            plugins::install_plugin_package,
            plugins::grant_plugin_consent,
            plugins::uninstall_plugin,
            plugins::set_plugin_enabled,
            plugins::list_installed_plugins,
            plugins::start_plugin_execution,
            plugins::cancel_plugin_execution,
            plugins::get_plugin_execution_status,
            batch::add_batch_job,
            redaction::apply_redactions,
            redaction::plan_secure_redaction,
            redaction::execute_secure_redaction,
            redaction::sanitize_pdf_document,
            sanitization::inspect_document_sanitization,
            sanitization_publication::execute_document_sanitization,
            redaction::scan_sensitive_data,
            ocr::export_as_searchable_pdf,
            ai::configure_ai_endpoint,
            ai::preview_ai_endpoint,
            ai::get_ai_endpoint_status,
            ai::clear_ai_endpoint_config,
            ai::cancel_ai_request,
            ai::build_semantic_index,
            ai::semantic_search,
            ai::query_document_rag,
            ai::summarize_selection_or_page,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri uygulaması başlatılırken hata oluştu")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let coordinator = app_handle.state::<AppShutdownCoordinator>();
                if !coordinator
                    .0
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
                {
                    api.prevent_exit();
                    app_handle.state::<AppShutdownToken>().0.cancel();
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        {
                            let store = handle.state::<document_session::DocumentSessionStore>();
                            let active_tasks = handle.state::<ActiveRenderTasks>();
                            let render_service =
                                handle.state::<pdf_engine::render_service::RenderService>();
                            let warnings = store
                                .shutdown_all_resources(&active_tasks, &render_service)
                                .await;
                            for warning in warnings {
                                eprintln!("{}: {}", warning.code, warning.message);
                            }
                        }
                        handle.exit(0);
                    });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_corrupted_or_missing_pdf_handling() {
        // Hatalı (olmayan) bir PDF dosyasının açılması durumunda panic yerine
        // Err dönüldüğünün testi.
        let pdfium_res = get_pdfium();
        assert!(pdfium_res.is_ok(), "Pdfium library failed to load");

        let pdfium = pdfium_res.unwrap();
        let result = pdfium.load_pdf_from_file("/this/path/does/not/exist.pdf", None);

        assert!(
            result.is_err(),
            "Açılmayan dosya hata (Err) döndürmelidir, Ok değil."
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn platform_process_runner_handles_success_and_timeout() {
        let success = run_child_with_timeout(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("exit 0"),
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        assert!(success.success());

        let timeout = run_child_with_timeout(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("sleep 5"),
            std::time::Duration::from_millis(10),
        );
        assert!(timeout.unwrap_err().contains("zaman aşımı"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn vision_ocr_temp_image_is_removed_on_drop() {
        let temp_image = write_vision_ocr_temp_image(RenderedPage {
            width: 1,
            height: 1,
            data: vec![255, 255, 255, 255],
        })
        .unwrap();
        let path = temp_image.path().to_path_buf();
        assert!(path.is_file());
        drop(temp_image);
        assert!(!path.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn vision_ocr_temp_image_is_removed_when_encoding_input_is_invalid() {
        let directory = tempfile::tempdir().unwrap();
        let result = write_vision_ocr_temp_image_in(
            RenderedPage {
                width: 2,
                height: 2,
                data: vec![255, 255, 255, 255],
            },
            Some(directory.path()),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
