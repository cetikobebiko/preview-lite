<script>
  import ReadingRuler from "./ReadingRuler.svelte";
  import BionicReadingModal from "./BionicReadingModal.svelte";
  import AnalyticsModal from "./AnalyticsModal.svelte";
  import MetadataModal from "./MetadataModal.svelte";
  import ExportModal from "./ExportModal.svelte";
  import ShortcutsModal from "./ShortcutsModal.svelte";
  import PageOrganizer from "./PageOrganizer.svelte";
  import MergeSplitModal from "./MergeSplitModal.svelte";
  import PageLabelsModal from "./PageLabelsModal.svelte";
  import OutlineEditorModal from "./OutlineEditorModal.svelte";
  import PageMarksModal from "./PageMarksModal.svelte";

  let { ui, viewer, documentSession, featureUi, activePdfDisplayPath, fileName, onToast, onSessionMutation } = $props();

  const editable = $derived(documentSession.active?.editable !== false);
  const physicalPageIndex = $derived(viewer.pageOrder[viewer.currentPageIndex] ?? viewer.currentPageIndex);
</script>

{#if ui.isModalOpen("export")}<ExportModal sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} maxPages={viewer.pageOrder.length} pageOrder={viewer.pageOrder} onclose={() => ui.closeModal("export")} onsuccess={(message) => onToast(message)} />{/if}
{#if ui.isModalOpen("shortcuts")}<ShortcutsModal onclose={() => ui.closeModal("shortcuts")} />{/if}
{#if ui.isModalOpen("page_organizer") && featureUi("page_manipulation").visible}<PageOrganizer sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} onclose={() => ui.closeModal("page_organizer")} onorderSaved={(detail) => onSessionMutation(detail.snapshot, detail)} onpageMutated={(detail) => onSessionMutation(detail.snapshot, detail)} />{/if}
{#if ui.isModalOpen("merge_split") && featureUi("page_manipulation").visible}<MergeSplitModal currentFilePath={activePdfDisplayPath} maxPages={viewer.pageOrder.length} pageOrder={viewer.pageOrder} sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} onclose={() => ui.closeModal("merge_split")} onsuccess={(message) => onToast(message)} />{/if}
<ReadingRuler isActive={ui.panels.readingRuler} />
<MetadataModal isOpen={ui.isModalOpen("metadata")} metadata={viewer.pdfMetadata} sessionId={documentSession.activeSessionId ?? ""} contentGeneration={documentSession.contentGeneration} {editable} onsaved={(snapshot, metadata) => { viewer.pdfMetadata = metadata; onToast("Meta veriler kaydedildi."); onSessionMutation(snapshot); }} onclose={() => ui.closeModal("metadata")} />
{#if ui.isModalOpen("page_labels") && documentSession.activeSessionId}<PageLabelsModal sessionId={documentSession.activeSessionId} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} {editable} onsaved={(snapshot) => { onToast("Sayfa etiketleri kaydedildi."); onSessionMutation(snapshot); }} onclose={() => ui.closeModal("page_labels")} />{/if}
{#if ui.isModalOpen("page_marks") && documentSession.activeSessionId}<PageMarksModal sessionId={documentSession.activeSessionId} contentGeneration={documentSession.contentGeneration} pageCount={viewer.pageOrder.length} pageOrder={viewer.pageOrder} {editable} onsaved={(snapshot) => { onToast("Sayfa işaretleri uygulandı."); onSessionMutation(snapshot); }} onclose={() => ui.closeModal("page_marks")} />{/if}
{#if ui.isModalOpen("outline_editor") && documentSession.activeSessionId}<OutlineEditorModal sessionId={documentSession.activeSessionId} contentGeneration={documentSession.contentGeneration} currentPageIndex={physicalPageIndex} pageCount={viewer.pageOrder.length} {editable} onsaved={(snapshot) => { onToast("Yer imleri kaydedildi."); onSessionMutation(snapshot); }} onclose={() => ui.closeModal("outline_editor")} />{/if}
<BionicReadingModal isOpen={ui.isModalOpen("bionic")} text={viewer.bionicText} source={viewer.bionicSource} onclose={() => ui.closeModal("bionic")} />
<AnalyticsModal isOpen={ui.isModalOpen("analytics")} {fileName} totalPages={viewer.pageOrder.length} currentPage={viewer.currentPageIndex + 1} readingTimeSeconds={viewer.readingTimeSeconds} scale={viewer.scale} onclose={() => ui.closeModal("analytics")} />
