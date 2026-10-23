/**
 * Placeholder geometry for PdfPage.
 *
 * Page sizes come from the worker (`get_session_page_sizes`) in PDF points with the page's
 * own /Rotate applied. `estimateRenderedPageCssSize` mirrors how the worker sizes a
 * `RenderPage { target: Scale(scale * dpr) }` bitmap so the placeholder already has the
 * height the rendered page will get:
 *   targetWidth = trunc(widthPt * renderScale)                      (worker/pdfium.rs)
 *   k = f32(targetWidth) / f32(widthPt); side = round(f32(side * k)) (pdfium-render apply_to_page)
 * For a viewer rotation of 90/270 the constraints are rotated, so the bitmap is
 * round(heightPt * k) x round(widthPt * k). The CSS size is the bitmap size / dpr.
 * src-tauri/tests/fixtures/contracts/page-render-geometry-v1.json pins real PDFium sizes.
 */

/** Used until any size of the document is known (A4 portrait). */
export const DEFAULT_PAGE_SIZE_PT = Object.freeze({ widthPt: 595.276, heightPt: 841.89 });

export const MAX_RENDER_DEVICE_PIXEL_RATIO = 2;

export function renderDevicePixelRatio(devicePixelRatio) {
  const ratio = Number(devicePixelRatio);
  return Math.min(Number.isFinite(ratio) && ratio > 0 ? ratio : 1, MAX_RENDER_DEVICE_PIXEL_RATIO);
}

/**
 * Pixel budget of one viewer page bitmap: 16 Mi pixels, 64 MiB of RGBA. At 4x zoom and
 * dpr 2 an A4 page would need 32 Mpx (128 MB) and an A3 page 64 Mpx (256 MB); the app
 * process can hold several such frames the webview has not consumed yet, which took it to
 * ~2.6 GB in the zoom stress run. Above the budget the page renders at a lower device
 * pixel ratio and the canvas is shown at the same CSS size, so very high zoom looks softer
 * instead of costing memory. Tiled rendering would keep it sharp; it is not built.
 */
export const MAX_VIEWER_RENDER_PIXELS = 16 * 1024 * 1024;

/**
 * Device pixel ratio a page is rendered with: the screen's (capped at 2), lowered so the
 * bitmap stays within MAX_VIEWER_RENDER_PIXELS. Requests and CSS sizing must both use it.
 */
export function pageRenderDevicePixelRatio({ pageSize, scale, devicePixelRatio = 1 }) {
  const dpr = renderDevicePixelRatio(devicePixelRatio);
  if (!isValidPageSize(pageSize) || !(scale > 0)) return dpr;
  const pixelsAtRatioOne = pageSize.widthPt * scale * pageSize.heightPt * scale;
  return Math.min(dpr, Math.sqrt(MAX_VIEWER_RENDER_PIXELS / pixelsAtRatioOne));
}

export function isValidPageSize(size) {
  return Boolean(size)
    && Number.isFinite(size.widthPt) && size.widthPt > 0
    && Number.isFinite(size.heightPt) && size.heightPt > 0;
}

/** Worker page-size list -> array of { widthPt, heightPt } or null for unusable entries. */
export function normalizePageSizes(raw) {
  if (!Array.isArray(raw)) return null;
  return raw.map((entry) => {
    const size = { widthPt: Number(entry?.widthPt), heightPt: Number(entry?.heightPt) };
    return isValidPageSize(size) ? size : null;
  });
}

export function normalizeRotation(rotation) {
  return ((Math.round(Number(rotation) || 0) % 360) + 360) % 360;
}

/** CSS width/height the rendered page will occupy, or null when it cannot be estimated. */
export function estimateRenderedPageCssSize({ pageSize, scale, rotation = 0, devicePixelRatio = 1 }) {
  if (!isValidPageSize(pageSize) || !(scale > 0)) return null;
  const dpr = pageRenderDevicePixelRatio({ pageSize, scale, devicePixelRatio });
  // The worker holds f32 points; JSON hands them over as their shortest decimal form.
  const widthPt = Math.fround(pageSize.widthPt);
  const heightPt = Math.fround(pageSize.heightPt);
  const targetWidth = Math.trunc(widthPt * (scale * dpr));
  if (targetWidth < 1) return null;
  const k = Math.fround(Math.fround(targetWidth) / widthPt);
  const scaledWidth = Math.max(1, Math.round(Math.fround(widthPt * k)));
  const scaledHeight = Math.max(1, Math.round(Math.fround(heightPt * k)));
  const normalized = normalizeRotation(rotation);
  const sideways = normalized === 90 || normalized === 270;
  return {
    width: (sideways ? scaledHeight : scaledWidth) / dpr,
    height: (sideways ? scaledWidth : scaledHeight) / dpr,
  };
}

/**
 * Remembers the last measured page size of the current document so pages without a
 * worker size borrow a real one. Forgets it when the document key (session + content
 * generation) changes.
 */
export function createPageSizeFallback() {
  let documentKey = "";
  let size = null;
  return {
    get(key) {
      if (key !== documentKey) return DEFAULT_PAGE_SIZE_PT;
      return size ?? DEFAULT_PAGE_SIZE_PT;
    },
    remember(key, measured) {
      if (!key || !isValidPageSize(measured)) return;
      documentKey = key;
      size = { widthPt: measured.widthPt, heightPt: measured.heightPt };
    },
    reset() {
      documentKey = "";
      size = null;
    },
  };
}
