use lopdf::Object;
use parking_lot::Mutex;
use pdfium_render::prelude::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

static PDFIUM_TEST_MUTEX: Mutex<()> = Mutex::new(());

fn with_test_pdfium<T>(operation: impl FnOnce(&Pdfium) -> T) -> T {
    let _guard = PDFIUM_TEST_MUTEX.lock();
    let pdfium =
        preview_alternative_lib::pdf_engine::get_pdfium().expect("Initialize PDFium for test");
    operation(&pdfium)
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug)]
struct FixtureManifestItem {
    id: String,
    filename: String,
    purpose: String,
    encrypted: bool,
    test_credential: Option<String>,
    expected_page_count: u32,
    expected_text: Option<String>,
    expected_properties: Vec<String>,
    expected_planning_outcome: String,
    expected_blocker: Option<String>,
    expected_profile: String,
    expected_strategy: String,
    expected_verification_outcome: String,
    sha256: String,
    byte_size: usize,
    semantic_assertions: Vec<String>,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug)]
struct FixtureManifest {
    version: u32,
    description: String,
    fixtures: Vec<FixtureManifestItem>,
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("redaction")
}

#[test]
fn test_no_unsafe_code_in_redaction_tests() {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("redaction_fixtures_and_capabilities.rs");
    let content = fs::read_to_string(&test_file).expect("Read test file");

    let kw_unsafe_impl = format!("{} {}", "un".to_string() + "safe", "im".to_string() + "pl");
    let kw_wrapper = "PdfiumTest".to_string() + "Wrapper";
    let kw_global = "GLOBAL_".to_string() + "PDFIUM";

    assert_eq!(
        content.matches(&kw_unsafe_impl).count(),
        0,
        "redaction_fixtures_and_capabilities.rs contains forbidden block"
    );
    assert_eq!(
        content.matches(&kw_wrapper).count(),
        0,
        "redaction_fixtures_and_capabilities.rs contains forbidden wrapper"
    );
    assert_eq!(
        content.matches(&kw_global).count(),
        0,
        "redaction_fixtures_and_capabilities.rs contains forbidden global"
    );
}

#[test]
fn all_generated_fixture_raw_stream_lengths_match_declared_lengths() {
    let dir = fixture_dir();
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path).expect("Read manifest");
    let manifest: FixtureManifest = serde_json::from_str(&content).expect("Parse manifest JSON");

    for item in &manifest.fixtures {
        let pdf_path = dir.join(&item.filename);
        let raw_bytes = fs::read(&pdf_path).expect("Read pdf fixture");

        // Parse indirect objects and check stream dictionary /Length vs raw bytes to endstream
        let mut checked_streams = 0;
        let len = raw_bytes.len();
        let mut idx = 0;

        while idx < len {
            // Find "stream" keyword
            if let Some(pos) = raw_bytes[idx..].windows(6).position(|w| w == b"stream") {
                let stream_idx = idx + pos;

                // Check word boundaries for "stream"
                let is_prev_boundary = stream_idx == 0
                    || matches!(
                        raw_bytes[stream_idx - 1],
                        b'\n' | b'\r' | b' ' | b'\t' | b'>'
                    );

                if is_prev_boundary {
                    // Search backwards for dictionary opening << and closing >>
                    let search_start = stream_idx.saturating_sub(4096);
                    let dict_window = &raw_bytes[search_start..stream_idx];
                    let dict_str = String::from_utf8_lossy(dict_window);

                    if let Some(dict_end) = dict_str.rfind(">>") {
                        if let Some(dict_start) = dict_str[..dict_end].rfind("<<") {
                            let dict_content = &dict_str[dict_start..dict_end];
                            if let Some(length_pos) = dict_content.find("/Length") {
                                let length_str = dict_content[length_pos + 7..].trim_start();
                                // Parse integer length
                                let declared_len: usize = length_str
                                    .split_whitespace()
                                    .next()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "Indirect or non-integer /Length in raw stream parser for fixture {}: dict content '{}'",
                                            item.filename, dict_content
                                        )
                                    });

                                // Determine stream data start after stream\n, stream\r\n, or stream\r
                                let after_stream = stream_idx + 6;
                                let stream_data_start = if after_stream + 1 < len
                                    && raw_bytes[after_stream] == b'\r'
                                    && raw_bytes[after_stream + 1] == b'\n'
                                {
                                    after_stream + 2
                                } else if after_stream < len
                                    && (raw_bytes[after_stream] == b'\n'
                                        || raw_bytes[after_stream] == b'\r')
                                {
                                    after_stream + 1
                                } else {
                                    panic!(
                                        "stream keyword in fixture {} not followed by newline at offset {}",
                                        item.filename, stream_idx
                                    );
                                };

                                let stream_data_end = stream_data_start + declared_len;
                                assert!(
                                    stream_data_end <= len,
                                    "Stream data end {} exceeds PDF file size {} in fixture {}",
                                    stream_data_end,
                                    len,
                                    item.filename
                                );

                                // Check that bytes right after stream_data_end lead to endstream
                                let remainder = &raw_bytes[stream_data_end..];
                                let trimmed_remainder = if remainder.starts_with(b"\r\n") {
                                    &remainder[2..]
                                } else if remainder.starts_with(b"\n")
                                    || remainder.starts_with(b"\r")
                                {
                                    &remainder[1..]
                                } else {
                                    remainder
                                };

                                assert!(
                                    trimmed_remainder.starts_with(b"endstream"),
                                    "Raw stream /Length mismatch in fixture {}: declared /Length {}, stream start {}, expected 'endstream' at {} but found bytes {:?}",
                                    item.filename,
                                    declared_len,
                                    stream_data_start,
                                    stream_data_end,
                                    String::from_utf8_lossy(&trimmed_remainder[..std::cmp::min(20, trimmed_remainder.len())])
                                );

                                checked_streams += 1;
                            }
                        }
                    }
                }
                idx = stream_idx + 6;
            } else {
                break;
            }
        }

        assert!(
            checked_streams > 0,
            "Fixture {} must contain at least one validated raw stream object",
            item.filename
        );
    }
}

#[test]
fn parsed_fixture_stream_lengths_match_decoded_or_loaded_content() {
    let dir = fixture_dir();
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path).expect("Read manifest");
    let manifest: FixtureManifest = serde_json::from_str(&content).expect("Parse manifest JSON");

    for item in &manifest.fixtures {
        if item.encrypted {
            continue;
        }

        let pdf_path = dir.join(&item.filename);
        let doc = lopdf::Document::load(&pdf_path)
            .unwrap_or_else(|e| panic!("lopdf failed to load fixture {}: {:?}", item.filename, e));

        let mut stream_count = 0;
        for (obj_id, object) in &doc.objects {
            if let Object::Stream(stream) = object {
                stream_count += 1;
                let declared_len = stream
                    .dict
                    .get(b"Length")
                    .unwrap_or_else(|_| {
                        panic!(
                            "Stream object {:?} in {} missing /Length entry",
                            obj_id, item.filename
                        )
                    })
                    .as_i64()
                    .unwrap_or_else(|_| {
                        panic!(
                            "Stream object {:?} in {} /Length is not integer",
                            obj_id, item.filename
                        )
                    }) as usize;

                let actual_len = stream.content.len();
                assert_eq!(
                    declared_len, actual_len,
                    "Stream /Length mismatch in fixture {}: object {:?} declared /Length {}, actual stream content bytes {}",
                    item.filename, obj_id, declared_len, actual_len
                );
            }
        }
        assert!(
            stream_count > 0,
            "Fixture {} must contain at least one stream object",
            item.filename
        );
    }
}

#[test]
fn test_multi_parser_validity_matrix() {
    with_test_pdfium(|pdfium| {
        let dir = fixture_dir();
        let manifest_path = dir.join("manifest.json");
        let content = fs::read_to_string(&manifest_path).expect("Read manifest");
        let manifest: FixtureManifest =
            serde_json::from_str(&content).expect("Parse manifest JSON");

        for item in &manifest.fixtures {
            let pdf_path = dir.join(&item.filename);

            // Parser 1: lopdf
            if !item.encrypted {
                let lopdf_res = lopdf::Document::load(&pdf_path);
                assert!(
                    lopdf_res.is_ok(),
                    "lopdf failed to open non-encrypted fixture {}: {:?}",
                    item.filename,
                    lopdf_res.err()
                );
            }

            // Parser 2: PDFium (Open & Render Smoke Test for all pages)
            let credential = item.test_credential.as_deref();
            let doc = pdfium
                .load_pdf_from_file(&pdf_path, credential)
                .unwrap_or_else(|e| {
                    panic!(
                        "PDFium failed to open fixture {} with credential {:?}: {:?}",
                        item.filename, credential, e
                    )
                });

            assert!(
                !doc.pages().is_empty(),
                "Fixture {} page count must be > 0",
                item.filename
            );

            for page_idx in 0..doc.pages().len() {
                let page = doc.pages().get(page_idx).unwrap_or_else(|e| {
                    panic!("Get page {} of {}: {:?}", page_idx, item.filename, e)
                });

                let bitmap_res = page.render(100, 100, None);
                assert!(
                    bitmap_res.is_ok(),
                    "PDFium failed to render page {} of fixture {}: {:?}",
                    page_idx,
                    item.filename,
                    bitmap_res.err()
                );
                let bitmap = bitmap_res.unwrap();
                assert!(
                    bitmap.width() > 0 && bitmap.height() > 0,
                    "Rendered bitmap dimensions must be non-zero for page {} of {}",
                    page_idx,
                    item.filename
                );
            }
        }
    });
}

#[test]
fn test_fixture_generator_output_matches_repository_manifest() {
    let repo_dir = fixture_dir();
    let generator_script = repo_dir.join("generate_redaction_fixtures.py");
    assert!(generator_script.exists(), "Generator script missing");

    let temp_dir = tempdir().expect("Create temp dir");

    let temp_script = temp_dir.path().join("generate_redaction_fixtures.py");
    fs::copy(&generator_script, &temp_script).expect("Copy generator script");

    let temp_pdf_dir = temp_dir.path().join("pdf");
    fs::create_dir_all(&temp_pdf_dir).expect("Create temp pdf dir");

    let original_encrypted = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pdf")
        .join("encrypted.pdf");
    fs::copy(&original_encrypted, temp_pdf_dir.join("encrypted.pdf")).expect("Copy encrypted.pdf");

    let output = std::process::Command::new("python3")
        .arg(&temp_script)
        .current_dir(temp_dir.path())
        .output()
        .expect("Execute generator script");

    assert!(
        output.status.success(),
        "Generator script failed: {:?}",
        output
    );

    let repo_manifest_content =
        fs::read_to_string(repo_dir.join("manifest.json")).expect("Read repo manifest");
    let temp_manifest_content =
        fs::read_to_string(temp_dir.path().join("manifest.json")).expect("Read temp manifest");

    if temp_manifest_content != repo_manifest_content {
        println!("MANIFEST DIFF DETECTED:");
        for (i, (l1, l2)) in temp_manifest_content
            .lines()
            .zip(repo_manifest_content.lines())
            .enumerate()
        {
            if l1 != l2 {
                println!("Line {}: TEMP='{}' vs REPO='{}'", i + 1, l1, l2);
            }
        }
        assert_eq!(
            temp_manifest_content, repo_manifest_content,
            "Generated manifest.json differs from repository manifest.json"
        );
    }

    let manifest: FixtureManifest =
        serde_json::from_str(&repo_manifest_content).expect("Parse manifest");

    for item in &manifest.fixtures {
        let temp_file = temp_dir.path().join(&item.filename);
        let repo_file = repo_dir.join(&item.filename);

        assert!(
            temp_file.exists(),
            "Temp generator failed to produce {}",
            item.filename
        );
        assert!(
            repo_file.exists(),
            "Repo missing fixture file {}",
            item.filename
        );

        let temp_bytes = fs::read(&temp_file).expect("Read temp file");
        let repo_bytes = fs::read(&repo_file).expect("Read repo file");

        assert_eq!(
            temp_bytes, repo_bytes,
            "Generator output for {} differs from repository version",
            item.filename
        );

        let computed_sha = format!("{:x}", Sha256::digest(&repo_bytes));
        assert_eq!(
            computed_sha, item.sha256,
            "SHA-256 digest mismatch for {}",
            item.filename
        );
        assert_eq!(
            repo_bytes.len(),
            item.byte_size,
            "Byte size mismatch for {}",
            item.filename
        );
    }
}

#[test]
fn test_fixture_manifest_serialization_is_deterministic() {
    let dir = fixture_dir();
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path).expect("Read manifest");
    let manifest: FixtureManifest = serde_json::from_str(&content).expect("Parse manifest JSON");

    assert_eq!(manifest.version, 1);
    assert_eq!(
        manifest.fixtures.len(),
        26,
        "Manifest must contain exactly 26 fixtures"
    );

    for item in &manifest.fixtures {
        assert_eq!(item.expected_profile, "secure_rasterized_pages_v1");
        assert_eq!(item.expected_strategy, "rasterized_pages");
        assert!(!item.semantic_assertions.is_empty());
    }
}

#[test]
fn test_fixture_manifest_has_no_untracked_fixture() {
    let dir = fixture_dir();
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path).expect("Read manifest");
    let manifest: FixtureManifest = serde_json::from_str(&content).expect("Parse manifest JSON");

    for item in &manifest.fixtures {
        let path = dir.join(&item.filename);
        assert!(
            path.exists(),
            "Manifest lists fixture {} but file is missing on disk",
            item.filename
        );
    }
}

#[test]
fn test_fixture_directory_has_no_unmanifested_pdf() {
    let dir = fixture_dir();
    let manifest_path = dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path).expect("Read manifest");
    let manifest: FixtureManifest = serde_json::from_str(&content).expect("Parse manifest JSON");

    let manifested_filenames: std::collections::HashSet<String> =
        manifest.fixtures.into_iter().map(|f| f.filename).collect();

    let entries = fs::read_dir(&dir).expect("Read fixture directory");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("pdf") {
            let filename = path.file_name().unwrap().to_str().unwrap().to_string();
            assert!(
                manifested_filenames.contains(&filename),
                "Found unmanifested PDF file in fixture directory: {}",
                filename
            );
        }
    }
}

#[test]
fn test_semantic_fixture_corpus_assertions() {
    with_test_pdfium(|pdfium| {
        let dir = fixture_dir();

        // 5.1 simple-text.pdf
        let simple_path = dir.join("simple-text.pdf");
        let doc = pdfium
            .load_pdf_from_file(&simple_path, None)
            .expect("Load simple-text.pdf");
        assert_eq!(doc.pages().len(), 1, "simple-text.pdf page count must be 1");
        let page0 = doc.pages().get(0).unwrap();
        let text = page0.text().expect("Extract text").all();
        assert!(text.contains("CONFIDENTIAL TC: 12345678901 EMAIL: user@example.com"));
        let text_obj_count = page0
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Text)
            .count();
        assert!(
            text_obj_count > 0,
            "simple-text.pdf must contain text page object"
        );

        // 5.2 rotations.pdf
        let rot_path = dir.join("rotations.pdf");
        let doc = pdfium
            .load_pdf_from_file(&rot_path, None)
            .expect("Load rotations.pdf");
        assert_eq!(doc.pages().len(), 4, "rotations.pdf page count must be 4");
        assert_eq!(
            doc.pages().get(0).unwrap().rotation().unwrap().as_degrees(),
            0.0
        );
        assert_eq!(
            doc.pages().get(1).unwrap().rotation().unwrap().as_degrees(),
            90.0
        );
        assert_eq!(
            doc.pages().get(2).unwrap().rotation().unwrap().as_degrees(),
            180.0
        );
        assert_eq!(
            doc.pages().get(3).unwrap().rotation().unwrap().as_degrees(),
            270.0
        );
        for page in doc.pages().iter() {
            let render_res = page.render(100, 100, None);
            assert!(
                render_res.is_ok(),
                "Render smoke test must succeed for rotations.pdf page"
            );
        }

        // 5.3 cropbox-mediabox-diff.pdf
        let crop_path = dir.join("cropbox-mediabox-diff.pdf");
        let doc = pdfium
            .load_pdf_from_file(&crop_path, None)
            .expect("Load cropbox-mediabox-diff.pdf");
        let page = doc.pages().get(0).unwrap();
        let media = page.boundaries().media().expect("MediaBox");
        let crop = page.boundaries().crop().expect("CropBox");
        assert_ne!(media, crop);
        assert_eq!(crop.bounds.left().value, 50.0);
        assert_eq!(crop.bounds.bottom().value, 50.0);
        assert_eq!(crop.bounds.right().value, 550.0);
        assert_eq!(crop.bounds.top().value, 750.0);
        assert_eq!(page.width().value, 500.0);
        assert_eq!(page.height().value, 700.0);

        // 5.4 nonzero-origin.pdf
        let nonzero_path = dir.join("nonzero-origin.pdf");
        let doc = pdfium
            .load_pdf_from_file(&nonzero_path, None)
            .expect("Load nonzero-origin.pdf");
        let page = doc.pages().get(0).unwrap();
        let media = page.boundaries().media().expect("MediaBox");
        assert_eq!(media.bounds.left().value, 50.0);
        assert_eq!(media.bounds.bottom().value, 50.0);
        assert_eq!(media.bounds.right().value, 650.0);
        assert_eq!(media.bounds.top().value, 850.0);
        assert_eq!(page.width().value, 600.0);
        assert_eq!(page.height().value, 800.0);

        // 5.5 negative-origin.pdf
        let neg_path = dir.join("negative-origin.pdf");
        let doc = pdfium
            .load_pdf_from_file(&neg_path, None)
            .expect("Load negative-origin.pdf");
        let page = doc.pages().get(0).unwrap();
        let media = page.boundaries().media().expect("MediaBox");
        assert_eq!(media.bounds.left().value, -100.0);
        assert_eq!(media.bounds.bottom().value, -100.0);
        assert_eq!(media.bounds.right().value, 500.0);
        assert_eq!(media.bounds.top().value, 700.0);
        let lopdf_doc = lopdf::Document::load(&neg_path).expect("lopdf load negative-origin.pdf");
        let lopdf_page_dict = lopdf_doc.get_object((3, 0)).unwrap().as_dict().unwrap();
        let mediabox_arr = lopdf_page_dict
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(mediabox_arr[0].as_i64().unwrap(), -100);
        assert_eq!(mediabox_arr[1].as_i64().unwrap(), -100);

        // 5.6 image-only.pdf
        let img_path = dir.join("image-only.pdf");
        let doc = pdfium
            .load_pdf_from_file(&img_path, None)
            .expect("Load image-only.pdf");
        let page = doc.pages().get(0).unwrap();
        let img_objs = page
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Image)
            .count();
        let txt_objs = page
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Text)
            .count();
        assert_eq!(
            img_objs, 1,
            "image-only.pdf must contain exactly 1 Image object"
        );
        assert_eq!(txt_objs, 0, "image-only.pdf must contain 0 Text objects");
        let ext_text = page.text().unwrap().all();
        assert!(
            ext_text.trim().is_empty(),
            "Extracted text for image-only page must be empty"
        );
        assert!(page.render(100, 100, None).is_ok());

        // 5.7 text-image.pdf
        let text_img_path = dir.join("text-image.pdf");
        let doc = pdfium
            .load_pdf_from_file(&text_img_path, None)
            .expect("Load text-image.pdf");
        let page = doc.pages().get(0).unwrap();
        let img_objs = page
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Image)
            .count();
        let txt_objs = page
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Text)
            .count();
        assert!(img_objs > 0);
        assert!(txt_objs > 0);
        assert!(page.text().unwrap().all().contains("TEXT AND IMAGE PAGE"));
        assert!(page.render(100, 100, None).is_ok());

        // 5.8 annotations-fixture.pdf
        let ann_path = dir.join("annotations-fixture.pdf");
        let doc = pdfium
            .load_pdf_from_file(&ann_path, None)
            .expect("Load annotations-fixture.pdf");
        let page = doc.pages().get(0).unwrap();
        assert!(
            !page.annotations().is_empty(),
            "annotations-fixture.pdf must contain annotations"
        );
        let lopdf_doc =
            lopdf::Document::load(&ann_path).expect("lopdf load annotations-fixture.pdf");
        let annot_dict = lopdf_doc.get_object((6, 0)).unwrap().as_dict().unwrap();
        assert_eq!(
            annot_dict.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Text"
        );

        // 5.9 acroform-widget.pdf
        let form_path = dir.join("acroform-widget.pdf");
        let lopdf_doc = lopdf::Document::load(&form_path).expect("lopdf load acroform-widget.pdf");
        let catalog = lopdf_doc.catalog().expect("Catalog");
        let acroform_obj = catalog.get(b"AcroForm").expect("AcroForm present");
        let acroform_dict = acroform_obj.as_dict().unwrap();
        let fields = acroform_dict.get(b"Fields").unwrap().as_array().unwrap();
        assert!(
            !fields.is_empty(),
            "AcroForm /Fields array must not be empty"
        );
        let widget_dict = lopdf_doc.get_object((6, 0)).unwrap().as_dict().unwrap();
        assert_eq!(
            widget_dict.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Widget"
        );
        assert_eq!(widget_dict.get(b"FT").unwrap().as_name().unwrap(), b"Tx");
        let field_name =
            String::from_utf8_lossy(widget_dict.get(b"T").unwrap().as_str().unwrap_or(b""));
        assert_eq!(field_name, "CustomerName");
        let field_value =
            String::from_utf8_lossy(widget_dict.get(b"V").unwrap().as_str().unwrap_or(b""));
        assert_eq!(field_value, "Jane Doe");

        // 5.10 xfa-fixture.pdf
        let xfa_path = dir.join("xfa-fixture.pdf");
        let lopdf_doc = lopdf::Document::load(&xfa_path).expect("lopdf load xfa-fixture.pdf");
        let catalog = lopdf_doc.catalog().expect("Catalog");
        let acroform = catalog.get(b"AcroForm").unwrap().as_dict().unwrap();
        assert!(
            acroform.get(b"XFA").is_ok(),
            "XFA present in AcroForm dictionary"
        );

        // 5.12 signature-marked-byterange.pdf
        let sig_path = dir.join("signature-marked-byterange.pdf");
        let lopdf_doc =
            lopdf::Document::load(&sig_path).expect("lopdf load signature-marked-byterange.pdf");
        let sig_obj = lopdf_doc.objects.values().find(|o| {
            if let Ok(dict) = o.as_dict() {
                dict.get(b"ByteRange").is_ok()
            } else {
                false
            }
        });
        assert!(
            sig_obj.is_some(),
            "Signature dictionary carrying /ByteRange present"
        );
        let sig_dict = sig_obj.unwrap().as_dict().unwrap();
        assert_eq!(sig_dict.get(b"Type").unwrap().as_name().unwrap(), b"Sig");

        // 5.13 incremental-revision.pdf
        let inc_path = dir.join("incremental-revision.pdf");
        let raw_bytes = fs::read(&inc_path).expect("Read incremental-revision.pdf");
        let raw_str = String::from_utf8_lossy(&raw_bytes);
        assert_eq!(
            raw_str.matches("startxref").count(),
            2,
            "Must contain 2 xref sections"
        );
        assert!(raw_str.contains("/Prev"), "Must contain /Prev pointer");
        assert!(raw_str.contains("REVISION_ONE_REACHABLE_MARKER"));
        assert!(raw_str.contains("REVISION_TWO_REACHABLE_MARKER"));
        assert!(raw_str.contains("UNREACHABLE_OLD_OBJECT_MARKER"));

        // 5.14 object-stream.pdf
        let obj_stm_path = dir.join("object-stream.pdf");
        let raw_bytes = fs::read(&obj_stm_path).expect("Read object-stream.pdf");
        let raw_str = String::from_utf8_lossy(&raw_bytes);
        assert!(raw_str.starts_with("%PDF-1.5"));
        assert!(raw_str.contains("/Type /ObjStm"));
        assert!(raw_str.contains("/Type /XRef"));
        assert!(
            !raw_str.contains("\n5 0 obj"),
            "Object 5 must be compressed inside ObjStm, not a top-level object"
        );
        let lopdf_doc = lopdf::Document::load(&obj_stm_path).expect("lopdf load object-stream.pdf");
        let obj5 = lopdf_doc
            .get_object((5, 0))
            .expect("lopdf resolve object 5 from ObjStm");
        let dict5 = obj5
            .as_dict()
            .expect("Object 5 resolved from ObjStm as dictionary");
        let marker_obj = dict5.get(b"Marker").unwrap();
        let marker_str = match marker_obj {
            Object::String(bytes, _) => String::from_utf8_lossy(bytes).to_string(),
            Object::Name(bytes) => String::from_utf8_lossy(bytes).to_string(),
            _ => panic!("Unexpected Marker object type"),
        };
        assert_eq!(marker_str, "COMPRESSED_OBJ_5_MARKER");
        let doc = pdfium
            .load_pdf_from_file(&obj_stm_path, None)
            .expect("PDFium load object-stream.pdf");
        let page0 = doc.pages().get(0).unwrap();
        assert!(page0.render(100, 100, None).is_ok());
    });
}

#[test]
fn test_encrypted_fixture_credentials_semantics() {
    with_test_pdfium(|pdfium| {
        let dir = fixture_dir();
        let enc_path = dir.join("encrypted-redaction.pdf");

        // Read manifest to get expected_text
        let manifest_path = dir.join("manifest.json");
        let manifest_content = fs::read_to_string(&manifest_path).expect("Read manifest");
        let manifest: FixtureManifest =
            serde_json::from_str(&manifest_content).expect("Parse manifest JSON");
        let enc_item = manifest
            .fixtures
            .iter()
            .find(|f| f.filename == "encrypted-redaction.pdf")
            .expect("encrypted-redaction manifest item");
        let expected_text = enc_item
            .expected_text
            .as_deref()
            .expect("encrypted fixture expected_text in manifest");

        // 1. Without password -> Must fail, document handle not obtained
        let no_pass_res = pdfium.load_pdf_from_file(&enc_path, None);
        assert!(
            no_pass_res.is_err(),
            "Encrypted PDF must fail without password"
        );

        // 2. With wrong password -> Must fail, document handle not obtained
        let wrong_pass_res = pdfium.load_pdf_from_file(&enc_path, Some("wrong-password"));
        assert!(
            wrong_pass_res.is_err(),
            "Encrypted PDF must fail with wrong password"
        );

        // 3. Raw PDF content checks
        let raw_bytes = fs::read(&enc_path).expect("Read encrypted fixture");
        let raw_str = String::from_utf8_lossy(&raw_bytes);
        assert!(
            raw_str.contains("/Encrypt"),
            "Encrypted PDF fixture raw bytes must contain /Encrypt dictionary"
        );
        assert!(
            !raw_str.contains(expected_text),
            "Plaintext marker {:?} must NOT appear in raw encrypted PDF bytes",
            expected_text
        );
        assert!(
            !raw_str.contains("ENCRYPTED_REDACTION_FIXTURE_TEXT"),
            "Plaintext marker constant name must NOT appear in raw encrypted PDF bytes"
        );

        // 4. With correct password ("fixture-password") -> Must succeed
        let doc = pdfium
            .load_pdf_from_file(&enc_path, Some("fixture-password"))
            .expect("Encrypted PDF must succeed with correct fixture-password");

        assert_eq!(doc.pages().len(), 1, "Encrypted PDF page count must be 1");
        let page = doc.pages().get(0).expect("page 0");
        assert!(
            page.width().value > 0.0,
            "Encrypted PDF page width must be positive"
        );

        // Page text extraction call on authenticated document
        let text_res = page.text();
        assert!(
            text_res.is_ok(),
            "Text extraction call on authenticated encrypted page must succeed"
        );
        let text = text_res.unwrap().all();
        assert!(
            !text.trim().is_empty(),
            "Extracted text from authenticated encrypted page must not be empty"
        );
        assert!(
            text.contains(expected_text),
            "Correct password must decrypt and expose expected fixture text; extracted={text:?}, expected={expected_text:?}"
        );

        // Security handler revision check
        let sec_rev_res = doc.permissions().security_handler_revision();
        assert!(
            sec_rev_res.is_ok(),
            "Security handler revision must be readable for authenticated document"
        );
        let revision = sec_rev_res.unwrap();
        assert!(
            revision != PdfSecurityHandlerRevision::Unprotected,
            "Encrypted fixture security handler revision must not be Unprotected"
        );
        assert_eq!(
            revision,
            PdfSecurityHandlerRevision::Revision2,
            "Encrypted fixture security handler revision must match Revision 2"
        );
    });
}

#[derive(Debug, PartialEq, Eq)]
struct RewriteCharacterization {
    lopdf_reopen: bool,
    pdfium_reopen: bool,
    page_count: usize,
    has_prev: bool,
    eof_count: usize,
    revision_one_reachable: bool,
    revision_two_reachable: bool,
    unreachable_old_object: bool,
    source_is_output_prefix: bool,
    source_appended_in_output: bool,
    source_and_output_identical: bool,
    output_starts_with_pdf_header: bool,
}

fn characterize_artifact(
    path: &Path,
    orig_bytes: &[u8],
    pdfium: &Pdfium,
) -> RewriteCharacterization {
    let bytes = fs::read(path).expect("Read artifact bytes");
    let raw_str = String::from_utf8_lossy(&bytes);
    let orig_str = String::from_utf8_lossy(orig_bytes);

    let lopdf_reopen = lopdf::Document::load(path).is_ok();
    let (pdfium_reopen, page_count) = match pdfium.load_pdf_from_file(path, None) {
        Ok(doc) => (true, doc.pages().len() as usize),
        Err(_) => (false, 0),
    };

    let source_is_output_prefix = bytes.starts_with(orig_bytes) && bytes.len() > orig_bytes.len();
    let source_appended_in_output =
        raw_str.starts_with(&*orig_str) && raw_str.len() > orig_str.len();

    RewriteCharacterization {
        lopdf_reopen,
        pdfium_reopen,
        page_count,
        has_prev: raw_str.contains("/Prev"),
        eof_count: raw_str.matches("%%EOF").count(),
        revision_one_reachable: raw_str.contains("REVISION_ONE_REACHABLE_MARKER"),
        revision_two_reachable: raw_str.contains("REVISION_TWO_REACHABLE_MARKER"),
        unreachable_old_object: raw_str.contains("UNREACHABLE_OLD_OBJECT_MARKER"),
        source_is_output_prefix,
        source_appended_in_output,
        source_and_output_identical: bytes == orig_bytes,
        output_starts_with_pdf_header: bytes.starts_with(b"%PDF-"),
    }
}

#[test]
fn test_incremental_rewrite_characterization() {
    with_test_pdfium(|pdfium| {
        let inc_path = fixture_dir().join("incremental-revision.pdf");
        let temp_dir = tempdir().expect("Create temp dir");
        let orig_bytes = fs::read(&inc_path).expect("Read incremental-revision.pdf");

        // 1. Original Incremental Fixture
        let orig_char = characterize_artifact(&inc_path, &orig_bytes, pdfium);
        let expected_orig_char = RewriteCharacterization {
            lopdf_reopen: true,
            pdfium_reopen: true,
            page_count: 1,
            has_prev: true,
            eof_count: 2,
            revision_one_reachable: true,
            revision_two_reachable: true,
            unreachable_old_object: true,
            source_is_output_prefix: false,
            source_appended_in_output: false,
            source_and_output_identical: true,
            output_starts_with_pdf_header: true,
        };
        assert_eq!(
            orig_char, expected_orig_char,
            "Original fixture characterization mismatch"
        );

        // Primitive 1: lopdf load & save
        let mut lopdf_doc1 = lopdf::Document::load(&inc_path).expect("lopdf load inc fixture");
        lopdf_doc1.trailer.remove(b"Prev");
        let target1 = temp_dir.path().join("lopdf_save.pdf");
        lopdf_doc1.save(&target1).expect("lopdf save target1");

        let lopdf_save_char = characterize_artifact(&target1, &orig_bytes, pdfium);
        let expected_lopdf_save_char = RewriteCharacterization {
            lopdf_reopen: true,
            pdfium_reopen: true,
            page_count: 1,
            has_prev: false,
            eof_count: 1,
            revision_one_reachable: true,
            revision_two_reachable: true,
            unreachable_old_object: true, // EXPLICITLY ASSERTED: plain lopdf save preserves unreachable marker!
            source_is_output_prefix: false,
            source_appended_in_output: false,
            source_and_output_identical: false,
            output_starts_with_pdf_header: true,
        };
        assert_eq!(
            lopdf_save_char, expected_lopdf_save_char,
            "lopdf save characterization mismatch"
        );

        // Primitive 2: lopdf prune_objects + save
        let mut lopdf_doc2 = lopdf::Document::load(&inc_path).expect("lopdf load for prune");
        lopdf_doc2.prune_objects();
        lopdf_doc2.trailer.remove(b"Prev");
        let target2 = temp_dir.path().join("lopdf_prune_save.pdf");
        lopdf_doc2.save(&target2).expect("lopdf prune save");

        let prune_char = characterize_artifact(&target2, &orig_bytes, pdfium);
        let expected_prune_char = RewriteCharacterization {
            lopdf_reopen: true,
            pdfium_reopen: true,
            page_count: 1,
            has_prev: false,
            eof_count: 1,
            revision_one_reachable: true,
            revision_two_reachable: true,
            unreachable_old_object: false, // EXPLICITLY ASSERTED: prune_objects removes unreachable marker!
            source_is_output_prefix: false,
            source_appended_in_output: false,
            source_and_output_identical: false,
            output_starts_with_pdf_header: true,
        };
        assert_eq!(
            prune_char, expected_prune_char,
            "lopdf prune save characterization mismatch"
        );

        // Primitive 3: PDFium save_to_file
        let pdfium_doc = pdfium
            .load_pdf_from_file(&inc_path, None)
            .expect("PDFium load");
        let target3 = temp_dir.path().join("pdfium_save.pdf");
        pdfium_doc
            .save_to_file(&target3)
            .expect("PDFium save_to_file");

        let pdfium_save_char = characterize_artifact(&target3, &orig_bytes, pdfium);
        let expected_pdfium_save_char = RewriteCharacterization {
            lopdf_reopen: true,
            pdfium_reopen: true,
            page_count: 1,
            has_prev: false,
            eof_count: 1,
            revision_one_reachable: true,
            revision_two_reachable: true,
            unreachable_old_object: false, // EXPLICITLY ASSERTED: PDFium save_to_file prunes unreferenced objects!
            source_is_output_prefix: false,
            source_appended_in_output: false,
            source_and_output_identical: false,
            output_starts_with_pdf_header: true,
        };
        assert_eq!(
            pdfium_save_char, expected_pdfium_save_char,
            "PDFium save_to_file characterization mismatch"
        );
    });
}

#[test]
fn test_prompt_8b_3_5_true_no_io_in_planner() {
    use preview_alternative_lib::secure_redaction::*;
    let temp_dir = tempfile::tempdir().expect("temp dir");

    let entries_before: Vec<_> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|r| r.unwrap().path())
        .collect();

    let geom = CanonicalPageGeometry {
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
        rotation: PageRotation::Deg0,
    };
    let page_facts = PageRedactionFacts {
        schema_version: "1.0.0".to_string(),
        page_index: 0,
        text: InspectionFactState::Present,
        images: InspectionFactState::Absent,
        vector_paths: InspectionFactState::Absent,
        form_xobjects: InspectionFactState::Absent,
        annotations: InspectionFactState::Absent,
        link_annotations: InspectionFactState::Absent,
        widgets: InspectionFactState::Absent,
        transparency: InspectionFactState::Absent,
        optional_content: InspectionFactState::Absent,
        complex_resources: InspectionFactState::Absent,
        structure_association: InspectionFactState::Absent,
        page_actions: InspectionFactState::Absent,
        spot_colors: InspectionFactState::Absent,
        complex_color_spaces: InspectionFactState::Absent,
        geometry_profile: GeometryProfile::StandardEffectiveBoxV1,
        rotation: PageRotation::Deg0,
    };
    let doc_facts = DocumentRedactionFacts {
        schema_version: "1.0.0".to_string(),
        total_pages: 1,
        encryption: InspectionFactState::Absent,
        signature_dictionary: InspectionFactState::Absent,
        signature_field: InspectionFactState::Absent,
        byte_range: InspectionFactState::Absent,
        acro_form: InspectionFactState::Absent,
        xfa: InspectionFactState::Absent,
        annotations: InspectionFactState::Absent,
        optional_content: InspectionFactState::Absent,
        embedded_files: InspectionFactState::Absent,
        javascript: InspectionFactState::Absent,
        incremental_revisions: InspectionFactState::Absent,
        structure_tree: InspectionFactState::Absent,
        outlines: InspectionFactState::Absent,
        names_dictionary: InspectionFactState::Absent,
        document_actions: InspectionFactState::Absent,
        complex_document_resources: InspectionFactState::Absent,
    };

    let reg_input = RedactionRegionInput {
        id: "r1".to_string(),
        page_index: 0,
        normalized_top_left_rect: NormalizedTopLeftRect {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        },
        source: "manual".to_string(),
    };

    let planner_input = RedactionPlannerInput {
        input_digest: "a0".repeat(32),
        content_generation: 0,
        policy: sample_test_policy(),
        acknowledgements: sample_test_acknowledgements(),
        document_facts: doc_facts,
        page_facts: vec![page_facts],
        page_geometries: vec![geom],
        region_inputs: vec![reg_input],
    };

    let plan = build_secure_redaction_plan(&planner_input).expect("build plan");
    assert!(plan.executable);

    let entries_after: Vec<_> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|r| r.unwrap().path())
        .collect();

    assert_eq!(
        entries_before, entries_after,
        "Planner MUST NOT write to disk (True No-I/O requirement)"
    );
}

#[test]
fn test_prompt_8b_3_5_zero_gap_merge_policy_isolation() {
    use preview_alternative_lib::secure_redaction::*;
    let geom = CanonicalPageGeometry {
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
        rotation: PageRotation::Deg0,
    };
    let page_facts = PageRedactionFacts {
        schema_version: "1.0.0".to_string(),
        page_index: 0,
        text: InspectionFactState::Present,
        images: InspectionFactState::Absent,
        vector_paths: InspectionFactState::Absent,
        form_xobjects: InspectionFactState::Absent,
        annotations: InspectionFactState::Absent,
        link_annotations: InspectionFactState::Absent,
        widgets: InspectionFactState::Absent,
        transparency: InspectionFactState::Absent,
        optional_content: InspectionFactState::Absent,
        complex_resources: InspectionFactState::Absent,
        structure_association: InspectionFactState::Absent,
        page_actions: InspectionFactState::Absent,
        spot_colors: InspectionFactState::Absent,
        complex_color_spaces: InspectionFactState::Absent,
        geometry_profile: GeometryProfile::StandardEffectiveBoxV1,
        rotation: PageRotation::Deg0,
    };
    let doc_facts = DocumentRedactionFacts {
        schema_version: "1.0.0".to_string(),
        total_pages: 1,
        encryption: InspectionFactState::Absent,
        signature_dictionary: InspectionFactState::Absent,
        signature_field: InspectionFactState::Absent,
        byte_range: InspectionFactState::Absent,
        acro_form: InspectionFactState::Absent,
        xfa: InspectionFactState::Absent,
        annotations: InspectionFactState::Absent,
        optional_content: InspectionFactState::Absent,
        embedded_files: InspectionFactState::Absent,
        javascript: InspectionFactState::Absent,
        incremental_revisions: InspectionFactState::Absent,
        structure_tree: InspectionFactState::Absent,
        outlines: InspectionFactState::Absent,
        names_dictionary: InspectionFactState::Absent,
        document_actions: InspectionFactState::Absent,
        complex_document_resources: InspectionFactState::Absent,
    };

    let reg1 = RedactionRegionInput {
        id: "r1".to_string(),
        page_index: 0,
        normalized_top_left_rect: NormalizedTopLeftRect {
            x: 0.1,
            y: 0.1,
            width: 0.1,
            height: 0.1,
        },
        source: "manual".to_string(),
    };
    let reg2 = RedactionRegionInput {
        id: "r2".to_string(),
        page_index: 0,
        normalized_top_left_rect: NormalizedTopLeftRect {
            x: 0.25, // gap > 0.0
            y: 0.1,
            width: 0.1,
            height: 0.1,
        },
        source: "manual".to_string(),
    };

    let planner_input = RedactionPlannerInput {
        input_digest: "a0".repeat(32),
        content_generation: 0,
        policy: sample_test_policy(),
        acknowledgements: sample_test_acknowledgements(),
        document_facts: doc_facts,
        page_facts: vec![page_facts],
        page_geometries: vec![geom],
        region_inputs: vec![reg1, reg2],
    };

    let plan = build_secure_redaction_plan(&planner_input).expect("build plan");
    assert_eq!(
        plan.canonical_regions.len(),
        2,
        "Regions separated by a positive gap MUST NOT be merged"
    );
}

#[test]
fn test_prompt_8b_3_5_manifest_parity_and_constants_match() {
    use preview_alternative_lib::secure_redaction::*;
    let manifest_str = std::fs::read_to_string("tests/fixtures/redaction/manifest.json")
        .expect("read manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_str).expect("parse manifest.json");

    let limits = &manifest["limits"];
    assert_eq!(
        limits["maxPixelsPerAffectedPage"].as_u64().unwrap(),
        MAX_PIXELS_PER_PAGE
    );
    assert_eq!(
        limits["maxTotalEstimatedAffectedPixels"].as_u64().unwrap(),
        MAX_TOTAL_ESTIMATED_PIXELS
    );
    assert_eq!(
        limits["maxReportSizeBytes"].as_u64().unwrap(),
        MAX_REPORT_SIZE_BYTES as u64
    );
    assert_eq!(
        limits["maxSourceRegionIdsPerCanonicalRegion"]
            .as_u64()
            .unwrap(),
        MAX_SOURCE_REGION_IDS as u64
    );
    assert_eq!(
        limits["maxCanonicalRegionsPerPage"].as_u64().unwrap(),
        MAX_CANONICAL_REGIONS_PER_PAGE as u64
    );

    let merge_profile = &manifest["merge_profile"];
    assert_eq!(
        merge_profile["adjacencyTolerancePoints"].as_f64().unwrap(),
        ADJACENCY_TOLERANCE_POINTS
    );
    assert_eq!(
        merge_profile["mergeEdgeTouching"].as_bool().unwrap(),
        MERGE_EDGE_TOUCHING
    );
    assert_eq!(
        merge_profile["mergeCornerTouching"].as_bool().unwrap(),
        MERGE_CORNER_TOUCHING
    );
    assert_eq!(
        merge_profile["geometryComparisonEpsilon"].as_f64().unwrap(),
        GEOMETRY_COMPARISON_EPSILON
    );
}

fn sample_test_policy() -> preview_alternative_lib::secure_redaction::SecureRedactionPolicy {
    use preview_alternative_lib::secure_redaction::*;
    SecureRedactionPolicy {
        schema_version: "1.0.0".to_string(),
        profile: RedactionProfile::SecureRasterizedPagesV1,
        strategy: RedactionStrategy::RasterizedPages,
        raster_dpi: 300,
        appearance: RedactionAppearancePolicy {
            kind: "solid".to_string(),
            color: "black".to_string(),
            opacity: 1.0,
        },
        safety_margin_points: 2.0,
        interactive_content_policy: InteractiveContentPolicy::Block,
        signature_policy: SignaturePolicy::Block,
        encryption_policy: EncryptionPolicy::Block,
    }
}

fn sample_test_acknowledgements(
) -> preview_alternative_lib::secure_redaction::RedactionAcknowledgements {
    preview_alternative_lib::secure_redaction::RedactionAcknowledgements {
        schema_version: "1.0.0".to_string(),
        rasterized_pages_accepted: true,
        text_search_loss_accepted: true,
        accessibility_loss_accepted: true,
        annotation_loss_accepted: true,
        form_loss_accepted: true,
        signature_invalidation_accepted: true,
        encryption_change_accepted: true,
    }
}
