use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::subprocess_manager::WORKER_METRICS_TARGET;

pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_IDENTIFIER_LENGTH: usize = 128;
pub const MAX_PATH_LENGTH: usize = 4096;
pub const MAX_SCALE: f64 = 16.0;
pub const MAX_PIXEL_DIMENSION: u32 = 16_384;
pub const MAX_RENDER_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_CACHE_ENTRY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;

const MAX_CACHE_ENTRIES: usize = 256;
const MAX_DIMENSION_ENTRIES: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderScope {
    MainPage,
    SidebarThumbnail,
    OrganizerThumbnail,
}

impl RenderScope {
    fn is_main(self) -> bool {
        matches!(self, Self::MainPage)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MainPage => "main_page",
            Self::SidebarThumbnail => "sidebar_thumbnail",
            Self::OrganizerThumbnail => "organizer_thumbnail",
        }
    }
}

/// Why the frontend cancelled a render request. Diagnostic only: it is logged
/// to the worker-metrics target and never changes cancellation behavior.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderCancelReason {
    Superseded,
    ViewportExit,
    Zoom,
    Rerender,
    Timeout,
    Destroy,
    Unspecified,
}

impl RenderCancelReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::ViewportExit => "viewport_exit",
            Self::Zoom => "zoom",
            Self::Rerender => "rerender",
            Self::Timeout => "timeout",
            Self::Destroy => "destroy",
            Self::Unspecified => "unspecified",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveRenderDocument {
    pub document_id: String,
    pub path: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RenderRequestIdentity {
    pub document_id: String,
    pub path: String,
    pub generation: u64,
    pub page_index: u32,
    pub scope: RenderScope,
    pub request_id: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RenderSlot {
    document_id: String,
    generation: u64,
    page_index: u32,
    scope: RenderScope,
}

impl From<&RenderRequestIdentity> for RenderSlot {
    fn from(identity: &RenderRequestIdentity) -> Self {
        Self {
            document_id: identity.document_id.clone(),
            generation: identity.generation,
            page_index: identity.page_index,
            scope: identity.scope,
        }
    }
}

#[derive(Clone)]
struct ActiveRequest {
    path: String,
    request_id: String,
    token: CancellationToken,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RenderSizingKey {
    ScaleBits(u64),
    TargetWidth(u32),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RenderTargetKey {
    pub document_id: String,
    pub generation: u64,
    pub page_index: u32,
    pub sizing: RenderSizingKey,
    pub rotation: i32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RenderCacheKey {
    pub document_id: String,
    pub generation: u64,
    pub page_index: u32,
    pub target_width: u32,
    pub target_height: u32,
    pub rotation: i32,
}

/// RGBA pixels shared by the render cache and the responses built from it,
/// so a cache insert or hit is a reference-count bump instead of a full
/// pixel copy. `Arc<Vec<u8>>` rather than `Arc<[u8]>`: wrapping the worker's
/// `Vec` moves only its pointer, while `Arc<[u8]>::from(Vec)` copies.
pub type RenderPixels = Arc<Vec<u8>>;

#[derive(Clone, Debug)]
pub struct CachedRender {
    pub width: u32,
    pub height: u32,
    pub data: RenderPixels,
}

impl CachedRender {
    fn byte_len(&self) -> usize {
        self.data.len()
    }
}

/// Successful viewer render. Sent to the webview only as a binary
/// `render-frame/1` body (`render_wire.rs`), never as JSON.
#[derive(Clone, Debug)]
pub struct RenderResponse {
    pub width: u32,
    pub height: u32,
    pub data: RenderPixels,
    pub page_index: u32,
    pub generation: u64,
    pub request_id: String,
}

impl RenderResponse {
    fn from_cached(cached: CachedRender, identity: &RenderRequestIdentity) -> Self {
        Self {
            width: cached.width,
            height: cached.height,
            data: cached.data,
            page_index: identity.page_index,
            generation: identity.generation,
            request_id: identity.request_id.clone(),
        }
    }
}

struct ByteBudgetCache {
    entries: LruCache<RenderCacheKey, CachedRender>,
    current_bytes: usize,
    max_entry_bytes: usize,
    max_bytes: usize,
}

impl ByteBudgetCache {
    fn new() -> Self {
        Self::with_limits(MAX_CACHE_ENTRIES, MAX_CACHE_ENTRY_BYTES, MAX_CACHE_BYTES)
    }

    fn with_limits(capacity: usize, max_entry_bytes: usize, max_bytes: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).expect("render cache capacity is non-zero");
        Self {
            entries: LruCache::new(capacity),
            current_bytes: 0,
            max_entry_bytes,
            max_bytes,
        }
    }

    fn get(&mut self, key: &RenderCacheKey) -> Option<CachedRender> {
        self.entries.get(key).cloned()
    }

    fn put(&mut self, key: RenderCacheKey, value: CachedRender) -> bool {
        let value_bytes = value.byte_len();
        if value_bytes > self.max_entry_bytes || value_bytes > self.max_bytes {
            return false;
        }

        if let Some(replaced) = self.entries.pop(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(replaced.byte_len());
        }

        while self.current_bytes.saturating_add(value_bytes) > self.max_bytes {
            let Some((_evicted_key, evicted)) = self.entries.pop_lru() else {
                break;
            };
            self.current_bytes = self.current_bytes.saturating_sub(evicted.byte_len());
        }

        self.current_bytes = self.current_bytes.saturating_add(value_bytes);
        if let Some((_evicted_key, evicted)) = self.entries.push(key, value) {
            self.current_bytes = self.current_bytes.saturating_sub(evicted.byte_len());
        }
        true
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.current_bytes = 0;
    }
}

struct CoordinatorState {
    active_document: Option<ActiveRenderDocument>,
    active_requests: HashMap<RenderSlot, ActiveRequest>,
    dimensions: LruCache<RenderTargetKey, (u32, u32)>,
    cache: ByteBudgetCache,
}

impl CoordinatorState {
    fn new() -> Self {
        let dimension_capacity =
            NonZeroUsize::new(MAX_DIMENSION_ENTRIES).expect("dimension cache capacity is non-zero");
        Self {
            active_document: None,
            active_requests: HashMap::new(),
            dimensions: LruCache::new(dimension_capacity),
            cache: ByteBudgetCache::new(),
        }
    }

    fn document_matches(&self, identity: &RenderRequestIdentity) -> bool {
        self.active_document.as_ref().is_some_and(|active| {
            active.document_id == identity.document_id
                && active.path == identity.path
                && active.generation == identity.generation
        })
    }

    fn request_matches(&self, identity: &RenderRequestIdentity) -> bool {
        let slot = RenderSlot::from(identity);
        self.active_requests.get(&slot).is_some_and(|active| {
            active.path == identity.path && active.request_id == identity.request_id
        })
    }
}

#[derive(Default)]
struct SchedulerState {
    running: bool,
    next_ticket: u64,
    main_queue: VecDeque<u64>,
    thumbnail_queue: VecDeque<u64>,
}

pub struct PdfiumRenderScheduler {
    state: parking_lot::Mutex<SchedulerState>,
    notify: Notify,
}

impl PdfiumRenderScheduler {
    fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(SchedulerState::default()),
            notify: Notify::new(),
        }
    }

    async fn acquire(
        &self,
        scope: RenderScope,
        token: &CancellationToken,
    ) -> Result<PdfiumRenderPermit<'_>, String> {
        let ticket = {
            let mut state = self.state.lock();
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1);
            if scope.is_main() {
                state.main_queue.push_back(ticket);
            } else {
                state.thumbnail_queue.push_back(ticket);
            }
            ticket
        };
        let mut waiter = PdfiumWaiterGuard::new(self, scope, ticket);

        loop {
            if token.is_cancelled() {
                return Err("render_cancelled".to_string());
            }

            let notified = self.notify.notified();
            let acquired = {
                let mut state = self.state.lock();
                let queue_is_ready = if scope.is_main() {
                    state.main_queue.front() == Some(&ticket)
                } else {
                    state.main_queue.is_empty() && state.thumbnail_queue.front() == Some(&ticket)
                };

                if !state.running && queue_is_ready {
                    state.running = true;
                    if scope.is_main() {
                        state.main_queue.pop_front();
                    } else {
                        state.thumbnail_queue.pop_front();
                    }
                    true
                } else {
                    false
                }
            };

            if acquired {
                waiter.disarm();
                return Ok(PdfiumRenderPermit { scheduler: self });
            }

            tokio::select! {
                _ = token.cancelled() => {
                    tracing::debug!(
                        target: WORKER_METRICS_TARGET,
                        event = "render_cancel_observed",
                        phase = "scheduler_queue",
                        scope = scope.label(),
                    );
                    return Err("render_cancelled".to_string());
                }
                _ = notified => {}
            }
        }
    }

    fn remove_waiter(&self, scope: RenderScope, ticket: u64) {
        {
            let mut state = self.state.lock();
            let queue = if scope.is_main() {
                &mut state.main_queue
            } else {
                &mut state.thumbnail_queue
            };
            if let Some(position) = queue.iter().position(|queued| *queued == ticket) {
                queue.remove(position);
            }
        }
        self.notify.notify_waiters();
    }

    fn release(&self) {
        {
            let mut state = self.state.lock();
            state.running = false;
        }
        self.notify.notify_waiters();
    }

    #[cfg(test)]
    fn queued_waiter_count(&self) -> usize {
        let state = self.state.lock();
        state.main_queue.len() + state.thumbnail_queue.len()
    }
}

struct PdfiumWaiterGuard<'a> {
    scheduler: &'a PdfiumRenderScheduler,
    scope: RenderScope,
    ticket: u64,
    armed: bool,
}

impl<'a> PdfiumWaiterGuard<'a> {
    fn new(scheduler: &'a PdfiumRenderScheduler, scope: RenderScope, ticket: u64) -> Self {
        Self {
            scheduler,
            scope,
            ticket,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PdfiumWaiterGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.scheduler.remove_waiter(self.scope, self.ticket);
        }
    }
}

pub struct PdfiumRenderPermit<'a> {
    scheduler: &'a PdfiumRenderScheduler,
}

impl Drop for PdfiumRenderPermit<'_> {
    fn drop(&mut self) {
        self.scheduler.release();
    }
}

pub struct RenderService {
    state: parking_lot::Mutex<CoordinatorState>,
    scheduler: PdfiumRenderScheduler,
}

impl Default for RenderService {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderService {
    pub fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(CoordinatorState::new()),
            scheduler: PdfiumRenderScheduler::new(),
        }
    }

    // Lock invariants:
    // - `state` and scheduler locks are never held together.
    // - no synchronous lock is held across `.await`.
    // - PDFium work is run only after all service locks have been released.
    // - request cleanup removes a slot only when its request ID still matches.
    pub fn set_active_document(
        &self,
        document: Option<ActiveRenderDocument>,
    ) -> Result<bool, String> {
        if let Some(document) = &document {
            validate_document_fields(&document.document_id, &document.path, document.generation)?;
        }

        let tokens = {
            let mut state = self.state.lock();
            if state.active_document == document {
                return Ok(false);
            }

            let tokens = state
                .active_requests
                .values()
                .map(|request| request.token.clone())
                .collect::<Vec<_>>();
            state.active_document = document;
            state.active_requests.clear();
            state.dimensions.clear();
            state.cache.clear();
            tokens
        };

        tracing::debug!(
            target: WORKER_METRICS_TARGET,
            event = "render_cancel",
            trigger = "document_switch",
            cancelled_requests = tokens.len(),
        );
        for token in tokens {
            token.cancel();
        }
        Ok(true)
    }

    pub fn begin_request(
        &self,
        identity: RenderRequestIdentity,
    ) -> Result<RenderTaskGuard<'_>, String> {
        validate_request_identity(&identity)?;
        let token = CancellationToken::new();
        let slot = RenderSlot::from(&identity);

        let replaced = {
            let mut state = self.state.lock();
            if !state.document_matches(&identity) {
                return Err("render_inactive_document".to_string());
            }
            state.active_requests.insert(
                slot,
                ActiveRequest {
                    path: identity.path.clone(),
                    request_id: identity.request_id.clone(),
                    token: token.clone(),
                },
            )
        };

        if let Some(replaced) = replaced {
            tracing::debug!(
                target: WORKER_METRICS_TARGET,
                event = "render_cancel",
                trigger = "slot_replaced",
                scope = identity.scope.label(),
            );
            replaced.token.cancel();
        }

        Ok(RenderTaskGuard {
            service: self,
            identity,
            token,
        })
    }

    pub fn cancel_request(&self, identity: &RenderRequestIdentity) -> Result<bool, String> {
        validate_request_identity(identity)?;
        let token = {
            let mut state = self.state.lock();
            if !state.document_matches(identity) {
                return Ok(false);
            }
            let slot = RenderSlot::from(identity);
            let matches = state.active_requests.get(&slot).is_some_and(|active| {
                active.path == identity.path && active.request_id == identity.request_id
            });
            matches
                .then(|| state.active_requests.remove(&slot))
                .flatten()
                .map(|active| active.token)
        };

        if let Some(token) = token {
            token.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn cancel_all(&self) {
        let tokens = {
            let mut state = self.state.lock();
            let tokens = state
                .active_requests
                .values()
                .map(|request| request.token.clone())
                .collect::<Vec<_>>();
            state.active_requests.clear();
            state.dimensions.clear();
            state.cache.clear();
            tokens
        };
        for token in tokens {
            token.cancel();
        }
    }

    pub async fn acquire(
        &self,
        scope: RenderScope,
        token: &CancellationToken,
    ) -> Result<PdfiumRenderPermit<'_>, String> {
        self.scheduler.acquire(scope, token).await
    }

    pub fn is_current(&self, identity: &RenderRequestIdentity) -> bool {
        let state = self.state.lock();
        state.document_matches(identity) && state.request_matches(identity)
    }

    pub fn dimensions(&self, key: &RenderTargetKey) -> Option<(u32, u32)> {
        self.state.lock().dimensions.get(key).copied()
    }

    pub fn record_dimensions(
        &self,
        identity: &RenderRequestIdentity,
        key: RenderTargetKey,
        dimensions: (u32, u32),
    ) -> bool {
        let mut state = self.state.lock();
        if !state.document_matches(identity) || !state.request_matches(identity) {
            return false;
        }
        state.dimensions.put(key, dimensions);
        true
    }

    pub fn cached_response(
        &self,
        identity: &RenderRequestIdentity,
        key: &RenderCacheKey,
    ) -> Option<RenderResponse> {
        let mut state = self.state.lock();
        if !state.document_matches(identity) || !state.request_matches(identity) {
            return None;
        }
        state
            .cache
            .get(key)
            .map(|cached| RenderResponse::from_cached(cached, identity))
    }

    pub fn cache_response(
        &self,
        identity: &RenderRequestIdentity,
        key: RenderCacheKey,
        rendered: CachedRender,
    ) -> bool {
        let mut state = self.state.lock();
        if !state.document_matches(identity) || !state.request_matches(identity) {
            return false;
        }
        state.cache.put(key, rendered)
    }

    #[cfg(test)]
    pub(crate) fn cached_entry_count(&self) -> usize {
        self.state.lock().cache.entries.len()
    }

    fn cleanup_request(&self, identity: &RenderRequestIdentity) {
        let mut state = self.state.lock();
        let slot = RenderSlot::from(identity);
        let should_remove = state
            .active_requests
            .get(&slot)
            .is_some_and(|active| active.request_id == identity.request_id);
        if should_remove {
            state.active_requests.remove(&slot);
        }
    }
}

pub struct RenderTaskGuard<'a> {
    service: &'a RenderService,
    identity: RenderRequestIdentity,
    token: CancellationToken,
}

impl RenderTaskGuard<'_> {
    pub fn identity(&self) -> &RenderRequestIdentity {
        &self.identity
    }

    pub fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl Drop for RenderTaskGuard<'_> {
    fn drop(&mut self) {
        self.service.cleanup_request(&self.identity);
    }
}

pub fn checked_rgba_len(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > MAX_PIXEL_DIMENSION || height > MAX_PIXEL_DIMENSION {
        return Err("render_invalid_dimensions".to_string());
    }

    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "render_size_overflow".to_string())?;
    let bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| "render_size_overflow".to_string())?;
    if bytes > MAX_RENDER_BYTES {
        return Err("render_too_large".to_string());
    }
    Ok(bytes)
}

pub fn validate_document_fields(
    document_id: &str,
    path: &str,
    generation: u64,
) -> Result<(), String> {
    if document_id.is_empty() || document_id.chars().count() > MAX_IDENTIFIER_LENGTH {
        return Err("render_invalid_document_id".to_string());
    }
    if path.is_empty() || path.chars().count() > MAX_PATH_LENGTH {
        return Err("render_invalid_path".to_string());
    }
    validate_generation(generation)
}

pub fn validate_generation(generation: u64) -> Result<(), String> {
    if generation > MAX_SAFE_INTEGER {
        return Err("render_invalid_generation".to_string());
    }
    Ok(())
}

pub fn validate_request_identity(identity: &RenderRequestIdentity) -> Result<(), String> {
    validate_document_fields(&identity.document_id, &identity.path, identity.generation)?;
    if identity.request_id.is_empty() || identity.request_id.chars().count() > MAX_IDENTIFIER_LENGTH
    {
        return Err("render_invalid_request_id".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn document(id: &str, generation: u64) -> ActiveRenderDocument {
        ActiveRenderDocument {
            document_id: id.to_string(),
            path: format!("/{id}.pdf"),
            generation,
        }
    }

    fn identity(
        document_id: &str,
        generation: u64,
        page_index: u32,
        scope: RenderScope,
        request_id: &str,
    ) -> RenderRequestIdentity {
        RenderRequestIdentity {
            document_id: document_id.to_string(),
            path: format!("/{document_id}.pdf"),
            generation,
            page_index,
            scope,
            request_id: request_id.to_string(),
        }
    }

    fn cache_key(document_id: &str, generation: u64, page_index: u32) -> RenderCacheKey {
        RenderCacheKey {
            document_id: document_id.to_string(),
            generation,
            page_index,
            target_width: 10,
            target_height: 10,
            rotation: 0,
        }
    }

    fn cached(byte_count: usize) -> CachedRender {
        CachedRender {
            width: 1,
            height: (byte_count / 4) as u32,
            data: Arc::new(vec![255; byte_count]),
        }
    }

    #[test]
    fn same_slot_supersedes_but_other_page_and_scope_are_isolated() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");

        let first = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "old"))
            .expect("first request");
        let other_page = service
            .begin_request(identity("doc", 1, 1, RenderScope::MainPage, "other"))
            .expect("other page");
        let thumbnail = service
            .begin_request(identity(
                "doc",
                1,
                0,
                RenderScope::SidebarThumbnail,
                "thumb",
            ))
            .expect("thumbnail");
        let replacement = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "new"))
            .expect("replacement");

        assert!(first.token().is_cancelled());
        assert!(!other_page.token().is_cancelled());
        assert!(!thumbnail.token().is_cancelled());
        assert!(!replacement.token().is_cancelled());
    }

    #[test]
    fn cleanup_and_delayed_cancel_cannot_remove_or_cancel_replacement() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");
        let old_identity = identity("doc", 1, 0, RenderScope::MainPage, "old");
        let old = service
            .begin_request(old_identity.clone())
            .expect("old request");
        let new_identity = identity("doc", 1, 0, RenderScope::MainPage, "new");
        let new = service
            .begin_request(new_identity.clone())
            .expect("new request");

        drop(old);
        assert!(service.is_current(&new_identity));
        assert!(!service.cancel_request(&old_identity).expect("old cancel"));
        assert!(!new.token().is_cancelled());
    }

    #[test]
    fn exact_cancel_is_idempotent_and_cannot_cancel_a_later_request() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");
        let request_identity = identity("doc", 1, 0, RenderScope::MainPage, "request");
        let request = service
            .begin_request(request_identity.clone())
            .expect("request");

        assert!(service
            .cancel_request(&request_identity)
            .expect("first cancel"));
        assert!(request.token().is_cancelled());
        assert!(!service
            .cancel_request(&request_identity)
            .expect("second cancel"));

        let replacement_identity = identity("doc", 1, 0, RenderScope::MainPage, "replacement");
        let replacement = service
            .begin_request(replacement_identity.clone())
            .expect("replacement");
        assert!(!service
            .cancel_request(&request_identity)
            .expect("delayed cancel"));
        assert!(!replacement.token().is_cancelled());
        assert!(service.is_current(&replacement_identity));
    }

    #[test]
    fn cancellation_with_wrong_path_cannot_cancel_active_request() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");
        let request_identity = identity("doc", 1, 0, RenderScope::MainPage, "request");
        let request = service
            .begin_request(request_identity.clone())
            .expect("request");
        let mut wrong_path = request_identity.clone();
        wrong_path.path = "/different.pdf".to_string();

        assert!(!service
            .cancel_request(&wrong_path)
            .expect("wrong-path cancellation"));
        assert!(!request.token().is_cancelled());
        assert!(service.is_current(&request_identity));
    }

    #[test]
    fn activation_cancels_requests_and_clears_cache() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");
        let request = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "one"))
            .expect("request");
        assert!(service.cache_response(request.identity(), cache_key("doc", 1, 0), cached(400)));

        assert!(service
            .set_active_document(Some(document("next", 2)))
            .expect("next activation"));
        assert!(request.token().is_cancelled());
        assert!(service
            .cached_response(request.identity(), &cache_key("doc", 1, 0))
            .is_none());
        assert!(!service
            .set_active_document(Some(document("next", 2)))
            .expect("idempotent activation"));
    }

    #[test]
    fn cache_hit_stamps_current_request_metadata_and_oversized_entry_is_skipped() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");
        let first = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "first"))
            .expect("first");
        let key = cache_key("doc", 1, 0);
        assert!(service.cache_response(first.identity(), key.clone(), cached(400)));
        drop(first);

        let second = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "second"))
            .expect("second");
        let response = service
            .cached_response(second.identity(), &key)
            .expect("cache hit");
        assert_eq!(response.request_id, "second");
        assert_eq!(response.generation, 1);
        assert!(!service.cache_response(
            second.identity(),
            RenderCacheKey {
                page_index: 1,
                ..key
            },
            cached(MAX_CACHE_ENTRY_BYTES + 4),
        ));
    }

    #[test]
    fn byte_budget_evicts_least_recently_used_entry() {
        let mut cache = ByteBudgetCache::with_limits(4, 12, 16);
        let first = cache_key("doc", 1, 0);
        let second = cache_key("doc", 1, 1);
        let third = cache_key("doc", 1, 2);

        assert!(cache.put(first.clone(), cached(8)));
        assert!(cache.put(second.clone(), cached(8)));
        assert!(cache.get(&first).is_some());
        assert!(cache.put(third.clone(), cached(8)));

        assert!(cache.get(&first).is_some());
        assert!(cache.get(&second).is_none());
        assert!(cache.get(&third).is_some());
        assert_eq!(cache.current_bytes, 16);
    }

    #[test]
    fn entry_capacity_eviction_updates_byte_accounting() {
        let mut cache = ByteBudgetCache::with_limits(2, 32, 128);
        let first = cache_key("doc", 1, 0);
        let second = cache_key("doc", 1, 1);
        let third = cache_key("doc", 1, 2);

        assert!(cache.put(first.clone(), cached(8)));
        assert!(cache.put(second.clone(), cached(12)));
        assert_eq!(cache.current_bytes, 20);
        assert!(cache.put(third.clone(), cached(16)));

        assert!(cache.get(&first).is_none());
        assert!(cache.get(&second).is_some());
        assert!(cache.get(&third).is_some());
        assert_eq!(cache.current_bytes, 28);
    }

    #[test]
    fn cache_key_isolates_document_generation_page_size_and_rotation() {
        let base = cache_key("doc", 1, 0);
        let variants = [
            RenderCacheKey {
                document_id: "other".to_string(),
                ..base.clone()
            },
            RenderCacheKey {
                generation: 2,
                ..base.clone()
            },
            RenderCacheKey {
                page_index: 1,
                ..base.clone()
            },
            RenderCacheKey {
                target_width: 11,
                ..base.clone()
            },
            RenderCacheKey {
                target_height: 11,
                ..base.clone()
            },
            RenderCacheKey {
                rotation: 90,
                ..base.clone()
            },
        ];

        for variant in variants {
            assert_ne!(base, variant);
        }
    }

    #[test]
    fn cancelled_or_superseded_request_cannot_populate_cache() {
        let service = RenderService::new();
        service
            .set_active_document(Some(document("doc", 1)))
            .expect("activation");
        let old = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "old"))
            .expect("old request");
        let replacement = service
            .begin_request(identity("doc", 1, 0, RenderScope::MainPage, "new"))
            .expect("replacement");

        assert!(old.token().is_cancelled());
        assert!(!service.cache_response(old.identity(), cache_key("doc", 1, 0), cached(400),));
        assert!(service.cache_response(
            replacement.identity(),
            cache_key("doc", 1, 0),
            cached(400),
        ));
    }

    #[test]
    fn validates_safe_generation_identifier_lengths_and_rgba_bounds() {
        assert!(validate_generation(MAX_SAFE_INTEGER).is_ok());
        assert_eq!(
            validate_generation(MAX_SAFE_INTEGER + 1),
            Err("render_invalid_generation".to_string())
        );
        assert!(validate_document_fields(&"ü".repeat(128), "/doc.pdf", 1).is_ok());
        assert_eq!(
            validate_document_fields(&"x".repeat(129), "/doc.pdf", 1),
            Err("render_invalid_document_id".to_string())
        );

        let mut request = identity("doc", 1, 0, RenderScope::MainPage, &"r".repeat(129));
        assert_eq!(
            validate_request_identity(&request),
            Err("render_invalid_request_id".to_string())
        );
        request.request_id = "request".to_string();
        assert!(validate_request_identity(&request).is_ok());
        assert_eq!(
            checked_rgba_len(0, 1),
            Err("render_invalid_dimensions".to_string())
        );
        assert_eq!(
            checked_rgba_len(MAX_PIXEL_DIMENSION, MAX_PIXEL_DIMENSION),
            Err("render_too_large".to_string())
        );
    }

    #[tokio::test]
    async fn main_request_runs_before_queued_thumbnails() {
        let scheduler = Arc::new(PdfiumRenderScheduler::new());
        let blocker_token = CancellationToken::new();
        let blocker = scheduler
            .acquire(RenderScope::SidebarThumbnail, &blocker_token)
            .await
            .expect("blocker");

        let order = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let thumb_scheduler = scheduler.clone();
        let thumb_order = order.clone();
        let thumbnail = tokio::spawn(async move {
            let token = CancellationToken::new();
            let _permit = thumb_scheduler
                .acquire(RenderScope::OrganizerThumbnail, &token)
                .await
                .expect("thumbnail permit");
            thumb_order.lock().push("thumbnail");
        });

        tokio::time::sleep(Duration::from_millis(10)).await;

        let main_scheduler = scheduler.clone();
        let main_order = order.clone();
        let main = tokio::spawn(async move {
            let token = CancellationToken::new();
            let _permit = main_scheduler
                .acquire(RenderScope::MainPage, &token)
                .await
                .expect("main permit");
            main_order.lock().push("main");
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        drop(blocker);
        main.await.expect("main join");
        thumbnail.await.expect("thumbnail join");
        assert_eq!(&*order.lock(), &["main", "thumbnail"]);
    }

    #[tokio::test]
    async fn cancelled_thumbnail_leaves_queue_before_execution() {
        let scheduler = Arc::new(PdfiumRenderScheduler::new());
        let blocker_token = CancellationToken::new();
        let blocker = scheduler
            .acquire(RenderScope::MainPage, &blocker_token)
            .await
            .expect("blocker");

        let token = CancellationToken::new();
        let queued_token = token.clone();
        let queued_scheduler = scheduler.clone();
        let queued = tokio::spawn(async move {
            queued_scheduler
                .acquire(RenderScope::SidebarThumbnail, &queued_token)
                .await
                .map(|_permit| ())
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        token.cancel();
        assert_eq!(
            queued.await.expect("queued join"),
            Err("render_cancelled".to_string())
        );
        drop(blocker);
    }

    #[tokio::test]
    async fn aborted_waiter_is_removed_and_next_request_progresses() {
        for aborted_scope in [RenderScope::SidebarThumbnail, RenderScope::MainPage] {
            let scheduler = Arc::new(PdfiumRenderScheduler::new());
            let blocker_token = CancellationToken::new();
            let blocker = scheduler
                .acquire(RenderScope::MainPage, &blocker_token)
                .await
                .expect("blocker");

            let aborted_scheduler = scheduler.clone();
            let aborted = tokio::spawn(async move {
                let token = CancellationToken::new();
                let _permit = aborted_scheduler
                    .acquire(aborted_scope, &token)
                    .await
                    .expect("aborted waiter must not acquire");
            });
            tokio::time::sleep(Duration::from_millis(10)).await;

            let next_scheduler = scheduler.clone();
            let next = tokio::spawn(async move {
                let token = CancellationToken::new();
                let _permit = next_scheduler
                    .acquire(aborted_scope, &token)
                    .await
                    .expect("next waiter permit");
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(scheduler.queued_waiter_count(), 2);

            aborted.abort();
            assert!(aborted.await.expect_err("aborted join").is_cancelled());
            assert_eq!(scheduler.queued_waiter_count(), 1);

            drop(blocker);
            tokio::time::timeout(Duration::from_secs(1), next)
                .await
                .expect("next waiter timed out")
                .expect("next waiter join");
            assert_eq!(scheduler.queued_waiter_count(), 0);
        }
    }
}
