<script>
  let {
    isPdf, viewer, onGoBack, onGoForward, onPreviousPage, onNextPage, onPageInputChange,
    onRotateLeft, onRotateRight, onFlipH, onFlipV, onZoomOut, onResetZoom, onZoomIn,
  } = $props();
</script>

{#if isPdf}
  <div class="hud-group">
    <button class="hud-btn" onclick={onGoBack} disabled={viewer.historyStack.length === 0}>[BACK]</button>
    <button class="hud-btn" onclick={onGoForward} disabled={viewer.forwardStack.length === 0}>[FWD]</button>
    <button class="hud-btn" onclick={onPreviousPage} disabled={viewer.currentPageIndex === 0}>[PREV]</button>
    <div class="hud-telemetry-badge">
      <span>PG:</span>
      <input type="number" class="hud-page-input" value={viewer.pageOrder.length > 0 ? viewer.currentPageIndex + 1 : 0} min="1" max={Math.max(1, viewer.pageOrder.length)} disabled={viewer.pageOrder.length === 0} onchange={onPageInputChange} />
      <span>/ {String(viewer.pageOrder.length).padStart(3, "0")}</span>
    </div>
    <button class="hud-btn" onclick={onNextPage} disabled={viewer.currentPageIndex >= viewer.pageOrder.length - 1}>[NEXT]</button>
  </div>
{:else}
  <div class="hud-group">
    <button class="hud-btn" onclick={onRotateLeft}>[SOLA 90°]</button>
    <button class="hud-btn" onclick={onRotateRight}>[SAĞA 90°]</button>
    <button class="hud-btn {viewer.flipH ? 'active' : ''}" onclick={onFlipH}>[FLIP_H]</button>
    <button class="hud-btn {viewer.flipV ? 'active' : ''}" onclick={onFlipV}>[FLIP_V]</button>
  </div>
{/if}

<div class="hud-group">
  <button class="hud-btn" onclick={onZoomOut} disabled={viewer.scale <= 0.25}>[Z:-]</button>
  <button class="hud-btn" onclick={onResetZoom}>[Z: {Math.round(viewer.scale * viewer.visualScale * 100)}%]</button>
  <button class="hud-btn" onclick={onZoomIn} disabled={viewer.scale >= 4}>[Z:+]</button>
</div>
