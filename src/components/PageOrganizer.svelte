<script>
  import { getContext, onMount, onDestroy, tick } from "svelte";
  import {
    cancelRenderRequest,
    renderThumbnail as renderThumbnailBackend,
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
  import { fade, fly } from "svelte/transition";
  import { SvelteMap } from "svelte/reactivity";
  import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
  import { rotatePage, deletePages, reorderPages } from "../pdfState.js";
  import { paintBufferToCanvas } from "../utils/canvasUtils.js";
  import {
    pageFailureMessage,
    pageFailureRegistry,
  } from "../services/pageFailureRegistry.js";
  import { pageMappingAfterDelete, pageMappingAfterReorder } from "../services/app/pageIndexMapping.js";
  import FeatureNotice from "./FeatureNotice.svelte";

  let {
    sessionId = "",
    contentGeneration = 0,
    pageCount = 0,
    onclose,
    onpageMutated,
    onorderSaved,
  } = $props();

  let pages = $state([]); // { id, index, canvas, rotation, status, error }
  let draggedIndex = $state(null);
  let isSaving = $state(false);
  let isMounted = $state(false);
  let loadedIdentityKey = $state("");
  let loadGeneration = 0;
  let thumbnailGeneration = 0;
  const pageRequestIds = new SvelteMap();
  const pageCoordinators = new SvelteMap();
  const renderSession = getContext(RENDER_SESSION_CONTEXT);

  function coordinatorFor(pageIndex) {
    if (!pageCoordinators.has(pageIndex)) {
      pageCoordinators.set(pageIndex, createRenderRequestCoordinator({
        scope: "organizer_thumbnail",
        cancel: (request, reason) => cancelRenderRequest({
          sessionId: request.sessionId,
          pageIndex: request.pageIndex,
          contentGeneration: request.contentGeneration,
          requestId: request.requestId,
          scope: request.scope,
        }, reason),
      }));
    }
    return pageCoordinators.get(pageIndex);
  }

  function releaseCanvasBackingStore(canvas) {
    if (!canvas) return;
    canvas.width = 0;
    canvas.height = 0;
  }

  function invalidatePageThumbnail(page) {
    coordinatorFor(page.index).cancelActive("rerender");
    const requestId = ++thumbnailGeneration;
    pageRequestIds.set(page.index, requestId);
    releaseCanvasBackingStore(page.canvas);
    return requestId;
  }

  function invalidateAllThumbnails() {
    loadGeneration += 1;
    thumbnailGeneration += 1;
    pageRequestIds.clear();
    pages.forEach((page) => releaseCanvasBackingStore(page.canvas));
    pageCoordinators.forEach((coordinator) => coordinator.destroy());
    pageCoordinators.clear();
  }

  function isValidRgbaPayload(rendered) {
    const width = Number(rendered?.width);
    const height = Number(rendered?.height);
    const data = rendered?.data;
    const hasPixelData = Array.isArray(data) || ArrayBuffer.isView(data);
    const dataLength = Number(data?.length);
    const expectedLength = width * height * 4;

    return (
      hasPixelData &&
      Number.isInteger(width) &&
      Number.isInteger(height) &&
      width > 0 &&
      height > 0 &&
      Number.isSafeInteger(expectedLength) &&
      expectedLength > 0 &&
      Number.isInteger(dataLength) &&
      dataLength === expectedLength
    );
  }

  function requestIsCurrent(
    page,
    sourceSessionId,
    sourceContentGeneration,
    requestId,
    canvas,
  ) {
    return (
      isMounted &&
      sessionId === sourceSessionId &&
      contentGeneration === sourceContentGeneration &&
      pageRequestIds.get(page.index) === requestId &&
      pages.includes(page) &&
      page.canvas === canvas
    );
  }

  function updatePageStatus(page, status, error = "") {
    if (!pages.includes(page)) return;
    page.status = status;
    page.error = error;
    pages = [...pages];
  }

  async function renderThumbnail(
    page,
    sourceSessionId = sessionId,
    sourceContentGeneration = contentGeneration,
  ) {
    const requestId = ++thumbnailGeneration;
    pageRequestIds.set(page.index, requestId);
    const canvas = page.canvas;
    const coordinator = coordinatorFor(page.index);
    let requestSnapshot;

    const source = {
      sessionId: sourceSessionId,
      contentGeneration: sourceContentGeneration,
      pageIndex: page.index,
      surface: "thumbnail",
    };
    const blockedBy = pageFailureRegistry.failure(source);
    if (blockedBy) {
      // Not requested again: each repeat would kill the shared PDF worker.
      updatePageStatus(page, "error", pageFailureMessage(blockedBy, page.index));
      return false;
    }

    updatePageStatus(page, "loading");
    if (!canvas) {
      if (
        requestIsCurrent(
          page,
          sourceSessionId,
          sourceContentGeneration,
          requestId,
          canvas,
        )
      ) {
        updatePageStatus(page, "error", "Minyatür canvas'ı hazırlanamadı.");
      }
      return false;
    }

    try {
      const session = requireActiveRenderSession(
        renderSession,
        sourceSessionId,
        sourceContentGeneration,
      );
      const requestHandle = coordinator.begin({
        sessionId: session.sessionId,
        pageIndex: page.index,
        targetWidth: 180,
        contentGeneration: session.contentGeneration,
      });
      requestSnapshot = requestHandle.snapshot;
      const rendered = validateRenderResponse(
        await requestHandle.wait(renderThumbnailBackend(requestSnapshot)),
        requestSnapshot,
      );
      if (
        !requestIsCurrent(
          page,
          sourceSessionId,
          sourceContentGeneration,
          requestId,
          canvas,
        ) ||
        !coordinator.isCurrent(requestSnapshot)
      ) return false;

      if (!isValidRgbaPayload(rendered)) {
        releaseCanvasBackingStore(canvas);
        updatePageStatus(page, "error", "Geçersiz minyatür verisi.");
        return false;
      }

      paintBufferToCanvas(canvas, rendered);
      if (
        !requestIsCurrent(
          page,
          sourceSessionId,
          sourceContentGeneration,
          requestId,
          canvas,
        )
      ) {
        releaseCanvasBackingStore(canvas);
        return false;
      }

      updatePageStatus(page, "ready");
      return true;
    } catch (error) {
      const recorded = pageFailureRegistry.record(source, error);
      if (
        requestIsCurrent(
          page,
          sourceSessionId,
          sourceContentGeneration,
          requestId,
          canvas,
        ) &&
        !isExpectedRenderCancellation(error)
      ) {
        releaseCanvasBackingStore(canvas);
        updatePageStatus(
          page,
          "error",
          recorded
            ? pageFailureMessage(error.code, page.index)
            : error?.message || String(error) || "Minyatür yüklenemedi.",
        );
      }
      return false;
    } finally {
      if (requestSnapshot) coordinator.settle(requestSnapshot);
    }
  }

  onMount(() => {
    isMounted = true;
    loadedIdentityKey = `${sessionId}:${contentGeneration}`;
    void loadThumbnails();
  });

  onDestroy(() => {
    isMounted = false;
    invalidateAllThumbnails();
  });

  async function loadThumbnails() {
    invalidateAllThumbnails();
    const currentLoadGeneration = loadGeneration;
    const sourceSessionId = sessionId;
    const sourceContentGeneration = contentGeneration;
    const sourcePageCount = pageCount;
    pages = Array.from({ length: sourcePageCount }, (_, index) => ({
      id: `${sourceSessionId}:${sourceContentGeneration}:${index}`,
      index,
      canvas: null,
      rotation: 0,
      status: "loading",
      error: "",
    }));
    await tick();
    if (
      !isMounted ||
      sessionId !== sourceSessionId ||
      contentGeneration !== sourceContentGeneration ||
      currentLoadGeneration !== loadGeneration
    ) {
      return;
    }

    for (const page of pages) {
      if (
        !isMounted ||
        sessionId !== sourceSessionId ||
        contentGeneration !== sourceContentGeneration ||
        currentLoadGeneration !== loadGeneration
      ) {
        return;
      }
      await renderThumbnail(page, sourceSessionId, sourceContentGeneration);
    }
  }

  $effect(() => {
    const identityKey = `${sessionId}:${contentGeneration}`;
    if (isMounted && identityKey !== loadedIdentityKey) {
      loadedIdentityKey = identityKey;
      void loadThumbnails();
    }
  });

  function handleDragStart(e, index) {
    draggedIndex = index;
    e.dataTransfer.effectAllowed = "move";
  }

  function handleDragOver(e, index) {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
  }

  function handleDrop(e, index) {
    e.preventDefault();
    if (draggedIndex !== null && draggedIndex !== index) {
      const newPages = [...pages];
      const [draggedItem] = newPages.splice(draggedIndex, 1);
      newPages.splice(index, 0, draggedItem);
      pages = newPages;
    }
    draggedIndex = null;
  }

  async function handleRotate(index, degrees) {
    const page = pages[index];
    if (!page) return;

    const sourceSessionId = sessionId;
    const sourceContentGeneration = contentGeneration;
    invalidatePageThumbnail(page);
    updatePageStatus(page, "loading");
    isSaving = true;
    try {
      const sessionSnapshot = await rotatePage(
        sourceSessionId,
        sourceContentGeneration,
        page.index,
        degrees,
      );
      if (
        !isMounted ||
        sessionId !== sourceSessionId ||
        contentGeneration !== sourceContentGeneration ||
        !pages.includes(page)
      ) return;

      page.rotation = ((page.rotation + degrees) % 360 + 360) % 360;
      if (Number.isSafeInteger(sessionSnapshot?.contentGeneration)) {
        contentGeneration = sessionSnapshot.contentGeneration;
      }
      onpageMutated?.({ snapshot: sessionSnapshot, rotation: { pageIndex: page.index, degrees } });
    } catch (e) {
      if (
        isMounted &&
        sessionId === sourceSessionId &&
        contentGeneration === sourceContentGeneration &&
        pages.includes(page)
      ) {
        releaseCanvasBackingStore(page.canvas);
        updatePageStatus(page, "error", e?.message || String(e));
        alert("Döndürme hatası: " + e);
      }
    } finally {
      if (isMounted) isSaving = false;
    }
  }

  async function handleDelete(index) {
    const confirmed = await confirmDialog(
      "Bu sayfayı silmek istediğinizden emin misiniz?",
      { title: "Sayfayı Sil", kind: "warning" },
    );
    if (!confirmed) return;
    isSaving = true;
    try {
      const page = pages[index];
      invalidatePageThumbnail(page);
      const sessionSnapshot = await deletePages(
        sessionId,
        contentGeneration,
        [page.index],
      );
      if (Number.isSafeInteger(sessionSnapshot?.contentGeneration)) {
        contentGeneration = sessionSnapshot.contentGeneration;
      }
      releaseCanvasBackingStore(page.canvas);
      pages.splice(index, 1);
      pages = [...pages];
      pageCount -= 1;
      onpageMutated?.({ snapshot: sessionSnapshot, mapPageIndex: pageMappingAfterDelete([page.index]) });
    } catch (e) {
      alert("Silme hatası: " + e);
    }
    isSaving = false;
  }

  async function handleSaveOrder() {
    isSaving = true;
    try {
      const newOrder = pages.map(p => p.index);
      const sessionSnapshot = await reorderPages(
        sessionId,
        contentGeneration,
        newOrder,
      );
      if (Number.isSafeInteger(sessionSnapshot?.contentGeneration)) {
        contentGeneration = sessionSnapshot.contentGeneration;
      }
      onorderSaved?.({ snapshot: sessionSnapshot, mapPageIndex: pageMappingAfterReorder(newOrder) });
    } catch(e) {
      alert("Sıralama kaydedilemedi: " + e);
    }
    isSaving = false;
  }
</script>

<div class="organizer-overlay" transition:fade={{duration: 200}}>
  <div class="organizer-container glass-panel" transition:fly={{y: 30, duration: 300}}>
    <FeatureNotice featureId="page_manipulation" compact />
    <div class="header">
      <h2>Sayfa Düzenleyici (Organizer)</h2>
      <div class="actions">
        <button class="tech-button primary" onclick={handleSaveOrder} disabled={isSaving}>
          {isSaving ? 'Kaydediliyor...' : 'Sıralamayı Kaydet'}
        </button>
        <button class="tech-button" onclick={() => onclose?.()} disabled={isSaving}>
          Kapat
        </button>
      </div>
    </div>

    <div class="grid-container" role="list">
      {#each pages as page, i (page.id)}
        <div
          class="page-card"
          role="listitem"
          draggable="true"
          ondragstart={(e) => handleDragStart(e, i)}
          ondragover={(e) => handleDragOver(e, i)}
          ondrop={(e) => handleDrop(e, i)}
          ondragend={() => draggedIndex = null}
          class:dragging={draggedIndex === i}
        >
          <div class="card-number">{i + 1}</div>
          <div class="thumbnail-frame">
            <canvas
              bind:this={page.canvas}
              class:hidden={page.status !== "ready"}
              aria-label="Sayfa {page.index + 1} minyatürü"
            ></canvas>
            {#if page.status !== "ready"}
              <div class="thumbnail-placeholder" class:error={page.status === "error"} title={page.error || undefined}>
                <span>{page.status === "error" ? "ÖNİZLEME YÜKLENEMEDİ" : "YÜKLENİYOR…"}</span>
              </div>
            {/if}
          </div>

          <div class="card-actions">
            <button class="icon-btn" onclick={() => handleRotate(i, -90)} title="Sola Döndür">⤺</button>
            <button class="icon-btn" onclick={() => handleRotate(i, 90)} title="Sağa Döndür">⤻</button>
            <button class="icon-btn delete-btn" onclick={() => handleDelete(i)} title="Sayfayı Sil">✕</button>
          </div>
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .organizer-overlay {
    position: fixed;
    top: 0; left: 0; width: 100vw; height: 100vh;
    background: rgba(10, 10, 15, 0.85);
    backdrop-filter: blur(8px);
    z-index: 9000;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .organizer-container {
    width: 90vw;
    height: 90vh;
    display: flex;
    flex-direction: column;
    padding: 24px;
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 24px;
    padding-bottom: 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  }

  .grid-container {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 24px;
    overflow-y: auto;
    padding: 12px;
  }

  .page-card {
    background: rgba(30, 30, 40, 0.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 12px;
    overflow: hidden;
    position: relative;
    cursor: grab;
    transition: all 0.2s;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 12px;
  }

  .page-card:hover {
    transform: translateY(-4px) scale(1.02);
    box-shadow: 0 10px 20px rgba(0,0,0,0.3);
    border-color: rgba(255,255,255,0.3);
  }

  .page-card:hover .card-actions {
    opacity: 1;
  }

  .page-card:active {
    cursor: grabbing;
  }

  .dragging {
    opacity: 0.5;
    transform: scale(0.95);
  }

  .card-number {
    position: absolute;
    top: 8px; left: 8px;
    background: rgba(0,0,0,0.6);
    color: white;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 12px;
    font-size: 12px;
  }

  .thumbnail-frame {
    width: 100%;
    height: 200px;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
    margin-bottom: 40px;
  }

  .thumbnail-frame canvas {
    max-width: 100%;
    max-height: 200px;
    object-fit: contain;
    box-shadow: 0 4px 12px rgba(0,0,0,0.2);
    background: white;
  }

  .thumbnail-frame canvas.hidden {
    visibility: hidden;
  }

  .thumbnail-placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px dashed rgba(255, 255, 255, 0.2);
    color: rgba(255, 255, 255, 0.55);
    font-family: var(--font-mono);
    font-size: 10px;
    text-align: center;
  }

  .thumbnail-placeholder.error {
    color: #f87171;
    border-color: rgba(248, 113, 113, 0.45);
  }

  .card-actions {
    position: absolute;
    bottom: 8px;
    left: 0; width: 100%;
    display: flex;
    justify-content: center;
    gap: 8px;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .icon-btn {
    width: 32px; height: 32px;
    border-radius: 50%;
    border: none;
    background: rgba(255,255,255,0.1);
    color: white;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 16px;
    transition: all 0.2s;
  }

  .icon-btn:hover {
    background: rgba(255,255,255,0.2);
    transform: scale(1.1);
  }

  .delete-btn:hover {
    background: #ef4444;
  }

  /* Scrollbar for grid */
  .grid-container::-webkit-scrollbar {
    width: 8px;
  }
  .grid-container::-webkit-scrollbar-track {
    background: rgba(0,0,0,0.1);
  }
  .grid-container::-webkit-scrollbar-thumb {
    background: rgba(255,255,255,0.2);
    border-radius: 4px;
  }
</style>
