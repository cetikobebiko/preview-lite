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
  import { createVisibilityDwell } from "../utils/visibilityDwell.js";
  import { findScrollRoot } from "../utils/scrollRoot.js";
  import {
    pageFailureMessage,
    pageFailureRegistry,
  } from "../services/pageFailureRegistry.js";
  import {
    createRenderRequestCoordinator,
    isExpectedRenderCancellation,
    validateRenderResponse,
  } from "../utils/renderRequestCoordinator.js";

  let {
    sessionId = "",
    contentGeneration = 0,
    pageIndex = 0,
    pageNumber = undefined,
    isActive = false,
    onSelect = () => {},
  } = $props();

  const LOAD_MARGIN = "200px";
  const RELEASE_MARGIN = "1000px";
  const RESIZE_DEBOUNCE_MS = 150;

  const displayedPageNumber = $derived(pageNumber ?? pageIndex + 1);
  const renderSession = getContext(RENDER_SESSION_CONTEXT);
  const coordinator = createRenderRequestCoordinator({
    scope: "sidebar_thumbnail",
    cancel: (request, reason) => cancelRenderRequest({
      sessionId: request.sessionId,
      pageIndex: request.pageIndex,
      contentGeneration: request.contentGeneration,
      requestId: request.requestId,
      scope: request.scope,
    }, reason),
  });

  let container = $state();
  let canvas = $state();
  let isVisible = $state(false);
  let loading = $state(false);
  let loadedKey = $state("");
  let observedSourceKey = $state("");
  let mounted = $state(false);
  // A worker-safety failure keeps the page from being requested again until
  // a retry in the viewer clears it or the generation changes.
  const failureCode = $derived(($pageFailureRegistry, pageFailureRegistry.failure({
    sessionId,
    contentGeneration,
    pageIndex,
    surface: "thumbnail",
  })));
  let wasBlocked = false;
  $effect(() => {
    const blocked = Boolean(failureCode);
    if (wasBlocked && !blocked && mounted && isVisible && !loading) {
      visibilityDwell.enter();
    }
    wasBlocked = blocked;
  });

  function measuredTargetWidth() {
    const cssWidth = container?.getBoundingClientRect?.().width || container?.clientWidth || 0;
    if (!Number.isFinite(cssWidth) || cssWidth <= 0) return 0;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    return Math.max(1, Math.min(16_384, Math.round(cssWidth * dpr)));
  }

  const visibilityDwell = createVisibilityDwell({
    onDwell: () => {
      if (
        mounted &&
        isVisible &&
        !loading &&
        loadedKey !== `${sessionId}:${contentGeneration}:${pageIndex}`
      ) {
        void loadThumbnail();
      }
    },
  });

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
        visibilityDwell.enter();
      } else if (!isVisible) {
        visibilityDwell.leave();
        coordinator.cancelActive("viewport_exit");
      }
    }, { rootMargin: LOAD_MARGIN });
    // A painted thumbnail is kept until it is far outside the list, so
    // scrolling back a little does not render it again. The margin needs the
    // list's scroll container as root; the viewport root would clip it away.
    const releaseObserver = new IntersectionObserver((entries) => {
      if (entries[0]?.isIntersecting === true) return;
      loadedKey = "";
      releaseCanvas();
    }, { root: findScrollRoot(container), rootMargin: RELEASE_MARGIN });
    // A panel drag resizes every step; render once the width has settled.
    let resizeTimer = null;
    const resizeObserver = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeTimer = null;
        if (!mounted || !loadedKey || loading) return;
        loadedKey = "";
        if (isVisible) {
          coordinator.cancelActive("rerender");
          void loadThumbnail();
        } else {
          releaseCanvas();
        }
      }, RESIZE_DEBOUNCE_MS);
    });
    if (container) {
      observer.observe(container);
      releaseObserver.observe(container);
      resizeObserver?.observe(container);
    }
    return () => {
      mounted = false;
      clearTimeout(resizeTimer);
      visibilityDwell.destroy();
      observer.disconnect();
      releaseObserver.disconnect();
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
      coordinator.cancelActive("rerender");
      loadedKey = "";
      releaseCanvas();
      if (isVisible) void loadThumbnail();
    });
  });

  async function loadThumbnail() {
    const targetWidth = measuredTargetWidth();
    if (!mounted || !isVisible || !canvas || targetWidth <= 0) return;
    const source = { sessionId, contentGeneration, pageIndex, surface: "thumbnail" };
    if (pageFailureRegistry.failure(source)) return;
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
      pageFailureRegistry.record(source, err);
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
  <canvas bind:this={canvas} aria-label="Page {displayedPageNumber}"></canvas>
  {#if !loadedKey}
    <div
      class="placeholder"
      class:failed={Boolean(failureCode)}
      title={failureCode ? pageFailureMessage(failureCode, pageIndex) : undefined}
    >
      <span>{failureCode ? "!" : displayedPageNumber}</span>
    </div>
  {/if}
  <div class="page-label">{displayedPageNumber}</div>
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
  .placeholder.failed {
    color: #ef4444;
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
