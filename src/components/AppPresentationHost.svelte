<script>
  import ReadingRuler from "./ReadingRuler.svelte";
  import BionicReadingModal from "./BionicReadingModal.svelte";
  import AnalyticsModal from "./AnalyticsModal.svelte";
  import MetadataModal from "./MetadataModal.svelte";
  import ExportModal from "./ExportModal.svelte";
  import ShortcutsModal from "./ShortcutsModal.svelte";
  import PageOrganizer from "./PageOrganizer.svelte";
  import MergeSplitModal from "./MergeSplitModal.svelte";

  let { ui, viewer, documentSession, featureUi, activePdfDisplayPath, fileName, onToast, onSessionMutation } = $props();
</script>

{#if ui.isModalOpen("export")}<ExportModal sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} maxPages={viewer.pageOrder.length} pageOrder={viewer.pageOrder} onclose={() => ui.closeModal("export")} onsuccess={(message) => onToast(message)} />{/if}
{#if ui.isModalOpen("shortcuts")}<ShortcutsModal onclose={() => ui.closeModal("shortcuts")} />{/if}
{#if ui.isModalOpen("page_organizer") && featureUi("page_manipulation").visible}<PageOrganizer sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} onclose={() => ui.closeModal("page_organizer")} onorderSaved={(detail) => onSessionMutation(detail.snapshot, detail)} onpageMutated={(detail) => onSessionMutation(detail.snapshot, detail)} />{/if}
{#if ui.isModalOpen("merge_split") && featureUi("page_manipulation").visible}<MergeSplitModal currentFilePath={activePdfDisplayPath} maxPages={viewer.pageOrder.length} pageOrder={viewer.pageOrder} sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} onclose={() => ui.closeModal("merge_split")} onsuccess={(message) => onToast(message)} />{/if}
<ReadingRuler isActive={ui.panels.readingRuler} />
<MetadataModal isOpen={ui.isModalOpen("metadata")} metadata={viewer.pdfMetadata} onclose={() => ui.closeModal("metadata")} />
<BionicReadingModal isOpen={ui.isModalOpen("bionic")} text={viewer.bionicText} source={viewer.bionicSource} onclose={() => ui.closeModal("bionic")} />
<AnalyticsModal isOpen={ui.isModalOpen("analytics")} {fileName} totalPages={viewer.pageOrder.length} currentPage={viewer.currentPageIndex + 1} readingTimeSeconds={viewer.readingTimeSeconds} scale={viewer.scale} onclose={() => ui.closeModal("analytics")} />
