import { evaluateRenderGuard } from "./renderGuard.js";
import { MAX_SCALE, MIN_SCALE, normalizeScale } from "./zoomLimits.js";

/**
 * Smallest CSS-transform zoom one gesture may show before the new scale is committed.
 * Zooming out under the transform makes WebKit raster a far larger part of the page list on every
 * step (at 4x committed and the 0.25x floor: 1/16, 256x the viewport area), and painted page canvases
 * multiply it. On 2026-10-13 that pushed WebContent past 38 GB and froze an 8 GB Mac. Halving per
 * gesture keeps the peak near the committed layout's own cost; the next gesture continues from the
 * committed scale.
 */
export const MIN_GESTURE_VISUAL_SCALE = 0.5;

/**
 * Zoom per pixel of ctrl+wheel deltaY, applied exponentially so equal deltas in and out cancel.
 * A macOS trackpad pinch sends a run of small deltas (0–8 px, often exactly 0), ~60 px for one
 * pinch, which this turns into about 1.8x.
 */
export const WHEEL_ZOOM_PER_PIXEL = 0.01;
/** Largest log-zoom change of one wheel event, so a mouse-wheel notch (~100 px) stays one ~10% step. */
export const MAX_WHEEL_ZOOM_STEP = 0.1;
const WHEEL_LINE_HEIGHT_PX = 16;

/**
 * Visual-scale factor of one ctrl+wheel event: >1 zooms in (negative deltaY), <1 out, and exactly 1
 * when the event carries no vertical delta, which a trackpad pinch sends many of.
 */
export function wheelZoomFactor({ deltaY, deltaMode = 0 } = {}) {
  if (typeof deltaY !== "number" || !Number.isFinite(deltaY) || deltaY === 0) return 1;
  // deltaMode 1 is lines; 2 (pages) is always at least a full step.
  const pixels = deltaMode === 1 ? deltaY * WHEEL_LINE_HEIGHT_PX : deltaMode === 2 ? Math.sign(deltaY) * Infinity : deltaY;
  const step = Math.min(Math.max(-pixels * WHEEL_ZOOM_PER_PIXEL, -MAX_WHEEL_ZOOM_STEP), MAX_WHEEL_ZOOM_STEP);
  return Math.exp(step);
}

/**
 * Inline CSS box for the image canvas: its bitmap size at the committed scale. Empty until a bitmap is
 * painted, so the canvas keeps its intrinsic size. The bitmap is not re-rendered per scale; zooming in
 * upscales it.
 */
export function imageCanvasStyle(size, scale) {
  if (!(size?.width > 0) || !(size?.height > 0)) return "";
  const factor = normalizeScale(scale);
  return `width: ${size.width * factor}px; height: ${size.height * factor}px;`;
}

/** Owns all worker document activation/deactivation serialization for one App. */
export function createViewerController({
  setActiveRenderDocument,
  renderSession,
  getRenderContext,
  readImage,
  paint,
  onError = () => {},
  onRendered = () => {},
  onImagePainted = () => {},
  onRenderFinished = () => {},
} = {}) {
  let activationQueue = Promise.resolve();
  let renderRequest = 0;
  let frame = null;
  let zoomTimer = null;
  let resizeTimer = null;
  let resizeObserver = null;
  let viewport = null;
  let removeWheelListener = null;
  let destroyed = false;

  const enqueueActivation = (operation) => {
    const run = () => Promise.resolve(operation());
    activationQueue = activationQueue.then(run, run);
    return activationQueue;
  };

  function activatePdf(snapshot) {
    return enqueueActivation(async () => {
      await setActiveRenderDocument({ sessionId: snapshot.id, contentGeneration: snapshot.contentGeneration });
      renderSession.activate(snapshot.id, snapshot.contentGeneration);
    });
  }

  function deactivatePdf(contentGeneration = renderSession.snapshot().contentGeneration) {
    return enqueueActivation(async () => {
      await setActiveRenderDocument({ sessionId: null, contentGeneration });
      renderSession.clear(contentGeneration);
    });
  }

  function cancelRender() {
    renderRequest += 1;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
  }

  function paintImage(canvas, rendered) {
    if (!canvas || !rendered) return;
    paint(canvas, rendered);
    onImagePainted({ width: canvas.width, height: canvas.height });
  }

  function renderImage({ canvas, rotation, flipH, flipV, overlay }) {
    const context = getRenderContext();
    if (!context?.imagePath || !canvas || destroyed) return;
    const imagePath = context.imagePath;
    const request = ++renderRequest;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = requestAnimationFrame(async () => {
      frame = null;
      const current = () => evaluateRenderGuard({
        isMounted: !destroyed && getRenderContext()?.mounted === true,
        request,
        activeRequest: renderRequest,
        isPdf: false,
        capturedSessionId: null,
        currentSessionId: null,
        capturedGeneration: null,
        currentGeneration: null,
        capturedImagePath: imagePath,
        currentImagePath: getRenderContext()?.imagePath ?? "",
      });
      if (!current()) return;
      try {
        const rendered = await readImage(imagePath, rotation, flipH, flipV);
        if (!current()) return;
        paintImage(canvas, rendered);
        if (!current()) return;
        overlay?.();
        if (current()) onRendered();
      } catch (error) {
        if (current()) onError(error);
      } finally {
        if (current()) onRenderFinished();
      }
    });
  }

  function scheduleRender(onRender, delayMs) {
    if (zoomTimer !== null) clearTimeout(zoomTimer);
    zoomTimer = setTimeout(() => {
      zoomTimer = null;
      if (!destroyed) onRender();
    }, delayMs);
  }

  function attachViewport({ element, viewer, hasDocument, isMounted, onRender, onZoomStart = () => {}, onZoomCommit = () => {} }) {
    detachViewport();
    if (!element) return;
    viewport = element;
    const renderIfCurrent = () => {
      if (!destroyed && isMounted() && hasDocument()) onRender({ silent: true });
    };
    const handleWheel = (event) => {
      if (!hasDocument() || (!event.ctrlKey && !event.metaKey)) return;
      event.preventDefault();
      const zoomFactor = wheelZoomFactor(event);
      if (zoomFactor === 1) return;
      if (!viewer.isZooming) onZoomStart();
      let visualScale = viewer.visualScale * zoomFactor;
      const effectiveScale = viewer.scale * visualScale;
      if (effectiveScale < MIN_SCALE) visualScale = MIN_SCALE / viewer.scale;
      if (effectiveScale > MAX_SCALE) visualScale = MAX_SCALE / viewer.scale;
      viewer.visualScale = Math.max(visualScale, MIN_GESTURE_VISUAL_SCALE);
      viewer.isZooming = true;
      scheduleRender(() => {
        viewer.scale = normalizeScale(viewer.scale * viewer.visualScale);
        viewer.visualScale = 1;
        viewer.isZooming = false;
        renderIfCurrent();
        if (!destroyed && isMounted() && hasDocument()) onZoomCommit();
      }, 300);
    };
    element.addEventListener("wheel", handleWheel, { passive: false });
    removeWheelListener = () => element.removeEventListener("wheel", handleWheel);
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(() => {
        if (!isMounted() || !hasDocument()) return;
        if (resizeTimer !== null) clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          resizeTimer = null;
          renderIfCurrent();
        }, 200);
      });
      resizeObserver.observe(element);
    }
  }

  function commitZoom({ viewer, nextScale, onRender, delayMs = 250 }) {
    if (!viewer || destroyed) return;
    if (nextScale / viewer.scale < MIN_GESTURE_VISUAL_SCALE) {
      // Too far to show as a transform (e.g. reset from 4x): lay out the new scale directly.
      if (zoomTimer !== null) clearTimeout(zoomTimer);
      zoomTimer = null;
      viewer.scale = normalizeScale(nextScale);
      viewer.visualScale = 1;
      viewer.isZooming = false;
      onRender({ silent: true });
      return;
    }
    viewer.visualScale = nextScale / viewer.scale;
    viewer.isZooming = true;
    scheduleRender(() => {
      viewer.scale = normalizeScale(nextScale);
      viewer.visualScale = 1;
      viewer.isZooming = false;
      onRender({ silent: true });
    }, delayMs);
  }

  /**
   * Drops the deferred commit of a wheel or button zoom so the caller can commit the scale itself.
   * Returns true when one was pending.
   */
  function cancelPendingZoom() {
    if (zoomTimer === null) return false;
    clearTimeout(zoomTimer);
    zoomTimer = null;
    return true;
  }

  function detachViewport() {
    removeWheelListener?.();
    removeWheelListener = null;
    resizeObserver?.disconnect();
    resizeObserver = null;
    viewport = null;
    if (resizeTimer !== null) clearTimeout(resizeTimer);
    resizeTimer = null;
  }

  function stop() {
    destroyed = true;
    detachViewport();
    if (zoomTimer !== null) clearTimeout(zoomTimer);
    zoomTimer = null;
    cancelRender();
    return activationQueue;
  }

  return Object.freeze({ activatePdf, deactivatePdf, attachViewport, detachViewport, commitZoom, cancelPendingZoom, paintImage, renderImage, cancelRender, stop, whenIdle: () => activationQueue });
}
