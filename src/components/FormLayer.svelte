<script>
  let { scale = 1.0 } = $props();
</script>

<div
  class="form-layer"
  style="transform: scale({scale}); transform-origin: top left;"
  aria-hidden="true"
></div>

<style>
  .form-layer {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none; /* Arka plandaki etkileşimleri engellememek için */
  }
  
</style>
