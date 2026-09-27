//! Independent observation of the internal secure-redaction artifact.
//!
//! This module intentionally does not call the transform builder. It is only
//! reached by a second private job-worker invocation after transform has
//! terminated, and reopens both the immutable snapshot and the serialized
//! artifact from disk.

use crate::document_session::SessionJobOperationError;
use crate::secure_redaction::{
    make_redaction_error, RedactionBlocker, RedactionVerificationCheck, SecureRedactionErrorCode,
    SecureRedactionPlan, SecureRedactionTransformReport, SecureRedactionVerificationReport,
    VerificationCheckStatus, MAX_SECURE_REDACTION_OUTPUT_BYTES, REDACTION_SCHEMA_VERSION,
    REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS, SECURE_REDACTION_VERIFICATION_VERSION,
};
use crate::secure_redaction_raster::{
    calculate_raster_dimensions, create_redaction_render_config, map_pdf_rect_to_device_pixel_rect,
    render_affected_page_raster, verify_opaque_black_overwrite, RedactionAnnotationPolicy,
    RedactionBackgroundPolicy, RedactionPixelFormat, RedactionRasterBuffer, RedactionRasterSpec,
    RedactionWidgetFormPolicy, MAX_RENDER_BYTES,
};
use pdfium_render::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const ARTIFACT_NAME: &str = crate::job_worker::SECURE_REDACTION_TRANSFORM_OUTPUT_NAME;

fn verification_error(message: impl Into<String>) -> SessionJobOperationError {
    make_redaction_error(
        SecureRedactionErrorCode::RedactionInvalidPolicy,
        message.into(),
    )
}

fn check(
    code: &str,
    status: VerificationCheckStatus,
    message: impl Into<String>,
) -> RedactionVerificationCheck {
    RedactionVerificationCheck {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        code: code.to_string(),
        status,
        message: message.into(),
        details: None,
    }
}

fn blocker(code: &str, message: impl Into<String>) -> RedactionBlocker {
    RedactionBlocker {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        code: code.to_string(),
        message: message.into(),
        details: None,
    }
}

fn artifact_path(job_dir: &Path) -> Result<PathBuf, SessionJobOperationError> {
    let dir_meta = fs::symlink_metadata(job_dir)
        .map_err(|e| verification_error(format!("Job directory metadata unavailable: {e}")))?;
    if !dir_meta.file_type().is_dir() || dir_meta.file_type().is_symlink() {
        return Err(verification_error(
            "Job directory is not a regular non-symlink directory",
        ));
    }
    let path = job_dir.join(ARTIFACT_NAME);
    let meta = fs::symlink_metadata(&path).map_err(|e| {
        verification_error(format!("Internal transform artifact is unavailable: {e}"))
    })?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() {
        return Err(verification_error(
            "Internal transform artifact is not a regular non-symlink file",
        ));
    }
    if meta.len() > MAX_SECURE_REDACTION_OUTPUT_BYTES {
        return Err(verification_error(
            "Internal transform artifact exceeds output size limit",
        ));
    }
    Ok(path)
}

/// Proves that the serialized artifact is a fresh, self-contained document.
/// This deliberately does not inspect the policy-level unsafe payloads handled
/// by `no_forbidden_document_structures()` below.
fn full_rewrite_structure(
    doc: &lopdf::Document,
    pdfium_page_count: usize,
    expected_page_count: usize,
) -> Result<(), SessionJobOperationError> {
    if doc.trailer.has(b"Prev") {
        return Err(verification_error(
            "Output trailer retains a prior-revision /Prev chain",
        ));
    }
    let root = doc
        .trailer
        .get(b"Root")
        .map_err(|_| verification_error("Output trailer has no /Root"))?;
    let root_id = root
        .as_reference()
        .map_err(|_| verification_error("Output trailer /Root is not an indirect catalog"))?;
    let catalog = doc
        .get_dictionary(root_id)
        .map_err(|_| verification_error("Output trailer /Root cannot be resolved"))?;
    if catalog
        .get(b"Type")
        .and_then(lopdf::Object::as_name)
        .map_err(|_| verification_error("Output root catalog has no /Type /Catalog"))?
        != b"Catalog"
    {
        return Err(verification_error(
            "Output root is not a /Catalog dictionary",
        ));
    }
    if catalog
        .get(b"Pages")
        .and_then(lopdf::Object::as_reference)
        .is_err()
    {
        return Err(verification_error(
            "Output catalog has no resolvable /Pages tree",
        ));
    }

    let pages = doc.get_pages();
    let page_ids: HashSet<_> = pages.values().copied().collect();
    if pages.len() != expected_page_count
        || pages.len() != pdfium_page_count
        || page_ids.len() != pages.len()
    {
        return Err(verification_error(
            "PDFium/lopdf page-tree count or page references disagree with the plan",
        ));
    }
    for page_id in page_ids {
        let page = doc
            .get_dictionary(page_id)
            .map_err(|_| verification_error("Page tree contains an unresolvable page"))?;
        if page
            .get(b"Type")
            .and_then(lopdf::Object::as_name)
            .map_err(|_| verification_error("Page tree member has no /Type /Page"))?
            != b"Page"
        {
            return Err(verification_error(
                "Page tree member is not a /Page dictionary",
            ));
        }
    }

    // lopdf traverses from the trailer and returns exactly the indirect objects
    // not reachable from the serialized document graph. Do not allow-list them:
    // a full rewrite artifact must not carry orphan payload.
    let mut pruned = doc.clone();
    let unreachable = pruned.prune_objects();
    if !unreachable.is_empty() {
        return Err(verification_error(format!(
            "Output contains {} unreachable indirect object(s)",
            unreachable.len()
        )));
    }
    Ok(())
}

/// Proves the absence of active/interactive/security payloads. Serialization
/// reachability is intentionally owned by `full_rewrite_structure()`.
fn no_forbidden_document_structures(doc: &lopdf::Document) -> Result<(), SessionJobOperationError> {
    if doc.trailer.has(b"Encrypt") {
        return Err(verification_error("Output trailer contains /Encrypt"));
    }
    let catalog = doc
        .catalog()
        .map_err(|_| verification_error("Output catalog cannot be resolved"))?;
    for key in [
        b"AcroForm".as_slice(),
        b"Names",
        b"OpenAction",
        b"AA",
        b"OCProperties",
        b"StructTreeRoot",
        b"Collection",
        b"Requirements",
        b"NeedsRendering",
        b"AF",
    ] {
        if catalog.has(key) {
            return Err(verification_error(format!(
                "Output catalog contains forbidden key /{}",
                String::from_utf8_lossy(key)
            )));
        }
    }
    for object in doc.objects.values() {
        if let lopdf::Object::Dictionary(dict) = object {
            for key in [
                b"ByteRange".as_slice(),
                b"XFA",
                b"EF",
                b"EmbeddedFiles",
                b"JavaScript",
                b"JS",
            ] {
                if dict.has(key) {
                    return Err(verification_error(format!(
                        "Output object graph contains forbidden key /{}",
                        String::from_utf8_lossy(key)
                    )));
                }
            }
            if let Ok(lopdf::Object::Name(kind)) = dict.get(b"Type") {
                if kind.as_slice() == b"Sig" {
                    return Err(verification_error("Output contains a signature dictionary"));
                }
            }
            if let Ok(lopdf::Object::Name(kind)) = dict.get(b"FT") {
                if kind.as_slice() == b"Sig" {
                    return Err(verification_error("Output contains a signature field"));
                }
            }
        }
    }
    Ok(())
}

fn affected_page_serialized_image_only(
    doc: &lopdf::Document,
    page_number: u32,
) -> Result<(), SessionJobOperationError> {
    let page_id = *doc
        .get_pages()
        .get(&page_number)
        .ok_or_else(|| verification_error("Affected page is absent from lopdf page tree"))?;
    let content = lopdf::content::Content::decode(
        &doc.get_page_content(page_id)
            .map_err(|_| verification_error("Affected page content stream cannot be decoded"))?,
    )
    .map_err(|_| verification_error("Affected page content operators cannot be parsed"))?;
    let mut invoked_name = None;
    for operation in &content.operations {
        match operation.operator.as_str() {
            // PDFium's current image-only serialization also emits RGB color
            // state (`RG`/`rg`) before the placement. It is harmless without
            // any paint operator and is kept as a characterized equivalent.
            "q" | "Q" | "cm" | "RG" | "rg" | "w" | "J" | "j" | "M" | "d" | "i" | "gs" => {}
            "Do" => {
                if operation.operands.len() != 1 || invoked_name.is_some() {
                    return Err(verification_error(
                        "Affected page has an unexpected XObject invocation count",
                    ));
                }
                invoked_name = Some(
                    operation.operands[0]
                        .as_name()
                        .map_err(|_| verification_error("Affected page Do operand is not a name"))?
                        .to_vec(),
                );
            }
            _ => {
                return Err(verification_error(format!(
                    "Affected page content uses forbidden operator '{}'",
                    operation.operator
                )));
            }
        }
    }
    let invoked_name = invoked_name
        .ok_or_else(|| verification_error("Affected page content has no image invocation"))?;
    let (resources, inherited) = doc
        .get_page_resources(page_id)
        .map_err(|_| verification_error("Affected page resources cannot be resolved"))?;
    let resources = resources
        .or_else(|| {
            inherited
                .first()
                .and_then(|id| doc.get_dictionary(*id).ok())
        })
        .ok_or_else(|| verification_error("Affected page has no resources"))?;
    let xobjects = resources
        .get(b"XObject")
        .map_err(|_| verification_error("Affected page resources have no /XObject"))?;
    let xobjects = match xobjects {
        lopdf::Object::Dictionary(dict) => dict,
        lopdf::Object::Reference(id) => doc
            .get_dictionary(*id)
            .map_err(|_| verification_error("Affected page /XObject cannot be resolved"))?,
        _ => {
            return Err(verification_error(
                "Affected page /XObject is not a dictionary",
            ))
        }
    };
    if xobjects.len() != 1 {
        return Err(verification_error(
            "Affected page must have exactly one serialized XObject resource",
        ));
    }
    let image_ref = xobjects
        .get(&invoked_name)
        .map_err(|_| verification_error("Affected page invokes an unregistered XObject"))?;
    let image = match image_ref {
        lopdf::Object::Reference(id) => doc
            .get_object(*id)
            .map_err(|_| verification_error("Affected page image XObject cannot be resolved"))?,
        object => object,
    };
    let image = image
        .as_stream()
        .map_err(|_| verification_error("Affected page XObject is not an image stream"))?;
    if image
        .dict
        .get(b"Subtype")
        .and_then(lopdf::Object::as_name)
        .map_err(|_| verification_error("Affected XObject has no /Subtype /Image"))?
        != b"Image"
    {
        return Err(verification_error(
            "Affected XObject is not /Subtype /Image",
        ));
    }
    // PDFium serializes the opaque RGBA raster with an /SMask on the current
    // supported platform. Its opacity is verified through the processed
    // PDFium image below; a color-key /Mask has no such bounded proof here.
    if image.dict.has(b"Mask") {
        return Err(verification_error(
            "Affected image has an unsupported color-key mask representation",
        ));
    }
    Ok(())
}

/// The rectangle returned by `FPDFPageObj_GetBounds()`. This is deliberately
/// separate from canonical geometry: the PDFium API writes each edge through
/// an `f32*`, whereas canonical page geometry remains an `f64` contract.
#[derive(Clone, Copy, Debug)]
struct PdfiumObjectBounds {
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
}

impl PdfiumObjectBounds {
    fn from_effective_box(box_: &crate::secure_redaction::PdfPointRect) -> Self {
        Self {
            left: box_.left,
            bottom: box_.bottom,
            right: box_.left + box_.width,
            top: box_.bottom + box_.height,
        }
    }

    fn width(self) -> f64 {
        self.right - self.left
    }

    fn height(self) -> f64 {
        self.top - self.bottom
    }

    fn page_coordinate_scale(self) -> f64 {
        [
            self.left.abs(),
            self.bottom.abs(),
            self.right.abs(),
            self.top.abs(),
            self.width().abs(),
            self.height().abs(),
            1.0,
        ]
        .into_iter()
        .fold(1.0_f64, f64::max)
    }
}

#[derive(Clone, Copy, Debug)]
struct PdfiumObjectBoundsComparison {
    tolerance: f64,
    left_delta: f64,
    bottom_delta: f64,
    right_delta: f64,
    top_delta: f64,
}

impl PdfiumObjectBoundsComparison {
    fn matches(self) -> bool {
        self.left_delta.abs() <= self.tolerance
            && self.bottom_delta.abs() <= self.tolerance
            && self.right_delta.abs() <= self.tolerance
            && self.top_delta.abs() <= self.tolerance
    }
}

/// Returns one ULP at the effective page's coordinate magnitude after it has
/// crossed PDFium's f32 page-object API boundary. `FPDFPageObj_GetBounds()`
/// writes `float` edges, and `PdfMatrix` / `PdfPoints` are f32-backed as well.
/// This is an observation budget only; it must never be used for canonical
/// geometry, planning, mapping, or pixel-coverage validation.
fn pdfium_object_bounds_precision(expected: PdfiumObjectBounds) -> Option<f64> {
    let scale = expected.page_coordinate_scale();
    if !scale.is_finite() {
        return None;
    }
    let f32_scale = scale as f32;
    if !f32_scale.is_finite() {
        return None;
    }
    // `scale >= 1.0`, so advancing the positive IEEE-754 bit pattern is the
    // next representable f32 and cannot cross a sign or subnormal boundary.
    let next_f32_scale = f32::from_bits(f32_scale.to_bits().checked_add(1)?);
    Some((next_f32_scale as f64) - (f32_scale as f64))
}

fn compare_pdfium_object_bounds(
    actual: PdfiumObjectBounds,
    expected: PdfiumObjectBounds,
) -> Option<PdfiumObjectBoundsComparison> {
    let tolerance = pdfium_object_bounds_precision(expected)?;
    Some(PdfiumObjectBoundsComparison {
        tolerance,
        left_delta: actual.left - expected.left,
        bottom_delta: actual.bottom - expected.bottom,
        right_delta: actual.right - expected.right,
        top_delta: actual.top - expected.top,
    })
}

fn placement_edge_direction(edge: &str, delta: f64) -> &'static str {
    let extends_outward = match edge {
        "left" | "bottom" => delta < 0.0,
        "right" | "top" => delta > 0.0,
        _ => false,
    };
    if delta == 0.0 {
        "exact"
    } else if extends_outward {
        "outward"
    } else {
        "inward"
    }
}

fn image_placement_covers_effective_page(
    object: &PdfPageObject<'_>,
    geometry: &crate::secure_redaction::CanonicalPageGeometry,
    raster_dpi: u32,
    raster_width: u32,
    raster_height: u32,
) -> Result<(), SessionJobOperationError> {
    let bounds = object
        .bounds()
        .map_err(|_| verification_error("Affected image bounds are unavailable"))?
        .to_rect();
    let actual = PdfiumObjectBounds {
        left: bounds.left().value as f64,
        bottom: bounds.bottom().value as f64,
        right: bounds.right().value as f64,
        top: bounds.top().value as f64,
    };
    let expected = PdfiumObjectBounds::from_effective_box(&geometry.effective_box);
    let comparison = compare_pdfium_object_bounds(actual, expected)
        .ok_or_else(|| verification_error("PDFium object-bounds precision scale is not finite"))?;

    // Bounded numeric geometry only: no source text, document content, or
    // region payload is emitted. This lets diagnostics characterize PDFium's
    // f32 observation surface without becoming a sensitive-content log.
    tracing::debug!(
        target: "secure_redaction_verifier",
        expected_left = expected.left,
        expected_bottom = expected.bottom,
        expected_right = expected.right,
        expected_top = expected.top,
        expected_width = expected.width(),
        expected_height = expected.height(),
        actual_left = actual.left,
        actual_bottom = actual.bottom,
        actual_right = actual.right,
        actual_top = actual.top,
        actual_width = actual.width(),
        actual_height = actual.height(),
        delta_left = comparison.left_delta,
        delta_bottom = comparison.bottom_delta,
        delta_right = comparison.right_delta,
        delta_top = comparison.top_delta,
        tolerance = comparison.tolerance,
        page_width = expected.width(),
        page_height = expected.height(),
        raster_dpi,
        raster_width,
        raster_height,
        rotation = ?geometry.rotation,
        "PDFium affected image object-bounds observation"
    );

    if !comparison.matches() {
        let ulps = |delta: f64| delta.abs() / comparison.tolerance;
        return Err(verification_error(format!(
            "PDFium observed affected image placement differs from the canonical effective page: expected left={:.12}, bottom={:.12}, right={:.12}, top={:.12}, width={:.12}, height={:.12}; actual left={:.12}, bottom={:.12}, right={:.12}, top={:.12}, width={:.12}, height={:.12}; absolute delta left={:.12} ({:.3} ULP, {}), bottom={:.12} ({:.3} ULP, {}), right={:.12} ({:.3} ULP, {}), top={:.12} ({:.3} ULP, {}); f32 observation tolerance={:.12}; page width={:.12}, height={:.12}; raster dpi={}, width={}, height={}; rotation={:?}",
            expected.left, expected.bottom, expected.right, expected.top, expected.width(), expected.height(),
            actual.left, actual.bottom, actual.right, actual.top, actual.width(), actual.height(),
            comparison.left_delta.abs(), ulps(comparison.left_delta), placement_edge_direction("left", comparison.left_delta),
            comparison.bottom_delta.abs(), ulps(comparison.bottom_delta), placement_edge_direction("bottom", comparison.bottom_delta),
            comparison.right_delta.abs(), ulps(comparison.right_delta), placement_edge_direction("right", comparison.right_delta),
            comparison.top_delta.abs(), ulps(comparison.top_delta), placement_edge_direction("top", comparison.top_delta),
            comparison.tolerance, expected.width(), expected.height(), raster_dpi, raster_width, raster_height, geometry.rotation,
        )));
    }
    Ok(())
}

/// Renders the reopened serialized page with the canonical redaction render
/// configuration. Reusing the production render authority is important here:
/// PDFium's image-smoothing flag participates in its image-placement edge
/// semantics, so a verifier-only flag change would observe a different device
/// raster than the one described by the shared coordinate conversion.
fn render_reopened_page_exact_pixels(
    page: &PdfPage<'_>,
    spec: &RedactionRasterSpec,
) -> Result<(RedactionRasterBuffer, PdfRenderConfig), SessionJobOperationError> {
    let config = create_redaction_render_config(page, spec)?;
    let bitmap = page
        .render_with_config(&config)
        .map_err(|_| verification_error("Reopened page render failed"))?;
    let width = bitmap.width() as u32;
    let height = bitmap.height() as u32;
    if width != spec.viewport_width || height != spec.viewport_height {
        return Err(verification_error(
            "Reopened page render dimensions differ from expected raster",
        ));
    }
    let pixels = bitmap.as_rgba_bytes();
    let stride = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| verification_error("Reopened page render stride overflow"))?;
    let expected_len = (height as usize)
        .checked_mul(stride)
        .ok_or_else(|| verification_error("Reopened page render length overflow"))?;
    if pixels.len() != expected_len || expected_len > MAX_RENDER_BYTES {
        return Err(verification_error(
            "Reopened page render buffer violates verifier limits",
        ));
    }
    Ok((
        RedactionRasterBuffer {
            width,
            height,
            stride,
            pixel_format: RedactionPixelFormat::Rgba8,
            pixels,
        },
        config,
    ))
}

fn page_has_structural_interactive_content(doc: &lopdf::Document, page_number: u32) -> bool {
    let Some(id) = doc.get_pages().get(&page_number).copied() else {
        return true;
    };
    let Ok(page) = doc.get_dictionary(id) else {
        return true;
    };
    page.has(b"Annots") || page.has(b"AA") || page.has(b"StructParents")
}

fn canonical_page_lists(plan: &SecureRedactionPlan) -> (Vec<u32>, Vec<u32>) {
    let mut affected = Vec::new();
    let mut unaffected = Vec::new();
    for page in &plan.page_plans {
        match page.strategy {
            crate::secure_redaction::PageRedactionStrategy::RasterizePage => {
                affected.push(page.page_index)
            }
            crate::secure_redaction::PageRedactionStrategy::ImportUnaffectedPage => {
                unaffected.push(page.page_index)
            }
        }
    }
    (affected, unaffected)
}

pub struct SecureRedactionVerificationArgs<'a> {
    pub job_dir: &'a Path,
    pub snapshot_path: &'a Path,
    pub credential: Option<&'a str>,
    pub job_id: &'a str,
    pub session_id: &'a str,
    pub content_generation: u64,
    pub policy: crate::secure_redaction::SecureRedactionPolicy,
    pub acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
    pub regions: Vec<crate::secure_redaction::RedactionRegionInput>,
    pub transform_report: SecureRedactionTransformReport,
}

/// Verifies an artifact without depending on the transform worker's in-memory state.
pub fn verify_secure_redaction(
    args: SecureRedactionVerificationArgs<'_>,
) -> Result<SecureRedactionVerificationReport, SessionJobOperationError> {
    let SecureRedactionVerificationArgs {
        job_dir,
        snapshot_path,
        credential,
        job_id,
        session_id,
        content_generation,
        policy,
        acknowledgements,
        regions,
        transform_report,
    } = args;
    crate::secure_redaction::validate_transform_report(&transform_report)?;
    let artifact = artifact_path(job_dir)?;
    let initial_digest = crate::document_io::sha256_file(&artifact)
        .map_err(|e| verification_error(format!("Artifact digest unavailable: {e}")))?;
    let initial_size = fs::metadata(&artifact)
        .map_err(|e| verification_error(format!("Artifact metadata unavailable: {e}")))?
        .len();
    let source_digest = crate::document_io::sha256_file(snapshot_path)
        .map_err(|e| verification_error(format!("Snapshot digest unavailable: {e}")))?;
    let plan = crate::secure_redaction_inspection::inspect_and_plan_secure_redaction(
        snapshot_path,
        credential,
        content_generation,
        policy,
        acknowledgements,
        regions,
    )?;
    crate::secure_redaction::validate_secure_redaction_plan(&plan)?;

    let mut checks: Vec<RedactionVerificationCheck> = REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS
        .iter()
        .map(|c| {
            check(
                c,
                VerificationCheckStatus::NotRun,
                "Not run after an earlier verification failure",
            )
        })
        .collect();
    let mut blockers = Vec::new();
    let mut set =
        |index: usize, result: Result<(), SessionJobOperationError>, blocker_code: &str| -> bool {
            match result {
                Ok(()) => {
                    checks[index] = check(
                        REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS[index],
                        VerificationCheckStatus::Passed,
                        "Independent observation passed",
                    );
                    true
                }
                Err(e) => {
                    let message = e.to_string();
                    checks[index] = check(
                        REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS[index],
                        VerificationCheckStatus::Failed,
                        &message,
                    );
                    blockers.push(blocker(blocker_code, message));
                    false
                }
            }
        };

    let source_ok = set(
        0,
        (|| {
            if transform_report.job_id != job_id
                || transform_report.session_id != session_id
                || transform_report.input_digest != source_digest
                || transform_report.plan_digest != plan.plan_digest
                || !plan.executable
                || !plan.ready_for_execution
                || !plan.blockers.is_empty()
            {
                return Err(verification_error(
                    "Source snapshot / authoritative identity / regenerated plan binding mismatch",
                ));
            }
            Ok(())
        })(),
        "REDACTION_VERIFICATION_SOURCE_BINDING_FAILED",
    );
    let report_ok = set(
        1,
        (|| {
            if transform_report.status != "transformed_unverified"
                || transform_report.output_relative_name != ARTIFACT_NAME
                || transform_report.page_count != plan.page_plans.len() as u32
            {
                return Err(verification_error(
                    "Transform report status or internal artifact identity is inconsistent",
                ));
            }
            let (affected, unaffected) = canonical_page_lists(&plan);
            if transform_report.affected_pages != affected
                || transform_report.unaffected_pages != unaffected
            {
                return Err(verification_error(
                    "Transform page strategy lists do not match regenerated plan",
                ));
            }
            Ok(())
        })(),
        "REDACTION_VERIFICATION_ARTIFACT_MISMATCH",
    );
    let path_ok = set(
        2,
        artifact_path(job_dir).map(|_| ()),
        "REDACTION_VERIFICATION_ARTIFACT_MISMATCH",
    );
    let digest_ok = set(
        3,
        if initial_digest == transform_report.output_digest
            && initial_size == transform_report.output_size_bytes
        {
            Ok(())
        } else {
            Err(verification_error(
                "Actual artifact digest or byte size differs from transform report",
            ))
        },
        "REDACTION_VERIFICATION_ARTIFACT_MISMATCH",
    );

    let pdfium = crate::pdf_engine::get_pdfium()
        .map_err(|e| verification_error(format!("PDFium unavailable: {e}")))?;
    let output = crate::pdf_engine::open_pdfium_doc(&pdfium, &artifact.to_string_lossy(), None)
        .map_err(|_| verification_error("PDFium cannot reopen output artifact"));
    let lopdf = lopdf::Document::load(&artifact)
        .map_err(|_| verification_error("lopdf cannot reopen output artifact"));
    let parsers_ok = set(
        4,
        match (&output, &lopdf) {
            (Ok(_), Ok(_)) => Ok(()),
            _ => Err(verification_error(
                "Both PDFium and lopdf must reopen output artifact",
            )),
        },
        "REDACTION_VERIFICATION_STRUCTURE_FAILED",
    );

    if source_ok && report_ok && path_ok && digest_ok && parsers_ok {
        let output = output.expect("checked above");
        let lopdf = lopdf.expect("checked above");
        set(
            5,
            full_rewrite_structure(
                &lopdf,
                usize::from(output.pages().len()),
                plan.page_plans.len(),
            ),
            "REDACTION_VERIFICATION_STRUCTURE_FAILED",
        );
        set(
            6,
            no_forbidden_document_structures(&lopdf),
            "REDACTION_VERIFICATION_UNSAFE_CONTENT",
        );
        set(
            7,
            if usize::from(output.pages().len()) == plan.page_plans.len() {
                Ok(())
            } else {
                Err(verification_error(
                    "Output page count differs from regenerated plan",
                ))
            },
            "REDACTION_VERIFICATION_ARTIFACT_MISMATCH",
        );
        let geometry = (|| {
            for (i, page_plan) in plan.page_plans.iter().enumerate() {
                let page = output
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Output page missing"))?;
                crate::secure_redaction_raster::validate_pdfium_page_geometry(
                    &page,
                    &page_plan.geometry,
                    page_plan.page_index,
                )?;
            }
            Ok(())
        })();
        set(8, geometry, "REDACTION_VERIFICATION_GEOMETRY_FAILED");
        let image_only = (|| {
            for (i, page_plan) in plan.page_plans.iter().enumerate() {
                if page_plan.strategy
                    != crate::secure_redaction::PageRedactionStrategy::RasterizePage
                {
                    continue;
                }
                let page = output
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Output affected page missing"))?;
                let objects: Vec<_> = page.objects().iter().collect();
                let types: Vec<_> = objects.iter().map(|o| o.object_type()).collect();
                if types != vec![PdfPageObjectType::Image]
                    || !page
                        .text()
                        .map_err(|_| verification_error("Affected page text inspection failed"))?
                        .all()
                        .trim()
                        .is_empty()
                {
                    return Err(verification_error(
                        "Affected page is not exactly one image with no extractable text",
                    ));
                }
                affected_page_serialized_image_only(&lopdf, (i + 1) as u32)?;
                let (expected_width, expected_height) =
                    calculate_raster_dimensions(&page_plan.geometry, plan.policy.raster_dpi)?;
                let image = objects[0]
                    .as_image_object()
                    .ok_or_else(|| verification_error("PDFium affected object is not an image"))?;
                if image.has_transparency() {
                    return Err(verification_error(
                        "PDFium reports transparency on the affected image object",
                    ));
                }
                let processed = image
                    .get_processed_image(&output)
                    .map_err(|_| verification_error("Affected image alpha cannot be observed"))?
                    .to_rgba8();
                if processed.pixels().any(|pixel| pixel.0[3] != 255) {
                    return Err(verification_error(
                        "Affected image contains non-opaque processed alpha",
                    ));
                }
                let raw = image
                    .get_raw_image()
                    .map_err(|_| verification_error("Affected image raw pixels cannot be read"))?
                    .to_rgba8();
                if raw.width() != expected_width || raw.height() != expected_height {
                    return Err(verification_error(
                        "Affected embedded image dimensions do not match regenerated raster dimensions",
                    ));
                }
                image_placement_covers_effective_page(
                    &objects[0],
                    &page_plan.geometry,
                    plan.policy.raster_dpi,
                    expected_width,
                    expected_height,
                )?;
            }
            Ok(())
        })();
        set(9, image_only, "REDACTION_VERIFICATION_UNSAFE_CONTENT");
        let interactive = (|| {
            for (i, page_plan) in plan.page_plans.iter().enumerate() {
                if page_plan.strategy
                    != crate::secure_redaction::PageRedactionStrategy::RasterizePage
                {
                    continue;
                }
                let page = output
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Output affected page missing"))?;
                if !page.annotations().is_empty()
                    || page_has_structural_interactive_content(&lopdf, (i + 1) as u32)
                {
                    return Err(verification_error(
                        "Affected page retains annotation or interactive structure",
                    ));
                }
            }
            Ok(())
        })();
        set(10, interactive, "REDACTION_VERIFICATION_UNSAFE_CONTENT");
        let coverage = (|| {
            let source_document = crate::pdf_engine::open_pdfium_doc(
                &pdfium,
                &snapshot_path.to_string_lossy(),
                credential,
            )
            .map_err(|_| verification_error("Cannot reopen snapshot for coverage mapping"))?;
            // Use a fresh PDFium document for the actual reopened-render
            // observation. Earlier structural checks extract processed/raw
            // image data from `output`; PDFium may mutate internal image/cache
            // state on that handle. A second reopen keeps the rendered
            // artifact observation isolated from embedded-raster inspection.
            let render_document =
                crate::pdf_engine::open_pdfium_doc(&pdfium, &artifact.to_string_lossy(), None)
                    .map_err(|_| {
                        verification_error(
                            "Cannot independently reopen artifact for rendered coverage",
                        )
                    })?;
            for (i, page_plan) in plan.page_plans.iter().enumerate() {
                if page_plan.strategy
                    != crate::secure_redaction::PageRedactionStrategy::RasterizePage
                {
                    continue;
                }
                let page = output
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Output affected page missing"))?;
                let render_page = render_document.pages().get(i as u16).map_err(|_| {
                    verification_error("Independently reopened output affected page missing")
                })?;
                let source_page = source_document
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Source affected page missing"))?;
                let (w, h) =
                    calculate_raster_dimensions(&page_plan.geometry, plan.policy.raster_dpi)?;
                let spec = RedactionRasterSpec {
                    page_index: page_plan.page_index,
                    dpi: plan.policy.raster_dpi,
                    rotation: page_plan.geometry.rotation,
                    viewport_width: w,
                    viewport_height: h,
                    background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
                    annotation_policy: RedactionAnnotationPolicy::DoNotRender,
                    widget_form_policy: RedactionWidgetFormPolicy::BlockedIfPresent,
                };
                let (source_buffer, config) =
                    render_affected_page_raster(&source_page, &spec, None)?;
                // The source raster establishes the canonical PDFium mapping
                // configuration only. It is no longer needed once `config`
                // has been created, so retaining its full high-DPI RGBA buffer
                // through the independent reopened-artifact observation would
                // create an avoidable second large allocation peak.
                drop(source_buffer);
                // Observe the reopened page before raw-image extraction.
                // PDFium's extraction path can mutate internal image/cache
                // state on the page handle on supported builds.
                let (output_buffer, output_config) =
                    render_reopened_page_exact_pixels(&render_page, &spec)?;
                let objects = page.objects();
                let object = objects.get(0).map_err(|_| {
                    verification_error("Affected image object missing for raw coverage")
                })?;
                let image = object.as_image_object().ok_or_else(|| {
                    verification_error("Affected object is not an image for raw coverage")
                })?;
                let raw = image
                    .get_raw_image()
                    .map_err(|_| {
                        verification_error("observation=embedded_raster raw image unavailable")
                    })?
                    .to_rgba8();
                if raw.width() != w || raw.height() != h {
                    return Err(verification_error(
                        "observation=embedded_raster image dimensions differ from plan raster dimensions",
                    ));
                }
                let embedded = crate::secure_redaction_raster::RedactionRasterBuffer {
                    width: raw.width(),
                    height: raw.height(),
                    stride: (raw.width() as usize).checked_mul(4).ok_or_else(|| {
                        verification_error("observation=embedded_raster stride overflow")
                    })?,
                    pixel_format: crate::secure_redaction_raster::RedactionPixelFormat::Rgba8,
                    pixels: raw.into_raw(),
                };
                for region in &page_plan.canonical_regions {
                    let (embedded_rect, _) = map_pdf_rect_to_device_pixel_rect(
                        &source_page,
                        &region.applied_pdf_point_rect,
                        &config,
                        w,
                        h,
                    )?;
                    verify_opaque_black_overwrite(&embedded, std::slice::from_ref(&embedded_rect)).map_err(|error| {
                            verification_error(format!(
                                "observation=embedded_raster coverage failed for page_index {}, canonical_region_id {}, source_region_ids [{}]: {}",
                                page_plan.page_index,
                                region.region_id,
                                region.source_region_ids.join(", "),
                                error,
                            ))
                        })?;
                    let (output_rect, _) = map_pdf_rect_to_device_pixel_rect(
                        &render_page,
                        &region.applied_pdf_point_rect,
                        &output_config,
                        output_buffer.width,
                        output_buffer.height,
                    )
                    .map_err(|error| {
                        verification_error(format!(
                            "observation=reopened_render mapping failed for page_index {}, canonical_region_id {}, source_region_ids [{}], canonical_pdf_rect ({}, {}, {}, {}), rotation {:?}: {}",
                            page_plan.page_index,
                            region.region_id,
                            region.source_region_ids.join(", "),
                            region.applied_pdf_point_rect.left,
                            region.applied_pdf_point_rect.bottom,
                            region.applied_pdf_point_rect.width,
                            region.applied_pdf_point_rect.height,
                            page_plan.geometry.rotation,
                            error,
                        ))
                    })?;
                    verify_opaque_black_overwrite(&output_buffer, std::slice::from_ref(&output_rect))
                        .map_err(|error| {
                            verification_error(format!(
                                "observation=reopened_render coverage failed for page_index {}, canonical_region_id {}, source_region_ids [{}], canonical_pdf_rect ({}, {}, {}, {}), mapped_output_device_rect ({}, {}, {}, {}), rotation {:?}: {}",
                                page_plan.page_index,
                                region.region_id,
                                region.source_region_ids.join(", "),
                                region.applied_pdf_point_rect.left,
                                region.applied_pdf_point_rect.bottom,
                                region.applied_pdf_point_rect.width,
                                region.applied_pdf_point_rect.height,
                                output_rect.left,
                                output_rect.top,
                                output_rect.width,
                                output_rect.height,
                                page_plan.geometry.rotation,
                                error,
                            ))
                        })?;
                }
            }
            Ok(())
        })();
        set(11, coverage, "REDACTION_VERIFICATION_COVERAGE_FAILED");
        let unaffected = (|| {
            let source = crate::pdf_engine::open_pdfium_doc(
                &pdfium,
                &snapshot_path.to_string_lossy(),
                credential,
            )
            .map_err(|_| verification_error("Cannot reopen snapshot for unaffected comparison"))?;
            for (i, page_plan) in plan.page_plans.iter().enumerate() {
                if page_plan.strategy
                    != crate::secure_redaction::PageRedactionStrategy::ImportUnaffectedPage
                {
                    continue;
                }
                let source_page = source
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Source page missing"))?;
                let output_page = output
                    .pages()
                    .get(i as u16)
                    .map_err(|_| verification_error("Output page missing"))?;
                if source_page.text().ok().map(|t| t.all())
                    != output_page.text().ok().map(|t| t.all())
                {
                    return Err(verification_error(
                        "Unaffected page text differs from snapshot",
                    ));
                }
                let (w, h) =
                    calculate_raster_dimensions(&page_plan.geometry, plan.policy.raster_dpi)?;
                let spec = RedactionRasterSpec {
                    page_index: page_plan.page_index,
                    dpi: plan.policy.raster_dpi,
                    rotation: page_plan.geometry.rotation,
                    viewport_width: w,
                    viewport_height: h,
                    background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
                    annotation_policy: RedactionAnnotationPolicy::DoNotRender,
                    widget_form_policy: RedactionWidgetFormPolicy::BlockedIfPresent,
                };
                let (source_render, _) = render_affected_page_raster(&source_page, &spec, None)?;
                let (output_render, _) = render_affected_page_raster(&output_page, &spec, None)?;
                if source_render.width != output_render.width
                    || source_render.height != output_render.height
                    || source_render.pixels != output_render.pixels
                {
                    return Err(verification_error(
                        "Unaffected page deterministic PDFium render differs from snapshot",
                    ));
                }
            }
            Ok(())
        })();
        set(
            12,
            unaffected,
            "REDACTION_VERIFICATION_UNAFFECTED_PAGE_MISMATCH",
        );
    }
    let final_digest = crate::document_io::sha256_file(&artifact)
        .map_err(|e| verification_error(format!("Final artifact digest unavailable: {e}")))?;
    set(
        13,
        if final_digest == initial_digest {
            Ok(())
        } else {
            Err(verification_error("Artifact changed during verification"))
        },
        "REDACTION_VERIFICATION_ARTIFACT_MISMATCH",
    );
    let is_verified = blockers.is_empty()
        && checks
            .iter()
            .all(|c| c.status == VerificationCheckStatus::Passed);
    let report = SecureRedactionVerificationReport {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        verification_version: SECURE_REDACTION_VERIFICATION_VERSION.to_string(),
        job_id: job_id.to_string(),
        session_id: session_id.to_string(),
        plan_digest: plan.plan_digest,
        transform_digest: final_digest,
        is_verified,
        verification_checks: checks,
        blockers,
        warnings: Vec::new(),
    };
    crate::secure_redaction::validate_verification_report(&report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{compare_pdfium_object_bounds, pdfium_object_bounds_precision, PdfiumObjectBounds};
    use crate::secure_redaction::{MAX_RASTER_DPI, MIN_RASTER_DPI};

    fn expected_bounds() -> PdfiumObjectBounds {
        PdfiumObjectBounds {
            left: 0.0,
            bottom: 0.0,
            right: 612.0,
            top: 792.0,
        }
    }

    #[test]
    fn pdfium_object_bounds_precision_uses_page_coordinate_f32_ulp() {
        let expected = expected_bounds();
        let tolerance = pdfium_object_bounds_precision(expected).unwrap();
        assert_eq!(tolerance, 0.000_061_035_156_25);

        // A zero-valued edge still receives the page-scale f32 ULP, rather
        // than the much smaller subnormal ULP at zero.
        let observed = PdfiumObjectBounds {
            bottom: -tolerance,
            ..expected
        };
        assert!(compare_pdfium_object_bounds(observed, expected)
            .unwrap()
            .matches());
    }

    #[test]
    fn pdfium_object_bounds_precision_has_deterministic_boundaries() {
        let expected = expected_bounds();
        let tolerance = pdfium_object_bounds_precision(expected).unwrap();
        let just_over_tolerance = f64::from_bits(tolerance.to_bits() + 1);

        for delta in [-tolerance, 0.0, tolerance] {
            let observed = PdfiumObjectBounds {
                left: expected.left + delta,
                ..expected
            };
            assert!(compare_pdfium_object_bounds(observed, expected)
                .unwrap()
                .matches());
        }
        for delta in [-just_over_tolerance, just_over_tolerance] {
            let observed = PdfiumObjectBounds {
                left: expected.left + delta,
                ..expected
            };
            assert!(!compare_pdfium_object_bounds(observed, expected)
                .unwrap()
                .matches());
        }
    }

    #[test]
    fn pdfium_object_bounds_precision_rejects_real_placement_corruption() {
        let expected = expected_bounds();

        let inward = PdfiumObjectBounds {
            bottom: expected.bottom + 0.01,
            ..expected
        };
        assert!(!compare_pdfium_object_bounds(inward, expected)
            .unwrap()
            .matches());

        let outward = PdfiumObjectBounds {
            top: expected.top + 0.01,
            ..expected
        };
        assert!(!compare_pdfium_object_bounds(outward, expected)
            .unwrap()
            .matches());

        let translated = PdfiumObjectBounds {
            left: expected.left + 0.01,
            bottom: expected.bottom + 0.01,
            right: expected.right + 0.01,
            top: expected.top + 0.01,
        };
        assert!(!compare_pdfium_object_bounds(translated, expected)
            .unwrap()
            .matches());

        let scaled = PdfiumObjectBounds {
            right: expected.right + 0.01,
            ..expected
        };
        assert!(!compare_pdfium_object_bounds(scaled, expected)
            .unwrap()
            .matches());
    }

    #[test]
    fn pdfium_object_bounds_precision_is_safe_for_nonzero_and_negative_origins() {
        let nonzero = PdfiumObjectBounds {
            left: 100.0,
            bottom: 200.0,
            right: 712.0,
            top: 992.0,
        };
        let negative = PdfiumObjectBounds {
            left: -612.0,
            bottom: -792.0,
            right: 0.0,
            top: 0.0,
        };
        assert_eq!(
            pdfium_object_bounds_precision(nonzero),
            pdfium_object_bounds_precision(negative)
        );
    }

    #[test]
    fn pdfium_object_bounds_precision_is_independent_of_raster_dpi() {
        let expected = expected_bounds();
        let expected_tolerance = pdfium_object_bounds_precision(expected);

        // DPI deliberately never reaches the precision helper: PDFium object
        // bounds are observed in PDF point space, not device-pixel space.
        for dpi in [MIN_RASTER_DPI, 300, 450, MAX_RASTER_DPI] {
            assert_eq!(
                pdfium_object_bounds_precision(expected),
                expected_tolerance,
                "DPI {dpi} must not change the page-coordinate f32 ULP budget"
            );
        }
    }
}
