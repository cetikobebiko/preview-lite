use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PdfMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub page_index: u32,
    pub level: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchResult {
    pub page_index: u32,
    pub match_index: u32,
    pub snippet: String,
    pub is_ocr: bool,
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

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: Option<String>,
}

fn get_pdfium() -> Result<Pdfium, String> {
    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./src-tauri/lib"))
        .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./src-tauri")))
        .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./lib")))
        .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")))
        .or_else(|_| Pdfium::bind_to_system_library());

    match bindings {
        Ok(b) => Ok(Pdfium::new(b)),
        Err(e) => Err(format!("Pdfium sürücüsü yüklenemedi: {}", e)),
    }
}

#[tauri::command]
fn get_page_count(path: String) -> Result<u32, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("Dosya bulunamadı: {}", path));
    }

    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    if ext != "pdf" {
        return Ok(1);
    }

    if let Ok(doc) = Document::load(&path) {
        return Ok(doc.get_pages().len() as u32);
    }

    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("PDF okuma hatası: {}", e))?;
    Ok(document.pages().len() as u32)
}

#[tauri::command]
fn render_page(path: String, page_index: u32, scale: f32) -> Result<RenderedPage, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("PDF belgesi açılamadı: {}", e))?;

    let pages = document.pages();
    if page_index >= pages.len() as u32 {
        return Err(format!("Geçersiz sayfa numarası: {}", page_index));
    }

    let page = pages
        .get(page_index as u16)
        .map_err(|e| format!("Sayfa alınamadı: {}", e))?;

    let effective_scale = if scale <= 0.1 { 1.0 } else { scale };
    let target_width = (page.width().value * effective_scale * 1.5) as i32;
    
    let render_config = PdfRenderConfig::new().set_target_width(target_width);
    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("Sayfa render etme hatası: {}", e))?;

    let image = bitmap.as_image();
    let rgba_image = image.to_rgba8();

    Ok(RenderedPage {
        width: rgba_image.width(),
        height: rgba_image.height(),
        data: rgba_image.into_raw(),
    })
}

#[tauri::command]
fn render_thumbnail(path: String, page_index: u32, target_width: u32) -> Result<RenderedPage, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("PDF belgesi açılamadı: {}", e))?;

    let pages = document.pages();
    if page_index >= pages.len() as u32 {
        return Err(format!("Geçersiz sayfa numarası: {}", page_index));
    }

    let page = pages
        .get(page_index as u16)
        .map_err(|e| format!("Sayfa alınamadı: {}", e))?;

    let width = if target_width == 0 { 160 } else { target_width } as i32;
    let render_config = PdfRenderConfig::new().set_target_width(width);

    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("Minyatür render etme hatası: {}", e))?;

    let rgba_image = bitmap.as_image().to_rgba8();

    Ok(RenderedPage {
        width: rgba_image.width(),
        height: rgba_image.height(),
        data: rgba_image.into_raw(),
    })
}

// Görsele ve SVG'ye Dönüştürme (Export Pages to PNG, JPEG, SVG)
#[tauri::command]
fn export_pdf_page_to_image(
    path: String,
    page_index: u32,
    target_path: String,
    format: String,
) -> Result<(), String> {
    let rendered = render_page(path, page_index, 2.0)?;
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        rendered.width,
        rendered.height,
        rendered.data,
    ).ok_or("Görsel tamponu oluşturulamadı")?;

    let fmt = format.to_lowercase();
    if fmt == "svg" {
        let mut png_bytes = Vec::new();
        let dynamic_img = image::DynamicImage::ImageRgba8(img);
        dynamic_img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .map_err(|e| format!("PNG kodlama hatası: {}", e))?;

        let b64 = base64_encode(&png_bytes);
        let svg_content = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}">
  <image width="{}" height="{}" href="data:image/png;base64,{}"/>
</svg>"#,
            rendered.width, rendered.height, rendered.width, rendered.height, b64
        );
        std::fs::write(&target_path, svg_content).map_err(|e| format!("SVG dosyası yazılamadı: {}", e))?;
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

fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        s.push(CHARS[(triple >> 18) & 0x3F] as char);
        s.push(CHARS[(triple >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            s.push(CHARS[(triple >> 6) & 0x3F] as char);
        } else {
            s.push('=');
        }
        if chunk.len() > 2 {
            s.push(CHARS[triple & 0x3F] as char);
        } else {
            s.push('=');
        }
    }
    s
}

// Meta Veri (Metadata) Okuma ve Düzenleme
#[tauri::command]
fn get_pdf_metadata(path: String) -> Result<PdfMetadata, String> {
    let doc = Document::load(&path).map_err(|e| format!("PDF okuma hatası: {}", e))?;
    let mut meta = PdfMetadata::default();

    if let Ok(info_ref) = doc.trailer.get(b"Info").and_then(|obj| obj.as_reference()) {
        if let Ok(Object::Dictionary(dict)) = doc.get_object(info_ref) {
            if let Ok(title) = dict.get(b"Title").and_then(|o| o.as_str()) {
                meta.title = String::from_utf8_lossy(title).to_string();
            }
            if let Ok(author) = dict.get(b"Author").and_then(|o| o.as_str()) {
                meta.author = String::from_utf8_lossy(author).to_string();
            }
            if let Ok(subject) = dict.get(b"Subject").and_then(|o| o.as_str()) {
                meta.subject = String::from_utf8_lossy(subject).to_string();
            }
            if let Ok(keywords) = dict.get(b"Keywords").and_then(|o| o.as_str()) {
                meta.keywords = String::from_utf8_lossy(keywords).to_string();
            }
            if let Ok(creator) = dict.get(b"Creator").and_then(|o| o.as_str()) {
                meta.creator = String::from_utf8_lossy(creator).to_string();
            }
        }
    }
    Ok(meta)
}

#[tauri::command]
fn set_pdf_metadata(
    source_path: String,
    metadata: PdfMetadata,
    target_path: String,
) -> Result<(), String> {
    let mut doc = Document::load(&source_path).map_err(|e| format!("PDF okuma hatası: {}", e))?;

    let info_dict = dictionary! {
        "Title" => Object::String(metadata.title.into_bytes(), lopdf::StringFormat::Literal),
        "Author" => Object::String(metadata.author.into_bytes(), lopdf::StringFormat::Literal),
        "Subject" => Object::String(metadata.subject.into_bytes(), lopdf::StringFormat::Literal),
        "Keywords" => Object::String(metadata.keywords.into_bytes(), lopdf::StringFormat::Literal),
        "Creator" => Object::String(metadata.creator.into_bytes(), lopdf::StringFormat::Literal),
    };

    let info_id = doc.add_object(info_dict);
    doc.trailer.set("Info", info_id);

    doc.save(&target_path).map_err(|e| format!("Meta veri kaydedilemedi: {}", e))?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Cross-Platform Native API Adaptations (macOS vs Windows/Linux Fallbacks)
// -----------------------------------------------------------------------------

// 1. Süreklilik Kamerası (Continuity Camera / iPhone Camera Document Scan)
#[cfg(target_os = "macos")]
#[tauri::command]
fn trigger_continuity_camera_scan(source_pdf_path: String) -> Result<String, String> {
    let temp_scan_img = format!("/tmp/continuity_scan_{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());

    let _output = std::process::Command::new("swift")
        .arg("-e")
        .arg(format!(
            r#"
            import AppKit
            import AVCapture

            let savePath = "{}"
            let session = AVCaptureSession()
            session.sessionPreset = .photo

            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device) else {{
                exit(1)
            }}
            session.addInput(input)

            let output = AVCapturePhotoOutput()
            session.addOutput(output)
            session.startRunning()

            let settings = AVCapturePhotoSettings()
            struct Delegate: NSObject, AVCapturePhotoCaptureDelegate {{
                let path: String
                func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {{
                    if let data = photo.fileDataRepresentation() {{
                        try? data.write(to: URL(fileURLWithPath: path))
                    }}
                    exit(0)
                }}
            }}
            let delegate = Delegate(path: savePath)
            output.capturePhoto(with: settings, delegate: delegate)
            RunLoop.current.run(until: Date().addingTimeInterval(3.0))
            "#,
            temp_scan_img
        ))
        .output();

    if Path::new(&temp_scan_img).exists() {
        let temp_page_pdf = format!("/tmp/scanned_page_{}.pdf", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
        let img = image::open(&temp_scan_img).map_err(|e| e.to_string())?;
        save_image_as_pdf(&img, &temp_page_pdf)?;

        let target_merged_pdf = source_pdf_path.clone();
        merge_pdfs(vec![source_pdf_path, temp_page_pdf.clone()], target_merged_pdf.clone())?;

        let _ = std::fs::remove_file(&temp_scan_img);
        let _ = std::fs::remove_file(&temp_page_pdf);

        Ok("Kamera ile fiziksel belge tarandı ve yeni sayfa olarak eklendi!".to_string())
    } else {
        Err("Kamera veya iPhone Süreklilik Kamerası görüntüsü alınamadı.".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn trigger_continuity_camera_scan(_source_pdf_path: String) -> Result<String, String> {
    Err("iPhone Süreklilik Kamerası sadece macOS platformunda desteklenmektedir.".to_string())
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
        let _ = std::process::Command::new("powershell").arg("-Command").arg(ps_cmd).spawn();
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
        let _ = std::process::Command::new("taskkill").arg("/F").arg("/IM").arg("powershell.exe").output();
    }
    Ok(())
}

// 3. macOS Paylaşım Menüsü (Share Sheet)
#[cfg(target_os = "macos")]
#[tauri::command]
fn share_file_macos(file_path: String) -> Result<(), String> {
    if !Path::new(&file_path).exists() {
        return Err(format!("Paylaşılacak dosya bulunamadı: {}", file_path));
    }

    let output = std::process::Command::new("swift")
        .arg("-e")
        .arg(format!(
            r#"
            import AppKit

            let fileURL = URL(fileURLWithPath: "{}")
            let picker = NSSharingServicePicker(items: [fileURL])
            if let window = NSApplication.shared.windows.first {{
                picker.show(relativeTo: .zero, of: window.contentView!, preferredEdge: .minY)
            }}
            "#,
            file_path
        ))
        .output();

    match output {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("macOS Paylaşım menüsü açılamadı: {}", e)),
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn share_file_macos(_file_path: String) -> Result<(), String> {
    Err("macOS Paylaşım Menüsü (Share Sheet) yalnızca macOS üzerinde desteklenmektedir.".to_string())
}

// 4. Yerel OCR Engine (Apple Vision API / Native OCR Bridge)
#[cfg(target_os = "macos")]
#[tauri::command]
fn perform_ocr_on_page(path: String, page_index: u32) -> Result<String, String> {
    let rendered = render_page(path, page_index, 1.5)?;
    let temp_img_path = format!("/tmp/preview_ocr_page_{}.png", page_index);
    
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        rendered.width,
        rendered.height,
        rendered.data,
    ).ok_or("ImageData dönüşüm hatası")?;
    
    img.save(&temp_img_path).map_err(|e| format!("Geçici OCR resmi kaydedilemedi: {}", e))?;

    let output = std::process::Command::new("swift")
        .arg("-e")
        .arg(format!(
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
            temp_img_path
        ))
        .output();

    let _ = std::fs::remove_file(&temp_img_path);

    match output {
        Ok(out) if out.status.success() => {
            let recognized = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if recognized.is_empty() {
                Ok("Sayfada metin tespit edilemedi.".to_string())
            } else {
                Ok(recognized)
            }
        }
        _ => Err("macOS Apple Vision OCR yürütülemedi.".to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn perform_ocr_on_page(_path: String, _page_index: u32) -> Result<String, String> {
    Err("Apple Vision yerel OCR motoru sadece macOS üzerinde desteklenmektedir.".to_string())
}

// -----------------------------------------------------------------------------
// Cross-Platform Core Commands
// -----------------------------------------------------------------------------

// İçindekiler (Bookmarks / Outline Tree) Reading
fn traverse_bookmark(
    bookmark: &PdfBookmark,
    level: u32,
    result: &mut Vec<OutlineItem>,
) {
    let title = bookmark.title().unwrap_or_else(|| "Başlıksız".to_string());
    let page_index = bookmark
        .destination()
        .and_then(|dest| dest.page_index().ok())
        .unwrap_or(0) as u32;

    result.push(OutlineItem {
        title,
        page_index,
        level,
    });

    if let Some(child) = bookmark.first_child() {
        let mut current_child = Some(child);
        while let Some(c) = current_child {
            traverse_bookmark(&c, level + 1, result);
            current_child = c.next_sibling();
        }
    }
}

#[tauri::command]
fn get_pdf_outline(path: String) -> Result<Vec<OutlineItem>, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("PDF açılamadı: {}", e))?;

    let mut result = Vec::new();
    let bookmarks = document.bookmarks();

    for top_bookmark in bookmarks.iter() {
        traverse_bookmark(&top_bookmark, 0, &mut result);
    }

    Ok(result)
}

// Belge İçi Metin Arama (Scanned OCR Fallback desteği ile)
#[tauri::command]
fn search_pdf_text(path: String, query: String) -> Result<Vec<SearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("PDF açılamadı: {}", e))?;

    let mut results = Vec::new();
    let query_lower = query.to_lowercase();
    let mut match_idx = 0;

    for (i, page) in document.pages().iter().enumerate() {
        let mut full_text = String::new();
        let mut is_ocr = false;

        if let Ok(text_page) = page.text() {
            full_text = text_page.all();
        }

        if full_text.trim().is_empty() {
            if let Ok(ocr_text) = perform_ocr_on_page(path.clone(), i as u32) {
                full_text = ocr_text;
                is_ocr = true;
            }
        }

        let full_text_lower = full_text.to_lowercase();

        for (byte_offset, _) in full_text_lower.match_indices(&query_lower) {
            let start = byte_offset.saturating_sub(20);
            let end = (byte_offset + query_lower.len() + 20).min(full_text.len());
            let snippet = format!("...{}...", &full_text[start..end].replace('\n', " "));

            results.push(SearchResult {
                page_index: i as u32,
                match_index: match_idx,
                snippet,
                is_ocr,
            });
            match_idx += 1;
        }
    }

    Ok(results)
}

// Çevrimdışı LLM Entegrasyonu (Ollama / Local AI Bridge)
#[tauri::command]
fn ask_local_ai(model: Option<String>, prompt: String, context: String) -> Result<String, String> {
    let target_model = model.unwrap_or_else(|| "llama3".to_string());
    let full_prompt = if context.trim().is_empty() {
        prompt
    } else {
        format!(
            "Aşağıdaki belge metnini temel alarak soruyu Türkçe olarak yanıtla veya özetle:\n\n---\nBELGE METNİ:\n{}\n---\n\nKULLANICI İSTEMİ: {}",
            context, prompt
        )
    };

    let body = OllamaRequest {
        model: target_model.clone(),
        prompt: full_prompt,
        stream: false,
    };

    let response = ureq::post("http://localhost:11434/api/generate")
        .timeout(std::time::Duration::from_secs(60))
        .send_json(&body);

    match response {
        Ok(resp) => {
            let parsed: OllamaResponse = resp.into_json().map_err(|e| format!("Ollama yanıtı okunamadı: {}", e))?;
            parsed.response.ok_or_else(|| "Boş AI yanıtı alındı".to_string())
        }
        Err(_) => {
            Err(format!(
                "Yerel Ollama servisine (http://localhost:11434) bağlanılamadı.\nLütfen bilgisayarınızda Ollama'yı başlatın:\n'ollama run {}'",
                target_model
            ))
        }
    }
}

// PDF Sayfa Bölme (Extract Selected Pages)
#[tauri::command]
fn extract_pages(
    source_path: String,
    page_indices: Vec<u32>,
    target_path: String,
) -> Result<(), String> {
    if page_indices.is_empty() {
        return Err("Dışa aktarılacak sayfa seçilmedi".to_string());
    }

    reorder_pdf_pages(source_path, page_indices, target_path)
}

// Görsel İşleme Komutları
fn apply_image_transformations(
    mut img: image::DynamicImage,
    rotation: i32,
    flip_h: bool,
    flip_v: bool,
) -> image::DynamicImage {
    let normalized_angle = (rotation % 360 + 360) % 360;
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
    let content_stream = Stream::new(dictionary! {}, content.encode().unwrap());
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

    doc.save(target_path).map_err(|e| format!("PDF olarak kaydetme hatası: {}", e))?;
    Ok(())
}

#[tauri::command]
fn reorder_pdf_pages(
    source_path: String,
    page_order: Vec<u32>,
    target_path: String,
) -> Result<(), String> {
    let mut doc = Document::load(&source_path).map_err(|e| format!("PDF okuma hatası: {}", e))?;
    let pages = doc.get_pages();

    let mut new_kids = Vec::new();
    for &idx in &page_order {
        let page_num = idx + 1;
        if let Some(&page_id) = pages.get(&page_num) {
            new_kids.push(Object::Reference(page_id));
        }
    }

    if new_kids.is_empty() {
        return Err("PDF içerisinde kaydedilecek sayfa kalmadı".to_string());
    }

    let catalog = doc.catalog().map_err(|e| format!("PDF Katalog okunamadı: {}", e))?;
    let pages_dict_id = catalog
        .get(b"Pages")
        .and_then(|obj| obj.as_reference())
        .map_err(|e| format!("Pages objesi bulunamadı: {}", e))?;

    if let Ok(pages_obj) = doc.get_object_mut(pages_dict_id) {
        if let Object::Dictionary(ref mut dict) = pages_obj {
            dict.set("Kids", Object::Array(new_kids.clone()));
            dict.set("Count", Object::Integer(new_kids.len() as i64));
        }
    }

    doc.save(&target_path).map_err(|e| format!("PDF kaydedilemedi: {}", e))?;
    Ok(())
}

#[tauri::command]
fn delete_pdf_page(
    source_path: String,
    page_to_delete: u32,
    target_path: String,
) -> Result<(), String> {
    let doc = Document::load(&source_path).map_err(|e| format!("PDF okuma hatası: {}", e))?;
    let total_pages = doc.get_pages().len() as u32;

    if total_pages <= 1 {
        return Err("Tek sayfalı bir PDF'ten son sayfa silinemez".to_string());
    }

    let mut new_order = Vec::new();
    for i in 0..total_pages {
        if i != page_to_delete {
            new_order.push(i);
        }
    }

    reorder_pdf_pages(source_path, new_order, target_path)
}

#[tauri::command]
fn merge_pdfs(pdf_paths: Vec<String>, target_path: String) -> Result<(), String> {
    if pdf_paths.is_empty() {
        return Err("Birleştirilecek PDF dosyaları seçilmedi".to_string());
    }

    let mut max_id = 1;
    let mut merged_doc = Document::with_version("1.5");
    let pages_id = merged_doc.new_object_id();

    let mut all_kids = Vec::new();

    for path in &pdf_paths {
        let mut doc = Document::load(path).map_err(|e| format!("{} açılamadı: {}", path, e))?;
        
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        let pages = doc.get_pages();
        for (_, page_id) in pages {
            all_kids.push(Object::Reference(page_id));
        }

        for (obj_id, obj) in doc.objects {
            merged_doc.objects.insert(obj_id, obj);
        }
    }

    let pages_dict = dictionary! {
        "Type" => "Pages",
        "Kids" => all_kids.clone(),
        "Count" => all_kids.len() as i64,
    };
    merged_doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let catalog_id = merged_doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    merged_doc.trailer.set("Root", catalog_id);

    merged_doc.save(&target_path).map_err(|e| format!("Birleştirilen PDF kaydedilemedi: {}", e))?;
    Ok(())
}

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
fn burn_annotations_to_pdf(
    source_path: String,
    annotations_json: String,
    target_path: String,
) -> Result<(), String> {
    let items: Vec<AnnotationItem> = serde_json::from_str(&annotations_json)
        .map_err(|e| format!("Annotation JSON okuma hatası: {}", e))?;

    let mut doc = Document::load(&source_path).map_err(|e| format!("PDF yüklenemedi: {}", e))?;
    let pages = doc.get_pages();

    let mut page_map: HashMap<u32, Vec<&AnnotationItem>> = HashMap::new();
    for item in &items {
        page_map.entry(item.page_index).or_default().push(item);
    }

    for (page_idx, annos) in page_map {
        let page_num = page_idx + 1;
        if let Some(&page_id) = pages.get(&page_num) {
            let mut ops = vec![Operation::new("q", vec![])];

            for anno in annos {
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
                                ops.push(Operation::new("m", vec![first.x.into(), (ch - first.y).into()]));
                                for pt in &pts[1..] {
                                    ops.push(Operation::new("l", vec![pt.x.into(), (ch - pt.y).into()]));
                                }
                                ops.push(Operation::new("S", vec![]));
                            }
                        }
                    }
                    "rectangle" => {
                        if let (Some(x), Some(y), Some(w), Some(h)) = (anno.x, anno.y, anno.w, anno.h) {
                            let pdf_y = ch - y - h;
                            ops.push(Operation::new("re", vec![x.into(), pdf_y.into(), w.into(), h.into()]));
                            ops.push(Operation::new("S", vec![]));
                        }
                    }
                    "arrow" => {
                        if let (Some(sx), Some(sy), Some(ex), Some(ey)) = (anno.x, anno.y, anno.end_x, anno.end_y) {
                            let pdf_sy = ch - sy;
                            let pdf_ey = ch - ey;
                            ops.push(Operation::new("m", vec![sx.into(), pdf_sy.into()]));
                            ops.push(Operation::new("l", vec![ex.into(), pdf_ey.into()]));
                            ops.push(Operation::new("S", vec![]));
                        }
                    }
                    "text" => {
                        if let (Some(x), Some(y), Some(txt)) = (anno.x, anno.y, &anno.text) {
                            let pdf_y = ch - y - 12.0;
                            ops.push(Operation::new("BT", vec![]));
                            ops.push(Operation::new("Td", vec![x.into(), pdf_y.into()]));
                            ops.push(Operation::new("Tj", vec![Object::String(txt.as_bytes().to_vec(), lopdf::StringFormat::Literal)]));
                            ops.push(Operation::new("ET", vec![]));
                        }
                    }
                    _ => {}
                }
            }

            ops.push(Operation::new("Q", vec![]));

            let content = Content { operations: ops };
            let new_content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

            if let Ok(page_obj) = doc.get_object_mut(page_id) {
                if let Object::Dictionary(ref mut dict) = page_obj {
                    if let Ok(contents) = dict.get(b"Contents") {
                        let mut new_contents = Vec::new();
                        match contents {
                            Object::Array(arr) => {
                                new_contents.extend(arr.clone());
                                new_contents.push(Object::Reference(new_content_id));
                            }
                            Object::Reference(id) => {
                                new_contents.push(Object::Reference(*id));
                                new_contents.push(Object::Reference(new_content_id));
                            }
                            _ => {
                                new_contents.push(Object::Reference(new_content_id));
                            }
                        }
                        dict.set("Contents", Object::Array(new_contents));
                    } else {
                        dict.set("Contents", Object::Reference(new_content_id));
                    }
                }
            }
        }
    }

    doc.save(&target_path).map_err(|e| format!("PDF kaydedilemedi: {}", e))?;
    Ok(())
}

#[tauri::command]
fn read_binary_file(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_page_count,
            render_page,
            render_thumbnail,
            read_binary_file,
            read_image_file,
            convert_and_save_image,
            reorder_pdf_pages,
            delete_pdf_page,
            merge_pdfs,
            burn_annotations_to_pdf,
            get_pdf_outline,
            search_pdf_text,
            extract_pages,
            perform_ocr_on_page,
            ask_local_ai,
            trigger_continuity_camera_scan,
            speak_text,
            stop_speaking,
            share_file_macos,
            export_pdf_page_to_image,
            get_pdf_metadata,
            set_pdf_metadata
        ])
        .run(tauri::generate_context!())
        .expect("Tauri uygulaması başlatılırken hata oluştu");
}
