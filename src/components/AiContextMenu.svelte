<script>
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { summarizeSelectionOrPage } from "../pdfState.js";
  import { listen } from "../services/platform/events.js";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("local_ai", featureContext);

  let selectedText = "";
  let menuX = 0;
  let menuY = 0;
  let isVisible = false;
  let aiResponse = "";
  let isProcessing = false;

  let unlistenToken = null;
  let unlistenDone = null;
  let selectionTimer = null;

  function handleMouseUp(e) {
    // Biraz bekle (seçimin DOM'a tam yansıması için)
    if (selectionTimer !== null) clearTimeout(selectionTimer);
    selectionTimer = setTimeout(() => {
      selectionTimer = null;
      const selection = window.getSelection();
      const text = selection.toString().trim();
      
      if (text.length > 5) {
        selectedText = text;
        const range = selection.getRangeAt(0);
        const rect = range.getBoundingClientRect();
        
        // Menüyü seçimin altına hizala
        menuX = rect.left + window.scrollX;
        menuY = rect.bottom + window.scrollY + 10;
        isVisible = true;
        aiResponse = "";
      } else {
        isVisible = false;
      }
    }, 100);
  }

  function handleMouseDown(e) {
    // Eğer menü içerisine tıklanmıyorsa menüyü gizle
    if (isVisible && !e.target.closest('.ai-context-menu')) {
      isVisible = false;
      aiResponse = "";
    }
  }

  onMount(() => {
    let destroyed = false;
    document.addEventListener("mouseup", handleMouseUp);
    document.addEventListener("mousedown", handleMouseDown);
    void listen("ai_summary_token", (event) => {
      aiResponse += event.payload;
    }).then((dispose) => { if (destroyed) dispose(); else unlistenToken = dispose; })
      .catch((error) => console.warn("AI summary token listener kurulamadı:", error));
    void listen("ai_summary_done", () => {
      isProcessing = false;
    }).then((dispose) => { if (destroyed) dispose(); else unlistenDone = dispose; })
      .catch((error) => console.warn("AI summary completion listener kurulamadı:", error));
    return () => {
      destroyed = true;
      document.removeEventListener("mouseup", handleMouseUp);
      document.removeEventListener("mousedown", handleMouseDown);
      if (selectionTimer !== null) clearTimeout(selectionTimer);
      selectionTimer = null;
      unlistenToken?.();
      unlistenDone?.();
      unlistenToken = null;
      unlistenDone = null;
    };
  });

  async function callAiAction(targetLength) {
    try {
      return await guardFeatureAction(
        "local_ai",
        featureContext,
        () => performAiAction(targetLength),
      );
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
    let ollamaUrl = localStorage.getItem("ollamaUrl") || "http://127.0.0.1:11434";
    let llmModel = localStorage.getItem("llmModel") || "llama3";
    try {
      await summarizeSelectionOrPage(selectedText, targetLength, ollamaUrl, llmModel);
    } catch (e) {
      aiResponse = "Hata: " + e;
      isProcessing = false;
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
    <p class="privacy-warning">Seçili içerik yapılandırılan AI endpoint’ine gönderilebilir.</p>
    {#if !aiResponse && !isProcessing}
      <div class="actions">
        <button on:click={() => callAiAction("short")} disabled={!featurePresentation.runnable}>✨ Özetle</button>
        <button on:click={() => callAiAction("detailed")} disabled={!featurePresentation.runnable}>📚 Açıkla</button>
        <button on:click={() => callAiAction("bullet")} disabled={!featurePresentation.runnable}>📝 Maddeleştir</button>
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
    box-shadow: 0 10px 30px rgba(0,0,0,0.5);
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
