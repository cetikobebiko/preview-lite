<script>
  import Sidebar from "./Sidebar.svelte";
  import SensitiveDataScanPanel from "./SensitiveDataScanPanel.svelte";
  import A11yPanel from "./A11yPanel.svelte";
  import PrepressPanel from "./PrepressPanel.svelte";

  let {
    isPdf, ui, viewer, documentSession, sensitiveScanState, featureUi, physicalPageIndex, isLoadedPdfDocument,
    onJump, onScanSensitiveData, onSessionSnapshot, onSeparations, onHeatmap, onChannels, onGuides, onApplyBoxes,
  } = $props();
</script>

{#if isPdf && ui.panels.sidebar}
  <Sidebar sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} currentPageIndex={viewer.currentPageIndex} onJump={onJump} />
{/if}
{#if ui.panels.sensitiveDataScan && isPdf && featureUi("sensitive_data_scan").visible}
  <SensitiveDataScanPanel
    scanState={sensitiveScanState}
    onScan={onScanSensitiveData}
    onNavigate={(physicalPageIndex) => {
      const displayIndex = viewer.pageOrder.indexOf(physicalPageIndex);
      if (displayIndex >= 0) {
        onJump(displayIndex, true);
      }
    }}
  />
{/if}
{#if ui.panels.a11y && isPdf && featureUi("pdf_ua_validation").visible}
  <A11yPanel sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} on:sessionSnapshot={(event) => onSessionSnapshot(event.detail)} on:close={() => ui.setPanel("a11y", false)} />
{/if}
{#if ui.panels.prepress && isPdf && featureUi("prepress_analysis").visible}
  <div class="prepress-sidebar" style="width: 320px; border-right: 1px solid var(--border-raw); background: var(--bg-surface); flex-shrink: 0; overflow-y: auto;">
    <PrepressPanel scale={viewer.scale} sessionId={documentSession.activeSessionId ?? ""} pageIndex={physicalPageIndex} contentGeneration={documentSession.contentGeneration} isDocumentReady={isLoadedPdfDocument} on:separationsReady={onSeparations} on:heatmapReady={onHeatmap} on:channelsChanged={onChannels} on:guidesChanged={onGuides} on:applyBoxes={onApplyBoxes} />
  </div>
{/if}
