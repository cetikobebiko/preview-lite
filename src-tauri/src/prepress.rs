use crate::pdf_engine::{get_pdfium, open_pdfium_doc};
use lopdf::{dictionary, Document, Object, StringFormat};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ColorProfileReport {
    pub rgb_count: u32,
    pub cmyk_count: u32,
    pub grayscale_count: u32,
    pub spot_count: u32,
    pub icc_profiles: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InkCoverageResult {
    pub width: u32,
    pub height: u32,
    pub heatmap_png_base64: String,
}

#[derive(Deserialize, Debug)]
pub struct PageBoxes {
    pub media_box: Option<[f32; 4]>,
    pub crop_box: Option<[f32; 4]>,
    pub bleed_box: Option<[f32; 4]>,
    pub trim_box: Option<[f32; 4]>,
    pub art_box: Option<[f32; 4]>,
}

// 1. Analyze Color Spaces
#[tauri::command]
pub fn analyze_color_spaces(file_path: String) -> Result<ColorProfileReport, String> {
    analyze_color_spaces_with_password(file_path, None).map_err(|error| error.to_string())
}

pub(crate) fn analyze_color_spaces_with_password(
    file_path: String,
    password: Option<String>,
) -> Result<ColorProfileReport, crate::document_session::SessionJobOperationError> {
    if password.is_some() {
        return Err(
            crate::document_session::SessionJobOperationError::failed_with_code(
                "SESSION_JOB_ENCRYPTED_INPUT_UNSUPPORTED",
                "Renk uzayı analizi şifreli session snapshot girdisini desteklemiyor",
            ),
        );
    }
    let doc = Document::load(&file_path).map_err(|e| format!("PDF belgesi okunamadı: {}", e))?;

    let mut rgb_count = 0;
    let mut cmyk_count = 0;
    let mut grayscale_count = 0;
    let mut spot_count = 0;

    for page_id in doc.get_pages().values() {
        if let Ok(page_dict) = doc.get_dictionary(*page_id) {
            if let Ok(resources) = page_dict.get(b"Resources").and_then(|r| {
                r.as_dict()
                    .or_else(|_| r.as_reference().and_then(|id| doc.get_dictionary(id)))
            }) {
                if let Ok(color_spaces) = resources.get(b"ColorSpace").and_then(|cs| {
                    cs.as_dict()
                        .or_else(|_| cs.as_reference().and_then(|id| doc.get_dictionary(id)))
                }) {
                    for (_, cs_obj) in color_spaces.iter() {
                        let name = match cs_obj {
                            Object::Name(n) => String::from_utf8_lossy(n).to_string(),
                            Object::Array(arr) => {
                                if arr.is_empty() {
                                    continue;
                                }
                                if let Object::Name(n) = &arr[0] {
                                    String::from_utf8_lossy(n).to_string()
                                } else {
                                    "".to_string()
                                }
                            }
                            _ => "".to_string(),
                        };

                        match name.as_str() {
                            "DeviceRGB" | "CalRGB" => rgb_count += 1,
                            "DeviceCMYK" => cmyk_count += 1,
                            "DeviceGray" | "CalGray" => grayscale_count += 1,
                            "Separation" | "DeviceN" => spot_count += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    Ok(ColorProfileReport {
        rgb_count,
        cmyk_count,
        grayscale_count,
        spot_count,
        icc_profiles: vec!["Default Profile".to_string()],
    })
}

// 2. Ink Coverage (TIC)
#[tauri::command]
pub fn calculate_ink_coverage(
    file_path: String,
    page_index: u32,
    max_limit: f32,
    scale: f32,
) -> Result<InkCoverageResult, String> {
    calculate_ink_coverage_with_cancellation(file_path, page_index, max_limit, scale, None, None)
        .map_err(|error| error.to_string())
}

pub(crate) fn calculate_ink_coverage_with_cancellation(
    file_path: String,
    page_index: u32,
    max_limit: f32,
    scale: f32,
    password: Option<String>,
    cancellation: Option<&crate::document_session::SessionJobCancellationCheckpoint>,
) -> Result<InkCoverageResult, crate::document_session::SessionJobOperationError> {
    if let Some(cancellation) = cancellation {
        cancellation.check_cancelled()?;
    }
    let pdfium = get_pdfium().map_err(crate::document_session::SessionJobOperationError::from)?;
    let doc = open_pdfium_doc(&pdfium, &file_path, password.as_deref())
        .map_err(crate::document_session::SessionJobOperationError::from)?;

    let pages = doc.pages();
    if page_index >= pages.len() as u32 {
        return Err(crate::document_session::SessionJobOperationError::failed(
            format!("Geçersiz sayfa numarası: {page_index}"),
        ));
    }
    let page = pages.get(page_index as u16).map_err(|e| {
        crate::document_session::SessionJobOperationError::failed(format!(
            "Sayfa yüklenemedi: {:?}",
            e
        ))
    })?;

    let target_width = (page.width().value * scale) as i32;
    let render_config = PdfRenderConfig::new().set_target_width(target_width);
    let bitmap = page.render_with_config(&render_config).map_err(|e| {
        crate::document_session::SessionJobOperationError::failed(format!(
            "Sayfa render edilemedi: {:?}",
            e
        ))
    })?;

    let rgba_img = bitmap.as_image().to_rgba8();
    let width = rgba_img.width();
    let height = rgba_img.height();

    let mut heatmap_img = image::ImageBuffer::<image::Rgba<u8>, _>::new(width, height);
    let limit_ratio = max_limit / 100.0;

    for y in 0..height {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        for x in 0..width {
            let pixel = rgba_img.get_pixel(x, y);
            let r = pixel[0] as f32 / 255.0;
            let g = pixel[1] as f32 / 255.0;
            let b = pixel[2] as f32 / 255.0;

            let k = 1.0 - r.max(g).max(b);
            let c = if k < 1.0 {
                (1.0 - r - k) / (1.0 - k)
            } else {
                0.0
            };
            let m = if k < 1.0 {
                (1.0 - g - k) / (1.0 - k)
            } else {
                0.0
            };
            let y_val = if k < 1.0 {
                (1.0 - b - k) / (1.0 - k)
            } else {
                0.0
            };

            let tic = c + m + y_val + k;
            if tic > limit_ratio {
                heatmap_img.put_pixel(x, y, image::Rgba([255, 0, 0, 180]));
            } else {
                heatmap_img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
    }

    let mut png_buffer = Vec::new();
    heatmap_img
        .write_to(
            &mut std::io::Cursor::new(&mut png_buffer),
            image::ImageFormat::Png,
        )
        .map_err(|e| {
            crate::document_session::SessionJobOperationError::failed(format!(
                "PNG oluşturma hatası: {}",
                e
            ))
        })?;

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let heatmap_png_base64 = STANDARD.encode(&png_buffer);

    Ok(InkCoverageResult {
        width,
        height,
        heatmap_png_base64,
    })
}

// 3. Render Separation Channels
#[derive(Debug, Deserialize, Serialize)]
pub struct SeparationResult {
    pub c_base64: String,
    pub m_base64: String,
    pub y_base64: String,
    pub k_base64: String,
}

#[tauri::command]
pub async fn render_separation_channels(
    file_path: String,
    page_index: u32,
    scale: f32,
) -> Result<SeparationResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        render_separation_channels_with_password(file_path, page_index, scale, None)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "Separation render worker tamamlanamadı".to_string())?
}

pub(crate) fn render_separation_channels_with_password(
    file_path: String,
    page_index: u32,
    scale: f32,
    password: Option<String>,
) -> Result<SeparationResult, crate::document_session::SessionJobOperationError> {
    let pdfium = get_pdfium().map_err(crate::document_session::SessionJobOperationError::from)?;
    let doc = open_pdfium_doc(&pdfium, &file_path, password.as_deref())
        .map_err(crate::document_session::SessionJobOperationError::from)?;

    let pages = doc.pages();
    if page_index >= pages.len() as u32 {
        return Err(crate::document_session::SessionJobOperationError::failed(
            format!("Geçersiz sayfa numarası: {page_index}"),
        ));
    }
    let page = pages.get(page_index as u16).map_err(|e| {
        crate::document_session::SessionJobOperationError::failed(format!(
            "Sayfa yüklenemedi: {e:?}"
        ))
    })?;

    let target_width = (page.width().value * scale) as i32;
    let render_config = PdfRenderConfig::new().set_target_width(target_width);
    let bitmap = page.render_with_config(&render_config).map_err(|e| {
        crate::document_session::SessionJobOperationError::failed(format!(
            "Sayfa render edilemedi: {e:?}"
        ))
    })?;

    let rgba_img = bitmap.as_image().to_rgba8();
    let width = rgba_img.width();
    let height = rgba_img.height();

    let mut c_img = image::ImageBuffer::<image::Rgba<u8>, _>::new(width, height);
    let mut m_img = image::ImageBuffer::<image::Rgba<u8>, _>::new(width, height);
    let mut y_img = image::ImageBuffer::<image::Rgba<u8>, _>::new(width, height);
    let mut k_img = image::ImageBuffer::<image::Rgba<u8>, _>::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let pixel = rgba_img.get_pixel(x, y);
            let r = pixel[0] as f32 / 255.0;
            let g = pixel[1] as f32 / 255.0;
            let b = pixel[2] as f32 / 255.0;

            let k_val = 1.0 - r.max(g).max(b);
            let c_val = if k_val < 1.0 {
                (1.0 - r - k_val) / (1.0 - k_val)
            } else {
                0.0
            };
            let m_val = if k_val < 1.0 {
                (1.0 - g - k_val) / (1.0 - k_val)
            } else {
                0.0
            };
            let y_val = if k_val < 1.0 {
                (1.0 - b - k_val) / (1.0 - k_val)
            } else {
                0.0
            };

            let c_u8 = (c_val * 255.0) as u8;
            let m_u8 = (m_val * 255.0) as u8;
            let y_u8 = (y_val * 255.0) as u8;
            let k_u8 = (k_val * 255.0) as u8;

            if c_u8 > 0 {
                c_img.put_pixel(x, y, image::Rgba([0, 255 - c_u8, 255 - c_u8, c_u8]));
            }
            if m_u8 > 0 {
                m_img.put_pixel(x, y, image::Rgba([255 - m_u8, 0, 255 - m_u8, m_u8]));
            }
            if y_u8 > 0 {
                y_img.put_pixel(x, y, image::Rgba([255 - y_u8, 255 - y_u8, 0, y_u8]));
            }
            if k_u8 > 0 {
                k_img.put_pixel(x, y, image::Rgba([0, 0, 0, k_u8]));
            }
        }
    }

    let mut c_buf = Vec::new();
    let mut m_buf = Vec::new();
    let mut y_buf = Vec::new();
    let mut k_buf = Vec::new();

    for (image, buffer) in [
        (&c_img, &mut c_buf),
        (&m_img, &mut m_buf),
        (&y_img, &mut y_buf),
        (&k_img, &mut k_buf),
    ] {
        image
            .write_to(&mut std::io::Cursor::new(buffer), image::ImageFormat::Png)
            .map_err(|error| {
                crate::document_session::SessionJobOperationError::failed(format!(
                    "Separation PNG üretilemedi: {error}"
                ))
            })?;
    }

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    Ok(SeparationResult {
        c_base64: STANDARD.encode(&c_buf),
        m_base64: STANDARD.encode(&m_buf),
        y_base64: STANDARD.encode(&y_buf),
        k_base64: STANDARD.encode(&k_buf),
    })
}

// 4. Set Page Boxes
#[tauri::command]
pub fn set_page_boxes(
    source_path: String,
    target_path: String,
    page_index: u32,
    boxes: PageBoxes,
) -> Result<(), String> {
    let mut doc = Document::load(&source_path).map_err(|e| format!("PDF okuma hatası: {}", e))?;

    let pages = doc.get_pages();
    let page_id = *pages.get(&(page_index + 1)).ok_or("Sayfa bulunamadı")?;

    let to_array = |arr: [f32; 4]| -> Object {
        Object::Array(vec![
            Object::Real(arr[0]),
            Object::Real(arr[1]),
            Object::Real(arr[2]),
            Object::Real(arr[3]),
        ])
    };

    if let Ok(page_dict) = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut()) {
        if let Some(b) = boxes.media_box {
            page_dict.set("MediaBox", to_array(b));
        }
        if let Some(b) = boxes.crop_box {
            page_dict.set("CropBox", to_array(b));
        }
        if let Some(b) = boxes.bleed_box {
            page_dict.set("BleedBox", to_array(b));
        }
        if let Some(b) = boxes.trim_box {
            page_dict.set("TrimBox", to_array(b));
        }
        if let Some(b) = boxes.art_box {
            page_dict.set("ArtBox", to_array(b));
        }
    }

    doc.save(&target_path)
        .map_err(|e| format!("Kutu güncellemeleri kaydedilemedi: {}", e))?;
    Ok(())
}

// 5. PDF/X Export
pub fn export_pdf_x_internal(
    source_path: String,
    target_path: String,
    standard: String,
    profile_name: String,
    password: Option<String>,
) -> Result<(), crate::document_session::SessionJobOperationError> {
    if password.is_some() {
        return Err(
            crate::document_session::SessionJobOperationError::failed_with_code(
                "SESSION_JOB_ENCRYPTED_INPUT_UNSUPPORTED",
                "PDF/X export şifreli session snapshot girdisini desteklemiyor",
            ),
        );
    }
    let mut doc = Document::load(&source_path).map_err(|e| format!("PDF okuma hatası: {}", e))?;

    if standard.contains("X-1a") {
        doc.version = "1.3".to_string();
    } else {
        doc.version = "1.6".to_string();
    }

    let gts_version = if standard.contains("X-1a") {
        "PDF/X-1a:2001"
    } else {
        "PDF/X-4"
    };
    let info_id = doc.trailer.get(b"Info").and_then(|o| o.as_reference()).ok();

    if let Some(id) = info_id {
        if let Ok(info_dict) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
            info_dict.set(
                "GTS_PDFXVersion",
                Object::String(gts_version.as_bytes().to_vec(), StringFormat::Literal),
            );
        }
    } else {
        let dict = dictionary! {
            "GTS_PDFXVersion" => Object::String(gts_version.as_bytes().to_vec(), StringFormat::Literal),
        };
        let new_id = doc.add_object(dict);
        doc.trailer.set("Info", new_id);
    }

    let intent_dict = dictionary! {
        "Type" => "OutputIntent",
        "S" => "GTS_PDFX",
        "OutputConditionIdentifier" => Object::String(profile_name.as_bytes().to_vec(), StringFormat::Literal),
        "RegistryName" => Object::String(b"http://www.color.org".to_vec(), StringFormat::Literal),
        "Info" => Object::String(profile_name.as_bytes().to_vec(), StringFormat::Literal),
    };

    let intent_id = doc.add_object(intent_dict);

    if let Ok(catalog_id) = doc.trailer.get(b"Root").and_then(|o| o.as_reference()) {
        if let Ok(catalog) = doc.get_object_mut(catalog_id).and_then(|o| o.as_dict_mut()) {
            let intents = vec![Object::Reference(intent_id)];
            catalog.set("OutputIntents", Object::Array(intents));
        }
    }

    doc.save(&target_path)
        .map_err(|e| format!("PDF/X dışa aktarma hatası: {}", e))?;
    Ok(())
}
