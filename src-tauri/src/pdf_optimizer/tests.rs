use super::walk::{dict_of, int, name};
use super::*;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::path::PathBuf;

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn image_settings(downsample: bool, compression: ImageCompression) -> ImageSettings {
    ImageSettings {
        downsample,
        target_ppi: 150.0,
        above_ppi: 225.0,
        compression,
    }
}

fn settings() -> OptimizeSettings {
    let images = image_settings(true, ImageCompression::Jpeg { quality: 60 });
    OptimizeSettings {
        color: images,
        gray: images,
        remove_unused_xobjects: true,
        discard_thumbnails: true,
        discard_alternate_images: true,
        flate_unencoded: true,
        object_streams: true,
    }
}

/// Deterministic noise: incompressible for Flate, so a resampled JPEG is clearly smaller.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as u8
        })
        .collect()
}

fn rgb_image(w: i64, h: i64, seed: u32) -> Stream {
    let samples = noise((w * h * 3) as usize, seed);
    Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w,
            "Height" => h,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        miniz_oxide::deflate::compress_to_vec_zlib(&samples, 6),
    )
}

/// A one-page document whose page draws each `(name, image, side)` as a `side`-point square,
/// with `extra` names listed in the resources but not drawn.
struct Builder {
    document: Document,
    pages_id: ObjectId,
    xobjects: Dictionary,
    content: String,
}

impl Builder {
    fn new() -> Self {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        Builder {
            document,
            pages_id,
            xobjects: Dictionary::new(),
            content: String::new(),
        }
    }

    fn add(&mut self, object: impl Into<Object>) -> ObjectId {
        self.document.add_object(object)
    }

    fn draw(&mut self, key: &str, id: ObjectId, side: f64) {
        self.xobjects.set(key, id);
        self.content
            .push_str(&format!("q {side} 0 0 {side} 10 10 cm /{key} Do Q\n"));
    }

    fn list(&mut self, key: &str, id: ObjectId) {
        self.xobjects.set(key, id);
    }

    fn finish(mut self, page_extra: Dictionary) -> (Document, ObjectId) {
        let content = std::mem::take(&mut self.content).into_bytes();
        let content = self.add(Stream::new(Dictionary::new(), content));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content,
            "Resources" => dictionary! { "XObject" => self.xobjects },
        };
        page.extend(&page_extra);
        let page_id = self.document.add_object(page);
        self.document.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog = self.document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => self.pages_id,
        });
        self.document.trailer.set("Root", catalog);
        (self.document, page_id)
    }
}

fn size(document: &Document, id: ObjectId) -> (i64, i64, Vec<u8>) {
    let reference = Object::Reference(id);
    let dict = dict_of(document, &reference).unwrap();
    (
        int(dict, b"Width").unwrap(),
        int(dict, b"Height").unwrap(),
        name(dict, b"Filter").unwrap_or_default().to_vec(),
    )
}

fn page_xobjects(document: &Document, page: ObjectId) -> Vec<Vec<u8>> {
    let page = document.get_dictionary(page).unwrap();
    let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
    let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
    xobjects.iter().map(|(key, _)| key.clone()).collect()
}

#[test]
fn effective_resolution_follows_the_matrices_and_forms() {
    let mut builder = Builder::new();
    let image = builder.add(rgb_image(600, 300, 1));
    // 600 px over 288 pt = 150 ppi across, 300 px over 288 pt = 75 ppi down: 75.
    builder.draw("Im1", image, 288.0);
    // The form doubles the scale: its use is at 37.5 ppi, the smaller one.
    let form = builder.add(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Matrix" => vec![2.into(), 0.into(), 0.into(), 2.into(), 0.into(), 0.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im" => image } },
        },
        b"/Im Do".to_vec(),
    ));
    builder.draw("Fm1", form, 288.0);
    let (document, page) = builder.finish(Dictionary::new());
    let resolutions = walk::effective_resolutions(&document, &[page]);
    let ppi = resolutions.ppi[&image];
    assert!((ppi - 37.5).abs() < 1e-6, "{ppi}");
}

#[test]
fn unreadable_content_keeps_every_image_at_its_resolution() {
    let mut builder = Builder::new();
    let image = builder.add(rgb_image(1200, 1200, 2));
    builder.draw("Im1", image, 288.0);
    builder
        .content
        .push_str("q 1 0 0 1 0 0 cm /Im1 Do Q (unterminated");
    // The stream can't be read to its end, so the image's largest use is unknown.
    let (mut document, page) = builder.finish(Dictionary::new());
    let resolutions = walk::effective_resolutions(&document, &[page]);
    assert!(resolutions
        .ppi
        .get(&image)
        .is_none_or(|ppi| ppi.is_infinite()));
    optimize(&mut document, &settings());
    assert_eq!(size(&document, image).0, 1200);
}

#[test]
fn images_above_the_threshold_are_resampled_to_the_target() {
    let mut builder = Builder::new();
    // 1200 px over 288 pt (4 in): 300 ppi, above 225 → 150 ppi = 600 px.
    let large = builder.add(rgb_image(1200, 1200, 3));
    builder.draw("Im1", large, 288.0);
    // 100 px over 288 pt: 25 ppi, below the threshold; small enough to stay as it is.
    let small = builder.add(rgb_image(40, 40, 4));
    builder.draw("Im2", small, 288.0);
    let (mut document, _) = builder.finish(Dictionary::new());
    let report = optimize(&mut document, &settings());
    assert_eq!(size(&document, large), (600, 600, b"DCTDecode".to_vec()));
    assert_eq!(size(&document, small), (40, 40, b"FlateDecode".to_vec()));
    assert_eq!(report.images, 2);
    assert_eq!(report.images_resampled, 1);
    assert!(report.image_bytes_after < report.image_bytes_before);
    // The JPEG decodes at its new size.
    let Object::Stream(stream) = &document.objects[&large] else {
        panic!()
    };
    let decoded = image::load_from_memory(&stream.content).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (600, 600));
}

#[test]
fn flate_keeps_resampled_images_lossless() {
    let mut builder = Builder::new();
    let large = builder.add(rgb_image(1200, 1200, 5));
    builder.draw("Im1", large, 288.0);
    let (mut document, _) = builder.finish(Dictionary::new());
    let mut settings = settings();
    settings.color = image_settings(true, ImageCompression::Flate);
    optimize(&mut document, &settings);
    assert_eq!(size(&document, large), (600, 600, b"FlateDecode".to_vec()));
}

#[test]
fn an_image_drawn_beyond_the_pages_is_not_resampled() {
    let mut builder = Builder::new();
    let large = builder.add(rgb_image(1200, 1200, 6));
    builder.draw("Im1", large, 288.0);
    // An annotation appearance draws the same image too, where the walk doesn't look.
    let appearance = builder.add(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 600.into(), 600.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im" => large } },
        },
        b"q 600 0 0 600 0 0 cm /Im Do Q".to_vec(),
    ));
    let annotation = builder.add(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Stamp",
        "Rect" => vec![0.into(), 0.into(), 600.into(), 600.into()],
        "AP" => dictionary! { "N" => appearance },
    });
    let (mut document, _) = builder.finish(dictionary! { "Annots" => vec![annotation.into()] });
    let report = optimize(&mut document, &settings());
    // Recompressed at its own size, never resampled.
    assert_eq!(size(&document, large), (1200, 1200, b"DCTDecode".to_vec()));
    assert_eq!(report.images_resampled, 0);
    assert_eq!(report.images_recompressed, 1);
}

#[test]
fn a_soft_mask_is_resampled_with_its_image() {
    let mut builder = Builder::new();
    let mask = builder.add(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 1200,
            "Height" => 1200,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        noise(1200 * 1200, 7),
    ));
    let mut image = rgb_image(1200, 1200, 8);
    image.dict.set("SMask", mask);
    let image = builder.add(image);
    builder.draw("Im1", image, 288.0);
    let (mut document, _) = builder.finish(Dictionary::new());
    optimize(&mut document, &settings());
    assert_eq!(size(&document, image), (600, 600, b"DCTDecode".to_vec()));
    assert_eq!(size(&document, mask), (600, 600, b"FlateDecode".to_vec()));
}

#[test]
fn a_shared_soft_mask_keeps_its_images_at_their_size() {
    let mut builder = Builder::new();
    let mask = builder.add(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 1200,
            "Height" => 1200,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        noise(1200 * 1200, 9),
    ));
    let mut first = rgb_image(1200, 1200, 10);
    first.dict.set("SMask", mask);
    let first = builder.add(first);
    let mut second = rgb_image(1200, 1200, 11);
    second.dict.set("SMask", mask);
    let second = builder.add(second);
    builder.draw("Im1", first, 288.0);
    builder.draw("Im2", second, 288.0);
    let (mut document, _) = builder.finish(Dictionary::new());
    optimize(&mut document, &settings());
    assert_eq!(size(&document, first).0, 1200);
    assert_eq!(size(&document, mask).0, 1200);
}

#[test]
fn colour_spaces_and_masks_it_cannot_read_are_left_alone() {
    let mut builder = Builder::new();
    let mut cmyk = rgb_image(1200, 1200, 12);
    cmyk.dict.set("ColorSpace", "DeviceCMYK");
    let cmyk = builder.add(cmyk);
    let mut inverted = rgb_image(1200, 1200, 13);
    let decode: Vec<Object> = [1, 0, 1, 0, 1, 0].into_iter().map(Object::from).collect();
    inverted.dict.set("Decode", decode);
    let inverted = builder.add(inverted);
    let mut by_reference = rgb_image(1200, 1200, 14);
    let parms = builder.add(dictionary! { "Predictor" => 1 });
    by_reference.dict.set("DecodeParms", parms);
    let by_reference = builder.add(by_reference);
    builder.draw("Im1", cmyk, 288.0);
    builder.draw("Im2", inverted, 288.0);
    builder.draw("Im3", by_reference, 288.0);
    let (mut document, _) = builder.finish(Dictionary::new());
    let report = optimize(&mut document, &settings());
    for id in [cmyk, inverted, by_reference] {
        assert_eq!(size(&document, id), (1200, 1200, b"FlateDecode".to_vec()));
    }
    assert_eq!(report.images_resampled + report.images_recompressed, 0);
}

#[test]
fn ycbcr_jpegs_are_read_and_others_left_alone() {
    let samples = noise(64 * 64 * 3, 15);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode(&samples, 64, 64, image::ExtendedColorType::Rgb8)
        .unwrap();
    let header = images::jpeg_header(&jpeg).unwrap();
    let stream = |parms: Option<Dictionary>| {
        let mut dict = dictionary! { "Filter" => "DCTDecode" };
        if let Some(parms) = parms {
            dict.set("DecodeParms", parms);
        }
        Stream::new(dict, jpeg.clone())
    };
    assert!(images::jpeg_reads_alike(&stream(None), 3, &header));
    assert!(images::jpeg_reads_alike(
        &stream(Some(dictionary! { "ColorTransform" => 1 })),
        3,
        &header
    ));
    // Stored RGB per the filter, while the decoder would convert from YCbCr.
    assert!(!images::jpeg_reads_alike(
        &stream(Some(dictionary! { "ColorTransform" => 0 })),
        3,
        &header
    ));
    // The wrong number of components.
    assert!(!images::jpeg_reads_alike(&stream(None), 1, &header));
}

fn gray_jpeg_in_flate(w: u32, h: u32, seed: u32, parms: Option<Object>) -> Stream {
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode(
            &noise((w * h) as usize, seed),
            w,
            h,
            image::ExtendedColorType::L8,
        )
        .unwrap();
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(w),
        "Height" => i64::from(h),
        "ColorSpace" => "DeviceGray",
        "BitsPerComponent" => 8,
        "Filter" => vec![Object::from("FlateDecode"), Object::from("DCTDecode")],
    };
    if let Some(parms) = parms {
        dict.set("DecodeParms", parms);
    }
    Stream::new(dict, miniz_oxide::deflate::compress_to_vec_zlib(&jpeg, 6))
}

#[test]
fn a_jpeg_inside_flate_is_read_through_the_flate_layer() {
    let mut builder = Builder::new();
    // Scanners write `[/FlateDecode /DCTDecode]`; 1200 px over 288 pt is 300 ppi.
    let wrapped = builder.add(gray_jpeg_in_flate(1200, 1200, 30, None));
    builder.draw("Im1", wrapped, 288.0);
    // A single parameter dictionary for two filters is ambiguous: left alone.
    let ambiguous = builder.add(gray_jpeg_in_flate(
        1200,
        1200,
        31,
        Some(Object::Dictionary(dictionary! { "Predictor" => 1 })),
    ));
    builder.draw("Im2", ambiguous, 288.0);
    let (mut document, _) = builder.finish(Dictionary::new());
    let report = optimize(&mut document, &settings());
    assert_eq!(size(&document, wrapped), (600, 600, b"DCTDecode".to_vec()));
    assert_eq!(size(&document, ambiguous).0, 1200);
    assert_eq!(report.images_resampled, 1);
}

#[test]
fn listed_but_undrawn_xobjects_are_removed() {
    let mut builder = Builder::new();
    let drawn = builder.add(rgb_image(20, 20, 16));
    let unused = builder.add(rgb_image(20, 20, 17));
    builder.draw("Im1", drawn, 72.0);
    builder.list("Im2", unused);
    let (mut document, page) = builder.finish(Dictionary::new());
    let report = optimize(&mut document, &settings());
    assert_eq!(report.unused_xobjects, 1);
    assert_eq!(page_xobjects(&document, page), vec![b"Im1".to_vec()]);
    // The rewrite then drops the image itself.
    document.prune_objects();
    assert!(!document.objects.contains_key(&unused));
}

#[test]
fn a_name_written_in_any_stream_keeps_its_xobject() {
    let mut builder = Builder::new();
    let drawn = builder.add(rgb_image(20, 20, 18));
    let unused = builder.add(rgb_image(20, 20, 19));
    builder.draw("Im1", drawn, 72.0);
    builder.list("Im2", unused);
    // A form somewhere else writes the name: it might draw it through these resources.
    let other = builder.add(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()] },
        b"/Im2 Do".to_vec(),
    ));
    let annotation = builder.add(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Stamp",
        "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        "AP" => dictionary! { "N" => other },
    });
    let (mut document, page) = builder.finish(dictionary! { "Annots" => vec![annotation.into()] });
    let report = optimize(&mut document, &settings());
    assert_eq!(report.unused_xobjects, 0);
    assert_eq!(page_xobjects(&document, page).len(), 2);
}

#[test]
fn shared_resources_are_not_cleaned() {
    let mut builder = Builder::new();
    let drawn = builder.add(rgb_image(20, 20, 20));
    let unused = builder.add(rgb_image(20, 20, 21));
    builder.draw("Im1", drawn, 72.0);
    builder.list("Im2", unused);
    let (mut document, page) = builder.finish(Dictionary::new());
    // Move the XObject dictionary into an object a form shares with the page.
    let xobjects = document
        .get_dictionary(page)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"XObject")
        .unwrap()
        .clone();
    let shared = document.add_object(xobjects);
    let form = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Resources" => dictionary! { "XObject" => shared },
        },
        Vec::new(),
    ));
    let Object::Dictionary(page_dict) = document.objects.get_mut(&page).unwrap() else {
        panic!()
    };
    page_dict.set(
        "Resources",
        dictionary! { "XObject" => shared, "Hidden" => form },
    );
    let report = optimize(&mut document, &settings());
    assert_eq!(report.unused_xobjects, 0);
}

#[test]
fn thumbnails_alternates_and_unencoded_streams_are_cleaned() {
    let mut builder = Builder::new();
    let thumbnail = builder.add(rgb_image(10, 10, 22));
    let alternate = builder.add(rgb_image(10, 10, 23));
    let mut image = rgb_image(20, 20, 24);
    image.dict.set(
        "Alternates",
        vec![Object::Dictionary(dictionary! { "Image" => alternate })],
    );
    let image = builder.add(image);
    builder.draw("Im1", image, 72.0);
    let metadata = builder.add(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"<x:xmpmeta xmlns:x='adobe:ns:meta/'>".repeat(20),
    ));
    let (mut document, page) = builder.finish(dictionary! { "Thumb" => thumbnail });
    let catalog = document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    if let Object::Dictionary(dict) = document.objects.get_mut(&catalog).unwrap() {
        dict.set("Metadata", metadata);
    }
    // The page content ("q 72 0 0 72 10 10 cm /Im1 Do Q") is stored without a filter.
    builder_content_long_enough(&mut document, page);
    let report = optimize(&mut document, &settings());
    assert_eq!(report.thumbnails, 1);
    assert_eq!(report.alternate_images, 1);
    assert!(report.streams_compressed >= 1);
    assert!(!document.get_dictionary(page).unwrap().has(b"Thumb"));
    let Object::Stream(metadata) = &document.objects[&metadata] else {
        panic!()
    };
    assert!(!metadata.dict.has(b"Filter"));
}

/// Pads the page content so compressing it pays.
fn builder_content_long_enough(document: &mut Document, page: ObjectId) {
    let contents = document.get_page_contents(page)[0];
    if let Object::Stream(stream) = document.objects.get_mut(&contents).unwrap() {
        let mut content = stream.content.clone();
        content.extend(b"q Q\n".repeat(64));
        stream.set_content(content);
    }
}

#[test]
fn settings_out_of_range_are_refused() {
    let mut bad = settings();
    bad.color.target_ppi = 5.0;
    assert!(validate_request(&bad).is_err());
    let mut bad = settings();
    bad.gray.above_ppi = 100.0;
    assert!(validate_request(&bad).is_err());
    let mut bad = settings();
    bad.color.compression = ImageCompression::Jpeg { quality: 0 };
    assert!(validate_request(&bad).is_err());
    assert!(validate_request(&settings()).is_ok());
}

#[test]
fn settings_wire_format_is_camel_case_with_tagged_compression() {
    let json = serde_json::json!({
        "color": { "downsample": true, "targetPpi": 150.0, "abovePpi": 225.0, "compression": { "kind": "jpeg", "quality": 60 } },
        "gray": { "downsample": false, "targetPpi": 150.0, "abovePpi": 225.0, "compression": { "kind": "flate" } },
        "removeUnusedXobjects": true,
        "discardThumbnails": true,
        "discardAlternateImages": false,
        "flateUnencoded": true,
        "objectStreams": true,
    });
    let parsed: OptimizeSettings = serde_json::from_value(json).unwrap();
    assert_eq!(
        parsed.color.compression,
        ImageCompression::Jpeg { quality: 60 }
    );
    assert_eq!(parsed.gray.compression, ImageCompression::Flate);
    let unknown = serde_json::json!({ "color": {}, "extra": 1 });
    assert!(serde_json::from_value::<OptimizeSettings>(unknown).is_err());
}

fn write_document(document: &mut Document, dir: &Path) -> PathBuf {
    let path = dir.join("input.pdf");
    document.save(&path).unwrap();
    path
}

#[test]
fn an_optimized_copy_is_smaller_and_keeps_its_pages() {
    let pdfium = crate::pdf_engine::get_pdfium().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut builder = Builder::new();
    let large = builder.add(rgb_image(1200, 1200, 25));
    builder.draw("Im1", large, 288.0);
    let unused = builder.add(rgb_image(300, 300, 26));
    builder.list("Im2", unused);
    let (mut document, _) = builder.finish(Dictionary::new());
    let input = write_document(&mut document, dir.path());
    let output = dir.path().join("optimized.pdf");
    let report =
        write_optimized_copy(pdfium.bindings(), &input, None, &settings(), &output).unwrap();
    assert_eq!(report.images_resampled, 1);
    assert_eq!(report.unused_xobjects, 1);
    assert!(!report.signatures_invalidated);
    assert!(report.output_bytes * 4 < report.input_bytes, "{report:?}");
    assert_eq!(report.input_bytes, std::fs::metadata(&input).unwrap().len());
    // No temporary plain copy is left beside the output.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    // The copy is a PDF 1.5+ file with object streams.
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.windows(b"/ObjStm".len()).any(|w| w == b"/ObjStm"));
}

#[test]
fn fixtures_survive_optimization_with_the_same_pages() {
    let pdfium = crate::pdf_engine::get_pdfium().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "pdf/minimal.pdf",
        "pdf/annotations.pdf",
        "pdf/reorder-rich.pdf",
        "pdf/reorder-with-form.pdf",
        "pdf/three-distinct-pages.pdf",
        "pdf/ai-text-rich.pdf",
    ] {
        let output = dir.path().join("out.pdf");
        write_optimized_copy(
            pdfium.bindings(),
            &fixture(name),
            None,
            &settings(),
            &output,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn password_protected_documents_are_refused() {
    let pdfium = crate::pdf_engine::get_pdfium().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.pdf");
    let error = write_optimized_copy(
        pdfium.bindings(),
        &fixture("pdf/encrypted.pdf"),
        Some("fixture-password"),
        &settings(),
        &output,
    )
    .unwrap_err();
    assert!(error.contains("Parola korumalı"), "{error}");
    assert!(!output.exists());
}
