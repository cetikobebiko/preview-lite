<script>
  let {
    isPdf, viewer, ui, fileName, activePdfDisplayPath, featureUi, featureControlLabel,
    onFeatureAction, onSelectFile,
  } = $props();
</script>

<header class="telemetry-header">
  <div class="sidebar-drag-area" data-tauri-drag-region>
    <span class="system-tag">{(isPdf || viewer.activeImageSourcePath) ? `[SYS_DOC // ${viewer.fileExt.toUpperCase()}]` : "[SYS_IDLE]"}</span>
    <span class="file-title-raw" title={isPdf ? activePdfDisplayPath : viewer.activeImageSourcePath}>{(isPdf || viewer.activeImageSourcePath) ? fileName : "NO_DOCUMENT_LOADED"}</span>
  </div>
  <div class="header-actions">
    {#if isPdf}
      <button class="hud-btn {ui.panels.sidebar ? 'active' : ''}" onclick={() => ui.togglePanel("sidebar")} title="Sol Paneli Aç/Kapat (Cmd + B)">[SIDEBAR]</button>
      {#if featureUi("basic_accessibility_audit").visible}
        <button class="hud-btn {ui.panels.a11y ? 'active' : ''}" onclick={() => onFeatureAction("basic_accessibility_audit", () => ui.togglePanel("a11y"))} title={featureUi("basic_accessibility_audit").warning} disabled={!featureUi("basic_accessibility_audit").runnable}>{featureControlLabel("basic_accessibility_audit", "A11Y")}</button>
      {/if}
      {#if featureUi("print_resource_inspection").visible || featureUi("rgb_derived_print_preview").visible}
        <button class="hud-btn {ui.panels.prepress ? 'active' : ''}" onclick={() => onFeatureAction("print_resource_inspection", () => ui.togglePanel("prepress"))} title={featureUi("print_resource_inspection").warning} disabled={!featureUi("print_resource_inspection").runnable}>{featureControlLabel("print_resource_inspection", "PRINT")}</button>
      {/if}
      {#if featureUi("sensitive_data_scan").visible}
        <button class="hud-btn {ui.panels.sensitiveDataScan ? 'active' : ''}" onclick={() => onFeatureAction("sensitive_data_scan", () => ui.togglePanel("sensitiveDataScan"))} title={featureUi("sensitive_data_scan").warning} disabled={!featureUi("sensitive_data_scan").runnable}>{featureControlLabel("sensitive_data_scan", "SCAN")}</button>
      {/if}
    {/if}
    <button class="hud-btn" onclick={onSelectFile} title="Farklı Dosya Aç (Cmd + O)">[OPEN]</button>
    {#if featureUi("page_manipulation").visible}
      <button class="hud-btn {ui.isModalOpen("page_organizer") ? 'active' : ''}" onclick={() => onFeatureAction("page_manipulation", () => ui.openModal("page_organizer"))} title={featureUi("page_manipulation").warning} disabled={!featureUi("page_manipulation").runnable}>{featureControlLabel("page_manipulation", "ORGANIZE")}</button>
      <button class="hud-btn {ui.isModalOpen("merge_split") ? 'active' : ''}" onclick={() => onFeatureAction("page_manipulation", () => ui.openModal("merge_split"))} title={featureUi("page_manipulation").warning} disabled={!featureUi("page_manipulation").runnable}>{featureControlLabel("page_manipulation", "MERGE/SPLIT")}</button>
    {/if}
    {#if featureUi("command_palette").visible}
      <button class="hud-btn" onclick={() => onFeatureAction("command_palette", () => ui.openModal("command_palette"))} title={featureUi("command_palette").warning} disabled={!featureUi("command_palette").runnable}>{featureControlLabel("command_palette", "CMD")}</button>
    {/if}
  </div>
</header>
