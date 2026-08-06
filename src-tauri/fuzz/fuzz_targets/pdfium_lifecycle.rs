#![no_main]
use libfuzzer_sys::fuzz_target;
use pdfium_render::prelude::*;
use std::sync::Once;

static INIT: Once = Once::new();
static mut PDFIUM: Option<Pdfium> = None;

fuzz_target!(|data: &[u8]| {
    unsafe {
        INIT.call_once(|| {
            // macOS default path or Linux/Windows fallback for pdfium dynamic library
            let bind = Pdfium::bind_to_system_library()
                .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")));
            if let Ok(b) = bind {
                PDFIUM = Some(Pdfium::new(b).unwrap());
            }
        });
    }

    let pdfium = unsafe {
        if let Some(p) = &PDFIUM {
            p
        } else {
            return;
        }
    };

    // FPDF_LoadMemDocument -> equivalent to load_pdf_from_byte_slice
    if let Ok(doc) = pdfium.load_pdf_from_byte_slice(data, None) {
        // FPDF_LoadPage -> equivalent to pages().get()
        if let Ok(pages) = doc.pages() {
            if pages.len() > 0 {
                // Test the first page, just as an example
                if let Ok(page) = pages.get(0) {
                    // FPDF_RenderPageBitmap -> render_with_config
                    let config = PdfRenderConfig::new()
                        .set_target_width(100)
                        .set_clear_color(PdfColor::WHITE);
                    let _ = page.render_with_config(&config);

                    // FPDFText_LoadPage / FPDFText_GetText -> page.text()
                    if let Ok(text) = page.text() {
                        let _ = text.all();
                    }
                }
            }
        }
    }
});
