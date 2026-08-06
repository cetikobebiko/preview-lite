<script>
  import { createEventDispatcher } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { runScriptDirect } from "../pluginStore.js";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  const dispatch = createEventDispatcher();
  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("plugin_system", featureContext);
  let scriptContent = 'let a = 40; let b = 2; a + b'; // default
  let resultOutput = "";
  let isRunning = false;

  async function handleRun() {
    try {
      return await guardFeatureAction("plugin_system", featureContext, async () => {
        isRunning = true;
        try {
          const res = await runScriptDirect(scriptContent);
          resultOutput = "👉 Sonuç:\n" + res;
        } finally {
          isRunning = false;
        }
      });
    } catch(error) {
      if (error instanceof FeatureUnavailableError) {
        resultOutput = "⚠️ Kullanılamıyor:\n" + (error.presentation?.warning || error.message);
        return;
      }
      throw error;
    }
  }
</script>

<div class="modal-overlay" transition:fade={{ duration: 150 }} on:click={() => dispatch("close")}>
  <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }} on:click|stopPropagation>
      <div class="modal-header">
          <h3>Geliştirici Konsolu (Rhai Script)</h3>
          <button class="close-btn" on:click={() => dispatch("close")}>✕</button>
      </div>
      <div class="modal-body">
          <FeatureNotice featureId="plugin_system" />
          <textarea 
            class="tech-input code-area" 
            bind:value={scriptContent} 
            placeholder="Rhai betiğini buraya yazın..."
            spellcheck="false"
            disabled={!featurePresentation.runnable}
          ></textarea>
          
          <div class="output-box">
             <pre>{resultOutput || "Çıktı bekleniyor..."}</pre>
          </div>
      </div>
      <div class="modal-footer">
          <button class="tech-button" on:click={() => dispatch("close")}>İptal</button>
          <button class="tech-button primary" on:click={handleRun} disabled={isRunning || !featurePresentation.runnable}>Kullanılamıyor · EXPERIMENTAL</button>
      </div>
  </div>
</div>

<style>
  .modal-overlay {
      position: fixed; top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(0, 0, 0, 0.7);
      display: flex; align-items: center; justify-content: center;
      z-index: 3000;
  }
  
  .modal-content {
      width: 600px;
      padding: 20px;
      border-radius: 12px;
      display: flex; flex-direction: column; gap: 16px;
  }
  
  .modal-header {
      display: flex; justify-content: space-between; align-items: center;
  }
  .modal-header h3 { margin: 0; font-size: 18px; color: var(--accent-industrial); }
  .close-btn { background: none; border: none; color: white; cursor: pointer; font-size: 18px; opacity:0.7;}
  .close-btn:hover { opacity: 1;}
  
  .modal-body { display: flex; flex-direction: column; gap: 12px; }
  
  .code-area {
      height: 150px;
      font-family: monospace;
      font-size: 14px;
      resize: vertical;
      background: rgba(0,0,0,0.5);
      color: #0f0;
  }
  
  .output-box {
      background: rgba(0,0,0,0.3);
      padding: 12px;
      border-radius: 6px;
      border: 1px solid rgba(255,255,255,0.1);
      min-height: 80px;
  }
  .output-box pre {
      margin: 0;
      white-space: pre-wrap;
      font-family: monospace;
      font-size: 13px;
      color: #eee;
  }
  
  .modal-footer {
      display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px;
  }
</style>
