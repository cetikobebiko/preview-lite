// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.16.1, crates/optimize/src/lib.rs (MIT OR Apache-2.0, used under MIT).
// Changes from the original: rewritten on lopdf 0.46 (PdfCraft's own object layer is not used);
// black-and-white compression (CCITT, JBIG2), merging image fragments, link clean-up, discarding
// tags and print settings, removing an ASCII wrapper, LZW to Flate (lopdf's LZW decoder is not
// strict) and target-version choice are not ported;
// the copy is written from PDFium's plain copy of the snapshot and checked with PDFium.

//! PDF Optimizer (Acrobat: Reduce File Size / Optimize PDF), as a "save an optimized copy" job.
//!
//! - **Images:** each colour or grey image's effective resolution is measured where the pages
//!   draw it ([`walk`]); images above a threshold are resampled (bicubic) to a target resolution
//!   and stored as JPEG or Flate ([`images`]). A new image replaces the old one only if smaller.
//! - **Clean up** ([`cleanup`]): images and forms a page lists but never draws, page thumbnails
//!   and alternate images go; streams without a filter are Flate-compressed.
//! - **Object streams:** non-stream objects are packed into compressed object streams.
//!
//! Like `document_security`, PDFium opens the snapshot with the session's credential and writes
//! a plain full copy into the job directory; lopdf only reads, changes and serializes that copy
//! (AGENTS.md: structural writing), never the user's file. The output is opened again with
//! PDFium: page count, page sizes and the number of page objects must match the source.
//! The copy is a full rewrite, so signatures are invalid in it; the session's file is unchanged.

mod cleanup;
mod images;
mod walk;

use crate::incremental_save::{save_copy, RawDocument, RawPage};
use lopdf::Object;
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_DWORD};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// `FPDF_SaveAsCopy`: full rewrite.
const FPDF_NO_INCREMENTAL: FPDF_DWORD = 2;
/// Lowest and highest resolution a setting may ask for (ppi).
const MIN_PPI: f64 = 9.0;
const MAX_PPI: f64 = 2400.0;

/// How resampled or recompressed images are stored.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageCompression {
    /// JPEG with a quality from 1 to 100.
    Jpeg { quality: u8 },
    /// Lossless (zlib).
    Flate,
    /// Keep the image's own compression (resampled images become Flate if they weren't JPEG).
    Retain,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageSettings {
    /// Resample images whose effective resolution is above `above_ppi` down to `target_ppi`.
    pub downsample: bool,
    pub target_ppi: f64,
    pub above_ppi: f64,
    pub compression: ImageCompression,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizeSettings {
    pub color: ImageSettings,
    pub gray: ImageSettings,
    pub remove_unused_xobjects: bool,
    pub discard_thumbnails: bool,
    pub discard_alternate_images: bool,
    /// Flate-compress streams stored without a filter.
    pub flate_unencoded: bool,
    /// Pack non-stream objects into compressed object streams (PDF 1.5).
    pub object_streams: bool,
}

/// What an optimization did.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeReport {
    /// Size of the session's document and of the copy, in bytes.
    pub input_bytes: u64,
    pub output_bytes: u64,
    /// Colour and grey images the pages draw, and how many of them changed.
    pub images: usize,
    pub images_resampled: usize,
    pub images_recompressed: usize,
    /// Encoded bytes of the images that changed, before and after.
    pub image_bytes_before: u64,
    pub image_bytes_after: u64,
    pub unused_xobjects: usize,
    pub thumbnails: usize,
    pub alternate_images: usize,
    pub streams_compressed: usize,
    /// The document had signatures; they are invalid in the copy.
    pub signatures_invalidated: bool,
}

fn validate_images(settings: &ImageSettings, what: &str) -> Result<(), String> {
    if !(MIN_PPI..=MAX_PPI).contains(&settings.target_ppi)
        || !(MIN_PPI..=MAX_PPI).contains(&settings.above_ppi)
    {
        return Err(format!(
            "{what} görüntü çözünürlükleri {MIN_PPI}–{MAX_PPI} ppi aralığında olmalı"
        ));
    }
    if settings.above_ppi < settings.target_ppi {
        return Err(format!(
            "{what} görüntülerde eşik çözünürlüğü hedef çözünürlükten küçük olamaz"
        ));
    }
    if let ImageCompression::Jpeg { quality } = settings.compression {
        if !(1..=100).contains(&quality) {
            return Err(format!("{what} görüntülerde JPEG kalitesi 1–100 olmalı"));
        }
    }
    Ok(())
}

pub fn validate_request(settings: &OptimizeSettings) -> Result<(), String> {
    validate_images(&settings.color, "Renkli")?;
    validate_images(&settings.gray, "Gri tonlamalı")
}

/// Optimize `document` in place.
fn optimize(document: &mut lopdf::Document, settings: &OptimizeSettings) -> OptimizeReport {
    let mut report = OptimizeReport::default();
    let pages: Vec<lopdf::ObjectId> = document.get_pages().into_values().collect();
    images::run(document, &pages, settings, &mut report);
    if settings.remove_unused_xobjects {
        report.unused_xobjects = cleanup::remove_unused(document, &pages);
    }
    if settings.discard_thumbnails {
        report.thumbnails = cleanup::discard_thumbnails(document, &pages);
    }
    if settings.discard_alternate_images {
        report.alternate_images = cleanup::discard_alternate_images(document);
    }
    if settings.flate_unencoded {
        report.streams_compressed = cleanup::flate_unencoded(document);
    }
    report
}

/// Whether the document holds a signature dictionary (a signed signature field's value).
fn has_signatures(document: &lopdf::Document) -> bool {
    document.objects.values().any(|object| {
        let dict = match object {
            Object::Dictionary(dict) => dict,
            _ => return false,
        };
        matches!(dict.get(b"Type"), Ok(Object::Name(name)) if name == b"Sig")
            || (matches!(dict.get(b"FT"), Ok(Object::Name(name)) if name == b"Sig")
                && dict.has(b"V"))
    })
}

/// Each page's size (pt) and number of page objects, as PDFium reads them.
fn page_signatures(document: &RawDocument) -> Result<Vec<(i64, i64, i32)>, String> {
    let bindings = document.bindings;
    let count = document.page_count().map_err(|failure| failure.detail)?;
    let mut out = Vec::with_capacity(usize::from(count));
    for index in 0..u32::from(count) {
        let page = RawPage::load(document, index).map_err(|failure| failure.detail)?;
        // Hundredths of a point: the copy rewrites numbers, which may round differently.
        let width = (bindings.FPDF_GetPageWidthF(page.handle) * 100.0).round() as i64;
        let height = (bindings.FPDF_GetPageHeightF(page.handle) * 100.0).round() as i64;
        out.push((width, height, bindings.FPDFPage_CountObjects(page.handle)));
    }
    Ok(out)
}

/// Writes an optimized copy of `snapshot` to `output`. `credential` is the password the
/// session opened the document with.
pub(crate) fn write_optimized_copy(
    bindings: &dyn PdfiumLibraryBindings,
    snapshot: &Path,
    credential: Option<&str>,
    settings: &OptimizeSettings,
    output: &Path,
) -> Result<OptimizeReport, String> {
    validate_request(settings)?;
    let path = snapshot.to_str().ok_or("PDF yolu UTF-8 değil")?;
    let source = RawDocument::open(bindings, path, credential).ok_or_else(|| {
        format!(
            "PDF açılamadı (PDFium hata kodu {})",
            bindings.FPDF_GetLastError()
        )
    })?;
    if bindings.FPDF_GetSecurityHandlerRevision(source.handle) != -1 {
        // A plain copy would drop the protection; re-protecting needs the owner's settings.
        return Err("Parola korumalı belgeler optimize edilemez; önce Güvenlik ile korumasız bir kopya kaydedip onu optimize edin".into());
    }
    let expected = page_signatures(&source)?;
    let plain = output.with_file_name(format!(
        "{}.plain",
        output
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Çıktı adı geçersiz")?
    ));
    let written = save_copy(bindings, source.handle, &plain, FPDF_NO_INCREMENTAL)
        .map_err(|failure| failure.detail);
    drop(source);
    let result = written.and_then(|()| write_output(&plain, settings, output));
    let _ = std::fs::remove_file(&plain);
    let mut report = result?;
    verify_copy(bindings, output, &expected)?;
    report.input_bytes = std::fs::metadata(snapshot)
        .map_err(|error| format!("Belge boyutu okunamadı: {error}"))?
        .len();
    report.output_bytes = std::fs::metadata(output)
        .map_err(|error| format!("Kopya boyutu okunamadı: {error}"))?
        .len();
    Ok(report)
}

/// Optimizes PDFium's plain copy and serializes it to `output`.
fn write_output(
    plain: &Path,
    settings: &OptimizeSettings,
    output: &Path,
) -> Result<OptimizeReport, String> {
    let size = std::fs::metadata(plain)
        .map_err(|error| format!("Belge kopyası okunamadı: {error}"))?
        .len();
    if size > crate::document_structure::MAX_STRUCTURE_INPUT_BYTES as u64 {
        return Err("Belge optimizasyon için çok büyük (256 MiB sınırı)".into());
    }
    let mut document = lopdf::Document::load(plain)
        .map_err(|error| format!("Belge kopyası ayrıştırılamadı: {error}"))?;
    if crate::lopdf_compat::was_encrypted(&document) {
        return Err("PDFium şifresiz kopya yazamadı".into());
    }
    let mut report = optimize(&mut document, settings);
    report.signatures_invalidated = has_signatures(&document);
    document.prune_objects();
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(output).map_err(|error| format!("Kopya açılamadı: {error}"))?,
    );
    let saved = if settings.object_streams {
        document.save_modern(&mut file)
    } else {
        document.save_to(&mut file)
    };
    saved.map_err(|error| format!("Kopya yazılamadı: {error}"))?;
    let file = file
        .into_inner()
        .map_err(|error| format!("Kopya yazılamadı: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Kopya diske yazılamadı: {error}"))?;
    Ok(report)
}

fn verify_copy(
    bindings: &dyn PdfiumLibraryBindings,
    output: &Path,
    expected: &[(i64, i64, i32)],
) -> Result<(), String> {
    let path = output.to_str().ok_or("Çıktı yolu UTF-8 değil")?;
    let document = RawDocument::open(bindings, path, None)
        .ok_or("Optimize edilmiş kopya PDFium ile açılamadı")?;
    let actual = page_signatures(&document)?;
    if actual.len() != expected.len() {
        return Err(format!(
            "Kopyanın sayfa sayısı değişti ({} → {})",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (before, after)) in expected.iter().zip(&actual).enumerate() {
        if before.0 != after.0 || before.1 != after.1 {
            return Err(format!("Kopyada {}. sayfanın boyutu değişti", index + 1));
        }
        if before.2 != after.2 {
            return Err(format!(
                "Kopyada {}. sayfanın nesne sayısı değişti ({} → {})",
                index + 1,
                before.2,
                after.2
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
