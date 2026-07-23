<script>
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { pdfState, renderPdfThumbnail, cleanupCanvasMemory } from "../pdfState.js";

  export let filePath = "";
  export let pageIndex = 0;
  export let displayIndex = 0;
  export let isActive = false;
  export let onSelect = () => {};
  export let onDelete = () => {};
  export let onDragStart = () => {};
  export let onDragOver = () => {};
  export let onDrop = () => {};

  let containerRef;
  let canvasRef;
  let isLoaded = false;
  let isLoading = false;
  let isDragOver = false;
  let observer;
  let isMounted = false;
  let thumbnailRequest = 0;
  let sourceKey = "";

  onMount(() => {
    isMounted = true;
    sourceKey = `${filePath}:${pageIndex}`;
    if (typeof IntersectionObserver !== "undefined") {
      observer = new IntersectionObserver(
        (entries) => {
          entries.forEach((entry) => {
            if (entry.isIntersecting && !isLoaded && !isLoading) {
              void loadThumbnail();
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
    thumbnailRequest += 1;
    if (observer) {
      if (containerRef) observer.unobserve(containerRef);
      observer.disconnect();
    }
    if (canvasRef) {
      cleanupCanvasMemory(canvasRef);
    }
  });

  async function loadThumbnail() {
    if (!filePath || !canvasRef) return;
    const request = ++thumbnailRequest;
    const sourceFilePath = filePath;
    const sourcePageIndex = pageIndex;
    const canvas = canvasRef;
    const isCurrentRequest = () =>
      isMounted &&
      request === thumbnailRequest &&
      canvasRef === canvas &&
      filePath === sourceFilePath &&
      pageIndex === sourcePageIndex;

    isLoading = true;
    try {
      const documentProxy = pdfState.pdfDocProxy;
      if (documentProxy) {
        const success = await renderPdfThumbnail(sourcePageIndex, canvas, 140);
        if (!isCurrentRequest() || pdfState.pdfDocProxy !== documentProxy) return;
        if (success) {
          isLoaded = true;
          return;
        }
      }

      const rendered = await invoke("render_thumbnail", {
        path: sourceFilePath,
        pageIndex: sourcePageIndex,
        targetWidth: 150,
      });
      if (!isCurrentRequest()) return;

      const { width, height, data } = rendered;
      if (!Number.isFinite(width) || !Number.isFinite(height) || !data) {
        throw new Error("Geçersiz küçük resim verisi.");
      }

      const safeWidth = Math.max(1, Math.floor(width));
      const safeHeight = Math.max(1, Math.floor(height));
      if (safeWidth * safeHeight * 4 > data.length) {
        throw new Error("Eksik küçük resim piksel verisi.");
      }
      canvas.width = safeWidth;
      canvas.height = safeHeight;

      const ctx = canvas.getContext("2d", { alpha: false });
      if (!ctx) throw new Error("Canvas 2D bağlamı oluşturulamadı.");
      const clampedData = new Uint8ClampedArray(data);
      const imageData = new ImageData(clampedData, safeWidth, safeHeight);
      ctx.putImageData(imageData, 0, 0);

      isLoaded = true;
    } catch (err) {
      if (isCurrentRequest() && err?.name !== "RenderingCancelledException" && err?.name !== "AbortException") {
        console.error(`Thumbnail ${sourcePageIndex} render error:`, err);
      }
    } finally {
      if (isCurrentRequest()) isLoading = false;
    }
  }

  $: if (isMounted && sourceKey !== `${filePath}:${pageIndex}`) {
    sourceKey = `${filePath}:${pageIndex}`;
    thumbnailRequest += 1;
    isLoaded = false;
    isLoading = false;
    if (canvasRef) cleanupCanvasMemory(canvasRef);
    void loadThumbnail();
  }

  let wasActive = false;
  $: if (isActive && !wasActive && containerRef) {
    containerRef.scrollIntoView({ behavior: "smooth", block: "nearest" });
    wasActive = true;
  } else if (!isActive && wasActive) {
    wasActive = false;
  }

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
  on:dragstart={(e) => onDragStart(e, displayIndex)}
  on:dragover={handleDragOver}
  on:dragleave={handleDragLeave}
  on:drop={handleDrop}
  on:click={() => onSelect(displayIndex)}
  role="button"
  tabindex="0"
  on:keydown={(e) => e.key === 'Enter' && onSelect(displayIndex)}
>
  <div class="thumb-canvas-wrapper">
    <button
      class="delete-btn"
      on:click|stopPropagation={() => onDelete(displayIndex)}
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
    <canvas bind:this={canvasRef} class={isLoaded ? 'loaded' : 'hidden'}></canvas>
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
