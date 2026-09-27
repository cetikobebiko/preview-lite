use std::fs;
use std::path::Path;

#[test]
fn test_signature_module_and_handlers_are_completely_removed() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let signature_rs = manifest_dir.join("src").join("signature.rs");
    assert!(
        !signature_rs.exists(),
        "src/signature.rs must not exist in production codebase"
    );

    let lib_rs = fs::read_to_string(manifest_dir.join("src").join("lib.rs"))
        .expect("Failed to read src/lib.rs");

    assert!(
        !lib_rs.contains("pub mod signature;"),
        "lib.rs must not declare signature module"
    );
    assert!(
        !lib_rs.contains("mod signature;"),
        "lib.rs must not declare signature module"
    );

    let forbidden_handlers = [
        "signature::verify_pdf_signatures",
        "signature::list_system_certificates",
        "signature::sign_pdf_pades",
        "signature::enable_ltv",
        "verify_pdf_signatures",
        "list_system_certificates",
        "sign_pdf_pades",
        "enable_ltv",
    ];

    for handler in forbidden_handlers {
        assert!(
            !lib_rs.contains(handler),
            "lib.rs generate_handler! must not contain {}",
            handler
        );
    }
}

fn collect_rs_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
}

#[test]
fn test_no_mock_signature_dtos_or_synthetic_validity_in_production_rust() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let mut rs_files = Vec::new();
    collect_rs_files(&src_dir, &mut rs_files);

    assert!(
        !rs_files.is_empty(),
        "Production src directory must contain rust files"
    );

    let forbidden_identifiers = [
        "SignatureValidationResult",
        "VisualSignatureOptions",
        "SignatureOptions",
        "pub async fn list_system_certificates",
        "pub async fn sign_pdf_pades",
        "pub async fn verify_pdf_signatures",
        "pub async fn enable_ltv",
    ];

    for path in rs_files {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", path, e));

        for identifier in forbidden_identifiers {
            assert!(
                !content.contains(identifier),
                "Production file {:?} must not define or reference legacy mock signature identifier: {}",
                path.strip_prefix(manifest_dir).unwrap_or(&path),
                identifier
            );
        }
    }
}
