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
  sanitizePdfDocumentAction,
  shareExternalFileMacosAction,
  shareFileMacosAction,
  speakTextAction,
  startSensitiveDataScanAction,
} from "../actions.js";
import {
  exportSessionPageToImage,
  extractSessionPages,
  searchSessionPdfText,
} from "../backend/session.js";
import {
  burnAnnotationsToPdf,
  convertAndSaveImage,
} from "../backend/document.js";
import { selectViewportHighlights } from "../../utils/pdfGeometry.js";
import { open, save } from "../platform/dialogs.js";
import { buildAnnotationMarkdown } from "./annotationMarkdown.js";
import { createCommandActionRouter } from "./commandActionRouter.js";
import { createDocumentExportController } from "./documentExportController.js";
import { createDocumentTransformController } from "./documentTransformController.js";
import { createOcrActionController } from "./ocrActionController.js";
import { createReadingPresentationController } from "./readingPresentationController.js";
import { createSearchController } from "./searchController.js";
import { createSensitiveDataScanController } from "./sensitiveDataScanController.js";
import { createShareActionController } from "./shareActionController.js";
import { createTtsActionController } from "./ttsActionController.js";

const defaultExportActions = Object.freeze({
  exportPage: exportSessionPageToImage,
  extractPages: extractSessionPages,
  convertImage: convertAndSaveImage,
  burnAnnotations: burnAnnotationsToPdf,
});

/** Composes feature workflows and search without owning document lifecycle state. */
export function createFeatureRuntime({
  viewer,
  annotation,
  documentSession,
  searchState,
  sensitiveScanState,
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
}) {
  const featureContext = getRuntimeFeatureContext();
  const featureUi = (featureId) => getFeaturePresentation(featureId, featureContext);
  const featureControlLabel = (featureId, label) => {
    const badge = featureUi(featureId).badge;
    return badge ? `[${label} · ${badge}]` : `[${label}]`;
  };

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
      globalThis.document?.getElementById(`page-${pageIndex}`)?.scrollIntoView({ behavior: "auto", block: "center" });
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

  function start() {
    searchController.start();
    sensitiveScanController.start();
  }

  function destroy() {
    searchController.destroy();
    sensitiveScanController.destroy();
  }

  const featureRequestContext = () => ({
    isPdf: documentRuntime.isPdf,
    sessionId: documentSession.activeSessionId,
    contentGeneration: documentSession.contentGeneration,
    pageIndex: viewer.currentPageIndex,
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
    present: (pageIndex, text) => globalThis.alert?.(`Sayfa ${pageIndex + 1} OCR Sonucu:\n\n${text}`),
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
    performOcr: performOcrOnPageAction,
    speak: speakTextAction,
    stop: stopSpeaking,
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
    onToast,
  });
  const reading = createReadingPresentationController({
    getContext: featureRequestContext,
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
    sanitize: sanitizePdfDocumentAction,
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
      isLoadedPdfDocument: documentRuntime.isLoadedPdfDocument,
      sessionId: documentSession.activeSessionId,
      contentGeneration: documentSession.contentGeneration,
      imagePath: viewer.activeImageSourcePath,
      pageIndex: viewer.currentPageIndex,
      pageOrder: viewer.pageOrder,
      fileName: documentRuntime.fileName,
      annotations: Object.values(annotation.annotationsMap).flat(),
      rotation: viewer.rotation,
      flipH: viewer.flipH,
      flipV: viewer.flipV,
    }),
    saveFile: save,
    requestInput: (options) => asyncPrompt.request(options),
    guard: guardFeatureAction,
    featureContext,
    reportUnavailable,
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
      "burn-in-pdf": documentExportController.burnInPdf,
    },
  });

  return Object.freeze({
    featureUi,
    featureControlLabel,
    runUiAction,
    commandRouter,
    searchController,
    start,
    destroy,
    scanSensitiveData: (patterns) => sensitiveScanController.scan(patterns),
    reconcileSensitiveDataContext: () => sensitiveScanController.reconcileContext(),
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
    sanitize: transform.sanitizeDocument,
    mergePdf: transform.mergePdf,
    exportCurrentPageImage: documentExportController.exportCurrentPageImage,
    exportPdfStructure: documentExportController.exportPdfStructure,
    exportImage: documentExportController.exportImage,
    splitPdf: documentExportController.splitPdf,
    burnInPdf: documentExportController.burnInPdf,
  });
}

