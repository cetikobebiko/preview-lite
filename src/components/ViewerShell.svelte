<script>
  import FileTextIcon from "@lucide/svelte/icons/file-text";
  import { Button } from "$lib/components/ui/button";
  import PdfPage from "./PdfPage.svelte";
  import {
    DOCUMENT_QUARANTINED,
    pageFailureMessage,
    pageFailureRegistry,
  } from "../services/pageFailureRegistry.js";
  import { imageCanvasStyle } from "../services/app/viewerController.js";

  let {
    isPdf, viewer, annotation, documentSession, searchState, spreads, physicalPageIndex,
    prepress, featureUi, errorMsg, getActivePrepressPayload, getSensitiveDataHighlights = () => [],
    secureRedactionState, secureRedaction,
    onSelectFile, onReopenDocument = null, onPageIntersect, onSearchHighlightReady,
    onSessionSnapshot, onCanvasMouseDown, onCanvasMouseMove,
    onCanvasMouseUp, viewerAreaRef = $bindable(), canvasRefLeft = $bindable(),
  } = $props();

  // Per-page props are looked up from values derived once per state change, and
  // pages without data share one frozen empty array: a PdfPage whose prop value
  // keeps its identity does not re-run the reactive blocks that read it.
  const EMPTY = Object.freeze([]);
  const nonEmpty = (items) => (Array.isArray(items) && items.length > 0 ? items : EMPTY);
  const pendingRegionsByPage = $derived.by(() => {
    const byPage = new Map();
    for (const region of secureRedactionState?.pendingRegions ?? EMPTY) {
      const regions = byPage.get(region.pageIndex);
      if (regions) regions.push(region);
      else byPage.set(region.pageIndex, [region]);
    }
    return byPage;
  });
  // A prepress payload only ever belongs to the current physical page.
  const coveragePreviewPayload = $derived(getActivePrepressPayload(prepress.coveragePreview, physicalPageIndex, documentSession.activeSessionId, documentSession.contentGeneration, physicalPageIndex, "result"));
  const processPreviewPayload = $derived(getActivePrepressPayload(prepress.processPreview, physicalPageIndex, documentSession.activeSessionId, documentSession.contentGeneration, physicalPageIndex, "result"));
  // PdfPage reads currentPage only in single-page mode.
  const singlePageCurrent = $derived(viewer.isSinglePageMode ? viewer.pageOrder[viewer.currentPageIndex] ?? viewer.currentPageIndex : -1);

  // Set once any request of the active generation came back quarantined; no
  // page of it is requested again, so reopening is the way forward.
  const documentQuarantined = $derived(($pageFailureRegistry, isPdf &&
    pageFailureRegistry.isDocumentQuarantined(
      documentSession.activeSessionId,
      documentSession.contentGeneration,
    )));
</script>

<div class="viewer-area" bind:this={viewerAreaRef}>
  {#if prepress.processPreview || prepress.coveragePreview}
    <div class="approximate-preview-legend" role="status">APPROXIMATE RGB-DERIVED PREVIEW</div>
  {/if}
  {#if !isPdf && !viewer.activeImageSourcePath}
    <div class="empty-state m-auto flex flex-col items-center text-center">
      <FileTextIcon class="mb-4 size-14 text-muted-foreground/60" strokeWidth={1.25} />
      <h2 class="text-base font-semibold">Belge açılmadı</h2>
      <p class="mt-1 mb-5 max-w-xs text-[13px] text-muted-foreground">Bir PDF veya görseli bu pencereye sürükleyin ya da bilgisayarınızdan seçin.</p>
      <Button variant="outline" size="sm" onclick={onSelectFile}>Dosya Seç…</Button>
    </div>
  {:else}
    <!-- No transform transition: visualScale only returns to 1 in the same update that lays out the
         committed scale, so animating it would show the new layout at visualScale× for its duration. -->
    <div class="canvas-container {viewer.isBookMode || viewer.isSplitView ? 'book-mode' : ''} {viewer.isSmartDarkMode ? 'smart-dark-mode' : ''}" style="transform: scale({viewer.visualScale}); transform-origin: 50% {viewer.zoomOriginY ?? 0}px; will-change: transform;">
      {#if viewer.isRendering}<div class="loading-overlay" role="status"><div class="spinner"></div><p>Görüntüleniyor…</p></div>{/if}
      {#if errorMsg}<div class="error-card"><p>{errorMsg}</p></div>{/if}
      {#if documentQuarantined}
        <div class="error-card document-quarantine" role="alert">
          <p>{pageFailureMessage(DOCUMENT_QUARANTINED, 0)} Belgeyi yeniden açarak tekrar deneyebilirsiniz.</p>
          {#if onReopenDocument}<Button variant="outline" size="sm" class="mt-3" onclick={onReopenDocument}>Belgeyi yeniden aç</Button>{/if}
        </div>
      {/if}
      <div class="pdf-continuous-scroll" style="width: 100%; display: flex; flex-direction: column; align-items: center;">
        {#if isPdf}
          {#each spreads as spread (spread.join("-"))}
            <div class="book-spread-row {viewer.isBookMode ? 'book-mode' : ''}">
              {#each spread as pageIdx (pageIdx)}
                <PdfPage sessionId={documentSession.activeSessionId ?? ""} pageIndex={pageIdx} currentPage={singlePageCurrent} isSinglePageMode={viewer.isSinglePageMode} isZooming={viewer.isZooming} generationId={viewer.renderGenerationId} documentGenerationId={documentSession.contentGeneration} scale={viewer.scale} rotation={viewer.rotation} colorMode={viewer.isSmartDarkMode ? "dark" : "normal"} searchRequestId={searchState.requestId} searchMatches={nonEmpty(searchState.matchesByPage[pageIdx])} sensitiveDataHighlights={nonEmpty(getSensitiveDataHighlights(pageIdx, viewer.rotation))} secureRedactionEnabled={featureUi("redaction").runnable} secureRedactionSelectionMode={secureRedactionState?.selectionMode} pendingRedactionRegions={pendingRegionsByPage.get(pageIdx) ?? EMPTY} onRedactionRegionSelected={secureRedaction?.addManualRegion} onIntersect={onPageIntersect} onsearchHighlightReady={(detail) => onSearchHighlightReady({ detail })} activeTool={annotation.activeTool} activeColor={annotation.activeColor} approximateCoveragePreview={pageIdx === physicalPageIndex ? coveragePreviewPayload : null} approximateProcessPreview={pageIdx === physicalPageIndex ? processPreviewPayload : null} approximateProcessChannels={prepress.channels} onsessionSnapshot={onSessionSnapshot} />
              {/each}
            </div>
          {/each}
        {:else}
          <div class="page-wrapper"><canvas bind:this={canvasRefLeft} style={imageCanvasStyle(viewer.imagePixelSize, viewer.scale)} class={annotation.activeTool === "pointer" ? "pointer-mode" : ""} onmousedown={onCanvasMouseDown} onmousemove={onCanvasMouseMove} onmouseup={onCanvasMouseUp} onmouseleave={onCanvasMouseUp}></canvas></div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .book-spread-row {
    display: flex;
    justify-content: center;
    gap: 20px;
    width: 100%;
    margin-bottom: 24px;
    min-height: 800px;
  }
  .approximate-preview-legend { position: sticky; top: 8px; z-index: 100; margin: 0 auto -28px; width: max-content; padding: 4px 12px; border-radius: 999px; background: #ffcc00; color: #1d1d1f; font-size: 12px; font-weight: 600; box-shadow: 0 2px 8px rgb(0 0 0 / 0.15); pointer-events: none; }
</style>
