use pdfium_render::prelude::*;
use std::path::Path;
use std::process::Command;
use tauri::State;

use crate::document_session::{
    SessionJobCancellationCheckpoint, SessionJobChildProcessOwner, SessionJobOperationError,
};
use crate::pdf_engine::{get_cached_password, get_pdfium, open_pdfium_doc};
use crate::PasswordCache;

#[tauri::command]
pub fn print_pdf(
    file_path: String,
    page_range: Option<Vec<u32>>,
    cache: State<'_, PasswordCache>,
) -> Result<(), String> {
    let password = get_cached_password(&file_path, &cache);
    print_pdf_with_password(file_path, page_range, password, None, None)
        .map_err(|error| error.to_string())
}

pub(crate) fn print_pdf_with_password(
    file_path: String,
    page_range: Option<Vec<u32>>,
    password: Option<String>,
    cancellation: Option<&SessionJobCancellationCheckpoint>,
    child_processes: Option<&SessionJobChildProcessOwner>,
) -> Result<(), SessionJobOperationError> {
    if let Some(cancellation) = cancellation {
        cancellation.check_cancelled()?;
    }
    let mut print_path = file_path.clone();

    if let Some(pages) = page_range {
        if !pages.is_empty() {
            let pdfium = get_pdfium().map_err(SessionJobOperationError::from)?;
            let src_doc = open_pdfium_doc(&pdfium, &file_path, password.as_deref())
                .map_err(SessionJobOperationError::from)?;
            let mut temp_doc = pdfium.create_new_pdf().map_err(|e| {
                SessionJobOperationError::failed(format!("Geçici PDF oluşturulamadı: {:?}", e))
            })?;

            for (dest_index, page_index) in pages.into_iter().enumerate() {
                if let Some(cancellation) = cancellation {
                    cancellation.check_cancelled()?;
                }
                temp_doc
                    .pages_mut()
                    .copy_page_from_document(&src_doc, page_index as u16, dest_index as u16)
                    .map_err(|e| {
                        SessionJobOperationError::failed(format!("Sayfa kopyalama hatası: {:?}", e))
                    })?;
            }

            let temp_name = format!(
                "print_{}.pdf",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
            );
            let temp_file = std::env::temp_dir().join(temp_name);
            temp_doc.save_to_file(&temp_file).map_err(|e| {
                SessionJobOperationError::failed(format!("Geçici dosya kaydedilemedi: {:?}", e))
            })?;

            print_path = temp_file.to_string_lossy().to_string();
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("lpr");
        command.arg(&print_path);
        let status = match (child_processes, cancellation) {
            (Some(owner), Some(cancellation)) => owner.run_with_timeout(
                &mut command,
                std::time::Duration::from_secs(120),
                cancellation,
            )?,
            _ => command.status().map_err(|e| {
                SessionJobOperationError::failed(format!("Yazdırma komutu çalıştırılamadı: {e}"))
            })?,
        };
        if !status.success() {
            return Err(SessionJobOperationError::failed(
                "Yazdırma işlemi başarısız oldu",
            ));
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("lp");
        command.arg(&print_path);
        let status = match (child_processes, cancellation) {
            (Some(owner), Some(cancellation)) => owner.run_with_timeout(
                &mut command,
                std::time::Duration::from_secs(120),
                cancellation,
            )?,
            _ => command.status().map_err(|e| {
                SessionJobOperationError::failed(format!("Yazdırma komutu çalıştırılamadı: {e}"))
            })?,
        };
        if !status.success() {
            return Err(SessionJobOperationError::failed(
                "Yazdırma işlemi başarısız oldu",
            ));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("powershell");
        command.args(&[
            "-Command",
            &format!("Start-Process -FilePath \"{}\" -Verb Print", print_path),
        ]);
        let status = match (child_processes, cancellation) {
            (Some(owner), Some(cancellation)) => owner.run_with_timeout(
                &mut command,
                std::time::Duration::from_secs(120),
                cancellation,
            )?,
            _ => command.status().map_err(|e| {
                SessionJobOperationError::failed(format!("Yazdırma komutu çalıştırılamadı: {e}"))
            })?,
        };
        if !status.success() {
            return Err(SessionJobOperationError::failed(
                "Yazdırma işlemi başarısız oldu",
            ));
        }
    }

    Ok(())
}

pub(crate) fn submit_print_file(
    print_path: &Path,
    cancellation: &SessionJobCancellationCheckpoint,
    child_processes: &SessionJobChildProcessOwner,
) -> Result<(), SessionJobOperationError> {
    cancellation.check_cancelled()?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("lpr");
        command.arg(print_path);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("lp");
        command.arg(print_path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("powershell");
        command.args([
            "-Command",
            &format!(
                "Start-Process -FilePath \"{}\" -Verb Print",
                print_path.display()
            ),
        ]);
        command
    };
    let status = child_processes.run_with_timeout(
        &mut command,
        std::time::Duration::from_secs(120),
        cancellation,
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(SessionJobOperationError::failed(
            "Yazdırma işlemi başarısız oldu",
        ))
    }
}

#[tauri::command]
pub fn export_pages_as_images(
    file_path: String,
    pages: Vec<u32>,
    output_dir: String,
    format: String,
    dpi: f32,
    cache: State<'_, PasswordCache>,
) -> Result<Vec<String>, String> {
    let password = get_cached_password(&file_path, &cache);
    export_pages_as_images_with_password(file_path, pages, output_dir, format, dpi, password, None)
        .map_err(|error| error.to_string())
}

pub(crate) fn export_pages_as_images_with_password(
    file_path: String,
    pages: Vec<u32>,
    output_dir: String,
    format: String,
    dpi: f32,
    password: Option<String>,
    cancellation: Option<&SessionJobCancellationCheckpoint>,
) -> Result<Vec<String>, SessionJobOperationError> {
    let pdfium = get_pdfium().map_err(SessionJobOperationError::from)?;
    let document = open_pdfium_doc(&pdfium, &file_path, password.as_deref())
        .map_err(SessionJobOperationError::from)?;

    let mut generated_files = Vec::new();
    let doc_pages = document.pages();

    for page_idx in pages {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if page_idx >= doc_pages.len() as u32 {
            continue;
        }

        let page = doc_pages.get(page_idx as u16).map_err(|e| {
            SessionJobOperationError::failed(format!("Sayfa {} yüklenemedi: {:?}", page_idx, e))
        })?;

        let target_width = (page.width().value * (dpi / 72.0)) as i32;
        let render_config = PdfRenderConfig::new().set_target_width(target_width);

        let bitmap = page.render_with_config(&render_config).map_err(|e| {
            SessionJobOperationError::failed(format!("Sayfa render etme hatası: {:?}", e))
        })?;

        let img = bitmap.as_image();

        let ext = if format.to_lowercase() == "jpeg" {
            "jpg"
        } else {
            "png"
        };
        let out_path = Path::new(&output_dir).join(format!("sayfa_{}.{}", page_idx + 1, ext));

        img.save(&out_path).map_err(|e| {
            SessionJobOperationError::failed(format!("Görsel kaydedilemedi: {}", e))
        })?;

        generated_files.push(out_path.to_string_lossy().to_string());
    }

    Ok(generated_files)
}

#[tauri::command]
pub fn export_text_content(
    file_path: String,
    pages: Vec<u32>,
    format: String,
    cache: State<'_, PasswordCache>,
) -> Result<String, String> {
    let password = get_cached_password(&file_path, &cache);
    export_text_content_with_password(file_path, pages, format, password, None)
        .map_err(|error| error.to_string())
}

pub(crate) fn export_text_content_with_password(
    file_path: String,
    pages: Vec<u32>,
    format: String,
    password: Option<String>,
    cancellation: Option<&SessionJobCancellationCheckpoint>,
) -> Result<String, SessionJobOperationError> {
    let pdfium = get_pdfium().map_err(SessionJobOperationError::from)?;
    let document = open_pdfium_doc(&pdfium, &file_path, password.as_deref())
        .map_err(SessionJobOperationError::from)?;

    let mut out_text = String::new();
    let is_md = format.to_lowercase() == "markdown";
    let doc_pages = document.pages();

    for page_idx in pages {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }
        if page_idx >= doc_pages.len() as u32 {
            continue;
        }

        let page = doc_pages.get(page_idx as u16).map_err(|e| {
            SessionJobOperationError::failed(format!("Sayfa {} yüklenemedi: {:?}", page_idx, e))
        })?;

        let page_text = page.text().map(|t| t.all()).unwrap_or_default();

        if is_md {
            out_text.push_str(&format!("\n## Sayfa {}\n\n", page_idx + 1));
        } else {
            out_text.push_str(&format!("\n--- Sayfa {} ---\n\n", page_idx + 1));
        }

        out_text.push_str(&page_text);
    }

    Ok(out_text)
}
