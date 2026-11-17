<script>
  import { Input } from "$lib/components/ui/input";
  import { Label } from "$lib/components/ui/label";
  import { JPEG_QUALITIES, OPTIMIZE_PRESETS, presetSettings } from "../services/pdfOptimizer.js";

  // Shared by the single-document optimize dialog and the batch wizard.
  let {
    settings = $bindable(),
    presetId = $bindable("standard"),
    disabled = false,
    idPrefix = "optimize",
  } = $props();

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
</script>

<div class="grid gap-1">
  <Label for="{idPrefix}-preset" class="text-[11px] text-muted-foreground">Ayar</Label>
  <select id="{idPrefix}-preset" value={presetId} onchange={(event) => choosePreset(event.currentTarget.value)} class={selectClass} {disabled}>
    {#each OPTIMIZE_PRESETS as preset (preset.id)}
      <option value={preset.id}>{preset.label}</option>
    {/each}
    <option value="custom">Özel</option>
  </select>
  {#if presetDescription}<small class="text-[11px] text-muted-foreground">{presetDescription}</small>{/if}
</div>

{#each IMAGE_GROUPS as group (group.key)}
  <fieldset class="grid gap-2 rounded-md border border-border p-2.5 text-xs" {disabled}>
    <legend class="px-1 text-[11px] text-muted-foreground">{group.label}</legend>
    <label class="flex items-center gap-2">
      <input type="checkbox" bind:checked={settings[group.key].downsample} onchange={edited} />
      Yeniden örnekle (bikübik)
    </label>
    <div class="grid grid-cols-2 gap-2">
      <div class="grid gap-1">
        <Label for="{idPrefix}-{group.key}-target" class="text-[11px] text-muted-foreground">Hedef çözünürlük (ppi)</Label>
        <Input id="{idPrefix}-{group.key}-target" type="number" min="9" max="2400" bind:value={settings[group.key].targetPpi} oninput={edited} disabled={disabled || !settings[group.key].downsample} />
      </div>
      <div class="grid gap-1">
        <Label for="{idPrefix}-{group.key}-above" class="text-[11px] text-muted-foreground">Şu çözünürlüğün üstündekiler (ppi)</Label>
        <Input id="{idPrefix}-{group.key}-above" type="number" min="9" max="2400" bind:value={settings[group.key].abovePpi} oninput={edited} disabled={disabled || !settings[group.key].downsample} />
      </div>
      <div class="grid gap-1">
        <Label for="{idPrefix}-{group.key}-compression" class="text-[11px] text-muted-foreground">Sıkıştırma</Label>
        <select id="{idPrefix}-{group.key}-compression" value={settings[group.key].compression.kind} onchange={(event) => setCompression(group.key, event.currentTarget.value)} class={selectClass}>
          <option value="jpeg">JPEG</option>
          <option value="flate">Kayıpsız (Flate)</option>
          <option value="retain">Görüntünün kendi sıkıştırması</option>
        </select>
      </div>
      {#if settings[group.key].compression.kind === "jpeg"}
        <div class="grid gap-1">
          <Label for="{idPrefix}-{group.key}-quality" class="text-[11px] text-muted-foreground">JPEG kalitesi</Label>
          <select id="{idPrefix}-{group.key}-quality" bind:value={settings[group.key].compression.quality} onchange={edited} class={selectClass}>
            {#each JPEG_QUALITIES as quality (quality.value)}
              <option value={quality.value}>{quality.label}</option>
            {/each}
          </select>
        </div>
      {/if}
    </div>
  </fieldset>
{/each}

<fieldset class="grid gap-1.5 text-xs" {disabled}>
  <legend class="mb-1 text-[11px] text-muted-foreground">Temizlik</legend>
  {#each Object.entries(CLEAN_UP_LABELS) as [key, label] (key)}
    <label class="flex items-center gap-2"><input type="checkbox" bind:checked={settings[key]} onchange={edited} /> {label}</label>
  {/each}
</fieldset>
