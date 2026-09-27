<script>
  import { createEventDispatcher, onDestroy, onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { aiController } from "../services/app/aiController.js";
  import { aiState } from "../stores/aiState.svelte.js";
  import * as backendAi from "../services/backend/ai.js";

  const dispatch = createEventDispatcher();

  let endpointDraft = $state(aiState.endpointDraft);
  let remoteConsent = $state(aiState.remoteConsent);
  let privateConsent = $state(aiState.privateConsent);
  let currentLlmModel = $state(aiState.llmModel);
  let currentEmbedModel = $state(aiState.embedModel);
  let saveError = $state(null);

  let previewProfile = $state(null);
  let previewError = $state(null);
  let isPreviewing = $state(false);
  let previewRevision = $state(0);

  let isMounted = false;
  let debounceTimer = null;

  async function runPreview(draft, rev) {
    if (!isMounted || rev !== previewRevision) return;
    isPreviewing = true;
    try {
      const profile = await backendAi.previewAiEndpoint(draft);
      if (!isMounted || rev !== previewRevision) return;
      previewProfile = profile;
      previewError = null;
    } catch (err) {
      if (!isMounted || rev !== previewRevision) return;
      previewError = err?.message || String(err);
      previewProfile = null;
    } finally {
      if (isMounted && rev === previewRevision) {
        isPreviewing = false;
      }
    }
  }

  onMount(() => {
    isMounted = true;
    aiController._initStorageMigration();
    endpointDraft = aiState.endpointDraft;
    remoteConsent = aiState.remoteConsent;
    privateConsent = aiState.privateConsent;
    currentLlmModel = aiState.llmModel;
    currentEmbedModel = aiState.embedModel;
    previewProfile = aiState.activeProfile;

    if (endpointDraft) {
      void runPreview(endpointDraft, previewRevision);
    }
  });

  onDestroy(() => {
    isMounted = false;
    if (debounceTimer) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
  });

  function handleEndpointInput(e) {
    endpointDraft = e.target.value;
    remoteConsent = false;
    privateConsent = false;
    saveError = null;
    previewError = null;
    previewProfile = null;
    previewRevision += 1;

    const currentRev = previewRevision;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      void runPreview(endpointDraft, currentRev);
    }, 250);
  }

  async function handleSave() {
    if (previewError) {
      saveError = previewError;
      return;
    }
    saveError = null;
    try {
      await aiController.configureEndpoint(endpointDraft, remoteConsent, privateConsent);
      aiState.endpointDraft = endpointDraft;
      aiState.llmModel = currentLlmModel;
      aiState.embedModel = currentEmbedModel;
      aiController.persistSettings();

      dispatch("save", { ollamaUrl: endpointDraft, llmModel: currentLlmModel, embedModel: currentEmbedModel });
      dispatch("close");
    } catch (err) {
      saveError = err?.message || String(err);
    }
  }
</script>

<dialog
  open
  class="modal-overlay"
  transition:fade={{ duration: 150 }}
  oncancel={(event) => {
    event.preventDefault();
    dispatch("close");
  }}
>
  <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }}>
    <div class="modal-header">
      <h3>Yapay Zeka Ayarları (Ollama)</h3>
      <button type="button" class="close-btn" aria-label="Kapat" onclick={() => dispatch("close")}>✕</button>
    </div>
    <div class="modal-body">
      <FeatureNotice featureId="local_ai" />
      <p class="privacy-warning">
        {#if previewProfile?.mode === "Loopback" || (!previewProfile && aiState.activeProfile?.mode === "Loopback")}
          İstek doğrulanmış loopback endpoint’ine gönderilir. Endpoint’i çalıştıran servis veriyi kendi yapılandırmasına göre işleyebilir veya saklayabilir.
        {:else if previewProfile?.mode === "PrivateNetwork" || (!previewProfile && aiState.activeProfile?.mode === "PrivateNetwork")}
          İstek yerel ağdaki (RFC1918 / IPv6 ULA) özel AI endpoint’ine gönderilir. Belge metni ve sorgular yerel ağdaki bu adrese iletilecektir.
        {:else}
          İstek yerel cihaz dışındaki uzak HTTPS AI endpoint’ine gönderilir. Belge metni ve sorgular cihaz dışına aktarılacaktır.
        {/if}
      </p>

      {#if previewProfile}
        <div class="profile-preview-badge">
          <span class="badge-mode">{previewProfile.mode}</span>
          <span class="badge-origin">{previewProfile.canonical_origin}</span>
        </div>
      {/if}

      {#if previewError}
        <div class="error-banner">Endpoint Hatası: {previewError}</div>
      {/if}

      {#if saveError && saveError !== previewError}
        <div class="error-banner">{saveError}</div>
      {/if}

      <div class="form-group">
        <label for="ai-ollama-url">
          Ollama Endpoint URL
          {#if isPreviewing}
            <span class="preview-spinner">Doğrulanıyor...</span>
          {/if}
        </label>
        <input id="ai-ollama-url" type="text" value={endpointDraft} oninput={handleEndpointInput} class="tech-input" />
        <small class="hint">Ollama arka planda çalışıyor olmalıdır (Örn: http://127.0.0.1:11434).</small>
      </div>

      <div class="form-group consent-group">
        <label class="checkbox-label">
          <input type="checkbox" bind:checked={remoteConsent} />
          Uzak AI Endpoint Kullanımına Onay Ver (Remote Public)
        </label>
        <small class="hint">İsteklerin yerel makine dışındaki uzak HTTPS sunucularına gönderilmesine izin verir.</small>
      </div>

      <div class="form-group consent-group">
        <label class="checkbox-label">
          <input type="checkbox" bind:checked={privateConsent} />
          Özel Ağ Endpoint Kullanımına İkinci Onay Ver (Private Network)
        </label>
        <small class="hint">İsteklerin yerel ağdaki (RFC1918 / IPv6 ULA) sunuculara gönderilmesine izin verir (Açık 2. onay gereklidir).</small>
      </div>

      <div class="form-group">
        <label for="ai-llm-model">LLM Modeli (Chat / Özetleme)</label>
        <input id="ai-llm-model" type="text" bind:value={currentLlmModel} class="tech-input" placeholder="örn: llama3.2:latest" />
      </div>

      <div class="form-group">
        <label for="ai-embed-model">Embedding Modeli (Semantik Arama)</label>
        <input id="ai-embed-model" type="text" bind:value={currentEmbedModel} class="tech-input" placeholder="örn: nomic-embed-text:latest" />
      </div>
    </div>
    <div class="modal-footer">
      <button class="tech-button" onclick={() => dispatch("close")}>İptal</button>
      <button
        class="tech-button primary"
        onclick={handleSave}
        disabled={Boolean(previewError) || isPreviewing}
      >
        Kaydet
      </button>
    </div>
  </div>
</dialog>

<style>
  .modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }

  .modal-content {
    width: 440px;
    padding: 20px;
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .modal-header h3 {
    margin: 0;
    font-size: 18px;
  }
  .close-btn {
    background: none;
    border: none;
    color: white;
    cursor: pointer;
    font-size: 18px;
  }

  .modal-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .form-group label {
    font-size: 12px;
    opacity: 0.8;
    display: flex;
    justify-content: space-between;
  }
  .preview-spinner {
    font-size: 11px;
    color: #60a5fa;
  }
  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 12px;
  }
  .hint {
    font-size: 10px;
    opacity: 0.5;
    margin-top: 2px;
  }
  .privacy-warning {
    color: #fbbf24;
    font-size: 11px;
    line-height: 1.4;
    margin: 0;
  }
  .profile-preview-badge {
    display: flex;
    align-items: center;
    gap: 8px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 11px;
  }
  .badge-mode {
    background: rgba(96, 165, 250, 0.2);
    color: #93c5fd;
    padding: 2px 6px;
    border-radius: 4px;
    font-weight: bold;
  }
  .badge-origin {
    font-family: monospace;
    opacity: 0.8;
  }
  .error-banner {
    background: rgba(239, 68, 68, 0.2);
    border: 1px solid rgba(239, 68, 68, 0.4);
    color: #fca5a5;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
  }
</style>
