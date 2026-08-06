use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct SignatureOptions {
    pub reason: String,
    pub location: String,
    pub contact_info: Option<String>,
    pub visual_signature: Option<VisualSignatureOptions>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VisualSignatureOptions {
    pub page_index: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub signature_text: Option<String>,
    pub signature_image_base64: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CertificateInfo {
    pub id: String,
    pub subject: String,
    pub issuer: String,
    pub serial_number: String,
    pub not_before: String,
    pub not_after: String,
    pub token_label: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SignatureValidationResult {
    pub id: String,
    pub is_valid: bool,
    pub is_document_modified: bool,
    pub is_certificate_trusted: bool,
    pub is_certificate_expired: bool,
    pub has_timestamp: bool,
    pub ltv_enabled: bool,
    pub subject: String,
    pub signing_time: Option<String>,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn list_system_certificates() -> Result<Vec<CertificateInfo>, String> {
    Err(
        "FEATURE_UNAVAILABLE: Sistem sertifika deposu entegrasyonu mock-disabled durumdadır"
            .to_string(),
    )
}

#[tauri::command]
// Tauri IPC contract intentionally exposes each frontend field as a top-level argument.
#[allow(clippy::too_many_arguments)]
pub async fn sign_pdf_pades(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    output_path: String,
    cert_id: String,
    _pin: String,
    options: SignatureOptions,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<
    crate::document_session::SessionJobOutcome<()>,
    crate::document_session::DocumentSessionError,
> {
    let _ = (
        job_id,
        session_id,
        expected_content_generation,
        output_path,
        cert_id,
        options,
        store,
    );
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "PAdES imzalama mock-disabled durumdadır",
    ))
}

#[tauri::command]
pub async fn verify_pdf_signatures(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<
    crate::document_session::SessionJobOutcome<Vec<SignatureValidationResult>>,
    crate::document_session::DocumentSessionError,
> {
    let _ = (job_id, session_id, expected_content_generation, store);
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "PDF imza doğrulaması mock-disabled durumdadır",
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn verify_pdf_signatures_for_session(
    store: &crate::document_session::DocumentSessionStore,
    job_id: &str,
    session_id: &str,
    expected_content_generation: u64,
) -> Result<
    crate::document_session::SessionJobOutcome<Vec<SignatureValidationResult>>,
    crate::document_session::DocumentSessionError,
> {
    let _ = (store, job_id, session_id, expected_content_generation);
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "PDF imza doğrulaması mock-disabled durumdadır",
    ))
}

#[tauri::command]
pub async fn enable_ltv(
    _session_id: String,
    _expected_content_generation: u64,
    _signature_id: String,
    _store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<(), crate::document_session::DocumentSessionError> {
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "LTV/DSS yerleştirme bu sürümde mock-disabled durumdadır",
    ))
}
