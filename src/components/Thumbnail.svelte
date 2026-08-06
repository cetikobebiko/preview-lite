<script>
  import { getContext, onMount } from "svelte";
  import {
    cancelRenderRequest,
    renderThumbnail,
  } from "../services/backend/viewer.js";
  import {
    RENDER_SESSION_CONTEXT,
    requireActiveRenderSession,
  } from "../services/renderSession.js";
  import { paintBufferToCanvas } from "../utils/canvasUtils.js";
  import {
    createRenderRequestCoordinator,
    isExpectedRenderCancellation,
    validateRenderResponse,
  } from "../utils/renderRequestCoordinator.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    isActive = false,
    onSelect = () => {},
  } = $props();

  const renderSession = getContext(RENDER_SESSION_CONTEXT);
  const coordinator = createRenderRequestCoordinator({
    scope: "sidebar_thumbnail",
    cancel: (request) => cancelRenderRequest({
      sessionId: request.sessionId,
      pageIndex: request.pageIndex,
      contentGeneration: request.contentGeneration,
      requestId: request.requestId,
      scope: request.scope,
    }),
  });

  let container = $state();
  let canvas = $state();
  let isVisible = $state(false);
  let loading = $state(false);
  let loadedKey = $state("");
  let observedSourceKey = $state("");
  let mounted = $state(false);

  function measuredTargetWidth() {
    const cssWidth = container?.getBoundingClientRect?.().width || container?.clientWidth || 0;
    if (!Number.isFinite(cssWidth) || cssWidth <= 0) return 0;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    return Math.max(1, Math.min(16_384, Math.round(cssWidth * dpr)));
  }

  function releaseCanvas() {
    if (!canvas) return;
    canvas.width = 0;
    canvas.height = 0;
  }

  onMount(() => {
    observedSourceKey = `${sessionId}:${contentGeneration}:${pageIndex}`;
    mounted = true;
    const observer = new IntersectionObserver((entries) => {
      isVisible = entries[0]?.isIntersecting === true;
      if (
        isVisible &&
        !loading &&
        loadedKey !== `${sessionId}:${contentGeneration}:${pageIndex}`
      ) {
        void loadThumbnail();
      } else if (!isVisible) {
        coordinator.cancelActive();
        loadedKey = "";
        releaseCanvas();
      }
    }, { rootMargin: "200px" });
    const resizeObserver = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      if (isVisible && loadedKey && !loading) {
        loadedKey = "";
        coordinator.cancelActive();
        void loadThumbnail();
      }
    });
    if (container) {
      observer.observe(container);
      resizeObserver?.observe(container);
    }
    return () => {
      mounted = false;
      observer.disconnect();
      resizeObserver?.disconnect();
      coordinator.destroy();
      releaseCanvas();
    };
  });

  $effect(() => {
    const nextKey = `${sessionId}:${contentGeneration}:${pageIndex}`;
    if (!mounted || observedSourceKey === nextKey) return;
    observedSourceKey = nextKey;
    queueMicrotask(() => {
      coordinator.cancelActive();
      loadedKey = "";
      releaseCanvas();
      if (isVisible) void loadThumbnail();
    });
  });

  async function loadThumbnail() {
    const targetWidth = measuredTargetWidth();
    if (!mounted || !isVisible || !canvas || targetWidth <= 0) return;
    loading = true;
    let requestSnapshot;
    try {
      const session = requireActiveRenderSession(
        renderSession,
        sessionId,
        contentGeneration,
      );
      const request = coordinator.begin({
        sessionId: session.sessionId,
        pageIndex,
        targetWidth,
        contentGeneration: session.contentGeneration,
      });
      requestSnapshot = request.snapshot;
      const rendered = validateRenderResponse(
        await request.wait(renderThumbnail(requestSnapshot)),
        requestSnapshot,
      );
      if (
        !coordinator.isCurrent(requestSnapshot) ||
        !mounted ||
        !isVisible ||
        sessionId !== requestSnapshot.sessionId ||
        contentGeneration !== requestSnapshot.contentGeneration ||
        pageIndex !== requestSnapshot.pageIndex
      ) {
        return;
      }
      paintBufferToCanvas(canvas, rendered);
      loadedKey = `${sessionId}:${contentGeneration}:${pageIndex}`;
    } catch (err) {
      if (!isExpectedRenderCancellation(err)) {
        console.error("Thumbnail load error", err);
      }
    } finally {
      if (requestSnapshot) coordinator.settle(requestSnapshot);
      loading = false;
    }
  }
</script>

<div
  bind:this={container}
  class="thumbnail-container {isActive ? 'active' : ''}"
  onclick={onSelect}
  onkeydown={(event) => {
    if (event.key === "Enter" || event.key === " ") onSelect();
  }}
  role="button"
  tabindex="0"
>
  <canvas bind:this={canvas} aria-label="Page {pageIndex + 1}"></canvas>
  {#if !loadedKey}
    <div class="placeholder">
      <span>{pageIndex + 1}</span>
    </div>
  {/if}
  <div class="page-label">{pageIndex + 1}</div>
</div>

<style>
  .thumbnail-container {
    width: 100%;
    aspect-ratio: 1 / 1.414;
    background: #1e1e1e;
    border: 2px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    overflow: hidden;
    position: relative;
    transition: all 0.2s;
  }
  .thumbnail-container:hover {
    border-color: #4b5563;
  }
  .thumbnail-container.active {
    border-color: #3b82f6;
    box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.4);
  }
  canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    background: white;
  }
  .placeholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #4b5563;
    font-size: 18px;
    font-weight: bold;
    background: #2a2a2a;
  }
  .page-label {
    position: absolute;
    bottom: 4px;
    right: 4px;
    background: rgba(0,0,0,0.6);
    color: white;
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 4px;
    backdrop-filter: blur(2px);
  }
</style>
