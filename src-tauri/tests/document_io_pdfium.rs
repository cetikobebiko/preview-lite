use pdfium_render::prelude::*;
use preview_alternative_lib::get_pdfium;
use preview_alternative_lib::manipulation::{reorder_pdf_to_target, ReorderMode};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pdf")
        .join(name)
}

fn copied_fixture(name: &str, directory: &Path) -> PathBuf {
    let target = directory.join(name);
    fs::copy(fixture(name), &target).unwrap();
    target
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VisiblePage {
    text: String,
    rotation: String,
    annotation_contents: Vec<Option<String>>,
}

fn visible_pages(path: &Path) -> Vec<VisiblePage> {
    let pdfium = get_pdfium().unwrap();
    let document = pdfium.load_pdf_from_file(path, None).unwrap();
    document
        .pages()
        .iter()
        .map(|page| {
            let annotations = page.annotations();
            let annotation_contents = (0..annotations.len())
                .map(|index| annotations.get(index).unwrap().contents())
                .collect();
            VisiblePage {
                text: page.text().unwrap().all(),
                rotation: format!("{:?}", page.rotation().unwrap()),
                annotation_contents,
            }
        })
        .collect()
}

#[test]
fn reorder_full_and_subset_preserve_text_rotation_and_annotations() {
    let directory = tempfile::tempdir().unwrap();
    let source = copied_fixture("reorder-rich.pdf", directory.path());
    let subset = directory.path().join("subset.pdf");
    let before = visible_pages(&source);

    reorder_pdf_to_target(
        source.to_string_lossy().to_string(),
        vec![2, 0],
        subset.to_string_lossy().to_string(),
        None,
        ReorderMode::OrderedSubset,
    )
    .unwrap();
    assert_eq!(
        visible_pages(&subset),
        vec![before[2].clone(), before[0].clone()]
    );

    reorder_pdf_to_target(
        source.to_string_lossy().to_string(),
        vec![2, 0, 1],
        source.to_string_lossy().to_string(),
        None,
        ReorderMode::FullPermutation,
    )
    .unwrap();
    assert_eq!(
        visible_pages(&source),
        vec![before[2].clone(), before[0].clone(), before[1].clone()]
    );
}

#[test]
fn reorder_rejects_document_features_it_cannot_preserve() {
    for fixture_name in [
        "reorder-with-metadata.pdf",
        "reorder-with-outline.pdf",
        "reorder-with-named-destination.pdf",
        "reorder-with-form.pdf",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = copied_fixture(fixture_name, directory.path());
        let original = fs::read(&source).unwrap();
        let error = reorder_pdf_to_target(
            source.to_string_lossy().to_string(),
            vec![2, 0, 1],
            source.to_string_lossy().to_string(),
            None,
            ReorderMode::FullPermutation,
        )
        .unwrap_err();

        assert!(error.contains("güvenli biçimde koruyamıyor"), "{error}");
        assert_eq!(fs::read(source).unwrap(), original);
    }
}
