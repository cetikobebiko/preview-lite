<script>
  import { createEventDispatcher } from "svelte";
  import { fade, fly } from "svelte/transition";
  import FeatureNotice from "./FeatureNotice.svelte";

  export let ollamaUrl = localStorage.getItem("ollamaUrl") || "http://127.0.0.1:11434";
  export let llmModel = localStorage.getItem("llmModel") || "llama3";
  export let embedModel = localStorage.getItem("embedModel") || "nomic-embed-text";

  const dispatch = createEventDispatcher();

  function handleSave() {
    localStorage.setItem("ollamaUrl", ollamaUrl);
    localStorage.setItem("llmModel", llmModel);
    localStorage.setItem("embedModel", embedModel);
    dispatch("save", { ollamaUrl, llmModel, embedModel });
    dispatch("close");
  }
</script>

<div class="modal-overlay" transition:fade={{ duration: 150 }} on:click={() => dispatch("close")}>
  <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }} on:click|stopPropagation>
      <div class="modal-header">
          <h3>Yapay Zeka Ayarları (Ollama)</h3>
          <button class="close-btn" on:click={() => dispatch("close")}>✕</button>
      </div>
      <div class="modal-body">
          <FeatureNotice featureId="local_ai" />
          <p class="privacy-warning">Belge içeriği yapılandırılan AI endpoint’ine gönderilebilir. Yalnızca loopback endpoint kullanıldığında trafik cihaz içinde kalır.</p>
          <div class="form-group">
              <label for="ai-ollama-url">Ollama Endpoint URL</label>
              <input id="ai-ollama-url" type="text" bind:value={ollamaUrl} class="tech-input" />
              <small class="hint">Ollama arka planda çalışıyor olmalıdır.</small>
          </div>
          <div class="form-group">
              <label for="ai-llm-model">LLM Modeli (Chat / Özetleme)</label>
              <input id="ai-llm-model" type="text" bind:value={llmModel} class="tech-input" placeholder="örn: llama3, phi3" />
          </div>
          <div class="form-group">
              <label for="ai-embed-model">Embedding Modeli (Semantik Arama)</label>
              <input id="ai-embed-model" type="text" bind:value={embedModel} class="tech-input" placeholder="örn: nomic-embed-text" />
          </div>
      </div>
      <div class="modal-footer">
          <button class="tech-button" on:click={() => dispatch("close")}>İptal</button>
          <button class="tech-button primary" on:click={handleSave}>Kaydet</button>
      </div>
  </div>
</div>

<style>
  .modal-overlay {
      position: fixed; top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(0, 0, 0, 0.6);
      display: flex; align-items: center; justify-content: center;
      z-index: 2000;
  }
  
  .modal-content {
      width: 400px;
      padding: 20px;
      border-radius: 12px;
      display: flex; flex-direction: column; gap: 16px;
  }
  
  .modal-header {
      display: flex; justify-content: space-between; align-items: center;
  }
  .modal-header h3 { margin: 0; font-size: 18px; }
  .close-btn { background: none; border: none; color: white; cursor: pointer; font-size: 18px; }
  
  .modal-body { display: flex; flex-direction: column; gap: 12px; }
  .form-group { display: flex; flex-direction: column; gap: 4px; }
  .form-group label { font-size: 12px; opacity: 0.8; }
  .hint { font-size: 10px; opacity: 0.5; margin-top: 2px; }
  .privacy-warning { color: #fbbf24; font-size: 11px; line-height: 1.4; margin: 0; }
  
  .modal-footer {
      display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px;
  }
</style>
