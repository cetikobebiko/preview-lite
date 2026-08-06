<script>
  import {
    getFeaturePresentation,
    getRuntimeFeatureContext,
  } from "../featureRegistry.js";

  let {
    featureId,
    context = getRuntimeFeatureContext(),
    compact = false,
  } = $props();

  const presentation = $derived(getFeaturePresentation(featureId, context));
</script>

{#if presentation.visible && (presentation.badge || presentation.warning)}
  <div
    class="feature-notice"
    class:compact
    role="note"
    data-feature-id={featureId}
    data-feature-status={presentation.status}
  >
    <strong>
      {presentation.feature?.label || featureId}
      {#if presentation.badge}
        · {presentation.badge}
      {/if}
    </strong>
    {#if presentation.warning}
      <span>{presentation.warning}</span>
    {/if}
  </div>
{/if}

<style>
  .feature-notice {
    display: grid;
    gap: 4px;
    margin: 12px 0;
    padding: 10px 12px;
    border: 1px solid rgba(245, 158, 11, 0.45);
    background: rgba(245, 158, 11, 0.1);
    color: #fbbf24;
    font-size: 12px;
    line-height: 1.4;
  }

  .feature-notice.compact {
    margin: 6px 0;
    padding: 7px 9px;
    font-size: 10px;
  }

  .feature-notice span {
    color: inherit;
  }
</style>
