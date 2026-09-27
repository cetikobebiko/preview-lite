use preview_alternative_lib::plugins::limits::*;
use preview_alternative_lib::plugins::registry::PluginHookResultV1;
use preview_alternative_lib::plugins::worker::{
    WorkerControlAction, WorkerControlMessage, WorkerExecuteRequest, WorkerExecuteResponse,
    WorkerTerminalStatus,
};
use std::io::{self, Read, Write};
use std::time::Duration;

fn write_framed_bytes(stdout: &mut dyn Write, bytes: &[u8]) -> io::Result<()> {
    let len = (bytes.len() as u32).to_be_bytes();
    stdout.write_all(&len)?;
    stdout.write_all(bytes)?;
    stdout.flush()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut mode = "valid-response".to_string();
    while let Some(arg) = args.next() {
        if arg == "--mode" {
            if let Some(m) = args.next() {
                mode = m;
            }
        } else if !arg.starts_with("--") {
            mode = arg;
        }
    }

    if mode == "stderr-flood" {
        let mut stderr = io::stderr().lock();
        for _ in 0..512 {
            let _ = stderr.write_all(&[b'x'; 8192]);
        }
        let _ = stderr.flush();
    }

    if mode == "exit-before-response" {
        std::process::exit(101);
    }

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    // Read 4-byte length prefix
    let mut len_buf = [0u8; 4];
    if stdin.read_exact(&mut len_buf).is_err() {
        return;
    }
    let req_len = u32::from_be_bytes(len_buf) as usize;
    if req_len > MAX_IPC_REQUEST_BYTES {
        return;
    }
    let mut req_bytes = vec![0u8; req_len];
    if stdin.read_exact(&mut req_bytes).is_err() {
        return;
    }

    let req: WorkerExecuteRequest = match serde_json::from_slice(&req_bytes) {
        Ok(r) => r,
        Err(_) => return,
    };

    match mode.as_str() {
        "valid-response" | "stderr-flood" => {
            let resp = WorkerExecuteResponse {
                protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
                request_id: req.request_id,
                execution_id: req.execution_id.clone(),
                status: WorkerTerminalStatus::Success,
                result: Some(PluginHookResultV1 {
                    schema_version: CURRENT_PLUGIN_RESULT_SCHEMA_VERSION,
                    execution_id: req.execution_id,
                    text: Some("helper test output".to_string()),
                    logs: vec!["helper log".to_string()],
                    notifications: vec![],
                    warnings: vec![],
                }),
                error: None,
            };
            let bytes = serde_json::to_vec(&resp).unwrap();
            let _ = write_framed_bytes(&mut stdout, &bytes);
        }
        "wrong-protocol-version" => {
            let resp = WorkerExecuteResponse {
                protocol_version: 999,
                request_id: req.request_id,
                execution_id: req.execution_id.clone(),
                status: WorkerTerminalStatus::Success,
                result: Some(PluginHookResultV1 {
                    schema_version: CURRENT_PLUGIN_RESULT_SCHEMA_VERSION,
                    execution_id: req.execution_id,
                    text: None,
                    logs: vec![],
                    notifications: vec![],
                    warnings: vec![],
                }),
                error: None,
            };
            let bytes = serde_json::to_vec(&resp).unwrap();
            let _ = write_framed_bytes(&mut stdout, &bytes);
        }
        "wrong-request-id" => {
            let resp = WorkerExecuteResponse {
                protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
                request_id: "wrong-request-id".to_string(),
                execution_id: req.execution_id.clone(),
                status: WorkerTerminalStatus::Success,
                result: Some(PluginHookResultV1 {
                    schema_version: CURRENT_PLUGIN_RESULT_SCHEMA_VERSION,
                    execution_id: req.execution_id,
                    text: None,
                    logs: vec![],
                    notifications: vec![],
                    warnings: vec![],
                }),
                error: None,
            };
            let bytes = serde_json::to_vec(&resp).unwrap();
            let _ = write_framed_bytes(&mut stdout, &bytes);
        }
        "wrong-execution-id" => {
            let resp = WorkerExecuteResponse {
                protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
                request_id: req.request_id,
                execution_id: "wrong-execution-id".to_string(),
                status: WorkerTerminalStatus::Success,
                result: Some(PluginHookResultV1 {
                    schema_version: CURRENT_PLUGIN_RESULT_SCHEMA_VERSION,
                    execution_id: "wrong-execution-id".to_string(),
                    text: None,
                    logs: vec![],
                    notifications: vec![],
                    warnings: vec![],
                }),
                error: None,
            };
            let bytes = serde_json::to_vec(&resp).unwrap();
            let _ = write_framed_bytes(&mut stdout, &bytes);
        }
        "malformed-length-prefix" => {
            // Write length prefix of 50 MB (> MAX_IPC_RESULT_BYTES)
            let len = (50 * 1024 * 1024u32).to_be_bytes();
            let _ = stdout.write_all(&len);
            let _ = stdout.flush();
        }
        "oversized-response" => {
            // Write length prefix > MAX_IPC_RESULT_BYTES (e.g. 2 MB) followed by data
            let size = 2 * 1024 * 1024;
            let len = (size as u32).to_be_bytes();
            let _ = stdout.write_all(&len);
            let _ = stdout.write_all(&vec![b'a'; size]);
            let _ = stdout.flush();
        }
        "truncated-frame" => {
            // Write length prefix of 100 bytes, but send only 10 bytes then close
            let len = 100u32.to_be_bytes();
            let _ = stdout.write_all(&len);
            let _ = stdout.write_all(&[0u8; 10]);
            let _ = stdout.flush();
        }
        "malformed-json" => {
            let bad_json = b"{not_valid_json:";
            let _ = write_framed_bytes(&mut stdout, bad_json);
        }
        "hang" => {
            std::thread::sleep(Duration::from_secs(60));
        }
        "duplicate-terminal-response" => {
            let resp = WorkerExecuteResponse {
                protocol_version: CURRENT_PLUGIN_PROTOCOL_VERSION,
                request_id: req.request_id.clone(),
                execution_id: req.execution_id.clone(),
                status: WorkerTerminalStatus::Success,
                result: Some(PluginHookResultV1 {
                    schema_version: CURRENT_PLUGIN_RESULT_SCHEMA_VERSION,
                    execution_id: req.execution_id.clone(),
                    text: Some("first response".to_string()),
                    logs: vec![],
                    notifications: vec![],
                    warnings: vec![],
                }),
                error: None,
            };
            let bytes = serde_json::to_vec(&resp).unwrap();
            let _ = write_framed_bytes(&mut stdout, &bytes);

            // Send immediate duplicate response
            let _ = write_framed_bytes(&mut stdout, &bytes);
            std::thread::sleep(Duration::from_millis(100));
        }
        "cancel-cooperative" => {
            // Read cancel control frame from stdin
            let mut ctrl_len_buf = [0u8; 4];
            if stdin.read_exact(&mut ctrl_len_buf).is_ok() {
                let ctrl_len = u32::from_be_bytes(ctrl_len_buf) as usize;
                if ctrl_len <= MAX_IPC_CONTROL_BYTES {
                    let mut ctrl_bytes = vec![0u8; ctrl_len];
                    if stdin.read_exact(&mut ctrl_bytes).is_ok() {
                        if let Ok(ctrl) =
                            serde_json::from_slice::<WorkerControlMessage>(&ctrl_bytes)
                        {
                            if ctrl.action == WorkerControlAction::Cancel {
                                // Exit cleanly upon cooperative cancel
                                std::process::exit(0);
                            }
                        }
                    }
                }
            }
        }
        "cancel-ignore" => {
            // Ignore cancel control messages and hang until hard killed
            std::thread::sleep(Duration::from_secs(60));
        }
        _ => {
            eprintln!("Unknown mode: {}", mode);
            std::process::exit(1);
        }
    }
}
