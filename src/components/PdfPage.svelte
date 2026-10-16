<script module>
  import { createPageSizeFallback } from "../utils/pageGeometry.js";
  import { createTextWidthMeasurer } from "../utils/textLayerUtils.js";

  // Last measured page size of the current document; forgotten when the session or
  // content generation changes.
  const pageSizeFallback = createPageSizeFallback();
  // Shared by all pages: common words are measured once per app run, not per page and zoom.
  const measureTextWidth = createTextWidthMeasurer(() =>
    typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d"),
  );
</script>

<script>
  import { getContext, onMount, onDestroy, tick, untrack } from "svelte";
  import {
    cancelRenderRequest,
    getPageTextLayout,
    renderPage,
    retryPage,
  } from "../services/backend/viewer.js";
  import {
    DOCUMENT_QUARANTINED,
    pageFailureMessage,
    pageFailureRegistry,
  } from "../services/pageFailureRegistry.js";
  import {
    RENDER_SESSION_CONTEXT,
    requireActiveRenderSession,
  } from "../services/renderSession.js";
  import {
    createRenderRequestCoordinator,
    isExpectedRenderCancellation,
    validateRenderResponse,
  } from "../utils/renderRequestCoordinator.js";
  import AnnotationLayer from "./AnnotationLayer.svelte";
  import FormLayer from "./FormLayer.svelte";
  import OcrLayer from "./OcrLayer.svelte";
  import { paintBufferToCanvas } from "../utils/canvasUtils.js";
  import { createTextLayerItems } from "../utils/textLayerUtils.js";
  import { positionSearchBox } from "../utils/searchResultUtils.js";
  import {
    logicalRectToViewportRect,
    viewportDragToLogicalRect,
  } from "../features/redaction/secureRedactionGeometry.js";
  import { validateRedactionRegionInputs } from "../features/redaction/redactionSchemas.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  import { pageSizeStore } from "../services/pageSizeStore.js";
  import {
    DEFAULT_PAGE_SIZE_PT,
    estimateRenderedPageCssSize,
    renderDevicePixelRatio,
  } from "../utils/pageGeometry.js";
  const renderSession = getContext(RENDER_SESSION_CONTEXT);
  const renderCoordinator = createRenderRequestCoordinator({
    scope: "main_page",
    cancel: (request, reason) => cancelRenderRequest({
      sessionId: request.sessionId,
      pageIndex: request.pageIndex,
      contentGeneration: request.contentGeneration,
      requestId: request.requestId,
      scope: request.scope,
    }, reason),
  });
  const featureContext = getRuntimeFeatureContext();
  const annotationPresentation = getFeaturePresentation("annotation", featureContext);
  const formPresentation = getFeaturePresentation("form_editing", featureContext);
  const ocrPresentation = getFeaturePresentation("ocr", featureContext);
  const searchPresentation = getFeaturePresentation("search", featureContext);

  let {
    sessionId = "",
    pageIndex = 0,
    scale = 1.0,
    rotation = 0,
    // For notifying parent which page is currently most visible
    onIntersect = null,
    activeTool = "pointer",
    activeColor = "#facc15",
    colorMode = "normal",

    isSinglePageMode = false,
    currentPage = 0,
    generationId = 0,
    documentGenerationId = 0,
    // True while a pinch or button zoom is shown as a CSS transform and `scale` is not committed yet.
    isZooming = false,

    approximateCoveragePreview = null,
    approximateProcessPreview = null,
    approximateProcessChannels = { c: true, m: true, y: true, k: true },
    sensitiveDataHighlights = [],
    secureRedactionEnabled = false,
    secureRedactionSelectionMode = false,
    pendingRedactionRegions = [],
    onRedactionRegionSelected = null,
    searchRequestId = 0,
    searchMatches = [],
    onsearchHighlightReady,
    onsessionSnapshot,
  } = $props();

  // Plain `let`s are bookkeeping that no template, derived value or effect condition reads.
  let pageRef;
  // $state: bound inside {#if}; read only in untracked or async code.
  let canvasRef = $state();
  let isVisible = false;
  let isRendering = false;
  let observer = null;
  let lastRenderedScale = $state(0);
  let renderErrorMsg = $state("");
  let isDestroyed = false;
  let renderRequestId = 0;
  let paintedRenderKey = $state("");
  let paintedSessionId = $state("");
  let paintedPageIndex = $state(-1);
  let paintedPageRotation = $state(0);
  let pendingRenderKey = $state("");
  let failedRenderKey = $state("");
  // Set by an explicit retry; a successful render then unblocks the page's
  // other surfaces (text layout, thumbnails) as well.
  let retryPending = false;

  let baseImgWidth = $state(0);
  let baseImgHeight = $state(0);
  let renderedCssWidth = $state(0);
  let renderedCssHeight = $state(0);
  let cssScale = $state(1.0);
  let zoomDebounceTimer = null;

  // Aşama 3: Metin ve Arama Verileri
  let textLayouts = $state.raw([]);
  let observedTextLayoutKey = "";
  let loadedTextLayoutKey = $state("");
  let pendingTextLayoutKey = $state("");
  let failedTextLayoutKey = $state("");
  let textLayoutRequestId = 0;
  let searchHighlightLayerRef = $state();
  let lastHighlightReadyKey = "";
  let highlightReadySequence = 0;

  let isBufferIntersecting = $state(false);
  // Worker page size (pt, page /Rotate applied); null until loaded or when unavailable.
  let pageSizePt = $state.raw(null);
  let pageSizeRequestKey = "";
  let paintedDocumentKey = $state("");
  // Rendered CSS height minus the container height just before the paint (layout shift).
  let placeholderHeightDelta = $state(null);
  let redactionDragStart = $state.raw(null);
  let redactionDragCurrent = $state.raw(null);
  let redactionPointerId = null;

  let visibilityObserver = null;
  let bufferObserver = null;

  const normalizedRotation = $derived(((rotation % 360) + 360) % 360);
  const unscaledWidth = $derived(lastRenderedScale > 0 ? baseImgWidth / lastRenderedScale : 1);
  const unscaledHeight = $derived(lastRenderedScale > 0 ? baseImgHeight / lastRenderedScale : 1);
  const desiredRenderKey = $derived(sessionId && scale
    ? `${sessionId}:${documentGenerationId}:${pageIndex}:${scale}:${normalizedRotation}:${colorMode}:${generationId}`
    : "");
  const desiredTextLayoutKey = $derived(sessionId
    ? `${sessionId}:${documentGenerationId}:${pageIndex}`
    : "");
  const documentKey = $derived(sessionId ? `${sessionId}:${documentGenerationId}` : "");

  // Strict Mode-based Page Scoping (Reactively reduced to IntersectionObserver state)
  const shouldRender = $derived(isSinglePageMode
    ? (pageIndex === currentPage)
    : isBufferIntersecting);

  const textLayoutItems = $derived(
    paintedRenderKey &&
    paintedSessionId === sessionId &&
    paintedPageIndex === pageIndex &&
    loadedTextLayoutKey === desiredTextLayoutKey &&
    renderedCssWidth > 0 &&
    renderedCssHeight > 0
      ? createTextLayerItems(textLayouts, {
          rotation: paintedPageRotation,
          pageCssWidth: renderedCssWidth,
          pageCssHeight: renderedCssHeight,
          measureTextWidth,
        })
      : []);

  const positionedSearchMatches = $derived(
    paintedRenderKey &&
    renderedCssWidth > 0 &&
    renderedCssHeight > 0 &&
    Array.isArray(searchMatches)
      ? searchMatches
          .map((match) => ({
            ...match,
            boxes: Array.isArray(match?.boxes)
              ? match.boxes
                  .map((box) =>
                    positionSearchBox(box, {
                      rotation: paintedPageRotation,
                      pageCssWidth: renderedCssWidth,
                      pageCssHeight: renderedCssHeight,
                    }),
                  )
                  .filter(Boolean)
              : [],
          }))
          .filter((match) => match.boxes.length > 0)
      : []);

  const positionedPendingRedactions = $derived(Array.isArray(pendingRedactionRegions)
    ? pendingRedactionRegions
        .map((region) => ({
          ...region,
          viewportRect: logicalRectToViewportRect(region.normalizedTopLeftRect, normalizedRotation),
        }))
        .filter((region) => region.viewportRect)
    : []);

  const redactionPreviewRect = $derived(redactionDragStart && redactionDragCurrent
    ? {
        x: Math.min(redactionDragStart.x, redactionDragCurrent.x),
        y: Math.min(redactionDragStart.y, redactionDragCurrent.y),
        width: Math.abs(redactionDragCurrent.x - redactionDragStart.x),
        height: Math.abs(redactionDragCurrent.y - redactionDragStart.y),
      }
    : null);

  const isTextLayerInteractive = $derived(activeTool === "pointer");

  // Placeholder height: the worker page size gives the rendered height before render.
  // Without it, a rendered height of this page/document/rotation is rescaled, else the
  // document's last measured page size (or A4) is used.
  const estimatedPageCssSize = $derived(estimateRenderedPageCssSize({
    pageSize: pageSizePt ?? pageSizeFallback.get(documentKey),
    scale,
    rotation: normalizedRotation,
    devicePixelRatio: currentDevicePixelRatio(),
  }));
  const cachedHeight = $derived(pageSizePt && estimatedPageCssSize
    ? estimatedPageCssSize.height
    : renderedCssHeight > 0 &&
        lastRenderedScale > 0 &&
        paintedDocumentKey === documentKey &&
        paintedPageRotation === normalizedRotation
      ? renderedCssHeight * (scale / lastRenderedScale)
      : estimatedPageCssSize?.height ?? DEFAULT_PAGE_SIZE_PT.heightPt * scale);

  // Worker-safety failures survive scrolling and zooming: the page is not
  // requested again until the user retries it or the generation changes.
  // Callers pass `$pageFailureRegistry` and the identity explicitly so the
  // surrounding effect or template expression re-runs on change.
  function pageFailure(_registryVersion, session, generation, page, surface) {
    return pageFailureRegistry.failure({
      sessionId: session,
      contentGeneration: generation,
      pageIndex: page,
      surface,
    });
  }

  // Effects. `$effect.pre` runs before the DOM update, like the legacy reactive statements they replace:
  // the canvas must be released while it is still mounted. Each effect tracks only its condition;
  // the work runs in `untrack`. They run in this order within a flush, and the order matters
  // (text-layout reset before the text-layout request; memory release before the render queue).
  // They also run once on creation, so everything they touch is declared above.

  $effect.pre(() => {
    const key = documentKey;
    const session = sessionId;
    const generation = documentGenerationId;
    const page = pageIndex;
    untrack(() => requestPageSize(key, session, generation, page));
  });

  // Must precede the text-layout request effect: in the other order a page or session change
  // would request the layout, cancel it here, and request it again.
  $effect.pre(() => {
    const key = desiredTextLayoutKey;
    if (key !== observedTextLayoutKey) {
      observedTextLayoutKey = key;
      untrack(resetTextLayoutForDocument);
    }
  });

  $effect.pre(() => {
    if (
      shouldRender &&
      desiredTextLayoutKey &&
      !pageFailure($pageFailureRegistry, sessionId, documentGenerationId, pageIndex, "textLayout") &&
      desiredTextLayoutKey !== loadedTextLayoutKey &&
      desiredTextLayoutKey !== pendingTextLayoutKey &&
      desiredTextLayoutKey !== failedTextLayoutKey
    ) {
      const snapshot = {
        key: desiredTextLayoutKey,
        session: sessionId,
        page: pageIndex,
        documentGeneration: documentGenerationId,
      };
      untrack(() => queueTextLayoutRequest(snapshot));
    }
  });

  $effect.pre(() => {
    const activeMatch = positionedSearchMatches.find((match) => match.isActive);
    const readyKey = activeMatch && paintedRenderKey
      ? `${searchRequestId}:${activeMatch.index}:${pageIndex}:${paintedRenderKey}`
      : "";
    if (
      readyKey &&
      readyKey !== lastHighlightReadyKey &&
      shouldRender
    ) {
      untrack(() => {
        lastHighlightReadyKey = readyKey;
        const sequence = ++highlightReadySequence;
        void notifySearchHighlightReady(activeMatch.index, readyKey, sequence);
      });
    }
  });

  // Ekranda veya aktif mod sınırında değilse görsel verilerini bellekten sil
  $effect.pre(() => {
    if (!shouldRender) {
      untrack(cleanupMemory);
    }
  });

  // Render anahtarı temizlendiğinde veya değiştiğinde görünür sayfayı yeniden çiz.
  // No new render during a zoom gesture: zooming out under the CSS transform brings many more
  // pages into the buffer, and each would be rendered at the old, larger scale (~128 MB per A4
  // page at 4x on a 2x display). They render once the new scale is committed.
  $effect.pre(() => {
    if (
      shouldRender &&
      !isZooming &&
      desiredRenderKey &&
      !pageFailure($pageFailureRegistry, sessionId, documentGenerationId, pageIndex, "render") &&
      desiredRenderKey !== paintedRenderKey &&
      desiredRenderKey !== pendingRenderKey &&
      desiredRenderKey !== failedRenderKey
    ) {
      untrack(() => {
        const shouldDebounce = paintedRenderKey && lastRenderedScale !== scale;
        cssScale = lastRenderedScale > 0 ? scale / lastRenderedScale : 1.0;
        queuePageRender({
          key: desiredRenderKey,
          session: sessionId,
          page: pageIndex,
          viewScale: scale,
          pageRotation: normalizedRotation,
          pageColorMode: colorMode,
          generation: generationId,
          documentGeneration: documentGenerationId,
        }, shouldDebounce ? 150 : 0);
      });
    }
  });

  function currentDevicePixelRatio() {
    return renderDevicePixelRatio(typeof window !== "undefined" ? window.devicePixelRatio : 1);
  }

  function requestPageSize(key, session, generation, page) {
    const requestKey = key ? `${key}:${page}` : "";
    if (requestKey === pageSizeRequestKey) return;
    pageSizeRequestKey = requestKey;
    pageSizePt = requestKey ? pageSizeStore.peek(session, generation, page) : null;
    if (!requestKey || pageSizePt) return;
    void pageSizeStore.requestPage(session, generation, page).then((size) => {
      if (requestKey === pageSizeRequestKey && !isDestroyed) pageSizePt = size;
    });
  }

  async function notifySearchHighlightReady(matchIndex, readyKey, sequence) {
    await tick();
    if (
      sequence !== highlightReadySequence ||
      readyKey !== lastHighlightReadyKey ||
      !shouldRender ||
      !paintedRenderKey ||
      !searchHighlightLayerRef
    ) {
      return;
    }

    const element = searchHighlightLayerRef.querySelector(
      `[data-search-match-index="${matchIndex}"]`,
    );
    if (!element) return;

    onsearchHighlightReady?.({
      searchRequestId,
      matchIndex,
      pageIndex,
      element,
    });
  }

  function invalidatePendingTextLayoutRequest() {
    textLayoutRequestId += 1;
    pendingTextLayoutKey = "";
  }

  function resetTextLayoutForDocument() {
    invalidatePendingTextLayoutRequest();
    textLayouts = [];
    loadedTextLayoutKey = "";
    failedTextLayoutKey = "";
  }

  function queueTextLayoutRequest(snapshot) {
    const requestId = ++textLayoutRequestId;
    pendingTextLayoutKey = snapshot.key;
    failedTextLayoutKey = "";
    void fetchTextLayout({ ...snapshot, requestId });
  }

  async function fetchTextLayout(snapshot) {
    const {
      key,
      session,
      page,
      documentGeneration,
      requestId,
    } = snapshot;

    try {
      const layout = await getPageTextLayout(
        session,
        documentGeneration,
        page,
      );
      const requestIsCurrent =
        requestId === textLayoutRequestId &&
        key === desiredTextLayoutKey &&
        session === sessionId &&
        page === pageIndex &&
        documentGeneration === documentGenerationId &&
        shouldRender &&
        !isDestroyed;

      if (!requestIsCurrent) return;

      textLayouts = Array.isArray(layout) ? layout : [];
      loadedTextLayoutKey = key;
      failedTextLayoutKey = "";
    } catch (err) {
      pageFailureRegistry.record({
        sessionId: session,
        contentGeneration: documentGeneration,
        pageIndex: page,
        surface: "textLayout",
      }, err);
      if (
        requestId === textLayoutRequestId &&
        key === desiredTextLayoutKey &&
        shouldRender &&
        !isDestroyed
      ) {
        failedTextLayoutKey = key;
        if (String(err) !== "superseded") {
          console.warn(`Sayfa ${page} metin katmanı yüklenemedi:`, err);
        }
      }
    } finally {
      if (requestId === textLayoutRequestId && pendingTextLayoutKey === key) {
        pendingTextLayoutKey = "";
      }
    }
  }

  function queuePageRender(snapshot, delay) {
    if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);
    // A zoom gesture re-queues on every step. Cancelling the in-flight render
    // each time would make every step a worker cancellation; let it finish
    // (its result is dropped as stale) and cancel once the debounce settles.
    const deferCancel = delay > 0;
    if (!deferCancel) renderCoordinator.cancelActive("rerender");
    const requestId = ++renderRequestId;
    pendingRenderKey = snapshot.key;
    failedRenderKey = "";
    renderErrorMsg = "";

    zoomDebounceTimer = setTimeout(() => {
      zoomDebounceTimer = null;
      if (deferCancel) renderCoordinator.cancelActive("zoom");
      void fetchPageImage({ ...snapshot, requestId });
    }, delay);
  }

  async function fetchPageImage(snapshot) {
    const {
      key,
      session,
      page,
      viewScale,
      pageRotation,
      pageColorMode,
      generation: viewGeneration,
      documentGeneration,
      requestId: queueRequestId,
    } = snapshot;

    if (!session || queueRequestId !== renderRequestId) return;

    isRendering = true;
    renderErrorMsg = "";
    let requestSnapshot;

    const requestIsCurrent = () =>
      queueRequestId === renderRequestId &&
      (!requestSnapshot || renderCoordinator.isCurrent(requestSnapshot)) &&
      viewGeneration === generationId &&
      documentGeneration === documentGenerationId &&
      session === sessionId &&
      page === pageIndex &&
      viewScale === scale &&
      pageRotation === normalizedRotation &&
      pageColorMode === colorMode &&
      key === desiredRenderKey &&
      shouldRender &&
      !isDestroyed;

    try {
      const requestDpr = currentDevicePixelRatio();
      const targetRenderScale = viewScale * requestDpr;
      const sessionSnapshot = requireActiveRenderSession(
        renderSession,
        session,
        documentGeneration,
      );
      const request = renderCoordinator.begin({
        sessionId: sessionSnapshot.sessionId,
        pageIndex: page,
        scale: targetRenderScale,
        rotation: pageRotation,
        contentGeneration: documentGeneration,
      });
      requestSnapshot = request.snapshot;
      const renderPromise = renderPage(requestSnapshot);
      const rendered = validateRenderResponse(
        await request.wait(renderPromise),
        requestSnapshot,
      );
      if (!requestIsCurrent() || !canvasRef) return;

      const { width, height } = paintBufferToCanvas(canvasRef, rendered, {
        colorMode: pageColorMode,
      });
      const outputWidth = width / requestDpr;
      const outputHeight = height / requestDpr;
      const isSideways = pageRotation === 90 || pageRotation === 270;
      const heightBeforePaint = cachedHeight;

      renderedCssWidth = outputWidth;
      renderedCssHeight = outputHeight;
      baseImgWidth = isSideways ? outputHeight : outputWidth;
      baseImgHeight = isSideways ? outputWidth : outputHeight;
      paintedDocumentKey = `${session}:${documentGeneration}`;
      pageSizeFallback.remember(paintedDocumentKey, {
        widthPt: baseImgWidth / viewScale,
        heightPt: baseImgHeight / viewScale,
      });
      placeholderHeightDelta = outputHeight - heightBeforePaint;
      if (Math.abs(placeholderHeightDelta) > 1) {
        console.debug(
          `Sayfa ${page + 1} placeholder yüksekliği ${placeholderHeightDelta.toFixed(2)}px saptı`,
        );
      }

      lastRenderedScale = viewScale;
      cssScale = 1.0;
      paintedRenderKey = key;
      paintedSessionId = session;
      paintedPageIndex = page;
      paintedPageRotation = pageRotation;
      failedRenderKey = "";
      renderErrorMsg = "";
      if (retryPending) {
        retryPending = false;
        pageFailureRegistry.clearPage({
          sessionId: session,
          contentGeneration: documentGeneration,
          pageIndex: page,
        });
      }

    } catch (err) {
      const recorded = pageFailureRegistry.record({
        sessionId: session,
        contentGeneration: documentGeneration,
        pageIndex: page,
        surface: "render",
      }, err);
      if (
        requestIsCurrent() &&
        !isExpectedRenderCancellation(err)
      ) {
        console.error(`Sayfa ${page} render hatası:`, err);
        retryPending = false;
        const message = err?.message === "render_timeout"
          ? "Sayfa render zaman aşımına uğradı ve iptal edildi."
          : err?.message || err;
        renderErrorMsg = recorded
          ? pageFailureMessage(err.code, page)
          : `Sayfa ${page + 1} yüklenemedi: ${message}`;
        failedRenderKey = key;
      }
    } finally {
      if (requestSnapshot) renderCoordinator.settle(requestSnapshot);
      if (pendingRenderKey === key) pendingRenderKey = "";
      if (queueRequestId === renderRequestId) isRendering = false;
    }
  }

  // Explicit user retry: lifts the backend page quarantine, then lets exactly
  // one new render request through. Other surfaces wait for its success.
  async function retryFailedPage() {
    const identity = {
      sessionId,
      contentGeneration: documentGenerationId,
      pageIndex,
    };
    try {
      await retryPage(identity.sessionId, identity.contentGeneration, identity.pageIndex);
    } catch (err) {
      pageFailureRegistry.record({ ...identity, surface: "render" }, err);
      renderErrorMsg = pageFailureMessage(err?.code, identity.pageIndex) ||
        `Sayfa ${identity.pageIndex + 1} yeniden denenemedi: ${err?.message || err}`;
      return;
    }
    retryPending = true;
    failedRenderKey = "";
    renderErrorMsg = "";
    pageFailureRegistry.clearPage(identity, ["render"]);
  }

  function cleanupMemory() {
    if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);
    zoomDebounceTimer = null;
    renderCoordinator.cancelActive("viewport_exit");
    renderRequestId += 1;
    pendingRenderKey = "";
    paintedRenderKey = "";
    paintedSessionId = "";
    paintedPageIndex = -1;
    failedRenderKey = "";
    isRendering = false;

    if (canvasRef) {
      canvasRef.width = 1;
      canvasRef.height = 1;
    }

    // DOM katmanı {#if shouldRender} ile kaldırılır; küçük normalize veri tekrar IPC'yi önler.
    invalidatePendingTextLayoutRequest();
    failedTextLayoutKey = "";
  }

  onMount(() => {
    const rootEl = document.querySelector('.viewer-area');

    visibilityObserver = new IntersectionObserver(
      (entries) => {
        // Several threshold crossings can be batched; the last entry is the newest.
        const entry = entries[entries.length - 1];
        const isVisibleRatio = entry.isIntersecting && entry.intersectionRatio >= 0.1;
        isVisible = isVisibleRatio;

        if (typeof onIntersect === "function") {
          onIntersect(pageIndex, entry.isIntersecting ? entry.intersectionRatio : 0);
        }
      },
      { root: rootEl || null, threshold: Array.from({ length: 21 }, (_, step) => step / 20) }
    );

    // 1.5 viewport heights above and below. Measured against 250%: no blank
    // page up to 4000 px/s with ~170 ms renders, two fewer live page canvases.
    bufferObserver = new IntersectionObserver(
      (entries) => {
        isBufferIntersecting = entries[0].isIntersecting;
      },
      { root: rootEl || null, rootMargin: '150% 0px 150% 0px' }
    );

    if (pageRef) {
      visibilityObserver.observe(pageRef);
      bufferObserver.observe(pageRef);
    }
  });

  onDestroy(() => {
    isDestroyed = true;
    if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);
    if (visibilityObserver) visibilityObserver.disconnect();
    if (typeof onIntersect === "function") onIntersect(pageIndex, 0);
    if (bufferObserver) bufferObserver.disconnect();
    cleanupMemory();
    renderCoordinator.destroy();
    textLayouts = [];
    loadedTextLayoutKey = "";
  });

  function getViewportPoint(event, element) {
    const bounds = element.getBoundingClientRect();
    if (!(bounds.width > 0) || !(bounds.height > 0)) return null;
    return {
      x: Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width)),
      y: Math.min(1, Math.max(0, (event.clientY - bounds.top) / bounds.height)),
    };
  }

  function startRedactionSelection(event) {
    if (!secureRedactionEnabled || !secureRedactionSelectionMode || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const point = getViewportPoint(event, event.currentTarget);
    if (!point) return;
    redactionPointerId = event.pointerId;
    redactionDragStart = point;
    redactionDragCurrent = point;
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function moveRedactionSelection(event) {
    if (redactionPointerId !== event.pointerId || !redactionDragStart) return;
    event.preventDefault();
    event.stopPropagation();
    redactionDragCurrent = getViewportPoint(event, event.currentTarget);
  }

  function finishRedactionSelection(event) {
    if (redactionPointerId !== event.pointerId || !redactionDragStart) return;
    event.preventDefault();
    event.stopPropagation();
    const bounds = event.currentTarget.getBoundingClientRect();
    const finalPoint = getViewportPoint(event, event.currentTarget);
    const startPoint = redactionDragStart;
    redactionPointerId = null;
    redactionDragStart = null;
    redactionDragCurrent = null;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    if (!finalPoint || !(bounds.width > 0) || !(bounds.height > 0)) return;
    const viewportWidth = Math.abs(finalPoint.x - startPoint.x) * bounds.width;
    const viewportHeight = Math.abs(finalPoint.y - startPoint.y) * bounds.height;
    if (viewportWidth < 3 || viewportHeight < 3) return;

    const normalizedTopLeftRect = viewportDragToLogicalRect(
      startPoint,
      finalPoint,
      normalizedRotation,
    );
    if (!normalizedTopLeftRect) return;
    try {
      validateRedactionRegionInputs([{
        id: "redaction-region:selection-validation",
        pageIndex,
        normalizedTopLeftRect,
        source: "manual_selection",
      }]);
    } catch (_) {
      return;
    }
    if (typeof onRedactionRegionSelected === "function") {
      onRedactionRegionSelected({ pageIndex, normalizedTopLeftRect });
    }
  }

  function cancelRedactionSelection(event) {
    if (redactionPointerId !== event.pointerId) return;
    redactionPointerId = null;
    redactionDragStart = null;
    redactionDragCurrent = null;
  }
</script>

<div
  class="pdf-page-container"
  bind:this={pageRef}
  id={`page-${pageIndex}`}
  style="height: {cachedHeight}px; min-height: {cachedHeight}px;"
  aria-hidden={isSinglePageMode && !shouldRender ? "true" : undefined}
  data-placeholder-height-delta={placeholderHeightDelta ?? undefined}
>
  {#if shouldRender}
    <div
      class="pdf-image-wrapper"
      style="width: {renderedCssWidth > 0 ? (renderedCssWidth * cssScale) + 'px' : '100%'}; height: {renderedCssHeight > 0 ? (renderedCssHeight * cssScale) + 'px' : cachedHeight + 'px'};"
    >
      <div
        class="display-surface"
        style="width: {renderedCssWidth > 0 ? renderedCssWidth + 'px' : '100%'}; height: {renderedCssHeight > 0 ? renderedCssHeight + 'px' : cachedHeight + 'px'}; transform: scale({cssScale});"
      >
        <canvas
          bind:this={canvasRef}
          class="pdf-canvas"
          aria-label="Sayfa {pageIndex + 1}"
          style="width: 100%; height: 100%; opacity: {approximateProcessPreview ? 0 : 1};"
        ></canvas>

        {#if secureRedactionEnabled && (secureRedactionSelectionMode || positionedPendingRedactions.length > 0)}
          <!-- Review-only layer: these rectangles are not PDF mutations or secure-redaction results. -->
          <div
            class="pending-redaction-review-layer"
            class:is-selecting={secureRedactionSelectionMode}
            role="presentation"
            aria-label="Bekleyen redaksiyon adayları"
            onpointerdown={startRedactionSelection}
            onpointermove={moveRedactionSelection}
            onpointerup={finishRedactionSelection}
            onpointercancel={cancelRedactionSelection}
          >
            {#each positionedPendingRedactions as region (region.id)}
              <div
                class="pending-redaction-region"
                style="left: {region.viewportRect.x * 100}%; top: {region.viewportRect.y * 100}%; width: {region.viewportRect.width * 100}%; height: {region.viewportRect.height * 100}%;"
              ><span>PENDING</span></div>
            {/each}
            {#if redactionPreviewRect}
              <div
                class="pending-redaction-region preview"
                style="left: {redactionPreviewRect.x * 100}%; top: {redactionPreviewRect.y * 100}%; width: {redactionPreviewRect.width * 100}%; height: {redactionPreviewRect.height * 100}%;"
              ><span>PENDING</span></div>
            {/if}
          </div>
        {/if}

        {#if approximateCoveragePreview}
          <img
            src={`data:image/png;base64,${approximateCoveragePreview.heatmapPngBase64}`}
            class="heatmap-overlay"
            style="position: absolute; top: 0; left: 0; width: 100%; height: 100%; pointer-events: none; mix-blend-mode: multiply;"
            alt="RGB-türevli tahmini eşik ısı haritası"
          />
        {/if}

        {#if paintedRenderKey && textLayoutItems.length > 0}
          <div
            class="textLayer"
            class:is-interactive={isTextLayerInteractive}
            data-page-index={pageIndex}
          >
            {#each textLayoutItems as item, itemIndex (`${loadedTextLayoutKey}:${itemIndex}`)}
              <span
                data-text-index={itemIndex}
                style="
                  left: {item.left}px;
                  top: {item.top}px;
                  width: {item.width}px;
                  height: {item.height}px;
                  font-size: {item.fontSize}px;
                  transform: {item.transform};
                "
              >{item.text}{item.hasTrailingSpace ? " " : ""}</span>
            {/each}
          </div>
        {/if}

        {#if paintedRenderKey && positionedSearchMatches.length > 0 && searchPresentation.runnable}
          <div
            bind:this={searchHighlightLayerRef}
            class="search-highlight-layer"
            aria-hidden="true"
            style="pointer-events: none;"
          >
            {#each positionedSearchMatches as match (match.index)}
              {#each match.boxes as box, boxIndex (`${match.index}:${boxIndex}`)}
                <div
                  class="highlight-rect"
                  class:is-active={match.isActive}
                  data-search-match-index={match.index}
                  data-search-box-index={boxIndex}
                  style="
                    left: {box.left}px;
                    top: {box.top}px;
                    width: {box.width}px;
                    height: {box.height}px;
                  "
                ></div>
              {/each}
            {/each}
          </div>
        {/if}

        {#if paintedRenderKey && Array.isArray(sensitiveDataHighlights) && sensitiveDataHighlights.length > 0}
          <div
            class="review-highlight-layer"
            aria-hidden="true"
            style="position: absolute; inset: 0; pointer-events: none; z-index: 15;"
          >
            {#each sensitiveDataHighlights as highlight, highlightIdx (`${paintedRenderKey}:${highlightIdx}`)}
              <div
                class="review-highlight-rect"
                style="
                  position: absolute;
                  left: {highlight.x * 100}%;
                  top: {highlight.y * 100}%;
                  width: {highlight.width * 100}%;
                  height: {highlight.height * 100}%;
                  background: rgba(245, 158, 11, 0.25);
                  border: 1px dashed #f59e0b;
                  box-sizing: border-box;
                "
              ></div>
            {/each}
          </div>
        {/if}

        {#if paintedRenderKey}
          <div
            class="overlay-plane"
            style="width: {baseImgWidth}px; height: {baseImgHeight}px;"
          >
            {#if approximateProcessPreview}
              <div class="approximate-process-preview" style="position: absolute; inset: 0; transform: rotate({normalizedRotation}deg);">
                {#if approximateProcessChannels.c} <img src={`data:image/png;base64,${approximateProcessPreview.cPreviewPngBase64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="Tahmini C görünümü"/> {/if}
                {#if approximateProcessChannels.m} <img src={`data:image/png;base64,${approximateProcessPreview.mPreviewPngBase64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="Tahmini M görünümü"/> {/if}
                {#if approximateProcessChannels.y} <img src={`data:image/png;base64,${approximateProcessPreview.yPreviewPngBase64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="Tahmini Y görünümü"/> {/if}
                {#if approximateProcessChannels.k} <img src={`data:image/png;base64,${approximateProcessPreview.kPreviewPngBase64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="Tahmini K görünümü"/> {/if}
              </div>
            {/if}

            {#if unscaledWidth > 0 && unscaledHeight > 0}
              <div style="position: absolute; inset: 0; transform: rotate({normalizedRotation}deg); pointer-events: none;">
                {#if formPresentation.runnable}
                  <FormLayer {scale} />
                {/if}
                {#if ocrPresentation.runnable}
                  <OcrLayer
                    {sessionId}
                    contentGeneration={documentGenerationId}
                    {pageIndex}
                    pageWidth={unscaledWidth}
                    pageHeight={unscaledHeight}
                  />
                {/if}
                {#if annotationPresentation.runnable}
                <AnnotationLayer
                  {sessionId}
                  contentGeneration={documentGenerationId}
                    {pageIndex}
                    rotation={normalizedRotation}
                    pageWidth={unscaledWidth}
                    pageHeight={unscaledHeight}
                    {activeTool}
                    {activeColor}
                    {onsessionSnapshot}
                  />
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      {#if !paintedRenderKey}
        {@const renderFailure = pageFailure($pageFailureRegistry, sessionId, documentGenerationId, pageIndex, "render")}
        <div class="loading-skeleton page-loading-state">
          {#if renderErrorMsg || renderFailure}
            <span class="page-error">⚠️ {renderErrorMsg || pageFailureMessage(renderFailure, pageIndex)}</span>
            {#if renderFailure && renderFailure !== DOCUMENT_QUARANTINED}
              <button type="button" class="hud-btn page-retry" onclick={retryFailedPage}>Yeniden dene</button>
            {/if}
          {:else}
            <div class="spinner"></div>
            <span class="page-num">Sayfa {pageIndex + 1} Yükleniyor...</span>
          {/if}
        </div>
      {/if}
    </div>
  {:else if !isSinglePageMode}
    <!-- Single-page mode keeps inactive pages as empty, correctly sized scroll anchors. -->
    <div class="loading-skeleton" style="width: {renderedCssWidth > 0 ? (renderedCssWidth * cssScale) + 'px' : '100%'}; height: {cachedHeight}px; justify-content: center;">
      <div class="spinner"></div>
      <span class="page-num">Sayfa {pageIndex + 1} Yükleniyor...</span>
    </div>
  {/if}
</div>

<style>
  .pending-redaction-review-layer {
    position: absolute;
    inset: 0;
    z-index: 40;
    pointer-events: none;
    user-select: none;
  }

  .pending-redaction-review-layer.is-selecting {
    pointer-events: auto;
    cursor: crosshair;
    touch-action: none;
  }

  .pending-redaction-region {
    position: absolute;
    box-sizing: border-box;
    border: 2px dashed #f59e0b;
    background: rgba(245, 158, 11, 0.18);
    color: #1c1917;
    pointer-events: none;
  }

  .pending-redaction-region.preview {
    border-color: #ef4444;
    background: rgba(239, 68, 68, 0.15);
  }

  .pending-redaction-region span {
    position: absolute;
    top: 0;
    left: 0;
    padding: 1px 3px;
    background: #f59e0b;
    color: #111827;
    font: 700 9px/1.2 monospace;
  }

  .pdf-page-container {
    position: relative;
    width: 100%;
    display: flex;
    justify-content: center;
    align-items: center;
    background: var(--bg-surface);
    margin-bottom: 24px;
    box-shadow: 0 4px 12px rgba(0,0,0,0.1);
    contain: layout;
    overflow: hidden;
  }
  .pdf-image-wrapper {
    position: relative;
    line-height: 0;
  }
  .display-surface {
    position: relative;
    transform-origin: top left;
  }
  .pdf-canvas {
    display: block;
    border: 0;
    background: #fff;
  }
  .overlay-plane {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    transform-origin: center;
    z-index: 20;
    pointer-events: none;
  }
  .page-loading-state {
    position: absolute;
    inset: 0;
    justify-content: center;
    background: var(--bg-surface);
  }
  .loading-skeleton {
    display: flex;
    flex-direction: column;
    align-items: center;
    color: var(--text-muted);
    font-family: var(--font-mono);
  }
  .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid var(--border-raw);
    border-top-color: var(--accent-industrial);
    border-radius: 50%;
    animation: spin 1s linear infinite;
    margin-bottom: 8px;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  .page-error {
    color: #ef4444;
    font-size: 13px;
    padding: 12px;
    text-align: center;
    max-width: 300px;
  }
  .page-retry {
    margin-top: 4px;
    pointer-events: auto;
  }

  .search-highlight-layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    z-index: 9;
    pointer-events: none; /* Arama kutuları tıklamayı engellememeli */
    overflow: hidden;
  }
  .highlight-rect {
    position: absolute;
    background: rgba(255, 230, 0, 0.4);
    mix-blend-mode: multiply; /* Altındaki metni karartarak fosforlu kalem etkisi verir */
    border-radius: 2px;
  }
  .highlight-rect.is-active {
    background: rgba(255, 85, 0, 0.48);
    outline: 1px solid var(--accent-industrial);
    outline-offset: -1px;
  }
</style>
