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
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos_helper;
pub mod manipulation;
pub mod ocr;
pub(crate) mod output_names;
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
pub mod tts;
pub mod worker;

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

pub struct PasswordCache(pub std::sync::Arc<parking_lot::Mutex<HashMap<String, String>>>);
pub struct SearchCache(pub parking_lot::Mutex<lru::LruCache<String, SearchResult>>);
pub struct ActiveSearchId(pub std::sync::Arc<std::sync::atomic::AtomicU64>);
pub struct ActiveRenderTasks(
    pub parking_lot::Mutex<HashMap<String, tokio_util::sync::CancellationToken>>,
);
pub struct AppShutdownToken(pub tokio_util::sync::CancellationToken);
pub struct AppShutdownCoordinator(pub std::sync::atomic::AtomicBool);
pub struct TextLayoutCache(pub parking_lot::Mutex<lru::LruCache<String, Vec<TextLayoutData>>>);

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

// 2. Yerel Metin Okuma (Text-to-Speech): bkz. `tts` modülü.

// 3. macOS Paylaşım Menüsü (Share Sheet)
#[cfg(target_os = "macos")]
fn run_child_with_timeout(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
) -> Result<(std::process::ExitStatus, Vec<u8>), String> {
    use std::io::Read;

    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Platform süreci başlatılamadı: {error}"))?;
    // Pipe tamponu dolup çocuk süreç bloklanmasın diye stderr eşzamanlı okunur.
    let stderr_reader = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        })
    });
    let collect_stderr = |reader: Option<std::thread::JoinHandle<Vec<u8>>>| {
        reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default()
    };
    let deadline = std::time::Instant::now() + timeout;

    loop {
        let polled = child.try_wait();
        match polled {
            Ok(Some(status)) => return Ok((status, collect_stderr(stderr_reader))),
            Ok(None) if std::time::Instant::now() < deadline => {}
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                // Okuyucu beklenmez: pipe'ı tutan bir torun süreç join'i bloklayabilir.
                drop(stderr_reader);
                return Err(match polled {
                    Err(error) => format!("Platform süreci izlenemedi: {error}"),
                    Ok(_) => "Platform süreci zaman aşımına uğradı".to_string(),
                });
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(target_os = "macos")]
fn macos_share_command(
    path: &Path,
    anchor: Option<macos_helper::ShareAnchor>,
) -> Result<std::process::Command, String> {
    if !path.is_file() {
        return Err(format!("Paylaşılacak dosya bulunamadı: {}", path.display()));
    }
    macos_helper::helper_command(macos_helper::share_command_args(path, anchor)?)
}

/// Komutu çağıran pencerenin içerik alanı; picker bu pencereye bağlanır.
/// Pencere küçültülmüş/gizliyse veya konum okunamazsa `None` (ekran ortası).
#[cfg(target_os = "macos")]
fn share_anchor_for_window(window: &tauri::WebviewWindow) -> Option<macos_helper::ShareAnchor> {
    if window.is_minimized().unwrap_or(true) || !window.is_visible().unwrap_or(false) {
        return None;
    }
    let position = window.inner_position().ok()?;
    let size = window.inner_size().ok()?;
    let scale_factor = window.scale_factor().ok()?;
    macos_helper::ShareAnchor::from_physical(
        position.x,
        position.y,
        size.width,
        size.height,
        scale_factor,
    )
}

fn child_stderr_detail(stderr: &[u8]) -> String {
    const MAX_CHARS: usize = 500;
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.chars().count() > MAX_CHARS {
        format!("{}…", text.chars().take(MAX_CHARS).collect::<String>())
    } else {
        text.to_string()
    }
}

#[cfg(target_os = "macos")]
fn share_exit_error(status: std::process::ExitStatus, stderr: &[u8]) -> String {
    let detail = child_stderr_detail(stderr);
    if detail.is_empty() {
        format!("macOS Paylaşım menüsü {status} durumuyla kapandı")
    } else {
        format!("macOS Paylaşım menüsü {status} durumuyla kapandı: {detail}")
    }
}

#[cfg(target_os = "macos")]
fn share_external_path_macos(
    path: &str,
    anchor: Option<macos_helper::ShareAnchor>,
) -> Result<(), String> {
    let (status, stderr) = run_child_with_timeout(
        &mut macos_share_command(Path::new(path), anchor)?,
        macos_helper::SHARE_PROCESS_TIMEOUT,
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(share_exit_error(status, &stderr))
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
    window: tauri::WebviewWindow,
    store: tauri::State<'_, document_session::DocumentSessionStore>,
) -> Result<document_session::SessionJobOutcome<()>, document_session::DocumentSessionError> {
    let anchor = share_anchor_for_window(&window);
    // Alıcı "snapshot.pdf" yerine belgenin kendi adını görsün. Oturum
    // bulunamazsa run_snapshot_job aynı hatayı zaten döndürür.
    let display_path = store
        .session_snapshot(&session_id)
        .await
        .ok()
        .map(|snapshot| snapshot.display_path);
    let file_name = macos_helper::sanitize_share_file_name(display_path.as_deref());
    share_session_snapshot_with(
        &store,
        &job_id,
        &session_id,
        expected_content_generation,
        move |context| {
            context.cancellation.check_cancelled()?;
            // Kopya job dizinindedir: süreç bitene (servis delegesi sonuç
            // bildirene) kadar yaşar, iptal/kapanışta job ile birlikte silinir.
            let share_path = document_session::stage_job_share_copy(&context, &file_name)?;
            let mut command = macos_share_command(&share_path, anchor)
                .map_err(document_session::SessionJobOperationError::from)?;
            let output = context.child_processes.run_with_output_timeout(
                &mut command,
                macos_helper::SHARE_PROCESS_TIMEOUT,
                &context.cancellation,
            )?;
            if output.status.success() {
                Ok(())
            } else {
                Err(document_session::SessionJobOperationError::failed(
                    share_exit_error(output.status, &output.stderr),
                ))
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

// Paylaşım servisi sonucu bildirene kadar (en fazla SHARE_PROCESS_TIMEOUT) çocuk süreci bekler; senkron
// Tauri komutları ana thread'de çalıştığından bekleme blocking havuzunda yapılır.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn share_external_file_macos(
    path: String,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let anchor = share_anchor_for_window(&window);
    tauri::async_runtime::spawn_blocking(move || share_external_path_macos(&path, anchor))
        .await
        .map_err(|error| format!("macOS Paylaşım menüsü görevi tamamlanamadı: {error}"))?
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn share_external_file_macos(_path: String) -> Result<(), String> {
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
    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
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
                            return Err(
                                document_session::SessionJobOperationError::failed_with_code(
                                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                                    "Vision OCR preparation worker beklenmeyen sonuç döndürdü",
                                ),
                            )
                        }
                    };
                    let temp_img_path = job_worker::validate_output_file(&context, &temp_img_path)?;

                    let mut command = macos_helper::ocr_command_args(&temp_img_path)
                        .and_then(macos_helper::helper_command)
                        .map_err(document_session::SessionJobOperationError::from)?;
                    let output = context.child_processes.run_with_output_timeout(
                        &mut command,
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
                        let detail = child_stderr_detail(&output.stderr);
                        Err(document_session::SessionJobOperationError::failed(
                            if detail.is_empty() {
                                format!("macOS Apple Vision OCR yürütülemedi ({}).", output.status)
                            } else {
                                format!("macOS Apple Vision OCR yürütülemedi: {detail}")
                            },
                        ))
                    }
                })
            },
        )
        .await
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

/// `read_image_file` frames reuse `render-frame/1`; the page index, generation
/// and request ID are fixed placeholders because an image is not a session.
const IMAGE_FRAME_REQUEST_ID: &str = "read_image_file";

/// User images share the PDF render bounds: at most 16384 px per side and
/// 256 MiB for both the decoder buffer and the RGBA output.
const MAX_IMAGE_DIMENSION: u32 = pdf_engine::render_service::MAX_PIXEL_DIMENSION;
const MAX_IMAGE_BYTES: u64 = pdf_engine::render_service::MAX_RENDER_BYTES as u64;

fn image_too_large_error() -> String {
    format!(
        "Görsel çok büyük: en fazla {0}×{0} piksel ve {1} MiB çözümlenmiş veri desteklenir.",
        MAX_IMAGE_DIMENSION,
        MAX_IMAGE_BYTES / (1024 * 1024)
    )
}

/// `image::open` with explicit limits; sizes are checked from the header
/// before any pixel buffer is allocated.
fn open_bounded_image(path: &str, error_prefix: &str) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;

    let map_error = |e: image::ImageError| match e {
        image::ImageError::Limits(_) => image_too_large_error(),
        e => format!("{}: {}", error_prefix, e),
    };
    let mut reader =
        image::ImageReader::open(path).map_err(|e| format!("{}: {}", error_prefix, e))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(map_error)?;
    let (width, height) = decoder.dimensions();
    let rgba_bytes = u64::from(width) * u64::from(height) * 4;
    if decoder.total_bytes() > MAX_IMAGE_BYTES || rgba_bytes > MAX_IMAGE_BYTES {
        return Err(image_too_large_error());
    }
    image::DynamicImage::from_decoder(decoder).map_err(map_error)
}

/// Runs image decode/encode off the async runtime threads.
async fn run_image_job<T, F>(job: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| format!("Görsel işlemi tamamlanamadı: {}", e))?
}

fn decode_image_rgba(
    path: &str,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> Result<RenderedPage, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("Görsel dosyası bulunamadı: {}", path));
    }

    let loaded_img = open_bounded_image(path, "Görsel okunamadı")?;
    let transformed = apply_image_transformations(loaded_img, rotation, flip_h, flip_v);
    let rgba = transformed.to_rgba8();

    Ok(RenderedPage {
        width: rgba.width(),
        height: rgba.height(),
        data: rgba.into_raw(),
    })
}

fn encode_image_frame(image: RenderedPage) -> Result<Vec<u8>, String> {
    pdf_engine::render_wire::encode_render_frame(&pdf_engine::render_service::RenderResponse {
        width: image.width,
        height: image.height,
        data: std::sync::Arc::new(image.data),
        page_index: 0,
        generation: 0,
        request_id: IMAGE_FRAME_REQUEST_ID.to_string(),
    })
}

#[tauri::command]
async fn read_image_file(
    path: String,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> Result<tauri::ipc::Response, String> {
    run_image_job(move || {
        decode_image_rgba(&path, rotation, flip_h, flip_v).and_then(encode_image_frame)
    })
    .await
    .map(tauri::ipc::Response::new)
}

fn convert_image_file(
    source_path: &str,
    target_path: &str,
    format: &str,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> Result<(), String> {
    let p = Path::new(source_path);
    if !p.exists() {
        return Err(format!("Kaynak dosya bulunamadı: {}", source_path));
    }

    let loaded_img = open_bounded_image(source_path, "Görsel açılamadı")?;
    let transformed = apply_image_transformations(loaded_img, rotation, flip_h, flip_v);

    let target_format = format.to_lowercase();

    if target_format == "pdf" {
        return save_image_as_pdf(&transformed, target_path);
    }

    let img_format = match target_format.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        "bmp" => image::ImageFormat::Bmp,
        _ => return Err(format!("Desteklenmeyen format: {}", format)),
    };

    write_image_target(target_path, "Görsel kaydetme hatası", |file| {
        let mut writer = std::io::BufWriter::new(file);
        transformed.write_to(&mut writer, img_format)?;
        writer
            .into_inner()
            .map(drop)
            .map_err(|e| image::ImageError::IoError(e.into_error()))
    })
}

/// Replaces `target_path` only after the encoded output is fully on disk. The
/// source image is already decoded into memory, so converting a file onto its
/// own path is safe.
fn write_image_target<E: std::fmt::Display>(
    target_path: &str,
    error_prefix: &str,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), E>,
) -> Result<(), String> {
    document_io::write_atomically(Path::new(target_path), write)
        .map_err(|error| format!("{}: {}", error_prefix, error))
}

#[tauri::command]
async fn convert_and_save_image(
    source_path: String,
    target_path: String,
    format: String,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
    store: tauri::State<'_, document_session::DocumentSessionStore>,
) -> Result<(), String> {
    convert_and_save_image_outside(
        &store.temp_root,
        source_path,
        target_path,
        format,
        rotation,
        flip_h,
        flip_v,
    )
    .await
}

async fn convert_and_save_image_outside(
    private_root: &Path,
    source_path: String,
    target_path: String,
    format: String,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> Result<(), String> {
    // Session/job temp roots hold decrypted working copies; never export there.
    document_session::reject_private_target(Path::new(&target_path), private_root)
        .map_err(|error| error.to_string())?;
    run_image_job(move || {
        convert_image_file(
            &source_path,
            &target_path,
            &format,
            rotation,
            flip_h,
            flip_v,
        )
    })
    .await
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

    write_image_target(target_path, "PDF olarak kaydetme hatası", |file| {
        let mut writer = std::io::BufWriter::new(file);
        doc.save_to(&mut writer)?;
        writer.into_inner().map(drop).map_err(|e| e.into_error())
    })
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

/// Keeps the non-blocking log writer alive until `RunEvent::Exit`, where it is
/// dropped so buffered lines reach disk. `None` in debug builds and when file
/// logging fell back to stderr.
pub struct LogGuard(pub parking_lot::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>);

#[cfg(not(debug_assertions))]
const LOG_FILE_PREFIX: &str = "antigravity.log";

/// Validates and creates the release log directory resolved by Tauri. A
/// relative path would depend on the process working directory (`/` on macOS,
/// the install directory on Windows), so it is rejected like a resolver error.
#[cfg_attr(all(debug_assertions, not(test)), allow(dead_code))]
fn prepare_log_dir<E: std::fmt::Display>(
    app_log_dir: Result<std::path::PathBuf, E>,
) -> Result<std::path::PathBuf, String> {
    let dir = app_log_dir.map_err(|error| format!("uygulama log dizini çözümlenemedi: {error}"))?;
    if !dir.is_absolute() {
        return Err(format!("log dizini göreli: {}", dir.display()));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("log dizini oluşturulamadı ({}): {error}", dir.display()))?;
    Ok(dir)
}

#[cfg(debug_assertions)]
fn init_debug_tracing() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let registry = tracing_subscriber::registry();

    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());

    registry
        .with(
            tracing_subscriber::fmt::layer().pretty().with_filter(
                EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()),
            ),
        )
        .init();
}

/// Installs the release JSON logger: a daily rolling file in the app log
/// directory, or stderr when that directory or file cannot be prepared.
#[cfg(not(debug_assertions))]
fn init_release_tracing(
    app_log_dir: tauri::Result<std::path::PathBuf>,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::fmt::writer::BoxMakeWriter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let appender = prepare_log_dir(app_log_dir).and_then(|dir| {
        tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(LOG_FILE_PREFIX)
            .build(&dir)
            .map_err(|error| format!("log dosyası açılamadı ({}): {error}", dir.display()))
    });
    let (writer, guard) = match appender {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (BoxMakeWriter::new(writer), Some(guard))
        }
        Err(reason) => {
            eprintln!("Dosya loglaması kullanılamıyor, stderr'e yazılıyor: {reason}");
            (BoxMakeWriter::new(std::io::stderr), None)
        }
    };

    let registry = tracing_subscriber::registry();

    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());

    registry
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(writer)
                .with_filter(
                    EnvFilter::from_default_env().add_directive(tracing::Level::WARN.into()),
                ),
        )
        .init();
    guard
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Release logging needs the app log directory, so it is installed in
    // `setup`; events emitted before that point are not written to the file.
    #[cfg(debug_assertions)]
    init_debug_tracing();

    let shutdown_token = tokio_util::sync::CancellationToken::new();
    let password_cache = std::sync::Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let document_sessions = document_session::DocumentSessionStore::initialize_with_password_cache(
        std::sync::Arc::clone(&password_cache),
    )
    .expect("Güvenli document-session temp alanı başlatılamadı");
    let pdfium_worker_broker = document_sessions.worker_broker();

    tauri::Builder::default()
        .setup(|app| {
            #[cfg(not(debug_assertions))]
            let log_guard = init_release_tracing(app.path().app_log_dir());
            #[cfg(debug_assertions)]
            let log_guard = None;
            app.manage(LogGuard(parking_lot::Mutex::new(log_guard)));

            app.state::<document_session::DocumentSessionStore>()
                .start_job_reaper();
            // Bitiş olaylarını yayınlamak için AppHandle gerekir; komutlar setup'tan sonra çalışır.
            app.manage(tts::SpeechController::for_app(app.handle().clone()));

            if let Ok(resource_dir) = app.path().resource_dir() {
                let runtime_path = resource_dir
                    .join("lib")
                    .join(crate::pdf_engine::runtime_metadata::RUNTIME_FILENAME);
                std::env::set_var("PDFIUM_PATH", &runtime_path);
                std::env::set_var("TAURI_RESOURCE_DIR", &resource_dir);
            }
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
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
            document_session::get_session_page_sizes,
            document_session::get_session_pdf_outline,
            document_session::get_session_pdf_metadata,
            document_session::get_session_structure_tree,
            document_session::audit_session_accessibility,
            document_session::get_session_page_annotations,
            document_session::search_session_pdf_text,
            document_session::get_session_page_text_layout,
            document_session::retry_session_page,
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
            ocr::detect_scanned_pages,
            ocr::get_available_ocr_languages,
            ocr::run_ocr_on_page,
            perform_ocr_on_page,
            share_file_macos,
            share_external_file_macos,
            trigger_continuity_camera_scan,
            read_image_file,
            convert_and_save_image,
            tts::speak_text,
            tts::stop_speaking,
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
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
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
            tauri::RunEvent::Exit => {
                // Dropping the guard flushes buffered log lines before the process exits.
                if let Some(log_guard) = app_handle.try_state::<LogGuard>() {
                    drop(log_guard.0.lock().take());
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_log_dir_creates_missing_absolute_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("Logs").join("antigravity");

        let resolved = prepare_log_dir::<tauri::Error>(Ok(dir.clone())).unwrap();

        assert_eq!(resolved, dir);
        assert!(dir.is_dir());
    }

    #[test]
    fn prepare_log_dir_reports_resolver_error() {
        let error = prepare_log_dir::<&str>(Err("home dizini yok")).unwrap_err();
        assert!(error.contains("home dizini yok"), "{error}");
    }

    #[test]
    fn prepare_log_dir_rejects_relative_directory() {
        let relative = std::path::PathBuf::from("logs-relative-test");

        let error = prepare_log_dir::<tauri::Error>(Ok(relative.clone())).unwrap_err();

        assert!(error.contains("göreli"), "{error}");
        assert!(!relative.exists());
    }

    #[test]
    fn prepare_log_dir_reports_uncreatable_directory() {
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("not-a-directory");
        std::fs::write(&blocker, b"file").unwrap();

        let error = prepare_log_dir::<tauri::Error>(Ok(blocker.join("logs"))).unwrap_err();

        assert!(error.contains("oluşturulamadı"), "{error}");
    }

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
        let (success, _) = run_child_with_timeout(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("exit 0"),
            std::time::Duration::from_secs(1),
        )
        .unwrap();
        assert!(success.success());

        let (failure, stderr) = run_child_with_timeout(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("echo 'Paylaşım başarısız: test' >&2; exit 1"),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(!failure.success());
        assert_eq!(
            share_exit_error(failure, &stderr),
            format!("macOS Paylaşım menüsü {failure} durumuyla kapandı: Paylaşım başarısız: test")
        );

        let timeout = run_child_with_timeout(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("sleep 5"),
            std::time::Duration::from_millis(10),
        );
        assert!(timeout.unwrap_err().contains("zaman aşımı"));
    }

    fn image_frame_bytes(response: tauri::ipc::Response) -> Vec<u8> {
        use tauri::ipc::{InvokeResponseBody, IpcResponse};
        match response.body().unwrap() {
            InvokeResponseBody::Raw(bytes) => bytes,
            InvokeResponseBody::Json(json) => panic!("image returned JSON: {json:.64}"),
        }
    }

    /// 2x3 PNG whose pixels encode their own (x, y) position.
    fn write_test_png(directory: &Path) -> std::path::PathBuf {
        let path = directory.join("source.png");
        image::RgbaImage::from_fn(2, 3, |x, y| {
            image::Rgba([x as u8 * 100, y as u8 * 50, 7, 255])
        })
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();
        path
    }

    fn pixel(x: u8, y: u8) -> [u8; 4] {
        [x * 100, y * 50, 7, 255]
    }

    #[tokio::test]
    async fn read_image_file_frame_matches_previous_rgba_output() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_test_png(directory.path());
        let path = path.to_str().unwrap().to_string();

        let frame = image_frame_bytes(
            read_image_file(path.clone(), 0, false, false)
                .await
                .unwrap(),
        );
        let decoded = pdf_engine::render_wire::decode_render_frame(&frame).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 3));
        assert_eq!((decoded.page_index, decoded.generation), (0, 0));
        assert_eq!(decoded.request_id, IMAGE_FRAME_REQUEST_ID);
        let expected: Vec<u8> = [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)]
            .into_iter()
            .flat_map(|(x, y)| pixel(x, y))
            .collect();
        assert_eq!(*decoded.data, expected);

        // rotate90 then fliph: a 3x2 image whose rows read the source columns top-down.
        let frame = image_frame_bytes(
            read_image_file(path.clone(), 450, true, false)
                .await
                .unwrap(),
        );
        let decoded = pdf_engine::render_wire::decode_render_frame(&frame).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        let expected: Vec<u8> = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]
            .into_iter()
            .flat_map(|(x, y)| pixel(x, y))
            .collect();
        assert_eq!(*decoded.data, expected);

        let previous = decode_image_rgba(&path, 450, true, false).unwrap();
        assert_eq!(
            (previous.width, previous.height, previous.data),
            (3, 2, expected)
        );
    }

    #[tokio::test]
    async fn read_image_file_reports_missing_and_corrupt_files() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.png");
        let error = read_image_file(missing.to_str().unwrap().to_string(), 0, false, false)
            .await
            .err()
            .unwrap();
        assert!(error.starts_with("Görsel dosyası bulunamadı"), "{error}");

        let corrupt = directory.path().join("corrupt.png");
        std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\nnot really a png").unwrap();
        let error = read_image_file(corrupt.to_str().unwrap().to_string(), 0, false, false)
            .await
            .err()
            .unwrap();
        assert!(error.starts_with("Görsel okunamadı"), "{error}");
    }

    #[tokio::test]
    async fn convert_and_save_image_output_matches_sync_conversion() {
        let directory = tempfile::tempdir().unwrap();
        let source = write_test_png(directory.path());
        let source = source.to_str().unwrap().to_string();
        for format in ["png", "bmp", "pdf"] {
            let async_target = directory.path().join(format!("async.{format}"));
            let sync_target = directory.path().join(format!("sync.{format}"));
            convert_and_save_image_outside(
                Path::new(UNRELATED_PRIVATE_ROOT),
source.clone(),
                async_target.to_str().unwrap().to_string(),
                format.to_uppercase(),
                90,
                false,
                true,
            )
            .await
            .unwrap();
            convert_image_file(
                &source,
                sync_target.to_str().unwrap(),
                format,
                90,
                false,
                true,
            )
            .unwrap();
            assert_eq!(
                std::fs::read(&async_target).unwrap(),
                std::fs::read(&sync_target).unwrap(),
                "{format}"
            );
        }
        let reopened = image::open(directory.path().join("async.png"))
            .unwrap()
            .to_rgba8();
        assert_eq!(reopened.dimensions(), (3, 2));
    }

    #[tokio::test]
    async fn convert_and_save_image_reports_errors_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let corrupt = directory.path().join("corrupt.png");
        std::fs::write(&corrupt, b"not an image").unwrap();
        let target = directory.path().join("out.png");
        let error = convert_and_save_image_outside(
            Path::new(UNRELATED_PRIVATE_ROOT),
corrupt.to_str().unwrap().to_string(),
            target.to_str().unwrap().to_string(),
            "png".to_string(),
            0,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("Görsel açılamadı"), "{error}");
        assert!(!target.exists());

        let source = write_test_png(directory.path());
        let error = convert_and_save_image_outside(
            Path::new(UNRELATED_PRIVATE_ROOT),
source.to_str().unwrap().to_string(),
            target.to_str().unwrap().to_string(),
            "tiff".to_string(),
            0,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "Desteklenmeyen format: tiff");
        assert!(!target.exists());
    }

    /// Does not exist anywhere, so it never covers a test target.
    const UNRELATED_PRIVATE_ROOT: &str = "/nonexistent/antigravity-private-root";

    fn leftover_write_temps(directory: &Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".antigravity-write-"))
            })
            .collect()
    }

    #[tokio::test]
    async fn convert_and_save_image_replaces_existing_target_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let source = write_test_png(directory.path());
        for format in ["png", "pdf"] {
            let target = directory.path().join(format!("existing.{format}"));
            std::fs::write(&target, b"previous contents").unwrap();
            convert_and_save_image_outside(
                Path::new(UNRELATED_PRIVATE_ROOT),
                source.to_str().unwrap().to_string(),
                target.to_str().unwrap().to_string(),
                format.to_string(),
                0,
                false,
                false,
            )
            .await
            .unwrap();
            let written = std::fs::read(&target).unwrap();
            assert_ne!(written, b"previous contents", "{format}");
            if format == "pdf" {
                assert!(written.starts_with(b"%PDF-1.5"));
            } else {
                assert_eq!(image::open(&target).unwrap().to_rgba8().dimensions(), (2, 3));
            }
        }
        assert!(leftover_write_temps(directory.path()).is_empty());
    }

    #[tokio::test]
    async fn convert_and_save_image_onto_its_own_source_is_safe() {
        let directory = tempfile::tempdir().unwrap();
        let source = write_test_png(directory.path());
        let path = source.to_str().unwrap().to_string();

        convert_and_save_image_outside(
            Path::new(UNRELATED_PRIVATE_ROOT),
            path.clone(),
            path.clone(),
            "png".to_string(),
            90,
            false,
            false,
        )
        .await
        .unwrap();

        // The whole source was decoded before the target was replaced, so the
        // result is the complete rotated image rather than a truncated file.
        let rotated = image::open(&source).unwrap().to_rgba8();
        assert_eq!(rotated.dimensions(), (3, 2));
        // rotate90: destination (x, y) comes from source (y, height - 1 - x).
        assert_eq!(rotated.get_pixel(0, 0).0, pixel(0, 2));
        assert_eq!(rotated.get_pixel(2, 1).0, pixel(1, 0));
        assert!(leftover_write_temps(directory.path()).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn convert_and_save_image_keeps_existing_target_when_directory_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let source = write_test_png(directory.path());
        let locked = directory.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let targets = [
            (locked.join("out.png"), "png", "Görsel kaydetme hatası"),
            (locked.join("out.pdf"), "pdf", "PDF olarak kaydetme hatası"),
        ];
        for (target, _, _) in &targets {
            std::fs::write(target, b"original contents").unwrap();
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Privileged runs (root, permissive ACLs) can still create files here.
        let probe = locked.join("probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipped: read-only directory is still writable for this user");
            return;
        }

        let mut outcomes = Vec::new();
        for (target, format, _) in &targets {
            outcomes.push(
                convert_and_save_image_outside(
                    Path::new(UNRELATED_PRIVATE_ROOT),
                    source.to_str().unwrap().to_string(),
                    target.to_str().unwrap().to_string(),
                    format.to_string(),
                    0,
                    false,
                    false,
                )
                .await,
            );
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        for ((target, format, prefix), outcome) in targets.iter().zip(outcomes) {
            let error = outcome.unwrap_err();
            assert!(error.starts_with(prefix), "{format}: {error}");
            assert_eq!(std::fs::read(target).unwrap(), b"original contents", "{format}");
        }
        assert!(leftover_write_temps(&locked).is_empty());
    }

    #[tokio::test]
    async fn convert_and_save_image_rejects_session_temp_root_targets() {
        let directory = tempfile::tempdir().unwrap();
        let source = write_test_png(directory.path());
        // Like the session store's temp_root, this comes from
        // std::env::temp_dir() unresolved (on macOS: under the /var symlink).
        let private = tempfile::tempdir().unwrap();
        let nested = private.path().join("instance-x").join("sessions");
        std::fs::create_dir_all(&nested).unwrap();
        let existing = nested.join("working.png");
        std::fs::write(&existing, b"working copy").unwrap();
        let canonical_root = private.path().canonicalize().unwrap();

        let targets = [
            private.path().join("out.png"),
            nested.join("out.pdf"),
            existing.clone(),
            canonical_root.join("instance-x").join("resolved.png"),
        ];
        for root in [private.path(), canonical_root.as_path()] {
            for target in &targets {
                let format = target.extension().unwrap().to_str().unwrap().to_string();
                let error = convert_and_save_image_outside(
                    root,
                    source.to_str().unwrap().to_string(),
                    target.to_str().unwrap().to_string(),
                    format,
                    0,
                    false,
                    false,
                )
                .await
                .unwrap_err();
                assert_eq!(
                    error, "Session/job temp root'u Save veya Save As hedefi olamaz",
                    "{}",
                    target.display()
                );
            }
        }
        assert_eq!(std::fs::read(&existing).unwrap(), b"working copy");
        assert!(!private.path().join("out.png").exists());
        assert!(!nested.join("out.pdf").exists());
        assert!(!canonical_root.join("instance-x").join("resolved.png").exists());
    }

    /// Header-only 24-bit BMP: the decoder reports these dimensions without
    /// any pixel data being present or allocated.
    fn write_bmp_header(path: &Path, width: i32, height: i32) {
        let mut header = Vec::with_capacity(54);
        header.extend_from_slice(b"BM");
        header.extend_from_slice(&54u32.to_le_bytes()); // file size (header only)
        header.extend_from_slice(&0u32.to_le_bytes()); // reserved
        header.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
        header.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER size
        header.extend_from_slice(&width.to_le_bytes());
        header.extend_from_slice(&height.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes()); // planes
        header.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
        header.extend_from_slice(&[0; 24]); // BI_RGB, sizes, resolution, palette
        std::fs::write(path, header).unwrap();
    }

    #[tokio::test]
    async fn oversized_images_are_rejected_before_decoding() {
        let directory = tempfile::tempdir().unwrap();
        // Too wide: 16385 px exceeds the per-side limit.
        let wide = directory.path().join("wide.bmp");
        write_bmp_header(&wide, MAX_IMAGE_DIMENSION as i32 + 1, 1);
        // Within both side limits and the 24-bit decoder buffer (~192 MiB),
        // but the RGBA output (16384 × 4097 × 4 bytes) exceeds 256 MiB.
        let rgba_heavy = directory.path().join("rgba-heavy.bmp");
        write_bmp_header(&rgba_heavy, MAX_IMAGE_DIMENSION as i32, 4097);
        const { assert!(16_384u64 * 4097 * 3 <= MAX_IMAGE_BYTES) };
        const { assert!(16_384u64 * 4097 * 4 > MAX_IMAGE_BYTES) };

        for source in [&wide, &rgba_heavy] {
            let source = source.to_str().unwrap().to_string();
            let error = read_image_file(source.clone(), 0, false, false)
                .await
                .err()
                .unwrap();
            assert_eq!(error, image_too_large_error(), "{source}");

            let target = directory.path().join("out.png");
            let error = convert_and_save_image_outside(
                Path::new(UNRELATED_PRIVATE_ROOT),
source.clone(),
                target.to_str().unwrap().to_string(),
                "png".to_string(),
                0,
                false,
                false,
            )
            .await
            .unwrap_err();
            assert_eq!(error, image_too_large_error(), "{source}");
            assert!(!target.exists());
        }
        assert!(image_too_large_error().contains("16384×16384"));
        assert!(image_too_large_error().contains("256 MiB"));
    }

    #[test]
    fn images_at_the_side_limit_still_decode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("edge.png");
        image::GrayImage::new(MAX_IMAGE_DIMENSION, 1)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode_image_rgba(path.to_str().unwrap(), 0, false, false).unwrap();
        assert_eq!((decoded.width, decoded.height), (MAX_IMAGE_DIMENSION, 1));
        assert_eq!(decoded.data.len(), MAX_IMAGE_DIMENSION as usize * 4);
    }

    /// On a current-thread runtime the job can only observe the signal if it
    /// runs on a separate blocking thread while the runtime thread keeps going.
    #[tokio::test(flavor = "current_thread")]
    async fn image_jobs_do_not_block_the_async_runtime_thread() {
        let runtime_thread = std::thread::current().id();
        let (signal_tx, signal_rx) = std::sync::mpsc::channel::<()>();
        let job = tokio::spawn(run_image_job(move || {
            if std::thread::current().id() == runtime_thread {
                return Err("job ran on the runtime thread".to_string());
            }
            signal_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|_| "runtime thread was blocked".to_string())
        }));
        tokio::task::yield_now().await;
        signal_tx.send(()).unwrap();
        job.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn image_job_panics_become_errors() {
        let error = run_image_job::<(), _>(|| panic!("decoder panic"))
            .await
            .unwrap_err();
        assert!(error.starts_with("Görsel işlemi tamamlanamadı"), "{error}");
    }
}
