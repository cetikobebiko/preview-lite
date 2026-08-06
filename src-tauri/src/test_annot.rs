use pdfium_render::prelude::*;

pub fn test() -> Result<(), PdfiumError> {
    let pdfium = Pdfium::default();
    let mut doc = pdfium.load_pdf_from_file("test.pdf", None)?;
    let mut page = doc.pages_mut().get(0)?;
    let mut annots = page.annotations_mut();
    
    // cause error to see methods
    let _: () = annots;
    
    Ok(())
}
