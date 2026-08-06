<script>
  let { annotation, ui, featureUi, featureControlLabel, colorPalette, isDirty, onSave } = $props();
</script>

<div class="hud-group">
  <button class="hud-btn {annotation.activeTool === 'pointer' ? 'active' : ''}" onclick={() => (annotation.activeTool = "pointer")} title="İşaretçi (Seçim)">[PTR]</button>
  {#if featureUi("annotation").visible}
    <span class="hud-telemetry-badge" title={featureUi("annotation").warning}>{featureControlLabel("annotation", "ANNOTATION")}</span>
    <button class="hud-btn {annotation.activeTool === 'highlight' ? 'active' : ''}" onclick={() => (annotation.activeTool = "highlight")} title={featureUi("annotation").warning} disabled={!featureUi("annotation").runnable}>[HL]</button>
    <button class="hud-btn {annotation.activeTool === 'underline' ? 'active' : ''}" onclick={() => (annotation.activeTool = "underline")} title={featureUi("annotation").warning} disabled={!featureUi("annotation").runnable}>[UL]</button>
    <button class="hud-btn {annotation.activeTool === 'text' ? 'active' : ''}" onclick={() => (annotation.activeTool = "text")} title={featureUi("annotation").warning} disabled={!featureUi("annotation").runnable}>[TXT]</button>
    <input type="color" bind:value={annotation.activeColor} class="hud-color-picker" title="Açıklama Rengi" disabled={!featureUi("annotation").runnable} />
    {#if isDirty}<button class="hud-btn active save-btn" onclick={onSave} title="Çalışma kopyasını atomik olarak kaydet">[KAYDET]</button>{/if}
    <button class="hud-btn {ui.settingsPopover ? 'active' : ''}" onclick={() => (ui.settingsPopover = !ui.settingsPopover)} disabled={!featureUi("annotation").runnable} title="Annotation çizim ayarları">⚙</button>
  {/if}
</div>

{#if ui.settingsPopover && featureUi("annotation").runnable}
  <div class="settings-popover">
    <div style="display: flex; gap: 6px; align-items: center;">
      {#each colorPalette as color}
        <button class="color-swatch {annotation.strokeColor === color ? 'active' : ''}" style="background-color: {color};" onclick={() => { annotation.strokeColor = color; ui.settingsPopover = false; }} title="Renk: {color}"></button>
      {/each}
    </div>
    <div style="display: flex; align-items: center; gap: 6px; border-left: 1px solid var(--border-raw); padding-left: 10px;">
      <span style="font-size: 10px; color: var(--text-muted);">({annotation.strokeSize}px):</span>
      <input type="range" min="1" max="16" bind:value={annotation.strokeSize} class="stroke-slider" />
    </div>
  </div>
{/if}
