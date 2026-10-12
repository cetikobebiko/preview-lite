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

{#if ui.isModalOpen("export")}<ExportModal sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} maxPages={viewer.pageOrder.length} on:close={() => ui.closeModal("export")} on:success={(event) => onToast(event.detail)} />{/if}
{#if ui.isModalOpen("shortcuts")}<ShortcutsModal on:close={() => ui.closeModal("shortcuts")} />{/if}
{#if ui.isModalOpen("page_organizer") && featureUi("page_manipulation").visible}<PageOrganizer sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} on:close={() => ui.closeModal("page_organizer")} on:orderSaved={(event) => onSessionMutation(event.detail.snapshot, event.detail)} on:pageMutated={(event) => onSessionMutation(event.detail.snapshot, event.detail)} />{/if}
{#if ui.isModalOpen("merge_split") && featureUi("page_manipulation").visible}<MergeSplitModal currentFilePath={activePdfDisplayPath} maxPages={viewer.pageOrder.length} sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} on:close={() => ui.closeModal("merge_split")} />{/if}
<ReadingRuler isActive={ui.panels.readingRuler} />
<MetadataModal isOpen={ui.isModalOpen("metadata")} metadata={viewer.pdfMetadata} on:close={() => ui.closeModal("metadata")} />
<BionicReadingModal isOpen={ui.isModalOpen("bionic")} text={viewer.bionicText} source={viewer.bionicSource} onclose={() => ui.closeModal("bionic")} />
<AnalyticsModal isOpen={ui.isModalOpen("analytics")} {fileName} totalPages={viewer.pageOrder.length} currentPage={viewer.currentPageIndex + 1} readingTimeSeconds={viewer.readingTimeSeconds} scale={viewer.scale} on:close={() => ui.closeModal("analytics")} />
