<script>
  import ViewerNavigationDock from "./ViewerNavigationDock.svelte";
  import AnnotationHudControls from "./AnnotationHudControls.svelte";

  let {
    isPdf, viewer, ui, annotation, featureUi, featureControlLabel, colorPalette, isDirty,
    onFeatureAction, onGoBack, onGoForward, onPreviousPage, onNextPage, onPageInputChange,
    onRotateLeft, onRotateRight, onFlipH, onFlipV, onZoomOut, onResetZoom, onZoomIn,
    onToggleSinglePage, onToggleBookMode, onToggleDarkMode, onOpenBionic, onRunOcr,
    onSpeak, onStopSpeaking, onSave, onExportImage,
  } = $props();
</script>

{#if isPdf || viewer.activeImageSourcePath}
  <div class="hud-dock">
    <ViewerNavigationDock
      {isPdf} {viewer} {onGoBack} {onGoForward} {onPreviousPage} {onNextPage}
      {onPageInputChange} {onRotateLeft} {onRotateRight} {onFlipH} {onFlipV}
      {onZoomOut} {onResetZoom} {onZoomIn}
    />

    <AnnotationHudControls {annotation} {ui} {featureUi} {featureControlLabel} {colorPalette} {isDirty} onSave={onSave} />

    <div class="hud-group">
      {#if featureUi("search").visible}
        <button class="hud-btn {ui.panels.search ? 'active' : ''}" onclick={() => onFeatureAction("search", () => ui.togglePanel("search"))} title={featureUi("search").warning} disabled={!featureUi("search").runnable}>{featureControlLabel("search", "FIND")}</button>
      {/if}
      <button class="hud-btn {viewer.isSinglePageMode ? 'active' : ''}" onclick={onToggleSinglePage}>[SINGLE]</button>
      <button class="hud-btn {viewer.isBookMode ? 'active' : ''}" onclick={onToggleBookMode}>[BOOK]</button>
      {#if viewer.isBookMode}
        <button class="hud-btn {viewer.coverMode ? 'active' : ''}" onclick={() => (viewer.coverMode = !viewer.coverMode)}>[COVER {viewer.coverMode ? "ON" : "OFF"}]</button>
      {/if}
      <button class="hud-btn {viewer.isSmartDarkMode ? 'active' : ''}" onclick={onToggleDarkMode}>[DARK]</button>
      <button class="hud-btn" onclick={onOpenBionic}>[BIONIC]</button>
      <button class="hud-btn {ui.panels.readingRuler ? 'active' : ''}" onclick={() => ui.togglePanel("readingRuler")}>[RULER]</button>
      {#if featureUi("macos_vision_ocr").visible}
        <button class="hud-btn" onclick={onRunOcr} title={featureUi("macos_vision_ocr").warning} disabled={!featureUi("macos_vision_ocr").runnable}>{featureControlLabel("macos_vision_ocr", "OCR")}</button>
      {/if}
      {#if featureUi("text_to_speech").visible}
        <button class="hud-btn {viewer.isSpeaking ? 'active' : ''}" onclick={viewer.isSpeaking ? onStopSpeaking : onSpeak} title={featureUi("text_to_speech").warning} disabled={!featureUi("text_to_speech").runnable}>{viewer.isSpeaking ? "[STOP]" : featureControlLabel("text_to_speech", "TTS")}</button>
      {/if}
      {#if featureUi("local_ai").visible}
        <button class="hud-btn {ui.isModalOpen("ai_assistant") ? 'active' : ''}" onclick={() => onFeatureAction("local_ai", () => ui.openModal("ai_assistant"))} title={featureUi("local_ai").warning} disabled={!featureUi("local_ai").runnable}>{featureControlLabel("local_ai", "AI")}</button>
      {/if}
      <button class="hud-btn {ui.isModalOpen("export") ? 'active' : ''}" onclick={() => ui.openModal("export")}>[EXPORT]</button>
      {#if featureUi("plugin_system").visible}
        <button class="hud-btn {ui.isModalOpen("plugin_manager") ? 'active' : ''}" onclick={() => onFeatureAction("plugin_system", () => ui.openModal("plugin_manager"))} disabled={!featureUi("plugin_system").runnable} title={featureUi("plugin_system").warning}>{featureControlLabel("plugin_system", "PLUGINS")}</button>
      {/if}
    </div>

    {#if !isPdf}
      <button class="hud-btn active" onclick={onExportImage}>[CONVERT]</button>
    {/if}
  </div>
{/if}
