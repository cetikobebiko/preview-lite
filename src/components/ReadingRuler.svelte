<script>
  let { isActive = false } = $props();
  let mouseY = $state(300);
  const rulerHeight = 44;

  function handleMouseMove(e) {
    if (isActive) {
      mouseY = e.clientY;
    }
  }
</script>

<svelte:window onmousemove={handleMouseMove} />

{#if isActive}
  <div class="ruler-container">
    <div class="ruler-mask-top" style="height: {Math.max(0, mouseY - rulerHeight / 2)}px;"></div>
    <div class="ruler-focus-line" style="top: {mouseY - rulerHeight / 2}px; height: {rulerHeight}px;">
      <div class="ruler-guide-line"></div>
    </div>
    <div class="ruler-mask-bottom" style="top: {mouseY + rulerHeight / 2}px;"></div>
  </div>
{/if}

<style>
  .ruler-container {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    pointer-events: none;
    z-index: 80;
  }

  .ruler-mask-top,
  .ruler-mask-bottom {
    position: absolute;
    left: 0;
    right: 0;
    background: rgba(10, 10, 14, 0.65);
    transition: height 0.05s ease-out, top 0.05s ease-out;
  }

  .ruler-mask-bottom {
    bottom: 0;
  }

  .ruler-focus-line {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 1px solid var(--primary);
    border-bottom: 1px solid var(--primary);
    pointer-events: none;
    transition: top 0.05s ease-out;
  }

  .ruler-guide-line {
    width: 100%;
    height: 100%;
    background: color-mix(in srgb, var(--primary) 6%, transparent);
  }
</style>
