import { getCommandById } from "../../commandRegistry.js";
import {
  FeatureUnavailableError,
  guardFeatureAction,
} from "../../featureActionGuard.js";
import {
  getFeaturePresentation,
  getRuntimeFeatureContext,
} from "../../featureRegistry.js";
import { stopSpeaking } from "../../ttsActions.js";
import {
  mergePdfsDirectAction,
  performOcrOnPageAction,
  shareExternalFileMacosAction,
  shareFileMacosAction,
  speakTextAction,
  startSensitiveDataScanAction,
} from "../actions.js";
import {
  exportSessionPageToImage,
  exportSessionTextContent,
  extractSessionPages,
  searchSessionPdfText,
} from "../backend/session.js";
import { convertAndSaveImage } from "../backend/document.js";
import { selectViewportHighlights } from "../../utils/pdfGeometry.js";
import { open, save } from "../platform/dialogs.js";
import { listen } from "../platform/events.js";
import { buildAnnotationMarkdown } from "./annotationMarkdown.js";
import { toPhysicalPageIndex } from "./viewerNavigationModel.js";
import { createCommandActionRouter } from "./commandActionRouter.js";
import { createDocumentExportController } from "./documentExportController.js";
import { createDocumentTransformController } from "./documentTransformController.js";
import { createOcrActionController } from "./ocrActionController.js";
import { createReadingPresentationController } from "./readingPresentationController.js";
import { createSearchController } from "./searchController.js";
import { createSensitiveDataScanController } from "./sensitiveDataScanController.js";
import { createSecureRedactionController } from "./secureRedactionController.js";
import { createSanitizationController } from "./sanitizationController.js";
import { createShareActionController } from "./shareActionController.js";
import { createTtsActionController } from "./ttsActionController.js";

const defaultExportActions = Object.freeze({
  exportPage: exportSessionPageToImage,
  extractPages: extractSessionPages,
  convertImage: convertAndSaveImage,
});

/** Composes feature workflows and search without owning document lifecycle state. */
export function createFeatureRuntime({
  viewer,
  annotation,
  documentSession,
  searchState,
  sensitiveScanState,
  secureRedactionState,
  sanitizationState,
  ui,
  asyncPrompt,
  viewerRuntime,
  documentRuntime,
  callbacks,
  tick,
  onToast,
  onError,
  exportActions = defaultExportActions,
  createExportController = createDocumentExportController,
  createSearchController: createSearch = createSearchController,
  createSensitiveDataScanController: createSensitiveScan = createSensitiveDataScanController,
  createSecureRedactionController: createSecureRedaction = createSecureRedactionController,
  createSanitizationController: createSanitization = createSanitizationController,
  listenToEvent = listen,
}) {
  const featureContext = getRuntimeFeatureContext();
  const featureUi = (featureId) => getFeaturePresentation(featureId, featureContext);
  function reportUnavailable(error) {
    if (!(error instanceof FeatureUnavailableError)) return false;
    onToast(error.presentation?.warning || error.presentation?.reason || error.message);
    return true;
  }

  function runUiAction(featureId, callback) {
    void guardFeatureAction(featureId, featureContext, callback)
      .catch((error) => { if (!reportUnavailable(error)) throw error; });
  }

  const searchController = createSearch({
    state: searchState,
    search: searchSessionPdfText,
    getContext: () => ({
      mounted: callbacks.isMounted(),
      isPdf: documentRuntime.isPdf,
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
    }),
    getPageOrder: () => viewer.pageOrder,
    setPageIndex: (index) => { viewer.currentPageIndex = index; },
    tick,
    scrollToPage: (pageIndex) => {
      viewerRuntime.navigation.scrollToPhysicalPage(pageIndex, { block: "center" });
    },
    onDroppedMatches: (count) => console.warn(`${count} PDF arama eşleşmesi geçerli kutusu olmadığı için gösterilmedi.`),
    onError: (error) => console.error("Arama hatası:", error),
  });

  const sensitiveScanController = createSensitiveScan({
    state: sensitiveScanState,
    startScan: (sessionId, contentGeneration, patterns) =>
      startSensitiveDataScanAction(
        sessionId,
        contentGeneration,
        patterns,
        featureContext,
      ),
    getContext: () => ({
      panelOpen: !!ui.panels.sensitiveDataScan,
      isPdf: documentRuntime.isPdf,
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
    }),
  });

  const secureRedactionController = secureRedactionState
    ? createSecureRedaction({
        state: secureRedactionState,
        getContext: () => ({
          panelOpen: !!ui.panels.secureRedaction,
          isPdf: documentRuntime.isPdf,
          sessionId: documentSession.activeSessionId,
          contentGeneration: documentSession.contentGeneration,
          displayPath: documentRuntime.activeDisplayPath,
          fileName: documentRuntime.fileName,
        }),
      })
    : Object.freeze({
        start() {}, destroy() {}, plan() {}, execute() {}, cancelPlanning() {}, cancelExecution() {},
        reconcileContext() {}, addManualRegion() {}, promoteSensitiveScanItem() {}, removeRegion() {},
        clearRegions() {}, updatePolicy() {}, setAcknowledgement() {}, setSelectionMode() {}, newRedaction() {},
      });

  const sanitizationController = sanitizationState
    ? createSanitization({
        state: sanitizationState,
        getContext: () => ({
          panelOpen: !!ui.isModalOpen?.("sanitization"),
          isPdf: documentRuntime.isPdf,
          sessionId: documentSession.activeSessionId,
          contentGeneration: documentSession.contentGeneration,
          fileName: documentRuntime.fileName,
        }),
      })
    : Object.freeze({
        start() {}, destroy() {}, inspect() {}, updatePolicy() {}, setAcknowledgement() {}, execute() {},
        cancel() {}, cancelInspection() {}, cancelExecution() {}, reconcileContext() {}, reset() {},
      });

  function start() {
    searchController.start();
    sensitiveScanController.start();
    secureRedactionController.start();
    sanitizationController.start();
    tts.start();
  }

  function destroy() {
    searchController.destroy();
    sensitiveScanController.destroy();
    secureRedactionController.destroy();
    sanitizationController.destroy();
    tts.destroy();
  }

  const featureRequestContext = () => ({
    isPdf: documentRuntime.isPdf,
    sessionId: documentSession.activeSessionId,
    contentGeneration: documentSession.contentGeneration,
    // Backend commands take physical page indices; labels use the display position.
    pageIndex: toPhysicalPageIndex(viewer.pageOrder, viewer.currentPageIndex),
    displayPageIndex: viewer.currentPageIndex,
    imagePath: viewer.activeImageSourcePath,
    viewer,
    ui,
  });

  const ocr = createOcrActionController({
    getContext: featureRequestContext,
    runOcr: performOcrOnPageAction,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
    onToast,
    present: (displayPageIndex, text) => globalThis.alert?.(`Sayfa ${displayPageIndex + 1} OCR Sonucu:\n\n${text}`),
  });
  const share = createShareActionController({
    getContext: featureRequestContext,
    shareSession: shareFileMacosAction,
    sharePath: shareExternalFileMacosAction,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
    onToast,
  });
  const tts = createTtsActionController({
    getContext: featureRequestContext,
    extractText: exportSessionTextContent,
    performOcr: performOcrOnPageAction,
    speak: speakTextAction,
    stop: stopSpeaking,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
    onToast,
    listenToEvent,
  });
  const reading = createReadingPresentationController({
    getContext: featureRequestContext,
    extractText: exportSessionTextContent,
    performOcr: performOcrOnPageAction,
    featureContext,
    onToast,
  });
  const transform = createDocumentTransformController({
    getContext: () => ({
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
      path: documentRuntime.activeDisplayPath,
    }),
    openFile: open,
    saveFile: save,
    merge: mergePdfsDirectAction,
    openDocument: documentRuntime.openFile,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
    onToast,
    onError,
  });
  const documentExportController = createExportController({
    getContext: () => ({
      isPdf: documentRuntime.isPdf,
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
      imagePath: viewer.activeImageSourcePath,
      pageIndex: toPhysicalPageIndex(viewer.pageOrder, viewer.currentPageIndex),
      displayPageIndex: viewer.currentPageIndex,
      pageOrder: viewer.pageOrder,
      fileName: documentRuntime.fileName,
      rotation: viewer.rotation,
      flipH: viewer.flipH,
      flipV: viewer.flipV,
    }),
    saveFile: save,
    requestInput: (options) => asyncPrompt.request(options),
    onToast,
    onError,
    ...exportActions,
  });

  function toggleDarkMode() {
    viewer.isSmartDarkMode = !viewer.isSmartDarkMode;
    onToast(viewer.isSmartDarkMode ? "Akıllı Gece Modu Açıldı" : "Normal Mod");
    callbacks.saveSettings();
  }

  async function continuityCamera() {
    try {
      return await guardFeatureAction("macos_continuity_camera", featureContext, () => {
        if (!documentRuntime.isPdf) onToast("Süreklilik kamerası taraması sadece PDF belgeleri için aktiftir.");
        else onToast("Süreklilik kamerası session-safe sayfa ekleme tamamlanana kadar devre dışı.");
      });
    } catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  async function exportNotesMarkdown() {
    try {
      return await guardFeatureAction("markdown_notes_export", featureContext, async () => {
        const { totalNotes } = buildAnnotationMarkdown({
          fileName: documentRuntime.fileName,
          annotationsMap: annotation.annotationsMap,
        });
        if (totalNotes === 0) { onToast("Dışa aktarılacak henüz not veya vurgu yok!"); return; }
        const baseName = documentRuntime.fileName.replace(/\.[^.]+$/, "");
        const savePath = await save({
          title: "Notları Markdown (.md) Olarak Kaydet",
          defaultPath: `${baseName}_Notlar.md`,
          filters: [{ name: "Markdown Belgesi (*.md)", extensions: ["md"] }],
        });
        if (savePath) onToast("Notlar Markdown belgesi olarak aktarıldı!");
      });
    } catch (error) { if (!reportUnavailable(error)) throw error; }
  }

  const commandRouter = createCommandActionRouter({
    getCommand: getCommandById,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
    onUnknown: () => onToast("Bilinmeyen komut çalıştırılmadı."),
    actions: {
      "open-metadata": () => ui.openModal("metadata"),
      "export-page-image": documentExportController.exportCurrentPageImage,
      "open-bionic": reading.openBionicReading,
      "toggle-ruler": () => { ui.togglePanel("readingRuler"); onToast(ui.panels.readingRuler ? "Odak Cetveli (Line Focus) Açıldı" : "Odak Cetveli Kapatıldı"); },
      "open-analytics": () => ui.openModal("analytics"),
      "camera-scan": continuityCamera,
      "speak-page": tts.speakCurrentPage,
      "stop-speak": tts.stopSpeaking,
      "share-macos": share.share,
      "open-ai-assistant": () => ui.openModal("ai_assistant"),
      "run-ocr-page": ocr.run,
      "toggle-dark-mode": toggleDarkMode,
      "toggle-book-mode": viewerRuntime.navigation.toggleBookMode,
      "toggle-split-view": () => { viewer.isSplitView = !viewer.isSplitView; viewerRuntime.render(); onToast(viewer.isSplitView ? "Çift Görünüm (Split View) Aktif" : "Tek Görünüm"); },
      "toggle-sidebar": () => ui.togglePanel("sidebar"),
      "open-search": () => ui.setPanel("search", true),
      "export-notes-md": exportNotesMarkdown,
      "split-pdf": documentExportController.splitPdf,
      "merge-pdf": transform.mergePdf,
      "reset-zoom": viewerRuntime.navigation.resetZoom,
    },
  });

  const secureRedaction = Object.freeze({
    plan: () => runUiAction("redaction", secureRedactionController.plan),
    execute: () => runUiAction("redaction", secureRedactionController.execute),
    cancelPlanning: secureRedactionController.cancelPlanning,
    cancelExecution: secureRedactionController.cancelExecution,
    addManualRegion: (detail) => runUiAction("redaction", () => secureRedactionController.addManualRegion(detail)),
    promoteSensitiveScanItem: (item) => runUiAction("redaction", () => secureRedactionController.promoteSensitiveScanItem(item)),
    removeRegion: (id) => runUiAction("redaction", () => secureRedactionController.removeRegion(id)),
    clearRegions: () => runUiAction("redaction", secureRedactionController.clearRegions),
    updatePolicy: (patch) => runUiAction("redaction", () => secureRedactionController.updatePolicy(patch)),
    setAcknowledgement: (name, value) => runUiAction("redaction", () => secureRedactionController.setAcknowledgement(name, value)),
    setSelectionMode: (value) => runUiAction("redaction", () => secureRedactionController.setSelectionMode(value)),
    newRedaction: () => runUiAction("redaction", secureRedactionController.newRedaction),
    reconcileContext: secureRedactionController.reconcileContext,
  });

  const sanitization = Object.freeze({
    inspect: () => runUiAction("sanitization", sanitizationController.inspect),
    updatePolicy: (patch) => runUiAction("sanitization", () => sanitizationController.updatePolicy(patch)),
    setAcknowledgement: (name, value) => runUiAction("sanitization", () => sanitizationController.setAcknowledgement(name, value)),
    execute: () => runUiAction("sanitization", sanitizationController.execute),
    cancel: sanitizationController.cancel,
    cancelInspection: sanitizationController.cancelInspection,
    cancelExecution: sanitizationController.cancelExecution,
    reconcileContext: sanitizationController.reconcileContext,
    reset: sanitizationController.reset,
  });

  return Object.freeze({
    featureUi,
    runUiAction,
    commandRouter,
    searchController,
    start,
    destroy,
    scanSensitiveData: (patterns) => sensitiveScanController.scan(patterns),
    reconcileSensitiveDataContext: () => sensitiveScanController.reconcileContext(),
    secureRedaction,
    sanitization,
    getSensitiveDataHighlightsForPage: (pageIndex, viewerRotation) =>
      selectViewportHighlights(
        sensitiveScanState?.logicalHighlightsByPage,
        pageIndex,
        viewerRotation,
      ),
    invalidateSearch: searchController.invalidate,
    handleSearchQuery: (event) => runUiAction("search", () => searchController.handleQueryEvent(event)),
    handleSearchHighlight: (event) => searchController.handleHighlight(event?.detail || {}),
    searchNext: searchController.next,
    searchPrevious: searchController.previous,
    toggleDarkMode,
    openBionicReading: reading.openBionicReading,
    runOcr: ocr.run,
    speak: tts.speakCurrentPage,
    stopSpeaking: tts.stopSpeaking,
    share: share.share,
    mergePdf: transform.mergePdf,
    exportCurrentPageImage: documentExportController.exportCurrentPageImage,
    exportPdfStructure: documentExportController.exportPdfStructure,
    exportImage: documentExportController.exportImage,
    splitPdf: documentExportController.splitPdf,
  });
}
