<script>
  import Settings2Icon from "@lucide/svelte/icons/settings-2";
  import Sidebar from "./Sidebar.svelte";
  import Inspector from "./Inspector.svelte";
  import SensitiveDataScanPanel from "./SensitiveDataScanPanel.svelte";
  import DocumentComparePanel from "./DocumentComparePanel.svelte";
  import SecureRedactionPanel from "./SecureRedactionPanel.svelte";
  import A11yPanel from "./A11yPanel.svelte";
  import PrepressPanel from "./PrepressPanel.svelte";
  import AiSidebar from "./AiSidebar.svelte";

  let {
    isPdf, ui, viewer, documentSession, sensitiveScanState, documentCompareState, secureRedactionState, secureRedaction, featureUi, physicalPageIndex, isLoadedPdfDocument,
    onJump, onJumpPhysical, onScanSensitiveData, onSessionSnapshot, onProcessPreview, onCoveragePreview, onChannels, children,
  } = $props();

  const inspectorPanels = $derived([
    ui.panels.secureRedaction && isPdf && featureUi("redaction").visible
      && { id: "secureRedaction", title: "Güvenli redaksiyon" },
    ui.panels.sensitiveDataScan && isPdf && featureUi("sensitive_data_scan").visible
      && { id: "sensitiveDataScan", title: "Hassas veri tespiti" },
    ui.panels.documentCompare && isPdf && documentCompareState && featureUi("document_compare").visible
      && { id: "documentCompare", title: "Belge karşılaştırma" },
    ui.panels.a11y && isPdf && featureUi("basic_accessibility_audit").visible
      && { id: "a11y", title: "Basic accessibility audit" },
    ui.panels.prepress && isPdf && (featureUi("print_resource_inspection").visible || featureUi("rgb_derived_print_preview").visible)
      && { id: "prepress", title: "Print Inspection & Approximate Preview" },
    ui.isModalOpen("ai_assistant") && featureUi("local_ai").visible
      && { id: "ai_assistant", title: "Yapay Zeka (Ollama)", actions: aiActions },
  ].filter(Boolean));

  function closePanel(id) {
    if (id === "ai_assistant") ui.closeModal("ai_assistant");
    else ui.setPanel(id, false);
  }

  const toDisplayIndex = (pageIndex) => viewer.pageOrder.indexOf(pageIndex);
</script>

{#snippet aiActions()}
  <button
    type="button"
    class="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
    title="Ayarlar"
    aria-label="Yapay zekâ ayarları"
    onclick={() => ui.openModal("ai_settings")}
  ><Settings2Icon class="size-4" /></button>
{/snippet}

{#snippet panelContent(id)}
  {#if id === "secureRedaction"}
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
        const displayIndex = toDisplayIndex(pageIndex);
        if (displayIndex >= 0) onJump(displayIndex, true);
      }}
    />
  {:else if id === "sensitiveDataScan"}
    <SensitiveDataScanPanel
      scanState={sensitiveScanState}
      onScan={onScanSensitiveData}
      canPromote={featureUi("redaction").runnable}
      onPromote={secureRedaction?.promoteSensitiveScanItem}
      onNavigate={(physicalPageIndex) => {
        const displayIndex = toDisplayIndex(physicalPageIndex);
        if (displayIndex >= 0) {
          onJump(displayIndex, true);
        }
      }}
    />
  {:else if id === "documentCompare"}
    <DocumentComparePanel
      compareState={documentCompareState}
      sessionId={documentSession.activeSessionId ?? ""}
      contentGeneration={documentSession.contentGeneration}
      onNavigate={(physicalPageIndex) => {
        const displayIndex = toDisplayIndex(physicalPageIndex);
        if (displayIndex >= 0) onJump(displayIndex, true);
      }}
    />
  {:else if id === "a11y"}
    <A11yPanel sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} />
  {:else if id === "prepress"}
    <PrepressPanel scale={viewer.scale} sessionId={documentSession.activeSessionId ?? ""} pageIndex={physicalPageIndex} contentGeneration={documentSession.contentGeneration} isDocumentReady={isLoadedPdfDocument} onprocessPreviewReady={(detail) => onProcessPreview({ detail })} oncoveragePreviewReady={(detail) => onCoveragePreview({ detail })} onchannelsChanged={(detail) => onChannels({ detail })} />
  {:else if id === "ai_assistant"}
    <AiSidebar sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} />
  {/if}
{/snippet}

{#if isPdf && ui.panels.sidebar}
  <Sidebar sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} pageOrder={viewer.pageOrder} currentPageIndex={viewer.currentPageIndex} onJump={onJump} onJumpPhysical={onJumpPhysical} onEditOutline={featureUi("page_manipulation").visible ? () => ui.openModal("outline_editor") : null} />
{/if}

{@render children?.()}

{#if inspectorPanels.length > 0}
  <Inspector panels={inspectorPanels} onclose={closePanel} content={panelContent} />
{/if}
