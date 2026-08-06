<script context="module">
  let globalAspectRatio = 0;
</script>

<script>
  import { createEventDispatcher, getContext, onMount, onDestroy, tick } from "svelte";
  import {
    cancelRenderRequest,
    getPageTextLayout,
    renderPage,
  } from "../services/backend/viewer.js";
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
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  const dispatch = createEventDispatcher();
  const renderSession = getContext(RENDER_SESSION_CONTEXT);
  const renderCoordinator = createRenderRequestCoordinator({
    scope: "main_page",
    cancel: (request) => cancelRenderRequest({
      sessionId: request.sessionId,
      pageIndex: request.pageIndex,
      contentGeneration: request.contentGeneration,
      requestId: request.requestId,
      scope: request.scope,
    }),
  });
  const featureContext = getRuntimeFeatureContext();
  const annotationPresentation = getFeaturePresentation("annotation", featureContext);
  const formPresentation = getFeaturePresentation("form_editing", featureContext);
  const ocrPresentation = getFeaturePresentation("ocr", featureContext);
  const searchPresentation = getFeaturePresentation("search", featureContext);

  export let sessionId = "";
  export let pageIndex = 0;
  export let scale = 1.0;
  export let rotation = 0;
  // For notifying parent which page is currently most visible
  export let onIntersect = null;
  export let activeTool = "pointer";
  export let activeColor = "#facc15";
  export let colorMode = "normal";


  export let isSinglePageMode = false;
  export let currentPage = 0;
  export let generationId = 0;
  export let documentGenerationId = 0;

  export let prepressHeatmap = null;
  export let prepressSeparations = null;
  export let prepressChannels = { c: true, m: true, y: true, k: true };
  export let prepressBoxes = null;
  export let sensitiveDataHighlights = [];

  let pageRef;
  let canvasRef;
  let isVisible = false;
  let isRendering = false;
  let observer = null;
  let lastRenderedScale = 0;
  let renderErrorMsg = "";
  let isDestroyed = false;
  let renderRequestId = 0;
  let paintedRenderKey = "";
  let paintedSessionId = "";
  let paintedPageIndex = -1;
  let paintedPageRotation = 0;
  let pendingRenderKey = "";
  let failedRenderKey = "";

  let baseImgWidth = 0;
  let baseImgHeight = 0;
  let renderedCssWidth = 0;
  let renderedCssHeight = 0;
  let cssScale = 1.0;
  let zoomDebounceTimer = null;

  // Aşama 3: Metin ve Arama Verileri
  let textLayouts = [];
  let textLayoutItems = [];
  let observedTextLayoutKey = "";
  let loadedTextLayoutKey = "";
  let pendingTextLayoutKey = "";
  let failedTextLayoutKey = "";
  let textLayoutRequestId = 0;
  let textMeasureCanvas = null;
  let textMeasureContext = null;
  export let searchRequestId = 0;
  export let searchMatches = [];
  let positionedSearchMatches = [];
  let searchHighlightLayerRef;
  let lastHighlightReadyKey = "";
  let highlightReadySequence = 0;

  let isBufferIntersecting = false;
  let cachedHeight = 1131;
  let containerWidth = 800;

  $: normalizedRotation = ((rotation % 360) + 360) % 360;
  $: unscaledWidth = lastRenderedScale > 0 ? baseImgWidth / lastRenderedScale : 1;
  $: unscaledHeight = lastRenderedScale > 0 ? baseImgHeight / lastRenderedScale : 1;
  $: desiredRenderKey = sessionId && scale
    ? `${sessionId}:${documentGenerationId}:${pageIndex}:${scale}:${normalizedRotation}:${colorMode}:${generationId}`
    : "";
  $: desiredTextLayoutKey = sessionId
    ? `${sessionId}:${documentGenerationId}:${pageIndex}`
    : "";

  // Strict Mode-based Page Scoping (Reactively reduced to IntersectionObserver state)
  $: shouldRender = isSinglePageMode
    ? (pageIndex === currentPage)
    : isBufferIntersecting;

  $: if (desiredTextLayoutKey !== observedTextLayoutKey) {
    observedTextLayoutKey = desiredTextLayoutKey;
    resetTextLayoutForDocument();
  }

  $: if (
    shouldRender &&
    desiredTextLayoutKey &&
    desiredTextLayoutKey !== loadedTextLayoutKey &&
    desiredTextLayoutKey !== pendingTextLayoutKey &&
    desiredTextLayoutKey !== failedTextLayoutKey
  ) {
    queueTextLayoutRequest({
      key: desiredTextLayoutKey,
      session: sessionId,
      page: pageIndex,
      documentGeneration: documentGenerationId,
    });
  }

  $: textLayoutItems =
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
      : [];

  $: positionedSearchMatches =
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
      : [];

  $: {
    const activeMatch = positionedSearchMatches.find((match) => match.isActive);
    const readyKey = activeMatch && paintedRenderKey
      ? `${searchRequestId}:${activeMatch.index}:${pageIndex}:${paintedRenderKey}`
      : "";
    if (
      readyKey &&
      readyKey !== lastHighlightReadyKey &&
      shouldRender
    ) {
      lastHighlightReadyKey = readyKey;
      const sequence = ++highlightReadySequence;
      void notifySearchHighlightReady(activeMatch.index, readyKey, sequence);
    }
  }

  $: isTextLayerInteractive = activeTool === "pointer";

  // Ekranda veya aktif mod sınırında değilse görsel verilerini bellekten sil
  $: if (!shouldRender) {
    cleanupMemory();
  }

  // Unrendered Page Height Fallback & Memory
  $: if (renderedCssHeight > 0 && lastRenderedScale > 0) {
      const activeScaleRatio = scale / lastRenderedScale;
      cachedHeight = renderedCssHeight * activeScaleRatio;
  } else {
      const baseRatio = globalAspectRatio > 0 ? globalAspectRatio : 0.707;
      const displayRatio = normalizedRotation === 90 || normalizedRotation === 270
        ? 1 / baseRatio
        : baseRatio;
      cachedHeight = (containerWidth / displayRatio) * scale;
  }

  // Render anahtarı temizlendiğinde veya değiştiğinde görünür sayfayı yeniden çiz.
  $: if (
    shouldRender &&
    desiredRenderKey &&
    desiredRenderKey !== paintedRenderKey &&
    desiredRenderKey !== pendingRenderKey &&
    desiredRenderKey !== failedRenderKey
  ) {
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
  }

  function measureTextWidth(text, fontSize) {
    if (!textMeasureCanvas && typeof document !== "undefined") {
      textMeasureCanvas = document.createElement("canvas");
      textMeasureContext = textMeasureCanvas.getContext("2d");
    }

    if (!textMeasureContext) return Number.NaN;
    textMeasureContext.font = `${fontSize}px sans-serif`;
    return textMeasureContext.measureText(text).width;
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

    dispatch("searchHighlightReady", {
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
        sessionId,
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
    renderCoordinator.cancelActive();
    const requestId = ++renderRequestId;
    pendingRenderKey = snapshot.key;
    failedRenderKey = "";

    zoomDebounceTimer = setTimeout(() => {
      zoomDebounceTimer = null;
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

    try {
      console.log("Rendering page:", page + 1);

      const requestDpr = Math.min(window.devicePixelRatio || 1.0, 2.0);
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
      const requestIsCurrent =
        queueRequestId === renderRequestId &&
        renderCoordinator.isCurrent(requestSnapshot) &&
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

      if (!requestIsCurrent || !canvasRef) {
        console.log(`Render result dropped for page ${page + 1}`);
        return;
      }

      const { width, height } = paintBufferToCanvas(canvasRef, rendered, {
        colorMode: pageColorMode,
      });
      const outputWidth = width / requestDpr;
      const outputHeight = height / requestDpr;
      const isSideways = pageRotation === 90 || pageRotation === 270;

      renderedCssWidth = outputWidth;
      renderedCssHeight = outputHeight;
      baseImgWidth = isSideways ? outputHeight : outputWidth;
      baseImgHeight = isSideways ? outputWidth : outputHeight;
      if (globalAspectRatio === 0 && baseImgHeight > 0) {
        globalAspectRatio = baseImgWidth / baseImgHeight;
      }

      lastRenderedScale = viewScale;
      cssScale = 1.0;
      paintedRenderKey = key;
      paintedSessionId = session;
      paintedPageIndex = page;
      paintedPageRotation = pageRotation;
      failedRenderKey = "";

    } catch (err) {
      if (
        queueRequestId === renderRequestId &&
        shouldRender &&
        !isDestroyed &&
        !isExpectedRenderCancellation(err)
      ) {
        console.error(`Sayfa ${page} render hatası:`, err);
        const message = err?.message === "render_timeout"
          ? "Sayfa render zaman aşımına uğradı ve iptal edildi."
          : err?.message || err;
        renderErrorMsg = `Sayfa ${page + 1} yüklenemedi: ${message}`;
        failedRenderKey = key;
      }
    } finally {
      if (requestSnapshot) renderCoordinator.settle(requestSnapshot);
      if (pendingRenderKey === key) pendingRenderKey = "";
      if (queueRequestId === renderRequestId) isRendering = false;
    }
  }

  function cleanupMemory() {
    if (zoomDebounceTimer) clearTimeout(zoomDebounceTimer);
    zoomDebounceTimer = null;
    renderCoordinator.cancelActive();
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

  let visibilityObserver = null;
  let bufferObserver = null;

  onMount(() => {
    const rootEl = document.querySelector('.viewer-area');

    visibilityObserver = new IntersectionObserver(
      (entries) => {
        const entry = entries[0];
        const isVisibleRatio = entry.isIntersecting && entry.intersectionRatio >= 0.1;
        isVisible = isVisibleRatio;

        if (isVisibleRatio && typeof onIntersect === "function") {
          onIntersect(pageIndex);
        }
      },
      { root: rootEl || null, threshold: [0, 0.1, 0.25, 0.5, 0.75, 1.0] }
    );

    bufferObserver = new IntersectionObserver(
      (entries) => {
        isBufferIntersecting = entries[0].isIntersecting;
      },
      { root: rootEl || null, rootMargin: '250% 0px 250% 0px' }
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
    if (bufferObserver) bufferObserver.disconnect();
    cleanupMemory();
    renderCoordinator.destroy();
    textLayouts = [];
    loadedTextLayoutKey = "";
    textMeasureCanvas = null;
    textMeasureContext = null;
  });

  function getNormalizedLocalCoords(clientX, clientY, element, rot) {
    const rect = element.getBoundingClientRect();
    const cx = rect.left + rect.width / 2;
    const cy = rect.top + rect.height / 2;

    const dx = clientX - cx;
    const dy = clientY - cy;

    const rad = -rot * (Math.PI / 180);
    const cos = Math.cos(rad);
    const sin = Math.sin(rad);

    const rx = dx * cos - dy * sin;
    const ry = dx * sin + dy * cos;

    const ow = element.offsetWidth;
    const oh = element.offsetHeight;

    const localX = rx + ow / 2;
    const localY = ry + oh / 2;

    return {
      x: localX / ow,
      y: localY / oh
    };
  }

  // --- Metin Vurgulama (Highlight) İşlemleri ---
  import { annotationsStore, addHighlight } from "../annotationsStore.js";

  $: pageHighlights = $annotationsStore ? $annotationsStore.filter(h => h.pageIndex === pageIndex) : [];

  function handleTextSelection(e) {
    if (activeTool !== 'highlight') return;

    const selection = window.getSelection();
    if (!selection || selection.isCollapsed) return;

    const range = selection.getRangeAt(0);
    const rects = range.getClientRects();
    if (rects.length === 0) return;

    const layerEl = e.currentTarget;

    let added = false;
    for (let i = 0; i < rects.length; i++) {
      const r = rects[i];
      const tlLocal = getNormalizedLocalCoords(r.left, r.top, layerEl, rotation);
      const brLocal = getNormalizedLocalCoords(r.right, r.bottom, layerEl, rotation);

      const x = Math.min(tlLocal.x, brLocal.x);
      const y = Math.min(tlLocal.y, brLocal.y);
      const width = Math.abs(brLocal.x - tlLocal.x);
      const height = Math.abs(brLocal.y - tlLocal.y);

      // Çok küçük rect'leri atla (artifacts)
      if (width * layerEl.offsetWidth > 2 && height * layerEl.offsetHeight > 2) {
         addHighlight(pageIndex, { x, y, width, height }, activeColor);
         added = true;
      }
    }

    if (added) {
      selection.removeAllRanges();
    }
  }
</script>

<div class="pdf-page-container" bind:this={pageRef} bind:clientWidth={containerWidth} id={`page-${pageIndex}`} style="height: {cachedHeight}px; min-height: {cachedHeight}px;">
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
          style="width: 100%; height: 100%; opacity: {prepressSeparations ? 0 : 1};"
        ></canvas>

        {#if prepressHeatmap}
          <img
            src={`data:image/png;base64,${prepressHeatmap}`}
            class="heatmap-overlay"
            style="position: absolute; top: 0; left: 0; width: 100%; height: 100%; pointer-events: none; mix-blend-mode: multiply;"
            alt="Heatmap"
          />
        {/if}

        {#if prepressBoxes}
          <div class="box-guides" style="position: absolute; top: 0; left: 0; width: 100%; height: 100%; pointer-events: none;">
            {#if prepressBoxes.bleed > 0}
              <div style="position: absolute; top: {prepressBoxes.bleed * 2.834 * lastRenderedScale}px; bottom: {prepressBoxes.bleed * 2.834 * lastRenderedScale}px; left: {prepressBoxes.bleed * 2.834 * lastRenderedScale}px; right: {prepressBoxes.bleed * 2.834 * lastRenderedScale}px; border: 1px dashed red;"></div>
            {/if}
            {#if prepressBoxes.trim > 0}
              <div style="position: absolute; top: {prepressBoxes.trim * 2.834 * lastRenderedScale}px; bottom: {prepressBoxes.trim * 2.834 * lastRenderedScale}px; left: {prepressBoxes.trim * 2.834 * lastRenderedScale}px; right: {prepressBoxes.trim * 2.834 * lastRenderedScale}px; border: 1px dashed green;"></div>
            {/if}
          </div>
        {/if}

        {#if paintedRenderKey && textLayoutItems.length > 0}
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div
            class="textLayer"
            class:is-interactive={isTextLayerInteractive}
            data-page-index={pageIndex}
            on:mouseup={handleTextSelection}
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
            {#if prepressSeparations}
              <div class="separations-container" style="position: absolute; inset: 0; transform: rotate({normalizedRotation}deg);">
                {#if prepressChannels.c} <img src={`data:image/png;base64,${prepressSeparations.c_base64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="C"/> {/if}
                {#if prepressChannels.m} <img src={`data:image/png;base64,${prepressSeparations.m_base64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="M"/> {/if}
                {#if prepressChannels.y} <img src={`data:image/png;base64,${prepressSeparations.y_base64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="Y"/> {/if}
                {#if prepressChannels.k} <img src={`data:image/png;base64,${prepressSeparations.k_base64}`} style="position: absolute; inset: 0; width: 100%; height: 100%; mix-blend-mode: multiply;" alt="K"/> {/if}
              </div>
            {/if}

            {#if unscaledWidth > 0 && unscaledHeight > 0}
              <div style="position: absolute; inset: 0; transform: rotate({normalizedRotation}deg); pointer-events: none;">
                {#if pageHighlights.length > 0}
                  <svg class="highlight-overlay-svg" style="position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; mix-blend-mode: multiply; z-index: 1;">
                    {#each pageHighlights as hl}
                      <rect
                        x="{hl.rect.x * 100}%"
                        y="{hl.rect.y * 100}%"
                        width="{hl.rect.width * 100}%"
                        height="{hl.rect.height * 100}%"
                        fill={hl.color}
                        opacity="0.6"
                      />
                    {/each}
                  </svg>
                {/if}

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
                    {scale}
                    pageWidth={unscaledWidth}
                    pageHeight={unscaledHeight}
                    {activeTool}
                    {activeColor}
                    on:sessionSnapshot
                  />
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      {#if !paintedRenderKey}
        <div class="loading-skeleton page-loading-state">
          {#if renderErrorMsg}
            <span class="page-error">⚠️ {renderErrorMsg}</span>
          {:else}
            <div class="spinner"></div>
            <span class="page-num">Sayfa {pageIndex + 1} Yükleniyor...</span>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="loading-skeleton" style="width: {renderedCssWidth > 0 ? (renderedCssWidth * cssScale) + 'px' : '100%'}; height: {cachedHeight}px; justify-content: center;">
      <div class="spinner"></div>
      <span class="page-num">Sayfa {pageIndex + 1} Yükleniyor...</span>
    </div>
  {/if}
</div>

<style>
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
