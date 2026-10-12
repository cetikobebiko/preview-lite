<script>
  import PdfPage from "./PdfPage.svelte";
  import {
    DOCUMENT_QUARANTINED,
    pageFailureMessage,
    pageFailureRegistry,
  } from "../services/pageFailureRegistry.js";

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
    <button class="drop-zone" onclick={onSelectFile} style="all: unset; display: flex; flex-direction: column; align-items: center; padding: 48px 36px; border: 1px dashed var(--border-raw); background: var(--bg-surface); text-align: center; cursor: pointer; font-family: var(--font-mono);">
      <div style="width: 56px; height: 56px; border: 1px solid var(--border-raw); color: var(--accent-industrial); display: flex; align-items: center; justify-content: center; margin-bottom: 16px; font-weight: 700; font-size: 20px;">[+]</div>
      <h2 style="font-size: 16px; font-weight: 700; margin-bottom: 6px; color: var(--text-primary); text-transform: uppercase;">// DOSYA_YÜKLEME_MERKEZİ</h2>
      <p style="color: var(--text-muted); font-size: 12px; margin-bottom: 20px; max-width: 340px;">Görüntülemek için bilgisayarınızdan PDF veya görsel belgesi seçin ya da pencereye sürükleyin</p>
      <span class="hud-btn active" style="padding: 4px 16px;">[ DOSYA SEÇ ]</span>
    </button>
  {:else}
    <div class="canvas-container {viewer.isBookMode || viewer.isSplitView ? 'book-mode' : ''} {viewer.isSmartDarkMode ? 'smart-dark-mode' : ''}" style="transform: scale({viewer.visualScale}); transform-origin: 50% {viewer.zoomOriginY ?? 0}px; will-change: transform; transition: {viewer.isZooming ? 'none' : 'transform 0.2s ease-out'};">
      {#if viewer.isRendering}<div class="loading-overlay"><div class="spinner"></div><p style="font-size: 11px; font-weight: 700; color: var(--accent-industrial);">// RENDER_İŞLENİYOR...</p></div>{/if}
      {#if errorMsg}<div class="error-card"><p>{errorMsg}</p></div>{/if}
      {#if documentQuarantined}
        <div class="error-card document-quarantine" role="alert">
          <p>{pageFailureMessage(DOCUMENT_QUARANTINED, 0)} Belgeyi yeniden açarak tekrar deneyebilirsiniz.</p>
          {#if onReopenDocument}<button class="hud-btn" onclick={onReopenDocument}>[ BELGEYİ YENİDEN AÇ ]</button>{/if}
        </div>
      {/if}
      <div class="pdf-continuous-scroll" style="width: 100%; display: flex; flex-direction: column; align-items: center;">
        {#if isPdf}
          {#each spreads as spread (spread.join("-"))}
            <div class="book-spread-row {viewer.isBookMode ? 'book-mode' : ''}">
              {#each spread as pageIdx (pageIdx)}
                <PdfPage sessionId={documentSession.activeSessionId ?? ""} pageIndex={pageIdx} currentPage={singlePageCurrent} isBookMode={viewer.isBookMode} isSinglePageMode={viewer.isSinglePageMode} generationId={viewer.renderGenerationId} documentGenerationId={documentSession.contentGeneration} scale={viewer.scale} rotation={viewer.rotation} colorMode={viewer.isSmartDarkMode ? "dark" : "normal"} searchRequestId={searchState.requestId} searchMatches={nonEmpty(searchState.matchesByPage[pageIdx])} sensitiveDataHighlights={nonEmpty(getSensitiveDataHighlights(pageIdx, viewer.rotation))} secureRedactionEnabled={featureUi("redaction").runnable} secureRedactionSelectionMode={secureRedactionState?.selectionMode} pendingRedactionRegions={pendingRegionsByPage.get(pageIdx) ?? EMPTY} onRedactionRegionSelected={secureRedaction?.addManualRegion} onIntersect={onPageIntersect} on:searchHighlightReady={onSearchHighlightReady} activeTool={annotation.activeTool} activeColor={annotation.activeColor} approximateCoveragePreview={pageIdx === physicalPageIndex ? coveragePreviewPayload : null} approximateProcessPreview={pageIdx === physicalPageIndex ? processPreviewPayload : null} approximateProcessChannels={prepress.channels} on:sessionSnapshot={(event) => onSessionSnapshot(event.detail)} />
              {/each}
            </div>
          {/each}
        {:else}
          <div class="page-wrapper"><canvas bind:this={canvasRefLeft} class={annotation.activeTool === "pointer" ? "pointer-mode" : ""} onmousedown={onCanvasMouseDown} onmousemove={onCanvasMouseMove} onmouseup={onCanvasMouseUp} onmouseleave={onCanvasMouseUp}></canvas></div>
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
  .approximate-preview-legend { position: sticky; top: 8px; z-index: 100; margin: 8px auto -36px; width: max-content; padding: 7px 12px; background: #f59e0b; color: #111; border: 2px solid #111; font: 800 12px var(--font-mono); pointer-events: none; }
</style>
