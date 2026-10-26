//! PDFium-independent atomic filesystem integration target.
//!
//! This compiles the production module in a dedicated test binary. Its nested
//! tests exercise replacement, cleanup, path identity, permissions, failure
//! injection, and same-target serialization without loading PDFium.

mod pdf_engine {
    use pdfium_render::prelude::Pdfium;

    pub(crate) fn with_pdfium<T, E>(
        _operation: impl FnOnce(&Pdfium) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<String>,
    {
        Err(E::from(
            "PDFium is deliberately unavailable in document_io_fs".to_string(),
        ))
    }
}

// `document_io` formats digests with `crate::hex_lower`.
use preview_alternative_lib::hex_lower;

#[path = "../src/document_io.rs"]
mod document_io;

#[test]
fn production_pdfium_entry_points_are_type_checked_but_not_executed_here() {
    use document_io::{DocumentIoFailure, TargetExpectation};
    use pdfium_render::prelude::{PdfDocument, Pdfium};
    use std::path::{Path, PathBuf};

    fn write(_pdfium: &Pdfium, _target: &Path) -> Result<(), DocumentIoFailure> {
        unreachable!("the filesystem-only target must not invoke PDFium")
    }

    fn validate(_document: &PdfDocument<'_>, _expected: &()) -> Result<(), DocumentIoFailure> {
        unreachable!("the filesystem-only target must not invoke PDFium")
    }

    if std::hint::black_box(false) {
        let source = PathBuf::from("unreachable-source.pdf");
        let target = Path::new("unreachable-target.pdf");
        let _ = document_io::ensure_unprotected;
        let _ = document_io::atomic_write_pdf("unreachable", &[], target, write, validate);
        let _ = document_io::atomic_copy_pdf(
            "unreachable",
            &source,
            target,
            Some(TargetExpectation::Missing),
            None,
        );
        let _ = document_io::atomic_copy_pdf(
            "unreachable",
            &source,
            target,
            Some(TargetExpectation::Digest("unreachable".to_string())),
            None,
        );
        #[cfg(feature = "secure-redaction-test-support")]
        let _ = document_io::atomic_publish_prevalidated_pdf_for_acceptance;
    }
}
