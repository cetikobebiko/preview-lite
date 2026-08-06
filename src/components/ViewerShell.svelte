<script>
  import PdfPage from "./PdfPage.svelte";

  let {
    isPdf, viewer, annotation, documentSession, searchState, spreads, physicalPageIndex,
    prepress, featureUi, errorMsg, getActivePrepressPayload, getSensitiveDataHighlights = () => [],
    onSelectFile, onPageIntersect, onSearchHighlightReady,
    onSessionSnapshot, onCanvasMouseDown, onCanvasMouseMove,
    onCanvasMouseUp, viewerAreaRef = $bindable(), canvasRefLeft = $bindable(),
  } = $props();
</script>

<div class="viewer-area" bind:this={viewerAreaRef}>
  {#if !isPdf && !viewer.activeImageSourcePath}
    <button class="drop-zone" onclick={onSelectFile} style="all: unset; display: flex; flex-direction: column; align-items: center; padding: 48px 36px; border: 1px dashed var(--border-raw); background: var(--bg-surface); text-align: center; cursor: pointer; font-family: var(--font-mono);">
      <div style="width: 56px; height: 56px; border: 1px solid var(--border-raw); color: var(--accent-industrial); display: flex; align-items: center; justify-content: center; margin-bottom: 16px; font-weight: 700; font-size: 20px;">[+]</div>
      <h2 style="font-size: 16px; font-weight: 700; margin-bottom: 6px; color: var(--text-primary); text-transform: uppercase;">// DOSYA_YÜKLEME_MERKEZİ</h2>
      <p style="color: var(--text-muted); font-size: 12px; margin-bottom: 20px; max-width: 340px;">Görüntülemek için bilgisayarınızdan PDF veya görsel belgesi seçin ya da pencereye sürükleyin</p>
      <span class="hud-btn active" style="padding: 4px 16px;">[ DOSYA SEÇ ]</span>
    </button>
  {:else}
    <div class="canvas-container {viewer.isBookMode || viewer.isSplitView ? 'book-mode' : ''} {viewer.isSmartDarkMode ? 'smart-dark-mode' : ''}" style="transform: scale({viewer.visualScale}); transform-origin: top center; will-change: transform; transition: {viewer.isZooming ? 'none' : 'transform 0.2s ease-out'};">
      {#if viewer.isRendering}<div class="loading-overlay"><div class="spinner"></div><p style="font-size: 11px; font-weight: 700; color: var(--accent-industrial);">// RENDER_İŞLENİYOR...</p></div>{/if}
      {#if errorMsg}<div class="error-card"><p>{errorMsg}</p></div>{/if}
      <div class="pdf-continuous-scroll" style="width: 100%; display: flex; flex-direction: column; align-items: center;">
        {#if isPdf}
          {#each spreads as spread (spread.join("-"))}
            <div class="book-spread-row {viewer.isBookMode ? 'book-mode' : ''}">
              {#each spread as pageIdx (pageIdx)}
                <PdfPage sessionId={documentSession.activeSessionId ?? ""} pageIndex={pageIdx} currentPage={viewer.pageOrder[viewer.currentPageIndex] ?? viewer.currentPageIndex} isBookMode={viewer.isBookMode} isSinglePageMode={viewer.isSinglePageMode} generationId={viewer.renderGenerationId} documentGenerationId={documentSession.contentGeneration} scale={viewer.scale} rotation={viewer.rotation} colorMode={viewer.isSmartDarkMode ? "dark" : "normal"} searchRequestId={searchState.requestId} searchMatches={searchState.matchesByPage[pageIdx] || []} sensitiveDataHighlights={getSensitiveDataHighlights(pageIdx, viewer.rotation)} onIntersect={onPageIntersect} on:searchHighlightReady={onSearchHighlightReady} activeTool={annotation.activeTool} activeColor={annotation.activeColor} prepressHeatmap={getActivePrepressPayload(prepress.heatmap, pageIdx, documentSession.activeSessionId, documentSession.contentGeneration, physicalPageIndex, "heatmap")} prepressSeparations={getActivePrepressPayload(prepress.separations, pageIdx, documentSession.activeSessionId, documentSession.contentGeneration, physicalPageIndex, "images")} prepressChannels={prepress.channels} prepressBoxes={prepress.boxes} on:sessionSnapshot={(event) => onSessionSnapshot(event.detail)} />
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
</style>
