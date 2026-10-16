<script>
  import ChevronLeftIcon from "@lucide/svelte/icons/chevron-left";
  import ChevronRightIcon from "@lucide/svelte/icons/chevron-right";
  import RotateCcwIcon from "@lucide/svelte/icons/rotate-ccw";
  import RotateCwIcon from "@lucide/svelte/icons/rotate-cw";
  import ArrowLeftRightIcon from "@lucide/svelte/icons/arrow-left-right";
  import ArrowUpDownIcon from "@lucide/svelte/icons/arrow-up-down";
  import ZoomInIcon from "@lucide/svelte/icons/zoom-in";
  import ZoomOutIcon from "@lucide/svelte/icons/zoom-out";
  import ToolbarButton from "./ToolbarButton.svelte";
  import { MAX_SCALE, MIN_SCALE } from "../services/app/zoomLimits.js";

  let {
    isPdf, viewer, onPreviousPage, onNextPage,
    onRotateLeft, onRotateRight, onFlipH, onFlipV, onZoomOut, onResetZoom, onZoomIn,
  } = $props();

  const zoomPercent = $derived(Math.round(viewer.scale * viewer.visualScale * 100));
</script>

<div class="flex items-center gap-0.5">
  {#if isPdf}
    <ToolbarButton label="Önceki sayfa" onclick={onPreviousPage} disabled={viewer.currentPageIndex === 0}><ChevronLeftIcon /></ToolbarButton>
    <ToolbarButton label="Sonraki sayfa" onclick={onNextPage} disabled={viewer.currentPageIndex >= viewer.pageOrder.length - 1}><ChevronRightIcon /></ToolbarButton>
  {:else}
    <ToolbarButton label="Sola döndür" onclick={onRotateLeft}><RotateCcwIcon /></ToolbarButton>
    <ToolbarButton label="Sağa döndür" onclick={onRotateRight}><RotateCwIcon /></ToolbarButton>
    <ToolbarButton label="Yatay çevir" pressed={viewer.flipH} onclick={onFlipH}><ArrowLeftRightIcon /></ToolbarButton>
    <ToolbarButton label="Dikey çevir" pressed={viewer.flipV} onclick={onFlipV}><ArrowUpDownIcon /></ToolbarButton>
  {/if}
</div>

<div class="flex items-center gap-0.5">
  <ToolbarButton label="Uzaklaştır" onclick={onZoomOut} disabled={viewer.scale <= MIN_SCALE}><ZoomOutIcon /></ToolbarButton>
  <button
    type="button"
    class="h-7 min-w-12 rounded-md px-1.5 text-xs font-medium tabular-nums text-foreground/75 hover:bg-muted hover:text-foreground"
    title="Gerçek boyut"
    aria-label="Yakınlaştırmayı sıfırla ({zoomPercent}%)"
    onclick={onResetZoom}
  >{zoomPercent}%</button>
  <ToolbarButton label="Yakınlaştır" onclick={onZoomIn} disabled={viewer.scale >= MAX_SCALE}><ZoomInIcon /></ToolbarButton>
</div>
