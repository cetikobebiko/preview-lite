use crate::ai::error::AiError;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub const MAX_CONCURRENT_REQUESTS: usize = 5;
pub const MAX_TERMINAL_TOMBSTONES: usize = 100;
pub const TOMBSTONE_TTL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestOwner {
    SessionNative {
        session_id: String,
        expected_content_generation: u64,
    },
    SelectionScoped {
        caller_attempt_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancellationCapability {
    StreamingWatch,
    SessionJob,
    NotCancellable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

pub struct ActiveRequestHandle {
    pub request_id: String,
    pub owner: RequestOwner,
    pub operation: String,
    pub created_at: Instant,
    pub cancel_tx: watch::Sender<bool>,
    pub capability: CancellationCapability,
}

impl ActiveRequestHandle {
    pub fn is_cancel_requested(&self) -> bool {
        *self.cancel_tx.borrow()
    }
}

#[derive(Clone, Debug)]
pub struct TerminalTombstone {
    pub request_id: String,
    pub state: RequestState,
    pub terminated_at: Instant,
}

struct AiRequestRegistryInner {
    active_requests: RwLock<HashMap<String, Arc<ActiveRequestHandle>>>,
    tombstones: RwLock<Vec<TerminalTombstone>>,
}

#[derive(Clone)]
pub struct AiRequestRegistry(Arc<AiRequestRegistryInner>);

pub struct RequestRegistryGuard {
    registry: AiRequestRegistry,
    request_id: String,
    disarmed: bool,
}

impl RequestRegistryGuard {
    pub fn finish(&mut self, state: RequestState) {
        if !self.disarmed {
            self.disarmed = true;
            self.registry.finish_request(&self.request_id, state);
        }
    }
}

impl Drop for RequestRegistryGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            self.disarmed = true;
            self.registry
                .finish_request(&self.request_id, RequestState::Failed);
        }
    }
}

impl AiRequestRegistry {
    pub fn new() -> Self {
        Self(Arc::new(AiRequestRegistryInner {
            active_requests: RwLock::new(HashMap::new()),
            tombstones: RwLock::new(Vec::new()),
        }))
    }

    pub fn register_request(
        &self,
        request_id: String,
        operation: String,
        owner: RequestOwner,
        capability: CancellationCapability,
    ) -> Result<
        (
            Arc<ActiveRequestHandle>,
            RequestRegistryGuard,
            watch::Receiver<bool>,
        ),
        AiError,
    > {
        let mut active = self.0.active_requests.write();
        if active.contains_key(&request_id) {
            return Err(AiError::RequestDuplicate(request_id));
        }
        if active.len() >= MAX_CONCURRENT_REQUESTS {
            return Err(AiError::RequestLimitExceeded(format!(
                "Max concurrent request limit {} reached",
                MAX_CONCURRENT_REQUESTS
            )));
        }

        let (tx, rx) = watch::channel(false);

        let handle = Arc::new(ActiveRequestHandle {
            request_id: request_id.clone(),
            owner,
            operation,
            created_at: Instant::now(),
            cancel_tx: tx,
            capability,
        });

        active.insert(request_id.clone(), handle.clone());

        let guard = RequestRegistryGuard {
            registry: self.clone(),
            request_id,
            disarmed: false,
        };

        Ok((handle, guard, rx))
    }

    pub fn finish_request(&self, request_id: &str, final_state: RequestState) {
        let mut active = self.0.active_requests.write();
        if active.remove(request_id).is_some() {
            let mut tombstones = self.0.tombstones.write();
            self.clean_stale_tombstones_locked(&mut tombstones);
            if tombstones.len() >= MAX_TERMINAL_TOMBSTONES {
                tombstones.remove(0);
            }
            tombstones.push(TerminalTombstone {
                request_id: request_id.to_string(),
                state: final_state,
                terminated_at: Instant::now(),
            });
        }
    }

    pub fn cancel_request(
        &self,
        request_id: &str,
        expected_owner: Option<&RequestOwner>,
    ) -> Result<(), AiError> {
        let active = self.0.active_requests.read();
        if let Some(handle) = active.get(request_id) {
            if let Some(exp) = expected_owner {
                if &handle.owner != exp {
                    return Err(AiError::RequestOwnerMismatch(
                        "Ownership validation failed".to_string(),
                    ));
                }
            }
            if handle.capability == CancellationCapability::NotCancellable {
                return Err(AiError::RequestNotCancellable(request_id.to_string()));
            }
            let _ = handle.cancel_tx.send(true);
            return Ok(());
        }

        let mut tombstones = self.0.tombstones.write();
        self.clean_stale_tombstones_locked(&mut tombstones);
        if tombstones.iter().any(|t| t.request_id == request_id) {
            return Err(AiError::RequestAlreadyTerminal(request_id.to_string()));
        }

        Err(AiError::RequestNotFound(request_id.to_string()))
    }

    pub fn cancel_all_for_endpoint_change(&self) {
        let active = self.0.active_requests.read();
        for handle in active.values() {
            if handle.capability != CancellationCapability::NotCancellable {
                let _ = handle.cancel_tx.send(true);
            }
        }
    }

    pub fn get_state(&self, request_id: &str) -> Option<RequestState> {
        let active = self.0.active_requests.read();
        if active.contains_key(request_id) {
            return Some(RequestState::Running);
        }
        drop(active);

        let mut tombstones = self.0.tombstones.write();
        self.clean_stale_tombstones_locked(&mut tombstones);
        tombstones
            .iter()
            .rev()
            .find(|t| t.request_id == request_id)
            .map(|t| t.state)
    }

    fn clean_stale_tombstones_locked(&self, tombstones: &mut Vec<TerminalTombstone>) {
        let now = Instant::now();
        tombstones.retain(|t| now.duration_since(t.terminated_at) < TOMBSTONE_TTL);
    }
}

impl Default for AiRequestRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_concurrency_and_cancellation() {
        let reg = AiRequestRegistry::new();
        let owner = RequestOwner::SessionNative {
            session_id: "s1".to_string(),
            expected_content_generation: 1,
        };

        let (_handle1, mut guard1, rx1) = reg
            .register_request(
                "req_1".to_string(),
                "chat".to_string(),
                owner.clone(),
                CancellationCapability::StreamingWatch,
            )
            .unwrap();

        assert!(!*rx1.borrow());

        // Duplicate
        assert!(matches!(
            reg.register_request(
                "req_1".to_string(),
                "chat".to_string(),
                owner.clone(),
                CancellationCapability::StreamingWatch
            ),
            Err(AiError::RequestDuplicate(_))
        ));

        // Cancel with matching owner
        assert!(reg.cancel_request("req_1", Some(&owner)).is_ok());
        assert!(*rx1.borrow());

        guard1.finish(RequestState::Cancelled);

        // Second cancel returns AlreadyTerminal
        assert!(matches!(
            reg.cancel_request("req_1", Some(&owner)),
            Err(AiError::RequestAlreadyTerminal(_))
        ));
    }

    #[test]
    fn test_guard_drop_failure_fallback() {
        let reg = AiRequestRegistry::new();
        let owner = RequestOwner::SelectionScoped {
            caller_attempt_id: "att_1".to_string(),
        };

        {
            let (_handle, _guard, _rx) = reg
                .register_request(
                    "req_drop".to_string(),
                    "summary".to_string(),
                    owner,
                    CancellationCapability::StreamingWatch,
                )
                .unwrap();
            // _guard dropped here without calling finish()
        }

        let tombstones = reg.0.tombstones.read();
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].state, RequestState::Failed);
    }

    #[test]
    fn test_not_cancellable_capability() {
        let reg = AiRequestRegistry::new();
        let owner = RequestOwner::SessionNative {
            session_id: "s1".to_string(),
            expected_content_generation: 1,
        };

        let (_handle, mut guard, _rx) = reg
            .register_request(
                "req_search".to_string(),
                "search".to_string(),
                owner.clone(),
                CancellationCapability::NotCancellable,
            )
            .unwrap();

        assert!(matches!(
            reg.cancel_request("req_search", Some(&owner)),
            Err(AiError::RequestNotCancellable(_))
        ));

        guard.finish(RequestState::Completed);
    }
}
