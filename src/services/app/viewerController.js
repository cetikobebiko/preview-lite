import { evaluateRenderGuard } from "./renderGuard.js";

/** Owns all worker document activation/deactivation serialization for one App. */
export function createViewerController({
  setActiveRenderDocument,
  renderSession,
  getRenderContext,
  readImage,
  paint,
  onError = () => {},
  onRendered = () => {},
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
    if (canvas && rendered) paint(canvas, rendered);
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
      if (!viewer.isZooming) onZoomStart();
      const zoomFactor = event.deltaY < 0 ? 1.05 : 0.95;
      let visualScale = viewer.visualScale * zoomFactor;
      const effectiveScale = viewer.scale * visualScale;
      if (effectiveScale < 0.25) visualScale = 0.25 / viewer.scale;
      if (effectiveScale > 4) visualScale = 4 / viewer.scale;
      viewer.visualScale = visualScale;
      viewer.isZooming = true;
      scheduleRender(() => {
        viewer.scale = Number(Math.min(Math.max(viewer.scale * viewer.visualScale, 0.25), 4).toFixed(2));
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
    viewer.visualScale = nextScale / viewer.scale;
    viewer.isZooming = true;
    scheduleRender(() => {
      viewer.scale = Number(nextScale.toFixed(2));
      viewer.visualScale = 1;
      viewer.isZooming = false;
      onRender({ silent: true });
    }, delayMs);
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

  return Object.freeze({ activatePdf, deactivatePdf, attachViewport, detachViewport, commitZoom, paintImage, renderImage, cancelRender, stop, whenIdle: () => activationQueue });
}
