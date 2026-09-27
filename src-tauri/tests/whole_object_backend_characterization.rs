use lopdf::{Document, Object};
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WholeObjectBackendDecision {
    Go,
    NoGo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MandatoryCapabilityGate {
    standalone_text_payload_removed: bool,
    standalone_image_payload_removed: bool,
    orphan_resources_pruned: bool,
    form_isolation_proven: bool,
    conservative_bounds_proven: bool,
    independent_verifier_complete: bool,
    detached_object_lifetime_proven: bool,
    shared_resource_isolation_proven: bool,
    outside_region_preservation_proven: bool,
    tagged_structure_update_proven: bool,
    page_geometry_preservation_proven: bool,
    visual_coverage_proven: bool,
}

impl MandatoryCapabilityGate {
    fn decision(self) -> WholeObjectBackendDecision {
        if self.standalone_text_payload_removed
            && self.standalone_image_payload_removed
            && self.orphan_resources_pruned
            && self.form_isolation_proven
            && self.conservative_bounds_proven
            && self.independent_verifier_complete
            && self.detached_object_lifetime_proven
            && self.shared_resource_isolation_proven
            && self.outside_region_preservation_proven
            && self.tagged_structure_update_proven
            && self.page_geometry_preservation_proven
            && self.visual_coverage_proven
        {
            WholeObjectBackendDecision::Go
        } else {
            WholeObjectBackendDecision::NoGo
        }
    }
}

fn with_test_pdfium<T>(operation: impl FnOnce(&Pdfium) -> T) -> T {
    preview_alternative_lib::pdf_engine::with_pdfium(|pdfium| Ok::<T, String>(operation(pdfium)))
        .expect("Initialize repository PDFium for whole-object characterization")
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("redaction")
        .join(name)
}

fn remove_top_level_object_and_save(
    pdfium: &Pdfium,
    source: &Path,
    output: &Path,
    object_type: PdfPageObjectType,
) {
    let document = pdfium
        .load_pdf_from_file(source, None)
        .expect("open characterization fixture");

    {
        let mut page = document.pages().get(0).expect("load fixture page");
        let index = page
            .objects()
            .iter()
            .position(|object| object.object_type() == object_type)
            .expect("fixture must contain requested top-level object");
        let detached = page
            .objects_mut()
            .remove_object_at_index(index)
            .expect("remove top-level object");
        // This helper characterizes serialization only. The separate lifetime probe
        // exercises the required destroy sequence in an isolated child process.
        std::mem::forget(detached);
    }

    document
        .save_to_file(output)
        .expect("full-save characterized object removal output");
}

fn decoded_streams_contain(document: &Document, marker: &[u8]) -> bool {
    document.objects.values().any(|object| {
        object
            .as_stream()
            .ok()
            .and_then(|stream| stream.decompressed_content().ok())
            .is_some_and(|bytes| bytes.windows(marker.len()).any(|window| window == marker))
    })
}

fn page_xobject_ids(document: &Document) -> Vec<lopdf::ObjectId> {
    let mut ids = Vec::new();
    for page_id in document.get_pages().values() {
        let page = document.get_object(*page_id).expect("page object");
        let page_dict = page.as_dict().expect("page dictionary");
        let resources = match page_dict.get(b"Resources") {
            Ok(Object::Reference(id)) => document
                .get_object(*id)
                .expect("referenced resources")
                .as_dict()
                .expect("resources dictionary"),
            Ok(Object::Dictionary(dict)) => dict,
            _ => continue,
        };
        let Ok(xobjects) = resources.get(b"XObject") else {
            continue;
        };
        let xobjects = match xobjects {
            Object::Reference(id) => document
                .get_object(*id)
                .expect("referenced XObject dictionary")
                .as_dict()
                .expect("XObject dictionary"),
            Object::Dictionary(dict) => dict,
            _ => continue,
        };
        ids.extend(
            xobjects
                .iter()
                .filter_map(|(_, value)| value.as_reference().ok()),
        );
    }
    ids
}

#[test]
fn mandatory_gate_is_fail_closed() {
    let complete = MandatoryCapabilityGate {
        standalone_text_payload_removed: true,
        standalone_image_payload_removed: true,
        orphan_resources_pruned: true,
        form_isolation_proven: true,
        conservative_bounds_proven: true,
        independent_verifier_complete: true,
        detached_object_lifetime_proven: true,
        shared_resource_isolation_proven: true,
        outside_region_preservation_proven: true,
        tagged_structure_update_proven: true,
        page_geometry_preservation_proven: true,
        visual_coverage_proven: true,
    };
    assert_eq!(complete.decision(), WholeObjectBackendDecision::Go);

    let fail_one = [
        |gate: &mut MandatoryCapabilityGate| gate.standalone_text_payload_removed = false,
        |gate: &mut MandatoryCapabilityGate| gate.standalone_image_payload_removed = false,
        |gate: &mut MandatoryCapabilityGate| gate.orphan_resources_pruned = false,
        |gate: &mut MandatoryCapabilityGate| gate.form_isolation_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.conservative_bounds_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.independent_verifier_complete = false,
        |gate: &mut MandatoryCapabilityGate| gate.detached_object_lifetime_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.shared_resource_isolation_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.outside_region_preservation_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.tagged_structure_update_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.page_geometry_preservation_proven = false,
        |gate: &mut MandatoryCapabilityGate| gate.visual_coverage_proven = false,
    ];
    for fail in fail_one {
        let mut gate = complete;
        fail(&mut gate);
        assert_eq!(gate.decision(), WholeObjectBackendDecision::NoGo);
    }
}

#[test]
fn top_level_text_removal_regenerates_and_reopens_without_text_payload() {
    with_test_pdfium(|pdfium| {
        let temp = tempfile::tempdir().expect("temporary output directory");
        let output = temp.path().join("text-removed.pdf");
        let marker = b"CONFIDENTIAL TC: 12345678901 EMAIL: user@example.com";

        remove_top_level_object_and_save(
            pdfium,
            &fixture_path("simple-text.pdf"),
            &output,
            PdfPageObjectType::Text,
        );

        let reopened = pdfium
            .load_pdf_from_file(&output, None)
            .expect("PDFium reopen text-removal output");
        let page = reopened.pages().get(0).expect("reopen page");
        assert!(page
            .text()
            .expect("extract reopened text")
            .all()
            .trim()
            .is_empty());
        assert_eq!(
            page.objects()
                .iter()
                .filter(|object| object.object_type() == PdfPageObjectType::Text)
                .count(),
            0
        );

        let parsed = Document::load(&output).expect("lopdf reopen text-removal output");
        assert!(!decoded_streams_contain(&parsed, marker));
        assert!(!std::fs::read(&output)
            .expect("read serialized text-removal output")
            .windows(marker.len())
            .any(|window| window == marker));
        assert!(parsed.trailer.get(b"Prev").is_err());
    });
}

#[test]
fn top_level_image_removal_prunes_source_image_resource_and_payload() {
    with_test_pdfium(|pdfium| {
        let temp = tempfile::tempdir().expect("temporary output directory");
        let output = temp.path().join("image-removed.pdf");
        let source_pixels = [0_u8, 255, 255, 0];

        remove_top_level_object_and_save(
            pdfium,
            &fixture_path("image-only.pdf"),
            &output,
            PdfPageObjectType::Image,
        );

        let reopened = pdfium
            .load_pdf_from_file(&output, None)
            .expect("PDFium reopen image-removal output");
        let page = reopened.pages().get(0).expect("reopen page");
        assert_eq!(page.objects().len(), 0, "placement object is removed");

        let parsed = Document::load(&output).expect("lopdf reopen image-removal output");
        assert!(
            page_xobject_ids(&parsed).is_empty(),
            "PDFium full-save prunes the removed image from page resources"
        );
        assert!(
            !decoded_streams_contain(&parsed, &source_pixels),
            "the exact source image payload must not remain in a serialized stream"
        );

        let mut pruned = parsed.clone();
        assert!(
            pruned.prune_objects().is_empty(),
            "lopdf pruning cannot remove the payload because PDFium kept it reachable"
        );
        assert!(parsed.trailer.get(b"Prev").is_err());
    });
}

#[test]
fn detached_object_destroy_child_probe() {
    if std::env::var_os("WHOLE_OBJECT_DESTROY_PROBE").is_none() {
        return;
    }

    with_test_pdfium(|pdfium| {
        let document = pdfium
            .load_pdf_from_file(&fixture_path("simple-text.pdf"), None)
            .expect("open lifetime probe fixture");
        let mut page = document.pages().get(0).expect("load lifetime probe page");
        let detached = page
            .objects_mut()
            .remove_object_at_index(0)
            .expect("remove text object and regenerate page content");
        drop(detached);
    });
}

#[cfg(target_os = "macos")]
#[test]
fn detached_object_destroy_sequence_is_not_safely_proven() {
    use std::os::unix::process::ExitStatusExt;

    let executable = std::env::current_exe().expect("current integration test executable");
    for attempt in 0..3 {
        let output = Command::new(&executable)
            .arg("--exact")
            .arg("detached_object_destroy_child_probe")
            .arg("--nocapture")
            .env("WHOLE_OBJECT_DESTROY_PROBE", "1")
            .output()
            .expect("run isolated detached-object lifetime probe");
        assert!(
            !output.status.success(),
            "attempt {attempt}: remove -> generate -> destroy unexpectedly succeeded; review the explicit NO-GO decision before changing this characterization"
        );
        assert_eq!(
            output.status.signal(),
            Some(libc::SIGSEGV),
            "attempt {attempt}: lifetime probe must reproduce the characterized SIGSEGV"
        );
    }
}

#[test]
fn adversarial_fixture_characterizes_bounds_and_form_enumeration() {
    with_test_pdfium(|pdfium| {
        let document = pdfium
            .load_pdf_from_file(&fixture_path("whole-object-adversarial.pdf"), None)
            .expect("PDFium reopen adversarial fixture");
        let page = document.pages().get(0).expect("load adversarial page");

        let mut text_count = 0;
        let mut image_count = 0;
        let mut path_count = 0;
        let mut form_count = 0;
        let mut nested_form_observed = false;
        let mut form_text_child_observed = false;

        for object in page.objects().iter() {
            let bounds = object.bounds().expect("PDFium object bounds");
            for value in [
                bounds.left().value,
                bounds.bottom().value,
                bounds.right().value,
                bounds.top().value,
            ] {
                assert!(value.is_finite(), "object bounds must be finite");
            }

            match object.object_type() {
                PdfPageObjectType::Text => text_count += 1,
                PdfPageObjectType::Image => image_count += 1,
                PdfPageObjectType::Path => path_count += 1,
                PdfPageObjectType::XObjectForm => {
                    form_count += 1;
                    let form = object
                        .as_x_object_form_object()
                        .expect("Form XObject downcast");
                    assert!(
                        !form.is_empty(),
                        "form child enumeration must return children"
                    );
                    for child in form.iter() {
                        let child_bounds = child.bounds().expect("transformed form child bounds");
                        assert!(child_bounds.left().value.is_finite());
                        if child.object_type() == PdfPageObjectType::Text {
                            form_text_child_observed = true;
                        }
                        if child.object_type() == PdfPageObjectType::XObjectForm {
                            nested_form_observed = true;
                            let nested = child
                                .as_x_object_form_object()
                                .expect("nested Form downcast");
                            assert!(!nested.is_empty(), "nested form children are enumerable");
                        }
                    }
                }
                _ => {}
            }
        }

        assert!(
            text_count >= 6,
            "plain/rotated/skewed/clipped/layer/tagged text"
        );
        assert_eq!(image_count, 2, "positive and negative image transforms");
        assert!(
            path_count >= 4,
            "fill/stroke/transparency/pattern path objects"
        );
        assert_eq!(
            form_count, 3,
            "nested host plus two shared Form invocations"
        );
        assert!(form_text_child_observed);
        assert!(nested_form_observed);

        let parsed = Document::load(fixture_path("whole-object-adversarial.pdf"))
            .expect("lopdf reopen adversarial fixture");
        let catalog = parsed.catalog().expect("catalog dictionary");
        assert!(catalog.has(b"OCProperties"));
        assert!(catalog.has(b"StructTreeRoot"));
        let bytes = std::fs::read(fixture_path("whole-object-adversarial.pdf"))
            .expect("read adversarial fixture");
        for marker in [
            b"/SMask".as_slice(),
            b"/Pattern".as_slice(),
            b"/Shading".as_slice(),
            b"/MCID".as_slice(),
        ] {
            assert!(bytes.windows(marker.len()).any(|window| window == marker));
        }
    });
}

#[test]
fn form_child_removal_is_unavailable_through_repository_wrapper() {
    with_test_pdfium(|pdfium| {
        let document = pdfium
            .load_pdf_from_file(&fixture_path("whole-object-adversarial.pdf"), None)
            .expect("open form-removal characterization fixture");
        let page = document.pages().get(0).expect("load form-removal page");
        let mut form_object = page
            .objects()
            .iter()
            .find(|object| object.object_type() == PdfPageObjectType::XObjectForm)
            .expect("top-level Form XObject");
        let form = form_object
            .as_x_object_form_object_mut()
            .expect("mutable Form XObject downcast");
        let child = form.first().expect("enumerated Form child");
        let error = match form.remove_object(child) {
            Ok(_) => panic!("Form child removal unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            PdfiumError::PageObjectsCollectionIsImmutable
        ));
    });
}

#[test]
fn repository_whole_object_decision_is_explicit_no_go() {
    let characterized_repository_capabilities = MandatoryCapabilityGate {
        standalone_text_payload_removed: true,
        standalone_image_payload_removed: true,
        orphan_resources_pruned: true,
        form_isolation_proven: false,
        conservative_bounds_proven: false,
        independent_verifier_complete: false,
        detached_object_lifetime_proven: false,
        shared_resource_isolation_proven: false,
        outside_region_preservation_proven: false,
        tagged_structure_update_proven: false,
        page_geometry_preservation_proven: false,
        visual_coverage_proven: false,
    };
    assert_eq!(
        characterized_repository_capabilities.decision(),
        WholeObjectBackendDecision::NoGo
    );
}
