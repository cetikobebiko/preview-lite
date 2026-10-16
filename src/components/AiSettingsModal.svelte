<script>
  import { onDestroy, onMount } from "svelte";
  import AppDialog from "./AppDialog.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { aiController } from "../services/app/aiController.js";
  import { aiState } from "../stores/aiState.svelte.js";
  import * as backendAi from "../services/backend/ai.js";

  let { onclose, onsave } = $props();

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

      onsave?.({ ollamaUrl: endpointDraft, llmModel: currentLlmModel, embedModel: currentEmbedModel });
      onclose?.();
    } catch (err) {
      saveError = err?.message || String(err);
    }
  }
</script>

<AppDialog title="Yapay Zeka Ayarları (Ollama)" onclose={() => onclose?.()}>
  <div class="grid gap-4">
    <FeatureNotice featureId="local_ai" />
    <p class="rounded-md bg-warning px-3 py-2 text-xs text-warning-foreground">
      {#if previewProfile?.mode === "Loopback" || (!previewProfile && aiState.activeProfile?.mode === "Loopback")}
        İstek doğrulanmış loopback endpoint’ine gönderilir. Endpoint’i çalıştıran servis veriyi kendi yapılandırmasına göre işleyebilir veya saklayabilir.
      {:else if previewProfile?.mode === "PrivateNetwork" || (!previewProfile && aiState.activeProfile?.mode === "PrivateNetwork")}
        İstek yerel ağdaki (RFC1918 / IPv6 ULA) özel AI endpoint’ine gönderilir. Belge metni ve sorgular yerel ağdaki bu adrese iletilecektir.
      {:else}
        İstek yerel cihaz dışındaki uzak HTTPS AI endpoint’ine gönderilir. Belge metni ve sorgular cihaz dışına aktarılacaktır.
      {/if}
    </p>

    {#if previewProfile}
      <div class="flex items-center gap-2 text-xs">
        <span class="rounded-sm bg-primary/10 px-1.5 py-0.5 font-medium text-primary">{previewProfile.mode}</span>
        <span class="truncate font-mono text-muted-foreground">{previewProfile.canonical_origin}</span>
      </div>
    {/if}

    {#if previewError}
      <div class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">Endpoint Hatası: {previewError}</div>
    {/if}

    {#if saveError && saveError !== previewError}
      <div class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">{saveError}</div>
    {/if}

    <div class="grid gap-1.5">
      <label for="ai-ollama-url" class="text-xs font-medium text-muted-foreground">
        Ollama Endpoint URL
        {#if isPreviewing}
          <span class="ml-1 font-normal text-primary">Doğrulanıyor...</span>
        {/if}
      </label>
      <Input id="ai-ollama-url" type="text" value={endpointDraft} oninput={handleEndpointInput} />
      <small class="text-[11px] text-muted-foreground">Ollama arka planda çalışıyor olmalıdır (Örn: http://127.0.0.1:11434).</small>
    </div>

    <div class="grid gap-1">
      <label class="flex items-center gap-2 text-[13px]">
        <input type="checkbox" class="accent-primary" bind:checked={remoteConsent} />
        Uzak AI Endpoint Kullanımına Onay Ver (Remote Public)
      </label>
      <small class="text-[11px] text-muted-foreground pl-5">İsteklerin yerel makine dışındaki uzak HTTPS sunucularına gönderilmesine izin verir.</small>
    </div>

    <div class="grid gap-1">
      <label class="flex items-center gap-2 text-[13px]">
        <input type="checkbox" class="accent-primary" bind:checked={privateConsent} />
        Özel Ağ Endpoint Kullanımına İkinci Onay Ver (Private Network)
      </label>
      <small class="text-[11px] text-muted-foreground pl-5">İsteklerin yerel ağdaki (RFC1918 / IPv6 ULA) sunuculara gönderilmesine izin verir (Açık 2. onay gereklidir).</small>
    </div>

    <div class="grid grid-cols-2 gap-3">
      <div class="grid gap-1.5">
        <label for="ai-llm-model" class="text-xs font-medium text-muted-foreground">LLM Modeli (Chat / Özetleme)</label>
        <Input id="ai-llm-model" type="text" bind:value={currentLlmModel} placeholder="örn: llama3.2:latest" />
      </div>
      <div class="grid gap-1.5">
        <label for="ai-embed-model" class="text-xs font-medium text-muted-foreground">Embedding Modeli (Semantik Arama)</label>
        <Input id="ai-embed-model" type="text" bind:value={currentEmbedModel} placeholder="örn: nomic-embed-text:latest" />
      </div>
    </div>
  </div>

  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={() => onclose?.()}>İptal</Button>
    <Button size="sm" onclick={handleSave} disabled={Boolean(previewError) || isPreviewing}>Kaydet</Button>
  {/snippet}
</AppDialog>
