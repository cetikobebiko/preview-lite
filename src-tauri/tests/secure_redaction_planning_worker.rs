use preview_alternative_lib::secure_redaction::*;
use preview_alternative_lib::secure_redaction_inspection::{
    inspect_and_plan_secure_redaction, resolve_effective_resources, resolve_inherited_box,
    InheritedLookup,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("redaction")
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

fn sample_region() -> RedactionRegionInput {
    RedactionRegionInput {
        id: "r1".to_string(),
        page_index: 0,
        normalized_top_left_rect: NormalizedTopLeftRect {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        },
        source: "manual".to_string(),
    }
}

#[test]
fn test_simple_text_fixture_inspection_and_planning() {
    let pdf_path = fixture_dir().join("simple-text.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for simple-text.pdf must succeed");

    assert_eq!(plan.schema_version, REDACTION_SCHEMA_VERSION);
    assert_eq!(plan.plan_version, SECURE_REDACTION_PLAN_VERSION);
    assert_eq!(plan.content_generation, 1);
    assert!(plan.executable);
    assert!(plan.ready_for_execution);
    assert!(plan.blockers.is_empty());

    let raw_bytes = fs::read(&pdf_path).unwrap();
    use sha2::{Digest, Sha256};
    let expected_digest = format!("{:x}", Sha256::digest(&raw_bytes));
    assert_eq!(plan.input_digest, expected_digest);
    assert_eq!(plan.facts.total_pages, 1);
    assert_eq!(plan.facts.encryption, InspectionFactState::Absent);
    assert_eq!(plan.page_plans[0].facts.text, InspectionFactState::Present);
    assert_eq!(plan.page_plans[0].facts.images, InspectionFactState::Absent);
}

#[test]
fn test_image_only_fixture_inspection() {
    let pdf_path = fixture_dir().join("image-only.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for image-only.pdf must succeed");

    assert_eq!(
        plan.page_plans[0].facts.images,
        InspectionFactState::Present
    );
    assert_eq!(plan.page_plans[0].facts.text, InspectionFactState::Absent);
}

#[test]
fn test_text_image_fixture_inspection() {
    let pdf_path = fixture_dir().join("text-image.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for text-image.pdf must succeed");

    assert_eq!(plan.page_plans[0].facts.text, InspectionFactState::Present);
    assert_eq!(
        plan.page_plans[0].facts.images,
        InspectionFactState::Present
    );
}

#[test]
fn test_annotations_fixture_inspection() {
    let pdf_path = fixture_dir().join("annotations-fixture.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for annotations-fixture.pdf must succeed");

    assert_eq!(plan.facts.annotations, InspectionFactState::Present);
    assert_eq!(
        plan.page_plans[0].facts.annotations,
        InspectionFactState::Present
    );
}

#[test]
fn test_acroform_widget_fixture_inspection() {
    let pdf_path = fixture_dir().join("acroform-widget.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for acroform-widget.pdf must succeed");

    assert_eq!(plan.facts.acro_form, InspectionFactState::Present);
    assert_eq!(plan.facts.xfa, InspectionFactState::Absent);
    assert_eq!(
        plan.page_plans[0].facts.widgets,
        InspectionFactState::Present
    );
}

#[test]
fn test_rotations_fixture_geometry_extraction() {
    let pdf_path = fixture_dir().join("rotations.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        2,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for rotations.pdf must succeed");

    assert_eq!(plan.facts.total_pages, 4);
    assert_eq!(plan.page_plans.len(), 4);

    let rots: Vec<PageRotation> = plan
        .page_plans
        .iter()
        .map(|p| p.geometry.rotation.clone())
        .collect();
    assert_eq!(
        rots,
        vec![
            PageRotation::Deg0,
            PageRotation::Deg90,
            PageRotation::Deg180,
            PageRotation::Deg270
        ]
    );
}

#[test]
fn test_cropbox_mediabox_diff_fixture_geometry() {
    let pdf_path = fixture_dir().join("cropbox-mediabox-diff.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        3,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for cropbox-mediabox-diff.pdf must succeed");

    let geom = &plan.page_plans[0].geometry;
    assert!(geom.crop_box.is_some());
    let crop = geom.crop_box.as_ref().unwrap();
    assert_eq!(crop.left, 50.0);
    assert_eq!(crop.bottom, 50.0);
    assert_eq!(crop.width, 500.0);
    assert_eq!(crop.height, 700.0);
    assert_ne!(geom.media_box, *crop);
    assert_eq!(geom.effective_box, *crop);
}

#[test]
fn test_inherited_geometry_fixture_inspection() {
    let pdf_path = fixture_dir().join("inherited-geometry.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for inherited-geometry.pdf must succeed");

    let geom = &plan.page_plans[0].geometry;
    assert_eq!(geom.media_box.left, 10.0);
    assert_eq!(geom.media_box.bottom, 20.0);
    assert_eq!(geom.media_box.width, 600.0);
    assert_eq!(geom.media_box.height, 800.0);

    assert!(geom.crop_box.is_some());
    let crop = geom.crop_box.as_ref().unwrap();
    assert_eq!(crop.left, 30.0);
    assert_eq!(crop.bottom, 40.0);
    assert_eq!(crop.width, 560.0);
    assert_eq!(crop.height, 760.0);

    assert_eq!(geom.effective_box, *crop);
}

#[test]
fn test_inherited_resources_fixture_inspection() {
    let pdf_path = fixture_dir().join("inherited-resources.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for inherited-resources.pdf must succeed");

    assert_eq!(
        plan.page_plans[0].facts.transparency,
        InspectionFactState::Present
    );
}

#[test]
fn test_nonzero_origin_fixture_inspection() {
    let pdf_path = fixture_dir().join("nonzero-origin.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for nonzero-origin.pdf must succeed");

    let geom = &plan.page_plans[0].geometry;
    assert_eq!(geom.media_box.left, 50.0);
    assert_eq!(geom.media_box.bottom, 50.0);
    assert_eq!(geom.media_box.width, 600.0);
    assert_eq!(geom.media_box.height, 800.0);
}

#[test]
fn test_negative_origin_fixture_inspection() {
    let pdf_path = fixture_dir().join("negative-origin.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for negative-origin.pdf must succeed");

    let geom = &plan.page_plans[0].geometry;
    assert_eq!(geom.media_box.left, -100.0);
    assert_eq!(geom.media_box.bottom, -100.0);
    assert_eq!(geom.media_box.width, 600.0);
    assert_eq!(geom.media_box.height, 800.0);
}

#[test]
fn test_embedded_files_fixture_inspection() {
    let pdf_path = fixture_dir().join("embedded-files.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for embedded-files.pdf must succeed");

    assert_eq!(plan.facts.embedded_files, InspectionFactState::Present);
}

#[test]
fn test_non_js_openaction_fixture_inspection() {
    let pdf_path = fixture_dir().join("non-js-openaction.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for non-js-openaction.pdf must succeed");

    assert_eq!(plan.facts.document_actions, InspectionFactState::Present);
    assert_eq!(plan.facts.javascript, InspectionFactState::Absent);
}

#[test]
fn test_js_action_fixture_inspection() {
    let pdf_path = fixture_dir().join("js-action.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for js-action.pdf must succeed");

    assert_eq!(plan.facts.javascript, InspectionFactState::Present);
}

#[test]
fn test_stream_literal_prev_false_positive_resistance() {
    let pdf_path = fixture_dir().join("stream-literal-prev.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for stream-literal-prev.pdf must succeed");

    assert_eq!(
        plan.facts.incremental_revisions,
        InspectionFactState::Absent
    );
}

#[test]
fn test_unresolved_acroform_produces_xfa_unknown() {
    let pdf_path = fixture_dir().join("unresolved-acroform.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for unresolved-acroform.pdf must succeed");

    assert_eq!(plan.facts.acro_form, InspectionFactState::Present);
    assert_eq!(plan.facts.xfa, InspectionFactState::Unknown);
}

#[test]
fn test_unresolved_resources_produces_unknown_facts() {
    let pdf_path = fixture_dir().join("unresolved-resources.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Plan generation for unresolved-resources.pdf must succeed");

    assert_eq!(
        plan.page_plans[0].facts.transparency,
        InspectionFactState::Unknown
    );
}

#[test]
fn test_encrypted_fixture_inspection_with_credential_produces_blocker() {
    let pdf_path = fixture_dir().join("encrypted-redaction.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        Some("fixture-password"),
        4,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Encrypted PDF authenticated planning must return plan with blocker");

    assert_eq!(plan.facts.encryption, InspectionFactState::Present);
    assert!(!plan.executable);
    assert!(!plan.ready_for_execution);
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.code == "REDACTION_ENCRYPTED_INPUT_UNSUPPORTED"));
}

#[test]
fn test_encrypted_wrong_credential_returns_open_auth_failure() {
    let pdf_path = fixture_dir().join("encrypted-redaction.pdf");
    let err = inspect_and_plan_secure_redaction(
        &pdf_path,
        Some("wrong-password"),
        4,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect_err("Wrong password must return typed open/auth error");

    match err {
        preview_alternative_lib::document_session::SessionJobOperationError::Failed(failure) => {
            assert_eq!(failure.code, "SESSION_JOB_PDF_OPEN_FAILED");
            assert!(!failure.message.contains("wrong-password"));
        }
        _ => panic!("Expected SessionJobOperationError::Failed"),
    }
}

#[test]
fn test_encrypted_missing_credential_returns_open_auth_failure() {
    let pdf_path = fixture_dir().join("encrypted-redaction.pdf");
    let err = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        4,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect_err("Missing password must return typed open/auth error");

    match err {
        preview_alternative_lib::document_session::SessionJobOperationError::Failed(failure) => {
            assert_eq!(failure.code, "SESSION_JOB_PDF_OPEN_FAILED");
        }
        _ => panic!("Expected SessionJobOperationError::Failed"),
    }
}

#[test]
fn test_signature_fixture_inspection_produces_signature_facts() {
    let pdf_path = fixture_dir().join("signature-marked-byterange.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        5,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Signature fixture planning must succeed with blocker");

    assert_eq!(
        plan.facts.signature_dictionary,
        InspectionFactState::Present
    );
    assert_eq!(plan.facts.byte_range, InspectionFactState::Present);
    assert!(!plan.executable);
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.code == "REDACTION_SIGNED_INPUT_UNSUPPORTED"));
}

#[test]
fn test_xfa_fixture_inspection_produces_xfa_blocker() {
    let pdf_path = fixture_dir().join("xfa-fixture.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        6,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("XFA fixture planning must succeed with blocker");

    assert_eq!(plan.facts.xfa, InspectionFactState::Present);
    assert!(!plan.executable);
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.code == "REDACTION_XFA_INPUT_UNSUPPORTED"));
}

#[test]
fn test_incremental_revision_fixture_inspection() {
    let pdf_path = fixture_dir().join("incremental-revision.pdf");
    let plan = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        7,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .expect("Incremental revision planning must succeed");

    assert_eq!(
        plan.facts.incremental_revisions,
        InspectionFactState::Present
    );
}

#[test]
fn test_plan_digest_determinism_and_variations() {
    let pdf_path = fixture_dir().join("simple-text.pdf");

    let plan1 = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        10,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .unwrap();

    let plan2 = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        10,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .unwrap();

    assert_eq!(plan1.plan_digest, plan2.plan_digest);

    let mut reg2 = sample_region();
    reg2.id = "r2".to_string();
    let plan3 = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        10,
        sample_policy(),
        sample_acknowledgements(),
        vec![reg2],
    )
    .unwrap();

    assert_ne!(plan1.plan_digest, plan3.plan_digest);

    let plan4 = inspect_and_plan_secure_redaction(
        &pdf_path,
        None,
        11,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .unwrap();

    assert_ne!(plan1.plan_digest, plan4.plan_digest);
}

#[test]
fn test_no_output_invariant_during_planning() {
    let temp_dir = tempdir().unwrap();
    let snapshot = temp_dir.path().join("snapshot.pdf");
    fs::copy(fixture_dir().join("simple-text.pdf"), &snapshot).unwrap();

    let entries_before: Vec<_> = fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|r| r.unwrap().path())
        .collect();

    let _plan = inspect_and_plan_secure_redaction(
        &snapshot,
        None,
        1,
        sample_policy(),
        sample_acknowledgements(),
        vec![sample_region()],
    )
    .unwrap();

    let entries_after: Vec<_> = fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|r| r.unwrap().path())
        .collect();

    assert_eq!(
        entries_before, entries_after,
        "Planning must not create output files or artifacts"
    );
}

#[test]
fn test_resources_nearest_value_inheritance_and_shadowing() {
    let mut doc = lopdf::Document::new();

    // Parent resources: ExtGState
    let parent_res_dict = lopdf::dictionary! {
        "ExtGState" => lopdf::Object::Dictionary(lopdf::dictionary!{})
    };
    let parent_res_id = doc.add_object(parent_res_dict);
    let parent_dict = lopdf::dictionary! {
        "Type" => "Pages",
        "Resources" => lopdf::Object::Reference(parent_res_id)
    };
    let parent_id = doc.add_object(parent_dict);

    // Leaf resources: Font only
    let leaf_res_dict = lopdf::dictionary! {
        "Font" => lopdf::Object::Dictionary(lopdf::dictionary!{})
    };
    let leaf_res_id = doc.add_object(leaf_res_dict);
    let leaf_dict = lopdf::dictionary! {
        "Type" => "Page",
        "Parent" => lopdf::Object::Reference(parent_id),
        "Resources" => lopdf::Object::Reference(leaf_res_id)
    };
    let leaf_id = doc.add_object(leaf_dict);

    let effective = resolve_effective_resources(&doc, leaf_id);
    match effective {
        InheritedLookup::Found(res) => {
            assert!(res.has(b"Font"));
            assert!(
                !res.has(b"ExtGState"),
                "Parent ExtGState must be shadowed by leaf Resources"
            );
        }
        other => panic!("Expected InheritedLookup::Found, got {:?}", other),
    }

    // Malformed local Resources + valid parent Resources -> Unknown
    let bad_leaf_dict = lopdf::dictionary! {
        "Type" => "Page",
        "Parent" => lopdf::Object::Reference(parent_id),
        "Resources" => lopdf::Object::Reference((9999, 0))
    };
    let bad_leaf_id = doc.add_object(bad_leaf_dict);
    let bad_effective = resolve_effective_resources(&doc, bad_leaf_id);
    assert_eq!(
        bad_effective,
        InheritedLookup::Unknown,
        "Malformed local Resources must not inherit parent"
    );
}

#[test]
fn test_resource_traversal_cycle_and_depth_exhaustion() {
    let mut doc = lopdf::Document::new();

    let id1 = (10, 0);
    let id2 = (11, 0);

    let node1 = lopdf::dictionary! {
        "Type" => "Pages",
        "Parent" => lopdf::Object::Reference(id2)
    };
    let node2 = lopdf::dictionary! {
        "Type" => "Pages",
        "Parent" => lopdf::Object::Reference(id1)
    };
    doc.objects.insert(id1, lopdf::Object::Dictionary(node1));
    doc.objects.insert(id2, lopdf::Object::Dictionary(node2));

    let res = resolve_effective_resources(&doc, id1);
    assert_eq!(
        res,
        InheritedLookup::Unknown,
        "Resource traversal cycle must return Unknown"
    );
}

#[test]
fn test_box_inheritance_shadowing_and_traversal_failures() {
    let mut doc = lopdf::Document::new();

    let parent_dict = lopdf::dictionary! {
        "Type" => "Pages",
        "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()]
    };
    let parent_id = doc.add_object(parent_dict);

    let malformed_leaf_dict = lopdf::dictionary! {
        "Type" => "Page",
        "Parent" => lopdf::Object::Reference(parent_id),
        "MediaBox" => "not_an_array"
    };
    let malformed_leaf_id = doc.add_object(malformed_leaf_dict);

    let leaf_dict_valid = lopdf::dictionary! {
        "Type" => "Page",
        "Parent" => lopdf::Object::Reference(parent_id)
    };
    let leaf_id_valid = doc.add_object(leaf_dict_valid);

    // Malformed local MediaBox -> Unknown (must NOT fall back to parent MediaBox)
    let bad_box = resolve_inherited_box(&doc, malformed_leaf_id, b"MediaBox");
    assert_eq!(
        bad_box,
        InheritedLookup::Unknown,
        "Malformed local MediaBox must return Unknown"
    );

    // Valid parent MediaBox inherited when leaf has no MediaBox
    let valid_box = resolve_inherited_box(&doc, leaf_id_valid, b"MediaBox");
    match valid_box {
        InheritedLookup::Found(b) => {
            assert_eq!(b.width, 600.0);
            assert_eq!(b.height, 800.0);
        }
        other => panic!("Expected InheritedLookup::Found, got {:?}", other),
    }
}
