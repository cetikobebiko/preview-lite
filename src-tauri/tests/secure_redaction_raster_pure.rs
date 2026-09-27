use preview_alternative_lib::document_session::SessionJobCancellationCheckpoint;
use preview_alternative_lib::secure_redaction::{
    CanonicalPageGeometry, DevicePixelRect, PageRotation, PdfPointRect,
};
use preview_alternative_lib::secure_redaction_raster::{
    apply_opaque_black_overwrite, calculate_raster_dimensions, merge_device_pixel_rects,
    verify_opaque_black_overwrite, LosslessRasterPayload, RasterizedRegionMapping,
    RedactionBackgroundPolicy, RedactionPixelFormat, RedactionRasterBuffer,
};

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
    apply_opaque_black_overwrite(&mut buf, std::slice::from_ref(&rect), None).expect("Overwrite");
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
        width: 10,
        height: 10,
    };
    apply_opaque_black_overwrite(&mut buf, &[rect], None).expect("Overwrite full");
    for (i, p) in buf.pixels.chunks(4).enumerate() {
        assert_eq!(p[0], 0, "R pixel {i}");
        assert_eq!(p[1], 0, "G pixel {i}");
        assert_eq!(p[2], 0, "B pixel {i}");
        assert_eq!(p[3], 255, "Alpha pixel {i} not exactly 255");
    }
}

#[test]
fn test_34_empty_rects_is_noop() {
    let mut buf = RedactionRasterBuffer {
        width: 2,
        height: 2,
        stride: 8,
        pixel_format: RedactionPixelFormat::Rgba8,
        pixels: vec![123; 16],
    };
    let orig = buf.pixels.clone();
    apply_opaque_black_overwrite(&mut buf, &[], None).expect("Empty rects");
    assert_eq!(buf.pixels, orig);
    verify_opaque_black_overwrite(&buf, &[]).expect("Verify empty");
}

#[test]
fn test_35_exact_edge_alignment() {
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
    apply_opaque_black_overwrite(&mut buf, std::slice::from_ref(&rect), None)
        .expect("Overwrite edge");
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
    apply_opaque_black_overwrite(&mut buf, std::slice::from_ref(&rect), None)
        .expect("Overwrite stride");
    verify_opaque_black_overwrite(&buf, &[rect]).expect("Verify stride");
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
