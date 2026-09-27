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
    class="ai-context-menu glass-panel"
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
    background: rgba(20, 25, 30, 0.95);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 8px;
    padding: 8px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    min-width: 200px;
    max-width: 350px;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .privacy-warning {
    color: #fbbf24;
    font-size: 10px;
    line-height: 1.35;
    margin: 4px;
  }

  .actions button {
    background: none;
    border: none;
    color: white;
    text-align: left;
    padding: 8px 12px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.2s;
  }

  .actions button:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .ai-response-box {
    padding: 8px;
    font-size: 13px;
    line-height: 1.5;
    color: #eee;
    max-height: 250px;
    overflow-y: auto;
  }

  .loader {
    font-size: 11px;
    opacity: 0.6;
    margin-top: 8px;
    font-style: italic;
  }
</style>
