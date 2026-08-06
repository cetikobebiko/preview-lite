pub mod render_service;
pub mod subprocess_manager;

use crate::{PasswordCache, RenderedPage};
use pdfium_render::prelude::*;
use render_service::{
    checked_rgba_len, validate_generation, ActiveRenderDocument, CachedRender, RenderCacheKey,
    RenderRequestIdentity, RenderResponse, RenderScope, RenderService, RenderSizingKey,
    RenderTargetKey, MAX_CACHE_ENTRY_BYTES, MAX_PIXEL_DIMENSION, MAX_SCALE,
};
use serde::Serialize;
use std::future::Future;
use tauri::State;
use tokio_util::sync::CancellationToken;

static PDFIUM_MUTEX: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub(crate) fn with_pdfium<T, E>(operation: impl FnOnce(&Pdfium) -> Result<T, E>) -> Result<T, E>
where
    E: From<String>,
{
    let _guard = PDFIUM_MUTEX.lock();
    let pdfium = get_pdfium().map_err(E::from)?;
    operation(&pdfium)
}

pub fn get_pdfium() -> Result<Pdfium, String> {
    let mut bind_attempts = Vec::new();

    if let Ok(path) = std::env::var("PDFIUM_PATH") {
        bind_attempts.push(Pdfium::bind_to_library(
            Pdfium::pdfium_platform_library_name_at_path(&path),
        ));
    }

    bind_attempts.push(Pdfium::bind_to_library(
        Pdfium::pdfium_platform_library_name_at_path("./src-tauri/lib"),
    ));
    bind_attempts.push(Pdfium::bind_to_library(
        Pdfium::pdfium_platform_library_name_at_path("./lib"),
    ));
    bind_attempts.push(Pdfium::bind_to_library(
        Pdfium::pdfium_platform_library_name_at_path("./"),
    ));
    bind_attempts.push(Pdfium::bind_to_system_library());

    let success_bindings = bind_attempts.into_iter().flatten().next();

    match success_bindings {
        Some(b) => Ok(Pdfium::new(b)),
        None => Err("PDFium kütüphanesi (pdfium.dll, libpdfium.dylib, libpdfium.so) bulunamadı veya yüklenemedi.".to_string()),
    }
}

pub fn get_cached_password(path: &str, cache: &PasswordCache) -> Option<String> {
    let lock = cache.0.lock();
    lock.get(path).cloned()
}

pub fn open_pdfium_doc<'a>(
    pdfium: &'a Pdfium,
    path: &str,
    password: Option<&'a str>,
) -> Result<PdfDocument<'a>, String> {
    let _ffi_span = tracing::info_span!(target: "pdfium_ffi", "pdfium_load_pdf").entered();
    pdfium.load_pdf_from_file(path, password).map_err(|e| {
        crate::error::AppError::Pdfium(format!("PDF açılırken hata oluştu: {:?}", e)).to_string()
    })
}

pub(crate) async fn render_page_rgba(
    path: String,
    page_index: u32,
    scale: f64,
    rotation: i32,
    password: Option<String>,
) -> Result<RenderedPage, String> {
    if scale.is_nan() || scale.is_infinite() || scale <= 0.0 {
        return Err("Geçersiz ölçek (scale) değeri".to_string());
    }

    let pdf_rotation = match rotation {
        0 => PdfPageRenderRotation::None,
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => return Err(format!("Geçersiz rotasyon (rotation) değeri: {}", rotation)),
    };

    tauri::async_runtime::spawn_blocking(move || {
        let span = tracing::Span::current();
        let _enter = span.enter();
        let _guard = PDFIUM_MUTEX.lock();
        let pdfium = get_pdfium()?;
        let document = open_pdfium_doc(&pdfium, &path, password.as_deref())?;

        let pages = document.pages();
        if page_index >= pages.len() as u32 {
            return Err(format!("Geçersiz sayfa numarası: {}", page_index));
        }

        let page = pages.get(page_index as u16).map_err(|e| {
            crate::error::AppError::Pdfium(format!("Sayfa alınamadı: {:?}", e)).to_string()
        })?;

        let target_width = (page.width().value as f64 * scale) as i32;

        let mut render_config = PdfRenderConfig::new();
        render_config = render_config.set_target_width(target_width);
        render_config = render_config.rotate(pdf_rotation, true);
        render_config = render_config.set_clear_color(PdfColor::WHITE);

        let bitmap = page.render_with_config(&render_config).map_err(|e| {
            crate::error::AppError::Image(format!("Sayfa render etme hatası: {:?}", e)).to_string()
        })?;

        let bgra_data = bitmap.as_raw_bytes();
        let mut rgba_data = vec![0; bgra_data.len()];
        for i in (0..bgra_data.len()).step_by(4) {
            rgba_data[i] = bgra_data[i + 2]; // R
            rgba_data[i + 1] = bgra_data[i + 1]; // G
            rgba_data[i + 2] = bgra_data[i]; // B
            rgba_data[i + 3] = bgra_data[i + 3]; // A
        }

        Ok(RenderedPage {
            width: bitmap.width() as u32,
            height: bitmap.height() as u32,
            data: rgba_data,
        })
    })
    .await
    .map_err(|e| crate::error::AppError::Thread(format!("Thread hatası: {}", e)).to_string())?
}

#[derive(Clone, Copy)]
pub(crate) enum ViewerRenderSizing {
    Scale(f64),
    TargetWidth(u32),
}

impl ViewerRenderSizing {
    fn key(self) -> RenderSizingKey {
        match self {
            Self::Scale(scale) => RenderSizingKey::ScaleBits(scale.to_bits()),
            Self::TargetWidth(width) => RenderSizingKey::TargetWidth(width),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderDocumentChange {
    changed: bool,
}

#[derive(Serialize)]
pub struct RenderCancellationResult {
    cancelled: bool,
}

#[tauri::command]
pub fn set_active_render_document(
    document_id: Option<String>,
    path: Option<String>,
    generation: u64,
    render_service: State<'_, RenderService>,
) -> Result<RenderDocumentChange, String> {
    validate_generation(generation)?;
    let document = match (document_id, path) {
        (Some(document_id), Some(path)) => Some(ActiveRenderDocument {
            document_id,
            path,
            generation,
        }),
        (None, None) => None,
        _ => return Err("render_invalid_document_activation".to_string()),
    };
    let changed = render_service.set_active_document(document)?;
    Ok(RenderDocumentChange { changed })
}

#[tauri::command]
pub fn cancel_render_request(
    path: String,
    document_id: String,
    page_index: u32,
    generation: u64,
    request_id: String,
    scope: RenderScope,
    render_service: State<'_, RenderService>,
) -> Result<RenderCancellationResult, String> {
    let identity = RenderRequestIdentity {
        document_id,
        path,
        generation,
        page_index,
        scope,
        request_id,
    };
    let cancelled = render_service.cancel_request(&identity)?;
    Ok(RenderCancellationResult { cancelled })
}

pub(crate) async fn render_viewer_request_with_renderer<Renderer, RenderFuture>(
    identity: RenderRequestIdentity,
    sizing: ViewerRenderSizing,
    rotation: i32,
    password_cache: &PasswordCache,
    render_service: &RenderService,
    renderer: Renderer,
) -> Result<RenderResponse, String>
where
    Renderer: FnOnce(
        String,
        u32,
        ViewerRenderSizing,
        i32,
        Option<String>,
        CancellationToken,
    ) -> RenderFuture,
    RenderFuture: Future<Output = Result<RenderedPage, String>>,
{
    let total_started = std::time::Instant::now();
    let task = render_service.begin_request(identity)?;
    let target_key = RenderTargetKey {
        document_id: task.identity().document_id.clone(),
        generation: task.identity().generation,
        page_index: task.identity().page_index,
        sizing: sizing.key(),
        rotation,
    };

    if let Some((width, height)) = render_service.dimensions(&target_key) {
        let cache_key = cache_key(task.identity(), width, height, rotation);
        let _cache_span = tracing::info_span!("render_cache_lookup").entered();
        if let Some(response) = render_service.cached_response(task.identity(), &cache_key) {
            tracing::debug!(
                total_ms = total_started.elapsed().as_secs_f64() * 1000.0,
                cache_hit = true,
                "render completed"
            );
            return Ok(response);
        }
    }

    let scheduler_started = std::time::Instant::now();
    let _permit = render_service
        .acquire(task.identity().scope, task.token())
        .await?;
    tracing::debug!(
        scheduler_wait_ms = scheduler_started.elapsed().as_secs_f64() * 1000.0,
        "render scheduler permit acquired"
    );

    if task.token().is_cancelled() || !render_service.is_current(task.identity()) {
        return Err("render_cancelled".to_string());
    }

    let password = get_cached_password(&task.identity().path, password_cache);
    let render_result = renderer(
        task.identity().path.clone(),
        task.identity().page_index,
        sizing,
        rotation,
        password,
        task.token().clone(),
    )
    .await;

    if task.token().is_cancelled() || !render_service.is_current(task.identity()) {
        return Err("render_cancelled".to_string());
    }
    let rendered = render_result?;

    let expected = checked_rgba_len(rendered.width, rendered.height)?;
    if rendered.data.len() != expected {
        return Err("render_invalid_rgba_payload".to_string());
    }

    render_service.record_dimensions(
        task.identity(),
        target_key,
        (rendered.width, rendered.height),
    );
    let cache_key = cache_key(task.identity(), rendered.width, rendered.height, rotation);
    if rendered.data.len() <= MAX_CACHE_ENTRY_BYTES {
        render_service.cache_response(
            task.identity(),
            cache_key,
            CachedRender {
                width: rendered.width,
                height: rendered.height,
                data: rendered.data.clone(),
            },
        );
    }

    let response = RenderResponse {
        width: rendered.width,
        height: rendered.height,
        data: rendered.data,
        page_index: task.identity().page_index,
        generation: task.identity().generation,
        request_id: task.identity().request_id.clone(),
    };
    tracing::debug!(
        total_ms = total_started.elapsed().as_secs_f64() * 1000.0,
        cache_hit = false,
        "render completed"
    );
    Ok(response)
}

fn cache_key(
    identity: &RenderRequestIdentity,
    width: u32,
    height: u32,
    rotation: i32,
) -> RenderCacheKey {
    RenderCacheKey {
        document_id: identity.document_id.clone(),
        generation: identity.generation,
        page_index: identity.page_index,
        target_width: width,
        target_height: height,
        rotation,
    }
}

pub(crate) fn validate_scale(scale: f64) -> Result<(), String> {
    if !scale.is_finite() || scale <= 0.0 || scale > MAX_SCALE {
        return Err("render_invalid_scale".to_string());
    }
    Ok(())
}

pub(crate) fn validate_target_width(target_width: u32) -> Result<(), String> {
    if target_width == 0 || target_width > MAX_PIXEL_DIMENSION {
        return Err("render_invalid_target_width".to_string());
    }
    Ok(())
}

pub(crate) fn validate_rotation(rotation: i32) -> Result<(), String> {
    pdf_rotation(rotation).map(|_| ())
}

#[cfg(test)]
mod canonical_render_validation_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn scale_target_width_and_rotation_limits_match_frontend_contracts() {
        assert!(validate_scale(MAX_SCALE).is_ok());
        assert!(validate_scale(0.0).is_err());
        assert!(validate_scale(f64::NAN).is_err());
        assert!(validate_scale(MAX_SCALE + 0.01).is_err());
        assert!(validate_target_width(1).is_ok());
        assert!(validate_target_width(MAX_PIXEL_DIMENSION).is_ok());
        assert!(validate_target_width(0).is_err());
        assert!(validate_target_width(MAX_PIXEL_DIMENSION + 1).is_err());
        for rotation in [0, 90, 180, 270] {
            assert!(validate_rotation(rotation).is_ok());
        }
        assert!(validate_rotation(45).is_err());
    }

    fn active_document(generation: u64) -> ActiveRenderDocument {
        ActiveRenderDocument {
            document_id: "doc".to_string(),
            path: "/doc.pdf".to_string(),
            generation,
        }
    }

    fn request(generation: u64, request_id: &str) -> RenderRequestIdentity {
        RenderRequestIdentity {
            document_id: "doc".to_string(),
            path: "/doc.pdf".to_string(),
            generation,
            page_index: 0,
            scope: RenderScope::MainPage,
            request_id: request_id.to_string(),
        }
    }

    fn mock_rendered_page() -> RenderedPage {
        RenderedPage {
            width: 2,
            height: 2,
            data: vec![255; 16],
        }
    }

    fn password_cache() -> PasswordCache {
        PasswordCache(std::sync::Arc::new(parking_lot::Mutex::new(HashMap::new())))
    }

    #[tokio::test]
    async fn orchestration_discards_render_when_generation_changes_during_renderer() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate generation one");
        let cache = password_cache();

        let result = render_viewer_request_with_renderer(
            request(1, "generation-one"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                service
                    .set_active_document(Some(active_document(2)))
                    .expect("activate generation two");
                async { Ok(mock_rendered_page()) }
            },
        )
        .await;

        assert_eq!(
            result.expect_err("generation change must cancel"),
            "render_cancelled"
        );
        assert_eq!(service.cached_entry_count(), 0);
    }

    #[tokio::test]
    async fn orchestration_discards_cancelled_result_before_renderer_returns() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();
        let identity = request(1, "cancel-before-return");
        let cancellation_identity = identity.clone();

        let result = render_viewer_request_with_renderer(
            identity,
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                assert!(service
                    .cancel_request(&cancellation_identity)
                    .expect("cancel request"));
                async { Ok(mock_rendered_page()) }
            },
        )
        .await;

        assert_eq!(
            result.expect_err("cancelled render must be discarded"),
            "render_cancelled"
        );
        assert_eq!(service.cached_entry_count(), 0);
    }

    #[tokio::test]
    async fn orchestration_cache_hit_uses_current_request_metadata() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();

        let first = render_viewer_request_with_renderer(
            request(1, "first"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| async { Ok(mock_rendered_page()) },
        )
        .await
        .expect("first render");
        assert_eq!(first.request_id, "first");
        assert_eq!(service.cached_entry_count(), 1);

        let renderer_calls = AtomicUsize::new(0);
        let second = render_viewer_request_with_renderer(
            request(1, "second"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                renderer_calls.fetch_add(1, Ordering::SeqCst);
                async { Err("renderer_should_not_run".to_string()) }
            },
        )
        .await
        .expect("cache hit");

        assert_eq!(renderer_calls.load(Ordering::SeqCst), 0);
        assert_eq!(second.request_id, "second");
        assert_eq!(second.generation, 1);
        assert_eq!(second.page_index, 0);
    }

    #[tokio::test]
    async fn orchestration_distinguishes_cancellation_from_renderer_failure() {
        let service = RenderService::new();
        service
            .set_active_document(Some(active_document(1)))
            .expect("activate");
        let cache = password_cache();

        let renderer_error = render_viewer_request_with_renderer(
            request(1, "renderer-error"),
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| async { Err("mock_renderer_failure".to_string()) },
        )
        .await;
        assert_eq!(
            renderer_error.expect_err("renderer error must propagate"),
            "mock_renderer_failure"
        );

        let cancellation_identity = request(1, "cancelled-error");
        let cancel_lookup = cancellation_identity.clone();
        let cancelled = render_viewer_request_with_renderer(
            cancellation_identity,
            ViewerRenderSizing::TargetWidth(10),
            0,
            &cache,
            &service,
            |_, _, _, _, _, _| {
                assert!(service
                    .cancel_request(&cancel_lookup)
                    .expect("cancel request"));
                async { Err("mock_renderer_failure".to_string()) }
            },
        )
        .await;
        assert_eq!(
            cancelled.expect_err("cancellation must take precedence"),
            "render_cancelled"
        );
        assert_eq!(service.cached_entry_count(), 0);
    }
}

fn pdf_rotation(rotation: i32) -> Result<PdfPageRenderRotation, String> {
    match rotation {
        0 => Ok(PdfPageRenderRotation::None),
        90 => Ok(PdfPageRenderRotation::Degrees90),
        180 => Ok(PdfPageRenderRotation::Degrees180),
        270 => Ok(PdfPageRenderRotation::Degrees270),
        _ => Err(format!("Geçersiz rotasyon (rotation) değeri: {}", rotation)),
    }
}

#[tauri::command]
pub fn clear_search_cache(
    _path: Option<String>,
    search_cache: State<'_, crate::SearchCache>,
) -> Result<(), String> {
    let mut cache = search_cache.0.lock();
    cache.clear();
    Ok(())
}
