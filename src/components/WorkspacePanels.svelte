<script>
  import Sidebar from "./Sidebar.svelte";
  import SensitiveDataScanPanel from "./SensitiveDataScanPanel.svelte";
  import SecureRedactionPanel from "./SecureRedactionPanel.svelte";
  import A11yPanel from "./A11yPanel.svelte";
  import PrepressPanel from "./PrepressPanel.svelte";

  let {
    isPdf, ui, viewer, documentSession, sensitiveScanState, secureRedactionState, secureRedaction, featureUi, physicalPageIndex, isLoadedPdfDocument,
    onJump, onScanSensitiveData, onSessionSnapshot, onSeparations, onHeatmap, onChannels, onGuides, onApplyBoxes,
  } = $props();
</script>

{#if isPdf && ui.panels.sidebar}
  <Sidebar sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} currentPageIndex={viewer.currentPageIndex} onJump={onJump} />
{/if}
{#if ui.panels.secureRedaction && isPdf && featureUi("redaction").visible}
  <SecureRedactionPanel
    state={secureRedactionState}
    onPlan={secureRedaction.plan}
    onExecute={secureRedaction.execute}
    onCancelPlan={secureRedaction.cancelPlanning}
    onCancelExecution={secureRedaction.cancelExecution}
    onToggleSelection={secureRedaction.setSelectionMode}
    onRemoveRegion={secureRedaction.removeRegion}
    onClearRegions={secureRedaction.clearRegions}
    onPolicyChange={secureRedaction.updatePolicy}
    onAcknowledgementChange={secureRedaction.setAcknowledgement}
    onNewRedaction={secureRedaction.newRedaction}
    onNavigate={(pageIndex) => {
      const displayIndex = viewer.pageOrder.indexOf(pageIndex);
      if (displayIndex >= 0) onJump(displayIndex, true);
    }}
  />
{/if}
{#if ui.panels.sensitiveDataScan && isPdf && featureUi("sensitive_data_scan").visible}
  <SensitiveDataScanPanel
    scanState={sensitiveScanState}
    onScan={onScanSensitiveData}
    canPromote={featureUi("redaction").runnable}
    onPromote={secureRedaction?.promoteSensitiveScanItem}
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
