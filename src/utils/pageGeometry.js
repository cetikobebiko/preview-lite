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
  const dpr = renderDevicePixelRatio(devicePixelRatio);
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
