use crate::document_session::SessionJobCancellationCheckpoint;
use crate::secure_redaction::{
    make_redaction_error, validate_canonical_page_geometry, validate_device_pixel_rect,
    CanonicalPageGeometry, DevicePixelRect, PageRotation, PdfPointRect, SecureRedactionErrorCode,
    SecureRedactionPlan, GEOMETRY_EPSILON, MAX_CANONICAL_REGIONS_PER_PAGE, MAX_PIXELS_PER_PAGE,
    MAX_RASTER_DPI, MIN_RASTER_DPI,
};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

// Renderer limits from pdf_engine / system architecture ceiling
pub const MAX_PIXEL_DIMENSION: u32 = 16_384;
pub const MAX_RENDER_BYTES: usize = 256 * 1024 * 1024;

// ---------------------------------------------------------------------------
// 1. Data Models for Secure Raster Pipeline Evidence & Payload
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionPixelFormat {
    Rgba8,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionBackgroundPolicy {
    OpaqueWhiteV1,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionAnnotationPolicy {
    DoNotRender,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionWidgetFormPolicy {
    BlockedIfPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionRasterSpec {
    pub page_index: u32,
    pub dpi: u32,
    pub rotation: PageRotation,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub background_policy: RedactionBackgroundPolicy,
    pub annotation_policy: RedactionAnnotationPolicy,
    pub widget_form_policy: RedactionWidgetFormPolicy,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RasterizedRegionMapping {
    pub page_index: u32,
    pub canonical_region_id: String,
    pub source_region_ids: Vec<String>,
    pub applied_pdf_point_rect: PdfPointRect,
    pub device_pixel_rect: DevicePixelRect,
    pub bitmap_width: u32,
    pub bitmap_height: u32,
    pub boundary_clamped: bool,
    pub rotation: PageRotation,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub dpi: u32,
}

#[derive(Debug, Clone)]
pub struct RedactionRasterBuffer {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub pixel_format: RedactionPixelFormat,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LosslessRasterPayload {
    pub page_index: u32,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub pixel_format: RedactionPixelFormat,
    pub background_policy: RedactionBackgroundPolicy,
    pub pixels: Vec<u8>,
    pub region_mappings: Vec<RasterizedRegionMapping>,
    pub merged_device_pixel_rects: Vec<DevicePixelRect>,
}

// ---------------------------------------------------------------------------
// 2. Preflight & Dimension Calculations
// ---------------------------------------------------------------------------

pub fn calculate_raster_dimensions(
    geom: &CanonicalPageGeometry,
    dpi: u32,
) -> Result<(u32, u32), crate::document_session::SessionJobOperationError> {
    if !(MIN_RASTER_DPI..=MAX_RASTER_DPI).contains(&dpi) {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Invalid raster DPI: {dpi}. Must be between {MIN_RASTER_DPI} and {MAX_RASTER_DPI}"
            ),
        ));
    }

    let eff = &geom.effective_box;
    if !eff.width.is_finite() || !eff.height.is_finite() || eff.width <= 0.0 || eff.height <= 0.0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            "Effective box width and height must be finite numbers > 0",
        ));
    }

    let dpi_scale = (dpi as f64) / 72.0;

    let unrotated_width_f = eff.width * dpi_scale;
    let unrotated_height_f = eff.height * dpi_scale;

    if !unrotated_width_f.is_finite() || !unrotated_height_f.is_finite() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Calculated raster dimensions resulted in non-finite float value",
        ));
    }

    let (width_f, height_f) = match geom.rotation {
        PageRotation::Deg0 | PageRotation::Deg180 => (unrotated_width_f, unrotated_height_f),
        PageRotation::Deg90 | PageRotation::Deg270 => (unrotated_height_f, unrotated_width_f),
    };

    if width_f < 1.0
        || height_f < 1.0
        || width_f > (MAX_PIXEL_DIMENSION as f64)
        || height_f > (MAX_PIXEL_DIMENSION as f64)
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Calculated raster dimension ({width_f:.2}x{height_f:.2}) is outside allowed bounds (1..{MAX_PIXEL_DIMENSION})"
            ),
        ));
    }

    let width_rounded = width_f.round();
    let height_rounded = height_f.round();

    if !width_rounded.is_finite() || !height_rounded.is_finite() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Rounded raster dimensions resulted in non-finite float value",
        ));
    }

    let width_u64 = width_rounded as u64;
    let height_u64 = height_rounded as u64;

    if width_u64 == 0 || height_u64 == 0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Calculated raster dimensions resulted in zero width or height",
        ));
    }

    if width_u64 > (MAX_PIXEL_DIMENSION as u64) || height_u64 > (MAX_PIXEL_DIMENSION as u64) {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Calculated raster dimension ({width_u64}x{height_u64}) exceeds maximum allowed dimension ceiling ({MAX_PIXEL_DIMENSION})"
            ),
        ));
    }

    let width = width_u64 as u32;
    let height = height_u64 as u32;

    let total_pixels = (width as u64).checked_mul(height as u64).ok_or_else(|| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Arithmetic overflow calculating total raster pixels",
        )
    })?;

    if total_pixels > MAX_PIXELS_PER_PAGE {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Page pixel count ({total_pixels}) exceeds maximum limit ({MAX_PIXELS_PER_PAGE})"
            ),
        ));
    }

    let total_bytes = (total_pixels as usize).checked_mul(4).ok_or_else(|| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Arithmetic overflow calculating total raster buffer bytes",
        )
    })?;

    if total_bytes > MAX_RENDER_BYTES {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!("Page render byte size ({total_bytes}) exceeds maximum allowed byte ceiling ({MAX_RENDER_BYTES})"),
        ));
    }

    Ok((width, height))
}

// ---------------------------------------------------------------------------
// 3. Shared Render Configuration Authority
// ---------------------------------------------------------------------------

pub fn create_redaction_render_config(
    pdfium_page: &PdfPage<'_>,
    spec: &RedactionRasterSpec,
) -> Result<PdfRenderConfig, crate::document_session::SessionJobOperationError> {
    let target_deg = match spec.rotation {
        PageRotation::Deg0 => 0,
        PageRotation::Deg90 => 90,
        PageRotation::Deg180 => 180,
        PageRotation::Deg270 => 270,
    };
    let intrinsic_deg = match pdfium_page.rotation() {
        Ok(PdfPageRenderRotation::None) => 0,
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        Err(e) => {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Failed to read PDF page intrinsic rotation: {e}"),
            ));
        }
    };

    let rel_deg = (target_deg + 360 - intrinsic_deg) % 360;
    let pdf_rotation = match rel_deg {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    };

    let mut config = PdfRenderConfig::new();
    // `set_fixed_size()` allocates the requested bitmap dimensions but leaves
    // PDFium's page-to-device scale at 1.0 when form rendering is disabled.
    // That makes a high-DPI payload serialize at point resolution and moves
    // its redaction pixels inward on a reopen/render round trip. A target size
    // carries the same deterministic bitmap dimensions *and* the required
    // page-to-device scale into both PDFium coordinate-conversion APIs and
    // rendering.
    config = config.set_target_size(spec.viewport_width as i32, spec.viewport_height as i32);
    config = config.rotate(pdf_rotation, true);
    config = config.clear_before_rendering(true);
    config = config.set_clear_color(PdfColor::WHITE);
    config = config.render_annotations(false);
    config = config.render_form_data(false);
    config = config.set_reverse_byte_order(true);
    config = config.use_lcd_text_rendering(false);
    config = config.disable_native_text_rendering(true);
    config = config.set_text_smoothing(true);
    config = config.set_image_smoothing(true);
    config = config.set_path_smoothing(true);
    config = config.use_print_quality(false);
    Ok(config)
}

// ---------------------------------------------------------------------------
// 4. PDFium Page-to-Device Mapping & Outward Rounding
// ---------------------------------------------------------------------------

pub fn map_pdf_rect_to_device_pixel_rect(
    pdfium_page: &PdfPage<'_>,
    applied_pdf_point_rect: &PdfPointRect,
    render_config: &PdfRenderConfig,
    viewport_width: u32,
    viewport_height: u32,
) -> Result<(DevicePixelRect, bool), crate::document_session::SessionJobOperationError> {
    if !applied_pdf_point_rect.left.is_finite()
        || !applied_pdf_point_rect.bottom.is_finite()
        || !applied_pdf_point_rect.width.is_finite()
        || !applied_pdf_point_rect.height.is_finite()
        || applied_pdf_point_rect.width <= 0.0
        || applied_pdf_point_rect.height <= 0.0
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "applied_pdf_point_rect must have finite coordinates and positive dimensions",
        ));
    }

    let p1 = (applied_pdf_point_rect.left, applied_pdf_point_rect.bottom);
    let p2 = (
        applied_pdf_point_rect.left + applied_pdf_point_rect.width,
        applied_pdf_point_rect.bottom,
    );
    let p3 = (
        applied_pdf_point_rect.left,
        applied_pdf_point_rect.bottom + applied_pdf_point_rect.height,
    );
    let p4 = (
        applied_pdf_point_rect.left + applied_pdf_point_rect.width,
        applied_pdf_point_rect.bottom + applied_pdf_point_rect.height,
    );

    let corners_pdf = [p1, p2, p3, p4];
    let mut dev_coords = Vec::with_capacity(4);

    for (px, py) in corners_pdf {
        let (dx, dy) = pdfium_page
            .points_to_pixels(
                PdfPoints::new(px as f32),
                PdfPoints::new(py as f32),
                render_config,
            )
            .map_err(|e| {
                make_redaction_error(
                    SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                    format!("PDFium points_to_pixels conversion failed: {e}"),
                )
            })?;
        dev_coords.push((dx as f64, dy as f64));
    }

    let min_x = dev_coords
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let max_x = dev_coords
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = dev_coords
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_y = dev_coords
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);

    if !min_x.is_finite() || !max_x.is_finite() || !min_y.is_finite() || !max_y.is_finite() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            "PDFium page-to-device mapping returned non-finite coordinates",
        ));
    }

    // Outward rounding: min edge -> floor, max edge -> ceil
    let left_i32 = min_x.floor() as i32;
    let top_i32 = min_y.floor() as i32;
    let mut right_i32 = max_x.ceil() as i32;
    let mut bottom_i32 = max_y.ceil() as i32;

    // Enforce minimum 1-pixel extent for positive PDF rects if outward rounding collapsed it
    if right_i32 <= left_i32 {
        right_i32 = left_i32.checked_add(1).ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "DevicePixelRect left addition overflow",
            )
        })?;
    }
    if bottom_i32 <= top_i32 {
        bottom_i32 = top_i32.checked_add(1).ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "DevicePixelRect top addition overflow",
            )
        })?;
    }

    // Bitmap boundary explicit intersection
    let v_width_i32 = viewport_width as i32;
    let v_height_i32 = viewport_height as i32;

    let clamped_left = left_i32.clamp(0, v_width_i32);
    let clamped_right = right_i32.clamp(0, v_width_i32);
    let clamped_top = top_i32.clamp(0, v_height_i32);
    let clamped_bottom = bottom_i32.clamp(0, v_height_i32);

    let boundary_clamped = clamped_left != left_i32
        || clamped_right != right_i32
        || clamped_top != top_i32
        || clamped_bottom != bottom_i32;

    if clamped_left >= clamped_right || clamped_top >= clamped_bottom {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            "Redaction area falls completely outside page bitmap bounds (zero-area intersection rejected)",
        ));
    }

    let width = (clamped_right - clamped_left) as u32;
    let height = (clamped_bottom - clamped_top) as u32;

    let dev_rect = DevicePixelRect {
        left: clamped_left,
        top: clamped_top,
        width,
        height,
    };

    validate_device_pixel_rect(&dev_rect)?;

    Ok((dev_rect, boundary_clamped))
}

// ---------------------------------------------------------------------------
// 5. Deterministic Pixel-Space Rectangle Merge
// ---------------------------------------------------------------------------

pub fn merge_device_pixel_rects(
    mappings: &[RasterizedRegionMapping],
) -> Result<
    (Vec<DevicePixelRect>, Vec<RasterizedRegionMapping>),
    crate::document_session::SessionJobOperationError,
> {
    if mappings.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let page_index = mappings[0].page_index;
    for m in mappings {
        if m.page_index != page_index {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidRegion,
                format!(
                    "merge_device_pixel_rects received region mappings from multiple pages ({} and {})",
                    page_index, m.page_index
                ),
            ));
        }
    }

    // Work on mutable region mapping copies sorted deterministically
    let mut items: Vec<RasterizedRegionMapping> = mappings.to_vec();
    items.sort_by(|a, b| {
        a.device_pixel_rect
            .top
            .cmp(&b.device_pixel_rect.top)
            .then_with(|| a.device_pixel_rect.left.cmp(&b.device_pixel_rect.left))
            .then_with(|| a.device_pixel_rect.width.cmp(&b.device_pixel_rect.width))
            .then_with(|| a.device_pixel_rect.height.cmp(&b.device_pixel_rect.height))
            .then_with(|| a.canonical_region_id.cmp(&b.canonical_region_id))
    });

    // Iterative merge of overlapping & edge-touching rectangles
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        'outer: while i < items.len() {
            let mut j = i + 1;
            while j < items.len() {
                let r1 = &items[i].device_pixel_rect;
                let r2 = &items[j].device_pixel_rect;

                let r1_right = r1.left.checked_add(r1.width as i32).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Overflow calculating rect1 right bound in merge",
                    )
                })?;
                let r1_bottom = r1.top.checked_add(r1.height as i32).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Overflow calculating rect1 bottom bound in merge",
                    )
                })?;
                let r2_right = r2.left.checked_add(r2.width as i32).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Overflow calculating rect2 right bound in merge",
                    )
                })?;
                let r2_bottom = r2.top.checked_add(r2.height as i32).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Overflow calculating rect2 bottom bound in merge",
                    )
                })?;

                // Check overlap or edge-touching
                let x_touch_or_overlap = r1.left <= r2_right && r2.left <= r1_right;
                let y_touch_or_overlap = r1.top <= r2_bottom && r2.top <= r1_bottom;

                if x_touch_or_overlap && y_touch_or_overlap {
                    // Merge into bounding box
                    let merged_left = r1.left.min(r2.left);
                    let merged_top = r1.top.min(r2.top);
                    let merged_right = r1_right.max(r2_right);
                    let merged_bottom = r1_bottom.max(r2_bottom);

                    let merged_width = (merged_right - merged_left) as u32;
                    let merged_height = (merged_bottom - merged_top) as u32;

                    let merged_rect = DevicePixelRect {
                        left: merged_left,
                        top: merged_top,
                        width: merged_width,
                        height: merged_height,
                    };

                    // Combined source region IDs deduplicated and sorted
                    let mut combined_sources = items[i].source_region_ids.clone();
                    combined_sources.extend(items[j].source_region_ids.clone());
                    combined_sources.sort();
                    combined_sources.dedup();

                    items[i].device_pixel_rect = merged_rect;
                    items[i].source_region_ids = combined_sources;
                    items[i].boundary_clamped =
                        items[i].boundary_clamped || items[j].boundary_clamped;

                    items.remove(j);
                    changed = true;
                    continue 'outer;
                }
                j += 1;
            }
            i += 1;
        }
    }

    items.sort_by(|a, b| {
        a.device_pixel_rect
            .top
            .cmp(&b.device_pixel_rect.top)
            .then_with(|| a.device_pixel_rect.left.cmp(&b.device_pixel_rect.left))
            .then_with(|| a.device_pixel_rect.width.cmp(&b.device_pixel_rect.width))
            .then_with(|| a.device_pixel_rect.height.cmp(&b.device_pixel_rect.height))
            .then_with(|| a.canonical_region_id.cmp(&b.canonical_region_id))
    });

    let merged_rects: Vec<DevicePixelRect> = items
        .iter()
        .map(|item| item.device_pixel_rect.clone())
        .collect();

    Ok((merged_rects, items))
}

// ---------------------------------------------------------------------------
// 6. Deterministic PDFium Render & Opaque Overwrite Pipeline
// ---------------------------------------------------------------------------

pub fn render_affected_page_raster(
    pdfium_page: &PdfPage<'_>,
    spec: &RedactionRasterSpec,
    cancellation: Option<&SessionJobCancellationCheckpoint>,
) -> Result<
    (RedactionRasterBuffer, PdfRenderConfig),
    crate::document_session::SessionJobOperationError,
> {
    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    let render_config = create_redaction_render_config(pdfium_page, spec)?;

    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    let bitmap = pdfium_page
        .render_with_config(&render_config)
        .map_err(|e| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                format!("PDFium page render failed: {e}"),
            )
        })?;

    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    let width = bitmap.width() as u32;
    let height = bitmap.height() as u32;

    if width != spec.viewport_width || height != spec.viewport_height {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "Rendered bitmap dimensions ({}x{}) do not match expected mapping viewport ({}x{})",
                width, height, spec.viewport_width, spec.viewport_height
            ),
        ));
    }

    let rgba_bytes = bitmap.as_rgba_bytes();

    let stride = (width as usize).checked_mul(4).ok_or_else(|| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Stride calculation overflow",
        )
    })?;

    let expected_len = (height as usize).checked_mul(stride).ok_or_else(|| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Bitmap pixel buffer length overflow",
        )
    })?;

    if rgba_bytes.len() != expected_len {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "PDFium RGBA bitmap buffer length ({}) does not match expected length ({expected_len})",
                rgba_bytes.len()
            ),
        ));
    }

    if expected_len > MAX_RENDER_BYTES {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Rendered bitmap buffer size ({expected_len}) exceeds limit ({MAX_RENDER_BYTES})"
            ),
        ));
    }

    Ok((
        RedactionRasterBuffer {
            width,
            height,
            stride,
            pixel_format: RedactionPixelFormat::Rgba8,
            pixels: rgba_bytes,
        },
        render_config,
    ))
}

pub fn apply_opaque_black_overwrite(
    buffer: &mut RedactionRasterBuffer,
    merged_rects: &[DevicePixelRect],
    cancellation: Option<&SessionJobCancellationCheckpoint>,
) -> Result<(), crate::document_session::SessionJobOperationError> {
    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    let expected_stride = (buffer.width as usize).checked_mul(4).ok_or_else(|| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Buffer width stride overflow",
        )
    })?;

    if buffer.stride != expected_stride {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Invalid buffer stride: expected tightly-packed {} but got {}",
                expected_stride, buffer.stride
            ),
        ));
    }

    let expected_pixel_bytes = expected_stride
        .checked_mul(buffer.height as usize)
        .ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Buffer pixel byte count overflow",
            )
        })?;

    if buffer.pixels.len() != expected_pixel_bytes {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Invalid buffer pixel byte length: expected {} but got {}",
                expected_pixel_bytes,
                buffer.pixels.len()
            ),
        ));
    }

    let buf_width = buffer.width as usize;
    let buf_height = buffer.height as usize;
    let pixels = &mut buffer.pixels;

    for (rect_idx, rect) in merged_rects.iter().enumerate() {
        if let Some(cancel) = cancellation {
            if rect_idx % 10 == 0 {
                cancel.check_cancelled()?;
            }
        }

        let left = rect.left.clamp(0, buffer.width as i32) as usize;
        let top = rect.top.clamp(0, buffer.height as i32) as usize;

        let right_i32 = rect.left.checked_add(rect.width as i32).ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Rect right coordinate addition overflow",
            )
        })?;
        let bottom_i32 = rect.top.checked_add(rect.height as i32).ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Rect bottom coordinate addition overflow",
            )
        })?;

        let right = right_i32.clamp(0, buffer.width as i32) as usize;
        let bottom = bottom_i32.clamp(0, buffer.height as i32) as usize;

        for (row_count, y) in (top..bottom).enumerate() {
            if let Some(cancel) = cancellation {
                if row_count > 0 && row_count % 64 == 0 {
                    cancel.check_cancelled()?;
                }
            }

            if y >= buf_height {
                continue;
            }
            let row_start = y.checked_mul(expected_stride).ok_or_else(|| {
                make_redaction_error(
                    SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                    "Row start index overflow during pixel paint",
                )
            })?;

            for x in left..right {
                if x >= buf_width {
                    continue;
                }
                let x_offset = x.checked_mul(4).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "X offset overflow during pixel paint",
                    )
                })?;
                let pixel_offset = row_start.checked_add(x_offset).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Pixel offset overflow during pixel paint",
                    )
                })?;

                let alpha_offset = pixel_offset.checked_add(3).ok_or_else(|| {
                    make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Alpha channel offset overflow during pixel paint",
                    )
                })?;

                if alpha_offset < pixels.len() {
                    pixels[pixel_offset] = 0; // R = 0
                    pixels[pixel_offset + 1] = 0; // G = 0
                    pixels[pixel_offset + 2] = 0; // B = 0
                    pixels[pixel_offset + 3] = 255; // A = 255
                }
            }
        }
    }

    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    Ok(())
}

pub fn verify_opaque_black_overwrite(
    buffer: &RedactionRasterBuffer,
    merged_rects: &[DevicePixelRect],
) -> Result<(), crate::document_session::SessionJobOperationError> {
    let buf_width = buffer.width as usize;
    let buf_height = buffer.height as usize;
    let pixels = &buffer.pixels;

    for rect in merged_rects {
        let left = rect.left.clamp(0, buffer.width as i32) as usize;
        let top = rect.top.clamp(0, buffer.height as i32) as usize;

        let right_i32 = rect.left.checked_add(rect.width as i32).ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Rect right coordinate addition overflow",
            )
        })?;
        let bottom_i32 = rect.top.checked_add(rect.height as i32).ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Rect bottom coordinate addition overflow",
            )
        })?;

        let right = right_i32.clamp(0, buffer.width as i32) as usize;
        let bottom = bottom_i32.clamp(0, buffer.height as i32) as usize;

        for y in top..bottom {
            if y >= buf_height {
                continue;
            }
            let row_start = y.checked_mul(buf_width * 4).ok_or_else(|| {
                make_redaction_error(
                    SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                    "Row start index overflow during paint verification",
                )
            })?;

            for x in left..right {
                if x >= buf_width {
                    continue;
                }
                let offset = row_start + x * 4;
                if offset + 3 >= pixels.len() {
                    return Err(make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        "Pixel index out of bounds during paint verification",
                    ));
                }
                if pixels[offset..offset + 4] != [0, 0, 0, 255] {
                    let actual = &pixels[offset..offset + 4];
                    return Err(make_redaction_error(
                        SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                        format!(
                            "Pixel paint verification failed in expected device rect \
                             ({}, {}, {}, {}) at ({x}, {y}): expected [0, 0, 0, 255], \
                             actual [{}, {}, {}, {}]",
                            rect.left,
                            rect.top,
                            rect.width,
                            rect.height,
                            actual[0],
                            actual[1],
                            actual[2],
                            actual[3],
                        ),
                    ));
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 7. Complete Lossless Raster Payload Pipeline
// ---------------------------------------------------------------------------

fn pdfium_boundary_to_pdf_point_rect(
    bounds: PdfPageBoundaryBox,
    box_name: &str,
    page_index: u32,
) -> Result<PdfPointRect, crate::document_session::SessionJobOperationError> {
    let left = bounds.bounds.left().value as f64;
    let bottom = bounds.bounds.bottom().value as f64;
    let right = bounds.bounds.right().value as f64;
    let top = bounds.bounds.top().value as f64;
    let width = right - left;
    let height = top - bottom;

    if !left.is_finite()
        || !bottom.is_finite()
        || !right.is_finite()
        || !top.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "PDFium {box_name} for page_index {page_index} is non-finite or has non-positive dimensions"
            ),
        ));
    }

    Ok(PdfPointRect {
        left,
        bottom,
        width,
        height,
    })
}

fn validate_pdf_point_rect_identity(
    actual: &PdfPointRect,
    expected: &PdfPointRect,
    label: &str,
    page_index: u32,
) -> Result<(), crate::document_session::SessionJobOperationError> {
    let differs = (actual.left - expected.left).abs() > GEOMETRY_EPSILON
        || (actual.bottom - expected.bottom).abs() > GEOMETRY_EPSILON
        || (actual.width - expected.width).abs() > GEOMETRY_EPSILON
        || (actual.height - expected.height).abs() > GEOMETRY_EPSILON;

    if differs {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "PDFium {label} ({:.6}, {:.6}, {:.6}, {:.6}) does not match plan ({:.6}, {:.6}, {:.6}, {:.6}) for page_index {page_index}",
                actual.left,
                actual.bottom,
                actual.width,
                actual.height,
                expected.left,
                expected.bottom,
                expected.width,
                expected.height,
            ),
        ));
    }

    Ok(())
}

pub fn validate_pdfium_page_geometry(
    pdfium_page: &PdfPage<'_>,
    expected: &CanonicalPageGeometry,
    page_index: u32,
) -> Result<(), crate::document_session::SessionJobOperationError> {
    // This mirrors the planner's StandardEffectiveBoxV1 semantics: MediaBox is
    // mandatory; CropBox is effective only when it differs from MediaBox.
    let media = pdfium_page
        .boundaries()
        .media()
        .map_err(|e| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Failed to read PDFium MediaBox for page_index {page_index}: {e}"),
            )
        })
        .and_then(|bounds| pdfium_boundary_to_pdf_point_rect(bounds, "MediaBox", page_index))?;

    let crop = match pdfium_page.boundaries().crop() {
        Ok(bounds) => {
            let crop = pdfium_boundary_to_pdf_point_rect(bounds, "CropBox", page_index)?;
            let same_as_media = (crop.left - media.left).abs() <= GEOMETRY_EPSILON
                && (crop.bottom - media.bottom).abs() <= GEOMETRY_EPSILON
                && (crop.width - media.width).abs() <= GEOMETRY_EPSILON
                && (crop.height - media.height).abs() <= GEOMETRY_EPSILON;
            (!same_as_media).then_some(crop)
        }
        // PDFium reports an absent optional CropBox as an error. It is only
        // accepted when the immutable plan also records no distinct CropBox.
        Err(_e) if expected.crop_box.is_none() => None,
        Err(e) => {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Failed to read PDFium CropBox for page_index {page_index}: {e}"),
            ));
        }
    };

    let intrinsic_rotation = match pdfium_page.rotation() {
        Ok(PdfPageRenderRotation::None) => PageRotation::Deg0,
        Ok(PdfPageRenderRotation::Degrees90) => PageRotation::Deg90,
        Ok(PdfPageRenderRotation::Degrees180) => PageRotation::Deg180,
        Ok(PdfPageRenderRotation::Degrees270) => PageRotation::Deg270,
        Err(e) => {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Failed to read PDF page rotation for page_index {page_index}: {e}"),
            ));
        }
    };

    validate_pdf_point_rect_identity(&media, &expected.media_box, "MediaBox", page_index)?;
    match (&crop, &expected.crop_box) {
        (Some(actual), Some(expected)) => {
            validate_pdf_point_rect_identity(actual, expected, "CropBox", page_index)?;
        }
        (None, None) => {}
        _ => {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("PDFium CropBox presence does not match plan for page_index {page_index}"),
            ));
        }
    }

    for (label, actual, expected) in [
        (
            "TrimBox",
            pdfium_page.boundaries().trim().ok(),
            expected.trim_box.as_ref(),
        ),
        (
            "BleedBox",
            pdfium_page.boundaries().bleed().ok(),
            expected.bleed_box.as_ref(),
        ),
        (
            "ArtBox",
            pdfium_page.boundaries().art().ok(),
            expected.art_box.as_ref(),
        ),
    ] {
        match (actual, expected) {
            (Some(actual), Some(expected)) => {
                let actual = pdfium_boundary_to_pdf_point_rect(actual, label, page_index)?;
                validate_pdf_point_rect_identity(&actual, expected, label, page_index)?;
            }
            (None, None) => {}
            _ => {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                    format!(
                        "PDFium {label} presence does not match plan for page_index {page_index}"
                    ),
                ));
            }
        }
    }

    let effective = crop.as_ref().unwrap_or(&media);
    validate_pdf_point_rect_identity(
        effective,
        &expected.effective_box,
        "effective box",
        page_index,
    )?;

    if intrinsic_rotation != expected.rotation {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "PDFium intrinsic rotation {:?} does not match plan rotation {:?} for page_index {page_index}",
                intrinsic_rotation, expected.rotation
            ),
        ));
    }

    Ok(())
}

/// Maps the display-oriented raster produced by the 8B-5 primitive back into
/// physical page coordinates. The mapping is sampled from PDFium's own
/// `pixels_to_points()` transform rather than inferred from /Rotate formulas.
pub fn raster_image_placement_matrix(
    pdfium_page: &PdfPage<'_>,
    geometry: &CanonicalPageGeometry,
    bitmap_width: u32,
    bitmap_height: u32,
) -> Result<PdfMatrix, crate::document_session::SessionJobOperationError> {
    let width = i32::try_from(bitmap_width).map_err(|_| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Raster bitmap width exceeds PDFium coordinate limit",
        )
    })?;
    let height = i32::try_from(bitmap_height).map_err(|_| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Raster bitmap height exceeds PDFium coordinate limit",
        )
    })?;
    let spec = RedactionRasterSpec {
        page_index: geometry.page_index,
        dpi: MIN_RASTER_DPI,
        rotation: geometry.rotation,
        viewport_width: bitmap_width,
        viewport_height: bitmap_height,
        background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
        annotation_policy: RedactionAnnotationPolicy::DoNotRender,
        widget_form_policy: RedactionWidgetFormPolicy::BlockedIfPresent,
    };
    let config = create_redaction_render_config(pdfium_page, &spec)?;
    let (bl_x, bl_y) = pdfium_page
        .pixels_to_points(0, height, &config)
        .map_err(|e| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Unable to map raster lower-left corner: {e}"),
            )
        })?;
    let (br_x, br_y) = pdfium_page
        .pixels_to_points(width, height, &config)
        .map_err(|e| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Unable to map raster lower-right corner: {e}"),
            )
        })?;
    let (tl_x, tl_y) = pdfium_page.pixels_to_points(0, 0, &config).map_err(|e| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!("Unable to map raster upper-left corner: {e}"),
        )
    })?;

    Ok(PdfMatrix::new(
        br_x.value - bl_x.value,
        br_y.value - bl_y.value,
        tl_x.value - bl_x.value,
        tl_y.value - bl_y.value,
        bl_x.value,
        bl_y.value,
    ))
}

pub fn build_lossless_raster_payload_for_page(
    plan: &SecureRedactionPlan,
    document: &PdfDocument<'_>,
    page_index: u32,
    cancellation: Option<&SessionJobCancellationCheckpoint>,
) -> Result<LosslessRasterPayload, crate::document_session::SessionJobOperationError> {
    // Checkpoint 1: Before raster preflight & full plan readiness validation
    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    if !plan.executable || !plan.ready_for_execution || !plan.blockers.is_empty() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionBlockedDocumentFeature,
            format!(
                "Full plan (digest {}) is not executable, not ready for execution, or contains document-level blockers. Rasterization refused.",
                plan.plan_digest
            ),
        ));
    }

    let page_plan = plan
        .page_plans
        .iter()
        .find(|p| p.page_index == page_index)
        .ok_or_else(|| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!("Page plan for page_index {page_index} not found in plan"),
            )
        })?;

    validate_canonical_page_geometry(&page_plan.geometry)?;

    let pdfium_page_index = u16::try_from(page_index).map_err(|_| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionPageNotFound,
            format!("page_index {page_index} cannot be represented by PDFium"),
        )
    })?;
    let pdfium_page = document.pages().get(pdfium_page_index).map_err(|e| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionPageNotFound,
            format!("Failed to acquire page_index {page_index} from bound document: {e}"),
        )
    })?;

    // Page identity authority is the bound document plus exact page index. Do
    // not accept a caller-provided PdfPage handle at this plan-bound boundary.
    validate_pdfium_page_geometry(&pdfium_page, &page_plan.geometry, page_index)?;

    if !page_plan.executable || !page_plan.blockers.is_empty() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionBlockedDocumentFeature,
            format!(
                "Page {} plan is not executable or contains blockers. Rasterization refused.",
                page_index
            ),
        ));
    }

    if page_plan.strategy != crate::secure_redaction::PageRedactionStrategy::RasterizePage {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Page {} strategy is {:?}, but raster primitive requires RasterizePage strategy",
                page_index, page_plan.strategy
            ),
        ));
    }

    if page_plan.canonical_regions.is_empty() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            format!(
                "Page {} has strategy RasterizePage but its canonical regions list is empty",
                page_index
            ),
        ));
    }

    if page_plan.canonical_regions.len() > MAX_CANONICAL_REGIONS_PER_PAGE {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Page {} canonical regions count ({}) exceeds maximum limit ({MAX_CANONICAL_REGIONS_PER_PAGE})",
                page_index,
                page_plan.canonical_regions.len()
            ),
        ));
    }

    let policy = &plan.policy;

    let (viewport_width, viewport_height) =
        calculate_raster_dimensions(&page_plan.geometry, policy.raster_dpi)?;

    let spec = RedactionRasterSpec {
        page_index: page_plan.page_index,
        dpi: policy.raster_dpi,
        rotation: page_plan.geometry.rotation,
        viewport_width,
        viewport_height,
        background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
        annotation_policy: RedactionAnnotationPolicy::DoNotRender,
        widget_form_policy: RedactionWidgetFormPolicy::BlockedIfPresent,
    };

    // Checkpoint 2: PDFium render with shared render_config
    let (mut buffer, render_config) =
        render_affected_page_raster(&pdfium_page, &spec, cancellation)?;

    // Checkpoint 4: Device mapping with points_to_pixels and the same render_config
    let mut unmerged_mappings = Vec::with_capacity(page_plan.canonical_regions.len());
    for canonical_region in &page_plan.canonical_regions {
        if let Some(cancel) = cancellation {
            cancel.check_cancelled()?;
        }

        let (dev_rect, boundary_clamped) = map_pdf_rect_to_device_pixel_rect(
            &pdfium_page,
            &canonical_region.applied_pdf_point_rect,
            &render_config,
            viewport_width,
            viewport_height,
        )?;

        unmerged_mappings.push(RasterizedRegionMapping {
            page_index: page_plan.page_index,
            canonical_region_id: canonical_region.region_id.clone(),
            source_region_ids: canonical_region.source_region_ids.clone(),
            applied_pdf_point_rect: canonical_region.applied_pdf_point_rect.clone(),
            device_pixel_rect: dev_rect,
            bitmap_width: viewport_width,
            bitmap_height: viewport_height,
            boundary_clamped,
            rotation: spec.rotation,
            viewport_width,
            viewport_height,
            dpi: policy.raster_dpi,
        });
    }

    // Checkpoint 5: Rectangle merge
    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    let (merged_rects, _merged_mappings) = merge_device_pixel_rects(&unmerged_mappings)?;

    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    // Checkpoint 6: Opaque black pixel overwrite & verification
    apply_opaque_black_overwrite(&mut buffer, &merged_rects, cancellation)?;
    verify_opaque_black_overwrite(&buffer, &merged_rects)?;

    // Checkpoint 7: Lossless payload construction
    if let Some(cancel) = cancellation {
        cancel.check_cancelled()?;
    }

    Ok(LosslessRasterPayload {
        page_index: page_plan.page_index,
        width: buffer.width,
        height: buffer.height,
        stride: buffer.stride,
        pixel_format: buffer.pixel_format,
        background_policy: spec.background_policy,
        pixels: buffer.pixels,
        region_mappings: unmerged_mappings,
        merged_device_pixel_rects: merged_rects,
    })
}
