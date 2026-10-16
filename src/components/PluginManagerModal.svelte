<script>
  import { onMount } from "svelte";
  import AppDialog from "./AppDialog.svelte";
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

<AppDialog title="Eklenti Yöneticisi (Plugin Manager)" size="lg" onclose={() => onclose?.()}>
    <div class="plugin-manager-body">
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

    {#snippet footer()}
      <button
        class="tech-button primary"
        onclick={handlePickPackage}
        disabled={isLoading || !featurePresentation.runnable}
      >
        {isLoading ? "İnceleniyor..." : "Yeni Eklenti Paketi Seç..."}
      </button>
    {/snippet}
</AppDialog>

<style>
  .plugin-manager-body { display: flex; flex-direction: column; gap: 12px; }
  .plugin-warning, .plugin-success { padding: 8px 12px; border-radius: var(--radius); font-size: 12px; margin: 0; }
  .plugin-warning { color: var(--destructive); background: color-mix(in srgb, var(--destructive) 10%, transparent); }
  .plugin-success { color: var(--success); background: color-mix(in srgb, var(--success) 10%, transparent); }
  .inspection-card {
    background: var(--card); border: 1px solid color-mix(in srgb, var(--primary) 45%, transparent); border-radius: var(--radius); padding: 14px;
    display: flex; flex-direction: column; gap: 10px;
  }
  .unsigned-warning-box { background: var(--warning); color: var(--warning-foreground); padding: 8px 12px; border-radius: calc(var(--radius) * 0.8); font-size: 12px; }
  .permission-explanation-list { margin: 6px 0 0 0; padding-left: 18px; list-style: disc; font-size: 12px; display: flex; flex-direction: column; gap: 4px; }
  .perm-desc { color: var(--muted-foreground); font-size: 12px; }
  .consent-checkbox-row { margin-top: 10px; padding: 8px 10px; background: var(--muted); border-radius: calc(var(--radius) * 0.8); }
  .consent-label { display: flex; align-items: center; gap: 8px; font-size: 12px; cursor: pointer; }
  .consent-label input[type="checkbox"] { cursor: pointer; accent-color: var(--primary); }
  .inspection-header { display: flex; justify-content: space-between; align-items: center; }
  .inspection-header h4 { margin: 0; font-size: 13px; font-weight: 600; }
  .inspection-details p { margin: 4px 0; font-size: 12px; }
  .digest { font-family: var(--font-code); font-size: 11px; word-break: break-all; color: var(--muted-foreground); }
  .perms-section { margin-top: 6px; font-size: 12px; }
  .dim { color: var(--muted-foreground); }
  .badge-list { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
  .badge { background: var(--muted); color: var(--muted-foreground); padding: 1px 6px; border-radius: calc(var(--radius) * 0.6); font-size: 11px; }
  .badge.permission { background: color-mix(in srgb, var(--primary) 12%, transparent); color: var(--primary); }
  .badge.hook { background: color-mix(in srgb, var(--success) 14%, transparent); color: var(--success); }
  .badge.warning { background: var(--warning); color: var(--warning-foreground); }
  .badge.active-badge { background: color-mix(in srgb, var(--success) 14%, transparent); color: var(--success); }
  .badge.small { font-size: 10px; padding: 0 4px; }
  .inspection-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
  .plugin-list { display: flex; flex-direction: column; gap: 8px; }
  .empty-state { text-align: center; color: var(--muted-foreground); padding: 24px; font-size: 13px; }
  .plugin-card { background: var(--card); border: 1px solid var(--border); border-radius: var(--radius); padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  .plugin-title-row { display: flex; align-items: center; gap: 8px; }
  .plugin-id { font-size: 11px; color: var(--muted-foreground); font-family: var(--font-code); }
  .plugin-desc { font-size: 12px; color: var(--muted-foreground); margin: 4px 0; }
  .plugin-actions { display: flex; gap: 6px; justify-content: flex-end; }
  .execution-box { background: var(--muted); border-radius: calc(var(--radius) * 0.8); padding: 8px; font-size: 12px; display: flex; flex-direction: column; gap: 4px; }
  .exec-info { display: flex; justify-content: space-between; align-items: center; }
  .result-box { background: var(--background); border: 1px solid var(--border); padding: 6px; border-radius: calc(var(--radius) * 0.6); font-family: var(--font-code); font-size: 11px; max-height: 120px; overflow: auto; margin: 0; }
  .error-box { color: var(--destructive); font-size: 11px; margin: 0; }
  .tech-button {
    background: var(--secondary); color: var(--secondary-foreground); border: 1px solid var(--border); border-radius: calc(var(--radius) * 0.8);
    padding: 4px 12px; font-size: 12px; font-weight: 500; cursor: pointer;
  }
  .tech-button:hover:not(:disabled) { background: var(--accent); }
  .tech-button:disabled { opacity: 0.5; cursor: not-allowed; }
  .tech-button.primary { background: var(--primary); border-color: transparent; color: var(--primary-foreground); }
  .tech-button.primary:hover:not(:disabled) { filter: brightness(1.08); }
  .tech-button.danger { background: color-mix(in srgb, var(--destructive) 10%, transparent); border-color: transparent; color: var(--destructive); }
  .tech-button.danger:hover:not(:disabled) { background: color-mix(in srgb, var(--destructive) 18%, transparent); }
  .tech-button.run { background: color-mix(in srgb, var(--success) 14%, transparent); border-color: transparent; color: var(--success); }
  .tech-button.small { padding: 2px 6px; font-size: 11px; }
</style>
