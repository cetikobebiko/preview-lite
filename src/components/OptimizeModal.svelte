<script>
  import AppDialog from "./AppDialog.svelte";
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { Button } from "$lib/components/ui/button";
  import { save } from "@tauri-apps/plugin-dialog";
  import { saveSessionOptimizedCopy } from "../services/backend/session.js";
  import {
    JPEG_QUALITIES,
    OPTIMIZE_PRESETS,
    optimizeDetails,
    optimizeSettingsError,
    optimizeSummary,
    presetSettings,
  } from "../services/pdfOptimizer.js";
  import { notifyPluginExportCompleted } from "../services/app/pluginHookDispatcher.js";

  let { sessionId = "", contentGeneration = 0, fileName = "", pageCount = 0, onsaved, onclose } = $props();

  const selectClass = "h-9 w-full rounded-md border border-input bg-background px-2.5 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";
  const IMAGE_GROUPS = Object.freeze([
    { key: "color", label: "Renkli görüntüler" },
    { key: "gray", label: "Gri tonlamalı görüntüler" },
  ]);
  const CLEAN_UP_LABELS = Object.freeze({
    removeUnusedXobjects: "Sayfada listelenip çizilmeyen görüntü ve formları at",
    discardThumbnails: "Sayfa küçük resimlerini at",
    discardAlternateImages: "Alternatif görüntüleri at",
    flateUnencoded: "Sıkıştırılmamış akışları sıkıştır",
    objectStreams: "Nesne akışları kullan (PDF 1.5)",
  });

  let presetId = $state("standard");
  let settings = $state(presetSettings("standard"));
  let saving = $state(false);
  let error = $state("");
  let report = $state(null);

  const validationError = $derived(optimizeSettingsError(settings));
  const canSave = $derived(Boolean(sessionId) && !saving && !validationError);
  const presetDescription = $derived(OPTIMIZE_PRESETS.find((preset) => preset.id === presetId)?.description ?? "");

  function choosePreset(id) {
    presetId = id;
    if (id !== "custom") settings = presetSettings(id);
  }

  function edited() {
    presetId = "custom";
  }

  function setCompression(key, kind) {
    settings[key].compression = kind === "jpeg" ? { kind, quality: 60 } : { kind };
    edited();
  }

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
    <div class="grid gap-1">
      <Label for="optimize-preset" class="text-[11px] text-muted-foreground">Ayar</Label>
      <select id="optimize-preset" value={presetId} onchange={(event) => choosePreset(event.currentTarget.value)} class={selectClass} disabled={saving}>
        {#each OPTIMIZE_PRESETS as preset (preset.id)}
          <option value={preset.id}>{preset.label}</option>
        {/each}
        <option value="custom">Özel</option>
      </select>
      {#if presetDescription}<small class="text-[11px] text-muted-foreground">{presetDescription}</small>{/if}
    </div>

    {#each IMAGE_GROUPS as group (group.key)}
      <fieldset class="grid gap-2 rounded-md border border-border p-2.5 text-xs" disabled={saving}>
        <legend class="px-1 text-[11px] text-muted-foreground">{group.label}</legend>
        <label class="flex items-center gap-2">
          <input type="checkbox" bind:checked={settings[group.key].downsample} onchange={edited} />
          Yeniden örnekle (bikübik)
        </label>
        <div class="grid grid-cols-2 gap-2">
          <div class="grid gap-1">
            <Label for="optimize-{group.key}-target" class="text-[11px] text-muted-foreground">Hedef çözünürlük (ppi)</Label>
            <Input id="optimize-{group.key}-target" type="number" min="9" max="2400" bind:value={settings[group.key].targetPpi} oninput={edited} disabled={saving || !settings[group.key].downsample} />
          </div>
          <div class="grid gap-1">
            <Label for="optimize-{group.key}-above" class="text-[11px] text-muted-foreground">Şu çözünürlüğün üstündekiler (ppi)</Label>
            <Input id="optimize-{group.key}-above" type="number" min="9" max="2400" bind:value={settings[group.key].abovePpi} oninput={edited} disabled={saving || !settings[group.key].downsample} />
          </div>
          <div class="grid gap-1">
            <Label for="optimize-{group.key}-compression" class="text-[11px] text-muted-foreground">Sıkıştırma</Label>
            <select id="optimize-{group.key}-compression" value={settings[group.key].compression.kind} onchange={(event) => setCompression(group.key, event.currentTarget.value)} class={selectClass}>
              <option value="jpeg">JPEG</option>
              <option value="flate">Kayıpsız (Flate)</option>
              <option value="retain">Görüntünün kendi sıkıştırması</option>
            </select>
          </div>
          {#if settings[group.key].compression.kind === "jpeg"}
            <div class="grid gap-1">
              <Label for="optimize-{group.key}-quality" class="text-[11px] text-muted-foreground">JPEG kalitesi</Label>
              <select id="optimize-{group.key}-quality" bind:value={settings[group.key].compression.quality} onchange={edited} class={selectClass}>
                {#each JPEG_QUALITIES as quality (quality.value)}
                  <option value={quality.value}>{quality.label}</option>
                {/each}
              </select>
            </div>
          {/if}
        </div>
      </fieldset>
    {/each}

    <fieldset class="grid gap-1.5 text-xs" disabled={saving}>
      <legend class="mb-1 text-[11px] text-muted-foreground">Temizlik</legend>
      {#each Object.entries(CLEAN_UP_LABELS) as [key, label] (key)}
        <label class="flex items-center gap-2"><input type="checkbox" bind:checked={settings[key]} onchange={edited} /> {label}</label>
      {/each}
    </fieldset>

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
