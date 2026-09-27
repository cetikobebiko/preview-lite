//! Plugin registry data structures, typed envelopes, and thread-safe in-memory state tracking.

use super::error::PluginErrorWire;
use super::manifest::{PluginHook, PluginPermission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledPluginInfoV1 {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    pub permissions: Vec<PluginPermission>,
    pub hooks: Vec<PluginHook>,
    pub main_script: String,
    pub enabled: bool,
    pub installed_at_rfc3339: String,
    pub install_dir: String,
    pub package_digest: String,
    pub manifest_digest: String,
    pub script_digest: String,
    pub reviewed_package_digest: String,
    pub reviewed_permissions: Vec<PluginPermission>,
    pub consent_granted: bool,
    pub consent_policy_version: u32,
}

pub type InstalledPluginInfo = InstalledPluginInfoV1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryEnvelopeV1 {
    pub schema_version: u32, // Strictly 1
    pub entries: Vec<InstalledPluginInfoV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHookRequestV1 {
    pub schema_version: u32, // Strictly 1
    pub execution_id: String,
    pub plugin_id: String,
    pub hook: PluginHook,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHookResultV1 {
    pub schema_version: u32, // Strictly 1
    pub execution_id: String,
    pub text: Option<String>,
    pub logs: Vec<String>,
    pub notifications: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionState {
    Pending,
    Running,
    Completed,
    Cancelled,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginExecutionStatus {
    pub execution_id: String,
    pub plugin_id: String,
    pub hook_name: String,
    pub state: ExecutionState,
    #[serde(default)]
    pub result: Option<PluginHookResultV1>,
    #[serde(default)]
    pub error: Option<PluginErrorWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginExecutionStartResult {
    pub execution_id: String,
    pub plugin_id: String,
    pub state: ExecutionState,
}

pub struct ActiveExecutionRecord {
    pub status: PluginExecutionStatus,
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Clone, Default)]
pub struct PluginExecutionRegistry {
    records: Arc<RwLock<HashMap<String, ActiveExecutionRecord>>>,
}

impl PluginExecutionRegistry {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Checks if a plugin already has an active (Pending/Running) execution.
    pub async fn is_plugin_busy(&self, plugin_id: &str) -> bool {
        let lock = self.records.read().await;
        lock.values().any(|rec| {
            rec.status.plugin_id == plugin_id
                && matches!(
                    rec.status.state,
                    ExecutionState::Pending | ExecutionState::Running
                )
        })
    }

    /// Registers a newly initiated execution.
    pub async fn register_execution(
        &self,
        execution_id: String,
        plugin_id: String,
        hook_name: String,
        cancel_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let mut lock = self.records.write().await;
        lock.insert(
            execution_id.clone(),
            ActiveExecutionRecord {
                status: PluginExecutionStatus {
                    execution_id,
                    plugin_id,
                    hook_name,
                    state: ExecutionState::Running,
                    result: None,
                    error: None,
                },
                cancel_tx: Some(cancel_tx),
            },
        );
    }

    /// Retrieves status of an execution.
    pub async fn get_status(&self, execution_id: &str) -> Option<PluginExecutionStatus> {
        let lock = self.records.read().await;
        lock.get(execution_id).map(|rec| rec.status.clone())
    }

    /// Marks an execution as completed with a successful structured result.
    pub async fn finish_success(&self, execution_id: &str, result: PluginHookResultV1) {
        let mut lock = self.records.write().await;
        if let Some(rec) = lock.get_mut(execution_id) {
            rec.status.state = ExecutionState::Completed;
            rec.status.result = Some(result);
            rec.cancel_tx = None;
        }
    }

    /// Marks an execution as failed with an error.
    pub async fn finish_error(
        &self,
        execution_id: &str,
        state: ExecutionState,
        error: PluginErrorWire,
    ) {
        let mut lock = self.records.write().await;
        if let Some(rec) = lock.get_mut(execution_id) {
            rec.status.state = state;
            rec.status.error = Some(error);
            rec.cancel_tx = None;
        }
    }

    /// Triggers cooperative cancellation on the execution.
    pub async fn cancel_execution(&self, execution_id: &str) -> bool {
        let mut lock = self.records.write().await;
        if let Some(rec) = lock.get_mut(execution_id) {
            if matches!(
                rec.status.state,
                ExecutionState::Pending | ExecutionState::Running
            ) {
                if let Some(tx) = rec.cancel_tx.take() {
                    let _ = tx.send(());
                }
                return true;
            }
        }
        false
    }
}
