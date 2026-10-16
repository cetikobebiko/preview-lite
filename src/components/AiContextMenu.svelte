<script>
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";
  import { aiController } from "../services/app/aiController.js";
  import { aiState } from "../stores/aiState.svelte.js";

  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("local_ai", featureContext);

  let selectedText = $state("");
  let menuX = $state(0);
  let menuY = $state(0);
  let isVisible = $state(false);
  let aiResponse = $state("");
  let isProcessing = $state(false);
  let selectionTimer = null;
  let activeHandle = null;

  function cleanupActiveHandle() {
    if (activeHandle) {
      void activeHandle.cancel();
      activeHandle = null;
    }
  }

  function handleMouseUp() {
    if (selectionTimer !== null) clearTimeout(selectionTimer);
    selectionTimer = setTimeout(() => {
      selectionTimer = null;
      const selection = window.getSelection();
      const text = selection ? selection.toString().trim() : "";

      if (text.length > 5) {
        cleanupActiveHandle();
        selectedText = text;
        const range = selection.getRangeAt(0);
        const rect = range.getBoundingClientRect();

        menuX = rect.left + window.scrollX;
        menuY = rect.bottom + window.scrollY + 10;
        isVisible = true;
        aiResponse = "";
      } else {
        cleanupActiveHandle();
        isVisible = false;
      }
    }, 100);
  }

  function handleMouseDown(e) {
    if (isVisible && !e.target.closest(".ai-context-menu")) {
      cleanupActiveHandle();
      isVisible = false;
      aiResponse = "";
    }
  }

  onMount(() => {
    document.addEventListener("mouseup", handleMouseUp);
    document.addEventListener("mousedown", handleMouseDown);

    return () => {
      document.removeEventListener("mouseup", handleMouseUp);
      document.removeEventListener("mousedown", handleMouseDown);
      if (selectionTimer !== null) clearTimeout(selectionTimer);
      selectionTimer = null;
      cleanupActiveHandle();
    };
  });

  async function callAiAction(targetLength) {
    try {
      return await guardFeatureAction("local_ai", featureContext, () => performAiAction(targetLength));
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        aiResponse = error.presentation?.warning || error.message;
        return;
      }
      throw error;
    }
  }

  async function performAiAction(targetLength) {
    isProcessing = true;
    aiResponse = "";
    cleanupActiveHandle();
    try {
      const handle = aiController.startSummary(selectedText, targetLength, (delta) => {
        aiResponse += delta;
      });
      activeHandle = handle;
      await handle.done;
    } catch (e) {
      if (e?.code === "AI_REQUEST_CANCELLED") {
        aiResponse += " [İptal Edildi]";
      } else {
        aiResponse = "Hata: " + (e?.message || e);
      }
    } finally {
      isProcessing = false;
      activeHandle = null;
    }
  }
</script>

{#if isVisible}
  <div
    class="ai-context-menu"
    style="left: {menuX}px; top: {menuY}px;"
    transition:fly={{ y: -10, duration: 200 }}
  >
    <FeatureNotice featureId="local_ai" compact />
    <p class="privacy-warning">
      {aiState.modeLabel}: Seçili metin, özetleme/açıklama talimatı ve LLM model adı ({aiState.llmModel}) yapılandırılan endpoint'e iletilir.
    </p>

    {#if !aiResponse && !isProcessing}
      <div class="actions">
        <button onclick={() => callAiAction("short")} disabled={!featurePresentation.runnable}>✨ Özetle</button>
        <button onclick={() => callAiAction("detailed")} disabled={!featurePresentation.runnable}>📚 Açıkla</button>
        <button onclick={() => callAiAction("bullet")} disabled={!featurePresentation.runnable}>📝 Maddeleştir</button>
      </div>
    {:else}
      <div class="ai-response-box">
        <div class="response-text">{aiResponse}</div>
        {#if isProcessing}
          <div class="loader">Yapay zeka düşünüyor...</div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .ai-context-menu {
    position: absolute;
    z-index: 9999;
    background: var(--popover);
    color: var(--popover-foreground);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 6px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.16);
    min-width: 200px;
    max-width: 350px;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .privacy-warning {
    margin: 4px;
    padding: 6px 8px;
    border-radius: calc(var(--radius) * 0.8);
    background: var(--warning);
    color: var(--warning-foreground);
    font-size: 11px;
    line-height: 1.35;
  }

  .actions button {
    background: none;
    border: none;
    color: inherit;
    text-align: left;
    padding: 6px 10px;
    border-radius: calc(var(--radius) * 0.6);
    cursor: pointer;
    font-size: 13px;
  }

  .actions button:hover:not(:disabled) {
    background: var(--accent);
  }

  .actions button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ai-response-box {
    padding: 8px;
    font-size: 13px;
    line-height: 1.5;
    max-height: 250px;
    overflow-y: auto;
  }

  .loader {
    font-size: 11px;
    color: var(--muted-foreground);
    margin-top: 8px;
    font-style: italic;
  }
</style>
