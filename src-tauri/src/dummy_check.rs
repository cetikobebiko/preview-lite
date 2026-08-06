use pdfium_render::prelude::*;
fn check_api() {
    let mut config = PdfRenderConfig::new();
    config = config.rotate(PdfBitmapRotation::Degrees90); 
}
