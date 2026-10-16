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
  import { createVisibilityDwell } from "../utils/visibilityDwell.js";
  import { findScrollRoot } from "../utils/scrollRoot.js";

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
  let releaseObserver = null;
  let isMounted = $state(false);
  let isIntersecting = $state(false);
  let usesIntersectionObserver = $state(false);
  let thumbnailRequest = $state(0);
  let sourceKey = $state("");
  const renderSession = getContext(RENDER_SESSION_CONTEXT);
  const renderCoordinator = createRenderRequestCoordinator({
    scope: "organizer_thumbnail",
    cancel: (request, reason) => cancelRenderRequest({
      sessionId: request.sessionId,
      pageIndex: request.pageIndex,
      contentGeneration: request.contentGeneration,
      requestId: request.requestId,
      scope: request.scope,
    }, reason),
  });

  function releaseCanvasBackingStore(canvas = canvasRef) {
    if (!canvas) return;
    canvas.width = 0;
    canvas.height = 0;
  }

  const visibilityDwell = createVisibilityDwell({
    onDwell: () => {
      if (isMounted && isIntersecting && !isLoaded && !isLoading) {
        void loadThumbnail();
      }
    },
  });

  function invalidateThumbnail(reason) {
    renderCoordinator.cancelActive(reason);
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
              visibilityDwell.enter();
            } else if (!isIntersecting) {
              visibilityDwell.leave();
              if (isLoading) invalidateThumbnail("viewport_exit");
            }
          });
        },
        { rootMargin: "100px" }
      );
      // A painted thumbnail is kept until it is far outside the grid, so
      // scrolling back a little does not render it again. The margin needs the
      // grid's scroll container as root; the viewport root would clip it away.
      releaseObserver = new IntersectionObserver(
        (entries) => {
          const entry = entries[entries.length - 1];
          if (entry?.isIntersecting) return;
          if (isLoaded || isLoading || canvasRef?.width || canvasRef?.height) {
            invalidateThumbnail("viewport_exit");
          }
        },
        { root: findScrollRoot(containerRef), rootMargin: "1000px" }
      );
      if (containerRef) {
        observer.observe(containerRef);
        releaseObserver.observe(containerRef);
      }
    } else {
      void loadThumbnail();
    }
  });

  onDestroy(() => {
    isMounted = false;
    isIntersecting = false;
    visibilityDwell.destroy();
    invalidateThumbnail("destroy");
    renderCoordinator.destroy();
    if (observer) {
      if (containerRef) observer.unobserve(containerRef);
      observer.disconnect();
    }
    releaseObserver?.disconnect();
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
      invalidateThumbnail("rerender");
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
    background: var(--card);
    border: 1px solid var(--border);
    border-left: 2px solid transparent;
    position: relative;
  }

  .thumb-item:hover {
    border-color: var(--ring);
    background: var(--secondary);
  }

  .thumb-item.active {
    border-left: 2px solid var(--primary);
    border-color: var(--ring);
    background: var(--secondary);
  }

  .thumb-item.drag-over {
    border-color: var(--primary);
    background: var(--secondary);
  }

  .thumb-canvas-wrapper {
    width: 140px;
    height: 180px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--background);
    border: 1px solid var(--border);
    overflow: hidden;
    position: relative;
  }

  .delete-btn {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 20px;
    height: 20px;
    background: var(--destructive);
    color: white;
    border: 1px solid var(--border);
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
    background: var(--destructive);
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
    color: var(--muted-foreground);
  }

  .thumb-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--primary);
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .thumb-label {
    margin-top: 4px;
    font-size: 10px;
    font-weight: 700;
    color: var(--muted-foreground);
    letter-spacing: 0.5px;
  }

  .thumb-item.active .thumb-label {
    color: var(--primary);
  }
</style>
