<script>
  import { createEventDispatcher } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { activePlugins, loadPlugin, unloadPlugin } from "../pluginStore.js";
  import { open } from "@tauri-apps/plugin-dialog";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  const dispatch = createEventDispatcher();
  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("plugin_system", featureContext);
  let isLoading = false;
  let errorMessage = "";

  async function handleLoadPlugin() {
    errorMessage = "";
    try {
      return await guardFeatureAction("plugin_system", featureContext, async () => {
        const selected = await open({
          multiple: false,
          filters: [{ name: 'Eklenti Manifesti', extensions: ['json'] }]
        });

        if (selected) {
          isLoading = true;
          try {
            await loadPlugin(selected);
          } finally {
            isLoading = false;
          }
        }
      });
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMessage = error.presentation?.warning || error.message;
        return;
      }
      throw error;
    }
  }

  async function handleUnload(pluginId) {
    errorMessage = "";
    try {
      return await guardFeatureAction(
        "plugin_system",
        featureContext,
        () => unloadPlugin(pluginId),
      );
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMessage = error.presentation?.warning || error.message;
        return;
      }
      throw error;
    }
  }
</script>

<div class="modal-overlay" transition:fade={{ duration: 150 }} on:click={() => dispatch("close")}>
  <div class="modal-content glass-panel" transition:fly={{ y: 20, duration: 250 }} on:click|stopPropagation>
      <div class="modal-header">
          <h3>Eklenti Yöneticisi (Plugin Manager)</h3>
          <button class="close-btn" on:click={() => dispatch("close")}>✕</button>
      </div>
      <div class="modal-body">
          <FeatureNotice featureId="plugin_system" />
          {#if errorMessage}
            <p class="plugin-warning" role="status">{errorMessage}</p>
          {/if}
          <div class="plugin-list">
              {#if $activePlugins.length === 0}
                 <div class="empty-state">Henüz yüklü eklenti yok.</div>
              {/if}
              
              {#each $activePlugins as plugin}
                 <div class="plugin-card">
                    <div class="plugin-info">
                       <strong>{plugin.name}</strong> <span class="badge">v{plugin.version}</span>
                       <div class="plugin-id">{plugin.id}</div>
                    </div>
                    <button class="tech-button danger" on:click={() => handleUnload(plugin.id)} disabled={!featurePresentation.runnable}>Kaldır</button>
                 </div>
              {/each}
          </div>
          
      </div>
      <div class="modal-footer">
          <button class="tech-button primary" on:click={handleLoadPlugin} disabled={isLoading || !featurePresentation.runnable}>
              {isLoading ? "Yükleniyor..." : "Kullanılamıyor · EXPERIMENTAL"}
          </button>
      </div>
  </div>
</div>

<style>
  .modal-overlay {
      position: fixed; top: 0; left: 0; right: 0; bottom: 0;
      background: rgba(0, 0, 0, 0.7);
      display: flex; align-items: center; justify-content: center;
      z-index: 2000;
  }
  
  .modal-content {
      width: 500px;
      padding: 20px;
      border-radius: 12px;
      display: flex; flex-direction: column; gap: 16px;
  }
  
  .modal-header {
      display: flex; justify-content: space-between; align-items: center;
  }
  .modal-header h3 { margin: 0; font-size: 18px; color: white;}
  .close-btn { background: none; border: none; color: white; cursor: pointer; font-size: 18px; opacity: 0.7; }
  .close-btn:hover { opacity: 1; }
  
  .modal-body { display: flex; flex-direction: column; gap: 12px; min-height: 200px; }
  
  .plugin-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
  }
  .empty-state { text-align: center; opacity: 0.5; padding: 20px; font-style: italic; }
  .plugin-warning { color: #f59e0b; font-size: 12px; }
  
  .plugin-card {
      background: rgba(255, 255, 255, 0.05);
      border: 1px solid rgba(255,255,255,0.1);
      padding: 12px;
      border-radius: 8px;
      display: flex;
      justify-content: space-between;
      align-items: center;
  }
  .plugin-info { display: flex; flex-direction: column; gap: 4px; }
  .badge { background: rgba(0, 255, 0, 0.2); color: #0f0; padding: 2px 6px; border-radius: 4px; font-size: 11px; }
  .plugin-id { font-size: 11px; opacity: 0.5; font-family: monospace; }
  
  .modal-footer {
      display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px;
  }
</style>
