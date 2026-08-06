<script>
  import { getContext, onMount, onDestroy } from "svelte";
  import {
    cancelRenderRequest,
    renderThumbnail,
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
  import { paintBufferToCanvas } from "../utils/canvasUtils.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    displayIndex = 0,
    isActive = false,
    onSelect = () => {},
    onDelete = () => {},
    onDragStart = () => {},
    onDragOver = () => {},
    onDrop = () => {},
  } = $props();

  let containerRef = $state();
  let canvasRef = $state();
  let isLoaded = $state(false);
  let isLoading = $state(false);
  let isDragOver = $state(false);
  let observer = $state();
  let isMounted = $state(false);
  let isIntersecting = $state(false);
  let usesIntersectionObserver = $state(false);
  let thumbnailRequest = $state(0);
  let sourceKey = $state("");
  const renderSession = getContext(RENDER_SESSION_CONTEXT);
  const renderCoordinator = createRenderRequestCoordinator({
    scope: "organizer_thumbnail",
    cancel: (request) => cancelRenderRequest({
      sessionId: request.sessionId,
      pageIndex: request.pageIndex,
      contentGeneration: request.contentGeneration,
      requestId: request.requestId,
      scope: request.scope,
    }),
  });

  function releaseCanvasBackingStore(canvas = canvasRef) {
    if (!canvas) return;
    canvas.width = 0;
    canvas.height = 0;
  }

  function invalidateThumbnail() {
    renderCoordinator.cancelActive();
    thumbnailRequest += 1;
    isLoaded = false;
    isLoading = false;
    releaseCanvasBackingStore();
  }

  onMount(() => {
    isMounted = true;
    sourceKey = `${sessionId}:${contentGeneration}:${pageIndex}`;
    if (typeof IntersectionObserver !== "undefined") {
      usesIntersectionObserver = true;
      observer = new IntersectionObserver(
        (entries) => {
          entries.forEach((entry) => {
            isIntersecting = entry.isIntersecting;
            if (isIntersecting && !isLoaded && !isLoading) {
              void loadThumbnail();
            } else if (
              !isIntersecting &&
              (isLoaded || isLoading || canvasRef?.width || canvasRef?.height)
            ) {
              invalidateThumbnail();
            }
          });
        },
        { rootMargin: "100px" }
      );
      if (containerRef) {
        observer.observe(containerRef);
      }
    } else {
      void loadThumbnail();
    }
  });

  onDestroy(() => {
    isMounted = false;
    isIntersecting = false;
    invalidateThumbnail();
    renderCoordinator.destroy();
    if (observer) {
      if (containerRef) observer.unobserve(containerRef);
      observer.disconnect();
    }
  });

  async function loadThumbnail() {
    if (!sessionId || !canvasRef) return;
    const request = ++thumbnailRequest;
    const sourceSessionId = sessionId;
    const sourceContentGeneration = contentGeneration;
    const sourcePageIndex = pageIndex;
    const canvas = canvasRef;
    let requestSnapshot;
    const isCurrentRequest = () =>
      isMounted &&
      request === thumbnailRequest &&
      canvasRef === canvas &&
      sessionId === sourceSessionId &&
      contentGeneration === sourceContentGeneration &&
      pageIndex === sourcePageIndex &&
      (!usesIntersectionObserver || isIntersecting);

    isLoading = true;
    try {
      const session = requireActiveRenderSession(
        renderSession,
        sourceSessionId,
        sourceContentGeneration,
      );
      const requestHandle = renderCoordinator.begin({
        sessionId: session.sessionId,
        pageIndex: sourcePageIndex,
        targetWidth: 150,
        contentGeneration: session.contentGeneration,
      });
      requestSnapshot = requestHandle.snapshot;
      const rendered = validateRenderResponse(
        await requestHandle.wait(renderThumbnail(requestSnapshot)),
        requestSnapshot,
      );
      if (!isCurrentRequest() || !renderCoordinator.isCurrent(requestSnapshot)) return;

      paintBufferToCanvas(canvas, rendered);
      isLoaded = true;
    } catch (err) {
      if (isCurrentRequest() && !isExpectedRenderCancellation(err)) {
        console.error(`Thumbnail ${sourcePageIndex} render error:`, err);
      }
    } finally {
      if (requestSnapshot) renderCoordinator.settle(requestSnapshot);
      if (isCurrentRequest()) {
        isLoading = false;
      }
    }
  }

  let wasActive = $state(false);
  $effect(() => {
    const nextSourceKey = `${sessionId}:${contentGeneration}:${pageIndex}`;
    if (isMounted && sourceKey !== nextSourceKey) {
      sourceKey = nextSourceKey;
      invalidateThumbnail();
      if (!usesIntersectionObserver || isIntersecting) {
        void loadThumbnail();
      }
    }

    if (isActive && !wasActive && containerRef) {
      containerRef.scrollIntoView({ behavior: "smooth", block: "nearest" });
      wasActive = true;
    } else if (!isActive && wasActive) {
      wasActive = false;
    }
  });

  function handleDragOver(e) {
    e.preventDefault();
    isDragOver = true;
    onDragOver(e, displayIndex);
  }

  function handleDragLeave() {
    isDragOver = false;
  }

  function handleDrop(e) {
    e.preventDefault();
    isDragOver = false;
    onDrop(e, displayIndex);
  }
</script>

<div
  bind:this={containerRef}
  class="thumb-item {isActive ? 'active' : ''} {isDragOver ? 'drag-over' : ''}"
  draggable="true"
  ondragstart={(e) => onDragStart(e, displayIndex)}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
  onclick={() => onSelect(displayIndex)}
  role="button"
  tabindex="0"
  onkeydown={(e) => e.key === 'Enter' && onSelect(displayIndex)}
>
  <div class="thumb-canvas-wrapper">
    <button
      class="delete-btn"
      onclick={(event) => {
        event.stopPropagation();
        onDelete(displayIndex);
      }}
      title="Bu sayfayı sil"
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
        <polyline points="3 6 5 6 21 6"></polyline>
        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
      </svg>
    </button>

    {#if !isLoaded}
      <div class="thumb-placeholder">
        {#if isLoading}
          <div class="thumb-spinner"></div>
        {:else}
          <span class="placeholder-icon">📄</span>
        {/if}
      </div>
    {/if}
    <canvas
      bind:this={canvasRef}
      class={isLoaded ? 'loaded' : 'hidden'}
      aria-label="Sayfa {pageIndex + 1} önizleme"
    ></canvas>
  </div>
  <div class="thumb-label">
    <span>[PG: {String(displayIndex + 1).padStart(3, '0')}]</span>
  </div>
</div>

<style>
  .thumb-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px;
    border-radius: 0px;
    cursor: grab;
    background: var(--bg-surface);
    border: 1px solid var(--border-raw);
    border-left: 2px solid transparent;
    position: relative;
    font-family: var(--font-mono);
  }

  .thumb-item:hover {
    border-color: var(--border-focus);
    background: var(--bg-panel);
  }

  .thumb-item.active {
    border-left: 2px solid var(--accent-industrial);
    border-color: var(--border-focus);
    background: var(--bg-panel);
  }

  .thumb-item.drag-over {
    border-color: var(--accent-industrial);
    background: var(--bg-panel);
  }

  .thumb-canvas-wrapper {
    width: 140px;
    height: 180px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-primary);
    border: 1px solid var(--border-raw);
    overflow: hidden;
    position: relative;
  }

  .delete-btn {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 20px;
    height: 20px;
    background: var(--error-color);
    color: white;
    border: 1px solid var(--border-raw);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    opacity: 0;
    z-index: 10;
  }

  .thumb-item:hover .delete-btn {
    opacity: 1;
  }

  .delete-btn:hover {
    background: var(--error-color);
  }

  canvas {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border: none;
  }

  canvas.hidden {
    display: none;
  }

  .thumb-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    color: var(--text-muted);
  }

  .thumb-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border-raw);
    border-top-color: var(--accent-industrial);
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .thumb-label {
    margin-top: 4px;
    font-size: 10px;
    font-weight: 700;
    color: var(--text-muted);
    letter-spacing: 0.5px;
  }

  .thumb-item.active .thumb-label {
    color: var(--accent-industrial);
  }
</style>
