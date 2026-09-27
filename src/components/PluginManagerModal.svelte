<script>
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import {
    installedPlugins,
    inspectionCandidate,
    activeExecutions,
    refreshInstalledPlugins,
    inspectPlugin,
    installPlugin,
    grantConsent,
    uninstallPlugin,
    togglePluginEnabled,
    startPluginHook,
    cancelPlugin,
    pollExecutionStatus,
  } from "../pluginStore.js";
  import { open } from "@tauri-apps/plugin-dialog";
  import FeatureNotice from "./FeatureNotice.svelte";
  import { FeatureUnavailableError, guardFeatureAction } from "../featureActionGuard.js";
  import { getFeaturePresentation, getRuntimeFeatureContext } from "../featureRegistry.js";

  let { onclose } = $props();

  const featureContext = getRuntimeFeatureContext();
  const featurePresentation = getFeaturePresentation("plugin_system", featureContext);

  let isLoading = $state(false);
  let errorMessage = $state("");
  let statusMessage = $state("");
  let selectedHookPayload = $state("{}");
  let consentAcknowledged = $state(false);

  const PERMISSION_EXPLANATIONS = {
    "plugin:log": "Eklenti yürütme günlüklerini ana uygulamaya iletir. Dosya, ağ veya belge metni erişimi sağlamaz.",
    "ui:notification": "Kullanıcı arayüzünde bilgilendirme bildirimi gösterir. Dosya, ağ veya belge metni erişimi sağlamaz.",
  };

  onMount(() => {
    refreshInstalledPlugins().catch(() => {});
  });

  async function handlePickPackage() {
    errorMessage = "";
    statusMessage = "";
    consentAcknowledged = false;
    try {
      await guardFeatureAction("plugin_system", featureContext, async () => {
        const selected = await open({
          directory: true,
          multiple: false,
          title: "Eklenti Paketi Klasörünü Seçin",
        });

        if (selected) {
          isLoading = true;
          try {
            await inspectPlugin(selected);
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
      errorMessage = error?.message || String(error);
    }
  }

  async function handleConfirmInstall() {
    if (!$inspectionCandidate || !consentAcknowledged) return;
    errorMessage = "";
    statusMessage = "";
    isLoading = true;
    try {
      await guardFeatureAction("plugin_system", featureContext, async () => {
        const candidate = $inspectionCandidate;
        const targetPath = candidate.package_path || candidate.manifest.id;
        
        // 1. Install & Stage
        await installPlugin(targetPath, candidate.package_digest);
        
        // 2. Grant explicit consent
        await grantConsent(
          candidate.manifest.id,
          candidate.package_digest,
          candidate.manifest.permissions
        );

        statusMessage = `Eklenti başarıyla yüklendi ve yetkilendirildi: ${candidate.manifest.name}`;
      });
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMessage = error.presentation?.warning || error.message;
      } else {
        errorMessage = error?.message || String(error);
      }
    } finally {
      isLoading = false;
    }
  }

  async function handleToggleEnabled(plugin) {
    errorMessage = "";
    try {
      await guardFeatureAction("plugin_system", featureContext, async () => {
        await togglePluginEnabled(plugin.id, !plugin.enabled);
      });
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMessage = error.presentation?.warning || error.message;
      } else {
        errorMessage = error?.message || String(error);
      }
    }
  }

  async function handleUninstall(pluginId) {
    errorMessage = "";
    try {
      await guardFeatureAction("plugin_system", featureContext, async () => {
        await uninstallPlugin(pluginId);
        statusMessage = "Eklenti başarıyla kaldırıldı.";
      });
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMessage = error.presentation?.warning || error.message;
      } else {
        errorMessage = error?.message || String(error);
      }
    }
  }

  async function handleRunHook(plugin, hookName) {
    errorMessage = "";
    try {
      await guardFeatureAction("plugin_system", featureContext, async () => {
        const startResult = await startPluginHook(plugin.id, hookName, selectedHookPayload);
        const executionId = startResult.execution_id;

        // Poll execution status
        const pollInterval = setInterval(async () => {
          try {
            const status = await pollExecutionStatus(executionId);
            if (["COMPLETED", "CANCELLED", "FAILED", "TIMED_OUT"].includes(status.state)) {
              clearInterval(pollInterval);
            }
          } catch {
            clearInterval(pollInterval);
          }
        }, 300);
      });
    } catch (error) {
      if (error instanceof FeatureUnavailableError) {
        errorMessage = error.presentation?.warning || error.message;
      } else {
        errorMessage = error?.message || String(error);
      }
    }
  }

  async function handleCancelExecution(executionId) {
    try {
      await cancelPlugin(executionId);
    } catch (error) {
      errorMessage = error?.message || String(error);
    }
  }
</script>

<div class="modal-overlay" role="presentation" transition:fade={{ duration: 150 }} onclick={onclose}>
  <div class="modal-content glass-panel" role="dialog" aria-modal="true" tabindex="-1" transition:fly={{ y: 20, duration: 250 }} onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()}>
    <div class="modal-header">
      <h3>Eklenti Yöneticisi (Plugin Manager)</h3>
      <button class="close-btn" onclick={onclose}>✕</button>
    </div>

    <div class="modal-body">
      <FeatureNotice featureId="plugin_system" />

      {#if errorMessage}
        <p class="plugin-warning" role="status">{errorMessage}</p>
      {/if}
      {#if statusMessage}
        <p class="plugin-success" role="status">{statusMessage}</p>
      {/if}

      <!-- Candidate Inspection & Consent Review -->
      {#if $inspectionCandidate}
        <div class="inspection-card">
          <div class="inspection-header">
            <h4>Eklenti İnceleme & Güvenlik Onayı</h4>
            <span class="badge warning">Onay Bekliyor</span>
          </div>

          <div class="unsigned-warning-box">
            <strong>⚠️ İmzasız Eklenti Uyarısı:</strong> Bu eklenti paketi yerel ve imzasızdır (unsigned). Yalnızca güvendiğiniz kaynaklardan gelen eklentileri yükleyiniz.
          </div>

          <div class="inspection-details">
            <p><strong>Ad:</strong> {$inspectionCandidate.manifest.name} (v{$inspectionCandidate.manifest.version})</p>
            <p><strong>ID:</strong> <code>{$inspectionCandidate.manifest.id}</code></p>
            {#if $inspectionCandidate.manifest.description}
              <p><strong>Açıklama:</strong> {$inspectionCandidate.manifest.description}</p>
            {/if}
            <p><strong>Paket Özeti (SHA256):</strong> <code class="digest">{$inspectionCandidate.package_digest}</code></p>
            
            <div class="perms-section">
              <strong>İstenen İzinler ve Veri Etkisi:</strong>
              {#if $inspectionCandidate.manifest.permissions.length === 0}
                <p class="perm-desc">Hiçbir ek izin talep edilmiyor.</p>
              {:else}
                <ul class="permission-explanation-list">
                  {#each $inspectionCandidate.manifest.permissions as perm}
                    <li>
                      <span class="badge permission">{perm}</span>:
                      <span class="perm-desc">{PERMISSION_EXPLANATIONS[perm] || "Belirtilmemiş izin etkisi."}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>

            <div class="perms-section">
              <strong>Bildirilen Kancalar:</strong>
              {#if $inspectionCandidate.manifest.hooks.length === 0}
                <span class="dim">Kanca bildirilmemiş</span>
              {:else}
                <div class="badge-list">
                  {#each $inspectionCandidate.manifest.hooks as hook}
                    <span class="badge hook">{hook}</span>
                  {/each}
                </div>
              {/if}
            </div>

            <div class="consent-checkbox-row">
              <label class="consent-label">
                <input
                  type="checkbox"
                  bind:checked={consentAcknowledged}
                  id="plugin-consent-acknowledgement"
                />
                <span>Yukarıdaki izinleri, veri etkilerini ve güvenlik uyarısını anladım, onaylıyorum.</span>
              </label>
            </div>
          </div>

          <div class="inspection-actions">
            <button class="tech-button" onclick={() => { inspectionCandidate.set(null); consentAcknowledged = false; }}>Vazgeç</button>
            <button
              class="tech-button primary"
              onclick={handleConfirmInstall}
              disabled={isLoading || !consentAcknowledged}
            >
              {isLoading ? "Yükleniyor..." : "İzinleri Onayla ve Yükle"}
            </button>
          </div>
        </div>
      {/if}

      <!-- Installed Plugins List -->
      <div class="plugin-list">
        {#if $installedPlugins.length === 0}
          <div class="empty-state">Henüz yüklü eklenti bulunmuyor.</div>
        {/if}

        {#each $installedPlugins as plugin (plugin.id)}
          <div class="plugin-card {plugin.enabled ? 'enabled' : 'disabled'}">
            <div class="plugin-info">
              <div class="plugin-title-row">
                <strong>{plugin.name}</strong>
                <span class="badge">v{plugin.version}</span>
                <span class="badge {plugin.enabled ? 'active-badge' : 'inactive-badge'}">
                  {plugin.enabled ? "Etkin" : "Devre Dışı"}
                </span>
              </div>
              <div class="plugin-id">{plugin.id}</div>
              {#if plugin.description}
                <p class="plugin-desc">{plugin.description}</p>
              {/if}
              <div class="meta-row">
                <span class="badge-list">
                  {#each plugin.permissions as perm}
                    <span class="badge permission small">{perm}</span>
                  {/each}
                  {#each plugin.hooks as hook}
                    <span class="badge hook small">{hook}</span>
                  {/each}
                </span>
              </div>
            </div>

            <div class="plugin-actions">
              <button
                class="tech-button {plugin.enabled ? 'secondary' : 'primary'}"
                onclick={() => handleToggleEnabled(plugin)}
                disabled={!featurePresentation.runnable}
              >
                {plugin.enabled ? "Kapat" : "Etkinleştir"}
              </button>
              
              {#if plugin.enabled && plugin.hooks.length > 0}
                <button
                  class="tech-button run"
                  onclick={() => handleRunHook(plugin, plugin.hooks[0])}
                  disabled={!featurePresentation.runnable}
                >
                  Çalıştır ({plugin.hooks[0]})
                </button>
              {/if}

              <button
                class="tech-button danger"
                onclick={() => handleUninstall(plugin.id)}
                disabled={!featurePresentation.runnable}
              >
                Kaldır
              </button>
            </div>

            <!-- Active Execution Card for this plugin -->
            {#each Object.values($activeExecutions).filter(e => e.plugin_id === plugin.id) as exec (exec.execution_id)}
              <div class="execution-box state-{exec.state.toLowerCase()}">
                <div class="exec-info">
                  <span>Yürütme: <code>{exec.hook_name}</code></span>
                  <span class="badge status-{exec.state.toLowerCase()}">{exec.state}</span>
                </div>
                {#if exec.state === "RUNNING"}
                  <button class="tech-button small danger" onclick={() => handleCancelExecution(exec.execution_id)}>İptal Et</button>
                {/if}
                {#if exec.result_json}
                  <pre class="result-box">{exec.result_json}</pre>
                {/if}
                {#if exec.error}
                  <p class="error-box">[{exec.error.code}] {exec.error.message}</p>
                {/if}
              </div>
            {/each}
          </div>
        {/each}
      </div>
    </div>

    <div class="modal-footer">
      <button
        class="tech-button primary"
        onclick={handlePickPackage}
        disabled={isLoading || !featurePresentation.runnable}
      >
        {isLoading ? "İnceleniyor..." : "Yeni Eklenti Paketi Seç..."}
      </button>
    </div>
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed; top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .modal-content {
    background: #1e1e24; color: #fff;
    border-radius: 8px; width: 620px; max-width: 90vw; max-height: 85vh;
    display: flex; flex-direction: column; border: 1px solid #333;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
  }
  .modal-header {
    padding: 16px; border-bottom: 1px solid #333;
    display: flex; justify-content: space-between; align-items: center;
  }
  .modal-header h3 { margin: 0; font-size: 1.1rem; }
  .close-btn { background: none; border: none; color: #888; font-size: 1.2rem; cursor: pointer; }
  .close-btn:hover { color: #fff; }
  .modal-body { padding: 16px; overflow-y: auto; flex: 1; display: flex; flex-direction: column; gap: 12px; }
  .plugin-warning { color: #f87171; background: rgba(248, 113, 113, 0.1); border: 1px solid #f87171; padding: 8px 12px; border-radius: 6px; font-size: 0.85rem; margin: 0; }
  .plugin-success { color: #4ade80; background: rgba(74, 222, 128, 0.1); border: 1px solid #4ade80; padding: 8px 12px; border-radius: 6px; font-size: 0.85rem; margin: 0; }
  .inspection-card {
    background: #25252b; border: 1px solid #3b82f6; border-radius: 6px; padding: 14px;
    display: flex; flex-direction: column; gap: 10px;
  }
  .unsigned-warning-box {
    background: rgba(234, 179, 8, 0.1); border: 1px solid #eab308; color: #fef08a;
    padding: 8px 12px; border-radius: 4px; font-size: 0.8rem;
  }
  .permission-explanation-list {
    margin: 6px 0 0 0; padding-left: 18px; font-size: 0.8rem; display: flex; flex-direction: column; gap: 4px;
  }
  .perm-desc { color: #cbd5e1; font-size: 0.8rem; }
  .consent-checkbox-row {
    margin-top: 10px; padding: 8px 10px; background: #1c1c22; border-radius: 4px; border: 1px solid #3f3f46;
  }
  .consent-label {
    display: flex; align-items: center; gap: 8px; font-size: 0.8rem; cursor: pointer; color: #f4f4f5;
  }
  .consent-label input[type="checkbox"] { cursor: pointer; }
  .inspection-header { display: flex; justify-content: space-between; align-items: center; }
  .inspection-header h4 { margin: 0; font-size: 0.95rem; color: #60a5fa; }
  .inspection-details p { margin: 4px 0; font-size: 0.85rem; }
  .digest { font-size: 0.75rem; word-break: break-all; color: #a3e635; }
  .perms-section { margin-top: 6px; font-size: 0.85rem; }
  .badge-list { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .badge { background: #333; padding: 2px 6px; border-radius: 4px; font-size: 0.75rem; }
  .badge.permission { background: #1e3a8a; color: #93c5fd; }
  .badge.hook { background: #065f46; color: #6ee7b7; }
  .badge.warning { background: #854d0e; color: #fef08a; }
  .badge.active-badge { background: #14532d; color: #86efac; }
  .badge.inactive-badge { background: #3f3f46; color: #a1a1aa; }
  .badge.small { font-size: 0.7rem; padding: 1px 4px; }
  .inspection-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
  .plugin-list { display: flex; flex-direction: column; gap: 10px; }
  .empty-state { text-align: center; color: #777; padding: 24px; font-size: 0.9rem; }
  .plugin-card {
    background: #282830; border: 1px solid #3a3a44; border-radius: 6px; padding: 12px;
    display: flex; flex-direction: column; gap: 8px;
  }
  .plugin-title-row { display: flex; align-items: center; gap: 8px; }
  .plugin-id { font-size: 0.75rem; color: #888; font-family: monospace; }
  .plugin-desc { font-size: 0.8rem; color: #ccc; margin: 4px 0; }
  .plugin-actions { display: flex; gap: 6px; justify-content: flex-end; }
  .execution-box {
    background: #18181b; border: 1px solid #27272a; border-radius: 4px; padding: 8px;
    font-size: 0.8rem; display: flex; flex-direction: column; gap: 4px;
  }
  .exec-info { display: flex; justify-content: space-between; align-items: center; }
  .result-box { background: #09090b; padding: 6px; border-radius: 4px; font-size: 0.75rem; max-height: 120px; overflow: auto; margin: 0; }
  .error-box { color: #f87171; font-size: 0.75rem; margin: 0; }
  .modal-footer {
    padding: 16px; border-top: 1px solid #333; display: flex; justify-content: flex-end;
  }
  .tech-button {
    background: #3a3a44; color: #fff; border: 1px solid #4a4a55; border-radius: 4px;
    padding: 6px 12px; font-size: 0.85rem; cursor: pointer;
  }
  .tech-button:hover:not(:disabled) { background: #4a4a55; }
  .tech-button:disabled { opacity: 0.5; cursor: not-allowed; }
  .tech-button.primary { background: #2563eb; border-color: #3b82f6; }
  .tech-button.primary:hover:not(:disabled) { background: #1d4ed8; }
  .tech-button.danger { background: #991b1b; border-color: #b91c1c; }
  .tech-button.danger:hover:not(:disabled) { background: #7f1d1d; }
  .tech-button.run { background: #047857; border-color: #059669; }
  .tech-button.small { padding: 2px 6px; font-size: 0.75rem; }
</style>
