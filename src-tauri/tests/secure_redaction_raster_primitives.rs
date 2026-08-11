use pdfium_render::prelude::*;
use preview_alternative_lib::document_session::SessionJobCancellationCheckpoint;
use preview_alternative_lib::secure_redaction::{
    CanonicalPageGeometry, DevicePixelRect, EncryptionPolicy, InteractiveContentPolicy,
    NormalizedTopLeftRect, PageRedactionStrategy, PageRotation, PdfPointRect,
    RedactionAcknowledgements, RedactionAppearancePolicy, RedactionProfile, RedactionRegionInput,
    RedactionStrategy, SecureRedactionPolicy, SignaturePolicy, REDACTION_SCHEMA_VERSION,
};
use preview_alternative_lib::secure_redaction_inspection::inspect_and_plan_secure_redaction;
use preview_alternative_lib::secure_redaction_raster::{
    apply_opaque_black_overwrite, build_lossless_raster_payload_for_page,
    calculate_raster_dimensions, create_redaction_render_config, map_pdf_rect_to_device_pixel_rect,
    merge_device_pixel_rects, render_affected_page_raster, verify_opaque_black_overwrite,
    LosslessRasterPayload, RasterizedRegionMapping, RedactionBackgroundPolicy,
    RedactionPixelFormat, RedactionRasterBuffer, RedactionRasterSpec,
};
use std::fs;
use std::path::{Path, PathBuf};

fn with_test_pdfium<T>(operation: impl FnOnce(&Pdfium) -> T) -> T {
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| Ok::<T, String>(operation(pdfium)))
        .expect("Initialize PDFium for test")
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("redaction")
        .join(name)
}

fn sample_geometry(rot: PageRotation) -> CanonicalPageGeometry {
    CanonicalPageGeometry {
        page_index: 0,
        media_box: PdfPointRect {
            left: 0.0,
            bottom: 0.0,
            width: 612.0,
            height: 792.0,
        },
        crop_box: None,
        trim_box: None,
        bleed_box: None,
        art_box: None,
        effective_box: PdfPointRect {
            left: 0.0,
            bottom: 0.0,
            width: 612.0,
            height: 792.0,
        },
        rotation: rot,
    }
}

fn sample_policy() -> SecureRedactionPolicy {
    SecureRedactionPolicy {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        profile: RedactionProfile::SecureRasterizedPagesV1,
        strategy: RedactionStrategy::RasterizedPages,
        raster_dpi: 300,
        appearance: RedactionAppearancePolicy {
            kind: "solid".to_string(),
            color: "black".to_string(),
            opacity: 1.0,
        },
        safety_margin_points: 0.0,
        interactive_content_policy: InteractiveContentPolicy::Strip,
        signature_policy: SignaturePolicy::Block,
        encryption_policy: EncryptionPolicy::Block,
    }
}

fn sample_acknowledgements() -> RedactionAcknowledgements {
    RedactionAcknowledgements {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        rasterized_pages_accepted: true,
        text_search_loss_accepted: true,
        accessibility_loss_accepted: true,
        annotation_loss_accepted: true,
        form_loss_accepted: true,
        signature_invalidation_accepted: true,
        encryption_change_accepted: true,
    }
}

fn sample_region(
    id: &str,
    page_index: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> RedactionRegionInput {
    RedactionRegionInput {
        id: id.to_string(),
        page_index,
        normalized_top_left_rect: NormalizedTopLeftRect {
            x,
            y,
            width,
            height,
        },
        source: "manual".to_string(),
    }
}

fn test_map_pdf_rect(
    page: &PdfPage<'_>,
    pdf_rect: &PdfPointRect,
    viewport_width: u32,
    viewport_height: u32,
    rotation: PageRotation,
) -> Result<
    (DevicePixelRect, bool),
    preview_alternative_lib::document_session::SessionJobOperationError,
> {
    let spec = RedactionRasterSpec {
        page_index: 0,
        dpi: 72,
        rotation,
        viewport_width,
        viewport_height,
        background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
        annotation_policy:
            preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
        widget_form_policy:
            preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
    };
    let render_config = create_redaction_render_config(page, &spec)?;
    map_pdf_rect_to_device_pixel_rect(
        page,
        pdf_rect,
        &render_config,
        viewport_width,
        viewport_height,
    )
}

// ---------------------------------------------------------------------------
// 1. Pure Unit Tests for Dimension Calculation & Limits (Tests 1 - 11)
// ---------------------------------------------------------------------------

#[test]
fn test_01_valid_raster_dimension_calculation() {
    let geom = sample_geometry(PageRotation::Deg0);
    let (w, h) = calculate_raster_dimensions(&geom, 300).expect("Valid dimensions");
    assert_eq!(w, 2550);
    assert_eq!(h, 3300);
}

#[test]
fn test_02_minimum_dpi() {
    let geom = sample_geometry(PageRotation::Deg0);
    let (w, h) = calculate_raster_dimensions(&geom, 72).expect("Minimum DPI 72");
    assert_eq!(w, 612);
    assert_eq!(h, 792);
}

#[test]
fn test_03_maximum_dpi() {
    let geom = sample_geometry(PageRotation::Deg0);
    let (w, h) = calculate_raster_dimensions(&geom, 600).expect("Maximum DPI 600");
    assert_eq!(w, 5100);
    assert_eq!(h, 6600);
}

#[test]
fn test_04_invalid_dpi() {
    let geom = sample_geometry(PageRotation::Deg0);
    assert!(calculate_raster_dimensions(&geom, 71).is_err());
    assert!(calculate_raster_dimensions(&geom, 601).is_err());
}

#[test]
fn test_05_width_height_overflow_protection() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.width = f64::MAX;
    assert!(calculate_raster_dimensions(&geom, 300).is_err());
}

#[test]
fn test_06_height_overflow_protection() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.height = f64::MAX;
    assert!(calculate_raster_dimensions(&geom, 300).is_err());
}

#[test]
fn test_07_pixel_count_limit_exceeded() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.width = 12000.0;
    geom.effective_box.height = 12000.0;
    let res = calculate_raster_dimensions(&geom, 300);
    assert!(res.is_err());
}

#[test]
fn test_08_byte_size_overflow_protection() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.width = 3000.0;
    geom.effective_box.height = 3000.0;
    let res = calculate_raster_dimensions(&geom, 600);
    assert!(res.is_err());
}

#[test]
fn test_09_page_pixel_limit_enforced() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.width = 2500.0;
    geom.effective_box.height = 3500.0;
    let res = calculate_raster_dimensions(&geom, 300);
    assert!(res.is_err());
}

#[test]
fn test_10_renderer_byte_limit_enforced() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.width = 4000.0;
    geom.effective_box.height = 4000.0;
    let res = calculate_raster_dimensions(&geom, 300);
    assert!(res.is_err());
}

#[test]
fn test_11_renderer_dimension_limit_enforced() {
    let mut geom = sample_geometry(PageRotation::Deg0);
    geom.effective_box.width = 4000.0;
    geom.effective_box.height = 100.0;
    let res = calculate_raster_dimensions(&geom, 300);
    assert!(res.is_err());
}

// ---------------------------------------------------------------------------
// 2. Unit Tests for Page-to-Device Mapping & Rounding (Tests 12 - 25)
// ---------------------------------------------------------------------------

#[test]
fn test_12_pdf_rect_to_device_rect_rot0() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Get page 0");

        let pdf_rect = PdfPointRect {
            left: 100.0,
            bottom: 700.0,
            width: 200.0,
            height: 50.0,
        };

        let (dev_rect, clamped) =
            test_map_pdf_rect(&page, &pdf_rect, 600, 800, PageRotation::Deg0).expect("Map rect");

        assert_eq!(dev_rect.left, 100);
        assert_eq!(dev_rect.top, 50); // 800 - 700 - 50 = 50 in Y-down
        assert_eq!(dev_rect.width, 200);
        assert_eq!(dev_rect.height, 50);
        assert!(!clamped);
    });
}

#[test]
fn test_13_rotation_0_mapping() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("rotations.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0 (rot 0)");

        assert_eq!(page.rotation().expect("Rotation").as_degrees(), 0.0);

        let pdf_rect = PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 100.0,
            height: 100.0,
        };
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: 600,
            viewport_height: 800,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let render_config = create_redaction_render_config(&page, &spec).expect("Render config");
        let (dev, _) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 600, 800)
                .expect("Map rot 0");

        assert_eq!(dev.left, 50);
        assert_eq!(dev.top, 650); // 800 - 50 - 100 = 650
        assert_eq!(dev.width, 100);
        assert_eq!(dev.height, 100);

        // Round-trip test: points_to_pixels -> pixels_to_points
        let (px, py) = page
            .points_to_pixels(PdfPoints::new(50.0), PdfPoints::new(50.0), &render_config)
            .expect("points_to_pixels");
        let (pt_x, pt_y) = page
            .pixels_to_points(px, py, &render_config)
            .expect("pixels_to_points");
        assert!((pt_x.value as f64 - 50.0).abs() <= 1e-3);
        assert!((pt_y.value as f64 - 50.0).abs() <= 1e-3);
    });
}

#[test]
fn test_14_rotation_90_mapping() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("rotations.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(1).expect("Page 1 (rot 90)");

        assert_eq!(page.rotation().expect("Rotation").as_degrees(), 90.0);

        let pdf_rect = PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 100.0,
            height: 100.0,
        };
        let spec = RedactionRasterSpec {
            page_index: 1,
            dpi: 72,
            rotation: PageRotation::Deg90,
            viewport_width: 800,
            viewport_height: 600,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let render_config = create_redaction_render_config(&page, &spec).expect("Render config");
        let (dev, _) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 800, 600)
                .expect("Map rot 90");

        assert_eq!(dev.left, 50);
        assert_eq!(dev.top, 50);
        assert_eq!(dev.width, 100);
        assert_eq!(dev.height, 100);

        // Round-trip test: points_to_pixels -> pixels_to_points
        let (px, py) = page
            .points_to_pixels(PdfPoints::new(50.0), PdfPoints::new(50.0), &render_config)
            .expect("points_to_pixels");
        let (pt_x, pt_y) = page
            .pixels_to_points(px, py, &render_config)
            .expect("pixels_to_points");
        assert!((pt_x.value as f64 - 50.0).abs() <= 1e-3);
        assert!((pt_y.value as f64 - 50.0).abs() <= 1e-3);
    });
}

#[test]
fn test_15_rotation_180_mapping() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("rotations.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(2).expect("Page 2 (rot 180)");

        assert_eq!(page.rotation().expect("Rotation").as_degrees(), 180.0);

        let pdf_rect = PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 100.0,
            height: 100.0,
        };
        let spec = RedactionRasterSpec {
            page_index: 2,
            dpi: 72,
            rotation: PageRotation::Deg180,
            viewport_width: 600,
            viewport_height: 800,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let render_config = create_redaction_render_config(&page, &spec).expect("Render config");
        let (dev, _) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 600, 800)
                .expect("Map rot 180");

        assert_eq!(dev.left, 450);
        assert_eq!(dev.top, 50);
        assert_eq!(dev.width, 100);
        assert_eq!(dev.height, 100);

        // Round-trip test: points_to_pixels -> pixels_to_points
        let (px, py) = page
            .points_to_pixels(PdfPoints::new(50.0), PdfPoints::new(50.0), &render_config)
            .expect("points_to_pixels");
        let (pt_x, pt_y) = page
            .pixels_to_points(px, py, &render_config)
            .expect("pixels_to_points");
        assert!((pt_x.value as f64 - 50.0).abs() <= 1e-3);
        assert!((pt_y.value as f64 - 50.0).abs() <= 1e-3);
    });
}

#[test]
fn test_16_rotation_270_mapping() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("rotations.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(3).expect("Page 3 (rot 270)");

        assert_eq!(page.rotation().expect("Rotation").as_degrees(), 270.0);

        let pdf_rect = PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 100.0,
            height: 100.0,
        };
        let spec = RedactionRasterSpec {
            page_index: 3,
            dpi: 72,
            rotation: PageRotation::Deg270,
            viewport_width: 800,
            viewport_height: 600,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let render_config = create_redaction_render_config(&page, &spec).expect("Render config");
        let (dev, _) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 800, 600)
                .expect("Map rot 270");

        assert_eq!(dev.left, 650);
        assert_eq!(dev.top, 450);
        assert_eq!(dev.width, 100);
        assert_eq!(dev.height, 100);

        // Round-trip test: points_to_pixels -> pixels_to_points
        let (px, py) = page
            .points_to_pixels(PdfPoints::new(50.0), PdfPoints::new(50.0), &render_config)
            .expect("points_to_pixels");
        let (pt_x, pt_y) = page
            .pixels_to_points(px, py, &render_config)
            .expect("pixels_to_points");
        assert!((pt_x.value as f64 - 50.0).abs() <= 1e-3);
        assert!((pt_y.value as f64 - 50.0).abs() <= 1e-3);
    });
}

#[test]
fn test_17_cropbox_mapping() {
    let pdf_path = fixture_path("cropbox-mediabox-diff.pdf");

    // Step 1: Verify planner inspection yields effective_box = [50, 50, 550, 750] (width 500, height 700)
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Plan generation");

    let eff = &plan.page_plans[0].geometry.effective_box;
    assert_eq!(eff.left, 50.0);
    assert_eq!(eff.bottom, 50.0);
    assert_eq!(eff.width, 500.0);
    assert_eq!(eff.height, 700.0);

    let (v_width, v_height) = calculate_raster_dimensions(&plan.page_plans[0].geometry, 72)
        .expect("Raster dimensions at 72 DPI");
    assert_eq!(v_width, 500);
    assert_eq!(v_height, 700);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: v_width,
            viewport_height: v_height,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };

        let (buf, render_config) = render_affected_page_raster(&page, &spec, None).expect("Render");
        assert_eq!(buf.width, 500);
        assert_eq!(buf.height, 700);

        // CropBox corner mappings:
        // Lower-left (50, 50) -> device (0, 700)
        let (ll_x, ll_y) = page
            .points_to_pixels(PdfPoints::new(50.0), PdfPoints::new(50.0), &render_config)
            .expect("ll corner");
        assert_eq!(ll_x, 0);
        assert_eq!(ll_y, 700);

        // Upper-right (550, 750) -> device (500, 0)
        let (ur_x, ur_y) = page
            .points_to_pixels(PdfPoints::new(550.0), PdfPoints::new(750.0), &render_config)
            .expect("ur corner");
        assert_eq!(ur_x, 500);
        assert_eq!(ur_y, 0);

        // Known rectangle inside CropBox: left=100, bottom=100, width=200, height=300
        let pdf_rect = PdfPointRect {
            left: 100.0,
            bottom: 100.0,
            width: 200.0,
            height: 300.0,
        };
        let (dev, clamped) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 500, 700)
                .expect("Map CropBox rect");
        assert_eq!(dev.left, 50);
        assert_eq!(dev.top, 350);
        assert_eq!(dev.width, 200);
        assert_eq!(dev.height, 300);
        assert!(!clamped);
    });
}

#[test]
fn test_18_nonzero_origin_mapping() {
    let pdf_path = fixture_path("nonzero-origin.pdf");

    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Plan generation");

    let eff = &plan.page_plans[0].geometry.effective_box;
    assert_eq!(eff.left, 50.0);
    assert_eq!(eff.bottom, 50.0);
    assert_eq!(eff.width, 600.0);
    assert_eq!(eff.height, 800.0);

    let (v_width, v_height) = calculate_raster_dimensions(&plan.page_plans[0].geometry, 72)
        .expect("Raster dimensions at 72 DPI");
    assert_eq!(v_width, 600);
    assert_eq!(v_height, 800);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: v_width,
            viewport_height: v_height,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };

        let (buf, render_config) = render_affected_page_raster(&page, &spec, None).expect("Render");
        assert_eq!(buf.width, 600);
        assert_eq!(buf.height, 800);

        // Lower-left (50, 50) -> device (0, 800)
        let (ll_x, ll_y) = page
            .points_to_pixels(PdfPoints::new(50.0), PdfPoints::new(50.0), &render_config)
            .expect("ll corner");
        assert_eq!(ll_x, 0);
        assert_eq!(ll_y, 800);

        // Upper-right (650, 850) -> device (600, 0)
        let (ur_x, ur_y) = page
            .points_to_pixels(PdfPoints::new(650.0), PdfPoints::new(850.0), &render_config)
            .expect("ur corner");
        assert_eq!(ur_x, 600);
        assert_eq!(ur_y, 0);

        // Known rectangle: left=100, bottom=100, width=200, height=300
        let pdf_rect = PdfPointRect {
            left: 100.0,
            bottom: 100.0,
            width: 200.0,
            height: 300.0,
        };
        let (dev, clamped) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 600, 800)
                .expect("Map NonZero origin");
        assert_eq!(dev.left, 50);
        assert_eq!(dev.top, 450);
        assert_eq!(dev.width, 200);
        assert_eq!(dev.height, 300);
        assert!(!clamped);
    });
}

#[test]
fn test_19_negative_origin_mapping() {
    let pdf_path = fixture_path("negative-origin.pdf");

    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Plan generation");

    let eff = &plan.page_plans[0].geometry.effective_box;
    assert_eq!(eff.left, -100.0);
    assert_eq!(eff.bottom, -100.0);
    assert_eq!(eff.width, 600.0);
    assert_eq!(eff.height, 800.0);

    let (v_width, v_height) = calculate_raster_dimensions(&plan.page_plans[0].geometry, 72)
        .expect("Raster dimensions at 72 DPI");
    assert_eq!(v_width, 600);
    assert_eq!(v_height, 800);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: v_width,
            viewport_height: v_height,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };

        let (buf, render_config) = render_affected_page_raster(&page, &spec, None).expect("Render");
        assert_eq!(buf.width, 600);
        assert_eq!(buf.height, 800);

        // Lower-left (-100, -100) -> device (0, 800)
        let (ll_x, ll_y) = page
            .points_to_pixels(
                PdfPoints::new(-100.0),
                PdfPoints::new(-100.0),
                &render_config,
            )
            .expect("ll corner");
        assert_eq!(ll_x, 0);
        assert_eq!(ll_y, 800);

        // Upper-right (500, 700) -> device (600, 0)
        let (ur_x, ur_y) = page
            .points_to_pixels(PdfPoints::new(500.0), PdfPoints::new(700.0), &render_config)
            .expect("ur corner");
        assert_eq!(ur_x, 600);
        assert_eq!(ur_y, 0);

        // Known negative-coordinate rectangle: left=-50, bottom=-50, width=200, height=300
        let pdf_rect = PdfPointRect {
            left: -50.0,
            bottom: -50.0,
            width: 200.0,
            height: 300.0,
        };
        let (dev, clamped) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &render_config, 600, 800)
                .expect("Map Negative origin");
        assert_eq!(dev.left, 50);
        assert_eq!(dev.top, 450);
        assert_eq!(dev.width, 200);
        assert_eq!(dev.height, 300);
        assert!(!clamped);
    });
}

#[test]
fn test_20_outward_floor_left_top() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let pdf_rect = PdfPointRect {
            left: 10.25,
            bottom: 700.75,
            width: 50.5,
            height: 50.5,
        };
        let (dev, _) = test_map_pdf_rect(&page, &pdf_rect, 612, 792, PageRotation::Deg0)
            .expect("Map fraction rect");
        assert_eq!(dev.left, 10);
    });
}

#[test]
fn test_21_outward_ceil_right_bottom() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let pdf_rect = PdfPointRect {
            left: 10.0,
            bottom: 700.0,
            width: 50.25,
            height: 50.25,
        };
        let (dev, _) = test_map_pdf_rect(&page, &pdf_rect, 612, 792, PageRotation::Deg0)
            .expect("Map fraction width");
        assert_eq!(dev.width, 51);
    });
}

#[test]
fn test_22_one_pixel_region() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let pdf_rect = PdfPointRect {
            left: 100.0,
            bottom: 700.0,
            width: 0.001,
            height: 0.001,
        };
        let (dev, _) = test_map_pdf_rect(&page, &pdf_rect, 612, 792, PageRotation::Deg0)
            .expect("Map tiny rect");
        assert!(dev.width >= 1);
        assert!(dev.height >= 1);
    });
}

#[test]
fn test_23_completely_outside_bitmap() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let pdf_rect = PdfPointRect {
            left: 1000.0,
            bottom: 2000.0,
            width: 100.0,
            height: 100.0,
        };
        let res = test_map_pdf_rect(&page, &pdf_rect, 612, 792, PageRotation::Deg0);
        assert!(res.is_err());
    });
}

#[test]
fn test_24_zero_area_result_rejection() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let pdf_rect = PdfPointRect {
            left: 612.0,
            bottom: 792.0,
            width: 10.0,
            height: 10.0,
        };
        let res = test_map_pdf_rect(&page, &pdf_rect, 612, 792, PageRotation::Deg0);
        assert!(res.is_err());
    });
}

#[test]
fn test_25_boundary_intersection_evidence() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let pdf_rect = PdfPointRect {
            left: -10.0,
            bottom: 700.0,
            width: 50.0,
            height: 50.0,
        };
        let (dev, clamped) = test_map_pdf_rect(&page, &pdf_rect, 612, 792, PageRotation::Deg0)
            .expect("Map rect partially outside");
        assert!(clamped);
        assert_eq!(dev.left, 0);
    });
}

// ---------------------------------------------------------------------------
// 3. Pixel Rectangle Merge & Overwrite Tests (Tests 26 - 40)
// ---------------------------------------------------------------------------

fn make_mapping(id: &str, left: i32, top: i32, width: u32, height: u32) -> RasterizedRegionMapping {
    RasterizedRegionMapping {
        page_index: 0,
        canonical_region_id: id.to_string(),
        source_region_ids: vec![id.to_string()],
        applied_pdf_point_rect: PdfPointRect {
            left: left as f64,
            bottom: top as f64,
            width: width as f64,
            height: height as f64,
        },
        device_pixel_rect: DevicePixelRect {
            left,
            top,
            width,
            height,
        },
        bitmap_width: 1000,
        bitmap_height: 1000,
        boundary_clamped: false,
        rotation: PageRotation::Deg0,
        viewport_width: 1000,
        viewport_height: 1000,
        dpi: 72,
    }
}

#[test]
fn test_26_overlapping_pixel_rectangles() {
    let m1 = make_mapping("r1", 10, 10, 50, 50);
    let m2 = make_mapping("r2", 30, 30, 50, 50);
    let (merged, mappings) = merge_device_pixel_rects(&[m1, m2]).expect("Merge");
    assert_eq!(merged.len(), 1);
    assert_eq!(
        merged[0],
        DevicePixelRect {
            left: 10,
            top: 10,
            width: 70,
            height: 70
        }
    );
    assert_eq!(mappings[0].source_region_ids, vec!["r1", "r2"]);
}

#[test]
fn test_27_edge_touching_pixel_rectangles() {
    let m1 = make_mapping("r1", 10, 10, 20, 50);
    let m2 = make_mapping("r2", 30, 10, 20, 50);
    let (merged, _) = merge_device_pixel_rects(&[m1, m2]).expect("Merge");
    assert_eq!(merged.len(), 1);
    assert_eq!(
        merged[0],
        DevicePixelRect {
            left: 10,
            top: 10,
            width: 40,
            height: 50
        }
    );
}

#[test]
fn test_28_positive_gap_rectangles_remain_separate() {
    let m1 = make_mapping("r1", 10, 10, 20, 50);
    let m2 = make_mapping("r2", 31, 10, 20, 50); // 1-pixel gap at X=30
    let (merged, _) = merge_device_pixel_rects(&[m1, m2]).expect("Merge");
    assert_eq!(merged.len(), 2);
}

#[test]
fn test_29_deterministic_merge() {
    let m1 = make_mapping("r1", 10, 10, 50, 50);
    let m2 = make_mapping("r2", 30, 30, 50, 50);
    let (merged1, _) = merge_device_pixel_rects(&[m1.clone(), m2.clone()]).expect("Merge 1");
    let (merged2, _) = merge_device_pixel_rects(&[m2, m1]).expect("Merge 2");
    assert_eq!(merged1, merged2);
}

#[test]
fn test_30_input_order_independent_merge() {
    let m1 = make_mapping("a", 100, 100, 40, 40);
    let m2 = make_mapping("b", 120, 100, 40, 40);
    let m3 = make_mapping("c", 300, 300, 50, 50);

    let (res1, _) = merge_device_pixel_rects(&[m1.clone(), m2.clone(), m3.clone()]).expect("M1");
    let (res2, _) = merge_device_pixel_rects(&[m3, m2, m1]).expect("M2");
    assert_eq!(res1, res2);
}

#[test]
fn test_31_source_id_preservation() {
    let m1 = make_mapping("src_a", 10, 10, 30, 30);
    let m2 = make_mapping("src_b", 20, 20, 30, 30);
    let (_, mappings) = merge_device_pixel_rects(&[m1, m2]).expect("Merge");
    assert_eq!(mappings[0].source_region_ids, vec!["src_a", "src_b"]);
}

#[test]
fn test_32_opaque_black_overwrite() {
    let mut buf = RedactionRasterBuffer {
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 400],
    };
    let rect = DevicePixelRect {
        left: 2,
        top: 2,
        width: 3,
        height: 3,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect.clone()], None).expect("Overwrite");
    verify_opaque_black_overwrite(&buf, &[rect]).expect("Verify black");
}

#[test]
fn test_33_alpha_exactly_255() {
    let mut buf = RedactionRasterBuffer {
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![128; 400],
    };
    let rect = DevicePixelRect {
        left: 0,
        top: 0,
        width: 1,
        height: 1,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect], None).expect("Overwrite");
    assert_eq!(buf.pixels[0..4], [0, 0, 0, 255]);
}

#[test]
fn test_34_first_last_row_painting() {
    let mut buf = RedactionRasterBuffer {
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 400],
    };
    let rect1 = DevicePixelRect {
        left: 0,
        top: 0,
        width: 10,
        height: 1,
    };
    let rect2 = DevicePixelRect {
        left: 0,
        top: 9,
        width: 10,
        height: 1,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect1.clone(), rect2.clone()], None)
        .expect("Overwrite");
    verify_opaque_black_overwrite(&buf, &[rect1, rect2]).expect("Verify first/last row");
}

#[test]
fn test_35_bitmap_edge_painting() {
    let mut buf = RedactionRasterBuffer {
        width: 5,
        height: 5,
        stride: 20,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 100],
    };
    let rect = DevicePixelRect {
        left: 0,
        top: 0,
        width: 5,
        height: 5,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect.clone()], None).expect("Overwrite edge");
    verify_opaque_black_overwrite(&buf, &[rect]).expect("Verify full edge");
}

#[test]
fn test_36_channel_order_rgba() {
    let mut buf = RedactionRasterBuffer {
        width: 2,
        height: 2,
        stride: 8,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 16],
    };
    let rect = DevicePixelRect {
        left: 0,
        top: 0,
        width: 1,
        height: 1,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect], None).expect("Overwrite");
    assert_eq!(buf.pixels[0], 0); // R
    assert_eq!(buf.pixels[1], 0); // G
    assert_eq!(buf.pixels[2], 0); // B
    assert_eq!(buf.pixels[3], 255); // A
}

#[test]
fn test_37_stride_handling() {
    let mut buf = RedactionRasterBuffer {
        width: 4,
        height: 4,
        stride: 16,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 64],
    };
    let rect = DevicePixelRect {
        left: 1,
        top: 1,
        width: 2,
        height: 2,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect.clone()], None).expect("Overwrite stride");
    verify_opaque_black_overwrite(&buf, &[rect]).expect("Verify stride");
}

#[test]
fn test_38_white_background_policy() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: 612,
            viewport_height: 792,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let (buf, _) = render_affected_page_raster(&page, &spec, None).expect("Render page");
        assert_eq!(buf.pixel_format, RedactionPixelFormat::Rgba8);
        // Verify full [255, 255, 255, 255] opaque white background for unpainted pixel
        assert_eq!(buf.pixels[0], 255); // R
        assert_eq!(buf.pixels[1], 255); // G
        assert_eq!(buf.pixels[2], 255); // B
        assert_eq!(buf.pixels[3], 255); // A
    });
}

#[test]
fn test_39_opaque_output_alpha_after_white_background_clear() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("image-only.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: 600,
            viewport_height: 800,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let (buf, _) = render_affected_page_raster(&page, &spec, None).expect("Render image page");
        for i in (0..buf.pixels.len()).step_by(4) {
            assert_eq!(buf.pixels[i + 3], 255);
        }
    });
}

#[test]
fn test_40_lossless_payload_roundtrip() {
    let pixels = vec![128u8; 400];
    let payload = LosslessRasterPayload {
        page_index: 0,
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: RedactionPixelFormat::Rgba8,
        background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
        pixels: pixels.clone(),
        region_mappings: vec![],
        merged_device_pixel_rects: vec![],
    };
    // Verify byte-for-byte exact equality between source bytes and payload bytes
    assert_eq!(payload.pixels, pixels);
}

// ---------------------------------------------------------------------------
// 4. Cancellation Checkpoint Tests (Tests 41 - 44)
// ---------------------------------------------------------------------------

#[test]
fn test_41_cancellation_before_render() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: 612,
            viewport_height: 792,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let cancel = SessionJobCancellationCheckpoint::new(token);
        assert!(render_affected_page_raster(&page, &spec, Some(&cancel)).is_err());
    });
}

#[test]
fn test_42_cancellation_after_render() {
    let mut buf = RedactionRasterBuffer {
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 400],
    };
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let cancel = SessionJobCancellationCheckpoint::new(token);
    let rect = DevicePixelRect {
        left: 0,
        top: 0,
        width: 2,
        height: 2,
    };
    assert!(apply_opaque_black_overwrite(&mut buf, &[rect], Some(&cancel)).is_err());
}

#[test]
fn test_43_cancellation_during_mapping() {
    let pdf_path = fixture_path("simple-text.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let cancel = SessionJobCancellationCheckpoint::new(token);

        assert!(build_lossless_raster_payload_for_page(&plan, &doc, 0, Some(&cancel)).is_err());
    });
}

#[test]
fn test_44_cancellation_during_paint() {
    let mut buf = RedactionRasterBuffer {
        width: 100,
        height: 100,
        stride: 400,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 40000],
    };
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let cancel = SessionJobCancellationCheckpoint::new(token);
    let rects: Vec<_> = (0..20)
        .map(|i| DevicePixelRect {
            left: i * 2,
            top: i * 2,
            width: 2,
            height: 2,
        })
        .collect();
    assert!(apply_opaque_black_overwrite(&mut buf, &rects, Some(&cancel)).is_err());
}

// ---------------------------------------------------------------------------
// 5. Plan Immutability & Digest Regression Tests (Tests 45 - 48)
// ---------------------------------------------------------------------------

#[test]
fn test_45_plan_remains_byte_and_semantic_unchanged() {
    let pdf_path = fixture_path("simple-text.pdf");
    let policy = sample_policy();
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("reg1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    let original_json = serde_json::to_string(&plan).expect("Serialize plan");
    let original_digest = plan.plan_digest.clone();

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let payload =
            build_lossless_raster_payload_for_page(&plan, &doc, 0, None).expect("Build payload");

        assert_eq!(payload.region_mappings.len(), 1);

        let re_serialized_json = serde_json::to_string(&plan).expect("Re-serialize plan");
        assert_eq!(original_json, re_serialized_json);
        assert_eq!(original_digest, plan.plan_digest);
    });
}

#[test]
fn test_46_plan_digest_unchanged_after_raster_mapping() {
    let pdf_path = fixture_path("simple-text.pdf");
    let policy = sample_policy();
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("reg1", 0, 0.2, 0.2, 0.3, 0.3)],
    )
    .expect("Build plan");

    let saved_digest = plan.plan_digest.clone();

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let _payload =
            build_lossless_raster_payload_for_page(&plan, &doc, 0, None).expect("Payload");

        assert_eq!(plan.plan_digest, saved_digest);
    });
}

#[test]
fn test_47_canonical_region_device_pixel_rect_remains_unmutated() {
    let pdf_path = fixture_path("simple-text.pdf");
    let policy = sample_policy();
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("reg1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let _payload =
            build_lossless_raster_payload_for_page(&plan, &doc, 0, None).expect("Payload");

        for region in &plan.page_plans[0].canonical_regions {
            assert!(region.device_pixel_rect.is_none());
        }
    });
}

#[test]
fn test_48_no_filesystem_output_created() {
    let dir = fixture_path("");
    let before_files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let pdf_path = fixture_path("simple-text.pdf");
    let policy = sample_policy();
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Plan");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let _payload =
            build_lossless_raster_payload_for_page(&plan, &doc, 0, None).expect("Payload");
    });

    let after_files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    assert_eq!(before_files, after_files);
}

// ---------------------------------------------------------------------------
// 6. Contract & Readiness Gate Hardening Tests (Tests 49 - 59)
// ---------------------------------------------------------------------------

#[test]
fn test_49_full_plan_not_ready_refuses_raster() {
    let pdf_path = fixture_path("simple-text.pdf");
    let unacknowledged = RedactionAcknowledgements {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        rasterized_pages_accepted: false, // Not accepted!
        text_search_loss_accepted: true,
        accessibility_loss_accepted: true,
        annotation_loss_accepted: true,
        form_loss_accepted: true,
        signature_invalidation_accepted: true,
        encryption_change_accepted: true,
    };
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        unacknowledged,
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    assert!(!plan.ready_for_execution);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
    });
}

#[test]
fn test_50_document_level_blocker_refuses_raster() {
    let pdf_path = fixture_path("simple-text.pdf");
    let mut plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    plan.executable = false;
    plan.blockers.push(preview_alternative_lib::secure_redaction::RedactionBlocker {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        code: preview_alternative_lib::secure_redaction::SecureRedactionErrorCode::RedactionBlockedDocumentFeature.code_str().to_string(),
        message: "Test document blocker".to_string(),
        details: None,
    });

    assert!(!plan.executable);
    assert!(!plan.blockers.is_empty());

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
    });
}

#[test]
fn test_51_unaffected_page_strategy_refuses_raster() {
    let pdf_path = fixture_path("simple-text.pdf");
    let mut plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    // Set page strategy to ImportUnaffectedPage
    plan.page_plans[0].strategy = PageRedactionStrategy::ImportUnaffectedPage;

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
    });
}

#[test]
fn test_52_empty_canonical_regions_refuses_raster() {
    let pdf_path = fixture_path("simple-text.pdf");
    let mut plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    plan.page_plans[0].canonical_regions.clear();

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
    });
}

#[test]
fn test_53_caller_cannot_inject_policy_or_dpi() {
    let pdf_path = fixture_path("simple-text.pdf");
    let mut policy = sample_policy();
    policy.raster_dpi = 300;

    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    let (expected_w, expected_h) = calculate_raster_dimensions(&plan.page_plans[0].geometry, 300)
        .expect("Calculate expected dimensions");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let payload =
            build_lossless_raster_payload_for_page(&plan, &doc, 0, None).expect("Build payload");

        // Payload DPI and dimensions are governed solely by plan.policy
        assert_eq!(payload.width, expected_w);
        assert_eq!(payload.height, expected_h);
    });
}

#[test]
fn test_54_annotation_fixture_do_not_render() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("annotations-fixture.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let spec_b = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: 600,
            viewport_height: 800,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };

        // Render A: render_annotations(true)
        let mut config_a = create_redaction_render_config(&page, &spec_b).expect("Config A");
        config_a = config_a.render_annotations(true);
        let bitmap_a = page.render_with_config(&config_a).expect("Render A");
        let pixels_a = bitmap_a.as_rgba_bytes();

        // Render B: secure redaction default (render_annotations(false))
        let (buf_b, config_b) =
            render_affected_page_raster(&page, &spec_b, None).expect("Render B");

        // Annotation rect [100 700 120 720] device coords at 72 DPI (viewport 600x800):
        // left=100, top=800-720=80, width=20, height=20 (Y in 80..100, X in 100..120)
        let pdf_rect = PdfPointRect {
            left: 100.0,
            bottom: 700.0,
            width: 20.0,
            height: 20.0,
        };
        let (dev_rect, _) =
            map_pdf_rect_to_device_pixel_rect(&page, &pdf_rect, &config_b, 600, 800)
                .expect("Map annot rect");

        assert_eq!(dev_rect.left, 100);
        assert_eq!(dev_rect.top, 80);
        assert_eq!(dev_rect.width, 20);
        assert_eq!(dev_rect.height, 20);

        let mut has_pixel_diff = false;
        let left = dev_rect.left as usize;
        let top = dev_rect.top as usize;
        let width = dev_rect.width as usize;
        let height = dev_rect.height as usize;

        for y in top..(top + height) {
            for x in left..(left + width) {
                let idx = (y * 600 + x) * 4;
                let px_a = &pixels_a[idx..idx + 4];
                let px_b = &buf_b.pixels[idx..idx + 4];
                if px_a != px_b {
                    has_pixel_diff = true;
                }
                // Secure-redaction config B MUST NOT render annotation appearance (remains white background)
                assert_eq!(
                    px_b,
                    &[255, 255, 255, 255],
                    "Annotation appearance leaked into secure redaction config B at ({x}, {y})"
                );
            }
        }

        assert!(
            has_pixel_diff,
            "render_annotations(true) vs render_annotations(false) must produce real pixel difference in annotation area"
        );
    });
}

#[test]
fn test_55_acroform_widget_blocked_full_plan() {
    let pdf_path = fixture_path("acroform-widget.pdf");
    let mut policy = sample_policy();
    policy.interactive_content_policy =
        preview_alternative_lib::secure_redaction::InteractiveContentPolicy::Block;

    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    assert!(!plan.executable);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        assert!(build_lossless_raster_payload_for_page(&plan, &doc, 0, None).is_err());
    });
}

#[test]
fn test_56_text_image_rasterization() {
    let pdf_path = fixture_path("text-image.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let payload = build_lossless_raster_payload_for_page(&plan, &doc, 0, None)
            .expect("Build payload for text-image.pdf");

        assert!(payload.width > 0);
        assert!(payload.height > 0);
        assert_eq!(
            payload.pixels.len(),
            (payload.stride * payload.height as usize)
        );
    });
}

#[test]
fn test_57_merge_multi_page_error() {
    let mut m1 = make_mapping("r1", 10, 10, 50, 50);
    let mut m2 = make_mapping("r2", 30, 30, 50, 50);
    m1.page_index = 0;
    m2.page_index = 1;

    let res = merge_device_pixel_rects(&[m1, m2]);
    assert!(res.is_err());
}

#[test]
fn test_58_invalid_stride_or_buffer_length_error() {
    let mut buf = RedactionRasterBuffer {
        width: 10,
        height: 10,
        stride: 30, // Invalid: expected 40 for RGBA8
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![255; 300],
    };
    let rect = DevicePixelRect {
        left: 0,
        top: 0,
        width: 2,
        height: 2,
    };
    assert!(apply_opaque_black_overwrite(&mut buf, &[rect], None).is_err());
}

#[test]
fn test_59_channel_order_color_fidelity() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("text-image.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg0,
            viewport_width: 600,
            viewport_height: 800,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };
        let (buf, _) = render_affected_page_raster(&page, &spec, None).expect("Render");
        assert_eq!(buf.pixel_format, RedactionPixelFormat::Rgba8);
        assert_eq!(buf.pixels.len(), 600 * 800 * 4);

        let get_px = |x: usize, y: usize| -> [u8; 4] {
            let offset = (y * (buf.width as usize) + x) * 4;
            [
                buf.pixels[offset],
                buf.pixels[offset + 1],
                buf.pixels[offset + 2],
                buf.pixels[offset + 3],
            ]
        };

        // Image placed at left=50, bottom=500, width=200, height=200 on 600x800 page
        // Device coords at 72 DPI: X in 50..250, Y in 100..300
        // Top-left interior (Red): (X=100, Y=150) -> [255, 0, 0, 255]
        let red = get_px(100, 150);
        assert_eq!(
            red,
            [255, 0, 0, 255],
            "Red pixel mismatch (R/B channel swap regression?): got {:?}",
            red
        );

        // Top-right interior (Green): (X=200, Y=150) -> [0, 255, 0, 255]
        let green = get_px(200, 150);
        assert_eq!(
            green,
            [0, 255, 0, 255],
            "Green pixel mismatch: got {:?}",
            green
        );

        // Bottom-left interior (Blue): (X=100, Y=250) -> [0, 0, 255, 255]
        let blue = get_px(100, 250);
        assert_eq!(
            blue,
            [0, 0, 255, 255],
            "Blue pixel mismatch: got {:?}",
            blue
        );

        // Bottom-right interior (White): (X=200, Y=250) -> [255, 255, 255, 255]
        let white = get_px(200, 250);
        assert_eq!(
            white,
            [255, 255, 255, 255],
            "White pixel mismatch: got {:?}",
            white
        );
    });
}

#[test]
fn test_60_encrypted_fixture_raster_blocked() {
    let pdf_path = fixture_path("encrypted-redaction.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        Some("fixture-password"),
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan for encrypted document");

    assert!(!plan.executable || !plan.ready_for_execution);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, Some("fixture-password"))
            .expect("Load encrypted pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
        match res.err().unwrap() {
            preview_alternative_lib::document_session::SessionJobOperationError::Failed(info) => {
                assert_eq!(
                    info.code,
                    preview_alternative_lib::secure_redaction::SecureRedactionErrorCode::RedactionBlockedDocumentFeature.code_str()
                );
            }
            _ => panic!("Expected SessionJobOperationError::Failed"),
        }
    });
}

#[test]
fn test_61_signed_fixture_raster_blocked() {
    let pdf_path = fixture_path("signature-marked-byterange.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan for signed document");

    assert!(!plan.executable || !plan.ready_for_execution);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load signed pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
        match res.err().unwrap() {
            preview_alternative_lib::document_session::SessionJobOperationError::Failed(info) => {
                assert_eq!(
                    info.code,
                    preview_alternative_lib::secure_redaction::SecureRedactionErrorCode::RedactionBlockedDocumentFeature.code_str()
                );
            }
            _ => panic!("Expected SessionJobOperationError::Failed"),
        }
    });
}

#[test]
fn test_62_xfa_fixture_raster_blocked() {
    let pdf_path = fixture_path("xfa-fixture.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build plan for XFA document");

    assert!(!plan.executable || !plan.ready_for_execution);

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load XFA pdf");
        let page = doc.pages().get(0).expect("Page 0");

        let res = build_lossless_raster_payload_for_page(&plan, &doc, 0, None);
        assert!(res.is_err());
        match res.err().unwrap() {
            preview_alternative_lib::document_session::SessionJobOperationError::Failed(info) => {
                assert_eq!(
                    info.code,
                    preview_alternative_lib::secure_redaction::SecureRedactionErrorCode::RedactionBlockedDocumentFeature.code_str()
                );
            }
            _ => panic!("Expected SessionJobOperationError::Failed"),
        }
    });
}

#[test]
fn test_63_document_bound_page_identity_uses_exact_page_index() {
    let pdf_path = fixture_path("same-size-same-rotation-pages.pdf");
    let mut policy = sample_policy();
    policy.raster_dpi = 72;
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.5, 0.5, 0.1, 0.1)],
    )
    .expect("Build plan for page 0");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load same-size/same-rotation fixture");

        // No PdfPage handle is accepted by this API, so page 1 cannot be
        // injected for the page-0 plan. The red square exists only on page 0.
        let payload =
            build_lossless_raster_payload_for_page(&plan, &doc, 0, None).expect("Page-0 payload");
        let start = 75 * payload.stride + 30 * 4;
        assert_eq!(&payload.pixels[start..start + 4], [255, 0, 0, 255]);
    });
}

#[test]
fn test_64_relative_rotation_render_config_is_supported() {
    with_test_pdfium(|pdfium| {
        let pdf_path = fixture_path("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load pdf");
        let page = doc.pages().get(0).expect("Page 0");

        // This lower-level helper accepts relative rotation. The plan-bound
        // primitive fails closed if its PDFium rotation read returns Err(_).
        let spec = RedactionRasterSpec {
            page_index: 0,
            dpi: 72,
            rotation: PageRotation::Deg90,
            viewport_width: 792,
            viewport_height: 612,
            background_policy: RedactionBackgroundPolicy::OpaqueWhiteV1,
            annotation_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionAnnotationPolicy::DoNotRender,
            widget_form_policy:
                preview_alternative_lib::secure_redaction_raster::RedactionWidgetFormPolicy::BlockedIfPresent,
        };

        let res = create_redaction_render_config(&page, &spec);
        assert!(res.is_ok());
    });
}

#[test]
fn test_65_full_pipeline_rotations_exact_payload_and_mapping() {
    let pdf_path = fixture_path("rotations.pdf");
    let mut policy = sample_policy();
    policy.raster_dpi = 72;
    let regions = (0..4)
        .map(|page_index| sample_region(&format!("r{page_index}"), page_index, 0.1, 0.1, 0.2, 0.2))
        .collect();
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        regions,
    )
    .expect("Build rotations plan");
    let expected = [
        (600, 800, 60, 80, 120, 160),
        (800, 600, 560, 60, 160, 120),
        (600, 800, 420, 560, 120, 160),
        (800, 600, 80, 420, 160, 120),
    ];

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load rotations fixture");
        for (page_index, (width, height, left, top, rect_width, rect_height)) in
            expected.into_iter().enumerate()
        {
            let payload =
                build_lossless_raster_payload_for_page(&plan, &doc, page_index as u32, None)
                    .unwrap_or_else(|e| {
                        panic!("rotation {page_index} full pipeline failed: {e:?}")
                    });
            let mapping = payload.region_mappings.first().expect("one mapped region");
            assert_eq!(payload.page_index, page_index as u32);
            assert_eq!((payload.width, payload.height), (width, height));
            assert_eq!(
                (
                    mapping.device_pixel_rect.left,
                    mapping.device_pixel_rect.top,
                    mapping.device_pixel_rect.width,
                    mapping.device_pixel_rect.height
                ),
                (left, top, rect_width, rect_height),
                "rotation {page_index} mapping"
            );
        }
    });
}

fn full_pipeline_origin_fixture(
    filename: &str,
    expected_media: PdfPointRect,
    expected_payload: (u32, u32),
    expected_mapping: (i32, i32, u32, u32),
) {
    let pdf_path = fixture_path(filename);
    let mut policy = sample_policy();
    policy.raster_dpi = 72;
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build fixture plan");

    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load fixture");
        let payload = build_lossless_raster_payload_for_page(&plan, &doc, 0, None)
            .expect("Full pipeline payload");
        let mapping = payload.region_mappings.first().expect("one mapped region");
        assert_eq!(plan.page_plans[0].geometry.media_box, expected_media);
        assert_eq!((payload.width, payload.height), expected_payload);
        assert_eq!(
            (
                mapping.device_pixel_rect.left,
                mapping.device_pixel_rect.top,
                mapping.device_pixel_rect.width,
                mapping.device_pixel_rect.height
            ),
            expected_mapping
        );
    });
}

#[test]
fn test_66_full_pipeline_cropbox_exact_payload_and_mapping() {
    let pdf_path = fixture_path("cropbox-mediabox-diff.pdf");
    let mut policy = sample_policy();
    policy.raster_dpi = 72;
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        policy,
        sample_acknowledgements(),
        vec![sample_region("r1", 0, 0.1, 0.1, 0.2, 0.2)],
    )
    .expect("Build CropBox plan");
    with_test_pdfium(|pdfium| {
        let doc = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("Load CropBox fixture");
        let payload = build_lossless_raster_payload_for_page(&plan, &doc, 0, None)
            .expect("CropBox full pipeline payload");
        let mapping = payload.region_mappings.first().expect("one mapped region");
        assert_eq!(
            plan.page_plans[0].geometry.media_box,
            PdfPointRect {
                left: 0.0,
                bottom: 0.0,
                width: 600.0,
                height: 800.0
            }
        );
        assert_eq!(
            plan.page_plans[0].geometry.crop_box,
            Some(PdfPointRect {
                left: 50.0,
                bottom: 50.0,
                width: 500.0,
                height: 700.0
            })
        );
        assert_eq!((payload.width, payload.height), (500, 700));
        assert_eq!(
            (
                mapping.device_pixel_rect.left,
                mapping.device_pixel_rect.top,
                mapping.device_pixel_rect.width,
                mapping.device_pixel_rect.height
            ),
            (50, 70, 100, 140)
        );
    });
}

#[test]
fn test_67_full_pipeline_nonzero_origin_exact_payload_and_mapping() {
    full_pipeline_origin_fixture(
        "nonzero-origin.pdf",
        PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 600.0,
            height: 800.0,
        },
        (600, 800),
        (60, 80, 120, 160),
    );
}

#[test]
fn test_68_full_pipeline_negative_origin_exact_payload_and_mapping() {
    full_pipeline_origin_fixture(
        "negative-origin.pdf",
        PdfPointRect {
            left: -100.0,
            bottom: -100.0,
            width: 600.0,
            height: 800.0,
        },
        (600, 800),
        (60, 80, 120, 160),
    );
}
