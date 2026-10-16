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
  import RotateCcwIcon from "@lucide/svelte/icons/rotate-ccw";
  import RotateCwIcon from "@lucide/svelte/icons/rotate-cw";
  import Trash2Icon from "@lucide/svelte/icons/trash-2";
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
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

  // Release a thumbnail canvas when its element is removed. The canvases live
  // inside the dialog's snippet, which tears down before onDestroy runs, so
  // their bound references are already null by then.
  function releaseOnRemoval(canvas) {
    return () => releaseCanvasBackingStore(canvas);
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
      onpageMutated?.({ snapshot: sessionSnapshot });
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

<AppDialog
  title="Sayfa Düzenleyici (Organizer)"
  description="Sıralamak için sayfaları sürükleyin."
  size="xl"
  class="h-[85vh] sm:max-w-[min(1100px,92vw)]"
  bodyClass="pt-1"
  dismissible={!isSaving}
  onclose={() => onclose?.()}
>
  <FeatureNotice featureId="page_manipulation" compact />

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
        <div class="thumbnail-frame">
          <canvas
            bind:this={page.canvas}
            {@attach releaseOnRemoval}
            class:hidden={page.status !== "ready"}
            aria-label="Sayfa {page.index + 1} minyatürü"
          ></canvas>
          {#if page.status !== "ready"}
            <div class="thumbnail-placeholder" class:error={page.status === "error"} title={page.error || undefined}>
              <span>{page.status === "error" ? "Önizleme yüklenemedi" : "Yükleniyor…"}</span>
            </div>
          {/if}
        </div>

        <div class="card-footer">
          <span class="card-number">{i + 1}</span>
          <div class="card-actions">
            <button type="button" class="icon-btn" onclick={() => handleRotate(i, -90)} title="Sola Döndür" aria-label="Sola Döndür"><RotateCcwIcon class="size-3.5" /></button>
            <button type="button" class="icon-btn" onclick={() => handleRotate(i, 90)} title="Sağa Döndür" aria-label="Sağa Döndür"><RotateCwIcon class="size-3.5" /></button>
            <button type="button" class="icon-btn delete-btn" onclick={() => handleDelete(i)} title="Sayfayı Sil" aria-label="Sayfayı Sil"><Trash2Icon class="size-3.5" /></button>
          </div>
        </div>
      </div>
    {/each}
  </div>

  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={() => onclose?.()} disabled={isSaving}>Kapat</Button>
    <Button size="sm" onclick={handleSaveOrder} disabled={isSaving}>
      {isSaving ? 'Kaydediliyor...' : 'Sıralamayı Kaydet'}
    </Button>
  {/snippet}
</AppDialog>

<style>
  .grid-container {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 16px;
    padding: 4px 2px;
  }

  .page-card {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 10px 10px 8px;
    border-radius: var(--radius);
    cursor: grab;
    transition: background-color 0.1s;
  }

  .page-card:hover {
    background: var(--muted);
  }

  .page-card:active {
    cursor: grabbing;
  }

  .dragging {
    opacity: 0.4;
  }

  .thumbnail-frame {
    position: relative;
    width: 100%;
    height: 180px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .thumbnail-frame canvas {
    max-width: 100%;
    max-height: 180px;
    object-fit: contain;
    background: #ffffff;
    box-shadow: 0 0 0 1px rgb(0 0 0 / 0.1), 0 1px 3px rgb(0 0 0 / 0.12);
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
    border-radius: calc(var(--radius) * 0.6);
    background: var(--muted);
    color: var(--muted-foreground);
    font-size: 10px;
    text-align: center;
  }

  .thumbnail-placeholder.error {
    color: var(--destructive);
    background: color-mix(in srgb, var(--destructive) 8%, transparent);
  }

  .card-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: 26px;
  }

  .card-number {
    min-width: 20px;
    padding: 0 6px;
    border-radius: 999px;
    color: var(--muted-foreground);
    font-size: 11px;
    line-height: 18px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .card-actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 0.1s;
  }

  .page-card:hover .card-actions,
  .page-card:focus-within .card-actions {
    opacity: 1;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: calc(var(--radius) * 0.8);
    color: var(--muted-foreground);
    cursor: pointer;
  }

  .icon-btn:hover {
    background: var(--accent);
    color: var(--foreground);
  }

  .delete-btn:hover {
    background: color-mix(in srgb, var(--destructive) 12%, transparent);
    color: var(--destructive);
  }
</style>
