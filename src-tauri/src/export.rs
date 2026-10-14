use pdfium_render::prelude::*;
use std::path::Path;
use std::process::Command;

use crate::document_session::{
    SessionJobCancellationCheckpoint, SessionJobChildProcessOwner, SessionJobOperationError,
};
use crate::pdf_engine::{get_pdfium, open_pdfium_doc};

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
