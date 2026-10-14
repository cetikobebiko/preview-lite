/// Batch processing has no real job execution yet. The command stays
/// registered so the frontend contract is stable, but fails closed before
/// reading any argument, like the other disabled feature stubs.
#[tauri::command]
pub async fn add_batch_job(
    _job_type: String,
    _file_path: String,
    _parameters: String,
) -> Result<String, String> {
    Err("FEATURE_UNAVAILABLE: Toplu iş işleme bu sürümde devre dışıdır".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_batch_job_fails_closed() {
        let error = add_batch_job("ocr".into(), "/a.pdf".into(), "{}".into())
            .await
            .unwrap_err();
        assert!(error.starts_with("FEATURE_UNAVAILABLE:"), "{error}");
    }
}
