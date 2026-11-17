<script>
  import AppDialog from "./AppDialog.svelte";
  import OptimizeSettingsFields from "./OptimizeSettingsFields.svelte";
  import { Button } from "$lib/components/ui/button";
  import { save } from "@tauri-apps/plugin-dialog";
  import { saveSessionOptimizedCopy } from "../services/backend/session.js";
  import {
    optimizeDetails,
    optimizeSettingsError,
    optimizeSummary,
    presetSettings,
  } from "../services/pdfOptimizer.js";
  import { notifyPluginExportCompleted } from "../services/app/pluginHookDispatcher.js";

  let { sessionId = "", contentGeneration = 0, fileName = "", pageCount = 0, onsaved, onclose } = $props();

  let presetId = $state("standard");
  let settings = $state(presetSettings("standard"));
  let saving = $state(false);
  let error = $state("");
  let report = $state(null);

  const validationError = $derived(optimizeSettingsError(settings));
  const canSave = $derived(Boolean(sessionId) && !saving && !validationError);

  async function run() {
    error = "";
    report = null;
    const stem = (fileName || "belge").replace(/\.pdf$/i, "");
    const target = await save({
      title: "Optimize edilmiş kopyayı kaydet",
      defaultPath: `${stem}_kucultulmus.pdf`,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!target) return;
    saving = true;
    try {
      report = await saveSessionOptimizedCopy(sessionId, contentGeneration, $state.snapshot(settings), target);
      notifyPluginExportCompleted("pdf", pageCount);
      onsaved?.(optimizeSummary(report));
    } catch (e) {
      error = e?.message ?? String(e);
    } finally {
      saving = false;
    }
  }

  function close() {
    if (!saving) onclose?.();
  }
</script>

<AppDialog title="Dosya Boyutunu Küçült" description="Görüntüleri yeniden örnekleyip sıkıştırarak ve kullanılmayan nesneleri atarak belgenin daha küçük bir kopyasını kaydedin. Açık belge değişmez." size="lg" onclose={close}>
  <div class="grid gap-3">
    <OptimizeSettingsFields bind:settings bind:presetId disabled={saving} />

    <p class="text-[11px] text-muted-foreground">Görüntü yalnız daha küçük olacaksa değiştirilir. Kopya yeniden yazılır; belgede dijital imza varsa kopyada geçersiz olur. Parola korumalı belgeler optimize edilemez. Siyah-beyaz (1 bit) görüntülere dokunulmaz.</p>

    {#if validationError}
      <p class="text-xs text-destructive">{validationError}</p>
    {/if}
    {#if report}
      <div class="grid gap-1 rounded-md bg-black/5 px-3 py-2 text-xs dark:bg-white/10" role="status">
        <p class="font-medium">{optimizeSummary(report)}</p>
        {#each optimizeDetails(report) as line, index (index)}
          <p class="text-muted-foreground">{line}</p>
        {/each}
      </div>
    {/if}
    {#if error}
      <p class="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive" role="alert">{error}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" size="sm" onclick={close} disabled={saving}>Kapat</Button>
    <Button size="sm" onclick={run} disabled={!canSave}>{saving ? "Küçültülüyor…" : "Kopyayı kaydet…"}</Button>
  {/snippet}
</AppDialog>
