use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub job_type: String, // e.g. "ocr", "watermark", "plugin_macro"
    pub file_path: String,
    pub parameters: String, // JSON serialized parameters
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub job_id: String,
    pub status: String, // "pending", "processing", "completed", "error"
    pub error_msg: Option<String>,
}

pub struct BatchQueue {
    pub sender: mpsc::Sender<BatchJob>,
}

pub async fn start_batch_worker(
    app_handle: AppHandle,
    mut receiver: mpsc::Receiver<BatchJob>,
    shutdown_token: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => {
                tracing::info!("Batch worker shutting down due to shutdown token");
                break;
            }
            job_opt = receiver.recv() => {
                match job_opt {
                    Some(job) => {
                        let _ = app_handle.emit("batch_job_status", JobStatus {
                            job_id: job.id.clone(),
                            status: "processing".into(),
                            error_msg: None,
                        });

                        // Simüle edilmiş işleme süresi
                        tokio::select! {
                            _ = sleep(Duration::from_millis(500)) => {
                                let _ = app_handle.emit("batch_job_status", JobStatus {
                                    job_id: job.id.clone(),
                                    status: "completed".into(),
                                    error_msg: None,
                                });
                            }
                            _ = shutdown_token.cancelled() => {
                                tracing::info!("Batch worker shutting down during job execution");
                                break;
                            }
                        }
                    }
                    None => {
                        tracing::info!("Batch worker channel closed");
                        break;
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub async fn add_batch_job(
    job_type: String,
    file_path: String,
    parameters: String,
    queue: State<'_, BatchQueue>,
) -> Result<String, String> {
    let job_id = uuid::Uuid::new_v4().to_string();
    let job = BatchJob {
        id: job_id.clone(),
        job_type,
        file_path,
        parameters,
    };

    queue
        .sender
        .send(job)
        .await
        .map_err(|e| format!("İş kuyruğa eklenemedi: {}", e))?;
    Ok(job_id)
}
