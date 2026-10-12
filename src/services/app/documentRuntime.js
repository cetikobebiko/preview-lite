import { ask, open, save } from "../platform/dialogs.js";
import {
  addRecentFile,
  getDocumentState,
  saveDocumentState,
} from "../../configManager.js";
import {
  authenticateDocumentSession,
  beginSessionTransition,
  cancelTransition,
  closeAllDocumentSessions,
  commitCandidateActivation,
  discardCandidateSession,
  discardTransition,
  getActiveDocumentSessionSnapshot,
  getDocumentSessionSnapshot,
  getSessionPageCount,
  getSessionPdfMetadata,
  getSessionPdfOutline,
  openDocumentSession,
  prepareCandidateActivation,
  probeCandidateRender,
  readImageFile,
  rollbackCandidateActivation,
  saveTransition,
} from "../backend/index.js";
import { createDocumentLifecycleController } from "./documentLifecycleController.js";
import { createDocumentOpenController } from "./documentOpenController.js";
import { createDocumentPositionMemory } from "./documentPositionMemory.js";
import { toPhysicalPageIndex } from "./viewerNavigationModel.js";

const transitionApi = Object.freeze({
  beginSessionTransition,
  saveTransition,
  discardTransition,
  cancelTransition,
  closeAllDocumentSessions,
});

/** Owns document open/transition/save/recovery and post-mutation document refresh. */
export function createDocumentRuntime({
  documentSession,
  renderSession,
  viewer,
  annotation,
  viewerController,
  ui,
  tick,
  getCanvas,
  getIsMounted,
  invalidateSearch,
  onToast,
  onError,
  passwordPresentation,
  applyPageScopedMutation = () => {},
}) {
  const isPdf = () => Boolean(documentSession.active);
  const activeDisplayPath = () => documentSession.active?.displayPath ?? "";
  const fileName = () => isPdf()
    ? (documentSession.active?.fileName ?? "")
    : (viewer.activeImageSourcePath.split("/").pop() || "Dosya");
  const isLoadedPdfDocument = () => isPdf()
    && !viewer.isDocumentLoading
    && viewer.pageOrder.length > 0;

  function applyBackendSessionSnapshot(snapshot, { activateRender = false } = {}) {
    if (!snapshot || snapshot.id !== documentSession.activeSessionId) return false;
    if (!documentSession.applySnapshot(snapshot)) return false;
    if (activateRender) {
      void viewerController.activatePdf(snapshot).catch(() => {
        console.warn("Render document activation failed.");
      });
    }
    return true;
  }

  const positionMemory = createDocumentPositionMemory({
    viewer,
    getPath: () => activeDisplayPath() || viewer.activeImageSourcePath,
    getState: getDocumentState,
    saveState: saveDocumentState,
    onToast,
    scrollToPage: (displayIndex) => {
      const physicalIndex = toPhysicalPageIndex(viewer.pageOrder, displayIndex);
      if (physicalIndex < 0) return;
      globalThis.document?.getElementById(`page-${physicalIndex}`)?.scrollIntoView({ behavior: "auto", block: "start" });
    },
  });

  async function chooseDocumentTransition({ snapshot }) {
    if (!snapshot.dirty) return { action: "discard" };
    const shouldSave = await ask(
      "Kaydedilmemiş değişiklikleri kaydetmek ister misiniz?",
      { title: "Kaydedilmemiş Değişiklikler", kind: "warning", okLabel: "Kaydet", cancelLabel: "Diğer Seçenekler" },
    );
    if (shouldSave) return { action: "save" };
    const shouldDiscard = await ask(
      "Değişiklikler kalıcı olarak atılsın mı?",
      { title: "Değişiklikleri At", kind: "warning", okLabel: "Vazgeç", cancelLabel: "İptal" },
    );
    return shouldDiscard ? { action: "discard" } : { action: "cancel" };
  }

  async function chooseTransitionSaveRecovery({ error }) {
    const code = error?.cause?.code;
    if (code === "TARGET_NOT_FOUND" || code === "DOCUMENT_SESSION_CONFLICT") {
      const useSaveAs = await ask(
        "Kayıt hedefi değişti veya silindi. Farklı Kaydet açılsın mı?",
        { title: "Kayıt Çakışması", kind: "warning", okLabel: "Farklı Kaydet", cancelLabel: "İptal" },
      );
      if (!useSaveAs) return { action: "cancel" };
      const targetPath = await save({
        title: "PDF Belgesini Farklı Kaydet",
        defaultPath: fileName(),
        filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
      });
      return targetPath ? { action: "save_as", targetPath } : { action: "cancel" };
    }
    const retry = await ask(
      "Belge kaydedilemedi. Aynı transition ile yeniden denensin mi?",
      { title: "Kayıt Başarısız", kind: "error", okLabel: "Yeniden Dene", cancelLabel: "İptal" },
    );
    return retry ? { action: "save" } : { action: "cancel" };
  }

  const lifecycleController = createDocumentLifecycleController({
    documentSession,
    applySnapshot: applyBackendSessionSnapshot,
    chooseSavePath: ({ defaultPath }) => save({
      title: "PDF Belgesini Farklı Kaydet",
      defaultPath,
      filters: [{ name: "PDF Belgesi (*.pdf)", extensions: ["pdf"] }],
    }),
    askSaveConflict: () => ask(
      "Kayıt hedefi haricen değişti veya silindi. Farklı Kaydet açılsın mı?",
      { title: "Kayıt Çakışması", kind: "warning", okLabel: "Farklı Kaydet", cancelLabel: "İptal" },
    ),
    getDefaultFileName: fileName,
    addRecentFile,
    onSaved: ({ kind }) => onToast(kind === "save_as" ? "Belge farklı kaydedildi." : "Belge kaydedildi."),
    onError: ({ kind, error }) => onError(`${kind === "save_as" ? "Farklı kaydetme" : "Kayıt"} başarısız: ${error}`),
    onPasswordPrompt: passwordPresentation.show,
    onPasswordError: passwordPresentation.error,
    onPasswordDismiss: passwordPresentation.dismiss,
  });

  const openController = createDocumentOpenController({
    backend: {
      openDocumentSession,
      authenticateDocumentSession,
      getDocumentSessionSnapshot,
      getActiveDocumentSessionSnapshot,
      probeCandidateRender,
      discardCandidateSession,
      beginSessionTransition,
      saveTransition,
      discardTransition,
      cancelTransition,
      prepareCandidateActivation,
      commitCandidateActivation,
      rollbackCandidateActivation,
      readImageFile,
    },
    documentSession,
    documentLifecycleController: lifecycleController,
    viewer,
    annotation,
    viewerController,
    renderSession,
    getCanvas,
    getIsMounted,
    tick,
    applyBackendSessionSnapshot,
    chooseTransition: chooseDocumentTransition,
    chooseTransitionSaveRecovery,
    invalidateSearch,
    restoreDocumentPosition: positionMemory.restore,
    addRecentFile,
    getOutline: getSessionPdfOutline,
    getMetadata: getSessionPdfMetadata,
    chooseFile: () => open({
      multiple: false,
      filters: [{
        name: "Desteklenen Belgeler ve Görseller",
        extensions: ["pdf", "png", "jpg", "jpeg", "webp", "bmp", "svg"],
      }],
    }),
    setError: onError,
  });

  async function applyMutationSnapshot(snapshot, { mapPageIndex = null, rotation = null } = {}) {
    // The backend already renumbered/rotated the pages; apply even if a newer snapshot won the race.
    if ((mapPageIndex || rotation) && snapshot?.id && snapshot.id === documentSession.activeSessionId) {
      applyPageScopedMutation(snapshot.id, { mapPageIndex, rotation });
    }
    if (!applyBackendSessionSnapshot(snapshot, { activateRender: true })) return;
    const pageCount = await getSessionPageCount(snapshot.id, snapshot.contentGeneration);
    if (documentSession.activeSessionId !== snapshot.id
      || documentSession.contentGeneration !== snapshot.contentGeneration) return;
    viewer.pageOrder = Array.from({ length: pageCount }, (_, index) => index);
    viewer.currentPageIndex = Math.min(viewer.currentPageIndex, Math.max(0, pageCount - 1));
    // Physical indices were renumbered by the backend; history entries no longer name the same pages.
    viewer.historyStack = [];
    viewer.forwardStack = [];
    ui.closeModal("page_organizer");
  }

  return Object.freeze({
    get isPdf() { return isPdf(); },
    get activeDisplayPath() { return activeDisplayPath(); },
    get fileName() { return fileName(); },
    get isLoadedPdfDocument() { return isLoadedPdfDocument(); },
    get isDirty() { return documentSession.dirty; },
    lifecycleController,
    openController,
    coordinator: openController.coordinator,
    transitionApi,
    chooseDocumentTransition,
    applyBackendSessionSnapshot,
    applyMutationSnapshot,
    save: lifecycleController.save,
    saveAs: lifecycleController.saveAs,
    selectFile: openController.selectFile,
    openFile: openController.requestFile,
    savePosition: positionMemory.save,
  });
}
